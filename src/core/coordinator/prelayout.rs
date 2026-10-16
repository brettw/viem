//! Speculative layout policy; workers receive only immutable jobs.
//!
//! A document whose source is at most [`FULL_PRELAYOUT_SOURCE_BYTES`] is laid
//! out completely in the background and its geometry is retained, so ordinary
//! scrolling neither shapes on the input thread nor changes the scroll extent.
//! Larger documents use a window of the same size around the viewport and
//! discard the cached lines farthest from it.
use super::*;
use crate::layout::{HardLineLayoutRegion, LayoutJobRequest};
use std::ops::Range;

/// The single tuning point for speculative presentation work: documents up to
/// this many source bytes are pre-rendered completely (layout, table widths,
/// image dimensions, syntax highlighting) and retained; beyond it, the same
/// amount of text around the viewport is prepared and distant work discarded.
pub const FULL_PRELAYOUT_SOURCE_BYTES: usize = 100 * 1024;

/// Screens in each direction that are filled before the rest of the window.
const NEAR_PAGES: usize = 3;
// Keep the byte bound, but batch short/empty lines so a wrapped screen does
// not need several UI-dispatch round trips before its worker result is ready.
const MAX_JOB_LINES: usize = 128;
const MAX_JOB_BYTES: usize = 16 * 1024;
/// Beyond the retention threshold, a window may use this much of the cache
/// budget, leaving the remainder for the visible snapshot and its overscan.
const WINDOW_BUDGET_FRACTION: f64 = 0.75;
/// Discard lines outside the window before the budget would evict lines
/// inside it, and stop filling when the budget is nearly used.
const TRIM_FRACTION: f64 = 0.75;
const FILL_FRACTION: f64 = 0.9;

/// Whether `document` is small enough to pre-render and retain completely.
pub(super) fn retains_complete_presentation(document: &Document) -> bool {
    document.source_byte_len() <= FULL_PRELAYOUT_SOURCE_BYTES
}

/// The window of hard lines to prepare, and the scan order within it:
/// nearby lines in the direction of travel, nearby lines behind, then the
/// rest of the window ahead and behind.
struct PrelayoutPlan {
    window: Range<usize>,
    bands: [(Range<usize>, bool); 4],
}

impl<P: TextMeasurementProvider> Core<P> {
    pub(super) fn sync_prelayout_retention(&mut self, view_id: ViewId) {
        let retains = retains_complete_presentation(&self.document);
        if let Some(view) = self.views.get_mut(&view_id) {
            view.layout.set_regional_cache_retains_document(retains);
        }
    }

    fn prelayout_plan(&self, view_id: ViewId, forward: bool) -> Result<Option<PrelayoutPlan>, CoreError> {
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
        let line_count = snapshot.coverage.document_hard_line_count();
        let coverage = snapshot.coverage.hard_lines();
        // Observed hard lines per screen is more useful than estimated heights
        // for prose: an unmeasured paragraph can wrap into many visual rows.
        let near = (last.hard_line_index + 1 - first.hard_line_index).saturating_mul(NEAR_PAGES).max(1);
        let window = if view.layout.regional_cache_retains_document() {
            0..line_count
        } else {
            let projection = self.document.projection();
            let byte = |line: usize, end: bool| projection.presentation_line_range(line.min(line_count.saturating_sub(1)))
                .map_or(0, |range| if end { range.end } else { range.start });
            let line_at = |offset: usize| projection.presentation_line_at_offset(offset.min(projection.text_tree().byte_len()))
                .unwrap_or(line_count.saturating_sub(1));
            let (ahead, behind) = (FULL_PRELAYOUT_SOURCE_BYTES * 2 / 3, FULL_PRELAYOUT_SOURCE_BYTES / 3);
            let (after, before) = if forward { (ahead, behind) } else { (behind, ahead) };
            let mut start = line_at(byte(coverage.start, false).saturating_sub(before)).min(coverage.start);
            let mut end = (line_at(byte(coverage.end.saturating_sub(1), true).saturating_add(after)) + 1)
                .max(coverage.end).min(line_count);
            // Never plan more than the cache can retain beside the viewport.
            let budget = (limits.max_hard_lines as f64 * WINDOW_BUDGET_FRACTION) as usize;
            let (after, before) = if forward { (budget * 2 / 3, budget / 3) } else { (budget / 3, budget * 2 / 3) };
            start = start.max(coverage.start.saturating_sub(before));
            end = end.min(coverage.end.saturating_add(after));
            start..end
        };
        let clip = |range: Range<usize>| range.start.max(window.start)..range.end.min(window.end).max(range.start.max(window.start));
        let ahead_near = clip(coverage.end..coverage.end.saturating_add(near));
        let behind_near = clip(coverage.start.saturating_sub(near)..coverage.start);
        let ahead_rest = clip(ahead_near.end..window.end);
        let behind_rest = clip(window.start..behind_near.start);
        let bands = if forward {
            [(ahead_near, true), (behind_near, false), (ahead_rest, true), (behind_rest, false)]
        } else {
            [(behind_near, false), (ahead_near, true), (behind_rest, false), (ahead_rest, true)]
        };
        Ok(Some(PrelayoutPlan { window, bands }))
    }

