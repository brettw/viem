//! Logical paragraph ownership shared by keyboard actions and source edits.
//! Inline markup and caret affinity do not create additional block boundaries.
use super::{Block, Document, DocumentError};
use std::ops::Range;

impl Document {
    /// Deleting the final unterminated row also removes its preceding break.
    /// Keep this separate from the requested rows: that supporting boundary
    /// does not select the preceding paragraph, especially an empty owner.
    pub(super) fn line_deletion_text_range(&self, range: Range<usize>) -> Range<usize> {
        let lines = self.hard_line_snapshot();
        if range.end != lines.text_length() || range.start == 0 { return range; }
        if !range.is_empty() && lines.previous_grapheme_boundary(range.end)
            .and_then(|at| lines.line_at_offset(at).ok())
            .and_then(|line| line.separator_range())
            .is_some_and(|separator| separator.end == range.end) { return range; }
        let Some(previous) = lines.line_at_offset(range.start).ok()
            .filter(|line| line.content_range().start == range.start)
            .and_then(|line| line.index().checked_sub(1)).and_then(|index| lines.line(index))
            else { return range; };
        previous.separator_range().map_or(range.clone(), |separator| separator.start..range.end)
    }

    /// The semantic paragraph owning a current formatted boundary. Internal
    /// hard breaks and soft wrapping do not split this range.
    pub fn paragraph_range_at(&self, at: usize) -> Result<Range<usize>, DocumentError> {
        Ok(paragraph_at(self, at)?.map_or(at..at, |block| block.range))
    }
}

pub(super) fn paragraph_at(document: &Document, at: usize) -> Result<Option<Block>, DocumentError> {
    document.text_point(at)?;
    Ok(document
        .projection()
        .blocks_for_region(&(at..at))
        .into_iter()
        .filter(|block| block.range.start <= at && at <= block.range.end)
        .max_by_key(|block| block.range.start))
}

/// Code is structural context, including a fence nested in a quote or item;
/// its current named paragraph assignment alone does not identify that context.
pub(super) fn is_code_paragraph(document: &Document, block: &Block) -> Result<bool, DocumentError> {
    if block.style.0 == "Code Block" {
        return Ok(true);
    }
    match document.format() {
        format if format.is_markdown() => super::markdown_quotes::is_fenced_block(document, block),

        _ => Ok(false),
    }
}

/// Removing any separator joins its two paragraphs. The paragraph at the
/// selection's beginning owns the result, including when all its text is cut.
pub(super) fn merged_paragraphs(document: &Document, range: &Range<usize>) -> Vec<Block> {
    if range.is_empty() {
        return Vec::new();
    }
    let blocks = document.projection().blocks_for_region(range);
    let crosses = blocks.windows(2).any(|pair| {
        range.start <= pair[0].range.end
            && pair[0].range.end < range.end
            && pair[0].range.end < pair[1].range.start
    });
    if !crosses {
        return Vec::new();
    }
    blocks
        .into_iter()
        .filter(|block| block.range.end >= range.start && block.range.start <= range.end)
        .collect()
}
