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
/// equals the current cursor. Committed source maps explicitly rebase both
/// coordinates; missing lineage retires the disposable cache.
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
    /// Source-backed current line without decoding its possibly huge body.
    pub fn physical_source_line_index(&self, document: &Document) -> Result<usize, DocumentError> {
        document.physical_line_index_at_source(self.physical_source_cursor(document)?)
    }

    /// Source index for display queries, including a caret inside a shared
    /// contributor that has no independently editable source boundary.
    pub(crate) fn physical_display_source_line_index(
        &self,
        document: &Document,
    ) -> Result<usize, DocumentError> {
        document.physical_line_index_at_source(self.physical_display_source_cursor(document)?)
    }

    fn physical_display_source_cursor(&self, document: &Document) -> Result<usize, DocumentError> {
        if let Some(source) = self.retained_physical_cursor_source(document) {
            return Ok(source.offset());
        }
        document.text_point(self.cursor)?;
        document.projection().source_insertion_point(self.cursor, true)
            .or_else(|| {
                // A shared source contributor can yield several logical
                // graphemes (for example &fjlig;). Its interior has no
                // editable source boundary. Display queries identify that
                // construct without creating an insertion point or anchor.
                document.projection().provenance_touching(&(self.cursor..self.cursor))
                    .into_iter().find(|span| !span.source.is_empty()
                        && span.formatted.start < self.cursor && self.cursor < span.formatted.end)
                    .map(|span| span.source.start)
            })
            .ok_or(DocumentError::AmbiguousProjection)
    }

    fn physical_line(
        &self,
        document: &Document,
    ) -> Result<crate::document::PhysicalSourceLine, DocumentError> {
        document.physical_line_at_source(self.physical_source_cursor(document)?)
    }
    pub(super) fn remember_physical_cursor(&mut self, document: &Document, at: usize) {
        self.physical_cursor = document
            .source_point(at)
            .ok()
            .zip(document.text_point(self.cursor).ok())
            .map(|(source, text)| PhysicalCursor { source, text });
    }

    pub(super) fn retained_physical_cursor_source(&self, document: &Document) -> Option<crate::document::SourcePoint> {
        self.physical_cursor.filter(|cached| cached.source.document() == document.id()
            && cached.source.revision() == document.revision()
            && cached.text.revision() == document.revision()
            && cached.text.offset() == self.cursor).map(|cached| cached.source)
    }

    pub(super) fn rebase_physical_cursor(
        &mut self, map: &PositionMap, association: Association,
    ) -> Result<(), PositionError> {
        let Some(previous) = self.physical_cursor else { return Ok(()); };
        if previous.source.revision() == map.target_revision() { return Ok(()); }
        if previous.source.revision() != map.source_revision()
            || previous.text.revision() != map.source_revision() {
            self.physical_cursor = None;
            return Ok(());
        }
        let Some(source_map) = map.source_position_map() else {
            self.physical_cursor = None;
            return Ok(());
        };
        let source = source_map.map_source_point(previous.source, association,
            DeletionRecovery::PreferFollowingThenPreceding)?.value().copied();
        let text = map.map_text_point(previous.text, association, self.boundary_affinity,
            DeletionRecovery::PreferFollowingThenPreceding)?.value().copied();
        self.physical_cursor = source.zip(text)
            .filter(|(_, text)| text.offset() == self.cursor)
            .map(|(source, text)| PhysicalCursor { source, text });
        Ok(())
    }

    /// Recover only the displayed caret from a retained exact source identity.
    /// Hidden syntax can supply no unique glyph; the existing downstream
    /// presentation policy selects its usable neighboring caret geometry.
    pub(crate) fn recover_physical_cursor_display(
        &mut self, document: &Document,
    ) -> Result<bool, DocumentError> {
        let Some(cached) = self.physical_cursor else { return Ok(false); };
        if cached.source.document() != document.id()
            || cached.source.revision() != document.revision()
            || cached.text.revision() != document.revision()
            || cached.text.offset() != self.cursor {
            self.physical_cursor = None;
            return Ok(false);
        }
        let visible = document.visible_point_for_source(cached.source.offset(), true)?;
        self.cursor = if matches!(self.mode, Mode::Insert | Mode::Replace) { visible } else {
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), visible)
        };
        self.remember_physical_cursor(document, cached.source.offset());
        Ok(true)
    }
    /// Absolute line addresses use file-line ordinals in every view mode.
    /// The final editable boundary is not a numbered file line.
    pub(super) fn physical_line_destination(
        &self,
        document: &Document,
        one_based: usize,
    ) -> Result<(usize, usize), DocumentError> {
        let index = one_based
            .saturating_sub(1)
            .min(document.physical_source_line_number_count().saturating_sub(1));
        let source = document.physical_first_nonblank_source(index)?;
        let point = document.visible_point_for_source(source, true)?;
        Ok((
            source,
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), point),
        ))
    }

    pub(super) fn goto_physical_line(
        &mut self,
        document: &Document,
        one_based: usize,
    ) -> CommandOutput {
        self.goto_physical_line_with_column(document, one_based, !self.vim_options.flag("startofline"))
    }
    pub(super) fn goto_physical_line_with_column(
        &mut self, document: &Document, one_based: usize, preserve_column: bool,
    ) -> CommandOutput {
        let (source, point) = match self.physical_line_destination(document, one_based) {
            Ok(destination) => destination,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::Error(format!("physical line navigation failed: {error}")),
                    ..CommandOutput::complete()
                };
            }
        };
        let old = self.cursor;
        let old_source = self.physical_source_cursor(document).ok();
        let (source, point) = if preserve_column {
            let lines = document.hard_line_snapshot();
            let target = options::at_logical_column(&lines, point, options::logical_column(&lines, old));
            (document.projection().source_insertion_point(target, true).unwrap_or(source), target)
        } else { (source, point) };
        self.cursor = point;
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.preferred_column = None;
        self.desired_x = None;
        self.visual_to_line_end = false;
        self.remember_physical_cursor(document, source);
        let output = CommandOutput {
            cursor_moved: old != self.cursor || old_source != Some(source),
            ..CommandOutput::complete()
        };
        if old != self.cursor {
            self.record_successful_jump(document, old, output)
        } else {
            if output.cursor_moved {
                self.retire_typing_context();
            }
            output
        }
    }
    pub fn line_mode(&self) -> LineMode {
        self.line_mode
    }
    /// Explicitly choose this view's line mode. Choosing its format's default
    /// keeps the view following that default across format changes.
    pub fn set_line_mode(
        &mut self,
        document: &Document,
        mode: LineMode,
    ) -> Result<(), DocumentError> {
        self.line_mode_explicit = mode != LineMode::default_for(document.format());
        self.install_line_mode(mode);
        Ok(())
    }

    /// Adopt the default line mode of a newly selected format unless the view
    /// explicitly chose another mode.
    pub(crate) fn follow_format_line_mode(&mut self, format: crate::document::Format) {
        let mode = LineMode::default_for(format);
        if !self.line_mode_explicit && self.line_mode != mode {
            self.install_line_mode(mode);
        }
    }

    fn install_line_mode(&mut self, mode: LineMode) {
        self.line_mode = mode;
        self.physical_cursor = None;
        self.clear_pending();
        self.preferred_column = None;
        self.desired_x = None;
    }
}

