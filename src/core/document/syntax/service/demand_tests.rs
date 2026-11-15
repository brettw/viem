use super::*;
use crate::document::syntax::SyntaxStyleName;

fn accept(service: &mut SyntaxService, input: &SyntaxInputSnapshot, range: Range<usize>, coverage: Coverage) {
    service.mailbox.lock().unwrap().ready = Some(SyntaxResult {
        input: input.identity(), configuration: service.configuration.clone(),
        range: range.clone(),
        runs: if coverage == Coverage::Missing { Vec::new() } else {
            vec![SyntaxRun { range: range.start..range.start + 1, name: SyntaxStyleName("Token".into()), origin: "fixture".into(), priority: 0 }]
        },
        coverage, diagnostics: Vec::new(), continuation: false,
    });
    assert!(service.poll(input.identity()));
}

fn settled_demands_survive_coverage_eviction(coverage: Coverage) {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let input = tests::input(1);
    let ranges = (0..MAX_CACHED_REGIONS + 1).map(|index| index * 100..index * 100 + 10).collect::<Vec<_>>();
    let mut service = SyntaxService::default();
    service.set_language(Some("rust".into()));
    service.rebase_input(input.clone(), None, None);
    service.set_visible_demands(&input, &ranges);
    // Registry initialization can reset the current input, as a real package
    // replacement does; owner request capture then establishes this snapshot.
    service.rebase_input(input.clone(), None, None);
    for range in &ranges { accept(&mut service, &input, range.clone(), coverage); }
    assert_eq!(service.cache.len(), MAX_CACHED_REGIONS);
    assert!(!service.has_cached_coverage(input.identity(), &ranges[0]));
    assert!(service.covers(&input, ranges[0].clone()));
    if coverage != Coverage::Missing {
        assert_eq!(service.runs(input.identity()).len(), ranges.len(), "metadata eviction keeps independently budgeted runs");
    } else {
        assert!(!service.covers(&input, 0..5), "a terminal failure settles only its exact demand");
    }

    let requests = service.statistics.requests;
    service.set_visible_demands(&input, &ranges);
    for range in &ranges {
        service.request(input.clone(), range.clone());
        assert!(service.prepare_wait(input.clone(), range.clone(), &mut SyntaxWaitState::default(),
            Instant::now() + SYNTAX_PRESENTATION_WAIT).is_none());
    }
    assert_eq!(service.statistics.requests, requests, "unchanged views and grace paints stay settled");
    assert!(!service.has_pending_work());
    service.set_visible_demands(&input, &ranges[1..]);
    assert_eq!(service.visible_demands.as_ref().unwrap().ranges.len(), MAX_CACHED_REGIONS);
    assert!(!service.covers(&input, ranges[0].clone()), "removed demands do not retain a history of old viewports");
}

#[test]
fn more_visible_demands_than_cache_entries_settle_including_grace_waits() {
    settled_demands_survive_coverage_eviction(Coverage::Exact);
}

#[test]
fn terminal_visible_demands_survive_eviction_without_covering_smaller_requests() {
    settled_demands_survive_coverage_eviction(Coverage::Missing);
}

#[test]
fn visible_demand_completion_is_bound_to_input_and_configuration() {
    let _registry = super::super::treesitter::package_registry_test_guard();
    let mut service = SyntaxService::default();
    let old = tests::input(1);
    let new = tests::input(2);
    service.set_visible_demands(&old, &[0..10]);
    service.rebase_input(old.clone(), None, None);
    accept(&mut service, &old, 0..10, Coverage::Exact);
    assert!(service.covers(&old, 0..10));
    let stale = SyntaxResult::missing(&SyntaxRequest {
        input: old.clone(), configuration: service.configuration.clone(), range: 0..10,
    }, "old result");
    service.set_visible_demands(&new, &[0..10]);
    service.rebase_input(new.clone(), None, None);
    service.mailbox.lock().unwrap().ready = Some(stale);
    assert!(!service.poll(new.identity()));
    assert!(!service.covers(&new, 0..10));
    accept(&mut service, &new, 0..10, Coverage::Exact);
    service.set_language(Some("rust".into()));
    service.set_visible_demands(&new, &[0..10]);
    assert!(!service.covers(&new, 0..10));
    assert!(!service.visible_demands.as_ref().unwrap().ranges[0].satisfied);
}

#[test]
fn empty_visible_demands_still_refresh_registry_configuration() {
    let mut service = SyntaxService::default();
    let generation = service.configuration.generation;
    service.set_visible_demands(&tests::input(1), &[]);
    assert_ne!(service.configuration.generation, generation);
    assert_eq!(service.registry_generation, super::super::treesitter::package_registry_generation());
}
