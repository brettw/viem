//! Bounded transient text capture for marked-text layout.
use super::{LayoutError, LayoutJobError, LongLineLayoutCheckpoint, ViewLayout, MAX_LONG_LINE_LAYOUT_SLICE_BYTES};
use crate::document::FormattedTextTree;
use std::ops::Range;

pub(crate) fn capture_range(
    tree: &FormattedTextTree,
    start: usize,
    full: Range<usize>,
    view: &ViewLayout,
    checkpoint: Option<&LongLineLayoutCheckpoint>,
    cancellation: &dyn super::engine::LayoutCancellationProbe,
) -> Result<(usize, Range<usize>), LayoutJobError> {
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
    if end < full.end {
        // A byte-limited prefix of one indivisible word contains no row the
        // layout engine can publish. Match ordinary long-line capture: extend
        // through its first real break and stream an overflow row if needed.
        end = end.max(super::line_breaks::first_line_break(
            tree,
            start..full.end,
            view.paragraph_flow(),
            &mut checkpoint.map_or_else(|| view.initial_wrap_break_state(), |c| c.wrap_break_state),
            cancellation,
        )?);
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

#[cfg(test)]
mod tests {
    use super::super::LayoutCancellationToken;
    use super::*;

    #[test]
    fn capture_reaches_real_break_with_flow_and_preserves_unicode_boundaries() {
        let prefix = "a".repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 64);
        let tree = FormattedTextTree::try_from_text(format!("{prefix} \u{301}next\nend")).unwrap();
        for flow in [false, true] {
            let mut view = ViewLayout::new(100.0, 100.0);
            view.set_paragraph_flow(flow);
            let (end, capture) = capture_range(
                &tree,
                0,
                0..tree.byte_len(),
                &view,
                None,
                &LayoutCancellationToken::new(),
            )
            .unwrap();
            // The opportunity after the first space lies inside its combining
            // grapheme and is suppressed; the next real boundary follows next.
            assert_eq!(end, prefix.len() + " \u{301}next\n".len());
            assert!(tree.is_grapheme_boundary(end).unwrap());
            assert!(tree.is_grapheme_boundary(capture.end).unwrap());
        }
        let cancelled = LayoutCancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            capture_range(&tree, 0, 0..tree.byte_len(), &ViewLayout::new(100.0, 100.0), None, &cancelled),
            Err(LayoutJobError::Cancelled)
        ));
    }
}
