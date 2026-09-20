//! Pending character declarations are view policy, never empty source syntax.
use super::*;
use crate::document::{ResolvedCharacterStyle, StyleId, StyleProperty, StylePropertyValue};
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TypingStyle {
    pub named: Option<StyleId>,
    pub values: Vec<(StyleProperty, StylePropertyValue)>,
    pub inherited: Option<crate::document::ReplacementTypingContext>,
}
impl TypingStyle {
    pub fn is_empty(&self) -> bool {
        self.named.is_none() && self.values.is_empty() && self.inherited.is_none()
    }
    pub fn for_repeat(&self) -> Self {
        Self { named: self.named.clone(), values: self.values.clone(), inherited: None }
    }
}
// Values enter only after finite-value validation by Document.
impl Eq for TypingStyle {}
impl CommandInterpreter {
    pub(crate) fn restore_typing_style(
        &mut self,
        named: Option<StyleId>,
        values: Vec<(StyleProperty, StylePropertyValue)>,
        inherited: Option<crate::document::ReplacementTypingContext>,
    ) {
        self.typing_style = TypingStyle { named, values, inherited };
    }
    /// Publish validated pending style after a native paragraph transaction,
    /// retaining its place in the insert-session repeat program.
    pub(crate) fn install_typing_style(
        &mut self,
        named: Option<StyleId>,
        values: Vec<(StyleProperty, StylePropertyValue)>,
    ) {
        self.restore_typing_style(named, values, None);
        if let Some(session) = self.insert_session.as_mut() {
            if !session.replaying_program {
                if let Some(program) = session.repeat_program.as_mut() {
                    program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
                }
            }
        }
    }
    pub(crate) fn typing_named_style(&self) -> Option<&StyleId> {
        self.typing_style.named.as_ref()
    }
    pub(crate) fn typing_properties(&self) -> &[(StyleProperty, StylePropertyValue)] {
        &self.typing_style.values
    }
    pub(crate) fn typing_inherited_context(&self) -> Option<&crate::document::ReplacementTypingContext> {
        self.typing_style.inherited.as_ref()
    }
    pub fn set_typing_named_style(
        &mut self,
        document: &Document,
        style: StyleId,
    ) -> Result<(), DocumentError> {
        if !matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Replace) {
            return Err(DocumentError::UnsupportedFormatting);
        }
        document.validate_typing_named_style(&style)?;
        if self.typing_style.named.as_ref() != Some(&style) || !self.typing_style.values.is_empty() {
            self.typing_style.named = Some(style);
            self.typing_style.values.clear();
            if let Some(session) = self.insert_session.as_mut() {
                if !session.replaying_program {
                    if let Some(program) = session.repeat_program.as_mut() {
                        program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
                    }
                }
            }
        }
        Ok(())
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
        let exit = document
            .html_typing_exit(self.cursor, &next.values)?
            .or(document.markdown_source_typing_exit(self.cursor, &next.values)?);
        if let Some((at, preserved)) = exit {
            // A combining scalar can attach to a visible closing delimiter.
            // Never move a source caret into that grapheme's interior.
            document.text_point(at)?;
            for (property, value) in preserved {
                if !next.values.iter().any(|(current, _)| *current == property) {
                    next.values.push((property, value));
                }
            }
            // This is a presentation-only movement across closing markup;
            // no source transaction or empty undo entry is created.
            self.cursor = at;
            self.position_revision = Some(document.revision());
            self.boundary_affinity = if at == document.projection().text_tree().byte_len() {
                BoundaryAffinity::Upstream
            } else {
                BoundaryAffinity::Downstream
            };
            self.visual_position = None;
            self.desired_x = None;
            self.preferred_column = None;
        }
        if next != self.typing_style {
            if let Some(session) = self.insert_session.as_mut() {
                if !session.replaying_program {
                    if let Some(program) = session.repeat_program.as_mut() {
                        program.push(EditSessionStep::TypingStyle(next.for_repeat()));
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
                program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
            }
        }
    }
    pub fn clear_typing_properties(&mut self) {
        if self.typing_style.values.is_empty() {
            return;
        }
        self.typing_style.values.clear();
        if let Some(session) = self.insert_session.as_mut() {
            if !session.replaying_program {
                if let Some(program) = session.repeat_program.as_mut() {
                    program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
                }
            }
        }
    }
    pub fn apply_typing_presentation(
        &self,
        document: &Document,
        style: &mut ResolvedCharacterStyle,
    ) -> Result<(), DocumentError> {
        if let Some(context) = &self.typing_style.inherited {
            *style = context.character.clone();
        }
        if let Some(named) = &self.typing_style.named {
            *style = document.typing_named_style_at(
                self.cursor,
                self.insertion_boundary_affinity(),
                named,
            )?;
        }
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
                (StyleProperty::CharacterScriptPosition, V::ScriptPosition(v)) => style.script_position = *v,
                _ => {}
            }
        }
        Ok(())
    }
}
