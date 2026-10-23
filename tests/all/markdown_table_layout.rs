use viem_core::document::{Encoding, Format};
use viem_core::layout::{LayoutEngine, LayoutPoint, MockTextMeasurementProvider, ViewLayout};
use viem_core::Document;
use viem_core::command::{CommandStatus, InputEvent, Key};

fn audit_key(core: &mut viem_core::Core<MockTextMeasurementProvider>, view: viem_core::ViewId, key: Key) {
    let outcome = core.handle_with_layout(view, viem_core::CoreEvent::Input(InputEvent::Key(key.clone())))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}"));
    if let Some(command) = outcome.command {
        assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending)
            || (key == Key::Escape && matches!(command.status, CommandStatus::Cancelled)),
            "{key:?}: {:?}", command.status);
    }
}

fn audit_place(core: &mut viem_core::Core<MockTextMeasurementProvider>, view: viem_core::ViewId, at: usize) {
    core.handle(view, viem_core::CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: viem_core::document::BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
}

fn audit_caret(core: &viem_core::Core<MockTextMeasurementProvider>, view: viem_core::ViewId) {
    let commands = core.command_state(view).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    snapshot.logical_endpoint_geometry(commands.cursor(), commands.boundary_affinity())
        .or_else(|_| snapshot.logical_endpoint_geometry(commands.cursor(), viem_core::document::BoundaryAffinity::Upstream))
        .unwrap_or_else(|error| panic!("caret {}: {error:?}; text {:?}; rows {:?}", commands.cursor(), core.document().text(),
            snapshot.rows.iter().map(|row| (&row.text_range, row.table_cell.as_ref().map(|cell| (cell.row,cell.column)))).collect::<Vec<_>>()));
}

#[test]
fn audit_ligature_interiors_support_vertical_and_row_edge_motion() {
    for format in [Format::PlainText, Format::Markdown, Format::Code] {
        for key in [Key::Down, Key::Up, Key::Home, Key::End, Key::Char('j'), Key::Char('k')] {
            let mut core = viem_core::Core::new(document("office\nnext line here", format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
            audit_place(&mut core, view, 2);
            audit_key(&mut core, view, key);
            audit_caret(&core, view);
        }
    }
}

#[test]
fn audit_narrow_table_resize_scroll_and_zoom_remain_editable() {
    for source in ["| a | b | c |\n| :-: | --- | ---: |\n| 1 | 2 | 3 |", "- | a | b |\n  | - | - |\n  | 1 | 2 |"] {
        let mut core = viem_core::Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 900.);
        for (width, height) in [(120.,200.), (40.,30.), (300.,200.)] {
            core.handle(view, viem_core::CoreEvent::Resize { width, height }).unwrap();
            assert!(core.layout(view).unwrap().snapshot().is_some());
            audit_key(&mut core, view, Key::SelectAll);
            core.handle(view, viem_core::CoreEvent::SetScale(0.5))
                .unwrap_or_else(|error| panic!("{source:?}, {width} x {height}: {error:?}"));
            audit_caret(&core, view);
            audit_key(&mut core, view, Key::Escape);
            core.handle(view, viem_core::CoreEvent::SetScale(1.)).unwrap();
            core.handle(view, viem_core::CoreEvent::SetViewportOrigin {left: 0., top: Some(f32::MAX)}).unwrap();
        }
        audit_key(&mut core, view, Key::SelectAll);
        for scale in [0.5, 2., 1.] {
            core.handle(view, viem_core::CoreEvent::SetScale(scale)).unwrap();
            audit_caret(&core, view);
        }
    }
}

#[test]
fn audit_wide_table_moves_between_rows_and_keeps_every_cell_caret() {
    let source = "| one | two | three | four | five | six |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |\n| alpha | beta | gamma | delta | epsilon | zeta |";
    for width in [70., 120., 300.] {
        let mut core = viem_core::Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), width, 500.);
        audit_key(&mut core, view, Key::Char('i'));
        for _ in 0..70 {
            audit_key(&mut core, view, Key::Right);
            audit_caret(&core, view);
        }
        let at = core.document().projection().tables()[0].rows[0].cells[5].range.start;
        let below = core.document().projection().tables()[0].rows[1].cells[5].range.clone();
        audit_place(&mut core, view, at);
        audit_key(&mut core, view, Key::Down);
        let cursor = core.command_state(view).unwrap().cursor();
        assert!(below.start <= cursor && cursor <= below.end, "{width}: {cursor} outside {below:?}");
        audit_caret(&core, view);
    }
}

