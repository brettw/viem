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

mod metrics;

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
        assert_eq!(core.syntax.service.configuration.filename.as_deref(), Some("first.vim"));
        let old = core.syntax.service.configuration.clone();
        core.redetect_code_language(Some("second.vim"));
        assert_eq!(core.syntax.service.configuration.filename.as_deref(), Some("second.vim"));
        assert_eq!(core.syntax.service.configuration.language, old.language);
        assert_ne!(core.syntax.service.configuration.generation, old.generation);
        core.set_syntax_provider_factory(Arc::new(|| Box::new(NoSyntax)));
        assert_eq!(core.syntax.service.configuration.filename.as_deref(), Some("second.vim"));
    }
}

pub(super) struct CoreSyntax {
    service: SyntaxService,
    pub(super) selection: LanguageSelection,
    pub(super) automatic_detection: Option<Detection>,
    pub(super) automatic_mode: bool,
    detection: Option<Detection>,
    pub(super) detected_for_code: bool,
    filename_associations: Vec<detection::FilenameAssociation>,
    detection_profile: Arc<detection::DetectionProfile>,
    filename: String,
    pub(super) published: Option<(SyntaxInputIdentity, u64)>,
    referenced_names: Vec<String>,
    implicit_limited: bool,
    sheet: Arc<crate::document::StyleSheet>,
    /// Whether `sheet` has any automatic style with non-default metrics,
    /// memoized by its revision.
    metric_styles: Option<(crate::document::StyleSheetRevision, bool)>,
}
impl Default for CoreSyntax {
    fn default() -> Self {
        Self {
            service: Default::default(),
            selection: LanguageSelection::Automatic,
            automatic_detection: None,
            automatic_mode: true,
            detection: None,
            detected_for_code: false,
            filename_associations: Vec::new(),
            detection_profile: detection::bundled_profile(),
            filename: String::new(),
            published: None,
            referenced_names: Vec::new(),
            implicit_limited: false,
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
        let export = self.document.prepare_html_export()
            .with_tabstop(self.views[&view].commands.indentation_options().tabstop);
        if !self.document.format().is_code() { return Ok(export); }
        let input = self.syntax_input();
        let mut service = self.syntax.service.for_export();
        if !self.syntax.detected_for_code {
            let detection = detection::detect_with_profile(&input, &self.syntax.filename,
                &self.syntax.selection, &self.syntax.filename_associations, &self.syntax.detection_profile);
            service.set_language(detection.language);
        }
        Ok(export.with_syntax(service, input))
    }

    fn syntax_input(&self) -> SyntaxInputSnapshot {
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
        self.syntax.service.set_filename((!filename.is_empty()).then(|| filename.to_owned()));
        let detection = self.detect_code_language(&self.syntax.selection);
        if allow_auto_code
            && detection.language.is_some()
            && self.document.format() == Format::PlainText
        {
            self.document.initialize_code_format()?;
        }
        self.syntax.service.set_language(detection.language.clone());
        self.syntax.automatic_detection = Some(detection.clone());
        self.syntax.detection = Some(detection);
        self.syntax.detected_for_code = self.document.format().is_code();
        self.publish_reflow_language();
        Ok(())
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
    fn refresh_code_detection(&mut self) {
        self.syntax.automatic_detection = Some(self.detect_code_language(&LanguageSelection::Automatic));
        self.syntax.service.set_filename((!self.syntax.filename.is_empty()).then(|| self.syntax.filename.clone()));
        let detection = self.detect_code_language(&self.syntax.selection);
        self.syntax.service.set_language(detection.language.clone());
        self.syntax.detection = Some(detection);
        self.syntax.published = None;
        self.publish_reflow_language();
    }
    /// Explicit policy changes/redetection are the only post-load detector
    /// entry points; ordinary edits and viewport requests do not call these.
    pub fn set_code_filename_associations(
        &mut self,
        associations: Vec<detection::FilenameAssociation>,
    ) -> Result<(), String> {
        if associations.len() > 256
            || associations.iter().any(|a| {
                a.pattern.is_empty()
                    || a.pattern.len() > 256
                    || a.pattern.contains('\0')
                    || a.language.is_empty()
                    || a.language.len() > 128
                    || !a
                        .language
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_+.#-".contains(&b))
            })
        {
            return Err("Filename association profile exceeds its bounds".into());
        }
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
        self.syntax.detected_for_code = self.document.format().is_code();
    }
    pub fn code_language_detection(&self) -> Option<&Detection> {
        self.syntax.detection.as_ref()
    }
    pub fn configure_syntax(&mut self, directory: &str) {
        self.syntax.service.set_vim_directory(directory.to_owned());
        self.syntax.published = None;
    }

    /// Replace the platform's syntax integration without changing document
    /// commands or rendering. Factories and provider destruction run on syntax
    /// workers; configuration and language selection remain portable core state.
    pub fn set_syntax_provider_factory(
        &mut self,
        factory: crate::document::syntax::service::SyntaxProviderFactory,
    ) {
        let mut service = SyntaxService::with_factory(factory);
        service.set_language(self.syntax.service.configuration.language.clone());
        service.set_vim_directory(self.syntax.service.configuration.vim_directory.clone());
        service.set_filename(self.syntax.service.configuration.filename.clone());
        self.syntax.service = service;
        self.syntax.published = None;
    }
    pub fn syntax_diagnostics(&self) -> String {
        let mut diagnostics = self.syntax.service.diagnostics();
        if self.syntax.implicit_limited {
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
        self.syntax.service.statistics
    }

    /// Names in the accepted, bounded syntax cache, including unresolved names.
    /// Menu inspection never runs a provider or scans unhighlighted document text.
    pub fn syntax_style_names(&self) -> &[String] {
        let current = (self.syntax_input().identity(), self.syntax.service.configuration.generation);
        if self.document.format().is_code() && self.syntax.published == Some(current) {
            &self.syntax.referenced_names
        } else {
            &[]
        }
    }

    /// Bounded publication and immutable request capture only. No provider
    /// function, parsing, or wait occurs under the coordinator's serial lock.
    pub fn poll_syntax(&mut self) -> bool {
        if !self.document.format().is_code() {
            self.syntax.service.cancel();
            self.syntax.published = None;
            return false;
        }
        let input = self.syntax_input();
        if !self.syntax.detected_for_code {
            let detection = self.detect_code_language(&self.syntax.selection);
            self.syntax.service.set_language(detection.language.clone());
            self.syntax.detection = Some(detection);
            self.syntax.detected_for_code = true;
        }
        let map = self.document.code_presentation_change_map();
        let hull = map
            .and_then(|map| self.document.layout_change_between(map.source_revision(), map.target_revision()))
            .map(|change| (change.old, change.new));
        self.syntax.service.rebase_input(input.clone(), map, hull);
        let mut requests = Vec::new();
        for view in self.views.values() {
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
                let rows = snapshot
                    .rows
                    .iter()
                    .filter(|r| {
                        r.y + r.height() >= view.layout.viewport_top()
                            && r.y <= view.layout.viewport_top() + view.layout.height()
                    })
                    .collect::<Vec<_>>();
                if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                    range = first.text_range.start..last.text_range.end;
                }
            }
            if !requests.contains(&range) {
                requests.push(range);
            }
        }
        for range in requests {
            self.syntax.service.request(input.clone(), range);
        }
        let completed = self.syntax.service.poll(input.identity());
        let mut sheet = code_style::snapshot();
        let style_changed = sheet.revision != self.syntax.sheet.revision;
        let publication = (
            input.identity(),
            self.syntax.service.configuration.generation,
        );
        let input_changed = self.syntax.published != Some(publication);
        if !completed && !style_changed && !input_changed {
            return false;
        }
        self.syntax.referenced_names = self.syntax.service.referenced_names();
        let generated =
            code_style::materialize(self.syntax.referenced_names.iter().map(String::as_str));
        self.syntax.implicit_limited = generated.limited;
        if let Some(generated) = generated.sheet {
            sheet = generated;
        }
        let delta = self.syntax.service.take_publication_delta();
        self.publish_code_presentation(sheet, &delta);
        self.syntax.published = Some(publication);
        true
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

    fn caret_anchors_for_publication(&self) -> BTreeMap<ViewId, ViewportTextAnchor> {
        self.views.iter()
            .filter_map(|(id, view)| capture_caret_baseline_anchor(&self.document, view).map(|anchor| (*id, anchor)))
            .collect()
    }

    pub(super) fn publish_code_presentation(
        &mut self,
        sheet: Arc<crate::document::StyleSheet>,
        delta: &crate::document::syntax::service::PublicationDelta,
    ) {
        let caret_anchors = self.caret_anchors_for_publication();
        let sheet_changed = code_metrics_changed(&self.syntax.sheet, &sheet);
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
            .install_code_presentation_delta(sheet.clone(), self.syntax.service.run_store(), delta);
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

    fn apply_code_presentation_change(
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
                let count = projection.presentation_line_count(false);
                match (
                    projection.presentation_line_at_offset(range.start, false),
                    projection.presentation_line_at_offset(range.end, false),
                ) {
                    (Some(first), Some(last)) => Some(first.saturating_sub(1)..(last + 2).min(count)),
                    _ => None,
                }
            }
            _ => None,
        };
        self.syntax.sheet = sheet;
        for (id, view) in &mut self.views {
            cancel_active_layout_work(view);
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
