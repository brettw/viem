//! View-local meaning of unprefixed line commands. Explicit screen/g motions
//! remain layout operations; physical source lines belong to the text pipeline.
use super::*;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum LineMode {
    #[default]
    Visual = 0,
    PhysicalSource = 1,
}
/// Cached exact source position for source-only lines. It is reused only in
/// the named source snapshot and while its accompanying text point still
/// equals the current cursor. An edit invalidates it; the current persistent
/// text cursor then supplies the declared visible-boundary recovery policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PhysicalCursor {
    source: crate::document::SourcePoint,
    text: crate::document::TextPoint,
}
impl CommandInterpreter {
    pub(super) fn move_to_document_edge(
        &mut self,
        document: &Document,
        end: bool,
    ) -> CommandOutput {
        let origin = self.cursor;
        let at = if end {
            document.projection().text_tree().byte_len()
        } else {
            0
        };
        self.cursor = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            at
        } else {
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), at)
        };
        self.clear_pending();
        self.typing_style = Default::default();
        self.input_assistance.clear_tag();
        self.preferred_column = None;
        self.desired_x = None;
        self.visual_position = None;
        self.visual_to_line_end = false;
        self.physical_cursor = None;
        if self.line_mode == LineMode::PhysicalSource {
            self.remember_physical_cursor(
                document,
                if end { document.source_byte_len() } else { 0 },
            );
        }
        let output = CommandOutput {
            cursor_moved: origin != self.cursor,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, origin, output)
    }

    fn physical_source_cursor(&self, document: &Document) -> Result<usize, DocumentError> {
        if let Some(cached) = self.physical_cursor.filter(|cached| {
            cached.source.document() == document.id()
                && cached.source.revision() == document.revision()
                && cached.text.offset() == self.cursor
        }) {
            return Ok(cached.source.offset());
        }
        document.text_point(self.cursor)?;
        Ok(document
            .projection()
            .source_insertion_point(self.cursor, true)
            .unwrap_or(0))
    }
    fn physical_line(
        &self,
        document: &Document,
    ) -> Result<crate::document::PhysicalSourceLine, DocumentError> {
        document.physical_line_at_source(self.physical_source_cursor(document)?)
    }
    fn remember_physical_cursor(&mut self, document: &Document, at: usize) {
        self.physical_cursor = document
            .source_point(at)
            .ok()
            .zip(document.text_point(self.cursor).ok())
            .map(|(source, text)| PhysicalCursor { source, text });
    }
    pub fn line_mode(&self) -> LineMode {
        self.line_mode
    }
    pub fn set_line_mode(
        &mut self,
        document: &Document,
        mode: LineMode,
    ) -> Result<(), DocumentError> {
        if mode == LineMode::PhysicalSource && document.format() == crate::document::Format::Rtf {
            return Err(DocumentError::UnsupportedFormatting);
        }
        self.line_mode = mode;
        self.physical_cursor = None;
        self.clear_pending();
        self.preferred_column = None;
        self.desired_x = None;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LineShape {
    Whole,
    End,
    Start,
    StartNonblank,
    Down,
    Up,
}
struct LineExtent {
    text: Range<usize>,
    source: Option<Range<usize>>,
    whole: bool,
}
impl CommandInterpreter {
    pub(super) fn mode_line_key(&self, key: Key) -> bool {
        let key = self.resolved_clipboard_copy_key(key);
        if self.register_pending
            || self.count_overflowed
            || !matches!(
                self.mode,
                Mode::Normal | Mode::VisualCharacter | Mode::VisualLine
            )
        {
            return false;
        }
        if self.mode != Mode::Normal
            && matches!(
                key,
                Key::Char('C' | 'D' | 'Y' | 'S' | 'A' | 'I' | 'o' | 'O' | '.')
            )
        {
            return false;
        }
        match self.pending {
            Pending::Operator(pending) => {
                !pending.g_prefix
                    && (key == Key::Char(pending.operator.doubled_key())
                        || matches!(
                            key,
                            Key::Char('$' | '0' | '^' | 'j' | 'k' | '+' | '-' | '_')
                                | Key::Home
                                | Key::End
                                | Key::Up
                                | Key::Down
                        ))
            }
            Pending::None => match key {
                Key::Char('0') => self.count.is_none(),
                Key::Char(
                    '$' | '^' | 'C' | 'D' | 'Y' | 'S' | 'A' | 'I' | 'V' | 'o' | 'O' | 'J' | 'j'
                    | 'k' | '+' | '-' | '_' | '|',
                )
                | Key::Home
                | Key::End
                | Key::Up
                | Key::Down
                | Key::Enter => true,
                Key::Char('.') => matches!(
                    &self.last_repeat,
                    Some(
                        RepeatAction::Operator {
                            command: OperatorRepeat {
                                target: RepeatTarget::ViewLine(..),
                                ..
                            },
                            ..
                        } | RepeatAction::Join { .. }
                            | RepeatAction::Insert {
                                placement: InsertPlacement::LineStart
                                    | InsertPlacement::LineEnd
                                    | InsertPlacement::OpenAbove
                                    | InsertPlacement::OpenBelow,
                                ..
                            }
                    )
                ),
                _ => false,
            },
            Pending::G { .. } => key == Key::Char('J'),
            _ => false,
        }
    }
    fn mode_extent(
        &self,
        document: &Document,
        shape: LineShape,
        count: usize,
    ) -> Result<Result<LineExtent, LayoutMotionError>, DocumentError> {
        let count = count.max(1);
        if self.line_mode == LineMode::PhysicalSource {
            let current = self.physical_line(document)?;
            let index = if shape == LineShape::Up {
                current.index.saturating_sub(count)
            } else {
                current.index
            };
            let end = if shape == LineShape::Up {
                current.index
            } else {
                current
                    .index
                    .saturating_add(if shape == LineShape::Down {
                        count
                    } else {
                        count - 1
                    })
                    .min(document.physical_line_count()? - 1)
            };
            let first = document.physical_line(index)?;
            let last = document.physical_line(end)?;
            let at = self
                .physical_source_cursor(document)?
                .clamp(current.content_range.start, current.content_range.end);
            let source = match shape {
                LineShape::End => at..last.content_range.end,
                LineShape::Start => current.content_range.start..at,
                LineShape::StartNonblank => {
                    let blank =
                        current.text.len() - current.text.trim_start_matches([' ', '\t']).len();
                    let first = current.content_range.start
                        + document
                            .encoding()
                            .encode_fragment(&current.text[..blank])?
                            .len();
                    first.min(at)..first.max(at)
                }
                _ => first.source_range.start..last.source_range.end,
            };
            let a = document.visible_point_for_source(source.start, true)?;
            let b = document.visible_point_for_source(source.end, false)?;
            return Ok(Ok(LineExtent {
                text: a.min(b)..a.max(b),
                source: Some(source),
                whole: !matches!(
                    shape,
                    LineShape::End | LineShape::Start | LineShape::StartNonblank
                ),
            }));
        }
        let Some(snapshot) = self
            .line_layout
            .as_ref()
            .filter(|snapshot| snapshot.document_revision == document.revision())
        else {
            return Ok(Err(LayoutMotionError::EmptyLayout));
        };
        let current = match self.current_visual_position(snapshot) {
            Ok(current) => current,
            Err(error) => return Ok(Err(error)),
        };
        let delta = match shape {
            LineShape::Up => -(count.min(isize::MAX as usize) as isize),
            LineShape::Down => count.min(isize::MAX as usize) as isize,
            LineShape::Start | LineShape::StartNonblank => 0,
            _ => (count - 1).min(isize::MAX as usize) as isize,
        };
        let rows = match layout_motion::command_row_span(snapshot, current, delta) {
            Ok(rows) => rows,
            Err(error) => return Ok(Err(error)),
        };
        let first = &snapshot.rows[rows.start];
        let last = &snapshot.rows[rows.end - 1];
        let start = if shape == LineShape::End {
            self.cursor
        } else if shape == LineShape::StartNonblank {
            match g_caret(snapshot, document.text(), current) {
                Ok(position) => position.text_offset,
                Err(error) => return Ok(Err(error)),
            }
        } else {
            first.text_range.start
        };
        let mut end = if matches!(shape, LineShape::Start | LineShape::StartNonblank) {
            self.cursor
        } else if shape == LineShape::End {
            let target = VisualPosition {
                text_offset: last.text_range.start,
                affinity: BoundaryAffinity::Downstream,
            };
            match g_dollar_for_document(document, snapshot, target) {
                Ok(position) => position.text_offset,
                Err(error) => return Ok(Err(error)),
            }
        } else {
            last.text_range.end
        };
        let whole = !matches!(
            shape,
            LineShape::End | LineShape::Start | LineShape::StartNonblank
        );
        if whole
            && !last.wraps_to_next
            && end < document.text().len()
            && document.text().as_bytes()[end] == b'\n'
        {
            end += 1;
        }
        Ok(Ok(LineExtent {
            text: start.min(end)..start.max(end),
            source: None,
            whole,
        }))
    }
    pub(super) fn try_mode_line_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if !self.mode_line_key(key)
            || (self.line_mode == LineMode::Visual && self.line_layout.is_none())
        {
            return Ok(None);
        }
        if key == Key::Char('V') || matches!(self.pending, Pending::G { .. }) {
            return Ok(None);
        }
        if key == Key::Char('.') {
            let count = self.count.take();
            return self.repeat_last_change(document, count).map(Some);
        }
        if let Pending::Operator(pending) = self.pending {
            let count = match effective_operator_count(pending) {
                Ok(count) => count,
                Err(error) => return Ok(Some(CommandOutput::count_error(error))),
            };
            let shape = if key == Key::Char(pending.operator.doubled_key()) {
                LineShape::Whole
            } else {
                match key {
                    Key::Char('$') | Key::End => LineShape::End,
                    Key::Char('0') | Key::Home => LineShape::Start,
                    Key::Char('^') => LineShape::StartNonblank,
                    Key::Char('k' | '-') | Key::Up => LineShape::Up,
                    Key::Char('_') => LineShape::Whole,
                    _ => LineShape::Down,
                }
            };
            self.pending = Pending::None;
            return self
                .apply_mode_line_operator(
                    document,
                    pending.operator,
                    shape,
                    count,
                    pending.register,
                )
                .map(Some);
        }
        let count = self.count.take().unwrap_or(1).max(1);
        if self.mode == Mode::Normal {
            if matches!(key, Key::Char('A' | 'I')) {
                return Ok(Some(self.enter_insert(
                    document,
                    if key == Key::Char('A') {
                        InsertPlacement::LineEnd
                    } else {
                        InsertPlacement::LineStart
                    },
                    count,
                )));
            }
            if matches!(key, Key::Char('o' | 'O')) {
                return self
                    .open_line(document, key == Key::Char('O'), count)
                    .map(Some);
            }
            if key == Key::Char('J') {
                return self.join_lines(document, count, true).map(Some);
            }
            let action = match key {
                Key::Char('D') => Some((Operator::Delete, LineShape::End)),
                Key::Char('C') => Some((Operator::Change, LineShape::End)),
                Key::Char('Y') => Some((Operator::Yank, LineShape::Whole)),
                Key::Char('S') => Some((Operator::Change, LineShape::Whole)),
                _ => None,
            };
            if let Some((operator, shape)) = action {
                let register = self.requested_register.take();
                return self
                    .apply_mode_line_operator(document, operator, shape, count, register)
                    .map(Some);
            }
        }
        if self.line_mode == LineMode::Visual {
            let snapshot = self.line_layout.as_ref().unwrap().clone();
            let current = match self.current_visual_position(&snapshot) {
                Ok(current) => current,
                Err(error) => return Ok(Some(layout_error(error))),
            };
            let mut desired_x = None;
            let moved = match key {
                Key::Char('j') | Key::Down => {
                    gj(&snapshot, current, count, self.desired_x).map(|motion| {
                        desired_x = Some(motion.desired_x);
                        motion.position
                    })
                }
                Key::Char('k') | Key::Up => {
                    gk(&snapshot, current, count, self.desired_x).map(|motion| {
                        desired_x = Some(motion.desired_x);
                        motion.position
                    })
                }
                Key::Char('+') | Key::Enter => gj(&snapshot, current, count, None)
                    .and_then(|motion| g_caret(&snapshot, document.text(), motion.position)),
                Key::Char('-') => gk(&snapshot, current, count, None)
                    .and_then(|motion| g_caret(&snapshot, document.text(), motion.position)),
                Key::Char('0') | Key::Home => g0(&snapshot, current),
                Key::Char('^') => g_caret(&snapshot, document.text(), current),
                Key::Char('$') | Key::End => {
                    if count == 1 {
                        g_dollar_for_document(document, &snapshot, current)
                    } else {
                        gj(&snapshot, current, count - 1, None).and_then(|motion| {
                            g_dollar_for_document(document, &snapshot, motion.position)
                        })
                    }
                }
                Key::Char('_') => {
                    if count == 1 {
                        g_caret(&snapshot, document.text(), current)
                    } else {
                        gj(&snapshot, current, count - 1, None)
                            .and_then(|motion| g_caret(&snapshot, document.text(), motion.position))
                    }
                }
                Key::Char('|') => {
                    layout_motion::command_row_span(&snapshot, current, 0).map(|rows| {
                        let row = &snapshot.rows[rows.start];
                        let offset = document.text()[row.text_range.clone()]
                            .grapheme_indices(true)
                            .nth(count - 1)
                            .map_or(row.text_range.end, |(offset, _)| {
                                row.text_range.start + offset
                            });
                        VisualPosition {
                            text_offset: offset,
                            affinity: if offset == row.text_range.end {
                                BoundaryAffinity::Upstream
                            } else {
                                BoundaryAffinity::Downstream
                            },
                        }
                    })
                }
                _ => return Ok(None),
            };
            let position = match moved {
                Ok(position) => position,
                Err(error) => return Ok(Some(layout_error(error))),
            };
            let output = self.install_visual_position(document, &snapshot, position);
            self.desired_x = desired_x;
            return Ok(Some(output));
        }
        let line = self.physical_line(document)?;
        let target_index = match key {
            Key::Char('j' | '+') | Key::Down | Key::Enter => line
                .index
                .saturating_add(count)
                .min(document.physical_line_count()? - 1),
            Key::Char('k' | '-') | Key::Up => line.index.saturating_sub(count),
            Key::Char('$' | '_') | Key::End => line
                .index
                .saturating_add(count - 1)
                .min(document.physical_line_count()? - 1),
            _ => line.index,
        };
        let vertical = matches!(key, Key::Char('j' | 'k') | Key::Up | Key::Down);
        let column = if vertical {
            let at = self
                .physical_source_cursor(document)?
                .clamp(line.content_range.start, line.content_range.end);
            let column = self.preferred_column.unwrap_or(
                document
                    .physical_source_text(line.content_range.start..at)?
                    .graphemes(true)
                    .count(),
            );
            self.preferred_column = Some(column);
            column
        } else {
            self.preferred_column = None;
            0
        };
        let target = document.physical_line(target_index)?;
        let source = match key {
            Key::Char('$') | Key::End => target.content_range.end,
            Key::Char('^' | '+' | '-' | '_') | Key::Enter => {
                let blank = target.text.len() - target.text.trim_start_matches([' ', '\t']).len();
                target.content_range.start
                    + document
                        .encoding()
                        .encode_fragment(&target.text[..blank])?
                        .len()
            }
            Key::Char('j' | 'k') | Key::Up | Key::Down => {
                let prefix = target
                    .text
                    .trim_end_matches('\n')
                    .graphemes(true)
                    .take(column)
                    .collect::<String>();
                target.content_range.start + document.encoding().encode_fragment(&prefix)?.len()
            }
            Key::Char('|') => {
                let text = target.text.trim_end_matches('\n');
                let prefix = text.graphemes(true).take(count - 1).collect::<String>();
                target.content_range.start + document.encoding().encode_fragment(&prefix)?.len()
            }
            _ => target.content_range.start,
        };
        let point =
            document.visible_point_for_source(source, !matches!(key, Key::Char('$') | Key::End))?;
        let before = self.cursor;
        self.typing_style = Default::default();
        self.cursor =
            normalize_normal_cursor(document.text(), &document.hard_line_snapshot(), point);
        self.boundary_affinity = if matches!(key, Key::Char('$') | Key::End) {
            BoundaryAffinity::Upstream
        } else {
            BoundaryAffinity::Downstream
        };
        self.visual_position = None;
        self.remember_physical_cursor(document, source);
        Ok(Some(CommandOutput {
            cursor_moved: self.cursor != before,
            ..CommandOutput::complete()
        }))
    }
    pub(super) fn apply_mode_line_operator(
        &mut self,
        document: &mut Document,
        operator: Operator,
        shape: LineShape,
        count: usize,
        register: Option<char>,
    ) -> Result<CommandOutput, DocumentError> {
        self.apply_mode_line_operator_repeated(document, operator, shape, count, register, 1)
    }
    pub(super) fn apply_mode_line_operator_repeated(
        &mut self,
        document: &mut Document,
        operator: Operator,
        shape: LineShape,
        count: usize,
        register: Option<char>,
        applications: usize,
    ) -> Result<CommandOutput, DocumentError> {
        let extent = match self.mode_extent(document, shape, count)? {
            Ok(extent) => extent,
            Err(error) => return Ok(layout_error(error)),
        };
        self.apply_mode_line_extent(
            document,
            operator,
            shape,
            count,
            register,
            extent,
            applications,
        )
    }
    fn apply_mode_line_extent(
        &mut self,
        document: &mut Document,
        operator: Operator,
        shape: LineShape,
        count: usize,
        register: Option<char>,
        extent: LineExtent,
        applications: usize,
    ) -> Result<CommandOutput, DocumentError> {
        if matches!(
            operator,
            Operator::Yank | Operator::Delete | Operator::Change
        ) {
            if let Err(output) = self.require_register_write(register) {
                return Ok(output);
            }
        }
        if matches!(
            operator,
            Operator::Indent | Operator::Outdent | Operator::Reindent
        ) && applications > 250_000
        {
            return Ok(repetition_too_large(applications));
        }
        if operator == Operator::Yank && register == Some('*') && self.clipboard_copy_as_seen {
            let value = if let Some(source) =
                extent.source.as_ref().filter(|_| !document.format().is_wysiwyg())
            {
                let bytes = document.source_bytes();
                RegisterValue::characterwise(document.encoding().decode(&bytes[source.clone()])?.text)
            } else {
                RegisterValue::from_clipboard_fragment(document.clipboard_fragment(extent.text.clone())?.as_seen())
                    .map_err(|_| DocumentError::UnsupportedFormatting)?
            };
            self.yank_register(register, value);
            return Ok(CommandOutput::complete());
        }
        let before = document.revision();
        let mut output = if let Some(mut source) = extent.source {
            let source_len = document.source_byte_len();
            let normalized = document.physical_source_text(source.clone())?;
            let value = if extent.whole {
                RegisterValue::linewise(if normalized.ends_with('\n') {
                    normalized.clone()
                } else {
                    format!("{normalized}\n")
                })
            } else {
                RegisterValue::characterwise(normalized.clone())
            };
            if operator == Operator::Yank {
                self.yank_register(register, value);
                return Ok(CommandOutput::complete());
            }
            let replacement = match operator {
                Operator::Delete | Operator::Change => {
                    if operator == Operator::Change && extent.whole && source.end < source_len {
                        "\n".to_owned()
                    } else {
                        String::new()
                    }
                }
                Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                    change_case(&normalized, operator)
                }
                Operator::Indent | Operator::Outdent | Operator::Reindent => normalized
                    .split_inclusive('\n')
                    .map(|line| match operator {
                        Operator::Indent => {
                            format!("{}{line}", " ".repeat(4usize.saturating_mul(applications)))
                        }
                        Operator::Outdent => line[line
                            .bytes()
                            .take_while(|byte| *byte == b' ')
                            .take(4usize.saturating_mul(applications))
                            .count()..]
                            .to_owned(),
                        Operator::Reindent => line.trim_start_matches([' ', '\t']).to_owned(),
                        _ => unreachable!(),
                    })
                    .collect::<String>(),
                Operator::Yank => unreachable!(),
            };
            if operator == Operator::Delete
                && extent.whole
                && source.end == source_len
                && source.start > 0
            {
                let index = document.physical_line_at_source(source.start)?.index;
                if index > 0 {
                    source.start = document.physical_line(index - 1)?.content_range.end;
                }
            }
            if operator == Operator::Change {
                document.begin_edit_group();
            }
            document.replace_physical_source(source.clone(), replacement)?;
            if matches!(operator, Operator::Delete | Operator::Change) {
                self.delete_register(
                    register,
                    value,
                    if extent.whole {
                        DeletionClass::Large
                    } else {
                        DeletionClass::Small
                    },
                );
            }
            self.cursor = document
                .visible_point_for_source(source.start.min(document.source_byte_len()), true)?;
            self.remember_physical_cursor(document, source.start.min(document.source_byte_len()));
            CommandOutput {
                document_changed: before != document.revision(),
                cursor_moved: true,
                ..CommandOutput::complete()
            }
        } else {
            // A wrapped row contains text, not an implicit paragraph separator.
            // Complete logical paragraphs retain the existing structural path.
            let mut range = extent.text.clone();
            let lines = document.hard_line_snapshot();
            let whole_hard_lines = lines
                .line_at_offset(range.start)
                .is_ok_and(|line| line.content_range().start == range.start)
                && lines
                    .line_at_offset(range.end.min(document.text().len()))
                    .is_ok_and(|line| {
                        line.content_range().start == range.end
                            || range.end == document.text().len()
                    });
            let value = if extent.whole && whole_hard_lines {
                register_value(
                    document,
                    &lines,
                    &MotionExtent {
                        range: extent.text.clone(),
                        kind: MotionKind::Linewise,
                    },
                    register,
                )
            } else {
                register_value(
                    document,
                    &lines,
                    &MotionExtent {
                        range: extent.text.clone(),
                        kind: MotionKind::Characterwise,
                    },
                    register,
                )
            };
            if operator == Operator::Yank {
                self.yank_register(register, value);
                return Ok(CommandOutput::complete());
            }
            if extent.whole && whole_hard_lines && operator == Operator::Delete {
                range = linewise_edit_range(&lines, range);
            }
            if operator == Operator::Change
                && extent.whole
                && whole_hard_lines
                && document.text().as_bytes().get(range.end.saturating_sub(1)) == Some(&b'\n')
            {
                range.end -= 1;
            }
            let replacement = match operator {
                Operator::Delete | Operator::Change => String::new(),
                Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                    change_case(&document.text()[range.clone()], operator)
                }
                _ => String::new(),
            };
            if operator == Operator::Change {
                document.begin_edit_group();
            }
            if matches!(
                operator,
                Operator::Indent | Operator::Outdent | Operator::Reindent
            ) {
                let edits = self.visual_indent_edits(document, &range, operator, applications)?;
                document.apply_edits(edits)?;
            } else if operator == Operator::Delete && extent.whole {
                document.delete_visual_text(range.clone())?;
            } else {
                document.replace(range.clone(), &replacement)?;
            }
            if matches!(operator, Operator::Delete | Operator::Change) {
                self.delete_register(
                    register,
                    value,
                    if extent.whole {
                        DeletionClass::Large
                    } else {
                        DeletionClass::Small
                    },
                );
            }
            self.cursor = range.start.min(document.text().len());
            CommandOutput {
                document_changed: before != document.revision(),
                cursor_moved: true,
                ..CommandOutput::complete()
            }
        };
        if operator == Operator::Change {
            output.merge(self.enter_insert(document, InsertPlacement::Before, 1));
            // The change itself already opened the shared undo group.
            document.end_edit_group();
        } else {
            self.cursor = normalize_normal_cursor(
                document.text(),
                &document.hard_line_snapshot(),
                self.cursor,
            );
        }
        if !self.replaying && operator != Operator::Yank {
            let command = OperatorRepeat {
                operator,
                target: RepeatTarget::ViewLine(shape, applications),
                count,
                register,
            };
            if operator == Operator::Change {
                if let Some(session) = self.insert_session.as_mut() {
                    session.repeat_operator = Some(command);
                }
            } else {
                self.last_repeat = Some(RepeatAction::Operator {
                    command,
                    edits: None,
                });
            }
        }
        self.visual_position = None;
        self.desired_x = None;
        Ok(output)
    }
}

