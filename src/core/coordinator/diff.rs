//! Diff mode's coordinator side. A [`DiffSession`] compares a read-only base
//! core with the live core of the document being edited, keeps the line diff
//! current as the live document changes, and installs each side's
//! presentation (changed lines, changed spans, aligned filler and collapsed
//! context) in its view.
//! See `docs/diff_mode.md`.

use super::*;
use crate::diff::{CollapsibleRun, DiffLimits, DiffState, Hunk, LineEdit, LineSource, Side, SidePresentation, DIFF_CONTEXT_LINES};
use crate::layout::{DiffLayoutInput, DiffMargin};
use crate::diff::state::{DiffLineBasis, retained_pair_spans};
use std::borrow::Cow;
use std::collections::BTreeMap;
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

/// Height differences, in unscaled points, below which two corresponding
/// lines count as level.
const BALANCE_TOLERANCE: f32 = 0.05;

/// Filler that keeps an aligned pair level where the two sides' heights
/// differ, for example because a line wraps to more rows on one side, in
/// unscaled points.
#[derive(Clone, Debug, Default, PartialEq)]
struct Balance {
    /// Below an unchanged line that is taller on the other side, by line.
    lines: BTreeMap<usize, f32>,
    /// Added to or taken from a change's filler on this side, keyed like
    /// [`crate::diff::Filler::before_line`].
    changes: BTreeMap<usize, f32>,
}

impl Balance {
    /// The lines whose margins differ between `self` and `other`, sorted,
    /// on a side of `count` lines.
    fn changed_lines(&self, other: &Self, count: usize) -> Vec<Range<usize>> {
        let mut lines: Vec<usize> = Vec::new();
        let mut differ = |a: &BTreeMap<usize, f32>, b: &BTreeMap<usize, f32>, line: fn(usize, usize) -> usize| {
            for (key, value) in a.iter() {
                if b.get(key) != Some(value) {
                    lines.push(line(*key, count));
                }
            }
            for key in b.keys() {
                if !a.contains_key(key) {
                    lines.push(line(*key, count));
                }
            }
        };
        differ(&self.lines, &other.lines, |line, _| line);
        differ(&self.changes, &other.changes, |before, count| before.min(count.saturating_sub(1)));
        lines.sort_unstable();
        lines.dedup();
        lines.into_iter().map(|line| line..line + 1).collect()
    }
}

#[derive(Clone, Debug, Default)]
struct InstalledSide {
    view: Option<ViewId>,
    document: Option<(DocumentId, Revision)>,
    presentation: SidePresentation,
    /// Filler keeping the sides level beyond what line counts give.
    balance: Balance,
    aligned: bool,
    /// One default row, unscaled: a filler line and a divider's height.
    row_points: f32,
}

/// The last synchronized caret line. An added line or an isolated pairing
/// can be verified against the same frozen base before the edit's first
/// layout. At most one bounded base line is retained per live diff view.
pub(super) struct DiffTypingLine {
    line: usize,
    basis: DiffLineBasis<String>,
}

impl DiffTypingLine {
    fn for_caret<P: TextMeasurementProvider>(base: &Document, new: &Core<P>, view: ViewId, diff: &DiffState) -> Option<Self> {
        if !new.view_format(view).is_ok_and(|format| format.is_code()) { return None; }
        let line = new.document().hard_line_at_offset(new.views[&view].commands.cursor())?;
        let base_text = |index| {
            let projection = base.projection();
            let range = projection.hard_line_range(index)?;
            if range.len() > crate::diff::inline::MAX_INLINE_LINE_BYTES { return None; }
            projection.text_tree().slice(range).ok()
        };
        let basis = match diff.line_basis(line)? {
            DiffLineBasis::Added => DiffLineBasis::Added,
            DiffLineBasis::Single(index) => DiffLineBasis::Single(base_text(index)?),
            DiffLineBasis::Paired(index) => DiffLineBasis::Paired(base_text(index)?),
        };
        Some(Self { line, basis })
    }

