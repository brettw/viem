//! Formatted-to-source contributor storage for rich projections.
//!
//! Most decoded text maps each scalar onto its own encoded source bytes. Such
//! single-scalar contributors are stored as bounded divisible runs and are
//! expanded back into exactly those contributors only for the range a query
//! asks about. Queries therefore return the same spans, in the same order, as
//! a per-scalar list would, while ordinary text costs a few bytes per run
//! rather than per scalar. Indivisible contributors (entities, escapes,
//! folded separators, synthetic content and caret seeds) are stored as is.
use super::{ProvenanceSpan, Side, SourceTextBoundary};
use crate::document::formatted_text::FormattedTextTree;
use crate::document::range_index::{IntervalRangeStore, RangedItem};
use std::ops::Range;

/// Formatted bytes per divisible run. Non-ASCII runs are scanned from their
/// start for interior lookups, so this bounds that work; ASCII runs map
/// arithmetically. Runs never contain U+000A, so they stay within a hard line.
pub(super) const RUN_BYTES: usize = 256;

/// Source-width rules that every scalar of a run satisfies.
const UTF8: u8 = 1;
const LATIN1: u8 = 2;
const UTF16: u8 = 4;
/// Every scalar is ASCII, so each has one formatted byte and the run's single
/// per-scalar source width.
const ASCII: u8 = 8;
const WIDTHS: u8 = UTF8 | LATIN1 | UTF16;

fn width_rules(ch: char, source_len: usize) -> u8 {
    let mut rules = 0;
    if source_len == ch.len_utf8() {
        rules |= UTF8;
    }
    if source_len == 1 && u32::from(ch) <= 0xff {
        rules |= LATIN1;
    }
    if source_len == 2 * ch.len_utf16() {
        rules |= UTF16;
    }
    rules
}

fn source_width(rules: u8, ch: char) -> usize {
    if rules & UTF8 != 0 {
        ch.len_utf8()
    } else if rules & LATIN1 != 0 {
        1
    } else {
        2 * ch.len_utf16()
    }
}

/// One stored contributor: a divisible run (`rules != 0`) or an indivisible
/// contributor (`rules == 0`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProvenanceRun {
    pub(super) formatted: Range<usize>,
    pub(super) source: Range<usize>,
    rules: u8,
}

impl ProvenanceRun {
    pub(super) fn indivisible(span: ProvenanceSpan) -> Self {
        Self { formatted: span.formatted, source: span.source, rules: 0 }
    }

    pub(super) fn is_divisible(&self) -> bool {
        self.rules != 0
    }

    /// The stored contributor itself; a divisible run is a contiguous union.
    pub(super) fn hull(&self) -> ProvenanceSpan {
        ProvenanceSpan { formatted: self.formatted.clone(), source: self.source.clone() }
    }

    /// Single-scalar contributors whose formatted range satisfies `keep`.
    fn expand(&self, text: &FormattedTextTree, keep: &impl Fn(&Range<usize>) -> bool,
        output: &mut Vec<ProvenanceSpan>) {
        if !self.is_divisible() {
            output.push(self.hull());
            return;
        }
        self.scalars(text, |formatted, source| {
            if keep(&formatted) {
                output.push(ProvenanceSpan { formatted, source });
            }
            true
        });
    }

    /// Visit each scalar contributor in order until `visit` returns false.
    fn scalars(&self, text: &FormattedTextTree,
        mut visit: impl FnMut(Range<usize>, Range<usize>) -> bool) {
        if self.rules & ASCII != 0 {
            let width = self.source.len() / self.formatted.len();
            for index in 0..self.formatted.len() {
                let formatted = self.formatted.start + index..self.formatted.start + index + 1;
                let source = self.source.start + index * width..self.source.start + (index + 1) * width;
                if !visit(formatted, source) {
                    return;
                }
            }
            return;
        }
        let Ok(slice) = text.slice(self.formatted.clone()) else {
            return;
        };
        let mut source = self.source.start;
        for (offset, ch) in slice.char_indices() {
            let start = self.formatted.start + offset;
            let width = source_width(self.rules, ch);
            if !visit(start..start + ch.len_utf8(), source..source + width) {
                return;
            }
            source += width;
        }
    }

