//! Diff mode's coordinator side. A [`DiffSession`] compares a read-only base
//! core with the live core of the document being edited, keeps the line diff
//! current as the live document changes, and installs each side's
//! presentation (changed lines, changed spans, aligned filler and collapsed
//! context) in its view.
//! See `docs/diff_mode.md`.

use super::*;
use crate::diff::{CollapsibleRun, DiffLimits, DiffState, Hunk, LineEdit, LineSource, Side, SidePresentation, DIFF_CONTEXT_LINES};
use crate::layout::{DiffLayoutInput, DiffMargin};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

/// A document's hard lines as diff input. Lines exclude their delimiter.
pub struct DocumentLines<'a>(pub &'a Document);

impl LineSource for DocumentLines<'_> {
    fn line_count(&self) -> usize {
        self.0.line_count()
    }

    fn line(&self, index: usize) -> Cow<'_, str> {
        let projection = self.0.projection();
        projection
            .hard_line_range(index)
            .and_then(|range| projection.text_tree().slice(range).ok())
            .map_or(Cow::Borrowed(""), Cow::Owned)
    }
}

/// How a sync updated the diff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffSync {
    /// Both sides were already current.
    Unchanged,
    /// The live document changed by a bounded edit and the diff was updated
    /// around it.
    Incremental,
    /// The diff was recomputed from scratch.
    Full,
}

/// Which way [`DiffSession::navigate_change`] moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeDirection {
    Next,
    Previous,
}

/// Where [`DiffSession::navigate_change`] went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeNavigation {
    /// Zero-based index of the change among `change_count`.
    pub change: usize,
    pub change_count: usize,
    /// The new side's hard line that received the caret.
    pub caret_line: usize,
    /// Whether the views were repositioned; false when the change was
    /// already visible on both sides.
    pub scrolled: bool,
}

/// A stretch of one side's vertical scrollbar track that marks a change, in
/// layout units from the top of the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarMark {
    pub top: f32,
    pub bottom: f32,
    /// Added (new side) rather than removed (base side) lines.
    pub added: bool,
}

#[derive(Clone, Debug, Default)]
struct InstalledSide {
    view: Option<ViewId>,
    document: Option<(DocumentId, Revision)>,
    presentation: SidePresentation,
    aligned: bool,
    /// One default row, unscaled: a filler line and a divider's height.
    row_points: f32,
}

/// One diff mode comparison. Hosts keep it beside the two cores and call
/// [`DiffSession::sync`] after every event that may change either side.
pub struct DiffSession {
    state: DiffState,
    line_totals: (usize, usize),
    limits: DiffLimits,
    base_document: DocumentId,
    base_revision: Revision,
    new_document: DocumentId,
    new_revision: Revision,
    aligned: bool,
    /// Whether unchanged runs collapse behind dividers.
    collapsed: bool,
    /// Base-side ranges of the collapsible runs the user opened. Keyed by
    /// the read-only base, they survive edits that leave the lines equal.
    opened: Vec<Range<usize>>,
    /// The runs collapsed at the last install.
    runs: Vec<CollapsibleRun>,
    /// Until the first install, a caret inside a collapsed run moves beside
    /// it; afterwards a caret there keeps its run open.
    relocate_carets: bool,
    installed: [InstalledSide; 2],
}

impl DiffSession {
    /// Compute the diff between a base document and the live document.
    pub fn new(base: &Document, new: &Document, aligned: bool) -> Self {
        Self::with_limits(base, new, aligned, DiffLimits::default())
    }

    pub fn with_limits(base: &Document, new: &Document, aligned: bool, limits: DiffLimits) -> Self {
        let state = DiffState::compute(&DocumentLines(base), &DocumentLines(new), limits);
        Self {
            line_totals: Self::count_lines(&state, base, new),
            state,
            limits,
            base_document: base.id(),
            base_revision: base.revision(),
            new_document: new.id(),
            new_revision: new.revision(),
            aligned,
            collapsed: false,
            opened: Vec::new(),
            runs: Vec::new(),
            relocate_carets: true,
            installed: Default::default(),
        }
    }

    pub fn state(&self) -> &DiffState {
        &self.state
    }

    fn count_lines(state: &DiffState, base: &Document, new: &Document) -> (usize, usize) {
        let ends_empty = |doc: &Document| {
            doc.projection().hard_line_range(doc.line_count().saturating_sub(1)).is_none_or(|line| line.is_empty())
        };
        state.line_totals(ends_empty(base), ends_empty(new))
    }

    pub fn line_totals(&self) -> (usize, usize) {
        self.line_totals
    }

    pub fn aligned(&self) -> bool {
        self.aligned
    }

