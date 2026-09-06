//! Proportional-font Visual Block selection and edit planning.
//!
//! A block is stored as two visual-row caret identities plus two x coordinates.
//! Resolving it always uses an exact immutable layout snapshot. The resulting
//! range set retains per-row and bidi-affinity information; soft wraps never
//! become stored text boundaries.

use std::cmp::Ordering;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::document::{Document, DocumentError, DocumentId, Revision, TextEdit};
use crate::layout::{
    BoundaryAffinity, CaretPoint, LayoutRevision, LayoutSnapshot, PositionedCaret,
    PositionedCluster, VisualRow,
};

#[derive(Clone, Debug, PartialEq)]
pub struct BlockSelection {
    pub anchor: CaretPoint,
    pub active: CaretPoint,
    /// Display-space caret x for the fixed endpoint. This is kept separately
    /// from the rectangle edge because Vim's Visual Block selection includes
    /// the shaped item under each endpoint caret.
    pub anchor_x: f32,
    /// Display-space caret x for the moving endpoint.
    pub active_x: f32,
    rectangle_left_x: f32,
    rectangle_right_x: f32,
}

impl BlockSelection {
    pub fn new(
        anchor: CaretPoint,
        active: CaretPoint,
        anchor_x: f32,
        active_x: f32,
    ) -> Result<Self, VisualBlockError> {
        if anchor.document_id != active.document_id {
            return Err(VisualBlockError::WrongDocument {
                expected: anchor.document_id,
                actual: active.document_id,
            });
        }
        if anchor.document_revision != active.document_revision {
            return Err(VisualBlockError::WrongDocumentRevision {
                expected: anchor.document_revision,
                actual: active.document_revision,
            });
        }
        if anchor.layout_revision != active.layout_revision {
            return Err(VisualBlockError::StaleLayout {
                expected: anchor.layout_revision,
                actual: active.layout_revision,
            });
        }
        if !anchor_x.is_finite() || !active_x.is_finite() {
            return Err(VisualBlockError::InvalidX);
        }
        Ok(Self {
            anchor,
            active,
            anchor_x,
            active_x,
            rectangle_left_x: anchor_x.min(active_x),
            rectangle_right_x: anchor_x.max(active_x),
        })
    }

    pub fn left_x(&self) -> f32 {
        self.rectangle_left_x
    }

    pub fn right_x(&self) -> f32 {
        self.rectangle_right_x
    }

    /// Construct exact endpoints with an inclusive rectangle measured on a
    /// different row. Repeat uses the top row to preserve horizontal intent
    /// when its bottom endpoint clamps onto a shorter row.
    pub(crate) fn from_rectangle(
        anchor: CaretPoint,
        active: CaretPoint,
        anchor_x: f32,
        active_x: f32,
        rectangle_left_x: f32,
        rectangle_right_x: f32,
    ) -> Result<Self, VisualBlockError> {
        let mut selection = Self::new(anchor, active, anchor_x, active_x)?;
        if !rectangle_left_x.is_finite()
            || !rectangle_right_x.is_finite()
            || rectangle_left_x > rectangle_right_x
        {
            return Err(VisualBlockError::InvalidX);
        }
        selection.rectangle_left_x = rectangle_left_x;
        selection.rectangle_right_x = rectangle_right_x;
        Ok(selection)
    }

    /// Convert endpoint caret positions into the outer display-space edges of
    /// an inclusive Vim block. Keeping these edges distinct from the caret x
    /// values lets `O` exchange the active horizontal corner without changing
    /// the selected rectangle.
    pub(crate) fn update_inclusive_rectangle(
        &mut self,
        snapshot: &LayoutSnapshot,
    ) -> Result<(), VisualBlockError> {
        validate_layout_identity(self, snapshot)?;
        let anchor_row = locate_endpoint(snapshot, self.anchor)?;
        let active_row = locate_endpoint(snapshot, self.active)?;
        let anchor_span =
            endpoint_item_span(&snapshot.rows[anchor_row], self.anchor, self.anchor_x);
        let active_span =
            endpoint_item_span(&snapshot.rows[active_row], self.active, self.active_x);
        self.rectangle_left_x = anchor_span.0.min(active_span.0);
        self.rectangle_right_x = anchor_span.1.max(active_span.1);
        Ok(())
    }

