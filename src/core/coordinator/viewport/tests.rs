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

const LINE: &str = "let value = 42; // comment\n";

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
                    size: Some(size),
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
    core.publish_code_presentation(Arc::new(sheet), &runs);
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
    for format in [Format::PlainText, Format::Markdown, Format::Html, Format::Rtf] {
        // Every format contains many independent paragraphs so edits must
        // invalidate only the active neighborhood of the large document.
        let (mut core, view) = if format.is_rich_text() {
            let source = if format == Format::Html {
                "<p>An ordinary paragraph with several words that wrap across the narrow text view.</p>".repeat(20_000)
            } else {
                format!(r"{{\rtf1 {}}}", r"An ordinary paragraph with several words that wrap across the narrow text view.\par ".repeat(20_000))
            };
            let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 240.);
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document.revision(), text_offset: 60_006,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            let row = caret_row(&core, view);
            core.handle(view, CoreEvent::SetViewportOrigin {
                left: 0., top: Some(row.baseline - 220.),
            }).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
            (core, view)
        } else {
            fixture_for_format(220., format)
        };
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
