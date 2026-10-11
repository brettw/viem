use viem_core::command::{
    ex_execute::{ExFrontendRequest, ExInfoRequest, ExOptionName, ExOptionValue},
    CommandStatus, InputEvent, Key,
};
use viem_core::document::{Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn opened(
    source: &str,
    format: Format,
    width: f32,
    height: f32,
) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), width, height);
    (core, view)
}
fn ex(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    command: &str,
) -> viem_core::command::CommandOutput {
    for key in command.chars() {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
    }
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap()
        .command
        .unwrap()
}

#[test]
fn number_boolean_forms_are_view_local_atomic_and_source_preserving() {
    let (mut core, view) = opened("first\nsecond", Format::PlainText, 250., 180.);
    let other = core.add_view(MockTextMeasurementProvider::new(), 250., 180.);
    let revision = core.document().revision();
    let layout_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
    assert!(!core.number(view).unwrap());
    for (command, expected) in [
        (":set nu", true),
        (":setlocal nonu", false),
        (":set invnumber", true),
        (":set number!", false),
        (":setlocal number", true),
        (":set number&", false),
    ] {
        assert_eq!(
            ex(&mut core, view, command).status,
            CommandStatus::Complete,
            "{command}"
        );
        assert_eq!(core.number(view).unwrap(), expected);
        assert!(!core.number(other).unwrap());
    }
    ex(&mut core, view, ":set nu");
    let query = ex(&mut core, view, ":set nu?").ex_outcome.unwrap();
    assert!(query
        .frontend_requests
        .iter()
        .any(|request| matches!(request,
        ExFrontendRequest::Info(ExInfoRequest::Options(options)) if options.iter().any(|option|
            option.name == ExOptionName::Number && option.value == ExOptionValue::Boolean(true)))));
    assert_ne!(
        ex(&mut core, view, ":set nonumber unknownoption").status,
        CommandStatus::Complete
    );
    assert!(
        core.number(view).unwrap(),
        "failed compound set must not partially hide the gutter"
    );
    assert_eq!(core.document().source_bytes(), b"first\nsecond");
    assert_eq!(core.document().revision(), revision);
    assert_eq!(
        core.layout(view).unwrap().snapshot().unwrap().revision,
        layout_revision
    );
}

#[test]
fn folded_source_lines_sharing_one_row_show_only_the_first_ordinal() {
    let source = "first\nsecond\nthird\n\nlast";
    let (mut core, view) = opened(source, Format::Markdown, 900., 500.);
    core.set_number(view, true).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.rows.len(), 2);
    let row = &snapshot.rows[0];
    let bands = core.line_number_regions(view).unwrap();
    assert_eq!(
        bands.iter().map(|band| band.source_line).collect::<Vec<_>>(),
        vec![1, 5],
        "a shared prose row shows its earliest physical source ordinal"
    );
    assert_eq!(bands[0].top, row.y);
    assert_eq!(bands[0].bottom, row.y + row.line_advance);
    assert!(bands[0].current);
    assert!(!bands[1].current);
    let cursor = core
        .document()
        .visible_point_for_source(source.find("third").unwrap(), true)
        .unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: cursor,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert_eq!(core.current_physical_source_line(view).unwrap(), 3);
    assert_eq!(core.line_number_regions(view).unwrap(), bands);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn wrapped_and_empty_physical_lines_have_one_full_allocated_band() {
    let (mut core, view) = opened(
        "a line with enough words to wrap over several rows\n\nlast\n",
        Format::PlainText,
        160.,
        500.,
    );
    core.set_number(view, true).unwrap();
    let bands = core.line_number_regions(view).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(
        snapshot
            .rows
            .iter()
            .filter(|row| row.hard_line_index == 0)
            .count()
            > 1
    );
    assert_eq!(
        bands.len(),
        core.document().physical_line_count().unwrap()
    );
    for band in bands {
        let rows: Vec<_> = snapshot
            .rows
            .iter()
            .filter(|row| row.hard_line_index + 1 == band.source_line)
            .collect();
        assert_eq!(band.top, rows.first().unwrap().y);
        assert_eq!(
            band.bottom,
            rows.last().unwrap().y + rows.last().unwrap().line_advance
        );
        assert!(band.bottom > band.top);
        assert!(!band.estimated);
    }
    let before = core.line_number_regions(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(before[0].bottom / 2.),
        },
    )
    .unwrap();
    let after = core.line_number_regions(view).unwrap();
    assert_eq!(
        after[0], before[0],
        "a partially visible wrapped line retains its whole available region"
    );
}

