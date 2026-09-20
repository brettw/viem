//! Pure visual-row and viewport motions over an immutable layout snapshot.
//!
//! These helpers deliberately do not mutate [`crate::document::Document`] or
//! controller state. The command interpreter can use the returned position and
//! desired x in a revision-bound command plan, then let the coordinator publish
//! the result atomically.

use crate::document::{BoundaryAffinity, DocumentId, Revision};
use crate::layout::{
    nearest_caret, LayoutRevision, LayoutSnapshot, MetricsGeneration, PositionedCaret,
    ViewConfigurationGeneration, VisualRow,
};
use std::cmp::Ordering;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualPosition {
    pub text_offset: usize,
    pub affinity: BoundaryAffinity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualMotionResult {
    pub position: VisualPosition,
    /// The layout-unit x retained for subsequent vertical motions.
    pub desired_x: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub top: f32,
    pub height: f32,
}

impl Viewport {
    pub fn new(top: f32, height: f32) -> Result<Self, LayoutMotionError> {
        if !top.is_finite() || !height.is_finite() || height <= 0.0 {
            return Err(LayoutMotionError::InvalidViewport);
        }
        Ok(Self {
            top: top.max(0.0),
            height,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportLine {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportAlignment {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenMotion {
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    /// Vim Ctrl-E: move the viewport toward later text by visual rows.
    ScrollDown,
    /// Vim Ctrl-Y: move the viewport toward earlier text by visual rows.
    ScrollUp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenMotionResult {
    pub motion: VisualMotionResult,
    pub viewport: Viewport,
}

/// Which materialized edge prevented an exact visual-row command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutDemandEdge {
    Before,
    After,
    Both,
}

/// Revision-bound work requested by a visual command at a partial-layout edge.
///
/// `requested_hard_lines` includes the currently materialized interval plus a
/// conservative adjacent extension. Installing it as a viewport request makes
/// the command retryable without asking the frontend to invent an expansion
/// direction or size. A scheduler must still validate the identities through
/// the coordinator because a later edit, resize, or metrics change makes this
/// demand stale.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutDemand {
    document_id: DocumentId,
    document_revision: Revision,
    layout_revision: LayoutRevision,
    configuration_generation: ViewConfigurationGeneration,
    metrics_generation: MetricsGeneration,
    edge: LayoutDemandEdge,
    materialized_hard_lines: Range<usize>,
    requested_hard_lines: Range<usize>,
    minimum_additional_visual_rows: usize,
    horizontal_focus_offset: Option<usize>,
    horizontal_x_bits: Option<u32>,
    complete_horizontal_geometry: bool,
}

impl LayoutDemand {
    pub fn requires_complete_horizontal_geometry(&self) -> bool { self.complete_horizontal_geometry }
    pub fn horizontal_focus(&self) -> Option<(usize, Option<f32>)> {
        self.horizontal_focus_offset.map(|offset| (offset, self.horizontal_x_bits.map(f32::from_bits)))
    }
    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn layout_revision(&self) -> LayoutRevision {
        self.layout_revision
    }

    pub fn configuration_generation(&self) -> ViewConfigurationGeneration {
        self.configuration_generation
    }

    pub fn metrics_generation(&self) -> MetricsGeneration {
        self.metrics_generation
    }

    pub fn edge(&self) -> LayoutDemandEdge {
        self.edge
    }

    pub fn materialized_hard_lines(&self) -> Range<usize> {
        self.materialized_hard_lines.clone()
    }

    /// The complete hard-line interval to use for the replacement viewport
    /// snapshot, not merely a cache-only adjacent fill.
    pub fn requested_hard_lines(&self) -> Range<usize> {
        self.requested_hard_lines.clone()
    }

    pub fn minimum_additional_visual_rows(&self) -> usize {
        self.minimum_additional_visual_rows
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutMotionError {
    EmptyLayout,
    PositionNotInLayout(VisualPosition),
    /// The requested destination may exist in the document, but its exact
    /// visual geometry is not present in this partial snapshot. The attached
    /// demand is sufficient to prepare the next bounded viewport request.
    OutsideMaterializedCoverage(LayoutDemand),
    InvalidViewport,
    TextDoesNotMatchLayout,
}

/// `gj`: move down by visual rows, clamping only at the true document end.
/// A partial snapshot edge requests more layout instead.
pub fn gj(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    count: usize,
    desired_x: Option<f32>,
) -> Result<VisualMotionResult, LayoutMotionError> {
    move_visual_rows(snapshot, current, positive_count(count), desired_x)
}

/// `gk`: move up by visual rows, clamping only at the true document start.
/// A partial snapshot edge requests more layout instead.
pub fn gk(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    count: usize,
    desired_x: Option<f32>,
) -> Result<VisualMotionResult, LayoutMotionError> {
    move_visual_rows(snapshot, current, -positive_count(count), desired_x)
}

/// `g0`: logical start of the current visual row with downstream affinity.
pub fn g0(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
) -> Result<VisualPosition, LayoutMotionError> {
    let (row_index, _) = locate(snapshot, current)?;
    Ok(row_start(&snapshot.rows[row_index]))
}

/// `g^`: first non-blank grapheme on the current visual row.
pub fn g_caret(
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
    current: VisualPosition,
) -> Result<VisualPosition, LayoutMotionError> {
    let (row_index, _) = locate(snapshot, current)?;
    first_non_blank(&snapshot.rows[row_index], formatted_text)
}

/// Document-backed `g^`/Home without a flat document or row-sized copy.
/// Sparse overflow rows can cover an entire giant word or whitespace run;
/// only the immutable leaves before the first nonblank scalar are visited.
pub(crate) fn g_caret_for_document(
    document: &crate::document::Document,
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
) -> Result<VisualPosition, LayoutMotionError> {
    if snapshot.document_id != document.id()
        || snapshot.document_revision != document.revision()
    {
        return Err(LayoutMotionError::TextDoesNotMatchLayout);
    }
    let (row_index, _) = locate(snapshot, current)?;
    first_non_blank_for_document(document, snapshot.rows[row_index].text_range.clone())
}

pub(super) fn first_non_blank_for_document(
    document: &crate::document::Document,
    range: Range<usize>,
) -> Result<VisualPosition, LayoutMotionError> {
    let tree = document.projection().text_tree();
    let lines = document.hard_line_snapshot();
    if range.start > range.end
        || range.end > tree.byte_len()
        || !lines.is_grapheme_boundary(range.start)
        || !lines.is_grapheme_boundary(range.end)
    {
        return Err(LayoutMotionError::TextDoesNotMatchLayout);
    }
    let mut at = range.start;
    while at < range.end {
        let chunk = tree.byte_chunk_at(at);
        let chunk = &chunk[..chunk.len().min(range.end - at)];
        let text = std::str::from_utf8(chunk)
            .map_err(|_| LayoutMotionError::TextDoesNotMatchLayout)?;
        if let Some((local, _)) = text.char_indices().find(|(_, ch)| !ch.is_whitespace()) {
            let scalar = at + local;
            // A space followed by a combining mark is one nonblank grapheme.
            // Resolve only this local boundary rather than copying that cluster.
            let offset = if lines.is_grapheme_boundary(scalar) {
                scalar
            } else {
                lines.previous_grapheme_boundary(scalar)
                    .filter(|offset| *offset >= range.start)
                    .ok_or(LayoutMotionError::TextDoesNotMatchLayout)?
            };
            return Ok(VisualPosition {
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
            });
        }
        at += chunk.len();
    }
    Ok(VisualPosition {
        text_offset: range.start,
        affinity: BoundaryAffinity::Downstream,
    })
}

/// `g$`: logical end of the current visual row with upstream affinity.
pub fn g_dollar(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
) -> Result<VisualPosition, LayoutMotionError> {
    let (row_index, _) = locate(snapshot, current)?;
    Ok(row_end(&snapshot.rows[row_index]))
}

/// A flowed list's wrap separator belongs between words. Its space remains
/// logical content, but the visual end command addresses the last visible
/// grapheme rather than the whitespace consumed at the wrap.
pub(crate) fn g_dollar_for_document(
    document: &crate::document::Document,
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
) -> Result<VisualPosition, LayoutMotionError> {
    let (row_index, _) = locate(snapshot, current)?;
    let row = &snapshot.rows[row_index];
    if row.wraps_to_next
        && document
            .projection()
            .blocks_for_region(&(row.text_range.start..row.text_range.start))
            .iter()
            .any(|block| {
                block.style.0 != "Code Block"
                    && matches!(
                        block.kind,
                        crate::document::BlockKind::ListItem {
                            marker_is_decoration: true,
                            ..
                        }
                    )
            })
    {
        let text = document
            .projection()
            .text_tree()
            .slice(row.text_range.clone())
            .map_err(|_| LayoutMotionError::TextDoesNotMatchLayout)?;
        let end = row.text_range.start + text.trim_end_matches([' ', '\t']).len();
        if end > row.text_range.start {
            return Ok(VisualPosition {
                text_offset: end,
                affinity: BoundaryAffinity::Upstream,
            });
        }
    }
    Ok(row_end(row))
}

/// Shared desired-x implementation for visual vertical movement.
pub fn move_visual_rows(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    row_delta: isize,
    desired_x: Option<f32>,
) -> Result<VisualMotionResult, LayoutMotionError> {
    let (current_row, current_caret) = locate(snapshot, current)?;
    let x = match desired_x {
        Some(value) if value.is_finite() => value,
        _ => current_caret.x,
    };
    let target_row = shift_row_index(snapshot, current_row, row_delta)?;
    let row = &snapshot.rows[target_row];
    if !snapshot.horizontal_geometry_is_materialized(target_row, x) {
        return Err(horizontal_demand(snapshot, current.text_offset, Some(x)));
    }
    let target = nearest_caret(row, x).ok_or(LayoutMotionError::EmptyLayout)?;
    let mut position = position_of(target);
    if position.text_offset == row.text_range.start && !row.text_range.is_empty() {
        // Some shapers expose both affinities at a paragraph start. An
        // upstream realization there must not associate the preceding
        // paragraph separator when Normal mode chooses its block cursor.
        position.affinity = BoundaryAffinity::Downstream;
    }
    Ok(VisualMotionResult {
        position,
        desired_x: x,
    })
}

/// Implements H/M/L over visual rows intersecting the supplied viewport.
/// H and L honor Vim's count as an inset from the corresponding edge; M ignores
/// it and uses the middle visible row.
pub fn viewport_line(
    snapshot: &LayoutSnapshot,
    formatted_text: &str,
    viewport: Viewport,
    target: ViewportLine,
    count: usize,
) -> Result<VisualPosition, LayoutMotionError> {
    validate_viewport(viewport)?;
    let visible = visible_rows(snapshot, viewport)?;
    let count = count.max(1);
    let row_index = match target {
        ViewportLine::Top => visible[(count - 1).min(visible.len() - 1)],
        ViewportLine::Middle => visible[visible.len() / 2],
        ViewportLine::Bottom => visible[visible.len().saturating_sub(count)],
    };
    first_non_blank(&snapshot.rows[row_index], formatted_text)
}

/// Implements `zt`, `zz`, and `zb` without changing the caret.
pub fn align_viewport(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    viewport: Viewport,
    alignment: ViewportAlignment,
) -> Result<Viewport, LayoutMotionError> {
    validate_viewport(viewport)?;
    let (row_index, _) = locate(snapshot, current)?;
    let row = &snapshot.rows[row_index];
    let bounds = row.reveal_bounds();
    let visible = snapshot.reveal_vertical_range(row, viewport.height);
    let requested = match alignment {
        ViewportAlignment::Top => bounds.start - visible.start,
        ViewportAlignment::Middle => (bounds.start + bounds.end - visible.start - visible.end) / 2.0,
        ViewportAlignment::Bottom => bounds.end - visible.end,
    };
    Ok(Viewport {
        top: clamped_viewport_top(snapshot, requested, viewport.height)?,
        height: viewport.height,
    })
}

/// Implements full-page, half-page, and Ctrl-E/Ctrl-Y screen motions.
///
/// Full pages retain two visual rows of overlap when possible. Page and
/// half-page operations move both viewport and caret by the same visual-row
/// count. Ctrl-E/Ctrl-Y retain the caret while it remains visible and otherwise
/// place it on the newly exposed nearest viewport edge at desired x.
pub fn screen_motion(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    viewport: Viewport,
    motion: ScreenMotion,
    count: usize,
    desired_x: Option<f32>,
) -> Result<ScreenMotionResult, LayoutMotionError> {
    validate_viewport(viewport)?;
    let visible = visible_rows(snapshot, viewport)?;
    let located = if snapshot.coverage.contains_text_offset(current.text_offset) {
        Some(locate(snapshot, current)?)
    } else {
        None
    };
    let visible_count = visible.len();
    let direction = match motion {
        ScreenMotion::PageDown | ScreenMotion::HalfPageDown | ScreenMotion::ScrollDown => 1,
        ScreenMotion::PageUp | ScreenMotion::HalfPageUp | ScreenMotion::ScrollUp => -1,
    };
    let amount_rows = match motion {
        ScreenMotion::PageDown | ScreenMotion::PageUp => visible_count
            .saturating_sub(2)
            .max(1)
            .saturating_mul(count.max(1)),
        // A zero amount is the internal spelling for Vim's unset window-local
        // 'scroll' value: derive half of the current viewport. Once the user
        // supplies an explicit CTRL-D/CTRL-U count, the interpreter passes
        // that remembered exact visual-row amount instead.
        ScreenMotion::HalfPageDown | ScreenMotion::HalfPageUp if count == 0 => {
            (visible_count / 2).max(1)
        }
        ScreenMotion::HalfPageDown | ScreenMotion::HalfPageUp => count,
        ScreenMotion::ScrollDown | ScreenMotion::ScrollUp => count.max(1),
    };
    let amount = isize::try_from(amount_rows).unwrap_or(isize::MAX);
    let row_delta = amount * direction;
    let new_first = shift_row_index(snapshot, visible[0], row_delta)?;
    // Padding and paragraph spacing belong to the document, not its first or
    // last row. Saturate at the same endpoints as an absolute scrollbar move.
    let requested = if direction < 0 && new_first == 0 && snapshot.contains_document_start() {
        0.0
    } else if direction > 0 && new_first == snapshot.rows.len() - 1 && snapshot.contains_document_end() {
        snapshot.maximum_viewport_top(viewport.height).expect("materialized document end")
    } else {
        // Preserve the within-row offset from continuous scrolling or a prior
        // margin reveal. Re-aligning to row.y can cancel a one-row step.
        snapshot.rows[new_first].y + (viewport.top - snapshot.rows[visible[0]].y)
    };
    let mut viewport = Viewport {
        top: clamped_viewport_top(snapshot, requested, viewport.height)?,
        height: viewport.height,
    };
    let new_visible = visible_rows(snapshot, viewport)?;
    let inside: Vec<_> = new_visible.iter().copied().filter(|index| {
        let row = &snapshot.rows[*index];
        let bounds = row.reveal_bounds();
        let visible = snapshot.reveal_vertical_range(row, viewport.height);
        bounds.start >= viewport.top + visible.start
            && bounds.end <= viewport.top + visible.end
    }).collect();
    let caret_rows = if inside.is_empty() { &new_visible } else { &inside };
    let x = match desired_x {
        Some(value) if value.is_finite() => value,
        _ => located.map_or(0.0, |(_, caret)| caret.x),
    };

    let nearest_visible = || {
        if current.text_offset < snapshot.rows[caret_rows[0]].text_range.start {
            caret_rows[0]
        } else {
            *caret_rows.last().expect("visible rows is non-empty")
        }
    };

    let target_row = match motion {
        ScreenMotion::PageDown
        | ScreenMotion::PageUp
        | ScreenMotion::HalfPageDown
        | ScreenMotion::HalfPageUp => {
            if let Some((current_row, _)) = located.filter(|(row, _)| visible.contains(row)) {
                shift_row_index(snapshot, current_row, row_delta)?
            } else {
                // A preceding wheel/scrollbar move can leave the caret off
                // screen. Paging follows the displayed viewport, rather than
                // revealing that old caret in the opposite scroll direction.
                nearest_visible()
            }
        }
        ScreenMotion::ScrollDown | ScreenMotion::ScrollUp => {
            // Row scrolling owns the viewport. Move an obscured caret into
            // its reserved area instead of moving the viewport back to it.
            if let Some((current_row, _)) = located.filter(|(row, _)| caret_rows.contains(row)) {
                current_row
            } else {
                nearest_visible()
            }
        }
    };
    if !snapshot.horizontal_geometry_is_materialized(target_row, x) {
        return Err(horizontal_demand(snapshot, current.text_offset, Some(x)));
    }
    let target =
        nearest_caret(&snapshot.rows[target_row], x).ok_or(LayoutMotionError::EmptyLayout)?;
    // Away from an explicit document endpoint, include the active row's full
    // ink and configured margins. Finalize this here even when its text point
    // is unchanged; the coordinator must not apply a second scroll policy.
    let at_start = viewport.top == 0.0 && snapshot.contains_document_start();
    let at_end = snapshot.maximum_viewport_top(viewport.height) == Some(viewport.top);
    if !at_start && !at_end && !matches!(motion, ScreenMotion::ScrollDown | ScreenMotion::ScrollUp) {
        viewport.top = clamped_viewport_top(snapshot,
            snapshot.reveal_viewport_top(&snapshot.rows[target_row], viewport.top, viewport.height),
            viewport.height)?;
    }
    Ok(ScreenMotionResult {
        motion: VisualMotionResult {
            position: position_of(target),
            desired_x: x,
        },
        viewport,
    })
}

fn locate(
    snapshot: &LayoutSnapshot,
    position: VisualPosition,
) -> Result<(usize, &PositionedCaret), LayoutMotionError> {
    if !snapshot.coverage.contains_text_offset(position.text_offset) {
        let before = snapshot
            .rows
            .first()
            .is_some_and(|row| position.text_offset < row.hard_line_range.start);
        let after = snapshot
            .rows
            .last()
            .is_some_and(|row| position.text_offset > row.hard_line_range.end);
        return Err(outside_materialized_coverage(
            snapshot,
            match (before, after) {
                (true, false) => LayoutDemandEdge::Before,
                (false, true) => LayoutDemandEdge::After,
                _ => LayoutDemandEdge::Both,
            },
            1,
        ));
    }
    for (row_index, row) in snapshot.rows.iter().enumerate() {
        if let Some(caret) = row.carets.iter().find(|caret| {
            caret.point.text_offset == position.text_offset
                && caret.point.affinity == position.affinity
        }) {
            return Ok((row_index, caret));
        }
    }
    if !snapshot.horizontal_text_is_materialized(position.text_offset) {
        return Err(horizontal_demand(snapshot, position.text_offset, None));
    }
    Err(if snapshot.rows.is_empty() {
        LayoutMotionError::EmptyLayout
    } else {
        LayoutMotionError::PositionNotInLayout(position)
    })
}

fn position_of(caret: &PositionedCaret) -> VisualPosition {
    VisualPosition {
        text_offset: caret.point.text_offset,
        affinity: caret.point.affinity,
    }
}

fn row_start(row: &VisualRow) -> VisualPosition {
    VisualPosition {
        text_offset: row.text_range.start,
        affinity: BoundaryAffinity::Downstream,
    }
}

fn row_end(row: &VisualRow) -> VisualPosition {
    VisualPosition {
        text_offset: row.text_range.end,
        affinity: BoundaryAffinity::Upstream,
    }
}

fn first_non_blank(
    row: &VisualRow,
    formatted_text: &str,
) -> Result<VisualPosition, LayoutMotionError> {
    let Some(slice) = formatted_text.get(row.text_range.clone()) else {
        return Err(LayoutMotionError::TextDoesNotMatchLayout);
    };
    let offset = slice
        .grapheme_indices(true)
        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .map_or(row.text_range.start, |(offset, _)| {
            row.text_range.start + offset
        });
    Ok(VisualPosition {
        text_offset: offset,
        affinity: BoundaryAffinity::Downstream,
    })
}

fn visible_rows(
    snapshot: &LayoutSnapshot,
    viewport: Viewport,
) -> Result<Vec<usize>, LayoutMotionError> {
    if snapshot.rows.is_empty() {
        return Err(LayoutMotionError::EmptyLayout);
    }
    ensure_viewport_covered(snapshot, viewport)?;
    let bottom = viewport.top + viewport.height;
    let mut rows: Vec<usize> = snapshot
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.y < bottom && row.y + row.height() > viewport.top)
        .map(|(index, _)| index)
        .collect();
    if rows.is_empty() {
        let nearest = snapshot
            .rows
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                distance_to_row(viewport.top, left)
                    .partial_cmp(&distance_to_row(viewport.top, right))
                    .unwrap_or(Ordering::Equal)
            })
            .map(|(index, _)| index)
            .ok_or(LayoutMotionError::EmptyLayout)?;
        rows.push(nearest);
    }
    Ok(rows)
}

fn distance_to_row(y: f32, row: &VisualRow) -> f32 {
    if y < row.y {
        row.y - y
    } else if y > row.y + row.height() {
        y - (row.y + row.height())
    } else {
        0.0
    }
}

fn clamped_viewport_top(
    snapshot: &LayoutSnapshot,
    requested: f32,
    height: f32,
) -> Result<f32, LayoutMotionError> {
    let top = snapshot.clamp_viewport_top(requested, height);
    ensure_viewport_covered(
        snapshot,
        Viewport {
            top,
            height,
        },
    )?;
    Ok(top)
}

fn shift_row_index(
    snapshot: &LayoutSnapshot,
    index: usize,
    delta: isize,
) -> Result<usize, LayoutMotionError> {
    let length = snapshot.rows.len();
    if length == 0 {
        return Err(LayoutMotionError::EmptyLayout);
    }
    if delta < 0 {
        let amount = delta.unsigned_abs();
        if amount > index && !snapshot.contains_document_start() {
            Err(outside_materialized_coverage(
                snapshot,
                LayoutDemandEdge::Before,
                amount - index,
            ))
        } else {
            Ok(index.saturating_sub(amount))
        }
    } else {
        let target = index.saturating_add(delta as usize);
        if target >= length && !snapshot.contains_document_end() {
            Err(outside_materialized_coverage(
                snapshot,
                LayoutDemandEdge::After,
                target - (length - 1),
            ))
        } else {
            Ok(target.min(length - 1))
        }
    }
}

fn ensure_viewport_covered(
    snapshot: &LayoutSnapshot,
    viewport: Viewport,
) -> Result<(), LayoutMotionError> {
    let (before, after) = snapshot.missing_viewport_edges(viewport.top, viewport.height);
    if before || after {
        Err(outside_materialized_coverage(
            snapshot,
            match (before, after) {
                (true, true) => LayoutDemandEdge::Both,
                (true, false) => LayoutDemandEdge::Before,
                (false, true) => LayoutDemandEdge::After,
                (false, false) => unreachable!("the viewport is covered"),
            },
            1,
        ))
    } else {
        Ok(())
    }
}

fn outside_materialized_coverage(
    snapshot: &LayoutSnapshot,
    edge: LayoutDemandEdge,
    minimum_additional_visual_rows: usize,
) -> LayoutMotionError {
    const MAX_DEMAND_EXTENSION_HARD_LINES: usize = 4_096;

    let materialized = snapshot.coverage.hard_lines();
    let document_hard_line_count = snapshot.coverage.document_hard_line_count();
    let minimum_additional_visual_rows =
        minimum_additional_visual_rows.clamp(1, MAX_DEMAND_EXTENSION_HARD_LINES);
    // One hard line always contributes at least one visual row. Extending by
    // the missing row count is therefore conservative even when wrapping adds
    // more rows; using at least the current span also provides useful bounded
    // viewport overscan for one-row edge crossings.
    let extension = materialized
        .len()
        .max(minimum_additional_visual_rows)
        .clamp(1, MAX_DEMAND_EXTENSION_HARD_LINES);
    let requested_start = if matches!(edge, LayoutDemandEdge::Before | LayoutDemandEdge::Both) {
        materialized.start.saturating_sub(extension)
    } else {
        materialized.start
    };
    let requested_end = if matches!(edge, LayoutDemandEdge::After | LayoutDemandEdge::Both) {
        materialized
            .end
            .saturating_add(extension)
            .min(document_hard_line_count)
    } else {
        materialized.end
    };
    LayoutMotionError::OutsideMaterializedCoverage(LayoutDemand {
        document_id: snapshot.document_id,
        document_revision: snapshot.document_revision,
        layout_revision: snapshot.revision,
        configuration_generation: snapshot.configuration_generation,
        metrics_generation: snapshot.metrics_generation,
        edge,
        materialized_hard_lines: materialized,
        requested_hard_lines: requested_start..requested_end,
        minimum_additional_visual_rows,
        horizontal_focus_offset: None,
        horizontal_x_bits: None,
        complete_horizontal_geometry: false,
    })
}

fn horizontal_demand(snapshot: &LayoutSnapshot, offset: usize, x: Option<f32>) -> LayoutMotionError {
    let materialized = snapshot.coverage.hard_lines();
    LayoutMotionError::OutsideMaterializedCoverage(LayoutDemand {
        document_id: snapshot.document_id, document_revision: snapshot.document_revision,
        layout_revision: snapshot.revision, configuration_generation: snapshot.configuration_generation,
        metrics_generation: snapshot.metrics_generation, edge: LayoutDemandEdge::Both,
        materialized_hard_lines: materialized.clone(), requested_hard_lines: materialized,
        minimum_additional_visual_rows: 0,
        horizontal_focus_offset: Some(offset), horizontal_x_bits: x.map(f32::to_bits),
        complete_horizontal_geometry: false,
    })
}

pub(crate) fn complete_horizontal_demand(snapshot: &LayoutSnapshot, offset: usize) -> LayoutMotionError {
    let LayoutMotionError::OutsideMaterializedCoverage(mut demand) = horizontal_demand(snapshot, offset, None) else { unreachable!() };
    demand.complete_horizontal_geometry = true;
    LayoutMotionError::OutsideMaterializedCoverage(demand)
}

fn validate_viewport(viewport: Viewport) -> Result<(), LayoutMotionError> {
    if !viewport.top.is_finite()
        || !viewport.height.is_finite()
        || viewport.height <= 0.0
        || !(viewport.top + viewport.height).is_finite()
    {
        return Err(LayoutMotionError::InvalidViewport);
    }
    Ok(())
}

fn positive_count(count: usize) -> isize {
    isize::try_from(count.max(1)).unwrap_or(isize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::layout::{
        compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
        LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
        LayoutJobId, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider, ViewLayout,
        ViewportLayoutRegion,
    };

    fn snapshot_for(text: &str, width: f32) -> (Document, LayoutSnapshot) {
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(width, 500.0);
        engine.relayout(&document, &mut view).unwrap();
        (document, view.snapshot().unwrap().clone())
    }

    fn partial_snapshot_for(
        text: &str,
        hard_lines: std::ops::Range<usize>,
    ) -> (Document, LayoutSnapshot) {
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(500.0, 10.0);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(hard_lines, 16.0, 10.0).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        install_layout_job(
            &mut view,
            LayoutInstallTarget {
                document_id: document.id(),
                document_revision: document.revision(),
                measurement_environment_id: requirements.measurement_environment_id,
                metrics_generation: requirements.metrics_generation,
            },
            candidate,
        )
        .unwrap();
        (document, view.snapshot().unwrap().clone())
    }

    #[test]
    fn g_motions_address_visual_rows() {
        let (document, snapshot) = snapshot_for("one two three", 55.0);
        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| row.text_range.clone())
                .collect::<Vec<_>>(),
            vec![0..4, 4..8, 8..13]
        );
        let start = VisualPosition {
            text_offset: 0,
            affinity: BoundaryAffinity::Downstream,
        };
        let down = gj(&snapshot, start, 1, None).unwrap();
        assert_eq!(down.position.text_offset, 4);
        assert_eq!(g0(&snapshot, down.position).unwrap().text_offset, 4);
        assert_eq!(
            g_dollar(&snapshot, down.position).unwrap(),
            VisualPosition {
                text_offset: 8,
                affinity: BoundaryAffinity::Upstream,
            }
        );
        assert_eq!(
            gk(&snapshot, down.position, 1, Some(down.desired_x))
                .unwrap()
                .position,
            start
        );
        assert_eq!(document.text(), "one two three");
    }

    #[test]
    fn g_caret_uses_first_nonblank_grapheme() {
        let (document, snapshot) = snapshot_for("   e\u{301}x", 500.0);
        let end = VisualPosition {
            text_offset: document.text().len(),
            affinity: BoundaryAffinity::Upstream,
        };
        assert_eq!(
            g_caret(&snapshot, document.text(), end).unwrap(),
            VisualPosition {
                text_offset: 3,
                affinity: BoundaryAffinity::Downstream,
            }
        );
    }

    #[test]
    fn document_home_scan_matches_grapheme_whitespace_semantics() {
        for text in ["", " \t", "\u{2003}\u{a0}x", " \u{301}x", " \t e\u{301}x"] {
            let document = Document::new(text);
            let expected = text.grapheme_indices(true)
                .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
                .map_or(0, |(offset, _)| offset);
            assert_eq!(first_non_blank_for_document(&document, 0..text.len()).unwrap(),
                VisualPosition { text_offset: expected, affinity: BoundaryAffinity::Downstream });
            assert!(!document.projection().compatibility_text_is_materialized());
        }
        let document = Document::new("prefix \u{301}suffix");
        assert_eq!(first_non_blank_for_document(&document, 6..15).unwrap().text_offset, 6);
        assert_eq!(first_non_blank_for_document(&document, 7..15),
            Err(LayoutMotionError::TextDoesNotMatchLayout), "ranges cannot split graphemes");
    }

    #[test]
    fn document_home_giant_whitespace_and_combining_runs_keep_flat_text_unmaterialized() {
        let bytes = 2 * 1024 * 1024;
        for (text, expected) in [
            (" ".repeat(bytes), 0),
            (format!("{}\u{301}x", " ".repeat(bytes)), bytes - 1),
            (format!(" {}x", "\u{301}".repeat(bytes / 2)), 0),
        ] {
            let document = Document::new(&text);
            assert_eq!(first_non_blank_for_document(&document, 0..text.len()).unwrap().text_offset,
                expected);
            assert!(!document.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn document_home_insert_and_replace_keep_large_literal_documents_unflattened() {
        use crate::command::{CommandStatus, InputEvent, Key, Mode};
        use crate::document::{Encoding, Format};
        use crate::{Core, CoreEvent};

        let bytes = 2 * 1024 * 1024;
        let ordinary = "   ordinary words on a logical line with enough text to wrap\n";
        for (text, wrap, focus, sparse) in [
            (ordinary.repeat(40_000), true, ordinary.len() * 20_000 + 15, false),
            ("a".repeat(bytes), true, bytes / 2, true),
            (format!("{}tail", " ".repeat(bytes)), false, 12, true),
        ] {
            for format in [Format::PlainText, Format::Code] {
                for (key, mode) in [(Key::Char('i'), Mode::Insert), (Key::Char('R'), Mode::Replace)] {
                    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                    let mut core = Core::new(document);
                    let view = core.add_view(MockTextMeasurementProvider::new(), 180.0, 100.0);
                    core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
                    core.handle(view, CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(), text_offset: focus,
                        affinity: BoundaryAffinity::Downstream, extend_selection: false,
                    }).unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                    let commands = core.command_state(view).unwrap();
                    assert_eq!(commands.mode(), mode);
                    let current = VisualPosition {
                        text_offset: commands.cursor(), affinity: commands.boundary_affinity(),
                    };
                    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                    assert_eq!(snapshot.has_horizontal_materialization(), sparse);
                    let expected = g_caret(snapshot, &text, current).unwrap();
                    assert!(!core.document().projection().compatibility_text_is_materialized(),
                        "fixture should reach Home without a flat copy: {format:?} {mode:?}");
                    let result = core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Home))).unwrap();
                    assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
                    let commands = core.command_state(view).unwrap();
                    assert_eq!(commands.cursor(), expected.text_offset, "{format:?} {mode:?}");
                    assert_eq!(commands.boundary_affinity(), expected.affinity);
                    assert_eq!(commands.mode(), mode);
                    assert!(!core.document().projection().compatibility_text_is_materialized(),
                        "Home must scan tree leaves without copying the document: {format:?} {mode:?}");
                    assert!(!core.document().history_status().is_dirty);
                }
            }
        }
    }

    #[test]
    fn vertical_motion_retains_layout_x_across_mixed_advances() {
        let (_document, snapshot) = snapshot_for("iiii\nWWWW", 500.0);
        let initial = VisualPosition {
            text_offset: 3,
            affinity: BoundaryAffinity::Downstream,
        };
        let down = gj(&snapshot, initial, 1, None).unwrap();
        assert_eq!(down.position.text_offset, 6);
        let back = gk(&snapshot, down.position, 1, Some(down.desired_x)).unwrap();
        assert_eq!(back.position, initial);
        assert_eq!(back.desired_x, down.desired_x);
    }

    #[test]
    fn viewport_h_m_l_use_visible_rows_and_counts() {
        let (document, snapshot) = snapshot_for("  a\n  b\n  c\n  d\n  e", 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(snapshot.rows[1].y, row_height * 3.0).unwrap();
        assert_eq!(
            viewport_line(&snapshot, document.text(), viewport, ViewportLine::Top, 1)
                .unwrap()
                .text_offset,
            6
        );
        assert_eq!(
            viewport_line(
                &snapshot,
                document.text(),
                viewport,
                ViewportLine::Middle,
                1
            )
            .unwrap()
            .text_offset,
            10
        );
        assert_eq!(
            viewport_line(
                &snapshot,
                document.text(),
                viewport,
                ViewportLine::Bottom,
                2
            )
            .unwrap()
            .text_offset,
            10
        );
    }

    #[test]
    fn page_and_scroll_motions_update_viewport_and_cursor() {
        let text = (0..10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (_document, snapshot) = snapshot_for(&text, 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(0.0, row_height * 5.0).unwrap();
        let first = VisualPosition {
            text_offset: 0,
            affinity: BoundaryAffinity::Downstream,
        };

        let page =
            screen_motion(&snapshot, first, viewport, ScreenMotion::PageDown, 1, None).unwrap();
        assert_eq!(page.motion.position.text_offset, 6);
        assert_eq!(page.viewport.top, snapshot.rows[3].y);

        let scroll = screen_motion(
            &snapshot,
            first,
            viewport,
            ScreenMotion::ScrollDown,
            1,
            None,
        )
        .unwrap();
        assert_eq!(scroll.motion.position.text_offset, 2);
        assert_eq!(scroll.viewport.top, snapshot.rows[1].y);
    }

    #[test]
    fn half_page_motion_distinguishes_dynamic_default_from_exact_row_amount() {
        let text = (0..10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (_document, snapshot) = snapshot_for(&text, 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(0.0, row_height * 5.0).unwrap();
        let first = VisualPosition {
            text_offset: 0,
            affinity: BoundaryAffinity::Downstream,
        };

        let default_half = screen_motion(
            &snapshot,
            first,
            viewport,
            ScreenMotion::HalfPageDown,
            0,
            None,
        )
        .unwrap();
        assert_eq!(default_half.motion.position.text_offset, 4);
        assert!((default_half.viewport.top - snapshot.rows[2].y).abs() < 0.001);

        let exact_one = screen_motion(
            &snapshot,
            first,
            viewport,
            ScreenMotion::HalfPageDown,
            1,
            None,
        )
        .unwrap();
        assert_eq!(exact_one.motion.position.text_offset, 2);
        assert!((exact_one.viewport.top - snapshot.rows[1].y).abs() < 0.001);
    }

    #[test]
    fn z_commands_align_the_current_visual_row() {
        let text = (0..10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (_document, snapshot) = snapshot_for(&text, 500.0);
        let row_height = snapshot.rows[0].height();
        let viewport = Viewport::new(0.0, row_height * 5.0).unwrap();
        let current = VisualPosition {
            text_offset: 10,
            affinity: BoundaryAffinity::Downstream,
        };

        let top = align_viewport(&snapshot, current, viewport, ViewportAlignment::Top).unwrap();
        let middle =
            align_viewport(&snapshot, current, viewport, ViewportAlignment::Middle).unwrap();
        let bottom =
            align_viewport(&snapshot, current, viewport, ViewportAlignment::Bottom).unwrap();
        assert!((top.top - snapshot.rows[5].y).abs() < 0.001);
        assert!((middle.top - snapshot.rows[3].y).abs() < 0.001);
        assert!((bottom.top - snapshot.rows[1].y).abs() < 0.001);
    }

    #[test]
    fn invalid_or_visually_indivisible_position_is_not_guessed() {
        let (_document, snapshot) = snapshot_for("office", 500.0);
        let inside_ligature = VisualPosition {
            text_offset: 2,
            affinity: BoundaryAffinity::Downstream,
        };
        assert_eq!(
            gj(&snapshot, inside_ligature, 1, None),
            Err(LayoutMotionError::PositionNotInLayout(inside_ligature))
        );
    }

    #[test]
    fn partial_snapshot_edges_request_more_layout_instead_of_clamping() {
        let (document, snapshot) = partial_snapshot_for("zero\none\ntwo", 1..2);
        let current = VisualPosition {
            text_offset: document.line_start(1).unwrap(),
            affinity: BoundaryAffinity::Downstream,
        };
        let before = match gk(&snapshot, current, 1, None).unwrap_err() {
            LayoutMotionError::OutsideMaterializedCoverage(demand) => demand,
            error => panic!("unexpected layout error: {error:?}"),
        };
        assert_eq!(before.edge(), LayoutDemandEdge::Before);
        assert_eq!(before.materialized_hard_lines(), 1..2);
        assert_eq!(before.requested_hard_lines(), 0..2);

        let after = match gj(&snapshot, current, 1, None).unwrap_err() {
            LayoutMotionError::OutsideMaterializedCoverage(demand) => demand,
            error => panic!("unexpected layout error: {error:?}"),
        };
        assert_eq!(after.edge(), LayoutDemandEdge::After);
        assert_eq!(after.requested_hard_lines(), 1..3);

        let outside_position = match g0(
            &snapshot,
            VisualPosition {
                text_offset: 0,
                affinity: BoundaryAffinity::Downstream,
            },
        )
        .unwrap_err()
        {
            LayoutMotionError::OutsideMaterializedCoverage(demand) => demand,
            error => panic!("unexpected layout error: {error:?}"),
        };
        assert_eq!(outside_position.edge(), LayoutDemandEdge::Before);

        let coverage = snapshot.coverage.vertical_range().unwrap();
        let outside = Viewport::new(coverage.end, 10.0).unwrap();
        let outside_viewport =
            match viewport_line(&snapshot, document.text(), outside, ViewportLine::Top, 1)
                .unwrap_err()
            {
                LayoutMotionError::OutsideMaterializedCoverage(demand) => demand,
                error => panic!("unexpected layout error: {error:?}"),
            };
        assert_eq!(outside_viewport.edge(), LayoutDemandEdge::After);
    }
}

pub(crate) fn command_row_span(
    snapshot: &LayoutSnapshot,
    current: VisualPosition,
    delta: isize,
) -> Result<Range<usize>, LayoutMotionError> {
    let (index, _) = locate(snapshot, current)?;
    let target = shift_row_index(snapshot, index, delta)?;
    Ok(index.min(target)..index.max(target) + 1)
}
