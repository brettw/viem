//! Pending character declarations are view policy, never empty source syntax.
use super::*;
use crate::document::{ResolvedCharacterStyle, StyleId, StyleProperty, StylePropertyValue};
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TypingStyle {
    pub named: Option<StyleId>,
    pub values: Vec<(StyleProperty, StylePropertyValue)>,
    pub inherited: Option<crate::document::ReplacementTypingContext>,
    pub link_disabled: bool,
}
impl TypingStyle {
    pub fn is_empty(&self) -> bool {
        self.named.is_none()
            && self.values.is_empty()
            && self.inherited.is_none()
            && !self.link_disabled
    }
    pub fn for_repeat(&self) -> Self {
        Self {
            named: self.named.clone(),
            values: self.values.clone(),
            inherited: None,
            link_disabled: self.link_disabled,
        }
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
        self.typing_style = TypingStyle {
            named,
            values,
            inherited,
            link_disabled: false,
        };
    }
    /// Publish validated pending style after a native formatting transaction,
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
    pub(crate) fn typing_inherited_context(
        &self,
    ) -> Option<&crate::document::ReplacementTypingContext> {
        self.typing_style.inherited.as_ref()
    }
    pub(crate) fn typing_link_disabled(&self) -> bool {
        self.typing_style.link_disabled
    }
    pub(crate) fn restore_typing_link_disabled(&mut self, disabled: bool) {
        self.typing_style.link_disabled = disabled;
    }
    pub(crate) fn set_typing_link_disabled(
        &mut self,
        document: &Document,
    ) -> Result<(), DocumentError> {
        if !matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Replace)
            || !document.can_exit_link_typing(
                self.cursor,
                if self.mode == Mode::Normal {
                    BoundaryAffinity::Downstream
                } else {
                    self.insertion_boundary_affinity()
                },
            )
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let current = crate::layout::DocumentLayoutStyles::character_at(
            document.projection(),
            self.cursor,
            self.mode != Mode::Normal
                && self.insertion_boundary_affinity() == BoundaryAffinity::Upstream,
        )
        .map_err(|_| DocumentError::AmbiguousProjection)?;
        let mut next = self.typing_style.clone();
        let affinity = if self.mode == Mode::Normal {
            BoundaryAffinity::Downstream
        } else {
            self.insertion_boundary_affinity()
        };
        if next.named.is_none() && document.is_code_at(self.cursor, affinity)? {
            next.named = Some("Code".into());
        }
        let authored_current = crate::layout::DocumentLayoutStyles::semantic_character_at(
            document.projection(),
            self.cursor,
            self.mode != Mode::Normal
                && self.insertion_boundary_affinity() == BoundaryAffinity::Upstream,
        )
        .map_err(|_| DocumentError::AmbiguousProjection)?;
        use StylePropertyValue as V;
        for value in [
            (StyleProperty::CharacterBold, V::Boolean(current.bold)),
            (StyleProperty::CharacterSlant, V::FontSlant(current.slant)),
            (
                StyleProperty::CharacterUnderline,
                V::Boolean(authored_current.underline),
            ),
            (
                StyleProperty::CharacterSuperscript,
                V::Boolean(authored_current.superscript),
            ),
            (
                StyleProperty::CharacterSubscript,
                V::Boolean(authored_current.subscript),
            ),
            (
                StyleProperty::CharacterStrikethrough,
                V::Boolean(current.strikethrough),
            ),
        ] {
            if !next.values.iter().any(|(property, _)| *property == value.0) {
                next.values.push(value);
            }
        }
        next.link_disabled = true;
        self.typing_style = next;
        if let Some(program) = self
            .insert_session
            .as_mut()
            .and_then(|session| session.repeat_program.as_mut())
        {
            program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
        }
        Ok(())
    }
    pub fn set_typing_named_style(
        &mut self,
        document: &Document,
        style: StyleId,
    ) -> Result<(), DocumentError> {
        if !matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Replace) {
            return Err(DocumentError::UnsupportedFormatting);
        }
        document.validate_typing_named_style_at(
            self.cursor,
            if self.mode == Mode::Normal {
                BoundaryAffinity::Downstream
            } else {
                self.insertion_boundary_affinity()
            },
            &style,
        )?;
        if self.typing_style.named.as_ref() != Some(&style) || !self.typing_style.values.is_empty()
        {
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
        if !matches!(self.mode, Mode::Normal | Mode::Insert | Mode::Replace) {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut next = self.typing_style.clone();
        for (property, value) in values {
            if value == StylePropertyValue::Boolean(true) {
                let opposite = match property {
                    StyleProperty::CharacterSuperscript => Some(StyleProperty::CharacterSubscript),
                    StyleProperty::CharacterSubscript => Some(StyleProperty::CharacterSuperscript),
                    _ => None,
                };
                if let Some(opposite) = opposite {
                    next.values.retain(|(p, _)| *p != opposite);
                    next.values
                        .push((opposite, StylePropertyValue::Boolean(false)));
                }
            }
            next.values.retain(|(p, _)| *p != property);
            next.values.push((property, value));
        }
        document.validate_typing_properties_at(
            self.cursor,
            if self.mode == Mode::Normal {
                BoundaryAffinity::Downstream
            } else {
                self.insertion_boundary_affinity()
            },
            &next.values,
        )?;
        let exit = document.markdown_source_typing_exit(self.cursor, &next.values)?;
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
                (StyleProperty::CharacterSlant, V::FontSlant(v)) => style.slant = *v,
                (StyleProperty::CharacterUnderline, V::Boolean(v)) => style.underline = *v,
                (StyleProperty::CharacterSuperscript, V::Boolean(v)) => {
                    style.superscript = *v;
                    if *v {
                        style.subscript = false;
                    }
                }
                (StyleProperty::CharacterSubscript, V::Boolean(v)) => {
                    style.subscript = *v;
                    if *v {
                        style.superscript = false;
                    }
                }
                (StyleProperty::CharacterStrikethrough, V::Boolean(v)) => style.strikethrough = *v,
                _ => {}
            }
        }
        Ok(())
    }

    /// Native character controls start typing without synthesizing Vim input.
    /// Call only after validating the requested view-local declaration.
    pub(crate) fn begin_native_character_typing(&mut self, document: &mut Document) -> bool {
        if self.mode != Mode::Normal {
            return false;
        }
        let pending = self.typing_style.clone();
        self.enter_insert(document, InsertPlacement::Before, 1);
        self.typing_style = pending;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        if !self.typing_style.is_empty() {
            if let Some(program) = self
                .insert_session
                .as_mut()
                .and_then(|session| session.repeat_program.as_mut())
            {
                program.steps.clear();
                program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
            }
        }
        true
    }
}