#[test]
fn scrolling_folded_long_source_lines_preserves_regions_and_materializes_both_directions() {
    let source = "word ".repeat(30_000) + "\n" + &"word ".repeat(30_000);
    let (mut core, view) = opened(&source, Format::Markdown, 400., 100.);
    core.set_number(view, true).unwrap();
    let initial = core.line_number_regions(view).unwrap()[0];
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let end = snapshot.rows.last().unwrap();
    let boundary = end.y + end.height();
    assert!(initial.estimated && initial.bottom > (end.y + end.height()) * 2.);
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(f32::MAX),
        },
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(0.),
        },
    )
    .unwrap();
    let origin = core.line_number_regions(view).unwrap()[0];
    let first_end = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .last()
        .unwrap()
        .text_range
        .end;
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(origin.bottom * 0.8),
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(
        snapshot.rows.first().unwrap().text_range.start >= first_end,
        "the scroll must install a disjoint partial chunk: first_end={first_end}, new_first={}, new_last={}, origin={origin:?}, top={}",
        snapshot.rows.first().unwrap().text_range.start,
        snapshot.rows.last().unwrap().text_range.end,
        core.layout(view).unwrap().viewport_top(),
    );
    let middle = core.line_number_regions(view).unwrap()[0];
    assert_eq!(middle.source_line, 1);
    assert_eq!(middle.top, origin.top);
    assert!(
        (middle.bottom - origin.bottom).abs() <= snapshot.rows[0].height() * 2.,
        "the full line region follows indexed source boundaries: {origin:?} -> {middle:?}"
    );
    assert!(middle.estimated);
    for top in [
        boundary - 50.,
        boundary + 25.,
        origin.bottom * 0.8,
        boundary + 25.,
        boundary - 50.,
        0.,
    ] {
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(top),
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        let coverage = snapshot.coverage.vertical_range().unwrap();
        assert!((layout.viewport_top() - top).abs() < 0.05);
        assert!(
            (top == 0. || coverage.start <= top) && top + layout.height() <= coverage.end,
            "finite scroll {top} must have exact rows across both viewport edges: {coverage:?}"
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(0.),
        },
    )
    .unwrap();
    assert_eq!(core.line_number_regions(view).unwrap()[0], origin);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn finite_scroll_across_a_giant_line_end_includes_following_paragraphs() {
    let source = "word ".repeat(30_000) + "\n" + &"short next\n".repeat(20) + "last";
    let (mut core, view) = opened(&source, Format::PlainText, 400., 100.);
    core.set_number(view, true).unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 150_000 - 1,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let edge = core
        .layout(view)
        .unwrap()
        .hard_line_range_height(0..1)
        .unwrap()
        .height() as f32;
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 0,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let top = edge - 50.;
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        },
    )
    .unwrap();
    let layout = core.layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    let coverage = snapshot.coverage.vertical_range().unwrap();
    assert!(
        (layout.viewport_top() - top).abs() < 0.05,
        "requested {top}, actual {}, first line height {edge}, coverage {coverage:?}",
        layout.viewport_top()
    );
    assert!(coverage.start <= top && top + layout.height() <= coverage.end);
    assert!(snapshot.rows.iter().any(|row| row.hard_line_index == 0));
    assert!(snapshot.rows.iter().any(|row| row.hard_line_index == 1));
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn exact_spacing_allocates_disjoint_number_bands_even_when_text_ink_overlaps() {
    let mut document = Document::new("one\ntwo\nthree");
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["block"]["line_spacing"] = serde_json::json!({ "Exact": 3. });
            style["block"]["margin_top"] = 0.into();
            style["block"]["margin_bottom"] = 0.into();
            style["character"]["size"] = 20.into();
        }
    }
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 250., 180.);
    core.set_number(view, true).unwrap();
    let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
    assert!(rows
        .iter()
        .all(|row| row.natural_height() > row.line_advance));
    let bands = core.line_number_regions(view).unwrap();
    assert_eq!(bands.len(), 3);
    for band in &bands {
        assert!((band.bottom - band.top - 3.).abs() < 0.001);
    }
    assert!(bands.windows(2).all(|pair| pair[0].bottom <= pair[1].top));
}

