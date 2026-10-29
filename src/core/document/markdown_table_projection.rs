//! Structural edits confined to an existing table parse only that table. The
//! surrounding projection, text and source-line indexes retain shared subtrees.
use super::*;
use crate::document::{Block, BlockKind};
impl Document {
    pub(super) fn prepare_table_local_reprojection(
        &self,
        patches: &[SourcePatch],
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !self.format().is_markdown() {
            return Ok(None);
        }
        let Some(first) = patches.first() else {
            return Ok(None);
        };
        let Some(table) = self.projection().tables().iter().find(|table| {
            table.source_range.start <= first.range.start
                && patches.last().unwrap().range.end <= table.source_range.end
        }) else {
            return Ok(None);
        };
        // A physical ending belongs to the table, but the caret after it
        // belongs to the following row. Parsing inserted text as a table
        // suffix would invent a break before that retained row and miss any
        // newly admitted table cells there.
        if first.range.start == table.source_range.end
            && table.source_rows.last().is_some_and(|row| row.source_body.end < table.source_range.end)
        {
            return Ok(None);
        }
        // A table nested in a list or quote is parsed without its owners. Its
        // cells inherit their owner context from the old table, after proving
        // that the inheritance reproduces the old table exactly.
        let old_blocks = self.projection().blocks_for_region(&table.range).into_iter()
            .filter(|block| table.range.start <= block.range.start && block.range.end <= table.range.end)
            .collect::<Vec<_>>();
        let nested = old_blocks.iter().any(|block| !block.containers.is_empty());
        // The final ending separates this table from the following source row
        // (or exposes the terminal empty row in Source). The table-only parse
        // omits that ending, so it cannot prove an edit which consumes it.
        if let Some(last) = table.source_rows.last() {
            let boundary = last.source_body.end..table.source_range.end;
            if patches.iter().any(|patch| patch.range.start < boundary.end
                && boundary.start < patch.range.end)
            {
                return Ok(None);
            }
        }
        // Prefix/container grammar and destruction of the header need broader
        // context. Ordinary cells and row/column operations remain self-contained.
        let old_source = table.source_range.clone();
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_source = old_source.start
            ..rebase_source_boundary(old_source.end, patches, Association::AfterInsertion)?;
        let revision = Revision(self.next_revision);
        let parse = |bytes: Vec<u8>,
                     range: Range<usize>|
         -> Result<
            (FormattedDocument, Vec<Range<usize>>, Range<usize>),
            ModelTransactionError,
        > {
            let decoded = self.encoding().decode_region(&bytes, range.start)?;
            let mut normalized = normalize(&decoded, self.file_format());
            let mut rows = Vec::new();
            let mut start = range.start;
            for ending in &normalized.endings {
                rows.push(start..ending.source.end);
                start = ending.source.end;
            }
            if start < range.end || rows.is_empty() {
                rows.push(start..range.end);
            }
            // The outer table ending belongs to the retained following boundary.
            // Excluding it from parser input avoids a fictitious trailing cell.
            let terminal = normalized
                .endings
                .last()
                .filter(|ending| ending.source.end == range.end)
                .cloned();
            if let Some(ending) = &terminal {
                normalized.truncate(ending.normalized.start);
            }
            let parsed_source = range.start
                ..terminal
                    .as_ref()
                    .map_or(range.end, |ending| ending.source.start);
            let mut projection = project(
                &normalized,
                self.format(),
                revision,
                parsed_source.start,
                parsed_source.end,
            );
            if terminal.is_some() {
                projection.extend_table_terminal_source(range.end);
            }
            Ok((projection, rows, parsed_source))
        };
        let old_bytes = self
            .state()
            .source
            .bytes_in(old_source.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        let new_bytes = source
            .bytes_in(new_source.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        let (mut old_local, _, old_parsed_source) = parse(old_bytes.clone(), old_source.clone())?;
        let (mut regional, source_ranges, new_parsed_source) =
            parse(new_bytes.clone(), new_source.clone())?;
        if nested {
            if old_local.tables().len() != 1 || regional.tables().len() != 1
                || !inherit_table_owner_context(&mut old_local, &old_blocks)
                || !inherit_table_owner_context(&mut regional, &old_blocks)
            {
                return Ok(None);
            }
            let inherited = old_local.blocks_for_region(&(0..old_local.text_tree().byte_len()));
            let attributes = |block: &Block| {
                let mut attributes = (*block.attributes).clone();
                attributes.direct_formatting = None;
                attributes
            };
            if inherited.len() != old_blocks.len()
                || inherited.iter().zip(&old_blocks).any(|(new, old)| {
                    new.range.start + table.range.start != old.range.start
                        || new.range.end + table.range.start != old.range.end
                        || attributes(new) != attributes(old)
                })
            {
                return Ok(None);
            }
            // Owner continuation depends only on each row's prefix. A row
            // keeps its owners when it reuses the spelling of a retained
            // continuation row; the first row's prefix may also open them.
            let prefix = |source: &super::super::source::SourceSnapshot, row: &super::super::MarkdownTableSourceRow| {
                source.bytes_in(row.source_range.start..row.source_body.start)
            };
            let old_prefixes = table.source_rows.iter()
                .map(|row| prefix(&self.state().source, row))
                .collect::<Option<Vec<_>>>()
                .ok_or(DocumentError::VerificationFailed)?;
            let rows = &regional.tables()[0].source_rows;
            for (index, row) in rows.iter().enumerate() {
                let spelling = prefix(&source, row).ok_or(DocumentError::VerificationFailed)?;
                let retained = if index == 0 { spelling == old_prefixes[0] }
                    else { old_prefixes[1..].contains(&spelling) };
                if !retained {
                    return Ok(None);
                }
            }
        }
        if old_local.tables().len() != 1
            || regional.tables().len() != 1
            || old_local.tables()[0].range != (0..old_local.text_tree().byte_len())
            || regional.tables()[0].range != (0..regional.text_tree().byte_len())
            || old_local.text()
                != self
                    .projection()
                    .text_tree()
                    .slice(table.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?
        {
            return Ok(None);
        }
        // Match source contributors inside the table so unchanged cells retain
        // their anchors even when rows or columns move to different ordinals.
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches);
        let edits = cell_local_edits(&old_local, &regional, &edits).unwrap_or(edits)
            .into_iter()
            .map(|edit| {
                TextEdit::new(
                    edit.range.start + table.range.start..edit.range.end + table.range.start,
                    edit.replacement,
                )
            })
            .collect::<Vec<_>>();
        let persistent = edits
            .iter()
            .map(|edit| (edit.range.clone(), edit.replacement.as_str()))
            .collect::<Vec<_>>();
        let (target_text, text_work) = self
            .projection()
            .text_tree()
            .splice_prevalidated_batch_with_stats(&persistent)
            .map_err(DocumentError::FormattedTextStorage)?;
        let first_line = self
            .projection()
            .hard_line_at_offset(table.range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let last_line = self
            .projection()
            .hard_line_at_offset(table.range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        let mut next_id = self.next_projected_block_id;
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let (projection, range_work) = splice_line_local_projection(
            self.projection(),
            regional,
            revision,
            first_line..last_line + 1,
            table.range.clone(),
            old_parsed_source,
            new_parsed_source,
            target_text,
            source.len(),
            false,
            true,
            &edits,
            patches,
            None,
            &mut next_id,
        )
        .map_err(super::super::block_identity_document_error)?;
        if nested {
            // Every cell must reconcile to one owner path. An owner that also
            // holds paragraphs outside the table keeps its identity rather than
            // splitting from them; one wholly inside it may be renewed, as by
            // complete reprojection, when no cell text survives.
            let (first, last) = (&old_blocks[0], &old_blocks[old_blocks.len() - 1]);
            let shared = first.containers.iter().zip(last.containers.iter())
                .map(|(start, end)| (!start.starts_here || !end.ends_here).then_some(start.container.id))
                .collect::<Vec<_>>();
            let range = table.range.start..table.range.start + projected_bytes;
            let cells = projection.blocks_for_region(&range).into_iter()
                .filter(|block| range.start <= block.range.start && block.range.end <= range.end)
                .collect::<Vec<_>>();
            let owners = |block: &Block| block.containers.iter()
                .map(|member| member.container.id).collect::<Vec<_>>();
            let path = cells.first().map(owners).unwrap_or_default();
            if path.len() != shared.len()
                || path.iter().zip(&shared).any(|(id, shared)| shared.is_some_and(|shared| shared != *id))
                || cells.iter().any(|block| owners(block) != path)
            {
                return Ok(None);
            }
        }
        let first_source = self
            .state()
            .source_hard_lines
            .line_at_offset(old_source.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let last_source = self
            .state()
            .source_hard_lines
            .line_at_offset(old_source.end.saturating_sub(1))
            .ok_or(DocumentError::VerificationFailed)?;
        let (source_hard_lines, source_work) = self
            .state()
            .source_hard_lines
            .replace_ranges_with_stats(first_source..last_source + 1, &source_ranges)
            .ok_or(DocumentError::VerificationFailed)?;
        let splices = grapheme_closed_snapshot_map_splices(self.projection(), &projection, &edits)?;
        let map = PositionMap::for_text_snapshots(
            self.id,
            self.revision(),
            revision,
            self.projection(),
            &projection,
            splices,
        )?;
        let formatted_splices = edits
            .iter()
            .map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let work = ProjectionWorkStatistics::regional(
            old_bytes.len() + new_bytes.len(),
            projected_bytes,
            projected_lines,
            text_work,
            range_work,
            source_work,
        );
        let state = DocumentState {
            transient_list_separator: None,
            revision,
            source,
            projection,
            source_hard_lines,
            encoding: self.state().encoding,
            format: self.format(),
            file_format: self.file_format(),
            file_format_origin: self.file_format_origin(),
            line_ending_evidence: self.line_ending_evidence(),
            has_bom: self.state().has_bom,
        };
        Ok(Some(self.prepared(
            revision,
            ModelChangeSummary {
                kind: ModelChangeKind::TextEdits,
                source_patches: patches.to_vec(),
                formatted_splices,
                projection_work: work,
                style_change: None,
                conversion_warnings: Vec::new(),
            },
            map,
            None,
            next_id,
            PreparedPublication::State(state),
        )))
    }
}

impl Document {
    pub(super) fn build_table_row_candidate(
        &self,
        source: &super::super::source::SourceSnapshot,
        revision: Revision,
        target_text: &FormattedTextTree,
        text_work: FormattedTextSpliceStats,
        edits: &[TextEdit],
        patches: &[SourcePatch],
    ) -> Result<Option<TextEditCandidate>, ModelTransactionError> {
        let Some(first) = edits.first() else {
            return Ok(None);
        };
        let Some((table, _, _)) = self.projection().table_cell_at(first.range.start) else {
            return Ok(None);
        };
        if self.projection().blocks_for_region(&table.range).iter()
            .any(|block| !block.containers.is_empty()) {
            return Ok(None);
        }
        let index = table
            .rows
            .partition_point(|row| row.range.start <= first.range.start)
            .saturating_sub(1);
        let row = &table.rows[index];
        let source_row = &table.source_rows[if index == 0 { 0 } else { index + 1 }];
        if edits
            .iter()
            .any(|edit| edit.range.start < row.range.start || edit.range.end > row.range.end)
            || patches.iter().any(|patch| {
                patch.range.start < source_row.source_body.start
                    || patch.range.end > source_row.source_body.end
            })
            || source_row.source_body.start != source_row.source_range.start
        {
            return Ok(None);
        }
        let new_start = rebase_source_boundary(
            source_row.source_body.start,
            patches,
            Association::BeforeInsertion,
        )?;
        let new_end = rebase_source_boundary(
            source_row.source_body.end,
            patches,
            Association::AfterInsertion,
        )?;
        let bytes = source
            .bytes_in(new_start..new_end)
            .ok_or(DocumentError::VerificationFailed)?;
        let decoded = self.encoding().decode_region(&bytes, new_start)?;
        if decoded.text.contains(['\r', '\n']) {
            return Ok(None);
        }
        let columns = table.columns.len();
        let header = format!("|{}", " h |".repeat(columns));
        let delimiter = format!("|{}", " --- |".repeat(columns));
        let ending = self.file_format().spelling();
        let prefix = if index == 0 {
            String::new()
        } else {
            format!("{header}{ending}{delimiter}{ending}")
        };
        let prefix_bytes = self.encoding().encode_fragment(&prefix)?;
        let mut stub = prefix_bytes.clone();
        stub.extend_from_slice(&bytes);
        if index == 0 {
            stub.extend(
                self.encoding()
                    .encode_fragment(&format!("{ending}{delimiter}"))?,
            );
        }
        let decoded = self.encoding().decode_region(&stub, 0)?;
        let normalized = normalize(&decoded, self.file_format());
        let projected = project(&normalized, self.format(), revision, 0, stub.len());
        let Some(parsed_table) = projected
            .tables()
            .first()
            .filter(|table| table.columns.len() == columns)
        else {
            return Ok(None);
        };
        if parsed_table.rows.len() != if index == 0 { 1 } else { 2 } {
            return Ok(None);
        }
        let source_delta = new_start as i128 - prefix_bytes.len() as i128;
        let physical_end = rebase_source_boundary(
            source_row.source_range.end,
            patches,
            Association::AfterInsertion,
        )?;
        let Some((regional, mut parsed_row, mut parsed_source_row)) =
            projected.extract_table_row(usize::from(index != 0), source_delta, new_start..new_end)
        else {
            return Ok(None);
        };
        let delta: isize = edits
            .iter()
            .map(|edit| edit.replacement.len() as isize - edit.range.len() as isize)
            .sum();
        let new_row_end = row
            .range
            .end
            .checked_add_signed(delta)
            .ok_or(DocumentError::VerificationFailed)?;
        if regional.text()
            != target_text
                .slice(row.range.start..new_row_end)
                .map_err(DocumentError::FormattedTextStorage)?
        {
            return Ok(None);
        }
        use super::super::range_index::RangedItem;
        parsed_row = parsed_row
            .with_transform(row.range.start..new_row_end, 0, None)
            .ok_or(DocumentError::VerificationFailed)?;
        parsed_source_row = parsed_source_row
            .with_transform(row.range.start..new_row_end, 0, None)
            .ok_or(DocumentError::VerificationFailed)?;
        parsed_row.source_range.end = physical_end;
        parsed_source_row.source_range.end = physical_end;
        let first_line = self
            .projection()
            .hard_line_at_offset(row.range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let last_line = self
            .projection()
            .hard_line_at_offset(row.range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (mut projection, mut range_work) = splice_line_local_projection(
            self.projection(),
            regional,
            revision,
            first_line..last_line + 1,
            row.range.clone(),
            source_row.source_body.clone(),
            new_start..new_end,
            target_text.clone(),
            source.len(),
            false,
            true,
            edits,
            patches,
            None,
            &mut next_id,
        )
        .map_err(super::super::block_identity_document_error)?;
        range_work.add_range_statistics(
            projection
                .replace_table_row_metadata(table.id, index, parsed_row, parsed_source_row)
                .ok_or(DocumentError::VerificationFailed)?,
        );
        let physical = self
            .state()
            .source_hard_lines
            .line_at_offset(source_row.source_range.start)
            .ok_or(DocumentError::VerificationFailed)?;
        let (source_hard_lines, source_work) = self
            .state()
            .source_hard_lines
            .replace_ranges_with_stats(
                physical..physical + 1,
                &[source_row.source_range.start..physical_end],
            )
            .ok_or(DocumentError::VerificationFailed)?;
        Ok(Some(TextEditCandidate {
            state: DocumentState {
                transient_list_separator: None,
                revision,
                source: source.clone(),
                projection,
                source_hard_lines,
                encoding: self.encoding(),
                format: self.format(),
                file_format: self.file_format(),
                file_format_origin: self.file_format_origin(),
                line_ending_evidence: self.line_ending_evidence(),
                has_bom: self.state().has_bom,
            },
            work: ProjectionWorkStatistics::regional(
                stub.len(),
                projected_bytes,
                projected_lines,
                text_work,
                range_work,
                source_work,
            ),
            block_ids_already_reconciled: true,
            next_projected_block_id: next_id,
            reprojected_region: Some((row.range.clone(),
                row.range.start..row.range.start + projected_bytes)),
        }))
    }
}

/// Give the cells of a table parsed without its owners the owner context of
/// the old table's cells. Corresponding cells keep their own context; after a
/// structural change the first and last cells keep the old edge memberships
/// and interior cells share the old interior context, which must be uniform.
fn inherit_table_owner_context(projection: &mut FormattedDocument, old: &[Block]) -> bool {
    let mut blocks = projection.blocks_for_region(&(0..projection.text_tree().byte_len()));
    let (count, old_count) = (blocks.len(), old.len());
    if count == 0 || old_count == 0 || count != old_count && (count < 2 || old_count < 2) {
        return false;
    }
    let context = |block: &Block| (block.kind.clone(), block.quote_depth, block.list_loose, block.list_editing, block.task_checkbox, block.checkbox_insertable,
        block.containers.iter().map(|member| (member.container.id, member.starts_here, member.ends_here))
            .collect::<Vec<_>>());
    let interior = old.get(1..old_count.saturating_sub(1)).unwrap_or_default();
    if interior.iter().any(|block| context(block) != context(&interior[0])) {
        return false;
    }
    for (index, block) in blocks.iter_mut().enumerate() {
        let source = if count == old_count {
            &old[index]
        } else if index == 0 {
            &old[0]
        } else if index + 1 == count {
            &old[old_count - 1]
        } else if let Some(source) = interior.first() {
            source
        } else {
            return false;
        };
        if matches!(source.kind, BlockKind::ListItem { .. }) {
            block.kind = source.kind.clone();
        }
        block.quote_depth = source.quote_depth;
        block.list_loose = source.list_loose;
        block.list_editing = source.list_editing;
        block.task_checkbox = source.task_checkbox;
        block.checkbox_insertable = source.checkbox_insertable;
        block.containers = source.containers.clone();
    }
    projection.try_install_paragraph_partition(blocks).is_ok()
}

/// Cell separators are structural and have no source contributor, so a change
/// to adjacent cells matches as one edit spanning them. When the table keeps
/// its shape, confine each change to its own cell so that every cell keeps its
/// identity and anchors, as the separators between them do.
fn cell_local_edits(old: &FormattedDocument, new: &FormattedDocument, edits: &[TextEdit]) -> Option<Vec<TextEdit>> {
    let cells = |projection: &FormattedDocument| projection.tables().first().map(|table| table.rows.iter()
        .map(|row| row.cells.iter().map(|cell| cell.range.clone()).collect::<Vec<_>>()).collect::<Vec<_>>());
    let (old_rows, new_rows) = (cells(old)?, cells(new)?);
    if old_rows.len() != new_rows.len()
        || old_rows.iter().zip(&new_rows).any(|(old, new)| old.len() != new.len())
    {
        return None;
    }
    let (old_cells, new_cells) = (old_rows.concat(), new_rows.concat());
    let contained = |edit: &TextEdit, cell: &Range<usize>| cell.start <= edit.range.start && edit.range.end <= cell.end;
    if edits.iter().all(|edit| old_cells.iter().any(|cell| contained(edit, cell))) {
        return None;
    }
    let (old_text, new_text) = (old.text(), new.text());
    let mut result = Vec::new();
    for (old_cell, new_cell) in old_cells.iter().zip(&new_cells) {
        let crossing = edits.iter().any(|edit| !contained(edit, old_cell)
            && edit.range.start <= old_cell.end && old_cell.start <= edit.range.end);
        if !crossing {
            result.extend(edits.iter().filter(|edit| contained(edit, old_cell)).cloned());
            continue;
        }
        let (before, after) = (&old_text[old_cell.clone()], &new_text[new_cell.clone()]);
        let prefix = before.char_indices().zip(after.chars())
            .find(|((_, a), b)| a != b).map_or(before.len().min(after.len()), |((at, _), _)| at);
        let suffix = before[prefix..].chars().rev().zip(after[prefix..].chars().rev())
            .take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum::<usize>();
        if prefix + suffix < before.len().max(after.len()) {
            result.push(TextEdit::new(old_cell.start + prefix..old_cell.end - suffix,
                &after[prefix..after.len() - suffix]));
        }
    }
    // The cells changed only as these edits say, and everything between them
    // is identical, exactly when they reproduce the new table text.
    let mut text = String::with_capacity(new_text.len());
    let mut at = 0;
    for edit in &result {
        text.push_str(&old_text[at..edit.range.start]);
        text.push_str(&edit.replacement);
        at = edit.range.end;
    }
    text.push_str(&old_text[at..]);
    (text == new_text).then_some(result)
}
