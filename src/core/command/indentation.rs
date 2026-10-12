//! Logical whitespace editing. Font-dependent indentation is layout policy.
use super::*;
use crate::document::{
    comment_continuation_prefix, leading_whitespace_len,
    IndentationOptions, MappingOutcome,
};

#[derive(Clone, Debug)]
pub(super) struct GeneratedIndent {
    start: TextAnchor,
    end: TextAnchor,
    text: String,
    comment: Option<GeneratedComment>,
}

/// The closing spelling is resolved at the originating comment, before the
/// edit. A shared middle marker never has to identify a block afterwards.
#[derive(Clone, Debug)]
struct GeneratedComment {
    block: usize,
    closing: String,
    revision: crate::document::Revision,
    language: Option<String>,
    profile: String,
}

#[derive(Default)]
pub(super) struct GeneratedPrefix {
    pub text: String,
    block: Option<usize>,
    closing: Option<String>,
}

impl From<String> for GeneratedPrefix {
    fn from(text: String) -> Self {
        Self { text, block: None, closing: None }
    }
}

impl GeneratedIndent {
    fn capture(document: &Document, start: usize, text: String) -> Option<Self> {
        let anchor = |at| {
            document
                .text_anchor(
                    document.text_point(at).ok()?,
                    Association::BeforeInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::Unresolvable,
                )
                .ok()
        };
        Some(Self {
            start: anchor(start)?,
            end: anchor(start + text.len())?,
            text,
            comment: None,
        })
    }
    fn range(&self, document: &Document) -> Option<Range<usize>> {
        let resolve = |anchor| match document.resolve_text_anchor(anchor).ok()? {
            MappingOutcome::Exact(point) | MappingOutcome::Moved(point) => Some(point.offset()),
            _ => None,
        };
        let range = resolve(self.start)?..resolve(self.end)?;
        (document
            .hard_line_snapshot()
            .slice_utf8(range.clone())
            .ok()?
            == self.text)
            .then_some(range)
    }
}

