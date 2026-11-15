use crate::ffi::*;
use std::sync::{Arc, Condvar, Mutex, atomic::{AtomicUsize, Ordering}};

#[derive(Default)]
struct Receiver {
    wakes: AtomicUsize,
    retains: AtomicUsize,
    releases: AtomicUsize,
    released: (Mutex<usize>, Condvar),
}
unsafe extern "C" fn wake(context: *mut c_void) {
    let receiver = unsafe { &*(context as *const Receiver) };
    receiver.wakes.fetch_add(1, Ordering::SeqCst);
    // Acquiring this lock would deadlock if the bridge ran under the registry.
    drop(super::super::core_registry().lock().unwrap());
}
unsafe extern "C" fn retain(context: *mut c_void) {
    let receiver = unsafe { &*(context as *const Receiver) };
    receiver.retains.fetch_add(1, Ordering::SeqCst);
    unsafe { Arc::increment_strong_count(context as *const Receiver); }
}
unsafe extern "C" fn release(context: *mut c_void) {
    let receiver = unsafe { &*(context as *const Receiver) };
    receiver.releases.fetch_add(1, Ordering::SeqCst);
    {
        let mut count = receiver.released.0.lock().unwrap();
        *count += 1;
        receiver.released.1.notify_all();
    }
    unsafe { Arc::decrement_strong_count(context as *const Receiver); }
}
fn await_release(receiver: &Receiver) {
    let count = receiver.released.0.lock().unwrap();
    let (count, _) = receiver.released.1.wait_timeout_while(count, std::time::Duration::from_secs(5), |count| *count == 0).unwrap();
    assert_eq!(*count, 1, "one retained context releases after in-flight callbacks finish");
}
fn callbacks(receiver: &Arc<Receiver>) -> ViemBackgroundWakeCallbacks {
    ViemBackgroundWakeCallbacks {
        context: Arc::as_ptr(receiver) as *mut c_void,
        wake: Some(wake), retain: Some(retain), release: Some(release),
    }
}
fn text_core() -> ViemCoreHandle {
    register_core(Core::new(Document::from_bytes(b"text".to_vec(), Encoding::Utf8, Format::PlainText).unwrap())).unwrap()
}
fn collect(handle: ViemCoreHandle) -> ViemBackgroundWorkStatus {
    let mut status = ViemBackgroundWorkStatus::default();
    assert_eq!(unsafe { viem_core_service_background_work(handle, &mut status) }, ViemStatus::Ok);
    status
}

#[test]
fn text_background_collection_has_no_deadline_or_ready_work() {
    let handle = text_core();
    let receiver = Arc::new(Receiver::default());
    assert_eq!(unsafe { viem_core_set_background_wake(handle, &callbacks(&receiver)) }, ViemStatus::Ok);
    // Global package/style events from other parallel tests may legitimately
    // notify this receiver. Neither they nor Text collection need a deadline.
    collect(handle);
    collect(handle);
    for _ in 0..20 {
        let status = collect(handle);
        assert_eq!(status.next_deadline_ms, u32::MAX);
        assert_eq!(status.more_ready, 0);
        let mut revision = 0;
        assert_eq!(unsafe { viem_core_revision(handle, &mut revision) }, ViemStatus::Ok);
    }
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn replacing_and_destroying_subscription_balances_context_ownership() {
    let handle = text_core();
    let first = Arc::new(Receiver::default());
    let second = Arc::new(Receiver::default());
    assert_eq!(unsafe { viem_core_set_background_wake(handle, &callbacks(&first)) }, ViemStatus::Ok);
    assert_eq!(first.retains.load(Ordering::SeqCst), 1);
    assert_eq!(unsafe { viem_core_set_background_wake(handle, &callbacks(&second)) }, ViemStatus::Ok);
    await_release(&first);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    await_release(&second);
}

#[test]
fn unregister_suppresses_future_signals_even_with_retained_worker_signal() {
    let handle = text_core();
    let receiver = Arc::new(Receiver::default());
    assert_eq!(unsafe { viem_core_set_background_wake(handle, &callbacks(&receiver)) }, ViemStatus::Ok);
    let signal = with_core_mut(handle, |core| Ok(core.background_work_wake())).unwrap();
    assert_eq!(unsafe { viem_core_set_background_wake(handle, std::ptr::null()) }, ViemStatus::Ok);
    await_release(&receiver);
    let calls = receiver.wakes.load(Ordering::SeqCst);
    signal.acknowledge();
    signal.signal();
    assert_eq!(receiver.wakes.load(Ordering::SeqCst), calls);
    await_release(&receiver);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn failed_destroy_preserves_the_live_subscription() {
    let handle = text_core();
    let receiver = Arc::new(Receiver::default());
    assert_eq!(unsafe { viem_core_set_background_wake(handle, &callbacks(&receiver)) }, ViemStatus::Ok);
    let signal = with_core_mut(handle, |core| Ok(core.background_work_wake())).unwrap();
    let lease = checkout_core(handle).unwrap();
    assert_eq!(viem_core_destroy(handle), ViemStatus::CoreBusy);
    assert_eq!(receiver.releases.load(Ordering::SeqCst), 0);
    assert!(signal.has_callback());
    signal.acknowledge();
    signal.signal();
    drop(lease);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    await_release(&receiver);
}

#[test]
fn background_status_rejects_null_output() {
    let handle = text_core();
    assert_eq!(unsafe { viem_core_service_background_work(handle, std::ptr::null_mut()) }, ViemStatus::NullPointer);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}