#[test]
fn composition_wraps_use_presentation_bands_without_inventing_source_lines() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    use viem_core::document::Association;

    let source = "one\nnext\nlast";
    let (mut core, view) = opened(source, Format::PlainText, 160., 700.);
    core.set_number(view, true).unwrap();
    let original = core.line_number_regions(view).unwrap();
    let revision = core.document().revision();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    let marked = "long marked text that wraps across multiple rows\nnew preview line ";
    core.handle_with_layout(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
            marked,
            marked.len()..marked.len(),
        ))),
    )
    .unwrap();

    let overlay = core.composition_overlay(view).unwrap().unwrap();
    let second_start = overlay
        .overlay_offset_for_base_boundary(4, Association::AfterInsertion)
        .unwrap();
    let snapshot = core.presentation_layout(view).unwrap().snapshot().unwrap();
    let first_rows: Vec<_> = snapshot
        .rows
        .iter()
        .filter(|row| row.text_range.start < second_start)
        .collect();
    assert!(
        first_rows.len() > 2,
        "marked text must wrap and add a preview hard break"
    );
    let second_row = snapshot
        .rows
        .iter()
        .find(|row| row.text_range.start == second_start)
        .unwrap();
    let bands = core.line_number_regions(view).unwrap();
    assert_eq!(
        bands
            .iter()
            .map(|band| band.source_line)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(bands[0].top, first_rows.first().unwrap().y);
    assert_eq!(
        bands[0].bottom,
        first_rows.last().unwrap().y + first_rows.last().unwrap().line_advance
    );
    assert_eq!(bands[1].top, second_row.y);
    assert_eq!(
        bands.iter().filter(|band| band.current).map(|band| band.source_line).collect::<Vec<_>>(),
        vec![1],
        "preview caret ownership follows the composed row without adding physical ordinals"
    );
    assert!(bands[1].top > original[1].top);
    assert_eq!(core.current_physical_source_line(view).unwrap(), 1);
    assert_eq!(core.document().physical_source_line_number_count(), 3);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision(), revision);

    core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Cancel))
        .unwrap();
    assert_eq!(core.line_number_regions(view).unwrap(), original);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision(), revision);
}

#[test]
fn wysiwyg_numbers_use_physical_provenance_and_union_table_cells() {
    let source = "first source line\nsecond source line\n\n| A | B |\n| --- | --- |\n| one | a long cell that wraps over several rows |\n\nlast";
    let (mut core, view) = opened(source, Format::Markdown, 250., 500.);
    core.set_number(view, true).unwrap();
    let bands = core.line_number_regions(view).unwrap();
    let numbers: Vec<_> = bands.iter().map(|band| band.source_line).collect();
    assert!(
        numbers.contains(&1) && numbers.contains(&2),
        "each physical line owning wrapped rows retains its ordinal: {numbers:?}"
    );
    assert!(
        !numbers.contains(&5),
        "the hidden table delimiter has no rendered row"
    );
    let table = bands.iter().find(|band| band.source_line == 6).unwrap();
    let cells: Vec<_> = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .table_cells()
        .iter()
        .filter(|cell| cell.row == 1)
        .collect();
    assert_eq!(cells.len(), 2);
    assert_eq!(
        table.top,
        cells
            .iter()
            .map(|cell| cell.rect.y)
            .min_by(f32::total_cmp)
            .unwrap()
    );
    assert_eq!(
        table.bottom,
        cells
            .iter()
            .map(|cell| cell.rect.y + cell.rect.height)
            .max_by(f32::total_cmp)
            .unwrap()
    );
    assert!(table.bottom > table.top);
    assert_eq!(bands.iter().filter(|band| band.source_line == 6).count(), 1);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    let source_cursor = source.find("second").unwrap();
    let cursor = core
        .document()
        .visible_point_for_source(source_cursor, true)
        .unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: cursor,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert_eq!(
        core.current_physical_source_line(view).unwrap(),
        2,
        "gutter current-line identity stays physical when both source lines share a visual row"
    );
}

