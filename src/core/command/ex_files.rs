//! Portable policy for command files and linewise insertion of host-loaded data.
use super::*;

impl CommandInterpreter {
    /// Stage `text` as a typed `:` command line; the caller dispatches Enter
    /// through the ordinary command pipeline, which runs it as Vim script.
    pub(crate) fn stage_command_line(&mut self, text: &str) -> Result<(), String> {
        if self.substitute_confirmation.is_some() {
            return Err("finish substitute confirmation before running commands".into());
        }
        if self.mode != Mode::Normal {
            return Err("command lines run from a completed Normal-mode command".into());
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        let text = text.trim_start();
        let text = text.strip_prefix(':').unwrap_or(text);
        self.enter_command_line(CommandLineKind::Ex);
        self.command_line_state
            .as_mut()
            .expect("entered command line")
            .buffer
            .set(text.to_owned());
        Ok(())
    }
}

/// Loaded text is decoded independently of the current rich format. `:read`
/// inserts literal lines through the existing lossless formatted payload path.
pub(crate) fn read_file_edit(
    document: &Document,
    after: ex_execute::ExInsertionPoint,
    bytes: &[u8],
) -> Result<Option<(crate::document::ModelRequest, usize)>, String> {
    if bytes.is_empty() { return Ok(None); }
    let loaded = Document::from_bytes_detect_encoding(bytes.to_vec(), crate::document::Format::PlainText)
        .map_err(|error| error.to_string())?;
    if loaded.projection().text_tree().byte_len() == 0 { return Ok(None); }
    let raw = loaded.text();
    let body = raw.strip_suffix('\n').unwrap_or(raw);
    let empty = document.projection().text_tree().byte_len() == 0;
    let eof = if after.source { after.offset == document.source_byte_len() }
        else { after.offset == document.projection().text_tree().byte_len() };
    let needs_prefix = !empty && if after.source {
        if eof {
            let last = document.physical_source_line_number_count() - 1;
            let (source, content) = document.physical_line_ranges(last).map_err(|error| error.to_string())?;
            source.end == content.end
        } else { false }
    } else {
        let line = document.hard_line_snapshot().line_at_offset(after.offset)
            .map_err(|error| error.to_string())?;
        line.content_range().start < after.offset && after.offset <= line.content_range().end
    };
    let mut text = String::new();
    if needs_prefix { text.push('\n'); }
    text.push_str(body);
    if !empty && (!eof || !needs_prefix) { text.push('\n'); }
    let first_nonblank = body.grapheme_indices(true)
        .take_while(|(_, value)| *value != "\n")
        .find(|(_, value)| !value.chars().all(char::is_whitespace))
        .map_or(0, |(offset, _)| offset);
    let breaks = text.match_indices('\n').map(|(offset, _)| offset).collect();
    let payload = crate::document::FormattedTextPayload::new(&document.hard_line_snapshot(), text, breaks)
        .map_err(|error| error.to_string())?;
    let request = if after.source {
        crate::document::ModelRequest::InsertAtSource { document: document.id(), revision: document.revision(),
            at: document.source_point(after.offset).map_err(|error| error.to_string())?, payload }
    } else {
        document.text_point(after.offset).map_err(|error| error.to_string())?;
        crate::document::ModelRequest::ApplyFragmentEdits { document: document.id(), revision: document.revision(),
            edits: vec![crate::document::FragmentEdit { range: after.offset..after.offset,
                fragments: vec![crate::document::ReplacementFragment::Literal(payload)] }] }
    };
    // Resolve the cursor against the verified candidate before the native
    // transaction publishes, including an invisible source destination.
    let prepared = document.prepare_model_request(request.clone()).map_err(|error| error.to_string())?;
    let candidate = document.prepared_candidate_document(&prepared).map_err(|error| error.to_string())?;
    let start = if after.source { candidate.visible_point_for_source(after.offset, true).map_err(|error| error.to_string())? }
        else { super::prepared_cursor(document, &prepared, after.offset, crate::document::Association::BeforeInsertion).map_err(|error| error.to_string())? };
    let cursor = start + usize::from(needs_prefix) + first_nonblank;
    candidate.text_point(cursor).map_err(|error| error.to_string())?;
    Ok(Some((request, cursor)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Revision;

    fn read_point(document: &Document, line: usize) -> ex_execute::ExInsertionPoint {
        let offset = if line == 0 { document.physical_line_ranges(0).unwrap().0.start }
            else { document.physical_line_ranges(line - 1).map_or(usize::MAX, |(line, _)| line.end) };
        ex_execute::ExInsertionPoint { offset, source: true }
    }

    fn read(document: &mut Document, after: usize, bytes: &[u8]) {
        if let Some((edit, _)) = read_file_edit(document, read_point(document, after), bytes).unwrap() {
            let prepared = document
                .prepare_model_request(edit)
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
        assert!(read_file_edit(&document, read_point(&document, 99), b"bad").is_err());
        assert_eq!(document.history_status(), before);
    }

    fn run(c: &mut CommandInterpreter, d: &mut Document, line: &str) -> CommandOutput {
        c.stage_command_line(line).unwrap();
        c.handle(d, InputEvent::Key(Key::Enter)).unwrap()
    }

    #[test]
    fn staged_lines_run_as_scripts_and_keep_mapping_trailing_spaces() {
        let mut c = CommandInterpreter::new();
        let mut d = Document::new("first\nsecond");
        assert!(matches!(run(&mut c, &mut d, "  \" comment\r").status, CommandStatus::Complete));
        assert_eq!(d.revision(), Revision(0));
        assert!(matches!(run(&mut c, &mut d, "map Y iabc  \r").status, CommandStatus::Complete));
        c.handle(&mut d, InputEvent::Key(Key::Char('Y'))).unwrap();
        assert!(d.text().starts_with("abc  first"), "{}", d.text());
    }

    #[test]
    fn scripts_reject_substitute_confirmation_including_nested_normal() {
        let mut c = CommandInterpreter::new();
        let mut d = Document::new("one one");
        let output = run(&mut c, &mut d, "exe \"normal :s/one/two/gc\\r\" | echo 'after'");
        assert!(!matches!(output.status, CommandStatus::Pending));
        assert!(c.substitute_confirmation_prompt().is_none());
        assert_eq!(d.text(), "one one");
        let output = run(&mut c, &mut d, "s/one/two/gc | echo 'after'");
        assert!(format!("{:?}", output.status).contains("Substitute confirmation is not available in scripts"), "{output:?}");
        assert!(c.substitute_confirmation_prompt().is_none());

        // Typed alone, the command keeps its interactive prompt.
        run(&mut c, &mut d, "s/one/two/gc");
        assert!(c.substitute_confirmation_prompt().is_some());
        assert!(c.stage_command_line("set wrap").is_err());
        c.handle(&mut d, InputEvent::Key(Key::Char('q'))).unwrap();
        assert!(c.substitute_confirmation_prompt().is_none());
        assert_eq!(d.text(), "one one");
    }
    #[test]
    fn read_cursor_targets_first_nonblank_of_first_inserted_line() {
        for (text, after, expected) in [
            ("first\nlast", 1, 8),
            ("last", 0, 2),
            ("last", 1, 7),
            ("last\n", 1, 7),
            ("", 1, 2),
        ] {
            let document = Document::new(text);
            let (_, cursor) = read_file_edit(&document, read_point(&document, after), b"  one\n  two\n")
                .unwrap()
                .unwrap();
            assert_eq!(cursor, expected, "{text:?} after {after}");
        }
        let document = Document::new("first\nlast");
        let (_, cursor) = read_file_edit(&document, read_point(&document, 1), b"  \n  next\n").unwrap().unwrap();
        assert_eq!(cursor, 6, "blank first imported line stays at its own boundary");
    }
}
