//! Stable screen placement while editing and refining syntax metrics.

use super::*;
use crate::layout::{CaretGeometry, LayoutSnapshot, ViewLayout};

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
    let overlay = if let Some(composition) = &view.composition {
        composition.overlay(document).ok()?
    } else {
        view.completion.as_ref()?.overlay(document).ok()??
    };
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
        .or_else(|_| snapshot.logical_endpoint_geometry(position.text_offset, match position.affinity {
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
            BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
        }))
        .ok()?;
    let row = &snapshot.rows[geometry.row_index];
    let top = view.layout.viewport_top();
    if row.baseline < top || row.baseline > top + view.layout.height() {
        return None;
    }
    // A sparse unwrapped row can retain its whole logical range while only
    // the band around the caret has geometry. Its offscreen start can disappear
    // from the next band, so use the visible caret as that edit's anchor.
    let row_start = row_start && snapshot
        .logical_endpoint_geometry(row.text_range.start, BoundaryAffinity::Downstream)
        .is_ok_and(|geometry| {
            geometry.rect.x >= view.layout.viewport_left()
                && geometry.rect.x <= view.layout.viewport_left() + view.layout.width()
        });
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
    let row = &snapshot.rows[geometry.row_index];
    let top = view.layout.reveal_viewport_top(row, view.layout.viewport_top());
    view.layout.set_viewport_top(top)
}

pub(super) fn reveal_caret_row<P: TextMeasurementProvider>(
    document: &Document,
    view: &mut View<P>,
) -> Result<(), LayoutError> {
    let target = view.commands.caret_target(document);
    reveal_layout_range(&mut view.layout, target.range(), target.affinity())
}

impl<P: TextMeasurementProvider> Core<P> {
    /// History retains the current viewport, then reveals the changed area
    /// with the smallest movement. Large or sparsely materialized changes
    /// fall back to the restored caret without shaping the whole edit.
    pub(super) fn reveal_history_change(&mut self, view_id: ViewId, map: &PositionMap, previous: crate::document::HistoryNodeId) -> Result<(), CoreError> {
        let snapshot = self.views[&view_id].layout.snapshot().ok_or(LayoutError::NoRows)?;
        if snapshot.document_id != self.document.id() { return Err(LayoutError::WrongDocument.into()); }
        if snapshot.document_revision != self.document.revision() { return Err(LayoutError::WrongDocumentRevision.into()); }
        let result = reveal_caret_row(&self.document, self.views.get_mut(&view_id).expect("validated history view"));
        match result {
            Err(LayoutError::OutsideMaterializedCoverage) => {
                self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
            }
            result => result?,
        }
        let cursor = self.views[&view_id].commands.cursor();
        let changed = map.replacements().map(|(_, new)| new).reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .or_else(|| self.document.history_formatting_change_range(previous));
        let view = self.views.get_mut(&view_id).expect("validated history view");
        if let Some(changed) = changed {
            reveal_changed_area(&mut view.layout, changed, cursor)?;
        }
        // The logical range edge can be on the other visual side in bidi
        // text. The restored caret's actual affinity and full cell win.
        reveal_caret_row(&self.document, view)?;
        update_viewport_anchor(&self.document, view);
        if viewport_extension_needed(view) != (false, false) {
            self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::PreserveViewport)?;
        }
        Ok(())
    }
}

fn reveal_changed_area(layout: &mut ViewLayout, changed: std::ops::Range<usize>, cursor: usize) -> Result<(), LayoutError> {
    let snapshot = layout.snapshot().ok_or(LayoutError::NoRows)?;
    let range = changed.start.min(cursor)..changed.end.max(cursor);
    // A partial long-line snapshot can have holes. Adjacent hard-line ranges
    // omit their one-byte break; larger gaps do not supply exact geometry.
    if let crate::layout::LayoutCoverage::PartialHardLines { text_ranges, .. } = &snapshot.coverage {
        let mut end = range.start;
        for covered in text_ranges.iter().filter(|covered| covered.end >= range.start) {
            if covered.start > end.saturating_add(1) { break; }
            end = end.max(covered.end);
            if end >= range.end { break; }
        }
        if end < range.end { return Ok(()); }
    }
    let first = snapshot.logical_endpoint_geometry(range.start, BoundaryAffinity::Downstream);
    let last = snapshot.logical_endpoint_geometry(range.end, BoundaryAffinity::Upstream);
    let (Ok(first), Ok(last)) = (first, last) else { return Ok(()); };
    let rows = &snapshot.rows[first.row_index.min(last.row_index)..=first.row_index.max(last.row_index)];
    let mut vertical = f32::INFINITY..f32::NEG_INFINITY;
    let mut horizontal = first.rect.x.min(last.rect.x)..(first.rect.x + first.rect.width).max(last.rect.x + last.rect.width);
    let mut covered_bytes = 0;
    for row in rows {
        let bounds = row.reveal_bounds();
        vertical.start = vertical.start.min(bounds.start);
        vertical.end = vertical.end.max(bounds.end);
        for cluster in row.clusters.iter().filter(|cluster| cluster.text_range.start < range.end && range.start < cluster.text_range.end) {
            covered_bytes += cluster.text_range.end.min(range.end) - cluster.text_range.start.max(range.start);
            horizontal.start = horizontal.start.min(cluster.x.min(cluster.typographic_bounds.x));
            horizontal.end = horizontal.end.max((cluster.x + cluster.advance).max(cluster.typographic_bounds.x + cluster.typographic_bounds.width));
        }
    }
    let top = layout.reveal_fitting_vertical_bounds(vertical);
    // Sparse horizontal geometry is insufficient to fit the whole change.
    let left = if horizontal.end - horizontal.start <= layout.width()
        && (!snapshot.has_horizontal_materialization() || covered_bytes == range.len()) {
        layout.reveal_viewport_left(horizontal, first.rect.x)
    } else { layout.viewport_left() };
    layout.set_viewport_top(top)?;
    layout.set_viewport_left(left)
}

