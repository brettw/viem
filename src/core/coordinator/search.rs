//! Revision-bound search presentation. Matching yields between bounded input
//! batches; only the current viewport's matches occupy retained memory.
use super::*;
use crate::command::search_regex::{scanner::RegexScanner, CompiledRegex, RegexLimits};
use crate::command::{RevealedSearchMatch, SearchPresentation, SearchPresentationKey};
use std::ops::Range;

const SEARCH_BATCH_BYTES: usize = 8_192;
const SEARCH_OVERSCAN_BYTES: usize = 4_096;
const MAX_VISIBLE_MATCHES: usize = 16_384;

#[derive(Clone, PartialEq)]
struct ScanKey {
    document: DocumentId,
    revision: Revision,
    pattern: String,
    insensitive: bool,
    scope: Range<usize>,
}

struct KnownSearchMatch {
    key: ScanKey,
    range: Range<usize>,
}

struct PreviewViewport {
    document: DocumentId,
    revision: Revision,
    origin: usize,
    anchor: Option<ViewportTextAnchor>,
    left: f32,
    top: f32,
    layout: ViewLayout,
    style_revision: StyleSheetRevision,
}

#[derive(Default)]
pub(super) struct SearchViewState {
    key: Option<SearchPresentationKey>,
    presentation: Option<SearchPresentation>,
    scan_key: Option<ScanKey>,
    scanner: Option<RegexScanner>,
    region: Option<Range<usize>>,
    matches: Vec<Range<usize>>,
    /// The exact navigation result is independent of viewport scan progress.
    revealed_match: Option<KnownSearchMatch>,
    style_revision: Option<StyleSheetRevision>,
    metrics_overlay: bool,
    viewport: Option<PreviewViewport>,
    diagnostic: Option<String>,
}

impl<P: TextMeasurementProvider> View<P> {
    pub(super) fn search_preview_destination(&self, document: &Document) -> Option<usize> {
        self.search_preview_range(document).map(|range| range.start)
    }

    pub(super) fn search_preview_range(&self, document: &Document) -> Option<Range<usize>> {
        if self.search.key.as_ref() != Some(&self.commands.search_presentation_key(document)) {
            return None;
        }
        self.search.presentation.as_ref()?.incremental_match.as_ref()
            .map(|matched| matched.matched_range.clone())
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// True also for stale query state, so inactive panes discover shared
    /// pattern/option changes before exporting another presentation snapshot.
    pub fn search_work_pending(&self, view_id: ViewId) -> Result<bool, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let state = &view.search;
        let key = view.commands.search_presentation_key(&self.document);
        Ok(state.key.as_ref() != Some(&key)
            || state.style_revision != Some(self.document.projection().style_sheet().revision)
            || state
                .scanner
                .as_ref()
                .is_some_and(|scanner| !scanner.is_complete())
            || (state.scan_key.is_some()
                && !region_contains(state.region.as_ref(), &visible_range(view))))
    }

    /// One cooperative presentation turn. It never edits source, history,
    /// registers, the authoritative cursor, or the current Visual selection.
    pub fn poll_search(&mut self, view_id: ViewId) -> Result<bool, CoreError> {
        self.poll_search_with_match(view_id, None)
    }

