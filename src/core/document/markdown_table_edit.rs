//! Verified table source transactions. Structural edits patch only affected
//! rows/cells; original source remains the serialization authority.
use super::*;
use crate::document::{MarkdownTable, TableAlignment, TableEditIntent};

fn unsupported(reason: &'static str) -> ModelTransactionError {
    DocumentError::UnsupportedTableEdit(reason).into()
}
impl Document {
    pub fn can_insert_table(&self, range: Range<usize>) -> bool {
        if !self.format().is_markdown() || self.validate_range(&range).is_err() {
            return false;
        }
        if self
            .projection()
            .tables()
            .iter()
            .any(|table| table.range.start <= range.end && range.start <= table.range.end)
        {
            return false;
        }
        let blocks = self.projection().blocks_for_region(&range);
        if blocks
            .iter()
            .any(|block| block.style.0 == "Code Block" || block.markdown_html)
        {
            return false;
        }
        if let Some(first) = blocks.first() {
            if blocks.iter().any(|block| {
                block.containers != first.containers || block.quote_depth != first.quote_depth
            }) {
                return false;
            }
        }
        !self
            .projection()
            .style_spans_for_region(&range)
            .iter()
            .any(|span| {
                span.range.start <= range.start
                    && range.start < span.range.end
                    && matches!(
                        span.application,
                        StyleApplication::Semantic(SemanticInlineStyle::Code)
                    )
            })
    }
    /// Prepare one table action and its exact caret in the candidate snapshot.
    pub fn prepare_table_edit(
        &self,
        document: DocumentId,
        revision: Revision,
        intent: TableEditIntent,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if document != self.id() {
            return Err(ModelTransactionError::WrongDocument {
                expected: document,
                actual: self.id(),
            });
        }
        if revision != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: revision,
                actual: self.revision(),
            });
        }
        if !self.format().is_markdown() {
            return Err(unsupported("Tables are available in Markdown documents."));
        }
        if let TableEditIntent::Insert {
            range,
            columns,
            body_rows,
        } = intent
        {
            return self.prepare_table_insertion(range, columns, body_rows);
        }
        if let TableEditIntent::PasteCells {
            table,
            row,
            column,
            cells,
        } = intent
        {
            return self.prepare_table_matrix_paste(table, row, column, cells);
        }
        let id = match &intent {
            TableEditIntent::InsertRow { table, .. }
            | TableEditIntent::DeleteRow { table, .. }
            | TableEditIntent::InsertColumn { table, .. }
            | TableEditIntent::DeleteColumn { table, .. }
            | TableEditIntent::SetAlignment { table, .. }
            | TableEditIntent::ClearCells { table, .. }
            | TableEditIntent::ReplaceCells { table, .. }
            | TableEditIntent::SetCellsSemanticStyle { table, .. }
            | TableEditIntent::SetCellsStrikethrough { table, .. }
            | TableEditIntent::AssignCellsNamedStyle { table, .. } => *table,
            _ => unreachable!(),
        };
        let table = self
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == id)
            .ok_or_else(|| unsupported("The table changed. Choose the table action again."))?;
        let mut patches = Vec::new();
        let mut target_row = 0;
        let mut target_column = 0;
        let mut expected_rows = table.rows.len();
        let mut expected_columns = table.columns.len();
        let mut remove = false;
        let encode = |value: &str| {
            self.encoding()
                .encode_fragment(value)
                .map_err(ModelTransactionError::from)
        };
        let newline = self.file_format().spelling();
        match intent.clone() {
            TableEditIntent::InsertRow { row, after, .. } => {
                if row >= table.rows.len() || row == 0 && !after {
                    return Err(unsupported("A body row can be inserted below the header."));
                }
                let index = row + usize::from(after);
                target_row = index;
                let source = if row == 0 {
                    table.source_rows[1].source_range.end
                } else if after {
                    table.rows[row].source_range.end
                } else {
                    table.rows[row].source_range.start
                };
                let prototype = &table.source_rows[if row == 0 { 1 } else { row + 1 }];
                let prefix = self
                    .table_source_text(prototype.source_range.start..prototype.source_body.start)?;
                let mut body = format!("{prefix}|{}", "  |".repeat(table.columns.len()));
                let previous = self.table_source_text(if row == 0 {
                    table.source_rows[1].source_range.clone()
                } else {
                    table.rows[row].source_range.clone()
                })?;
                if after && !previous.ends_with(['\n', '\r']) {
                    body = format!("{newline}{body}");
                } else {
                    body.push_str(newline);
                }
                patches.push(SourcePatch::primary(source..source, encode(&body)?));
                expected_rows += 1;
            }
            TableEditIntent::DeleteRow { row, .. } => {
                if row >= table.rows.len() {
                    return Err(unsupported("The selected table row no longer exists."));
                }
                if table.rows.len() == 1 {
                    remove = true;
                } else if row == 0 {
                    let promoted = &table.source_rows[2];
                    if promoted.source_cells.len() > table.columns.len() {
                        return Err(unsupported("This row has extra source cells and cannot be promoted to the header without losing them."));
                    }
                    let header = &table.source_rows[0];
                    let mut replacement = self
                        .table_source_text(header.source_range.start..header.source_body.start)?;
                    replacement.push_str(&self.table_source_text(
                        promoted.source_body.start..promoted.source_range.end,
                    )?);
                    if promoted.source_cells.len() < table.columns.len() {
                        let tail = self.table_source_text(
                            promoted.source_cells.last().unwrap().end..promoted.source_range.end,
                        )?;
                        let cut = replacement.len() - tail.len();
                        let outer = if tail.contains('|') { "" } else { " |" };
                        replacement.insert_str(
                            cut,
                            &format!(
                                "{}{outer}",
                                " | ".repeat(table.columns.len() - promoted.source_cells.len())
                            ),
                        );
                    }
                    if !replacement.ends_with(['\n', '\r']) {
                        replacement.push_str(newline);
                    }
                    patches.push(SourcePatch::primary(
                        table.rows[0].source_range.clone(),
                        encode(&replacement)?,
                    ));
                    patches.push(SourcePatch::primary(
                        table.rows[1].source_range.clone(),
                        Vec::new(),
                    ));
                } else {
                    patches.push(SourcePatch::primary(
                        table.rows[row].source_range.clone(),
                        Vec::new(),
                    ));
                }
                expected_rows -= 1;
                target_row = row.min(expected_rows.saturating_sub(1));
            }
            TableEditIntent::InsertColumn { column, after, .. } => {
                if column >= table.columns.len() {
                    return Err(unsupported("The selected table column no longer exists."));
                }
                let index = column + usize::from(after);
                target_column = index;
                for row in &table.source_rows {
                    let spelling = if row.delimiter {
                        delimiter(table.columns[column])
                    } else {
                        " ".to_owned()
                    };
                    if let Some(cell) = row.source_cells.get(index) {
                        patches.push(SourcePatch::primary(
                            cell.start..cell.start,
                            encode(&format!(" {spelling} |"))?,
                        ));
                    } else {
                        let at = row
                            .source_cells
                            .last()
                            .map_or(row.source_body.end, |cell| cell.end);
                        let missing = index.saturating_sub(row.source_cells.len());
                        let outer = if self
                            .table_source_text(at..row.source_body.end)?
                            .contains('|')
                        {
                            ""
                        } else {
                            " |"
                        };
                        patches.push(SourcePatch::primary(
                            at..at,
                            encode(&format!("{} | {spelling} {outer}", " | ".repeat(missing)))?,
                        ));
                    }
                }
                expected_columns += 1;
            }
            TableEditIntent::DeleteColumn { column, .. } => {
                if column >= table.columns.len() {
                    return Err(unsupported("The selected table column no longer exists."));
                }
                if table.columns.len() == 1 {
                    remove = true;
                } else {
                    for row in &table.source_rows {
                        if let Some(cell) = row.source_cells.get(column) {
                            let range = if column == 0 {
                                cell.start
                                    ..row.source_cells.get(1).map_or(cell.end, |next| next.start)
                            } else {
                                row.source_cells[column - 1].end..cell.end
                            };
                            patches.push(SourcePatch::primary(range, Vec::new()));
                        }
                    }
                }
                expected_columns -= 1;
                target_column = column.min(expected_columns.saturating_sub(1));
            }
            TableEditIntent::SetAlignment {
                column, alignment, ..
            } => {
                if table.columns.get(column) == Some(&alignment) {
                    return Ok((
                        self.no_op_prepared(),
                        table.rows[0].cells[column].range.start,
                    ));
                }
                let row = &table.source_rows[1];
                let range = row
                    .source_cells
                    .get(column)
                    .ok_or_else(|| unsupported("The selected table column no longer exists."))?;
                let raw = self.table_source_text(range.clone())?;
                let leading = raw.len() - raw.trim_start_matches([' ', '\t']).len();
                let trailing = raw.len() - raw.trim_end_matches([' ', '\t']).len();
                let body = raw.trim_matches([' ', '\t']);
                let dashes = body.bytes().filter(|b| *b == b'-').count().max(1);
                let marker = match alignment {
                    TableAlignment::Unspecified => "-".repeat(dashes),
                    TableAlignment::Left => format!(":{}", "-".repeat(dashes)),
                    TableAlignment::Center => format!(":{}:", "-".repeat(dashes)),
                    TableAlignment::Right => format!("{}:", "-".repeat(dashes)),
                };
                patches.push(SourcePatch::primary(
                    range.clone(),
                    encode(&format!(
                        "{}{}{}",
                        &raw[..leading],
                        marker,
                        &raw[raw.len() - trailing..]
                    ))?,
                ));
                target_column = column;
            }
            TableEditIntent::ClearCells { rows, columns, .. }
            | TableEditIntent::ReplaceCells {
                rows,
                columns,
                text: _,
                ..
            }
            | TableEditIntent::SetCellsSemanticStyle { rows, columns, .. }
            | TableEditIntent::SetCellsStrikethrough { rows, columns, .. }
            | TableEditIntent::AssignCellsNamedStyle { rows, columns, .. } => {
                if rows.is_empty()
                    || columns.is_empty()
                    || rows.end > table.rows.len()
                    || columns.end > table.columns.len()
                {
                    return Err(unsupported("The cell selection changed."));
                }
                target_row = rows.start;
                target_column = columns.start;
                if let TableEditIntent::ReplaceCells {
                    anchor_row,
                    anchor_column,
                    ..
                } = &intent
                {
                    if !rows.contains(anchor_row) || !columns.contains(anchor_column) {
                        return Err(unsupported(
                            "The replacement anchor is outside the cell selection.",
                        ));
                    }
                    target_row = *anchor_row;
                    target_column = *anchor_column;
                }
                for row_index in rows.clone() {
                    for column in columns.clone() {
                        let cell = &table.rows[row_index].cells[column];
                        let replacement = match &intent {
                            TableEditIntent::ReplaceCells {
                                text,
                                anchor_row,
                                anchor_column,
                                ..
                            } if row_index == *anchor_row && column == *anchor_column => {
                                Some(self.table_literal_text(text))
                            }
                            TableEditIntent::SetCellsSemanticStyle { style, enabled, .. } => {
                                if cell.range.is_empty() {
                                    continue;
                                }
                                let prepared = self.prepare_semantic_style(
                                    cell.range.clone(),
                                    *style,
                                    *enabled,
                                )?;
                                patches.extend(prepared.summary.source_patches);
                                None
                            }
                            TableEditIntent::SetCellsStrikethrough { enabled, .. } => {
                                if cell.range.is_empty() {
                                    continue;
                                }
                                let prepared =
                                    self.prepare_strikethrough(cell.range.clone(), *enabled)?;
                                patches.extend(prepared.summary.source_patches);
                                None
                            }
                            TableEditIntent::AssignCellsNamedStyle { style, .. } => {
                                if cell.range.is_empty() {
                                    continue;
                                }
                                let prepared = self.prepare_character_style_choice(
                                    cell.range.clone(),
                                    style.clone(),
                                )?;
                                patches.extend(prepared.summary.source_patches);
                                None
                            }
                            _ => Some(String::new()),
                        };
                        if let Some(value) = replacement {
                            if !cell.range.is_empty() || !value.is_empty() {
                                patches.extend(
                                    self.table_cell_replacement(table, row_index, column, &value)?,
                                );
                            }
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        if remove {
            patches = vec![SourcePatch::primary(table.source_range.clone(), Vec::new())];
        }
        let prepared = self.prepare_reprojected_source_patches(patches)?;
        let candidate = match &prepared.publication {
            PreparedPublication::State(state) => &state.projection,
            _ => {
                return Ok((
                    prepared,
                    table.rows[target_row].cells[target_column].range.start,
                ))
            }
        };
        let caret = if remove {
            table.range.start.min(candidate.text_tree().byte_len())
        } else {
            let candidate_table = candidate
                .tables()
                .iter()
                .find(|candidate| candidate.source_range.start == table.source_range.start)
                .ok_or(DocumentError::VerificationFailed)?;
            if candidate_table.rows.len() != expected_rows
                || candidate_table.columns.len() != expected_columns
            {
                return Err(DocumentError::VerificationFailed.into());
            }
            let cell = &candidate_table.rows[target_row].cells[target_column];
            if matches!(intent, TableEditIntent::ReplaceCells { .. }) {
                cell.range.end
            } else {
                cell.range.start
            }
        };
        Ok((prepared, caret))
    }
    pub(super) fn prepare_table_line_deletion(
        &self,
        range: &Range<usize>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.format() != Format::Markdown {
            return Ok(None);
        }
        let Some(table) = self
            .projection()
            .tables()
            .iter()
            .find(|table| range.start < table.range.end && table.range.start < range.end)
        else {
            return Ok(None);
        };
        if range.start < table.range.start
            || range.end
                > table.range.end
                    + usize::from(self.text().as_bytes().get(table.range.end) == Some(&b'\n'))
        {
            return Err(unsupported(
                "This line deletion crosses table and paragraph boundaries.",
            ));
        }
        if range.start <= table.range.start && range.end >= table.range.end {
            return self
                .prepare_reprojected_source_patches(vec![SourcePatch::primary(
                    table.source_range.clone(),
                    Vec::new(),
                )])
                .map(Some);
        }
        let table_index = self
            .projection()
            .tables()
            .iter()
            .position(|candidate| candidate.id == table.id)
            .unwrap();
        let first = table
            .rows
            .partition_point(|row| row.range.end <= range.start);
        let last = table
            .rows
            .partition_point(|row| row.range.start < range.end);
        let mut full_rows = Vec::new();
        let mut partial = Vec::new();
        for index in first..last {
            let row = &table.rows[index];
            if range.start <= row.range.start && range.end >= row.range.end {
                full_rows.push(index);
                continue;
            }
            let first_cell = row
                .cells
                .partition_point(|cell| cell.range.end <= range.start);
            for cell in row.cells[first_cell..]
                .iter()
                .take_while(|cell| cell.range.start < range.end)
            {
                let selected = range.start.max(cell.range.start)..range.end.min(cell.range.end);
                if !selected.is_empty() {
                    partial.push(selected);
                }
            }
        }
        let mut scratch = self.scratch_document();
        let mut composition = super::replacement::PatchComposition::new(self.source_byte_len());
        let mut apply = |scratch: &mut Document,
                         prepared: PreparedModelTransaction|
         -> Result<(), ModelTransactionError> {
            for patch in prepared.summary.source_patches.iter().rev() {
                composition.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };
        // Later source/text ranges first; earlier immutable offsets stay valid.
        for selected in partial.into_iter().rev() {
            let prepared = scratch.prepare_text_edits(vec![TextEdit::new(selected, "")])?;
            apply(&mut scratch, prepared)?;
        }
        for row in full_rows.into_iter().rev() {
            let table = scratch.projection().tables()[table_index].id;
            let (prepared, _) = scratch.prepare_table_edit(
                scratch.id(),
                scratch.revision(),
                TableEditIntent::DeleteRow { table, row },
            )?;
            apply(&mut scratch, prepared)?;
        }
        self.prepare_reprojected_source_patches(composition.source_patches())
            .map(Some)
    }
    pub(super) fn table_style_fragments(
        &self,
        range: &Range<usize>,
    ) -> Result<Vec<Range<usize>>, ModelTransactionError> {
        let text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let mut boundaries = vec![range.start, range.end];
        for span in self.projection().style_spans_for_region(&range) {
            boundaries.push(span.range.start.max(range.start));
            boundaries.push(span.range.end.min(range.end));
        }
        for (at, ch) in text.char_indices() {
            if ch == '\n' {
                boundaries.push(range.start + at);
                boundaries.push(range.start + at + 1);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        Ok(boundaries
            .windows(2)
            .filter_map(|pair| {
                let part = pair[0]..pair[1];
                (!part.is_empty()
                    && &text[part.start - range.start..part.end - range.start] != "\n")
                    .then_some(part)
            })
            .collect())
    }
    pub(super) fn prepare_table_semantic_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        // Split at inline scope boundaries and internal breaks. Each fragment
        // has a uniform inherited context, so inserting/removing its trait
        // cannot cross another Markdown scope or consume a cell separator.
        let fragments = self.table_style_fragments(&range)?;
        let mut scratch = self.scratch_document();
        let mut composition = super::replacement::PatchComposition::new(self.source_byte_len());
        for selected in fragments {
            let prepared = scratch.prepare_typing_markdown_style(selected, style, enabled)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                composition.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
        }
        self.prepare_source_only_patches(composition.source_patches())
    }
    pub(super) fn table_source_text(
        &self,
        range: Range<usize>,
    ) -> Result<String, ModelTransactionError> {
        let bytes = self
            .state()
            .source
            .bytes_in(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        Ok(self.encoding().decode_region(&bytes, range.start)?.text)
    }
    fn table_literal_text(&self, text: &str) -> String {
        text.split('\n')
            .map(|line| {
                let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
                let trailing = line.trim_end_matches([' ', '\t']).len().max(leading);
                let spaces = |value: &str| {
                    value
                        .chars()
                        .map(|ch| if ch == ' ' { "&#32;" } else { "&#9;" })
                        .collect::<String>()
                };
                format!(
                    "{}{}{}",
                    spaces(&line[..leading]),
                    escape_markdown_insert_in_encoding(&line[leading..trailing], self.encoding())
                        .replace('|', "\\|"),
                    spaces(&line[trailing..])
                )
            })
            .collect::<Vec<_>>()
            .join("<br>")
    }
    fn table_cell_replacement(
        &self,
        table: &MarkdownTable,
        row: usize,
        column: usize,
        value: &str,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let cell = &table.rows[row].cells[column];
        let source_row = &table.source_rows[if row == 0 { 0 } else { row + 1 }];
        if !cell.missing {
            let first = column == 0
                && !self
                    .table_source_text(source_row.source_body.start..cell.source_range.start)?
                    .contains('|');
            let last = column + 1 == source_row.source_cells.len()
                && !self
                    .table_source_text(cell.source_range.end..source_row.source_body.end)?
                    .contains('|');
            let value = format!(
                "{}{}{}",
                if first { "| " } else { "" },
                value,
                if last { " |" } else { "" }
            );
            return Ok(vec![SourcePatch::primary(
                cell.source_range.clone(),
                self.encoding().encode_fragment(&value)?,
            )]);
        }
        let at = source_row
            .source_cells
            .last()
            .map_or(source_row.source_body.end, |cell| cell.end);
        let missing = column.saturating_sub(source_row.source_cells.len());
        let outer = if self
            .table_source_text(at..source_row.source_body.end)?
            .contains('|')
        {
            ""
        } else {
            " |"
        };
        Ok(vec![SourcePatch::primary(
            at..at,
            self.encoding()
                .encode_fragment(&format!("{} | {value} {outer}", " | ".repeat(missing)))?,
        )])
    }
    fn prepare_table_insertion(
        &self,
        range: Range<usize>,
        columns: usize,
        body_rows: usize,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        self.validate_range(&range)?;
        if columns == 0 || columns > 20 || body_rows == 0 || body_rows > 50 {
            return Err(unsupported("Choose 1–20 columns and 1–50 body rows."));
        }
        if !self.can_insert_table(range.clone()) {
            return Err(unsupported(
                "A table cannot be inserted inside another table or literal code.",
            ));
        }
        let mut source = if range.is_empty() {
            let at = self
                .projection()
                .source_insertion_point(range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            at..at
        } else {
            self.projection()
                .source_range(range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let mut scopes = Vec::new();
        if self.format() == Format::Markdown && range.is_empty() {
            for span in self.projection().style_spans_for_region(
                &(range.start.saturating_sub(1)
                    ..range.end.saturating_add(1).min(self.text().len())),
            ) {
                if !(span.range.start <= range.start && range.end <= span.range.end) {
                    continue;
                }
                let content = self
                    .projection()
                    .source_range(span.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let patches = match span.application {
                    StyleApplication::Semantic(
                        style @ (SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis),
                    ) => self.markdown_style_removal_patches(&content, style)?,
                    StyleApplication::Automatic(ref name) if name.0 == "Strikethrough" => {
                        self.markdown_strike_removal_patches(&content)?
                    }
                    _ => continue,
                };
                if range.start == span.range.start {
                    source.start = source.start.min(patches[0].range.start);
                    source.end = source.start;
                    continue;
                }
                if range.start == span.range.end {
                    source.end = source.end.max(patches[1].range.end);
                    source.start = source.end;
                    continue;
                }
                scopes.push((
                    patches[0].range.start,
                    self.table_source_text(patches[0].range.clone())?,
                    self.table_source_text(patches[1].range.clone())?,
                ));
            }
            scopes.sort_by_key(|scope| scope.0);
        }
        let newline = self.file_format().spelling();
        let block = self
            .projection()
            .blocks_for_region(&(range.start..range.start))
            .into_iter()
            .find(|block| block.range.start <= range.start && range.start <= block.range.end);
        let mut prefix = String::new();
        if let Some(block) = block {
            prefix = "> ".repeat(block.quote_depth);
            if matches!(block.kind, super::super::BlockKind::ListItem { .. }) {
                let line = self
                    .state()
                    .source_hard_lines
                    .line_at_offset(source.start)
                    .and_then(|index| self.state().source_hard_lines.get(index))
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let raw = self.table_source_text(line)?;
                let quoted = super::super::markdown_quotes::prefix(&raw);
                let body = &raw[quoted..];
                let indent = super::super::markdown_blocks::marker_prefix_length(body)
                    .unwrap_or_else(|| body.len() - body.trim_start_matches([' ', '\t']).len());
                prefix.push_str(&" ".repeat(indent));
            }
        }
        let mut body = scopes
            .iter()
            .rev()
            .map(|scope| scope.2.as_str())
            .collect::<String>();
        let needs_before = source.start
            > self
                .state()
                .source_hard_lines
                .get(0)
                .map_or(0, |range| range.start);
        if needs_before {
            body.push_str(newline);
            body.push_str(&prefix);
            body.push_str(newline);
            body.push_str(&prefix);
        }
        let table_start = source.start + self.encoding().encode_fragment(&body)?.len()
            - self
                .encoding()
                .encode_fragment(&prefix)?
                .len()
                .min(self.encoding().encode_fragment(&body)?.len());
        body.push('|');
        body.push_str(&"  |".repeat(columns));
        body.push_str(newline);
        body.push_str(&prefix);
        body.push('|');
        body.push_str(&" --- |".repeat(columns));
        body.push_str(newline);
        body.push_str(&prefix);
        for row in 0..body_rows {
            body.push('|');
            body.push_str(&"  |".repeat(columns));
            if row + 1 < body_rows {
                body.push_str(newline);
                body.push_str(&prefix);
            }
        }
        body.push_str(newline);
        body.push_str(&prefix);
        body.push_str(newline);
        body.push_str(&prefix);
        for (_, opening, _) in &scopes {
            body.push_str(opening);
        }
        let prepared = self.prepare_reprojected_source_patches(vec![SourcePatch::primary(
            source,
            self.encoding().encode_fragment(&body)?,
        )])?;
        let PreparedPublication::State(state) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let table = state
            .projection
            .tables()
            .iter()
            .find(|table| table.source_range.start == table_start)
            .ok_or(DocumentError::VerificationFailed)?;
        if table.columns.len() != columns || table.rows.len() != body_rows + 1 {
            return Err(DocumentError::VerificationFailed.into());
        }
        if self.format() == Format::Markdown {
            let before = &self.text()[..range.start];
            let after = &self.text()[range.end..];
            let candidate_before = &state.projection.text()[..table.range.start];
            let candidate_after = &state.projection.text()[table.range.end..];
            if candidate_before.trim_end_matches('\n') != before.trim_end_matches('\n')
                || candidate_after.trim_start_matches('\n') != after.trim_start_matches('\n')
            {
                return Err(unsupported(
                    "This insertion cannot preserve the surrounding inline structure.",
                ));
            }
        }
        let caret = table.rows[0].cells[0].range.start;
        Ok((prepared, caret))
    }
}
fn delimiter(alignment: TableAlignment) -> String {
    match alignment {
        TableAlignment::Unspecified => "---",
        TableAlignment::Left => ":---",
        TableAlignment::Center => ":---:",
        TableAlignment::Right => "---:",
    }
    .to_owned()
}

impl Document {
    pub(super) fn table_text_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if self.format() != Format::Markdown {
            return Ok(None);
        }
        let Some((table, row, cell)) = self.projection().table_cell_at(edit.range.start) else {
            return Ok(None);
        };
        if edit.range == table.range && !edit.range.is_empty() {
            // The physical ending after the last row also supports the
            // unselected following paragraph boundary. Keep its original
            // spelling when the table has a following formatted owner.
            let end = if table.range.end < self.projection().text_tree().byte_len() {
                table
                    .source_rows
                    .last()
                    .map_or(table.source_range.end, |row| row.source_body.end)
            } else {
                table.source_range.end
            };
            return Ok(Some(vec![SourcePatch::primary(
                table.source_range.start..end,
                self.encoding()
                    .encode_fragment(&self.table_literal_text(&edit.replacement))?,
            )]));
        }
        if edit.range.end > cell.range.end {
            return Err(unsupported(
                "Editing across table cell boundaries requires a cell selection or a row action.",
            ));
        }
        let row_index = table
            .rows
            .partition_point(|value| value.range.start < row.range.start);
        let column = row
            .cells
            .partition_point(|value| value.range.start < cell.range.start);
        if cell.missing
            || edit.range == cell.range && !edit.range.is_empty() && edit.replacement.is_empty()
        {
            return self
                .table_cell_replacement(
                    table,
                    row_index,
                    column,
                    &self.table_literal_text(&edit.replacement),
                )
                .map(Some);
        }
        if !edit.replacement.contains(['|', '\n']) {
            return Ok(None);
        }
        let in_code = self
            .projection()
            .markdown_replacement_begins_in_code(&edit.range);
        if in_code && edit.replacement.contains('\n') {
            return Err(unsupported(
                "A literal code span cannot contain a table cell line break.",
            ));
        }
        let syntax = if in_code {
            edit.replacement.replace('|', "\\|")
        } else {
            self.table_literal_text(&edit.replacement)
        };
        let source = if edit.range.is_empty() {
            let at = super::super::source_edit::insertion_point(
                self.projection(),
                edit.range.start,
                None,
            )
            .ok_or(DocumentError::AmbiguousProjection)?;
            at..at
        } else {
            self.projection()
                .source_range(edit.range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        Ok(Some(vec![SourcePatch::primary(
            source,
            self.encoding().encode_fragment(&syntax)?,
        )]))
    }
}

impl Document {
    fn prepare_table_matrix_paste(
        &self,
        id: u64,
        row: usize,
        column: usize,
        cells: Vec<Vec<crate::document::TableClipboardCell>>,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let width = cells.first().map_or(0, Vec::len);
        if width == 0
            || cells.iter().any(|row| row.len() != width)
            || cells
                .len()
                .checked_mul(width)
                .is_none_or(|count| count > 1_000_000)
        {
            return Err(unsupported(
                "The clipboard cell matrix is empty, ragged, or too large.",
            ));
        }
        let table = self
            .projection()
            .tables()
            .iter()
            .find(|table| table.id == id)
            .ok_or_else(|| unsupported("The table changed."))?;
        if row >= table.rows.len() || column >= table.columns.len() {
            return Err(unsupported("The destination cell changed."));
        }
        let table_index = self
            .projection()
            .tables()
            .iter()
            .position(|table| table.id == id)
            .unwrap();
        let mut scratch = self.scratch_document();
        let mut composition = super::replacement::PatchComposition::new(self.source_byte_len());
        let mut apply = |scratch: &mut Document,
                         prepared: PreparedModelTransaction|
         -> Result<(), ModelTransactionError> {
            for patch in prepared.summary.source_patches.iter().rev() {
                composition.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };
        while scratch.projection().tables()[table_index].columns.len() < column + width {
            let table = &scratch.projection().tables()[table_index];
            let (prepared, _) = scratch.prepare_table_edit(
                scratch.id(),
                scratch.revision(),
                TableEditIntent::InsertColumn {
                    table: table.id,
                    column: table.columns.len() - 1,
                    after: true,
                },
            )?;
            apply(&mut scratch, prepared)?;
        }
        while scratch.projection().tables()[table_index].rows.len() < row + cells.len() {
            let table = &scratch.projection().tables()[table_index];
            let (prepared, _) = scratch.prepare_table_edit(
                scratch.id(),
                scratch.revision(),
                TableEditIntent::InsertRow {
                    table: table.id,
                    row: table.rows.len() - 1,
                    after: true,
                },
            )?;
            apply(&mut scratch, prepared)?;
        }
        for (row_offset, values) in cells.iter().enumerate() {
            for (column_offset, value) in values.iter().enumerate() {
                let target = &scratch.projection().tables()[table_index].rows[row + row_offset]
                    .cells[column + column_offset];
                let mut range = target.range.clone();
                // A matrix supplies complete cell values, including their inline
                // formatting. Retire the destination's old scopes before rich
                // insertion so an old bold/code scope cannot leak into the copy.
                if !range.is_empty() {
                    let prepared = scratch.prepare_text_edits(vec![TextEdit::new(range, "")])?;
                    apply(&mut scratch, prepared)?;
                    range = scratch.projection().tables()[table_index].rows[row + row_offset].cells
                        [column + column_offset]
                        .range
                        .clone();
                }
                if value.text.is_empty() {
                    continue;
                }
                let prepared = match scratch.prepare_clipboard_fragment(
                    range.clone(),
                    &value.fragment,
                    &value.text,
                )? {
                    Some(prepared) => prepared,
                    None => scratch.prepare_text_edits(vec![TextEdit::new(range, &value.text)])?,
                };
                apply(&mut scratch, prepared)?;
            }
        }
        let prepared = self.prepare_reprojected_source_patches(composition.source_patches())?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Ok((prepared, table.rows[row].cells[column].range.start));
        };
        let caret = candidate.projection.tables()[table_index].rows[row].cells[column]
            .range
            .start;
        Ok((prepared, caret))
    }
}