pub(super) fn reveal_presentation_caret_row<P: TextMeasurementProvider>(
    document: &Document,
    view: &mut View<P>,
) -> Result<(), LayoutError> {
    if let Some(range) = view.search_preview_range(document) {
        reveal_search_match(&mut view.layout, range)
    } else {
        reveal_caret_row(document, view)
    }
}

/// Source-backed and composed layouts share one row reveal policy. Reserve
/// the application margins without reducing painting or materialization
/// coverage, and include the complete row's typography and ink.
pub(super) fn reveal_layout_row_at(
    layout: &mut ViewLayout,
    offset: usize,
    affinity: BoundaryAffinity,
) -> Result<(), LayoutError> {
    reveal_layout_range(layout, offset..offset, affinity)
}

/// Search starts with the logical cursor centered, then shifts only enough
/// to fit the match's first visual row. Oversized matches expose their start.
pub(super) fn reveal_search_match(
    layout: &mut ViewLayout,
    range: std::ops::Range<usize>,
) -> Result<(), LayoutError> {
    reveal_range(layout, range, BoundaryAffinity::Downstream, true)
}

fn reveal_layout_range(
    layout: &mut ViewLayout,
    range: std::ops::Range<usize>,
    affinity: BoundaryAffinity,
) -> Result<(), LayoutError> {
    reveal_range(layout, range, affinity, false)
}

/// Read only materialized clusters, including indivisible shaping/regex
/// endpoints. Ordinary cursor movement keeps the minimum-reveal policy.
fn reveal_range(
    layout: &mut ViewLayout,
    range: std::ops::Range<usize>,
    affinity: BoundaryAffinity,
    center_search: bool,
) -> Result<(), LayoutError> {
    let offset = range.start;
    let snapshot = layout.snapshot().ok_or(LayoutError::NoRows)?;
    let geometry = snapshot.logical_endpoint_geometry(offset, affinity).or_else(|_| {
        snapshot.logical_endpoint_geometry(offset, match affinity {
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
            BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
        })
    })?;
    let row = &snapshot.rows[geometry.row_index];
    let top = layout.reveal_viewport_top(row, layout.viewport_top());
    let cursor_bounds = geometry.rect.x..geometry.rect.x + geometry.rect.width.max(crate::layout::CARET_REVEAL_WIDTH);
    let mut bounds = cursor_bounds.clone();
    let first_row_end = range.end.min(row.text_range.end);
    let mut covered_bytes = 0;
    let right_to_left = row.clusters.iter()
        .find(|cluster| cluster.text_range.contains(&range.start))
        .is_some_and(|cluster| cluster.bidi_level % 2 == 1);
    if !range.is_empty() {
        let mut selected = row.clusters.iter().filter(|cluster| {
            cluster.text_range.start < range.end && range.start < cluster.text_range.end
        });
        if let Some(first) = selected.next() {
            let mut cluster_bounds = |cluster: &crate::layout::PositionedCluster| {
                // Shaped clusters partition logical text, even when their
                // visual order is reversed. Count coverage to detect gaps in
                // sparse snapshots; isolated end probes do not fill them.
                covered_bytes += cluster.text_range.end.min(first_row_end)
                    .saturating_sub(cluster.text_range.start.max(range.start));
                cluster.x.min(cluster.typographic_bounds.x)
                    ..(cluster.x + cluster.advance).max(cluster.typographic_bounds.x + cluster.typographic_bounds.width)
            };
            bounds = cluster_bounds(first);
            for cluster in selected {
                let next = cluster_bounds(cluster);
                bounds.start = bounds.start.min(next.start);
                bounds.end = bounds.end.max(next.end);
            }
        }
    }
    let left = if center_search {
        // An indivisible cluster has no caret stop at the logical endpoint.
        // Its fallback rectangle is visual; use the logical leading edge.
        let cursor = geometry.rect.x + if geometry.is_cluster_fallback && right_to_left {
            geometry.rect.width
        } else {
            0.0
        };
        bounds.start = bounds.start.min(cursor_bounds.start);
        bounds.end = bounds.end.max(cursor_bounds.end).max(cursor + crate::layout::CARET_REVEAL_WIDTH);
        let complete = !snapshot.has_horizontal_materialization()
            || covered_bytes == first_row_end.saturating_sub(range.start);
        layout.reveal_search_viewport_left(bounds, cursor, right_to_left, complete)
    } else {
        layout.reveal_viewport_left(bounds, geometry.rect.x)
    };
    // Vertical movement changes the visible-row horizontal extent. Clamp x
    // only after y has reached the target row.
    layout.set_viewport_top(top)?;
    layout.set_viewport_left(left)
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
