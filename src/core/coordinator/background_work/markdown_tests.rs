use super::*;
use crate::document::syntax::{Coverage, service::{SyntaxProvider, SyntaxRequest, SyntaxResult}};
use crate::layout::MockTextMeasurementProvider;
use std::sync::{Arc, Condvar, Mutex, mpsc, atomic::{AtomicBool, AtomicUsize, Ordering}};

#[derive(Default)]
struct Permits {
    state: Mutex<(usize, bool)>,
    available: Condvar,
}
impl Permits {
    fn release_one(&self) {
        self.state.lock().unwrap().0 += 1;
        self.available.notify_one();
    }
    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        while state.0 == 0 && !state.1 { state = self.available.wait(state).unwrap(); }
        if !state.1 { state.0 -= 1; }
    }
    fn release_all(&self) {
        self.state.lock().unwrap().1 = true;
        self.available.notify_all();
    }
}
struct Cleanup {
    permits: Arc<Permits>,
    wake: WorkWake,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.wake.replace_callback(None);
        self.permits.release_all();
    }
}
struct GatedResult {
    permits: Arc<Permits>,
    calls: Arc<AtomicUsize>,
    terminal_failure: bool,
}
impl SyntaxProvider for GatedResult {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        self.permits.wait();
        self.calls.fetch_add(1, Ordering::AcqRel);
        let mut result = SyntaxResult::missing(request, "fixture syntax budget exhausted");
        if !self.terminal_failure { result.coverage = Coverage::Exact; }
        result
    }
}

fn ready_offscreen_blocks_drain_and_park(terminal_failure: bool) {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let source = (0..9).map(|index| format!("```rust\nlet item_{index} = {index};\n```\n\n")).collect::<String>();
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    core.initialize_code_detection("fixture.md", false).unwrap();
    let permits = Arc::new(Permits::default());
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_syntax_provider_factory({
        let permits = permits.clone();
        let calls = calls.clone();
        Arc::new(move || Box::new(GatedResult { permits: permits.clone(), calls: calls.clone(), terminal_failure }))
    });

    // Finish the separate projection job first, using its own publication
    // notification. The drain scenario then needs neither a future debounce
    // deadline nor an unrelated projection completion to keep it moving.
    let projection_wake = WorkWake::default();
    let (projection_sent, projection_received) = mpsc::channel();
    projection_wake.replace_callback(Some(Arc::new(move || { let _ = projection_sent.send(()); })));
    core.document.prefetch_view_format(Format::Code, projection_wake);
    projection_received.recv_timeout(Duration::from_secs(5)).expect("alternate projection completed");
    assert!(core.document.view_format_prefetch_ready(Format::Code));

    let now = Instant::now();
    core.syntax.view_format_changed = Some((
        (core.document.id(), core.document.revision()), now - Duration::from_millis(400),
    ));
    let wake = core.background_work_wake();
    let _cleanup = Cleanup { permits: permits.clone(), wake: wake.clone() };
    let initial = core.service_background_work_at(now);
    assert!(initial.next_deadline.is_none());
    assert!(core.views.is_empty(), "every fixture block is offscreen");
    assert_eq!(core.syntax.markdown.services.len(), 9);
    assert!(core.syntax.prepared_code.is_some());
    assert_eq!(calls.load(Ordering::Acquire), 0, "providers remain suspended until all services exist");

    let (completed, completions) = mpsc::channel();
    let callback_wake = wake.clone();
    wake.replace_callback(Some(Arc::new(move || {
        // The owner releases only one provider permit between acknowledgments,
        // so concurrent worker signals cannot coalesce away a barrier arrival.
        callback_wake.acknowledge();
        let _ = completed.send(());
    })));
    let total = core.syntax.markdown.services.len() + 1; // Prepared Code also settles.
    for _ in 0..total {
        permits.release_one();
        completions.recv_timeout(Duration::from_secs(5)).expect("provider publication completed");
    }
    assert_eq!(calls.load(Ordering::Acquire), total);
    assert!(core.syntax.markdown.services.values().all(|block| block.service.has_ready_work()));
    assert!(core.syntax.prepared_code.as_ref().unwrap().service.has_ready_work());
    let requests_before = core.syntax.markdown.services.values().map(|block| block.service.statistics.requests).sum::<u64>();

    // All worker publications already exist, and there are no more permits.
    // The owner must drive the remaining bounded passes using more_ready alone.
    for remaining in [5, 1, 0] {
        let status = core.service_background_work_at(now);
        assert_eq!(core.syntax.markdown.services.values().filter(|block| block.service.has_ready_work()).count(), remaining);
        assert_eq!(status.more_ready, remaining != 0);
        assert!(status.next_deadline.is_none());
    }
    let settled = core.service_background_work_at(now + Duration::from_secs(60));
    assert!(!settled.more_ready && settled.next_deadline.is_none());
    assert!(!settled.changed && !settled.diagnostics_changed);
    assert!(core.syntax.markdown.services.values().all(|block| !block.service.has_pending_work()));
    assert!(!core.syntax.prepared_code.as_ref().unwrap().service.has_pending_work());
    assert_eq!(core.syntax.markdown.services.values().map(|block| block.service.statistics.requests).sum::<u64>(), requests_before);
    assert_eq!(calls.load(Ordering::Acquire), total, "draining never restarts settled analysis");
    assert!(matches!(completions.try_recv(), Err(mpsc::TryRecvError::Empty)));
    if terminal_failure {
        assert!(core.syntax.markdown.services.values().all(|block| block.service.diagnostics().contains("budget exhausted")));
    }
}

