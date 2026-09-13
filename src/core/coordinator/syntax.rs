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

pub(super) struct CoreSyntax {
    service: SyntaxService,
    selection: LanguageSelection,
    detection: Option<Detection>,
    detected_for_code: bool,
    filename_associations: Vec<detection::FilenameAssociation>,
    detection_profile: Arc<detection::DetectionProfile>,
    filename: String,
    published: Option<(SyntaxInputIdentity, u64)>,
    referenced_names: Vec<String>,
    sheet: Arc<crate::document::StyleSheet>,
}
impl Default for CoreSyntax {
    fn default() -> Self {
        Self {
            service: Default::default(),
            selection: LanguageSelection::Automatic,
            detection: None,
            detected_for_code: false,
            filename_associations: Vec::new(),
            detection_profile: detection::bundled_profile(),
            filename: String::new(),
            published: None,
            referenced_names: Vec::new(),
            sheet: code_style::snapshot(),
        }
    }
}
impl<P: TextMeasurementProvider> Core<P> {
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
    pub fn initialize_code_detection(
        &mut self,
        filename: &str,
        allow_auto_code: bool,
    ) -> Result<(), crate::document::StyleDefaultsError> {
        if !self.views.is_empty() {
            return Err(crate::document::StyleDefaultsError::NotPristine);
        }
        self.syntax.filename = filename.to_owned();
        let detection = detection::detect_with_profile(
            &self.syntax_input(),
            filename,
            &self.syntax.selection,
            &self.syntax.filename_associations,
            &self.syntax.detection_profile,
        );
        if allow_auto_code
            && detection.language.is_some()
            && self.document.format() == Format::PlainText
        {
            self.document.initialize_code_format()?;
        }
        self.syntax.service.set_language(detection.language.clone());
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
        self.syntax.selection = selection;
        let detection = detection::detect_with_profile(
            &self.syntax_input(),
            &self.syntax.filename,
            &self.syntax.selection,
            &self.syntax.filename_associations,
            &self.syntax.detection_profile,
        );
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
            self.set_code_language(self.syntax.selection.clone());
        }
        Ok(())
    }
    /// Install a validated portable detection policy. Explicit configuration
    /// changes redetect; ordinary editing keeps the load-time selection.
    pub fn set_code_detection_profile(&mut self, profile: Arc<detection::DetectionProfile>) {
        self.syntax.detection_profile = profile;
        if self.syntax.detection.is_some() {
            self.set_code_language(self.syntax.selection.clone());
        }
    }
    pub fn redetect_code_language(&mut self, filename: Option<&str>) {
        if let Some(filename) = filename {
            self.syntax.filename = filename.to_owned();
        }
        self.set_code_language(self.syntax.selection.clone());
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
        self.syntax.service = service;
        self.syntax.published = None;
    }
    pub fn syntax_diagnostics(&self) -> String {
        self.syntax.service.diagnostics()
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
            let detection = detection::detect_with_profile(
                &input,
                &self.syntax.filename,
                &self.syntax.selection,
                &self.syntax.filename_associations,
                &self.syntax.detection_profile,
            );
            self.syntax.service.set_language(detection.language.clone());
            self.syntax.detection = Some(detection);
            self.syntax.detected_for_code = true;
        }
        self.syntax.service.rebase_input(input.clone(), self.document.code_presentation_change_map());
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
        let sheet = code_style::snapshot();
        let style_changed = sheet.revision != self.syntax.sheet.revision;
        let publication = (
            input.identity(),
            self.syntax.service.configuration.generation,
        );
        let input_changed = self.syntax.published != Some(publication);
        if !completed && !style_changed && !input_changed {
            return false;
        }
        let runs = self.syntax.service.runs(input.identity());
        self.syntax.referenced_names = runs
            .iter()
            .map(|run| run.name.0.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        self.publish_code_presentation(sheet, &runs);
        self.syntax.published = Some(publication);
        true
    }

    pub(super) fn publish_code_presentation(
        &mut self,
        sheet: Arc<crate::document::StyleSheet>,
        runs: &[crate::document::syntax::SyntaxRun],
    ) {
        let previous = self.document.projection().clone();
        let caret_anchors: BTreeMap<_, _> = self.views.iter()
            .filter_map(|(id, view)| capture_caret_baseline_anchor(&self.document, view).map(|anchor| (*id, anchor)))
            .collect();
        self.document.install_code_presentation(sheet.clone(), runs);
        let metrics_changed = code_metrics_changed(&self.syntax.sheet, &sheet)
            || metrics::runs_change_metrics(&previous, self.document.projection());
        self.syntax.sheet = sheet;
        for (id, view) in &mut self.views {
            cancel_active_layout_work(view);
            if metrics_changed {
                if let Some(anchor) = caret_anchors.get(id) {
                    view.viewport_anchor = Some(*anchor);
                }
            }
            view.layout.invalidate_syntax_presentation(metrics_changed);
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
                name.and_then(|name| code_style::resolve_name(sheet, name)),
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
