//! Atomic batches of direct declarations. Each source translation is verified
//! on a scratch snapshot, then its minimal patches are composed and published
//! as one transaction and undo entry.
use super::replacement::PatchComposition;
use super::*;

impl Document {
    pub(super) fn prepare_direct_properties(
        &self,
        mut range: Range<usize>,
        values: Vec<(StyleProperty, Option<StylePropertyValue>)>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if values.is_empty() || values.len() > 32 {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let mut characters = CharacterProperties::default();
        let mut paragraphs = BlockProperties::default();
        let mut clear_characters = BTreeSet::new();
        let mut clear_paragraphs = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for (property, value) in values {
            if !seen.insert(property)
                || super::super::style::CANVAS_STYLE_PROPERTIES.contains(&property)
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let character = super::super::style::is_character_property(property);
            match (character, value) {
                (true, Some(value)) => super::super::style::set_character_property(
                    &StyleId::from("Direct"),
                    &mut characters,
                    property,
                    &value,
                )?,
                (false, Some(value)) => super::super::style::set_block_property(
                    &StyleId::from("Direct"),
                    &mut paragraphs,
                    property,
                    &value,
                )?,
                (true, None) => {
                    clear_characters.insert(property);
                }
                (false, None) => {
                    clear_paragraphs.insert(property);
                }
            }
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        // HTML paragraph direction also supplies inherited character direction;
        // clear that shared declaration before clearing inline overrides.
        for stage in 0..4 {
            let target = TextRange::new(
                scratch.text_point(range.start)?,
                scratch.text_point(range.end)?,
            )?;
            let intent = match stage {
                0 if !clear_paragraphs.is_empty() => {
                    PersistedStyleIntent::ClearDirectBlockProperties {
                        target: StyleBlockTarget::Paragraphs(target),
                        properties: clear_paragraphs.clone(),
                    }
                }
                1 if !clear_characters.is_empty() => {
                    PersistedStyleIntent::ClearDirectCharacterProperties {
                        range: target,
                        properties: clear_characters.clone(),
                    }
                }
                2 if !characters.declared_properties().is_empty() => {
                    PersistedStyleIntent::SetDirectCharacterProperties {
                        range: target,
                        properties: characters.clone(),
                    }
                }
                3 if !paragraphs.declared_properties().is_empty() => {
                    PersistedStyleIntent::SetDirectBlockProperties {
                        target: StyleBlockTarget::Paragraphs(target),
                        properties: paragraphs.clone(),
                    }
                }
                _ => continue,
            };
            let prepared = scratch.prepare_persisted_style_intent(intent)?;
            let mapped = |point, association| -> Result<usize, ModelTransactionError> {
                Ok(prepared
                    .text_position_map()
                    .map_text_point(
                        point,
                        association,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )?
                    .value()
                    .ok_or(DocumentError::AmbiguousProjection)?
                    .offset())
            };
            range = mapped(target.start(), Association::BeforeInsertion)?
                ..mapped(target.end(), Association::AfterInsertion)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            formatted.record_formatted(&prepared)?;
            scratch.commit_model_transaction(prepared)?;
        }
        let patches = sources.source_patches();
        let edits = formatted.formatted_edits();
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }
}
