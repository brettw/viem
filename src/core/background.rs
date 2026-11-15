//! Coalesced wake hints. Results stay in their owning mailboxes; a wake never
//! authorizes publication. Call `signal` only after releasing all locks.
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, OnceLock, Weak};

pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    pending: AtomicBool,
    enabled: AtomicBool,
    subscribed: AtomicBool,
    callback: Mutex<Option<WakeCallback>>,
}

#[derive(Clone, Default)]
pub struct WorkWake(Arc<State>);

impl WorkWake {
    pub fn signal(&self) {
        if self.0.pending.swap(true, Ordering::AcqRel) { return; }
        let callback = self.0.callback.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(callback) = callback { callback(); }
    }

    /// Acknowledge before collecting results, never after: a completion racing
    /// collection must leave another durable wake behind.
    pub fn acknowledge(&self) { self.0.pending.store(false, Ordering::Release); }

    pub fn is_pending(&self) -> bool { self.0.pending.load(Ordering::Acquire) }
    pub fn has_callback(&self) -> bool { self.0.enabled.load(Ordering::Acquire) }

    /// Return the old callback so its foreign context can be released outside
    /// the caller's coordinator lease as well as outside this lock.
    pub fn replace_callback(&self, callback: Option<WakeCallback>) -> Option<WakeCallback> {
        let enabled = callback.is_some();
        self.0.enabled.store(enabled, Ordering::Release);
        let previous = std::mem::replace(&mut *self.0.callback.lock().unwrap_or_else(|e| e.into_inner()), callback);
        if !enabled && self.0.subscribed.swap(false, Ordering::AcqRel) {
            let key = Arc::downgrade(&self.0);
            let mut subscribers = subscribers().lock().unwrap_or_else(|e| e.into_inner());
            subscribers.retain(|entry| !entry.ptr_eq(&key) && entry.strong_count() != 0);
            let len = subscribers.len();
            if len <= subscribers.capacity() / 4 { subscribers.shrink_to(len.max(16)); }
        }
        previous
    }

    pub fn subscribe_registry_changes(&self) {
        if self.0.subscribed.swap(true, Ordering::AcqRel) { return; }
        let mut subscribers = subscribers().lock().unwrap_or_else(|e| e.into_inner());
        subscribers.retain(|entry| entry.strong_count() != 0);
        subscribers.push(Arc::downgrade(&self.0));
    }
}

fn subscribers() -> &'static Mutex<Vec<Weak<State>>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<Weak<State>>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Default::default)
}

/// Registry publishers call this only after releasing the registry lock.
pub(crate) fn registry_changed() {
    let receivers = {
        let mut subscribers = subscribers().lock().unwrap_or_else(|e| e.into_inner());
        subscribers.retain(|entry| entry.strong_count() != 0);
        subscribers.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    };
    for receiver in receivers { WorkWake(receiver).signal(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn completions_coalesce_until_collection_begins() {
        let wake = WorkWake::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        wake.replace_callback(Some(Arc::new(move || { count.fetch_add(1, Ordering::Relaxed); })));
        wake.signal();
        wake.signal();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        wake.acknowledge();
        wake.signal();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(wake.is_pending());
    }

    #[test]
    fn callback_can_reenter_notification_state_without_a_lock() {
        let wake = WorkWake::default();
        let nested = wake.clone();
        wake.replace_callback(Some(Arc::new(move || {
            nested.acknowledge();
            nested.replace_callback(None);
        })));
        wake.signal();
        assert!(!wake.is_pending());
    }

    #[test]
    fn unregister_retains_an_in_flight_callback_until_it_returns() {
        use std::{sync::mpsc, time::Duration};
        struct Context(Arc<AtomicUsize>);
        impl Drop for Context {
            fn drop(&mut self) { self.0.fetch_add(1, Ordering::SeqCst); }
        }
        let released = Arc::new(AtomicUsize::new(0));
        let context = Context(released.clone());
        let (started, entering) = mpsc::channel();
        let (finish, finishing) = mpsc::channel();
        let finishing = Mutex::new(finishing);
        let wake = WorkWake::default();
        wake.replace_callback(Some(Arc::new(move || {
            let _context = &context;
            started.send(()).unwrap();
            finishing.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        })));
        let worker_wake = wake.clone();
        let worker = std::thread::spawn(move || worker_wake.signal());
        entering.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(wake.replace_callback(None));
        assert_eq!(released.load(Ordering::SeqCst), 0);
        finish.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