impl CommandInterpreter {
    pub fn set_indentation_defaults(
        &mut self,
        options: IndentationOptions,
    ) -> Result<(), &'static str> {
        options.validate()?;
        self.indentation.set_default(options);
        self.reflow_options.set_continuation_defaults(options.continue_comments_on_enter, options.continue_comments_on_open_line);
        Ok(())
    }
    pub fn indentation_options(&self) -> IndentationOptions {
        let mut value = self.indentation.effective();
        value.continue_comments_on_enter = self.reflow_options.has('r');
        value.continue_comments_on_open_line = self.reflow_options.has('o');
        value
    }

    pub(super) fn literal_indentation(&self, document: &Document) -> bool {
        document.format().is_literal() || document.format().is_source_view()
    }

    fn commit_indent_edit(
        &mut self,
        document: &mut Document,
        edit: TextEdit,
        floor: usize,
    ) -> Result<(), DocumentError> {
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![edit],
            })
            .map_err(command_document_error)?;
        let cursor = prepared_cursor(document, &prepared, self.cursor, Association::AfterInsertion)?;
        // Explicit indentation changes bring the edited prefix into the Insert
        // unit. A still-valid old offset can otherwise end up after the caret.
        let floor = self
            .insert_session
            .as_ref()
            .map(|session| {
                prepared_cursor(
                    document,
                    &prepared,
                    session.unit_floor.min(floor),
                    Association::BeforeInsertion,
                )
            })
            .transpose()?;
        document
            .commit_model_transaction(prepared)
            .map_err(command_document_error)?;
        self.cursor = cursor;
        if let (Some(session), Some(floor)) = (self.insert_session.as_mut(), floor) {
            session.unit_floor = floor;
        }
        Ok(())
    }

    pub(super) fn continuation_indent(
        &self,
        document: &Document,
        at: usize,
        open: bool,
        above: bool,
    ) -> GeneratedPrefix {
        if !self.literal_indentation(document) {
            return GeneratedPrefix::default();
        }
        let options = self.indentation_options();
        let snapshot = document.hard_line_snapshot();
        let Ok(line) = snapshot.line_at_offset(at) else {
            return GeneratedPrefix::default();
        };
        let range = line.content_range();
        let text = snapshot
            .slice_utf8(range.start..at.min(range.end))
            .unwrap_or_default();
        let comments = if open {
            options.continue_comments_on_open_line
        } else {
            options.continue_comments_on_enter
        };
        if comments && document.format().is_code() {
            let profile = self.reflow_options.profile();
            if let Some(continuation) = comment_continuation_prefix(
                &snapshot, line.index(), &text, &profile,
                self.reflow_options.language(), above, options.tabstop as usize,
            ) {
                // `/` applies only to lowercase o after trailing line comments.
                // Enter still continues them; O never copies one above its code.
                if !continuation.trailing || (!above && !(open && self.reflow_options.has('/'))) {
                    let normalize = |prefix: &str| {
                        let leading = leading_whitespace_len(prefix);
                        format!("{}{}", options.whitespace(0, options.columns(&prefix[..leading])), &prefix[leading..])
                    };
                    return GeneratedPrefix {
                        text: normalize(&continuation.prefix),
                        block: continuation.block,
                        closing: continuation.closing_prefix.as_deref().map(normalize),
                    };
                }
            }
        }
        if options.autoindent {
            let prefix = &text[..leading_whitespace_len(&text)];
            options.whitespace(0, options.columns(prefix)).into()
        } else {
            GeneratedPrefix::default()
        }
    }

    pub(super) fn capture_generated_indent(&mut self, document: &Document, prefix: impl Into<GeneratedPrefix>) {
        let prefix = prefix.into();
        self.generated_indent = if prefix.text.is_empty() {
            None
        } else {
            GeneratedIndent::capture(document, self.cursor - prefix.text.len(), prefix.text).map(|mut generated| {
                generated.comment = prefix.block.zip(prefix.closing).map(|(block, closing)| GeneratedComment {
                    block,
                    closing,
                    revision: document.revision(),
                    language: self.reflow_options.language().map(str::to_owned),
                    profile: self.reflow_options.comments().to_owned(),
                });
                generated
            })
        };
    }

    /// Only remove unchanged, automatically generated indentation from an
    /// otherwise empty line. Authored blank lines and whitespace are retained.
    pub(super) fn cleanup_generated_indent(
        &mut self,
        document: &mut Document,
    ) -> Result<bool, DocumentError> {
        let Some(generated) = self.generated_indent.take() else {
            return Ok(false);
        };
        if generated.text.bytes().any(|b| !matches!(b, b' ' | b'\t')) {
            return Ok(false);
        }
        let Some(range) = generated.range(document) else {
            return Ok(false);
        };
        let snapshot = document.hard_line_snapshot();
        if self.cursor != range.end
            || snapshot
                .line_at_offset(range.start)
                .map(|l| l.content_range())
                != Ok(range.clone())
        {
            return Ok(false);
        }
        self.cursor = delete_with_cursor(document, range)?;
        if let Some(session) = self.insert_session.as_mut() {
            remove_inserted_suffix(
                &mut session.last_inserted,
                &generated.text,
            );
        }
        Ok(true)
    }

    fn record_semantic_insert(
        &mut self,
        document: &mut Document,
        text: &str,
        step: EditSessionStep,
    ) -> Result<CommandOutput, DocumentError> {
        let replaying = self
            .insert_session
            .as_ref()
            .is_some_and(|s| s.replaying_program);
        if let Some(session) = self.insert_session.as_mut() {
            session.replaying_program = true;
        }
        let replace_tab = self.mode == Mode::Replace && matches!(step, EditSessionStep::Tab);
        let result =
            if self.mode == Mode::Replace && !matches!(step, EditSessionStep::IndentedEnter) {
                self.replace_text(document, text)
            } else {
                self.insert_text(document, text)
            };
        if let Some(session) = self.insert_session.as_mut() {
            session.replaying_program = replaying;
            if result.is_ok() {
                if replace_tab {
                    let start = session
                        .replace_journal
                        .len()
                        .saturating_sub(text.graphemes(true).count());
                    let entries = &session.replace_journal[start..];
                    if !entries.is_empty() && entries.iter().all(|entry| entry.source_record.is_some()) {
                        let records = entries.iter().filter_map(|entry| entry.source_record.clone()).collect::<Vec<_>>();
                        let record = document.combine_recorded_replacements(&records)?;
                        let combined = ReplaceJournalEntry {
                            autoindent: false, start: record.before_cursor, inserted: record.inserted.clone(),
                            original: record.original.clone(), source_record: Some(record),
                        };
                        session.replace_journal.truncate(start);
                        session.replace_journal.push(combined);
                    } else
                    if !entries.is_empty()
                        && entries.iter().all(|entry| entry.source_record.is_none())
                        && entries
                            .iter()
                            .map(|entry| entry.inserted.as_str())
                            .collect::<String>()
                            == text
                    {
                        let combined = ReplaceJournalEntry {
                            autoindent: false,
                            start: entries[0].start,
                            inserted: text.to_owned(),
                            original: Some(
                                entries
                                    .iter()
                                    .filter_map(|entry| entry.original.as_deref())
                                    .collect(),
                            ),
                            source_record: None,
                        };
                        session.replace_journal.truncate(start);
                        session.replace_journal.push(combined);
                    }
                }
                session.record_inserted(&RegisterValue::characterwise(text), Some(step));
            }
        }
        result
    }

    pub(super) fn insert_indented_break(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        let prefix = self
            .restored_indent
            .take()
            .map(|columns| self.indentation_options().whitespace(0, columns).into())
            .unwrap_or_else(|| self.continuation_indent(document, self.cursor, false, false));
        self.cleanup_generated_indent(document)?;
        let start = self.cursor;
        if let Some((prepared, cursor)) = document.prepare_literal_terminal_break(start, &prefix.text)
            .map_err(command_document_error)?
        {
            let inserted = format!("\n{}", prefix.text);
            let record = if prefix.text.is_empty() || self.mode == Mode::Replace {
                Some(document.recorded_source_replacement(&prepared, start, cursor, inserted.clone(), None)
                    .map_err(command_document_error)?)
            } else { None };
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            self.cursor = cursor;
            if let Some(session) = self.insert_session.as_mut() {
                if let Some(record) = record {
                    session.replace_journal.push(ReplaceJournalEntry {
                        autoindent: false, start, inserted: inserted.clone(), original: record.original.clone(),
                        source_record: Some(record),
                    });
                }
                session.record_inserted(&RegisterValue::characterwise(inserted), Some(EditSessionStep::IndentedEnter));
            }
            self.capture_generated_indent(document, prefix);
            return Ok(CommandOutput { document_changed: true, cursor_moved: true, ..CommandOutput::complete() });
        }
        if self.mode == Mode::Replace {
            let snapshot = document.hard_line_snapshot();
            let line_end = line_end(&snapshot, start);
            let suffix = snapshot.slice_utf8(start..line_end).map_err(DocumentError::FormattedTextStorage)?;
            let removed_len = if self.indentation_options().autoindent && self.literal_indentation(document) {
                leading_whitespace_len(&suffix)
            } else { 0 };
            let inserted = format!("\n{}", prefix.text);
            let payload = FormattedTextPayload::new(&snapshot, inserted.clone(), vec![0])
                .expect("Enter contains one semantic break");
            let edit = FormattedPayloadEdit::new(start..start + removed_len, payload)
                .with_boundary_affinity(self.markdown_typing_affinity(document));
            let use_markdown_typing = document.format().is_wysiwyg() && !self.typing_style.link_disabled
                && self.markdown_autodetect();
            let mut exit_source = None;
            let heading_enter = document.format().is_wysiwyg() && prefix.text.is_empty()
                && document.projection().blocks_for_region(&(start..start)).iter()
                    .any(|block| matches!(block.kind, crate::document::BlockKind::Heading(_))
                        && block.range.start <= start && start <= block.range.end);
            let (prepared, after) = if heading_enter {
                let prepared = document.prepare_model_request(ModelRequest::ContinueList {
                    document: document.id(), revision: document.revision(), at: start,
                }).map_err(command_document_error)?;
                let after = prepared_list_enter_cursor(document, &prepared, start)?;
                (prepared, after)
            } else if use_markdown_typing {
                let (prepared, after, _, exit) = document.prepare_markdown_typing_batch(edit,
                    self.typing_style.named.as_ref(), &self.typing_style.values,
                    self.typing_style.inherited.as_ref(), self.input_assistance.literal, self.markdown_typing_exit(document),
                ).map_err(command_document_error)?;
                exit_source = exit;
                (prepared, after)
            } else if !self.typing_style.is_empty() {
                let prepare = if self.typing_style.link_disabled { Document::prepare_insertion_without_link }
                    else { Document::prepare_insertion_with_typing_context };
                let (prepared, after, _) = prepare(document, edit, self.typing_style.named.as_ref(),
                    &self.typing_style.values, self.typing_style.inherited.as_ref()).map_err(command_document_error)?;
                (prepared, after)
            } else {
                let prepared = document.prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                    document.id(), document.revision(), vec![edit.clone()],
                )).map_err(command_document_error)?;
                let after = prepared_payload_caret(document, &prepared, &edit)?;
                (prepared, after)
            };
            let record = document.recorded_source_replacement(&prepared, start, after,
                inserted.clone(), Some(suffix[..removed_len].to_owned())).map_err(command_document_error)?;
            let changed = !prepared.is_no_op();
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            self.cursor = after;
            if use_markdown_typing { self.note_markdown_typing_exit(document, exit_source)?; }
            if let Some(session) = self.insert_session.as_mut() {
                session.replace_journal.push(ReplaceJournalEntry {
                    autoindent: false, start, inserted: inserted.clone(), original: record.original.clone(), source_record: Some(record),
                });
                session.record_inserted(&RegisterValue::characterwise(inserted), Some(EditSessionStep::IndentedEnter));
            }
            self.capture_generated_indent(document, prefix);
            return Ok(CommandOutput { document_changed: changed, cursor_moved: true, ..CommandOutput::complete() });
        }
        let mut removed_whitespace = String::new();
        if self.indentation_options().autoindent && self.literal_indentation(document) {
            let snapshot = document.hard_line_snapshot();
            let end = line_end(&snapshot, self.cursor);
            let suffix = snapshot
                .slice_utf8(self.cursor..end)
                .expect("validated split suffix");
            let whitespace = leading_whitespace_len(&suffix);
            if whitespace > 0 {
                removed_whitespace = suffix[..whitespace].to_owned();
                let prepared = document.prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(), revision: document.revision(),
                    edits: vec![TextEdit::new(self.cursor..self.cursor + whitespace, "")],
                }).map_err(command_document_error)?;
                let floor = self.insert_session.as_ref().map(|session|
                    prepared_cursor(document, &prepared, session.unit_floor, Association::BeforeInsertion)
                ).transpose()?;
                let cursor = prepared_cursor(document, &prepared, self.cursor, Association::BeforeInsertion)?;
                document.commit_model_transaction(prepared).map_err(command_document_error)?;
                self.cursor = cursor;
                if let (Some(session), Some(floor)) = (self.insert_session.as_mut(), floor) {
                    session.unit_floor = floor;
                }
            }
        }
        let output = self.record_semantic_insert(
            document,
            &format!("\n{}", prefix.text),
            EditSessionStep::IndentedEnter,
        )?;
        if self.mode == Mode::Replace {
            if let Some(session) = self.insert_session.as_mut() {
                session.replace_journal.clear();
                session.replace_journal.push(ReplaceJournalEntry {
                    autoindent: false,
                    start,
                    inserted: "\n".into(),
                    original: Some(removed_whitespace),
                    source_record: None,
                });
                if !prefix.text.is_empty() {
                    session.replace_journal.push(ReplaceJournalEntry {
                        autoindent: leading_whitespace_len(&prefix.text) == prefix.text.len(),
                        start: start + 1,
                        inserted: prefix.text.clone(),
                        original: None,
                        source_record: None,
                    });
                }
            }
        }
        self.capture_generated_indent(document, prefix);
        Ok(output)
    }

    pub(super) fn insert_tab(
        &mut self,
        document: &mut Document,
    ) -> Result<CommandOutput, DocumentError> {
        if !self.literal_indentation(document) {
            return self.record_semantic_insert(document, "\t", EditSessionStep::Tab);
        }
        self.generated_indent = None;
        let options = self.indentation_options();
        let snapshot = document.hard_line_snapshot();
        let start = line_start(&snapshot, self.cursor);
        let before = snapshot
            .slice_utf8(start..self.cursor)
            .expect("validated current-line prefix");
        let column = options.columns(&before);
        let leading = leading_whitespace_len(&before) == before.len();
        let width = if leading && options.smarttab {
            options.shift_width()
        } else {
            options.soft_tab_width()
        };
        let width = if width == 0 {
            options.tabstop as usize
        } else {
            width
        };
        let next = column + width - column % width;
        let text = if !options.expandtab
            && options.soft_tab_width() == 0
            && !(leading && options.smarttab)
        {
            "\t".to_owned()
        } else {
            options.whitespace(column, next)
        };
        if !options.expandtab
            && self.mode == Mode::Insert
            && (options.soft_tab_width() != 0 || (leading && options.smarttab))
        {
            let retained = before.trim_end_matches(' ').len();
            let retained_column = options.columns(&before[..retained]);
            let replacement = options.whitespace(retained_column, next);
            if retained < before.len() && replacement.contains('\t') {
                document.replace(start + retained..self.cursor, &replacement)?;
                self.cursor = start + retained + replacement.len();
                self.finish_typing_caret(document)?;
                if let Some(session) = self.insert_session.as_mut() {
                    session.record_step(EditSessionStep::Tab);
                }
                return Ok(CommandOutput {
                    document_changed: true,
                    cursor_moved: true,
                    ..CommandOutput::complete()
                });
            }
        }
        self.record_semantic_insert(document, &text, EditSessionStep::Tab)
    }

    /// Backspace to a soft stop, splitting a hard tab into spaces when the
    /// target lies inside it. Non-whitespace never gets consumed.
    pub(super) fn soft_tab_backspace(
        &mut self,
        document: &mut Document,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if !self.literal_indentation(document) || self.mode != Mode::Insert {
            return Ok(None);
        }
        let options = self.indentation_options();
        let snapshot = document.hard_line_snapshot();
        let start = line_start(&snapshot, self.cursor);
        let before = snapshot
            .slice_utf8(start..self.cursor)
            .expect("validated current-line prefix");
        if before.is_empty() || !before.ends_with([' ', '\t']) {
            return Ok(None);
        }
        let leading = leading_whitespace_len(&before) == before.len();
        let width = if leading && options.smarttab {
            options.shift_width()
        } else {
            options.soft_tab_width()
        };
        if width == 0 {
            return Ok(None);
        }
        let column = options.columns(&before);
        let target = column.saturating_sub(1) / width * width;
        let trailing_start = before.trim_end_matches([' ', '\t']).len();
        let mut remove_start = trailing_start;
        let mut remove_column = options.columns(&before[..trailing_start]);
        for (index, grapheme) in before[trailing_start..].grapheme_indices(true) {
            let next = if grapheme == "\t" {
                remove_column + options.tabstop as usize - remove_column % options.tabstop as usize
            } else {
                remove_column + options.columns(grapheme)
            };
            if next > target {
                break;
            }
            remove_start = trailing_start + index + grapheme.len();
            remove_column = next;
        }
        if remove_start == before.len() {
            return Ok(None);
        }
        let replacement = " ".repeat(target.saturating_sub(remove_column));
        let removed = &before[remove_start..];
        self.commit_indent_edit(document,
            TextEdit::new(start + remove_start..self.cursor, replacement),
            if leading { start } else { start + remove_start },
        )?;
        self.finish_typing_caret(document)?;
        self.generated_indent = None;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_deleted(removed, EditSessionStep::Backspace);
        }
        Ok(Some(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        }))
    }

    pub(super) fn insert_shift(
        &mut self,
        document: &mut Document,
        outdent: bool,
    ) -> Result<CommandOutput, DocumentError> {
        if !self.literal_indentation(document) {
            return Ok(CommandOutput::complete());
        }
        self.invalidate_replace_restoration();
        let options = self.indentation_options();
        let snapshot = document.hard_line_snapshot();
        let line = snapshot
            .line_at_offset(self.cursor)
            .expect("validated cursor")
            .content_range();
        let text = snapshot
            .slice_utf8(line.clone())
            .expect("validated current line");
        let len = leading_whitespace_len(&text);
        let columns = options.columns(&text[..len]);
        let prefix = &text[..self.cursor.saturating_sub(line.start).min(text.len())];
        let special =
            outdent && self.cursor == line.start + len + 1 && prefix.ends_with(['0', '^']);
        if special {
            return self.reset_insert_indent(document, prefix.ends_with('^'));
        }
        let width = options.shift_width();
        let target = if outdent {
            columns.saturating_sub(1) / width * width
        } else {
            (columns / width + 1) * width
        };
        let replacement = options.whitespace(0, target);
        self.commit_indent_edit(document,
            TextEdit::new(line.start..line.start + len, replacement), line.start,
        )?;
        self.finish_typing_caret(document)?;
        self.generated_indent = None;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_step(EditSessionStep::ShiftIndent { outdent });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    pub(super) fn reset_insert_indent(
        &mut self,
        document: &mut Document,
        restore: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let options = self.indentation_options();
        let snapshot = document.hard_line_snapshot();
        let start = line_start(&snapshot, self.cursor);
        let before = snapshot
            .slice_utf8(start..self.cursor)
            .expect("validated indent reset");
        let len = leading_whitespace_len(&before);
        if restore {
            self.restored_indent = Some(options.columns(&before[..len]));
        }
        self.commit_indent_edit(document, TextEdit::new(start..self.cursor, ""), start)?;
        self.generated_indent = None;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_step(EditSessionStep::ResetIndent { restore });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }

    pub(super) fn close_generated_comment(
        &mut self,
        document: &mut Document,
        character: char,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        let Some(generated) = &self.generated_indent else {
            return Ok(None);
        };
        let Some(comment) = &generated.comment else { return Ok(None); };
        if !document.format().is_code()
            || comment.revision != document.revision()
            || comment.language.as_deref() != self.reflow_options.language()
            || comment.profile != self.reflow_options.comments()
        {
            self.generated_indent = None;
            return Ok(None);
        }
        let profile = self.reflow_options.profile();
        let Some(block) = profile.blocks.get(comment.block) else { return Ok(None); };
        if !block.automatic_end || !block.close.ends_with(character) { return Ok(None); }
        let Some(range) = generated.range(document).filter(|r| r.end == self.cursor) else {
            return Ok(None);
        };
        let replacement = comment.closing.clone();
        let generated_text = generated.text.clone();
        document.replace(range.clone(), &replacement)?;
        self.cursor = range.start + replacement.len();
        self.generated_indent = None;
        if let Some(session) = self.insert_session.as_mut() {
            session.record_edit(|program, inserted| {
                if let Some(program) = program { program.push(EditSessionStep::CloseComment(character)); }
                remove_inserted_suffix(inserted, &generated_text);
                inserted.append_inserted_payload(&RegisterValue::characterwise(replacement));
            });
        }
        Ok(Some(CommandOutput {
            document_changed: true,
            ..CommandOutput::complete()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};
    fn key(c: &mut CommandInterpreter, d: &mut Document, k: Key) {
        let result = c.handle(d, InputEvent::Key(k)).unwrap();
        assert!(
            !matches!(result.status, CommandStatus::Error(_)),
            "{k:?}: {result:?}"
        );
    }
    fn keys(c: &mut CommandInterpreter, d: &mut Document, input: &str) {
        for ch in input.chars() {
            key(c, d, Key::Char(ch));
        }
    }
    fn ex(c: &mut CommandInterpreter, d: &mut Document, input: &str) {
        keys(c, d, &format!(":{input}"));
        key(c, d, Key::Enter);
    }
    #[test]
    fn enter_copies_indent_and_esc_cleans_only_generated_blank() {
        let mut d = Document::new("  text");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "A");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "  text\n  \n");
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "  text\n\n");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "  text");
        keys(&mut c, &mut d, "o");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "  text\n\n  \n");
        key(&mut c, &mut d, Key::Tab);
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "  text\n\n    \n");
    }
    #[test]
    fn tabs_backspace_and_inherited_options() {
        let mut d = Document::new("");
        let mut c = CommandInterpreter::new();
        ex(&mut c, &mut d, "set ts=8 sw=3 sts=-1 noet");
        assert_eq!(c.indentation_options().shift_width(), 3);
        keys(&mut c, &mut d, "i");
        for _ in 0..3 {
            key(&mut c, &mut d, Key::Tab);
        }
        assert_eq!(d.text(), "\t ");
        key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "      ");
        key(&mut c, &mut d, Key::Escape);
        ex(&mut c, &mut d, "set sw< sts< et<");
        c.set_indentation_defaults(IndentationOptions {
            shiftwidth: 4,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.indentation_options().tabstop, 8);
        assert_eq!(c.indentation_options().shift_width(), 4);
    }
    #[test]
    fn hard_tab_backspace_preserves_partial_tab_width() {
        let mut d = Document::new("\t");
        let mut c = CommandInterpreter::new();
        c.set_indentation_defaults(IndentationOptions {
            tabstop: 8,
            ..Default::default()
        })
        .unwrap();
        keys(&mut c, &mut d, "A");
        key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "      ");
        key(&mut c, &mut d, Key::Escape);
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "\t");
    }
    #[test]
    fn comments_share_profiles_and_preserve_leader_and_star_convention() {
        for (source, expected) in [
            ("  // text", "  // text\n  // "),
            ("/// docs", "/// docs\n/// "),
            ("//! docs", "//! docs\n//! "),
            ("/* text", "/* text\n * "),
            ("/* text */", "/* text */\n"),
            ("* pointer", "* pointer\n"),
        ] {
            let mut d =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code)
                    .unwrap();
            let mut c = CommandInterpreter::new();
            c.reflow_options.format_options = Some("qjro".into());
            c.set_reflow_language(Some("cpp".into()));
            keys(&mut c, &mut d, "A");
            key(&mut c, &mut d, Key::Enter);
            assert_eq!(d.text(), format!("{expected}\n"));
        }
        let mut d = Document::from_bytes(b"/*".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut c = CommandInterpreter::new();
        c.reflow_options.format_options = Some("qjro".into());
        c.set_reflow_language(Some("c".into()));
        keys(&mut c, &mut d, "A");
        key(&mut c, &mut d, Key::Enter);
        keys(&mut c, &mut d, "/");
        assert_eq!(d.text(), "/*\n */\n");
        key(&mut c, &mut d, Key::Escape);
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "/*");
    }
    #[test]
    fn counted_open_and_dot_recompute_context_and_group_undo() {
        let mut d = Document::new("  a\n    b");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "2ox");
        key(&mut c, &mut d, Key::Escape);
        assert_eq!(d.text(), "  a\n  x\n  x\n    b");
        keys(&mut c, &mut d, "G.");
        assert_eq!(d.text(), "  a\n  x\n  x\n    b\n    x\n    x\n");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "  a\n  x\n  x\n    b");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "  a\n    b");
    }

    #[test]
    fn shared_profile_refresh_retires_a_generated_closer_even_after_reset() {
        let mut d = Document::from_bytes(b"/*".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut c = CommandInterpreter::new();
        c.set_reflow_language(Some("cpp".into()));
        c.reflow_options.format_options = Some("qjr".into());
        keys(&mut c, &mut d, "A");
        key(&mut c, &mut d, Key::Enter);
        let initial = c.export_buffer_state();
        let mut changed = initial.clone();
        changed.reflow_options.comments = Some("s1:/*,mb:*,ex:*/".into());
        c.install_buffer_state(&changed);
        c.install_buffer_state(&initial);
        keys(&mut c, &mut d, "/");
        assert_eq!(d.text(), "/*\n * /\n");
    }
    #[test]
    fn shifts_tabs_registers_and_visual_counts() {
        let mut d = Document::new("\ta\n b");
        let mut c = CommandInterpreter::new();
        ex(&mut c, &mut d, "set ts=8 sw=2 noet");
        keys(&mut c, &mut d, "<<");
        assert_eq!(d.text(), "      a\n b");
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "\ta\n b");
        keys(&mut c, &mut d, "ggVj2>");
        assert_eq!(d.text(), "\t    a\n     b");
        assert!(c.register('"').is_none_or(|value| value.text.is_empty()));
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "\ta\n b");
    }
    #[test]
    fn insert_control_shifts_zero_and_caret_forms_repeat_and_undo() {
        let mut d = Document::new("   text");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "I");
        key(&mut c, &mut d, Key::Ctrl('t'));
        assert_eq!(d.text(), "    text");
        key(&mut c, &mut d, Key::Ctrl('d'));
        assert_eq!(d.text(), "  text");
        keys(&mut c, &mut d, "^");
        key(&mut c, &mut d, Key::Ctrl('d'));
        assert_eq!(d.text(), "text");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "\n  text");
        key(&mut c, &mut d, Key::Escape);
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "   text");
        keys(&mut c, &mut d, "I0");
        key(&mut c, &mut d, Key::Ctrl('d'));
        assert_eq!(d.text(), "text");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "\ntext");
    }
    #[test]
    fn paste_preserves_literal_whitespace_and_never_continues_comments() {
        let mut d = Document::from_bytes(b"// old".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        let mut c = CommandInterpreter::new();
        c.reflow_options.format_options = Some("qjro".into());
        c.set_reflow_language(Some("cpp".into()));
        keys(&mut c, &mut d, "A");
        c.handle(&mut d, InputEvent::text("\n\tnew\n  tail"))
            .unwrap();
        assert_eq!(d.text(), "// old\n\tnew\n  tail");
        key(&mut c, &mut d, Key::Escape);
        keys(&mut c, &mut d, "u");
        assert_eq!(d.text(), "// old");
    }
    #[test]
    fn replace_tab_and_enter_backspace_restore_the_whole_semantic_key() {
        let mut d = Document::new("  abc");
        let mut c = CommandInterpreter::new();
        c.set_cursor(&d, 2);
        keys(&mut c, &mut d, "R");
        key(&mut c, &mut d, Key::Tab);
        assert_eq!(d.text(), "    c");
        key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "  abc");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "  \n  abc");
        key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "  abc");
        assert_eq!(c.cursor(), 2);
        key(&mut c, &mut d, Key::Escape);
        let mut d = Document::new("  one  two");
        let mut c = CommandInterpreter::new();
        c.set_cursor(&d, 5);
        keys(&mut c, &mut d, "R");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "  one\n  two");
        key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "  one  two");
        assert_eq!(c.cursor(), 5);
        let mut d = Document::new("\tabc"); let mut c = CommandInterpreter::new();
        c.set_indentation_defaults(IndentationOptions { tabstop: 8, expandtab: false, ..Default::default() }).unwrap();
        c.set_cursor(&d, 1); keys(&mut c, &mut d, "R");
        key(&mut c, &mut d, Key::Enter); key(&mut c, &mut d, Key::Backspace);
        assert_eq!(d.text(), "\tabc");
        assert_eq!(c.cursor(), 1);
    }

    #[test]
    fn continuation_uses_the_lexical_owner_of_midline_openers() {
        for (source, prefix) in [
            ("const char *s = \"/*\";", ""),
            ("code(); /* text", "         * "),
            ("// /* text", "// "),
        ] {
            let mut d =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code)
                    .unwrap();
            let mut c = CommandInterpreter::new();
            c.reflow_options.format_options = Some("qjro".into());
            c.set_reflow_language(Some("cpp".into()));
            keys(&mut c, &mut d, "A");
            key(&mut c, &mut d, Key::Enter);
            let expected = format!("{source}\n{prefix}\n");
            assert_eq!(d.text(), expected);
        }
        let mut d = Document::from_bytes(
            b"const char *s = \"/*\";\n * ordinary".to_vec(),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap();
        let mut c = CommandInterpreter::new();
        c.reflow_options.format_options = Some("qjro".into());
        c.set_reflow_language(Some("cpp".into()));
        keys(&mut c, &mut d, "GA");
        key(&mut c, &mut d, Key::Enter);
        assert_eq!(d.text(), "const char *s = \"/*\";\n * ordinary\n \n");
    }

    #[test]
    fn splitting_a_line_consumes_suffix_indentation_and_repeats_semantically() {
        for (at, expected) in [
            (0, "\none  two"),
            (1, " \n one  two"),
            (2, "  \n  one  two"),
            (5, "  one\n  two"),
        ] {
            let mut d = Document::new("  one  two");
            let mut c = CommandInterpreter::new();
            c.set_cursor(&d, at);
            keys(&mut c, &mut d, "i");
            key(&mut c, &mut d, Key::Enter);
            key(&mut c, &mut d, Key::Escape);
            assert_eq!(d.text(), expected);
            keys(&mut c, &mut d, "u");
            assert_eq!(d.text(), "  one  two");
        }
    }

    #[test]
    fn invalid_ex_is_atomic_and_buffer_options_survive_view_state_install() {
        let mut d = Document::new("unchanged");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, ":set ts=8 sw=-1");
        let output = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
        assert_ne!(output.status, CommandStatus::Complete);
        assert_eq!(c.indentation_options(), IndentationOptions::default());
        ex(&mut c, &mut d, "set ts=8 noai");
        let mut other = CommandInterpreter::new();
        other.install_buffer_state(&c.export_buffer_state());
        assert_eq!(other.indentation_options().tabstop, 8);
        assert!(!other.indentation_options().autoindent);
        assert_eq!(d.text(), "unchanged");
    }

    #[test]
    fn million_lines_indent_and_comment_enter_do_not_flatten() {
        let mut bytes = b"  // prose\n".repeat(1_000_000);
        bytes.extend_from_slice(b"tail");
        let mut d = Document::from_bytes(bytes, Encoding::Utf8, Format::Code).unwrap();
        d.replace(0..0, " ").unwrap();
        let mut c = CommandInterpreter::new();
        c.reflow_options.format_options = Some("qjro".into());
        c.set_reflow_language(Some("cpp".into()));
        c.set_cursor(&d, 5_500_006);
        for event in [
            InputEvent::key('A'),
            InputEvent::Key(Key::Enter),
            InputEvent::text("word"),
            InputEvent::Key(Key::Escape),
            InputEvent::key('>'),
            InputEvent::key('>'),
            InputEvent::key('R'),
            InputEvent::Key(Key::Tab),
            InputEvent::Key(Key::Backspace),
            InputEvent::Key(Key::Escape),
        ] {
            let previous = d.projection().clone();
            c.handle(&mut d, event.clone()).unwrap();
            assert!(
                !previous.compatibility_text_is_materialized(),
                "{event:?} flattened previous"
            );
            assert!(!d.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn replace_enter_at_unterminated_eof_restores_its_supporting_terminator() {
        for format in [Format::PlainText, Format::Code] {
            for source in ["body", ""] {
                for enter in [Key::Enter, Key::ShiftEnter] {
                    let mut d = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                    let mut c = CommandInterpreter::new();
                    if !source.is_empty() { c.set_cursor(&d, source.len() - 1); }
                    key(&mut c, &mut d, Key::Char('R'));
                    // Replacing the last character reaches the insertion
                    // boundary; set_cursor deliberately uses Normal stops.
                    if let Some(last) = source.chars().last() { key(&mut c, &mut d, Key::Char(last)); }
                    assert_eq!(c.cursor(), source.len());
                    key(&mut c, &mut d, enter);
                    assert_eq!(d.source_bytes(), format!("{source}\n\n").as_bytes());
                    assert_eq!(c.cursor(), source.len() + 1);
                    key(&mut c, &mut d, Key::Backspace);
                    assert_eq!(d.source_bytes(), source.as_bytes());
                    assert_eq!(c.cursor(), source.len());
                    key(&mut c, &mut d, enter);
                    key(&mut c, &mut d, Key::Char('x'));
                    key(&mut c, &mut d, Key::Escape);
                    assert_eq!(d.source_bytes(), format!("{source}\nx\n").as_bytes());
                    key(&mut c, &mut d, Key::Char('u'));
                    assert_eq!(d.source_bytes(), source.as_bytes());
                }
            }
        }
    }
}
