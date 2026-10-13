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
    fn auxiliary_range(&self) -> Option<&Range<usize>> { Some(&self.source) }

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

/// Reverse lookup keyed by source range. An indivisible contributor records
/// its start and end edges; a divisible run records every scalar edge it
/// contains, with both sides at interior edges. Either item covers its whole
/// source range, so positions inside a contributor are found by the index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryItem {
    pub(super) source: Range<usize>,
    pub(super) formatted: Range<usize>,
    /// Divisible-run width rules; zero for an indivisible contributor.
    rules: u8,
}

impl BoundaryItem {
    fn order(&self) -> (usize, usize, usize, usize) {
        (self.source.start, self.formatted.start, self.source.end, self.formatted.end)
    }

    fn run(&self) -> ProvenanceRun {
        ProvenanceRun { formatted: self.formatted.clone(), source: self.source.clone(), rules: self.rules }
    }

    /// Every boundary whose source position satisfies `keep`.
    fn expand(&self, text: &FormattedTextTree, keep: &impl Fn(usize) -> bool,
        output: &mut Vec<SourceTextBoundary>) {
        self.edges(text, |source, formatted, side| {
            if keep(source) {
                output.push(SourceTextBoundary { source: source..source, formatted, side });
            }
            true
        });
    }

    /// Edges in source order as `(source, formatted, side)`, until `visit`
    /// returns false. A divisible run's interior edges have both sides.
    fn edges(&self, text: &FormattedTextTree, mut visit: impl FnMut(usize, usize, Side) -> bool) {
        if self.rules == 0 {
            if visit(self.source.start, self.formatted.start, Side::Downstream) {
                visit(self.source.end, self.formatted.end, Side::Upstream);
            }
            return;
        }
        let end = self.source.end;
        self.run().scalars(text, |formatted, source| {
            visit(source.start, formatted.start, Side::Downstream)
                && visit(source.end, formatted.end, Side::Upstream)
                && source.end < end
        });
    }

    /// The contributor whose source strictly contains `source`: the whole
    /// item, or the scalar of a divisible run.
    fn containing(&self, source: usize, text: &FormattedTextTree) -> Option<ProvenanceSpan> {
        if !(self.source.start < source && source < self.source.end) {
            return None;
        }
        if self.rules == 0 {
            return Some(ProvenanceSpan { formatted: self.formatted.clone(), source: self.source.clone() });
        }
        let mut found = None;
        self.run().scalars(text, |formatted, scalar| {
            let before = scalar.end < source;
            if scalar.start < source && source < scalar.end {
                found = Some(ProvenanceSpan { formatted, source: scalar });
            }
            found.is_none() && before
        });
        found
    }

    pub(super) fn transformed_eq(&self, other: &Self, source_shift: i128, formatted_shift: i128) -> bool {
        self.rules == other.rules
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
            rules: self.rules })
    }
}

/// Boundary items for stored contributors, in canonical order.
pub(super) fn boundary_items(runs: impl IntoIterator<Item = ProvenanceRun>) -> Vec<BoundaryItem> {
    let mut items: Vec<_> = runs.into_iter()
        .map(|run| BoundaryItem { source: run.source, formatted: run.formatted, rules: run.rules })
        .collect();
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
    let mut runs = RunBuilder::default();
    runs.runs.reserve(spans.len() / 8 + 1);
    for span in spans {
        runs.push(span, text);
    }
    runs.finish(text).0
}

/// Incremental `compress` for adapters that emit contributors in text order.
/// The most recent contributor stays separate until the next one arrives, so
/// an adapter can still take it back.
#[derive(Default)]
pub(super) struct RunBuilder {
    runs: Vec<ProvenanceRun>,
    pending: Option<ProvenanceSpan>,
    starts: Vec<usize>,
    previous_source: Option<usize>,
    source_unordered: bool,
}

impl RunBuilder {
    /// Append a contributor. `text` must contain the previously pushed one.
    pub(super) fn push(&mut self, span: ProvenanceSpan, text: &str) {
        self.flush(text);
        self.pending = Some(span);
    }

