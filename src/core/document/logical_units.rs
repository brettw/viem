//! Run-length storage for normalized text's scalar-to-source mapping.
//!
//! Rich parsers address one logical unit per normalized scalar, but nearly
//! every scalar maps onto exactly its own encoded source bytes. Such units are
//! stored as bounded runs and expanded on demand, so every query returns the
//! units a per-scalar list would while ordinary text costs a few bytes per run.
//! Units whose source width differs from their scalar's encoded width (CRLF,
//! decoding diagnostics, adapter replacements) remain explicit single units.
use super::super::encoding::{DecodingDiagnosticKind, MAPPING_CHUNK_BYTES};
use super::{LogicalUnit, NormalizedText};
use std::ops::Range;

/// Normalized bytes per non-ASCII run for rich parsers. Ordinal lookups scan
/// such a run from its start; ASCII runs, and UTF-8 offset lookups, map
/// arithmetically. Literal adapters address runs, not scalars, and keep the
/// conversion chunk bound instead.
const RICH_SCALAR_RUN_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunKind {
    /// ASCII scalars: one normalized byte and one equal source width each.
    Ascii,
    /// Scalars each mapping onto `Encoding::scalar_source_width` bytes.
    Scalars,
    /// One explicit unit.
    Single(Option<DecodingDiagnosticKind>),
}

#[derive(Clone, Debug)]
struct UnitRun {
    /// Ordinal of the run's first unit.
    first: usize,
    normalized: Range<usize>,
    source: Range<usize>,
    kind: RunKind,
}

#[derive(Clone, Debug)]
pub(crate) struct LogicalUnits {
    runs: Vec<UnitRun>,
    len: usize,
    scalar_run_bytes: usize,
}

impl Default for LogicalUnits {
    fn default() -> Self {
        Self { runs: Vec::new(), len: 0, scalar_run_bytes: RICH_SCALAR_RUN_BYTES }
    }
}

impl LogicalUnits {
    pub(super) fn literal() -> Self {
        Self { scalar_run_bytes: MAPPING_CHUNK_BYTES, ..Self::default() }
    }

    fn count(&self, index: usize) -> usize {
        self.runs.get(index + 1).map_or(self.len, |next| next.first) - self.runs[index].first
    }

    /// The run containing a normalized offset.
    fn run_at(&self, at: usize) -> Option<usize> {
        let index = self.runs.partition_point(|run| run.normalized.start <= at).checked_sub(1)?;
        (at < self.runs[index].normalized.end).then_some(index)
    }
}

impl UnitRun {
    fn single(&self) -> LogicalUnit {
        let RunKind::Single(decoding_diagnostic) = self.kind else { unreachable!() };
        LogicalUnit { normalized: self.normalized.clone(), source: self.source.clone(), decoding_diagnostic }
    }

    fn ascii(&self, offset: usize) -> LogicalUnit {
        let width = self.source.len() / self.normalized.len();
        let normalized = self.normalized.start + offset;
        let source = self.source.start + offset * width;
        LogicalUnit { normalized: normalized..normalized + 1, source: source..source + width, decoding_diagnostic: None }
    }
}

impl NormalizedText {
    #[cfg(test)]
    pub(crate) fn unit_count(&self) -> usize {
        self.units.len
    }

    pub(crate) fn has_units(&self) -> bool {
        self.units.len != 0
    }

    /// Visit the units of one run from its start until `visit` returns false.
    fn run_units(&self, run: &UnitRun, mut visit: impl FnMut(LogicalUnit) -> bool) {
        match run.kind {
            RunKind::Single(_) => { visit(run.single()); }
            RunKind::Ascii => {
                for offset in 0..run.normalized.len() {
                    if !visit(run.ascii(offset)) { return; }
                }
            }
            RunKind::Scalars => {
                let mut source = run.source.start;
                for (offset, ch) in self.text[run.normalized.clone()].char_indices() {
                    let normalized = run.normalized.start + offset;
                    let width = self.encoding.scalar_source_width(ch);
                    if !visit(LogicalUnit {
                        normalized: normalized..normalized + ch.len_utf8(),
                        source: source..source + width,
                        decoding_diagnostic: None,
                    }) { return; }
                    source += width;
                }
            }
        }
    }

