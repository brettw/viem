//! Opt-in work accounting across an entire synchronous document operation.
//! Counters include scratch preparation and pre-transaction context queries;
//! unlike candidate projection statistics they do not stop at commit boundaries.
use std::cell::Cell;

/// Explicit causes of a broader HTML query or parse. Counts are independent
/// of candidate counts: one operation can encounter several causes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum DocumentWorkFallback {
    /// A complete HTML candidate was built. This identifies the broader
    /// parse; it does not distinguish every structural/style policy causing it.
    FullHtmlGrammarProjection,
    /// Incremental lexical scope maintenance could not prove a safe splice.
    IndexRegionNonConvergence,
    /// A regional HTML fragment could not reproduce its enclosing recovery
    /// or inherited style context safely.
    HtmlRecoveryContext,
}

/// Aggregate actual work, including repeated work on discarded candidates.
/// Byte counts measure input consumed/copied, not unique source coverage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DocumentWorkStatistics {
    /// Records and tree nodes inspected by structural list affordances.
    pub list_capability_blocks_visited: usize,
    pub list_capability_nodes_visited: usize,
    pub source_full_materializations: usize,
    pub source_full_materialized_bytes: usize,
    pub source_range_materializations: usize,
    pub source_range_materialized_bytes: usize,
    pub source_decode_calls: usize,
    pub source_decoded_bytes: usize,
    pub html_tokenization_calls: usize,
    pub html_tokenized_bytes: usize,
    pub html_tree_tokenization_calls: usize,
    pub html_tree_tokenized_bytes: usize,
    pub html_scope_queries: usize,
    pub html_scope_entries_visited: usize,
    pub html_scope_entries_copied: usize,
    pub html_scope_syntax_bytes_copied: usize,
    pub html_index_nodes_visited: usize,
    pub html_index_nodes_copied: usize,
    pub html_index_entries_rebuilt: usize,
    /// Largest newly charged index allocation graph in one ledger visit.
    /// An existing ledger skips shared nodes; an initial open charges the complete index.
    pub html_index_newly_retained_bytes: usize,
    pub fallback_counts: [usize; 3],
    pub scratch_documents: usize,
    pub retained_memory_allocation_visits: usize,
    pub retained_memory_allocations_registered: usize,
    pub retained_memory_release_visits: usize,
    pub retained_memory_allocations_released: usize,
    pub model_requests: usize,
    pub transaction_commits: usize,
    pub full_projection_candidates: usize,
    pub regional_projection_candidates: usize,
    pub projected_formatted_bytes: usize,
    pub projection_persistent_nodes_visited: usize,
    pub projection_persistent_nodes_copied: usize,
    pub formatted_full_materialized_bytes: usize,
}

impl DocumentWorkStatistics {
    fn accumulate(&mut self, other: Self) {
        macro_rules! sum { ($($field:ident),+ $(,)?) => { $(self.$field = self.$field.saturating_add(other.$field);)+ }; }
        sum!(
            list_capability_blocks_visited,
            list_capability_nodes_visited,
            source_full_materializations,
            source_full_materialized_bytes,
            source_range_materializations,
            source_range_materialized_bytes,
            source_decode_calls,
            source_decoded_bytes,
            html_tokenization_calls,
            html_tokenized_bytes,
            html_tree_tokenization_calls,
            html_tree_tokenized_bytes,
            html_scope_queries,
            html_scope_entries_visited,
            html_scope_entries_copied,
            html_scope_syntax_bytes_copied,
            html_index_nodes_visited,
            html_index_nodes_copied,
            html_index_entries_rebuilt,
            scratch_documents,
            retained_memory_allocation_visits,
            retained_memory_allocations_registered,
            retained_memory_release_visits,
            retained_memory_allocations_released,
            model_requests,
            transaction_commits,
            full_projection_candidates,
            regional_projection_candidates,
            projected_formatted_bytes,
            projection_persistent_nodes_visited,
            projection_persistent_nodes_copied,
            formatted_full_materialized_bytes
        );
        self.html_index_newly_retained_bytes = self
            .html_index_newly_retained_bytes
            .max(other.html_index_newly_retained_bytes);
        for (total, value) in self.fallback_counts.iter_mut().zip(other.fallback_counts) {
            *total = total.saturating_add(value);
        }
    }