    pub(super) fn rebase(&self, document: &Document, change: &DocumentLayoutChange, layout: &mut ViewLayout) {
        let DocumentLayoutChange::Local { old_line_count, new_line_count, new_hull, .. } = change else { return };
        if !document.format().is_code() || old_line_count != new_line_count { return; }
        let projection = document.projection();
        let Some(line) = projection.hard_line_range(self.line) else { return };
        // A Code change is local to these bytes on both sides. No new break
        // is inside the hull, and an old break would have changed the count.
        if new_hull.start < line.start || new_hull.end > line.end { return; }
        let spans = match &self.basis {
            DiffLineBasis::Added => Vec::new(),
            DiffLineBasis::Single(base) | DiffLineBasis::Paired(base) => {
                if line.len() > crate::diff::inline::MAX_INLINE_LINE_BYTES { return; }
                let Ok(text) = projection.text_tree().slice(line.clone()) else { return };
                match retained_pair_spans(base, &text) {
                    Some(spans) => spans.new,
                    None if matches!(self.basis, DiffLineBasis::Single(_)) && base != &text => Vec::new(),
                    None => return,
                }
            }
        };
        layout.rebase_diff_line(document.id(), change, line, spans);
    }
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
        // The live edit has no bounded line map. The base, however, is
        // usually the very same revision. Its installed markings can still
        // identify exactly which lines need reflow after a live reload.
        self.installed[1].document = None;
        if self.installed[0].document != Some((self.base_document, self.base_revision)) {
            self.installed[0].document = None;
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
        let balance = &mut side.balance;
        balance.lines = std::mem::take(&mut balance.lines).into_iter()
            .filter_map(|(line, points)| Some((edit.map_line(line)?, points))).collect();
        balance.changes = std::mem::take(&mut balance.changes).into_iter()
            .filter_map(|(before, points)| Some((edit.map_boundary(before)?, points))).collect();
    }

