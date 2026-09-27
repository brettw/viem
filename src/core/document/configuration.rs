//! Application style configuration is a session overlay, never an undo node.
use super::*;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Configuration {
    json: Arc<[u8]>,
    generation: u64,
    format: Format,
}

impl Configuration {
    fn applies_to(&self, format: Format) -> bool {
        self.format == format || (self.format.is_markdown() && format.is_markdown())
    }
}

impl Document {
    /// Replace application defaults in an edited document. Source declarations,
    /// exact positions, the source revision and all history records are kept.
    pub fn replace_style_defaults(
        &mut self,
        json: &[u8],
    ) -> Result<Vec<String>, StyleDefaultsError> {
        if self.format().is_code() {
            return Err(StyleDefaultsError::Json(
                "Code uses its application-wide stylesheet".into(),
            ));
        }
        let (mut sheet, diagnostics) = self
            .projection()
            .style_sheet()
            .replacing_default_json(json, self.projection())?;
        let generation = self
            .next_revision
            .max(sheet.revision.0)
            .checked_add(1)
            .ok_or_else(|| StyleDefaultsError::Json("style generation exhausted".into()))?;
        sheet.theme_generation = generation;
        sheet.set_configuration_revision(StyleSheetRevision(generation));
        let mut state = self.state().clone();
        let assignment = state.projection.document_style().clone();
        state
            .projection
            .install_configuration_styles(state.revision, sheet, assignment);
        self.configuration = Some(Configuration {
            json: Arc::from(json),
            generation,
            format: state.format,
        });
        self.configuration_state = Some(state);
        self.next_revision = generation;
        Ok(diagnostics)
    }

    /// History can select a snapshot recorded under an older application
    /// theme. Reapply only then; ordinary style edits to the current generation
    /// remain normal document operations and preserve their own declarations.
    pub(super) fn refresh_configuration(&mut self) {
        self.configuration_state = None;
        let Some(configuration) = &self.configuration else {
            return;
        };
        let state = self.history.current();
        if !configuration.applies_to(state.format)
            || state.projection.style_sheet().theme_generation == configuration.generation
        {
            return;
        }
        // The same bytes were validated at publication. Source-owned definitions
        // remain authoritative, including dependencies retained by the parser.
        let Ok((mut sheet, _)) = state
            .projection
            .style_sheet()
            .replacing_default_json(&configuration.json, &state.projection)
        else {
            return;
        };
        sheet.theme_generation = configuration.generation;
        sheet.set_configuration_revision(StyleSheetRevision(
            sheet.revision.0.max(configuration.generation),
        ));
        let mut state = (**state).clone();
        let assignment = state.projection.document_style().clone();
        state
            .projection
            .install_configuration_styles(state.revision, sheet, assignment);
        self.configuration_state = Some(state);
    }
}