    /// Exchange only the horizontal corners while retaining each endpoint's
    /// visual row. The endpoint carets are re-hit-tested on their respective
    /// rows so the controller cursor actually follows the new active corner.
    pub(crate) fn swap_horizontal_corners(
        &mut self,
        snapshot: &LayoutSnapshot,
    ) -> Result<(), VisualBlockError> {
        validate_layout_identity(self, snapshot)?;
        let anchor_row = locate_endpoint(snapshot, self.anchor)?;
        let active_row = locate_endpoint(snapshot, self.active)?;
        let new_anchor = nearest_caret(&snapshot.rows[anchor_row], self.active_x)
            .ok_or(VisualBlockError::EmptyLayout)?;
        let new_active = nearest_caret(&snapshot.rows[active_row], self.anchor_x)
            .ok_or(VisualBlockError::EmptyLayout)?;
        self.anchor = new_anchor.point;
        self.active = new_active.point;
        self.anchor_x = new_anchor.x;
        self.active_x = new_active.x;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBlockSelection {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub layout_revision: LayoutRevision,
    pub anchor_row: usize,
    pub active_row: usize,
    pub rows: Vec<ResolvedBlockRow>,
    pub range_set: BlockRangeSet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBlockRow {
    pub row_index: usize,
    pub hard_line_index: usize,
    pub visual_left: BlockEdge,
    pub visual_right: BlockEdge,
    /// Sorted logical ranges intersected by the visual rectangle. This can be
    /// discontiguous when bidirectional visual order differs from logical order.
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockEdge {
    pub point: CaretPoint,
    pub x: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockRangeSet {
    /// Logical-order segments, tagged with their visual-row origin. Adjacent
    /// segments on different visual rows deliberately remain distinct.
    pub segments: Vec<BlockRangeSegment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockRangeSegment {
    pub row_index: usize,
    pub hard_line_index: usize,
    pub range: Range<usize>,
    /// Number of logical extended grapheme clusters in `range`. Shaping may
    /// expose only the outer stops of a multi-grapheme cluster, but logical
    /// replacement still operates once per grapheme.
    pub grapheme_count: usize,
    pub left_affinity: BoundaryAffinity,
    pub right_affinity: BoundaryAffinity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockInsertEdge {
    Left,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisualBlockError {
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    WrongDocumentRevision {
        expected: Revision,
        actual: Revision,
    },
    StaleLayout {
        expected: LayoutRevision,
        actual: LayoutRevision,
    },
    InvalidX,
    EndpointNotInLayout(CaretPointIdentity),
    EmptyLayout,
    TextDoesNotMatchLayout,
    NonGraphemeBoundary(usize),
    OverlappingRanges,
    ReplacementTooLarge {
        unit_bytes: usize,
        grapheme_count: usize,
    },
}

/// Eq-friendly identity used in structured errors because layout's public
/// `CaretPoint` intentionally promises only `PartialEq`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaretPointIdentity {
    pub document_id: DocumentId,
    pub document_revision: Revision,
    pub layout_revision: LayoutRevision,
    pub text_offset: usize,
    pub affinity: BoundaryAffinity,
}

impl From<CaretPoint> for CaretPointIdentity {
    fn from(point: CaretPoint) -> Self {
        Self {
            document_id: point.document_id,
            document_revision: point.document_revision,
            layout_revision: point.layout_revision,
            text_offset: point.text_offset,
            affinity: point.affinity,
        }
    }
}

/// Re-hit-test every covered row and derive a fresh tagged range set.
pub fn resolve_block_selection(
    selection: &BlockSelection,
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
) -> Result<ResolvedBlockSelection, VisualBlockError> {
    resolve_block_selection_impl(selection, snapshot, formatted_text, false)
}

/// Resolve a Visual Block whose right edge was established with `$`.
///
/// `$` is semantic rather than a very large stored x coordinate: every target
/// visual row extends to its own rightmost legal caret after a fresh layout.
pub(crate) fn resolve_block_selection_to_line_end(
    selection: &BlockSelection,
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
) -> Result<ResolvedBlockSelection, VisualBlockError> {
    resolve_block_selection_impl(selection, snapshot, formatted_text, true)
}

fn resolve_block_selection_impl(
    selection: &BlockSelection,
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
    to_line_end: bool,
) -> Result<ResolvedBlockSelection, VisualBlockError> {
    validate_snapshot(selection, snapshot, formatted_text)?;
    let anchor_row = locate_endpoint(snapshot, selection.anchor)?;
    let active_row = locate_endpoint(snapshot, selection.active)?;
    let first_row = anchor_row.min(active_row);
    let last_row = anchor_row.max(active_row);
    let mut rows = Vec::with_capacity(last_row - first_row + 1);

    for row_index in first_row..=last_row {
        let row = &snapshot.rows[row_index];
        let left = nearest_edge_caret(row, selection.left_x(), RectangleEdge::Left)
            .ok_or(VisualBlockError::EmptyLayout)?;
        let right_x = if to_line_end {
            row.carets
                .iter()
                .map(|caret| caret.x)
                .filter(|x| x.is_finite())
                .max_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal))
                .ok_or(VisualBlockError::EmptyLayout)?
        } else {
            selection.right_x()
        };
        let right = nearest_edge_caret(row, right_x, RectangleEdge::Right)
            .ok_or(VisualBlockError::EmptyLayout)?;
        let (left, right) = order_visual_edges(left, right);
        let ranges = ranges_between_visual_edges(row, left, right);
        for range in &ranges {
            validate_grapheme_range(formatted_text, range)?;
        }
        rows.push(ResolvedBlockRow {
            row_index,
            hard_line_index: row.hard_line_index,
            visual_left: BlockEdge {
                point: left.point,
                x: left.x,
            },
            visual_right: BlockEdge {
                point: right.point,
                x: right.x,
            },
            ranges,
        });
    }

    let mut segments = Vec::new();
    for row in &rows {
        for range in &row.ranges {
            segments.push(BlockRangeSegment {
                row_index: row.row_index,
                hard_line_index: row.hard_line_index,
                range: range.clone(),
                grapheme_count: formatted_text[range.clone()].graphemes(true).count(),
                left_affinity: row.visual_left.point.affinity,
                right_affinity: row.visual_right.point.affinity,
            });
        }
    }
    segments.sort_by(|left, right| {
        left.range
            .start
            .cmp(&right.range.start)
            .then_with(|| left.range.end.cmp(&right.range.end))
            .then_with(|| left.row_index.cmp(&right.row_index))
    });
    if segments
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        return Err(VisualBlockError::OverlappingRanges);
    }

    Ok(ResolvedBlockSelection {
        document_id: snapshot.document_id,
        document_revision: snapshot.document_revision,
        layout_revision: snapshot.revision,
        anchor_row,
        active_row,
        rows,
        range_set: BlockRangeSet { segments },
    })
}

/// Delete every non-empty segment. Passing the result to `Document::apply_edits`
/// commits the entire rectangle as one transaction.
pub fn delete_text_edits(resolved: &ResolvedBlockSelection) -> Vec<TextEdit> {
    resolved
        .range_set
        .segments
        .iter()
        .filter(|segment| !segment.range.is_empty())
        .map(|segment| TextEdit::new(segment.range.clone(), ""))
        .collect()
}

/// Replace once per selected logical grapheme. A shaping cluster may contain
/// multiple graphemes and a bidi row may yield multiple logical segments, so
/// neither a segment count nor a row count is a valid replacement count.
/// Empty/short rows contain no selected grapheme and therefore contribute no
/// replacement edit.
pub fn replace_text_edits(
    resolved: &ResolvedBlockSelection,
    replacement: &str,
) -> Result<Vec<TextEdit>, VisualBlockError> {
    let mut edits = Vec::new();
    for segment in resolved
        .range_set
        .segments
        .iter()
        .filter(|segment| segment.grapheme_count > 0)
    {
        let count = segment.grapheme_count;
        let Some(capacity) = replacement.len().checked_mul(count) else {
            return Err(VisualBlockError::ReplacementTooLarge {
                unit_bytes: replacement.len(),
                grapheme_count: count,
            });
        };
        let mut repeated = String::new();
        if repeated.try_reserve_exact(capacity).is_err() {
            return Err(VisualBlockError::ReplacementTooLarge {
                unit_bytes: replacement.len(),
                grapheme_count: count,
            });
        }
        for _ in 0..count {
            repeated.push_str(replacement);
        }
        edits.push(TextEdit::new(segment.range.clone(), repeated));
    }
    Ok(edits)
}

/// Insert once per visual row at the chosen display-space edge. The selected
/// caret's affinity remains available in `ResolvedBlockRow` for adapters whose
/// reverse projection distinguishes two source sides at one logical boundary.
pub fn insert_text_edits(
    resolved: &ResolvedBlockSelection,
    edge: BlockInsertEdge,
    text: &str,
) -> Vec<TextEdit> {
    let mut edits: Vec<_> = resolved
        .rows
        .iter()
        .map(|row| {
            let at = match edge {
                BlockInsertEdge::Left => row.visual_left.point.text_offset,
                BlockInsertEdge::Right => row.visual_right.point.text_offset,
            };
            TextEdit::new(at..at, text)
        })
        .collect();
    edits.sort_by_key(|edit| edit.range.start);
    edits
}

/// Convenience that preserves the document model's all-or-nothing verification
/// and single undo record for the generated row edits.
pub fn apply_block_edits(
    document: &mut Document,
    edits: Vec<TextEdit>,
) -> Result<(), DocumentError> {
    document.apply_edits(edits)
}

fn validate_snapshot(
    selection: &BlockSelection,
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
) -> Result<(), VisualBlockError> {
    validate_layout_identity(selection, snapshot)?;
    let materialized_text_end = snapshot
        .rows
        .last()
        .map(|row| row.hard_line_range.end)
        .ok_or(VisualBlockError::EmptyLayout)?;
    let materializes_document_end =
        snapshot.coverage.hard_lines().end == snapshot.coverage.document_hard_line_count();
    if materialized_text_end > formatted_text.len()
        || (materializes_document_end && materialized_text_end != formatted_text.len())
    {
        return Err(VisualBlockError::TextDoesNotMatchLayout);
    }
    Ok(())
}

fn validate_layout_identity(
    selection: &BlockSelection,
    snapshot: &LayoutSnapshot,
) -> Result<(), VisualBlockError> {
    if selection.anchor.document_id != snapshot.document_id {
        return Err(VisualBlockError::WrongDocument {
            expected: selection.anchor.document_id,
            actual: snapshot.document_id,
        });
    }
    if selection.anchor.document_revision != snapshot.document_revision {
        return Err(VisualBlockError::WrongDocumentRevision {
            expected: selection.anchor.document_revision,
            actual: snapshot.document_revision,
        });
    }
    if selection.anchor.layout_revision != snapshot.revision {
        return Err(VisualBlockError::StaleLayout {
            expected: selection.anchor.layout_revision,
            actual: snapshot.revision,
        });
    }
    Ok(())
}

/// Geometry of the formatted item associated with an endpoint caret, shifted
/// to the stored display x. Association/affinity, rather than logical order,
/// selects the correct item at bidi and soft-wrap split carets.
fn endpoint_item_span(row: &VisualRow, point: CaretPoint, stored_x: f32) -> (f32, f32) {
    let Some(caret) = row
        .carets
        .iter()
        .filter(|caret| caret.point == point)
        .min_by(|left, right| {
            (left.x - stored_x)
                .abs()
                .partial_cmp(&(right.x - stored_x).abs())
                .unwrap_or(Ordering::Equal)
        })
    else {
        return (stored_x, stored_x);
    };
    let associated = associated_cluster(row, point);
    let Some(cluster) = associated else {
        return (stored_x, stored_x);
    };
    let delta = stored_x - caret.x;
    let first = cluster.x + delta;
    let second = cluster.x + cluster.advance + delta;
    (first.min(second), first.max(second))
}

fn associated_cluster(row: &VisualRow, point: CaretPoint) -> Option<&PositionedCluster> {
    row.clusters.iter().find(|cluster| match point.affinity {
        BoundaryAffinity::Downstream => {
            cluster.text_range.start <= point.text_offset
                && point.text_offset < cluster.text_range.end
        }
        BoundaryAffinity::Upstream => {
            cluster.text_range.start < point.text_offset
                && point.text_offset <= cluster.text_range.end
        }
    })
}

fn locate_endpoint(
    snapshot: &LayoutSnapshot,
    endpoint: CaretPoint,
) -> Result<usize, VisualBlockError> {
    snapshot
        .rows
        .iter()
        .position(|row| row.carets.iter().any(|caret| caret.point == endpoint))
        .ok_or_else(|| VisualBlockError::EndpointNotInLayout(endpoint.into()))
}

fn nearest_caret(row: &VisualRow, x: f32) -> Option<&PositionedCaret> {
    row.carets.iter().min_by(|left, right| {
        (left.x - x)
            .abs()
            .partial_cmp(&(right.x - x).abs())
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                affinity_rank(left.point.affinity).cmp(&affinity_rank(right.point.affinity))
            })
            .then_with(|| left.point.text_offset.cmp(&right.point.text_offset))
    })
}

#[derive(Clone, Copy)]
enum RectangleEdge {
    Left,
    Right,
}

/// Hit-test a rectangle edge while preserving which bidi caret side faces the
/// rectangle interior. At a directional boundary, upstream and downstream
/// stops may have the same x; a global affinity preference would attach the
/// edge to the adjacent, unselected cluster half of the time.
fn nearest_edge_caret(row: &VisualRow, x: f32, edge: RectangleEdge) -> Option<&PositionedCaret> {
    row.carets.iter().min_by(|left, right| {
        (left.x - x)
            .abs()
            .partial_cmp(&(right.x - x).abs())
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                edge_interior_rank(row, left, edge).cmp(&edge_interior_rank(row, right, edge))
            })
            .then_with(|| {
                affinity_rank(left.point.affinity).cmp(&affinity_rank(right.point.affinity))
            })
            .then_with(|| left.point.text_offset.cmp(&right.point.text_offset))
    })
}

