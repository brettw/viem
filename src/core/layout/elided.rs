//! Diff mode's collapsed context: the hard-line runs a view hides.
//!
//! The runs are sorted and disjoint. Queries visit only the runs they cross.
//! Every change that stops hiding lines returns them, so the view can give
//! them estimated heights again; [`ElidedLines::hide_in`] gives hidden lines
//! exact zero height.
use std::ops::Range;

use super::height_index::ViewHeightIndex;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ElidedLines {
    runs: Vec<Range<usize>>,
    /// Runs the caret opened, kept until diff mode reads them.
    opened: Vec<Range<usize>>,
}

impl ElidedLines {
    pub(crate) fn runs(&self) -> &[Range<usize>] {
        &self.runs
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The run containing `line`, if any.
    pub(crate) fn run_at(&self, line: usize) -> Option<Range<usize>> {
        let index = self.runs.partition_point(|run| run.end <= line);
        self.runs.get(index).filter(|run| run.start <= line).cloned()
    }

    /// The runs intersecting `range`, unclipped. An empty `range` intersects
    /// the run containing its start.
    fn intersecting(&self, range: Range<usize>) -> impl Iterator<Item = &Range<usize>> {
        let first = self.runs.partition_point(|run| run.end <= range.start);
        self.runs[first..].iter().take_while(move |run| run.start < range.end)
    }

    /// The hidden parts of `range`.
    pub(crate) fn within(&self, range: Range<usize>) -> Vec<Range<usize>> {
        self.intersecting(range.clone())
            .map(|run| run.start.max(range.start)..run.end.min(range.end))
            .filter(|run| !run.is_empty())
            .collect()
    }

    /// The number of lines of `range` that are not hidden.
    pub(crate) fn visible_count(&self, range: Range<usize>) -> usize {
        range.len() - self.within(range).iter().map(Range::len).sum::<usize>()
    }

    /// Move from `line` by `count` visible lines in either direction,
    /// stopping at 0 and `total`. Hidden runs are crossed without counting.
    pub(crate) fn step(&self, line: usize, count: usize, forward: bool, total: usize) -> usize {
        let mut at = line.min(total);
        let mut remaining = count;
        if forward {
            let mut index = self.runs.partition_point(|run| run.end <= at);
            while remaining > 0 && at < total {
                match self.runs.get(index) {
                    Some(run) if run.start <= at => {
                        at = run.end;
                        index += 1;
                    }
                    next => {
                        let step = (next.map_or(total, |run| run.start.min(total)) - at).min(remaining);
                        at += step;
                        remaining -= step;
                    }
                }
            }
        } else {
            let mut index = self.runs.partition_point(|run| run.start < at);
            while remaining > 0 && at > 0 {
                match index.checked_sub(1).map(|previous| &self.runs[previous]) {
                    Some(run) if run.end >= at => {
                        at = run.start;
                        index -= 1;
                    }
                    previous => {
                        let step = (at - previous.map_or(0, |run| run.end)).min(remaining);
                        at -= step;
                        remaining -= step;
                    }
                }
            }
        }
        at
    }

    /// Hide `runs`, sorted and disjoint, clipped to `line_count`. Returns the
    /// previously hidden runs that are no longer hidden, or `None` when
    /// nothing changed.
    #[must_use]
    pub(crate) fn replace(&mut self, runs: Vec<Range<usize>>, line_count: usize) -> Option<Vec<Range<usize>>> {
        let runs: Vec<Range<usize>> = runs.into_iter()
            .map(|run| run.start.min(line_count)..run.end.min(line_count))
            .filter(|run| !run.is_empty())
            .collect();
        if runs == self.runs {
            return None;
        }
        let shown = self.runs.iter()
            .filter(|run| runs.binary_search_by_key(&run.start, |kept| kept.start)
                .map_or(true, |index| runs[index] != **run))
            .cloned()
            .collect();
        self.runs = runs;
        Some(shown)
    }

    /// Stop hiding `run` because the caret moved onto it, and keep it for
    /// [`Self::take_opened`]. Returns whether it was hidden.
    #[must_use]
    pub(crate) fn open(&mut self, run: &Range<usize>) -> bool {
        let Ok(index) = self.runs.binary_search_by_key(&run.start, |hidden| hidden.start) else { return false };
        if self.runs[index] != *run {
            return false;
        }
        self.runs.remove(index);
        self.opened.push(run.clone());
        true
    }

    pub(crate) fn has_opened(&self) -> bool {
        !self.opened.is_empty()
    }

    /// The runs the caret opened since the last call.
    pub(crate) fn take_opened(&mut self) -> Vec<Range<usize>> {
        std::mem::take(&mut self.opened)
    }

    /// Move the runs through an edit that replaced the lines `old` with
    /// `new`; both start at the same line. Runs after the edit move with
    /// their lines. A run the edit touched is no longer hidden: the result
    /// holds its surviving lines, with the edit's, in new coordinates.
    #[must_use]
    pub(crate) fn rebase(&mut self, old: Range<usize>, new: Range<usize>) -> Vec<Range<usize>> {
        debug_assert_eq!(old.start, new.start);
        let delta = new.end as i128 - old.end as i128;
        let shift = |line: usize| usize::try_from(line as i128 + delta).ok();
        let mut shown = Vec::new();
        self.runs = std::mem::take(&mut self.runs).into_iter().filter_map(|run| {
            if run.end <= old.start {
                Some(run)
            } else if run.start >= old.end {
                Some(shift(run.start)?..shift(run.end)?)
            } else {
                shown.push(run.start.min(new.start)..shift(run.end).unwrap_or(0).max(new.end));
                None
            }
        }).collect();
        shown
    }

    /// Stop hiding every run, returning them.
    #[must_use]
    pub(crate) fn clear(&mut self) -> Vec<Range<usize>> {
        std::mem::take(&mut self.runs)
    }

    /// Give the hidden lines of every run intersecting `within` (every run,
    /// for `None`) exact zero height, whatever else was invalidated.
    pub(crate) fn hide_in(&self, index: &mut ViewHeightIndex, within: Option<Range<usize>>) {
        let count = index.hard_line_count();
        for run in self.intersecting(within.unwrap_or(0..usize::MAX)) {
            let run = run.start.min(count)..run.end.min(count);
            if !run.is_empty() {
                let _ = index.set_hidden_range(run);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(runs: &[Range<usize>]) -> ElidedLines {
        let mut elided = ElidedLines::default();
        assert_eq!(elided.replace(runs.to_vec(), 100), Some(Vec::new()));
        elided
    }

    #[test]
    fn steps_count_only_visible_lines_and_stop_at_the_edges() {
        let elided = lines(&[3..6, 6..8, 10..12]);
        // 2, then 8 after both adjacent runs, then 9.
        assert_eq!(elided.step(1, 3, true, 20), 9);
        // Starting inside a run leaves it before counting.
        assert_eq!(elided.step(4, 1, true, 20), 9);
        // A count that ends at a run's start stops there.
        assert_eq!(elided.step(1, 2, true, 20), 3);
        assert_eq!(elided.step(9, 50, true, 20), 20);
        assert_eq!(elided.step(9, 3, false, 20), 1);
        assert_eq!(elided.step(12, 1, false, 20), 9);
        assert_eq!(elided.step(8, 1, false, 20), 2);
        assert_eq!(elided.step(9, 50, false, 20), 0);
        assert_eq!(elided.step(30, 0, true, 20), 20);
    }

    #[test]
    fn steps_match_a_line_by_line_walk() {
        let elided = lines(&[0..2, 4..5, 5..9, 12..13, 15..16]);
        let total = 16;
        for line in 0..=total {
            for count in 0..=total {
                for forward in [true, false] {
                    let mut at = line;
                    let mut remaining = count;
                    while remaining > 0 {
                        if forward {
                            if at >= total { break; }
                            if let Some(run) = elided.run_at(at) { at = run.end; continue; }
                            at += 1;
                        } else {
                            if at == 0 { break; }
                            if let Some(run) = elided.run_at(at - 1) { at = run.start; continue; }
                            at -= 1;
                        }
                        remaining -= 1;
                    }
                    assert_eq!(elided.step(line, count, forward, total), at, "{line} {count} {forward}");
                }
            }
        }
    }

    #[test]
    fn counts_and_clips_hidden_lines_within_a_range() {
        let elided = lines(&[3..6, 10..12]);
        assert_eq!(elided.within(4..11), [4..6, 10..11]);
        assert_eq!(elided.visible_count(0..20), 15);
        assert_eq!(elided.visible_count(4..5), 0);
        assert!(elided.within(6..10).is_empty());
        assert_eq!(elided.run_at(5), Some(3..6));
        assert_eq!(elided.run_at(6), None);
    }

    #[test]
    fn replacing_runs_returns_only_the_runs_no_longer_hidden() {
        let mut elided = lines(&[3..6, 10..12]);
        assert_eq!(elided.replace(vec![3..6, 10..12], 100), None);
        assert_eq!(elided.replace(vec![3..6, 10..11, 90..120], 95), Some(vec![10..12]));
        assert_eq!(elided.runs(), [3..6, 10..11, 90..95]);
        assert_eq!(elided.replace(vec![200..300], 95), Some(vec![3..6, 10..11, 90..95]));
        assert!(elided.is_empty());
    }

    #[test]
    fn opening_a_run_keeps_it_until_it_is_taken() {
        let mut elided = lines(&[3..6, 10..12]);
        assert!(!elided.open(&(3..5)));
        assert!(elided.open(&(10..12)));
        assert!(!elided.open(&(10..12)));
        assert_eq!(elided.runs(), [3..6]);
        assert!(elided.has_opened());
        assert_eq!(elided.take_opened(), [10..12]);
        assert!(!elided.has_opened());
    }

    #[test]
    fn an_edit_moves_later_runs_and_shows_the_runs_it_touches() {
        let mut elided = lines(&[0..2, 4..8, 10..12, 20..25]);
        // Lines 6..11 become 6..8: 4..8 and 10..12 lose lines and are shown.
        let shown = elided.rebase(6..11, 6..8);
        assert_eq!(elided.runs(), [0..2, 17..22]);
        assert_eq!(shown, [4..8, 6..9]);
        // An insertion at a run's end leaves it hidden.
        assert!(elided.rebase(2..2, 2..5).is_empty());
        assert_eq!(elided.runs(), [0..2, 20..25]);
        assert_eq!(elided.clear(), [0..2, 20..25]);
        assert!(elided.is_empty());
    }
}
