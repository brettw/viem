use super::*;
use crate::document::{Document, Encoding, FileFormat, Format, LineEndingOpenPolicy};

fn document(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap()
}

fn projection(document: &Document) -> Arc<FormattedDocument> {
    Arc::new(document.projection().clone())
}

fn complete(snapshot: &mut IndentSnapshot) {
    let count = snapshot.projection.presentation_line_count();
    snapshot
        .ensure(count, &LayoutRunControl::synchronous(&NeverCancelled))
        .unwrap();
}

fn verify_complete(snapshot: &mut IndentSnapshot) {
    complete(snapshot);
    for line in 0..snapshot.projection.presentation_line_count() {
        let expected = indentation(&snapshot.projection, line, snapshot.tabstop).1;
        assert_eq!(
            snapshot.index.line(line).unwrap().columns,
            expected,
            "line {line}"
        );
    }
}

#[test]
fn partial_prefix_survives_suffix_edits_and_truncates_straddling_edits() {
    let mut document = document("zero\n  one\n    two\n  three\nzero\n    five\n  six");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    snapshot
        .ensure(3, &LayoutRunControl::synchronous(&NeverCancelled))
        .unwrap();
    let root = snapshot.index.clone();
    let suffix = document
        .projection()
        .presentation_line_range(5)
        .unwrap()
        .start;
    document.insert(suffix, "      inserted\n").unwrap();
    let dirty = snapshot.synchronize(projection(&document), 4);
    assert_eq!(dirty.end, document.projection().presentation_line_count());
    assert_eq!(snapshot.index.len(), 3);
    assert!(snapshot.index.shares_root_with(&root));
    let start = document
        .projection()
        .presentation_line_range(2)
        .unwrap()
        .start;
    let end = document
        .projection()
        .presentation_line_range(5)
        .unwrap()
        .start;
    document.replace(start..end, " new\n      new2\n").unwrap();
    let dirty = snapshot.synchronize(projection(&document), 4);
    assert_eq!(snapshot.index.len(), 2);
    assert_eq!(dirty.start, 2);
    assert_eq!(dirty.end, document.projection().presentation_line_count());
    verify_complete(&mut snapshot);
}

#[test]
fn local_insert_delete_and_reindent_keep_fresh_index_results() {
    let mut document = document("zero\n  one\n    two\n  three\nzero\n    five\n  six");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    verify_complete(&mut snapshot);
    for replacement in ["\tinsert\n      second\n", "x", "", "  reindented\n"] {
        let range = document.projection().presentation_line_range(1).unwrap();
        document.replace(range, replacement).unwrap();
        let dirty = snapshot.synchronize(projection(&document), 4);
        assert!(dirty.start <= 1);
        verify_complete(&mut snapshot);
    }
    snapshot.synchronize(projection(&document), 8);
    assert!(snapshot.index.is_empty());
    verify_complete(&mut snapshot);
}

#[test]
fn large_insertion_defers_new_prefix_analysis_to_worker() {
    let mut document = document("root\n  before\n  after\nroot");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let at = document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start;
    document.insert(at, &"    pasted\n".repeat(1_000)).unwrap();
    let dirty = snapshot.synchronize(projection(&document), 4);
    assert_eq!(snapshot.index.len(), 1);
    assert_eq!(dirty, 1..document.projection().presentation_line_count());
    verify_complete(&mut snapshot);
}

#[test]
fn giant_prefix_edit_defers_after_a_bounded_foreground_scan() {
    let mut document = document(&format!(
        "root\n\t{}body\n  child",
        " ".repeat(INDENT_SYNC_MAX_BYTES * 4)
    ));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let at = document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start;
    document.insert(at, " ").unwrap();
    assert_eq!(snapshot.synchronize(projection(&document), 4), 1..3);
    assert_eq!(
        snapshot.index.len(),
        1,
        "oversize prefixes are deferred instead of scanned synchronously"
    );
    assert!(!snapshot.prefixes.contains_key(&1));
    let mut budget = INDENT_SYNC_MAX_BYTES;
    assert_eq!(
        scan_indentation(
            &snapshot.projection,
            1,
            4,
            &LayoutRunControl::synchronous(&NeverCancelled),
            Some(&mut budget)
        )
        .unwrap(),
        None
    );
    assert_eq!(
        budget, 0,
        "the deferred scanner consumes at most its explicit byte budget"
    );
    verify_complete(&mut snapshot);
}