/// One-based location without forcing layout of preceding document content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineLocation {
    pub mode: LineMode,
    pub line: Option<usize>,
    pub column: usize,
    pub hard_line: usize,
    pub fragment: usize,
}
impl CommandInterpreter {
    pub fn line_location(
        &self,
        document: &Document,
        snapshot: Option<&LayoutSnapshot>,
    ) -> Result<LineLocation, crate::coordinator::CoreError> {
        if self.line_mode == LineMode::PhysicalSource {
            let line = self.physical_line(document)?;
            let at = self
                .physical_source_cursor(document)?
                .clamp(line.content_range.start, line.content_range.end);
            let prefix = document.physical_source_text(line.content_range.start..at)?;
            return Ok(LineLocation {
                mode: self.line_mode,
                line: Some(line.index + 1),
                column: prefix.graphemes(true).count() + 1,
                hard_line: line.index + 1,
                fragment: 1,
            });
        }
        let snapshot = snapshot
            .filter(|snapshot| snapshot.document_revision == document.revision())
            .ok_or(crate::coordinator::CoreError::LayoutMotion(
                LayoutMotionError::EmptyLayout,
            ))?;
        let current = self
            .current_visual_position(snapshot)
            .map_err(crate::coordinator::CoreError::LayoutMotion)?;
        let rows = layout_motion::command_row_span(snapshot, current, 0)
            .map_err(crate::coordinator::CoreError::LayoutMotion)?;
        let row = &snapshot.rows[rows.start];
        let exact_prefix = snapshot
            .rows
            .first()
            .is_some_and(|first| first.hard_line_index == 0 && first.fragment_index == 0)
            && snapshot.rows[..=rows.start].windows(2).all(|pair| {
                (pair[1].hard_line_index == pair[0].hard_line_index
                    && pair[1].fragment_index == pair[0].fragment_index + 1)
                    || (pair[1].hard_line_index == pair[0].hard_line_index + 1
                        && pair[1].fragment_index == 0
                        && !pair[0].wraps_to_next)
            });
        let at = self.cursor.clamp(row.text_range.start, row.text_range.end);
        Ok(LineLocation {
            mode: self.line_mode,
            line: if row.hard_line_index == 0 {
                // A resumed first hard line already knows the exact number
                // of preceding rows from its validated wrap checkpoint.
                Some(row.fragment_index + 1)
            } else {
                exact_prefix.then_some(rows.start + 1)
            },
            column: document.text()[row.text_range.start..at]
                .graphemes(true)
                .count()
                + 1,
            hard_line: row.hard_line_index + 1,
            fragment: row.fragment_index + 1,
        })
    }
}

