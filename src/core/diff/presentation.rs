//! What one side of a diff draws: changed lines (Added line or Removed line),
//! changed spans within them (Added text or Removed text), and, when the
//! sides are aligned, blank filler that keeps corresponding lines level.

use super::state::DiffState;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Side {
    /// The read-only earlier version; its changed lines are removed lines.
    Base,
    /// The live buffer; its changed lines are added lines.
    New,
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
    pub fillers: Vec<Filler>,
}

impl DiffState {
    /// One side's presentation. Filler is produced only when `aligned`.
    pub fn presentation(&self, side: Side, aligned: bool) -> SidePresentation {
        let mut presentation = SidePresentation::default();
        for hunk in self.hunks() {
            let (own, other) = match side {
                Side::Base => (hunk.base.clone(), hunk.new.clone()),
                Side::New => (hunk.new.clone(), hunk.base.clone()),
            };
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

    #[test]
    fn filler_at_the_end_follows_the_last_line() {
        let diff = state("a", "a\nb\nc");
        assert_eq!(diff.presentation(Side::Base, true).fillers, vec![Filler { before_line: 1, lines: 2 }]);
    }
}
