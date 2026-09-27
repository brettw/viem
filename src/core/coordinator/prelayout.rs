//! Bounded speculative layout policy; workers receive only immutable jobs.
use super::*;
use crate::layout::{HardLineLayoutRegion, LayoutJobRequest};
use std::ops::Range;

const AHEAD_PAGES: usize = 3;
const MAX_AHEAD_LINES: usize = 128;
// Keep the byte bound, but batch short/empty lines so a wrapped screen does
// not need several UI-dispatch round trips before its worker result is ready.
const MAX_JOB_LINES: usize = 32;
const MAX_JOB_BYTES: usize = 16 * 1024;

impl<P: TextMeasurementProvider> Core<P> {
    fn prelayout_band(&self, view_id: ViewId, forward: bool) -> Result<Option<Range<usize>>, CoreError> {
        let view = self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        if view.composition.is_some() { return Ok(None); }
        let limits = view.layout.regional_cache_limits();
        if limits.max_hard_lines == 0 || limits.max_visual_rows == 0 || limits.max_estimated_bytes == 0 {
            return Ok(None);
        }
        let requirements = inspect_layout_provider(&view.engine);
        let Some(snapshot) = current_snapshot_for_layout(&self.document, &view.layout, requirements) else {
            return Ok(None);
        };
        let top = view.layout.viewport_top();
        let bottom = top + view.layout.height();
        let mut rows = snapshot.rows.iter().filter(|row| row.y < bottom && row.y + row.height() > top);
        let Some(first) = rows.next() else { return Ok(None); };
        let last = rows.last().unwrap_or(first);
        // Observed hard lines per screen is more useful than estimated heights
        // for prose: an unmeasured paragraph can wrap into many visual rows.
        let count = (last.hard_line_index + 1 - first.hard_line_index)
            .saturating_mul(AHEAD_PAGES).min(MAX_AHEAD_LINES).min(limits.max_hard_lines / 2);
        let range = if forward {
            // Visible snapshots already include synchronous overscan. Extend
            // beyond that coverage so short/wrapped screens still get useful
            // speculative work rather than only rediscovering cached rows.
            let start = snapshot.coverage.hard_lines().end;
            start..start.saturating_add(count).min(snapshot.coverage.document_hard_line_count())
        } else {
            let end = snapshot.coverage.hard_lines().start;
            end.saturating_sub(count)..end
        };
        Ok((!range.is_empty()).then_some(range))
    }

    /// Capture the nearest uncached chunk within roughly three screens in the
    /// direction of travel. Call again after installation until None. No text
    /// shaping occurs here, and giant individual lines are left to demand layout.
    pub fn prepare_view_prelayout(&mut self, view_id: ViewId, forward: bool) -> Result<Option<LayoutJobRequest>, CoreError> {
        // This cache-only path has no presentation-change result for its host.
        // Syntax publication can invalidate the visible layout, so leave it to
        // the explicit poll API that notifies frontends before they scroll.
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        refresh_observed_metrics(view);
        let requirements = inspect_layout_provider(&view.engine);
        cancel_obsolete_layout_work(view, self.document.revision(), requirements);
        let Some(band) = self.prelayout_band(view_id, forward)? else { return Ok(None); };
        let view = &self.views[&view_id];
        let flow = view.layout.paragraph_flow();
        let lines: Box<dyn Iterator<Item = usize>> = if forward { Box::new(band) } else { Box::new(band.rev()) };
        let mut requested: Option<Range<usize>> = None;
        let mut text: Option<Range<usize>> = None;
        for line in lines {
            if view.layout.regional_hard_line_layout(line).is_some() {
                if requested.is_some() { break; }
                continue;
            }
            let range = self.document.projection().presentation_line_range(line, flow)
                .ok_or(LayoutJobError::InvalidDocumentLineIndex { hard_line: line })?;
            if range.len() > MAX_JOB_BYTES {
                if requested.is_some() { break; }
                continue;
            }
            let extent = text.as_ref().map_or_else(|| range.clone(), |previous| {
                previous.start.min(range.start)..previous.end.max(range.end)
            });
            if extent.len() > MAX_JOB_BYTES { break; }
            text = Some(extent);
            let region = requested.get_or_insert(line..line + 1);
            region.start = region.start.min(line);
            region.end = region.end.max(line + 1);
            if region.len() == MAX_JOB_LINES { break; }
        }
        let Some(requested) = requested else { return Ok(None); };
        let job_id = self.allocate_layout_job_id()?;
        let cancellation = LayoutCancellationToken::new();
        let view = self.views.get_mut(&view_id).expect("view remains attached during preparation");
        let request = crate::layout::prepare_cache_layout_job(&self.document, &mut view.layout,
            requirements, job_id, HardLineLayoutRegion::new(requested)?, cancellation.clone())?;
        let next = ActiveLayoutWork {
            job_id, cancellation,
            document_revision: request.document_revision(),
            configuration_generation: request.configuration_generation(),
            measurement_environment_id: request.measurement_environment_id(),
            metrics_generation: request.metrics_generation(),
        };
        if let Some(previous) = view.active_prelayout_work.replace(next) {
            previous.cancellation.cancel();
        }
        Ok(Some(request))
    }

