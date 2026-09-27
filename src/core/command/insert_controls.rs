//! Insert controls whose meaning is independent of platform fonts and keys.
use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct InsertControls {
    pub g_pending: bool,
    pub join_next_horizontal: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};
    use crate::coordinator::{Core, CoreEvent};
    use crate::layout::MockTextMeasurementProvider;

    fn key(c: &mut CommandInterpreter, d: &mut Document, key: Key) -> CommandOutput {
        c.handle(d, InputEvent::Key(key)).unwrap()
    }
    fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
        for ch in text.chars() {
            let output = key(c, d, Key::Char(ch));
            assert!(!command_status_stops_compound(&output.status), "{ch:?}: {:?}", output.status);
        }
    }

    #[test]
    fn replace_word_and_line_restore_unicode_and_styles_then_repeat_and_undo() {
        for control in ['w', 'u'] {
            for format in [Format::PlainText, Format::Code, Format::Rtf] {
                let source = if format == Format::Rtf { r"{\rtf1{\b ab}cdef}" } else { "abcdef" };
                let mut d = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                let original_styles = [0, 2].map(|at| crate::layout::DocumentLayoutStyles::semantic_character_at(d.projection(), at, false).unwrap());
                let mut c = CommandInterpreter::new();
                keys(&mut c, &mut d, "R");
                c.handle(&mut d, InputEvent::Text(if control == 'w' { "αβ" } else { "α🙂" }.into())).unwrap();
                assert_eq!(key(&mut c, &mut d, Key::Ctrl(control)).status, CommandStatus::Complete);
                assert_eq!(d.text(), "abcdef", "{format:?} Ctrl-{control}");
                assert_eq!([0, 2].map(|at| crate::layout::DocumentLayoutStyles::semantic_character_at(d.projection(), at, false).unwrap()), original_styles);
                if !format.is_rich_text() { assert_eq!(d.source_bytes(), source.as_bytes()); }
                assert_eq!(c.cursor(), 0);
                keys(&mut c, &mut d, "Z");
                key(&mut c, &mut d, Key::Escape);
                assert_eq!(d.text(), "Zbcdef");
                keys(&mut c, &mut d, "u");
                assert_eq!(d.source_bytes(), source.as_bytes());
                key(&mut c, &mut d, Key::Ctrl('r'));
                assert_eq!(d.text(), "Zbcdef");
            }
            let mut d = Document::new("abcdef");
            let mut c = CommandInterpreter::new();
            keys(&mut c, &mut d, "2RXY");
            key(&mut c, &mut d, Key::Ctrl(control));
            keys(&mut c, &mut d, "Z");
            key(&mut c, &mut d, Key::Escape);
            assert_eq!(d.text(), "Zbcdef");
            keys(&mut c, &mut d, "u.");
            assert_eq!(d.text(), "Zbcdef");
        }
    }

    #[test]
    fn copy_uses_tab_columns_and_preserves_graphemes_without_typing_assistance() {
        for (above, column, expected) in [("\tZ", 0, "\t"), ("\tZ", 1, "\t"),
            ("\tZ", 2, "Z"), ("a\u{301}🙂Z", 0, "a\u{301}"),
            ("a\u{301}🙂Z", 2, "🙂"), ("\"", 0, "\"")] {
            for mode in ['i', 'R'] {
                let mut d = Document::new(format!("{above}\nxxxx"));
                let mut c = CommandInterpreter::new();
                c.set_smart_quotes(true);
                assert!(c.set_cursor(&d, above.len() + 1 + column));
                key(&mut c, &mut d, Key::Char(mode));
                key(&mut c, &mut d, Key::Ctrl('y'));
                let suffix = if mode == 'R' { 3 - column } else { 4 - column };
                assert_eq!(d.text(), format!("{above}\n{}{expected}{}", "x".repeat(column), "x".repeat(suffix)));
                if mode == 'R' {
                    key(&mut c, &mut d, Key::Backspace);
                    assert_eq!(d.text(), format!("{above}\nxxxx"));
                }
            }
        }
    }

    #[test]
    fn copied_character_counts_and_dot_keep_the_copied_value() {
        let mut d = Document::new("abc\n\nXYZ\n");
        let mut c = CommandInterpreter::new();
        c.set_cursor(&d, 4);
        keys(&mut c, &mut d, "2i");
        key(&mut c, &mut d, Key::Ctrl('y'));
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "abc\naa\nXYZ\n");
        c.set_cursor(&d, d.text().len());
        keys(&mut c, &mut d, ".");
        assert_eq!(d.text(), "abc\naa\nXYZ\naa");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "abc\naa\nXYZ\n");
    }

    #[test]
    fn copied_character_macros_read_the_new_neighbour() {
        let mut d = Document::new("abc\n\nXYZ\n");
        let mut c = CommandInterpreter::new();
        assert!(c.set_cursor(&d, 4));
        keys(&mut c, &mut d, "qai");
        key(&mut c, &mut d, Key::Ctrl('y'));
        key(&mut c, &mut d, Key::Escape);
        keys(&mut c, &mut d, "q");
        assert_eq!(d.text(), "abc\na\nXYZ\n");
        assert!(c.set_cursor(&d, d.text().len()));
        keys(&mut c, &mut d, "@a");
        assert_eq!(d.text(), "abc\na\nXYZ\nX");
    }

    #[test]
    fn copy_below_and_missing_columns_are_safe_and_do_not_materialize_large_documents() {
        let source = format!("{}abc\nxy\n", "unchanged\n".repeat(20_000));
        let mut d = Document::new(&source);
        let mut c = CommandInterpreter::new();
        let at = source.len() - 7;
        c.set_cursor(&d, at);
        keys(&mut c, &mut d, "i");
        key(&mut c, &mut d, Key::Ctrl('e'));
        assert_eq!(d.projection().text_tree().slice(at..at + 4).unwrap(), "xabc");
        assert!(!d.projection().compatibility_text_is_materialized());
        let revision = d.revision();
        key(&mut c, &mut d, Key::Ctrl('v'));
        key(&mut c, &mut d, Key::Ctrl('y'));
        assert_ne!(d.revision(), revision);
        assert_eq!(d.projection().text_tree().slice(at..at + 2).unwrap(), "x\u{19}");
        key(&mut c, &mut d, Key::Escape);
        c.set_cursor(&d, d.projection().text_tree().byte_len());
        keys(&mut c, &mut d, "i");
        let revision = d.revision();
        key(&mut c, &mut d, Key::Ctrl('e'));
        assert_eq!(d.revision(), revision);
    }

    #[test]
    fn undo_break_splits_typing_but_preserves_the_repeat_recipe() {
        let mut d = Document::new("");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "iabc");
        key(&mut c, &mut d, Key::Ctrl('g'));
        c.handle(&mut d, InputEvent::Text("udef".into())).unwrap();
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "abcdef");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "abc");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "");
        keys(&mut c, &mut d, ".");
        assert_eq!(d.text(), "abcdef");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "");
    }

    #[test]
    fn undo_join_keeps_horizontal_move_and_dot_inside_one_change() {
        let mut core = Core::new(Document::new(""));
        let view = core.add_view(MockTextMeasurementProvider::new(), 80., 150.);
        let sequence = [Key::Char('i'), Key::Char('a'), Key::Char('b'), Key::Ctrl('g'),
            Key::Char('U'), Key::Left, Key::Char('X'), Key::Escape];
        for input in sequence { core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input))).unwrap(); }
        assert_eq!(core.document().text(), "aXb");
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
        assert_eq!(core.document().text(), "");
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('.')))).unwrap();
        assert_eq!(core.document().text(), "aXb");
    }

    #[test]
    fn undo_join_expires_on_intervening_keys_and_bulk_text_in_planned_input() {
        for tail in [
            vec![InputEvent::Key(Key::Char('U')), InputEvent::Key(Key::Char('X'))],
            vec![InputEvent::Text("U".into()), InputEvent::Text("X".into())],
            vec![InputEvent::Text("UX".into())],
        ] {
            let mut core = Core::new(Document::new(""));
            let view = core.add_view(MockTextMeasurementProvider::new(), 80., 150.);
            let events = [Key::Char('i'), Key::Char('a'), Key::Char('b'), Key::Ctrl('g')]
                .into_iter().map(InputEvent::Key).chain(tail).chain(
                    [Key::Left, Key::Char('Y'), Key::Escape, Key::Char('u')]
                        .into_iter().map(InputEvent::Key));
            for event in events {
                let output = core.handle_with_layout(view, CoreEvent::Input(event)).unwrap();
                let command = output.command.expect("input has a command outcome");
                assert!(!command_status_stops_compound(&command.status), "{:?}", command.status);
            }
            assert_eq!(core.document().text(), "abX");
        }
    }

    #[test]
    fn undo_join_is_consumed_by_other_operations_but_not_empty_text() {
        for operation in [Key::Backspace, Key::Ctrl('e'), Key::Ctrl('v'),
            Key::Ctrl('r'), Key::Ctrl('g'), Key::Tab, Key::WordLeft] {
            let mut d = Document::new("abcd");
            let mut c = CommandInterpreter::new();
            keys(&mut c, &mut d, "iab");
            key(&mut c, &mut d, Key::Ctrl('g'));
            c.handle(&mut d, InputEvent::Text("U".into())).unwrap();
            c.handle(&mut d, InputEvent::Text(String::new())).unwrap();
            assert!(c.insert_controls.join_next_horizontal);
            key(&mut c, &mut d, operation);
            assert!(!c.insert_controls.join_next_horizontal, "{operation:?}");
        }
    }

    #[test]
    fn repeated_joined_movement_updates_the_insert_deletion_floor() {
        for mode in ['i', 'R'] {
            let mut d = Document::new("abcd");
            let mut c = CommandInterpreter::new();
            assert!(c.set_cursor(&d, 2));
            key(&mut c, &mut d, Key::Char(mode));
            key(&mut c, &mut d, Key::Ctrl('g'));
            keys(&mut c, &mut d, "U");
            key(&mut c, &mut d, Key::Left);
            keys(&mut c, &mut d, "x");
            key(&mut c, &mut d, Key::Ctrl('u'));
            key(&mut c, &mut d, Key::Escape);
            assert_eq!(d.text(), "abcd");
            assert!(c.set_cursor(&d, 2));
            keys(&mut c, &mut d, ".");
            assert_eq!(d.text(), "abcd", "{mode}");
        }
    }

    #[test]
    fn repeated_joined_movement_invalidates_old_replace_restoration() {
        let mut d = Document::new("abcdef");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "RXY");
        key(&mut c, &mut d, Key::Ctrl('g'));
        keys(&mut c, &mut d, "U");
        key(&mut c, &mut d, Key::Left);
        keys(&mut c, &mut d, "Z");
        key(&mut c, &mut d, Key::Backspace);
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "XYcdef");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "abcdef");
        keys(&mut c, &mut d, ".");
        assert_eq!(d.text(), "XYcdef");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "abcdef");
    }

    #[test]
    fn copied_character_ignores_mixed_font_metrics_and_wrap_width() {
        let source = br"{\rtf1\ansi\fs96 iii\fs16 W\par xxxx}";
        for width in [25., 400.] {
            let document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), width, 200.);
            for input in "G0llli".chars().map(Key::Char).chain([Key::Ctrl('y'), Key::Escape]) {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input))).unwrap();
            }
            assert_eq!(core.document().text(), "iiiW\nxxxWx", "wrap width {width}");
        }
    }

    #[test]
    fn core_replace_delete_uses_restoration_instead_of_the_flat_delete_plan() {
        for control in ['w', 'u'] {
            let mut core = Core::new(Document::new("abcdef"));
            let view = core.add_view(MockTextMeasurementProvider::new(), 100., 150.);
            for input in [Key::Char('R'), Key::Char('X'), Key::Char('Y'), Key::Ctrl(control)] {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(input))).unwrap();
            }
            assert_eq!(core.document().text(), "abcdef");
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        }
    }
}

