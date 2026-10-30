//! Logical indentation dependencies and lazy, provider-authoritative geometry.
//! The persistent index stores columns, never font metrics or source offsets.
use super::*;
use crate::document::FormattedDocument;
use crate::layout::indent_projection::IndentProjection;
use crate::layout::{IndentAnchorIndex, LineIndentation};
use unicode_width::UnicodeWidthStr;

const INDENT_SYNC_MAX_LINES: usize = 128;
const INDENT_SYNC_MAX_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IndentPrefix {
    end: usize,
    columns: usize,
    /// Recorded while scanning; columns == byte length does not prove this.
    has_tabs: bool,
    /// The prefix reaches the line end: the line is empty or only spaces and
    /// tabs. Blank lines never anchor, so alignment skips back across them.
    blank: bool,
}

impl IndentPrefix {
    fn pair(self) -> (usize, usize) { (self.end, self.columns) }
    fn summary(self) -> LineIndentation { LineIndentation { columns: self.columns, anchor_eligible: !self.blank } }
}

#[derive(Clone, Debug)]
pub(crate) struct IndentSnapshot {
    projection: Arc<IndentProjection>,
    index: IndentAnchorIndex,
    tabstop: u32,
    prefixes: BTreeMap<usize, IndentPrefix>,
}

impl PartialEq for IndentSnapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.projection, &other.projection)
            && self.tabstop == other.tabstop
            && self.index.shares_root_with(&other.index)
    }
}

impl IndentSnapshot {
    pub(crate) fn has_longer_compatible_prefix(&self, other: &Self) -> bool {
        self.tabstop == other.tabstop && self.index.len() > other.index.len()
            && self.projection.text_tree().shares_root_with(other.projection.text_tree())
            && self.projection.shares_presentation_lines_with(&other.projection)
    }

    fn new(projection: Arc<FormattedDocument>, tabstop: u32) -> Self {
        Self::new_input(Arc::new(IndentProjection::new(projection)), tabstop)
    }
    fn new_input(projection: Arc<IndentProjection>, tabstop: u32) -> Self {
        Self { projection, index: IndentAnchorIndex::default(), tabstop, prefixes: BTreeMap::new() }
    }

    /// Synchronize only the edited hard-line hull; immutable suffix blocks stay
    /// shared. The index may end before EOF until a viewport needs that prefix.
    pub(crate) fn synchronize(&mut self, next: Arc<FormattedDocument>, tabstop: u32) -> Range<usize> {
        self.synchronize_input(Arc::new(IndentProjection::new(next)), tabstop)
    }
    fn synchronize_input(&mut self, next: Arc<IndentProjection>, tabstop: u32) -> Range<usize> {
        let count = next.presentation_line_count();
        if tabstop != self.tabstop {
            *self = Self::new_input(next, tabstop);
            return 0..count;
        }
        let Some((old, new)) = self.projection.text_tree().changed_extent(next.text_tree()) else {
            if !self.projection.shares_presentation_lines_with(&next) {
                *self = Self::new_input(next, tabstop);
                return 0..count;
            }
            self.projection = next;
            return 0..0;
        };
        let start = self.projection.presentation_line_at_offset(old.start).unwrap_or(0);
        let old_end = self.projection.presentation_line_at_offset(old.end).map_or(self.index.len(), |n| n + 1);
        let new_end = next.presentation_line_at_offset(new.end).map_or(count, |n| n + 1);
        if start < self.index.len() && (old_end > self.index.len()
            || old_end.saturating_sub(start) > INDENT_SYNC_MAX_LINES
            || new_end.saturating_sub(start) > INDENT_SYNC_MAX_LINES) {
            // Check before any per-line minimum or prefix scan. A distant
            // compound edit can have a large conservative hull despite only
            // changing a few source bytes at either end.
            return self.defer_synchronization(next, start);
        }
        let mut budget = INDENT_SYNC_MAX_BYTES;
        let previous_prefix = self.prefixes.get(&start).copied();
        // An untouched body character, or a replacement beginning with an
        // ASCII body character at the exact boundary, proves the prefix
        // unchanged without walking it. Non-ASCII combining text must rescan.
        // That boundary character also makes a previously blank line nonblank.
        let unchanged_prefix = previous_prefix.filter(|prefix| old.start > prefix.end
            || old.start == prefix.end && next.text_tree().byte_chunk_at(new.start)
                .first().is_some_and(u8::is_ascii_graphic))
            .map(|prefix| IndentPrefix { blank: prefix.blank && old.start > prefix.end, ..prefix });
        let body_only = unchanged_prefix.is_some() && old_end == start + 1 && new_end == start + 1;
        let updated_prefix = unchanged_prefix.or_else(|| {
            if old_end != start + 1 || new_end != start + 1 { return None; }
            let prefix = previous_prefix?;
            let line_start = self.projection.presentation_line_range(start)?.start;
            update_space_prefix(prefix, line_start, &old, &new, &next, tabstop, &mut budget)
        });
        let old_prefixes = std::mem::take(&mut self.prefixes);
        for (line, mut prefix) in old_prefixes {
            if line < start { self.prefixes.insert(line, prefix); }
            else if line >= old_end {
                prefix.end = new.end + (prefix.end - old.end);
                self.prefixes.insert(new_end + (line - old_end), prefix);
            }
        }
        if let Some(prefix) = updated_prefix { self.prefixes.insert(start, prefix); }
        let old_count = self.projection.presentation_line_count();
        let mut dirty_end = new_end;
        if start < self.index.len() {
            // Both the old and new forest matter when indentation reparents a
            // distant suffix. One threshold search covers all changed roots.
            let threshold = (start..old_end.min(self.index.len()))
                .filter_map(|n| self.index.line(n).map(|v| v.columns)).min().unwrap_or(0);
            // A proven single-line body edit cannot change the edited line's
            // own indent x. The next equal indent inherits that unchanged x
            // and cuts off geometric propagation from the changed body.
            let maximum = if body_only { threshold } else { threshold.saturating_sub(1) };
            let old_dependency_end = self.index.following_with_max(old_end, maximum)
                .unwrap_or(old_count);
            // The boundary is in the surviving suffix, so subtract the old
            // hull first, then add its new end. Unknown suffix dependencies
            // extend to EOF, including work that another candidate captured.
            dirty_end = dirty_end.max(new_end + old_dependency_end.saturating_sub(old_end));
            {
                let mut replacement = Vec::with_capacity(new_end.saturating_sub(start));
                for line in start..new_end.min(count) {
                    let prefix = if line == start { updated_prefix } else { None };
                    let prefix = match prefix {
                        Some(prefix) => prefix,
                        None => {
                            let prefix = scan_indentation(&next, line, tabstop,
                                &LayoutRunControl::synchronous(&NeverCancelled), Some(&mut budget))
                                .expect("a retained projection has valid line and grapheme boundaries");
                            let Some(prefix) = prefix else { return self.defer_synchronization(next, start); };
                            prefix
                        }
                    };
                    if self.prefixes.len() >= 256 { self.prefixes.pop_first(); }
                    self.prefixes.insert(line, prefix);
                    replacement.push(prefix.summary());
                }
                self.index.splice(start..old_end, replacement).expect("valid indexed edit hull");
                let threshold = (start..new_end.min(self.index.len()))
                    .filter_map(|n| self.index.line(n).map(|v| v.columns)).min().unwrap_or(0);
                let maximum = if body_only { threshold } else { threshold.saturating_sub(1) };
                let end = self.index.following_with_max(new_end, maximum)
                    .unwrap_or(count);
                dirty_end = dirty_end.max(end);
            }
        } else {
            // A captured worker can already be extending this unknown suffix.
            // The owner's shorter index cannot prove a dependency barrier.
            dirty_end = count;
        }
        self.projection = next;
        start..dirty_end.min(count)
    }