    /// Whether `side`'s installed presentation describes `view` showing
    /// `document` as it is now.
    fn is_current(&self, side: Side, view: ViewId, document: &Document) -> bool {
        let installed = self.installed(side);
        installed.view == Some(view) && installed.document == Some((document.id(), document.revision()))
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
        &mut self, base: &mut Core<P>, base_view: ViewId, new: &mut Core<Q>, new_view: ViewId, leader: Side,
    ) -> Result<(bool, bool), CoreError> {
        // Each side answers for its own view: a diff's live view is Code.
        base.focus_view(base_view);
        new.focus_view(new_view);
        if !self.aligned || !self.is_current(Side::Base, base_view, base.document())
            || !self.is_current(Side::New, new_view, new.document()) {
            return Ok((false, false));
        }
        let before_base = base.viewport_state(base_view)?;
        let before_new = new.viewport_state(new_view)?;
        let new_request = new.views[&new_view].layout.diff_scroll_request;
        let base_request = base.views[&base_view].layout.diff_scroll_request;
        let request = match leader {
            Side::New => new_request.or(base_request),
            Side::Base => base_request.or(new_request),
        };
        // Balancing keeps each view's text where it was; the leader's
        // position after it is the one to share.
        let mut presented = self.balance(base, base_view, new, new_view)?;
        let leader_top = |base: &Core<P>, new: &Core<Q>| match leader {
            Side::New => new.views[&new_view].layout.viewport_top(),
            Side::Base => base.views[&base_view].layout.viewport_top(),
        };
        let mut requested_top = request.unwrap_or(leader_top(base, new));
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
            // Newly measured lines may differ from their counterparts.
            let (base_balanced, new_balanced) = self.balance(base, base_view, new, new_view)?;
            presented = (presented.0 | base_balanced, presented.1 | new_balanced);
            let leader_balanced = match leader { Side::New => new_balanced, Side::Base => base_balanced };
            if request.is_none() && !end && leader_balanced {
                requested_top = leader_top(base, new);
            }
        }
        self.share_scroll_bounds(base, base_view, new, new_view)?;
        base.views.get_mut(&base_view).unwrap().layout.diff_scroll_request = None;
        new.views.get_mut(&new_view).unwrap().layout.diff_scroll_request = None;
        Ok((presented.0 || before_base != base.viewport_state(base_view)?,
            presented.1 || before_new != new.viewport_state(new_view)?))
    }

    /// Keep the aligned sides level where their heights differ: an unchanged
    /// line taller on one side gets filler below it on the other, and a
    /// change gets filler on its shorter side, beyond what line counts give.
    /// Heights are compared as measured or estimated now, only where either
    /// side's heights changed since the last call. Returns whether each
    /// side's presentation changed.
    fn balance<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &mut self, base: &mut Core<P>, base_view: ViewId, new: &mut Core<Q>, new_view: ViewId,
    ) -> Result<(bool, bool), CoreError> {
        // Changes wait in the views until their presentation is current.
        if !self.is_current(Side::Base, base_view, base.document()) || !self.is_current(Side::New, new_view, new.document()) {
            return Ok((false, false));
        }
        let base_changes = base.views.get_mut(&base_view).ok_or(CoreError::UnknownView(base_view))?.layout.take_height_changes();
        let new_changes = new.views.get_mut(&new_view).ok_or(CoreError::UnknownView(new_view))?.layout.take_height_changes();
        if !self.aligned || (base_changes.is_empty() && new_changes.is_empty()) {
            return Ok((false, false));
        }
        let heights = [
            SideHeights::new(&base.views[&base_view].layout, self.installed(Side::Base), base.document().line_count()),
            SideHeights::new(&new.views[&new_view].layout, self.installed(Side::New), new.document().line_count()),
        ];
        let changes = [&base_changes, &new_changes];
        let mut next = [self.installed(Side::Base).balance.clone(), self.installed(Side::New).balance.clone()];
        for (base_lines, new_lines, equal) in self.state.regions() {
            let lines = [base_lines, new_lines];
            if equal {
                let mut touched: Vec<Range<usize>> = Vec::new();
                for side in 0..2 {
                    if changes[side].all {
                        touched = vec![0..lines[0].len()];
                        break;
                    }
                    for changed in &changes[side].ranges {
                        let (start, end) = (changed.start.max(lines[side].start), changed.end.min(lines[side].end));
                        if start < end {
                            touched.push(start - lines[side].start..end - lines[side].start);
                        }
                    }
                }
                for offsets in merge_ranges(touched) {
                    for (side, balance) in next.iter_mut().enumerate() {
                        let own = lines[side].start + offsets.start..lines[side].start + offsets.end;
                        let inside = own.start.max(lines[side].start + 1)..own.end.min(lines[side].end);
                        clear(&mut balance.lines, own);
                        clear(&mut balance.changes, inside);
                    }
                    balance_unchanged(&heights, [&lines[0], &lines[1]], offsets, &mut next)?;
                }
            } else {
                let touched = (0..2).any(|side| changes[side].all || changes[side].ranges.iter()
                    .any(|changed| changed.start <= lines[side].end && lines[side].start <= changed.end));
                if !touched {
                    continue;
                }
                let content = [heights[0].content(lines[0].clone())?, heights[1].content(lines[1].clone())?];
                let tallest = content[0].max(content[1]);
                for (side, balance) in next.iter_mut().enumerate() {
                    let own = &lines[side];
                    clear(&mut balance.lines, own.clone());
                    clear(&mut balance.changes, own.start..own.end + 1);
                    let filler_rows = lines[1 - side].len().saturating_sub(own.len());
                    let extra = tallest - content[side] - filler_rows as f32 * heights[side].row;
                    if extra.abs() >= BALANCE_TOLERANCE {
                        balance.changes.insert(own.end, extra);
                    }
                }
            }
        }
        let mut presented = [false, false];
        let [base_next, new_next] = next;
        if base_next != self.installed(Side::Base).balance {
            self.install_balance(base, base_view, Side::Base, base_next)?;
            presented[0] = true;
        }
        if new_next != self.installed(Side::New).balance {
            self.install_balance(new, new_view, Side::New, new_next)?;
            presented[1] = true;
        }
        Ok((presented[0], presented[1]))
    }

    /// Install `side` again with a new balance, relaying out only the lines
    /// whose margins it changes.
    fn install_balance<P: TextMeasurementProvider>(&mut self, core: &mut Core<P>, view: ViewId, side: Side,
        balance: Balance) -> Result<(), CoreError> {
        let installed = self.installed(side);
        let count = core.document().line_count();
        let changed = installed.balance.changed_lines(&balance, count);
        let filler_points = if installed.aligned { installed.row_points } else { 0.0 };
        let input = layout_input(core.document(), side, &installed.presentation, &balance, filler_points, installed.row_points);
        let collapsed = installed.presentation.collapsed.clone();
        core.install_diff_presentation(view, Some(Arc::new(input)), Some(&changed), &collapsed)?;
        self.installed_mut(side).balance = balance;
        Ok(())
    }

    /// Update the diff and install both sides' presentations in their views.
    pub fn sync<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &mut self,
        base: &mut Core<P>,
        base_view: ViewId,
        new: &mut Core<Q>,
        new_view: ViewId,
    ) -> Result<DiffSync, CoreError> {
        // Each side answers for its own view: a diff's live view is Code.
        base.focus_view(base_view);
        new.focus_view(new_view);
        new.views.get_mut(&new_view).ok_or(CoreError::UnknownView(new_view))?.diff_typing_line = None;
        let live_changed = (self.new_document, self.new_revision) != (new.document().id(), new.document().revision())
            || new.views[&new_view].layout.awaits_live_diff(new.document().id(), new.document().revision());
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
        self.balance(base, base_view, new, new_view)?;
        self.share_scroll_bounds(base, base_view, new, new_view)?;
        let base_layout = &base.views[&base_view].layout;
        let live_layout = &new.views[&new_view].layout;
        if self.aligned && live_changed && (base_layout.viewport_top() != live_layout.viewport_top()
            || base_layout.diff_scroll_request.is_some() || live_layout.diff_scroll_request.is_some()) {
            // A live edit has already preserved its text anchor and revealed
            // the caret. The base may be anchored to text below growing
            // filler: settle it from the live side before either pane draws.
            self.sync_scroll(base, base_view, new, new_view, Side::New)?;
        }
        self.runs = runs;
        let typing = DiffTypingLine::for_caret(base.document(), new, new_view, &self.state);
        new.views.get_mut(&new_view).expect("installed live view").diff_typing_line = typing;
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
        // Undo can return to this exact revision before the host syncs. A
        // full history rebase already discarded the layout's margins and
        // elisions, even when the session's saved presentation still matches.
        let layout = &core.views[&view].layout;
        if comparable && installed.document == Some(document) && installed.presentation == presentation
            && layout.has_current_diff(document.0, document.1) && layout.elided().runs() == presentation.collapsed {
            return Ok(());
        }
        // Balance found for the lines as they were stays until those lines
        // are compared again; a full relayout compares every line.
        let balance = if comparable { installed.balance.clone() } else { Balance::default() };
        let filler_points = if self.aligned { row_points } else { 0.0 };
        let count = core.document().line_count();
        let changed = comparable.then(|| changed_lines(
            Margins { presentation: &installed.presentation, balance: &installed.balance, filler_points },
            Margins { presentation: &presentation, balance: &balance, filler_points }, count));
        let input = layout_input(core.document(), side, &presentation, &balance, filler_points, row_points);
        core.install_diff_presentation(view, Some(Arc::new(input)), changed.as_deref(), &presentation.collapsed)?;
        *self.installed_mut(side) = InstalledSide {
            view: Some(view), document: Some(document), presentation, balance, aligned: self.aligned, row_points,
        };
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
        // Each side answers for its own view: a diff's live view is Code.
        base.focus_view(base_view);
        new.focus_view(new_view);
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
        let (new_side, base_side) = (self.installed(Side::New), self.installed(Side::Base));
        let new_row = f64::from(new_side.row_points * new.views[&new_view].layout.scale());
        let base_row = f64::from(base_side.row_points * base.views[&base_view].layout.scale());
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

    /// Size a scrollbar export without measuring the geometry of every mark.
    pub(crate) fn scrollbar_mark_info<P: TextMeasurementProvider>(
        &self, core: &Core<P>, view: ViewId, side: Side,
    ) -> Result<(usize, f32), CoreError> {
        core.focus_view(view);
        let installed = self.installed(side);
        let layout = core.layout(view).ok_or(CoreError::UnknownView(view))?;
        let extent = if layout.has_shared_scroll_range() && layout.estimated_maximum_viewport_top() > 0.0 {
            layout.estimated_maximum_viewport_top() + layout.height()
        } else {
            layout.hard_line_prefix_height(core.document().line_count()).map_err(LayoutError::from)?.height() as f32
        };
        if !self.is_current(side, view, core.document()) { return Ok((0, extent)); }
        let other = if installed.aligned { installed.presentation.fillers.len() } else {
            self.state.regions().filter(|(base, new, equal)| !equal && match side {
                Side::Base => base.is_empty(), Side::New => new.is_empty(),
            }).count()
        };
        Ok((installed.presentation.lines.len() + other, extent))
    }

    /// The marks for one side's scrollbar track and the document extent they
    /// are measured against: that side's changed lines in its own colour,
    /// and the other side's changes where they line up, at its filler or,
    /// when unaligned, as an empty mark between lines.
    pub fn scrollbar_marks<P: TextMeasurementProvider>(
        &self,
        core: &Core<P>,
        view: ViewId,
        side: Side,
    ) -> Result<(Vec<ScrollbarMark>, f32), CoreError> {
        core.focus_view(view);
        let installed = self.installed(side);
        let count = core.document().line_count();
        let (mark_count, extent) = self.scrollbar_mark_info(core, view, side)?;
        if mark_count == 0 { return Ok((Vec::new(), extent)); }
        let row = f64::from(installed.row_points * core.layout(view).ok_or(CoreError::UnknownView(view))?.scale());
        let own = side == Side::New;
        let presentation = &installed.presentation;
        let mut marks = Vec::with_capacity(mark_count);
        for range in &presentation.lines {
            let block = side_block(core, view, installed, range.start..range.end, 0.0)?;
            marks.push(ScrollbarMark { top: block.start as f32, bottom: block.end as f32, added: own });
        }
        if installed.aligned {
            for filler in &presentation.fillers {
                let line = filler.before_line.min(count);
                let block = side_block(core, view, installed, line..line, row)?;
                marks.push(ScrollbarMark { top: block.start as f32, bottom: block.end as f32, added: !own });
            }
        } else {
            for hunk in self.state.hunks() {
                let lines = hunk.side(side);
                if lines.is_empty() {
                    let block = side_block(core, view, installed, lines.start..lines.start, 0.0)?;
                    marks.push(ScrollbarMark { top: block.start as f32, bottom: block.start as f32, added: !own });
                }
            }
        }
        marks.sort_by(|a, b| a.top.total_cmp(&b.top));
        Ok((marks, extent))
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
/// covers the trailing filler. `row` sizes one filler line; with `row` zero,
/// filler is left out.
fn side_block<P: TextMeasurementProvider>(
    core: &Core<P>,
    view: ViewId,
    installed: &InstalledSide,
    lines: Range<usize>,
    row: f64,
) -> Result<Range<f64>, CoreError> {
    let layout = core.layout(view).ok_or(CoreError::UnknownView(view))?;
    let count = core.document().line_count();
    let prefix = |line: usize| -> Result<f64, CoreError> {
        Ok(layout.hard_line_prefix_height(line.min(count)).map_err(LayoutError::from)?.height())
    };
    let presentation = &installed.presentation;
    let scale = f64::from(layout.scale());
    // Fillers are sorted by line: find each line's by binary search, so
    // marking every change stays linear in the changes.
    let filler = |line: usize| -> f64 {
        if row == 0.0 {
            return 0.0;
        }
        let first = presentation.fillers.partition_point(|filler| filler.before_line < line);
        let rows: usize = presentation.fillers[first..].iter().take_while(|filler| filler.before_line == line)
            .map(|filler| filler.lines).sum();
        let balance = installed.balance.changes.get(&line).map_or(0.0, |points| f64::from(*points) * scale);
        (rows as f64 * row + balance).max(0.0)
    };
    let start = if lines.start < count { prefix(lines.start)? } else { (prefix(count)? - filler(count)).max(0.0) };
    let end = if lines.end < count { prefix(lines.end)? + filler(lines.end) } else { prefix(count)? };
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
fn layout_input(document: &Document, side: Side, presentation: &SidePresentation, balance: &Balance,
    filler_points: f32, row_points: f32) -> DiffLayoutInput {
    let projection = document.projection();
    let count = document.line_count();
    let line = |index: usize| projection.hard_line_range(index.min(count.saturating_sub(1)));
    let mut input = DiffLayoutInput {
        added: side == Side::New,
        row_points,
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
    let margins = Margins { presentation, balance, filler_points };
    for (index, margin) in margins.by_line(count) {
        if let Some(range) = projection.hard_line_range(index) {
            input.margins.push(DiffMargin {
                line: index,
                at: range.start,
                filler_before: margin.filler_before,
                filler_after: margin.filler_after,
                divider: margin.divider,
            });
        }
    }
    input
}

/// The margins diff mode adds to one hard line: filler above it, and below
/// it for the last line or a line taller on the other side, in unscaled
/// points, and a collapsed run's divider.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct LineMargin {
    filler_before: f32,
    filler_after: f32,
    /// Whether the divider follows the line, and whether its hidden lines
    /// reach the start or end of the file.
    divider: Option<(bool, bool)>,
}

/// Everything that gives one side's lines margins.
#[derive(Clone, Copy)]
struct Margins<'a> {
    presentation: &'a SidePresentation,
    balance: &'a Balance,
    /// One filler line's height; zero when the sides aren't aligned.
    filler_points: f32,
}

impl Margins<'_> {
    /// The margins by hard line, for a side of `count` lines.
    fn by_line(&self, count: usize) -> BTreeMap<usize, LineMargin> {
        let mut margins = BTreeMap::<usize, LineMargin>::new();
        if self.filler_points > 0.0 {
            let mut fillers = BTreeMap::<usize, f32>::new();
            for filler in &self.presentation.fillers {
                *fillers.entry(filler.before_line).or_default() += filler.lines as f32 * self.filler_points;
            }
            for (before, points) in &self.balance.changes {
                *fillers.entry(*before).or_default() += points;
            }
            for (before, points) in fillers {
                let points = points.max(0.0);
                if points == 0.0 {
                    continue;
                }
                if before >= count {
                    margins.entry(count.saturating_sub(1)).or_default().filler_after += points;
                } else {
                    margins.entry(before).or_default().filler_before += points;
                }
            }
            // Filler below a line goes above the next one where it is shown,
            // so it adds to a change's filler there instead of collapsing
            // with it.
            let hidden = |line: usize| {
                let index = self.presentation.collapsed.partition_point(|run| run.end <= line);
                self.presentation.collapsed.get(index).is_some_and(|run| run.start <= line)
            };
            for (line, points) in &self.balance.lines {
                if line + 1 < count && !hidden(line + 1) {
                    margins.entry(line + 1).or_default().filler_before += points;
                } else if *line < count {
                    margins.entry(*line).or_default().filler_after += points;
                }
            }
        }
        line_dividers(self.presentation, count, &mut margins);
        margins
    }
}

/// Add each collapsed run's divider to the margins of the line it borders.
fn line_dividers(presentation: &SidePresentation, count: usize, margins: &mut BTreeMap<usize, LineMargin>) {
    for run in &presentation.collapsed {
        if let Some((line, after)) = divider_line(run, count) {
            margins.entry(line).or_default().divider = Some((after, !after || run.end >= count));
        }
    }
}

/// One side's measured and estimated heights, less what diff mode's own
/// margins and the document's insets add, in unscaled points.
struct SideHeights<'a> {
    layout: &'a crate::layout::ViewLayoutState,
    /// What margins and insets add to each line's height band.
    extra: BTreeMap<usize, f32>,
    scale: f32,
    /// One filler line's height.
    row: f32,
}