impl CommandInterpreter {
    pub(super) fn insert_control_g_pending(&self) -> bool {
        matches!(self.mode, Mode::Insert | Mode::Replace) && self.insert_controls.g_pending
    }

    pub(super) fn handle_insert_control_key(
        &mut self, document: &mut Document, key: Key,
    ) -> Result<CommandOutput, DocumentError> {
        self.insert_controls.g_pending = false;
        match key {
            Key::Char('u') => {
                document.end_edit_group();
                document.begin_edit_group();
                self.insert_controls.join_next_horizontal = false;
                if let Some(session) = self.insert_session.as_mut() {
                    session.record_step(EditSessionStep::UndoBreak);
                }
                Ok(CommandOutput::complete())
            }
            Key::Char('U') => {
                self.insert_controls.join_next_horizontal = true;
                Ok(CommandOutput::complete())
            }
            Key::Escape | Key::Ctrl('[') => self.finish_insert(document),
            _ => Ok(CommandOutput::unsupported("Insert Ctrl-G expects u or U")),
        }
    }

    pub(super) fn handle_insert_control_text(
        &mut self, document: &mut Document, input: &str,
    ) -> Result<CommandOutput, DocumentError> {
        let Some(first) = input.chars().next() else { return Ok(CommandOutput::pending()); };
        let mut output = self.handle_insert_control_key(document, Key::Char(first))?;
        if !command_status_stops_compound(&output.status) && first.len_utf8() < input.len() {
            output.merge(self.handle_text(document, input[first.len_utf8()..].to_owned())?);
        }
        Ok(output)
    }

