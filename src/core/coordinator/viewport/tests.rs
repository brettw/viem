use super::*;
use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult};
use crate::document::{
    code_style,
    syntax::{SyntaxRun, SyntaxStyleName},
    CharacterProperties, CharacterStyle, StyleDefinitionMetadata,
};
use crate::layout::MockTextMeasurementProvider;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod scroll_endpoints;

const LINE: &str = "let value = 42; // comment\n";

#[test]
fn percentage_sizes_refresh_visible_and_distant_text_without_shaping_a_large_document() {
    use crate::document::{StyleDefinitionFieldEdit, StyleNamespace, StyleProperty, StylePropertyValue};
    let source = "# `heading`\n\n`body`\n\n".repeat(10_000);
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 240.0);
    let edit = |core: &mut Core<MockTextMeasurementProvider>, namespace, id: &str, value| {
        core.handle(view, CoreEvent::EditGeneratedStyle {
            document: core.document.id(),
            revision: core.document.revision(),
            style_sheet_revision: core.document.projection().style_sheet().revision,
            namespace,
            style: id.into(),
            edit: StyleDefinitionFieldEdit::SetDeclaration { property: StyleProperty::CharacterSize, value },
        }).unwrap();
    };
    edit(&mut core, StyleNamespace::Block, "Paragraph", StylePropertyValue::Float(12.0));
    edit(&mut core, StyleNamespace::Block, "Heading1", StylePropertyValue::Percentage(200));
    edit(&mut core, StyleNamespace::Character, "Code", StylePropertyValue::Percentage(90));
    assert!((core.selected_character_style(view).unwrap().size - 21.6).abs() < 0.0001);
    let calls = core.views[&view].engine.provider().request_calls();
    let revision = core.layout(view).unwrap().snapshot().unwrap().revision;
    edit(&mut core, StyleNamespace::Block, "Paragraph", StylePropertyValue::Float(20.0));
    let reshaped = core.views[&view].engine.provider().request_calls() - calls;
    assert!(reshaped > 0 && reshaped < 1000, "Only visible/buffered rows need shaping: {reshaped}");
    assert_ne!(core.layout(view).unwrap().snapshot().unwrap().revision, revision);
    assert!((core.selected_character_style(view).unwrap().size - 36.0).abs() < 0.0001);
    let at = core.document.text().match_indices("heading").nth(5_000).unwrap().0;
    let calls = core.views[&view].engine.provider().request_calls();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    assert!((core.selected_character_style(view).unwrap().size - 36.0).abs() < 0.0001);
    assert!(core.views[&view].engine.provider().request_calls() - calls < 1000);
    assert_eq!(core.document.source_bytes(), source.as_bytes());
}

struct DeferredProvider;
impl SyntaxProvider for DeferredProvider {
    fn analyze(&mut self, request: &SyntaxRequest, cancellation: &AtomicBool) -> SyntaxResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancellation.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        SyntaxResult::missing(request, "controlled deferred syntax")
    }
}

fn fixture(screen_baseline: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    fixture_for_format(screen_baseline, Format::Code)
}

fn fixture_for_format(screen_baseline: f32, format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let paragraph = if format == Format::Markdown {
        "An ordinary paragraph with several words that wrap across the narrow text view.\n\n"
    } else {
        LINE
    };
    let document = Document::from_bytes(
        paragraph.repeat(20_000).into_bytes(),
        Encoding::Utf8,
        format,
    )
    .unwrap();
    let mut core = Core::new(document);
    core.set_syntax_provider_factory(Arc::new(|| Box::new(DeferredProvider)));
    core.set_code_language(crate::document::syntax::detection::LanguageSelection::None);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 240.);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document.revision(),
            text_offset: if format == Format::Markdown {
                core.document.text().find("narrow").unwrap() + 500_000
            } else {
                LINE.len() * 10_000 + 6
            },
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let row = caret_row(&core, view);
    let top = row.baseline - screen_baseline;
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        },
    )
    .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    (core, view)
}

