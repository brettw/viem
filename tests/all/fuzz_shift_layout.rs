//! Minimized from seed1-followup.jsonl, action 1397 (`3<<`). Nine LF bytes
//! suffice: scrolling away from cursor zero left the command's layout without
//! its starting row, even though the shift itself should be a no-op.
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{
    MeasurementEnvironmentId, MeasurementError, MetricsGeneration, MockTextMeasurementProvider,
    RenderRunPolicy, ShapeRequest, ShapedFragment, TextMeasurementProvider,
};
use viem_core::{Core, CoreEvent, CoreOutcome, ViewId};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn key<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, key: Key) -> CoreOutcome {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}"));
    if let Some(command) = &outcome.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            ),
            "{key:?}: {:?}",
            command.status
        );
    }
    outcome
}

fn keys<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, text: &str) {
    for ch in text.chars() {
        key(core, view, Key::Char(ch));
    }
}

fn scroll<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, top: f32) {
    if core.document().format().is_source_view() {
        core.handle(view, CoreEvent::SetParagraphFlow(false)).unwrap();
    }
    core.handle(view, CoreEvent::SetLineMode(viem_core::command::LineMode::PhysicalSource)).unwrap();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(
        !snapshot
            .coverage
            .contains_text_offset(core.command_state(view).unwrap().cursor()),
        "the regression requires a cursor outside the scrolled layout"
    );
}

fn assert_cursor_is_materialized<P: TextMeasurementProvider>(core: &Core<P>, view: ViewId) {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.document_revision, core.document().revision());
    snapshot
        .caret_point(
            core.command_state(view).unwrap().cursor(),
            BoundaryAffinity::Downstream,
        )
        .unwrap();
}

#[test]
fn counted_noop_outdent_recovers_an_offscreen_empty_line() {
    for format in [ Format::PlainText] {
        for count in [1, 3] {
            let source = "\n".repeat(9);
            let mut core = Core::new(document(&source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 180., 16.);
            scroll(&mut core, view, 1000.);
            let revision = core.document().revision();
            let history = core.document().history_status().current;
            keys(&mut core, view, &format!("{count}<<"));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status().current, history);
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
            assert_cursor_is_materialized(&core, view);
        }
    }
}

#[test]
fn counted_outdent_changes_only_the_requested_source_lines() {
    let tail = format!(
        "    keep\n{}<!--keep unknown='byte exact'>-->",
        "line\n".repeat(30)
    );
    let source = format!("  alpha\n\n   \n{tail}");
    let expected = format!("alpha\n\n\n{tail}");
    let mut core = Core::new(document(&source, Format::MarkdownSource));
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 16.);
    scroll(&mut core, view, 1000.);
    keys(&mut core, view, "3<<");
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_cursor_is_materialized(&core, view);
    let reopened = document(&expected, Format::MarkdownSource);
    assert_eq!(reopened.text(), core.document().text());
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
}

#[test]
fn another_views_edit_invalidates_the_scrolled_command_layout() {
    let source = format!("\n\n\n{}last", "line\n".repeat(40));
    let mut core = Core::new(document(&source, Format::PlainText));
    let editing = core.add_view(MockTextMeasurementProvider::new(), 240., 80.);
    let scrolled = core.add_view(MockTextMeasurementProvider::new(), 180., 16.);
    scroll(&mut core, scrolled, 800.);
    let old_snapshot = core.layout(scrolled).unwrap().snapshot().unwrap().clone();
    core.handle(
        editing,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: source.len() - 1,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    keys(&mut core, editing, "A!");
    key(&mut core, editing, Key::Escape);
    let edited = format!("{source}!");
    assert_eq!(core.document().source_bytes(), edited.as_bytes());
    assert_eq!(core.command_state(scrolled).unwrap().cursor(), 0);
    assert_ne!(old_snapshot.document_revision, core.document().revision());
    let history = core.document().history_status().current;
    keys(&mut core, scrolled, "3<<");
    assert_eq!(core.document().source_bytes(), edited.as_bytes());
    assert_eq!(core.document().history_status().current, history);
    assert_cursor_is_materialized(&core, scrolled);
    assert_eq!(core.command_state(editing).unwrap().cursor(), source.len());
    key(&mut core, scrolled, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    key(&mut core, scrolled, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), edited.as_bytes());
}

struct CountedProvider {
    inner: MockTextMeasurementProvider,
    generation: Arc<AtomicU64>,
    requests: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}

impl TextMeasurementProvider for CountedProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.inner.measurement_environment_id()
    }

    fn metrics_generation(&self) -> MetricsGeneration {
        MetricsGeneration(self.generation.load(Ordering::Relaxed))
    }

    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.inner.render_run_policy()
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.requests.fetch_add(requests.len(), Ordering::Relaxed);
        self.bytes.fetch_add(
            requests.iter().map(|request| request.text.len()).sum(),
            Ordering::Relaxed,
        );
        self.inner.set_metrics_generation(self.metrics_generation());
        self.inner.shape_batch(requests)
    }
}

#[test]
fn offscreen_shift_with_changed_metrics_shapes_bounded_large_document_work() {
    let source = format!("\n\n\n{}", "some text\n".repeat(20_000));
    let generation = Arc::new(AtomicU64::new(1));
    let requests = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    let provider = CountedProvider {
        inner: MockTextMeasurementProvider::new(),
        generation: Arc::clone(&generation),
        requests: Arc::clone(&requests),
        bytes: Arc::clone(&bytes),
    };
    let mut core = Core::new(document(&source, Format::MarkdownSource));
    let view = core.add_view(provider, 180., 80.);
    scroll(&mut core, view, 100_000.);
    let old_layout = core.layout(view).unwrap().snapshot().unwrap().revision;
    generation.store(2, Ordering::Relaxed);
    requests.store(0, Ordering::Relaxed);
    bytes.store(0, Ordering::Relaxed);
    keys(&mut core, view, "3<<");
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_ne!(snapshot.revision, old_layout);
    assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
    assert!(snapshot.rows.len() < 100);
    let requests = requests.load(Ordering::Relaxed);
    let bytes = bytes.load(Ordering::Relaxed);
    assert!(requests > 0 && requests < 256, "shaped {requests} requests");
    assert!(bytes < 4096, "shaped {bytes} bytes");
    assert_cursor_is_materialized(&core, view);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
