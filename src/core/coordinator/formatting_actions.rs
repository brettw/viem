//! Availability and state of the formatting actions that native toolbars,
//! menus and accelerators offer. Both frontends present this one decision
//! instead of deriving their own from structural selection facts.
use super::*;
use crate::document::{AlertKind, SelectedNamedStyles};

/// Stable identities, shared with the C ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormattingAction {
    Bold = 1,
    Italic,
    Underline,
    Strikethrough,
    Superscript,
    Subscript,
    /// Inline Code, a character style.
    Code,
    CodeBlock,
    BlockQuote,
    AlertNote,
    AlertTip,
    AlertImportant,
    AlertWarning,
    AlertCaution,
    BulletedList,
    NumberedList,
    TaskList,
    RemoveList,
    Indent,
    Unindent,
    ListParagraph,
    /// Base Paragraph.
    Heading0,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    /// The link toggle: edit a reference, exit or remove a link, or insert one.
    Link,
    Reference,
    Image,
    Table,
    /// Assigning any paragraph style that the style sheet allows.
    ParagraphStyles,
    /// Assigning any character style that the style sheet allows.
    CharacterStyles,
}

impl FormattingAction {
    pub const ALL: [Self; 34] = {
        use FormattingAction::*;
        [Bold, Italic, Underline, Strikethrough, Superscript, Subscript, Code, CodeBlock, BlockQuote,
            AlertNote, AlertTip, AlertImportant, AlertWarning, AlertCaution,
            BulletedList, NumberedList, TaskList, RemoveList, Indent, Unindent, ListParagraph,
            Heading0, Heading1, Heading2, Heading3, Heading4, Heading5, Heading6,
            Link, Reference, Image, Table, ParagraphStyles, CharacterStyles]
    };

    pub fn id(self) -> u32 {
        self as u32
    }

    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.get(id.checked_sub(1)? as usize).copied()
    }

    pub fn alert(kind: AlertKind) -> Self {
        Self::ALL[Self::AlertNote as usize - 1 + kind.ordinal() as usize - 1]
    }

    /// Level 0 is Base Paragraph.
    pub fn heading(level: u8) -> Option<Self> {
        (level <= 6).then(|| Self::ALL[Self::Heading0 as usize - 1 + level as usize])
    }

    const fn bit(self) -> u64 {
        1 << self as u32
    }
}

/// A set of formatting actions, as bits `1 << id`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FormattingActionSet(u64);

impl FormattingActionSet {
    pub const ALL: Self = {
        let mut bits = 0;
        let mut index = 0;
        while index < FormattingAction::ALL.len() {
            bits |= FormattingAction::ALL[index].bit();
            index += 1;
        }
        Self(bits)
    };

    /// Unknown bits are ignored.
    pub fn from_bits(bits: u64) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub fn of(actions: &[FormattingAction]) -> Self {
        Self(actions.iter().fold(0, |bits, action| bits | action.bit()))
    }

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn contains(self, action: FormattingAction) -> bool {
        self.0 & action.bit() != 0
    }

    fn intersects(self, actions: &[FormattingAction]) -> bool {
        self.0 & Self::of(actions).0 != 0
    }