    /// Change whether the sides are aligned; the next install applies it.
    pub fn set_aligned(&mut self, aligned: bool) {
        self.aligned = aligned;
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    /// Change whether unchanged runs collapse; the next install applies it.
    /// Either way every opened run closes again.
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
        self.opened.clear();
    }

    /// The collapsed runs as of the last install.
    pub fn collapsed_runs(&self) -> &[CollapsibleRun] {
        &self.runs
    }

    /// Open the collapsed run whose divider borders `line` on `side`: the
    /// run just after it, or a run at the start of the file just before it.
    /// The next sync shows it on both sides. Returns whether a run opened.
    pub fn open_collapsed_beside(&mut self, side: Side, line: usize) -> bool {
        let Some(run) = self.runs.iter().find(|run| {
            let own = run.side(side);
            own.start == line + 1 || (own.start == 0 && own.end == line) || own.contains(&line)
        }) else {
            return false;
        };
        self.opened.push(run.base.clone());
        true
    }

    /// The runs to collapse now: the state's collapsible runs except those
    /// that overlap an opened run.
    fn current_runs(&self) -> Vec<CollapsibleRun> {
        if !self.collapsed {
            return Vec::new();
        }
        self.state.collapsible_runs(DIFF_CONTEXT_LINES).into_iter()
            .filter(|run| !self.opened.iter().any(|open| open.start < run.base.end && run.base.start < open.end))
            .collect()
    }

    /// Record the runs a side's caret opened since the last sync.
    fn take_opened<P: TextMeasurementProvider>(&mut self, core: &mut Core<P>, view: ViewId, side: Side) {
        for lines in core.take_opened_collapsed(view) {
            let base = match side {
                Side::Base => lines,
                Side::New => {
                    let start = self.state.corresponding_line(Side::New, lines.start);
                    start..start + lines.len()
                }
            };
            if !base.is_empty() {
                self.opened.push(base);
            }
        }
    }

    /// A caret inside a run about to collapse: before the first install it
    /// moves beside the run; afterwards (toggling, or an edit that makes
    /// its lines equal) the run stays open. Returns whether a run opened.
    fn keep_caret_visible<P: TextMeasurementProvider>(&mut self, core: &mut Core<P>, view: ViewId, side: Side,
        runs: &[CollapsibleRun]) -> Result<bool, CoreError> {
        let Some(line) = core.diff_caret_line(view)? else { return Ok(false) };
        let installed = &self.installed(side).presentation.collapsed;
        let Some(run) = runs.iter().find(|run| run.side(side).contains(&line)) else { return Ok(false) };
        let own = run.side(side).clone();
        if installed.contains(&own) {
            return Ok(false);
        }
        let count = core.document().line_count();
        let beside = if own.start == 0 || (own.end < count && own.end - line <= line + 1 - own.start) {
            own.end
        } else {
            own.start - 1
        };
        if self.relocate_carets && beside < count && core.place_diff_caret(view, beside)? {
            return Ok(false);
        }
        self.opened.push(run.base.clone());
        Ok(true)
    }

    /// The line on the other side corresponding to `line`.
    pub fn corresponding_line(&self, from: Side, line: usize) -> usize {
        self.state.corresponding_line(from, line)
    }

    fn installed(&self, side: Side) -> &InstalledSide {
        &self.installed[usize::from(side == Side::New)]
    }

    fn installed_mut(&mut self, side: Side) -> &mut InstalledSide {
        &mut self.installed[usize::from(side == Side::New)]
    }

    /// Bring the diff up to date with both documents. A bounded change to
    /// the live document re-diffs only around it; anything else, including
    /// undo, reload or a changed base, recomputes from scratch.
    pub fn update(&mut self, base: &Document, new: &Document) -> DiffSync {
        if base.id() != self.base_document || base.revision() != self.base_revision || new.id() != self.new_document {
            self.recompute(base, new);
            return DiffSync::Full;
        }
        if new.revision() == self.new_revision {
            return DiffSync::Unchanged;
        }
        let edit = new
            .layout_change_chain(self.new_revision)
            .filter(|chain| chain.len() == 1)
            .and_then(|chain| line_edit(new, &chain[0]));
        match edit {
            Some(edit) if edit.fits(self.state.new_line_count(), new.line_count()) => {
                self.state.apply_edit(&DocumentLines(base), &DocumentLines(new), edit);
                self.line_totals = Self::count_lines(&self.state, base, new);
                self.new_revision = new.revision();
                self.remap_installed_new_side(edit);
                DiffSync::Incremental
            }
            _ => {
                self.recompute(base, new);
                DiffSync::Full
            }
        }
    }

