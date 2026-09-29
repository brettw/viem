use super::*;
use crate::document::syntax::{SyntaxInputIdentity, SyntaxRun, SyntaxStyleName};
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
    service.rebase_input(input.clone(), None, None);
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
    service.rebase_input(next.clone(), Some(&edits), None);
    assert!(
        service.cache.is_empty(),
        "Retained appearance must never satisfy a fresh syntax request"
    );
    assert_eq!(
        service.runs(next.identity()),
        vec![
            run(0..5, "Comment"),
            run(12..17, "Keyword")
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
    service.rebase_input(final_input.clone(), Some(&edits), None);
    assert_eq!(
        service.runs(final_input.identity()),
        vec![
            run(0..3, "Comment"),
            run(10..13, "Keyword")
        ]
    );
    assert!(service.runs(old.identity()).is_empty());
}

#[test]
fn inserted_and_replacement_text_inherits_only_the_preceding_style() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    for local in [false, true] {
        for (replaced, text, expected) in [
            (0..0, "Xab cd", vec![run(1..3, "Comment"), run(4..6, "Keyword")]),
            (1..1, "aXb cd", vec![run(0..3, "Comment"), run(4..6, "Keyword")]),
            (2..2, "abX cd", vec![run(0..3, "Comment"), run(4..6, "Keyword")]),
            (3..3, "ab Xcd", vec![run(0..2, "Comment"), run(4..6, "Keyword")]),
            (5..5, "ab cdX", vec![run(0..2, "Comment"), run(3..6, "Keyword")]),
            (1..4, "aXd", vec![run(0..2, "Comment"), run(2..3, "Keyword")]),
            (1..5, "aX", vec![run(0..2, "Comment")]),
            (0..5, "X", vec![]),
            (1..4, "ad", vec![run(0..1, "Comment"), run(1..2, "Keyword")]),
        ] {
            let old = input(1, "ab cd");
            let next = input(2, text);
            let inserted = next.byte_len() + replaced.len() - old.byte_len();
            let edits = map(&old, &next, vec![Splice::new(replaced.clone(), inserted).unwrap()]);
            let mut service = seeded(&old, vec![run(0..2, "Comment"), run(3..5, "Keyword")]);
            service.take_publication_delta();
            service.rebase_input(next.clone(), Some(&edits),
                local.then_some((replaced.clone(), replaced.start..replaced.start + inserted)));
            assert_eq!(service.runs(next.identity()), expected, "{text:?}, local={local}");
            assert!(service.cache.is_empty());
            assert_eq!(service.runs.bytes(), RunStore::new(service.runs.runs()).bytes());
            let delta = service.take_publication_delta();
            assert_eq!(delta.unbounded, !local);
            assert_eq!(service.statistics.requests, 0, "retention does not run a provider");
        }
    }
}

#[test]
fn unicode_predecessor_and_repeated_pending_typing_keep_one_style_until_replaced() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let mut current = input(1, "a\u{301}");
    let mut service = seeded(&current, vec![run(0..1, "Comment"), run(1..3, "String")]);
    for inserted in ["👩‍💻", "\n", "é", " tail"] {
        let end = current.byte_len();
        let next = SyntaxInputSnapshot::new(
            SyntaxInputIdentity { revision: current.identity().revision + 1, ..current.identity() },
            current.text_tree().splice(end..end, inserted).unwrap(),
        );
        let edits = map(&current, &next, vec![Splice::new(end..end, inserted.len()).unwrap()]);
        service.take_publication_delta();
        service.rebase_input(next.clone(), Some(&edits), Some((end..end, end..next.byte_len())));
        assert_eq!(service.runs(next.identity()), vec![run(0..next.byte_len(), "Comment")]);
        current = next;
    }
    let pending = result(&service, &current, 0..current.byte_len(), Vec::new(), Coverage::Missing);
    publish(&mut service, pending, &current);
    assert_eq!(service.runs(current.identity()), vec![run(0..current.byte_len(), "Comment")]);
    let ready = result(&service, &current, 0..current.byte_len(), vec![run(0..3, "String")], Coverage::Exact);
    assert!(publish(&mut service, ready, &current));
    assert_eq!(service.runs(current.identity()), vec![run(0..3, "String")]);
}