#[test]
fn source_edits_and_width_changes_refresh_only_the_local_number_geometry() {
    let (mut core, view) = opened(
        &"short physical line\n".repeat(100_000),
        Format::PlainText,
        220.,
        140.,
    );
    core.set_number(view, true).unwrap();
    let revision = core.document().revision();
    let count = core.document().physical_line_count().unwrap();
    let before = core.line_number_regions(view).unwrap();
    assert!(before.len() < 100);
    assert!(!core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .coverage
        .is_full_document());
    ex(&mut core, view, ":set number?");
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('o'))))
        .unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert!(core.document().revision() > revision);
    assert_eq!(core.document().physical_line_count().unwrap(), count + 1);
    assert_eq!(core.current_physical_source_line(view).unwrap(), 2);
    let edited = core.line_number_regions(view).unwrap();
    assert!(edited.iter().any(|band| band.source_line == 2));
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 100.,
            height: 140.,
        },
    )
    .unwrap();
    let resized = core.line_number_regions(view).unwrap();
    assert!(resized.len() < 100);
    assert!(resized[0].bottom > edited[0].bottom);
}

#[test]
fn startup_number_is_inherited_by_new_views() {
    let mut core = Core::new(Document::new("one\ntwo"));
    assert!(core.initialize_startup("set number\n").is_empty());
    let first = core.add_view(MockTextMeasurementProvider::new(), 250., 180.);
    let second = core.add_view(MockTextMeasurementProvider::new(), 250., 180.);
    assert!(core.number(first).unwrap() && core.number(second).unwrap());
    core.set_number(first, false).unwrap();
    assert!(core.number(second).unwrap());
}

#[test]
fn a_fitting_document_keeps_its_top_when_resize_or_gutter_causes_overflow() {
    let source = ("a paragraph with words that will wrap more at a narrow width ".repeat(4)
        + "\n\n")
        .repeat(3);
    let (mut core, view) = opened(&source, Format::Markdown, 920., 655.);
    assert_eq!(core.layout(view).unwrap().maximum_viewport_top(), Some(0.));
    core.set_number(view, true).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 330.,
            height: 220.,
        },
    )
    .unwrap();
    assert!(core.layout(view).unwrap().maximum_viewport_top().unwrap() > 0.);
    assert_eq!(core.layout(view).unwrap().viewport_top(), 0.);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.line_number_regions(view).unwrap()[0].source_line, 1);

    let mut document = Document::from_bytes(
        "word ".repeat(40).as_bytes().to_vec(),
        Encoding::Utf8,
        Format::PlainText,
    )
    .unwrap();
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let paragraph = defaults["block_styles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|style| style["id"] == "Paragraph")
        .unwrap();
    paragraph["character"]["size"] = 12.into();
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 225., 800.);
    let fitting_height = core.layout(view).unwrap().snapshot().unwrap().total_height + 0.5;
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 225.,
            height: fitting_height,
        },
    )
    .unwrap();
    assert_eq!(core.layout(view).unwrap().maximum_viewport_top(), Some(0.));
    core.set_number(view, true).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 200.,
            height: fitting_height,
        },
    )
    .unwrap();
    assert!(core.layout(view).unwrap().maximum_viewport_top().unwrap() > 0.);
    assert_eq!(core.layout(view).unwrap().viewport_top(), 0.);
    assert_eq!(core.line_number_regions(view).unwrap()[0].source_line, 1);

    // A real scroll to the end retains the existing bottom-pinning behavior.
    core.handle_with_layout(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(f32::MAX),
        },
    )
    .unwrap();
    assert!(core.layout(view).unwrap().viewport_top() > 0.);
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 150.,
            height: fitting_height,
        },
    )
    .unwrap();
    let layout = core.layout(view).unwrap();
    assert!((layout.viewport_top() - layout.maximum_viewport_top().unwrap()).abs() < 0.001);
}