    fn recompute(&mut self, base: &Document, new: &Document) {
        self.state = DiffState::compute(&DocumentLines(base), &DocumentLines(new), self.limits);
        self.line_totals = Self::count_lines(&self.state, base, new);
        self.base_document = base.id();
        self.base_revision = base.revision();
        self.new_document = new.id();
        self.new_revision = new.revision();
        // The previous presentation no longer describes these documents.
        for side in &mut self.installed {
            side.document = None;
        }
    }

    /// Line numbers of the previously installed new-side presentation, moved
    /// through an edit, so only lines whose appearance changes relayout.
    fn remap_installed_new_side(&mut self, edit: LineEdit) {
        let side = self.installed_mut(Side::New);
        if side.document.is_none() {
            return;
        }
        let presentation = &mut side.presentation;
        presentation.lines = presentation.lines.drain(..).flat_map(|lines| edit.map_surviving(lines)).collect();
        presentation.text = presentation.text.drain(..).filter_map(|(line, span)| Some((edit.map_line(line)?, span))).collect();
        presentation.fillers = presentation.fillers.drain(..)
            .filter_map(|filler| Some(crate::diff::Filler { before_line: edit.map_boundary(filler.before_line)?, ..filler }))
            .collect();
        presentation.collapsed = presentation.collapsed.drain(..).filter_map(|run| edit.map_range(run)).collect();
    }

