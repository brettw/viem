use super::*;
use crate::document::{
    code_style,
    syntax::{
        detection::{self, Detection, LanguageSelection},
        service::SyntaxService,
        SyntaxInputIdentity, SyntaxInputSnapshot,
    },
};
use std::sync::Arc;

pub(super) mod metrics;

/// Editing pause before a pre-rendered document is re-highlighted completely.
/// Visible text is always analyzed immediately.
const COMPLETE_SYNTAX_EDIT_PAUSE: std::time::Duration = std::time::Duration::from_millis(250);
/// Complete highlighting asks for one line-aligned stretch of about this
/// size at a time. A single whole-document query can exceed the provider's
/// time slice (a 28 KiB Rust file takes about 6 ms against a 4 ms slice),
/// and its repeated misses settle the request on fallback colours, which
/// then replace the exact colours already shown.
pub(super) const COMPLETE_SYNTAX_CHUNK_BYTES: usize = 8 * 1024;

#[cfg(test)]
mod filename_tests {
    use super::*;
    use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult};
    use crate::layout::MockTextMeasurementProvider;
    struct NoSyntax;
    impl SyntaxProvider for NoSyntax {
        fn analyze(&mut self, request: &SyntaxRequest, _: &std::sync::atomic::AtomicBool) -> SyntaxResult {
            SyntaxResult::missing(request, "fixture")
        }
    }
    #[test]
    fn code_filename_reaches_syntax_on_open_rename_and_provider_replacement() {
        let document = Document::from_bytes(b"token".to_vec(), crate::document::Encoding::Utf8, Format::Code).unwrap();
        let mut core = Core::<MockTextMeasurementProvider>::new(document);
        core.initialize_code_detection("first.vim", false).unwrap();
        assert_eq!(core.syntax.code.service.configuration.filename.as_deref(), Some("first.vim"));
        let old = core.syntax.code.service.configuration.clone();
        core.redetect_code_language(Some("second.vim"));
        assert_eq!(core.syntax.code.service.configuration.filename.as_deref(), Some("second.vim"));
        assert_eq!(core.syntax.code.service.configuration.language, old.language);
        assert_ne!(core.syntax.code.service.configuration.generation, old.generation);
        core.set_syntax_provider_factory(Arc::new(|| Box::new(NoSyntax)));
        assert_eq!(core.syntax.code.service.configuration.filename.as_deref(), Some("second.vim"));
    }
}

/// Code highlighting started for the projection a Markdown Preview switch
/// will install, keyed to that exact input identity. Any edit discards it.
pub(super) struct PreparedCodeSyntax {
    /// The Preview revision the switch was prepared from.
    pub(super) source_revision: u64,
    pub(super) identity: SyntaxInputIdentity,
    pub(super) service: SyntaxService,
}

/// A buffer's language selection and whether its mode is automatic, as the
/// language menu shows them. Reloads copy it whole into the replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeLanguagePolicy {
    selection: LanguageSelection,
    automatic_mode: bool,
}

/// The buffer's one Code style layer: its syntax service, publication state
/// and progress. Every Code view of the buffer, including a diff's live
/// side, shares it; nothing outside the styling path reads its fields.
pub(super) struct CodeLayer {
    pub(super) service: SyntaxService,
    pub(super) published: Option<(SyntaxInputIdentity, u64)>,
    /// When the analyzed input last changed. Speculative whole-document
    /// analysis waits for editing to pause.
    input_changed: Option<(SyntaxInputIdentity, std::time::Instant)>,
    /// How far complete highlighting of the current input has progressed.
    /// Finished chunks are never revisited, so evicted coverage entries
    /// cannot make the pass repeat itself.
    complete_progress: Option<(SyntaxInputIdentity, usize)>,
    referenced_names: Vec<String>,
    implicit_limited: bool,
}

impl Default for CodeLayer {
    fn default() -> Self {
        Self {
            service: Default::default(),
            published: None,
            input_changed: None,
            complete_progress: None,
            referenced_names: Vec::new(),
            implicit_limited: false,
        }
    }
}