    /// Ctrl-G U retains the current undo unit and repeat program for the next
    /// ordinary horizontal movement within a hard line. Other motions keep
    /// their existing undo boundaries.
    pub(super) fn join_insert_horizontal_move(
        &mut self, document: &Document, key: Key,
    ) -> Option<CommandOutput> {
        if !std::mem::take(&mut self.insert_controls.join_next_horizontal) { return None; }
        let direction = match key { Key::Left => -1, Key::Right => 1, _ => return None };
        let lines = document.hard_line_snapshot();
        let line = lines.line_at_offset(self.cursor).ok()?.content_range();
        let target = if direction < 0 {
            lines.previous_grapheme_boundary(self.cursor).unwrap_or(self.cursor)
        } else { lines.next_grapheme_boundary(self.cursor).unwrap_or(self.cursor) };
        if target < line.start || target > line.end { return None; }
        Some(self.move_joined_insert_horizontal(document, direction))
    }

    pub(super) fn move_joined_insert_horizontal(
        &mut self, document: &Document, direction: isize,
    ) -> CommandOutput {
        self.invalidate_replace_restoration();
        let output = self.move_cursor(document, Motion::InsertionHorizontal(direction), 1);
        if let Some(session) = self.insert_session.as_mut() {
            session.unit_floor = session.unit_floor.min(self.cursor);
            session.record_step(EditSessionStep::JoinedHorizontalMove(direction));
        }
        output
    }