    /// The nearest contiguous run of uncached lines in plan order, bounded by
    /// line count and captured text. A line larger than one job is prepared
    /// alone, as visible layout lays it out, up to the long-line slice size.
    /// Larger lines use resumable slices in visible layout. A retained
    /// document, which is no larger than the threshold, prepares them whole
    /// so its extent is exact; elsewhere they are left to demand layout.
    fn next_prelayout_lines(&self, view_id: ViewId, plan: &PrelayoutPlan) -> Option<Range<usize>> {
        let layout = &self.views[&view_id].layout;
        let projection = self.document.projection();
        let needed = |line: usize| !layout.regional_line_is_cached(line);
        for (band, forward) in &plan.bands {
            let mut remaining = band.clone();
            while let Some(line) = layout.first_uncached_regional_line(remaining.clone(), *forward) {
                let Some(range) = projection.presentation_line_range(line) else { return None; };
                if range.len() > crate::layout::MAX_LONG_LINE_LAYOUT_SLICE_BYTES {
                    if layout.regional_cache_retains_document() && projection.table_at(range.start).is_none() {
                        return Some(line..line + 1);
                    }
                    if *forward { remaining.start = line + 1; } else { remaining.end = line; }
                    continue;
                }
                if range.len() > MAX_JOB_BYTES { return Some(line..line + 1); }
                let mut lines = line..line + 1;
                let mut text = range;
                loop {
                    if lines.len() == MAX_JOB_LINES { break; }
                    let next = if *forward { lines.end } else if lines.start > band.start { lines.start - 1 } else { break };
                    if next >= band.end || !needed(next) { break; }
                    let Some(next_range) = projection.presentation_line_range(next) else { break };
                    let extent = text.start.min(next_range.start)..text.end.max(next_range.end);
                    if next_range.len() > MAX_JOB_BYTES || extent.len() > MAX_JOB_BYTES { break; }
                    text = extent;
                    if *forward { lines.end += 1; } else { lines.start -= 1; }
                }
                return Some(lines);
            }
        }
        None
    }

