//! Synchronous host input at an exact, bounded layout boundary.

use super::*;
use crate::command::layout_motion::LayoutDemandEdge;
use std::ops::Range;

#[derive(PartialEq)]
pub(super) struct InputLayoutRequestIdentity {
    requested: Range<usize>,
    horizontal_focus: Option<(usize, Option<f32>)>,
    complete_horizontal_geometry: bool,
    materialized_text: Option<Vec<Range<usize>>>,
}

impl<P: TextMeasurementProvider> Core<P> {
    /// A repeated hard-line interval can still advance through a giant line.
    /// Host input and compound replay use the same exact coverage progress key.
    pub(super) fn input_layout_request_identity(
        &self,
        view_id: ViewId,
        demand: &LayoutDemand,
        requested: Range<usize>,
    ) -> InputLayoutRequestIdentity {
        let materialized_text = self.views.get(&view_id).and_then(|view| view.layout.snapshot())
            .map(|snapshot| match &snapshot.coverage {
                LayoutCoverage::FullDocument { .. } => vec![0..self.document.projection().text_tree().byte_len()],
                LayoutCoverage::PartialHardLines { text_ranges, .. } => text_ranges.clone(),
            });
        InputLayoutRequestIdentity {
            requested,
            horizontal_focus: demand.horizontal_focus(),
            complete_horizontal_geometry: demand.requires_complete_horizontal_geometry(),
            materialized_text,
        }
    }

    /// Dispatch one host event, satisfying exact layout demands before retrying
    /// an input. The low-level [`Self::handle`] API still exposes demands to
    /// callers with an asynchronous layout scheduler. A retry uses the same
    /// input and captured clipboard, and publishes command effects only once.
    pub fn handle_with_layout(
        &mut self,
        view_id: ViewId,
        event: CoreEvent,
    ) -> Result<CoreOutcome, CoreError> {
        if !matches!(
            event,
            CoreEvent::Input(_) | CoreEvent::InputWithClipboard { .. }
        ) {
            return self.handle(view_id, event);
        }
        const MAX_LAYOUT_RETRIES: usize = 16;
        let mut layout_changed = false;
        let mut previous_request = None;
        for attempt in 0..=MAX_LAYOUT_RETRIES {
            let mut outcome = self.handle(view_id, event.clone())?;
            layout_changed |= outcome.layout_changed;
            outcome.layout_changed = layout_changed;
            let demand = match outcome.command.as_ref().map(|command| &command.status) {
                Some(CommandStatus::NeedsMoreLayout(
                    LayoutMotionError::OutsideMaterializedCoverage(demand),
                )) => demand.clone(),
                _ => return Ok(outcome),
            };
            if attempt == MAX_LAYOUT_RETRIES {
                return Err(LayoutMotionError::OutsideMaterializedCoverage(demand).into());
            }
            let Some(requested) = self.input_layout_region(view_id, &demand)? else {
                return Err(LayoutMotionError::OutsideMaterializedCoverage(demand).into());
            };
            let request_identity = self.input_layout_request_identity(view_id, &demand, requested.clone());
            if previous_request.as_ref() == Some(&request_identity) {
                // Preserve the typed failure if a provider cannot make more
                // geometry available; never publish the tentative command.
                return Err(LayoutMotionError::OutsideMaterializedCoverage(demand).into());
            }
            previous_request = Some(request_identity);
            self.satisfy_input_layout_demand(view_id, &demand, requested)?;
            layout_changed = true;
        }
        unreachable!("the final attempt returns its exact outcome")
    }

