use super::*;
use std::{collections::HashSet, sync::{atomic::AtomicUsize, mpsc}};

struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn new() -> Self { Self(Arc::new((Mutex::new(false), Condvar::new()))) }
    fn release(&self) {
        let (lock, changed) = &*self.0;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}
impl Drop for Gate {
    fn drop(&mut self) { self.release(); }
}
struct Blocked {
    started: mpsc::Sender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}
impl SyntaxProvider for Blocked {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        self.started.send(()).unwrap();
        let (lock, changed) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released { released = changed.wait(released).unwrap(); }
        SyntaxResult::missing(request, "blocker finished")
    }
}
struct TwoSlices {
    index: usize,
    slices: Arc<AtomicUsize>,
    completed: mpsc::Sender<(usize, usize)>,
}
impl SyntaxProvider for TwoSlices {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        let slice = self.slices.fetch_add(1, Ordering::AcqRel);
        self.completed.send((self.index, slice)).unwrap();
        let mut result = SyntaxResult::missing(request, "fixture");
        result.continuation = slice == 0;
        if !result.continuation { result.coverage = Coverage::Exact; }
        result
    }
}

#[test]
fn saturated_admission_and_continuations_progress_without_owner_retries() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let gate = Gate::new();
    let (started, starting) = mpsc::channel();
    let mut blockers = Vec::new();
    for _ in 0..WORKER_COUNT {
        let started = started.clone();
        let blocked = gate.0.clone();
        let mut service = SyntaxService::with_factory(Arc::new(move || Box::new(Blocked {
            started: started.clone(), gate: blocked.clone(),
        })));
        service.request(tests::input(1), 0..100);
        blockers.push(service);
    }
    for _ in 0..WORKER_COUNT { starting.recv_timeout(Duration::from_secs(5)).unwrap(); }

    let count = MAX_QUEUE_BUFFERS + 17;
    let (completed, completions) = mpsc::channel();
    let mut services = Vec::new();
    let mut counters = Vec::new();
    for index in 0..count {
        let slices = Arc::new(AtomicUsize::new(0));
        counters.push(slices.clone());
        let completed = completed.clone();
        let mut service = SyntaxService::with_factory(Arc::new(move || Box::new(TwoSlices {
            index, slices: slices.clone(), completed: completed.clone(),
        })));
        // Repeated owner requests while admission is full must not multiply
        // scheduler entries or keep additional immutable snapshots alive.
        service.request(tests::input(2), 0..100);
        service.request(tests::input(2), 0..100);
        services.push(service);
    }
    {
        let schedule = pool().schedule.lock().unwrap();
        assert_eq!(schedule.queue.len(), MAX_QUEUE_BUFFERS);
        let ours = services.iter().map(|service| Arc::as_ptr(&service.mailbox)).collect::<HashSet<_>>();
        let registrations = schedule.queue.iter().chain(schedule.waiting.iter())
            .filter(|entry| ours.contains(&entry.mailbox.as_ptr())).collect::<Vec<_>>();
        assert_eq!(registrations.len(), count, "one registration per pending service");
        assert_eq!(registrations.iter().map(|entry| entry.mailbox.as_ptr()).collect::<HashSet<_>>().len(), count);
        assert!(schedule.waiting.len() >= count - MAX_QUEUE_BUFFERS);
    }
    gate.release();
    let mut first_slices = HashSet::new();
    let mut terminal_slices = HashSet::new();
    for _ in 0..2 * count {
        let (index, slice) = completions.recv_timeout(Duration::from_secs(5)).unwrap();
        if slice == 0 { assert!(first_slices.insert(index)); }
        else { assert_eq!(slice, 1); assert!(terminal_slices.insert(index)); }
    }
    assert_eq!(first_slices.len(), count);
    assert_eq!(terminal_slices.len(), count);
    assert!(counters.iter().all(|count| count.load(Ordering::Acquire) == 2));
    // No request or poll was needed after freeing worker capacity. Completion
    // may still be crossing the publication boundary; wait on its own condvar.
    for service in &services {
        let slot = service.mailbox.lock().unwrap();
        let progress = slot.progress.clone();
        let (slot, timeout) = progress.wait_timeout_while(slot, Duration::from_secs(5), |slot| slot.running || slot.pending.is_some()).unwrap();
        assert!(!timeout.timed_out());
        assert!(slot.ready.as_ref().is_some_and(|result| !result.continuation));
    }
}