    /// Split a divisible run at an interior scalar boundary.
    fn split_at(&self, at: usize, text: &FormattedTextTree) -> Option<(Self, Self)> {
        let mut middle = None;
        self.scalars(text, |formatted, source| {
            if formatted.start == at {
                middle = Some(source.start);
                return false;
            }
            true
        });
        let middle = middle?;
        let rules = self.rules;
        Some((
            Self { formatted: self.formatted.start..at, source: self.source.start..middle, rules },
            Self { formatted: at..self.formatted.end, source: middle..self.source.end, rules },
        ))
    }
}

impl RangedItem for ProvenanceRun {
    fn range(&self) -> &Range<usize> {
        &self.formatted
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self { formatted: range, ..self.clone() }
    }

    fn with_transform(&self, range: Range<usize>, auxiliary_shift: i128, _revision: Option<u64>)
        -> Option<Self> {
        Some(Self { formatted: range, source: super::shift_range_i128(&self.source, auxiliary_shift)?,
            rules: self.rules })
    }
}

/// Reverse lookup keyed by source position. A point records one contributor
/// edge; a divisible run records every scalar edge it contains, with both
/// sides at interior edges.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryItem {
    source: Range<usize>,
    formatted: Range<usize>,
    /// `None` for a divisible run.
    side: Option<Side>,
    rules: u8,
}

impl BoundaryItem {
    fn order(&self) -> (usize, usize, bool) {
        (self.source.start, self.formatted.start, !matches!(self.side, Some(Side::Upstream)))
    }

    fn run(&self) -> ProvenanceRun {
        ProvenanceRun { formatted: self.formatted.clone(), source: self.source.clone(), rules: self.rules }
    }

    /// Every boundary whose source position satisfies `keep`.
    fn expand(&self, text: &FormattedTextTree, keep: &impl Fn(usize) -> bool,
        output: &mut Vec<SourceTextBoundary>) {
        let Some(side) = self.side else {
            let end = self.source.end;
            self.run().scalars(text, |formatted, source| {
                if keep(source.start) {
                    output.push(SourceTextBoundary { source: source.start..source.start,
                        formatted: formatted.start, side: Side::Downstream });
                }
                if keep(source.end) {
                    output.push(SourceTextBoundary { source: source.end..source.end,
                        formatted: formatted.end, side: Side::Upstream });
                }
                source.end < end
            });
            return;
        };
        if keep(self.source.start) {
            output.push(SourceTextBoundary { source: self.source.clone(),
                formatted: self.formatted.start, side });
        }
    }

    /// Scalar edges of a run in source order, as `(source, formatted)`.
    fn edges(&self, text: &FormattedTextTree, mut visit: impl FnMut(usize, usize) -> bool) {
        if self.side.is_some() {
            visit(self.source.start, self.formatted.start);
            return;
        }
        if !visit(self.source.start, self.formatted.start) {
            return;
        }
        self.run().scalars(text, |formatted, source| visit(source.end, formatted.end));
    }

    pub(super) fn transformed_eq(&self, other: &Self, source_shift: i128, formatted_shift: i128) -> bool {
        self.side == other.side && self.rules == other.rules
            && super::shift_range_i128(&self.source, source_shift).as_ref() == Some(&other.source)
            && super::shift_range_i128(&self.formatted, formatted_shift).as_ref() == Some(&other.formatted)
    }
}

impl RangedItem for BoundaryItem {
    fn range(&self) -> &Range<usize> {
        &self.source
    }

    fn with_range(&self, range: Range<usize>) -> Self {
        Self { source: range, ..self.clone() }
    }

    fn with_transform(&self, range: Range<usize>, auxiliary_shift: i128, _revision: Option<u64>)
        -> Option<Self> {
        Some(Self { source: range, formatted: super::shift_range_i128(&self.formatted, auxiliary_shift)?,
            side: self.side, rules: self.rules })
    }
}

