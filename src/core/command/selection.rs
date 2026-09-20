//! Native selection policy and Vim Select-mode grammar share Visual extents.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionOrigin {
    Mouse,
    Key,
    Command,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionOptions {
    pub autoselect: bool,
    pub keymodel: String,
    pub selectmode: String,
}
impl Default for SelectionOptions {
    fn default() -> Self {
        Self {
            autoselect: true,
            keymodel: String::new(),
            selectmode: String::new(),
        }
    }
}
impl SelectionOptions {
    pub fn contains(&self, origin: SelectionOrigin) -> bool {
        let name = match origin {
            SelectionOrigin::Mouse => "mouse",
            SelectionOrigin::Key => "key",
            SelectionOrigin::Command => "cmd",
        };
        self.selectmode.split(',').any(|part| part == name)
    }
    pub fn set(&mut self, keymodel: bool, value: &str) -> bool {
        let allowed: &[&str] = if keymodel {
            &["startsel", "stopsel"]
        } else {
            &["mouse", "key", "cmd"]
        };
        let mut parts = Vec::new();
        if !value.is_empty() {
            for part in value.split(',') {
                if !allowed.contains(&part) {
                    return false;
                }
                if !parts.contains(&part) {
                    parts.push(part);
                }
            }
        }
        if keymodel {
            self.keymodel = parts.join(",");
        } else {
            self.selectmode = parts.join(",");
        }
        true
    }
    fn keymodel(&self, name: &str) -> bool {
        self.keymodel.split(',').any(|part| part == name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKey {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    Home,
    End,
    DocumentStart,
    DocumentEnd,
    PageUp,
    PageDown,
}
impl NavigationKey {
    pub fn key(self) -> Key {
        match self {
            Self::Left => Key::Left,
            Self::Right => Key::Right,
            Self::WordLeft => Key::WordLeft,
            Self::WordRight => Key::WordRight,
            Self::Up => Key::Up,
            Self::Down => Key::Down,
            Self::Home => Key::Home,
            Self::End => Key::End,
            Self::DocumentStart => Key::DocumentStart,
            Self::DocumentEnd => Key::DocumentEnd,
            Self::PageUp => Key::PageUp,
            Self::PageDown => Key::PageDown,
        }
    }
    pub fn from_key(key: Key) -> Option<Self> {
        Some(match key {
            Key::Left => Self::Left,
            Key::Right => Self::Right,
            Key::WordLeft => Self::WordLeft,
            Key::WordRight => Self::WordRight,
            Key::Up => Self::Up,
            Key::Down => Self::Down,
            Key::Home => Self::Home,
            Key::End => Self::End,
            Key::DocumentStart => Self::DocumentStart,
            Key::DocumentEnd => Self::DocumentEnd,
            Key::PageUp => Self::PageUp,
            Key::PageDown => Self::PageDown,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectionBehavior {
    Visual,
    Select,
    Native,
}

impl CommandInterpreter {
    pub fn is_select_mode(&self) -> bool {
        self.is_text_selection() && self.selection_behavior == SelectionBehavior::Select
    }
    pub fn is_native_selection(&self) -> bool {
        self.is_text_selection() && self.selection_behavior == SelectionBehavior::Native
    }
    pub fn is_text_selection(&self) -> bool {
        self.selection_behavior != SelectionBehavior::Visual
            && matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            )
    }
    pub fn selection_options(&self) -> &SelectionOptions {
        &self.selection_options
    }
    pub fn set_selection_origin(&mut self, origin: SelectionOrigin) {
        self.selection_behavior =
            if origin != SelectionOrigin::Command && self.selection_options.autoselect {
                SelectionBehavior::Native
            } else if self.selection_options.contains(origin) {
                SelectionBehavior::Select
            } else {
                SelectionBehavior::Visual
            };
    }
    pub(crate) fn set_native_selection_origin(
        &mut self,
        document: &Document,
        origin: SelectionOrigin,
        return_mode: Mode,
    ) {
        self.set_selection_origin(origin);
        if self.is_native_selection() {
            self.selection_return_mode = return_mode;
        }
        if origin != SelectionOrigin::Command
            && self.is_native_selection()
            && self.mode == Mode::VisualCharacter
            && !self.selection_exclusive
        {
            let forward = self.cursor >= self.visual_anchor.unwrap_or(self.cursor);
            let range = self.visual_extent(document).range;
            self.visual_anchor = Some(if forward { range.start } else { range.end });
            self.cursor = if forward { range.end } else { range.start };
            self.selection_exclusive = true;
        }
    }
    pub(super) fn selection_input_requires_legacy(&self, event: &InputEvent) -> bool {
        self.is_text_selection()
            || self.select_visual_once
            || matches!(
                event,
                InputEvent::Key(Key::ModifiedNavigation { .. } | Key::Ctrl('g' | 'G'))
            )
            || (matches!(self.pending, Pending::G { .. })
                && matches!(
                    event,
                    InputEvent::Key(Key::Char('h' | 'H') | Key::Ctrl('h' | 'H'))
                ))
            || (matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            ) && matches!(event, InputEvent::Key(key) if NavigationKey::from_key(*key).is_some()))
    }
    pub(crate) fn finish_select_mapping(&mut self) {
        self.selection_behavior = if matches!(
            self.mode,
            Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
        ) {
            SelectionBehavior::Select
        } else {
            SelectionBehavior::Visual
        };
    }
    pub(super) fn finish_select_visual_once(&mut self, output: &CommandOutput) {
        if self.select_visual_just_started {
            self.select_visual_just_started = false;
            return;
        }
        if self.select_visual_once
            && output.status != CommandStatus::Pending
            && self.pending == Pending::None
            && !self.register_pending
        {
            self.select_visual_once = false;
            if self.mode == Mode::Normal
                && self.select_visual_yanked
                && !output.document_changed
                && output.status == CommandStatus::Complete
            {
                if let Some(memory) = self.last_visual {
                    self.mode = memory.mode;
                    self.visual_anchor = Some(memory.anchor);
                    self.cursor = memory.active;
                }
            }
            self.select_visual_yanked = false;
            self.selection_behavior = if matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            ) {
                self.select_visual_return
            } else {
                SelectionBehavior::Visual
            };
        }
    }
    pub(super) fn handle_selection_input(
        &mut self,
        document: &mut Document,
        event: &InputEvent,
        mut context: Option<&mut LayoutCommandContext<'_>>,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if self.literal_input_pending()
            || self.mode == Mode::CommandLine
            || self.substitute_confirmation.is_some()
        {
            return Ok(None);
        }
        if let (
            Pending::G { count, .. },
            InputEvent::Key(key @ (Key::Char('h' | 'H') | Key::Ctrl('h' | 'H'))),
        ) = (self.pending, event)
        {
            self.pending = Pending::None;
            let output = match key {
                Key::Ctrl(_) => match context.as_mut() {
                    Some(context) => self.enter_visual_block(document, context),
                    None => layout_required("Select Block entry"),
                },
                _ => self.enter_visual(
                    document,
                    if *key == Key::Char('H') {
                        Mode::VisualLine
                    } else {
                        Mode::VisualCharacter
                    },
                    count,
                    false,
                ),
            };
            if output.status == CommandStatus::Complete {
                self.selection_behavior = SelectionBehavior::Select;
                self.selection_exclusive = false;
                self.selection_return_mode = Mode::Normal;
            }
            return Ok(Some(output));
        }
        if matches!(event, InputEvent::Key(Key::Ctrl('g' | 'G')))
            && matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            )
        {
            self.selection_behavior = if self.is_text_selection() {
                SelectionBehavior::Visual
            } else {
                SelectionBehavior::Select
            };
            self.select_visual_once = false;
            return Ok(Some(CommandOutput {
                mode_changed: true,
                ..CommandOutput::complete()
            }));
        }
        let navigation = match event {
            InputEvent::Key(Key::ModifiedNavigation { key, modifiers }) => {
                Some((key.key(), modifiers & 1 != 0))
            }
            InputEvent::Key(key) if NavigationKey::from_key(*key).is_some() => Some((*key, false)),
            _ => None,
        };
        if let Some((key, shifted)) = navigation {
            // Without startsel, Vim's shifted horizontal arrows are word
            // motions. Native Selection keeps platform selection navigation.
            let key = if shifted
                && !self.selection_options.autoselect
                && !self.is_native_selection()
                && !self.selection_options.keymodel("startsel")
            {
                match key {
                    Key::Left => Key::WordLeft,
                    Key::Right => Key::WordRight,
                    _ => key,
                }
            } else {
                key
            };
            if self.pending != Pending::None || self.register_pending {
                if matches!(event, InputEvent::Key(Key::ModifiedNavigation { .. })) {
                    let output = if let Some(context) = context.as_mut() {
                        match self.try_handle_layout_key(document, key, context)? {
                            Some(output) => output,
                            None => self.handle_key(document, key)?,
                        }
                    } else {
                        self.handle_key(document, key)?
                    };
                    return Ok(Some(output));
                }
                return Ok(None);
            }
            self.typing_style = Default::default();
            if let Some(output) = self.finish_overflowed_count() {
                return Ok(Some(output));
            }
            let selecting = matches!(
                self.mode,
                Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
            );
            let mut output = CommandOutput::complete();
            if shifted
                && !selecting
                && (self.selection_options.autoselect
                    || self.selection_options.keymodel("startsel"))
            {
                let origin = self.cursor;
                self.selection_return_mode = self.mode;
                if matches!(self.mode, Mode::Insert | Mode::Replace) {
                    output = self.finish_insert(document)?;
                    self.cursor = origin;
                }
                self.mode = Mode::VisualCharacter;
                self.visual_anchor = Some(origin);
                self.set_selection_origin(SelectionOrigin::Key);
                self.selection_exclusive = self.is_native_selection();
                output.mode_changed = true;
            } else if !shifted
                && selecting
                && (self.is_native_selection() || self.selection_options.keymodel("stopsel"))
            {
                let return_mode = self.selection_return_mode;
                let collapsed = if self.is_native_selection() && self.mode != Mode::VisualBlock {
                    let range = if self.mode == Mode::VisualLine {
                        self.line_selection_range(document, context.as_ref().map(|c| c.snapshot))
                            .unwrap_or_else(|| self.visual_extent(document).range)
                    } else {
                        self.visual_extent(document).range
                    };
                    match key {
                        Key::Left => Some(range.start),
                        Key::Right => Some(range.end),
                        _ => None,
                    }
                } else {
                    None
                };
                self.leave_visual();
                if let Some(cursor) = collapsed {
                    self.cursor = cursor;
                }
                if matches!(return_mode, Mode::Insert | Mode::Replace) {
                    self.enter_insert(
                        document,
                        if return_mode == Mode::Replace {
                            InsertPlacement::Replace
                        } else {
                            InsertPlacement::Before
                        },
                        1,
                    );
                }
                output.mode_changed = true;
                if collapsed.is_some() {
                    output.cursor_moved = true;
                    return Ok(Some(output));
                }
            }
            if shifted
                || selecting
                || matches!(event, InputEvent::Key(Key::ModifiedNavigation { .. }))
            {
                let visual_mode = self.mode;
                let exclusive =
                    self.selection_exclusive && matches!(visual_mode, Mode::VisualCharacter);
                // Native extents use insertion boundaries, including EOF. Reuse
                // the existing Insert motion engine without creating a session.
                let movement_count = if exclusive {
                    self.count.take().unwrap_or(1).max(1)
                } else {
                    1
                };
                let group_depth = document.edit_group_depth();
                if exclusive {
                    self.mode = Mode::Insert;
                }
                let mut next = CommandOutput::complete();
                for _ in 0..movement_count {
                    let step = if let Some(context) = context.as_mut() {
                        match self.try_handle_layout_key(document, key, context)? {
                            Some(result) => result,
                            None => self.handle_key(document, key)?,
                        }
                    } else {
                        self.handle_key(document, key)?
                    };
                    let stop = command_status_stops_compound(&step.status) || !step.cursor_moved;
                    next.merge(step);
                    if stop {
                        break;
                    }
                }
                if exclusive {
                    self.mode = visual_mode;
                    document.restore_edit_group_depth(group_depth);
                }
                output.merge(next);
                return Ok(Some(output));
            }
            return Ok(None);
        }
        if !self.is_text_selection() {
            return Ok(None);
        }
        if self.select_register_pending {
            if let InputEvent::Text(text) = event {
                let Some(register) = text.chars().next() else {
                    return Ok(Some(CommandOutput::pending()));
                };
                let mut output = self
                    .handle_selection_input(
                        document,
                        &InputEvent::Key(Key::Char(register)),
                        context.as_deref_mut(),
                    )?
                    .expect("the pending Select register consumes its operand");
                let remainder = &text[register.len_utf8()..];
                if output.status == CommandStatus::Complete && !remainder.is_empty() {
                    output.merge(
                        self.handle_selection_input(
                            document,
                            &InputEvent::Text(remainder.to_owned()),
                            context,
                        )?
                        .expect("Select consumes committed replacement text"),
                    );
                }
                return Ok(Some(output));
            }
            self.select_register_pending = false;
            let InputEvent::Key(Key::Char(register)) = event else {
                return Ok(Some(CommandOutput::unsupported(
                    "Select CTRL-R expects a register",
                )));
            };
            if !is_valid_register(*register) {
                return Ok(Some(CommandOutput::unsupported("invalid register")));
            }
            if let Err(error) = self.require_register_write(Some(*register)) {
                return Ok(Some(error));
            }
            self.select_delete_register = Some(*register);
            return Ok(Some(CommandOutput::complete()));
        }
        if matches!(event, InputEvent::Key(Key::Ctrl('r' | 'R'))) {
            self.select_register_pending = true;
            return Ok(Some(CommandOutput::pending()));
        }
        if matches!(event, InputEvent::Key(Key::Ctrl('o' | 'O'))) {
            self.select_visual_return = self.selection_behavior;
            self.selection_behavior = SelectionBehavior::Visual;
            self.select_visual_once = true;
            self.select_visual_just_started = true;
            return Ok(Some(CommandOutput {
                mode_changed: true,
                ..CommandOutput::complete()
            }));
        }
        if matches!(event, InputEvent::Key(Key::Escape | Key::Ctrl('c' | 'C'))) {
            self.selection_behavior = SelectionBehavior::Visual;
            return Ok(None);
        }
        let replacement = match event {
            InputEvent::Text(text) if !text.is_empty() => Some(InputEvent::Text(text.clone())),
            InputEvent::Key(Key::Char(ch)) if !ch.is_control() => {
                Some(InputEvent::Text(ch.to_string()))
            }
            InputEvent::Key(Key::Enter | Key::ShiftEnter | Key::Tab) => Some(event.clone()),
            _ => None,
        };
        let delete = matches!(event, InputEvent::Key(Key::Backspace | Key::Delete));
        if replacement.is_some() || delete {
            if self.mode == Mode::VisualLine {
                let range = self
                    .line_selection_range(
                        document,
                        context.as_ref().map(|context| context.snapshot),
                    )
                    .unwrap_or_else(|| self.visual_extent(document).range);
                self.mode = Mode::VisualCharacter;
                self.visual_anchor = Some(range.start);
                self.cursor = range.end;
                self.selection_exclusive = true;
            }
            self.requested_register = self.select_delete_register.take();
            if self.mode == Mode::VisualCharacter && self.visual_extent(document).range.is_empty() {
                self.selection_behavior = SelectionBehavior::Visual;
                self.selection_exclusive = false;
                self.visual_anchor = None;
                let mut output = self.enter_insert(document, InsertPlacement::Before, 1);
                if let Some(event) = replacement {
                    output.merge(match self.handle_mapping(document, &event)? {
                        Some(mapped) => mapped,
                        None => self.dispatch_event(document, event)?,
                    });
                }
                return Ok(Some(output));
            }
            let operator = if delete && !self.is_native_selection() {
                Operator::Delete
            } else {
                Operator::Change
            };
            let mut output = if self.mode == Mode::VisualBlock {
                match context.as_mut() {
                    Some(context) => self.handle_visual_block_layout_key(
                        document,
                        Key::Char(if operator == Operator::Delete {
                            'd'
                        } else {
                            'c'
                        }),
                        context,
                    )?,
                    None => return Ok(Some(layout_required("Select Block replacement"))),
                }
            } else {
                self.apply_visual_operator(document, operator, 1)?
            };
            if output.status == CommandStatus::Complete {
                self.selection_behavior = SelectionBehavior::Visual;
                self.selection_exclusive = false;
                if let Some(event) = replacement {
                    output.merge(match self.handle_mapping(document, &event)? {
                        Some(mapped) => mapped,
                        None => self.dispatch_event(document, event)?,
                    });
                }
            }
            return Ok(Some(output));
        }
        Ok(None)
    }
}

pub(crate) struct SelectCompositionCommit {
    deleted: RegisterValue,
    class: DeletionClass,
    command: VisualOperatorRepeat,
}
impl CommandInterpreter {
    pub(crate) fn select_composition_commit(
        &self,
        document: &Document,
    ) -> Option<SelectCompositionCommit> {
        if !self.is_text_selection() || self.mode == Mode::VisualBlock {
            return None;
        }
        let extent = self.visual_extent(document);
        let lines = document.hard_line_snapshot();
        let register = self.select_delete_register;
        Some(SelectCompositionCommit {
            deleted: register_value(document, &lines, &extent, register),
            class: ordinary_deletion_class(&lines, &extent),
            command: VisualOperatorRepeat {
                operator: Operator::Change,
                shape: self.visual_repeat_shape(document),
                application_count: 1,
                register,
            },
        })
    }
    pub(crate) fn finish_select_composition_commit(
        &mut self,
        document: &mut Document,
        state: SelectCompositionCommit,
        cursor: usize,
        text: &str,
    ) {
        self.cursor = cursor;
        self.selection_behavior = SelectionBehavior::Visual;
        self.selection_exclusive = false;
        self.visual_anchor = None;
        self.select_delete_register = None;
        self.select_register_pending = false;
        self.delete_register(state.command.register, state.deleted, state.class);
        self.enter_insert(document, InsertPlacement::Before, 1);
        let mut program = EditSessionProgram::default();
        if !self.typing_style.is_empty() {
            program.push(EditSessionStep::TypingStyle(self.typing_style.for_repeat()));
        }
        program.append_text(&external_text_register_value(document, text));
        if let Some(session) = self.insert_session.as_mut() {
            session.repeat_program = Some(program);
            session.repeat_visual_operator = Some(state.command);
            session.last_inserted = external_text_register_value(document, text);
        }
    }
}