impl CommandInterpreter {
    pub(super) fn paste_physical_lines(
        &mut self,
        document: &mut Document,
        before: bool,
        count: usize,
        follow: bool,
        name: char,
        value: RegisterValue,
    ) -> Result<CommandOutput, DocumentError> {
        let line = self.physical_line(document)?;
        let mut repeated = match checked_register_repetition(&value, count) {
            Ok(value) => value,
            Err(error) => return Ok(error.into_command_output()),
        };
        let at = if before {
            line.source_range.start
        } else {
            line.source_range.end
        };
        let content_offset =
            if !before && at == document.source_byte_len() && !line.text.ends_with('\n') {
                repeated = match linewise_register_at_eof(repeated, count) {
                    Ok(value) => value,
                    Err(error) => return Ok(error.into_command_output()),
                };
                document
                    .encoding()
                    .encode_fragment(match document.file_format() {
                        FileFormat::Unix => "\n",
                        FileFormat::Dos => "\r\n",
                        FileFormat::Mac => "\r",
                    })?
                    .len()
            } else {
                0
            };
        repeated.text = self.assist_source_input(document, at..at, &repeated.text)?;
        document.replace_physical_source(at..at, repeated.text.clone())?;
        let source = if follow {
            at + document
                .encoding()
                .encode_fragment(&repeated.text.replace(
                    "\n",
                    match document.file_format() {
                        FileFormat::Unix => "\n",
                        FileFormat::Dos => "\r\n",
                        FileFormat::Mac => "\r",
                    },
                ))?
                .len()
        } else {
            at + content_offset
        };
        self.cursor = document.visible_point_for_source(source, true)?;
        self.cursor =
            normalize_normal_cursor(document.text(), &document.hard_line_snapshot(), self.cursor);
        self.visual_position = None;
        self.remember_physical_cursor(document, source);
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::Paste {
                before,
                follow,
                count,
                register: name,
            });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }
}

