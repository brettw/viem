use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{ArtifactOverwrite, ArtifactPath, ArtifactWriteIntent, BoundaryAffinity};
use viem_core::layout::{
    MeasurementEnvironmentId, MeasurementError, MetricsGeneration, MockTextMeasurementProvider,
    RenderRunPolicy, ShapeRequest, ShapedFragment, TextMeasurementProvider,
};
use viem_core::{
    CompositionEvent, CompositionTarget, CompositionUpdate, Core, CoreEvent, Document, ViewId,
};

struct Measured {
    mock: MockTextMeasurementProvider,
    bytes: Arc<AtomicUsize>,
    generation: Arc<AtomicU64>,
}
impl TextMeasurementProvider for Measured {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.mock.measurement_environment_id()
    }
    fn metrics_generation(&self) -> MetricsGeneration {
        MetricsGeneration(self.generation.load(Ordering::Relaxed))
    }
    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.mock.render_run_policy()
    }
    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.bytes.fetch_add(
            requests.iter().map(|request| request.text.len()).sum(),
            Ordering::Relaxed,
        );
        self.mock.set_metrics_generation(self.metrics_generation());
        self.mock.shape_batch(requests)
    }
}

fn key<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, key: Key) {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}
fn place<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, at: usize) {
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
}
fn finish<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId) {
    for _ in 0..20_000 {
        if !core
            .completion_presentation(view)
            .unwrap()
            .is_some_and(|menu| menu.searching)
        {
            return;
        }
        core.poll_completion(view).unwrap();
    }
    panic!("bounded completion did not finish");
}

#[test]
fn composition_begin_accepts_and_rebases_its_old_caret_then_has_separate_undo() {
    let mut core = Core::new(Document::new("alphabet\nal"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 160., 80.);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 11);
    key(&mut core, view, Key::Ctrl('p'));
    finish(&mut core, view);
    let target = CompositionTarget::at_offsets(core.document(), 11..11).unwrap();
    let outcome = core
        .handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
    assert!(outcome.document_changed);
    assert_eq!(core.document().text(), "alphabet\nalphabet");
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert_eq!(
        core.composition_overlay(view)
            .unwrap()
            .unwrap()
            .replacement_range(),
        17..17
    );
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("é", 2..2))),
    )
    .unwrap();
    assert_eq!(core.document().text(), "alphabet\nalphabet");
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "alphabet\nalphabet");
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "alphabet\nal");
}

#[test]
fn resizing_and_metrics_invalidate_preview_geometry_but_not_candidates_or_source() {
    let bytes = Arc::new(AtomicUsize::new(0));
    let generation = Arc::new(AtomicU64::new(1));
    let provider = Measured {
        mock: MockTextMeasurementProvider::new(),
        bytes: bytes.clone(),
        generation: generation.clone(),
    };
    let source = format!(
        "al\nalphabet alphanumeric\n{}",
        "unchanged writing\n".repeat(20_000)
    );
    let mut core = Core::new(Document::new(source.clone()));
    let view = core.add_view(provider, 180., 80.);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 2);
    let base_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
    bytes.store(0, Ordering::Relaxed);
    key(&mut core, view, Key::Ctrl('n'));
    let first = core
        .presentation_layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .revision;
    assert_ne!(first, base_revision);
    assert_eq!(
        core.layout(view).unwrap().snapshot().unwrap().revision,
        base_revision
    );
    let session = core
        .completion_presentation(view)
        .unwrap()
        .unwrap()
        .session_id;
    assert!(
        bytes.load(Ordering::Relaxed) < 20_000,
        "preview shaped offscreen document"
    );
    generation.store(2, Ordering::Relaxed);
    core.handle(
        view,
        CoreEvent::Resize {
            width: 95.,
            height: 80.,
        },
    )
    .unwrap();
    let resized = core.presentation_layout(view).unwrap().snapshot().unwrap();
    assert_ne!(resized.revision, first);
    assert_eq!(resized.metrics_generation, MetricsGeneration(2));
    assert_eq!(
        core.completion_presentation(view)
            .unwrap()
            .unwrap()
            .session_id,
        session
    );
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(
        bytes.load(Ordering::Relaxed) < 40_000,
        "resize flattened the document"
    );
    key(&mut core, view, Key::Escape);
    assert!(core.composition_overlay(view).unwrap().is_none());
    assert_eq!(
        core.presentation_layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .document_revision,
        core.document().revision()
    );
}

#[test]
fn literal_register_and_control_g_operands_keep_completion_key_ownership() {
    let mut core = Core::new(Document::new("alphabet\nal"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 160., 80.);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 11);
    key(&mut core, view, Key::Ctrl('v'));
    key(&mut core, view, Key::Ctrl('n'));
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert!(core.document().text().ends_with('\u{e}'));
    key(&mut core, view, Key::Ctrl('r'));
    key(&mut core, view, Key::Ctrl('p'));
    assert!(core.completion_presentation(view).unwrap().is_none());
    key(&mut core, view, Key::Ctrl('g'));
    key(&mut core, view, Key::Ctrl('n'));
    assert!(core.completion_presentation(view).unwrap().is_none());
}

#[test]
fn save_capture_accepts_visible_completion_and_removal_retires_preview() {
    let mut core = Core::new(Document::new("alphabet\nal"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 160., 80.);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 11);
    key(&mut core, view, Key::Ctrl('p'));
    finish(&mut core, view);
    let _prepared = core
        .prepare_artifact_write(ArtifactWriteIntent::SaveAs {
            destination: ArtifactPath::from("/tmp/viem-completion-test-not-written"),
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    assert_eq!(core.document().text(), "alphabet\nalphabet");
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    place(&mut core, view, 11);
    key(&mut core, view, Key::Ctrl('p'));
    let before = core.document().source_bytes();
    core.remove_view(view).unwrap();
    assert!(core.completion_presentation(view).is_err());
    assert_eq!(core.document().source_bytes(), before);
}