#[test]
fn audit_block_selection_in_prose_ignores_unrelated_wide_table() {
    let source = "para\n\n| a | b | c | d |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |";
    for key in ['I', 'A', 'd'] {
        let mut core = viem_core::Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 120., 200.);
        audit_key(&mut core, view, Key::Ctrl('v'));
        audit_key(&mut core, view, Key::Char(key));
        audit_caret(&core, view);
    }
}

#[test]
fn audit_empty_table_word_motion_always_has_geometry() {
    for source in ["| | |\n|-|-|\n| | |", "| a | b |\n| - | - |\n|  |  |"] {
        let mut core = viem_core::Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
        for (at, key) in [(1,Key::WordRight),(5,Key::WordLeft)] {
            if at > core.document().text().len() { continue; }
            audit_place(&mut core, view, at);
            audit_key(&mut core, view, key);
            audit_caret(&core, view);
        }
    }
}

#[test]
fn audit_markdown_demo_table_navigation_keeps_current_caret_geometry() {
    use viem_core::command::LineMode;
    let source = include_str!("../../docs/markdown_demo.md");
    for line_mode in [LineMode::Visual, LineMode::PhysicalSource] {
        for width in [120., 700.] {
            let mut core = viem_core::Core::new(document(source, Format::Markdown));
            let view = core.add_view(MockTextMeasurementProvider::new(), width, 300.);
            core.handle(view, viem_core::CoreEvent::SetLineMode(line_mode)).unwrap();
            let text = core.document().text();
            let mut offsets = Vec::new();
            for table in core.document().projection().tables() {
                for row in &table.rows {
                    for cell in &row.cells {
                        offsets.push(cell.range.start);
                        offsets.push(cell.range.end);
                        offsets.extend(text[cell.range.clone()].char_indices()
                            .step_by(7).map(|(offset, _)| cell.range.start + offset));
                    }
                }
            }
            offsets.sort_unstable();
            offsets.dedup();
            assert!(offsets.len() > 50, "demo must exercise its current tables");
            for at in offsets {
                for key in [Key::Left, Key::Right, Key::WordLeft, Key::WordRight,
                    Key::Char('b'), Key::Char('e'), Key::Char('w'),
                    Key::Char('{'), Key::Char('}')] {
                    audit_place(&mut core, view, at);
                    audit_key(&mut core, view, key);
                    audit_caret(&core, view);
                }
            }
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn audit_multiline_rtl_table_block_delete_has_exact_layout() {
    let source = "| a | b |\n| - | - |\n| 1 | 2 |\n\ntext after";
    let mut core = viem_core::Core::new(document(source, Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    audit_key(&mut core, view, Key::Char('a'));
    audit_key(&mut core, view, Key::Enter);
    core.handle_with_layout(view, viem_core::CoreEvent::Input(InputEvent::text("العربية"))).unwrap();
    audit_key(&mut core, view, Key::Escape);
    audit_key(&mut core, view, Key::Ctrl('v'));
    audit_key(&mut core, view, Key::Char('d'));
    audit_caret(&core, view);
}

#[test]
fn audit_block_edit_and_resize_keep_large_table_materialization_bounded() {
    let source = format!("para\n\n|{}\n|{}\n|{}\n\n{}",
        " header |".repeat(1000), " --- |".repeat(1000), " value |".repeat(1000),
        "unrelated paragraph\n\n".repeat(2000));
    let mut core = viem_core::Core::new(document(&source, Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 120., 200.);
    audit_key(&mut core, view, Key::Ctrl('v'));
    audit_key(&mut core, view, Key::Char('d'));
    let after_edit = core.document().source_bytes();
    assert_eq!(&after_edit, &source.as_bytes()[1..]);
    for width in [40., 300., 120.] {
        core.handle(view, viem_core::CoreEvent::Resize {width, height: 200.}).unwrap();
        audit_caret(&core, view);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.table_cells().len() < 100, "shaped unrelated table columns");
        assert!(snapshot.coverage.hard_lines().len() < 100, "laid out unrelated suffix");
    }
    assert_eq!(core.document().source_bytes(), after_edit);
    audit_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn audit_short_viewports_keep_styled_caret_geometry_after_escape() {
    for (format, source) in [(Format::Markdown, "**`code in bold`**"),
        ] {
        let mut core = viem_core::Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 120., 30.);
        audit_key(&mut core, view, Key::Char('i'));
        audit_key(&mut core, view, Key::Escape);
        audit_caret(&core, view);
        core.handle(view, viem_core::CoreEvent::Resize {width: 300., height: 200.}).unwrap();
        audit_caret(&core, view);
    }
}

#[test]
fn audit_narrow_nested_table_insert_and_row_delete_recover_on_resize() {
    use viem_core::document::TableEditIntent;
    let mut core = viem_core::Core::new(document(">> nested", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 900.);
    core.handle_with_layout(view, viem_core::CoreEvent::TableEdit {
        document: core.document().id(), revision: core.document().revision(),
        intent: TableEditIntent::Insert {range: 0..0, columns: 2, body_rows: 1},
    }).unwrap();
    core.handle(view, viem_core::CoreEvent::Resize {width: 40., height: 900.}).unwrap();
    assert!(core.layout(view).unwrap().snapshot().is_some());

    let mut core = viem_core::Core::new(document("- | a | b |\n  | - | - |\n  | 1 | 2 |", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 40., 30.);
    let table = core.document().projection().tables()[0].id;
    core.handle_with_layout(view, viem_core::CoreEvent::TableEdit {
        document: core.document().id(), revision: core.document().revision(),
        intent: TableEditIntent::DeleteRows { table, rows: 0..1 },
    }).unwrap();
    core.handle(view, viem_core::CoreEvent::SetViewportOrigin {left: 0., top: Some(f32::MAX)}).unwrap();
    for (width,height) in [(40.,30.),(300.,200.)] {
        core.handle(view, viem_core::CoreEvent::Resize {width,height}).unwrap();
        assert!(!core.layout(view).unwrap().snapshot().unwrap().rows.is_empty());
    }
}
fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
#[test]
fn rich_cells_share_columns_and_rows_without_wrapping() {
    let source = "| A | B |\n| --- | ---: |\n| narrow | extremely wide cell |\n| X<br>Y | |";
    let document = document(source, Format::Markdown);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(80., 500.);
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    assert_eq!(snapshot.table_cells().len(), 6);
    let cells = snapshot.table_cells();
    assert_eq!(cells[0].rect.width, cells[2].rect.width);
    assert_eq!(cells[1].rect.width, cells[3].rect.width);
    assert_eq!(cells[0].rect.y, cells[1].rect.y);
    assert!(cells[4].rect.height > cells[2].rect.height);
    assert!(snapshot.content_width > 80.);
    for cell in cells {
        let point = snapshot
            .hit_test(LayoutPoint {
                x: cell.rect.x + 8.,
                y: cell.rect.y + 8.,
            })
            .unwrap();
        assert!(
            cell.text_range.start <= point.text_offset && point.text_offset <= cell.text_range.end
        );
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, HardLineLayoutRegion,
    LayoutCancellationToken, LayoutExecutionContext, LayoutJobId, LayoutJobPriority,
    LayoutJobRegion, RegionalLayoutSnapshot,
};
fn region(
    document: &Document,
    engine: &mut LayoutEngine<MockTextMeasurementProvider>,
    view: &mut ViewLayout,
    lines: std::ops::Range<usize>,
    id: u64,
) -> RegionalLayoutSnapshot {
    let request = prepare_layout_job(
        document,
        view,
        inspect_layout_provider(engine),
        LayoutJobId(id),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(lines).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool)
        .unwrap()
        .regional_snapshot()
        .clone()
}

#[test]
fn regional_table_rows_match_complete_geometry_in_both_views() {
    let source = "| A | B |\n| --- | ---: |\n| X<br>Y | several words |\n| Z | |";
    for format in [Format::Markdown] {
        let document = document(source, format);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(100., 500.);
        engine.relayout(&document, &mut view).unwrap();
        let full = view.snapshot().unwrap().clone();
        for index in 0..document.projection().presentation_line_count() {
            let candidate = region(
                &document,
                &mut engine,
                &mut view,
                index..index + 1,
                index as u64 + 1,
            );
            let line = &candidate.lines()[0];
            let rows = full
                .rows
                .iter()
                .filter(|row| row.hard_line_index == index)
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), line.rows().len());
            for (expected, actual) in rows.iter().zip(line.rows()) {
                assert_eq!(expected.text_range, actual.text_range);
                assert_eq!(expected.width, actual.width);
                assert_eq!(expected.paragraph_content_x, actual.paragraph_content_x);
            }
            if format == Format::Markdown {
                let cell = rows[0].table_cell.as_ref().unwrap();
                assert!((line.height() as f32 - cell.rect.height).abs() < 0.01);
            }
        }
    }
}

#[test]
fn cold_large_table_is_bounded_and_local_maximum_deletion_shrinks_without_rescan() {
    let source = "| Header | Value |\n| --- | --- |\n".to_owned()
        + &(0..1000)
            .map(|row| format!("| row{row} | value{row} |\n"))
            .collect::<String>();
    let mut document = document(&source, Format::Markdown);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(300., 120.);
    let first = region(&document, &mut engine, &mut view, 0..3, 1);
    assert!(
        engine.provider().request_calls() < 180,
        "{}",
        engine.provider().request_calls()
    );
    assert_eq!(first.lines().len(), 3);
    assert!(first.work_statistics().table_measured_cells() < 150);
    let target = document.projection().tables()[0].rows[900].cells[1]
        .range
        .clone();
    document
        .replace(target, "A considerably wider offscreen maximum")
        .unwrap();
    let wide = region(&document, &mut engine, &mut view, 900..901, 2);
    let wide_width = wide.lines()[0].rows()[1]
        .table_cell
        .as_ref()
        .unwrap()
        .rect
        .width;
    let before = engine.provider().request_calls();
    let target = document.projection().tables()[0].rows[900].cells[1]
        .range
        .clone();
    document.replace(target, "x").unwrap();
    let narrow = region(&document, &mut engine, &mut view, 0..3, 3);
    let narrow_width = narrow.lines()[0].rows()[1]
        .table_cell
        .as_ref()
        .unwrap()
        .rect
        .width;
    assert!(narrow_width < wide_width);
    assert!(engine.provider().request_calls() - before < 150);
}

#[test]
fn visual_command_entry_follows_explicit_lines_then_same_table_column() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::{Core, CoreEvent};
    let document = document(
        "Before\n\n| H1 | H2 |\n| --- | --- |\n| a<br>b | c |\n| d | e |\n\nAfter",
        Format::Markdown,
    );
    let table = document.projection().tables()[0].clone();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    let place = |core: &mut Core<MockTextMeasurementProvider>, at| {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
    };
    let send = |core: &mut Core<MockTextMeasurementProvider>, keys: &str| {
        for ch in keys.chars() {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
                .unwrap();
        }
    };
    place(&mut core, table.rows[0].cells[1].range.start);
    send(&mut core, "gj");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        table.rows[1].cells[1].range.start
    );
    send(&mut core, "gj");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        table.rows[2].cells[1].range.start
    );
    send(&mut core, "2gk");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        table.rows[0].cells[1].range.start
    );
    place(&mut core, table.rows[1].cells[0].range.start);
    send(&mut core, "gj");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        table.rows[1].cells[0].range.start + 2
    );
    send(&mut core, "gj");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        table.rows[2].cells[0].range.start
    );
    send(&mut core, "gj");
    assert!(core.command_state(view).unwrap().cursor() > table.range.end);
}