impl CommandInterpreter {
    pub(super) fn mode_line_insertion(
        &self,
        document: &Document,
        at_end: bool,
        nonblank: bool,
    ) -> Option<usize> {
        if self.line_mode == LineMode::PhysicalSource {
            let line = self.physical_line(document).ok()?;
            let source = if at_end {
                line.content_range.end
            } else if nonblank {
                line.content_range.start
                    + document
                        .encoding()
                        .encode_fragment(
                            &line.text[..line.text.len()
                                - line.text.trim_start_matches([' ', '\t']).len()],
                        )
                        .ok()?
                        .len()
            } else {
                line.source_range.start
            };
            return document.visible_point_for_source(source, !at_end).ok();
        }
        let snapshot = self.line_layout.as_ref()?;
        let position = self.current_visual_position(snapshot).ok()?;
        let point = if at_end {
            g_dollar_for_document(document, snapshot, position)
        } else if nonblank {
            g_caret(snapshot, document.text(), position)
        } else {
            g0(snapshot, position)
        }
        .ok()?;
        Some(point.text_offset)
    }
    fn mode_visual_extent(
        &self,
        document: &Document,
        snapshot: Option<&LayoutSnapshot>,
    ) -> Option<(LineExtent, usize)> {
        if self.mode != Mode::VisualLine {
            return None;
        }
        let anchor = self.visual_anchor.unwrap_or(self.cursor);
        if self.line_mode == LineMode::PhysicalSource {
            let first = if let Some(source) = self.visual_source_anchor.filter(|source| {
                source.document() == document.id() && source.revision() == document.revision()
            }) {
                document.physical_line_at_source(source.offset()).ok()?
            } else {
                document.physical_line_at_text(anchor).ok()?
            };
            let last = self.physical_line(document).ok()?;
            let low = document.physical_line(first.index.min(last.index)).ok()?;
            let high = document.physical_line(first.index.max(last.index)).ok()?;
            let a = document
                .visible_point_for_source(low.source_range.start, true)
                .ok()?;
            let b = document
                .visible_point_for_source(high.source_range.end, false)
                .ok()?;
            return Some((
                LineExtent {
                    text: a.min(b)..a.max(b),
                    source: Some(low.source_range.start..high.source_range.end),
                    whole: true,
                },
                first.index.abs_diff(last.index) + 1,
            ));
        }
        let snapshot =
            snapshot.filter(|snapshot| snapshot.document_revision == document.revision())?;
        let a = layout_motion::command_row_span(
            snapshot,
            VisualPosition {
                text_offset: anchor,
                affinity: BoundaryAffinity::Downstream,
            },
            0,
        )
        .ok()?;
        let b = layout_motion::command_row_span(
            snapshot,
            self.current_visual_position(snapshot).ok()?,
            0,
        )
        .ok()?;
        let first = &snapshot.rows[a.start.min(b.start)];
        let last = &snapshot.rows[a.start.max(b.start)];
        let mut end = last.text_range.end;
        if !last.wraps_to_next && document.text().as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
        Some((
            LineExtent {
                text: first.text_range.start..end,
                source: None,
                whole: true,
            },
            a.start.abs_diff(b.start) + 1,
        ))
    }
    pub(crate) fn line_selection_range(
        &self,
        document: &Document,
        snapshot: Option<&LayoutSnapshot>,
    ) -> Option<Range<usize>> {
        self.mode_visual_extent(document, snapshot)
            .map(|(extent, _)| extent.text)
            .or_else(|| self.linear_visual_selection_range(document))
    }
    pub(super) fn apply_mode_visual_operator(
        &mut self,
        document: &mut Document,
        operator: Operator,
        applications: usize,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        let Some((extent, count)) = self.mode_visual_extent(document, self.line_layout.as_ref())
        else {
            return Ok(None);
        };
        self.remember_visual();
        let register = self.requested_register.take();
        let mut output = self.apply_mode_line_extent(
            document,
            operator,
            LineShape::Whole,
            count,
            register,
            extent,
            applications,
        )?;
        if output.status != CommandStatus::Complete {
            return Ok(Some(output));
        }
        self.visual_anchor = None;
        self.visual_to_line_end = false;
        if operator != Operator::Change {
            self.mode = Mode::Normal;
        }
        output.mode_changed = true;
        Ok(Some(output))
    }
}
impl CommandInterpreter {
    pub(super) fn open_physical_line(
        &mut self,
        document: &mut Document,
        above: bool,
    ) -> Result<(), DocumentError> {
        let line = self.physical_line(document)?;
        let source = if above {
            line.source_range.start
        } else {
            line.content_range.end
        };
        document.replace_physical_source(source..source, "\n")?;
        let at = source
            + if above {
                0
            } else {
                document
                    .encoding()
                    .encode_fragment(match document.file_format() {
                        FileFormat::Unix => "\n",
                        FileFormat::Dos => "\r\n",
                        FileFormat::Mac => "\r",
                    })?
                    .len()
            };
        self.cursor = document.visible_point_for_source(at, true)?;
        self.remember_physical_cursor(document, at);
        Ok(())
    }
}