    fn run_unit(&self, run: &UnitRun, offset: usize) -> LogicalUnit {
        match run.kind {
            RunKind::Single(_) => run.single(),
            RunKind::Ascii => run.ascii(offset),
            RunKind::Scalars => {
                let mut found = None;
                let mut remaining = offset;
                self.run_units(run, |unit| {
                    if remaining == 0 { found = Some(unit); return false; }
                    remaining -= 1;
                    true
                });
                found.expect("unit ordinal inside its run")
            }
        }
    }

    pub(crate) fn unit(&self, index: usize) -> Option<LogicalUnit> {
        if index >= self.units.len { return None; }
        let run = self.units.runs.partition_point(|run| run.first <= index) - 1;
        let run = &self.units.runs[run];
        Some(self.run_unit(run, index - run.first))
    }

    pub(crate) fn first_unit(&self) -> Option<LogicalUnit> {
        self.unit(0)
    }

    pub(crate) fn last_unit(&self) -> Option<LogicalUnit> {
        let run = self.units.runs.last()?;
        Some(match run.kind {
            RunKind::Single(_) => run.single(),
            RunKind::Ascii => run.ascii(run.normalized.len() - 1),
            RunKind::Scalars => {
                let (offset, ch) = self.text[run.normalized.clone()].char_indices().next_back().unwrap();
                let width = self.encoding.scalar_source_width(ch);
                LogicalUnit {
                    normalized: run.normalized.start + offset..run.normalized.end,
                    source: run.source.end - width..run.source.end,
                    decoding_diagnostic: None,
                }
            }
        })
    }

    /// `slice::partition_point` over the expanded units: the ordinal of the
    /// first unit for which a monotone `pred` is false.
    pub(crate) fn unit_partition_point(&self, mut pred: impl FnMut(&LogicalUnit) -> bool) -> usize {
        let runs = &self.units.runs;
        let following = runs.partition_point(|run| pred(&self.run_unit(run, 0)));
        let Some(index) = following.checked_sub(1) else { return 0; };
        let run = &runs[index];
        let count = self.units.count(index);
        match run.kind {
            RunKind::Single(_) => run.first + 1,
            RunKind::Ascii => {
                let mut low = 1;
                let mut high = count;
                while low < high {
                    let middle = low + (high - low) / 2;
                    if pred(&run.ascii(middle)) { low = middle + 1; } else { high = middle; }
                }
                run.first + low
            }
            RunKind::Scalars => {
                let mut offset = 0;
                self.run_units(run, |unit| {
                    if offset > 0 && !pred(&unit) { return false; }
                    offset += 1;
                    true
                });
                run.first + offset
            }
        }
    }

    /// The unit whose normalized range starts exactly at `at`.
    pub(crate) fn unit_starting_at(&self, at: usize) -> Option<LogicalUnit> {
        let run = &self.units.runs[self.units.run_at(at)?];
        match run.kind {
            RunKind::Single(_) => (run.normalized.start == at).then(|| run.single()),
            RunKind::Ascii => Some(run.ascii(at - run.normalized.start)),
            RunKind::Scalars => {
                if !self.text.is_char_boundary(at) { return None; }
                let ch = self.text[at..].chars().next()?;
                let width = self.encoding.scalar_source_width(ch);
                let source = if self.encoding == super::Encoding::Utf8 {
                    run.source.start + (at - run.normalized.start)
                } else {
                    run.source.start + self.encoding.encoded_text_len(&self.text[run.normalized.start..at])
                };
                Some(LogicalUnit { normalized: at..at + ch.len_utf8(), source: source..source + width, decoding_diagnostic: None })
            }
        }
    }

    /// The first unit whose normalized range starts at or after `at`.
    pub(crate) fn unit_at_or_after(&self, at: usize) -> Option<LogicalUnit> {
        self.unit(self.unit_partition_point(|unit| unit.normalized.start < at))
    }

    /// The first unit whose source range starts at or after `at`.
    pub(crate) fn unit_at_or_after_source(&self, at: usize) -> Option<LogicalUnit> {
        self.unit(self.unit_partition_point(|unit| unit.source.start < at))
    }

