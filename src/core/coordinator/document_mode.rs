//! Source-preserving mode selection shared by the native frontends.
use super::*;
use super::syntax::PreparedCodeSyntax;
use crate::document::syntax::{detection::LanguageSelection, languages, SyntaxInputIdentity, SyntaxInputSnapshot};

/// Editing must pause this long before the other Markdown view is prepared,
/// so typing never reprojects the whole document in the background.
const VIEW_FORMAT_PREFETCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(400);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentMode {
    Automatic,
    PlainText,
    Markdown,
    /// Select literal Code presentation while retaining its language policy.
    CodeFormat,
    Code(String),
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentModeState {
    pub document_id: u64,
    pub document_revision: u64,
    pub format: &'static str,
    pub automatic: bool,
    pub language: Option<String>,
    pub detected_language: Option<String>,
    pub detected_name: String,
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn document_mode_state(&self) -> DocumentModeState {
        let detected = self.syntax.automatic_detection.as_ref().and_then(|d| d.language.clone());
        DocumentModeState {
            document_id: self.document.id().0,
            document_revision: self.document.revision().0,
            format: if self.document.format().is_code() { "code" } else if self.document.format().is_markdown() { "markdown" } else { "plainText" },
            automatic: self.syntax.automatic_mode,
            language: match &self.syntax.selection { LanguageSelection::Language(id) => Some(id.clone()), _ => None },
            detected_name: detected.as_deref().map(languages::display_name).unwrap_or_else(|| "Plain Text".into()),
            detected_language: detected,
        }
    }

    /// Markdown Preview, or literal Code whose language is Markdown: the
    /// explicit language, otherwise the detected one. Frontends read this from
    /// the document state instead of exporting the mode on every refresh.
    pub fn is_markdown_source(&self) -> bool {
        match self.document.format() {
            Format::Markdown => true,
            Format::Code => match &self.syntax.selection {
                LanguageSelection::Language(id) => id == "markdown",
                _ => self.syntax.automatic_detection.as_ref()
                    .and_then(|detection| detection.language.as_deref()) == Some("markdown"),
            },
            _ => false,
        }
    }

    /// Markdown Preview and literal Markdown (Code with the Markdown language)
    /// prepare a switch to each other once editing pauses. Other formats and
    /// any change retire the preparation immediately.
    pub(super) fn poll_view_format_prefetch(&mut self) {
        let target = match self.document.format() {
            Format::Markdown => Some(Format::Code),
            Format::Code if self.reflow_language().as_deref() == Some("markdown") => Some(Format::Markdown),
            _ => None,
        };
        let Some(target) = target else {
            if self.syntax.view_format_changed.take().is_some() { self.document.cancel_view_format_prefetch(); }
            self.retire_stale_prepared_code();
            return;
        };
        let identity = (self.document.id(), self.document.revision());
        let now = std::time::Instant::now();
        let changed_at = match self.syntax.view_format_changed {
            Some((current, at)) if current == identity => at,
            _ => {
                self.document.cancel_view_format_prefetch();
                self.retire_stale_prepared_code();
                self.syntax.view_format_changed = Some((identity, now));
                now
            }
        };
        if now.duration_since(changed_at) >= VIEW_FORMAT_PREFETCH_PAUSE {
            self.document.prefetch_view_format(target);
            if target == Format::Code { self.prepare_code_syntax_for_switch(); }
        }
    }

    /// Turning Preview off would otherwise wait the syntax grace period for a
    /// cold Code analysis. Once the Code projection is prepared, analyze the
    /// lines every view will show against the input identity the switch
    /// installs. Nothing is published until the switch adopts the service.
    fn prepare_code_syntax_for_switch(&mut self) {
        let markdown = self.syntax_input();
        let visible = self.views.values()
            .filter_map(|view| self.document.projection().source_range(self.visible_syntax_range(&markdown, view)))
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end));
        let document = self.document.id().0;
        let Some((input, range)) = self.document.peek_prefetched_view_format(Format::Code, |revision, projection| {
            let input = SyntaxInputSnapshot::new(
                SyntaxInputIdentity { document, revision: revision.0, generation: 1 },
                projection.text_tree().clone());
            let tree = projection.text_tree();
            let lines = tree.hard_line_count();
            let boundary = |source: usize, affinity| projection.map_source_boundary(revision, source, affinity)
                .ok().map(|boundary| boundary.formatted_offset);
            let range = visible.and_then(|source| {
                let start = tree.hard_line_at_byte(boundary(source.start, BoundaryAffinity::Downstream)?).ok()?;
                let end = tree.hard_line_at_byte(boundary(source.end, BoundaryAffinity::Upstream)?).ok()?;
                // The same trailing margin as an ordinary visible request.
                Some(tree.hard_line_start(start).ok()?..tree.hard_line_start((end + 21).min(lines)).unwrap_or(tree.byte_len()))
            }).unwrap_or_else(|| 0..tree.hard_line_start(80.min(lines)).unwrap_or(tree.byte_len()));
            (input, range)
        }) else { return };
        let source_revision = self.document.revision().0;
        // Keep following the viewport while the preparation stays current.
        if let Some(prepared) = self.syntax.prepared_code.as_mut()
            .filter(|prepared| prepared.source_revision == source_revision && prepared.identity == input.identity()) {
            prepared.service.poll(input.identity());
            prepared.service.request(input, range);
            return;
        }
        let mut service = self.syntax.service.for_export();
        service.set_language(Some("markdown".into()));
        service.request(input.clone(), range);
        self.syntax.prepared_code = Some(PreparedCodeSyntax { source_revision, identity: input.identity(), service });
    }

    /// Keep a preparation only while Preview remains at its source revision,
    /// or while Code shows the revision its switch installed: that switch's
    /// own viewport materialization polls syntax before adoption. An edit can
    /// take the switch's predicted revision number, but not in Code.
    fn retire_stale_prepared_code(&mut self) {
        let (document, revision) = (self.document.id().0, self.document.revision().0);
        let markdown = self.document.format() == Format::Markdown;
        let current = self.syntax.prepared_code.as_ref().is_some_and(|prepared| prepared.identity.document == document
            && if markdown { prepared.source_revision == revision } else { prepared.identity.revision == revision });
        if !current { self.syntax.prepared_code = None; }
    }

    /// Continue analysis prepared before the switch when it targets exactly
    /// the installed input and the service's current configuration.
    pub(super) fn adopt_prepared_code_syntax(&mut self, input: SyntaxInputIdentity) {
        let Some(prepared) = &self.syntax.prepared_code else { return };
        let (current, candidate) = (&self.syntax.service.configuration, &prepared.service.configuration);
        if !self.document.format().is_code() || prepared.identity != input
            || current.language != candidate.language || current.filename != candidate.filename
            || current.vim_directory != candidate.vim_directory
            || current.registry_generation != candidate.registry_generation {
            return;
        }
        self.syntax.service = self.syntax.prepared_code.take().expect("checked preparation").service;
    }

    pub(super) fn set_document_mode(&mut self, view: ViewId, document: DocumentId, revision: Revision,
                                   mode: DocumentMode) -> Result<CoreOutcome, CoreError> {
        let automatic = self.detect_code_language(&LanguageSelection::Automatic);
        let target = match &mode {
            DocumentMode::PlainText => Format::PlainText,
            DocumentMode::Markdown => Format::Markdown,
            DocumentMode::CodeFormat => Format::Code,
            DocumentMode::Code(id) => {
                if !languages::supported_languages().iter().any(|language| &language.id == id) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                Format::Code
            }
            DocumentMode::Automatic => {
                if automatic.language.is_some() { Format::Code } else { Format::PlainText }
            }
        };
        // Validate and publish the reprojection before changing syntax policy.
        // This preserves stale-operation atomicity and maps every shared view.
        let mut outcome = self.apply_native_model_request(view, ModelRequest::SetViewFormat { document, revision, target })?;
        self.syntax.automatic_detection = Some(automatic);
        self.syntax.automatic_mode = mode == DocumentMode::Automatic;
        match mode {
            DocumentMode::CodeFormat => self.set_code_language(self.syntax.selection.clone()),
            DocumentMode::Code(id) => self.set_code_language(LanguageSelection::Language(id)),
            DocumentMode::Automatic => self.set_code_language(LanguageSelection::Automatic),
            _ => {}
        }
        self.syntax.detected_for_code = self.document.format().is_code();
        // The switch's materialization may already have adopted it; a later
        // language choice can still match. Either way the switch retires it.
        self.adopt_prepared_code_syntax(self.syntax_input().identity());
        self.syntax.prepared_code = None;
        self.syntax.published = None;
        // Syntax changes are presentation only, including selecting another
        // language while already in Code. Native observers still refresh.
        outcome.layout_changed = true;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult};
    use crate::document::syntax::{Coverage, SyntaxRun, SyntaxStyleName};
    use crate::layout::MockTextMeasurementProvider;
    use std::sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, Arc};
    use std::time::{Duration, Instant};

    /// Covers each literal Markdown region exactly and counts the analyses.
    struct CountingMarkdown(Arc<AtomicUsize>);
    impl SyntaxProvider for CountingMarkdown {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let mut result = SyntaxResult::missing(request, "fixture");
            if request.configuration.language.as_deref() != Some("markdown") { return result; }
            self.0.fetch_add(1, Ordering::SeqCst);
            result.coverage = Coverage::Exact;
            result.diagnostics.clear();
            result.runs.push(SyntaxRun {
                range: request.range.start..request.range.start + 1,
                name: SyntaxStyleName("Title".into()),
                origin: "fixture".into(),
                priority: 0,
            });
            result
        }
    }

    fn previewed(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId, Arc<AtomicUsize>) {
        let document = Document::from_bytes(source.as_bytes().to_vec(), crate::document::Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        core.set_syntax_provider_factory(Arc::new(move || Box::new(CountingMarkdown(counter.clone()))));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
        (core, view, calls)
    }

    /// Poll as an idle frontend does until the switch's Code syntax is ready.
    fn wait_for_prepared_code(core: &mut Core<MockTextMeasurementProvider>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !core.syntax.prepared_code.as_ref().is_some_and(|prepared| !prepared.service.has_pending_work()) {
            assert!(Instant::now() < deadline, "the Code switch is prepared after editing pauses");
            core.poll_syntax();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn turning_preview_off_adopts_visible_code_syntax_prepared_while_idle() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let source = (0..400).map(|index| format!("Paragraph {index} *text*.\n\n")).collect::<String>();
        let (mut core, view, calls) = previewed(&source);
        wait_for_prepared_code(&mut core);
        let prepared = calls.load(Ordering::SeqCst);
        assert!(prepared > 0, "visible Code lines were analyzed before the switch");
        assert_eq!(core.syntax.service.statistics.publications, 0, "nothing is published in Preview");

        let (document, revision) = (core.document.id(), core.document.revision());
        core.set_document_mode(view, document, revision, DocumentMode::Code("markdown".into())).unwrap();
        assert!(core.syntax.prepared_code.is_none());
        let (_, wait) = core.prepare_view_syntax_wait(view, Instant::now() + Duration::from_secs(5)).unwrap();
        assert!(wait.is_none(), "the visible Code text needs no grace-period wait");
        assert_eq!(calls.load(Ordering::SeqCst), prepared, "the switch reuses the prepared analysis");
        assert!(!core.syntax.service.runs(core.syntax_input().identity()).is_empty());
        assert!(!core.document.is_dirty(), "switching views changes no source");
    }

    #[test]
    fn edits_and_other_languages_discard_prepared_code_syntax() {
        let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
        let (mut core, view, _) = previewed("Paragraph *text*.\n\nAnother paragraph.\n");
        wait_for_prepared_code(&mut core);
        let stale = core.syntax.prepared_code.as_ref().unwrap().identity;
        core.handle(view, CoreEvent::Input(crate::command::InputEvent::key('x'))).unwrap();
        core.poll_syntax();
        assert!(core.syntax.prepared_code.as_ref().is_none_or(|prepared| prepared.identity != stale),
            "an edit retires the preparation keyed to the earlier revision");

        wait_for_prepared_code(&mut core);
        let (document, revision) = (core.document.id(), core.document.revision());
        core.set_document_mode(view, document, revision, DocumentMode::Code("rust".into())).unwrap();
        assert_eq!(core.syntax.service.configuration.language.as_deref(), Some("rust"));
        assert!(core.syntax.service.runs(core.syntax_input().identity()).is_empty(),
            "a different language never adopts Markdown analysis");
    }
}
