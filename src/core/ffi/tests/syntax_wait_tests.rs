use super::*;
use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult};
use crate::document::syntax::{Coverage, SyntaxRun, SyntaxStyleName};
use crate::ffi::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};

struct ReentrantSyntax(Arc<AtomicU64>);
impl SyntaxProvider for ReentrantSyntax {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        // Core registration follows view creation, which can enqueue syntax.
        let deadline = Instant::now() + Duration::from_secs(2);
        let handle = loop {
            let handle = self.0.load(Ordering::Acquire);
            if handle != 0 {
                break handle;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };
        std::thread::sleep(Duration::from_millis(10));
        // A detached wait must leave the core available to other callers. This
        // fixture cannot finish its result until the lease has been returned.
        loop {
            let mut state = ViemDocumentStateV1::default();
            let status = unsafe { viem_core_document_state(handle, &mut state) };
            if status == ViemStatus::Ok {
                break;
            }
            assert_eq!(status, ViemStatus::CoreBusy);
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let mut result = SyntaxResult::missing(request, "reentrant syntax completed");
        result.coverage = Coverage::Exact;
        result.runs = vec![SyntaxRun {
            range: request.range.clone(),
            name: SyntaxStyleName("Keyword".into()),
            origin: "fixture".into(),
            priority: 0,
        }];
        result
    }
}

#[test]
fn visible_syntax_wait_releases_core_lease_and_publishes_before_returning() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(
        Document::from_bytes(b"let value = 1;".to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("wait.rs", false).unwrap();
    let handle_cell = Arc::new(AtomicU64::new(0));
    let shared = handle_cell.clone();
    core.set_syntax_provider_factory(Arc::new(move || Box::new(ReentrantSyntax(shared.clone()))));
    let view = core.add_view(paint_test_provider(&mut storage, 591, 691), 240.0, 80.0);
    let original = summarize_document_state(core.document());
    let handle = register_core(core).unwrap();
    handle_cell.store(handle, Ordering::Release);
    let mut changed = 0;
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, &mut changed) },
        ViemStatus::Ok
    );
    assert_eq!(changed, 1);
    {
        let lease = checkout_core(handle).unwrap();
        assert!(lease
            .core()
            .syntax_style_names()
            .iter()
            .any(|name| name == "Keyword"));
        assert_eq!(summarize_document_state(lease.core().document()), original);
    }
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, &mut changed) },
        ViemStatus::Ok
    );
    assert_eq!(changed, 0, "ready coverage does not run the provider again");
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Gate {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}
struct BlockedSyntax(Arc<(Mutex<bool>, Condvar)>);
impl SyntaxProvider for BlockedSyntax {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        let mut open = self.0 .0.lock().unwrap();
        while !*open {
            open = self.0 .1.wait(open).unwrap();
        }
        SyntaxResult::missing(request, "unavailable fixture")
    }
}

#[test]
fn visible_syntax_wait_times_out_once_then_preserves_async_completion() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(
        Document::from_bytes(b"let value = 1;".to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("wait.rs", false).unwrap();
    let gate = Gate(Arc::new((Mutex::new(false), Condvar::new())));
    let shared = gate.0.clone();
    core.set_syntax_provider_factory(Arc::new(move || Box::new(BlockedSyntax(shared.clone()))));
    let view = core.add_view(paint_test_provider(&mut storage, 591, 691), 240.0, 80.0);
    let handle = register_core(core).unwrap();
    let mut changed = 0;
    let started = Instant::now();
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, &mut changed) },
        ViemStatus::Ok
    );
    assert!(started.elapsed() >= Duration::from_millis(90));
    assert!(started.elapsed() < Duration::from_secs(1));
    let started = Instant::now();
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, &mut changed) },
        ViemStatus::Ok
    );
    assert!(
        started.elapsed() < Duration::from_millis(80),
        "same viewport must not wait again"
    );
    drop(gate);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert_eq!(
            unsafe { viem_core_poll_syntax(handle, &mut changed) },
            ViemStatus::Ok
        );
        if checkout_core(handle)
            .unwrap()
            .core()
            .syntax_diagnostics()
            .contains("unavailable fixture")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn syntax_wait_validates_handles_view_and_output_before_work() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("plain text"));
    let view = core.add_view(paint_test_provider(&mut storage, 591, 691), 240.0, 80.0);
    let handle = register_core(core).unwrap();
    let mut changed = 99;
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, ptr::null_mut()) },
        ViemStatus::NullPointer
    );
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(0, view.0, &mut changed) },
        ViemStatus::InvalidHandle
    );
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0 + 1, &mut changed) },
        ViemStatus::InvalidView
    );
    assert_eq!(changed, 99);
    assert_eq!(
        unsafe { viem_core_view_wait_for_syntax(handle, view.0, &mut changed) },
        ViemStatus::Ok
    );
    assert_eq!(changed, 0);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}