impl<'a> SideHeights<'a> {
    fn new(layout: &'a crate::layout::ViewLayoutState, installed: &InstalledSide, count: usize) -> Self {
        let filler_points = if installed.aligned { installed.row_points } else { 0.0 };
        let margins = Margins { presentation: &installed.presentation, balance: &installed.balance, filler_points };
        let scale = layout.scale();
        let row = installed.row_points;
        // A band runs from a line's first row to the next line's, so the
        // filler above a line belongs to the line before it, unless the line
        // begins the laid-out document (as `DiffLayoutInput::band_margins`).
        let mut extra = BTreeMap::<usize, f32>::new();
        for (line, margin) in margins.by_line(count) {
            let own = margin.filler_after + if margin.divider.is_some() { row } else { 0.0 };
            if line == 0 || margin.divider.is_some_and(|(after, _)| !after) {
                *extra.entry(line).or_default() += margin.filler_before + own;
            } else {
                *extra.entry(line - 1).or_default() += margin.filler_before;
                *extra.entry(line).or_default() += own;
            }
        }
        let collapsed = &installed.presentation.collapsed;
        let first = collapsed.first().filter(|run| run.start == 0).map_or(0, |run| run.end);
        let last = collapsed.last().filter(|run| run.end >= count).map_or(count.checked_sub(1), |run| run.start.checked_sub(1));
        if let Some(insets) = layout.snapshot().map(|snapshot| snapshot.content_insets) {
            if first < count {
                *extra.entry(first).or_default() += insets.top / scale;
            }
            if let Some(last) = last {
                *extra.entry(last).or_default() += insets.bottom / scale;
            }
        }
        extra.retain(|_, points| *points != 0.0);
        Self { layout, extra, scale, row }
    }

