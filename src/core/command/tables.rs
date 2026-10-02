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
        let anchor = table
            .rows
            .get(extent.anchor_row)
            .and_then(|row| row.cells.get(extent.anchor_column))
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let active = table
            .rows
            .get(extent.active_row)
            .and_then(|row| row.cells.get(extent.active_column))
            .ok_or(DocumentError::UnsupportedFormatting)?;
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
        self.visual_anchor = Some(anchor.range.start);
        self.cursor = active.range.end;
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
                    )
                ))
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
                    if let Err(output) = self.require_register_write(requested) {
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
            return self.insert_text(document, "\n").map(Some);
        }
        if (matches!(event, InputEvent::Key(Key::Backspace)) && self.cursor == range.start)
            || (matches!(event, InputEvent::Key(Key::Delete)) && self.cursor == range.end)
        {
            return Ok(Some(CommandOutput::complete()));
        }
        Ok(None)
    }
}