/// Boundary items for stored contributors, in canonical order.
pub(super) fn boundary_items(runs: impl IntoIterator<Item = ProvenanceRun>) -> Vec<BoundaryItem> {
    let mut items = Vec::new();
    for run in runs {
        if run.is_divisible() {
            items.push(BoundaryItem { source: run.source, formatted: run.formatted, side: None, rules: run.rules });
        } else {
            items.push(BoundaryItem { source: run.source.start..run.source.start,
                formatted: run.formatted.start..run.formatted.start, side: Some(Side::Downstream), rules: 0 });
            items.push(BoundaryItem { source: run.source.end..run.source.end,
                formatted: run.formatted.end..run.formatted.end, side: Some(Side::Upstream), rules: 0 });
        }
    }
    normalize_boundary_items(&mut items);
    items
}

pub(super) fn normalize_boundary_items(items: &mut Vec<BoundaryItem>) {
    items.sort_by_key(BoundaryItem::order);
    items.dedup();
}

/// Group single-scalar contributors into bounded runs. Runs merge only
/// contributors that are adjacent in both coordinates and in list order, so
/// expansion reproduces them exactly. No other contributor may start inside
/// a run, which keeps start-ordered query output identical.
pub(super) fn compress(spans: Vec<ProvenanceSpan>, text: &str) -> Vec<ProvenanceRun> {
    let mut runs: Vec<ProvenanceRun> = Vec::with_capacity(spans.len() / 8 + 1);
    let mut starts = Vec::new();
    for span in spans {
        let rules = text.get(span.formatted.clone()).and_then(|value| {
            let mut chars = value.chars();
            let ch = chars.next()?;
            (chars.next().is_none() && ch != '\n').then(|| {
                let rules = width_rules(ch, span.source.len());
                if rules != 0 && ch.is_ascii() { rules | ASCII } else { rules }
            })
        }).unwrap_or(0);
        if rules != 0 {
            if let Some(run) = runs.last_mut().filter(|run| run.is_divisible()
                && run.formatted.end == span.formatted.start && run.source.end == span.source.start
                && run.rules & rules & WIDTHS != 0
                && run.formatted.len() + span.formatted.len() <= RUN_BYTES)
            {
                run.formatted.end = span.formatted.end;
                run.source.end = span.source.end;
                run.rules &= (rules & WIDTHS) | ASCII;
                if rules & ASCII == 0 {
                    run.rules &= WIDTHS;
                }
                continue;
            }
            runs.push(ProvenanceRun { formatted: span.formatted, source: span.source, rules });
        } else {
            starts.push(span.formatted.start);
            runs.push(ProvenanceRun::indivisible(span));
        }
    }
    // Out-of-order construction can place another contributor's start inside
    // a run; split there so stable start ordering is unchanged.
    starts.sort_unstable();
    starts.dedup();
    if starts.is_empty() || !runs.iter().any(|run| run.is_divisible()
        && starts.binary_search_by(|at| if *at <= run.formatted.start { std::cmp::Ordering::Less }
            else if *at < run.formatted.end { std::cmp::Ordering::Equal } else { std::cmp::Ordering::Greater }).is_ok())
    {
        return runs;
    }
    let tree = FormattedTextTree::try_from_text(text).expect("projected text is valid");
    let mut result = Vec::with_capacity(runs.len());
    for mut run in runs {
        if run.is_divisible() {
            let first = starts.partition_point(|at| *at <= run.formatted.start);
            for &at in starts[first..].iter().take_while(|at| **at < run.formatted.end) {
                if let Some((left, right)) = run.split_at(at, &tree) {
                    result.push(left);
                    run = right;
                }
            }
        }
        result.push(run);
    }
    result
}

/// Contributors for a projection, in formatted order, with a source-keyed
/// boundary index. Literal projections keep their own encoding runs as
/// indivisible items.
#[derive(Clone)]
pub(super) struct ProvenanceIndex {
    pub(super) runs: IntervalRangeStore<ProvenanceRun>,
    pub(super) boundaries: IntervalRangeStore<BoundaryItem>,
}

