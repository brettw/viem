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
            self.views.get_mut(&view_id).expect("validated view").commands
                .install_typing_style(Some(named), values);
        }
    }
}