/// Buffer syntax policy (language selection, detection, filename) and the
/// style layers it configures.
pub(super) struct CoreSyntax {
    pub(super) code: CodeLayer,
    pub(super) prepared_code: Option<PreparedCodeSyntax>,
    pub(super) markdown: super::markdown_code::MarkdownCodeSyntax,
    pub(super) selection: LanguageSelection,
    pub(super) automatic_detection: Option<Detection>,
    pub(super) automatic_mode: bool,
    detection: Option<Detection>,
    pub(super) detected_for_code: bool,
    filename_associations: Vec<detection::FilenameAssociation>,
    detection_profile: Arc<detection::DetectionProfile>,
    filename: String,
    /// When the document revision last changed, for preparing the other
    /// Markdown view only after editing pauses.
    pub(super) view_format_changed: Option<((crate::document::DocumentId, crate::document::Revision), std::time::Instant)>,
    sheet: Arc<crate::document::StyleSheet>,
    /// Whether `sheet` has any automatic style with non-default metrics,
    /// memoized by its revision.
    metric_styles: Option<(crate::document::StyleSheetRevision, bool)>,
}
impl Default for CoreSyntax {
    fn default() -> Self {
        Self {
            code: Default::default(),
            prepared_code: None,
            markdown: Default::default(),
            selection: LanguageSelection::Automatic,
            automatic_detection: None,
            automatic_mode: true,
            detection: None,
            detected_for_code: false,
            filename_associations: Vec::new(),
            detection_profile: detection::bundled_profile(),
            filename: String::new(),
            view_format_changed: None,
            sheet: code_style::snapshot(),
            metric_styles: None,
        }
    }
}
impl<P: TextMeasurementProvider> Core<P> {
    /// Capture a nonmutating whole-document export. The returned operation must
    /// render after releasing the coordinator lease; it uses no layout provider.
    pub fn prepare_html_export(&self, view: ViewId) -> Result<crate::document::HtmlExport, CoreError> {
        if !self.views.contains_key(&view) { return Err(CoreError::UnknownView(view)); }
        self.focus_view(view);
        let export = self.document.prepare_html_export()
            .with_tabstop(self.views[&view].commands.indentation_options().tabstop);
        if !self.document.format().is_code() { return Ok(export); }
        let input = self.syntax_input();
        let mut service = self.syntax.code.service.for_export();
        if !self.syntax.detected_for_code {
            let detection = detection::detect_with_profile(&input, &self.syntax.filename,
                &self.syntax.selection, &self.syntax.filename_associations, &self.syntax.detection_profile);
            service.set_language(detection.language);
        }
        Ok(export.with_syntax(service, input))
    }

