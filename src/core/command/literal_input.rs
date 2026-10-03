//! Vim literal-next input, independent of host key codes and text assistance.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct LiteralInput {
    radix: u32,
    limit: u8,
    digits: u8,
    value: u32,
}

impl Default for LiteralInput {
    fn default() -> Self {
        Self {
            radix: 10,
            limit: 3,
            digits: 0,
            value: 0,
        }
    }
}

impl VisualBlockInsertSession {
    pub(super) fn truncate_payload(&mut self, length: usize) {
        self.payload.truncate(length);
        self.literal_ranges.retain_mut(|range| {
            range.end = range.end.min(length);
            range.start < range.end
        });
    }

    pub(super) fn repeated_literal_ranges(&self, total_len: usize) -> Vec<Range<usize>> {
        if self.payload.is_empty() || self.literal_ranges.is_empty() {
            return Vec::new();
        }
        // Counts and coincident block-row insertions concatenate complete input
        // units. Protect the same original byte ranges in every copy.
        debug_assert_eq!(total_len % self.payload.len(), 0);
        (0..total_len)
            .step_by(self.payload.len())
            .flat_map(|offset| {
                self.literal_ranges
                    .iter()
                    .map(move |range| range.start + offset..range.end + offset)
            })
            .collect()
    }
}

impl CommandInterpreter {
    /// The next key belongs to literal/numeric input, not native text actions.
    pub fn literal_input_pending(&self) -> bool {
        self.literal_input.is_some()
    }

    pub(super) fn handles_literal_input(&self, event: &InputEvent) -> bool {
        self.literal_input_pending()
            || (matches!(self.mode, Mode::Insert | Mode::Replace | Mode::CommandLine)
                && !self.register_pending
                && !self.insert_control_g_pending()
                && !self.command_line_register_pending()
                && matches!(event, InputEvent::Key(Key::Ctrl('v' | 'V' | 'q' | 'Q'))))
    }