    fn defer_synchronization(&mut self, next: Arc<IndentProjection>, start: usize) -> Range<usize> {
        self.index.splice(start.min(self.index.len())..self.index.len(), []).expect("truncate indexed prefix");
        self.prefixes.retain(|&line, _| line < start);
        let count = next.presentation_line_count();
        self.projection = next;
        start..count
    }

    fn ensure(&mut self, through: usize, control: &LayoutRunControl<'_>) -> Result<(), LayoutComputationError> {
        let end = through.min(self.projection.presentation_line_count());
        // Bounded blocks make cancellation observable even on a first distant
        // scroll. No unrelated suffix is indexed to display the first viewport.
        while self.index.len() < end {
            control.checkpoint()?;
            let start = self.index.len();
            let stop = (start + 128).min(end);
            let mut entries = Vec::with_capacity(stop - start);
            for line in start..stop {
                control.checkpoint()?;
                entries.push(self.prefix(line, control)?.summary());
            }
            self.index.splice(start..start, entries).expect("append at index end");
        }
        Ok(())
    }

    /// Bounded disposable prefix geometry avoids rescanning a giant indent in
    /// every shape fragment or subsequent job. Cache eviction changes no index
    /// state; an entry is only valid for this exact immutable text snapshot.
    fn indentation(&mut self, line: usize, control: &LayoutRunControl<'_>) -> Result<(usize, usize), LayoutComputationError> {
        Ok(self.prefix(line, control)?.pair())
    }

    fn prefix(&mut self, line: usize, control: &LayoutRunControl<'_>) -> Result<IndentPrefix, LayoutComputationError> {
        control.checkpoint()?;
        if let Some(&prefix) = self.prefixes.get(&line) { return Ok(prefix); }
        let prefix = scan_indentation(&self.projection, line, self.tabstop, control, None)?
            .expect("unbudgeted indentation scan completes");
        if self.prefixes.len() >= 256 { self.prefixes.pop_first(); }
        self.prefixes.insert(line, prefix);
        Ok(prefix)
    }

    pub(crate) fn dependent_end(&self, lines: &Range<usize>) -> usize {
        if lines.is_empty() { return lines.end; }
        // A missing suffix or broad metric publication has no cheap proven
        // barrier. Invalidate conservatively instead of walking the index.
        if lines.end >= self.index.len() || lines.len() > INDENT_SYNC_MAX_LINES {
            return self.projection.presentation_line_count().max(lines.end);
        }
        let threshold = (lines.start..lines.end.min(self.index.len()))
            .filter_map(|n| self.index.line(n).map(|v| v.columns)).min();
        // A zero-indent line always has x=0. Its body affects positive-indent
        // descendants only until the next zero root, even though the logical
        // anchor forest connects those roots with equal-indentation edges.
        threshold.and_then(|n| self.index.following_with_max(lines.end, n.saturating_sub(1)))
            .unwrap_or(self.projection.presentation_line_count()).max(lines.end)
    }
}

#[cfg(test)]
#[path = "indent_snapshot_tests.rs"]
mod snapshot_tests;

/// Return the line's index summary from a fresh scan of its ASCII indent,
/// without reading the line body, including multi-megabyte hard lines.
#[cfg(test)]
fn indentation_summary(projection: &IndentProjection, line: usize, tabstop: u32) -> LineIndentation {
    scan_indentation(projection, line, tabstop, &LayoutRunControl::synchronous(&NeverCancelled), None)
        .expect("a retained projection has valid line and grapheme boundaries")
        .expect("unbudgeted indentation scan completes")
        .summary()
}

#[cfg(test)]
fn indentation_cancellable(projection: &IndentProjection, line: usize, tabstop: u32,
    control: &LayoutRunControl<'_>) -> Result<(usize, usize), LayoutComputationError> {
    Ok(scan_indentation(projection, line, tabstop, control, None)?.expect("unbudgeted indentation scan completes").pair())
}