    /// The height of `lines` without diff margins or insets.
    fn content(&self, lines: Range<usize>) -> Result<f32, CoreError> {
        let height = self.layout.hard_line_range_height(lines.clone()).map_err(LayoutError::from)?.height();
        let extra: f32 = self.extra.range(lines).map(|(_, points)| points).sum();
        Ok(height as f32 / self.scale - extra)
    }

    /// Runs of lines of equal height in `lines`, in unscaled points, with
    /// each line that has margins or insets as its own run without them.
    fn runs(&self, lines: Range<usize>) -> Result<Vec<(Range<usize>, f32)>, CoreError> {
        let mut runs = Vec::new();
        for (run, height) in self.layout.height_runs(lines).map_err(LayoutError::from)? {
            let height = height as f32 / self.scale;
            let mut start = run.start;
            for (&line, extra) in self.extra.range(run.clone()) {
                if start < line {
                    runs.push((start..line, height));
                }
                runs.push((line..line + 1, height - extra));
                start = line + 1;
            }
            if start < run.end {
                runs.push((start..run.end, height));
            }
        }
        Ok(runs)
    }
}

/// Balance the lines at `offsets` of one unchanged region, whose lines are
/// `lines` on each side: each line taller on one side gets filler below it on
/// the other.
fn balance_unchanged(heights: &[SideHeights; 2], lines: [&Range<usize>; 2], offsets: Range<usize>,
    balance: &mut [Balance; 2]) -> Result<(), CoreError> {
    let runs = [
        heights[0].runs(lines[0].start + offsets.start..lines[0].start + offsets.end)?,
        heights[1].runs(lines[1].start + offsets.start..lines[1].start + offsets.end)?,
    ];
    let (mut base_runs, mut new_runs) = (runs[0].iter().peekable(), runs[1].iter().peekable());
    let mut offset = offsets.start;
    while let (Some((base_run, base_height)), Some((new_run, new_height))) = (base_runs.peek(), new_runs.peek()) {
        let base_end = base_run.end - lines[0].start;
        let new_end = new_run.end - lines[1].start;
        let end = base_end.min(new_end);
        let difference = base_height - new_height;
        if difference.abs() >= BALANCE_TOLERANCE {
            let shorter = usize::from(difference > 0.0);
            for line in offset..end {
                balance[shorter].lines.insert(lines[shorter].start + line, difference.abs());
            }
        }
        offset = end;
        if base_end == end {
            base_runs.next();
        }
        if new_end == end {
            new_runs.next();
        }
    }
    Ok(())
}