    pub(super) fn poll_search_with_match(
        &mut self,
        view_id: ViewId,
        revealed_match: Option<RevealedSearchMatch>,
    ) -> Result<bool, CoreError> {
        let key = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?
            .commands
            .search_presentation_key(&self.document);
        let style_revision = self.document.projection().style_sheet().revision;
        let query_changed = self.views[&view_id].search.key.as_ref() != Some(&key);
        let style_changed = self.views[&view_id].search.style_revision != Some(style_revision);
        let old_preview = self.views[&view_id]
            .search
            .presentation
            .as_ref()
            .and_then(|presentation| presentation.incremental_match.as_ref())
            .map(|matched| matched.matched_range.clone());
        let mut restore = None;
        if query_changed {
            let presentation = self.views[&view_id]
                .commands
                .search_presentation(&self.document);
            let view = self.views.get_mut(&view_id).unwrap();
            if presentation.incremental_active && view.search.viewport.is_none() {
                update_viewport_anchor(&self.document, view);
                view.search.viewport = Some(PreviewViewport {
                    document: self.document.id(),
                    revision: self.document.revision(),
                    origin: view.commands.cursor(),
                    anchor: view.viewport_anchor,
                    left: view.layout.viewport_left(),
                    top: view.layout.viewport_top(),
                    layout: view.layout.clone(),
                    style_revision,
                });
            }
            let no_preview = presentation.incremental_match.is_none();
            if let Some(saved) = &view.search.viewport {
                // Invalid/empty patterns and Escape restore the pre-prompt
                // viewport. A successful Enter leaves the committed result.
                if no_preview
                    && view.commands.cursor() == saved.origin
                    && saved.document == self.document.id()
                    && saved.revision == self.document.revision()
                {
                    let reusable = saved.style_revision == style_revision
                        && saved.layout.same_presentation_configuration(&view.layout)
                        && current_snapshot_for_layout(
                            &self.document,
                            &saved.layout,
                            inspect_layout_provider(&view.engine),
                        )
                        .is_some();
                    restore = Some((
                        reusable.then(|| saved.layout.clone()),
                        saved.anchor,
                        saved.left,
                        saved.top,
                    ));
                }
            }
            if !presentation.incremental_active {
                view.search.viewport = None;
            }
            view.search.diagnostic = presentation.diagnostic.clone();
            view.search.key = Some(key);
            view.search.presentation = Some(presentation);
        }
        let preview = self.views[&view_id].search_preview_range(&self.document);
        let mut changed = false;
        if preview != old_preview && preview.is_some() {
            self.materialize_immediate_viewport(view_id, ImmediateLayoutIntent::RevealCaret)?;
            changed = true;
        }
        if let Some((layout, anchor, left, top)) = restore {
            let view = self.views.get_mut(&view_id).unwrap();
            cancel_active_layout_work(view);
            view.viewport_anchor = anchor;
            if let Some(layout) = layout {
                // Keep the exact pre-prompt geometry, including estimates
                // outside its viewport, when its dependencies are unchanged.
                view.layout = layout;
            } else {
                view.layout.set_viewport_top(top)?;
                view.layout.set_viewport_left(left)?;
                self.materialize_immediate_viewport(
                    view_id,
                    ImmediateLayoutIntent::PreserveViewport,
                )?;
            }
            changed = true;
        }
        let presentation = self.views[&view_id]
            .search
            .presentation
            .clone()
            .expect("query initialized");
        let scan_key = presentation
            .pattern
            .as_ref()
            .filter(|_| presentation.highlight_all)
            .and_then(|pattern| {
                presentation
                    .options
                    .case_insensitive(pattern)
                    .ok()
                    .map(|insensitive| ScanKey {
                        document: self.document.id(),
                        revision: self.document.revision(),
                        pattern: pattern.clone(),
                        insensitive,
                        scope: presentation
                            .search_range
                            .clone()
                            .unwrap_or(0..self.document.projection().text_tree().byte_len()),
                    })
            });
        {
            let state = &mut self.views.get_mut(&view_id).unwrap().search;
            if state.revealed_match.as_ref().is_some_and(|matched| {
                matched.key.document != self.document.id()
                    || matched.key.revision != self.document.revision()
            }) {
                state.revealed_match = None;
            }
            if let Some(matched) = revealed_match {
                if let Ok(insensitive) = matched.options.case_insensitive(&matched.pattern) {
                    // Keep the query that produced this range. Later steps of
                    // a compound command may have changed the active query.
                    state.revealed_match = Some(KnownSearchMatch {
                        key: ScanKey {
                            document: matched.document,
                            revision: matched.revision,
                            pattern: matched.pattern,
                            insensitive,
                            scope: 0..self.document.projection().text_tree().byte_len(),
                        },
                        range: matched.range,
                    });
                }
            }
        }
        let visible = visible_range(&self.views[&view_id]);
        let reset = self.views[&view_id].search.scan_key != scan_key
            || (scan_key.is_some()
                && !region_contains(self.views[&view_id].search.region.as_ref(), &visible));
        if reset {
            let snapshot = self.document.hard_line_snapshot();
            let start = visible.start.saturating_sub(SEARCH_OVERSCAN_BYTES);
            let end = visible
                .end
                .saturating_add(SEARCH_OVERSCAN_BYTES)
                .min(snapshot.text_length());
            let region = start..end;
            let state = &mut self.views.get_mut(&view_id).unwrap().search;
            state.matches.clear();
            state.scanner = None;
            state.scan_key = scan_key.clone();
            state.region = Some(region.clone());
            if let Some(scan_key) = &scan_key {
                let limits = RegexLimits::default();
                match CompiledRegex::compile(&scan_key.pattern, scan_key.insensitive, limits)
                    .and_then(|regex| {
                        RegexScanner::new(snapshot, regex, region.end, limits)
                            .within(scan_key.scope.clone())
                    }) {
                    Ok(scanner) => state.scanner = Some(scanner),
                    Err(error) => state.diagnostic = Some(error.to_string()),
                }
            }
        }
        {
            let state = &mut self.views.get_mut(&view_id).unwrap().search;
            if let Some(scanner) = state
                .scanner
                .as_mut()
                .filter(|scanner| !scanner.is_complete())
            {
                match scanner.advance(SEARCH_BATCH_BYTES) {
                    Ok(matches) => {
                        let region = state.region.as_ref().expect("scanner has viewport region");
                        for matched in matches {
                            let range = matched.range();
                            if range.start <= region.end && range.end >= region.start {
                                if state.matches.len() == MAX_VISIBLE_MATCHES {
                                    break;
                                }
                                state.matches.push(range);
                            }
                        }
                        if state.matches.len() >= MAX_VISIBLE_MATCHES {
                            state.scanner = None;
                            state.diagnostic =
                                Some("RegexResourceLimit: visible search matches".into());
                        }
                    }
                    Err(error) => state.diagnostic = Some(error.to_string()),
                }
            }
        }
        let snapshot = self.document.hard_line_snapshot();
        let state = &self.views[&view_id].search;
        let mut ranges: Vec<_> = state
            .matches
            .iter()
            .cloned()
            .chain(
                state.revealed_match.iter()
                    .filter(|matched| Some(&matched.key) == scan_key.as_ref())
                    .map(|matched| matched.range.clone()),
            )
            .chain(
                presentation
                    .incremental_match
                    .iter()
                    .map(|matched| matched.matched_range.clone()),
            )
            .filter_map(|range| display_range(&snapshot, range))
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| range.start <= previous.end)
            {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        let properties = self
            .document
            .projection()
            .style_sheet()
            .incremental_match_properties()
            .map_err(|error| {
                CoreError::Layout(LayoutError::DocumentStyle(
                    crate::layout::DocumentStyleError::Cascade(error),
                ))
            })?;
        let metrics_overlay = properties.font_families.is_some()
            || properties.size.is_some()
            || properties.weight.is_some()
            || properties.bold.is_some()
            || properties.slant.is_some()
            || properties.language.is_some()
            || properties.direction.is_some()
            || properties.open_type_features.is_some()
            || properties.letter_spacing.is_some();
        let view = self.views.get_mut(&view_id).unwrap();
        let changes_metrics = metrics_overlay || view.search.metrics_overlay;
        view.search.metrics_overlay = metrics_overlay;
        let had_matches = !view
            .layout
            .search_matches(self.document.id(), self.document.revision())
            .is_empty();
        let has_matches = !merged.is_empty();
        let ranges_changed = view.layout.set_search_matches(
            self.document.id(),
            self.document.revision(),
            merged,
            changes_metrics,
        );
        if style_changed && (had_matches || has_matches) && !ranges_changed {
            view.layout.invalidate_search_style(changes_metrics);
        }
        view.search.style_revision = Some(style_revision);
        if ranges_changed || (style_changed && (had_matches || has_matches)) {
            cancel_active_layout_work(view);
            // Newly revealed matches may acquire different metrics from the
            // highlight style. Apply visibility to that final geometry too.
            let intent = if preview != old_preview && preview.is_some() {
                ImmediateLayoutIntent::RevealCaret
            } else {
                ImmediateLayoutIntent::PreserveViewport
            };
            self.materialize_immediate_viewport(view_id, intent)?;
            self.rematerialize_active_composition(view_id, false)?;
            changed = true;
        }
        Ok(changed)
    }
}