    pub(super) fn input_layout_region(
        &self,
        view_id: ViewId,
        demand: &LayoutDemand,
    ) -> Result<Option<Range<usize>>, CoreError> {
        const MIN_OVERSCAN_LINES: usize = 8;
        const MAX_EXTENSION_LINES: usize = 4_096;
        const MAX_INPUT_LAYOUT_LINES: usize = 4_096;
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let snapshot = view.layout.snapshot().ok_or(LayoutError::NoRows)?;
        if demand.horizontal_focus().is_some() {
            return Ok(Some(demand.requested_hard_lines()));
        }
        let top = view.layout.viewport_top();
        let bottom = top + view.layout.height();
        let flow = view.layout.paragraph_flow();
        let line_at = |offset| {
            self.document
                .projection()
                .presentation_line_at_offset(offset, flow)
                .ok_or(LayoutError::InvalidTextOffset(offset))
        };
        let cursor = line_at(view.commands.cursor())?;
        let mut needed = cursor..cursor + 1;
        let mut visible_rows = 0usize;
        let mut visible_lines: Option<Range<usize>> = None;
        for row in snapshot
            .rows
            .iter()
            .filter(|row| row.y < bottom && row.y + row.height() > top)
        {
            visible_rows += 1;
            let lines = visible_lines.get_or_insert(row.hard_line_index..row.hard_line_index + 1);
            lines.start = lines.start.min(row.hard_line_index);
            lines.end = lines.end.max(row.hard_line_index + 1);
            needed.start = needed.start.min(row.hard_line_index);
            needed.end = needed.end.max(row.hard_line_index + 1);
        }
        if let Some((anchor, active)) = view.commands.active_visual_block_endpoint_offsets() {
            for at in [anchor, active] {
                let line = line_at(at)?;
                needed.start = needed.start.min(line);
                needed.end = needed.end.max(line + 1);
            }
        }
        // Keep only geometry needed by the uncommitted command and nearby
        // overscan. Extending the entire old interval on every page would
        // eventually materialize the whole document during ordinary scrolling.
        // A missing visual row is not a missing paragraph. Estimate the next
        // band using the observed wrapping density, then let exact demands
        // retry if it falls short. Treating rows as hard lines shaped several
        // unnecessary screens and could outrun the entire pre-layout band.
        let additional_lines = demand.minimum_additional_visual_rows()
            .saturating_mul(visible_lines.as_ref().map_or(1, Range::len))
            .div_ceil(visible_rows.max(1));
        let extension = needed
            .len()
            .max(additional_lines)
            .clamp(MIN_OVERSCAN_LINES, MAX_EXTENSION_LINES);
        let materialized = demand.materialized_hard_lines();
        let count = snapshot.coverage.document_hard_line_count();
        let mut requested = needed.start.saturating_sub(MIN_OVERSCAN_LINES)
            ..needed.end.saturating_add(MIN_OVERSCAN_LINES).min(count);
        if matches!(
            demand.edge(),
            LayoutDemandEdge::Before | LayoutDemandEdge::Both
        ) {
            requested.start = requested
                .start
                .min(materialized.start.saturating_sub(extension));
        }
        if matches!(
            demand.edge(),
            LayoutDemandEdge::After | LayoutDemandEdge::Both
        ) {
            requested.end = requested
                .end
                .max(materialized.end.saturating_add(extension).min(count));
        }
        // A viewport scrolled far from its caret or an extreme counted motion
        // can require two distant bands. Report the structured layout error
        // instead of capturing their whole hull on a synchronous host turn.
        Ok((requested.len() <= MAX_INPUT_LAYOUT_LINES).then_some(requested))
    }

    pub(super) fn satisfy_input_layout_demand(
        &mut self,
        view_id: ViewId,
        demand: &LayoutDemand,
        requested: Range<usize>,
    ) -> Result<(), CoreError> {
        let (top, height) = self.validate_view_layout_demand(view_id, demand)?;
        let preserved_anchor = {
            let view = self.views.get_mut(&view_id).expect("validated view");
            update_viewport_anchor(&self.document, view);
            view.viewport_anchor
        };
        if self.satisfy_long_line_input_demand(view_id, demand, top, height, preserved_anchor)? {
            return Ok(());
        }
        let mut region = ViewportLayoutRegion::new(requested, top, height)?;
        if let Some((offset, x)) = demand.horizontal_focus() { region = region.with_horizontal_focus(offset, x); }
        if demand.requires_complete_horizontal_geometry() { region = region.with_complete_horizontal_geometry(); }
        let request = self.prepare_view_layout_job(
            view_id,
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )?;
        let candidate = {
            let view = self.views.get_mut(&view_id).expect("validated view");
            compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?
        };
        self.install_view_layout_job(view_id, candidate)?;
        let view = self.views.get_mut(&view_id).expect("validated view");
        view.viewport_anchor = preserved_anchor;
        restore_viewport_anchor(view)?;
        update_viewport_anchor(&self.document, view);
        Ok(())
    }

