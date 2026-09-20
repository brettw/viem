//! Selection formatting policy shared by native menus and persistent panels.
use super::*;
use crate::document::{
    BlockProperties, CharacterProperties, ResolvedParagraphStyle, StyleProperty, StylePropertyValue,
};

impl<P: TextMeasurementProvider> Core<P> {
    /// A named choice retained for Select replacement also retains subsequent
    /// explicit character edits. Stage before mapping the selection and publish
    /// only after its source edit succeeds.
    pub(super) fn select_typing_after_character_properties(
        &self,
        view_id: ViewId,
        values: &[(StyleProperty, Option<StylePropertyValue>)],
    ) -> Option<(StyleId, Vec<(StyleProperty, StylePropertyValue)>)> {
        let commands = &self.views[&view_id].commands;
        if !commands.is_text_selection() {
            return None;
        }
        let named = commands.typing_named_style()?.clone();
        let mut pending = commands.typing_properties().to_vec();
        for (property, value) in values {
            if crate::document::is_character_property(*property) {
                pending.retain(|(current, _)| current != property);
                if let Some(value) = value {
                    pending.push((*property, value.clone()));
                }
            }
        }
        Some((named, pending))
    }

    pub(super) fn select_typing_after_direct_request(
        &self, view_id: ViewId, request: &ModelRequest,
    ) -> Option<(StyleId, Vec<(StyleProperty, StylePropertyValue)>)> {
        match request {
            ModelRequest::SetDirectCharacterProperties { values, .. } => {
                let values = values.iter().map(|(property, value)| (*property, Some(value.clone()))).collect::<Vec<_>>();
                self.select_typing_after_character_properties(view_id, &values)
            }
            ModelRequest::EditDirectProperty { property, value, .. } => {
                self.select_typing_after_character_properties(view_id, &[(*property, value.clone())])
            }
            ModelRequest::EditDirectProperties { values, .. } => {
                self.select_typing_after_character_properties(view_id, values)
            }
            _ => None,
        }
    }

    pub(super) fn publish_select_typing(
        &mut self, view_id: ViewId,
        pending: Option<(StyleId, Vec<(StyleProperty, StylePropertyValue)>)>,
    ) {
        if let Some((named, values)) = pending {
            self.views.get_mut(&view_id).expect("validated view").commands
                .install_typing_style(Some(named), values);
        }
    }

    pub fn selected_paragraph_style(
        &self,
        view_id: ViewId,
    ) -> Result<ResolvedParagraphStyle, CoreError> {
        let selection = self.list_selection_identity(view_id)?;
        self.paragraph_style_at_offset(selection.range().start)
    }

    pub(crate) fn paragraph_style_at_offset(
        &self,
        at: usize,
    ) -> Result<ResolvedParagraphStyle, CoreError> {
        self.document.text_point(at)?;
        let projection = self.document.projection();
        let blocks = if self.document.format().is_source_view() {
            projection
                .flow_blocks_for_region(&(at..at))
                .unwrap_or_else(|| projection.blocks_for_region(&(at..at)))
        } else {
            projection.blocks_for_region(&(at..at))
        };
        let block = blocks
            .iter()
            .find(|block| block.range.contains(&at))
            .or_else(|| blocks.iter().find(|block| block.range.start == at))
            .or_else(|| blocks.last());
        let Some(block) = block else {
            return Ok(ResolvedParagraphStyle::default());
        };
        let default_block = BlockProperties::default();
        let default_character = CharacterProperties::default();
        projection
            .style_sheet()
            .resolve_assigned_paragraph_style(
                projection.document_style(),
                &block.style,
                block
                    .direct_formatting
                    .as_ref()
                    .map_or(&default_block, |direct| &direct.direct_paragraph),
                block
                    .direct_formatting
                    .as_ref()
                    .map_or(&default_character, |direct| {
                        &direct.direct_default_character
                    }),
                None,
                &default_character,
            )
            .map_err(|error| CoreError::ModelTransaction(error.into()))
    }

    pub(super) fn edit_direct_properties(
        &mut self,
        view_id: ViewId,
        expected: LogicalSelectionIdentity,
        values: Vec<(StyleProperty, Option<StylePropertyValue>)>,
    ) -> Result<CoreOutcome, CoreError> {
        if self.list_selection_identity(view_id)? != expected {
            return Err(CoreError::StaleLogicalSelection);
        }
        if expected.kind() == LogicalSelectionKind::Block {
            return Err(CoreError::Document(DocumentError::UnsupportedFormatting));
        }
        let mut seen = std::collections::BTreeSet::new();
        if values.is_empty()
            || values.len() > 32
            || values.iter().any(|(property, _)| {
                *property < StyleProperty::ParagraphSpacingBefore || !seen.insert(*property)
            })
        {
            return Err(CoreError::Document(DocumentError::UnsupportedFormatting));
        }
        let has_characters = values
            .iter()
            .any(|(property, _)| crate::document::is_character_property(*property));
        if expected.kind() == LogicalSelectionKind::None && has_characters {
            // A collapsed character edit changes pending typing declarations.
            // Validate all fields before either paragraph or typing publication.
            let commands = &self.views[&view_id].commands;
            if !matches!(commands.mode(), Mode::Insert | Mode::Replace) {
                return Err(CoreError::Document(DocumentError::UnsupportedFormatting));
            }
            let previous_cursor = commands.cursor();
            let mut candidate = commands.clone();
            let mut paragraphs = Vec::new();
            let mut characters = Vec::new();
            for (property, value) in values {
                if crate::document::is_character_property(property) {
                    if let Some(value) = value {
                        characters.push((property, value));
                    } else {
                        candidate.clear_typing_property(property);
                    }
                } else {
                    paragraphs.push((property, value));
                }
            }
            candidate.set_typing_properties(&self.document, characters)?;
            if paragraphs.is_empty() {
                self.views
                    .get_mut(&view_id)
                    .expect("validated view")
                    .commands = candidate;
                return self.pending_typing_outcome(view_id, previous_cursor);
            }
            let outcome = self.apply_native_model_request(
                view_id,
                ModelRequest::EditDirectProperties {
                    document: expected.document(),
                    revision: expected.revision(),
                    range: expected.range(),
                    values: paragraphs,
                },
            )?;
            let commands = &mut self
                .views
                .get_mut(&view_id)
                .expect("validated view")
                .commands;
            commands.install_typing_style(
                candidate.typing_named_style().cloned(),
                candidate.typing_properties().to_vec(),
            );
            return Ok(outcome);
        }
        self.apply_native_model_request(
            view_id,
            ModelRequest::EditDirectProperties {
                document: expected.document(),
                revision: expected.revision(),
                range: expected.range(),
                values,
            },
        )
    }
}