fn caret_row(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> crate::layout::VisualRow {
    let view = &core.views[&view];
    let snapshot = view.layout.snapshot().unwrap();
    let position = view.commands.visual_position().unwrap_or_else(|| {
        crate::command::layout_motion::VisualPosition {
            text_offset: view.commands.cursor(),
            affinity: view.commands.boundary_affinity(),
        }
    });
    let geometry = snapshot
        .logical_endpoint_geometry(position.text_offset, position.affinity)
        .or_else(|_| snapshot.logical_endpoint_geometry(position.text_offset, match position.affinity {
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
            BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
        }))
        .unwrap();
    snapshot.rows[geometry.row_index].clone()
}

fn baseline(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> f32 {
    caret_row(core, view).baseline - core.views[&view].layout.viewport_top()
}

fn publish_large_runs(core: &mut Core<MockTextMeasurementProvider>, size: f32) {
    let mut sheet = code_style::default_sheet();
    sheet
        .insert_character_style(
            CharacterStyle {
                id: StyleId("test:large".into()),
                based_on: None,
                properties: CharacterProperties {
                    size: Some((size).into()),
                    ..Default::default()
                },
            },
            StyleDefinitionMetadata::generated("Large"),
        )
        .unwrap();
    // Test publication uses the current authority generation without changing
    // shared global state or allowing a subsequent no-op poll to replace it.
    sheet.revision = code_style::snapshot().revision;
    let runs = [9_998, 10_000].map(|line| SyntaxRun {
        range: line * LINE.len()..(line + 1) * LINE.len() - 1,
        name: SyntaxStyleName("Large".into()),
        origin: "controlled provider".into(),
        priority: 100,
    });
    core.publish_code_presentation_runs(Arc::new(sheet), &runs);
}

#[test]
fn typing_keeps_an_off_center_caret_and_scroll_stable_in_a_large_code_document() {
    let (mut core, view) = fixture(64.);
    let original = baseline(&core, view);
    let calls = core.views[&view].engine.provider().request_calls();
    for character in "changed_value".chars() {
        core.handle(
            view,
            CoreEvent::Input(InputEvent::Text(character.to_string())),
        )
        .unwrap();
        assert!((baseline(&core, view) - original).abs() < 0.05);
        assert!(
            core.views[&view]
                .layout
                .snapshot()
                .unwrap()
                .coverage
                .hard_lines()
                .len()
                < 128
        );
    }
    assert!(core.views[&view].engine.provider().request_calls() - calls < 256);
}

#[test]
fn incomplete_regional_coverage_does_not_scroll_a_visible_editing_row_to_the_bottom() {
    let (mut core, view) = fixture(96.);
    let before = baseline(&core, view);
    let top = core.layout(view).unwrap().viewport_top();
    let request = core.prepare_view_layout_job(
        view, LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(9_990..10_001, top, 240.).unwrap()),
        LayoutCancellationToken::new(),
    ).unwrap();
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    core.install_view_layout_job(view, candidate).unwrap();
    assert!((baseline(&core, view) - before).abs() < 0.05,
        "a regional layout boundary is not the document end: expected {before}, got {}", baseline(&core, view));
    assert!(viewport_extension_needed(&core.views[&view]).1,
        "retain the missing coverage so the coordinator can request it");
    let calls = core.views[&view].engine.provider().request_calls();
    for character in "typing".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Text(character.to_string()))).unwrap();
        assert!((baseline(&core, view) - before).abs() < 0.05);
        let layout = core.layout(view).unwrap();
        assert!(layout.snapshot().unwrap().coverage.vertical_range().unwrap().end
            >= layout.viewport_top() + layout.height());
        assert!(layout.snapshot().unwrap().coverage.hard_lines().len() < 128);
    }
    assert!(core.views[&view].engine.provider().request_calls() - calls < 128);
}

#[test]
fn appending_at_a_visible_hard_line_end_preserves_its_screen_position() {
    for wrap in [true, false] {
        let (mut core, view) = fixture(96.);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('A')))).unwrap();
        let row = caret_row(&core, view);
        core.handle(view, CoreEvent::SetViewportOrigin {
            left: 0., top: Some(row.baseline - 96.),
        }).unwrap();
        let before = baseline(&core, view);
        for character in "tail".chars() {
            core.handle(view, CoreEvent::Input(InputEvent::Text(character.to_string()))).unwrap();
            assert!((baseline(&core, view) - before).abs() < 0.05,
                "wrap={wrap}: appending should not scroll a visible row from {before} to {}", baseline(&core, view));
        }
    }
}

