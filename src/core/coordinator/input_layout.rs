//! Synchronous host input at an exact, bounded layout boundary.

use super::*;
use crate::command::layout_motion::LayoutDemandEdge;
use std::ops::Range;

impl<P: TextMeasurementProvider> Core<P> {
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
            if previous_request.as_ref() == Some(&requested) {
                // Preserve the typed failure if a provider cannot make more
                // geometry available; never publish the tentative command.
                return Err(LayoutMotionError::OutsideMaterializedCoverage(demand).into());
            }
            previous_request = Some(requested.clone());
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
        for row in snapshot
            .rows
            .iter()
            .filter(|row| row.y < bottom && row.y + row.height() > top)
        {
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
        let extension = needed
            .len()
            .max(demand.minimum_additional_visual_rows())
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
        let request = self.prepare_view_layout_job(
            view_id,
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(requested, top, height)?),
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