#[test]
fn fitting_table_has_no_horizontal_scroll_range() {
    let document = document(
        "| First | Second |\n| --- | --- |\n| small | table |",
        Format::Markdown,
    );
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(740., 500.);
    view.set_insets(viem_core::layout::EdgeInsets {
        left: 20.,
        right: 20.,
        top: 10.,
        bottom: 10.,
    });
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(view.snapshot().unwrap().content_width, 740.);
    assert_eq!(view.maximum_viewport_left(), Some(0.));
}

#[test]
fn cell_padding_borders_and_nearby_pixels_preserve_hit_column() {
    let document = document(
        "| Feature | Example |\n| --- | --- |\n| <br> | |",
        Format::Markdown,
    );
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(740., 500.);
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    for cell in snapshot.table_cells() {
        for y in [
            cell.rect.y,
            cell.rect.y + 0.1,
            cell.rect.y + cell.rect.height - 0.1,
        ] {
            for x in [
                cell.rect.x,
                cell.rect.x + cell.rect.width * 0.5,
                cell.rect.x + cell.rect.width - 0.1,
            ] {
                let hit = snapshot.hit_test(LayoutPoint { x, y }).unwrap();
                assert!(
                    cell.text_range.start <= hit.text_offset
                        && hit.text_offset <= cell.text_range.end,
                    "cell={cell:?}, x={x}, y={y}, hit={hit:?}"
                );
            }
        }
    }
    let second = &snapshot.table_cells()[1];
    let hit = snapshot
        .hit_test(LayoutPoint {
            x: second.rect.x + second.rect.width / 2.,
            y: second.rect.y - 0.1,
        })
        .unwrap();
    assert!(second.text_range.start <= hit.text_offset && hit.text_offset <= second.text_range.end);
}