    /// Capture the nearest chunk of lines missing from the retained geometry.
    /// Call again after installation until None. No text shaping occurs here.
    pub fn prepare_view_prelayout(&mut self, view_id: ViewId, forward: bool) -> Result<Option<LayoutJobRequest>, CoreError> {
        // This cache-only path has no presentation-change result for its host.
        // Syntax publication can invalidate the visible layout, so leave it to
        // the explicit poll API that notifies frontends before they scroll.
        self.sync_prelayout_retention(view_id);
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        refresh_observed_metrics(view);
        let requirements = inspect_layout_provider(&view.engine);
        cancel_obsolete_layout_work(view, self.document.revision(), requirements);
        let line_count = self.document.projection().presentation_line_count();
        let complete = view.layout.regional_cache_retains_document()
            && view.layout.regional_cache_identity_matches(self.document.id(), self.document.revision(), line_count)
            && view.layout.regional_cached_line_count() == line_count;
        if complete { return Ok(None); }
        let Some(plan) = self.prelayout_plan(view_id, forward)? else { return Ok(None); };
        let view = self.views.get_mut(&view_id).expect("planned view");
        if !view.layout.regional_cache_retains_document() {
            view.layout.evict_regional_lines_outside(plan.window.clone(), TRIM_FRACTION);
            if view.layout.regional_cache_usage() >= FILL_FRACTION { return Ok(None); }
        }
        let Some(requested) = self.next_prelayout_lines(view_id, &plan) else { return Ok(None); };
        let job_id = self.allocate_layout_job_id()?;
        let cancellation = LayoutCancellationToken::new();
        let view = self.views.get_mut(&view_id).expect("view remains attached during preparation");
        let requested_lines = requested.clone();
        let mut request = crate::layout::prepare_cache_layout_job(&self.document, &mut view.layout,
            requirements, job_id, HardLineLayoutRegion::new(requested)?, cancellation.clone())?;
        request.capture_table_measurements(&view.engine);
        let next = ActiveLayoutWork {
            job_id, cancellation,
            document_revision: request.document_revision(),
            configuration_generation: request.configuration_generation(),
            geometry: Some((request.geometry_generation(), requested_lines)),
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
        // A retained document's window is all of it, independent of a visible
        // snapshot that a paint-only change may have retired.
        let window = if view.layout.regional_cache_retains_document() {
            0..self.document.projection().presentation_line_count()
        } else {
            let Some(plan) = self.prelayout_plan(view_id, forward)? else { return Ok(false); };
            plan.window
        };
        let LayoutJobRegion::HardLines(region) = candidate.requested_region() else { return Ok(false); };
        let region = region.range();
        // Scrolling can expose part of an in-flight chunk before it finishes.
        // Keep the chunk while it still belongs to the current window.
        if region.start < window.start || region.end > window.end { return Ok(false); }
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let requirements = inspect_layout_provider(&view.engine);
        let table_measurements = candidate.table_measurements();
        let installed = crate::layout::install_layout_job_cache(&mut view.layout, LayoutInstallTarget {
            document_id: self.document.id(), document_revision: self.document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        }, candidate).is_ok();
        if installed {
            // Exact widths discovered by the worker make a table's first
            // visible layout exact; entries the view has since refined win.
            view.engine.merge_table_measurements(table_measurements);
            // Exact heights above the visible text move it; keep it where it
            // is on screen and publish the refined extent.
            match view.layout.realign_snapshot_with_heights() {
                Ok(_) => {}
                Err(error) => view.layout.record_error(LayoutError::from(error)),
            }
        }
        Ok(installed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{compute_layout_job, LayoutEngine, MockTextMeasurementProvider, RegionalLayoutCacheLimits};
    use crate::document::Format;

    /// A core that ignores concurrent tests' changes to the process-wide Code
    /// sheet, which republish every Markdown presentation and reshape.
    fn pinned_core(document: Document) -> Core<MockTextMeasurementProvider> {
        let mut core = Core::new(document);
        core.syntax.markdown.code_sheet = Some(crate::document::code_style::snapshot());
        core
    }

    fn fixture() -> (Core<MockTextMeasurementProvider>, ViewId) {
        let text = (0..10_000).map(|i| format!("Paragraph {i}: **office** and words that wrap into visual rows.\n\n")).collect::<String>();
        let document = Document::from_bytes_detect_encoding(text.into_bytes(), Format::Markdown).unwrap();
        let mut core = pinned_core(document);
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

    /// Text and screen offset of every row intersecting the viewport. Cache
    /// installs may retranslate or extend the snapshot, never move these.
    fn screen(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> Vec<(std::ops::Range<usize>, i32, usize)> {
        let layout = core.layout(view).unwrap();
        let top = layout.viewport_top();
        layout.snapshot().unwrap().rows.iter()
            .filter(|row| row.y + row.height() > top && row.y < top + layout.height())
            .map(|row| (row.text_range.clone(), ((row.y - top) * 64.0).round() as i32, row.clusters.len()))
            .collect()
    }

    fn fill(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, forward: bool) -> usize {
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut jobs = 0;
        while let Some(request) = core.prepare_view_prelayout(view, forward).unwrap() {
            let LayoutJobRegion::HardLines(region) = request.region() else { panic!() };
            assert!(request.captured_text_len() <= MAX_JOB_BYTES
                || region.range().len() == 1 && request.captured_text_len() <= FULL_PRELAYOUT_SOURCE_BYTES);
            assert!(region.range().len() <= MAX_JOB_LINES);
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            assert!(core.install_view_prelayout(view, forward, candidate).unwrap());
            jobs += 1;
            assert!(jobs <= core.document().projection().presentation_line_count(), "prefetch must become idle");
        }
        jobs
    }

    /// Ordinary writing within the retention threshold: headings, wrapped
    /// paragraphs and lists.
    fn small_markdown() -> Document {
        let mut text = String::new();
        let mut section = 0;
        while text.len() < FULL_PRELAYOUT_SOURCE_BYTES - 2_000 {
            section += 1;
            text += &format!("## Section {section}\n\n");
            text += &format!("Paragraph of section {section}: **office** and *words* that wrap across several visual rows.\n\n");
            for item in 0..8 {
                text += &format!("- item {item} *here*\n");
            }
            text += "\n";
        }
        assert!(retains_complete_presentation(&Document::from_bytes_detect_encoding(text.clone().into_bytes(), Format::Markdown).unwrap()));
        Document::from_bytes_detect_encoding(text.into_bytes(), Format::Markdown).unwrap()
    }

    fn verified_everywhere(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> bool {
        core.views[&view].layout.regional_cached_line_count() == core.document().projection().presentation_line_count()
    }

    #[test]
    fn small_document_is_prelaid_completely_and_scrolls_without_shaping_or_extent_changes() {
        let mut core = pinned_core(small_markdown());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        assert!(core.layout(view).unwrap().regional_cache_retains_document());
        let lines = core.document().projection().presentation_line_count();
        assert!(lines > DEFAULT_CACHE_LINES, "the fixture exceeds the incremental cache budget");
        fill(&mut core, view, true);
        assert!(verified_everywhere(&core, view));
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none(), "complete work is idle");
        let statistics = core.layout(view).unwrap().regional_cache_statistics();
        assert_eq!(statistics.hard_line_count(), lines);
        assert_eq!(statistics.eviction_count(), 0);
        let extent = core.layout(view).unwrap().content_height();
        let inexact = (0..lines).filter(|line| !core.layout(view).unwrap().hard_line_range_height(*line..*line + 1).unwrap().is_exact()).collect::<Vec<_>>();
        assert!(extent.is_exact(), "inexact lines {:?} of {lines}", &inexact[..inexact.len().min(20)]);
        let maximum = core.layout(view).unwrap().maximum_viewport_top();
        let calls = core.views[&view].engine.provider().request_calls();
        let mut previous_top = -1.0;
        while core.layout(view).unwrap().viewport_top() > previous_top {
            previous_top = core.layout(view).unwrap().viewport_top();
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
            assert_eq!(core.layout(view).unwrap().maximum_viewport_top(), maximum, "the scroll extent never changes");
        }
        assert!(previous_top > 0.0);
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls, "paging never shapes on the input thread");
        assert_eq!(core.layout(view).unwrap().content_height(), extent);
        assert_eq!(core.layout(view).unwrap().regional_cache_statistics().eviction_count(), 0);
    }

    const DEFAULT_CACHE_LINES: usize = 2048;

    #[test]
    fn prelayout_above_the_viewport_keeps_visible_text_in_place() {
        let mut core = pinned_core(small_markdown());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        let middle = core.layout(view).unwrap().content_height().height() as f32 / 2.0;
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(middle) }).unwrap();
        let screen_position = |core: &Core<MockTextMeasurementProvider>| {
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let row = snapshot.rows.iter().find(|row| row.y + row.height() > layout.viewport_top()).unwrap();
            (row.text_range.clone(), row.y - layout.viewport_top(), snapshot.revision)
        };
        let (range, offset, revision) = screen_position(&core);
        fill(&mut core, view, true);
        assert!(verified_everywhere(&core, view));
        let (after_range, after_offset, after_revision) = screen_position(&core);
        assert_ne!(after_revision, revision, "exact prefix heights retranslate the visible snapshot");
        assert_eq!(after_range, range);
        assert!((after_offset - offset).abs() < 0.01, "visible text keeps its screen position");
        let maximum = core.layout(view).unwrap().maximum_viewport_top().unwrap();
        assert!((maximum - (core.layout(view).unwrap().content_height().height() as f32 - 160.0)).abs() < 1.0,
            "the published extent is exact");
        let top = core.layout(view).unwrap().viewport_top();
        let calls = core.views[&view].engine.provider().request_calls();
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(top + 10.0) }).unwrap();
        let (_, scrolled, _) = screen_position(&core);
        assert!((scrolled - (after_offset - 10.0)).abs() < 0.01 || scrolled > after_offset - 10.0 - 0.01);
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
    }