#[test]
fn typing_near_a_wrapped_checkpoint_boundary_fills_the_viewport_without_scrolling() {
    for offset in [MAX_LONG_LINE_LAYOUT_SLICE_BYTES - 40, MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 40] {
        let document = Document::new("ordinary words ".repeat(14_000));
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 240.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
        let row = caret_row(&core, view);
        core.views.get_mut(&view).unwrap().layout.set_viewport_top(row.baseline - 96.).unwrap();
        let before = baseline(&core, view);
        let calls = core.views[&view].engine.provider().request_calls();
        for character in "edit".chars() {
            core.handle(view, CoreEvent::Input(InputEvent::Text(character.to_string()))).unwrap();
            assert!((baseline(&core, view) - before).abs() < 0.05);
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let coverage = snapshot.coverage.vertical_range().unwrap();
            assert!(coverage.start <= layout.viewport_top());
            assert!(coverage.end >= layout.viewport_top() + layout.height(),
                "checkpoint coverage ends at {}, before viewport bottom {}", coverage.end, layout.viewport_top() + layout.height());
            assert!(snapshot.rows.last().unwrap().text_range.end - snapshot.rows[0].text_range.start
                <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        }
        assert!(core.views[&view].engine.provider().request_calls() - calls < 1200);
    }
}

#[test]
fn syntax_metric_changes_preserve_the_caret_baseline_and_layout_only_its_neighborhood() {
    let (mut core, view) = fixture(96.);
    let original = baseline(&core, view);
    let calls = core.views[&view].engine.provider().request_calls();
    let history = core.document.history_status().node_count;
    let revision = core.document.revision();
    publish_large_runs(&mut core, 36.);
    core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport)
        .unwrap();
    assert!((baseline(&core, view) - original).abs() < 0.05);
    let row = caret_row(&core, view);
    assert!(
        row.ascent > 20.,
        "fixture must exercise the new font metrics"
    );
    let layout = &core.views[&view].layout;
    assert!(row.y >= layout.viewport_top());
    assert!(row.y + row.natural_height() <= layout.viewport_top() + layout.height());
    assert!(layout.snapshot().unwrap().coverage.hard_lines().len() < 128);
    assert!(core.views[&view].engine.provider().request_calls() - calls < 128);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status().node_count, history);
}

#[test]
fn externally_computed_syntax_layout_also_preserves_the_caret_baseline() {
    let (mut core, view) = fixture(96.);
    let original = baseline(&core, view);
    let top = core.views[&view].layout.viewport_top();
    publish_large_runs(&mut core, 36.);
    let request = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(9_980..10_030, top, 240.).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let candidate =
        compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    core.install_view_layout_job(view, candidate).unwrap();
    assert!(caret_row(&core, view).ascent > 20.);
    assert!((baseline(&core, view) - original).abs() < 0.05);
}

#[test]
fn taller_syntax_near_the_top_moves_only_enough_to_show_the_full_row() {
    let (mut core, view) = fixture(16.);
    let original = baseline(&core, view);
    publish_large_runs(&mut core, 48.);
    core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport)
        .unwrap();
    let row = caret_row(&core, view);
    assert!(
        row.ascent > 20.,
        "fixture must exercise the new font metrics"
    );
    let top = core.views[&view].layout.viewport_top();
    assert!(
        (row.y - top).abs() < 0.05,
        "tall row should just meet the top edge: row={}, viewport={top}, baseline={}",
        row.y,
        baseline(&core, view)
    );
    assert!((baseline(&core, view) - row.ascent).abs() < 0.05);
    assert!(baseline(&core, view) > original);
}

#[test]
fn newline_and_wrap_advance_the_caret_without_recentering_then_reveal_only_at_the_edge() {
    for at_edge in [false, true] {
        let (mut core, view) = fixture(if at_edge { 228. } else { 64. });
        let before = baseline(&core, view);
        let old_line = caret_row(&core, view).hard_line_index;
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        let row = caret_row(&core, view);
        let previous_row = core.views[&view]
            .layout
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.hard_line_index == old_line)
            .unwrap();
        let advance = row.baseline - previous_row.baseline;
        let expected = (before + advance)
            .min(core.views[&view].layout.height() - (row.natural_height() - row.ascent));
        assert!(core.views[&view].layout.last_error().is_none());
        assert!(
            (baseline(&core, view) - expected).abs() < 0.1,
            "newline: expected={expected}, actual={}",
            baseline(&core, view)
        );
    }

    let (mut core, view) = fixture(64.);
    let before = baseline(&core, view);
    let advance = caret_row(&core, view).line_advance;
    core.handle(view, CoreEvent::Input(InputEvent::Text("x".repeat(50))))
        .unwrap();
    let row = caret_row(&core, view);
    assert!(
        row.fragment_index > 0,
        "fixture must wrap the inserted text"
    );
    let expected = before + advance * row.fragment_index as f32;
    assert!(
        (baseline(&core, view) - expected).abs() < 0.1,
        "wrapping should advance normally without moving its original row"
    );
}