    /// Publish each pane's own bound to its peer. Reading only natural bounds
    /// lets a shared range shrink again after an edit, collapse or rewrap.
    fn share_scroll_bounds<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &self, base: &mut Core<P>, base_view: ViewId, new: &mut Core<Q>, new_view: ViewId,
    ) -> Result<(), CoreError> {
        let left = &mut base.views.get_mut(&base_view).ok_or(CoreError::UnknownView(base_view))?.layout;
        let right = &mut new.views.get_mut(&new_view).ok_or(CoreError::UnknownView(new_view))?.layout;
        let left_bound = left.own_vertical_bound();
        left.set_diff_peer_bound(self.aligned.then(|| right.own_vertical_bound()));
        right.set_diff_peer_bound(self.aligned.then_some(left_bound));
        Ok(())
    }

    /// Synchronize vertical layout coordinates, including blank space. A
    /// filler spanning hundreds of rows has no meaningful text-line fraction.
    /// Returns which views need to republish geometry or scrollbar state.
    pub fn sync_scroll<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &self, base: &mut Core<P>, base_view: ViewId, new: &mut Core<Q>, new_view: ViewId, leader: Side,
    ) -> Result<(bool, bool), CoreError> {
        let current = |side: Side, view: ViewId, document: &Document| {
            let installed = self.installed(side);
            installed.view == Some(view) && installed.document == Some((document.id(), document.revision()))
        };
        if !self.aligned || !current(Side::Base, base_view, base.document()) || !current(Side::New, new_view, new.document()) {
            return Ok((false, false));
        }
        let before_base = base.viewport_state(base_view)?;
        let before_new = new.viewport_state(new_view)?;
        let new_request = new.views[&new_view].layout.diff_scroll_request;
        let base_request = base.views[&base_view].layout.diff_scroll_request;
        let (leader, request) = match leader {
            Side::New => (before_new, new_request.or(base_request)),
            Side::Base => (before_base, base_request.or(new_request)),
        };
        let requested_top = request.unwrap_or(leader.top());
        let end = requested_top == f32::MAX;
        if end {
            // The shorter pane can reach its EOF before its peer knows the
            // height of its last row. A thumb drag to the end refines both.
            if new_request == Some(f32::MAX) && !base.views[&base_view].layout.own_vertical_bound().exact {
                base.materialize_requested_viewport(base_view, before_base.left(), f32::MAX)?;
            } else if base_request == Some(f32::MAX) && !new.views[&new_view].layout.own_vertical_bound().exact {
                new.materialize_requested_viewport(new_view, before_new.left(), f32::MAX)?;
            }
        }
        for _ in 0..3 {
            self.share_scroll_bounds(base, base_view, new, new_view)?;
            let range = &base.views[&base_view].layout;
            let top = if end { range.estimated_maximum_viewport_top() }
                else { range.maximum_viewport_top().map_or(requested_top, |max| requested_top.min(max)) };
            let move_base = base.views[&base_view].layout.viewport_top() != top;
            let move_new = new.views[&new_view].layout.viewport_top() != top;
            if !move_base && !move_new { break; }
            if move_base { base.materialize_requested_viewport(base_view, before_base.left(), top)?; }
            if move_new { new.materialize_requested_viewport(new_view, before_new.left(), top)?; }
        }
        self.share_scroll_bounds(base, base_view, new, new_view)?;
        base.views.get_mut(&base_view).unwrap().layout.diff_scroll_request = None;
        new.views.get_mut(&new_view).unwrap().layout.diff_scroll_request = None;
        Ok((before_base != base.viewport_state(base_view)?, before_new != new.viewport_state(new_view)?))
    }

    /// Update the diff and install both sides' presentations in their views.
    pub fn sync<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &mut self,
        base: &mut Core<P>,
        base_view: ViewId,
        new: &mut Core<Q>,
        new_view: ViewId,
    ) -> Result<DiffSync, CoreError> {
        let sync = self.update(base.document(), new.document());
        self.take_opened(base, base_view, Side::Base);
        self.take_opened(new, new_view, Side::New);
        let mut runs = self.current_runs();
        if !runs.is_empty() {
            let opened = self.keep_caret_visible(base, base_view, Side::Base, &runs)?
                | self.keep_caret_visible(new, new_view, Side::New, &runs)?;
            if opened {
                runs = self.current_runs();
            }
        }
        self.relocate_carets = false;
        self.install(base, base_view, Side::Base, &runs)?;
        self.install(new, new_view, Side::New, &runs)?;
        self.share_scroll_bounds(base, base_view, new, new_view)?;
        self.runs = runs;
        Ok(sync)
    }

    fn install<P: TextMeasurementProvider>(&mut self, core: &mut Core<P>, view: ViewId, side: Side,
        runs: &[CollapsibleRun]) -> Result<(), CoreError> {
        let row_points = core.diff_filler_line_height(view)?;
        let mut presentation = self.state.presentation(side, self.aligned);
        presentation.collapsed = runs.iter().map(|run| run.side(side).clone()).collect();
        let document = (core.document().id(), core.document().revision());
        let installed = self.installed(side);
        let comparable = installed.view == Some(view)
            && installed.aligned == self.aligned
            && installed.row_points == row_points
            && installed.document.is_some_and(|(id, _)| id == document.0);
        if comparable && installed.document == Some(document) && installed.presentation == presentation {
            return Ok(());
        }
        let changed = comparable.then(|| changed_lines(&installed.presentation, &presentation, core.document().line_count()));
        let filler_points = if self.aligned { row_points } else { 0.0 };
        let input = layout_input(core.document(), side, &presentation, filler_points, row_points);
        core.install_diff_presentation(view, Some(Arc::new(input)), changed.as_deref(), &presentation.collapsed)?;
        *self.installed_mut(side) = InstalledSide { view: Some(view), document: Some(document), presentation, aligned: self.aligned, row_points };
        Ok(())
    }

    /// Move to the next or previous change, wrapping within the file. The
    /// new side is the reference: its caret moves to the change's first
    /// added line, or to the line after a pure removal, and unless the
    /// change is already visible on both sides, the new view is positioned
    /// first and the base view is then placed as if the sides were aligned.
    /// `None` when the sides are identical.
    pub fn navigate_change<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &mut self,
        base: &mut Core<P>,
        base_view: ViewId,
        new: &mut Core<Q>,
        new_view: ViewId,
        direction: ChangeDirection,
    ) -> Result<Option<ChangeNavigation>, CoreError> {
        self.sync(base, base_view, new, new_view)?;
        let hunks: Vec<Hunk> = self.state.hunks().collect();
        if hunks.is_empty() {
            return Ok(None);
        }
        let count = new.document().line_count();
        let target = |hunk: &Hunk| hunk.new.start.min(count.saturating_sub(1));
        let cursor = new.views.get(&new_view).ok_or(CoreError::UnknownView(new_view))?.commands.cursor();
        let cursor_line = new.document().hard_line_at_offset(cursor).unwrap_or(0);
        let index = match direction {
            ChangeDirection::Next => hunks.iter().position(|hunk| target(hunk) > cursor_line).unwrap_or(0),
            ChangeDirection::Previous => {
                hunks.iter().rposition(|hunk| target(hunk) < cursor_line).unwrap_or(hunks.len() - 1)
            }
        };
        let hunk = &hunks[index];
        let caret_line = target(hunk);
        let offset = new.document().projection().hard_line_range(caret_line).map_or(0, |range| range.start);
        let revision = new.document().revision();
        new.handle(
            new_view,
            CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )?;
        let scrolled = self.position_change(base, base_view, new, new_view, hunk, caret_line)?;
        self.sync_scroll(base, base_view, new, new_view, Side::New)?;
        Ok(Some(ChangeNavigation { change: index, change_count: hunks.len(), caret_line, scrolled }))
    }

    /// Scroll both views to show `hunk` (see `docs/diff_mode.md`, Changes).
    fn position_change<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &self,
        base: &mut Core<P>,
        base_view: ViewId,
        new: &mut Core<Q>,
        new_view: ViewId,
        hunk: &Hunk,
        caret_line: usize,
    ) -> Result<bool, CoreError> {
        let new_row = f64::from(new.default_row_height(new_view)?);
        let base_row = f64::from(base.default_row_height(base_view)?);
        let (new_side, base_side) = (&self.installed(Side::New).presentation, &self.installed(Side::Base).presentation);
        let new_block = side_block(new, new_view, new_side, hunk.new.clone(), new_row)?;
        let base_block = side_block(base, base_view, base_side, hunk.base.clone(), base_row)?;
        let shows = |core_view: (f64, f64), block: &Range<f64>| {
            let (top, height) = core_view;
            block.start >= top - 0.5 && block.end <= top + height + 0.5
        };
        if shows(viewport(new, new_view)?, &new_block) && shows(viewport(base, base_view)?, &base_block) {
            return Ok(false);
        }
        // A newly materialized region can refine the heights it was placed
        // with; re-evaluate locally, with a finite retry budget.
        let mut new_top = 0.0;
        for _ in 0..3 {
            let block = side_block(new, new_view, new_side, hunk.new.clone(), new_row)?;
            let other = side_block(base, base_view, base_side, hunk.base.clone(), base_row)?;
            let (_, view_height) = viewport(new, new_view)?;
            // Pretend the sides are aligned: the change is as tall as its
            // taller side.
            let height = (block.end - block.start).max(other.end - other.start);
            let requested = if height <= view_height {
                block.start + height / 2.0 - view_height / 2.0
            } else {
                let below_context = block.start - DIFF_CONTEXT_LINES as f64 * new_row;
                let caret = side_block(new, new_view, new_side, caret_line..caret_line + 1, new_row)?;
                let caret_shows = caret.end <= below_context + view_height;
                if view_height / new_row > 4.0 || caret_shows { below_context } else { block.start }
            };
            let left = new.views[&new_view].layout.viewport_left();
            new.materialize_requested_viewport(new_view, left, requested.max(0.0) as f32)?;
            let settled = f64::from(new.views[&new_view].layout.viewport_top());
            let moved = (settled - new_top).abs() >= 0.01;
            new_top = settled;
            if !moved {
                break;
            }
        }
        // Aligned navigation uses the live pane's origin in both views.
        if self.aligned { return Ok(true); }
        for _ in 0..3 {
            let block = side_block(new, new_view, new_side, hunk.new.clone(), new_row)?;
            let other = side_block(base, base_view, base_side, hunk.base.clone(), base_row)?;
            let requested = other.start - (block.start - new_top);
            let left = base.views[&base_view].layout.viewport_left();
            let before = base.views[&base_view].layout.viewport_top();
            base.materialize_requested_viewport(base_view, left, requested.max(0.0) as f32)?;
            if (base.views[&base_view].layout.viewport_top() - before).abs() < 0.01 {
                break;
            }
        }
        Ok(true)
    }

    /// The marks for one side's scrollbar track and the document extent they
    /// are measured against: that side's changed lines in its own colour,
    /// and the other side's changes where they line up, at its filler or,
    /// when unaligned, as an empty mark between lines.
    pub fn scrollbar_marks<P: TextMeasurementProvider>(
        &self,
        core: &mut Core<P>,
        view: ViewId,
        side: Side,
    ) -> Result<(Vec<ScrollbarMark>, f32), CoreError> {
        let installed = self.installed(side);
        let layout = core.layout(view).ok_or(CoreError::UnknownView(view))?;
        let count = core.document().line_count();
        let extent = layout.hard_line_prefix_height(count).map_err(LayoutError::from)?.height();
        let extent = if layout.has_shared_scroll_range() && layout.estimated_maximum_viewport_top() > 0.0 {
            f64::from(layout.estimated_maximum_viewport_top() + layout.height())
        } else { extent };
        if installed.view != Some(view) || installed.document.is_none() {
            return Ok((Vec::new(), extent as f32));
        }
        let row = f64::from(core.default_row_height(view)?);
        let own = side == Side::New;
        let presentation = &installed.presentation;
        let mut marks = Vec::new();
        for range in &presentation.lines {
            let block = side_block(core, view, presentation, range.start..range.end, 0.0)?;
            marks.push(ScrollbarMark { top: block.start as f32, bottom: block.end as f32, added: own });
        }
        if installed.aligned {
            for filler in &presentation.fillers {
                let line = filler.before_line.min(count);
                let block = side_block(core, view, presentation, line..line, row)?;
                marks.push(ScrollbarMark { top: block.start as f32, bottom: block.end as f32, added: !own });
            }
        } else {
            for hunk in self.state.hunks() {
                let lines = hunk.side(side);
                if lines.is_empty() {
                    let block = side_block(core, view, presentation, lines.start..lines.start, 0.0)?;
                    marks.push(ScrollbarMark { top: block.start as f32, bottom: block.start as f32, added: !own });
                }
            }
        }
        marks.sort_by(|a, b| a.top.total_cmp(&b.top));
        Ok((marks, extent as f32))
    }

    /// Remove diff presentation from a view, for leaving diff mode.
    pub fn clear<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId) -> Result<(), CoreError> {
        core.install_diff_presentation(view, None, None, &[])
    }
}

