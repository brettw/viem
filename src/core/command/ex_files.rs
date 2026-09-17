//! Portable policy for command files and linewise insertion of host-loaded data.
use super::*;

pub const SOURCE_MAX_DEPTH: u32 = 16;
pub const SOURCE_MAX_BYTES: usize = 1024 * 1024;
pub const SOURCE_MAX_COMMANDS: usize = 10_000;

impl CommandInterpreter {
    pub(crate) fn finish_sourced_line(&mut self) {
        self.source_replay_depth = 0;
    }

    /// Prepare a sourced Ex line for the ordinary Enter dispatch. This bypasses
    /// Normal-mode mappings while retaining the normal Ex command interpreter.
    pub(crate) fn queue_sourced_line(&mut self, text: &str, depth: u32) -> Result<bool, String> {
        if depth == 0 || depth > SOURCE_MAX_DEPTH {
            return Err("source nesting limit exceeded".into());
        }
        if text.len() > SOURCE_MAX_BYTES {
            return Err("source command is too large".into());
        }
        if self.substitute_confirmation.is_some() {
            return Err("finish substitute confirmation before sourcing commands".into());
        }
        if self.mode != Mode::Normal {
            return Err("source requires a completed Normal-mode command".into());
        }
        let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
        let text = text.strip_suffix('\r').unwrap_or(text);
        let text = text.strip_prefix(':').unwrap_or(text).trim_start();
        if text.is_empty() || text.starts_with('"') {
            return Ok(false);
        }
        if let Ok(command) = ex::parse_ex(text) {
            let confirmation = self.ex_state.requests_confirmation(&command.action);
            if confirmation {
                return Err(
                    "interactive substitute confirmation is unavailable in source files".into(),
                );
            }
        }
        self.source_replay_depth = depth;
        self.enter_command_line(CommandLineKind::Ex);
        self.command_line_state
            .as_mut()
            .expect("entered command line")
            .buffer
            .set(text.to_owned());
        Ok(true)
    }
}