#[test]
fn missing_work_keeps_mapped_colors_and_ready_empty_coverage_clears_only_its_region() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let old = input(1, "red plain blue");
    let mut service = seeded(&old, vec![run(0..3, "Comment"), run(10..14, "Keyword")]);
    let next = input(2, "red plain blue");
    service.rebase_input(next.clone(), Some(&map(&old, &next, Vec::new())), None);
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
        None,
    );
    assert_eq!(
        service.runs(next.identity()),
        vec![run(0..3, "Comment"), run(4..9, "Keyword")],
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
        None,
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
    service.rebase_input(next.clone(), Some(&edits), None);
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
            run.origin = "x".repeat(MAX_CACHED_RUN_BYTES / 4).into();
            run
        })
        .collect();
    let mut service = seeded(&old, runs);
    let next = input(2, "abcdefgh");
    service.rebase_input(next.clone(), Some(&map(&old, &next, Vec::new())), None);
    assert_eq!(service.runs.len(), 3);
    let mut replacement = run(6..7, "String");
    replacement.origin = "y".repeat(MAX_CACHED_RUN_BYTES / 2).into();
    let ready = result(&service, &next, 6..8, vec![replacement], Coverage::Exact);
    assert!(publish(&mut service, ready, &next));
    assert_eq!(
        service.runs.runs().iter().map(|run| run.name.as_str()).collect::<Vec<_>>(),
        ["Comment", "String"],
        "Old presentation is evicted before newly completed coverage"
    );
    assert_eq!(service.cache.len(), 1);
    assert!(service.retained_result_bytes() <= MAX_CACHED_RUN_BYTES);
}

#[test]
fn local_publication_uses_displayed_grapheme_edges_without_dropping_or_duplicating_neighbors() {
    use crate::document::{code_style, Document, Encoding, Format, StyleApplication};
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    for (source, captures, insertion, expected) in [
        (
            "a\u{301}bc",
            vec![run(0..1, "Keyword"), run(1..5, "String")],
            4,
            vec![(0..3, "syntax:Keyword"), (3..6, "syntax:String")],
        ),
        (
            "ab\u{301}c",
            vec![run(0..2, "Keyword"), run(2..5, "String")],
            1,
            vec![(0..5, "syntax:Keyword"), (5..6, "syntax:String")],
        ),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let sheet = Arc::new(code_style::default_sheet());
        document.install_code_presentation(sheet.clone(), &captures);
        let snapshot = |document: &Document| SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: document.id().0,
                revision: document.revision().0,
                generation: 1,
            },
            document.projection().text_tree().clone(),
        );
        let old = snapshot(&document);
        let mut service = seeded(&old, captures);
        service.take_publication_delta();
        document.replace(insertion..insertion, "x").unwrap();
        let next = snapshot(&document);
        let map = document.code_presentation_change_map().unwrap().clone();
        service.rebase_input(next.clone(), Some(&map),
            Some((insertion..insertion, insertion..insertion + 1)));
        let delta = service.take_publication_delta();
        assert!(!delta.unbounded);
        assert!(document.install_code_presentation_delta(sheet.clone(), service.run_store(), &delta),
            "a local grapheme edge must not fall back to a full presentation rebuild");

        let spans = |document: &Document| document.projection().style_spans().iter()
            .filter_map(|span| match &span.application {
                StyleApplication::Automatic(id) => Some((span.range.clone(), id.0.clone())),
                _ => None,
            }).collect::<Vec<_>>();
        let expected = expected.into_iter().map(|(range, name)| (range, name.to_string())).collect::<Vec<_>>();
        assert_eq!(spans(&document), expected,
            "source={source:?}, insertion={insertion}: preserve the neighboring grapheme owner exactly once");
        let mut fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Code).unwrap();
        fresh.install_code_presentation(sheet, &service.runs(next.identity()));
        assert_eq!(spans(&document), spans(&fresh), "local and fresh syntax projections must agree");
    }
}
