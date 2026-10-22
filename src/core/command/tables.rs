//! Table navigation and semantic cell selections. Geometry never defines edits.
use super::*;
use crate::document::TableEditIntent;

#[derive(Clone, Debug)]
pub(crate) struct TableCellSelection {
    pub table: u64,
    pub anchor: u64,
    pub active: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableSelectionExtent {
    pub table: u64,
    pub anchor_row: usize,
    pub anchor_column: usize,
    pub active_row: usize,
    pub active_column: usize,
}
impl TableSelectionExtent {
    pub fn rows(&self) -> Range<usize> {
        self.anchor_row.min(self.active_row)..self.anchor_row.max(self.active_row) + 1
    }
    pub fn columns(&self) -> Range<usize> {
        self.anchor_column.min(self.active_column)..self.anchor_column.max(self.active_column) + 1
    }
}

impl CommandInterpreter {
    pub fn table_selection(&self, document: &Document) -> Option<TableSelectionExtent> {
        let selected = self.table_cells.as_ref()?;
        if document.format() != Format::Markdown {
            return None;
        }
        let table = document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == selected.table)?;
        let locate = |id, at| {
            let row = table
                .rows
                .partition_point(|row| row.range.start <= at)
                .checked_sub(1)?;
            let value = &table.rows[row];
            let column = value
                .cells
                .partition_point(|cell| cell.range.start <= at)
                .checked_sub(1)?;
            if value.cells[column].id == id {
                return Some((row, column));
            }
            // A structural action can move a surviving identity to a new
            // ordinal. Recovery scans only when the mapped endpoint changed
            // owners; ordinary paint/typing uses the logarithmic path above.
            table.rows.iter().enumerate().find_map(|(row, value)| {
                value
                    .cells
                    .iter()
                    .position(|cell| cell.id == id)
                    .map(|column| (row, column))
            })
        };
        let (anchor_row, anchor_column) = locate(selected.anchor, self.visual_anchor?)?;
        let (active_row, active_column) = locate(selected.active, self.cursor)?;
        Some(TableSelectionExtent {
            table: table.id,
            anchor_row,
            anchor_column,
            active_row,
            active_column,
        })
    }

    pub(crate) fn select_table_cells(
        &mut self,
        document: &Document,
        extent: &TableSelectionExtent,
    ) -> Result<(), DocumentError> {
        if document.format() != Format::Markdown {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let table = document
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == extent.table)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let cell = |row: usize, column: usize| table.rows.get(row).and_then(|value| value.cells.get(column))
            .ok_or(DocumentError::UnsupportedFormatting);
        let (mut anchor, mut active) = (cell(extent.anchor_row, extent.anchor_column)?,
            cell(extent.active_row, extent.active_column)?);
        // A rectangle of every cell selects the table itself, whichever pair
        // of opposite corners named it. Name it by its first and last cells
        // so that its offsets span exactly the table's text.
        if extent.rows() == (0..table.rows.len()) && extent.columns() == (0..table.columns.len()) {
            let (first, last) = (cell(0, 0)?, cell(table.rows.len() - 1, table.columns.len() - 1)?);
            (anchor, active) = if active.range.start < anchor.range.start { (last, first) } else { (first, last) };
        }
        let return_mode = if self.table_cells.is_some() {
            self.selection_return_mode
        } else {
            self.mode
        };
        self.clear_pending();
        self.retire_typing_context();
        self.invalidate_replace_restoration();
        self.table_cells = Some(TableCellSelection {
            table: table.id,
            anchor: anchor.id,
            active: active.id,
        });
        self.table_tab_selection = false;
        // The offsets span both edge cells in the rectangle's direction, so a
        // rectangle of every cell spans exactly the table's text.
        let backward = active.range.start < anchor.range.start;
        self.visual_anchor = Some(if backward { anchor.range.end } else { anchor.range.start });
        self.cursor = if backward { active.range.start } else { active.range.end };
        self.position_revision = Some(document.revision());
        self.mode = Mode::VisualCharacter;
        self.set_selection_origin(SelectionOrigin::Mouse);
        self.selection_exclusive = true;
        self.selection_return_mode = if matches!(return_mode, Mode::Insert | Mode::Replace) {
            return_mode
        } else {
            Mode::Normal
        };
        self.visual_position = None;
        self.visual_block = None;
        self.desired_x = None;
        Ok(())
    }