    pub(super) fn syntax_input(&self) -> SyntaxInputSnapshot {
        SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: self.document.id().0,
                revision: self.document.revision().0,
                generation: 1,
            },
            self.document.projection().text_tree().clone(),
        )
    }
    pub(super) fn detect_code_language(&self, selection: &LanguageSelection) -> Detection {
        detection::detect_sampled_lines(&self.document.code_detection_samples(), &self.syntax.filename,
            selection, &self.syntax.filename_associations, &self.syntax.detection_profile)
    }

    pub fn initialize_code_detection(
        &mut self,
        filename: &str,
        allow_auto_code: bool,
    ) -> Result<(), crate::document::StyleDefaultsError> {
        if !self.views.is_empty() {
            return Err(crate::document::StyleDefaultsError::NotPristine);
        }
        self.syntax.filename = filename.to_owned();
        self.syntax.code.service.set_filename((!filename.is_empty()).then(|| filename.to_owned()));
        let detection = self.detect_code_language(&self.syntax.selection);
        if allow_auto_code
            && detection.language.is_some()
            && self.document.buffer_format() == Format::PlainText
        {
            self.document.initialize_code_format()?;
        }
        self.syntax.code.service.set_language(detection.language.clone());
        self.syntax.automatic_detection = Some(detection.clone());
        self.syntax.detection = Some(detection);
        self.syntax.detected_for_code = self.code_layer_wanted();
        self.publish_reflow_language();
        self.adopt_markdown_language();
        Ok(())
    }
    /// Code whose language is Markdown is a Markdown buffer showing its
    /// source, as choosing the Markdown language makes it: any view may then
    /// show Preview, and the Markdown theme is kept for that view.
    fn adopt_markdown_language(&mut self) {
        if self.document.buffer_format() == Format::Code {
            self.document.set_markdown_buffer(self.is_markdown_source());
        }
    }
    /// The canonical language name selecting reflow's comment profile.
    pub(super) fn reflow_language(&self) -> Option<String> {
        self.syntax
            .detection
            .as_ref()
            .and_then(|detection| detection.language.clone())
    }
    pub fn set_code_language(&mut self, selection: LanguageSelection) {
        self.syntax.automatic_mode = selection == LanguageSelection::Automatic;
        self.syntax.selection = selection;
        self.refresh_code_detection();
    }
    /// The buffer's language choice, which survives reading its file again.
    pub fn code_language_policy(&self) -> CodeLanguagePolicy {
        CodeLanguagePolicy { selection: self.syntax.selection.clone(), automatic_mode: self.syntax.automatic_mode }
    }
    /// `initialize_code_detection` for the replacement of a reloaded buffer,
    /// created in that buffer's format. Its language choice carries over. Text
    /// that wasn't chosen by hand becomes Code when the new source declares a
    /// language, as opening it would; every other format stays.
    pub fn initialize_reloaded_code_detection(
        &mut self,
        filename: &str,
        policy: CodeLanguagePolicy,
    ) -> Result<(), crate::document::StyleDefaultsError> {
        self.initialize_code_detection(filename, policy.automatic_mode)?;
        if policy.selection != self.syntax.selection {
            self.set_code_language(policy.selection);
        }
        self.syntax.automatic_mode = policy.automatic_mode;
        self.adopt_markdown_language();
        Ok(())
    }
    fn refresh_code_detection(&mut self) {
        self.syntax.automatic_detection = Some(self.detect_code_language(&LanguageSelection::Automatic));
        self.syntax.code.service.set_filename((!self.syntax.filename.is_empty()).then(|| self.syntax.filename.clone()));
        let detection = self.detect_code_language(&self.syntax.selection);
        self.syntax.code.service.set_language(detection.language.clone());
        self.syntax.detection = Some(detection);
        self.syntax.code.published = None;
        self.publish_reflow_language();
    }
    /// Explicit policy changes/redetection are the only post-load detector
    /// entry points; ordinary edits and viewport requests do not call these.
    pub fn set_code_filename_associations(
        &mut self,
        associations: Vec<detection::FilenameAssociation>,
    ) -> Result<(), String> {
        detection::validate_filename_associations(&associations)?;
        self.syntax.filename_associations = associations;
        if self.syntax.detection.is_some() {
            self.refresh_code_detection();
        }
        Ok(())
    }
    /// Install a validated portable detection policy. Explicit configuration
    /// changes redetect; ordinary editing keeps the load-time selection.
    pub fn set_code_detection_profile(&mut self, profile: Arc<detection::DetectionProfile>) {
        self.syntax.detection_profile = profile;
        if self.syntax.detection.is_some() {
            self.refresh_code_detection();
        }
    }
    pub fn redetect_code_language(&mut self, filename: Option<&str>) {
        if let Some(filename) = filename {
            self.syntax.filename = filename.to_owned();
        }
        self.refresh_code_detection();
        self.syntax.detected_for_code = self.code_layer_wanted();
    }
    /// The buffer's file name as the frontend reported it (for `%` and
    /// autocommand patterns); empty for an unnamed buffer.
    pub(super) fn buffer_filename(&self) -> &str {
        &self.syntax.filename
    }
    pub fn code_language_detection(&self) -> Option<&Detection> {
        self.syntax.detection.as_ref()
    }
    pub fn configure_syntax(&mut self, directory: &str) {
        self.syntax.prepared_code = None;
        self.syntax.code.service.set_vim_directory(directory.to_owned());
        self.syntax.code.published = None;
        self.syntax.markdown = Default::default();
    }

    /// Replace the platform's syntax integration without changing document
    /// commands or rendering. Factories and provider destruction run on syntax
    /// workers; configuration and language selection remain portable core state.
    pub fn set_syntax_provider_factory(
        &mut self,
        factory: crate::document::syntax::service::SyntaxProviderFactory,
    ) {
        let mut service = SyntaxService::with_factory(factory);
        service.set_language(self.syntax.code.service.configuration.language.clone());
        service.set_vim_directory(self.syntax.code.service.configuration.vim_directory.clone());
        service.set_filename(self.syntax.code.service.configuration.filename.clone());
        self.syntax.code.service = service;
        self.syntax.prepared_code = None;
        self.syntax.markdown = Default::default();
        self.syntax.code.published = None;
    }
    pub fn syntax_diagnostics(&self) -> String {
        let mut diagnostics = self.syntax.code.service.diagnostics();
        if self.syntax.code.implicit_limited {
            if !diagnostics.is_empty() {
                diagnostics.push('\n');
            }
            diagnostics.push_str(&format!(
                "More than {} implicit Code styles; further syntax names use their nearest defined ancestor",
                code_style::MAX_IMPLICIT_DEFINITIONS
            ));
        }
        diagnostics
    }
    pub fn syntax_statistics(&self) -> crate::document::syntax::service::SyntaxServiceStatistics {
        self.syntax.code.service.statistics
    }

    /// Names in the accepted, bounded syntax cache, including unresolved names.
    /// Menu inspection never runs a provider or scans unhighlighted document text.
    pub fn syntax_style_names(&self) -> &[String] {
        let current = (self.syntax_input().identity(), self.syntax.code.service.configuration.generation);
        if self.code_layer_wanted() && self.syntax.code.published == Some(current) {
            &self.syntax.code.referenced_names
        } else {
            &[]
        }
    }

    /// Bounded publication and immutable request capture only. No provider
    /// function, parsing, or wait occurs under the coordinator's serial lock.
    pub fn poll_syntax(&mut self) -> bool {
        self.poll_view_format_prefetch();
        let previous = self.document.focus();
        // Each live style layer serves every view drawn from it: Preview's
        // code blocks and the buffer's one Code layer may both be live.
        let mut changed = false;
        if self.preview_layer_wanted() {
            self.document.set_focus(crate::document::ViewPresentation::preview());
            changed |= self.poll_markdown_code_syntax();
        } else {
            self.syntax.markdown = Default::default();
        }
        if self.code_layer_wanted() {
            self.document.set_focus(crate::document::ViewPresentation::diff());
            changed |= self.poll_code_syntax();
        } else {
            self.syntax.code.service.cancel();
            self.syntax.code.published = None;
        }
        self.document.set_focus(previous);
        changed
    }

    /// Whether some view of this buffer shows Code, so the buffer's one Code
    /// layer must analyze and publish.
    pub(super) fn code_layer_wanted(&self) -> bool {
        use crate::document::ProjectionKind;
        if !self.document.has_lineage(ProjectionKind::Literal) { return false; }
        if self.views.is_empty() { return self.document.buffer_format().is_code(); }
        self.views.values().any(|view| self.document.format_for(view.presentation).is_code())
    }

    /// Whether some view shows the buffer in Preview.
    fn preview_layer_wanted(&self) -> bool {
        use crate::document::ProjectionKind;
        if !self.document.has_lineage(ProjectionKind::Preview) { return false; }
        if self.views.is_empty() { return self.document.buffer_format().is_markdown(); }
        self.views.values().any(|view| self.document.format_for(view.presentation).is_markdown())
    }

    /// Whether `view` is drawn from the presentation now being published.
    fn shows_focused_presentation(&self, view: &View<P>) -> bool {
        self.document.format_for(view.presentation) == self.document.format()
    }

    fn poll_code_syntax(&mut self) -> bool {
        let input = self.syntax_input();
        if !self.syntax.detected_for_code {
            let detection = self.detect_code_language(&self.syntax.selection);
            self.syntax.code.service.set_language(detection.language.clone());
            self.syntax.detection = Some(detection);
            self.syntax.detected_for_code = true;
        }
        self.adopt_prepared_code_syntax(input.identity());
        let map = self.document.code_presentation_change_map();
        let hull = map
            .and_then(|map| self.document.layout_change_between(map.source_revision(), map.target_revision()))
            .map(|change| (change.old, change.new));
        self.syntax.code.service.rebase_input(input.clone(), map, hull);
        // Drain completed coverage before enqueueing: requesting first could
        // restart a finished regional query on every publication frame.
        let completed = self.syntax.code.service.poll(input.identity());
        let mut requests = Vec::new();
        for view in self.views.values().filter(|view| self.shows_focused_presentation(view)) {
            let range = self.visible_syntax_range(&input, view);
            if !requests.contains(&range) { requests.push(range); }
        }
        for range in &requests {
            self.syntax.code.service.request(input.clone(), range.clone());
        }
        self.request_complete_code_syntax(&input, &requests);
        let mut sheet = code_style::snapshot();
        let style_changed = sheet.revision != self.syntax.sheet.revision;
        let publication = (
            input.identity(),
            self.syntax.code.service.configuration.generation,
        );
        let input_changed = self.syntax.code.published != Some(publication);
        if !completed && !style_changed && !input_changed {
            return false;
        }
        self.syntax.code.referenced_names = self.syntax.code.service.referenced_names();
        let generated =
            code_style::materialize(self.syntax.code.referenced_names.iter().map(String::as_str));
        self.syntax.code.implicit_limited = generated.limited;
        if let Some(generated) = generated.sheet {
            sheet = generated;
        }
        let delta = self.syntax.code.service.take_publication_delta();
        self.publish_code_presentation(sheet, &delta, completed);
        self.syntax.code.published = Some(publication);
        true
    }

    /// A document within the pre-rendering threshold is highlighted completely
    /// once every view's visible text is covered and editing has paused, so
    /// scrolling never reveals unhighlighted text. Larger documents remain
    /// bounded to their visible requests.
    fn request_complete_code_syntax(&mut self, input: &SyntaxInputSnapshot, visible: &[std::ops::Range<usize>]) {
        if !super::prelayout::retains_complete_presentation(&self.document) { return; }
        let now = std::time::Instant::now();
        let changed_at = match self.syntax.code.input_changed {
            Some((identity, at)) if identity == input.identity() => at,
            _ => { self.syntax.code.input_changed = Some((input.identity(), now)); now }
        };
        if now.duration_since(changed_at) < COMPLETE_SYNTAX_EDIT_PAUSE { return; }
        if !visible.iter().all(|range| self.syntax.code.service.covers(input, range.clone())) { return; }
        let len = input.byte_len();
        let mut start = match self.syntax.code.complete_progress {
            Some((identity, offset)) if identity == input.identity() => offset,
            _ => 0,
        };
        while start < len {
            let end = complete_syntax_chunk_end(input, start);
            if !self.syntax.code.service.covers(input, start..end) {
                self.syntax.code.service.request(input.clone(), start..end);
                break;
            }
            start = end;
        }
        self.syntax.code.complete_progress = Some((input.identity(), start));
    }

    /// Capture the current view's syntax request without waiting under the
    /// coordinator lease. Frontends call this after materializing the viewport.
    pub fn prepare_view_syntax_wait(
        &mut self, view: ViewId, deadline: std::time::Instant,
    ) -> Result<(bool, Option<crate::document::syntax::service::SyntaxWait>), CoreError> {
        if !self.views.contains_key(&view) { return Err(CoreError::UnknownView(view)); }
        let changed = self.poll_syntax();
        self.focus_view(view);
        if !self.document.format().is_code() { return Ok((changed, None)); }
        let input = self.syntax_input();
        let range = self.visible_syntax_range(&input, &self.views[&view]);
        let state = &mut self.views.get_mut(&view).expect("validated view").syntax_wait;
        Ok((changed, self.syntax.code.service.prepare_wait(input, range, state, deadline)))
    }

    pub(super) fn visible_syntax_range(&self, input: &SyntaxInputSnapshot, view: &View<P>) -> std::ops::Range<usize> {
        let start = view
            .layout
            .hard_line_at_y(view.layout.viewport_top() as f64)
            .ok()
            .flatten()
            .map(|hit| hit.hard_line())
            .unwrap_or(0);
        let last = view
            .layout
            .hard_line_at_y((view.layout.viewport_top() + view.layout.height()) as f64)
            .ok()
            .flatten()
            .map(|hit| hit.hard_line() + 1)
            .unwrap_or(start + 80);
        let first_byte = input
            .text_tree()
            .hard_line_start(start.min(input.text_tree().hard_line_count() - 1))
            .unwrap_or(0);
        let mut range = first_byte
            ..input
                .text_tree()
                .hard_line_start((last + 20).min(input.text_tree().hard_line_count()))
                .unwrap_or(input.byte_len());
        // A huge wrapped line requests the displayed fragment, never a
        // fabricated whole-line string or an intervening prefix scan.
        if let Some(snapshot) = view
            .layout
            .snapshot()
            .filter(|s| s.document_revision == self.document.revision())
        {
            let mut rows = snapshot
                .rows
                .iter()
                .filter(|r| {
                    r.y + r.height() >= view.layout.viewport_top()
                        && r.y <= view.layout.viewport_top() + view.layout.height()
                });
            if let Some(first) = rows.next() {
                let last = rows.last().unwrap_or(first);
                range = first.text_range.start..last.text_range.end;
            }
        }
        range
    }

    /// Whether the current Code sheet can change metrics through a run.
    fn sheet_has_metric_styles(&mut self, sheet: &crate::document::StyleSheet) -> Option<bool> {
        match self.syntax.metric_styles {
            Some((revision, answer)) if revision == sheet.revision => Some(answer),
            _ => {
                let answer = metrics::sheet_has_metric_styles(sheet).ok()?;
                self.syntax.metric_styles = Some((sheet.revision, answer));
                Some(answer)
            }
        }
    }

    /// Publish explicit runs, bypassing the service store. Tests inject
    /// results this way; the interactive path publishes a delta.
    #[cfg(test)]
    pub(super) fn publish_code_presentation_runs(
        &mut self,
        sheet: Arc<crate::document::StyleSheet>,
        runs: &[crate::document::syntax::SyntaxRun],
    ) {
        let caret_anchors = self.caret_anchors_for_publication();
        let previous = self.document.projection().clone();
        let sheet_changed = code_metrics_changed(&self.syntax.sheet, &sheet);
        self.document.install_code_presentation(sheet.clone(), runs);
        let change = if sheet_changed {
            metrics::MetricChange::Unbounded
        } else {
            metrics::metric_change(&previous, self.document.projection())
        };
        self.apply_code_presentation_change(sheet, change, caret_anchors);
    }

    pub(super) fn caret_anchors_for_publication(&self) -> BTreeMap<ViewId, ViewportTextAnchor> {
        self.views.iter()
            .filter_map(|(id, view)| capture_caret_baseline_anchor(&self.document, view).map(|anchor| (*id, anchor)))
            .collect()
    }

    pub(super) fn publish_code_presentation(
        &mut self,
        sheet: Arc<crate::document::StyleSheet>,
        delta: &crate::document::syntax::service::PublicationDelta,
        completed_analysis: bool,
    ) {
        let sheet_changed = code_metrics_changed(&self.syntax.sheet, &sheet);
        if sheet_changed {
            // A new style choice supersedes any syntax adjustment that has
            // not reached the screen yet. Capture the displayed viewport.
            for view in self.views.values_mut() {
                update_viewport_anchor(&self.document, view);
            }
        }
        // Explicit style changes use ordinary viewport preservation, even if
        // a worker happens to finish in the same poll. Only delayed analysis
        // may compensate for a visible caret's changed font metrics.
        let caret_anchors = if completed_analysis && !sheet_changed {
            self.caret_anchors_for_publication()
        } else {
            BTreeMap::new()
        };
        let has_metric_styles = self.sheet_has_metric_styles(&sheet);
        // The previous presentation is only compared when a run can change
        // metrics; the unbounded comparison needs the projection as displayed.
        let previous_presentation = match has_metric_styles {
            Some(true) if !delta.unbounded => self.document.code_presentation_projection().cloned(),
            _ => None,
        };
        let previous = (delta.unbounded && has_metric_styles != Some(false))
            .then(|| self.document.projection().clone());
        let local = self
            .document
            .install_code_presentation_delta(sheet.clone(), self.syntax.code.service.run_store(), delta);
        let change = if sheet_changed {
            metrics::MetricChange::Unbounded
        } else if local {
            metrics::metric_change_local(
                previous_presentation.as_ref(),
                self.document.projection(),
                delta,
                has_metric_styles.unwrap_or(true),
            )
        } else if let Some(previous) = &previous {
            metrics::metric_change(previous, self.document.projection())
        } else if has_metric_styles == Some(false) {
            metrics::MetricChange::None
        } else {
            metrics::MetricChange::Unbounded
        };
        self.apply_code_presentation_change(sheet, change, caret_anchors);
    }

    pub(super) fn apply_code_presentation_change(
        &mut self,
        sheet: Arc<crate::document::StyleSheet>,
        change: metrics::MetricChange,
        caret_anchors: BTreeMap<ViewId, ViewportTextAnchor>,
    ) {
        let metrics_changed = change != metrics::MetricChange::None;
        // Metric changes are exact: only the lines whose runs changed lose
        // their exact heights and cached geometry. Paint-only changes keep both.
        let changed_lines = match &change {
            metrics::MetricChange::Local(range) => {
                let projection = self.document.projection();
                let count = projection.presentation_line_count();
                match (
                    projection.presentation_line_at_offset(range.start),
                    projection.presentation_line_at_offset(range.end),
                ) {
                    (Some(first), Some(last)) => Some(first.saturating_sub(1)..(last + 2).min(count)),
                    _ => None,
                }
            }
            _ => None,
        };
        self.syntax.sheet = sheet;
        let published = self.views.iter()
            .filter(|(_, view)| self.shows_focused_presentation(view))
            .map(|(id, _)| *id).collect::<Vec<_>>();
        for (id, view) in self.views.iter_mut().filter(|(id, _)| published.contains(id)) {
            // Visible jobs carry paint; speculative geometry survives a
            // paint-only publication and is validated by geometry at install.
            if metrics_changed {
                cancel_active_layout_work(view);
            } else if let Some(active) = view.active_layout_work.take() {
                active.cancellation.cancel();
            }
            if metrics_changed {
                if let Some(anchor) = caret_anchors.get(id) {
                    view.viewport_anchor = Some(*anchor);
                }
            }
            match (&change, &changed_lines) {
                (metrics::MetricChange::None, _) => view.layout.invalidate_syntax_presentation(false),
                (metrics::MetricChange::Local(_), Some(lines)) => {
                    view.layout.invalidate_syntax_presentation_lines(lines.clone())
                }
                _ => view.layout.invalidate_syntax_presentation(true),
            }
            if metrics_changed {
                view.long_line_checkpoints = LongLineCheckpointCache::default();
            }
        }
    }
}