    #[test]
    fn large_document_window_is_bounded_and_discards_distant_lines() {
        let (mut core, view) = fixture();
        assert!(!core.layout(view).unwrap().regional_cache_retains_document());
        let limits = core.layout(view).unwrap().regional_cache_limits();
        fill(&mut core, view, true);
        let cached = core.layout(view).unwrap().regional_cache_statistics().hard_line_count();
        assert!(cached > 128, "the window extends well beyond three screens");
        assert!(cached as f64 <= limits.max_hard_lines as f64 * FILL_FRACTION + MAX_JOB_LINES as f64);
        assert!(core.layout(view).unwrap().regional_hard_line_layout(1).is_some());
        let distant = core.layout(view).unwrap().content_height().height() as f32 * 0.8;
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(distant) }).unwrap();
        fill(&mut core, view, true);
        let statistics = core.layout(view).unwrap().regional_cache_statistics();
        assert!(statistics.hard_line_count() <= limits.max_hard_lines);
        assert!(statistics.eviction_count() > 0);
        assert!(core.layout(view).unwrap().regional_hard_line_layout(1).is_none(), "distant lines are discarded first");
        let coverage = core.layout(view).unwrap().snapshot().unwrap().coverage.hard_lines();
        assert!(core.layout(view).unwrap().regional_hard_line_layout(coverage.end + 64).is_some());
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none(), "a full window is idle");
    }

    #[test]
    fn paint_only_changes_keep_retained_geometry() {
        let mut core = pinned_core(small_markdown());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
        let lines = core.document().projection().presentation_line_count();
        core.views.get_mut(&view).unwrap().layout.invalidate_syntax_presentation(false);
        core.handle(view, CoreEvent::Resize { width: 300.0, height: 200.0 }).unwrap();
        assert!(verified_everywhere(&core, view), "paint and height-only changes keep every line");
        assert!(core.layout(view).unwrap().content_height().is_exact());
        let calls = core.views[&view].engine.provider().request_calls();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none());
        assert_eq!(core.layout(view).unwrap().regional_cache_statistics().hard_line_count(), lines);
    }

    #[test]
    fn paint_only_changes_keep_in_flight_speculation_and_geometry_changes_reject_it() {
        for geometry in [false, true] {
            let mut core = pinned_core(small_markdown());
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
            let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            core.views.get_mut(&view).unwrap().layout.invalidate_syntax_presentation(geometry);
            let generation = core.layout(view).unwrap().configuration_generation();
            assert_eq!(core.install_view_prelayout(view, true, candidate).unwrap(), !geometry);
            if !geometry {
                let LayoutJobRegion::HardLines(region) = request.region() else { panic!() };
                assert!(core.layout(view).unwrap().regional_hard_line_layout(region.range().start).is_some());
                assert_eq!(core.layout(view).unwrap().configuration_generation(), generation);
            }
        }
    }

    fn tables_markdown() -> String {
        let mut text = String::new();
        for table in 0..40 {
            text += &format!("Paragraph before table {table}, with words that wrap across the narrow view.\n\n");
            text += "| Name | Value | Notes |\n| --- | ---: | --- |\n";
            for row in 0..12 {
                text += &format!("| row {row} | {} | note {} |\n", row * 37, "x".repeat(row % 5));
            }
            text += "\n";
        }
        text
    }

    #[test]
    fn retained_tables_reuse_exact_rows_without_shaping() {
        let document = Document::from_bytes_detect_encoding(tables_markdown().into_bytes(), Format::Markdown).unwrap();
        let mut core = pinned_core(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 200.0);
        fill(&mut core, view, true);
        assert!(verified_everywhere(&core, view));
        let calls = core.views[&view].engine.provider().request_calls();
        let mut previous_top = -1.0;
        let mut table_rows = 0;
        while core.layout(view).unwrap().viewport_top() > previous_top {
            previous_top = core.layout(view).unwrap().viewport_top();
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(!snapshot.has_provisional_table_widths());
            table_rows += snapshot.table_cells().len();
        }
        assert!(table_rows > 100, "paging crossed many table rows");
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls, "exact table rows reuse retained geometry");
    }

    #[test]
    fn widening_one_cell_relayouts_retained_rows_with_the_new_widths() {
        let document = Document::from_bytes_detect_encoding(tables_markdown().into_bytes(), Format::Markdown).unwrap();
        let mut core = pinned_core(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 200.0);
        fill(&mut core, view, true);
        type Cells = std::collections::BTreeMap<(u64, usize, usize), (u32, u32)>;
        let cells = |core: &Core<MockTextMeasurementProvider>, view: ViewId| -> Cells {
            core.layout(view).unwrap().snapshot().unwrap().table_cells().iter()
                .map(|cell| ((cell.table_id, cell.row, cell.column), (cell.rect.x.to_bits(), cell.rect.width.to_bits()))).collect()
        };
        let before = cells(&core, view);
        // Widen the first column from the last row of the first table.
        for key in ":14\r0iwider wider wider".chars() {
            let key = if key == '\r' { Key::Enter } else { Key::Char(key) };
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(0.0) }).unwrap();
        let edited = cells(&core, view);
        let mut fresh = pinned_core(Document::from_bytes_detect_encoding(core.document().source_bytes(), Format::Markdown).unwrap());
        let fresh_view = fresh.add_view(MockTextMeasurementProvider::new(), 400.0, 200.0);
        let expected = cells(&fresh, fresh_view);
        let shared = expected.keys().filter(|key| edited.contains_key(key)).collect::<Vec<_>>();
        assert!(shared.len() >= 6, "both layouts show the first table's rows");
        for key in shared {
            assert_eq!(edited[key], expected[key], "cell {key:?} matches a fresh layout");
        }
        let first = before.keys().next().unwrap();
        assert_ne!(edited[first].1, before[first].1, "the unedited first row was relaid with the wider column");
    }

    #[test]
    fn line_local_metric_changes_reject_only_overlapping_speculation() {
        for overlapping in [false, true] {
            let mut core = pinned_core(small_markdown());
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
            let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
            let LayoutJobRegion::HardLines(region) = request.region() else { panic!() };
            let lines = region.range();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            let changed = if overlapping { lines.start + 1..lines.start + 3 } else { lines.end + 500..lines.end + 510 };
            core.views.get_mut(&view).unwrap().layout.invalidate_syntax_presentation_lines(changed);
            assert_eq!(core.install_view_prelayout(view, true, candidate).unwrap(), !overlapping);
        }
    }

    #[test]
    fn edits_in_a_retained_document_relayout_only_nearby_lines() {
        let mut core = pinned_core(small_markdown());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
        let lines = core.document().projection().presentation_line_count();
        // Open and type on a list item in the middle of the document.
        for key in ":2000\roxyz".chars() {
            let key = if key == '\r' { Key::Enter } else { Key::Char(key) };
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        let after = core.document().projection().presentation_line_count();
        assert!(after >= lines);
        let verified = core.views[&view].layout.regional_cached_line_count();
        assert!(verified + 8 >= after, "only lines near the edit need layout: {verified} of {after}");
        assert!(core.views[&view].layout.regional_cache_is_consistent());
        fill(&mut core, view, true);
        assert!(verified_everywhere(&core, view));
    }

    #[test]
    fn initial_padding_is_applied_once_and_equal_padding_only_reflows_stale_metrics() {
        let document = Document::from_bytes_detect_encoding("A paragraph that wraps.\n\n".repeat(10_000).into_bytes(), Format::Markdown).unwrap();
        let mut core = pinned_core(document);
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
        let mut core = pinned_core(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 400.0);
        let first = core.layout(view).unwrap().snapshot().unwrap();
        assert!(first.coverage.vertical_range().unwrap().end >= 400.0);
        assert!(first.coverage.hard_lines().len() <= 4, "estimated one-row heights must not trigger pages of initial shaping");
        let original = screen(&core, view);
        fill(&mut core, view, true);
        assert_eq!(screen(&core, view), original);
        core.handle(view, CoreEvent::Resize { width: 600.0, height: 400.0 }).unwrap();
        assert_eq!(core.layout(view).unwrap().snapshot().unwrap().viewport_width, 600.0);
    }

    #[test]
    fn large_markdown_prelayout_is_bounded_cache_only_and_reused_by_paging() {
        let (mut core, view) = fixture();
        let visible = screen(&core, view);
        let top = core.layout(view).unwrap().viewport_top();
        let cursor = core.command_state(view).unwrap().cursor();
        let source = core.document().source_bytes();
        assert!(fill(&mut core, view, true) > 0);
        assert_eq!(screen(&core, view), visible);
        assert_eq!(core.layout(view).unwrap().viewport_top(), top);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        assert!(core.layout(view).unwrap().regional_cache_statistics().hard_line_count()
            <= core.layout(view).unwrap().regional_cache_limits().max_hard_lines);
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
        let visible = screen(&core, view);
        assert_ne!(current.revision, original.revision);
        assert!(!request.cancellation_token().is_cancelled());
        assert!(core.install_view_prelayout(view, true, candidate).unwrap());
        assert_eq!(screen(&core, view), visible);
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
    fn prelayout_rejects_resize_metrics_edit_and_abandoned_window() {
        for change in 0..5 {
            let (mut core, view) = fixture();
            let request = core.prepare_view_prelayout(view, true).unwrap().unwrap();
            let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
            let candidate = compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
            match change {
                0 => { core.handle(view, CoreEvent::Resize { width: 150.0, height: 120.0 }).unwrap(); }
                1 => core.views.get_mut(&view).unwrap().engine.provider_mut().set_metrics_generation(MetricsGeneration(2)),
                2 => { core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('x')))).unwrap(); }
                3 => {
                    // A distant jump leaves the chunk outside the new window.
                    let bottom = core.layout(view).unwrap().content_height().height() as f32 * 0.8;
                    core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(bottom) }).unwrap();
                }
                _ => (),
            }
            // Reversing direction keeps nearby work: it is still in the window.
            assert_eq!(core.install_view_prelayout(view, change != 4, candidate).unwrap(), change == 4);
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
        let mut core = pinned_core(Document::new(format!("{}{}\n{}", "short\n".repeat(24), "long ".repeat(MAX_JOB_BYTES), "short\n".repeat(4096))));
        assert!(!retains_complete_presentation(core.document()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
        // Beyond the long-line slice size, the paragraph is left to demand layout.
        assert!(!core.views[&view].layout.regional_line_is_cached(24));
    }

    #[test]
    fn retained_documents_prepare_giant_lines_whole_and_keep_their_extent() {
        let long = "long ".repeat(MAX_JOB_BYTES);
        assert!(long.len() > crate::layout::MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        let mut core = pinned_core(Document::new(format!("{}{long}\n{}", "short\n".repeat(24), "after\n".repeat(24))));
        assert!(retains_complete_presentation(core.document()));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
        assert!(verified_everywhere(&core, view), "the giant line is prepared");
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none(), "complete work is idle");
        let extent = core.layout(view).unwrap().content_height();
        assert!(extent.is_exact());
        // Visible layout pages through the giant line in bounded slices, which
        // agree with the prepared height: the extent never changes.
        let maximum = core.layout(view).unwrap().maximum_viewport_top();
        let mut previous_top = -1.0;
        while core.layout(view).unwrap().viewport_top() > previous_top {
            previous_top = core.layout(view).unwrap().viewport_top();
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::PageDown))).unwrap();
            assert_eq!(core.layout(view).unwrap().maximum_viewport_top(), maximum, "the scroll extent never changes");
        }
        assert_eq!(core.layout(view).unwrap().content_height(), extent);
        assert!(verified_everywhere(&core, view), "slices do not displace the prepared line");
    }

    #[test]
    fn prelayout_prepares_a_paragraph_larger_than_one_job_alone() {
        let long = "long ".repeat(MAX_JOB_BYTES * 2 / 5);
        assert!(long.len() > MAX_JOB_BYTES && long.len() <= crate::layout::MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        let mut core = pinned_core(Document::new(format!("{}{long}\n{}", "short\n".repeat(24), "after\n".repeat(24))));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        fill(&mut core, view, true);
        let layout = &core.views[&view].layout;
        let lines = core.document().projection().presentation_line_count();
        assert!(layout.regional_line_is_cached(24));
        assert_eq!(layout.regional_cached_line_count(), lines, "the document is completely prepared");
        assert!(core.prepare_view_prelayout(view, true).unwrap().is_none());
    }
}