    fn insert(&mut self, action: FormattingAction) {
        self.0 |= action.bit();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FormattingActions {
    enabled: FormattingActionSet,
    on: FormattingActionSet,
    mixed: FormattingActionSet,
}

impl FormattingActions {
    pub fn is_enabled(&self, action: FormattingAction) -> bool {
        self.enabled.contains(action)
    }

    pub fn state(&self, action: FormattingAction) -> SemanticStyleState {
        if self.on.contains(action) {
            SemanticStyleState::On
        } else if self.mixed.contains(action) {
            SemanticStyleState::Mixed
        } else {
            SemanticStyleState::Off
        }
    }

    pub fn enabled(&self) -> FormattingActionSet {
        self.enabled
    }

    pub fn on(&self) -> FormattingActionSet {
        self.on
    }

    pub fn mixed(&self) -> FormattingActionSet {
        self.mixed
    }

    fn set(&mut self, action: FormattingAction, enabled: bool, state: SemanticStyleState) {
        if enabled {
            self.enabled.insert(action);
        }
        match state {
            SemanticStyleState::On => self.on.insert(action),
            SemanticStyleState::Mixed => self.mixed.insert(action),
            SemanticStyleState::Off => {}
        }
    }
}

fn on_if(value: bool) -> SemanticStyleState {
    if value { SemanticStyleState::On } else { SemanticStyleState::Off }
}

/// Present everywhere is on; present beside anything else is mixed.
fn membership(present: bool, other: bool) -> SemanticStyleState {
    match (present, other) {
        (false, _) => SemanticStyleState::Off,
        (true, false) => SemanticStyleState::On,
        (true, true) => SemanticStyleState::Mixed,
    }
}

fn alert_state(named: &SelectedNamedStyles, kind: AlertKind) -> SemanticStyleState {
    let bit = 1u8 << (kind.ordinal() - 1);
    membership(named.alerts & bit != 0, named.alerts != bit || named.has_plain_quote || named.has_non_quote)
}

/// Actions whose availability or state reads the selected named styles.
const NAMED: &[FormattingAction] = {
    use FormattingAction::*;
    &[Underline, Strikethrough, Superscript, Subscript, Code, CodeBlock, BlockQuote,
        AlertNote, AlertTip, AlertImportant, AlertWarning, AlertCaution,
        BulletedList, NumberedList, TaskList, RemoveList, Indent, Unindent,
        Heading0, Heading1, Heading2, Heading3, Heading4, Heading5, Heading6,
        ParagraphStyles, CharacterStyles]
};

impl<P: TextMeasurementProvider> Core<P> {
    /// Evaluate the requested actions for the view's current selection.
    /// Others report disabled and off, so validating one menu item costs no
    /// more than the query it needs. Markdown Preview alone offers them.
    pub fn formatting_actions(
        &self,
        view_id: ViewId,
        requested: FormattingActionSet,
    ) -> Result<FormattingActions, CoreError> {
        self.focus_view(view_id);
        use FormattingAction::*;
        let view = self.views.get(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let mut result = FormattingActions::default();
        let Ok(selection) = self.list_selection_identity(view_id) else { return Ok(result) };
        // A rectangular selection has no formatting identity.
        if !self.document.format().is_markdown() || selection.kind() == LogicalSelectionKind::Block {
            return Ok(result);
        }
        let commands = &view.commands;
        // Pending Vim grammar, prompts, literal-next and composition own the
        // next input before native formatting does.
        let input = view.composition.is_none() && !commands.command_input_pending() && commands.mode() != Mode::CommandLine;
        // `readonly` only gates saves; `nomodifiable` gates every edit.
        let editable = input && self.document.is_modifiable();

        for (action, style) in [(Bold, SemanticInlineStyle::Strong), (Italic, SemanticInlineStyle::Emphasis)] {
            if !requested.contains(action) { continue; }
            let Ok(presentation) = self.selection_semantic_style_presentation(view_id, style) else { continue };
            let target = presentation.selection().is_some_and(|selection|
                selection.kind() == LogicalSelectionKind::None || !selection.range().is_empty());
            let capable = if presentation.state() == SemanticStyleState::On { presentation.can_clear() }
                else { presentation.can_set() };
            result.set(action, editable && target && capable, presentation.state());
        }

        if requested.intersects(NAMED) {
            let Ok(named) = self.selected_named_styles(view_id) else { return Ok(result) };
            // At a caret, character actions stage pending typing choices,
            // which the document refuses where it refuses typing styles.
            let affinity = if commands.mode() == Mode::Normal { BoundaryAffinity::Downstream }
                else { commands.insertion_boundary_affinity() };
            let typing = selection.kind() != LogicalSelectionKind::None
                || self.document.validate_typing_properties_at(commands.cursor(), affinity, &[]).is_ok();
            let character_styles = editable && typing && !named.has_code_block;
            let paragraph_styles = editable && selection.kind() != LogicalSelectionKind::Cells && !named.has_table;
            result.set(ParagraphStyles, paragraph_styles, SemanticStyleState::Off);
            result.set(CharacterStyles, character_styles, SemanticStyleState::Off);
            for (action, property) in [
                (Underline, StyleProperty::CharacterUnderline),
                (Superscript, StyleProperty::CharacterSuperscript),
                (Subscript, StyleProperty::CharacterSubscript),
            ] {
                if requested.contains(action) {
                    let state = self.selection_inline_property_state(view_id, property).unwrap_or(SemanticStyleState::Off);
                    result.set(action, character_styles, state);
                }
            }
            if requested.contains(Strikethrough) {
                let state = self.selection_strikethrough_state(view_id).unwrap_or(SemanticStyleState::Off);
                result.set(Strikethrough, character_styles, state);
            }
            let character = |id: &str| named.character.as_ref().is_some_and(|style| style.0 == id);
            let paragraph = |id: &str| named.paragraph.as_ref().is_some_and(|style| style.0 == id);
            result.set(Code, character_styles, on_if(character("Code")));
            result.set(CodeBlock, paragraph_styles, on_if(paragraph("Code Block")));
            result.set(BlockQuote, paragraph_styles && !named.has_code_block,
                membership(named.has_quotes, named.has_non_quote));
            if requested.intersects(&[AlertNote, AlertTip, AlertImportant, AlertWarning, AlertCaution]) {
                let can_set = self.can_set_alert(view_id)?;
                for kind in AlertKind::ALL {
                    // Choosing the selection's only kind again restores a plain quote.
                    let state = alert_state(&named, kind);
                    result.set(FormattingAction::alert(kind),
                        paragraph_styles && !named.has_code_block && (can_set || state == SemanticStyleState::On), state);
                }
            }
            result.set(BulletedList, paragraph_styles,
                membership(named.has_bullets, named.has_numbering || named.has_tasks || named.has_non_list));
            result.set(NumberedList, paragraph_styles,
                membership(named.has_numbering, named.has_bullets || named.has_tasks || named.has_non_list));
            if requested.contains(TaskList) {
                result.set(TaskList, paragraph_styles && self.can_set_task_list(view_id)?,
                    membership(named.has_tasks, named.has_bullets || named.has_numbering || named.has_non_list));
            }
            result.set(RemoveList, paragraph_styles, SemanticStyleState::Off);
            if paragraph_styles && requested.intersects(&[Indent, Unindent]) {
                let (indent, unindent) = self.document.list_indent_capabilities(selection.range());
                result.set(Indent, indent, SemanticStyleState::Off);
                result.set(Unindent, unindent, SemanticStyleState::Off);
            }
            for level in 0..=6 {
                let id = if level == 0 { "Paragraph".to_owned() } else { format!("Heading{level}") };
                let defined = self.document.projection().style_sheet().block_style(&StyleId::from(id.as_str())).is_some();
                result.set(FormattingAction::heading(level).expect("standard level"), paragraph_styles && defined,
                    on_if(paragraph(&id)));
            }
        }

        if requested.contains(ListParagraph) {
            result.set(ListParagraph, input && self.can_insert_list_paragraph(view_id)?, SemanticStyleState::Off);
        }

        if requested.intersects(&[Link, Reference]) {
            if let Ok(context) = self.link_context(view_id) {
                let reference = context.reference.as_ref().map(|reference| reference.editable);
                let link = reference.unwrap_or(if !context.linked { context.can_insert }
                    else if context.selection.range().is_empty() { context.can_exit_link }
                    else { context.can_remove_selection });
                result.set(Link, editable && link, on_if(context.linked || reference.is_some()));
                result.set(Reference, editable && reference.unwrap_or(context.link.is_none() && context.can_insert),
                    on_if(reference.is_some()));
            }
        }

        if requested.contains(Image) {
            if let Ok(context) = self.image_context(view_id) {
                let range = context.selection.range();
                // The caret on an image, or a selection of exactly it, is that image.
                let active = context.image.as_ref().is_some_and(|image|
                    commands.mode() == Mode::Normal && range.is_empty() || range == image.range);
                result.set(Image, editable && !active && context.can_insert, on_if(active));
            }
        }

        if requested.contains(Table) {
            let can_insert = selection.kind() != LogicalSelectionKind::Cells
                && self.document.projection().table_cell_at(commands.cursor()).is_none()
                && self.document.can_insert_table(selection.range());
            result.set(Table, editable && can_insert, SemanticStyleState::Off);
        }

        // Unrequested actions computed alongside requested ones stay unreported.
        let mask = requested.bits();
        Ok(FormattingActions {
            enabled: FormattingActionSet(result.enabled.0 & mask),
            on: FormattingActionSet(result.on.0 & mask),
            mixed: FormattingActionSet(result.mixed.0 & mask),
        })
    }
}