#[test]
fn committing_marked_text_keeps_the_displayed_composition_baseline() {
    use crate::command::composition::{CompositionTarget, CompositionUpdate};
    let (mut core, view) = fixture(64.);
    let offset = core.views[&view].commands.cursor();
    let target = CompositionTarget::at_offsets(&core.document, offset..offset).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    let marked = "日本語";
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
            marked_text: marked.into(),
            selected_range: marked.len()..marked.len(),
        })),
    )
    .unwrap();
    let layout = core.presentation_layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    let geometry = snapshot
        .logical_endpoint_geometry(offset + marked.len(), BoundaryAffinity::Downstream)
        .unwrap();
    let screen_baseline = snapshot.rows[geometry.row_index].baseline - layout.viewport_top();
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    assert!((baseline(&core, view) - screen_baseline).abs() < 0.1);
    assert!(core.views[&view].layout.last_error().is_none());
}

#[test]
fn typing_near_bottom_preserves_visible_rows_in_every_text_format() {
    for format in [Format::PlainText, Format::Markdown] {
        // Every format contains many independent paragraphs so edits must
        // invalidate only the active neighborhood of the large document.
        let (mut core, view) = fixture_for_format(220., format);
        let initial = baseline(&core, view);
        let old_top = core.views[&view].layout.viewport_top();
        assert!(old_top > 500., "fixture must be scrolled far from the top");
        assert!(initial > 180., "{format:?} fixture must edit near the viewport bottom, got {initial}");
        let calls = core.views[&view].engine.provider().request_calls();
        core.handle(view, CoreEvent::Input(InputEvent::Text("x".into()))).unwrap();
        assert!((baseline(&core, view) - initial).abs() < 0.1,
            "{format:?}: visible typing must retain baseline {initial}, got {}", baseline(&core, view));
        assert!(core.views[&view].layout.last_error().is_none());
        assert!(core.views[&view].layout.snapshot().unwrap().rows.len() < 150);
        assert!(core.views[&view].engine.provider().request_calls() - calls < 128);
    }
}

#[test]
fn newline_near_bottom_in_markdown_reveals_only_the_clipped_row() {
    for screen_baseline in [64., 220.] {
        let (mut core, view) = fixture_for_format(screen_baseline, Format::Markdown);
        let before = baseline(&core, view);
        let prior = caret_row(&core, view);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        let row = caret_row(&core, view);
        let layout = &core.views[&view].layout;
        let previous = layout.snapshot().unwrap().rows.iter()
            .find(|row| row.text_range.start == prior.text_range.start).unwrap();
        let expected = (before + row.baseline - previous.baseline)
            .min(layout.height() - (row.natural_height() - row.ascent));
        assert!((baseline(&core, view) - expected).abs() < 0.1,
            "expected minimal reveal to baseline {expected}, got {}", baseline(&core, view));
        assert!(layout.last_error().is_none());
    }
}