#[test]
fn idle_width_refinement_survives_fresh_worker_engines_in_both_views() {
    for format in [Format::Markdown] {
        let source = "| H | V |\n| - | - |\n".to_owned()
            + &"| row | value |\n".repeat(180)
            + "| last | a much wider final offscreen value |";
        let document = document(&source, format);
        let mut core = viem_core::Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        let mut iterations = 0;
        while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
            iterations += 1;
            assert!(
                iterations < 10,
                "width discovery made no progress for {format:?}"
            );
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate =
                compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            assert!(
                candidate
                    .regional_snapshot()
                    .work_statistics()
                    .table_measured_cells()
                    <= 128 + 10 * candidate.regional_snapshot().lines().len()
            );
            assert!(core.install_view_table_refinement(view, candidate).unwrap());
        }
        assert!(iterations > 0, "fixture must require idle discovery");
        let actual = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!actual.has_provisional_table_widths());
        let mut full_view = ViewLayout::new(400., 100.);
        LayoutEngine::new(MockTextMeasurementProvider::new())
            .relayout(core.document(), &mut full_view)
            .unwrap();
        let full = full_view.snapshot().unwrap();
        assert_eq!(actual.rows[0].width, full.rows[0].width);
        assert_eq!(
            actual.rows[0]
                .clusters
                .iter()
                .map(|cluster| cluster.x)
                .collect::<Vec<_>>(),
            full.rows[0]
                .clusters
                .iter()
                .map(|cluster| cluster.x)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn late_styled_and_punctuation_edits_parse_only_the_affected_table_row() {
    use viem_core::document::{ModelRequest, TextEdit};
    for format in [Format::Markdown] {
        let source = "before\n\n| A | B |\n| - | - |\n".to_owned()
            + &"| **word** | value |\n".repeat(10000)
            + "\nafter";
        let mut document = document(&source, format);
        let row = &document.projection().tables()[0].rows[9000];
        let text = document
            .projection()
            .text_tree()
            .slice(row.cells[0].range.clone())
            .unwrap();
        let at = row.cells[0].range.start + text.find("word").unwrap() + 2;
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at, ".")],
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert!(work.decoded_source_bytes() < 512, "{format:?}: {work:?}");
        assert!(
            work.persistent_records_copied() < 4000,
            "{format:?}: {work:?}"
        );
        document.commit_model_transaction(prepared).unwrap();
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), fresh.text());
        for index in [0, 8999, 9000, 9001, 10000] {
            let actual = &document.projection().tables()[0].rows[index];
            let expected = &fresh.projection().tables()[0].rows[index];
            assert_eq!(actual.range, expected.range);
            assert_eq!(actual.source_range, expected.source_range);
            for (a, b) in actual.cells.iter().zip(&expected.cells) {
                assert_eq!(a.range, b.range);
                assert_eq!(a.source_range, b.source_range);
            }
        }
    }
}