/// The current viewport top and height of a view, in layout units.
fn viewport<P: TextMeasurementProvider>(core: &Core<P>, view: ViewId) -> Result<(f64, f64), CoreError> {
    let view = core.views.get(&view).ok_or(CoreError::UnknownView(view))?;
    Ok((f64::from(view.layout.viewport_top()), f64::from(view.layout.height())))
}

/// The vertical extent of `lines` on one side, in layout units, including
/// the filler that follows them there. An empty range is the point between
/// lines (with its filler, when aligned); a range at the end of the document
/// covers the trailing filler. `row` sizes one filler line.
fn side_block<P: TextMeasurementProvider>(
    core: &Core<P>,
    view: ViewId,
    presentation: &SidePresentation,
    lines: Range<usize>,
    row: f64,
) -> Result<Range<f64>, CoreError> {
    let layout = core.layout(view).ok_or(CoreError::UnknownView(view))?;
    let count = core.document().line_count();
    let prefix = |line: usize| -> Result<f64, CoreError> {
        Ok(layout.hard_line_prefix_height(line.min(count)).map_err(LayoutError::from)?.height())
    };
    // Fillers are sorted by line: find each line's by binary search, so
    // marking every change stays linear in the changes.
    let fillers_from = |line: usize| {
        let first = presentation.fillers.partition_point(|filler| filler.before_line < line);
        presentation.fillers[first..].iter()
    };
    let height = |filler: &crate::diff::Filler| filler.lines as f64 * row;
    let filler = |line: usize| -> f64 { fillers_from(line).take_while(|filler| filler.before_line == line).map(height).sum() };
    let total = prefix(count)?;
    let trailing: f64 = fillers_from(count).map(height).sum();
    let start = if lines.start < count { prefix(lines.start)? } else { (total - trailing).max(0.0) };
    let end = if lines.end < count { prefix(lines.end)? + filler(lines.end) } else { total };
    Ok(start..end.max(start))
}

