//! Diff mode's coordinator side. A [`DiffSession`] compares a read-only base
//! core with the live core of the document being edited, keeps the line diff
//! current as the live document changes, and installs each side's
//! presentation (changed lines, changed spans and aligned filler) in its view.
//! See `docs/diff_mode.md`.

use super::*;
use crate::diff::{DiffLimits, DiffState, LineEdit, LineSource, Side, SidePresentation};
use crate::layout::DiffLayoutInput;
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

#[derive(Clone, Debug, Default)]
struct InstalledSide {
    view: Option<ViewId>,
    document: Option<(DocumentId, Revision)>,
    presentation: SidePresentation,
    aligned: bool,
    filler_points: f32,
}

/// One diff mode comparison. Hosts keep it beside the two cores and call
/// [`DiffSession::sync`] after every event that may change either side.
pub struct DiffSession {
    state: DiffState,
    limits: DiffLimits,
    base_document: DocumentId,
    base_revision: Revision,
    new_document: DocumentId,
    new_revision: Revision,
    aligned: bool,
    installed: [InstalledSide; 2],
}

impl DiffSession {
    /// Compute the diff between a base document and the live document.
    pub fn new(base: &Document, new: &Document, aligned: bool) -> Self {
        Self::with_limits(base, new, aligned, DiffLimits::default())
    }

    pub fn with_limits(base: &Document, new: &Document, aligned: bool, limits: DiffLimits) -> Self {
        Self {
            state: DiffState::compute(&DocumentLines(base), &DocumentLines(new), limits),
            limits,
            base_document: base.id(),
            base_revision: base.revision(),
            new_document: new.id(),
            new_revision: new.revision(),
            aligned,
            installed: Default::default(),
        }
    }

    pub fn state(&self) -> &DiffState {
        &self.state
    }

    pub fn aligned(&self) -> bool {
        self.aligned
    }

    /// Change whether the sides are aligned; the next install applies it.
    pub fn set_aligned(&mut self, aligned: bool) {
        self.aligned = aligned;
    }