    pub(crate) fn retire_invalid_table_selection(&mut self, document: &Document) {
        if self.table_cells.is_some() && self.table_selection(document).is_none()
            || self.table_tab_selection
                && (document.format() != Format::Markdown
                    || document.projection().table_cell_at(self.cursor).is_none())
        {
            self.retire_table_selection(document);
        }
    }

    pub(crate) fn retire_table_selection(&mut self, document: &Document) {
        if self.table_cells.take().is_none() && !self.table_tab_selection {
            return;
        }
        self.table_tab_selection = false;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.select_visual_once = false;
        self.select_visual_just_started = false;
        self.clear_pending();
        self.retire_typing_context();
        self.mode = self.selection_return_mode;
        self.position_revision = Some(document.revision());
        self.visual_position = None;
        self.desired_x = None;
    }

    /// A Cell selection of every cell selects the table itself. Deleting or
    /// replacing it removes the table, so hand the ordinary text-selection
    /// path exactly the table's text.
    fn select_whole_table_text(&mut self, document: &Document, extent: &TableSelectionExtent) -> bool {
        let Some(table) = document.projection().tables().iter().find(|table| table.id == extent.table) else {
            return false;
        };
        if extent.rows() != (0..table.rows.len()) || extent.columns() != (0..table.columns.len()) {
            return false;
        }
        // The rectangle's offsets already span the table in its direction.
        self.table_cells = None;
        true
    }

    pub(crate) fn finish_native_table_edit(
        &mut self,
        document: &mut Document,
        caret: usize,
        insert: bool,
    ) {
        self.table_cells = None;
        self.table_tab_selection = false;
        self.visual_anchor = None;
        self.visual_block = None;
        self.active_visual_block = None;
        self.clear_pending();
        self.cursor = caret;
        self.position_revision = Some(document.revision());
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.visual_position = None;
        self.desired_x = None;
        if insert {
            if !matches!(self.mode, Mode::Insert | Mode::Replace) || self.insert_session.is_none() {
                self.enter_insert(document, InsertPlacement::Before, 1);
            }
            self.mode = Mode::Insert;
        } else if matches!(
            self.mode,
            Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.mode = Mode::Normal;
        }
    }

    pub(crate) fn table_input_requires_legacy(
        &self,
        document: &Document,
        event: &InputEvent,
    ) -> bool {
        self.table_cells.is_some()
            || self.table_tab_selection
            || self.table_edge_key_is_no_op(document, event)
            || (document.format() == Format::Markdown
                && self.mode == Mode::Normal
                && matches!(event, InputEvent::Key(Key::Char('p' | 'P')))
                && document.projection().table_cell_at(self.cursor).is_some())
            || (document.format() == Format::Markdown
                && matches!(self.mode, Mode::Insert | Mode::Replace)
                && document.projection().table_cell_at(self.cursor).is_some()
                && matches!(
                    event,
                    InputEvent::Key(
                        Key::Tab
                            | Key::BackTab
                            | Key::Enter
                            | Key::ShiftEnter
                            | Key::Backspace
                            | Key::Delete
                            | Key::Ctrl('w' | 'W' | 'u' | 'U')
                    )
                ))
    }

    fn table_edge_key_is_no_op(&self, document: &Document, event: &InputEvent) -> bool {
        if document.format() != Format::Markdown
            || !matches!(self.mode, Mode::Insert | Mode::Replace)
        {
            return false;
        }
        let lines = document.hard_line_snapshot();
        match event {
            InputEvent::Key(Key::Backspace | Key::Ctrl('w' | 'W' | 'u' | 'U'))
                if self.mode == Mode::Insert =>
            {
                lines
                    .previous_grapheme_boundary(self.cursor)
                    .and_then(|at| document.projection().table_at(at).map(|table| (at, table)))
                    .is_some_and(|(at, table)| at == table.range.end && self.cursor > at)
                    // Backspace may remove list/code treatment without
                    // joining the preceding table. Let that semantic action
                    // run before protecting the inter-owner separator.
                    && (!matches!(event, InputEvent::Key(Key::Backspace))
                        || matches!(self.paragraph_key_request(document, Key::Backspace), Ok(None)))
            }
            InputEvent::Key(Key::Delete) => lines
                .next_grapheme_boundary(self.cursor)
                .and_then(|at| document.projection().table_at(at).map(|table| (at, table)))
                .is_some_and(|(at, table)| at == table.range.start && self.cursor < at),
            _ => false,
        }
    }