/// Update a proven all-space prefix from its small changed extent. Tabs in
/// the old prefix would make later tab stops depend on the change, so decline
/// them explicitly rather than inferring their absence from column counts.
/// Only whitespace changes, so the untouched body keeps its blankness.
fn update_space_prefix(prefix: IndentPrefix, line_start: usize, old: &Range<usize>, new: &Range<usize>,
    next: &IndentProjection, tabstop: u32, budget: &mut usize) -> Option<IndentPrefix> {
    if prefix.has_tabs || old.start < line_start || old.end > prefix.end || new.len() > *budget { return None; }
    let mut at = new.start;
    let mut column = old.start - line_start;
    let mut has_tabs = false;
    while at < new.end {
        let chunk = next.text_tree().byte_chunk_at(at);
        let length = chunk.len().min(new.end - at);
        if length == 0 { return None; }
        for &byte in &chunk[..length] {
            *budget -= 1;
            if !matches!(byte, b' ' | b'\t') { return None; }
            has_tabs |= byte == b'\t';
            let advance = if byte == b'\t' { tabstop as usize - column % tabstop as usize } else { 1 };
            column = column.checked_add(advance)?;
        }
        at += length;
    }
    Some(IndentPrefix {
        end: prefix.end.checked_sub(old.len())?.checked_add(new.len())?,
        columns: column.checked_add(prefix.end - old.end)?,
        has_tabs,
        blank: prefix.blank,
    })
}