/// The hard-line edit described by one bounded formatted change, in the
/// current document's numbering.
fn line_edit(document: &Document, change: &crate::document::LocalFormattedChange) -> Option<LineEdit> {
    let start = document.hard_line_at_offset(change.new.start)?;
    let new_end = document.hard_line_at_offset(change.new.end)? + 1;
    let delta = change.new_line_count as isize - change.old_line_count as isize;
    let old_end = new_end as isize - delta;
    (old_end >= start as isize && change.new_line_count == document.line_count())
        .then(|| LineEdit { start, old_end: old_end as usize, new_end })
}

/// Convert one side's presentation from hard lines to formatted offsets.
fn layout_input(document: &Document, side: Side, presentation: &SidePresentation, filler_points: f32,
    divider_points: f32) -> DiffLayoutInput {
    let projection = document.projection();
    let count = document.line_count();
    let line = |index: usize| projection.hard_line_range(index.min(count.saturating_sub(1)));
    let mut input = DiffLayoutInput {
        added: side == Side::New,
        divider_points,
        ..Default::default()
    };
    for range in &presentation.lines {
        if let (Some(first), Some(last)) = (line(range.start), line(range.end - 1)) {
            input.lines.push(first.start..last.end);
        }
    }
    for (index, span) in &presentation.text {
        if let Some(range) = line(*index) {
            let start = (range.start + span.start).min(range.end);
            let end = (range.start + span.end).min(range.end);
            if start < end {
                input.text.push(start..end);
            }
        }
    }
    for (index, margin) in line_margins(presentation, count) {
        if let Some(range) = projection.hard_line_range(index) {
            input.margins.push(DiffMargin {
                at: range.start,
                filler_before: margin.filler_before as f32 * filler_points,
                filler_after: margin.filler_after as f32 * filler_points,
                divider: margin.divider,
            });
        }
    }
    input
}