    /// Crossing a wrapped slice boundary needs more of the same hard line,
    /// rather than another request for its unchanged hard-line interval. Keep
    /// the old caret and visible rows while adding the demanded adjacent slice;
    /// publishing only the tail would move the viewport before input is retried.
    fn satisfy_long_line_input_demand(
        &mut self,
        view_id: ViewId,
        demand: &LayoutDemand,
        top: f32,
        height: f32,
        preserved_anchor: Option<ViewportTextAnchor>,
    ) -> Result<bool, CoreError> {
        if demand.horizontal_focus().is_some() {
            return Ok(false);
        }
        let (line, hard_line, required) = {
            let view = &self.views[&view_id];
            if !view.layout.wrap() { return Ok(false); }
            let snapshot = view.layout.snapshot().ok_or(LayoutError::NoRows)?;
            let before = matches!(demand.edge(), LayoutDemandEdge::Before | LayoutDemandEdge::Both)
                .then(|| snapshot.rows.first()).flatten()
                .filter(|row| row.text_range.start > row.hard_line_range.start)
                .and_then(|row| self.document.previous_grapheme_boundary(row.text_range.start)
                    .map(|focus| (row.hard_line_range.clone(), row.hard_line_index, focus)));
            let after = matches!(demand.edge(), LayoutDemandEdge::After | LayoutDemandEdge::Both)
                .then(|| snapshot.rows.last()).flatten()
                .filter(|row| row.text_range.end < row.hard_line_range.end)
                .and_then(|row| self.document.next_grapheme_boundary(row.text_range.end)
                    .map(|focus| (row.hard_line_range.clone(), row.hard_line_index, focus)));
            let Some((line, hard_line, focus)) = before.or(after) else { return Ok(false); };
            if line.len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES { return Ok(false); }
            let mut required = focus..focus;
            // Explicit scrolling may leave the old caret far outside this
            // snapshot. Paging chooses a new visible caret; retaining that
            // absent position would turn a local refill into a distant scan.
            // Caret-relative inputs already acquire their caret geometry.
            for offset in std::iter::once(view.commands.cursor())
                .filter(|offset| snapshot.coverage.contains_text_offset(*offset))
                .chain(preserved_anchor.map(|anchor| anchor.anchor.offset()))
                .chain(view.commands.active_visual_block_endpoint_offsets()
                    .into_iter().flat_map(|(anchor, active)| [anchor, active]))
                .chain(snapshot.rows.iter()
                    .filter(|row| row.y < top + height && row.y + row.height() > top)
                    .flat_map(|row| [row.text_range.start, row.text_range.end]))
            {
                required.start = required.start.min(offset);
                required.end = required.end.max(offset);
            }
            // The ordinary multi-line path owns requests whose live command
            // geometry also spans another hard line.
            if required.start < line.start || required.end > line.end { return Ok(false); }
            (line, hard_line, required)
        };
        let mut checkpoint = {
            let view = self.views.get_mut(&view_id).expect("validated view");
            let requirements = inspect_layout_provider(&view.engine);
            view.long_line_checkpoints.discard_stale(&self.document, &view.layout, requirements);
            view.long_line_checkpoints.before(line.clone(), required.start)
        };
        let relative_top = top - self.views[&view_id].layout.hard_line_prefix_height(hard_line)
            .map_err(LayoutError::from)?.height() as f32;
        let mut preceding: Option<crate::layout::RegionalLayoutSnapshot> = None;
        // Ordinary paging needs at most the current and adjacent slices. Keep
        // pathological counted motions bounded, as for multi-line input demands.
        const MAX_INPUT_LAYOUT_SLICES: usize = 4;
        for _ in 0..MAX_INPUT_LAYOUT_SLICES {
            let work_start = checkpoint.as_ref().map_or(line.start, LongLineLayoutCheckpoint::next_text_offset);
            let region = match checkpoint.take() {
                Some(checkpoint) => ViewportLayoutRegion::resume_long_line(checkpoint, top, height)?,
                None => ViewportLayoutRegion::new(hard_line..hard_line + 1, top, height)?,
            };
            let request = self.prepare_view_layout_job(view_id, LayoutJobPriority::NewlyExposedRows,
                LayoutJobRegion::Viewport(region), LayoutCancellationToken::new())?;
            let mut candidate = {
                let view = self.views.get_mut(&view_id).expect("validated view");
                compute_layout_job(&mut view.engine, &request, view.immediate_layout_context)?
            };
            let next = candidate.next_long_line_checkpoint().cloned();
            // A cache miss may discover earlier chunks before reaching the
            // required band. Keep only their latest checkpoint and slice;
            // joining and trimming them against a distant viewport is invalid.
            if let Some(previous) = preceding.as_ref()
                .filter(|previous| previous.lines()[0].text_coverage().end >= required.start)
            {
                let current = candidate.regional_snapshot();
                let mut retained = relative_top..relative_top + height;
                for row in previous.lines()[0].rows().iter().chain(current.lines()[0].rows())
                    .filter(|row| row.text_range.end >= required.start && row.text_range.start <= required.end)
                {
                    let bounds = row.reveal_bounds();
                    retained.start = retained.start.min(bounds.start);
                    retained.end = retained.end.max(bounds.end);
                }
                retained.start -= height;
                // An unfinished join must keep its contiguous end for the
                // next checkpoint. Trim the completed band only once its
                // required text is available, with one screen of overscan.
                retained.end = if current.lines()[0].text_coverage().end < required.end {
                    f32::MAX
                } else { retained.end + height };
                candidate.prepend_long_line_slice_in_range(previous, retained);
            }
            let coverage = candidate.regional_snapshot().lines()[0].text_coverage();
            if coverage.start <= required.start && coverage.end >= required.end {
                self.install_view_layout_job(view_id, candidate)?;
                let view = self.views.get_mut(&view_id).expect("validated view");
                view.viewport_anchor = preserved_anchor;
                restore_viewport_anchor(view)?;
                update_viewport_anchor(&self.document, view);
                return Ok(true);
            }
            let Some(next) = next.filter(|next| next.next_text_offset() > work_start) else { break; };
            self.views.get_mut(&view_id).expect("validated view")
                .long_line_checkpoints.insert(&self.document, next.clone());
            preceding = Some(candidate.regional_snapshot().clone());
            checkpoint = Some(next);
        }
        Err(LayoutMotionError::OutsideMaterializedCoverage(demand.clone()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::LineMode;
    use crate::layout::MockTextMeasurementProvider;

    fn key(key: Key) -> CoreEvent {
        CoreEvent::Input(InputEvent::Key(key))
    }

    fn long_document() -> Document {
        Document::new(
            (0..20_000)
                .map(|line| format!("line {line:05}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn assert_source(core: &Core<MockTextMeasurementProvider>, expected: &str) {
        let actual = core.document().source_bytes();
        assert_eq!(actual.len(), expected.len(), "source byte length");
        assert_eq!(actual.iter().zip(expected.bytes()).position(|(left, right)| *left != right), None,
            "first differing source byte");
    }

    #[test]
    fn long_line_page_refill_does_not_reacquire_an_absent_old_caret() {
        let mut core = Core::new(Document::new("word ".repeat(80_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
        // Model a scrollbar's distant materialized viewport while leaving the
        // editing caret at zero. Its next page crosses another chunk boundary.
        for _ in 0..4 {
            let checkpoint = core.views[&view].long_line_checkpoints.values().last().unwrap().clone();
            let top = checkpoint.completed_height();
            let request = core.prepare_view_layout_job(view, LayoutJobPriority::NewlyExposedRows,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::resume_long_line(checkpoint, top, 120.0).unwrap()),
                LayoutCancellationToken::new()).unwrap();
            let candidate = {
                let view = core.views.get_mut(&view).unwrap();
                compute_layout_job(&mut view.engine, &request, view.immediate_layout_context).unwrap()
            };
            core.install_view_layout_job(view, candidate).unwrap();
        }
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.contains_text_offset(0));
        let top = snapshot.rows[snapshot.rows.len() - 8].y;
        let state = core.views.get_mut(&view).unwrap();
        assert!(state.commands.set_cursor(&core.document, 0));
        state.layout.set_viewport_top(top).unwrap();
        update_viewport_anchor(&core.document, state);
        let calls = state.engine.provider().request_calls();
        let result = core.handle(view, key(Key::PageDown)).unwrap();
        let Some(CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(demand))) =
            result.command.as_ref().map(|command| &command.status) else { panic!("fixture must cross a slice boundary"); };
        let requested = core.input_layout_region(view, demand).unwrap().unwrap();
        core.satisfy_input_layout_demand(view, demand, requested).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert!((core.layout(view).unwrap().viewport_top() - top).abs() < 0.01);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.contains_text_offset(0));
        assert!(snapshot.rows.len() < 100, "refill retains only the local viewport and overscan");
        assert!(core.views[&view].engine.provider().request_calls() - calls < 64,
            "an absent old caret must not trigger shaping from the document start");
        let result = core.handle_with_layout(view, key(Key::PageDown)).unwrap();
        assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
        assert!(core.layout(view).unwrap().viewport_top() > top);
        assert!(core.command_state(view).unwrap().cursor() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES * 4);
        assert!(!core.document.history_status().can_undo);
    }

    #[test]
    fn host_pages_cross_wrapped_long_line_slices_without_moving_preflight_state() {
        for motion in [Key::PageDown, Key::PageUp] {
            let source = "word ".repeat(80_000);
            let mut core = Core::new(Document::new(source.clone()));
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
            for input in [Key::Char('q'), Key::Char('a'), Key::Char('4'), motion, Key::Char('q')] {
                let result = core.handle_with_layout(view, key(input)).unwrap();
                assert!(matches!(result.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending));
            }
            if motion == Key::PageUp {
                let checkpoint = core.views[&view].long_line_checkpoints.values().next().unwrap().clone();
                let top = checkpoint.completed_height();
                let request = core.prepare_view_layout_job(view, LayoutJobPriority::NewlyExposedRows,
                    LayoutJobRegion::Viewport(ViewportLayoutRegion::resume_long_line(checkpoint, top, 120.0).unwrap()),
                    LayoutCancellationToken::new()).unwrap();
                let candidate = {
                    let view = core.views.get_mut(&view).unwrap();
                    compute_layout_job(&mut view.engine, &request, view.immediate_layout_context).unwrap()
                };
                core.install_view_layout_job(view, candidate).unwrap();
            }
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let index = if motion == Key::PageDown { snapshot.rows.len() - 8 } else { 4 };
            let top = snapshot.rows[index].y;
            let cursor = snapshot.rows[index + 1].text_range.start;
            let state = core.views.get_mut(&view).unwrap();
            assert!(state.commands.set_cursor(&core.document, cursor));
            state.layout.set_viewport_top(top).unwrap();
            update_viewport_anchor(&core.document, state);
            let revision = core.document.revision();
            let mut crossed = false;
            for _ in 0..8 {
                let before_cursor = core.command_state(view).unwrap().cursor();
                let before_top = core.layout(view).unwrap().viewport_top();
                let result = core.handle(view, key(motion)).unwrap();
                if let Some(CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(demand))) =
                    result.command.as_ref().map(|command| &command.status)
                {
                    assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);
                    assert_eq!(core.layout(view).unwrap().viewport_top(), before_top);
                    let previous = core.layout(view).unwrap().snapshot().unwrap().coverage.clone();
                    let requested = core.input_layout_region(view, demand).unwrap().unwrap();
                    assert_eq!(requested, 0..1, "the next rows remain in the same hard line");
                    core.satisfy_input_layout_demand(view, demand, requested).unwrap();
                    assert_eq!(core.command_state(view).unwrap().cursor(), before_cursor);
                    assert!((core.layout(view).unwrap().viewport_top() - before_top).abs() < 0.01);
                    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                    assert_ne!(snapshot.coverage, previous);
                    assert!(snapshot.coverage.contains_text_offset(before_cursor));
                    let covered_bytes = match &snapshot.coverage {
                        LayoutCoverage::PartialHardLines { text_ranges, .. } => text_ranges.iter().map(Range::len).sum::<usize>(),
                        _ => panic!("long-line paging must remain partial"),
                    };
                    assert!(covered_bytes <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES * 4);
                    let result = core.handle_with_layout(view, key(motion)).unwrap();
                    assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
                    let after = core.layout(view).unwrap().viewport_top();
                    assert!(if motion == Key::PageDown { after > before_top } else { after < before_top });
                    // Counted recorded pages use the compound replay loop,
                    // whose progress check must also accept same-line refills.
                    for input in "3@a".chars() {
                        let result = core.handle_with_layout(view, key(Key::Char(input))).unwrap();
                        assert!(matches!(result.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending));
                    }
                    for _ in 0..12 {
                        let old_top = core.layout(view).unwrap().viewport_top();
                        let outcome = core.handle_with_layout(view, key(motion)).unwrap();
                        assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
                        let new_top = core.layout(view).unwrap().viewport_top();
                        assert!(if motion == Key::PageDown { new_top > old_top } else { new_top < old_top });
                    }
                    crossed = true;
                    break;
                }
            }
            assert!(crossed, "fixture must reach the {motion:?} slice boundary");
            assert_eq!(core.document.revision(), revision);
            assert_source(&core, &source);
            assert!(!core.document.history_status().can_undo);
        }
    }

    #[test]
    fn wrapped_giant_word_after_new_short_line_keeps_two_views_bounded() {
        let mut document = Document::new(format!("before {} after", "a".repeat(2 * 1024 * 1024)));
        document.insert(0, "x").unwrap();
        document.insert(1, "\n").unwrap();
        let expected_len = document.projection().text_tree().byte_len();
        let mut core = Core::new(document);
        let views = [core.add_view(MockTextMeasurementProvider::new(), 320.0, 100.0),
            core.add_view(MockTextMeasurementProvider::new(), 220.0, 100.0)];
        for view in views {
            let layout = core.layout(view).unwrap();
            assert!(layout.wrap());
            assert!(layout.last_error().is_none(), "{:?}", layout.last_error());
            let snapshot = layout.snapshot().unwrap();
            assert_eq!(snapshot.rows.len(), 4, "short line, prefix, overflow word, and tail");
            assert_eq!(snapshot.rows.last().unwrap().text_range.end, expected_len);
            assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 5_000));
        }
        let view = views[0];
        for wrap in [false, true] {
            core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 5_000));
        }
        core.handle(view, CoreEvent::SetLineMode(LineMode::PhysicalSource)).unwrap();
        for character in "j$".chars() {
            let result = core.handle_with_layout(view, key(Key::Char(character))).unwrap();
            assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
        }
        assert_eq!(core.command_state(view).unwrap().cursor(), expected_len - 1);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.iter().all(|row| row.clusters.len() < 5_000));
    }

    #[test]
    fn giant_unwrapped_rows_refill_scroll_cursor_and_vertical_demand_without_dense_geometry() {
        let line = "AV fi word ".repeat(12_000);
        let mut core = Core::new(Document::new(format!("{line}\n{line}\nshort")));
        let view = core.add_view(MockTextMeasurementProvider::new(), 320.0, 100.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        let second = core.add_view(MockTextMeasurementProvider::new(), 220.0, 100.0);
        core.handle(second, CoreEvent::SetWrap(false)).unwrap();
        for left in [100_000.0, 500_000.0, 0.0] {
            core.handle(view, CoreEvent::SetViewportOrigin { left, top: None }).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 6_000));
            assert!(snapshot.hit_test(crate::layout::LayoutPoint { x: left + 100.0, y: snapshot.rows[0].y + 1.0 }).is_ok());
            assert_eq!(core.layout(second).unwrap().viewport_left(), 0.0);
        }
        for motion in ["60000l", "gj", "gk", "$", "0"] {
            for character in motion.chars() {
                let outcome = core.handle_with_layout(view, key(Key::Char(character))).unwrap();
                assert!(matches!(outcome.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending));
            }
            let commands = core.command_state(view).unwrap();
            let position = commands.visual_position().unwrap_or(crate::command::layout_motion::VisualPosition { text_offset: commands.cursor(), affinity: commands.boundary_affinity() });
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot.logical_endpoint_geometry(position.text_offset, position.affinity).is_ok(), "{motion}: {}", position.text_offset);
            assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 6_000));
        }
        for character in "iX".chars() { core.handle_with_layout(view, key(Key::Char(character))).unwrap(); }
        core.handle_with_layout(view, key(Key::Escape)).unwrap();
        core.handle_with_layout(view, key(Key::Char('u'))).unwrap();
        assert_source(&core, &format!("{line}\n{line}\nshort"));
        assert!(core.layout(view).unwrap().last_error().is_none());
        assert!(core.layout(second).unwrap().last_error().is_none());
    }

    #[test]
    fn giant_unwrapped_visual_block_edits_refine_complete_rows_before_changing_source() {
        let line = "ab ".repeat(24_000);
        let source = format!("{line}\n{line}");
        let mut core = Core::new(Document::new(source.clone()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 320.0, 100.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        assert!(core.layout(view).unwrap().snapshot().unwrap().has_horizontal_materialization());
        core.handle_with_layout(view, key(Key::Ctrl('v'))).unwrap();
        for character in "6000ljd".chars() {
            let output = core.handle_with_layout(view, key(Key::Char(character))).unwrap();
            assert!(matches!(output.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending));
        }
        let suffix = &line[6001..];
        assert_source(&core, &format!("{suffix}\n{suffix}"));
        core.handle_with_layout(view, key(Key::Char('u'))).unwrap();
        assert_source(&core, &source);
        for character in "gg0".chars() { core.handle_with_layout(view, key(Key::Char(character))).unwrap(); }
        // Normal-mode block put must also refine the full destination geometry,
        // even though no active Visual Block state remains to request it.
        let output = core.handle_with_layout(view, key(Key::Char('P'))).unwrap();
        assert_eq!(output.command.unwrap().status, CommandStatus::Complete);
        let prefix = &line[..6001];
        assert_source(&core, &format!("{prefix}{line}\n{prefix}{line}"));
    }

    #[test]
    fn repeated_host_pages_slide_bounded_coverage_in_both_directions() {
        let mut core = Core::new(long_document());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 80.0);
        core.views
            .get_mut(&view)
            .unwrap()
            .layout
            .set_regional_cache_limits(crate::layout::RegionalLayoutCacheLimits {
                max_hard_lines: 256,
                ..Default::default()
            });
        let revision = core.document().revision();
        let source = core.document().source_bytes();
        for (motion, pages) in [(Key::PageDown, 240), (Key::PageUp, 160)] {
            for _ in 0..pages {
                let previous = core.command_state(view).unwrap().cursor();
                let outcome = core.handle_with_layout(view, key(motion)).unwrap();
                assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
                let current = core.command_state(view).unwrap().cursor();
                assert!(if motion == Key::PageDown {
                    current > previous
                } else {
                    current < previous
                });
                assert!(core.document().text_point(current).is_ok());
                let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                assert!(
                    snapshot.coverage.hard_lines().len() <= 48,
                    "{:?}",
                    snapshot.coverage
                );
            }
        }
        let view_state = core.views.get(&view).unwrap();
        assert!(
            view_state
                .layout
                .snapshot()
                .unwrap()
                .coverage
                .hard_lines()
                .start
                > 0
        );
        assert!(view_state.engine.provider().request_calls() < 2_000);
        assert!(
            view_state
                .layout
                .regional_cache_statistics()
                .hard_line_count()
                <= 256
        );
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        assert!(!core.document().history_status().can_undo);
    }

    #[test]
    fn wrapped_page_refill_estimates_hard_lines_from_visual_row_density() {
        let paragraph = "Words in a paragraph that occupies several wrapped visual rows. ".repeat(12);
        let document = Document::from_bytes_detect_encoding(format!("{paragraph}\n\n").repeat(10_000).into_bytes(), Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 1_000.0);
        let mut checked = false;
        for _ in 0..8 {
            let outcome = core.handle(view, key(Key::PageDown)).unwrap();
            if let Some(CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(demand))) = outcome.command.as_ref().map(|command| &command.status) {
                let requested = core.input_layout_region(view, demand).unwrap().unwrap();
                assert!(requested.end - demand.materialized_hard_lines().end <= 8,
                    "wrapped missing rows must not be treated as whole missing paragraphs");
                checked = true;
                break;
            }
        }
        assert!(checked);
        for _ in 0..10 { core.handle_with_layout(view, key(Key::PageDown)).unwrap(); }
        let calls = core.views[&view].engine.provider().request_calls();
        for _ in 0..5 { core.handle_with_layout(view, key(Key::PageUp)).unwrap(); }
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
        let configuration = core.layout(view).unwrap().snapshot().unwrap().configuration_generation;
        core.handle(view, CoreEvent::Resize { width: 600.0, height: 1_000.0 }).unwrap();
        core.handle_with_layout(view, key(Key::PageDown)).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.viewport_width, 600.0);
        assert_ne!(snapshot.configuration_generation, configuration);
        core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2));
        core.handle_with_layout(view, key(Key::PageDown)).unwrap();
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().metrics_generation, MetricsGeneration(2));
        assert!(core.views[&view].engine.provider().request_calls() > calls);
    }

    #[test]
    fn host_page_refill_uses_current_width_and_metrics_after_cache_invalidation() {
        let mut core = Core::new(long_document());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 80.0);
        for _ in 0..30 {
            core.handle_with_layout(view, key(Key::PageDown)).unwrap();
        }
        let old = core.layout(view).unwrap().snapshot().unwrap().clone();
        core.handle_with_layout(
            view,
            CoreEvent::Resize {
                width: 90.0,
                height: 112.0,
            },
        )
        .unwrap();
        core.views
            .get_mut(&view)
            .unwrap()
            .engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        for _ in 0..40 {
            let previous = core.command_state(view).unwrap().cursor();
            let outcome = core.handle_with_layout(view, key(Key::PageDown)).unwrap();
            assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
            assert!(core.command_state(view).unwrap().cursor() > previous);
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert_ne!(
                snapshot.configuration_generation,
                old.configuration_generation
            );
            assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
            assert_eq!(snapshot.viewport_width, 90.0);
            assert!(snapshot.coverage.hard_lines().len() <= 48);
        }
    }

    #[test]
    fn excessive_host_motion_reports_a_layout_error_without_publishing_input() {
        let mut core = Core::new(long_document());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 80.0);
        for character in "9999g".chars() {
            core.handle_with_layout(view, key(Key::Char(character)))
                .unwrap();
        }
        let revision = core.document().revision();
        let previous_snapshot = core.layout(view).unwrap().snapshot().unwrap().revision;
        let error = core
            .handle_with_layout(view, key(Key::Char('j')))
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::LayoutMotion(LayoutMotionError::OutsideMaterializedCoverage(_))
        ));
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.document().revision(), revision);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            previous_snapshot
        );
        assert!(!core.document().history_status().can_undo);
        // The uncommitted count/operator grammar is still intact at the lower
        // level, and its demand remains available to an asynchronous caller.
        let pending = core.handle(view, key(Key::Char('j'))).unwrap();
        assert!(matches!(
            pending.command.unwrap().status,
            CommandStatus::NeedsMoreLayout(_)
        ));
    }

    #[test]
    fn recorded_page_replay_uses_the_same_sliding_layout_window() {
        let mut core = Core::new(long_document());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 80.0);
        for input in [
            Key::Char('q'),
            Key::Char('a'),
            Key::PageDown,
            Key::Char('q'),
        ] {
            core.handle_with_layout(view, key(input)).unwrap();
        }
        for character in "99@a".chars() {
            let outcome = core
                .handle_with_layout(view, key(Key::Char(character)))
                .unwrap();
            assert!(matches!(
                outcome.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ));
        }
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.coverage.hard_lines().start > 0);
        assert!(snapshot.coverage.hard_lines().len() <= 48);
        assert!(core.command_state(view).unwrap().cursor() > 1_000);
        assert!(!core.document().history_status().can_undo);
    }
}