    /// Remove the most recently pushed contributor.
    pub(super) fn pop(&mut self) {
        if self.pending.take().is_none() && self.runs.last().is_some_and(|run| !run.is_divisible()) {
            self.runs.pop();
        }
    }

    /// Store the pending contributor and return a position after which
    /// `indivisible_since` reports newly stored indivisible contributors.
    pub(super) fn mark(&mut self, text: &str) -> usize {
        self.flush(text);
        self.runs.len()
    }

    /// Indivisible contributors pushed since `mark`. Every contributor that
    /// is not one non-newline scalar mapping onto its own encoding is one.
    pub(super) fn indivisible_since(&self, mark: usize) -> impl Iterator<Item = ProvenanceSpan> + '_ {
        self.runs[mark.min(self.runs.len())..].iter().filter(|run| !run.is_divisible()).map(ProvenanceRun::hull)
            .chain(self.pending.iter().cloned())
    }


    fn flush(&mut self, text: &str) {
        let Some(span) = self.pending.take() else { return; };
        if !span.formatted.is_empty() || !span.source.is_empty() {
            // A caret seed contributes no character or source bytes. Its
            // point can lie inside the following break's contributor.
            self.source_unordered |= self.previous_source.is_some_and(|before| before > span.source.start);
            self.previous_source = Some(span.source.start);
        }
        let rules = text.get(span.formatted.clone()).and_then(|value| {
            let mut chars = value.chars();
            let ch = chars.next()?;
            (chars.next().is_none() && ch != '\n').then(|| {
                let rules = width_rules(ch, span.source.len());
                if rules != 0 && ch.is_ascii() { rules | ASCII } else { rules }
            })
        }).unwrap_or(0);
        if rules != 0 {
            if let Some(run) = self.runs.last_mut().filter(|run| run.is_divisible()
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
                return;
            }
            self.runs.push(ProvenanceRun { formatted: span.formatted, source: span.source, rules });
        } else {
            self.starts.push(span.formatted.start);
            self.runs.push(ProvenanceRun::indivisible(span));
        }
    }

    /// The stored runs, and whether contributor source starts never
    /// decrease (ignoring caret seeds).
    pub(super) fn finish(mut self, text: &str) -> (Vec<ProvenanceRun>, bool) {
        self.flush(text);
        let ordered = !self.source_unordered;
        (self.split_runs(text), ordered)
    }

    fn split_runs(self, text: &str) -> Vec<ProvenanceRun> {
        let (runs, mut starts) = (self.runs, self.starts);
        // Out-of-order construction can place another contributor's start
        // inside a run; split there so stable start ordering is unchanged.
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
                let end = run.formatted.end;
                for &at in starts[first..].iter().take_while(|at| **at < end) {
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
}

/// Contributors for a projection, in formatted order. Monotonic source
/// projections reverse-query the same tree; reordered contributors retain a
/// separate source-keyed index. Literal encoding runs remain indivisible.
#[derive(Clone)]
pub(super) struct ProvenanceIndex {
    pub(super) runs: IntervalRangeStore<ProvenanceRun>,
    pub(super) boundaries: Option<IntervalRangeStore<BoundaryItem>>,
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

    pub(super) fn from_runs(mut runs: Vec<ProvenanceRun>) -> Self {
        runs.sort_by_key(|run| run.formatted.start);
        let ordered = runs.windows(2).all(|pair| pair[0].source.start <= pair[1].source.start);
        let boundaries = (!ordered).then(|| IntervalRangeStore::new(boundary_items(runs.iter().cloned())));
        Self { runs: IntervalRangeStore::new(runs), boundaries }
    }

    pub(super) fn boundary_partition_start(&self, source: usize) -> usize {
        self.boundaries.as_ref().map_or_else(|| self.runs.partition_point_auxiliary_start(source),
            |boundaries| boundaries.partition_point_start(source))
    }

    pub(super) fn boundary_get(&self, index: usize) -> Option<BoundaryItem> {
        match &self.boundaries {
            Some(boundaries) => boundaries.get(index),
            None => self.runs.get(index).map(|run| BoundaryItem {
                source: run.source, formatted: run.formatted, rules: run.rules,
            }),
        }
    }

    fn boundary_query(&self, query: &Range<usize>) -> Vec<BoundaryItem> {
        match &self.boundaries {
            Some(boundaries) => boundaries.query_touching(query),
            None => self.runs.query_auxiliary_touching(query).into_iter().map(|run| BoundaryItem {
                source: run.source, formatted: run.formatted, rules: run.rules,
            }).collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn boundary_query_work(&self, query: &Range<usize>) -> (usize, usize) {
        let stats = match &self.boundaries {
            Some(boundaries) => boundaries.query_overlapping_with_stats(query).1,
            None => self.runs.query_auxiliary_touching_with_stats(query).1,
        };
        (stats.nodes_visited, stats.items_examined)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub(super) fn visit_retained_memory(&self, visitor: &mut crate::document::history_memory::MemoryVisitor<'_>) {
        self.runs.visit_retained_memory(visitor);
        if let Some(boundaries) = &self.boundaries { boundaries.visit_retained_memory(visitor); }
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
        let mut runs = self.runs.items();
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
    pub(super) fn stored(&self) -> impl Iterator<Item = ProvenanceRun> + '_ {
        self.runs.items()
    }

    /// Source boundaries touching `query`, in canonical order.
    pub(super) fn boundaries_touching(&self, query: &Range<usize>, text: &FormattedTextTree) -> Vec<SourceTextBoundary> {
        let keep = |at: usize| query.start <= at && at <= query.end;
        let mut output = Vec::new();
        for item in self.boundary_query(query) {
            item.expand(text, &keep, &mut output);
        }
        super::normalize_source_boundaries(&mut output);
        output
    }

    /// The first contributor, in formatted order, whose source strictly
    /// contains `source`.
    pub(super) fn containing_source(&self, source: usize, text: &FormattedTextTree) -> Option<ProvenanceSpan> {
        self.boundary_query(&(source..source)).into_iter()
            .filter_map(|item| item.containing(source, text))
            .min_by_key(|span| (span.formatted.start, span.formatted.end))
    }

    /// For a source position that no contributor contains or bounds, the
    /// nearest contributor end before it and start after it, as `(source,
    /// formatted)`. Exact for projections whose contributor sources are in
    /// formatted order.
    pub(super) fn uncovered_neighbors(&self, source: usize) -> (Option<(usize, usize)>, Option<(usize, usize)>) {
        let index = self.boundary_partition_start(source);
        let following = self.boundary_get(index).map(|item| (item.source.start, item.formatted.start));
        let preceding = index.checked_sub(1).and_then(|last| self.boundary_get(last)).and_then(|last| {
            // An earlier, longer contributor can end after the last start.
            std::iter::once(last.clone())
                .chain(self.boundary_query(&(last.source.start..source)))
                .filter(|item| item.source.start < source && item.source.end <= source)
                .map(|item| (item.source.end, item.formatted.end))
                .min_by_key(|&(end, formatted)| (std::cmp::Reverse(end), formatted))
        });
        (preceding, following)
    }

    /// The least boundary source position at or after `source`.
    pub(super) fn following_boundary(&self, source: usize, text: &FormattedTextTree) -> Option<usize> {
        let mut best = self.boundary_get(self.boundary_partition_start(source))
            .map(|item| item.source.start);
        for item in self.boundary_query(&(source..source)) {
            item.edges(text, |at, _, _| {
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
        let last = self.boundary_partition_start(source).checked_sub(1)?;
        let last_item = self.boundary_get(last)?;
        let mut best: Option<usize> = None;
        let mut consider = |item: &BoundaryItem| {
            item.edges(text, |at, _, _| {
                if at < source {
                    best = Some(best.map_or(at, |best| best.max(at)));
                    true
                } else {
                    false
                }
            });
        };
        consider(&last_item);
        for item in self.boundary_query(&(last_item.source.start..source)) {
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
        let runs: Vec<_> = index.stored().collect();
        assert!(runs.iter().all(|run| run.formatted.len() <= RUN_BYTES));
        assert!(runs.iter().all(|run| !run.is_divisible() || !run.formatted.contains(&(RUN_BYTES * 2 + 3))));
        assert_eq!(runs.len(), 5);
    }
}
