//! Document scroll bounds and row visibility shared by commands and hosts.
//!
//! A materialized band is not a document boundary. Only a proven document
//! endpoint may clamp scrolling; height estimates are for scrollbar display.

use super::{EdgeInsets, LayoutCoverage, LayoutSnapshot, VisualRow};
use std::ops::Range;

impl LayoutSnapshot {
    fn view_insets(&self) -> EdgeInsets {
        EdgeInsets {
            top: (self.content_insets.top - self.document_insets.top).max(0.0),
            bottom: (self.content_insets.bottom - self.document_insets.bottom).max(0.0),
            ..Default::default()
        }
    }

    pub(crate) fn reveal_vertical_range(&self, row: &VisualRow, height: f32) -> Range<f32> {
        let bounds = row.reveal_bounds();
        reveal_vertical_range(height, self.view_insets(), bounds.end - bounds.start)
    }

    pub(crate) fn reveal_viewport_top(&self, row: &VisualRow, current: f32, height: f32) -> f32 {
        reveal_viewport_top(row, current, height, self.view_insets())
    }

    pub(crate) fn contains_document_start(&self) -> bool {
        match &self.coverage {
            LayoutCoverage::FullDocument { .. } => true,
            LayoutCoverage::PartialHardLines { hard_lines, text_ranges, .. } => {
                hard_lines.start == 0 && text_ranges.first().is_some_and(|range| range.start == 0)
            }
        }
    }

    pub(crate) fn contains_document_end(&self) -> bool {
        match &self.coverage {
            LayoutCoverage::FullDocument { .. } => true,
            LayoutCoverage::PartialHardLines { hard_lines, document_hard_line_count, text_ranges, .. } => {
                hard_lines.end == *document_hard_line_count
                    && text_ranges.last().is_some_and(|range| range.end == self.text_len)
            }
        }
    }

    /// Proven maximum in this snapshot's coordinate system. A regional
    /// snapshot may have an estimated prefix while still knowing its exact
    /// local document end. Never substitute its last text row for that end:
    /// the extent already includes final-row ink, paragraph spacing and padding.
    pub fn maximum_viewport_top(&self, height: f32) -> Option<f32> {
        let bottom = if self.total_height_is_exact || self.coverage.is_full_document() {
            Some(self.total_height)
        } else if self.contains_document_end() {
            self.coverage.vertical_range().map(|range| range.end)
        } else {
            None
        };
        bottom.map(|bottom| (bottom - height).max(0.0))
    }

    pub fn estimated_maximum_viewport_top(&self, height: f32) -> f32 {
        self.maximum_viewport_top(height)
            .unwrap_or_else(|| (self.total_height - height).max(0.0))
    }

    pub(crate) fn clamp_viewport_top(&self, requested: f32, height: f32) -> f32 {
        let top = requested.max(0.0);
        self.maximum_viewport_top(height).map_or(top, |maximum| top.min(maximum))
    }

    /// Missing vertical geometry, excluding blank space beyond proven
    /// document edges. Text coverage matters for slices of a giant hard line.
    pub(crate) fn missing_viewport_edges(&self, top: f32, height: f32) -> (bool, bool) {
        self.coverage.vertical_range().map_or((false, false), |range| (
            top < range.start && !self.contains_document_start(),
            top + height > range.end && !self.contains_document_end(),
        ))
    }
}

/// Viewport-relative area reserved for an active row. Large margins yield
/// enough room for the row; source-authored canvas padding remains document
/// geometry rather than a permanently reserved viewport margin.
pub(super) fn reveal_vertical_range(height: f32, insets: EdgeInsets, row_height: f32) -> Range<f32> {
    let budget = (height - row_height.min(height)).max(0.0);
    let margins = insets.top + insets.bottom;
    let factor = if margins > budget { budget / margins } else { 1.0 };
    insets.top * factor..height - insets.bottom * factor
}

/// Minimum reveal, including glyph ink and natural height. Oversized rows
/// keep the baseline visible instead of alternating between impossible edges.
pub(super) fn reveal_viewport_top(row: &VisualRow, current: f32, height: f32, insets: EdgeInsets) -> f32 {
    let bounds = row.reveal_bounds();
    let visible = reveal_vertical_range(height, insets, bounds.end - bounds.start);
    if bounds.end - bounds.start > visible.end - visible.start {
        current.clamp(row.baseline - visible.end, row.baseline - visible.start)
    } else if bounds.start < current + visible.start {
        bounds.start - visible.start
    } else if bounds.end > current + visible.end {
        bounds.end - visible.end
    } else {
        current
    }
}

/// Minimum horizontal reveal, independent of wrapping. Oversized targets
/// expose their logical start, including a right-to-left leading edge.
pub(super) fn reveal_viewport_left(
    bounds: Range<f32>, start: f32, current: f32, width: f32, insets: EdgeInsets,
) -> f32 {
    let target_width = bounds.end - bounds.start;
    // Give fitting text priority over oversized margins, as vertical reveal
    // does for tall rows. Oversized matches reserve room for their first cell.
    let budget = (width - target_width.min(width).max(1.0)).max(0.0);
    let margins = insets.left + insets.right;
    let factor = if margins > budget { budget / margins } else { 1.0 };
    let left = insets.left * factor;
    let right = width - insets.right * factor;
    if target_width > width {
        if start >= bounds.end {
            start - right
        } else {
            start - left
        }
    } else if bounds.start < current + left {
        bounds.start - left
    } else if bounds.end > current + right {
        bounds.end - right
    } else {
        current
    }
}

/// Center a search cursor, then project that origin into the range that fits
/// the first match row. If it cannot fit, expose the logical leading edge and
/// as much following text as possible, with room for the thin cursor itself.
pub(super) fn reveal_search_viewport_left(
    bounds: Range<f32>, cursor: f32, width: f32, right_to_left: bool, complete: bool,
) -> f32 {
    if complete && bounds.end - bounds.start <= width {
        (cursor - width / 2.0).clamp(bounds.end - width, bounds.start)
    } else if right_to_left {
        cursor + super::CARET_REVEAL_WIDTH - width
    } else {
        cursor
    }
}

#[cfg(test)]
mod tests;