#[test]
fn giant_cells_discover_width_in_bounded_worker_slices_and_keep_native_glyphs_bounded() {
    for format in [Format::Markdown] {
        let following = "\n\nFollowing prose.".repeat(50);
        for tail in ["", following.as_str()] {
            let source = format!(
                "| H | V |\n| - | - |\n| {} | small |{}",
                "word ".repeat(100_000),
                tail
            );
            let document = document(&source, format);
            let mut core = viem_core::Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
            let first = core.layout(view).unwrap().snapshot().unwrap();
            assert!(first.has_provisional_table_widths());
            assert!(
                first
                    .rows
                    .iter()
                    .map(|row| row.clusters.len())
                    .sum::<usize>()
                    < 12000
            );
            let mut loops = 0;
            while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
                loops += 1;
                assert!(loops < 16, "{format:?} discovery did not finish");
                let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
                let candidate =
                    compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                        .unwrap();
                let snapshot = candidate.regional_snapshot();
                assert!(
                    snapshot.work_statistics().table_measured_text_bytes() < 140_000,
                    "{format:?}: {}",
                    snapshot.work_statistics().table_measured_text_bytes()
                );
                assert!(
                    snapshot
                        .lines()
                        .iter()
                        .flat_map(|line| line.rows())
                        .map(|row| row.clusters.len())
                        .sum::<usize>()
                        < 12000
                );
                assert!(core.install_view_table_refinement(view, candidate).unwrap());
            }
            assert!(loops > 3);
            assert!(!core
                .layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .has_provisional_table_widths());
            let table = &core.document().projection().tables()[0];
            let cell = table.rows[1].cells[0].range.clone();
            core.handle(
                view,
                viem_core::CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: cell.start + 1,
                    affinity: viem_core::document::BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let left = core.layout(view).unwrap().snapshot().unwrap().content_width / 2.;
            core.handle(
                view,
                viem_core::CoreEvent::SetViewportOrigin { left, top: None },
            )
            .unwrap();
            for width in [600., 450., 800., 600.] {
                core.handle(
                    view,
                    viem_core::CoreEvent::Resize {
                        width,
                        height: 200.,
                    },
                )
                .unwrap();
                assert!(
                    (core.layout(view).unwrap().viewport_left() - left).abs() < 1.,
                    "resize moved a manual viewport back to the offscreen caret"
                );
                let layout = core.layout(view).unwrap();
                assert!(
                    layout
                        .snapshot()
                        .unwrap()
                        .rows
                        .iter()
                        .flat_map(|row| &row.clusters)
                        .any(|cluster| cell.contains(&cluster.text_range.start)
                            && cluster.x + cluster.advance >= left
                            && cluster.x <= left + layout.width()),
                    "{format:?}: resize width {width} omitted manual band at {left}, clusters {:?}",
                    layout
                        .snapshot()
                        .unwrap()
                        .rows
                        .iter()
                        .filter(|row| row.text_range.contains(&cell.start))
                        .map(|row| (
                            row.text_range.clone(),
                            row.clusters.first().map(|c| (c.text_range.clone(), c.x)),
                            row.clusters.last().map(|c| (c.text_range.clone(), c.x))
                        ))
                        .collect::<Vec<_>>()
                );
            }
            let mut refinements = 0;
            loop {
                let layout = core.layout(view).unwrap();
                assert!(
                    (layout.viewport_left() - left).abs() < 1.,
                    "manual viewport moved during idle refinement"
                );
                assert!(
                    layout
                        .snapshot()
                        .unwrap()
                        .rows
                        .iter()
                        .flat_map(|row| &row.clusters)
                        .any(|cluster| cell.contains(&cluster.text_range.start)
                            && cluster.x + cluster.advance >= left
                            && cluster.x <= left + layout.width()),
                    "{format:?}: visible giant-cell glyph band was omitted"
                );
                let Some(request) = core.prepare_view_table_refinement(view).unwrap() else {
                    break;
                };
                refinements += 1;
                assert!(refinements < 20);
                let candidate = compute_layout_job(
                    &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
                    &request,
                    LayoutExecutionContext::WorkerPool,
                )
                .unwrap();
                assert!(core.install_view_table_refinement(view, candidate).unwrap());
            }
        }
    }
}

