//! The current presentation runs of one syntax input, indexed by formatted
//! range so an edit or a regional result touches only the runs it affects.
//!
//! The store is a persistent interval index: shifting every run after an edit
//! is one lazy coordinate change on the untouched subtree, and replacing a
//! query region's runs splices the overlapping index span. The delta records
//! what changed since the last publication so the Code presentation can be
//! spliced the same way rather than rebuilt from every retained run.
use super::SyntaxRun;
use crate::document::range_index::{IntervalRangeStore, RangeSpliceStats, RangedItem};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

impl RangedItem for SyntaxRun {
    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self {
            range,
            name: self.name.clone(),
            origin: self.origin.clone(),
            priority: self.priority,
        }
    }
}

/// Conservative retained charge for one run. Names and origins are shared
/// between runs; charging their length per run keeps the budget conservative.
pub(crate) fn run_bytes(run: &SyntaxRun) -> usize {
    std::mem::size_of::<SyntaxRun>() + run.name.len() + run.origin.len()
}

/// Formatted regions whose runs changed since the last publication, in the
/// current input's coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PublicationDelta {
    /// The change cannot be described locally; publish from every run.
    pub unbounded: bool,
    /// The one edit hull (old, new) applied since the last publication. Runs
    /// strictly inside the old hull were dropped and later runs shifted.
    pub hull: Option<(Range<usize>, Range<usize>)>,
    /// Regions whose runs were replaced by accepted results or eviction.
    pub replaced: Vec<Range<usize>>,
}

impl PublicationDelta {
    pub(super) fn unbounded() -> Self {
        Self {
            unbounded: true,
            ..Default::default()
        }
    }

    pub(super) fn note_hull(&mut self, old_hull: Range<usize>, new_hull: Range<usize>) {
        if self.unbounded {
            return;
        }
        if self.hull.is_some() || !self.replaced.is_empty() {
            // Two edits, or results installed between them, would need
            // composed coordinates; one whole publication is simpler.
            *self = Self::unbounded();
            return;
        }
        self.hull = Some((old_hull, new_hull));
    }

    pub(super) fn note_replaced(&mut self, range: Range<usize>) {
        if self.unbounded || range.is_empty() {
            return;
        }
        if let Some(last) = self.replaced.last_mut() {
            if last.end >= range.start && range.end >= last.start {
                last.start = last.start.min(range.start);
                last.end = last.end.max(range.end);
                return;
            }
        }
        self.replaced.push(range);
    }

    /// Whether the delta describes a change at all.
    pub fn is_empty(&self) -> bool {
        !self.unbounded && self.hull.is_none() && self.replaced.is_empty()
    }
}

#[derive(Clone)]
pub struct RunStore {
    store: IntervalRangeStore<SyntaxRun>,
    bytes: usize,
    names: BTreeMap<Arc<str>, usize>,
}

impl Default for RunStore {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl RunStore {
    pub(super) fn new(runs: Vec<SyntaxRun>) -> Self {
        let mut names: BTreeMap<Arc<str>, usize> = BTreeMap::new();
        let mut bytes = 0usize;
        for run in &runs {
            *names.entry(run.name.0.clone()).or_default() += 1;
            bytes = bytes.saturating_add(run_bytes(run));
        }
        Self {
            store: IntervalRangeStore::new(runs),
            bytes,
            names,
        }
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Retained bytes for the runs and their shared name table.
    pub fn bytes(&self) -> usize {
        self.bytes
            .saturating_add(self.names.keys().map(|name| name.len() + 32).sum::<usize>())
    }

    /// The distinct names referenced by the current runs, in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &Arc<str>> {
        self.names.keys()
    }

    /// Every current run in start order. Flattens the store; tests and
    /// unbounded publications only.
    pub fn runs(&self) -> Vec<SyntaxRun> {
        self.store.as_slice().to_vec()
    }

    /// The runs strictly overlapping `range`, in start order.
    pub fn runs_in(&self, range: &Range<usize>) -> Vec<SyntaxRun> {
        self.store.query_overlapping(range)
    }

    fn count(&mut self, run: &SyntaxRun, added: bool) {
        if added {
            *self.names.entry(run.name.0.clone()).or_default() += 1;
            self.bytes = self.bytes.saturating_add(run_bytes(run));
        } else {
            if let Some(count) = self.names.get_mut(&run.name.0) {
                *count -= 1;
                if *count == 0 {
                    self.names.remove(&run.name.0);
                }
            }
            self.bytes = self.bytes.saturating_sub(run_bytes(run));
        }
    }

