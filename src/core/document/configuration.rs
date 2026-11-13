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
        self.format == format
    }
}

impl Document {
    /// Whether exactly these defaults are installed for the current format.
    /// Publication reinstalls the last defaults when a view switch returns to
    /// their format, so a frontend reapplying the same theme has nothing to do.
    pub fn style_defaults_installed(&self, json: &[u8]) -> bool {
        self.configuration.as_ref().is_some_and(|configuration| configuration.applies_to(self.state().structure_format())
            && *configuration.json == *json
            && self.state().projection.style_sheet().theme_generation == configuration.generation)
    }

    /// Replace application defaults in an edited document. Source declarations,
    /// exact positions, the source revision and all history records are kept.
    /// Replacing the installed defaults with identical bytes changes nothing.
    pub fn replace_style_defaults(
        &mut self,
        json: &[u8],
    ) -> Result<Vec<String>, StyleDefaultsError> {
        // A Markdown buffer whose views all show its source has no Preview
        // projection to style yet: keep its theme for when one is built.
        if self.markdown && !self.lineages.has(ProjectionKind::Preview) {
            let generation = self.next_revision.checked_add(1)
                .ok_or_else(|| StyleDefaultsError::Json("style generation exhausted".into()))?;
            self.configuration = Some(Configuration { json: Arc::from(json), generation, format: Format::Markdown });
            self.next_revision = generation;
            return Ok(Vec::new());
        }
        if self.format().is_code() {
            return Err(StyleDefaultsError::Json(
                "Code uses its application-wide stylesheet".into(),
            ));
        }
        if self.style_defaults_installed(json) {
            return Ok(Vec::new());
        }
        let (mut sheet, diagnostics) = self
            .projection()
            .style_sheet()
            .replacing_default_json(json, self.projection(), self.format())?;
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
            format: state.structure_format(),
        });
        self.live_mut().configuration_state = Some(state);
        self.refresh_code_base();
        self.next_revision = generation;
        Ok(diagnostics)
    }

    /// History can select a snapshot recorded under an older application
    /// theme. Reapply only then; ordinary style edits to the current generation
    /// remain normal document operations and preserve their own declarations.
    pub(super) fn refresh_configuration(&mut self) {
        for kind in self.lineages.kinds().collect::<Vec<_>>() {
            self.with_kind_focus_mut(kind, Self::refresh_configuration_state);
        }
        self.refresh_code_base();
    }

    fn refresh_configuration_state(&mut self) {
        self.live_mut().configuration_state = None;
        let Some(configuration) = &self.configuration else {
            return;
        };
        let state = &self.live().state;
        if !configuration.applies_to(state.structure_format())
            || state.projection.style_sheet().theme_generation == configuration.generation
        {
            return;
        }
        // The same bytes were validated at publication. Source-owned definitions
        // remain authoritative, including dependencies retained by the parser.
        let Ok((mut sheet, _)) = state
            .projection
            .style_sheet()
            .replacing_default_json(&configuration.json, &state.projection, state.structure_format())
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
        self.live_mut().configuration_state = Some(state);
    }
}