#[test]
fn foreground_prefix_budget_is_shared_across_changed_lines() {
    let document = document("  first\n   second");
    let projection = IndentProjection::new(projection(&document));
    let mut budget = 5;
    let control = LayoutRunControl::synchronous(&NeverCancelled);
    assert_eq!(
        scan_indentation(&projection, 0, 4, &control, Some(&mut budget))
            .unwrap()
            .map(IndentPrefix::pair),
        Some((2, 2))
    );
    assert_eq!(budget, 2, "two spaces and one body byte were examined");
    assert_eq!(
        scan_indentation(&projection, 1, 4, &control, Some(&mut budget)).unwrap(),
        None
    );
    assert_eq!(budget, 0);
}

#[test]
fn large_deleted_hull_defers_before_walking_indentation_minima() {
    let mut document = document(&format!("root\n{}last", "  line\n".repeat(500)));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let start = document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start;
    let end = document
        .projection()
        .presentation_line_range(400)
        .unwrap()
        .start;
    document.replace(start..end, "  replacement\n").unwrap();
    assert_eq!(
        snapshot.synchronize(projection(&document), 4),
        1..document.projection().presentation_line_count()
    );
    assert_eq!(
        snapshot.index.len(),
        1,
        "the old hull is also subject to the foreground line budget"
    );
    verify_complete(&mut snapshot);
}

#[test]
fn broad_metric_ranges_fall_back_without_enumerating_all_indexed_lines() {
    let document = document(&format!(
        "{}root\n  final",
        "  line\n".repeat(INDENT_SYNC_MAX_LINES + 1)
    ));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let count = document.projection().presentation_line_count();
    assert_eq!(
        snapshot.dependent_end(&(0..INDENT_SYNC_MAX_LINES + 1)),
        count,
        "a broad query conservatively reaches EOF even when a later indexed barrier exists"
    );
    assert_eq!(snapshot.dependent_end(&(0..count)), count);
    assert_eq!(snapshot.dependent_end(&(0..0)), 0);
}

#[test]
fn unknown_dependency_barrier_reaches_eof_beyond_indexed_prefix() {
    let mut document = document("root\n  one\n    two\n      three\n    four\n  five\nzero");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    snapshot
        .ensure(3, &LayoutRunControl::synchronous(&NeverCancelled))
        .unwrap();
    let count = document.projection().presentation_line_count();
    assert_eq!(snapshot.dependent_end(&(1..2)), count);
    let at = document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start
        + 2;
    document.insert(at, "changed ").unwrap();
    assert_eq!(snapshot.synchronize(projection(&document), 4).end, count);
}

#[test]
fn zero_indent_body_edit_stops_at_next_zero_root() {
    let mut document = document("root\n  child\n    nested\nsecond\n  other\nlast");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    assert_eq!(snapshot.dependent_end(&(0..1)), 3);
    document.insert(1, "longer").unwrap();
    assert_eq!(snapshot.synchronize(projection(&document), 4), 0..3);
    verify_complete(&mut snapshot);
}

#[test]
fn proven_body_edit_stops_at_equal_indent_without_invalidating_its_descendants() {
    let mut document =
        document("root\n  changed body\n  unchanged body\n    nested\n  following\nlast");
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let at = document
        .projection()
        .presentation_line_range(1)
        .unwrap()
        .start
        + 4;
    document.insert(at, "longer").unwrap();
    assert_eq!(snapshot.synchronize(projection(&document), 4), 1..2);
    verify_complete(&mut snapshot);
}

#[test]
fn same_text_with_different_semantic_line_partition_discards_index() {
    let open = |text: &[u8]| {
        Document::from_bytes_with_line_ending_policy(
            text.to_vec(),
            Encoding::Utf8,
            Format::Code,
            LineEndingOpenPolicy::default().with_forced_file_format(FileFormat::Mac),
        )
        .unwrap()
    };
    let first = open(b"root\r  two\n    three");
    let second = open(b"root\n  two\r    three");
    // Literal CR/LF interpretation belongs to the projection, not the UTF-8
    // rope's lexical LF summary. Equal line counts cannot prove equivalence.
    assert_eq!(first.text(), second.text());
    assert_eq!(
        first.projection().presentation_line_count(),
        second.projection().presentation_line_count()
    );
    // Full reprojection can retain byte-identical text storage while changing
    // which LF represents a semantic break. Independent ropes deliberately
    // produce a conservative full changed extent instead, so share verified
    // storage here to exercise the no-text-change invalidation branch.
    let mut next = second.projection().clone();
    next.install_unchanged_text_storage(first.projection())
        .unwrap();
    assert!(next
        .text_tree()
        .shares_root_with(first.projection().text_tree()));
    assert!(!next.shares_presentation_lines_with(first.projection()));
    let mut snapshot = IndentSnapshot::new(projection(&first), 4);
    complete(&mut snapshot);
    let dirty = snapshot.synchronize(Arc::new(next), 4);
    assert_eq!(dirty, 0..second.projection().presentation_line_count());
    assert!(snapshot.index.is_empty());
    verify_complete(&mut snapshot);
}

