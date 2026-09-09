//! Logical paragraph ownership shared by keyboard actions and source edits.
//! Inline markup and caret affinity do not create additional block boundaries.
use super::{Block, Document, DocumentError, Format};
use std::ops::Range;

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
        Format::Markdown => super::markdown_quotes::is_fenced_block(document, block),
        Format::Html => super::html_quotes::in_native_pre(document, block.range.start),
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
