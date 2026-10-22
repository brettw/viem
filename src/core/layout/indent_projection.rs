//! Immutable text/line input for indentation, including disposable composition.
//! A composition is a sparse view over the real projection, never a forged
//! document with invalid provenance or source mappings.
use crate::document::{FormattedDocument, FormattedTextTree};
use super::{DocumentLayoutStyles, DocumentStyleError, ResolvedTextStyle, ShapeStyleRun};
use std::{ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct IndentProjection {
    base: Arc<FormattedDocument>,
    overlay: Option<Overlay>,
}
#[derive(Clone, Debug)]
struct Overlay {
    text: FormattedTextTree,
    replacement: Range<usize>,
    marked: Range<usize>,
    first: usize,
    old_end: usize,
    lines: Vec<Range<usize>>,
    inserted_style: ResolvedTextStyle,
}
impl IndentProjection {
    pub(crate) fn new(base: Arc<FormattedDocument>) -> Self { Self { base, overlay: None } }
    pub(crate) fn composition(base: Arc<FormattedDocument>, tree: FormattedTextTree, replaced: Range<usize>, marked_text: &str,
        inserted_style: ResolvedTextStyle) -> Result<Self, super::LayoutError> {
        let marked = replaced.start..replaced.start + marked_text.len();
        let first = base.presentation_line_at_offset(replaced.start).ok_or(super::LayoutError::InvalidTextOffset(replaced.start))?;
        let last = base.presentation_line_at_offset(replaced.end).ok_or(super::LayoutError::InvalidTextOffset(replaced.end))?;
        let first_range = base.presentation_line_range(first).unwrap();
        let last_range = base.presentation_line_range(last).unwrap();
        let start = first_range.start;
        // A partition may omit its terminal empty boundary. If a replacement
        // consumes that final separator there is no surviving content suffix;
        // subtracting its preceding line's content end from EOF would underflow.
        let end = marked.end + last_range.end.saturating_sub(replaced.end);
        // Preserve semantic line boundaries outside the replaced span. Only
        // committed marked newlines are temporary hard breaks inside it.
        let mut lines = Vec::new();
        let mut line_start = start;
        if replaced.start > first_range.end {
            // Insertion after a surviving final separator starts a new line.
            // Retain the existing semantic break if the supplied partition
            // does not include its empty EOF boundary as a presentation line.
            lines.push(first_range);
            line_start = replaced.start;
        }
        for (offset, byte) in marked_text.bytes().enumerate() {
            if byte == b'\n' {
                let at = marked.start + offset;
                lines.push(line_start..at); line_start = at + 1;
            }
        }
        // Newly marked newlines keep an empty caret line. An empty insertion
        // preserves whichever terminal boundary the base partition supplied.
        if line_start < end || lines.is_empty() || !marked_text.is_empty() {
            lines.push(line_start..end);
        }
        Ok(Self { base, overlay: Some(Overlay { text: tree, replacement: replaced,
            marked, first, old_end: last + 1, lines, inserted_style }) })
    }
    pub(crate) fn text_tree(&self) -> &FormattedTextTree {
        self.overlay.as_ref().map_or(self.base.text_tree(), |o| &o.text)
    }
    pub(crate) fn presentation_line_count(&self) -> usize {
        self.base.presentation_line_count() + self.overlay.as_ref().map_or(0, |o| o.lines.len())
            - self.overlay.as_ref().map_or(0, |o| o.old_end - o.first)
    }
    pub(crate) fn presentation_line_range(&self, line: usize) -> Option<Range<usize>> {
        let Some(o) = &self.overlay else { return self.base.presentation_line_range(line); };
        if line < o.first { return self.base.presentation_line_range(line); }
        if line < o.first + o.lines.len() { return o.lines.get(line - o.first).cloned(); }
        self.base.presentation_line_range(line - o.lines.len() + o.old_end - o.first)
            .map(|r| shift(r.start, o)..shift(r.end, o))
    }
    pub(crate) fn presentation_line_at_offset(&self, at: usize) -> Option<usize> {
        if at > self.text_tree().byte_len() { return None; }
        let Some(o) = &self.overlay else { return self.base.presentation_line_at_offset(at); };
        let first = o.lines.first()?.start;
        let last = o.lines.last()?.end;
        if at < first { return self.base.presentation_line_at_offset(at); }
        if at <= last { return Some(o.first + o.lines.partition_point(|r| r.end < at).min(o.lines.len() - 1)); }
        self.base.presentation_line_at_offset(unshift(at, o, false)).map(|n| n + o.lines.len() - (o.old_end - o.first))
    }
    pub(crate) fn shares_presentation_lines_with(&self, other: &Self) -> bool {
        match (&self.overlay, &other.overlay) {
            (None,None) => self.base.shares_presentation_lines_with(&other.base),
            (Some(a),Some(b)) => a.text.shares_root_with(&b.text) && a.first == b.first && a.old_end == b.old_end && a.lines == b.lines,
            _ => false,
        }
    }
    pub(crate) fn resolve_styles(&self, range: Range<usize>, format: crate::document::Format) -> Result<DocumentLayoutStyles, DocumentStyleError> {
        let base_range = self.overlay.as_ref().map_or_else(|| range.clone(), |o| unshift(range.start,o,false)..unshift(range.end,o,true));
        let mut styles = DocumentLayoutStyles::resolve_region_for_presentation(&self.base, base_range, &[], format)?;
        if let Some(o) = &self.overlay {
            styles.shaping_runs = splice_shaping_runs(styles.shaping_runs, &o.replacement, &o.marked,
                &o.inserted_style, &range);
        }
        Ok(styles)
    }
}
fn shift(at: usize, o: &Overlay) -> usize { at - o.replacement.end + o.marked.end }
fn unshift(at: usize, o: &Overlay, end: bool) -> usize {
    if at <= o.marked.start { at }
    else if at < o.marked.end { if end { o.replacement.end } else { o.replacement.start } }
    else { at - o.marked.end + o.replacement.end }
}
/// The coordinator chooses the marked text's exact style once, using the
/// composition's affinity. Keep unchanged prefixes/suffixes and add that one
/// marked run, as ordinary composition layout does. Expanding intersecting
/// source runs would create overlapping styles at an insertion boundary.
fn splice_shaping_runs(runs: Vec<ShapeStyleRun>, replaced: &Range<usize>, inserted: &Range<usize>,
    inserted_style: &ResolvedTextStyle, requested: &Range<usize>) -> Vec<ShapeStyleRun> {
    let mut output = Vec::with_capacity(runs.len() + 1);
    let mut push = |range: Range<usize>, style: ResolvedTextStyle| {
        let range = range.start.max(requested.start)..range.end.min(requested.end);
        if !range.is_empty() { output.push(ShapeStyleRun { text_range: range, style }); }
    };
    for run in runs {
        let before_end = run.text_range.end.min(replaced.start);
        if run.text_range.start < before_end {
            push(run.text_range.start..before_end, run.style.clone());
        }
        let after_start = run.text_range.start.max(replaced.end);
        if after_start < run.text_range.end {
            push((after_start - replaced.end + inserted.end)..(run.text_range.end - replaced.end + inserted.end), run.style);
        }
    }
    push(inserted.clone(), inserted_style.clone());
    output.sort_by_key(|run| run.text_range.start);
    let mut merged: Vec<ShapeStyleRun> = Vec::with_capacity(output.len());
    for run in output {
        if let Some(previous) = merged.last_mut() {
            debug_assert!(previous.text_range.end <= run.text_range.start);
            if previous.text_range.end == run.text_range.start && previous.style == run.style {
                previous.text_range.end = run.text_range.end;
                continue;
            }
        }
        merged.push(run);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};

    fn style(size: f32) -> ResolvedTextStyle { ResolvedTextStyle { size, ..Default::default() } }
    fn runs() -> Vec<ShapeStyleRun> {
        vec![ShapeStyleRun { text_range: 0..2, style: style(12.) },
             ShapeStyleRun { text_range: 2..4, style: style(24.) }]
    }
    fn values(runs: Vec<ShapeStyleRun>) -> Vec<(Range<usize>, f32)> {
        runs.into_iter().map(|run| (run.text_range, run.style.size)).collect()
    }

    fn assert_composition_lines(source: &str, replaced: Range<usize>, marked: &str, expected: Vec<Range<usize>>) {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let base = Arc::new(document.projection().clone());
        let tree = base.text_tree().splice(replaced.clone(), marked).unwrap();
        let overlay = IndentProjection::composition(base.clone(), tree, replaced, marked, style(16.)).unwrap();
        let actual = (0..overlay.presentation_line_count()).map(|n| overlay.presentation_line_range(n).unwrap()).collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(overlay.presentation_line_range(actual.len()), None);
        for at in 0..=overlay.text_tree().byte_len() {
            let line = expected.partition_point(|range| range.start <= at).saturating_sub(1);
            assert_eq!(overlay.presentation_line_at_offset(at), Some(line), "semantic owner at offset {at}");
        }
        assert_eq!(overlay.presentation_line_at_offset(overlay.text_tree().byte_len() + 1), None);
        assert_eq!(base.text_tree().slice(0..base.text_tree().byte_len()).unwrap(), source);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn marked_replacements_through_final_separator_keep_valid_semantic_ranges() {
        assert_composition_lines("abc\n", 0..4, "W", vec![0..1]);
        assert_composition_lines("abc\n", 0..4, "", vec![0..0]);
        assert_composition_lines("abc\n", 2..4, "W", vec![0..3]);
        assert_composition_lines("abc\n", 3..4, "W", vec![0..4]);
        assert_composition_lines("abc\n", 0..4, "W\n", vec![0..1, 2..2]);
        assert_composition_lines("a\nb\n", 2..4, "W", vec![0..1, 2..3]);
    }

    #[test]
    fn marked_insertion_after_final_separator_preserves_the_existing_break() {
        assert_composition_lines("abc\n", 4..4, "W", vec![0..3, 4..5]);
        assert_composition_lines("abc\n", 4..4, "", vec![0..3, 4..4]);
        assert_composition_lines("abc\n", 4..4, "W\n", vec![0..3, 4..5, 6..6]);
        assert_composition_lines("abc\n", 4..4, "\nW", vec![0..3, 4..4, 5..6]);
        assert_composition_lines("a\nb\n", 4..4, "W", vec![0..1, 2..3, 4..5]);
        assert_composition_lines("abc\n", 3..3, "W", vec![0..4, 5..5]);
        assert_composition_lines("abc", 3..3, "W", vec![0..4]);
    }

    #[test]
    fn empty_marked_insertion_preserves_the_base_presentation_partition() {
        // Presentation includes the terminal caret line even where the Vim
        // physical-motion domain excludes that final empty boundary.
        for source in ["", "abc", "abc\n", "abc\n\n", "a\nb\n"] {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
            let expected = (0..document.projection().presentation_line_count())
                .map(|line| document.projection().presentation_line_range(line).unwrap()).collect::<Vec<_>>();
            for at in 0..=source.len() {
                assert_composition_lines(source, at..at, "", expected.clone());
            }
        }
    }

    #[test]
    fn marked_insertion_uses_one_authoritative_affinity_style() {
        for (size, expected) in [(12., vec![(0..3, 12.), (3..5, 24.)]),
                                 (24., vec![(0..2, 12.), (2..5, 24.)]),
                                 (36., vec![(0..2, 12.), (2..3, 36.), (3..5, 24.)])] {
            assert_eq!(values(splice_shaping_runs(runs(), &(2..2), &(2..3), &style(size), &(0..5))), expected);
        }
    }

    #[test]
    fn marked_replacement_preserves_unselected_styles_and_clips_capture() {
        assert_eq!(values(splice_shaping_runs(runs(), &(1..3), &(1..4), &style(36.), &(0..5))),
            vec![(0..1, 12.), (1..4, 36.), (4..5, 24.)]);
        assert_eq!(values(splice_shaping_runs(runs(), &(1..3), &(1..4), &style(36.), &(2..4))),
            vec![(2..4, 36.)]);
        assert_eq!(values(splice_shaping_runs(runs(), &(1..3), &(1..1), &style(36.), &(0..2))),
            vec![(0..1, 12.), (1..2, 24.)]);
    }
}