fn edge_interior_rank(row: &VisualRow, caret: &PositionedCaret, edge: RectangleEdge) -> u8 {
    let Some(cluster) = associated_cluster(row, caret.point) else {
        return 1;
    };
    let center = cluster.x + cluster.advance / 2.0;
    let faces_interior = match edge {
        RectangleEdge::Left => center >= caret.x,
        RectangleEdge::Right => center <= caret.x,
    };
    u8::from(!faces_interior)
}

fn affinity_rank(affinity: BoundaryAffinity) -> u8 {
    match affinity {
        BoundaryAffinity::Downstream => 0,
        BoundaryAffinity::Upstream => 1,
    }
}

fn order_visual_edges<'a>(
    first: &'a PositionedCaret,
    second: &'a PositionedCaret,
) -> (&'a PositionedCaret, &'a PositionedCaret) {
    if first.x <= second.x {
        (first, second)
    } else {
        (second, first)
    }
}

fn ranges_between_visual_edges(
    row: &VisualRow,
    left: &PositionedCaret,
    right: &PositionedCaret,
) -> Vec<Range<usize>> {
    if left.x == right.x {
        return std::iter::once(left.point.text_offset..left.point.text_offset).collect();
    }
    let mut ranges: Vec<_> = row
        .clusters
        .iter()
        .filter(|cluster| {
            let cluster_right = cluster.x + cluster.advance;
            cluster.x < right.x && left.x < cluster_right
        })
        .map(|cluster| cluster.text_range.clone())
        .collect();
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut normalized: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = normalized.last_mut().filter(|last| last.end == range.start) {
            last.end = range.end;
        } else {
            normalized.push(range);
        }
    }
    if normalized.is_empty() {
        std::iter::once(left.point.text_offset..left.point.text_offset).collect()
    } else {
        normalized
    }
}

