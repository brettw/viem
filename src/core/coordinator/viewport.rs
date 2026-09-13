//! Stable screen placement while editing and refining syntax metrics.

use super::*;
use crate::layout::{CaretGeometry, LayoutSnapshot};

pub(super) fn anchor_geometry(
    snapshot: &LayoutSnapshot,
    anchor: ViewportTextAnchor,
) -> Result<CaretGeometry, LayoutError> {
    let offset = anchor.anchor.offset();
    let affinity = anchor.anchor.affinity();
    match snapshot.logical_endpoint_geometry(offset, affinity) {
        Err(LayoutError::NotACaretStop { .. }) => {
            // Inserting a hard break can leave the pre-insertion boundary at
            // a line end, where only upstream geometry exists. Reuse the same
            // logical boundary's remaining visual side; never move its text.
            let opposite = match affinity {
                BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
                BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
            };
            snapshot.logical_endpoint_geometry(offset, opposite)
        }
        result => result,
    }
}

/// Capture only a caret that is actually on screen in the displayed revision.
pub(super) fn capture_caret_baseline_anchor<P: TextMeasurementProvider>(
    document: &Document,
    view: &View<P>,
) -> Option<ViewportTextAnchor> {
    capture_baseline_anchor(document, view, false)
}

/// Preserve the original editing row, not the insertion boundary which may
/// itself advance when a growing word wraps. Syntax-only reflow instead pins
/// the current caret's baseline through its new glyph metrics.
pub(super) fn capture_edit_baseline_anchor<P: TextMeasurementProvider>(
    document: &Document,
    view: &View<P>,
) -> Option<ViewportTextAnchor> {
    capture_baseline_anchor(document, view, true)
}

pub(super) fn composition_caret_baseline<P: TextMeasurementProvider>(
    document: &Document,
    view: &View<P>,
) -> Option<f32> {
    let overlay = view.composition.as_ref()?.overlay(document).ok()?;
    let layout = view.composition_layout.as_ref()?;
    let snapshot = layout.snapshot()?;
    let offset = overlay.selected_range_in_overlay().end;
    let geometry = snapshot
        .logical_endpoint_geometry(offset, BoundaryAffinity::Downstream)
        .or_else(|_| snapshot.logical_endpoint_geometry(offset, BoundaryAffinity::Upstream))
        .ok()?;
    let baseline = snapshot.rows[geometry.row_index].baseline - layout.viewport_top();
    (baseline >= 0. && baseline <= layout.height()).then_some(baseline)
}

