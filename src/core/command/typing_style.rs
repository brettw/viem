//! Pending character declarations are view policy, never empty source syntax.
use super::*;
use crate::document::{ResolvedCharacterStyle, StyleProperty, StylePropertyValue};
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TypingStyle {
    pub values: Vec<(StyleProperty, StylePropertyValue)>,
}
// Values enter only after finite-value validation by Document.
impl Eq for TypingStyle {}
impl CommandInterpreter {
    pub(crate) fn restore_typing_properties(
        &mut self,
        values: Vec<(StyleProperty, StylePropertyValue)>,
    ) {
        self.typing_style.values = values;
    }
    pub(crate) fn typing_properties(&self) -> &[(StyleProperty, StylePropertyValue)] {
        &self.typing_style.values
    }
    pub fn set_typing_properties(
        &mut self,
        document: &Document,
        values: Vec<(StyleProperty, StylePropertyValue)>,
    ) -> Result<(), DocumentError> {
        if !matches!(self.mode, Mode::Insert | Mode::Replace) {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut next = self.typing_style.clone();
        for (property, value) in values {
            next.values.retain(|(p, _)| *p != property);
            next.values.push((property, value));
        }
        document.validate_typing_properties(&next.values)?;
        if document.format() == crate::document::Format::HtmlSource
            && !document.html_source_prose_at(self.cursor, self.insertion_boundary_affinity())?
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        if next != self.typing_style {
            if let Some(session) = self.insert_session.as_mut() {
                if !session.replaying_program {
                    if let Some(program) = session.repeat_program.as_mut() {
                        program.push(EditSessionStep::TypingStyle(next.clone()));
                    }
                }
            }
            self.typing_style = next;
        }
        Ok(())
    }
    pub fn clear_typing_property(&mut self, property: StyleProperty) {
        self.typing_style.values.retain(|(p, _)| *p != property);
        if let Some(session) = self.insert_session.as_mut() {
            if let Some(program) = session.repeat_program.as_mut() {
                program.push(EditSessionStep::TypingStyle(self.typing_style.clone()));
            }
        }
    }
    pub fn clear_typing_properties(&mut self) {
        self.typing_style = Default::default();
    }
    pub fn apply_typing_presentation(&self, style: &mut ResolvedCharacterStyle) {
        use StylePropertyValue as V;
        for (property, value) in &self.typing_style.values {
            match (property, value) {
                (StyleProperty::CharacterBold, V::Boolean(v)) => {
                    style.bold = *v;
                    style.weight = if *v {
                        style.base_weight.saturating_add(300).min(1000)
                    } else {
                        style.base_weight
                    };
                }
                (StyleProperty::CharacterFontFamilies, V::FontFamilies(v)) => {
                    style.font_families = v.clone()
                }
                (StyleProperty::CharacterSize, V::Float(v)) => style.size = *v,
                (StyleProperty::CharacterWeight, V::FontWeight(v)) => {
                    style.base_weight = *v;
                    style.weight = if style.bold {
                        v.saturating_add(300).min(1000)
                    } else {
                        *v
                    };
                }
                (StyleProperty::CharacterSlant, V::FontSlant(v)) => style.slant = *v,
                (StyleProperty::CharacterForeground, V::Color(v)) => {
                    style.foreground = *v;
                    style.foreground_is_default = false;
                }
                (StyleProperty::CharacterBackground, V::Color(v)) => style.background = Some(*v),
                (StyleProperty::CharacterUnderline, V::Boolean(v)) => style.underline = *v,
                (StyleProperty::CharacterStrikethrough, V::Boolean(v)) => style.strikethrough = *v,
                (StyleProperty::CharacterLanguage, V::Text(v)) => style.language = Some(v.clone()),
                (StyleProperty::CharacterDirection, V::WritingDirection(v)) => style.direction = *v,
                (StyleProperty::CharacterOpenTypeFeatures, V::OpenTypeFeatures(v)) => {
                    style.open_type_features = v.clone()
                }
                (StyleProperty::CharacterLetterSpacing, V::Float(v)) => style.letter_spacing = *v,
                (StyleProperty::CharacterBaselineShift, V::Float(v)) => style.baseline_shift = *v,
                _ => {}
            }
        }
    }
}