fn code_metrics_changed(
    old: &crate::document::StyleSheet,
    new: &crate::document::StyleSheet,
) -> bool {
    if old.revision == new.revision {
        return false;
    }
    let resolve = |sheet: &crate::document::StyleSheet, name: Option<&str>| {
        sheet
            .resolve_assigned_paragraph_style(
                &crate::document::DocumentStyleAssignment::new(sheet.base_paragraph.clone()),
                &sheet.base_paragraph,
                &Default::default(),
                &Default::default(),
                name.and_then(|name| code_style::resolve_syntax_name(sheet, name)),
                &Default::default(),
            )
            .ok()
    };
    let names = old
        .character_styles()
        .filter_map(|s| old.character_style_metadata(&s.id))
        .chain(
            new.character_styles()
                .filter_map(|s| new.character_style_metadata(&s.id)),
        )
        .map(|m| m.display_name.as_str());
    std::iter::once(None).chain(names.map(Some)).any(|name| {
        match (resolve(old, name), resolve(new, name)) {
            (Some(a), Some(b)) => a.changed_properties(&b).iter().any(|p| {
                p.invalidation_effect() != crate::document::StyleInvalidationEffect::Paint
            }),
            _ => true,
        }
    })
}

#[cfg(test)]
mod tests;

/// The end of the complete-highlighting chunk starting at `start`: the start
/// of the hard line after the one that reaches `COMPLETE_SYNTAX_CHUNK_BYTES`,
/// or the end of the document. Always past `start`.
pub(super) fn complete_syntax_chunk_end(input: &SyntaxInputSnapshot, start: usize) -> usize {
    let len = input.byte_len();
    let tree = input.text_tree();
    let reach = start.saturating_add(COMPLETE_SYNTAX_CHUNK_BYTES).min(len);
    if reach >= len {
        return len;
    }
    tree.hard_line_at_byte(reach)
        .ok()
        .and_then(|line| tree.hard_line_start(line + 1).ok())
        .filter(|end| *end > start && *end <= len)
        .unwrap_or(len)
}