#[test]
fn splitting_a_giant_markdown_paragraph_keeps_anchor_and_new_caret_materialized() {
    let source = "ordinary words ".repeat(14_000);
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 240.);
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: 100_006,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
    let prior = caret_row(&core, view);
    core.views.get_mut(&view).unwrap().layout.set_viewport_top(prior.baseline - 220.).unwrap();
    let before = baseline(&core, view);
    let shape_calls = core.views[&view].engine.provider().request_calls();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    assert!(core.views[&view].engine.provider().request_calls() - shape_calls < 600,
        "splitting two giant paragraphs must keep shape work bounded");
    let row = caret_row(&core, view);
    let layout = &core.views[&view].layout;
    let snapshot = layout.snapshot().unwrap();
    let previous = snapshot.rows.iter().find(|row| row.text_range.start == prior.text_range.start).unwrap();
    let expected = (before + row.baseline - previous.baseline)
        .min(layout.height() - (row.natural_height() - row.ascent));
    assert!((baseline(&core, view) - expected).abs() < 0.1);
    assert!(layout.last_error().is_none());
    assert_eq!(snapshot.document_revision, core.document.revision());
    assert_eq!(snapshot.coverage.hard_lines(), 0..2);
    // The old tail and new prefix remain two bounded captures, not a full
    // relayout of either of the giant paragraphs after source invalidation.
    assert!(snapshot.rows.last().unwrap().text_range.end - snapshot.rows[0].text_range.start
        < 2 * MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 1024);
    let baseline_after_enter = baseline(&core, view);
    let revision_after_enter = core.document.revision();
    let shape_calls = core.views[&view].engine.provider().request_calls();
    core.handle(view, CoreEvent::Input(InputEvent::Text("x".into()))).unwrap();
    assert!(core.views[&view].engine.provider().request_calls() - shape_calls < 600,
        "later typing must reuse the preceding paragraph's cached prefix");
    assert!(core.views[&view].long_line_checkpoints.values().any(|checkpoint| {
        checkpoint.hard_line_index() == 0 && checkpoint.document_revision() == core.document.revision()
    }));
    assert!((baseline(&core, view) - baseline_after_enter).abs() < 0.1);
    let layout = &core.views[&view].layout;
    let snapshot = layout.snapshot().unwrap();
    assert_ne!(snapshot.document_revision, revision_after_enter);
    assert_eq!(snapshot.document_revision, core.document.revision());
    assert!(snapshot.rows[0].y <= layout.viewport_top(), "the preserved viewport must remain fully materialized");
}

fn set_bottom_margin(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, bottom: f32) {
    core.set_view_insets(view, crate::layout::EdgeInsets {
        bottom,
        ..Default::default()
    }).unwrap();
}

fn row_ink_bottom(row: &crate::layout::VisualRow) -> f32 {
    row.ink_bounds().map_or(row.y + row.natural_height(), |ink| {
        (row.y + row.natural_height()).max(ink.y + ink.height)
    })
}

fn assert_row_above_margin(core: &Core<MockTextMeasurementProvider>, view: ViewId, margin: f32) {
    let row = caret_row(core, view);
    let layout = &core.views[&view].layout;
    assert!(row.y >= layout.viewport_top() - 0.05);
    assert!(row_ink_bottom(&row) <= layout.viewport_top() + layout.height() - margin + 0.05,
        "row bottom {} must be above reserved bottom {}", row_ink_bottom(&row),
        layout.viewport_top() + layout.height() - margin);
}

#[test]
fn typing_in_bottom_margin_reveals_the_row_and_retains_full_viewport_coverage() {
    for format in [Format::Code, Format::PlainText] {
        for wrap in [true, false] {
            let (mut core, view) = fixture_for_format(220., format);
            core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
            set_bottom_margin(&mut core, view, 28.);
            let row = caret_row(&core, view);
            core.handle(view, CoreEvent::SetViewportOrigin {
                left: 0., top: Some(row.baseline - 220.),
            }).unwrap();
            let previous_top = core.layout(view).unwrap().viewport_top();
            let shape_calls = core.views[&view].engine.provider().request_calls();
            core.handle(view, CoreEvent::Input(InputEvent::Text("x".into()))).unwrap();
            assert_row_above_margin(&core, view, 28.);
            let layout = core.layout(view).unwrap();
            let row = caret_row(&core, view);
            assert!(layout.viewport_top() > previous_top);
            assert!((row_ink_bottom(&row) - layout.viewport_top() - 212.).abs() < 0.05,
                "reveal should move only enough to clear the margin");
            assert_eq!(layout.height(), 240.);
            let snapshot = layout.snapshot().unwrap();
            assert_eq!(snapshot.viewport_height, 240.);
            assert!(snapshot.coverage.vertical_range().unwrap().end >= layout.viewport_top() + 240.);
            assert!(snapshot.rows.iter().any(|row| {
                let y = row.y - layout.viewport_top();
                y >= 212. && y < 240.
            }), "the reserved margin remains materialized for painting");
            assert!(snapshot.coverage.hard_lines().len() < 128);
            let visible_baseline = row.baseline - layout.viewport_top();
            for character in "value".chars() {
                core.handle(view, CoreEvent::Input(InputEvent::Text(character.to_string()))).unwrap();
                assert!((baseline(&core, view) - visible_baseline).abs() < 0.05,
                    "{format:?}, wrap={wrap}: typing should preserve the visible baseline");
            }
            assert!(core.views[&view].engine.provider().request_calls() - shape_calls < 128);
        }
    }
}

