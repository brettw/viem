use super::*;
use crate::document::syntax::{SyntaxInputIdentity, SyntaxStyleName};
use crate::document::{FormattedTextTree, Splice};
use std::sync::Arc;

fn input(revision: u64, text: impl Into<Arc<str>>) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 73,
            revision,
            generation: 1,
        },
        FormattedTextTree::try_from_text(text).unwrap(),
    )
}

fn run(range: Range<usize>, name: &str) -> SyntaxRun {
    SyntaxRun {
        range,
        name: SyntaxStyleName(name.into()),
        origin: "retention test".into(),
        priority: 0,
    }
}

fn result(
    service: &SyntaxService,
    input: &SyntaxInputSnapshot,
    range: Range<usize>,
    runs: Vec<SyntaxRun>,
    coverage: Coverage,
) -> SyntaxResult {
    SyntaxResult {
        input: input.identity(),
        configuration: service.configuration.clone(),
        range,
        runs,
        coverage,
        diagnostics: Vec::new(),
        continuation: false,
    }
}

fn publish(
    service: &mut SyntaxService,
    result: SyntaxResult,
    current: &SyntaxInputSnapshot,
) -> bool {
    service.mailbox.lock().unwrap().ready = Some(result);
    service.poll(current.identity())
}

fn seeded(input: &SyntaxInputSnapshot, runs: Vec<SyntaxRun>) -> SyntaxService {
    let mut service =
        SyntaxService::with_factory(Arc::new(|| panic!("This test injects results explicitly")));
    service.rebase_input(input.clone(), None);
    let result = result(&service, input, 0..input.byte_len(), runs, Coverage::Exact);
    assert!(publish(&mut service, result, input));
    service
}

fn map(old: &SyntaxInputSnapshot, next: &SyntaxInputSnapshot, splices: Vec<Splice>) -> PositionMap {
    PositionMap::for_text_snapshots(
        DocumentId(73),
        Revision(old.identity().revision),
        Revision(next.identity().revision),
        old.text_tree(),
        next.text_tree(),
        splices,
    )
    .unwrap()
}

#[test]
fn exact_maps_retain_surviving_text_across_insertions_deletions_and_disjoint_edits() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "red plain blue");
    let mut service = seeded(&old, vec![run(0..3, "Comment"), run(10..14, "Keyword")]);
    let next = input(2, "reXXd plain blZue");
    let edits = map(
        &old,
        &next,
        vec![
            Splice::new(2..2, 2).unwrap(),
            Splice::new(12..12, 1).unwrap(),
        ],
    );
    service.rebase_input(next.clone(), Some(&edits));
    assert!(
        service.cache.is_empty(),
        "Retained appearance must never satisfy a fresh syntax request"
    );
    assert_eq!(
        service.runs(next.identity()),
        vec![
            run(0..2, "Comment"),
            run(4..5, "Comment"),
            run(12..14, "Keyword"),
            run(15..17, "Keyword")
        ]
    );
    let final_input = input(3, "red plain bue");
    let edits = map(
        &next,
        &final_input,
        vec![
            Splice::new(2..4, 0).unwrap(),
            Splice::new(13..15, 0).unwrap(),
        ],
    );
    service.rebase_input(final_input.clone(), Some(&edits));
    assert_eq!(
        service.runs(final_input.identity()),
        vec![
            run(0..2, "Comment"),
            run(2..3, "Comment"),
            run(10..11, "Keyword"),
            run(11..13, "Keyword")
        ]
    );
    assert!(service.runs(old.identity()).is_empty());
}

#[test]
fn missing_work_keeps_mapped_colors_and_ready_empty_coverage_clears_only_its_region() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "red plain blue");
    let mut service = seeded(&old, vec![run(0..3, "Comment"), run(10..14, "Keyword")]);
    let next = input(2, "red plain blue");
    service.rebase_input(next.clone(), Some(&map(&old, &next, Vec::new())));
    let retained = service.runs(next.identity());
    let mut pending = result(&service, &next, 0..14, Vec::new(), Coverage::Missing);
    pending.continuation = true;
    assert!(!publish(&mut service, pending.clone(), &next));
    assert_eq!(service.runs(next.identity()), retained);
    pending.continuation = false;
    assert!(publish(&mut service, pending, &next));
    assert_eq!(service.runs(next.identity()), retained);
    let empty = result(&service, &next, 0..5, Vec::new(), Coverage::Exact);
    assert!(publish(&mut service, empty, &next));
    assert_eq!(service.runs(next.identity()), vec![run(10..14, "Keyword")]);
    let replacement = result(
        &service,
        &next,
        10..14,
        vec![run(10..14, "String")],
        Coverage::Provisional,
    );
    assert!(publish(&mut service, replacement, &next));
    assert_eq!(service.runs(next.identity()), vec![run(10..14, "String")]);
    let empty = result(&service, &next, 0..14, Vec::new(), Coverage::Exact);
    assert!(publish(&mut service, empty, &next));
    assert!(service.runs(next.identity()).is_empty());
}