    /// Copy by tab-expanded logical columns, never by shaped glyph widths.
    /// Only the current prefix and one neighbouring hard line are read.
    pub(super) fn copy_adjacent_character(
        &mut self, document: &mut Document, below: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let lines = document.hard_line_snapshot();
        let at = self.visual_block_insert.as_ref()
            .and_then(|session| session.rows.first()).map_or(self.cursor, |row| row.insertion_offset);
        let line = lines.line_at_offset(at).expect("insertion caret has a hard line");
        let index = if below { line.index().checked_add(1) } else { line.index().checked_sub(1) };
        let Some(neighbour) = index.and_then(|index| lines.line(index)) else {
            return Ok(CommandOutput::complete());
        };
        let options = self.indentation_options();
        let prefix = lines.slice_utf8(line.content_range().start..at)
            .expect("insertion prefix is a legal range");
        let mut column = options.columns(&prefix);
        if let Some(session) = &self.visual_block_insert {
            column = crate::document::indentation_end_column(column, &session.payload, options.tabstop as usize);
        }
        let text = lines.slice_utf8(neighbour.content_range()).expect("neighbour has a legal range");
        let mut start = 0;
        for grapheme in text.graphemes(true) {
            let end = crate::document::indentation_end_column(start, grapheme, options.tabstop as usize);
            // A zero-width standalone grapheme is still an editable item.
            if column < end || (column == start && end == start) {
                return self.insert_copied_character(document, grapheme);
            }
            start = end;
        }
        Ok(CommandOutput::complete())
    }