#[test]
fn ready_offscreen_markdown_blocks_finish_with_self_driven_bounded_drains() {
    ready_offscreen_blocks_drain_and_park(false);
}

#[test]
fn terminal_markdown_syntax_budgets_settle_without_a_work_loop() {
    ready_offscreen_blocks_drain_and_park(true);
}

#[test]
fn more_code_views_than_coverage_entries_settle_and_grace_paints_stay_idle() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let source = "let value = 0;\n".repeat(18_000);
    assert!(source.len() > super::super::prelayout::FULL_PRELAYOUT_SOURCE_BYTES);
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    core.initialize_code_detection("fixture.rs", false).unwrap();
    let permits = Arc::new(Permits::default());
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_syntax_provider_factory({
        let permits = permits.clone();
        let calls = calls.clone();
        Arc::new(move || Box::new(GatedResult { permits: permits.clone(), calls: calls.clone(), terminal_failure: false }))
    });
    let wake = core.background_work_wake();
    let _cleanup = Cleanup { permits: permits.clone(), wake: wake.clone() };
    let (completed, completions) = mpsc::channel();
    let callback_wake = wake.clone();
    wake.replace_callback(Some(Arc::new(move || {
        callback_wake.acknowledge();
        let _ = completed.send(());
    })));
    let count = crate::document::syntax::service::MAX_CACHED_REGIONS + 1;
    let mut views = Vec::new();
    for index in 0..count {
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 40.);
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(index as f32 * 4000.) }).unwrap();
        views.push(view);
    }
    let input = core.syntax_input();
    let mut ranges = core.views.values().map(|view| core.visible_syntax_range(&input, view)).collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    ranges.dedup();
    assert_eq!(ranges.len(), count, "each view contributes a distinct demand");
    assert!(ranges.windows(2).all(|pair| pair[0].end < pair[1].start), "demands cannot cover one another");

    let now = Instant::now();
    let mut settled = false;
    for _ in 0..count * 4 {
        let status = core.service_background_work_at(now);
        assert!(status.next_deadline.is_none(), "large Code documents have no speculative deadline");
        if !core.syntax.code.service.has_pending_work() {
            assert!(!status.more_ready);
            settled = true;
            break;
        }
        // Exactly one completion advances each owner turn. No elapsed-time
        // polling is needed, and coalescing cannot hide a barrier arrival.
        permits.release_one();
        completions.recv_timeout(Duration::from_secs(5)).expect("visible syntax result completed");
    }
    assert!(settled, "unchanged views must not evict and re-request each other's coverage indefinitely");
    let requests = core.syntax_statistics().requests;
    for view in views {
        let (_, wait) = core.prepare_view_syntax_wait(view, Instant::now() + Duration::from_millis(100)).unwrap();
        assert!(wait.is_none(), "a served visible range does not restart its paint grace period after cache eviction");
    }
    let status = core.service_background_work_at(now + Duration::from_secs(60));
    assert!(!status.more_ready && status.next_deadline.is_none());
    assert!(!core.syntax.code.service.has_pending_work());
    assert_eq!(core.syntax_statistics().requests, requests);
    assert!(matches!(completions.try_recv(), Err(mpsc::TryRecvError::Empty)));
}