#[test]
fn progressive_explicit_cell_lines_retain_maximum_width_and_complete_height() {
    let source = format!(
        "| H | V |\n| ---: | - |\n| {}last | x |",
        "a<br>".repeat(6000)
    );
    let document = document(&source, Format::Markdown);
    let mut core = viem_core::Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
    let mut loops = 0;
    while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
        loops += 1;
        assert!(loops < 16);
        let candidate = compute_layout_job(
            &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
            &request,
            LayoutExecutionContext::WorkerPool,
        )
        .unwrap();
        core.install_view_table_refinement(view, candidate).unwrap();
    }
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let cell = snapshot
        .table_cells()
        .iter()
        .find(|cell| cell.row == 1 && cell.column == 0)
        .unwrap();
    assert!(cell.rect.height > 6000. * 15., "{}", cell.rect.height);
    assert!(cell.rect.width < 200., "{}", cell.rect.width);
    let top = cell.rect.y + cell.rect.height * 0.6;
    core.handle(
        view,
        viem_core::CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let visible = snapshot
        .rows
        .iter()
        .filter(|row| row.y <= top + 200. && row.y + row.height() >= top)
        .flat_map(|row| &row.clusters)
        .filter(|cluster| cluster.text_range.start > 1000)
        .count();
    assert!(
        visible > 0,
        "vertical scroll omitted the materialized cell band; top {top}, actual {}, rows {:?}",
        core.layout(view).unwrap().viewport_top(),
        snapshot
            .rows
            .iter()
            .step_by(300)
            .map(|row| (row.y, row.text_range.clone()))
            .collect::<Vec<_>>()
    );
    let end = core.document().projection().tables()[0].rows[1].cells[0]
        .range
        .end;
    core.handle(
        view,
        viem_core::CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: end,
            affinity: viem_core::document::BoundaryAffinity::Upstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let geometry = snapshot
        .logical_endpoint_geometry(end, viem_core::document::BoundaryAffinity::Upstream)
        .unwrap();
    let cell = snapshot
        .table_cells()
        .iter()
        .find(|cell| cell.row == 1 && cell.column == 0)
        .unwrap();
    assert!(
        geometry.rect.y > cell.rect.y + cell.rect.height - 40.,
        "last explicit line was misplaced: {geometry:?}, {cell:?}"
    );
    let lines = snapshot
        .rows
        .iter()
        .filter(|row| {
            row.table_cell
                .as_ref()
                .is_some_and(|candidate| candidate.cell_id == cell.cell_id)
        })
        .collect::<Vec<_>>();
    assert!(
        lines.last().unwrap().fragment_index > 1,
        "internal table lines need distinct status ordinals"
    );
    let first_x = lines.first().unwrap().clusters.first().unwrap().x;
    let last_x = lines.last().unwrap().clusters.first().unwrap().x;
    assert!(
        first_x > last_x + 10.,
        "short explicit lines lost right alignment: {first_x} vs {last_x}"
    );
    core.handle(
        view,
        viem_core::CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(
        snapshot
            .rows
            .iter()
            .any(|row| row.y < top + 200. && row.y + row.height() > top),
        "focused cell ignored manual vertical scrolling"
    );
}

#[test]
fn very_wide_tables_only_shape_visible_columns_plus_bounded_discovery() {
    for format in [Format::Markdown] {
        let source = format!(
            "|{}\n|{}\n|{}",
            " header |".repeat(1000),
            " --- |".repeat(1000),
            " value |".repeat(1000)
        );
        let document = document(&source, format);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(400., 100.);
        let candidate = region(&document, &mut engine, &mut view, 0..2, 1);
        assert!(
            candidate.work_statistics().table_measured_cells() < 250,
            "{format:?}: {}",
            candidate.work_statistics().table_measured_cells()
        );
        assert!(
            candidate
                .lines()
                .iter()
                .flat_map(|line| line.rows())
                .flat_map(|row| &row.clusters)
                .count()
                < 1000
        );
    }
}

#[test]
fn horizontal_scroll_keeps_offscreen_multiline_cell_row_height() {
    let source = format!(
        "|{}\n|{}\n|{}\n",
        " H |".repeat(20),
        " - |".repeat(20),
        (0..20)
            .map(|column| if column == 19 {
                " first<br>second<br>third |"
            } else {
                " x |"
            })
            .collect::<String>()
    );
    let mut document = document(&source, Format::Markdown);
    let mut reference_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut reference_view = ViewLayout::new(180., 500.);
    reference_engine
        .relayout(&document, &mut reference_view)
        .unwrap();
    let expected = reference_view
        .snapshot()
        .unwrap()
        .table_cells()
        .iter()
        .find(|cell| cell.row == 1)
        .unwrap()
        .rect
        .height;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(180., 500.);
    let cold = region(&document, &mut engine, &mut view, 0..2, 401);
    let cold_height = cold.lines()[1]
        .rows()
        .first()
        .unwrap()
        .table_cell
        .as_ref()
        .unwrap()
        .rect
        .height;
    assert_eq!(
        cold_height, expected,
        "cold discovery must retain the offscreen multiline height"
    );
    view.set_viewport_left(1400.).unwrap();
    let right = region(&document, &mut engine, &mut view, 0..2, 402);
    assert_eq!(
        right.lines()[1]
            .rows()
            .first()
            .unwrap()
            .table_cell
            .as_ref()
            .unwrap()
            .rect
            .height,
        expected
    );
    view.set_viewport_left(0.).unwrap();
    let left = region(&document, &mut engine, &mut view, 0..2, 403);
    assert_eq!(
        left.lines()[1]
            .rows()
            .first()
            .unwrap()
            .table_cell
            .as_ref()
            .unwrap()
            .rect
            .height,
        expected
    );
    let target = document.projection().tables()[0].rows[1].cells[19]
        .range
        .clone();
    document.replace(target, "short").unwrap();
    let changed = region(&document, &mut engine, &mut view, 0..2, 404);
    let mut fresh_view = ViewLayout::new(180., 500.);
    LayoutEngine::new(MockTextMeasurementProvider::new())
        .relayout(&document, &mut fresh_view)
        .unwrap();
    let fresh = fresh_view
        .snapshot()
        .unwrap()
        .table_cells()
        .iter()
        .find(|cell| cell.row == 1)
        .unwrap()
        .rect
        .height;
    assert!(fresh < expected);
    assert_eq!(
        changed.lines()[1].rows()[0]
            .table_cell
            .as_ref()
            .unwrap()
            .rect
            .height,
        fresh
    );
}

#[test]
fn evicted_row_heights_are_rediscovered_without_revisiting_other_table_rows() {
    let body = (0..20)
        .map(|column| {
            if column == 19 {
                " first<br>second<br>third |"
            } else {
                " x |"
            }
        })
        .collect::<String>();
    let source = format!(
        "|{}\n|{}\n{}",
        " H |".repeat(20),
        " - |".repeat(20),
        format!("|{body}\n").repeat(300)
    );
    let document = document(&source, Format::Markdown);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(180., 500.);
    let first = region(&document, &mut engine, &mut view, 1..2, 500);
    let expected = first.lines()[0].rows()[0]
        .table_cell
        .as_ref()
        .unwrap()
        .rect
        .height;
    for row in 2..301 {
        region(
            &document,
            &mut engine,
            &mut view,
            row..row + 1,
            500 + row as u64,
        );
    }
    let restored = region(&document, &mut engine, &mut view, 1..2, 900);
    assert_eq!(
        restored.lines()[0].rows()[0]
            .table_cell
            .as_ref()
            .unwrap()
            .rect
            .height,
        expected
    );
    assert!(restored.work_statistics().table_measured_cells() <= DISCOVERY_TEST_LIMIT);
}
const DISCOVERY_TEST_LIMIT: usize = 160;