    pub(super) fn handle_literal_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let mut output = CommandOutput::pending();
        for (at, character) in input.char_indices() {
            if !self.literal_input_pending() {
                output.merge(self.handle_text(document, input[at..].to_owned())?);
                return Ok(output);
            }
            let next = self.consume_literal_key(document, Key::Char(character))?;
            let stop = command_status_stops_compound(&next.status);
            output.merge(next);
            if stop {
                break;
            }
        }
        Ok(output)
    }

    pub(super) fn handle_literal_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        if !self.literal_input_pending() {
            self.literal_input = Some(LiteralInput::default());
            return Ok(CommandOutput::pending());
        }
        self.consume_literal_key(document, key)
    }

    fn consume_literal_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        let mut state = self.literal_input.take().expect("literal input is pending");
        if let Key::Char(character) = key {
            let prefix = match character {
                'o' | 'O' => Some((8, 3)),
                'x' | 'X' => Some((16, 2)),
                'u' => Some((16, 4)),
                'U' => Some((16, 8)),
                _ => None,
            };
            if let Some((radix, limit)) = prefix {
                state.radix = radix;
                state.limit = limit;
                self.literal_input = Some(state);
                return Ok(CommandOutput::pending());
            }
            if let Some(digit) = character
                .to_digit(state.radix)
                .filter(|_| character.is_ascii())
            {
                state.value = state
                    .value
                    .saturating_mul(state.radix)
                    .saturating_add(digit);
                state.digits += 1;
                if state.digits < state.limit {
                    self.literal_input = Some(state);
                    return Ok(CommandOutput::pending());
                }
                return self.insert_literal_number(document, state);
            }
        }
        if state.digits > 0 {
            let mut output = self.insert_literal_number(document, state)?;
            if !command_status_stops_compound(&output.status) {
                let event = InputEvent::Key(key);
                if self.plan_compound_replay
                    && self.requires_layout_for_input(document, &event, None)
                {
                    self.pending_replay = Some(ReplayPlan::LiteralTerminator(event));
                } else {
                    output.merge(self.handle_key(document, key)?);
                }
            }
            return Ok(output);
        }
        match literal_key_text(key) {
            Some(text) => self.insert_quoted_text(document, &text),
            None => Ok(CommandOutput::unsupported(
                "this semantic action has no literal key representation",
            )),
        }
    }

    fn insert_literal_number(
        &mut self,
        document: &mut Document,
        state: LiteralInput,
    ) -> Result<CommandOutput, DocumentError> {
        let value = if state.limit <= 3 {
            state.value.min(255)
        } else {
            state.value
        };
        let Some(character) = char::from_u32(value) else {
            return Ok(CommandOutput {
                status: CommandStatus::Error(format!(
                    "U+{value:04X} is not a Unicode scalar value"
                )),
                ..CommandOutput::complete()
            });
        };
        self.insert_quoted_text(document, &character.to_string())
    }

    pub(super) fn insert_quoted_text(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let text = input
            .chars()
            .map(|character| match character {
                '\n' => '\0',
                '\r' if self.mode != Mode::CommandLine
                    && document.file_format() == FileFormat::Mac =>
                {
                    '\n'
                }
                value => value,
            })
            .collect::<String>();
        if self.mode == Mode::CommandLine {
            if let Some(state) = self.command_line_state.as_mut() {
                state.buffer.insert(&text);
            }
            return Ok(CommandOutput::pending());
        }
        if let Some(session) = self.visual_block_insert.as_mut() {
            let start = session.payload.len();
            // Quoted LF is content in a Mac-format document. The ordinary
            // collection path still rejects semantic hard-line input.
            session.payload.push_str(&text);
            let end = session.payload.len();
            if end > start {
                if let Some(range) = session
                    .literal_ranges
                    .last_mut()
                    .filter(|range| range.end == start)
                {
                    range.end = end;
                } else {
                    session.literal_ranges.push(start..end);
                }
            }
            return Ok(CommandOutput::complete());
        }
        self.generated_indent = None;
        self.input_assistance.literal = true;
        let program = self
            .insert_session
            .as_mut()
            .and_then(|session| session.repeat_program.take());
        let value = RegisterValue::try_new(&text, RegisterKind::Characterwise, Vec::new())
            .expect("quoted controls are literal content, not semantic line breaks");
        let result = if self.mode == Mode::Replace && text.chars().count() > 1 {
            self.replace_literal_notation(document, &text)
        } else if self.mode == Mode::Replace {
            self.replace_register_payload(document, &value)
        } else {
            self.insert_register_payload(document, &value)
        };
        self.input_assistance.literal = false;
        if let Some(session) = self.insert_session.as_mut() {
            session.repeat_program = program;
            if result.is_ok() && !session.replaying_program {
                session.record_step(EditSessionStep::LiteralText(input.to_owned()));
            }
        }
        result
    }

    fn replace_literal_notation(
        &mut self,
        document: &mut Document,
        text: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let start = self.cursor;
        let lines = document.hard_line_snapshot();
        let end = lines
            .grapheme_range_at(start)
            .filter(|range| !is_hard_line_separator(&lines, range))
            .map_or(start, |range| range.end);
        let original =
            (start < end).then(|| lines.slice_utf8(start..end).expect("validated replacement"));
        let value = RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new()).unwrap();
        let payload = FormattedTextPayload::new(&lines, text, Vec::new()).unwrap();
        let edit =
            FormattedPayloadEdit::new(start..end, payload)
                .with_boundary_affinity(self.insertion_boundary_affinity());
        document.validate_typing_payload(&edit)?;
        let before = document.revision();
        self.cursor = if self.typing_style.is_empty() {
            commit_typing_payload(document, edit)?
        } else {
            document
                .insert_with_typing_style(
                    edit,
                    self.typing_style.named.as_ref(),
                    &self.typing_style.values,
                )
                .map_err(command_document_error)?
        };
        self.finish_typing_caret(document)?;
        if let Some(session) = self.insert_session.as_mut() {
            let last = text.char_indices().last().expect("nonempty notation").0;
            for (at, character) in text.char_indices() {
                session.replace_journal.push(ReplaceJournalEntry {
                    autoindent: false,
                    start: start + at,
                    inserted: character.to_string(),
                    // Vim moves over its inserted notation prefix on BS.
                    original: if at == last {
                        original.clone()
                    } else {
                        Some(character.to_string())
                    },
                    source_record: None,
                });
            }
            session.record_inserted(&value, None);
        }
        Ok(CommandOutput {
            document_changed: document.revision() != before,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }
}

