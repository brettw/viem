//! Styling may select only part of a source entity. Materialize its complete
//! contributor without changing text or style, then style the original logical
//! selection. Only the final composed patches and identity text map are public.
use super::replacement::PatchComposition;
use super::*;

pub(super) fn boundary_materializations(
    document: &Document,
    range: &Range<usize>,
) -> Result<Vec<TextEdit>, DocumentError> {
    document.validate_range(range)?;
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let mut edits = Vec::new();
    for at in [range.start, range.end] {
        let edit = super::super::source_edit::complete_contributors(
            document.projection(),
            &TextEdit::new(at..at, ""),
        )?;
        if !edit.range.is_empty() && !edits.iter().any(|old: &TextEdit| old.range == edit.range) {
            edits.push(edit);
        }
    }
    Ok(edits)
}

impl Document {
    pub(super) fn prepare_materialized_character_intent(
        &self,
        intent: &PersistedStyleIntent,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let range = match intent {
            PersistedStyleIntent::AssignCharacterStyle { range, .. }
            | PersistedStyleIntent::SetDirectCharacterProperties { range, .. }
            | PersistedStyleIntent::ClearDirectCharacterProperties { range, .. } => *range,
            _ => return Ok(None),
        };
        self.validate_style_text_range(range)?;
        let selected = range.start().offset()..range.end().offset();
        self.prepare_with_materialized_style_boundaries(&selected, |scratch| {
            // Materialization is text-identical. Rebind the validated logical
            // selection to the speculative revision rather than reinterpreting
            // an old public TextRange in a newer snapshot.
            let rebound = TextRange::new(
                scratch.text_point(selected.start)?,
                scratch.text_point(selected.end)?,
            )?;
            let mut intent = intent.clone();
            match &mut intent {
                PersistedStyleIntent::AssignCharacterStyle { range, .. }
                | PersistedStyleIntent::SetDirectCharacterProperties { range, .. }
                | PersistedStyleIntent::ClearDirectCharacterProperties { range, .. } => {
                    *range = rebound
                }
                _ => unreachable!("character range selected above"),
            }
            scratch.prepare_persisted_style_intent(intent)
        })
    }

    pub(super) fn prepare_with_materialized_style_boundaries(
        &self,
        range: &Range<usize>,
        prepare: impl FnOnce(&Document) -> Result<PreparedModelTransaction, ModelTransactionError>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let edits = boundary_materializations(self, range)?;
        if edits.is_empty() {
            return Ok(None);
        }
        let mut patches = Vec::new();
        for edit in edits {
            patches.extend(super::super::source_edit::rich_text_patches(
                self, &edit, None,
            )?);
        }
        let mut scratch = self.scratch_document();
        let materialized = scratch.prepare_source_only_patches(patches)?;
        let mut sources = PatchComposition::new(self.source_byte_len());
        for patch in materialized.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(materialized)?;
        // Replacing the source spelling must preserve every existing style,
        // including the unselected part of each expanded contributor.
        if !super::super::rich_text::character_edit_verified(
            self.projection(),
            scratch.projection(),
            &(0..0),
            &CharacterProperties::default(),
        ) || !boundary_materializations(&scratch, range)?.is_empty()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let styled = prepare(&scratch)?;
        for patch in styled.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(styled)?;
        let prepared = self.prepare_source_only_patches(sources.source_patches())?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        if !candidate
            .projection
            .has_same_hard_line_structure(scratch.projection())
            || candidate.projection.style_spans() != scratch.projection().style_spans()
            || !super::super::rich_text::character_edit_verified(
                scratch.projection(),
                &candidate.projection,
                &(0..0),
                &CharacterProperties::default(),
            )
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(Some(prepared))
    }
}