fn capture_baseline_anchor<P: TextMeasurementProvider>(
    document: &Document,
    view: &View<P>,
    row_start: bool,
) -> Option<ViewportTextAnchor> {
    let snapshot = view.layout.snapshot()?;
    if snapshot.document_id != document.id() || snapshot.document_revision != document.revision() {
        return None;
    }
    let position = view.commands.visual_position().unwrap_or_else(|| {
        crate::command::layout_motion::VisualPosition {
            text_offset: view.commands.cursor(),
            affinity: view.commands.boundary_affinity(),
        }
    });
    let geometry = snapshot
        .logical_endpoint_geometry(position.text_offset, position.affinity)
        .ok()?;
    let row = &snapshot.rows[geometry.row_index];
    let top = view.layout.viewport_top();
    if row.baseline < top || row.baseline > top + view.layout.height() {
        return None;
    }
    let anchor = document
        .text_anchor(
            document
                .text_point(if row_start {
                    row.text_range.start
                } else {
                    position.text_offset
                })
                .ok()?,
            Association::BeforeInsertion,
            if row_start {
                BoundaryAffinity::Downstream
            } else {
                position.affinity
            },
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .ok()?;
    Some(ViewportTextAnchor {
        anchor,
        offset_from_reference: top - row.baseline,
        reference: ViewportAnchorReference::Baseline,
    })
}

/// Baseline preservation is subordinate only to visibility of the changed
/// row, including ink that extends beyond its nominal line spacing.
pub(super) fn reveal_anchored_row<P: TextMeasurementProvider>(
    view: &mut View<P>,
    anchor: ViewportTextAnchor,
) -> Result<(), LayoutError> {
    let snapshot = view.layout.snapshot().ok_or(LayoutError::NoRows)?;
    let geometry = anchor_geometry(snapshot, anchor)?;
    reveal_row_at(view, anchor.anchor.offset(), geometry.point.affinity)
}

pub(super) fn reveal_caret_row<P: TextMeasurementProvider>(
    view: &mut View<P>,
) -> Result<(), LayoutError> {
    let position = view.commands.visual_position().unwrap_or_else(|| {
        crate::command::layout_motion::VisualPosition {
            text_offset: view.commands.cursor(),
            affinity: view.commands.boundary_affinity(),
        }
    });
    reveal_row_at(view, position.text_offset, position.affinity)
}

fn reveal_row_at<P: TextMeasurementProvider>(
    view: &mut View<P>,
    offset: usize,
    affinity: BoundaryAffinity,
) -> Result<(), LayoutError> {
    let snapshot = view.layout.snapshot().ok_or(LayoutError::NoRows)?;
    let geometry = snapshot.logical_endpoint_geometry(offset, affinity)?;
    let row = &snapshot.rows[geometry.row_index];
    let ink = row.ink_bounds();
    let top = ink.map_or(row.y, |ink| row.y.min(ink.y));
    let bottom = ink.map_or(row.y + row.natural_height(), |ink| {
        (row.y + row.natural_height()).max(ink.y + ink.height)
    });
    let height = view.layout.height();
    let current = view.layout.viewport_top();
    let requested = if bottom - top > height {
        // No viewport can contain an oversized row. Keep its baseline visible
        // without oscillating between mutually impossible top/bottom reveals.
        current.clamp((row.baseline - height).max(0.0), row.baseline.max(0.0))
    } else if top < current {
        top
    } else if bottom > current + height {
        bottom - height
    } else {
        current
    };
    view.layout.set_viewport_top(requested)
}

/// Preserve coverage above an editing row at the start of a long paragraph.
/// A regional boundary is not a document edge and must not clamp the viewport.
pub(super) fn include_preceding_rows<P: TextMeasurementProvider>(
    core: &mut Core<P>,
    view_id: ViewId,
    mut candidate: LayoutJobCandidate,
    mut required_height: f32,
) -> Result<LayoutJobCandidate, CoreError> {
    let flow = core.presentation_flow(view_id);
    let height = core.views[&view_id].layout.height().max(f32::EPSILON);
    while required_height > 0. && candidate.regional_snapshot().hard_lines().start > 0 {
        let line = candidate.regional_snapshot().hard_lines().start - 1;
        let range = core.document.projection().presentation_line_range(line, flow)
            .ok_or(LayoutJobError::InvalidDocumentLineIndex { hard_line: line })?;
        let following = candidate.regional_snapshot().clone();
        let mut checkpoint = {
            let cache = &core.views[&view_id].long_line_checkpoints;
            cache.before(range.clone(), range.end).and_then(|last| {
                cache.before_height(range.clone(), (last.completed_height() - height).max(0.))
            })
        };
        let mut tail = None;
        loop {
            let work_start = checkpoint.as_ref().map_or(range.start, LongLineLayoutCheckpoint::next_text_offset);
            let region = match checkpoint.take() {
                Some(checkpoint) => ViewportLayoutRegion::resume_long_line(checkpoint, 0., height)?,
                None => ViewportLayoutRegion::new(line..line + 1, 0., height)?,
            };
            let request = core.prepare_view_layout_job(
                view_id, LayoutJobPriority::ChangedVisibleRows,
                LayoutJobRegion::Viewport(region), LayoutCancellationToken::new(),
            )?;
            let view = core.views.get_mut(&view_id).expect("validated editing view");
            candidate = compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?;
            let next = candidate.next_long_line_checkpoint().cloned();
            candidate.retain_viewport_tail(tail.as_ref());
            if let Some(next) = next {
                if next.next_text_offset() <= work_start {
                    return Err(LayoutJobError::InvalidLongLineCheckpoint("preceding viewport continuation did not advance").into());
                }
                core.views.get_mut(&view_id).expect("validated editing view")
                    .long_line_checkpoints.insert(&core.document, next.clone());
                tail = Some(candidate.regional_snapshot().clone());
                checkpoint = Some(next);
                continue;
            }
            required_height -= candidate.regional_snapshot().lines()[0].height() as f32;
            candidate.append_adjacent_region(&following);
            break;
        }
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests;