    fn splice(
        &mut self,
        indices: Range<usize>,
        removed: &[SyntaxRun],
        replacement: Vec<SyntaxRun>,
        old_end: usize,
        new_end: usize,
    ) -> bool {
        let mut stats = RangeSpliceStats::default();
        let Some(next) = self
            .store
            .splice(indices, replacement.clone(), old_end, new_end, &mut stats)
        else {
            return false;
        };
        for run in removed {
            self.count(run, false);
        }
        for run in &replacement {
            self.count(run, true);
        }
        self.store = next;
        true
    }

    /// Replace the runs overlapping `range` with `runs`, keeping the pieces of
    /// crossing runs that lie outside it. Returns the previous runs clipped to
    /// `range`, for appearance comparison.
    pub(super) fn replace(&mut self, range: &Range<usize>, runs: Vec<SyntaxRun>) -> Vec<SyntaxRun> {
        let indices = self.store.overlapping_index_span(range);
        let Some(items) = self.store.get_range(&indices) else {
            return Vec::new();
        };
        let mut kept = Vec::new();
        let mut previous = Vec::new();
        for item in &items {
            if !overlaps(&item.range, range) {
                kept.push(item.clone());
                continue;
            }
            previous.push(item.with_range(item.range.start.max(range.start)..item.range.end.min(range.end)));
            kept.extend(outside(item, range));
        }
        kept.extend(runs);
        self.splice(indices, &items, kept, 0, 0);
        previous
    }

    /// Remap only affected runs, including the predecessor at the hull start
    /// whose style an insertion can inherit. Shift the untouched suffix lazily.
    /// `predecessor_start` is the preceding grapheme's start, not its last byte.
    /// Return a hull covering the complete remapped runs so publication can
    /// replace them without leaving fragments of the old presentation behind.
    pub(super) fn rebase_for_edit(
        &mut self,
        old_hull: &Range<usize>,
        new_hull: &Range<usize>,
        predecessor_start: usize,
        mut remap: impl FnMut(&SyntaxRun) -> Vec<SyntaxRun>,
    ) -> Option<(Range<usize>, Range<usize>)> {
        let query = predecessor_start..old_hull.end;
        let indices = self.store.overlapping_index_span(&query);
        let items = self.store.get_range(&indices)?;
        let mut end = items.iter().map(|run| run.range.end).max().unwrap_or(old_hull.end).max(old_hull.end);
        let mut new_end = new_hull.end.checked_add(end - old_hull.end)?;
        let replacement: Vec<_> = items.iter().flat_map(&mut remap).collect();
        // Raw captures can start inside an earlier grapheme owned by another
        // run. Use their displayed starts so that owner's span stays outside
        // the publication hull unless it was actually remapped here.
        let start = replacement.iter().map(|run| run.range.start).min().unwrap_or(old_hull.start).min(old_hull.start);
        // A scalar capture's displayed style can reach to its grapheme's end.
        // Cover that normalized extent as well as the raw provider run.
        let mapped_end = replacement.iter().map(|run| run.range.end).max().unwrap_or(new_end);
        if mapped_end > new_end {
            end = end.checked_add(mapped_end - new_end)?;
            new_end = mapped_end;
        }
        self.splice(indices, &items, replacement, old_hull.end, new_hull.end)
            .then_some((start..end, start..new_end))
    }