/// Remove the entries keyed in `keys`, visiting only those.
fn clear(map: &mut BTreeMap<usize, f32>, keys: Range<usize>) {
    if keys.is_empty() {
        return;
    }
    let found: Vec<usize> = map.range(keys).map(|(key, _)| *key).collect();
    for key in found {
        map.remove(&key);
    }
}

/// Sort ranges and join those that overlap or touch.
fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
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

/// Trim entries that match on both sides of a local edit. A change to one
/// span only needs to compare the remaining spans on that line.
fn differing_entries<'a, T: PartialEq>(old: &'a [T], new: &'a [T]) -> (&'a [T], &'a [T]) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let (old, new) = (&old[prefix..], &new[prefix..]);
    let suffix = old.iter().rev().zip(new.iter().rev()).take_while(|(a, b)| a == b).count();
    (&old[..old.len() - suffix], &new[..new.len() - suffix])
}

/// Hard lines whose diff appearance differs between two presentations of
/// the same side of `count` lines, as sorted ranges.
fn changed_lines(old_margins: Margins, new_margins: Margins, count: usize) -> Vec<Range<usize>> {
    let (old, new) = (old_margins.presentation, new_margins.presentation);
    let mut lines: Vec<Range<usize>> = Vec::new();
    // Changed-line ranges: the symmetric difference of the two range sets.
    let mut edges: Vec<(usize, i32)> = Vec::new();
    let (old_lines, new_lines) = differing_entries(&old.lines, &new.lines);
    for range in old_lines {
        edges.push((range.start, 1));
        edges.push((range.end, -1));
    }
    for range in new_lines {
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
    let (mut before, mut after) = differing_entries(&old.text, &new.text);
    while let Some(line) = before.first().into_iter().chain(after.first()).map(|entry| entry.0).min() {
        let old_count = before.iter().take_while(|entry| entry.0 == line).count();
        let new_count = after.iter().take_while(|entry| entry.0 == line).count();
        if before[..old_count] != after[..new_count] {
            lines.push(line..line + 1);
        }
        before = &before[old_count..];
        after = &after[new_count..];
    }
    // Lines whose filler or divider differs. Lines that become hidden or
    // shown need nothing here: hiding visits no lines and showing
    // invalidates them.
    if old.fillers != new.fillers || old.collapsed != new.collapsed
        || old_margins.filler_points != new_margins.filler_points || old_margins.balance != new_margins.balance {
        let (old_margins, new_margins) = (old_margins.by_line(count), new_margins.by_line(count));
        for line in old_margins.keys().chain(new_margins.keys()) {
            if old_margins.get(line) != new_margins.get(line) {
                lines.push(*line..*line + 1);
            }
        }
    }
    merge_ranges(lines)
}

/// The mapped editing viewport while the live layout awaits its diff margins.
/// The provisional frame can expose a different top row inside missing filler.
pub(super) struct DiffEditViewport {
    anchor: ViewportTextAnchor,
    left: f32,
    intent: ImmediateLayoutIntent,
    history: Option<Range<usize>>,
    provisional_anchor: Option<ViewportTextAnchor>,
    origin: (f32, f32),
    caret: usize,
}

impl DiffEditViewport {
    pub(super) fn capture<P: TextMeasurementProvider>(view: &View<P>, anchor: ViewportTextAnchor, left: f32,
        intent: ImmediateLayoutIntent) -> Option<Self> {
        let snapshot = view.layout.snapshot()?;
        (snapshot.document_id == anchor.anchor.document() && snapshot.document_revision == anchor.anchor.revision())
            .then(|| Self { anchor, left, intent, history: None, provisional_anchor: view.viewport_anchor,
                origin: (view.layout.viewport_left(), view.layout.viewport_top()), caret: view.commands.cursor() })
    }

    fn matches_viewport<P: TextMeasurementProvider>(&self, view: &View<P>, id: DocumentId, revision: Revision) -> bool {
        self.anchor.anchor.document() == id && self.anchor.anchor.revision() == revision
            && view.layout.snapshot().is_some_and(|s| s.document_id == id && s.document_revision == revision)
            && view.layout.viewport_left() == self.origin.0
            && (view.layout.viewport_top() == self.origin.1
                || self.provisional_anchor.is_some_and(|anchor| view.viewport_anchor == Some(anchor)))
    }

    /// More than one command or a macro can edit before the host synchronizes.
    /// Map the original anchor through the next transaction as well.
    pub(super) fn continue_edit<P: TextMeasurementProvider>(&self, view: &mut View<P>, map: &PositionMap) -> Result<(), LayoutError> {
        if self.matches_viewport(view, map.document(), map.source_revision()) {
            view.viewport_anchor = Some(self.anchor);
            view.layout.set_viewport_left(self.left)?;
        }
        Ok(())
    }

    pub(super) fn after_history_reveal<P: TextMeasurementProvider>(self, view: &View<P>, range: Option<Range<usize>>) -> Option<Self> {
        let mut next = Self::capture(view, self.anchor, self.left, ImmediateLayoutIntent::PreserveViewportAndRevealCaret)?;
        next.history = range;
        Some(next)
    }
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
        let editing = view.diff_edit_viewport.take().filter(|edit| presentation.is_some()
            && edit.matches_viewport(view, id, revision) && view.commands.cursor() == edit.caret);
        if presentation.is_none() {
            view.layout.set_diff_peer_bound(None);
            view.diff_typing_line = None;
        }
        let presented = view.layout.set_diff_presentation(id, revision, presentation, changed_lines);
        let hidden = view.layout.set_elided_lines(collapsed.to_vec());
        if !presented && !hidden {
            return Ok(());
        }
        cancel_active_layout_work(view);
        if let Some(edit) = &editing {
            view.viewport_anchor = Some(edit.anchor);
            view.layout.set_viewport_left(edit.left)?;
        }
        if hidden {
            self.anchor_viewport_beside_hidden_lines(view_id)?;
        }
        let intent = editing.as_ref().map_or(ImmediateLayoutIntent::PreserveViewport, |edit| edit.intent);
        self.materialize_immediate_viewport(view_id, intent)?;
        if let Some(range) = editing.and_then(|edit| edit.history) {
            self.reveal_history_range(view_id, Some(&range))?;
        }
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
        self.focus_view(view_id);
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
        self.focus_view(view_id);
        let advance = self.default_row_height(view_id)?;
        let scale = self.views[&view_id].layout.scale();
        Ok(if scale > 0.0 { advance / scale } else { advance })
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