/// Loaded text is decoded independently of the current rich format. `:read`
/// inserts literal lines through the existing lossless formatted payload path.
pub(crate) fn read_file_edit(
    document: &Document,
    after: usize,
    bytes: &[u8],
) -> Result<Option<(crate::document::FragmentEdit, usize)>, String> {
    if after > document.line_count() {
        return Err("read address is outside the document".into());
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    let loaded =
        Document::from_bytes_detect_encoding(bytes.to_vec(), crate::document::Format::PlainText)
            .map_err(|error| error.to_string())?;
    if loaded.text().is_empty() {
        return Ok(None);
    }
    let first_line = loaded
        .hard_line_snapshot()
        .line(0)
        .expect("a document has one line")
        .content_range();
    let first_nonblank = loaded.text()[first_line]
        .grapheme_indices(true)
        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .map_or(0, |(offset, _)| offset);
    if document.projection().text_tree().byte_len() == 0 {
        let text = loaded
            .text()
            .strip_suffix('\n')
            .unwrap_or(loaded.text())
            .to_owned();
        let breaks = text.match_indices('\n').map(|(offset, _)| offset).collect();
        let cursor = first_nonblank;
        let payload = crate::document::FormattedTextPayload::new(
            &document.hard_line_snapshot(),
            text,
            breaks,
        )
        .map_err(|error| error.to_string())?;
        return Ok(Some((
            crate::document::FragmentEdit {
                range: 0..0,
                fragments: vec![crate::document::ReplacementFragment::Literal(payload)],
            },
            cursor,
        )));
    }
    let value = ExRegisterValue::linewise(loaded.text());
    let (edit, _) =
        ex_execute::plan_put(document, after, &value).map_err(|error| error.to_string())?;
    // :read selects the first inserted line, unlike :put's last-line cursor.
    let prefix_break = usize::from(after == document.line_count() && after != 0);
    let cursor = edit.range().start + prefix_break + first_nonblank;
    Ok(Some((
        crate::document::FragmentEdit {
            range: edit.range(),
            fragments: vec![crate::document::ReplacementFragment::Literal(
                edit.payload().clone(),
            )],
        },
        cursor,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ModelRequest, Revision};

    fn read(document: &mut Document, after: usize, bytes: &[u8]) {
        if let Some((edit, _)) = read_file_edit(document, after, bytes).unwrap() {
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyFragmentEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![edit],
                })
                .unwrap();
            document.commit_model_transaction(prepared).unwrap();
        }
    }

    #[test]
    fn read_decodes_boms_line_endings_and_preserves_untouched_target_source() {
        let mut document = Document::from_bytes(
            b"first\r\nlast".to_vec(),
            crate::document::Encoding::Utf8,
            crate::document::Format::PlainText,
        )
        .unwrap();
        read(&mut document, 1, b"\xef\xbb\xbf  middle\r\n  next\r\n");
        assert_eq!(document.text(), "first\n  middle\n  next\nlast");
        assert_eq!(
            document.source_bytes(),
            b"first\r\n  middle\r\n  next\r\nlast"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"first\r\nlast");
        assert!(!document.undo());
        assert!(document.redo());
        assert_eq!(document.text(), "first\n  middle\n  next\nlast");
    }

    #[test]
    fn read_zero_eof_empty_file_and_unicode_produce_single_verified_changes() {
        let mut document = Document::new("tail");
        read(&mut document, 0, &[0xff, 0xfe, 0xe9, 0x00, 0x0a, 0x00]);
        assert_eq!(document.text(), "é\ntail");
        read(&mut document, 2, b"end");
        assert_eq!(document.text(), "é\ntail\nend");
        let before = document.history_status();
        read(&mut document, 1, b"");
        assert_eq!(document.history_status(), before);
        assert!(read_file_edit(&document, 99, b"bad").is_err());
        assert_eq!(document.history_status(), before);
    }

    #[test]
    fn source_comments_limits_confirmation_and_mapping_trailing_spaces() {
        let mut c = CommandInterpreter::new();
        let mut d = Document::new("first\nsecond");
        assert!(!c.queue_sourced_line("\u{feff}  \" comment\r", 1).unwrap());
        assert!(c.queue_sourced_line("set wrap", 0).is_err());
        assert!(c
            .queue_sourced_line("set wrap", SOURCE_MAX_DEPTH + 1)
            .is_err());
        assert!(c.queue_sourced_line("s/i/I/c", 1).is_err());
        assert_eq!(c.mode(), Mode::Normal);
        assert_eq!(d.revision(), Revision(0));
        assert!(c.queue_sourced_line("map Y iabc  \r", 1).unwrap());
        assert_eq!(c.command_line(), Some("map Y iabc  "));
        assert!(matches!(
            c.handle(&mut d, InputEvent::Key(Key::Enter))
                .unwrap()
                .status,
            CommandStatus::Complete
        ));
        c.handle(&mut d, InputEvent::Key(Key::Char('Y'))).unwrap();
        assert!(d.text().starts_with("abc  first"), "{}", d.text());
    }
    #[test]
    fn sourced_normal_rejects_nested_confirmation_and_effective_flags() {
        let mut c = CommandInterpreter::new();
        let mut d = Document::new("one one");
        assert!(c.queue_sourced_line("normal :s/one/two/gc\n", 1).unwrap());
        let output = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
        c.finish_sourced_line();
        assert!(!matches!(output.status, CommandStatus::Pending));
        assert!(c.substitute_confirmation_prompt().is_none());
        assert_eq!(d.text(), "one one");

        c.enter_command_line(CommandLineKind::Ex);
        c.command_line_state
            .as_mut()
            .unwrap()
            .buffer
            .set("s/one/two/gc".into());
        c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
        assert!(c.substitute_confirmation_prompt().is_some());
        assert!(c.queue_sourced_line("set wrap", 1).is_err());
        c.handle(&mut d, InputEvent::Key(Key::Char('q'))).unwrap();
        assert!(c.queue_sourced_line("&&", 1).is_err());
        assert!(c.substitute_confirmation_prompt().is_none());
        assert_eq!(d.text(), "one one");
    }
    #[test]
    fn read_cursor_targets_first_nonblank_of_first_inserted_line() {
        for (text, after, expected) in [
            ("first\nlast", 1, 8),
            ("last", 0, 2),
            ("last", 1, 7),
            ("last\n", 2, 8),
            ("", 1, 2),
        ] {
            let document = Document::new(text);
            let (_, cursor) = read_file_edit(&document, after, b"  one\n  two\n")
                .unwrap()
                .unwrap();
            assert_eq!(cursor, expected, "{text:?} after {after}");
        }
        let document = Document::new("first\nlast");
        let (_, cursor) = read_file_edit(&document, 1, b"  \n  next\n").unwrap().unwrap();
        assert_eq!(cursor, 6, "blank first imported line stays at its own boundary");
    }
}
