use super::*;

// Install a worker's active request directly so timing tests do not depend on
// contention in the shared pool. Publication uses the same mailbox protocol.
fn active() -> (SyntaxService, SyntaxRequest) {
    let mut service = SyntaxService::with_factory(Arc::new(|| panic!("no worker needed")));
    service.registry_generation = super::super::treesitter::package_registry_generation();
    service.configuration.language = Some("rust".into());
    let input = super::tests::input(1);
    service.rebase_input(input.clone(), None, None);
    let request = SyntaxRequest {
        input,
        configuration: service.configuration.clone(),
        range: 0..100,
    };
    {
        let mut slot = service.mailbox.lock().unwrap();
        slot.running = true;
        slot.active = Some(request.clone());
    }
    (service, request)
}

#[test]
fn visible_wait_wakes_on_publication_and_does_not_hold_mailbox_lock() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    let mut state = SyntaxWaitState::default();
    let wait = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    let thread =
        std::thread::spawn(move || wait.wait_until(Instant::now() + Duration::from_secs(2)));
    // Acquiring and publishing while the waiter runs would deadlock if the
    // wait kept its mailbox lock. Empty exact coverage is still ready.
    let mut result = SyntaxResult::missing(&request, "");
    result.coverage = Coverage::Exact;
    {
        let mut slot = service.mailbox.lock().unwrap();
        slot.ready = Some(result);
        slot.progress.notify_all();
    }
    assert!(thread.join().unwrap());
    assert!(service.poll(request.input.identity()));
    assert!(service
        .prepare_wait(
            request.input.clone(),
            request.range,
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT
        )
        .is_none());
    assert_eq!(
        service.statistics.requests, 0,
        "ready coverage must not restart analysis"
    );
}

#[test]
fn completion_between_poll_and_request_does_not_restart_region_analysis() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    {
        let mut slot = service.mailbox.lock().unwrap();
        slot.running = false;
        slot.active = None;
        let mut result = SyntaxResult::missing(&request, "ready");
        result.coverage = Coverage::Exact;
        slot.ready = Some(result);
    }
    service.request(request.input.clone(), request.range.clone());
    assert_eq!(service.statistics.requests, 0);
    assert!(service.mailbox.lock().unwrap().pending.is_none());
    assert!(service.poll(request.input.identity()));
}

#[test]
fn visible_wait_timeout_and_repaints_share_one_deadline() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    let mut state = SyntaxWaitState::default();
    let start = Instant::now();
    let deadline = start + Duration::from_millis(10);
    let wait = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            deadline,
        )
        .unwrap();
    let next = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    assert_eq!(wait.deadline, next.deadline);
    assert!(wait.same_target(&next));
    assert_eq!(wait.deadline, deadline);
    assert!(!wait.wait_until(start + SYNTAX_PRESENTATION_WAIT));
    assert!(start.elapsed() >= Duration::from_millis(10));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "provider cannot hold presentation indefinitely"
    );
    assert!(service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT
        )
        .is_none());
    // A genuinely new viewport has its own bounded opportunity to finish.
    let next_range = 100..200;
    service
        .mailbox
        .lock()
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .range = next_range.clone();
    assert!(service
        .prepare_wait(
            request.input,
            next_range,
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT
        )
        .is_some());
}

#[test]
fn alternating_views_cannot_reset_each_others_expired_grace_period() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    let mut first = SyntaxWaitState::default();
    let mut second = SyntaxWaitState::default();
    let second_range = 100..200;
    service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut first,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    first.target.as_mut().unwrap().3 = Instant::now() - Duration::from_millis(1);
    service
        .mailbox
        .lock()
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .range = second_range.clone();
    service
        .prepare_wait(
            request.input.clone(),
            second_range.clone(),
            &mut second,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    second.target.as_mut().unwrap().3 = Instant::now() - Duration::from_millis(1);
    for _ in 0..3 {
        service
            .mailbox
            .lock()
            .unwrap()
            .active
            .as_mut()
            .unwrap()
            .range = request.range.clone();
        assert!(service
            .prepare_wait(
                request.input.clone(),
                request.range.clone(),
                &mut first,
                Instant::now() + SYNTAX_PRESENTATION_WAIT
            )
            .is_none());
        service
            .mailbox
            .lock()
            .unwrap()
            .active
            .as_mut()
            .unwrap()
            .range = second_range.clone();
        assert!(service
            .prepare_wait(
                request.input.clone(),
                second_range.clone(),
                &mut second,
                Instant::now() + SYNTAX_PRESENTATION_WAIT
            )
            .is_none());
    }
}

#[test]
fn continuation_keeps_deadline_and_terminal_failure_stops_waiting() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    let mut state = SyntaxWaitState::default();
    let wait = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    let mut result = SyntaxResult::missing(&request, "continuing");
    result.continuation = true;
    service.mailbox.lock().unwrap().ready = Some(result);
    assert!(wait.wait_until(wait.deadline));
    assert!(!service.poll(request.input.identity()));
    let next = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    assert_eq!(wait.deadline, next.deadline);
    service.mailbox.lock().unwrap().ready = Some(SyntaxResult::missing(&request, "unavailable"));
    assert!(service.poll(request.input.identity()));
    assert!(service
        .prepare_wait(
            request.input,
            request.range,
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT
        )
        .is_none());
}

#[test]
fn cancellation_supersession_and_close_wake_detached_waiters() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    for action in 0..3 {
        let (mut service, request) = active();
        let mut state = SyntaxWaitState::default();
        let wait = service
            .prepare_wait(
                request.input.clone(),
                request.range.clone(),
                &mut state,
                Instant::now() + SYNTAX_PRESENTATION_WAIT,
            )
            .unwrap();
        let thread =
            std::thread::spawn(move || wait.wait_until(Instant::now() + Duration::from_secs(2)));
        match action {
            0 => service.cancel(),
            1 => {
                // Supersession replaces the mailbox's cancellation token but
                // the old active provider still owns the cancelled one.
                service.request(super::tests::input(2), request.range.clone());
                assert!(!service
                    .mailbox
                    .lock()
                    .unwrap()
                    .cancellation
                    .load(Ordering::Acquire));
            }
            _ => drop(service),
        }
        assert!(!thread.join().unwrap());
    }
}

#[test]
fn waiting_never_makes_stale_results_current() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let (mut service, request) = active();
    let mut state = SyntaxWaitState::default();
    let wait = service
        .prepare_wait(
            request.input.clone(),
            request.range.clone(),
            &mut state,
            Instant::now() + SYNTAX_PRESENTATION_WAIT,
        )
        .unwrap();
    let mut result = SyntaxResult::missing(&request, "old revision");
    result.coverage = Coverage::Exact;
    service.mailbox.lock().unwrap().ready = Some(result);
    let newer = super::tests::input(2);
    service.rebase_input(newer.clone(), None, None);
    assert!(wait.wait_until(wait.deadline));
    assert!(!service.poll(newer.identity()));
    assert_eq!(service.statistics.stale_rejections, 1);
    assert!(service.cache.is_empty());
    assert_eq!(state.target.as_ref().unwrap().0, request.input.identity());
}
