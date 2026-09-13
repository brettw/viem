//! Mode-aware aliases are resolved after literal and register operand grammar.
use super::*;

impl CommandInterpreter {
    /// Ctrl-O closes this typing fragment before its temporary Normal command.
    /// Publish its recipe before resetting the resumable fragment, so an
    /// interrupt (or a Normal dot command) can still repeat the completed edit.
    pub(super) fn publish_insert_repeat_before_normal_command(&mut self) {
        let Some(session) = self.insert_session.as_ref() else {
            return;
        };
        if self.replaying || session.preserve_normal_repeat {
            return;
        }
        if session.repeat_program.as_ref().is_some_and(|program| program.steps.is_empty())
            && session.repeat_operator.is_none() && session.repeat_visual_operator.is_none()
            && !matches!(session.placement, InsertPlacement::OpenAbove | InsertPlacement::OpenBelow)
        {
            return;
        }
        self.last_repeat = Some(match session.repeat_program.clone() {
            Some(program) => {
                if let Some(command) = session.repeat_visual_operator.clone() {
                    RepeatAction::VisualOperator {
                        command,
                        edits: Some(program),
                    }
                } else if let Some(command) = session.repeat_operator.clone() {
                    RepeatAction::Operator {
                        command,
                        edits: Some(program),
                    }
                } else {
                    RepeatAction::Insert {
                        placement: session.placement,
                        program,
                        count: 1,
                    }
                }
            }
            None => RepeatAction::Noop,
        });
    }

    pub(super) fn normalized_input_event(&self, event: InputEvent) -> InputEvent {
        match event {
            InputEvent::Key(key) => InputEvent::Key(self.normalized_input_key(key)),
            event => event,
        }
    }