    /// Private matrix payloads retain their rectangular meaning. Plain text,
    /// including tabs and newlines, continues through ordinary one-cell input.
    pub(super) fn paste_table_matrix(
        &mut self,
        document: &mut Document,
        value: &RegisterValue,
        count: usize,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if document.format() != Format::Markdown {
            return Ok(None);
        }
        let Some(cells) = value
            .clipboard_fragment()
            .and_then(|fragment| fragment.table_cells())
        else {
            return Ok(None);
        };
        let target = if let Some(extent) = self.table_selection(document) {
            if cells.len() != extent.rows().len() || cells[0].len() != extent.columns().len() {
                return Ok(Some(CommandOutput {
                    status: CommandStatus::Error(format!(
                        "Cannot paste {} × {} cells into a {} × {} selection.",
                        cells[0].len(),
                        cells.len(),
                        extent.columns().len(),
                        extent.rows().len()
                    )),
                    ..CommandOutput::complete()
                }));
            }
            Some((extent.table, extent.rows().start, extent.columns().start))
        } else {
            document
                .projection()
                .table_cell_at(self.cursor)
                .map(|(table, row, cell)| {
                    (
                        table.id,
                        table
                            .rows
                            .partition_point(|candidate| candidate.range.start < row.range.start),
                        row.cells
                            .partition_point(|candidate| candidate.range.start < cell.range.start),
                    )
                })
        };
        let Some((table, row, column)) = target else {
            return Ok(None);
        };
        if count != 1 {
            return Ok(Some(CommandOutput::unsupported(
                "Counted table matrix paste is not supported. Paste the matrix once.",
            )));
        }
        let insert = matches!(self.mode, Mode::Insert | Mode::Replace)
            || self.is_text_selection()
            || self.select_visual_once;
        let (prepared, caret) = document
            .prepare_table_edit(
                document.id(),
                document.revision(),
                TableEditIntent::PasteCells {
                    table,
                    row,
                    column,
                    cells,
                },
            )
            .map_err(command_document_error)?;
        document
            .commit_model_transaction(prepared)
            .map_err(command_document_error)?;
        self.finish_native_table_edit(document, caret, insert);
        Ok(Some(CommandOutput {
            document_changed: true,
            cursor_moved: true,
            mode_changed: true,
            ..CommandOutput::complete()
        }))
    }

