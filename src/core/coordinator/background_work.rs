//! Owner-thread collection and explicit deadlines for disposable background work.
use super::*;
use crate::background::WorkWake;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct BackgroundWork {
    pub(super) wake: WorkWake,
    observed: Option<Dependencies>,
    views: Vec<ViewDependencies>,
    pub(super) deadline: Option<Instant>,
    pub(super) now: Option<Instant>,
    diagnostics: String,
}

#[derive(PartialEq, Eq)]
struct Dependencies {
    document: DocumentId,
    literal: Option<Revision>,
    preview: Option<Revision>,
    format: Format,
    syntax_configuration: u64,
    sheet: crate::document::StyleSheetRevision,
    registry: u64,
}

#[derive(PartialEq, Eq)]
struct ViewDependencies {
    id: ViewId,
    presentation: crate::document::ViewPresentation,
    viewport: [u32; 4],
    configuration: u64,
    layout: Option<u64>,
}

#[derive(Debug)]
pub struct BackgroundWorkStatus {
    pub changed: bool,
    pub diagnostics_changed: bool,
    pub more_ready: bool,
    pub next_deadline: Option<Duration>,
}

impl BackgroundWork {
    pub(super) fn schedule_at(&mut self, deadline: Instant) {
        self.deadline = Some(self.deadline.map_or(deadline, |old| old.min(deadline)));
    }
    pub(super) fn now(&self) -> Instant { self.now.unwrap_or_else(Instant::now) }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Connect existing and future worker mailboxes to this document's signal.
    /// Register the foreign receiver on the returned signal outside a core turn.
    pub fn background_work_wake(&mut self) -> WorkWake {
        let wake = self.background.wake.clone();
        self.syntax.code.service.set_work_wake(wake.clone());
        if let Some(prepared) = &mut self.syntax.prepared_code {
            prepared.service.set_work_wake(wake.clone());
        }
        for block in self.syntax.markdown.services.values_mut() {
            block.service.set_work_wake(wake.clone());
        }
        wake
    }

    pub(crate) fn detach_background_callback(&self) -> Option<crate::background::WakeCallback> {
        self.background.wake.replace_callback(None)
    }

    /// Called at the end of a foreign coordinator turn. Comparing dependency
    /// identities avoids per-input-path native hooks and does no document scan.
    /// Return the signal; invoke it only after returning the core to its owner.
    pub(crate) fn take_background_dependency_wake(&mut self) -> Option<WorkWake> {
        if !self.background.wake.has_callback() { return None; }
        let revision = |kind| self.document.has_lineage(kind).then(|| self.document.revision_of(kind));
        let dependencies = Dependencies {
            document: self.document.id(),
            literal: revision(crate::document::ProjectionKind::Literal),
            preview: revision(crate::document::ProjectionKind::Preview),
            format: self.document.buffer_format(),
            syntax_configuration: self.syntax.code.service.configuration.generation,
            sheet: crate::document::code_style::snapshot().revision,
            registry: crate::document::syntax::treesitter::package_registry_generation(),
        };
        let views = || self.views.iter().map(|(id, view)| ViewDependencies {
            id: *id,
            presentation: view.presentation,
            viewport: [view.layout.viewport_left().to_bits(), view.layout.viewport_top().to_bits(),
                view.layout.width().to_bits(), view.layout.height().to_bits()],
            configuration: view.layout.configuration_generation().0,
            layout: view.layout.snapshot().map(|snapshot| snapshot.revision.0),
        });
        if self.background.observed.as_ref() == Some(&dependencies)
            && self.background.views.len() == self.views.len()
            && self.background.views.iter().zip(views()).all(|(old, new)| old == &new) {
            return None;
        }
        self.background.views = views().collect();
        self.background.observed = Some(dependencies);
        Some(self.background.wake.clone())
    }

    pub fn service_background_work(&mut self) -> BackgroundWorkStatus {
        self.service_background_work_at(Instant::now())
    }