#[test]
fn snapshot_equality_uses_identity_without_flattening_text() {
    let document = document(&"root\n  child\n".repeat(1_000));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let copied = snapshot.clone();
    assert_eq!(snapshot, copied);
    snapshot
        .index
        .splice(1..2, [LineIndentation::new(6)])
        .unwrap();
    assert_ne!(snapshot, copied);
}

#[test]
fn bounded_prefix_cache_reuses_giant_indent_across_jobs_and_body_edits() {
    struct LimitedChecks(std::cell::Cell<usize>);
    impl LayoutCancellationProbe for LimitedChecks {
        fn is_cancelled(&self) -> bool {
            let remaining = self.0.get();
            self.0.set(remaining.saturating_sub(1));
            remaining == 0
        }
    }
    let mut document = document(&format!("{}body", " ".repeat(256 * 1024)));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    let checks = LimitedChecks(std::cell::Cell::new(2));
    assert_eq!(
        snapshot
            .indentation(0, &LayoutRunControl::synchronous(&checks))
            .unwrap(),
        (256 * 1024, 256 * 1024)
    );
    document.insert(256 * 1024 + 1, "more").unwrap();
    snapshot.synchronize(projection(&document), 4);
    let checks = LimitedChecks(std::cell::Cell::new(2));
    assert_eq!(
        snapshot
            .indentation(0, &LayoutRunControl::synchronous(&checks))
            .unwrap(),
        (256 * 1024, 256 * 1024)
    );

    let document = self::document(&"  line\n".repeat(700));
    snapshot.synchronize(projection(&document), 4);
    complete(&mut snapshot);
    assert!(snapshot.prefixes.len() <= 256);
}

#[test]
fn cached_all_space_prefix_updates_typing_backspace_and_first_body_replacement() {
    let length = INDENT_SYNC_MAX_BYTES * 4;
    let mut document = document(&format!("{}body", " ".repeat(length)));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    assert!(!snapshot.prefixes[&0].has_tabs);
    document.insert(100, " ").unwrap();
    snapshot.synchronize(projection(&document), 4);
    assert_eq!(
        snapshot.index.len(),
        1,
        "cached spaces are updated without truncation or a full scan"
    );
    assert_eq!(snapshot.index.line(0).unwrap().columns, length + 1);
    assert_eq!(snapshot.prefixes[&0].pair(), (length + 1, length + 1));
    document.delete(100..101).unwrap();
    snapshot.synchronize(projection(&document), 4);
    assert_eq!(snapshot.index.len(), 1);
    assert_eq!(snapshot.prefixes[&0].pair(), (length, length));
    document.replace(length..length + 1, "B").unwrap();
    snapshot.synchronize(projection(&document), 4);
    assert_eq!(
        snapshot.index.len(),
        1,
        "ASCII first-body replacement preserves its prefix proof"
    );
    assert_eq!(snapshot.prefixes[&0].pair(), (length, length));
    verify_complete(&mut snapshot);
}

#[test]
fn tab_provenance_is_recorded_even_when_columns_equal_prefix_bytes() {
    let length = INDENT_SYNC_MAX_BYTES * 4;
    let mut document = document(&format!("{}body", " ".repeat(length)));
    let mut snapshot = IndentSnapshot::new(projection(&document), 4);
    complete(&mut snapshot);
    // At column three this tab advances exactly one column: byte and column
    // counts still agree, but a later insertion changes that tab's advance.
    document.insert(3, "\t").unwrap();
    snapshot.synchronize(projection(&document), 4);
    assert_eq!(snapshot.index.len(), 1);
    assert_eq!(snapshot.prefixes[&0].pair(), (length + 1, length + 1));
    assert!(snapshot.prefixes[&0].has_tabs);
    document.insert(0, " ").unwrap();
    snapshot.synchronize(projection(&document), 4);
    assert!(
        snapshot.index.is_empty(),
        "a cached tab prefix must decline the all-space arithmetic path"
    );
    verify_complete(&mut snapshot);
    assert_eq!(snapshot.index.line(0).unwrap().columns, length + 5);
}
