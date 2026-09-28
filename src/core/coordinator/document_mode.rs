//! Source-preserving mode selection shared by the native frontends.
use super::*;
use crate::document::syntax::{detection::LanguageSelection, languages};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentMode {
    Automatic,
    PlainText,
    Markdown,
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

    pub(super) fn set_document_mode(&mut self, view: ViewId, document: DocumentId, revision: Revision,
                                   mode: DocumentMode, formatted_markdown: bool) -> Result<CoreOutcome, CoreError> {
        let automatic = self.detect_code_language(&LanguageSelection::Automatic);
        let markdown = if formatted_markdown { Format::Markdown } else { Format::MarkdownSource };
        let target = match &mode {
            DocumentMode::PlainText => Format::PlainText,
            DocumentMode::Markdown => markdown,
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