#[test]
fn overlapping_queries_replace_only_their_coverage_and_stale_packages_cannot_clear_it() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let input = input(1, "abcdefghijklmn");
    let mut service = seeded(&input, vec![run(0..14, "Comment")]);
    let replacement = result(
        &service,
        &input,
        6..8,
        vec![run(6..8, "String")],
        Coverage::Exact,
    );
    assert!(publish(&mut service, replacement, &input));
    let expected = vec![
        run(0..6, "Comment"),
        run(6..8, "String"),
        run(8..14, "Comment"),
    ];
    assert_eq!(service.runs(input.identity()), expected);
    let mut stale = result(&service, &input, 0..14, Vec::new(), Coverage::Exact);
    stale.input.revision = 0;
    assert!(!publish(&mut service, stale, &input));
    let mut stale = result(&service, &input, 0..14, Vec::new(), Coverage::Exact);
    stale.configuration.generation += 1;
    assert!(!publish(&mut service, stale, &input));
    assert_eq!(service.runs(input.identity()), expected);
    assert_eq!(service.statistics.stale_rejections, 2);
    service.set_language(Some("different".into()));
    assert!(
        service.runs(input.identity()).is_empty(),
        "A retired language's provisional colors cannot leak"
    );
}

#[test]
fn retention_requires_exact_document_revision_generation_and_unicode_boundaries() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "a\u{301} beta");
    let mut service = seeded(
        &old,
        vec![
            run(0..1, "Comment"),
            run(1..3, "String"),
            run(4..8, "Keyword"),
        ],
    );
    let next = input(2, "a\u{301} beta!");
    service.rebase_input(
        next.clone(),
        Some(&map(&old, &next, vec![Splice::new(8..8, 1).unwrap()])),
    );
    assert_eq!(
        service.runs(next.identity()),
        vec![run(0..3, "Comment"), run(4..8, "Keyword")],
        "Retain the first scalar's displayed style for the whole grapheme"
    );
    let unrelated = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 74,
            revision: 3,
            generation: 1,
        },
        next.text_tree().clone(),
    );
    service.rebase_input(
        unrelated.clone(),
        Some(&PositionMap::identity(
            DocumentId(73),
            PositionDomain::FormattedText,
            Revision(2),
            9,
        )),
    );
    assert!(service.runs(unrelated.identity()).is_empty());
}

#[test]
fn large_rope_retention_stays_bounded_to_cached_runs() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "x\n".repeat(16 * 1024 * 1024));
    let middle = old.byte_len() / 2;
    let mut service = seeded(
        &old,
        vec![
            run(0..1, "Comment"),
            run(middle..middle + 1, "String"),
            run(old.byte_len() - 2..old.byte_len() - 1, "Keyword"),
        ],
    );
    let next = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..old.identity()
        },
        old.text_tree().splice(middle..middle, "xx").unwrap(),
    );
    let edits = map(&old, &next, vec![Splice::new(middle..middle, 2).unwrap()]);
    service.rebase_input(next.clone(), Some(&edits));
    assert_eq!(service.runs(next.identity()).len(), 3);
    assert_eq!(
        service.runs(next.identity())[1].range,
        middle + 2..middle + 3
    );
    assert_eq!(
        service.runs(next.identity())[2].range,
        old.byte_len()..old.byte_len() + 1
    );
    assert!(service.retained_result_bytes() < 1024);
    assert_eq!(service.statistics.requests, 0);
    assert!(service.cache.is_empty());
}

#[test]
fn current_results_and_retained_colors_share_one_run_memory_budget() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "abcdefgh");
    let runs = [0..1, 2..3, 4..5]
        .into_iter()
        .map(|range| {
            let mut run = run(range, "Comment");
            run.origin = "x".repeat(MAX_CACHED_RUN_BYTES / 4);
            run
        })
        .collect();
    let mut service = seeded(&old, runs);
    let next = input(2, "abcdefgh");
    service.rebase_input(next.clone(), Some(&map(&old, &next, Vec::new())));
    assert_eq!(service.retained.len(), 3);
    let mut replacement = run(6..7, "String");
    replacement.origin = "y".repeat(MAX_CACHED_RUN_BYTES / 2);
    let ready = result(&service, &next, 6..8, vec![replacement], Coverage::Exact);
    assert!(publish(&mut service, ready, &next));
    assert_eq!(
        service.retained.len(),
        1,
        "Old presentation is evicted before newly completed coverage"
    );
    assert_eq!(service.cache.len(), 1);
    assert!(service.retained_result_bytes() <= MAX_CACHED_RUN_BYTES);
}