    /// Background completion is cache-only. Abandoned scroll regions, cancelled
    /// work, and stale dependencies are discarded without changing presentation.
    pub fn install_view_prelayout(&mut self, view_id: ViewId, forward: bool, candidate: LayoutJobCandidate) -> Result<bool, CoreError> {
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        refresh_observed_metrics(view);
        if !view.active_prelayout_work.as_ref().is_some_and(|active| active.job_id == candidate.job_id()) {
            return Ok(false);
        }
        // Completion retires only this speculative job, even when the viewport
        // has moved too far to retain its result. Visible jobs have their own
        // ordering and must survive a cache fill in either completion order.
        view.active_prelayout_work = None;
        let Some(band) = self.prelayout_band(view_id, forward)? else { return Ok(false); };
        let LayoutJobRegion::HardLines(region) = candidate.requested_region() else { return Ok(false); };
        let region = region.range();
        let coverage = self.views[&view_id].layout.snapshot().expect("band requires a current snapshot").coverage.hard_lines();
        // Scrolling can expose part of an in-flight chunk before it finishes.
        // Keep the chunk while it still belongs to the current nearby band.
        let retained = if forward { coverage.start..band.end } else { band.start..coverage.end };
        if region.start < retained.start || region.end > retained.end { return Ok(false); }
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let requirements = inspect_layout_provider(&view.engine);
        let installed = crate::layout::install_layout_job_cache(&mut view.layout, LayoutInstallTarget {
            document_id: self.document.id(), document_revision: self.document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        }, candidate).is_ok();
        Ok(installed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{compute_layout_job, LayoutEngine, MockTextMeasurementProvider, RegionalLayoutCacheLimits};
    use crate::document::Format;

    fn fixture() -> (Core<MockTextMeasurementProvider>, ViewId) {
        let text = (0..10_000).map(|i| format!("Paragraph {i}: **office** and words that wrap into visual rows.\n\n")).collect::<String>();
        let document = Document::from_bytes_detect_encoding(text.into_bytes(), Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        (core, view)
    }

    #[test]
    fn live_theme_cancels_layout_and_prelayout_without_shaping_or_moving_large_document() {
        let (mut core, view) = fixture();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('v')))).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('l')))).unwrap();
        let selection = core.active_linear_selection_identity(view).unwrap();
        assert!(selection.is_some());
        let background = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let background_result = compute_layout_job(&mut worker, &background, LayoutExecutionContext::WorkerPool).unwrap();
        let foreground = core.prepare_view_layout_job(view, LayoutJobPriority::ChangedVisibleRows,
            background.region().clone(), LayoutCancellationToken::new()).unwrap();
        let foreground_result = compute_layout_job(&mut worker, &foreground, LayoutExecutionContext::WorkerPool).unwrap();
        let source = core.document().source_bytes();
        let history = core.document().history_status();
        let cursor = core.command_state(view).unwrap().cursor();
        let top = core.layout(view).unwrap().viewport_top();
        let generation = core.layout(view).unwrap().configuration_generation();
        let calls = core.views[&view].engine.provider().request_calls();
        core.replace_style_defaults(br#"{"version":1,"block_styles":[{"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","character":{"size":23},"block":{}}]}"#).unwrap();
        assert!(background.cancellation_token().is_cancelled());
        assert!(foreground.cancellation_token().is_cancelled());
        assert!(!core.install_view_prelayout(view, true, background_result).unwrap());
        assert!(core.install_view_layout_job(view, foreground_result).is_err());
        assert!(core.layout(view).unwrap().configuration_generation() > generation);
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        assert_eq!(core.active_linear_selection_identity(view).unwrap(), selection);
        assert_eq!(core.layout(view).unwrap().viewport_top(), top);
    }

    fn fill(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, forward: bool) -> usize {
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut jobs = 0;
        while let Some(request) = core.prepare_view_prelayout(view, forward).unwrap() {
            assert!(request.captured_text_len() <= MAX_JOB_BYTES);
            let LayoutJobRegion::HardLines(region) = request.region() else { panic!() };
            assert!(region.range().len() <= MAX_JOB_LINES);
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            assert!(core.install_view_prelayout(view, forward, candidate).unwrap());
            jobs += 1;
            assert!(jobs <= MAX_AHEAD_LINES, "prefetch must become idle");
        }
        jobs
    }

    #[test]
    fn initial_padding_is_applied_once_and_equal_padding_only_reflows_stale_metrics() {
        let document = Document::from_bytes_detect_encoding("A paragraph that wraps.\n\n".repeat(10_000).into_bytes(), Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let mut layout = ViewLayout::new(300.0, 160.0);
        let insets = crate::layout::EdgeInsets { top: 10.0, left: 17.0, bottom: 10.0, right: 23.0 };
        layout.set_insets(insets);
        let view = core.try_add_view_with_initial_layout(MockTextMeasurementProvider::new(), layout, LayoutExecutionContext::WorkerPool).unwrap();
        let original = core.layout(view).unwrap().snapshot().unwrap().clone();
        assert_eq!(original.content_insets.left, 17.0);
        assert_eq!(original.content_insets.right, 23.0);
        let calls = core.views[&view].engine.provider().request_calls();
        core.set_view_insets(view, insets).unwrap();
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().revision, original.revision);
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
        core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2));
        core.set_view_insets(view, insets).unwrap();
        assert_ne!(core.layout(view).unwrap().snapshot().unwrap().revision, original.revision);
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().metrics_generation, MetricsGeneration(2));
    }

    #[test]
    fn first_large_prose_view_shapes_only_the_exact_initial_screen() {
        let paragraph = "many words in a long paragraph that wraps across the viewport. ".repeat(20);
        let document = Document::from_bytes_detect_encoding(format!("{paragraph}\n\n").repeat(10_000).into_bytes(), Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 400.0);
        let first = core.layout(view).unwrap().snapshot().unwrap();
        assert!(first.coverage.vertical_range().unwrap().end >= 400.0);
        assert!(first.coverage.hard_lines().len() <= 4, "estimated one-row heights must not trigger pages of initial shaping");
        let original = first.clone();
        fill(&mut core, view, true);
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap(), &original);
        core.handle(view, CoreEvent::Resize { width: 600.0, height: 400.0 }).unwrap();
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().viewport_width, 600.0);
    }

    #[test]
    fn large_markdown_prelayout_is_bounded_cache_only_and_reused_by_paging() {
        let (mut core, view) = fixture();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap().clone();
        let top = core.layout(view).unwrap().viewport_top();
        let cursor = core.command_state(view).unwrap().cursor();
        let source = core.document().source_bytes();
        assert!(fill(&mut core, view, true) > 0);
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().revision, snapshot.revision);
        assert_eq!(core.layout(view).unwrap().viewport_top(), top);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        assert!(core.layout(view).unwrap().regional_cache_statistics().hard_line_count() <= MAX_AHEAD_LINES + snapshot.coverage.hard_lines().len());
        let before = core.views[&view].engine.provider().request_calls();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
        assert_eq!(core.views[&view].engine.provider().request_calls(), before, "a prepared page must not shape on the input thread");
        assert_eq!(core.document().source_bytes(), source);
        assert!(!core.document().history_status().can_undo);
        for _ in 0..50 {
            fill(&mut core, view, true);
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
        }
        assert!(core.layout(view).unwrap().regional_cache_statistics().hard_line_count() <= 2_048);
        fill(&mut core, view, false);
        let before = core.views[&view].engine.provider().request_calls();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageUp))).unwrap();
        assert_eq!(core.views[&view].engine.provider().request_calls(), before);
    }

    #[test]
    fn wheel_scroll_reuses_exact_geometry_and_preserves_pending_prelayout() {
        let (mut core, view) = fixture();
        // The first display intentionally covers only its screen. Exercise
        // scroll reuse after demand layout has installed ordinary overscan.
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(500.0) }).unwrap();
        let origin = core.layout(view).unwrap().viewport_top();
        let original = core.layout(view).unwrap().snapshot().unwrap().clone();
        let cache = core.layout(view).unwrap().regional_cache_statistics();
        let before = core.views[&view].engine.provider().request_calls();
        let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        for top in [16.0, 32.0, 48.0, 32.0, 16.0].map(|delta| origin + delta) {
            core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(top) }).unwrap();
            assert_eq!(core.layout(view).unwrap().snapshot().unwrap(), &original);
            assert_eq!(core.layout(view).unwrap().regional_cache_statistics(), cache);
            assert_eq!(core.views[&view].engine.provider().request_calls(), before);
            assert!(!request.cancellation_token().is_cancelled());
        }
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
        assert!(core.install_view_prelayout(view, true, candidate).unwrap());
        core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2));
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(origin + 16.0) }).unwrap();
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().metrics_generation, MetricsGeneration(2));
        assert_ne!(core.layout(view).unwrap().snapshot().unwrap().revision, original.revision);
        assert!(core.views[&view].engine.provider().request_calls() > before);
    }

    #[test]
    fn prelayout_survives_foreground_scroll_beyond_existing_coverage() {
        let (mut core, view) = fixture();
        let original = core.layout(view).unwrap().snapshot().unwrap().clone();
        let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let top = original.coverage.vertical_range().unwrap().end - core.layout(view).unwrap().height() + 1.0;
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(top) }).unwrap();
        let current = core.layout(view).unwrap().snapshot().unwrap().clone();
        assert_ne!(current.revision, original.revision);
        assert!(!request.cancellation_token().is_cancelled());
        assert!(core.install_view_prelayout(view, true, candidate).unwrap());
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap(), &current);
    }

    #[test]
    fn visible_and_cache_jobs_have_independent_completion_order() {
        for cache_first in [true, false] {
            let (mut core, view) = fixture();
            let region = core.layout(view).unwrap().snapshot().unwrap().coverage.hard_lines();
            let visible = core.prepare_view_layout_job(view, LayoutJobPriority::ChangedVisibleRows,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(region, 0.0, 160.0).unwrap()),
                LayoutCancellationToken::new()).unwrap();
            let background = core.prepare_view_prelayout(view, true).unwrap().unwrap();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let visible_result = compute_layout_job(&mut worker, &visible, LayoutExecutionContext::WorkerPool).unwrap();
            let background_result = compute_layout_job(&mut worker, &background, LayoutExecutionContext::WorkerPool).unwrap();
            assert!(!visible.cancellation_token().is_cancelled());
            assert!(!background.cancellation_token().is_cancelled());
            if cache_first {
                assert!(core.install_view_prelayout(view, true, background_result).unwrap());
                core.install_view_layout_job(view, visible_result).unwrap();
            } else {
                core.install_view_layout_job(view, visible_result).unwrap();
                assert!(core.install_view_prelayout(view, true, background_result).unwrap());
            }
            assert!(core.views[&view].active_layout_work.is_none());
            assert!(core.views[&view].active_prelayout_work.is_none());
        }
    }

    #[test]
    fn replacement_prelayout_cancels_only_its_predecessor_and_rejects_token_reuse() {
        let (mut core, view) = fixture();
        let first = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let stale = compute_layout_job(&mut worker, &first, LayoutExecutionContext::WorkerPool).unwrap();
        let second = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        assert!(first.cancellation_token().is_cancelled());
        assert!(!core.install_view_prelayout(view, true, stale).unwrap());
        assert_eq!(core.views[&view].active_prelayout_work.as_ref().unwrap().job_id, second.job_id());
        let duplicate = core.prepare_view_layout_job(view, LayoutJobPriority::Background,
            second.region().clone(), second.cancellation_token().clone()).unwrap_err();
        assert!(matches!(duplicate, CoreError::LayoutJob(LayoutJobError::CancellationTokenAlreadyActive { .. })));
        let result = compute_layout_job(&mut worker, &second, LayoutExecutionContext::WorkerPool).unwrap();
        assert!(core.install_view_prelayout(view, true, result).unwrap());
    }

    #[test]
    fn prelayout_rejects_resize_metrics_edit_and_abandoned_scroll_direction() {
        for change in 0..4 {
            let (mut core, view) = fixture();
            let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            match change {
                0 => { core.handle(view, CoreEvent::Resize { width: 150.0, height: 120.0 }).unwrap(); }
                1 => core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2)),
                2 => { core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('x')))).unwrap(); }
                _ => (),
            }
            assert!(!core.install_view_prelayout(view, change != 3, candidate).unwrap());
        }
    }

    #[test]
    fn prelayout_skips_giant_paragraphs_and_disabled_caches_and_cancels_on_close() {
        let (mut core, view) = fixture();
        core.views.get_mut(&view).unwrap().layout.set_regional_cache_limits(RegionalLayoutCacheLimits { max_estimated_bytes: 0, ..Default::default() });
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none());
        let (mut core, view) = fixture();
        let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
        core.remove_view(view).unwrap();
        assert!(request.cancellation_token().is_cancelled());
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        assert!(matches!(compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool), Err(LayoutJobError::Cancelled)));
        let mut core = Core::new(Document::new(format!("{}{}\nshort", "short\n".repeat(24), "long ".repeat(MAX_JOB_BYTES))));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
    }
}