impl ProvenanceIndex {
    pub(super) fn new(spans: Vec<ProvenanceSpan>, text: &str, divisible: bool) -> Self {
        let runs = if divisible {
            compress(spans, text)
        } else {
            spans.into_iter().map(ProvenanceRun::indivisible).collect()
        };
        Self::from_runs(runs)
    }

    pub(super) fn from_runs(runs: Vec<ProvenanceRun>) -> Self {
        let boundaries = IntervalRangeStore::new(boundary_items(runs.iter().cloned()));
        Self { runs: IntervalRangeStore::new(runs), boundaries }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub(super) fn visit_retained_memory(&self, visitor: &mut crate::document::history_memory::MemoryVisitor<'_>) {
        self.runs.visit_retained_memory(visitor);
        self.boundaries.visit_retained_memory(visitor);
    }

    /// Contributors with a non-empty intersection with `query`.
    pub(super) fn query_overlapping(&self, query: &Range<usize>, text: &FormattedTextTree) -> Vec<ProvenanceSpan> {
        let keep = |range: &Range<usize>| range.start < query.end && query.start < range.end;
        let mut output = Vec::new();
        for run in self.runs.query_overlapping(query) {
            run.expand(text, &keep, &mut output);
        }
        output
    }

    /// Contributors whose closed extent touches `query`.
    pub(super) fn query_touching(&self, query: &Range<usize>, text: &FormattedTextTree) -> Vec<ProvenanceSpan> {
        let keep = |range: &Range<usize>| range.end >= query.start && range.start <= query.end;
        let mut output = Vec::new();
        for run in self.runs.query_touching(query) {
            run.expand(text, &keep, &mut output);
        }
        output
    }

    /// Every contributor in formatted order, expanded lazily.
    pub(super) fn spans<'a>(&'a self, text: &'a FormattedTextTree) -> impl Iterator<Item = ProvenanceSpan> + 'a {
        let mut runs = self.runs.as_slice().iter();
        let mut pending = std::collections::VecDeque::new();
        std::iter::from_fn(move || loop {
            if let Some(span) = pending.pop_front() {
                return Some(span);
            }
            let run = runs.next()?;
            if !run.is_divisible() {
                return Some(run.hull());
            }
            let mut expanded = Vec::new();
            run.expand(text, &|_| true, &mut expanded);
            pending.extend(expanded);
        })
    }

    /// Stored contributors in formatted order; divisible runs appear whole.
    pub(super) fn stored(&self) -> &[ProvenanceRun] {
        self.runs.as_slice()
    }

    /// Source boundaries touching `query`, in canonical order.
    pub(super) fn boundaries_touching(&self, query: &Range<usize>, text: &FormattedTextTree) -> Vec<SourceTextBoundary> {
        let keep = |at: usize| query.start <= at && at <= query.end;
        let mut output = Vec::new();
        for item in self.boundaries.query_touching(query) {
            item.expand(text, &keep, &mut output);
        }
        super::normalize_source_boundaries(&mut output);
        output
    }

    /// The least boundary source position at or after `source`.
    pub(super) fn following_boundary(&self, source: usize, text: &FormattedTextTree) -> Option<usize> {
        let mut best = self.boundaries.get(self.boundaries.partition_point_start(source))
            .map(|item| item.source.start);
        for item in self.boundaries.query_touching(&(source..source)) {
            item.edges(text, |at, _| {
                if at >= source {
                    best = Some(best.map_or(at, |best| best.min(at)));
                    return false;
                }
                true
            });
        }
        best
    }

    /// The greatest boundary source position before `source`.
    pub(super) fn preceding_boundary(&self, source: usize, text: &FormattedTextTree) -> Option<usize> {
        let last = self.boundaries.partition_point_start(source).checked_sub(1)?;
        let last_item = self.boundaries.get(last)?;
        let mut best: Option<usize> = None;
        let mut consider = |item: &BoundaryItem| {
            item.edges(text, |at, _| {
                if at < source {
                    best = Some(best.map_or(at, |best| best.max(at)));
                    true
                } else {
                    false
                }
            });
        };
        consider(&last_item);
        for item in self.boundaries.query_touching(&(last_item.source.start..source)) {
            if item.source.start < source {
                consider(&item);
            }
        }
        best
    }
}

impl PartialEq for ProvenanceIndex {
    fn eq(&self, other: &Self) -> bool {
        self.runs == other.runs
    }
}

impl std::fmt::Debug for ProvenanceIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.runs.as_slice()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_spans(text: &str, widths: impl Fn(char) -> usize) -> Vec<ProvenanceSpan> {
        let mut source = 0;
        text.char_indices().map(|(offset, ch)| {
            let width = widths(ch);
            let span = ProvenanceSpan { formatted: offset..offset + ch.len_utf8(), source: source..source + width };
            source += width;
            span
        }).collect()
    }