    pub(super) fn normalized_input_key(&self, key: Key) -> Key {
        if self.literal_input_pending() {
            return key;
        }
        if key == Key::Ctrl('[') {
            return Key::Escape;
        }
        if self.register_pending
            || self.command_line_register_pending()
            || self.insert_control_g_pending()
            || !matches!(self.pending, Pending::None | Pending::Operator(_))
        {
            return key;
        }
        match (self.mode, key) {
            (Mode::Insert | Mode::Replace | Mode::CommandLine, Key::Ctrl('h' | 'H')) => {
                Key::Backspace
            }
            (Mode::Insert | Mode::Replace | Mode::CommandLine, Key::Ctrl('i' | 'I')) => Key::Tab,
            (
                Mode::Insert | Mode::Replace | Mode::CommandLine,
                Key::Ctrl('j' | 'J' | 'm' | 'M'),
            ) => Key::Enter,
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::Ctrl('h' | 'H'),
            ) => Key::Left,
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::Ctrl('j' | 'J' | 'n' | 'N'),
            ) => Key::Down,
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::Ctrl('p' | 'P'),
            ) => Key::Up,
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::Ctrl('m' | 'M'),
            ) => Key::Enter,
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::Tab,
            ) => Key::Ctrl('i'),
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::WordLeft,
            ) => Key::Char('b'),
            (
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock,
                Key::WordRight,
            ) => Key::Char('w'),
            (_, key) => key,
        }
    }

    pub(super) fn is_cancel_input(&self, event: &InputEvent) -> bool {
        matches!(event, InputEvent::Key(Key::Ctrl('c' | 'C')))
            && !self.literal_input_pending()
            && !self.register_pending
            && !self.command_line_register_pending()
            && !matches!(self.pending, Pending::Window { .. })
    }

    pub(super) fn is_register_cancel_input(&self, event: &InputEvent) -> bool {
        self.register_pending
            && !self.literal_input_pending()
            && matches!(event, InputEvent::Key(Key::Ctrl('c' | 'C')))
    }

    pub(super) fn cancel_register_operand(&mut self, key: Key) -> Option<CommandOutput> {
        if !self.is_register_cancel_input(&InputEvent::Key(key)) {
            return None;
        }
        self.clear_pending();
        Some(CommandOutput {
            status: CommandStatus::Cancelled,
            ..CommandOutput::complete()
        })
    }

    pub(super) fn cancel_input(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        let old_mode = self.mode;
        let mut output = CommandOutput::complete();
        if let Some(session) = self.visual_block_insert.as_mut() {
            session.cancel_collection = true;
            if session.kind != VisualBlockInsertKind::Change {
                session.rows.truncate(1);
                session.repeat_shape.visual_row_count = 1;
            }
            session.count = 1;
            output = self.finish_visual_block_insert(document)?;
        } else if matches!(self.mode, Mode::Insert | Mode::Replace) {
            if let Some(session) = self.insert_session.as_mut() {
                session.entry_count = 1;
            }
            output = self.finish_insert(document)?;
        } else {
            // Ctrl-O already closed the typing group and positioned a Normal
            // cursor. Discard its suspended session without finishing it a
            // second time, which would step left and overwrite its recipe.
            self.insert_session = None;
            if let Some(prompt) = self.command_line_state.take() {
                self.mode = prompt.return_mode;
            }
            if self.mode == Mode::VisualBlock {
                self.leave_visual_block();
            } else if matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine) {
                self.leave_visual();
            }
            self.mode = Mode::Normal;
        }
        self.clear_pending();
        self.insert_normal_once = None;
        self.ctrl_o_just_started = false;
        self.typing_style = Default::default();
        self.input_assistance.clear_tag();
        output.mode_changed |= self.mode != old_mode;
        output.status = CommandStatus::Cancelled;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{Core, CoreEvent};
    use crate::layout::MockTextMeasurementProvider;

    fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) -> CommandOutput {
        commands.handle(document, InputEvent::Key(key)).unwrap()
    }

    #[test]
    fn insert_aliases_share_editing_semantics_and_keep_quoted_controls_literal() {
        for mode in ['i', 'R'] {
            for (alias, ordinary) in [
                ('h', Key::Backspace),
                ('i', Key::Tab),
                ('j', Key::Enter),
                ('m', Key::Enter),
            ] {
                let mut a = Document::new("abc");
                let mut b = Document::new("abc");
                let mut ca = CommandInterpreter::new();
                let mut cb = CommandInterpreter::new();
                for (document, commands) in [(&mut a, &mut ca), (&mut b, &mut cb)] {
                    key(commands, document, Key::Char(mode));
                    key(commands, document, Key::Char('x'));
                }
                key(&mut ca, &mut a, Key::Ctrl(alias));
                key(&mut cb, &mut b, ordinary);
                assert_eq!(a.text(), b.text(), "{mode} Ctrl-{alias}");
                assert_eq!(ca.cursor(), cb.cursor());
            }
        }
        let mut document = Document::new("");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        for (input, expected) in [
            ('h', "\u{8}"),
            ('i', "\t"),
            ('j', "\0"),
            ('m', "\r"),
            ('[', "\u{1b}"),
            ('c', "\u{3}"),
        ] {
            let before = document.text().to_owned();
            key(&mut commands, &mut document, Key::Ctrl('v'));
            key(&mut commands, &mut document, Key::Ctrl(input));
            assert_eq!(document.text(), before + expected);
            assert_eq!(commands.mode(), Mode::Insert);
        }
    }

    #[test]
    fn aliases_plan_and_acquire_the_same_layout_as_their_normal_and_visual_keys() {
        for prefix in [
            vec![],
            vec![Key::Char('v')],
            vec![Key::Char('V')],
            vec![Key::Ctrl('v')],
            vec![Key::Char('d')],
        ] {
            for (alias, ordinary) in [
                ('h', Key::Left),
                ('j', Key::Down),
                ('n', Key::Down),
                ('p', Key::Up),
                ('m', Key::Enter),
            ] {
                let run = |last| {
                    let mut core = Core::new(Document::new("abc def\n  second\n  third"));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 35., 150.);
                    for input in [Key::Char('j'), Key::Char('l')]
                        .into_iter()
                        .chain(prefix.clone())
                        .chain([last])
                    {
                        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                            .unwrap();
                    }
                    let state = core.command_state(view).unwrap();
                    (
                        core.document().text().to_owned(),
                        state.cursor(),
                        state.mode(),
                        state.visual_anchor(),
                    )
                };
                assert_eq!(
                    run(Key::Ctrl(alias)),
                    run(ordinary),
                    "{prefix:?} Ctrl-{alias}"
                );
            }
        }
    }

    #[test]
    fn physical_tab_follows_the_jump_list_and_control_escape_cancels_grammar() {
        let mut document = Document::new("a\nb\nc");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('G'));
        key(&mut commands, &mut document, Key::Ctrl('o'));
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut document, Key::Tab);
        assert_eq!(commands.cursor(), 4);
        for prefix in [
            Key::Char('d'),
            Key::Char('g'),
            Key::Char('3'),
            Key::Char('"'),
        ] {
            key(&mut commands, &mut document, prefix);
            key(&mut commands, &mut document, Key::Ctrl('['));
            assert_eq!(commands.pending, Pending::None);
            assert!(!commands.register_pending);
            assert_eq!(commands.count, None);
        }
    }

    #[test]
    fn control_c_stops_insert_counts_and_visual_replication_without_losing_typed_text() {
        for mode in ['i', 'R'] {
            let mut document = Document::new("abcdef");
            let mut commands = CommandInterpreter::new();
            for input in [
                Key::Char('3'),
                Key::Char(mode),
                Key::Char('x'),
                Key::Ctrl('c'),
            ] {
                key(&mut commands, &mut document, input);
            }
            assert_eq!(
                document.text(),
                if mode == 'i' { "xabcdef" } else { "xbcdef" }
            );
            assert_eq!(commands.mode(), Mode::Normal);
            key(&mut commands, &mut document, Key::Char('u'));
            assert_eq!(document.text(), "abcdef");
        }
        let mut core = Core::new(Document::new("a\nb"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200., 100.);
        for input in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('3'),
            Key::Char('I'),
            Key::Char('x'),
            Key::Ctrl('c'),
        ] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
        }
        assert_eq!(core.document().text(), "xa\nb");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    }

    #[test]
    fn control_c_cancels_pending_normal_visual_and_prompt_without_becoming_an_operand() {
        for prefix in ["3d", "g", "f", "v", "vg", ":abc", "/abc", "v:abc"] {
            let mut document = Document::new("abc");
            let mut commands = CommandInterpreter::new();
            commands
                .handle(&mut document, InputEvent::Text(prefix.into()))
                .unwrap();
            key(&mut commands, &mut document, Key::Ctrl('c'));
            assert_eq!(commands.mode(), Mode::Normal, "{prefix}");
            assert_eq!(commands.pending, Pending::None);
            assert_eq!(commands.count, None);
            assert_eq!(document.text(), "abc");
        }
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(
            key(&mut commands, &mut document, Key::Ctrl('c')).status,
            CommandStatus::Cancelled
        );
        assert_eq!(commands.mode(), Mode::Insert);
        assert!(!commands.register_pending);
    }

    #[test]
    fn modified_word_motion_is_grapheme_safe_and_can_reach_insert_eof() {
        let mut document = Document::new("one e\u{301}clair");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        key(&mut commands, &mut document, Key::WordRight);
        assert_eq!(commands.cursor(), 4);
        key(&mut commands, &mut document, Key::WordRight);
        assert_eq!(commands.cursor(), document.text().len());
        key(&mut commands, &mut document, Key::WordLeft);
        assert_eq!(commands.cursor(), 4);
        key(&mut commands, &mut document, Key::Ctrl('v'));
        key(&mut commands, &mut document, Key::WordLeft);
        assert_eq!(document.text(), "one <C-Left>e\u{301}clair");
    }

    #[test]
    fn cancelled_block_change_keeps_all_deletions_but_only_the_first_rows_input() {
        let mut core = Core::new(Document::new("ab\ncd\nef\ngh"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200., 100.);
        for input in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('c'),
            Key::Char('x'),
            Key::Ctrl('c'),
        ] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
        }
        assert_eq!(core.document().text(), "xb\nd\nef\ngh");
        for input in [Key::Char('2'), Key::Char('j'), Key::Char('.')] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
        }
        assert_eq!(core.document().text(), "xb\nd\nxf\nh");
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().text(), "xb\nd\nef\ngh");
    }

    #[test]
    fn prompt_word_motion_and_register_cancellation_preserve_their_own_modes() {
        let mut document = Document::new("abc");
        let mut commands = CommandInterpreter::new();
        commands
            .handle(
                &mut document,
                InputEvent::Text(":echo foo.bar e\u{301}clair".into()),
            )
            .unwrap();
        key(&mut commands, &mut document, Key::WordLeft);
        assert_eq!(commands.command_line_cursor(), Some("echo foo.bar ".len()));
        key(&mut commands, &mut document, Key::WordLeft);
        assert_eq!(commands.command_line_cursor(), Some("echo ".len()));
        key(&mut commands, &mut document, Key::WordRight);
        assert_eq!(commands.command_line_cursor(), Some("echo foo.bar ".len()));
        key(&mut commands, &mut document, Key::Ctrl('c'));
        for input in [
            Key::Char('2'),
            Key::Char('i'),
            Key::Ctrl('r'),
            Key::Ctrl('c'),
        ] {
            key(&mut commands, &mut document, input);
        }
        assert_eq!(commands.mode(), Mode::Insert);
        key(&mut commands, &mut document, Key::Char('x'));
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "xxabc");
        for prefix in ["\"", "v\""] {
            commands
                .handle(&mut document, InputEvent::Text(prefix.into()))
                .unwrap();
            let mode = commands.mode();
            assert_eq!(
                key(&mut commands, &mut document, Key::Ctrl('c')).status,
                CommandStatus::Cancelled
            );
            assert_eq!(commands.mode(), mode);
            assert!(!commands.register_pending);
        }
    }

    #[test]
    fn control_c_closes_suspended_insert_without_moving_or_orphaning_the_session() {
        for (source, mode, typed, expected, cursor) in [
            ("", 'i', "abc", "abc", 2),
            ("abcdef", 'R', "XY", "XYcdef", 2),
        ] {
            for pending in [None, Some(Key::Char(':')), Some(Key::Char('v'))] {
                let mut core = Core::new(Document::new(source));
                let view = core.add_view(MockTextMeasurementProvider::new(), 200., 100.);
                for input in [Key::Char('3'), Key::Char(mode)]
                    .into_iter()
                    .chain(typed.chars().map(Key::Char))
                    .chain([Key::Ctrl('o')])
                {
                    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                        .unwrap();
                }
                if let Some(input) = pending {
                    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                        .unwrap();
                }
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('c'))))
                    .unwrap();
                let commands = core.command_state(view).unwrap();
                assert_eq!(commands.mode(), Mode::Normal);
                assert_eq!(commands.cursor(), cursor, "{mode} {pending:?}");
                assert!(
                    commands.insert_session.is_none(),
                    "no suspended typing state"
                );
                assert!(commands.insert_normal_once.is_none());
                assert_eq!(commands.register('.').unwrap().text, typed);
                assert_eq!(
                    core.document().text(),
                    expected,
                    "the entry count is not expanded"
                );
                assert_eq!(core.document().edit_group_depth(), 0);
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('.'))))
                    .unwrap();
                assert_eq!(
                    core.document().text(),
                    if mode == 'i' { "ababcc" } else { "XYXYef" }
                );
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
                    .unwrap();
                assert_eq!(core.document().text(), expected);
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
                    .unwrap();
                assert_eq!(core.document().text(), source);
            }
        }
    }

    #[test]
    fn empty_insert_ctrl_o_keeps_the_previous_normal_repeat_and_resumes_insertion() {
        let mut document = Document::new("abcdef");
        let mut commands = CommandInterpreter::new();
        for input in [Key::Char('x'), Key::Char('i'), Key::Ctrl('o'), Key::Char('.')] {
            key(&mut commands, &mut document, input);
        }
        assert_eq!(document.text(), "cdef");
        assert_eq!(commands.mode(), Mode::Insert);
        assert!(commands.insert_session.is_some());
        assert_eq!(document.edit_group_depth(), 1);
        key(&mut commands, &mut document, Key::Escape);
        assert!(commands.insert_session.is_none());
        assert_eq!(document.edit_group_depth(), 0);
    }
}
