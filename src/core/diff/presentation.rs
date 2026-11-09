//! What one side of a diff draws: changed lines (Added line or Removed line),
//! changed spans within them (Added text or Removed text), when the sides are
//! aligned, blank filler that keeps corresponding lines level, and collapsed
//! context, unchanged lines hidden behind a divider.

use super::state::DiffState;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Side {
    /// The read-only earlier version; its changed lines are removed lines.
    Base,
    /// The live buffer; its changed lines are added lines.
    New,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Self::Base => Self::New,
            Self::New => Self::Base,
        }
    }
}

/// Blank, non-text space drawn before a hard line (or after the last line
/// when `before_line` equals the line count), measured in default lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Filler {
    pub before_line: usize,
    pub lines: usize,
}

/// Sorted, non-overlapping presentation of one side, in its hard lines.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SidePresentation {
    /// Ranges of hard lines styled as changed lines.
    pub lines: Vec<Range<usize>>,
    /// Changed spans: a hard line and a byte range within it.
    pub text: Vec<(usize, Range<usize>)>,
    /// Sorted by `before_line`.
    pub fillers: Vec<Filler>,
    /// Hidden runs of unchanged lines, each drawn as one divider.
    pub collapsed: Vec<Range<usize>>,
}

/// A run of equal lines that collapsed context can hide, in both sides'
/// numbering. Both ranges have the same length.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollapsibleRun {
    pub base: Range<usize>,
    pub new: Range<usize>,
}

impl CollapsibleRun {
    pub fn side(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Base => &self.base,
            Side::New => &self.new,
        }
    }
}

impl DiffState {
    /// One side's presentation. Filler is produced only when `aligned`.
    pub fn presentation(&self, side: Side, aligned: bool) -> SidePresentation {
        let mut presentation = SidePresentation::default();
        for hunk in self.hunks() {
            let (own, other) = (hunk.side(side).clone(), hunk.side(side.other()).clone());
            if !own.is_empty() {
                presentation.lines.push(own.clone());
            }
            for pair in hunk.pairs.iter() {
                let (line, spans) = match side {
                    Side::Base => (own.start + pair.base, &pair.spans.base),
                    Side::New => (own.start + pair.new, &pair.spans.new),
                };
                presentation.text.extend(spans.iter().map(|span| (line, span.clone())));
            }
            if aligned && other.len() > own.len() {
                presentation.fillers.push(Filler { before_line: own.end, lines: other.len() - own.len() });
            }
        }
        presentation
    }

    /// The runs of equal lines that collapsed context hides: each equal run
    /// less `context` lines next to every change beside it, kept only when
    /// that hides more than `context` lines. A run at the start or end of the
    /// file keeps context only towards its change, and a file without changes
    /// hides nothing.
    pub fn collapsible_runs(&self, context: usize) -> Vec<CollapsibleRun> {
        let hunks: Vec<_> = self.hunks().map(|hunk| (hunk.base, hunk.new)).collect();
        if hunks.is_empty() {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut push = |base: usize, new: usize, length: usize, before: bool, after: bool| {
            let keep_start = if before { context } else { 0 };
            let keep_end = if after { context } else { 0 };
            let hidden = length.saturating_sub(keep_start + keep_end);
            if hidden > context {
                runs.push(CollapsibleRun {
                    base: base + keep_start..base + keep_start + hidden,
                    new: new + keep_start..new + keep_start + hidden,
                });
            }
        };
        let (mut base, mut new) = (0, 0);
        for (index, (hunk_base, hunk_new)) in hunks.iter().enumerate() {
            push(base, new, hunk_base.start - base, index > 0, true);
            (base, new) = (hunk_base.end, hunk_new.end);
        }
        push(base, new, self.base_line_count() - base, true, false);
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::state::{DiffLimits, TextLines};

    fn state(base: &str, new: &str) -> DiffState {
        DiffState::compute(&TextLines::new(base.into()), &TextLines::new(new.into()), DiffLimits::default())
    }

    #[test]
    fn an_indented_line_is_a_changed_line_on_both_sides_with_spans_only_where_added() {
        let diff = state("a\nfoo();\nb", "a\n  foo();\nb");
        let new = diff.presentation(Side::New, true);
        assert_eq!(new.lines, vec![1..2]);
        assert_eq!(new.text, vec![(1, 0..2)]);
        assert!(new.fillers.is_empty());
        let base = diff.presentation(Side::Base, true);
        assert_eq!(base.lines, vec![1..2]);
        assert!(base.text.is_empty(), "nothing was removed from the base line");
    }

    #[test]
    fn the_shorter_side_gets_filler_after_its_hunk_only_when_aligned() {
        let diff = state("a\nb\nc\nd", "a\nd");
        let new = diff.presentation(Side::New, true);
        assert!(new.lines.is_empty());
        assert_eq!(new.fillers, vec![Filler { before_line: 1, lines: 2 }]);
        assert_eq!(diff.presentation(Side::Base, true).lines, vec![1..3]);
        assert!(diff.presentation(Side::Base, true).fillers.is_empty());
        assert!(diff.presentation(Side::New, false).fillers.is_empty());
    }

    fn lines(count: usize, changed: &[usize]) -> String {
        (0..count).map(|line| if changed.contains(&line) { format!("changed {line}") } else { format!("line {line}") })
            .collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn collapsible_runs_keep_context_beside_each_change_and_only_towards_it_at_the_ends() {
        let diff = state(&lines(40, &[]), &lines(40, &[10, 30]));
        assert_eq!(diff.collapsible_runs(3), vec![
            CollapsibleRun { base: 0..7, new: 0..7 },
            CollapsibleRun { base: 14..27, new: 14..27 },
            CollapsibleRun { base: 34..40, new: 34..40 },
        ]);
    }

    #[test]
    fn a_run_hiding_no_more_than_the_context_stays_shown() {
        // Lines 11..20 are equal: 3 + 3 kept leaves 3 (shown) and 4 (hidden).
        assert!(state(&lines(21, &[]), &lines(21, &[10, 20])).collapsible_runs(3).iter()
            .all(|run| run.new.start < 10));
        assert_eq!(state(&lines(22, &[]), &lines(22, &[10, 21])).collapsible_runs(3)[1].new, 14..18);
        assert!(state(&lines(7, &[]), &lines(7, &[])).collapsible_runs(3).is_empty(), "no changes, nothing hidden");
    }

    #[test]
    fn collapsible_runs_follow_each_side_when_line_counts_differ() {
        let diff = state(&lines(30, &[]), &("x\n".to_owned() + &lines(30, &[])));
        assert_eq!(diff.collapsible_runs(3), vec![CollapsibleRun { base: 3..30, new: 4..31 }]);
    }

    #[test]
    fn filler_at_the_end_follows_the_last_line() {
        let diff = state("a", "a\nb\nc");
        assert_eq!(diff.presentation(Side::Base, true).fillers, vec![Filler { before_line: 1, lines: 2 }]);
    }
}