#[test]
fn newline_respects_bottom_margin_without_moving_an_already_visible_row() {
    for screen_baseline in [64., 200.] {
        let (mut core, view) = fixture(screen_baseline);
        set_bottom_margin(&mut core, view, 28.);
        let prior = caret_row(&core, view);
        let before = baseline(&core, view);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        let row = caret_row(&core, view);
        let layout = core.layout(view).unwrap();
        let previous = layout.snapshot().unwrap().rows.iter()
            .find(|row| row.text_range.start == prior.text_range.start).unwrap();
        let expected = (before + row.baseline - previous.baseline)
            .min(layout.height() - 28. - (row_ink_bottom(&row) - row.baseline));
        assert!((baseline(&core, view) - expected).abs() < 0.05);
        assert_row_above_margin(&core, view, 28.);
    }
}

#[test]
fn typing_respects_the_top_margin_and_leaves_rows_inside_both_margins_in_place() {
    for screen_baseline in [20., 80., 180.] {
        let (mut core, view) = fixture(screen_baseline);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top: 28., bottom: 28., ..Default::default()
        }).unwrap();
        let row = caret_row(&core, view);
        core.handle(view, CoreEvent::SetViewportOrigin {
            left: 0., top: Some(row.baseline - screen_baseline),
        }).unwrap();
        let expected = screen_baseline.max(28. + row.baseline - row.reveal_bounds().start);
        for character in "edit".chars() {
            core.handle(view, CoreEvent::Input(InputEvent::Text(character.to_string()))).unwrap();
            assert!((baseline(&core, view) - expected).abs() < 0.05,
                "only a row outside the margins should move: expected {expected}, got {}", baseline(&core, view));
            let layout = core.layout(view).unwrap();
            let bounds = caret_row(&core, view).reveal_bounds();
            assert!(bounds.start >= layout.viewport_top() + 28. - 0.05);
            assert!(bounds.end <= layout.viewport_top() + layout.height() - 28. + 0.05);
        }
    }
}

#[test]
fn document_end_including_an_empty_final_line_can_scroll_above_bottom_margin() {
    for wrap in [true, false] {
        let mut core = Core::new(Document::new("line\n".repeat(80) + "end"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200., 80.);
        core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
        set_bottom_margin(&mut core, view, 28.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: core.document.text().len(),
            affinity: BoundaryAffinity::Upstream, extend_selection: false,
        }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a')))).unwrap();
        assert_row_above_margin(&core, view, 28.);
        for input in [InputEvent::Key(Key::Enter), InputEvent::Text("tail".into())] {
            core.handle(view, CoreEvent::Input(input)).unwrap();
            assert_row_above_margin(&core, view, 28.);
            let row = caret_row(&core, view);
            let layout = core.layout(view).unwrap();
            let total = layout.content_height().height() as f32;
            assert!((total - (row.y + row.height()) - 28.).abs() < 0.05,
                "document scroll extent includes the bottom margin exactly once");
            assert!((layout.viewport_top() + layout.height() - total).abs() < 0.05);
        }
    }
}



#[test]
fn exact_line_spacing_at_document_end_keeps_full_row_above_bottom_margin() {
    let source = format!("{}last", "line<br>".repeat(20));
    for wrap in [true, false] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut defaults: serde_json::Value = serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        let paragraph = defaults["block_styles"].as_array_mut().unwrap().iter_mut().find(|style| style["id"] == "Paragraph").unwrap();
        paragraph["character"]["size"] = serde_json::json!(20.0);
        paragraph["block"]["line_spacing"] = serde_json::json!({"Exact": 8.0});
        paragraph["block"]["margin_top"] = serde_json::json!(0.0);
        paragraph["block"]["margin_bottom"] = serde_json::json!(0.0);
        assert!(document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap().is_empty());

        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 60.);
        core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
        set_bottom_margin(&mut core, view, 28.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: core.document.text().len(),
            affinity: BoundaryAffinity::Upstream, extend_selection: false,
        }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a')))).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Text(" tail".into()))).unwrap();
        let row = caret_row(&core, view);
        assert!(row.natural_height() > row.line_advance);
        assert_row_above_margin(&core, view, 28.);
        let layout = core.layout(view).unwrap();
        assert!((row_ink_bottom(&row) - layout.viewport_top() - 32.).abs() < 0.05);
    }
}