    /// The line on the other side corresponding to `line`.
    pub fn corresponding_line(&self, from_new: bool, line: usize) -> usize {
        self.state.corresponding_line(from_new, line)
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
            Some(edit) if edit_fits(&self.state, new, edit) => {
                self.state.apply_edit(&DocumentLines(base), &DocumentLines(new), edit);
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
        let side = &mut self.installed[1];
        if side.document.is_none() {
            return;
        }
        let delta = edit.new_end as isize - edit.old_end as isize;
        let map = |line: usize| -> Option<usize> {
            if line < edit.start {
                Some(line)
            } else if line >= edit.old_end {
                Some((line as isize + delta) as usize)
            } else {
                None
            }
        };
        let presentation = &mut side.presentation;
        let mut lines = Vec::new();
        for range in presentation.lines.drain(..) {
            let before = range.start..range.end.min(edit.start);
            let after = range.start.max(edit.old_end)..range.end;
            for part in [before, after] {
                if !part.is_empty() {
                    lines.push(map(part.start).unwrap()..map(part.end - 1).unwrap() + 1);
                }
            }
        }
        presentation.lines = lines;
        presentation.text = presentation.text.drain(..).filter_map(|(line, span)| map(line).map(|line| (line, span))).collect();
        presentation.fillers = presentation
            .fillers
            .drain(..)
            .filter_map(|filler| {
                let before_line = if filler.before_line >= edit.old_end {
                    (filler.before_line as isize + delta) as usize
                } else if filler.before_line <= edit.start {
                    filler.before_line
                } else {
                    return None;
                };
                Some(crate::diff::Filler { before_line, ..filler })
            })
            .collect();
    }

    /// The viewport top for `to_view` that shows the text corresponding to
    /// the top of `from_view`: the same line (through the diff) at the same
    /// fraction of its height. Raw offsets are not enough while either side's
    /// offscreen heights are still estimates.
    pub fn corresponding_top<P: TextMeasurementProvider, Q: TextMeasurementProvider>(
        &self,
        from: &Core<P>,
        from_view: ViewId,
        from_new: bool,
        to: &Core<Q>,
        to_view: ViewId,
    ) -> Result<f32, CoreError> {
        let top = from.viewport_state(from_view)?.top();
        let from_layout = from.layout(from_view).ok_or(CoreError::UnknownView(from_view))?;
        let to_layout = to.layout(to_view).ok_or(CoreError::UnknownView(to_view))?;
        let Some(hit) = from_layout.hard_line_at_y(f64::from(top)).map_err(LayoutError::from)? else {
            return Ok(top);
        };
        let fraction = if hit.line_height() > 0.0 {
            ((f64::from(top) - hit.line_top()) / hit.line_height()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let count = to.document().line_count().max(1);
        let line = self.corresponding_line(from_new, hit.hard_line()).min(count - 1);
        let line_top = to_layout.hard_line_prefix_height(line).map_err(LayoutError::from)?.height();
        let line_height = to_layout.hard_line_range_height(line..line + 1).map_err(LayoutError::from)?.height();
        Ok((line_top + fraction * line_height) as f32)
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
        self.install(base, base_view, Side::Base)?;
        self.install(new, new_view, Side::New)?;
        Ok(sync)
    }

    fn install<P: TextMeasurementProvider>(&mut self, core: &mut Core<P>, view: ViewId, side: Side) -> Result<(), CoreError> {
        let index = usize::from(side == Side::New);
        let filler_points = if self.aligned { core.diff_filler_line_height(view)? } else { 0.0 };
        let presentation = self.state.presentation(side, self.aligned);
        let document = (core.document().id(), core.document().revision());
        let installed = &self.installed[index];
        let comparable = installed.view == Some(view)
            && installed.aligned == self.aligned
            && installed.filler_points == filler_points
            && installed.document.is_some_and(|(id, _)| id == document.0);
        if comparable && installed.document == Some(document) && installed.presentation == presentation {
            return Ok(());
        }
        let changed = comparable.then(|| changed_lines(&installed.presentation, &presentation));
        let input = layout_input(core.document(), side, &presentation, filler_points);
        core.install_diff_presentation(view, Some(Arc::new(input)), changed.as_deref())?;
        self.installed[index] = InstalledSide { view: Some(view), document: Some(document), presentation, aligned: self.aligned, filler_points };
        Ok(())
    }

    /// Remove diff presentation from a view, for leaving diff mode.
    pub fn clear<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId) -> Result<(), CoreError> {
        core.install_diff_presentation(view, None, None)
    }
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

fn edit_fits(state: &DiffState, new: &Document, edit: LineEdit) -> bool {
    edit.old_end <= state.new_line_count()
        && state.new_line_count() - (edit.old_end - edit.start) + (edit.new_end - edit.start) == new.line_count()
}

/// Convert one side's presentation from hard lines to formatted offsets.
fn layout_input(document: &Document, side: Side, presentation: &SidePresentation, filler_points: f32) -> DiffLayoutInput {
    let projection = document.projection();
    let count = document.line_count();
    let line = |index: usize| projection.hard_line_range(index.min(count.saturating_sub(1)));
    let mut input = DiffLayoutInput {
        added: side == Side::New,
        text_len: projection.text_tree().byte_len(),
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
    for filler in &presentation.fillers {
        let points = filler.lines as f32 * filler_points;
        if filler.before_line >= count {
            input.trailing_filler += points;
        } else if let Some(range) = line(filler.before_line) {
            input.fillers.push((range.start, points));
        }
    }
    input
}

/// Hard lines whose diff appearance differs between two presentations of
/// the same side, as sorted ranges.
fn changed_lines(old: &SidePresentation, new: &SidePresentation) -> Vec<Range<usize>> {
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
    // Lines whose filler differs, including the last line for end filler.
    let fillers = |presentation: &SidePresentation| {
        presentation.fillers.iter().map(|filler| (filler.before_line, filler.lines)).collect::<std::collections::BTreeMap<_, _>>()
    };
    let (old_fillers, new_fillers) = (fillers(old), fillers(new));
    for line in old_fillers.keys().chain(new_fillers.keys()) {
        if old_fillers.get(line) != new_fillers.get(line) {
            // Filler is the following line's margin, or the last line's
            // when it follows the end; cover both.
            lines.push(line.saturating_sub(1)..line + 1);
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
    ) -> Result<(), CoreError> {
        let (id, revision) = (self.document.id(), self.document.revision());
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        if !view.layout.set_diff_presentation(id, revision, presentation, changed_lines) {
            return Ok(());
        }
        cancel_active_layout_work(view);
        self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::PreserveViewport)?;
        Ok(())
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
        let code_sheet;
        let sheet = if self.document.format().is_code() {
            code_sheet = crate::document::code_style::snapshot();
            &code_sheet
        } else {
            self.document.projection().style_sheet()
        };
        let paragraph = sheet
            .default_paragraph_style()
            .map_err(|_| CoreError::Layout(LayoutError::MalformedMeasurement("Invalid default paragraph style".into())))?;
        let style = crate::layout::shaping_style(&paragraph.character)
            .map_err(|_| CoreError::Layout(LayoutError::MalformedMeasurement("Invalid default paragraph font".into())))?;
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let scale = view.layout.scale();
        let height = crate::layout::default_line_height(view.engine.provider_mut(), self.document.id(),
            self.document.revision(), &style, scale)
            .map_err(|error| CoreError::Layout(LayoutError::Measurement(error)))?;
        let advance = crate::layout::line_advance(height, paragraph.line_spacing);
        Ok(if scale > 0.0 { advance / scale } else { advance })
    }
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