fn validate_grapheme_range(text: &str, range: &Range<usize>) -> Result<(), VisualBlockError> {
    for endpoint in [range.start, range.end] {
        if endpoint > text.len()
            || (endpoint != text.len()
                && !text
                    .grapheme_indices(true)
                    .any(|(boundary, _)| boundary == endpoint))
        {
            return Err(VisualBlockError::NonGraphemeBoundary(endpoint));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

    fn layout(text: &str, width: f32) -> (Document, LayoutSnapshot) {
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(width, 500.0);
        engine.relayout(&document, &mut view).unwrap();
        (document, view.snapshot().unwrap().clone())
    }

    fn endpoint(row: &VisualRow) -> CaretPoint {
        row.carets.first().unwrap().point
    }

    #[test]
    fn proportional_rows_are_hit_tested_independently() {
        let (document, snapshot) = layout("iiii\nWWWW", 500.0);
        let first = &snapshot.rows[0];
        let second = &snapshot.rows[1];
        let left = first.clusters[1].x;
        let right = first.clusters[3].x + first.clusters[3].advance;
        let selection =
            BlockSelection::new(endpoint(first), endpoint(second), left, right).unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(resolved.rows.len(), 2);
        assert_eq!(resolved.rows[0].ranges, vec![1..4]);
        // Wide W glyphs mean the same display rectangle covers fewer graphemes.
        assert_eq!(resolved.rows[1].ranges, vec![5..7]);
    }

    #[test]
    fn nearest_proportional_boundaries_can_exclude_a_partially_overlapped_character() {
        let (document, snapshot) = layout("WWW", 500.0);
        let row = &snapshot.rows[0];
        let width = row.clusters[0].advance;
        for (fraction, expected) in [(0.49, vec![0..1]), (0.51, vec![1..1])] {
            let selection =
                BlockSelection::new(endpoint(row), endpoint(row), width * fraction, width * 1.49)
                    .unwrap();
            let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
            assert_eq!(resolved.rows[0].ranges, expected);
        }
    }

    #[test]
    fn mixed_direction_rectangle_keeps_discontiguous_logical_ranges_atomic() {
        let (mut document, snapshot) = layout("abאבcd", 500.0);
        let row = &snapshot.rows[0];
        let latin = row
            .clusters
            .iter()
            .find(|cluster| cluster.text_range == (1..2))
            .unwrap();
        let hebrew = row
            .clusters
            .iter()
            .find(|cluster| cluster.text_range == (4..6))
            .unwrap();
        let selection = BlockSelection::new(
            endpoint(row),
            endpoint(row),
            latin.x,
            hebrew.x + hebrew.advance,
        )
        .unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(resolved.rows[0].ranges, vec![1..2, 4..6]);
        apply_block_edits(&mut document, delete_text_edits(&resolved)).unwrap();
        assert_eq!(document.text(), "aאcd");
        assert!(document.undo());
        assert_eq!(document.text(), "abאבcd");
        assert!(!document.undo());
    }

    #[test]
    fn short_and_empty_rows_produce_legal_empty_ranges() {
        let (document, snapshot) = layout("long row\nx\n\ntail", 500.0);
        let selection = BlockSelection::new(
            endpoint(&snapshot.rows[0]),
            endpoint(&snapshot.rows[2]),
            500.0,
            600.0,
        )
        .unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(resolved.rows[1].ranges, vec![10..10]);
        assert_eq!(resolved.rows[2].ranges, vec![11..11]);
        let inserts = insert_text_edits(&resolved, BlockInsertEdge::Left, "!");
        assert_eq!(inserts.len(), 3);
    }

    #[test]
    fn wrapped_visual_rows_remain_separate_segments() {
        let (document, snapshot) = layout("one two three four", 50.0);
        assert!(snapshot.rows.len() >= 3);
        let last = snapshot.rows.len() - 1;
        let selection = BlockSelection::new(
            endpoint(&snapshot.rows[0]),
            endpoint(&snapshot.rows[last]),
            0.0,
            30.0,
        )
        .unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(resolved.rows.len(), snapshot.rows.len());
        assert!(resolved
            .range_set
            .segments
            .windows(2)
            .all(|pair| pair[0].row_index < pair[1].row_index));
    }

    #[test]
    fn bidi_edges_retain_provider_affinity() {
        let (document, snapshot) = layout("אבג", 500.0);
        let row = &snapshot.rows[0];
        let selection = BlockSelection::new(
            endpoint(row),
            endpoint(row),
            row.clusters.first().unwrap().x,
            row.clusters.last().unwrap().x + row.clusters.last().unwrap().advance,
        )
        .unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(
            resolved.rows[0].visual_left.point.affinity,
            BoundaryAffinity::Upstream
        );
        assert_eq!(
            resolved.rows[0].visual_right.point.affinity,
            BoundaryAffinity::Downstream
        );
        assert_eq!(resolved.rows[0].ranges, vec![0..document.text().len()]);
    }

    #[test]
    fn inclusive_bidi_carets_preserve_interior_affinity_from_both_directions() {
        let (document, snapshot) = layout("אבג", 500.0);
        for (point, expected) in [
            (
                snapshot
                    .caret_point(0, BoundaryAffinity::Downstream)
                    .unwrap(),
                "א",
            ),
            (
                snapshot
                    .caret_point(document.text().len(), BoundaryAffinity::Upstream)
                    .unwrap(),
                "ג",
            ),
        ] {
            let x = snapshot.caret_geometry(point).unwrap().rect.x;
            let mut selection = BlockSelection::new(point, point, x, x).unwrap();
            selection.update_inclusive_rectangle(&snapshot).unwrap();

            let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
            assert_eq!(resolved.rows[0].ranges.len(), 1);
            assert_eq!(
                &document.text()[resolved.rows[0].ranges[0].clone()],
                expected
            );
            assert_eq!(
                resolved.rows[0].visual_left.point.affinity,
                BoundaryAffinity::Upstream
            );
            assert_eq!(
                resolved.rows[0].visual_right.point.affinity,
                BoundaryAffinity::Downstream
            );
        }
    }

    #[test]
    fn grapheme_clusters_are_never_split() {
        let (document, snapshot) = layout("a\u{301}b\n😀x", 500.0);
        let first = &snapshot.rows[0];
        let second = &snapshot.rows[1];
        let selection = BlockSelection::new(
            endpoint(first),
            endpoint(second),
            first.clusters[0].x,
            second.clusters[0].x + second.clusters[0].advance * 0.51,
        )
        .unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        assert_eq!(resolved.rows[0].ranges[0], 0..3);
        assert_eq!(&document.text()[resolved.rows[1].ranges[0].clone()], "😀");
    }

    #[test]
    fn delete_replace_and_insert_are_atomic_document_batches() {
        let (mut document, snapshot) = layout("abcd\nefgh", 500.0);
        let first = &snapshot.rows[0];
        let second = &snapshot.rows[1];
        let left = first.clusters[1].x;
        let right = first.clusters[2].x + first.clusters[2].advance;
        let selection =
            BlockSelection::new(endpoint(first), endpoint(second), left, right).unwrap();
        let resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();

        apply_block_edits(&mut document, replace_text_edits(&resolved, "X").unwrap()).unwrap();
        assert_eq!(document.text(), "aXXd\neXXh");
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");

        apply_block_edits(&mut document, delete_text_edits(&resolved)).unwrap();
        assert_eq!(document.text(), "ad\neh");
        assert!(document.undo());

        apply_block_edits(
            &mut document,
            insert_text_edits(&resolved, BlockInsertEdge::Left, "!"),
        )
        .unwrap();
        assert_eq!(document.text(), "a!bcd\ne!fgh");
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn replacement_size_overflow_is_typed_before_document_mutation() {
        let (mut document, snapshot) = layout("ab", 500.0);
        let row = &snapshot.rows[0];
        let selection = BlockSelection::new(
            endpoint(row),
            endpoint(row),
            row.clusters[0].x,
            row.clusters[0].x + row.clusters[0].advance,
        )
        .unwrap();
        let mut resolved = resolve_block_selection(&selection, &snapshot, document.text()).unwrap();
        resolved.range_set.segments[0].grapheme_count = usize::MAX;

        assert!(matches!(
            replace_text_edits(&resolved, "XX"),
            Err(VisualBlockError::ReplacementTooLarge { .. })
        ));
        assert_eq!(document.text(), "ab");
        assert!(!document.undo());
    }

    #[test]
    fn stale_layout_and_wrong_document_are_rejected() {
        let (document, snapshot) = layout("one\ntwo", 500.0);
        let selection = BlockSelection::new(
            endpoint(&snapshot.rows[0]),
            endpoint(&snapshot.rows[1]),
            0.0,
            10.0,
        )
        .unwrap();

        let (_, newer) = layout("one\ntwo", 500.0);
        assert!(matches!(
            resolve_block_selection(&selection, &newer, document.text()),
            Err(VisualBlockError::WrongDocument { .. })
        ));

        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(300.0, 500.0);
        engine.relayout(&document, &mut view).unwrap();
        view.resize(250.0, 500.0);
        engine.relayout(&document, &mut view).unwrap();
        let new_snapshot = view.snapshot().unwrap();
        assert!(matches!(
            resolve_block_selection(&selection, new_snapshot, document.text()),
            Err(VisualBlockError::StaleLayout { .. })
        ));
    }

    #[test]
    fn nonfinite_geometry_and_mismatched_text_fail_explicitly() {
        let (document, snapshot) = layout("text", 500.0);
        assert_eq!(
            BlockSelection::new(
                endpoint(&snapshot.rows[0]),
                endpoint(&snapshot.rows[0]),
                f32::NAN,
                1.0
            ),
            Err(VisualBlockError::InvalidX)
        );
        let selection = BlockSelection::new(
            endpoint(&snapshot.rows[0]),
            endpoint(&snapshot.rows[0]),
            0.0,
            1.0,
        )
        .unwrap();
        assert_eq!(
            resolve_block_selection(&selection, &snapshot, "other"),
            Err(VisualBlockError::TextDoesNotMatchLayout)
        );
        assert_eq!(document.text(), "text");
    }
}
