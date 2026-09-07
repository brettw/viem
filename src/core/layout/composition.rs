//! Bounded transient text capture for marked-text layout.
use super::{LayoutError, MAX_LONG_LINE_LAYOUT_SLICE_BYTES};
use crate::document::FormattedTextTree;
use std::ops::Range;

pub(crate) fn capture_range(
    tree: &FormattedTextTree,
    start: usize,
    full: Range<usize>,
) -> Result<(usize, Range<usize>), LayoutError> {
    let invalid = |_| LayoutError::InvalidTextOffset(start);
    let mut end = start
        .saturating_add(MAX_LONG_LINE_LAYOUT_SLICE_BYTES)
        .min(full.end);
    while end > start && !tree.is_char_boundary(end).map_err(invalid)? {
        end -= 1;
    }
    if end < full.end && !tree.is_grapheme_boundary(end).map_err(invalid)? {
        end = tree
            .previous_grapheme_boundary(end)
            .map_err(invalid)?
            .unwrap_or(start);
    }
    if end == start && end < full.end {
        end = tree
            .next_grapheme_boundary(start)
            .map_err(invalid)?
            .ok_or(LayoutError::InvalidTextOffset(start))?;
    }
    let mut context_start = start;
    let mut context_end = end;
    while start - context_start < 128 && context_start > full.start {
        context_start = tree
            .previous_grapheme_boundary(context_start)
            .map_err(invalid)?
            .unwrap_or(full.start)
            .max(full.start);
    }
    while context_end - end < 128 && context_end < full.end {
        context_end = tree
            .next_grapheme_boundary(context_end)
            .map_err(invalid)?
            .unwrap_or(full.end)
            .min(full.end);
    }
    Ok((end, context_start..context_end))
}