    pub(crate) fn handle_table_input(
        &mut self,
        document: &mut Document,
        event: &InputEvent,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if self.literal_input_pending() || self.register_pending || self.pending != Pending::None {
            return Ok(None);
        }
        if let Some(extent) = self.table_selection(document) {
            if matches!(event, InputEvent::Key(Key::Escape | Key::Ctrl('['))) {
                self.table_cells = None;
                self.visual_anchor = None;
                self.mode = self.selection_return_mode;
                return Ok(Some(CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::complete()
                }));
            }
            // Native Cut/Paste use one Visual command with an explicit register.
            // Let the ordinary prefix parser retain its register and Ctrl-O grammar.
            if matches!(event, InputEvent::Key(Key::Ctrl('o'))) {
                return Ok(None);
            }
            if self.select_visual_once
                || self.selection_behavior == SelectionBehavior::Visual
                    && self.selection_return_mode == Mode::Normal
            {
                if matches!(event, InputEvent::Key(Key::Char('"'))) {
                    return Ok(None);
                }
                if let InputEvent::Key(Key::Char(digit @ '0'..='9')) = event {
                    if *digit != '0' || self.count.is_some() {
                        return Ok(Some(self.push_count(*digit)));
                    }
                }
                if let InputEvent::Key(Key::Char(operation @ ('d' | 'x' | 'y' | 'p' | 'P'))) = event
                {
                    let requested = self.requested_register;
                    if matches!(operation, 'p' | 'P') {
                        let value =
                            match self.require_register_value(document, requested.unwrap_or('"')) {
                                Ok(value) => value,
                                Err(output) => return Ok(Some(output)),
                            };
                        if let Some(output) =
                            self.paste_table_matrix(document, &value, self.count.unwrap_or(1))?
                        {
                            return Ok(Some(output));
                        }
                        let (prepared, caret) = document
                            .prepare_table_edit(
                                document.id(),
                                document.revision(),
                                TableEditIntent::ReplaceCells {
                                    table: extent.table,
                                    rows: extent.rows(),
                                    columns: extent.columns(),
                                    text: value.text.clone(),
                                    anchor_row: extent.anchor_row,
                                    anchor_column: extent.anchor_column,
                                },
                            )
                            .map_err(command_document_error)?;
                        document
                            .commit_model_transaction(prepared)
                            .map_err(command_document_error)?;
                        self.finish_native_table_edit(document, caret, true);
                        return Ok(Some(CommandOutput {
                            document_changed: true,
                            cursor_moved: true,
                            mode_changed: true,
                            ..CommandOutput::complete()
                        }));
                    }
                    if *operation != 'y' && self.select_whole_table_text(document, &extent) {
                        return Ok(None);
                    }
                    if let Err(output) = self.require_register_write_kind(requested, *operation == 'y') {
                        return Ok(Some(output));
                    }
                    let (fragment, _) = document.table_clipboard_fragment(
                        extent.table,
                        extent.rows(),
                        extent.columns(),
                    )?;
                    let value = RegisterValue::from_clipboard_fragment(fragment)
                        .map_err(|_| DocumentError::UnsupportedFormatting)?;
                    if *operation == 'y' {
                        self.yank_register(requested, value);
                        if let Some(table) = document.projection().tables().iter().find(|table| table.id == extent.table) {
                            let start = table.rows[extent.rows().start].cells[extent.columns().start].range.start;
                            let end = table.rows[extent.rows().end - 1].cells[extent.columns().end - 1].range.end;
                            self.note_affected_range(document, start..end, false, false);
                        }
                        self.requested_register = None;
                        return Ok(Some(CommandOutput::complete()));
                    }
                    let (prepared, caret) = document
                        .prepare_table_edit(
                            document.id(),
                            document.revision(),
                            TableEditIntent::ClearCells {
                                table: extent.table,
                                rows: extent.rows(),
                                columns: extent.columns(),
                            },
                        )
                        .map_err(command_document_error)?;
                    document
                        .commit_model_transaction(prepared)
                        .map_err(command_document_error)?;
                    // Register publication follows successful source verification.
                    self.delete_register(requested, value, DeletionClass::Large);
                    self.finish_native_table_edit(
                        document,
                        caret,
                        self.selection_return_mode == Mode::Insert,
                    );
                    return Ok(Some(CommandOutput {
                        document_changed: true,
                        cursor_moved: true,
                        mode_changed: true,
                        ..CommandOutput::complete()
                    }));
                }
                return Ok(Some(CommandOutput::unsupported(
                    "This command is unavailable for a cell selection.",
                )));
            }
            let replacement = match event {
                InputEvent::Text(text) => Some(text.clone()),
                InputEvent::Key(Key::Char(character)) => Some(character.to_string()),
                InputEvent::Key(Key::Enter | Key::ShiftEnter) => Some("\n".to_owned()),
                _ => None,
            };
            if replacement.is_some()
                || matches!(event, InputEvent::Key(Key::Delete | Key::Backspace))
            {
                if self.select_whole_table_text(document, &extent) {
                    return Ok(None);
                }
                let intent = match replacement.as_ref() {
                    Some(text) => TableEditIntent::ReplaceCells {
                        table: extent.table,
                        rows: extent.rows(),
                        columns: extent.columns(),
                        text: text.clone(),
                        anchor_row: extent.anchor_row,
                        anchor_column: extent.anchor_column,
                    },
                    None => TableEditIntent::ClearCells {
                        table: extent.table,
                        rows: extent.rows(),
                        columns: extent.columns(),
                    },
                };
                let (prepared, caret) = document
                    .prepare_table_edit(document.id(), document.revision(), intent)
                    .map_err(command_document_error)?;
                document
                    .commit_model_transaction(prepared)
                    .map_err(command_document_error)?;
                self.finish_native_table_edit(document, caret, true);
                return Ok(Some(CommandOutput {
                    document_changed: true,
                    cursor_moved: true,
                    mode_changed: true,
                    ..CommandOutput::complete()
                }));
            }
            if matches!(event, InputEvent::Key(Key::Escape | Key::Ctrl('['))) {
                self.table_cells = None;
                self.visual_anchor = None;
                self.mode = self.selection_return_mode;
                return Ok(Some(CommandOutput {
                    mode_changed: true,
                    ..CommandOutput::complete()
                }));
            }
            if matches!(
                event,
                InputEvent::Key(
                    Key::Left
                        | Key::Right
                        | Key::Up
                        | Key::Down
                        | Key::Home
                        | Key::End
                        | Key::DocumentStart
                        | Key::DocumentEnd
                        | Key::SelectAll
                )
            ) {
                self.table_cells = None;
                self.visual_anchor = None;
                self.mode = self.selection_return_mode;
                return Ok(None);
            }
            return Ok(Some(CommandOutput::unsupported(
                "This command is unavailable for a cell selection.",
            )));
        } else if self.table_cells.is_some() {
            self.table_cells = None;
            self.visual_anchor = None;
            self.mode = self.selection_return_mode;
        }
        if document.format() != Format::Markdown {
            self.table_tab_selection = false;
            return Ok(None);
        }
        let tab = matches!(event, InputEvent::Key(Key::Tab | Key::BackTab));
        if self.table_tab_selection && !tab {
            self.table_tab_selection = false;
        }
        if !matches!(self.mode, Mode::Insert | Mode::Replace) && !self.table_tab_selection {
            return Ok(None);
        }
        // Paragraph/table separators are structural, just like separators
        // between cells. A single editing key cannot merge the owners from
        // either side; selections and row actions retain their own policies.
        if self.table_edge_key_is_no_op(document, event) {
            return Ok(Some(CommandOutput::complete()));
        }
        let Some((table, row, cell)) = document.projection().table_cell_at(self.cursor) else {
            return Ok(None);
        };
        let table_id = table.id;
        let row_index = table
            .rows
            .partition_point(|candidate| candidate.range.start < row.range.start);
        let column = row
            .cells
            .partition_point(|candidate| candidate.range.start < cell.range.start);
        let range = cell.range.clone();
        if tab {
            let columns = table.columns.len();
            let backwards = matches!(event, InputEvent::Key(Key::BackTab));
            let ordinal = row_index * columns + column;
            if backwards && ordinal == 0 {
                return Ok(Some(CommandOutput::complete()));
            }
            let target = if backwards { ordinal - 1 } else { ordinal + 1 };
            let mut changed = false;
            let append_row = target >= table.rows.len() * columns;
            document.end_edit_group();
            document.begin_edit_group();
            if append_row {
                let (prepared, _) = document
                    .prepare_table_edit(
                        document.id(),
                        document.revision(),
                        TableEditIntent::InsertRow {
                            table: table_id,
                            row: row_index,
                            after: true,
                        },
                    )
                    .map_err(command_document_error)?;
                document
                    .commit_model_transaction(prepared)
                    .map_err(command_document_error)?;
                changed = true;
            }
            let table = document
                .projection()
                .tables()
                .iter()
                .find(|value| value.id == table_id)
                .or_else(|| document.projection().table_at(range.start))
                .ok_or(DocumentError::UnsupportedFormatting)?;
            let next = &table.rows[target / columns].cells[target % columns];
            let next_range = next.range.clone();
            self.retire_typing_context();
            self.invalidate_replace_restoration();
            self.table_tab_selection = !next_range.is_empty();
            self.selection_return_mode = Mode::Insert;
            self.mode = if next_range.is_empty() {
                Mode::Insert
            } else {
                Mode::VisualCharacter
            };
            self.selection_behavior = SelectionBehavior::Native;
            self.selection_exclusive = true;
            self.visual_anchor = (!next_range.is_empty()).then_some(next_range.start);
            self.cursor = next_range.end;
            self.position_revision = Some(document.revision());
            self.boundary_affinity = BoundaryAffinity::Downstream;
            if let Some(session) = self.insert_session.as_mut() {
                session.unit_floor = self.cursor;
            }
            return Ok(Some(CommandOutput {
                document_changed: changed,
                cursor_moved: true,
                mode_changed: true,
                ..CommandOutput::complete()
            }));
        }
        if matches!(event, InputEvent::Key(Key::Enter | Key::ShiftEnter)) {
            if self.mode == Mode::Replace {
                return self.insert_indented_break(document).map(Some);
            }
            return self.insert_text(document, "\n").map(Some);
        }
        if (matches!(
            event,
            InputEvent::Key(Key::Backspace | Key::Ctrl('w' | 'W' | 'u' | 'U'))
        ) && self.cursor == range.start)
            || (matches!(event, InputEvent::Key(Key::Delete)) && self.cursor == range.end)
        {
            return Ok(Some(CommandOutput::complete()));
        }
        Ok(None)
    }
}