    /// The unit whose source range starts exactly at `at`. Units are in
    /// source order, so this is the first such unit.
    pub(crate) fn unit_with_source_start(&self, at: usize) -> Option<LogicalUnit> {
        self.unit_at_or_after_source(at).filter(|unit| unit.source.start == at)
    }

    /// The source start of the first unit at or after a normalized offset,
    /// or the end of the last unit (zero without units).
    pub(crate) fn source_at(&self, at: usize) -> usize {
        self.unit_at_or_after(at)
            .map_or_else(|| self.last_unit().map_or(0, |unit| unit.source.end), |unit| unit.source.start)
    }

    /// The normalized start of the first unit at or after a source offset,
    /// or the end of the text.
    pub(crate) fn normalized_at(&self, at: usize) -> usize {
        self.unit_at_or_after_source(at).map_or(self.text.len(), |unit| unit.normalized.start)
    }

    /// The source hull of the units overlapping a nonempty normalized range.
    pub(crate) fn source_hull(&self, range: &Range<usize>) -> Option<Range<usize>> {
        let first = self.unit_partition_point(|unit| unit.normalized.end <= range.start);
        let end = self.unit_partition_point(|unit| unit.normalized.start < range.end);
        (first < end).then(|| self.unit(first).unwrap().source.start..self.unit(end - 1).unwrap().source.end)
    }

    /// Units in order, starting at an ordinal.
    pub(crate) fn units_from(&self, index: usize) -> Units<'_> {
        let mut units = Units { input: self, run: self.units.runs.len(), at: 0, source: 0 };
        if index < self.units.len {
            let run_index = self.units.runs.partition_point(|run| run.first <= index) - 1;
            let unit = self.run_unit(&self.units.runs[run_index], index - self.units.runs[run_index].first);
            units.run = run_index;
            units.at = unit.normalized.start;
            units.source = unit.source.start;
        }
        units
    }

    pub(crate) fn units(&self) -> Units<'_> {
        self.units_from(0)
    }

    /// The stored runs as units. Literal adapters use these coarse units;
    /// each covers contiguous scalars that map onto their own encoded bytes,
    /// or is one explicit unit.
    pub(crate) fn unit_runs(&self) -> impl Iterator<Item = LogicalUnit> + '_ {
        self.units.runs.iter().map(|run| LogicalUnit {
            normalized: run.normalized.clone(),
            source: run.source.clone(),
            decoding_diagnostic: match run.kind { RunKind::Single(kind) => kind, _ => None },
        })
    }

    /// Append the unit for normalized text already appended to `text`.
    pub(crate) fn push_unit(&mut self, unit: LogicalUnit) {
        let kind = if unit.decoding_diagnostic.is_some() {
            RunKind::Single(unit.decoding_diagnostic)
        } else {
            let mut chars = self.text[unit.normalized.clone()].chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if unit.source.len() == self.encoding.scalar_source_width(ch) =>
                    if ch.is_ascii() { RunKind::Ascii } else { RunKind::Scalars },
                _ => RunKind::Single(None),
            }
        };
        let units = &mut self.units;
        units.len += 1;
        if !matches!(kind, RunKind::Single(_)) {
            let scalar_run_bytes = units.scalar_run_bytes;
            if let Some(run) = units.runs.last_mut().filter(|run| {
                !matches!(run.kind, RunKind::Single(_))
                    && run.normalized.end == unit.normalized.start && run.source.end == unit.source.start
                    && unit.normalized.end - run.normalized.start <= if run.kind == RunKind::Ascii && kind == RunKind::Ascii {
                        MAPPING_CHUNK_BYTES
                    } else {
                        scalar_run_bytes
                    }
            }) {
                run.normalized.end = unit.normalized.end;
                run.source.end = unit.source.end;
                if kind == RunKind::Scalars { run.kind = RunKind::Scalars; }
                return;
            }
        }
        units.runs.push(UnitRun { first: units.len - 1, normalized: unit.normalized, source: unit.source, kind });
    }

    /// Keep only content before a normalized offset: text, the units that
    /// start before it and the endings that start before it.
    pub(crate) fn truncate(&mut self, end: usize) {
        let keep = self.unit_partition_point(|unit| unit.normalized.start < end);
        if keep < self.units.len {
            let kept = self.units.runs.partition_point(|run| run.first < keep);
            if let Some(index) = kept.checked_sub(1) {
                let retained = keep - self.units.runs[index].first;
                if retained < self.units.count(index) {
                    let cut = self.run_unit(&self.units.runs[index], retained);
                    let run = &mut self.units.runs[index];
                    run.normalized.end = cut.normalized.start;
                    run.source.end = cut.source.start;
                }
            }
            self.units.runs.truncate(kept);
            self.units.len = keep;
        }
        self.text.truncate(end);
        self.endings.retain(|ending| ending.normalized.start < end);
    }

    #[cfg(test)]
    pub(crate) fn unit_run_count(&self) -> usize {
        self.units.runs.len()
    }
}