fn region_contains(region: Option<&Range<usize>>, visible: &Range<usize>) -> bool {
    region.is_some_and(|region| region.start <= visible.start && region.end >= visible.end)
}

fn visible_range<P: TextMeasurementProvider>(view: &View<P>) -> Range<usize> {
    let Some(snapshot) = view.layout.snapshot() else {
        return view.commands.cursor()..view.commands.cursor();
    };
    let top = view.layout.viewport_top();
    let bottom = top + view.layout.height();
    let left = view.layout.viewport_left();
    let right = left + view.layout.width();
    let mut range = None::<Range<usize>>;
    for row in snapshot
        .rows
        .iter()
        .filter(|row| row.y <= bottom && row.y + row.height() >= top)
    {
        // Unwrapped rows describe the entire hard line even when only sparse
        // horizontal bands have geometry. Endpoint probe clusters outside the
        // viewport must not enlarge retained search coverage back to the line
        // start, or dense matches there can exhaust the visible-match limit.
        for cluster in &row.clusters {
            let mut cluster_left = cluster.x;
            let mut cluster_right = cluster.x + cluster.advance;
            for bounds in [cluster.typographic_bounds, cluster.ink_bounds] {
                if bounds.width > 0.0 {
                    cluster_left = cluster_left.min(bounds.x);
                    cluster_right = cluster_right.max(bounds.x + bounds.width);
                }
            }
            if cluster_right < left || cluster_left > right {
                continue;
            }
            if let Some(range) = &mut range {
                range.start = range.start.min(cluster.text_range.start);
                range.end = range.end.max(cluster.text_range.end);
            } else {
                range = Some(cluster.text_range.clone());
            }
        }
        if row.text_range.is_empty()
            && row
                .carets
                .iter()
                .any(|caret| left <= caret.x && caret.x <= right)
        {
            if let Some(range) = &mut range {
                range.start = range.start.min(row.text_range.start);
                range.end = range.end.max(row.text_range.end);
            } else {
                range = Some(row.text_range.clone());
            }
        }
    }
    range.unwrap_or_else(|| view.commands.cursor()..view.commands.cursor())
}