/// The margins diff mode adds to one hard line: filler lines above it (and
/// below the last line), and a collapsed run's divider.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct LineMargin {
    filler_before: usize,
    filler_after: usize,
    /// Whether the divider follows the line, and whether its hidden lines
    /// reach the start or end of the file.
    divider: Option<(bool, bool)>,
}

/// One side's margins by hard line, for a document of `count` lines.
fn line_margins(presentation: &SidePresentation, count: usize) -> std::collections::BTreeMap<usize, LineMargin> {
    let mut margins = std::collections::BTreeMap::<usize, LineMargin>::new();
    for filler in &presentation.fillers {
        if filler.before_line >= count {
            margins.entry(count.saturating_sub(1)).or_default().filler_after += filler.lines;
        } else {
            margins.entry(filler.before_line).or_default().filler_before += filler.lines;
        }
    }
    for run in &presentation.collapsed {
        if let Some((line, after)) = divider_line(run, count) {
            margins.entry(line).or_default().divider = Some((after, !after || run.end >= count));
        }
    }
    margins
}

/// The hard line whose margin draws a collapsed run's divider, and whether
/// the divider follows it: the line before the run, or for a run at the
/// start of the file the line after it.
fn divider_line(run: &Range<usize>, count: usize) -> Option<(usize, bool)> {
    if run.start > 0 {
        Some((run.start - 1, true))
    } else {
        (run.end < count).then_some((run.end, false))
    }
}