pub(crate) struct Units<'a> {
    input: &'a NormalizedText,
    run: usize,
    at: usize,
    source: usize,
}

impl Iterator for Units<'_> {
    type Item = LogicalUnit;

    fn next(&mut self) -> Option<LogicalUnit> {
        let run = self.input.units.runs.get(self.run)?;
        let unit = match run.kind {
            RunKind::Single(_) => run.single(),
            RunKind::Ascii => run.ascii(self.at - run.normalized.start),
            RunKind::Scalars => {
                let ch = self.input.text[self.at..].chars().next().unwrap();
                let width = self.input.encoding.scalar_source_width(ch);
                LogicalUnit {
                    normalized: self.at..self.at + ch.len_utf8(),
                    source: self.source..self.source + width,
                    decoding_diagnostic: None,
                }
            }
        };
        if unit.normalized.end >= run.normalized.end {
            self.run += 1;
            if let Some(next) = self.input.units.runs.get(self.run) {
                self.at = next.normalized.start;
                self.source = next.source.start;
            }
        } else {
            self.at = unit.normalized.end;
            self.source = unit.source.end;
        }
        Some(unit)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{normalize, normalize_literal, FileFormat, LineEnding};
    use super::*;
    use crate::document::Encoding;

    /// Rebuild with explicit per-scalar pushes, as rich adapters do.
    fn dense(input: &NormalizedText) -> Vec<LogicalUnit> {
        input.units().collect()
    }

    fn oracle(text: &str, encoding: Encoding) -> Vec<LogicalUnit> {
        let mut source = 0;
        text.char_indices().map(|(at, ch)| {
            let width = encoding.scalar_source_width(ch);
            source += width;
            LogicalUnit { normalized: at..at + ch.len_utf8(), source: source - width..source, decoding_diagnostic: None }
        }).collect()
    }

    #[test]
    fn runs_reproduce_scalar_units_and_answer_ordered_queries() {
        let body = format!("ab\u{e9}c\u{1f600}d{}\u{e9}{}x\r\ny\rz\n", "q".repeat(5_000), "\u{4e2d}".repeat(300));
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            let body = if encoding == Encoding::Latin1 { body.replace(['\u{1f600}', '\u{4e2d}'], "w") } else { body.clone() };
            let bytes = encoding.encode_fragment(&body).unwrap();
            let decoded = encoding.decode(&bytes).unwrap();
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let input = normalize(&decoded, format);
                let units = dense(&input);
                // CRLF folds to one unit in DOS; every other scalar is its own.
                let crlf = usize::from(format == FileFormat::Dos);
                assert_eq!(units.len(), body.chars().count() - crlf, "{encoding:?} {format:?}");
                assert_eq!(input.unit_count(), units.len());
                assert!(input.unit_run_count() * 20 < units.len(), "{} runs", input.unit_run_count());
                if format == FileFormat::Unix {
                    assert_eq!(units, oracle(&input.text, encoding));
                }
                for (index, unit) in units.iter().enumerate() {
                    assert_eq!(input.unit(index).as_ref(), Some(unit));
                    assert_eq!(input.unit_starting_at(unit.normalized.start).as_ref(), Some(unit));
                    assert_eq!(input.units_from(index).next().as_ref(), Some(unit));
                    if index % 7 == 0 {
                        assert!(input.units_from(index).eq(units[index..].iter().cloned()));
                    }
                    for at in [unit.normalized.start, unit.normalized.start + 1, unit.normalized.end] {
                        assert_eq!(input.unit_partition_point(|u| u.normalized.start < at),
                            units.partition_point(|u| u.normalized.start < at));
                        assert_eq!(input.unit_partition_point(|u| u.normalized.end <= at),
                            units.partition_point(|u| u.normalized.end <= at));
                    }
                    for at in [unit.source.start, unit.source.start + 1] {
                        assert_eq!(input.unit_partition_point(|u| u.source.start < at),
                            units.partition_point(|u| u.source.start < at));
                    }
                }
                assert_eq!(input.unit(units.len()), None);
                assert_eq!(input.first_unit().as_ref(), units.first());
                assert_eq!(input.last_unit().as_ref(), units.last());
                assert_eq!(input.unit_starting_at(input.text.len()), None);
                let boundaries: Vec<usize> = input.unit_runs().flat_map(|run| [run.normalized.start, run.normalized.end + 1]).collect();
                for end in [0, 1, 3, 4, 5, 5_003, input.text.len() - 1, input.text.len()].into_iter().chain(boundaries) {
                    let end = end.min(input.text.len());
                    let end = (end..=input.text.len()).find(|at| input.text.is_char_boundary(*at)).unwrap();
                    let mut cut = input.clone();
                    cut.truncate(end);
                    let expected: Vec<_> = units.iter().filter(|unit| unit.normalized.start < end).cloned().collect();
                    assert_eq!(dense(&cut), expected);
                    assert_eq!(cut.unit_count(), expected.len());
                    assert_eq!(cut.last_unit().as_ref(), expected.last());
                    let endings: Vec<&LineEnding> = input.endings.iter().filter(|e| e.normalized.start < end).collect();
                    assert_eq!(cut.endings.len(), endings.len());
                }
            }
        }
    }

    #[test]
    fn explicit_units_stay_single_and_rich_scalar_runs_are_bounded() {
        let mut input = NormalizedText {
            text: String::new(), units: LogicalUnits::default(), endings: Vec::new(), encoding: Encoding::Utf8,
        };
        let push = |input: &mut NormalizedText, text: &str, source: Range<usize>| {
            let start = input.text.len();
            input.text.push_str(text);
            input.push_unit(LogicalUnit { normalized: start..input.text.len(), source, decoding_diagnostic: None });
        };
        push(&mut input, "a", 0..1);
        push(&mut input, " ", 1..3); // A folded CRLF.
        push(&mut input, "b", 3..4);
        push(&mut input, "c", 9..10); // Hidden syntax before it.
        for index in 0..200 { push(&mut input, "\u{e9}", 10 + 2 * index..12 + 2 * index); }
        assert_eq!(input.unit_runs().map(|unit| (unit.normalized, unit.source)).collect::<Vec<_>>(), vec![
            (0..1, 0..1), (1..2, 1..3), (2..3, 3..4), (3..258, 9..264), (258..404, 264..410),
        ]);
        assert_eq!(input.unit(1).unwrap().source, 1..3);
        assert_eq!(input.unit_partition_point(|unit| unit.source.start < 3), 2);
    }

    #[test]
    fn literal_runs_keep_the_conversion_chunk_bound() {
        let text = format!("{}\n", "\u{e9}a".repeat(10_000));
        let decoded = Encoding::Utf8.decode(text.as_bytes()).unwrap();
        let literal = normalize_literal(&decoded, FileFormat::Unix);
        let rich = normalize(&decoded, FileFormat::Unix);
        assert!(literal.unit_runs().all(|run| run.normalized.len() <= MAPPING_CHUNK_BYTES));
        assert!(literal.unit_run_count() <= text.len().div_ceil(MAPPING_CHUNK_BYTES - 4));
        assert!(rich.unit_runs().all(|run| run.normalized.len() <= RICH_SCALAR_RUN_BYTES));
        assert_eq!(dense(&literal), dense(&rich));
    }
}
