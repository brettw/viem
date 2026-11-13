//! Application style configuration is a session overlay, never an undo node.
use super::*;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Configuration {
    json: Arc<[u8]>,
    generation: u64,
    format: Format,
}

#[derive(Clone, Copy)]
enum StyleTarget {
    /// A Markdown buffer with no Preview lineage: keep the theme for later.
    Deferred,
    /// Install into the live lineage of this kind.
    Lineage(ProjectionKind),
    /// Code uses the application-wide stylesheet.
    Code,
}

impl Configuration {
    fn applies_to(&self, format: Format) -> bool {
        self.format == format
    }
}

impl Document {
    /// Whether exactly these defaults are installed for the buffer's prose
    /// projection. Publication reinstalls the last defaults whenever a
    /// lineage is rebuilt, so a frontend reapplying the same theme has
    /// nothing to do.
    pub fn style_defaults_installed(&self, json: &[u8]) -> bool {
        match self.style_target() {
            StyleTarget::Deferred => self.configuration.as_ref()
                .is_some_and(|configuration| configuration.format == Format::Markdown && *configuration.json == *json),
            StyleTarget::Lineage(kind) => self.with_kind_focus(kind, |document| document.focused_style_defaults_installed(json)),
            StyleTarget::Code => false,
        }
    }

    fn focused_style_defaults_installed(&self, json: &[u8]) -> bool {
        self.configuration.as_ref().is_some_and(|configuration| configuration.applies_to(self.state().structure_format())
            && *configuration.json == *json
            && self.state().projection.style_sheet().theme_generation == configuration.generation)
    }

    /// Where application prose defaults belong. A Markdown buffer's theme
    /// always styles its Preview lineage, whichever view is focused; while no
    /// view shows Preview it is kept for when that lineage is built.
    fn style_target(&self) -> StyleTarget {
        if self.markdown {
            return if self.lineages.has(ProjectionKind::Preview) { StyleTarget::Lineage(ProjectionKind::Preview) }
                else { StyleTarget::Deferred };
        }
        if self.format().is_code() { StyleTarget::Code } else { StyleTarget::Lineage(self.projection_kind()) }
    }

    /// Keep a Markdown theme for the Preview lineage built later.
    fn defer_markdown_style_defaults(&mut self, json: &[u8]) -> Result<Vec<String>, StyleDefaultsError> {
        let generation = self.next_revision.checked_add(1)
            .ok_or_else(|| StyleDefaultsError::Json("style generation exhausted".into()))?;
        self.configuration = Some(Configuration { json: Arc::from(json), generation, format: Format::Markdown });
        self.next_revision = generation;
        Ok(Vec::new())
    }

    /// Load application defaults into a pristine document. The defaults are
    /// also recorded, like replaced ones, so a projection rebuilt later from
    /// source (another lineage, a reprojected history node) is styled alike.
    pub fn initialize_style_defaults(&mut self, json: &[u8]) -> Result<Vec<String>, StyleDefaultsError> {
        if self.revision().0 != 0 || self.history_status().node_count != 1 {
            return Err(StyleDefaultsError::NotPristine);
        }
        // A pristine document has a single lineage.
        if let StyleTarget::Deferred = self.style_target() { return self.defer_markdown_style_defaults(json); }
        let (mut sheet, diagnostics) = self.projection().style_sheet().with_default_json(json)?;
        let generation = sheet
            .revision
            .0
            .checked_add(1)
            .ok_or_else(|| StyleDefaultsError::Json("style generation exhausted".into()))?;
        sheet.set_configuration_revision(StyleSheetRevision(generation));
        sheet.theme_generation = generation;
        let mut state = self.state().clone();
        let assignment = state.projection.document_style().clone();
        state
            .projection
            .install_configuration_styles(state.revision, sheet, assignment);
        let format = state.structure_format();
        let state = Arc::new(state);
        self.history.initialize_projection(lineage::HistoryState::recording(&state));
        self.lineages = lineage::Lineages::single(lineage::LiveLineage::new(state));
        self.configuration = Some(Configuration { json: Arc::from(json), generation, format });
        self.refresh_code_base();
        // Reserve the consumed style generation so the next source/model
        // transaction still advances both style and projection identities.
        self.next_revision = self.next_revision.max(generation);
        Ok(diagnostics)
    }

    /// Replace application defaults in an edited document. Source declarations,
    /// exact positions, the source revision and all history records are kept.
    /// Replacing the installed defaults with identical bytes changes nothing.
    pub fn replace_style_defaults(
        &mut self,
        json: &[u8],
    ) -> Result<Vec<String>, StyleDefaultsError> {
        match self.style_target() {
            StyleTarget::Deferred => self.defer_markdown_style_defaults(json),
            StyleTarget::Code => Err(StyleDefaultsError::Json("Code uses its application-wide stylesheet".into())),
            StyleTarget::Lineage(kind) => self.with_kind_focus_mut(kind, |document| document.replace_focused_style_defaults(json)),
        }
    }

    fn replace_focused_style_defaults(&mut self, json: &[u8]) -> Result<Vec<String>, StyleDefaultsError> {
        if self.focused_style_defaults_installed(json) {
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