/// Hard lines whose diff appearance differs between two presentations of
/// the same side of `count` lines, as sorted ranges.
fn changed_lines(old: &SidePresentation, new: &SidePresentation, count: usize) -> Vec<Range<usize>> {
    let mut lines: Vec<Range<usize>> = Vec::new();
    // Changed-line ranges: the symmetric difference of the two range sets.
    let mut edges: Vec<(usize, i32)> = Vec::new();
    for range in &old.lines {
        edges.push((range.start, 1));
        edges.push((range.end, -1));
    }
    for range in &new.lines {
        edges.push((range.start, 2));
        edges.push((range.end, -2));
    }
    edges.sort_unstable();
    let (mut old_depth, mut new_depth) = (0, 0);
    let mut start = None;
    for (position, kind) in edges {
        match kind {
            1 | -1 => old_depth += kind,
            _ => new_depth += kind / 2,
        }
        let differs = (old_depth > 0) != (new_depth > 0);
        match (differs, start) {
            (true, None) => start = Some(position),
            (false, Some(from)) => {
                if from < position {
                    lines.push(from..position);
                }
                start = None;
            }
            _ => {}
        }
    }
    // Lines whose spans differ.
    let spans = |presentation: &SidePresentation| {
        let mut map: std::collections::BTreeMap<usize, Vec<Range<usize>>> = Default::default();
        for (line, span) in &presentation.text {
            map.entry(*line).or_default().push(span.clone());
        }
        map
    };
    let (old_spans, new_spans) = (spans(old), spans(new));
    for line in old_spans.keys().chain(new_spans.keys()) {
        if old_spans.get(line) != new_spans.get(line) {
            lines.push(*line..*line + 1);
        }
    }
    // Lines whose filler or divider differs. Lines that become hidden or
    // shown need nothing here: hiding visits no lines and showing
    // invalidates them.
    let (old_margins, new_margins) = (line_margins(old, count), line_margins(new, count));
    for line in old_margins.keys().chain(new_margins.keys()) {
        if old_margins.get(line) != new_margins.get(line) {
            lines.push(*line..*line + 1);
        }
    }
    lines.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in lines {
        match merged.last_mut() {
            Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Install or clear diff mode's presentation of this view's side.
    /// `changed_lines` names the hard lines whose appearance changed, when
    /// known, so other cached lines keep their geometry.
    pub(crate) fn install_diff_presentation(
        &mut self,
        view_id: ViewId,
        presentation: Option<Arc<DiffLayoutInput>>,
        changed_lines: Option<&[Range<usize>]>,
        collapsed: &[Range<usize>],
    ) -> Result<(), CoreError> {
        let (id, revision) = (self.document.id(), self.document.revision());
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        if presentation.is_none() { view.layout.set_diff_peer_bound(None); }
        let presented = view.layout.set_diff_presentation(id, revision, presentation, changed_lines);
        let hidden = view.layout.set_elided_lines(collapsed.to_vec());
        if !presented && !hidden {
            return Ok(());
        }
        cancel_active_layout_work(view);
        if hidden {
            self.anchor_viewport_beside_hidden_lines(view_id)?;
        }
        self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::PreserveViewport)?;
        Ok(())
    }

    /// Collapsing can hide the line the viewport is anchored to. Anchor it to
    /// the first line shown after those lines instead, with their divider
    /// just above the top, or for lines that reach the end of the file to the
    /// last line shown before them.
    fn anchor_viewport_beside_hidden_lines(&mut self, view_id: ViewId) -> Result<(), CoreError> {
        let divider = self.default_row_height(view_id)?;
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let Some(anchor) = view.viewport_anchor else { return Ok(()) };
        let projection = self.document.projection();
        let Some(run) = projection.presentation_line_at_offset(anchor.anchor.offset())
            .and_then(|line| view.layout.elided().run_at(line)) else { return Ok(()) };
        let count = projection.presentation_line_count();
        let (line, offset_from_reference) = if run.end < count {
            (run.end, -divider)
        } else if run.start > 0 {
            (run.start - 1, 0.0)
        } else {
            view.viewport_anchor = None;
            return Ok(());
        };
        let start = projection.presentation_line_range(line).map_or(0, |range| range.start);
        view.viewport_anchor = self.document.text_point(start).ok()
            .and_then(|point| self.document.text_anchor(point, Association::AfterInsertion,
                BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding).ok())
            .map(|anchor| ViewportTextAnchor { anchor, offset_from_reference, reference: ViewportAnchorReference::RowTop });
        Ok(())
    }

    /// Diff mode: a caret that moved onto collapsed context opens its run
    /// in this view before any caret geometry is requested, and records it
    /// so the session opens it on the other side too. Returns whether a run
    /// opened.
    pub(super) fn open_collapsed_at_caret(&mut self, view_id: ViewId) -> Result<bool, CoreError> {
        let view = self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        if view.layout.elided().is_empty() {
            return Ok(false);
        }
        let caret = view.search_preview_destination(&self.document).unwrap_or_else(|| view
            .commands
            .visual_position()
            .map_or(view.commands.cursor(), |position| position.text_offset));
        let projection = self.document.projection();
        let Some(line) = projection.presentation_line_at_offset(caret) else { return Ok(false) };
        let Some(run) = view.layout.elided().run_at(line) else { return Ok(false) };
        let divider = divider_line(&run, projection.presentation_line_count())
            .and_then(|(line, _)| projection.presentation_line_range(line).map(|range| (line, range.start)));
        let view = self.views.get_mut(&view_id).expect("validated view");
        view.layout.open_elided_run(run, divider);
        cancel_active_layout_work(view);
        Ok(true)
    }

    /// Whether this view's caret opened collapsed context that a diff
    /// session has not taken yet, so the host must sync to open it on the
    /// other side.
    pub fn has_opened_collapsed(&self, view_id: ViewId) -> Result<bool, CoreError> {
        Ok(self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?.layout.elided().has_opened())
    }

    /// The runs this view's caret opened since the last call.
    pub(crate) fn take_opened_collapsed(&mut self, view_id: ViewId) -> Vec<Range<usize>> {
        self.views.get_mut(&view_id).map_or_else(Vec::new, |view| view.layout.take_opened_elided())
    }

    /// The hard line of this view's caret.
    pub(crate) fn diff_caret_line(&self, view_id: ViewId) -> Result<Option<usize>, CoreError> {
        let view = self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        Ok(self.document.hard_line_at_offset(view.commands.cursor()))
    }

    /// Move a Normal-mode caret to the start of `line` without any other
    /// effect, so it is not left inside a run that is about to collapse.
    /// Returns false, moving nothing, in any other mode.
    pub(crate) fn place_diff_caret(&mut self, view_id: ViewId, line: usize) -> Result<bool, CoreError> {
        let Some(range) = self.document.projection().hard_line_range(line) else { return Ok(false) };
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        if view.commands.mode() != Mode::Normal {
            return Ok(false);
        }
        Ok(view.commands.set_cursor(&self.document, range.start))
    }

    /// Set the buffer-local `modifiable` policy, as `:set nomodifiable` does.
    /// Diff mode's base buffer uses it so no input path can edit the base.
    /// Changing it is not an edit and adds no history.
    pub fn set_modifiable(&mut self, modifiable: bool) {
        self.document.set_modifiable(modifiable);
    }

    /// The unscaled height of one default Code row, which sizes aligned
    /// filler so that a filler line matches a text line.
    pub fn diff_filler_line_height(&mut self, view_id: ViewId) -> Result<f32, CoreError> {
        let advance = self.default_row_height(view_id)?;
        let scale = self.views[&view_id].layout.scale();
        Ok(if scale > 0.0 { advance / scale } else { advance })
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