    pub fn fallback_count(self, reason: DocumentWorkFallback) -> usize {
        self.fallback_counts[reason as usize]
    }

    pub(crate) fn note_fallback(&mut self, reason: DocumentWorkFallback) {
        self.fallback_counts[reason as usize] += 1;
    }
}

thread_local! {
    static ACTIVE: Cell<Option<DocumentWorkStatistics>> = const { Cell::new(None) };
}

/// Measure all synchronous work performed by `operation` on this thread.
/// Nested measurements are included in their enclosing measurement. Other
/// threads are independent. Unwinding restores any enclosing measurement.
/// Disabled instrumentation allocates no records or per-query diagnostic logs.
pub fn measure_document_work<T>(operation: impl FnOnce() -> T) -> (T, DocumentWorkStatistics) {
    struct Scope(Option<DocumentWorkStatistics>);
    impl Drop for Scope {
        fn drop(&mut self) {
            ACTIVE.with(|active| {
                let measured = active.replace(None).unwrap_or_default();
                if let Some(mut parent) = self.0 {
                    parent.accumulate(measured);
                    active.set(Some(parent));
                }
            });
        }
    }
    let scope =
        Scope(ACTIVE.with(|active| active.replace(Some(DocumentWorkStatistics::default()))));
    let result = operation();
    let measured = ACTIVE.with(|active| active.get().unwrap_or_default());
    drop(scope);
    (result, measured)
}

#[inline]
pub(super) fn record(update: impl FnOnce(&mut DocumentWorkStatistics)) {
    ACTIVE.with(|active| {
        if let Some(mut counters) = active.get() {
            update(&mut counters);
            active.set(Some(counters));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counters_measure_actual_full_and_bounded_reads() {
        let source = super::super::source::SourceSnapshot::new(b"abcdef".to_vec());
        let (_, work) = measure_document_work(|| {
            assert_eq!(source.bytes(), b"abcdef");
            let selected = source.bytes_in(2..4).unwrap();
            super::super::Encoding::Utf8
                .decode_region(&selected, 2)
                .unwrap();
        });
        assert_eq!(work.source_full_materializations, 1);
        assert_eq!(work.source_full_materialized_bytes, 6);
        assert_eq!(work.source_range_materializations, 1);
        assert_eq!(work.source_range_materialized_bytes, 2);
        assert_eq!(work.source_decode_calls, 1);
        assert_eq!(work.source_decoded_bytes, 2);
        record(|_| panic!("disabled instrumentation must not evaluate diagnostic work"));
    }

    #[test]
    fn nested_scopes_accumulate_and_threads_are_independent() {
        let (_, outer) = measure_document_work(|| {
            record(|stats| stats.source_decoded_bytes += 2);
            let (_, inner) =
                measure_document_work(|| record(|stats| stats.source_decoded_bytes += 3));
            assert_eq!(inner.source_decoded_bytes, 3);
            std::thread::spawn(|| record(|stats| stats.source_decoded_bytes += 100))
                .join()
                .unwrap();
        });
        assert_eq!(outer.source_decoded_bytes, 5);
        assert_eq!(ACTIVE.with(Cell::get), None);
    }

    #[test]
    fn unwinding_restores_the_enclosing_scope() {
        let (_, outer) = measure_document_work(|| {
            let _ = std::panic::catch_unwind(|| {
                measure_document_work(|| {
                    record(|stats| stats.source_decoded_bytes += 7);
                    panic!("test unwind");
                })
            });
            record(|stats| stats.source_decoded_bytes += 2);
        });
        assert_eq!(outer.source_decoded_bytes, 9);
        assert_eq!(ACTIVE.with(Cell::get), None);
    }
}
