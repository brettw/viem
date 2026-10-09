//! Pending character formatting retained by native Select replacements.
use super::*;
use crate::document::{StyleProperty, StylePropertyValue};

impl<P: TextMeasurementProvider> Core<P> {
    /// Stage the named choice and explicit inline edit before mapping the
    /// selection; publish them only after the source transaction succeeds.
    pub(super) fn select_typing_after_character_property(
        &self,
        view_id: ViewId,
        property: StyleProperty,
        value: StylePropertyValue,
    ) -> Option<(StyleId, Vec<(StyleProperty, StylePropertyValue)>)> {
        let commands = &self.views[&view_id].commands;
        if !commands.is_text_selection() {
            return None;
        }
        let named = commands.typing_named_style()?.clone();
        let mut pending = commands.typing_properties().to_vec();
        if value == StylePropertyValue::Boolean(true) {
            let opposite = match property {
                StyleProperty::CharacterSuperscript => Some(StyleProperty::CharacterSubscript),
                StyleProperty::CharacterSubscript => Some(StyleProperty::CharacterSuperscript),
                _ => None,
            };
            if let Some(opposite) = opposite {
                pending.retain(|(current, _)| *current != opposite);
                pending.push((opposite, StylePropertyValue::Boolean(false)));
            }
        }
        pending.retain(|(current, _)| *current != property);
        pending.push((property, value));
        Some((named, pending))
    }

    pub(super) fn publish_select_typing(
        &mut self,
        view_id: ViewId,
        pending: Option<(StyleId, Vec<(StyleProperty, StylePropertyValue)>)>,
    ) {
        if let Some((named, values)) = pending {
            self.views
                .get_mut(&view_id)
                .expect("validated view")
                .commands
                .install_typing_style(Some(named), values);
        }
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn selection_inline_property_state(
        &self,
        view_id: ViewId,
        property: StyleProperty,
    ) -> Result<SemanticStyleState, CoreError> {
        if !matches!(
            property,
            StyleProperty::CharacterUnderline
                | StyleProperty::CharacterSuperscript
                | StyleProperty::CharacterSubscript
        ) {
            return Err(CoreError::StaleLogicalSelection);
        }
        let selection = self.list_selection_identity(view_id)?;
        if selection.kind() == LogicalSelectionKind::None {
            return Ok(
                if crate::document::resolved_inline_boolean(
                    &self.selected_character_style(view_id)?,
                    property,
                ) {
                    SemanticStyleState::On
                } else {
                    SemanticStyleState::Off
                },
            );
        }
        let ranges = if let Some(selected) = self.table_selection(view_id)? {
            let table = self
                .document
                .projection()
                .tables()
                .iter()
                .find(|table| table.id == selected.table)
                .expect("validated table");
            selected
                .rows()
                .flat_map(|row| {
                    selected
                        .columns()
                        .map(move |column| table.rows[row].cells[column].range.clone())
                })
                .collect::<Vec<_>>()
        } else {
            vec![selection.range()]
        };
        let mut on = false;
        let mut off = false;
        for range in ranges {
            let spans = self.document.projection().style_spans_for_region(&range);
            let mut boundaries = std::collections::BTreeSet::from([range.start, range.end]);
            for span in spans {
                boundaries.insert(span.range.start.max(range.start));
                boundaries.insert(span.range.end.min(range.end));
            }
            for block in self.document.projection().blocks_for_region(&range) {
                boundaries.insert(block.range.start.max(range.start));
                boundaries.insert(block.range.end.min(range.end));
            }
            let boundaries: Vec<_> = boundaries.into_iter().collect();
            for pair in boundaries.windows(2) {
                if pair[0] == pair[1]
                    || self
                        .document
                        .projection()
                        .text_tree()
                        .byte_chunk_at(pair[0])
                        .first()
                        == Some(&b'\n')
                {
                    continue;
                }
                let style =
                    DocumentLayoutStyles::character_at(self.document.projection(), pair[0], false)
                        .map_err(LayoutError::from)?;
                if crate::document::resolved_inline_boolean(&style, property) {
                    on = true;
                } else {
                    off = true;
                }
                if on && off {
                    return Ok(SemanticStyleState::Mixed);
                }
            }
        }
        Ok(if on {
            SemanticStyleState::On
        } else {
            SemanticStyleState::Off
        })
    }
}
