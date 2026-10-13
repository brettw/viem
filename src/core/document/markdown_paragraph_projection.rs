//! Bounded reprojection for structural edits inside independent prose.
use super::*;

impl Document {
    pub(super) fn prepare_independent_paragraph_reprojection(
        &self,
        patches: &[SourcePatch],
    ) -> Result<Option<(PreparedModelTransaction, Range<usize>, Range<usize>)>, ModelTransactionError> {
        if self.format() != Format::Markdown || patches.is_empty() {
            return Ok(None);
        }
        // An ATX heading is one source line opened by its own marker; a setext
        // heading's underline is a separate line, so it is not independent.
        let atx = |block: &super::super::Block| -> bool {
            let Some(content) = self.projection().source_range(block.range.clone()) else { return false; };
            let Ok(line) = self.physical_line_at_source(content.start) else { return false; };
            self.source_bytes_in(line.source_range.start..content.start).is_ok_and(|prefix| {
                self.encoding().decode_region(&prefix, line.source_range.start)
                    .is_ok_and(|prefix| prefix.text.trim_start_matches(' ').starts_with('#'))
            })
        };
        let ordinary = |block: &super::super::Block| (block.kind == super::super::BlockKind::Paragraph
                || matches!(block.kind, super::super::BlockKind::Heading(_)) && atx(block))
            && !block.range.is_empty() && block.containers.is_empty() && block.quote_depth == 0
            && !block.markdown_html && self.projection().table_at(block.range.start).is_none();
        // The edited paragraphs: one for edits inside a body, two adjacent
        // ones when an edit joins them across their separator.
        let first = &patches[0];
        let last = &patches[patches.len() - 1];
        let point = self.visible_point_for_source(first.range.start, true)?;
        let end_point = self.visible_point_for_source(last.range.end, false)?.max(point);
        let run = self.projection().blocks_for_region(&(point..end_point)).into_iter()
            .filter(|block| block.range.start <= end_point && point <= block.range.end)
            .collect::<Vec<_>>();
        let (Some(first_block), Some(last_block)) = (run.first(), run.last()) else { return Ok(None); };
        let block_range = first_block.range.start..last_block.range.end;
        if run.len() > 2 || !run.iter().all(ordinary)
            || !(first_block.range.start <= point && end_point <= last_block.range.end)
            || self.markdown_edit_needs_reference_context(&[TextEdit::new(block_range.clone(), "")])
        { return Ok(None); }
        // Blank rows beside a list can still belong to its final item; an
        // unclosed HTML block can consume all later source. Neither neighbor
        // can be retained independently merely because two endings remain.
        // Headings, rules, tables, code blocks and a following list end at
        // or begin with their own syntax: across an explicit separator they
        // neither absorb nor are absorbed by a paragraph, and a standalone
        // parse below rejects any edit that turns the paragraphs themselves
        // into code, a list or HTML.
        let neighbors = block_range.start.saturating_sub(1)
            ..block_range.end.saturating_add(1).min(self.projection().text_tree().byte_len());
        if self.projection().blocks_for_region(&neighbors).iter().any(|neighbor| {
            if run.iter().any(|block| block.id == neighbor.id) { return false; }
            let preceding = neighbor.range.end <= block_range.start;
            neighbor.range.is_empty() || neighbor.markdown_html || neighbor.quote_depth != 0
                || preceding && (matches!(neighbor.kind, super::super::BlockKind::ListItem { .. })
                    || neighbor.containers.iter().any(|member|
                        member.container.kind != super::super::ContainerKind::CodeBlock))
                || neighbor.containers.iter().any(|member|
                    member.container.kind == super::super::ContainerKind::Quote)
        }) || markdown_html_edit::edit_changes_literal_html_grammar(
                self, &[TextEdit::new(block_range.clone(), "")])?
        { return Ok(None); }
        // Both outer separators must be explicit paragraph boundaries before
        // and after the edit. Endings inserted into a single ending (after a
        // heading interrupting a paragraph) pair with that ending, which the
        // regional parse cannot see.
        for boundary in [block_range.start.checked_sub(1),
            (block_range.end < self.projection().text_tree().byte_len()).then_some(block_range.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return Ok(None); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, &[])?
                || !markdown_block_styles::explicit_paragraph_separator(self, &source, patches)?
            {
                return Ok(None);
            }
        }
        let content = self.projection().source_range(block_range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_line = self.state().source_hard_lines.line_at_offset(content.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last_line = self.state().source_hard_lines.line_at_offset(content.end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let trailing = self.state().source_hard_lines.get(last_line).ok_or(DocumentError::AmbiguousProjection)?;
        let start = self.state().source_hard_lines.get(first_line).ok_or(DocumentError::AmbiguousProjection)?.start;
        let end = if block_range.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.projection().source_range(block_range.end..block_range.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?.start
        };
        let old_source = start..end;
        if old_source.len() > 32768 || patches.iter().any(|patch|
            patch.part != SourcePartId::PRIMARY || patch.range.start < start || patch.range.end > end)
        { return Ok(None); }
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_source = start..rebase_source_boundary(end, patches, Association::AfterInsertion)?;
        if new_source.len() > 65536 { return Ok(None); }
        let revision = Revision(self.next_revision);
        let parse = |source: &super::super::source::SourceSnapshot, range: Range<usize>|
            -> Result<(FormattedDocument, Vec<Range<usize>>, usize), ModelTransactionError> {
            let bytes = source.bytes_in(range.clone()).ok_or(DocumentError::VerificationFailed)?;
            let decoded = self.encoding().decode_region(&bytes, range.start)?;
            let normalized = normalize(&decoded, self.file_format());
            // Terminal trivia shares insertion ownership with the final body.
            // Compound split/style edits require the complete boundary proof;
            // this optimization handles only independently delimited bodies.
            if range.end == source.len() && normalized.endings.last()
                .is_some_and(|ending| ending.source.end == range.end)
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            if normalized.text.contains('<')
                || super::super::markdown_syntax::needs_reference_context(&normalized.text)
            {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let mut rows = Vec::new();
            let mut cursor = range.start;
            for ending in &normalized.endings {
                rows.push(cursor..ending.source.end);
                cursor = ending.source.end;
            }
            // Source row records own their ending even though the unedited
            // paragraph separator remains outside the regional parser input.
            let trailing_end = rebase_source_boundary(trailing.end, patches, Association::AfterInsertion)?;
            rows.push(cursor..trailing_end.max(range.end));
            let projection = super::super::projection::project_band(&normalized, self.format(), revision, range.start, range.end,
                range.start == self.projection().source_content_start());
            Ok((projection, rows, bytes.len()))
        };
        let (old_local, _, old_bytes) = parse(&self.state().source, old_source.clone())?;
        let (regional, source_rows, new_bytes) = parse(&source, new_source.clone())?;
        let inherited_styles = self.projection().style_spans_for_region(&block_range);
        if inherited_styles.iter().any(|span|
            span.range.start < block_range.start || block_range.end < span.range.end)
        { return Ok(None); }
        let local_styles = inherited_styles.into_iter().map(|mut span| {
            span.range = span.range.start - block_range.start..span.range.end - block_range.start;
            span
        }).collect::<Vec<_>>();
        // Standalone parsing must reproduce the owner's complete original
        // semantics. New list/code/HTML owners require their broader grammar.
        if old_local.text() != self.projection().text_tree().slice(block_range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
            || old_local.style_spans() != local_styles
            || old_local.blocks().len() != run.len()
            || old_local.blocks().iter().zip(&run).any(|(local, block)|
                local.kind != block.kind || local.style != block.style)
            || regional.blocks().iter().any(|block| block.markdown_html
                || !block.containers.is_empty() || matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || block.style.0 == "Code Block")
        { return Ok(None); }
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches)
            .into_iter().map(|edit| TextEdit::new(
                edit.range.start + block_range.start..edit.range.end + block_range.start,
                edit.replacement)).collect::<Vec<_>>();
        let persistent = edits.iter().map(|edit| (edit.range.clone(), edit.replacement.as_str())).collect::<Vec<_>>();
        let (target_text, text_work) = self.projection().text_tree()
            .splice_prevalidated_batch_with_stats(&persistent).map_err(DocumentError::FormattedTextStorage)?;
        let first_hard = self.projection().hard_line_at_offset(block_range.start).ok_or(DocumentError::VerificationFailed)?;
        let last_hard = self.projection().hard_line_at_offset(block_range.end).ok_or(DocumentError::VerificationFailed)?;
        let new_formatted = block_range.start..block_range.start + regional.text_tree().byte_len();
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (projection, range_work) = splice_line_local_projection(self.projection(), regional, revision,
            first_hard..last_hard + 1, block_range.clone(), old_source, new_source,
            target_text, source.len(), false, true, &edits, patches, None, &mut next_id)
            .map_err(super::super::block_identity_document_error)?;
        let (source_hard_lines, source_work) = self.state().source_hard_lines
            .replace_ranges_with_stats(first_line..if end == self.source_byte_len() {
                self.state().source_hard_lines.len()
            } else { last_line + 1 }, &source_rows)
            .ok_or(DocumentError::VerificationFailed)?;
        if source_hard_lines.source_end() != source.len() { return Err(DocumentError::VerificationFailed.into()); }
        let splices = grapheme_closed_snapshot_map_splices(self.projection(), &projection, &edits)?;
        let map = PositionMap::for_text_snapshots(self.id, self.revision(), revision,
            self.projection(), &projection, splices)?;
        let formatted_splices = edits.iter().map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let state = DocumentState {
            transient_list_separator: None, revision, source, projection, source_hard_lines,
            encoding: self.encoding(), format: self.format(), file_format: self.file_format(),
            file_format_origin: self.file_format_origin(), line_ending_evidence: self.line_ending_evidence(),
            has_bom: self.state().has_bom,
        };
        Ok(Some((self.prepared(revision, ModelChangeSummary {
            kind: ModelChangeKind::TextEdits, source_patches: patches.to_vec(), formatted_splices,
            projection_work: ProjectionWorkStatistics::regional(old_bytes + new_bytes,
                projected_bytes, projected_lines, text_work, range_work, source_work),
            style_change: None, conversion_warnings: Vec::new(),
        }, map, None, next_id, PreparedPublication::State(state)), block_range, new_formatted)))
    }


    /// Reparse a complete list when its paragraph topology changes.
    /// Tight/loose spacing depends on all siblings, so a single-item restart
    /// cannot prove that the untouched siblings retain their presentation. The
    /// actual container bounds the work; a large list must not cause unrelated
    /// document content to be parsed by falling back at an arbitrary size cap.
    pub(super) fn prepare_list_paragraph_reprojection(
        &self,
        patches: &[SourcePatch],
    ) -> Result<Option<(PreparedModelTransaction, Range<usize>, Range<usize>)>, ModelTransactionError> {
        if self.format() != Format::Markdown || patches.is_empty() {
            return Ok(None);
        }
        let point = self.visible_point_for_source(patches[0].range.start, true)?;
        let blocks = self.projection().list_indentation_blocks(false);
        let Some(index) = blocks.index_touching_point(point) else { return Ok(None); };
        let Some(block) = blocks.get(index) else { return Ok(None); };
        // Enclosing quotes and HTML may extend beyond the list. They require
        // their own complete restart, rather than borrowing an outside owner.
        let Some(root) = block.containers.first().filter(|member|
            matches!(member.container.kind, super::super::ContainerKind::List { .. }))
        else { return Ok(None); };
        let owner = root.container.id;
        let owned = |block: &super::super::Block| block.containers.first()
            .is_some_and(|member| member.container.id == owner);
        let mut first = index;
        while first > 0 && blocks.get(first - 1).is_some_and(|block| owned(&block)) {
            first -= 1;
        }
        let mut end_index = index + 1;
        while end_index < blocks.len() && blocks.get(end_index).is_some_and(|block| owned(&block)) {
            end_index += 1;
        }
        let run = blocks.get_range(&(first..end_index)).ok_or(DocumentError::VerificationFailed)?;
        let block_range = run.first().unwrap().range.start..run.last().unwrap().range.end;
        let supported_body = |block: &super::super::Block| {
            (matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || block.kind == super::super::BlockKind::Paragraph && block.range.is_empty()
                    && block.containers.first().is_some_and(|member|
                        matches!(member.container.kind, super::super::ContainerKind::List { .. })))
                && !block.markdown_html && block.quote_depth == 0 && block.style.0 != "Code Block"
                && !block.thematic_break
        };
        if run.iter().any(|block| !supported_body(block)
            || self.projection().table_at(block.range.start).is_some())
        { return Ok(None); }
        // A bounded list must begin/end at its own container edges. An
        // adjacent unclosed HTML/quote owner cannot be retained independently.
        if !run.first().unwrap().containers[0].starts_here
            || !run.last().unwrap().containers[0].ends_here
            || first.checked_sub(1).and_then(|i| blocks.get(i)).is_some_and(|block|
                block.markdown_html || block.quote_depth != 0)
            || blocks.get(end_index).is_some_and(|block| block.markdown_html || block.quote_depth != 0)
        { return Ok(None); }
        for boundary in [block_range.start.checked_sub(1),
            (block_range.end < self.projection().text_tree().byte_len()).then_some(block_range.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return Ok(None); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, &[])?
                || !markdown_block_styles::explicit_paragraph_separator(self, &source, patches)?
            { return Ok(None); }
        }
        let content = self.projection().source_range(block_range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_line = self.state().source_hard_lines.line_at_offset(content.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last_line = self.state().source_hard_lines.line_at_offset(content.end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let trailing = self.state().source_hard_lines.get(last_line).ok_or(DocumentError::AmbiguousProjection)?;
        let start = self.state().source_hard_lines.get(first_line).ok_or(DocumentError::AmbiguousProjection)?.start;
        let end = if block_range.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.projection().source_range(block_range.end..block_range.end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?.start
        };
        let old_source = start..end;
        if patches.iter().any(|patch|
            patch.part != SourcePartId::PRIMARY || patch.range.start < start || patch.range.end > end)
            || self.markdown_edit_needs_reference_context(&[TextEdit::new(block_range.clone(), "")])
        { return Ok(None); }
        let source = apply_source_patches(&self.state().source, patches)?;
        let new_source = start..rebase_source_boundary(end, patches, Association::AfterInsertion)?;
        let revision = Revision(self.next_revision);
        let parse = |source: &super::super::source::SourceSnapshot, range: Range<usize>|
            -> Result<(FormattedDocument, Vec<Range<usize>>, usize), ModelTransactionError> {
            let bytes = source.bytes_in(range.clone()).ok_or(DocumentError::VerificationFailed)?;
            let decoded = self.encoding().decode_region(&bytes, range.start)?;
            let normalized = normalize(&decoded, self.file_format());
            // The authoring transaction uses an empty passive span to retain
            // an editable empty paragraph. Other HTML/reference grammar may
            // depend on source outside this complete list and stays excluded.
            if normalized.text.replace("<span>", "").replace("</span>", "").contains('<')
                || super::super::markdown_syntax::needs_reference_context(&normalized.text)
            { return Err(DocumentError::UnsupportedFormatting.into()); }
            let mut rows = Vec::new();
            let mut cursor = range.start;
            for ending in &normalized.endings {
                rows.push(cursor..ending.source.end);
                cursor = ending.source.end;
            }
            let trailing_end = rebase_source_boundary(trailing.end, patches, Association::AfterInsertion)?;
            // A complete list at EOF also owns terminal trivia and its empty
            // final source row. Include that row in the same replacement so
            // inserting a paragraph preserves final-terminator ownership.
            rows.push(cursor..trailing_end.max(range.end));
            let projection = super::super::projection::project_band(&normalized, self.format(), revision, range.start, range.end,
                range.start == self.projection().source_content_start());
            Ok((projection, rows, bytes.len()))
        };
        let (old_local, _, old_bytes) = parse(&self.state().source, old_source.clone())?;
        let (regional, source_rows, new_bytes) = parse(&source, new_source.clone())?;
        let inherited_styles = self.projection().style_spans_for_region(&block_range);
        if inherited_styles.iter().any(|span|
            span.range.start < block_range.start || block_range.end < span.range.end)
        { return Ok(None); }
        let local_styles = inherited_styles.into_iter().map(|mut span| {
            span.range = span.range.start - block_range.start..span.range.end - block_range.start;
            span
        }).collect::<Vec<_>>();
        if old_local.text() != self.projection().text_tree().slice(block_range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
            || old_local.style_spans() != local_styles
            || old_local.blocks().len() != run.len()
            || old_local.blocks().iter().zip(&run).any(|(local, block)|
                local.kind != block.kind || local.style != block.style
                || local.list_loose != block.list_loose || local.quote_depth != block.quote_depth
                || local.containers.len() != block.containers.len()
                || local.containers.iter().zip(block.containers.iter()).any(|(a, b)|
                    a.container.kind != b.container.kind || a.container.style != b.container.style
                    || a.starts_here != b.starts_here || a.ends_here != b.ends_here))
            || regional.blocks().iter().any(|block| !supported_body(block))
        { return Ok(None); }
        // Only paragraph topology may change: all item starts and their
        // nesting/numbering must remain in the same order within one owner.
        let item_kinds = |projection: &FormattedDocument| projection.blocks().iter()
            .filter(|block| matches!(block.kind, super::super::BlockKind::ListItem { item_start: true, .. }))
            .map(|block| block.kind.clone()).collect::<Vec<_>>();
        if item_kinds(&old_local) != item_kinds(&regional) { return Ok(None); }
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches)
            .into_iter().map(|edit| TextEdit::new(
                edit.range.start + block_range.start..edit.range.end + block_range.start,
                edit.replacement)).collect::<Vec<_>>();
        let persistent = edits.iter().map(|edit| (edit.range.clone(), edit.replacement.as_str())).collect::<Vec<_>>();
        let (target_text, text_work) = self.projection().text_tree()
            .splice_prevalidated_batch_with_stats(&persistent).map_err(DocumentError::FormattedTextStorage)?;
        let first_hard = self.projection().hard_line_at_offset(block_range.start).ok_or(DocumentError::VerificationFailed)?;
        let last_hard = self.projection().hard_line_at_offset(block_range.end).ok_or(DocumentError::VerificationFailed)?;
        let new_formatted = block_range.start..block_range.start + regional.text_tree().byte_len();
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (projection, range_work) = splice_line_local_projection(self.projection(), regional, revision,
            first_hard..last_hard + 1, block_range.clone(), old_source, new_source,
            target_text, source.len(), false, true, &edits, patches, None, &mut next_id)
            .map_err(super::super::block_identity_document_error)?;
        let (source_hard_lines, source_work) = self.state().source_hard_lines
            .replace_ranges_with_stats(first_line..if end == self.source_byte_len() {
                self.state().source_hard_lines.len()
            } else { last_line + 1 }, &source_rows)
            .ok_or(DocumentError::VerificationFailed)?;
        if source_hard_lines.source_end() != source.len() { return Err(DocumentError::VerificationFailed.into()); }
        let splices = grapheme_closed_snapshot_map_splices(self.projection(), &projection, &edits)?;
        let map = PositionMap::for_text_snapshots(self.id, self.revision(), revision,
            self.projection(), &projection, splices)?;
        let formatted_splices = edits.iter().map(|edit| Splice::new(edit.range.clone(), edit.replacement.len()))
            .collect::<Result<Vec<_>, _>>()?;
        let state = DocumentState {
            transient_list_separator: None, revision, source, projection, source_hard_lines,
            encoding: self.encoding(), format: self.format(), file_format: self.file_format(),
            file_format_origin: self.file_format_origin(), line_ending_evidence: self.line_ending_evidence(),
            has_bom: self.state().has_bom,
        };
        Ok(Some((self.prepared(revision, ModelChangeSummary {
            kind: ModelChangeKind::TextEdits, source_patches: patches.to_vec(), formatted_splices,
            projection_work: ProjectionWorkStatistics::regional(old_bytes + new_bytes,
                projected_bytes, projected_lines, text_work, range_work, source_work),
            style_change: None, conversion_warnings: Vec::new(),
        }, map, None, next_id, PreparedPublication::State(state)), block_range, new_formatted)))
    }
}