    /// Evict runs from the document start until the retained bytes fit the
    /// budget, preferring runs outside `protected` regions (current provider
    /// coverage). Returns the coordinate hull of the removed runs.
    pub(super) fn evict_front(&mut self, budget: usize, protected: &[Range<usize>]) -> Option<Range<usize>> {
        if self.bytes() <= budget || self.store.is_empty() {
            return None;
        }
        let mut excess = self.bytes() - budget;
        let mut removed: Option<Range<usize>> = None;
        for pass in 0..2 {
            let mut index = 0;
            while excess > 0 && index < self.store.len() {
                let chunk = index..(index + 64).min(self.store.len());
                let Some(items) = self.store.get_range(&chunk) else {
                    return removed;
                };
                let mut kept = Vec::new();
                let mut dropped = 0usize;
                for item in &items {
                    let shielded = pass == 0 && protected.iter().any(|region| overlaps(region, &item.range));
                    if excess == 0 || shielded {
                        kept.push(item.clone());
                        continue;
                    }
                    excess = excess.saturating_sub(run_bytes(item));
                    dropped += 1;
                    removed = Some(match removed.take() {
                        Some(range) => range.start.min(item.range.start)..range.end.max(item.range.end),
                        None => item.range.clone(),
                    });
                }
                let kept_len = kept.len();
                if dropped > 0 && !self.splice(chunk, &items, kept, 0, 0) {
                    return removed;
                }
                index += kept_len;
            }
            if excess == 0 {
                break;
            }
        }
        removed
    }

}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

pub(super) fn outside(run: &SyntaxRun, coverage: &Range<usize>) -> Vec<SyntaxRun> {
    if !overlaps(&run.range, coverage) {
        return vec![run.clone()];
    }
    let mut pieces = Vec::with_capacity(2);
    if run.range.start < coverage.start {
        pieces.push(run.with_range(run.range.start..coverage.start));
    }
    if run.range.end > coverage.end {
        pieces.push(run.with_range(coverage.end..run.range.end));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::syntax::SyntaxStyleName;

    fn run(range: Range<usize>, name: &str) -> SyntaxRun {
        SyntaxRun {
            range,
            name: SyntaxStyleName(name.into()),
            origin: "test".into(),
            priority: 0,
        }
    }

    #[test]
    fn replace_keeps_outside_pieces_and_reports_previous_clipped_runs() {
        let mut store = RunStore::new(vec![run(0..10, "A"), run(10..20, "B"), run(20..30, "C")]);
        let previous = store.replace(&(5..25), vec![run(5..15, "X"), run(15..25, "Y")]);
        assert_eq!(previous, vec![run(5..10, "A"), run(10..20, "B"), run(20..25, "C")]);
        assert_eq!(
            store.runs(),
            vec![run(0..5, "A"), run(5..15, "X"), run(15..25, "Y"), run(25..30, "C")]
        );
        assert_eq!(store.names().map(|n| n.to_string()).collect::<Vec<_>>(), ["A", "C", "X", "Y"]);
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn rebase_maps_only_affected_runs_and_expands_publication_to_their_edges() {
        let mut store = RunStore::new(vec![run(0..10, "A"), run(10..20, "B"), run(20..30, "C")]);
        let mut visited = Vec::new();
        let hull = store.rebase_for_edit(&(15..15), &(15..18), 14, |item| {
            visited.push(item.name.as_str().to_owned());
            vec![item.with_range(10..23)]
        });
        assert_eq!(visited, ["B"]);
        assert_eq!(hull, Some((10..20, 10..23)));
        assert_eq!(store.runs(), vec![run(0..10, "A"), run(10..23, "B"), run(23..33, "C")]);
        assert_eq!(store.bytes(), RunStore::new(store.runs()).bytes());
        // Include the predecessor even when it ends exactly at the insertion.
        let mut store = RunStore::new(vec![run(0..10, "A"), run(10..20, "B")]);
        assert_eq!(store.rebase_for_edit(&(10..10), &(10..12), 9, |item| vec![item.with_range(0..12)]),
            Some((0..10, 0..12)));
        assert_eq!(store.runs(), vec![run(0..12, "A"), run(12..22, "B")]);
    }

    #[test]
    fn local_rebase_work_does_not_grow_with_unrelated_cached_runs() {
        for count in [100, 20_000] {
            let mut store = RunStore::new((0..count).map(|i| run(i * 10..i * 10 + 6, "A")).collect());
            let mut visits = 0;
            let hull = store.rebase_for_edit(&(5..5), &(5..7), 4, |item| {
                visits += 1;
                vec![item.with_range(0..8)]
            });
            assert_eq!(visits, 1);
            assert_eq!(hull, Some((0..6, 0..8)));
            assert_eq!(store.len(), count);
            assert_eq!(store.runs_in(&(count * 10 - 8..count * 10)),
                vec![run(count * 10 - 8..count * 10 - 2, "A")]);
        }
    }

    #[test]
    fn eviction_removes_leading_unprotected_runs_and_reports_their_extent() {
        let runs = (0..100).map(|i| run(i * 10..i * 10 + 10, "A")).collect::<Vec<_>>();
        let mut store = RunStore::new(runs);
        let budget = store.bytes() - 25 * run_bytes(&run(0..1, "A"));
        let removed = store.evict_front(budget, &[0..50]).unwrap();
        // The five protected runs survive; the next twenty-five go.
        assert_eq!(removed, 50..300);
        assert_eq!(store.len(), 75);
        assert!(store.bytes() <= budget);
        assert_eq!(store.runs()[4], run(40..50, "A"));
        assert_eq!(store.runs()[5], run(300..310, "A"));
        assert!(store.evict_front(budget, &[]).is_none());
        // Protection yields when unprotected runs cannot cover the excess.
        let removed = store.evict_front(0, &[0..1000]).unwrap();
        assert_eq!(removed, 0..1000);
        assert!(store.is_empty());
    }

    #[test]
    fn delta_composes_one_hull_with_later_replacements_only() {
        let mut delta = PublicationDelta::default();
        delta.note_hull(5..5, 5..6);
        delta.note_replaced(0..100);
        delta.note_replaced(100..150);
        assert_eq!(delta.replaced, vec![0..150]);
        assert!(!delta.unbounded);
        delta.note_hull(7..7, 7..8);
        assert!(delta.unbounded);
    }
}