impl LineMode {
    /// Code counts physical source lines; formatted and plain text count
    /// displayed rows.
    pub const fn default_for(format: crate::document::Format) -> Self {
        if format.is_code() { Self::PhysicalSource } else { Self::Visual }
    }
}

#[cfg(test)]
mod status_tests {
    use crate::document::BoundaryAffinity;
    use crate::layout::MockTextMeasurementProvider;
    use crate::{Core, CoreEvent, Document};

    #[test]
    fn physical_status_does_not_decode_the_long_suffix_after_the_caret() {
        let document = Document::new("x".repeat(400_000));
        let commands = super::CommandInterpreter::new();
        let (location, work) = crate::document::measure_document_work(|| {
            commands.physical_line_location(&document).unwrap()
        });
        assert_eq!((location.line, location.column), (Some(1), 1));
        assert!(work.source_range_materialized_bytes <= 4, "{work:?}");
        assert_eq!(work.source_decoded_bytes, 0);
    }

    #[test]
    fn physical_status_locates_shared_entity_contributors_without_edit_anchors() {
        use crate::document::{Encoding, FileFormat, Format};
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let source = format!("first{0}{0}e\u{301} &fjlig;z", file_format.spelling());
                let bytes = encoding.encode_fragment(&source).unwrap();
                let document = Document::from_bytes_with_file_format(bytes.clone(), encoding, Format::Markdown, file_format).unwrap();
                let mut commands = super::CommandInterpreter::new();
                let at = document.text().find('j').unwrap();
                assert!(commands.set_cursor(&document, at));
                let before_revision = document.revision();
                let location = commands.physical_line_location(&document).unwrap();
                assert_eq!((location.line, location.column), (Some(3), 3), "{encoding:?} {file_format:?}");
                assert!(commands.physical_cursor.is_none(), "display recovery must not authorize an ambiguous edit");
                assert_eq!(commands.cursor(), at);
                assert_eq!(document.source_bytes(), bytes);
                assert_eq!(document.revision(), before_revision);
                assert!(commands.set_cursor(&document, document.text().find('z').unwrap()));
                let after = commands.physical_line_location(&document).unwrap();
                assert_eq!((after.line, after.column), (Some(3), 10));
            }
        }
    }

    #[test]
    fn entity_interior_gutter_source_line_matches_status_without_decoding() {
        use crate::document::{Encoding, Format};
        let source = b"first\n\n&fjlig;z".to_vec();
        let document = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find('j').unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 200.0);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        }).unwrap();
        let before_revision = core.document().revision();
        let (line, work) = crate::document::measure_document_work(|| {
            core.current_physical_source_line(view).unwrap()
        });
        assert_eq!(line, 3);
        assert_eq!(Some(line), core.physical_line_location(view).unwrap().line);
        assert_eq!(work.source_decoded_bytes, 0);
        assert_eq!(work.source_range_materialized_bytes, 0);
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), before_revision);
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
    }

    #[test]
    fn unicode_status_column_does_not_flatten_unrelated_document_text() {
        let prefix = "Ae\u{301}👩‍💻";
        let mut document = Document::new(format!("e\u{301}👩‍💻Z\n{}", "later line\n".repeat(20_000)));
        document.insert(0, "A").unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 200.0);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: prefix.len(),
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        }).unwrap();
        assert!(!core.document().projection().compatibility_text_is_materialized());
        let location = core.line_location(view).unwrap();
        assert_eq!(location.column, 4);
        assert!(!core.document().projection().compatibility_text_is_materialized());
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
pub(super) struct LineExtent {
    pub(super) text: Range<usize>,
    source: Option<Range<usize>>,
    whole: bool,
}
impl CommandInterpreter {
    pub(super) fn mode_line_key(&self, key: Key) -> bool {
        let key = self.resolved_clipboard_copy_key(key);
        // Native vertical navigation follows the displayed rows in every
        // editing mode. Vim operator motions still use the selected line domain.
        if self.pending == Pending::None && matches!(key, Key::Up | Key::Down) {
            return false;
        }
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
                                target: RepeatTarget::ViewLine(..) | RepeatTarget::VisualViewLine(..),
                                ..
                            },
                            ..
                        } | RepeatAction::Join { .. }
                            | RepeatAction::Insert {
                                placement: InsertPlacement::LineStart
                                    | InsertPlacement::LineColumnOne
                                    | InsertPlacement::LineEnd
                                    | InsertPlacement::OpenAbove
                                    | InsertPlacement::OpenBelow,
                                ..
                            }
                    )
                ),
                _ => false,
            },
            Pending::G { .. } => key == Key::Char('J') || self.mode == Mode::Normal && key == Key::Char('I'),
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
                    .min(document.physical_source_line_number_count() - 1)
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
        let mut rows = match layout_motion::command_row_span(snapshot, current, delta) {
            Ok(rows) => rows,
            Err(error) => return Ok(Err(error)),
        };
        let end = document.projection().text_tree().byte_len();
        if rows.len() > 1 && snapshot.rows[rows.end - 1].text_range == (end..end)
            && document.addressable_hard_line_count() < document.line_count()
        {
            rows.end -= 1;
        }
        let first = &snapshot.rows[rows.start];
        let last = &snapshot.rows[rows.end - 1];
        let start = if shape == LineShape::End {
            self.cursor
        } else if shape == LineShape::StartNonblank {
            match layout_motion::g_caret_for_document(document, snapshot, current) {
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
            && first.fragment_index == 0
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
        if self.pending == Pending::None && self.mode != Mode::Normal
            && matches!(key, Key::Char('J' | 'A' | 'I' | 'o' | 'O' | 'C' | 'D' | 'Y' | 'S')) {
            return Ok(None);
        }
        if key == Key::Char('.') {
            let count = self.count.take();
            return self.repeat_last_change(document, count).map(Some);
        }
        if let Pending::Operator(pending) = self.pending {
            if pending.operator.is_format() {
                // Reflow's implicit line motions and doubled forms count
                // complete hard lines in every line mode.
                return Ok(None);
            }
            let count = match effective_operator_count(pending) {
                Ok(count) => count,
                Err(error) => return Ok(Some(CommandOutput::count_error(error))),
            };
            let shape = if key == Key::Char(pending.operator.doubled_key()) {
                LineShape::Whole
            } else {
                match key {
                    Key::Char('$') | Key::End => LineShape::End,
                    Key::Char('0') => LineShape::Start,
                    Key::Char('^') | Key::Home => LineShape::StartNonblank,
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
                    .and_then(|motion| layout_motion::g_caret_for_document(document, &snapshot, motion.position)),
                Key::Char('-') => gk(&snapshot, current, count, None)
                    .and_then(|motion| layout_motion::g_caret_for_document(document, &snapshot, motion.position)),
                Key::Char('0') => g0(&snapshot, current),
                Key::Char('^') | Key::Home => layout_motion::g_caret_for_document(document, &snapshot, current),
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
                        layout_motion::g_caret_for_document(document, &snapshot, current)
                    } else {
                        gj(&snapshot, current, count - 1, None)
                            .and_then(|motion| layout_motion::g_caret_for_document(document, &snapshot, motion.position))
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
            let mut position = match moved {
                Ok(position) => position,
                Err(error) => return Ok(Some(layout_error(error))),
            };
            if position.text_offset == document.projection().text_tree().byte_len()
                && document.addressable_hard_line_count() < document.line_count()
            {
                position = match gk(&snapshot, position, 1, desired_x) {
                    Ok(motion) => motion.position,
                    Err(error) => return Ok(Some(layout_error(error))),
                };
                let edge = match key {
                    Key::Char('+' | '_') | Key::Enter => layout_motion::g_caret_for_document(document, &snapshot, position),
                    Key::Char('$') | Key::End => g_dollar_for_document(document, &snapshot, position),
                    _ => Ok(position),
                };
                position = match edge {
                    Ok(position) => position,
                    Err(error) => return Ok(Some(layout_error(error))),
                };
            }
            let output = self.install_visual_position(document, &snapshot, position);
            self.visual_to_line_end = self.mode == Mode::VisualCharacter
                && matches!(key, Key::Char('$') | Key::End);
            self.desired_x = desired_x;
            return Ok(Some(output));
        }
        let line = self.physical_line(document)?;
        let target_index = match key {
            Key::Char('j' | '+') | Key::Down | Key::Enter => line
                .index
                .saturating_add(count)
                .min(document.physical_source_line_number_count() - 1),
            Key::Char('k' | '-') | Key::Up => line.index.saturating_sub(count),
            Key::Char('$' | '_') | Key::End => line
                .index
                .saturating_add(count - 1)
                .min(document.physical_source_line_number_count() - 1),
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
            Key::Char('^' | '+' | '-' | '_') | Key::Home | Key::Enter => {
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
        self.visual_to_line_end = self.mode == Mode::VisualCharacter
            && matches!(key, Key::Char('$') | Key::End);
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
    pub(super) fn apply_physical_address_operator(
        &mut self,
        document: &mut Document,
        operator: Operator,
        motion: OperatorMotion,
        count: usize,
        register: Option<char>,
    ) -> Result<CommandOutput, DocumentError> {
        let address = if motion == OperatorMotion::Percentage {
            let Some(address) = physical_percentage_line(document, count) else {
                return Ok(invalid_percentage(count));
            };
            address
        } else { count };
        let current = self.physical_source_line_index(document)?;
        let target = address.saturating_sub(1)
            .min(document.physical_source_line_number_count().saturating_sub(1));
        let (first, _) = document.physical_line_ranges(current.min(target))?;
        let (last, _) = document.physical_line_ranges(current.max(target))?;
        let source = first.start..last.end;
        let a = document.visible_point_for_source(source.start, true)?;
        let b = document.visible_point_for_source(source.end, false)?;
        if matches!(operator, Operator::Format | Operator::FormatKeepCursor) {
            return self.apply_operator_with_repeat(document, operator,
                MotionExtent { range: a.min(b)..a.max(b), kind: MotionKind::Linewise },
                register, RepeatTarget::PhysicalLineAddress(motion), count);
        }
        let origin = if target != current {
            Some(document.text_anchor(document.text_point(self.cursor)?,
                Association::AfterInsertion, BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding).map_err(command_position_document_error)?)
        } else { None };
        let (result, map) = document.capture_position_maps(|document| self.apply_mode_line_extent(document,
            operator, LineShape::Whole, count, register,
            LineExtent { text: a.min(b)..a.max(b), source: Some(source), whole: true }, 1));
        let mut output = result?;
        if operator == Operator::Yank && target < current && output.status == CommandStatus::Complete {
            let old_cursor = self.cursor;
            let old_line = document.physical_line(current)?;
            let old_source = self.physical_source_cursor(document)?
                .clamp(old_line.content_range.start, old_line.content_range.end);
            let column = document.physical_source_text(old_line.content_range.start..old_source)?.graphemes(true).count();
            let line = document.physical_line(target)?;
            let prefix = line.text.trim_end_matches('\n').graphemes(true).take(column).collect::<String>();
            let source = line.content_range.start + document.encoding().encode_fragment(&prefix)?.len();
            self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(),
                document.visible_point_for_source(source, true)?);
            self.remember_physical_cursor(document, source);
            self.boundary_affinity = BoundaryAffinity::Downstream;
            output.cursor_moved = self.cursor != old_cursor;
        }
        if let Some(origin) = origin {
            if output.document_changed || operator == Operator::Yank {
                if let Some(mapped) = mapped_anchor(&map, origin).map_err(command_position_document_error)? {
                    self.record_jump(document, mapped, self.cursor);
                }
            }
        }
        if !self.replaying && operator != Operator::Yank && (output.document_changed || operator == Operator::Change) {
            let command = OperatorRepeat { operator, target: RepeatTarget::PhysicalLineAddress(motion), count, register };
            if operator == Operator::Change {
                if let Some(session) = self.insert_session.as_mut() { session.repeat_operator = Some(command); }
            } else {
                self.last_repeat = Some(RepeatAction::Operator { command, edits: None });
            }
        }
        Ok(output)
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
        let inherited_typing = if operator == Operator::Change && extent.source.is_none() {
            document.replacement_typing_context(extent.text.clone())?
        } else { None };
        if matches!(
            operator,
            Operator::Yank | Operator::Delete | Operator::Change
        ) {
            if let Err(output) = self.require_register_write_kind(register, operator == Operator::Yank) {
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
                extent.source.as_ref().filter(|_| !document.format().is_markdown())
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
        let mut physical_change_at = None;
        let mut retained_indent = String::new();
        let mut output = if let Some(mut source) = extent.source {
            let source_len = document.source_byte_len();
            let payload = document.physical_source_payload(source.clone())?;
            let normalized = payload.text().to_owned();
            let mut register_text = normalized.clone();
            let mut breaks = payload.break_offsets().to_vec();
            if extent.whole && !breaks.last().is_some_and(|index| index + 1 == register_text.len()) {
                breaks.push(register_text.len());
                register_text.push('\n');
            }
            let value = RegisterValue::try_new(register_text,
                if extent.whole { RegisterKind::Linewise } else { RegisterKind::Characterwise }, breaks)
                .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
            if operator == Operator::Yank {
                self.yank_register(register, value);
                return Ok(CommandOutput::complete());
            }
            let transform = match operator {
                Operator::Lowercase => Some(crate::document::PhysicalSourceTransform::Lowercase),
                Operator::Uppercase => Some(crate::document::PhysicalSourceTransform::Uppercase),
                Operator::ToggleCase => Some(crate::document::PhysicalSourceTransform::ToggleCase),
                Operator::Rot13 => Some(crate::document::PhysicalSourceTransform::Rot13),
                Operator::Indent => Some(crate::document::PhysicalSourceTransform::Indent { columns: self.indentation_options().shift_width().saturating_mul(applications), options: self.indentation_options() }),
                Operator::Outdent => Some(crate::document::PhysicalSourceTransform::Outdent { columns: self.indentation_options().shift_width().saturating_mul(applications), options: self.indentation_options() }),
                Operator::Reindent => Some(crate::document::PhysicalSourceTransform::Reindent { options: self.indentation_options() }),
                _ => None,
            };
            let replacement = match operator {
                Operator::Delete | Operator::Change => {
                    if operator == Operator::Change && extent.whole && self.literal_indentation(document)
                        && self.indentation_options().autoindent {
                        retained_indent = normalized[..crate::document::leading_whitespace_len(&normalized)].to_owned();
                    }
                    if operator == Operator::Change && extent.whole
                        && (source.end < source_len || payload.break_offsets().last().is_some_and(|offset| offset + 1 == normalized.len())) {
                        format!("{retained_indent}\n")
                    } else {
                        retained_indent.clone()
                    }
                }
                Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase | Operator::Rot13 => String::new(),
                Operator::Format | Operator::FormatKeepCursor => {
                    unreachable!("reflow bypasses view-line extents")
                }
                Operator::Indent | Operator::Outdent | Operator::Reindent => String::new(),
                Operator::Yank => unreachable!(),
            };
            if operator == Operator::Delete
                && extent.whole
                && source.end == source_len
                && source.start > 0
                && (source.is_empty() || !payload.break_offsets().last().is_some_and(|offset| offset + 1 == normalized.len()))
            {
                let index = document.physical_line_at_source(source.start)?.index;
                if index > 0 {
                    source.start = document.physical_line(index - 1)?.content_range.end;
                }
            }
            if operator == Operator::Change {
                document.begin_edit_group();
            }
            if let Some(transform) = transform {
                document.apply_model_request(ModelRequest::TransformPhysicalSource {
                    document: document.id(), revision: document.revision(), range: source.clone(), transform,
                }).map_err(command_document_error)?;
            } else {
                let result = document.replace_physical_source(source.clone(), replacement);
                if result.is_err() && operator == Operator::Change { document.end_edit_group(); }
                result?;
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
            let insertion_source = (source.start + document.encoding().encode_fragment(&retained_indent)?.len()).min(document.source_byte_len());
            self.cursor = document.visible_point_for_source(insertion_source, true)?;
            self.remember_physical_cursor(document, insertion_source);
            if operator == Operator::Change {
                physical_change_at = Some(document.source_point(insertion_source)
                    .map_err(|_| DocumentError::AmbiguousProjection)?);
            }
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
                && range.start < range.end
                && document.text().as_bytes().get(range.end.saturating_sub(1)) == Some(&b'\n')
            {
                range.end -= 1;
            }
            if operator == Operator::Change && extent.whole && whole_hard_lines
                && self.literal_indentation(document) && self.indentation_options().autoindent {
                let line = lines.line_at_offset(range.start).map_err(|_| DocumentError::AmbiguousProjection)?;
                let content = lines.slice_utf8(line.content_range()).map_err(DocumentError::FormattedTextStorage)?;
                let prefix = crate::document::leading_whitespace_len(&content);
                retained_indent = content[..prefix].to_owned();
                range.start += prefix;
            }
            let replacement = match operator {
                Operator::Delete | Operator::Change => String::new(),
                Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase | Operator::Rot13 => {
                    change_case(&document.text()[range.clone()], operator)
                }
                _ => String::new(),
            };
            if operator == Operator::Change {
                document.begin_edit_group();
            }
            let mut changed_cursor = None;
            if matches!(
                operator,
                Operator::Indent | Operator::Outdent | Operator::Reindent
            ) {
                let edits = self.visual_indent_edits(document, &range, operator, applications)?;
                document.apply_edits(edits)?;
            } else if operator == Operator::Delete && extent.whole {
                document.delete_visual_text(range.clone())?;
            } else if operator == Operator::Change {
                changed_cursor = Some(delete_with_cursor(document, range.clone())?);
            } else if matches!(operator, Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase | Operator::Rot13) {
                document.apply_edits(case_edits(&document.text()[range.clone()], range.start, operator))?;
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
            self.cursor = changed_cursor.unwrap_or_else(|| range.start.min(document.text().len()));
            CommandOutput {
                document_changed: before != document.revision(),
                cursor_moved: true,
                ..CommandOutput::complete()
            }
        };
        if operator == Operator::Change {
            output.merge(self.enter_insert(document, InsertPlacement::Before, 1));
            self.typing_style.inherited = inherited_typing;
            self.capture_generated_indent(document, retained_indent);
            if let Some(at) = physical_change_at {
                if document.source_insertion_requires_materialization(at)? {
                    if let Some(session) = self.insert_session.as_mut() {
                        session.source_destination = Some(at);
                        session.source_floor = Some(at);
                    }
                }
            }
            // The change itself already opened the shared undo group.
            document.end_edit_group();
        } else {
            let lines = document.hard_line_snapshot();
            if operator == Operator::Delete && extent.whole && self.literal_indentation(document) {
                self.cursor = cursor_after_linewise_deletion(document, self.cursor);
                self.physical_cursor = None;
            }
            self.cursor = normalize_normal_cursor(
                document.text(),
                &lines,
                self.cursor,
            );
            if let Some(at) = physical_change_at {
                if document.source_insertion_requires_materialization(at)? {
                    self.remember_physical_cursor(document, at.offset());
                }
            }
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
            return self.physical_line_location(document);
        }
        self.visual_line_location(document, snapshot)
    }

    /// Exact source coordinates, independent of command line mode or layout coverage.
    pub fn physical_line_location(
        &self,
        document: &Document,
    ) -> Result<LineLocation, crate::coordinator::CoreError> {
        let source = self.physical_display_source_cursor(document)?;
        let index = document.physical_line_index_at_source(source)?;
        // Location queries need only the prefix before the caret. Reading a
        // PhysicalSourceLine would also decode an arbitrarily long suffix.
        let (_, content) = document.physical_line_ranges(index)?;
        let at = source.clamp(content.start, content.end);
        let prefix = document.physical_source_text(content.start..at)?;
        Ok(LineLocation {
            mode: LineMode::PhysicalSource,
            line: Some(index + 1),
            column: prefix.graphemes(true).count() + 1,
            hard_line: index + 1,
            fragment: 1,
        })
    }

    fn visual_line_location(
        &self,
        document: &Document,
        snapshot: Option<&LayoutSnapshot>,
    ) -> Result<LineLocation, crate::coordinator::CoreError> {
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
        let range = row.text_range.start..at;
        let tree = document.projection().text_tree();
        // Equal byte and UTF-16 lengths prove the range is ASCII. Without an
        // LF, every byte is a grapheme; root aggregates then make even a giant
        // unwrapped ASCII prefix a logarithmic query. Unicode uses the exact
        // logical-boundary traversal of just the active visual fragment.
        let column = match (
            tree.utf16_offset_for_byte(range.start),
            tree.utf16_offset_for_byte(range.end),
            tree.hard_line_at_byte(range.start),
            tree.hard_line_at_byte(range.end),
        ) {
            (Ok(start), Ok(end), Ok(first), Ok(last))
                if end - start == range.len() && first == last => range.len(),
            _ => document
                .hard_line_snapshot()
                .grapheme_count(range)
                .ok_or(crate::coordinator::CoreError::LayoutMotion(
                    LayoutMotionError::EmptyLayout,
                ))?,
        } + 1;
        Ok(LineLocation {
            mode: self.line_mode,
            line: if row.hard_line_index == 0 {
                // A resumed first hard line already knows the exact number
                // of preceding rows from its validated wrap checkpoint.
                Some(row.fragment_index + 1)
            } else {
                exact_prefix.then_some(rows.start + 1)
            },
            column,
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
        if !matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.cursor = normalize_normal_cursor(document.text(), &document.hard_line_snapshot(), self.cursor);
        }
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
    /// Native line selectors share ^/$'s view policy while keeping the
    /// insertion boundary and edit-mode undo grouping intact.
    pub(super) fn move_edit_line_edge(
        &mut self,
        document: &Document,
        at_end: bool,
    ) -> Result<CommandOutput, DocumentError> {
        if self.line_mode == LineMode::Visual {
            if let Some(snapshot) = self.line_layout.clone() {
                let current = match self.current_visual_position(&snapshot) {
                    Ok(position) => position,
                    Err(error) => return Ok(layout_error(error)),
                };
                let moved = if at_end {
                    g_dollar_for_document(document, &snapshot, current)
                } else {
                    layout_motion::g_caret_for_document(document, &snapshot, current)
                };
                let position = match moved {
                    Ok(position) => position,
                    Err(error) => return Ok(layout_error(error)),
                };
                self.desired_x = None;
                self.preferred_column = None;
                return Ok(self.install_visual_position(document, &snapshot, position));
            }
            // Headless command interpretation retains its hard-line fallback.
            let output = self.move_cursor(document, if at_end {
                Motion::InsertionLineEnd
            } else {
                Motion::FirstNonBlank
            }, 1);
            self.boundary_affinity = if at_end { BoundaryAffinity::Upstream } else { BoundaryAffinity::Downstream };
            self.visual_position = None;
            return Ok(output);
        }
        let line = self.physical_line(document)?;
        let source = if at_end {
            line.content_range.end
        } else {
            let blank = line.text.len() - line.text.trim_start_matches([' ', '\t']).len();
            line.content_range.start + document.encoding().encode_fragment(&line.text[..blank])?.len()
        };
        let point = document.visible_point_for_source(source, !at_end)?;
        let before = (self.cursor, self.boundary_affinity);
        self.cursor = point;
        self.boundary_affinity = if at_end { BoundaryAffinity::Upstream } else { BoundaryAffinity::Downstream };
        self.visual_position = None;
        self.desired_x = None;
        self.preferred_column = None;
        self.typing_style = Default::default();
        self.remember_physical_cursor(document, source);
        Ok(CommandOutput {
            cursor_moved: before != (self.cursor, self.boundary_affinity),
            ..CommandOutput::complete()
        })
    }

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
            let rows = layout_motion::command_row_span(snapshot, position, 0).ok()?;
            let row = &snapshot.rows[rows.start];
            let text = document.hard_line_snapshot().slice_utf8(row.text_range.clone()).ok()?;
            let prefix = crate::document::leading_whitespace_len(&text);
            return Some(row.text_range.start + prefix);
        } else {
            g0(snapshot, position)
        }
        .ok()?;
        Some(point.text_offset)
    }
    pub(super) fn mode_visual_extent(
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
        if first.fragment_index == 0 && !last.wraps_to_next && document.text().as_bytes().get(end) == Some(&b'\n') {
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
        if operator.is_format() {
            // A Visual selection keeps its domain; reflow then expands the
            // checked extent to the intersected hard lines.
            return Ok(None);
        }
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
        let mut source_anchor = None;
        let active = if self.line_mode == LineMode::PhysicalSource {
            let line = self.physical_line(document).ok()?;
            source_anchor = document
                .source_point(self.physical_source_cursor(document).ok()?)
                .ok();
            let target = document
                .physical_line(
                    line.index
                        .saturating_add(count.saturating_sub(1))
                        .min(document.physical_source_line_number_count().saturating_sub(1)),
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
        self.set_selection_origin(SelectionOrigin::Command);
        self.selection_exclusive = false;
        self.selection_return_mode = Mode::Normal;
        self.visual_source_anchor = source_anchor;
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
        let options = self.indentation_options();
        let _shift = applications
            .checked_mul(options.shift_width())
            .filter(|size| *size <= 1_000_000)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let mut edits = Vec::new();
        let lists = document.projection().list_window(&range).structure.lists;
        for row in snapshot.rows.iter() {
            if row.text_range.start >= range.end || row.text_range.end <= range.start {
                continue;
            }
            let mut start = row.text_range.start;
            for list in &lists {
                for item in &list.items {
                    if item.marker_is_synthetic && item.marker_range.start == start {
                        start = item.marker_range.end;
                    }
                }
            }
            if start > row.text_range.end {
                continue;
            }
            let text = &document.text()[start..row.text_range.end];
            let prefix = crate::document::leading_whitespace_len(text);
            let old_columns = options.columns(&text[..prefix]);
            let columns = match operator {
                Operator::Indent | Operator::Outdent => options::shifted_columns(old_columns, options.shift_width(), applications, operator == Operator::Indent, self.vim_options.flag("shiftround"))
                    .ok_or(DocumentError::UnsupportedFormatting)?,
                Operator::Reindent => {
                    let lines = document.hard_line_snapshot();
                    let index = lines.line_at_offset(start).map_err(|_| DocumentError::AmbiguousProjection)?.index();
                    let mut columns = 0;
                    for previous in (index.saturating_sub(512)..index).rev() {
                        let line = lines.line(previous).ok_or(DocumentError::AmbiguousProjection)?;
                        let text = lines.slice_utf8(line.content_range()).map_err(DocumentError::FormattedTextStorage)?;
                        let prefix = crate::document::leading_whitespace_len(&text);
                        if prefix < text.len() { columns = options.columns(&text[..prefix]); break; }
                    }
                    columns
                },
                _ => unreachable!(),
            };
            let replacement = options.try_whitespace(0, columns).map_err(|_| DocumentError::UnsupportedFormatting)?;
            if replacement != text[..prefix] {
                edits.push(TextEdit::new(start..start + prefix, replacement));
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
            .min(document.physical_source_line_number_count().saturating_sub(1));
        if last_index == first.index {
            return Ok(CommandOutput::complete());
        }
        let mut joined = document.physical_source_text(first.content_range.clone())?;
        let mut cursor = joined.len();
        let mut previous = joined.clone();
        let profile = (document.format().is_code() && self.reflow_options.has('j')).then(|| self.reflow_options.profile());
        let snapshot = document.hard_line_snapshot();
        let mut comment_join = profile.as_ref().map(|profile|
            crate::document::CommentJoinContext::new(&snapshot, profile, self.reflow_options.language()));
        for index in first.index + 1..=last_index {
            let line = document.physical_line(index)?;
            let content = document.physical_source_text(line.content_range)?;
            cursor = joined.len();
            let value = if insert_space {
                if let Some(comment_join) = &mut comment_join { &content[comment_join.prefix(index - 1, &previous, &content)..] }
                else { content.trim_start_matches([' ', '\t']) }
            } else {
                content.as_str()
            };
            if insert_space
                && !joined.is_empty()
                && !joined.ends_with([' ', '\t'])
                && !value.is_empty()
                && !value.starts_with(')')
            {
                joined.push_str(options::join_separator(joined.chars().last(), value.chars().next(), self.vim_options.flag("joinspaces")));
            }
            joined.push_str(value);
            previous = content;
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
