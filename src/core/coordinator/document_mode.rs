//! Source-preserving mode selection shared by the native frontends.
use super::*;
use crate::document::syntax::{detection::LanguageSelection, languages};

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
            return;
        };
        let identity = (self.document.id(), self.document.revision());
        let now = std::time::Instant::now();
        let changed_at = match self.syntax.view_format_changed {
            Some((current, at)) if current == identity => at,
            _ => {
                self.document.cancel_view_format_prefetch();
                self.syntax.view_format_changed = Some((identity, now));
                now
            }
        };
        if now.duration_since(changed_at) >= VIEW_FORMAT_PREFETCH_PAUSE {
            self.document.prefetch_view_format(target);
        }
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
        self.syntax.published = None;
        // Syntax changes are presentation only, including selecting another
        // language while already in Code. Native observers still refresh.
        outcome.layout_changed = true;
        Ok(outcome)
    }
}