impl CommandInterpreter {
    pub(super) fn enter_mode_visual_line(
        &mut self,
        document: &Document,
        count: usize,
    ) -> Option<CommandOutput> {
        let origin = self.cursor;
        let mut source_target = None;
        let active = if self.line_mode == LineMode::PhysicalSource {
            let line = self.physical_line(document).ok()?;
            self.visual_source_anchor = document
                .source_point(self.physical_source_cursor(document).ok()?)
                .ok();
            let target = document
                .physical_line(
                    line.index
                        .saturating_add(count.saturating_sub(1))
                        .min(document.physical_line_count().ok()?.saturating_sub(1)),
                )
                .ok()?;
            source_target = Some(target.content_range.start);
            document
                .visible_point_for_source(target.content_range.start, true)
                .ok()?
        } else {
            let snapshot = self.line_layout.as_ref()?;
            let current = self.current_visual_position(snapshot).ok()?;
            let rows = match layout_motion::command_row_span(
                snapshot,
                current,
                count.saturating_sub(1).min(isize::MAX as usize) as isize,
            ) {
                Ok(rows) => rows,
                Err(error) => return Some(layout_error(error)),
            };
            if count == 1 {
                origin
            } else {
                snapshot.rows[rows.end - 1].text_range.start
            }
        };
        self.mode = Mode::VisualLine;
        self.visual_anchor = Some(origin);
        self.cursor = active;
        self.visual_to_line_end = false;
        self.visual_position = None;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_block = None;
        self.active_visual_block = None;
        if let Some(source) = source_target {
            self.remember_physical_cursor(document, source);
        }
        Some(CommandOutput {
            mode_changed: true,
            cursor_moved: active != origin,
            ..CommandOutput::complete()
        })
    }
}