    pub(super) fn insert_copied_character(
        &mut self, document: &mut Document, text: &str,
    ) -> Result<CommandOutput, DocumentError> {
        if let Some(session) = self.visual_block_insert.as_mut() {
            let start = session.payload.len();
            session.payload.push_str(text);
            session.literal_ranges.push(start..session.payload.len());
            return Ok(CommandOutput::complete());
        }
        self.generated_indent = None;
        self.input_assistance.literal = true;
        let program = self.insert_session.as_mut().and_then(|session| session.repeat_program.take());
        let value = RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
            .expect("copied grapheme is literal content");
        let result = if self.mode == Mode::Replace {
            self.replace_register_payload(document, &value)
        } else { self.insert_register_payload(document, &value) };
        self.input_assistance.literal = false;
        if let Some(session) = self.insert_session.as_mut() {
            session.repeat_program = program;
            if result.is_ok() {
                session.record_step(EditSessionStep::CopiedCharacter(text.to_owned()));
            }
        }
        result
    }

    /// Walk the same replacement frontier as Backspace, but record one semantic
    /// word/line operation. The enclosing command checkpoint makes failures
    /// atomic, including source-preserving rich-text restoration.
    pub(super) fn replace_delete_motion(
        &mut self, document: &mut Document, step: EditSessionStep,
    ) -> Result<CommandOutput, DocumentError> {
        let range = match self.insert_delete_motion_range(document, &step) {
            Ok(range) => range, Err(output) => return Ok(output),
        };
        if range.is_empty() { return Ok(CommandOutput::complete()); }
        let removed = document.hard_line_snapshot().slice_utf8(range.clone()).expect("validated deletion");
        let replaying = self.insert_session.as_ref().is_some_and(|session| session.replaying_program);
        if let Some(session) = self.insert_session.as_mut() { session.replaying_program = true; }
        let result = (|| {
            let mut output = CommandOutput::complete();
            while self.cursor > range.start {
                let frontier = self.insert_session.as_ref().and_then(|session| session.replace_journal.last())
                    .is_some_and(|entry| entry.frontier() == Some(self.cursor) && entry.start >= range.start);
                if !frontier {
                    output.merge(self.delete_insert_mode_range(document, range.start..self.cursor, step.clone())?);
                    break;
                }
                let before = self.cursor;
                output.merge(self.edit_mode_backspace(document)?);
                if self.cursor >= before { break; }
            }
            Ok(output)
        })();
        if let Some(session) = self.insert_session.as_mut() {
            session.replaying_program = replaying;
            if result.is_ok() { session.record_deleted(&removed, step); }
        }
        result
    }
}