#[test]
fn waiting_admission_is_fifo_and_close_removes_its_registration() {
    let local = Pool::default();
    let services = (0..MAX_QUEUE_BUFFERS * 5 + 3).map(|_| SyntaxService::default()).collect::<Vec<_>>();
    for service in &services {
        let closed = service.mailbox.lock().unwrap().closed.clone();
        local.enqueue(&service.mailbox, closed);
    }
    let (first, retire) = local.take_work();
    assert!(!retire);
    assert!(Arc::ptr_eq(&first.unwrap(), &services[0].mailbox));
    // A continuation joins behind every older waiting buffer.
    let closed = services[0].mailbox.lock().unwrap().closed.clone();
    local.enqueue(&services[0].mailbox, closed);
    let removed = MAX_QUEUE_BUFFERS + 1;
    services[removed].mailbox.lock().unwrap().closed.store(true, Ordering::Release);
    local.close(&services[removed].mailbox);
    for (index, service) in services.iter().enumerate().skip(1) {
        if index == removed { continue; }
        let (next, _) = local.take_work();
        assert!(Arc::ptr_eq(&next.unwrap(), &service.mailbox));
    }
    let (last, _) = local.take_work();
    assert!(Arc::ptr_eq(&last.unwrap(), &services[0].mailbox));
    let schedule = local.schedule.lock().unwrap();
    assert!(schedule.queue.is_empty() && schedule.waiting.is_empty());
    assert!(schedule.waiting.capacity() <= MAX_QUEUE_BUFFERS, "completed bursts release overflow storage");
}

#[test]
fn close_retirement_is_durable_before_a_worker_starts_waiting() {
    let local = Pool::default();
    let service = SyntaxService::default();
    service.mailbox.lock().unwrap().closed.store(true, Ordering::Release);
    local.close(&service.mailbox);
    // Consuming the predicate after notify already happened must not wait for
    // another event, even with a live strong mailbox reference.
    let (work, retire) = local.take_work();
    assert!(work.is_none() && retire);
    assert!(!local.schedule.lock().unwrap().retirement_requested);
}

#[test]
fn completion_wakes_the_owner_after_releasing_every_worker_lock() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    struct Terminal;
    impl SyntaxProvider for Terminal {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            SyntaxResult::missing(request, "terminal diagnostic")
        }
    }
    let mut service = SyntaxService::with_factory(Arc::new(|| Box::new(Terminal)));
    let wake = crate::background::WorkWake::default();
    let mailbox = Arc::downgrade(&service.mailbox);
    let (sent, received) = mpsc::channel();
    wake.replace_callback(Some(Arc::new(move || {
        let mailbox = mailbox.upgrade().unwrap();
        // Other tests may briefly use the shared pool; acquire each mutex
        // separately instead of treating incidental contention as a failure.
        drop(mailbox.lock().unwrap());
        drop(pool().schedule.lock().unwrap());
        drop(pool().idle.lock().unwrap());
        sent.send(()).unwrap();
    })));
    service.set_work_wake(wake.clone());
    assert!(!service.has_ready_work());
    service.request(tests::input(1), 0..100);
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(service.has_ready_work());
    wake.acknowledge();
    assert!(service.poll(tests::input(1).identity()));
    assert!(!service.has_ready_work());
    assert_eq!(service.diagnostics(), "terminal diagnostic");
    assert!(!wake.is_pending());
    wake.replace_callback(None);
}

fn superseded_active_request_is_requeued_without_owner_retries(explicit_cancel: bool) {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let gate = Gate::new();
    let (started, starting) = mpsc::channel();
    let mut service = SyntaxService::with_factory({
        let gate = gate.0.clone();
        Arc::new(move || Box::new(Blocked { started: started.clone(), gate: gate.clone() }))
    });
    let wake = crate::background::WorkWake::default();
    let (completed, completions) = mpsc::channel();
    wake.replace_callback(Some(Arc::new(move || { let _ = completed.send(()); })));
    service.set_work_wake(wake.clone());
    let input = tests::input(1);
    service.request(input.clone(), 0..100);
    starting.recv_timeout(Duration::from_secs(5)).unwrap();

    if explicit_cancel {
        service.cancel();
    } else {
        service.request(tests::input(2), 0..100);
    }
    service.request(input.clone(), 0..100);
    gate.release();

    // Returning to the canceled input must queue replacement work while the
    // first provider is still running. No owner poll or second request may
    // be needed to restart it after the canceled result is discarded.
    starting.recv_timeout(Duration::from_secs(5)).expect("replacement request starts");
    completions.recv_timeout(Duration::from_secs(5)).expect("replacement completion wakes its owner");
    assert!(service.poll(input.identity()));
    assert!(service.covers(&input, 0..100));
    assert!(!service.has_pending_work());
    wake.replace_callback(None);
}

#[test]
fn canceled_active_request_can_restart_before_the_worker_exits() {
    superseded_active_request_is_requeued_without_owner_retries(true);
}

#[test]
fn returning_to_an_active_input_after_supersession_requeues_it() {
    superseded_active_request_is_requeued_without_owner_retries(false);
}