impl CommandInterpreter {
    fn visual_indent_edits(
        &self,
        document: &Document,
        range: &Range<usize>,
        operator: Operator,
        applications: usize,
    ) -> Result<Vec<TextEdit>, DocumentError> {
        let Some(snapshot) = &self.line_layout else {
            return Err(DocumentError::AmbiguousProjection);
        };
        let shift = applications
            .checked_mul(4)
            .filter(|size| *size <= 1_000_000)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let mut edits = Vec::new();
        for row in &snapshot.rows {
            if row.text_range.start >= range.end || row.text_range.end < range.start {
                continue;
            }
            let mut start = row.text_range.start;
            for list in document.projection().list_structure().lists {
                for item in list.items {
                    if item.marker_is_synthetic && item.marker_range.start == start {
                        start = item.marker_range.end;
                    }
                }
            }
            if start > row.text_range.end {
                continue;
            }
            let text = &document.text()[start..row.text_range.end];
            match operator {
                Operator::Indent => edits.push(TextEdit::new(start..start, " ".repeat(shift))),
                Operator::Outdent => {
                    let count = text
                        .bytes()
                        .take_while(|byte| *byte == b' ')
                        .take(shift)
                        .count();
                    if count > 0 {
                        edits.push(TextEdit::new(start..start + count, ""));
                    }
                }
                Operator::Reindent => {
                    let count = text.len() - text.trim_start_matches([' ', '\t']).len();
                    if count > 0 {
                        edits.push(TextEdit::new(start..start + count, ""));
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(edits)
    }
}

impl CommandInterpreter {
    pub(super) fn mode_join_count(
        &self,
        document: &Document,
        count: usize,
    ) -> Result<Result<usize, LayoutMotionError>, DocumentError> {
        if self.line_mode != LineMode::Visual || self.line_layout.is_none() {
            return Ok(Ok(count.max(2)));
        }
        let extent = match self.mode_extent(document, LineShape::Whole, count.max(2))? {
            Ok(extent) => extent,
            Err(error) => return Ok(Err(error)),
        };
        Ok(Ok(hard_line_count_for_range(
            &document.hard_line_snapshot(),
            &extent.text,
        )))
    }
    pub(super) fn join_physical_lines(
        &mut self,
        document: &mut Document,
        count: usize,
        insert_space: bool,
    ) -> Result<CommandOutput, DocumentError> {
        let first = self.physical_line(document)?;
        let last_index = first
            .index
            .saturating_add(count.max(2) - 1)
            .min(document.physical_line_count()?.saturating_sub(1));
        if last_index == first.index {
            return Ok(CommandOutput::complete());
        }
        let mut joined = document.physical_source_text(first.content_range.clone())?;
        let mut cursor = joined.len();
        for index in first.index + 1..=last_index {
            let line = document.physical_line(index)?;
            let content = document.physical_source_text(line.content_range)?;
            cursor = joined.len();
            let value = if insert_space {
                content.trim_start_matches([' ', '\t'])
            } else {
                content.as_str()
            };
            if insert_space
                && !joined.is_empty()
                && !joined.ends_with([' ', '\t'])
                && !value.is_empty()
                && !value.starts_with(')')
            {
                joined.push(' ');
            }
            joined.push_str(value);
        }
        let last = document.physical_line(last_index)?;
        let source_cursor = first.source_range.start
            + document
                .encoding()
                .encode_fragment(&joined[..cursor])?
                .len();
        document
            .replace_physical_source(first.source_range.start..last.content_range.end, joined)?;
        self.cursor = document.visible_point_for_source(source_cursor, true)?;
        self.visual_position = None;
        self.remember_physical_cursor(document, source_cursor);
        if !self.replaying {
            self.last_repeat = Some(RepeatAction::Join {
                count,
                insert_space,
            });
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            ..CommandOutput::complete()
        })
    }
    pub(super) fn mode_visual_join(
        &mut self,
        document: &mut Document,
        insert_space: bool,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        let Some((extent, count)) = self.mode_visual_extent(document, self.line_layout.as_ref())
        else {
            return Ok(None);
        };
        self.remember_visual();
        self.cursor = extent.text.start;
        self.visual_position = None;
        if let Some(range) = extent.source {
            self.remember_physical_cursor(document, range.start);
        }
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.visual_to_line_end = false;
        let mut output = self.join_lines(document, count, insert_space)?;
        output.mode_changed = true;
        Ok(Some(output))
    }
}