fn display_range(
    snapshot: &crate::document::HardLineSnapshot,
    range: Range<usize>,
) -> Option<Range<usize>> {
    let start = if snapshot.is_grapheme_boundary(range.start) {
        range.start
    } else {
        snapshot.previous_grapheme_boundary(range.start)?
    };
    let end = if range.end > start && snapshot.is_grapheme_boundary(range.end) {
        range.end
    } else {
        snapshot
            .next_grapheme_boundary(range.end)
            .unwrap_or(snapshot.text_length())
    };
    if start < end {
        Some(start..end)
    } else {
        snapshot
            .previous_grapheme_boundary(start)
            .map(|previous| previous..start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;

    type TestCore = Core<MockTextMeasurementProvider>;
    fn setup(text: &str) -> (TestCore, ViewId) {
        let mut core = Core::new(Document::new(text));
        let view = core.add_view(MockTextMeasurementProvider::default(), 400.0, 100.0);
        (core, view)
    }
    fn key(core: &mut TestCore, view: ViewId, key: Key) {
        core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    fn type_text(core: &mut TestCore, view: ViewId, text: &str) {
        for ch in text.chars() {
            key(core, view, Key::Char(ch));
        }
    }
    fn command(core: &mut TestCore, view: ViewId, text: &str) {
        type_text(core, view, text);
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        assert_eq!(
            outcome.command.unwrap().status,
            CommandStatus::Complete,
            "{text}"
        );
    }
    fn finish(core: &mut TestCore, view: ViewId) {
        for _ in 0..1_000 {
            if !core.search_work_pending(view).unwrap() {
                return;
            }
            core.poll_search(view).unwrap();
        }
        panic!("search work did not converge");
    }
    fn has_background(core: &TestCore, view: ViewId, at: usize) -> bool {
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .paint_runs
            .iter()
            .any(|run| run.text_range.contains(&at) && run.paint.background.is_some())
    }

    #[test]
    fn default_incremental_search_paints_all_matches_only_until_enter_or_escape() {
        for ending in [Key::Enter, Key::Escape] {
            let (mut core, view) = setup("foo bar foo");
            let source = core.document.source_bytes();
            let revision = core.document.revision();
            let history = core.document.history_status();
            type_text(&mut core, view, "/foo");
            finish(&mut core, view);
            let presentation = core
                .command_state(view)
                .unwrap()
                .search_presentation(&core.document);
            assert!(presentation.options.incsearch);
            assert!(!presentation.options.hlsearch);
            assert!(has_background(&core, view, 0));
            assert!(has_background(&core, view, 8));
            assert!(!has_background(&core, view, 4));
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
            key(&mut core, view, ending);
            finish(&mut core, view);
            assert!(!has_background(&core, view, 0));
            assert!(!has_background(&core, view, 8));
            assert!(
                !core
                    .command_state(view)
                    .unwrap()
                    .search_presentation(&core.document)
                    .options
                    .hlsearch
            );
            assert_eq!(core.document.source_bytes(), source);
            assert_eq!(core.document.revision(), revision);
            assert_eq!(core.document.history_status(), history);
        }
    }

    #[test]
    fn incremental_search_restores_saved_highlights_or_suppression_on_cancel() {
        for suppressed in [false, true] {
            for ending in [Key::Enter, Key::Escape] {
                let accepted = ending == Key::Enter;
                let (mut core, view) = setup("old new old new");
                command(&mut core, view, ":set hls");
                command(&mut core, view, "/old");
                if suppressed {
                    command(&mut core, view, ":noh");
                }
                type_text(&mut core, view, "/new");
                finish(&mut core, view);
                for at in [4, 12] {
                    assert!(has_background(&core, view, at));
                }
                for at in [0, 8] {
                    assert!(!has_background(&core, view, at));
                }
                key(&mut core, view, ending);
                finish(&mut core, view);
                for at in [0, 8] {
                    assert_eq!(has_background(&core, view, at), !accepted && !suppressed);
                }
                for at in [4, 12] {
                    assert_eq!(has_background(&core, view, at), accepted);
                }
                let presentation = core
                    .command_state(view)
                    .unwrap()
                    .search_presentation(&core.document);
                assert!(presentation.options.hlsearch);
                assert!(!presentation.incremental_active);
                assert_eq!(
                    presentation.pattern.as_deref(),
                    Some(if accepted { "new" } else { "old" })
                );
            }
        }
    }

    #[test]
    fn noincsearch_preserves_persistent_highlights_until_the_search_is_accepted() {
        for persistent in [false, true] {
            let (mut core, view) = setup("old new old new");
            command(
                &mut core,
                view,
                if persistent {
                    ":set hls nois"
                } else {
                    ":set nohls nois"
                },
            );
            command(&mut core, view, "/old");
            type_text(&mut core, view, "/new");
            finish(&mut core, view);
            let presentation = core
                .command_state(view)
                .unwrap()
                .search_presentation(&core.document);
            assert!(!presentation.incremental_active);
            assert!(presentation.incremental_match.is_none());
            assert_eq!(core.command_state(view).unwrap().cursor(), 8);
            for at in [0, 8] {
                assert_eq!(has_background(&core, view, at), persistent);
            }
            for at in [4, 12] {
                assert!(!has_background(&core, view, at));
            }
            key(&mut core, view, Key::Enter);
            finish(&mut core, view);
            for at in [0, 8] {
                assert!(!has_background(&core, view, at));
            }
            for at in [4, 12] {
                assert_eq!(has_background(&core, view, at), persistent);
            }
        }
    }

    #[test]
    fn highlight_state_is_shared_but_not_persisted_or_undoable() {
        let (mut core, first) = setup("foo bar foo");
        let second = core.add_view(MockTextMeasurementProvider::default(), 300.0, 100.0);
        let source = core.document.source_bytes();
        let revision = core.document.revision();
        let history = core.document.history_status();
        command(&mut core, first, ":set hls");
        command(&mut core, first, "/foo");
        finish(&mut core, first);
        assert!(has_background(&core, first, 0));
        assert!(has_background(&core, first, 8));
        assert!(!has_background(&core, first, 4));
        assert!(core.search_work_pending(second).unwrap());
        finish(&mut core, second);
        assert!(has_background(&core, second, 0));
        command(&mut core, first, ":noh");
        finish(&mut core, second);
        assert!(!has_background(&core, first, 0));
        assert!(!has_background(&core, second, 0));
        key(&mut core, first, Key::Char('n'));
        finish(&mut core, first);
        assert!(has_background(&core, first, 0));
        assert_eq!(core.document.source_bytes(), source);
        assert_eq!(core.document.revision(), revision);
        assert_eq!(core.document.history_status(), history);
    }

    #[test]
    fn incremental_preview_restores_scrolled_away_viewport_and_keeps_cursor_at_origin() {
        let text = format!("{}needle\n{}", "line\n".repeat(100), "line\n".repeat(100));
        let (mut core, view) = setup(&text);
        command(&mut core, view, ":set incsearch");
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: Some(180.0),
            },
        )
        .unwrap();
        let before = core.viewport_state(view).unwrap();
        let history = core.document.history_status();
        type_text(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert!(core.viewport_state(view).unwrap().top() > before.top());
        assert!(has_background(&core, view, 500));
        key(&mut core, view, Key::Escape);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.viewport_state(view).unwrap().top(), before.top());
        assert_eq!(core.document.history_status(), history);
        type_text(&mut core, view, "/needle");
        key(&mut core, view, Key::Enter);
        assert_eq!(core.command_state(view).unwrap().cursor(), 500);
        assert!(!has_background(&core, view, 500));
    }

    #[test]
    fn invalid_or_missing_incremental_pattern_restores_baseline_without_stale_paint() {
        let text = format!("{}needle", "line\n".repeat(40));
        let (mut core, view) = setup(&text);
        command(&mut core, view, ":set incsearch");
        let top = core.viewport_state(view).unwrap().top();
        type_text(&mut core, view, "/needle");
        assert!(has_background(&core, view, 200));
        type_text(&mut core, view, "[");
        assert_eq!(core.viewport_state(view).unwrap().top(), top);
        assert!(core
            .layout(view)
            .unwrap()
            .search_matches(core.document.id(), core.document.revision())
            .is_empty());
        key(&mut core, view, Key::Backspace);
        assert!(has_background(&core, view, 200));
        type_text(&mut core, view, "missing");
        assert_eq!(core.viewport_state(view).unwrap().top(), top);
    }

    #[test]
    fn large_prefix_scanning_yields_and_retires_matches_after_edit() {
        let text = format!("{}needle", "a\n".repeat(20_000));
        let (mut core, view) = setup(&text);
        command(&mut core, view, ":set hls");
        command(&mut core, view, "/needle");
        assert!(core.search_work_pending(view).unwrap());
        finish(&mut core, view);
        assert!(has_background(&core, view, 40_000));
        type_text(&mut core, view, "xiX");
        key(&mut core, view, Key::Escape);
        finish(&mut core, view);
        assert!(!has_background(&core, view, 40_000));
        assert!(!core
            .document
            .projection()
            .compatibility_text_is_materialized());
    }

    #[test]
    fn distant_committed_matches_paint_before_scanning_after_enter_repeat_and_scroll() {
        let gap = "a\n".repeat(40_000);
        let text = format!("{gap}needle\n{gap}needle\n{gap}needle");
        let first = gap.len();
        let second = first + 7 + gap.len();
        let third = second + 7 + gap.len();
        for incremental in [false, true] {
            let (mut core, view) = setup(&text);
            command(
                &mut core,
                view,
                if incremental { ":set hls is" } else { ":set hls nois" },
            );
            command(&mut core, view, "/needle");
            assert_eq!(core.command_state(view).unwrap().cursor(), first);
            assert!(core.search_work_pending(view).unwrap());
            assert!(has_background(&core, view, first));
            assert!(has_background(&core, view, first + 5));

            key(&mut core, view, Key::Char('n'));
            assert_eq!(core.command_state(view).unwrap().cursor(), second);
            assert!(core.search_work_pending(view).unwrap());
            assert!(has_background(&core, view, second));
            assert!(has_background(&core, view, second + 5));

            command(&mut core, view, "/");
            assert_eq!(core.command_state(view).unwrap().cursor(), third);
            for _ in 0..3 {
                assert!(core.search_work_pending(view).unwrap());
                assert!(has_background(&core, view, third));
                assert!(has_background(&core, view, third + 5));
                core.poll_search(view).unwrap();
            }
            let top = core.viewport_state(view).unwrap().top();
            core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(0.0) })
                .unwrap();
            core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(top) })
                .unwrap();
            assert!(core.search_work_pending(view).unwrap());
            assert!(has_background(&core, view, third));
            assert!(!core.document.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn immediate_match_respects_query_suppression_cancel_and_document_revision() {
        let prefix = "a\n".repeat(40_000);
        let first = prefix.len();
        let other = first + 7;
        let (mut core, view) = setup(&format!("{prefix}needle other"));
        command(&mut core, view, ":set hls");
        command(&mut core, view, "/needle");
        assert!(has_background(&core, view, first));

        type_text(&mut core, view, "/other");
        assert!(has_background(&core, view, other));
        assert!(!has_background(&core, view, first));
        key(&mut core, view, Key::Escape);
        assert!(core.search_work_pending(view).unwrap());
        assert!(has_background(&core, view, first));
        assert!(!has_background(&core, view, other));

        command(&mut core, view, ":noh");
        assert!(!has_background(&core, view, first));
        command(&mut core, view, "/");
        assert!(has_background(&core, view, first));
        command(&mut core, view, ":set nohls");
        assert!(!has_background(&core, view, first));
        command(&mut core, view, ":set hls");
        assert!(core.search_work_pending(view).unwrap());
        assert!(has_background(&core, view, first));

        command(&mut core, view, "/other");
        assert!(core.search_work_pending(view).unwrap());
        assert!(has_background(&core, view, other));
        assert!(!has_background(&core, view, first));
        type_text(&mut core, view, "/[");
        assert!(!has_background(&core, view, other));
        key(&mut core, view, Key::Escape);
        assert!(has_background(&core, view, other));
        key(&mut core, view, Key::Char('x'));
        assert!(!has_background(&core, view, other));
    }

    #[test]
    fn distant_incremental_match_paints_synchronously_without_persistent_highlights() {
        let prefix = "a\n".repeat(40_000);
        let at = prefix.len();
        let (mut core, view) = setup(&format!("{prefix}needle"));
        type_text(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert!(has_background(&core, view, at));
        for _ in 0..2 {
            assert!(core.search_work_pending(view).unwrap());
            core.poll_search(view).unwrap();
            assert!(has_background(&core, view, at));
        }
        key(&mut core, view, Key::Enter);
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
        assert!(!has_background(&core, view, at));
    }

    #[test]
    fn compound_search_results_keep_the_query_that_produced_their_highlight() {
        let prefix = "a\n".repeat(40_000);
        let at = prefix.len();
        for (suffix, highlighted) in [
            (":set hls<CR>", true),
            (":set noic<CR>", false),
            ("/missing<CR>", false),
        ] {
            let (mut core, view) = setup(&format!("{prefix}NEEDLE"));
            command(&mut core, view, ":set hls ic nois");
            command(&mut core, view, &format!(":nnoremap Q /needle<CR>{suffix}"));
            key(&mut core, view, Key::Char('Q'));
            assert_eq!(core.command_state(view).unwrap().cursor(), at);
            assert_eq!(has_background(&core, view, at), highlighted, "{suffix}");
        }
    }

    #[test]
    fn highlight_finds_multiline_match_beginning_before_the_visible_region() {
        let text = format!("start\n{}end", "middle\n".repeat(1_000));
        let (mut core, view) = setup(&text);
        command(&mut core, view, ":set hls");
        command(&mut core, view, "/(?s:start.*end)");
        key(&mut core, view, Key::Char('G'));
        finish(&mut core, view);
        assert!(has_background(&core, view, text.len() - 2));
        assert_eq!(core.views[&view].search.matches, vec![0..text.len()]);
    }

    #[test]
    fn horizontal_scrolling_highlights_dense_matches_beyond_the_retention_limit() {
        let text = "a ".repeat(50_000);
        let (mut core, view) = setup(&text);
        command(&mut core, view, ":set nowrap hls");
        command(&mut core, view, "/a");
        finish(&mut core, view);
        let initial_region = core.views[&view].search.region.clone().unwrap();
        assert!(initial_region.end < text.len() / 4);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.has_horizontal_materialization());
        let left = snapshot.rows[0].width * 0.75;
        core.handle(view, CoreEvent::SetViewportOrigin { left, top: None })
            .unwrap();
        finish(&mut core, view);
        let visible = visible_range(&core.views[&view]);
        assert!(visible.start > MAX_VISIBLE_MATCHES * 2);
        assert!(visible.len() < SEARCH_OVERSCAN_BYTES);
        let first_match = visible.start + visible.start % 2;
        assert!(has_background(&core, view, first_match));
        assert!(core.views[&view].search.matches.len() < MAX_VISIBLE_MATCHES);
        assert!(core.views[&view].search.diagnostic.is_none());
        assert!(!core
            .document
            .projection()
            .compatibility_text_is_materialized());

        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.0,
                top: None,
            },
        )
        .unwrap();
        finish(&mut core, view);
        assert!(has_background(&core, view, 0));
        assert!(core.views[&view].search.region.as_ref().unwrap().end < text.len() / 4);
    }

    #[test]
    fn cancelling_search_preserves_presentation_settings_changed_during_preview() {
        let (mut core, view) = setup("first\nneedle\nlast");
        command(&mut core, view, ":set incsearch");
        type_text(&mut core, view, "/needle");
        let insets = crate::layout::EdgeInsets {
            top: 10.0,
            left: 25.0,
            bottom: 15.0,
            right: 20.0,
        };
        core.set_view_insets(view, insets).unwrap();
        core.handle(view, CoreEvent::SetScale(1.5)).unwrap();
        let configured = core.layout(view).unwrap().clone();
        key(&mut core, view, Key::Escape);
        assert!(core
            .layout(view)
            .unwrap()
            .same_presentation_configuration(&configured));
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }
}