#[test]
fn terminal_caret_rows_have_numbers_without_becoming_ex_addresses() {
    use viem_core::document::FileFormat;
    for (ending, format) in [
        ("\n", FileFormat::Unix),
        ("\r\n", FileFormat::Dos),
        ("\r", FileFormat::Mac),
    ] {
        for terminate in [false, true] {
            let source = (0..99)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join(ending)
                + if terminate { ending } else { "" };
            let document = Document::from_bytes_with_file_format(
                source.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                format,
            )
            .unwrap();
            assert_eq!(
                document.physical_source_line_number_count(),
                99,
                "{format:?} final terminator {terminate}"
            );
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 250., 180.);
            core.set_number(view, true).unwrap();
            ex(&mut core, view, ":99");
            assert_eq!(core.current_physical_source_line(view).unwrap(), 99);
            assert!(core
                .line_number_regions(view)
                .unwrap()
                .iter()
                .all(|band| band.source_line <= if terminate { 100 } else { 99 }));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
    for (source, count) in [("", 1), ("\n", 1), ("\n\n", 2), ("one\n\n", 2)] {
        let (mut core, view) = opened(source, Format::PlainText, 250., 180.);
        core.set_number(view, true).unwrap();
        assert_eq!(core.document().physical_source_line_number_count(), count);
        assert_eq!(core.line_number_regions(view).unwrap().len(), count + usize::from(source.ends_with("\n")));
    }
    let (mut core, view) = opened("first\nlast\n", Format::PlainText, 250., 180.);
    core.set_number(view, true).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let final_boundary = snapshot.rows.last().unwrap();
    assert!(final_boundary.text_range.is_empty());
    let end = final_boundary.text_range.start;
    let before = core.line_number_regions(view).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: end,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let after = core.line_number_regions(view).unwrap();
    assert_eq!(before.len(), 3);
    assert_eq!(after.len(), 3);
    assert_eq!(before[1].top, after[1].top);
    assert_eq!(before[1].bottom, after[1].bottom);
    assert_eq!(core.current_physical_source_line(view).unwrap(), 3);
    assert!(!after[0].current);
    assert!(!after[1].current);
    assert!(after[2].current);
}

#[test]
fn gutter_abi_validates_revision_viewport_capacity_and_pointer_aliasing() {
    use viem_core::ffi::*;
    let fixture =
        super::ffi_core_surface::table_ffi_fixture(b"one\ntwo\nthree", VIEM_FORMAT_PLAIN_TEXT);
    assert_eq!(
        viem_core_view_set_number(fixture.handle, fixture.view, 2),
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        viem_core_view_set_number(fixture.handle, fixture.view, 1),
        ViemStatus::Ok
    );
    let mut info = ViemLineNumberSnapshotInfoV1::default();
    assert_eq!(
        unsafe {
            viem_core_view_line_number_snapshot_info(fixture.handle, fixture.view, &mut info)
        },
        ViemStatus::Ok
    );
    assert_eq!(info.source_line_count, 3);
    assert_eq!(info.current_source_line, 1);
    assert_eq!(info.flags, VIEM_LINE_NUMBER_SNAPSHOT_ENABLED);
    let mut count = 0;
    assert_eq!(
        unsafe {
            viem_core_view_copy_line_number_regions(
                fixture.handle,
                fixture.view,
                &info.identity,
                &info.viewport,
                std::ptr::null_mut(),
                0,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(count, info.region_count);
    let sentinel = ViemLineNumberRegionV1 {
        source_line: 99,
        ..Default::default()
    };
    let mut regions = vec![sentinel; count as usize];
    let mut stale = info.identity;
    stale.document_revision += 1;
    assert_eq!(
        unsafe {
            viem_core_view_copy_line_number_regions(
                fixture.handle,
                fixture.view,
                &stale,
                &info.viewport,
                regions.as_mut_ptr(),
                regions.len() as u64,
                &mut count,
            )
        },
        ViemStatus::StaleRevision
    );
    assert!(regions.iter().all(|region| *region == sentinel));
    let mut stale_viewport = info.viewport;
    stale_viewport.y += 1.;
    assert_eq!(
        unsafe {
            viem_core_view_copy_line_number_regions(
                fixture.handle,
                fixture.view,
                &info.identity,
                &stale_viewport,
                regions.as_mut_ptr(),
                regions.len() as u64,
                &mut count,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(
        unsafe {
            viem_core_view_copy_line_number_regions(
                fixture.handle,
                fixture.view,
                &info.identity,
                &info.viewport,
                regions.as_mut_ptr(),
                regions.len() as u64,
                &mut count,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(
        regions
            .iter()
            .map(|region| region.source_line)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(regions[0].flags, VIEM_LINE_NUMBER_REGION_CURRENT);
    assert_eq!(regions[1].flags, 0);
    assert_eq!(regions[2].flags, 0);
    assert_eq!(
        unsafe {
            viem_core_view_copy_line_number_regions(
                fixture.handle,
                fixture.view,
                &info.identity,
                &info.viewport,
                regions.as_mut_ptr(),
                regions.len() as u64,
                regions.as_mut_ptr().cast::<u64>(),
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        viem_core_view_set_number(fixture.handle, fixture.view, 0),
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_view_line_number_snapshot_info(fixture.handle, fixture.view, &mut info)
        },
        ViemStatus::Ok
    );
    assert_eq!(info.region_count, 0);
    assert_eq!(info.source_line_count, 3);
}

#[test]
fn gutter_pointer_abi_rejects_stale_or_invalid_geometry_without_changing_selection() {
    use viem_core::ffi::*;
    let fixture = super::ffi_core_surface::table_ffi_fixture(b"one\ntwo\nthree", VIEM_FORMAT_PLAIN_TEXT);
    viem_core_view_set_number(fixture.handle, fixture.view, 1);
    let mut info = ViemLineNumberSnapshotInfoV1::default();
    assert_eq!(unsafe { viem_core_view_line_number_snapshot_info(fixture.handle, fixture.view, &mut info) }, ViemStatus::Ok);
    let mut regions = vec![ViemLineNumberRegionV1::default(); info.region_count as usize];
    let mut copied = 0;
    assert_eq!(unsafe { viem_core_view_copy_line_number_regions(fixture.handle, fixture.view, &info.identity, &info.viewport,
        regions.as_mut_ptr(), regions.len() as u64, &mut copied) }, ViemStatus::Ok);
    let mut outcome = ViemCoreOutcomeV1::default();
    let y = (regions[1].top + regions[1].bottom) / 2.;
    let mut hit = 0;
    assert_eq!(unsafe { viem_core_view_row_selection_hit_test(fixture.handle, fixture.view, &info.identity, -100., y, &mut hit) }, ViemStatus::Ok);
    assert_eq!(hit, 1);
    assert_eq!(unsafe { viem_core_view_row_selection_hit_test(fixture.handle, fixture.view, &info.identity, 100_000., y, &mut hit) }, ViemStatus::Ok);
    assert_eq!(hit, 0);
    let mut stale_hit_identity = info.identity; stale_hit_identity.layout_revision += 1;
    for (identity, x, y, status) in [(stale_hit_identity, -100., y, ViemStatus::StaleRevision),
        (info.identity, f32::NAN, y, ViemStatus::InvalidArgument),
        (info.identity, -100., f32::INFINITY, ViemStatus::InvalidArgument)] {
        hit = 99;
        assert_eq!(unsafe { viem_core_view_row_selection_hit_test(fixture.handle, fixture.view, &identity, x, y, &mut hit) }, status);
        assert_eq!(hit, 0);
    }
    let mut aliased = info.identity;
    let out_alias = std::ptr::addr_of_mut!(aliased).cast::<u32>();
    assert_eq!(unsafe { viem_core_view_row_selection_hit_test(fixture.handle, fixture.view, &aliased, -100., y, out_alias) }, ViemStatus::InvalidArgument);
    assert_eq!(aliased, info.identity);
    assert_eq!(unsafe { viem_core_view_select_pointer_row(fixture.handle, fixture.view, &info.identity, y, 0, &mut outcome) }, ViemStatus::Ok);
    let mut selected = ViemLogicalSelectionIdentityV1::default();
    assert_eq!(unsafe { viem_core_view_list_selection(fixture.handle, fixture.view, &mut selected) }, ViemStatus::Ok);
    assert_eq!((selected.text_start, selected.text_end), (4, 8));
    let mut stale = info.identity; stale.layout_revision += 1;
    for (expected, y, extend, status) in [
        (stale, y, 1, ViemStatus::StaleRevision),
        (info.identity, f32::NAN, 1, ViemStatus::InvalidArgument),
        (info.identity, y, 2, ViemStatus::InvalidArgument),
    ] {
        assert_eq!(unsafe { viem_core_view_select_pointer_row(fixture.handle, fixture.view, &expected, y, extend, &mut outcome) }, status);
        let mut after = ViemLogicalSelectionIdentityV1::default();
        assert_eq!(unsafe { viem_core_view_list_selection(fixture.handle, fixture.view, &mut after) }, ViemStatus::Ok);
        assert_eq!(after, selected);
    }
}
