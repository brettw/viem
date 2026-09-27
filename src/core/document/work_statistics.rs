//! Opt-in work accounting across an entire synchronous document operation.
//! Counters include scratch preparation and pre-transaction context queries;
//! unlike candidate projection statistics they do not stop at commit boundaries.
use std::cell::Cell;

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