#[test]
fn marked_text_and_its_commit_share_the_bottom_margin_reveal_policy() {
    use crate::command::composition::{CompositionTarget, CompositionUpdate};
    let (mut core, view) = fixture(200.);
    set_bottom_margin(&mut core, view, 28.);
    let offset = core.views[&view].commands.cursor();
    core.handle(view, CoreEvent::Composition(CompositionEvent::Begin(
        CompositionTarget::at_offsets(&core.document, offset..offset).unwrap(),
    ))).unwrap();
    let marked = "日本語\n語";
    core.handle(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
        marked_text: marked.into(), selected_range: marked.len()..marked.len(),
    }))).unwrap();
    let layout = core.presentation_layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    let geometry = snapshot.logical_endpoint_geometry(offset + marked.len(), BoundaryAffinity::Downstream).unwrap();
    let row = &snapshot.rows[geometry.row_index];
    assert!(row_ink_bottom(row) <= layout.viewport_top() + layout.height() - 28. + 0.05);
    let composed_baseline = row.baseline - layout.viewport_top();
    assert!((row_ink_bottom(row) - layout.viewport_top() - 212.).abs() < 0.05);
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit)).unwrap();
    assert_row_above_margin(&core, view, 28.);
    assert!((baseline(&core, view) - composed_baseline).abs() < 0.05);
}

#[test]
fn oversized_bottom_margin_yields_room_for_a_row_and_tall_rows_do_not_oscillate() {
    for (size, margin) in [(16., 200.), (64., 28.), (160., 28.)] {
        let mut core = Core::new(Document::new("line\n".repeat(30)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 80.);
        core.views.get_mut(&view).unwrap().layout.set_default_style(crate::layout::ResolvedTextStyle {
            size, ..Default::default()
        }).unwrap();
        set_bottom_margin(&mut core, view, margin);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: 52,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let row = caret_row(&core, view);
        let layout = core.layout(view).unwrap();
        if row.natural_height() <= layout.height() {
            assert!(row.y >= layout.viewport_top() - 0.05);
            assert!(row_ink_bottom(&row) <= layout.viewport_top() + layout.height() + 0.05);
        } else {
            assert!(row.baseline >= layout.viewport_top());
            assert!(row.baseline <= layout.viewport_top() + layout.height());
        }
        let top = layout.viewport_top();
        for _ in 0..4 {
            reveal_caret_row(core.views.get_mut(&view).unwrap()).unwrap();
            assert_eq!(core.layout(view).unwrap().viewport_top(), top);
        }
    }
}

#[test]
fn bottom_margin_change_invalidates_old_layout_without_measuring_the_document() {
    let (mut core, view) = fixture(64.);
    let before = core.layout(view).unwrap().configuration_generation();
    let top = core.layout(view).unwrap().viewport_top();
    let request = core.prepare_view_layout_job(
        view, LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(9_980..10_030, top, 240.).unwrap()),
        LayoutCancellationToken::new(),
    ).unwrap();
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let shape_calls = core.views[&view].engine.provider().request_calls();
    let revision = core.document.revision();
    set_bottom_margin(&mut core, view, 40.);
    assert_ne!(core.layout(view).unwrap().configuration_generation(), before);
    let installed = core.layout(view).unwrap().snapshot().unwrap().revision;
    assert!(core.install_view_layout_job(view, candidate).is_err());
    let layout = core.layout(view).unwrap();
    assert_eq!(layout.snapshot().unwrap().revision, installed);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(layout.height(), 240.);
    assert!((baseline(&core, view) - 64.).abs() < 0.05);
    assert!(layout.snapshot().unwrap().coverage.hard_lines().len() < 128);
    assert!(core.views[&view].engine.provider().request_calls() - shape_calls < 128);
}