    fn reference_boundaries(spans: &[ProvenanceSpan], query: &Range<usize>) -> Vec<SourceTextBoundary> {
        let mut all = super::super::source_text_boundaries(spans.iter().cloned());
        all.retain(|boundary| query.start <= boundary.source.start && boundary.source.start <= query.end);
        all
    }

    #[test]
    fn expansion_reproduces_scalar_contributors_and_boundaries() {
        let text = "plain ascii é 👍🏽 text\nnext line &amp; x";
        let encodings: [&dyn Fn(char) -> usize; 3] = [&|ch: char| ch.len_utf8(), &|ch: char| 2 * ch.len_utf16(),
            &|ch: char| if u32::from(ch) <= 0xff { 1 } else { ch.len_utf8() }];
        for widths in encodings {
            let mut spans = scalar_spans(text, widths);
            // An indivisible entity-like contributor and a caret seed.
            spans.insert(5, ProvenanceSpan { formatted: 5..5, source: spans[5].source.start..spans[5].source.start });
            let index = ProvenanceIndex::new(spans.clone(), text, true);
            let tree = FormattedTextTree::try_from_text(text).unwrap();
            assert!(index.runs.len() < spans.len());
            let mut flat = spans.clone();
            flat.sort_by_key(|span| span.formatted.start);
            assert_eq!(index.spans(&tree).collect::<Vec<_>>(), flat);
            for start in 0..=text.len() {
                for end in start..=text.len().min(start + 12) {
                    let query = start..end;
                    let overlapping = flat.iter().filter(|span| span.formatted.start < end && start < span.formatted.end)
                        .cloned().collect::<Vec<_>>();
                    let touching = flat.iter().filter(|span| span.formatted.end >= start && span.formatted.start <= end)
                        .cloned().collect::<Vec<_>>();
                    assert_eq!(index.query_overlapping(&query, &tree), overlapping, "{query:?}");
                    assert_eq!(index.query_touching(&query, &tree), touching, "{query:?}");
                }
            }
            let source_len = spans.iter().map(|span| span.source.end).max().unwrap();
            let all = super::super::source_text_boundaries(spans.iter().cloned());
            for source in 0..=source_len + 1 {
                for end in source..=(source + 6).min(source_len + 1) {
                    assert_eq!(index.boundaries_touching(&(source..end), &tree),
                        reference_boundaries(&spans, &(source..end)), "{source}..{end}");
                }
                let following = all.iter().map(|b| b.source.start).filter(|at| *at >= source).min();
                let preceding = all.iter().map(|b| b.source.start).filter(|at| *at < source).max();
                assert_eq!(index.following_boundary(source, &tree), following, "{source}");
                assert_eq!(index.preceding_boundary(source, &tree), preceding, "{source}");
            }
        }
    }

    #[test]
    fn runs_stop_at_line_breaks_and_their_byte_bound() {
        let text = format!("{}\n{}", "a".repeat(RUN_BYTES * 2 + 3), "b".repeat(10));
        let spans = scalar_spans(&text, |ch| ch.len_utf8());
        let index = ProvenanceIndex::new(spans, &text, true);
        let runs = index.stored();
        assert!(runs.iter().all(|run| run.formatted.len() <= RUN_BYTES));
        assert!(runs.iter().all(|run| !run.is_divisible() || !run.formatted.contains(&(RUN_BYTES * 2 + 3))));
        assert_eq!(runs.len(), 5);
    }
}