    pub(super) fn service_background_work_at(&mut self, now: Instant) -> BackgroundWorkStatus {
        self.background.wake.acknowledge();
        self.background.now = Some(now);
        let changed = self.poll_syntax();
        self.background.now = None;
        // At this stable owner boundary any usable Code preparation has been
        // adopted. Do not retire it inside adoption itself: a mode switch can
        // materialize Code before installing its final language configuration.
        if self.document.buffer_format().is_code() { self.syntax.prepared_code = None; }
        let diagnostics = self.syntax_diagnostics();
        let diagnostics_changed = self.background.diagnostics != diagnostics;
        self.background.diagnostics = diagnostics;
        let more_ready = self.syntax.code.service.has_ready_work()
            || (self.document.buffer_format().is_markdown() && !self.code_layer_wanted()
                && self.syntax.prepared_code.as_ref().is_some_and(|prepared| prepared.service.has_ready_work()))
            || self.syntax.markdown.services.values().any(|block| block.service.has_ready_work());
        BackgroundWorkStatus {
            changed, diagnostics_changed, more_ready,
            next_deadline: self.background.deadline.map(|deadline| deadline.saturating_duration_since(now)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult};
    use crate::layout::MockTextMeasurementProvider;
    use std::sync::{Arc, mpsc, atomic::AtomicBool};

    struct Exact;
    impl SyntaxProvider for Exact {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let mut result = SyntaxResult::missing(request, "fixture diagnostic");
            result.coverage = crate::document::syntax::Coverage::Exact;
            result
        }
    }

    fn fixture(format: Format, text: &[u8]) -> (Core<MockTextMeasurementProvider>, mpsc::Receiver<()>) {
        let mut core = Core::new(Document::from_bytes(text.to_vec(), Encoding::Utf8, format).unwrap());
        core.initialize_code_detection(if format.is_code() { "fixture.rs" } else { "fixture.md" }, false).unwrap();
        core.set_syntax_provider_factory(Arc::new(|| Box::new(Exact)));
        let (sender, receiver) = mpsc::channel();
        let wake = core.background_work_wake();
        wake.replace_callback(Some(Arc::new(move || { let _ = sender.send(()); })));
        (core, receiver)
    }

    fn settle_code(core: &mut Core<MockTextMeasurementProvider>, receiver: &mpsc::Receiver<()>, now: Instant) -> BackgroundWorkStatus {
        for _ in 0..100 {
            let status = core.service_background_work_at(now);
            if status.more_ready { continue; }
            if !core.syntax.code.service.has_pending_work() { return status; }
            receiver.recv_timeout(Duration::from_secs(5)).expect("worker completion wakes the owner");
        }
        panic!("bounded fixture must settle");
    }

    #[test]
    fn complete_highlighting_deadline_runs_once_then_parks() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let (mut core, receiver) = fixture(Format::Code, b"let answer = 42;");
        let now = Instant::now();
        let status = settle_code(&mut core, &receiver, now);
        assert!(status.next_deadline.is_some_and(|delay| delay <= Duration::from_millis(250)));
        let status = settle_code(&mut core, &receiver, now + Duration::from_millis(250));
        assert!(status.next_deadline.is_none());
        assert!(!status.more_ready);
        let requests = core.syntax_statistics().requests;
        let status = core.service_background_work_at(now + Duration::from_secs(60));
        assert!(status.next_deadline.is_none());
        assert!(!status.changed);
        assert!(!status.diagnostics_changed);
        assert_eq!(core.syntax_statistics().requests, requests);
    }

    #[test]
    fn alternate_projection_completion_wakes_owner_after_one_deadline() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let (mut core, receiver) = fixture(Format::Markdown, b"# Heading\n\nProse");
        let now = Instant::now();
        let status = core.service_background_work_at(now);
        assert_eq!(status.next_deadline, Some(Duration::from_millis(400)));
        assert!(!core.document.view_format_prefetch_ready(Format::Code));
        let status = core.service_background_work_at(now + Duration::from_millis(400));
        assert!(status.next_deadline.is_none());
        receiver.recv_timeout(Duration::from_secs(5)).expect("projection completion wakes the owner");
        assert!(core.document.view_format_prefetch_ready(Format::Code));
        core.service_background_work_at(now + Duration::from_millis(400));
        assert!(core.syntax.prepared_code.is_some(), "projection completion starts Code preparation");
    }

    #[test]
    fn complete_highlighting_restarts_after_language_changes_without_an_edit() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let (mut core, receiver) = fixture(Format::Code, b"token");
        let now = Instant::now();
        core.service_background_work_at(now);
        settle_code(&mut core, &receiver, now + Duration::from_millis(250));
        let requests = core.syntax_statistics().requests;
        core.set_code_language(crate::document::syntax::detection::LanguageSelection::Language("python".into()));
        settle_code(&mut core, &receiver, now + Duration::from_millis(250));
        assert!(core.syntax_statistics().requests > requests, "offscreen work must restart for the new language");
        assert!(!core.service_background_work_at(now + Duration::from_secs(1)).more_ready);
    }

    #[test]
    fn incompatible_prepared_code_cannot_leave_an_immediate_work_loop() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let (mut core, receiver) = fixture(Format::Code, b"token");
        let input = core.syntax_input();
        let mut prepared = core.syntax.code.service.for_export();
        prepared.set_filename(Some("previous.rs".into()));
        prepared.request(input.clone(), 0..5);
        core.syntax.prepared_code = Some(super::super::syntax::PreparedCodeSyntax {
            source_revision: input.identity().revision,
            identity: input.identity(), service: prepared,
        });
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        settle_code(&mut core, &receiver, Instant::now() + Duration::from_secs(1));
        assert!(core.syntax.prepared_code.is_none());
        assert!(!core.service_background_work().more_ready);
    }
}

#[cfg(test)]
mod markdown_tests;