pub(super) fn literal_key_text(key: Key) -> Option<String> {
    Some(match key {
        Key::Char(character) => character.to_string(),
        Key::Escape => "\u{1b}".into(),
        Key::Enter => "\r".into(),
        Key::Tab => "\t".into(),
        Key::Ctrl(character) => {
            let character = character.to_ascii_uppercase();
            if character == '?' {
                "\u{7f}".into()
            } else if character == ' ' || character == '@' {
                "\0".into()
            } else if ('A'..='_').contains(&character) {
                char::from_u32(character as u32 & 31)?.to_string()
            } else {
                format!("<C-{character}>")
            }
        }
        Key::Backspace => "<BS>".into(),
        Key::Delete => "<Del>".into(),
        Key::BackTab => "<S-Tab>".into(),
        Key::ShiftEnter => "<S-CR>".into(),
        Key::Left => "<Left>".into(),
        Key::Right => "<Right>".into(),
        Key::WordLeft => "<C-Left>".into(),
        Key::WordRight => "<C-Right>".into(),
        Key::ParagraphStart => "<ParagraphStart>".into(),
        Key::ParagraphEnd => "<ParagraphEnd>".into(),
        Key::NextParagraph => "<NextParagraph>".into(),
        Key::Up => "<Up>".into(),
        Key::Down => "<Down>".into(),
        Key::Home => "<Home>".into(),
        Key::End => "<End>".into(),
        Key::DocumentStart => "<C-Home>".into(),
        Key::DocumentEnd => "<C-End>".into(),
        Key::PageUp => "<PageUp>".into(),
        Key::PageDown => "<PageDown>".into(),
        Key::Function { number, modifiers } => {
            let mut name = String::from("<");
            for (bit, modifier) in [(2, "C-"), (1, "S-"), (4, "A-"), (8, "D-")] {
                if modifiers & bit != 0 { name.push_str(modifier); }
            }
            name.push_str(&format!("F{number}>"));
            name
        }
        Key::ModifiedNavigation { key, modifiers } => {
            let base = literal_key_text(key.key())?;
            let mut name = String::from("<");
            for (bit, modifier) in [(2, "C-"), (1, "S-"), (4, "A-"), (8, "D-")] { if modifiers & bit != 0 { name.push_str(modifier); } }
            name.push_str(&base[1..]);
            name
        }
        Key::SelectAll | Key::CopySelection => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};

    fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) -> CommandOutput {
        commands.handle(document, InputEvent::Key(key)).unwrap()
    }
    fn text(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        text: &str,
    ) -> CommandOutput {
        commands
            .handle(document, InputEvent::Text(text.into()))
            .unwrap()
    }
    fn quote(commands: &mut CommandInterpreter, document: &mut Document, input: Key) {
        assert_eq!(
            key(commands, document, Key::Ctrl('v')).status,
            CommandStatus::Pending
        );
        assert!(commands.literal_input_pending());
        let output = key(commands, document, input);
        assert!(
            !command_status_stops_compound(&output.status),
            "{:?}",
            output.status
        );
        assert!(!commands.literal_input_pending());
    }

    #[test]
    fn both_prefixes_quote_controls_without_leaving_insert_or_replace() {
        for prefix in ['v', 'q'] {
            for mode in ['i', 'R'] {
                for (input, expected) in [
                    (Key::Tab, "\t"),
                    (Key::Escape, "\u{1b}"),
                    (Key::Enter, "\r"),
                    (Key::Ctrl('j'), "\0"),
                    (Key::Ctrl('@'), "\0"),
                    (Key::Ctrl('h'), "\u{8}"),
                    (Key::Ctrl('?'), "\u{7f}"),
                    (Key::Ctrl('v'), "\u{16}"),
                ] {
                    let mut document = Document::new("abc");
                    let mut commands = CommandInterpreter::new();
                    key(&mut commands, &mut document, Key::Char(mode));
                    key(&mut commands, &mut document, Key::Ctrl(prefix));
                    key(&mut commands, &mut document, input);
                    assert_eq!(
                        commands.mode(),
                        if mode == 'i' {
                            Mode::Insert
                        } else {
                            Mode::Replace
                        }
                    );
                    assert_eq!(
                        document.text(),
                        format!("{expected}{}", if mode == 'i' { "abc" } else { "bc" })
                    );
                    assert!(!commands.literal_input_pending());
                }
            }
        }
    }

    #[test]
    fn numeric_forms_limits_and_early_terminators_match_vim() {
        for (input, expected) in [
            ("065", "A"),
            ("o101", "A"),
            ("O101", "A"),
            ("x41", "A"),
            ("X41", "A"),
            ("u263A", "☺"),
            ("U0001F642", "🙂"),
            ("256", "ÿ"),
            ("999", "ÿ"),
            ("o777", "ÿ"),
            ("000", "\0"),
            ("010", "\0"),
            ("xg", "g"),
            ("xxx41", "A"),
            ("1x2z", "\u{12}z"),
            ("u41!", "A!"),
        ] {
            let mut document = Document::new("");
            let mut commands = CommandInterpreter::new();
            key(&mut commands, &mut document, Key::Char('i'));
            key(&mut commands, &mut document, Key::Ctrl('v'));
            text(&mut commands, &mut document, input);
            assert_eq!(document.text(), expected, "numeric input {input}");
            assert!(!commands.literal_input_pending());
        }
        for digits in ["65", "x"] {
            let mut document = Document::new("");
            let mut commands = CommandInterpreter::new();
            key(&mut commands, &mut document, Key::Char('i'));
            key(&mut commands, &mut document, Key::Ctrl('v'));
            text(&mut commands, &mut document, digits);
            key(&mut commands, &mut document, Key::Tab);
            assert_eq!(document.text(), if digits == "65" { "A " } else { "\t" });
        }
    }

    #[test]
    fn invalid_unicode_cancels_pending_without_mutation_and_next_key_is_normal() {
        for value in ["uD800", "U00110000", "UFFFFFFFF"] {
            let mut document = Document::new("");
            let mut commands = CommandInterpreter::new();
            key(&mut commands, &mut document, Key::Char('i'));
            key(&mut commands, &mut document, Key::Ctrl('v'));
            let revision = document.revision();
            assert!(matches!(
                text(&mut commands, &mut document, value).status,
                CommandStatus::Error(_)
            ));
            assert_eq!(document.revision(), revision);
            assert!(!commands.literal_input_pending());
            text(&mut commands, &mut document, "x");
            assert_eq!(document.text(), "x");
        }
    }

    #[test]
    fn special_key_names_replace_one_grapheme_and_backspace_restores_only_the_tail() {
        let mut document = Document::new("e\u{301}BCDEF");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('R'));
        quote(&mut commands, &mut document, Key::Up);
        assert_eq!(document.text(), "<Up>BCDEF");
        for _ in 0..4 {
            key(&mut commands, &mut document, Key::Backspace);
        }
        assert_eq!(document.text(), "<Upe\u{301}BCDEF");
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut document, Key::Escape);
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.text(), "e\u{301}BCDEF");
    }

    #[test]
    fn quoted_tabs_have_one_undo_group_and_remain_literal_in_count_dot_and_macro_replay() {
        let mut document = Document::new("");
        let mut commands = CommandInterpreter::new();
        for input in [Key::Char('3'), Key::Char('i')] {
            key(&mut commands, &mut document, input);
        }
        quote(&mut commands, &mut document, Key::Tab);
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(document.text(), "\t\t\t");
        key(&mut commands, &mut document, Key::Char('.'));
        assert_eq!(document.text(), "\t".repeat(6));
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.text(), "\t\t\t");
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.text(), "");
        for input in [Key::Char('q'), Key::Char('a'), Key::Char('i')] {
            key(&mut commands, &mut document, input);
        }
        quote(&mut commands, &mut document, Key::Tab);
        for input in [Key::Escape, Key::Char('q'), Key::Char('@'), Key::Char('a')] {
            key(&mut commands, &mut document, input);
        }
        assert_eq!(document.text(), "\t\t");
    }

    #[test]
    fn literal_replay_bypasses_smart_quotes() {
        for format in [Format::PlainText, Format::MarkdownSource] {
            let mut document = Document::from_bytes(Vec::new(), Encoding::Utf8, format).unwrap();
            let mut commands = CommandInterpreter::new();
            commands.set_smart_quotes(true);
            key(&mut commands, &mut document, Key::Char('i'));
            quote(&mut commands, &mut document, Key::Char('"'));
            quote(&mut commands, &mut document, Key::Char('<'));
            key(&mut commands, &mut document, Key::Escape);
            assert_eq!(document.text(), "\"<");
            key(&mut commands, &mut document, Key::Char('.'));
            assert_eq!(document.text().chars().filter(|ch| *ch == '"').count(), 2);
            assert!(!document.text().contains('>'));
        }
    }

    #[test]
    fn command_line_literals_do_not_submit_cancel_complete_or_move() {
        let mut document = Document::new("");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char(':'));
        for input in [Key::Tab, Key::Enter, Key::Escape, Key::Left, Key::Ctrl('j')] {
            quote(&mut commands, &mut document, input);
        }
        assert_eq!(commands.mode(), Mode::CommandLine);
        assert_eq!(commands.command_line(), Some("\t\r\u{1b}<Left>\0"));
        assert_eq!(document.text(), "");
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.mode(), Mode::Normal);
    }

    #[test]
    fn quoted_return_in_mac_source_is_literal_line_feed_not_a_hard_break() {
        let mut document = Document::from_bytes_with_file_format(
            Vec::new(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char('i'));
        quote(&mut commands, &mut document, Key::Enter);
        assert_eq!(document.source_bytes(), b"\n");
        assert_eq!(document.line_count(), 1);
    }

    #[test]
    fn deferred_visual_block_insert_accepts_a_quoted_hard_tab() {
        use crate::coordinator::{Core, CoreEvent};
        let mut core = Core::new(Document::new("x\nx"));
        let view = core.add_view(
            crate::layout::MockTextMeasurementProvider::new(),
            200.,
            100.,
        );
        for input in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('I'),
            Key::Ctrl('q'),
            Key::Tab,
            Key::Escape,
        ] {
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
            assert!(!command_status_stops_compound(
                &output.command.unwrap().status
            ));
        }
        assert_eq!(core.document().text(), "\tx\n\tx");
    }

    #[test]
    fn mac_block_quoted_return_retains_literal_lf_in_dot_macro_and_undo() {
        use crate::coordinator::{Core, CoreEvent};
        let document = Document::from_bytes_with_file_format(
            b"a\rb\rc\rd".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(
            crate::layout::MockTextMeasurementProvider::new(),
            200.,
            100.,
        );
        let send = |core: &mut Core<crate::layout::MockTextMeasurementProvider>, input| {
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
            assert!(!command_status_stops_compound(
                &output.command.unwrap().status
            ));
        };
        for input in [
            Key::Char('q'),
            Key::Char('a'),
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('I'),
            Key::Ctrl('v'),
            Key::Enter,
            Key::Escape,
            Key::Char('q'),
        ] {
            send(&mut core, input);
        }
        assert_eq!(core.document().source_bytes(), b"\na\r\nb\rc\rd");
        assert_eq!(core.document().line_count(), 4);
        let last_insert = core.command_state(view).unwrap().register('.').unwrap();
        assert_eq!(last_insert.text, "\n");
        assert!(last_insert.hard_break_offsets().is_empty());
        for input in [Key::Char('2'), Key::Char('j'), Key::Char('.')] {
            send(&mut core, input);
        }
        assert_eq!(core.document().source_bytes(), b"\na\r\nb\r\nc\r\nd");
        assert_eq!(core.document().line_count(), 4);
        send(&mut core, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"\na\r\nb\rc\rd");
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: core.document().text().find('c').unwrap(),
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        for input in [Key::Char('@'), Key::Char('a')] {
            send(&mut core, input);
        }
        assert_eq!(core.document().source_bytes(), b"\na\r\nb\r\nc\r\nd");
        assert_eq!(core.document().line_count(), 4);
        send(&mut core, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"\na\r\nb\rc\rd");
        send(&mut core, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"a\rb\rc\rd");
    }

    #[test]
    fn deferred_block_literals_survive_backspace_count_and_dot_beside_smart_quotes() {
        use crate::coordinator::{Core, CoreEvent};
        let mut core = Core::new(Document::new("a\nb\nc\nd"));
        let view = core.add_view(
            crate::layout::MockTextMeasurementProvider::new(),
            200.,
            100.,
        );
        core.handle_with_layout(view, CoreEvent::SetSmartQuotes(true))
            .unwrap();
        let send = |core: &mut Core<crate::layout::MockTextMeasurementProvider>, input| {
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
            assert!(!command_status_stops_compound(
                &output.command.unwrap().status
            ));
        };
        for input in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('2'),
            Key::Char('I'),
            Key::Char('"'),
            Key::Ctrl('v'),
            Key::Char('"'),
            Key::Char('x'),
            Key::Ctrl('q'),
            Key::Char('\''),
            Key::Backspace,
            Key::Char('\''),
        ] {
            send(&mut core, input);
        }
        let session = core
            .command_state(view)
            .unwrap()
            .visual_block_insert
            .as_ref()
            .unwrap();
        assert_eq!(session.payload, "\"\"x'");
        assert_eq!(session.literal_ranges, vec![1..2]);
        send(&mut core, Key::Escape);
        let prefix = "“\"x’”\"x’";
        let after_first = format!("{prefix}a\n{prefix}b\nc\nd");
        assert_eq!(core.document().text(), after_first);
        for input in [Key::Char('2'), Key::Char('j'), Key::Char('.')] {
            send(&mut core, input);
        }
        assert_eq!(
            core.document().text(),
            format!("{prefix}a\n{prefix}b\n{prefix}c\n{prefix}d")
        );
        send(&mut core, Key::Char('u'));
        assert_eq!(core.document().text(), after_first);
    }

    #[test]
    fn protected_quote_offsets_include_source_syntax_and_unicode_prefixes() {
        let document =
            Document::from_bytes(Vec::new(), Encoding::Utf8, Format::MarkdownSource).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.set_smart_quotes(true);
        let input = "`code \"x\"` é\"y\"";
        let protected = input.find("\"y").unwrap();
        let transformed = commands
            .assist_input_payload_with_literals(
                &document,
                0..0,
                BoundaryAffinity::Downstream,
                &RegisterValue::characterwise(input),
                &[protected..protected + 1],
            )
            .unwrap();
        assert_eq!(transformed.text, "`code \"x\"` é\"y”");
    }

    #[test]
    fn deferred_block_word_and_line_deletion_discard_literal_protection() {
        use crate::coordinator::{Core, CoreEvent};
        let mut core = Core::new(Document::new("x\nx"));
        let view = core.add_view(
            crate::layout::MockTextMeasurementProvider::new(),
            200.,
            100.,
        );
        core.handle_with_layout(view, CoreEvent::SetSmartQuotes(true))
            .unwrap();
        for input in [
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('I'),
            Key::Ctrl('v'),
            Key::Char('"'),
            Key::Ctrl('w'),
            Key::Ctrl('v'),
            Key::Char('"'),
            Key::Ctrl('u'),
            Key::Char('"'),
            Key::Escape,
        ] {
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
            assert!(!command_status_stops_compound(
                &output.command.unwrap().status
            ));
        }
        assert_eq!(core.document().text(), "“x\n“x");
    }

    #[test]
    fn early_escape_terminates_numeric_input_but_is_literal_without_digits() {
        for numeric in ["x4", "x"] {
            let mut document = Document::new("");
            let mut commands = CommandInterpreter::new();
            key(&mut commands, &mut document, Key::Char('i'));
            key(&mut commands, &mut document, Key::Ctrl('v'));
            text(&mut commands, &mut document, numeric);
            key(&mut commands, &mut document, Key::Escape);
            assert_eq!(
                document.text(),
                if numeric == "x4" { "\u{4}" } else { "\u{1b}" }
            );
            assert_eq!(
                commands.mode(),
                if numeric == "x4" {
                    Mode::Normal
                } else {
                    Mode::Insert
                }
            );
        }
    }
}