fn scan_indentation(projection: &IndentProjection, line: usize, tabstop: u32,
    control: &LayoutRunControl<'_>, mut budget: Option<&mut usize>)
    -> Result<Option<IndentPrefix>, LayoutComputationError> {
    let Some(range) = projection.presentation_line_range(line) else {
        return Ok(Some(IndentPrefix { end: 0, columns: 0, has_tabs: false, blank: true }));
    };
    let tree = projection.text_tree();
    let mut at = range.start;
    let mut column = 0usize;
    let mut previous_column = 0;
    let mut has_tabs = false;
    while at < range.end {
        control.checkpoint()?;
        let chunk = tree.byte_chunk_at(at);
        let length = chunk.len().min(range.end - at).min(MAX_SHAPE_FRAGMENT_BYTES)
            .min(budget.as_deref().copied().unwrap_or(usize::MAX));
        if length == 0 { return Ok(None); }
        let mut consumed = 0;
        for &byte in &chunk[..length] {
            if byte != b' ' && byte != b'\t' { break; }
            has_tabs |= byte == b'\t';
            previous_column = column;
            column += if byte == b'\t' { tabstop as usize - column % tabstop as usize } else { 1 };
            consumed += 1;
        }
        if let Some(budget) = budget.as_deref_mut() {
            *budget -= consumed + usize::from(consumed < length);
        }
        at += consumed;
        if consumed < length || consumed == 0 { break; }
    }
    // A space followed by a combining mark is one body grapheme.
    if at > range.start && !tree.is_grapheme_boundary(at).map_err(|_| LayoutError::InvalidTextOffset(at))? {
        at -= 1;
        column = previous_column;
    }
    Ok(Some(IndentPrefix { end: at, columns: column, has_tabs, blank: at == range.end }))
}

/// Read only borrowed leaf chunks and resume sequential long-prefix slices.
/// The cursor is disposable; a backward request starts from the line boundary.
fn prefix_column(tree: &crate::document::FormattedTextTree, start: usize, end: usize,
    tabstop: u32, previous: Option<(usize, usize)>, control: &LayoutRunControl<'_>)
    -> Result<usize, LayoutComputationError> {
    let (mut at, mut column) = previous.filter(|(at, _)| start <= *at && *at <= end).unwrap_or((start, 0));
    while at < end {
        control.checkpoint()?;
        let chunk = tree.byte_chunk_at(at);
        let length = chunk.len().min(end - at).min(MAX_SHAPE_FRAGMENT_BYTES);
        if length == 0 { return Err(LayoutError::InvalidTextOffset(at).into()); }
        for &byte in &chunk[..length] {
            debug_assert!(byte == b' ' || byte == b'\t');
            column += if byte == b'\t' { tabstop as usize - column % tabstop as usize } else { 1 };
        }
        at += length;
    }
    Ok(column)
}

#[derive(Clone, Copy, Debug)]
enum AnchorCaret {
    Exact(usize),
    WithinCell { start: usize, end: usize, fraction: f64 },
}
impl AnchorCaret {
    fn end(self) -> usize {
        match self { Self::Exact(at) => at, Self::WithinCell { end, .. } => end }
    }
}

/// Find the native caret corresponding to a portable monospaced column. A tab
/// or double-width grapheme can contain the target column; interpolate only
/// within that cell, never between unrelated shaping clusters.
fn anchor_caret(text: &str, origin: usize, mut column: usize, target: usize, tabstop: u32)
    -> (usize, Option<AnchorCaret>) {
    for (offset, grapheme) in text.grapheme_indices(true) {
        let cells = if grapheme == "\t" { tabstop as usize - column % tabstop as usize }
            else { UnicodeWidthStr::width(grapheme) };
        let after = column + cells;
        if after >= target && cells > 0 {
            let end = origin + offset + grapheme.len();
            return (target, Some(if after == target { AnchorCaret::Exact(end) }
                else { AnchorCaret::WithinCell { start: origin + offset, end,
                    fraction: (target - column) as f64 / cells as f64 } }));
        }
        column = after;
    }
    (column, None)
}

/// Include surrounding styles for exactly the same context that the ordinary
/// streamed shaper supplies. Ownership can stop immediately after the target.
fn anchor_context(tree: &crate::document::FormattedTextTree, owned: Range<usize>, line: &Range<usize>)
    -> Result<Range<usize>, LayoutComputationError> {
    let mut start = owned.start;
    while start > line.start && owned.start - start < SHAPING_CONTEXT_BYTES {
        start = tree.previous_grapheme_boundary(start).map_err(|_| LayoutError::InvalidTextOffset(start))?
            .unwrap_or(line.start).max(line.start);
    }
    let mut end = owned.end;
    while end < line.end && end - owned.end < SHAPING_CONTEXT_BYTES {
        end = tree.next_grapheme_boundary(end).map_err(|_| LayoutError::InvalidTextOffset(end))?
            .unwrap_or(line.end).min(line.end);
    }
    Ok(start..end)
}

fn anchor_chunk_end(tree: &crate::document::FormattedTextTree, start: usize, end: usize)
    -> Result<usize, LayoutComputationError> {
    let mut stop = start.saturating_add(MAX_SHAPE_FRAGMENT_BYTES).min(end);
    while stop > start && !tree.is_char_boundary(stop).map_err(|_| LayoutError::InvalidTextOffset(stop))? { stop -= 1; }
    if tree.is_grapheme_boundary(stop).map_err(|_| LayoutError::InvalidTextOffset(stop))? { return Ok(stop); }
    let previous = tree.previous_grapheme_boundary(stop).map_err(|_| LayoutError::InvalidTextOffset(stop))?.unwrap_or(start);
    if previous > start { return Ok(previous); }
    tree.next_grapheme_boundary(start).map_err(|_| LayoutError::InvalidTextOffset(start))?
        .ok_or_else(|| LayoutError::InvalidTextOffset(start).into())
}

/// Match logical_endpoint_geometry: exact downstream native caret first, then
/// another native affinity, then the containing indivisible cluster's edge.
fn fragment_caret_x(clusters: &[ShapedCluster], at: usize) -> Result<f64, LayoutComputationError> {
    fragment_caret_x_with_affinity(clusters, at, BoundaryAffinity::Downstream)
}

fn fragment_caret_x_with_affinity(clusters: &[ShapedCluster], at: usize, affinity: BoundaryAffinity) -> Result<f64, LayoutComputationError> {
    let mut x = 0.0;
    let mut other = None;
    let mut indivisible = None;
    for cluster in clusters {
        for stop in &cluster.caret_stops {
            if stop.text_offset == at {
                let position = x + f64::from(stop.inline_offset);
                if stop.affinity == affinity { return Ok(position); }
                other = Some(position);
            }
        }
        if cluster.text_range.start < at && at < cluster.text_range.end { indivisible = Some(x); }
        x += f64::from(cluster.advance);
    }
    other.or(indivisible).ok_or_else(|| LayoutError::NotACaretStop { text_offset: at }.into())
}

fn row_caret_x(row: &VisualRow, at: usize) -> Result<f64, LayoutComputationError> {
    row_caret_x_with_affinity(row, at, BoundaryAffinity::Downstream)
}

fn row_caret_x_with_affinity(row: &VisualRow, at: usize, affinity: BoundaryAffinity) -> Result<f64, LayoutComputationError> {
    row.carets.iter().filter(|caret| caret.point.text_offset == at)
        .min_by_key(|caret| caret.point.affinity != affinity).map(|caret| f64::from(caret.x))
        .or_else(|| row.clusters.iter().find(|cluster| cluster.text_range.start < at && at < cluster.text_range.end)
            .map(|cluster| f64::from(cluster.x)))
        .ok_or_else(|| LayoutError::NotACaretStop { text_offset: at }.into())
}

#[derive(Default)]
pub(crate) struct IndentAlignment {
    pub(crate) snapshot: Option<IndentSnapshot>,
    widths: BTreeMap<usize, f64>,
    prefix_progress: BTreeMap<usize, (usize, usize)>,
}

impl<P: TextMeasurementProvider> LayoutEngine<P> {
    pub(crate) fn prepare_indent_alignment(&mut self, projection: Option<Arc<FormattedDocument>>, cached: Option<IndentSnapshot>, tabstop: u32) {
        self.indent_alignment.widths.clear();
        self.indent_alignment.prefix_progress.clear();
        self.indent_alignment.snapshot = projection.map(|projection| {
            let mut snapshot = cached.or_else(|| self.indent_alignment.snapshot.take())
                .unwrap_or_else(|| IndentSnapshot::new(projection.clone(), tabstop));
            snapshot.synchronize(projection, tabstop);
            snapshot
        });
    }

    pub(crate) fn prepare_indent_composition(&mut self, base: Arc<FormattedDocument>, tree: crate::document::FormattedTextTree, replaced: Range<usize>, marked: &str, inserted_style: ResolvedTextStyle,
        cached: Option<IndentSnapshot>, tabstop: u32) -> Result<(), LayoutError> {
        let projection = Arc::new(IndentProjection::composition(base, tree, replaced, marked, inserted_style)?);
        self.indent_alignment.widths.clear();
        self.indent_alignment.prefix_progress.clear();
        let mut snapshot = cached.unwrap_or_else(|| IndentSnapshot::new_input(projection.clone(), tabstop));
        snapshot.synchronize_input(projection, tabstop);
        self.indent_alignment.snapshot = Some(snapshot);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn aligned_indent_width_for_line(&mut self, line_start: usize,
        view: &LayoutJobViewConfiguration, style: &ResolvedTextStyle, id: DocumentId,
        revision: Revision, control: &LayoutRunControl<'_>) -> Result<Option<f64>, LayoutComputationError> {
        if !view.whitespace.format.is_code() || !view.whitespace.options.align_indents_to_previous_line
            || self.indent_alignment.snapshot.is_none() { return Ok(None); }
        if self.indent_font_is_monospaced(style, view, id, revision, control)? { return Ok(None); }
        let snapshot = self.indent_alignment.snapshot.as_mut().unwrap();
        let Some(line) = snapshot.projection.presentation_line_at_offset(line_start) else { return Ok(None); };
        let (_, columns) = snapshot.indentation(line, control)?;
        if columns == 0 { return Ok(Some(0.0)); }
        snapshot.ensure(line + 1, control)?;
        self.aligned_indent_width(line, view, style, id, revision, control).map(Some)
    }

    /// Called after ordinary whitespace geometry. Every prefix caret scales
    /// with its logical whitespace cell, so hit testing and markers agree.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn align_indent_clusters(
        &mut self, clusters: &mut [ShapedCluster], line_start: usize,
        view: &LayoutJobViewConfiguration, whitespace_style: &ResolvedTextStyle,
        document_id: DocumentId, revision: Revision, control: &LayoutRunControl<'_>,
    ) -> Result<(), LayoutComputationError> {
        if !view.whitespace.format.is_code() || !view.whitespace.options.align_indents_to_previous_line { return Ok(()); }
        if self.indent_font_is_monospaced(whitespace_style, view, document_id, revision, control)? { return Ok(()); }
        let Some(snapshot) = &self.indent_alignment.snapshot else { return Ok(()); };
        let Some(line) = snapshot.projection.presentation_line_at_offset(line_start) else { return Ok(()); };
        let projection = snapshot.projection.clone();
        let (prefix_end, columns) = self.indent_alignment.snapshot.as_mut().unwrap().indentation(line, control)?;
        if columns == 0 || clusters.first().is_none_or(|c| c.text_range.start >= prefix_end) { return Ok(()); }
        let Some(width) = self.aligned_indent_width_for_line(line_start, view, whitespace_style, document_id, revision, control)? else { return Ok(()); };
        let unit = width / columns as f64;
        // Obtain the starting logical column for a long-prefix continuation.
        let first = clusters.first().map_or(line_start, |c| c.text_range.start);
        let prefix_at = first.min(prefix_end);
        let mut column = if columns == prefix_end - line_start {
            // Every ASCII indentation byte consumes at least one column. Equal
            // totals prove unit-width bytes, so a huge spaces-only prefix needs
            // no rescanning when a later job resumes in its middle.
            prefix_at - line_start
        } else {
            prefix_column(projection.text_tree(), line_start, prefix_at, view.whitespace.tabstop,
                self.indent_alignment.prefix_progress.get(&line).copied(), control)?
        };
        if self.indent_alignment.prefix_progress.len() >= 256 && !self.indent_alignment.prefix_progress.contains_key(&line) {
            self.indent_alignment.prefix_progress.clear();
        }
        self.indent_alignment.prefix_progress.insert(line, (prefix_at, column));
        for cluster in clusters.iter_mut().take_while(|c| c.text_range.end <= prefix_end) {
            control.checkpoint()?;
            let spelling = projection.text_tree().slice(cluster.text_range.clone()).map_err(|_| LayoutError::InvalidTextOffset(cluster.text_range.start))?;
            let before = column;
            let mut stops = Vec::with_capacity(spelling.len() + 1);
            stops.push(0.0f32);
            for byte in spelling.bytes() {
                column += if byte == b'\t' { view.whitespace.tabstop as usize - column % view.whitespace.tabstop as usize } else { 1 };
                stops.push(((column - before) as f64 * unit) as f32);
            }
            cluster.advance = ((column - before) as f64 * unit) as f32;
            for stop in &mut cluster.caret_stops {
                if let Some(x) = stops.get(stop.text_offset - cluster.text_range.start) {
                    stop.inline_offset = if cluster.bidi_level % 2 == 0 { *x } else { cluster.advance - *x };
                }
            }
            cluster.typographic_bounds.width = cluster.advance;
            cluster.ink_bounds = ShapedBounds::default();
        }
        // Body tab stops depend on the corrected cumulative indent. The first
        // whitespace pass used the ordinary indent, so retarget its tabs before
        // the caller wraps or positions this fragment. Later streamed fragments
        // receive the corrected cumulative advance from the caller.
        let body_start = clusters.partition_point(|cluster| cluster.text_range.end <= prefix_end);
        if let Some(body) = clusters.get(body_start).filter(|cluster| cluster.text_range.start == prefix_end) {
            let body_range = body.text_range.start..clusters.last().unwrap().text_range.end;
            let mut at = body_range.start;
            let mut has_tab = false;
            while at < body_range.end && !has_tab {
                control.checkpoint()?;
                let chunk = projection.text_tree().byte_chunk_at(at);
                let length = chunk.len().min(body_range.end - at).min(MAX_SHAPE_FRAGMENT_BYTES);
                if length == 0 { return Err(LayoutError::InvalidTextOffset(at).into()); }
                has_tab = chunk[..length].contains(&b'\t');
                at += length;
            }
            if has_tab {
                let styles = projection.resolve_styles(body_range.clone(), view.whitespace.format).map_err(LayoutError::DocumentStyle)?;
                let default = if view.default_style_is_override { &view.default_style } else { &styles.default_shaping_style };
                let runs = if view.style_runs_are_override { view.alignment_style_runs.as_deref().unwrap_or(&view.style_runs).as_slice() } else { &styles.shaping_runs };
                let text = projection.text_tree().slice(body_range.clone()).map_err(|_| LayoutError::InvalidTextOffset(body_range.start))?;
                self.layout_whitespace_clusters(&mut clusters[body_start..], &text, body_range.start, &view.whitespace,
                    whitespace_style.size * view.scale * 0.5, default, runs, view.scale,
                    document_id, revision, control, false, width)?;
            }
        }
        Ok(())
    }

    fn indent_font_is_monospaced(&mut self, style: &ResolvedTextStyle, view: &LayoutJobViewConfiguration,
        id: DocumentId, revision: Revision, control: &LayoutRunControl<'_>) -> Result<bool, LayoutComputationError> {
        std::mem::swap(&mut self.shape_cache, &mut self.decoration_shape_cache);
        let result = self.shape_ranges_with_origin(id, revision, " ", &[0..1], &[0..1], 0, &[], &[],
            std::slice::from_ref(style), &[TextDirection::LeftToRight], view.scale,
            self.provider.measurement_environment_id(), self.provider.metrics_generation(), control);
        std::mem::swap(&mut self.shape_cache, &mut self.decoration_shape_cache);
        Ok(result?.first().is_some_and(|f| f.default_font_is_monospaced))
    }

    fn aligned_indent_width(&mut self, line: usize, view: &LayoutJobViewConfiguration,
        style: &ResolvedTextStyle, id: DocumentId, revision: Revision, control: &LayoutRunControl<'_>) -> Result<f64, LayoutComputationError> {
        let mut pending = Vec::new();
        let mut current = line;
        let unit = f64::from(self.whitespace_unit(&view.whitespace, style, view.scale, id, revision, control)?);
        let mut width;
        loop {
            control.checkpoint()?;
            if let Some(cached) = self.indent_alignment.widths.get(&current) { width = *cached; break; }
            let snapshot = self.indent_alignment.snapshot.as_ref().unwrap();
            let columns = snapshot.index.line(current).unwrap().columns;
            // Equal indents inherit precisely the same x, independent of their
            // bodies. Jump over the entire chain in one threshold query.
            let anchor = columns.checked_sub(1).and_then(|max| snapshot.index.preceding_with_max(current, max));
            match anchor {
                Some(anchor) => { pending.push((current, anchor, columns)); current = anchor; }
                None => { width = columns as f64 * unit; break; }
            }
        }
        while let Some((child, anchor, target)) = pending.pop() {
            control.checkpoint()?;
            let delta = self.anchor_body_advance(anchor, target, width, unit, view, style, id, revision, control)?;
            width += delta;
            // Bounded disposable geometry; index correctness never depends on it.
            if self.indent_alignment.widths.len() < 2048 { self.indent_alignment.widths.insert(child, width); }
        }
        if self.indent_alignment.widths.len() < 2048 { self.indent_alignment.widths.insert(line, width); }
        Ok(width)
    }

    #[allow(clippy::too_many_arguments)]
    fn anchor_body_advance(&mut self, anchor: usize, target: usize, anchor_width: f64, unit: f64,
        view: &LayoutJobViewConfiguration, whitespace_style: &ResolvedTextStyle,
        id: DocumentId, revision: Revision, control: &LayoutRunControl<'_>) -> Result<f64, LayoutComputationError> {
        let projection = self.indent_alignment.snapshot.as_ref().unwrap().projection.clone();
        let range = projection.presentation_line_range(anchor).unwrap();
        let (start, mut column) = self.indent_alignment.snapshot.as_mut().unwrap().indentation(anchor, control)?;
        let mut at = start;
        let mut advance = 0.0f64;
        while at < range.end && column < target {
            control.checkpoint()?;
            let mut end = anchor_chunk_end(projection.text_tree(), at, range.end)?;
            let text = projection.text_tree().slice(at..end).map_err(|_| LayoutError::InvalidTextOffset(at))?;
            let (_, target_caret) = anchor_caret(&text, at, column, target, view.whitespace.tabstop);
            if let Some(caret) = target_caret {
                // Own one following grapheme so a target at a direction/style
                // boundary resolves through its downstream native caret.
                end = if caret.end() < range.end {
                    projection.text_tree().next_grapheme_boundary(caret.end())
                        .map_err(|_| LayoutError::InvalidTextOffset(caret.end()))?.unwrap_or(range.end)
                } else { range.end };
            }
            let context = anchor_context(projection.text_tree(), at..end, &range)?;
            let styles = projection.resolve_styles(context, view.whitespace.format).map_err(LayoutError::DocumentStyle)?;
            let default = if view.default_style_is_override { &view.default_style } else { &styles.default_shaping_style };
            let runs = if view.style_runs_are_override { view.alignment_style_runs.as_deref().unwrap_or(&view.style_runs).as_slice() } else { &styles.shaping_runs };
            let mut fragment = self.shape_unwrapped_fragment(id, revision, projection.text_tree(), at..end, &range,
                default, runs, &[], f32::MAX, TextDirection::LeftToRight, view, control)?;
            // A ligature can own following context. Continue after its true end
            // and count those graphemes once, never restart inside the cluster.
            let actual_end = fragment.clusters.last().map_or(end, |cluster| cluster.text_range.end);
            let text = projection.text_tree().slice(at..actual_end).map_err(|_| LayoutError::InvalidTextOffset(at))?;
            let (next_column, target_caret) = anchor_caret(&text, at, column, target, view.whitespace.tabstop);
            if fragment.clusters.iter().any(|cluster| cluster.bidi_level % 2 != 0) {
                return self.anchor_bidi_advance(anchor, target, anchor_width, unit, view, id, revision, control);
            }
            self.layout_whitespace_clusters(&mut fragment.clusters, &text, at, &view.whitespace,
                whitespace_style.size * view.scale * 0.5, default, runs, view.scale, id, revision, control, false, anchor_width + advance)?;
            if let Some(caret) = target_caret {
                let x = match caret {
                    AnchorCaret::Exact(offset) => fragment_caret_x(&fragment.clusters, offset)?,
                    AnchorCaret::WithinCell { start, end, fraction } => {
                        let begin = fragment_caret_x(&fragment.clusters, start)?;
                        let end = fragment_caret_x_with_affinity(&fragment.clusters, end, BoundaryAffinity::Upstream)?;
                        begin + (end - begin) * fraction
                    }
                };
                return Ok(advance + x);
            }
            advance += fragment.clusters.iter().map(|cluster| f64::from(cluster.advance)).sum::<f64>();
            column = next_column;
            at = actual_end;
        }
        Ok(advance + target.saturating_sub(column) as f64 * unit)
    }

    /// Bidi can make the target's x depend on the rest of its directional run.
    /// Reuse the ordinary streamed unwrapped layout and its exact native caret
    /// placement; its finite fragment caches retain only requested geometry.
    #[allow(clippy::too_many_arguments)]
    fn anchor_bidi_advance(&mut self, anchor: usize, target: usize, anchor_width: f64, unit: f64,
        view: &LayoutJobViewConfiguration, id: DocumentId, revision: Revision,
        control: &LayoutRunControl<'_>) -> Result<f64, LayoutComputationError> {
        let projection = self.indent_alignment.snapshot.as_ref().unwrap().projection.clone();
        let range = projection.presentation_line_range(anchor).unwrap();
        let (mut at, mut column) = self.indent_alignment.snapshot.as_mut().unwrap().indentation(anchor, control)?;
        let mut target_caret = None;
        while at < range.end && column < target {
            control.checkpoint()?;
            let end = anchor_chunk_end(projection.text_tree(), at, range.end)?;
            let text = projection.text_tree().slice(at..end).map_err(|_| LayoutError::InvalidTextOffset(at))?;
            (column, target_caret) = anchor_caret(&text, at, column, target, view.whitespace.tabstop);
            at = end;
        }
        let target_caret = target_caret.unwrap_or(AnchorCaret::Exact(range.end));
        let following = projection.presentation_line_range(anchor + 1);
        let style_range = range.start..following.as_ref().map_or(range.end, |next| next.end);
        let styles = projection.resolve_styles(style_range, view.whitespace.format).map_err(LayoutError::DocumentStyle)?;
        let mut anchor_view = view.clone();
        anchor_view.wrap = false;
        anchor_view.horizontal_focus_offset = Some(target_caret.end());
        anchor_view.horizontal_anchor_offset = None;
        anchor_view.horizontal_desired_x = None;
        anchor_view.complete_horizontal_geometry = false;
        if view.style_runs_are_override {
            let runs = view.alignment_style_runs.as_deref().unwrap_or(&view.style_runs);
            let first = runs.partition_point(|run| run.text_range.end <= range.start);
            let last = runs.partition_point(|run| run.text_range.start < range.end);
            anchor_view.style_runs = runs[first..last].to_vec();
        }
        // This width has already been resolved while unwinding the anchor
        // chain. Pin it only for this nested geometry request, avoiding any
        // recursive ancestor traversal even when the disposable cache is full.
        let previous = self.indent_alignment.widths.insert(anchor, anchor_width);
        let result = self.layout_unwrapped_viewport_cancellable(id, revision, projection.text_tree(),
            std::slice::from_ref(&range), anchor, projection.presentation_line_count(), following,
            &styles, &anchor_view, control.cancellation);
        if let Some(previous) = previous { self.indent_alignment.widths.insert(anchor, previous); }
        else { self.indent_alignment.widths.remove(&anchor); }
        let region = result?;
        let row = region.lines.first().and_then(|line| line.rows.first())
            .ok_or(LayoutError::NoCaretStops)?;
        let x = match target_caret {
            AnchorCaret::Exact(offset) => row_caret_x(row, offset)?,
            AnchorCaret::WithinCell { start, end, fraction } => {
                let begin = row_caret_x(row, start)?;
                let end = row_caret_x_with_affinity(row, end, BoundaryAffinity::Upstream)?;
                begin + (end - begin) * fraction
            }
        };
        Ok(x - f64::from(row.paragraph_content_x) - anchor_width
            + target.saturating_sub(column) as f64 * unit)
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};
    use crate::layout::MockTextMeasurementProvider;

    fn fixture(text: &str) -> (Document, ViewLayout) {
        let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut view = ViewLayout::new(600.0, 160.0);
        view.set_wrap(false);
        let mut options = WhitespacePresentationOptions::default();
        options.align_indents_to_previous_line = true;
        view.set_whitespace_presentation(options, Format::Code, 4).unwrap();
        (document, view)
    }

    #[test]
    fn mixed_bidi_alignment_matches_native_downstream_carets() {
        let anchor = "  aאבz";
        for (columns, offset) in [(3, 3), (4, 5), (5, 7), (6, 8)] {
            let text = format!("{anchor}\n{}c", " ".repeat(columns));
            let (document, mut view) = fixture(&text);
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            engine.relayout(&document, &mut view).unwrap();
            let snapshot = view.snapshot().unwrap();
            let expected = row_caret_x(&snapshot.rows[0], offset).unwrap();
            let actual = row_caret_x(&snapshot.rows[1], anchor.len() + 1 + columns).unwrap();
            assert!((actual - expected).abs() < 0.001, "column {columns}: {actual} != {expected}");
        }
    }

    #[test]
    fn partial_rtl_tab_and_wide_cell_use_the_cells_own_edge_affinities() {
        for (anchor, end, columns) in [("  aא\tz", 6, 6), ("  aא😀z", 9, 5)] {
            let text = format!("{anchor}\n{}c", " ".repeat(columns));
            let (document, mut view) = fixture(&text);
            view.set_style_runs(vec![ShapeStyleRun { text_range: 5..end,
                style: ResolvedTextStyle { direction: TextDirection::RightToLeft, ..Default::default() } }]).unwrap();
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            engine.relayout(&document, &mut view).unwrap();
            let snapshot = view.snapshot().unwrap();
            let cell = snapshot.rows[0].clusters.iter().find(|cluster| cluster.text_range.start == 5).unwrap();
            let expected = f64::from(cell.x + cell.advance * 0.5);
            let actual = row_caret_x(&snapshot.rows[1], anchor.len() + 1 + columns).unwrap();
            assert!((actual - expected).abs() < 0.001, "{anchor}: {actual} != {expected}");
        }
    }

    #[test]
    fn ligature_alignment_uses_the_same_indivisible_cluster_geometry_as_navigation() {
        let anchor = "  office";
        for columns in [3, 4, 5, 6] {
            let text = format!("{anchor}\n{}c", " ".repeat(columns));
            let (document, mut view) = fixture(&text);
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            engine.relayout(&document, &mut view).unwrap();
            let snapshot = view.snapshot().unwrap();
            let expected = snapshot.logical_endpoint_geometry(columns, BoundaryAffinity::Downstream).unwrap().rect.x;
            let actual = row_caret_x(&snapshot.rows[1], anchor.len() + 1 + columns).unwrap();
            assert!((actual - f64::from(expected)).abs() < 0.001, "column {columns}");
        }
    }

    #[test]
    fn blank_and_whitespace_only_lines_are_skipped_for_the_previous_nonblank_anchor() {
        for blank in ["", "  ", "        ", "\t \t"] {
            let text = format!("  if (a\n{blank}\n\n      c");
            let (document, mut view) = fixture(&text);
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            engine.relayout(&document, &mut view).unwrap();
            let snapshot = view.snapshot().unwrap();
            let expected = row_caret_x(&snapshot.rows[0], 6).unwrap();
            let actual = row_caret_x(&snapshot.rows[3], text.len() - 1).unwrap();
            assert!((actual - expected).abs() < 0.001, "{blank:?}: {actual} != {expected}");
        }
    }

    #[test]
    fn adjusted_anchor_indentation_retargets_body_tabs_before_descendant_alignment() {
        let text = "mm\n  x\tY\n    z";
        let (document, mut view) = fixture(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        let y = text.find('Y').unwrap();
        let z = text.find('z').unwrap();
        let actual = row_caret_x(&snapshot.rows[2], z).unwrap();
        let expected = row_caret_x(&snapshot.rows[1], y).unwrap();
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        let relative = expected - f64::from(snapshot.rows[1].paragraph_content_x);
        assert!((relative % f64::from(snapshot.whitespace_unit * 4.0)).abs() < 0.001);
    }

    #[derive(Default)]
    struct CountingProvider { inner: MockTextMeasurementProvider, bytes: usize, maximum: usize }
    impl TextMeasurementProvider for CountingProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId { self.inner.measurement_environment_id() }
        fn metrics_generation(&self) -> MetricsGeneration { self.inner.metrics_generation() }
        fn render_run_policy(&self) -> Option<RenderRunPolicy> { self.inner.render_run_policy() }
        fn shape_batch(&mut self, requests: &[ShapeRequest<'_>]) -> Result<Vec<ShapedFragment>, MeasurementError> {
            for request in requests { self.bytes += request.text.len(); self.maximum = self.maximum.max(request.text.len()); }
            self.inner.shape_batch(requests)
        }
    }

    #[test]
    fn short_anchor_target_never_shapes_its_unrelated_megabyte_suffix() {
        let mut measurements = Vec::new();
        for suffix in [0, 1_000_000] {
            let (document, view) = fixture(&format!("  if (a{}\n      c", "x".repeat(suffix)));
            let mut engine = LayoutEngine::new(CountingProvider::default());
            engine.prepare_indent_alignment(Some(Arc::new(document.projection().clone())), None, 4);
            let captured = view.capture_for_layout_job();
            let start = document.projection().presentation_line_range(1).unwrap().start;
            let width = engine.aligned_indent_width_for_line(start, &captured, &ResolvedTextStyle::default(),
                document.id(), document.revision(), &LayoutRunControl::synchronous(&NeverCancelled)).unwrap().unwrap();
            assert!(engine.provider().maximum <= 5, "only the target prefix plus a following grapheme is owned");
            measurements.push((width, engine.provider().bytes));
        }
        assert_eq!(measurements[0], measurements[1]);
    }

    #[test]
    fn offscreen_anchor_preserves_full_override_style_context() {
        let text = "  mmX\n    z";
        let (document, mut view) = fixture(text);
        view.set_style_runs(vec![ShapeStyleRun { text_range: 2..4,
            style: ResolvedTextStyle { size: 28.0, ..Default::default() } }]).unwrap();
        let mut expected_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        expected_engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        let expected = row_caret_x(&snapshot.rows[0], 4).unwrap() - f64::from(snapshot.rows[0].paragraph_content_x);
        let range = document.projection().presentation_line_range(1).unwrap();
        let captured = view.capture_for_regional_layout_job(range.clone());
        assert!(captured.style_runs.is_empty());
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        engine.prepare_indent_alignment(Some(Arc::new(document.projection().clone())), None, 4);
        let actual = engine.aligned_indent_width_for_line(range.start, &captured, &ResolvedTextStyle::default(),
            document.id(), document.revision(), &LayoutRunControl::synchronous(&NeverCancelled)).unwrap().unwrap();
        assert!((actual - expected).abs() < 0.001);
    }

    #[test]
    fn giant_wrapped_indent_uses_the_whitespace_fonts_alignment_mode() {
        // Deliberately different authoritative font traits: 28-point requests
        // resolve to a fixed-pitch face; 14-point requests are proportional.
        struct FontTraits(MockTextMeasurementProvider);
        impl TextMeasurementProvider for FontTraits {
            fn measurement_environment_id(&self) -> MeasurementEnvironmentId { self.0.measurement_environment_id() }
            fn metrics_generation(&self) -> MetricsGeneration { self.0.metrics_generation() }
            fn render_run_policy(&self) -> Option<RenderRunPolicy> { self.0.render_run_policy() }
            fn shape_batch(&mut self, requests: &[ShapeRequest<'_>]) -> Result<Vec<ShapedFragment>, MeasurementError> {
                let mut fragments = self.0.shape_batch(requests)?;
                for (request, fragment) in requests.iter().zip(&mut fragments) {
                    fragment.default_font_is_monospaced = request.default_style.size == 28.0;
                }
                Ok(fragments)
            }
        }
        let (document, mut view) = fixture(&format!("mm\n{}x", " ".repeat(MAX_SHAPE_FRAGMENT_BYTES * 2)));
        view.set_wrap(true);
        let range = document.projection().presentation_line_range(1).unwrap();
        let control = LayoutRunControl::synchronous(&NeverCancelled);
        for (whitespace_size, body_size) in [(14.0, 28.0), (28.0, 14.0)] {
            let whitespace_style = ResolvedTextStyle { size: whitespace_size, ..Default::default() };
            let body_style = ResolvedTextStyle { size: body_size, ..Default::default() };
            view.set_default_style(body_style.clone()).unwrap();
            let captured = view.capture_for_layout_job();
            let mut reference = LayoutEngine::new(FontTraits(MockTextMeasurementProvider::new()));
            reference.prepare_indent_alignment(Some(Arc::new(document.projection().clone())), None, 4);
            let aligned = reference.aligned_indent_width_for_line(range.start, &captured, &whitespace_style,
                document.id(), document.revision(), &control).unwrap();
            assert_eq!(aligned.is_some(), whitespace_size == 14.0);
            let expected = aligned.unwrap_or((range.end - range.start - 1) as f64 * f64::from(whitespace_size * 0.5));
            let mut engine = LayoutEngine::new(FontTraits(MockTextMeasurementProvider::new()));
            engine.prepare_indent_alignment(Some(Arc::new(document.projection().clone())), None, 4);
            let (actual, _) = engine.code_leading_indent_from_tree(document.projection().text_tree(), &range,
                &captured, whitespace_size * 0.5, &whitespace_style, &body_style, &[],
                document.id(), document.revision(), &control).unwrap();
            assert!((actual - expected).abs() < 0.001, "whitespace {whitespace_size}, body {body_size}: {actual} != {expected}");
        }
    }

    #[test]
    fn giant_indentation_scan_observes_cancellation_between_bounded_chunks() {
        struct CancelAfter(std::cell::Cell<usize>);
        impl LayoutCancellationProbe for CancelAfter {
            fn is_cancelled(&self) -> bool {
                let count = self.0.get(); self.0.set(count + 1); count >= 2
            }
        }
        let (document, _) = fixture(&format!("{}x", " ".repeat(100_000)));
        let projection = IndentProjection::new(Arc::new(document.projection().clone()));
        let cancel = CancelAfter(std::cell::Cell::new(0));
        assert!(matches!(indentation_cancellable(&projection, 0, 4, &LayoutRunControl::cancellable(&cancel)),
            Err(LayoutComputationError::Cancelled)));
        assert_eq!(cancel.0.get(), 3);
    }
}
