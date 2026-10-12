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
        let first = &patches[0];
        let point = self.visible_point_for_source(first.range.start, true)?;
        let Some(block) = self.projection().blocks_for_region(&(point..point)).into_iter()
            .find(|block| block.range.start <= point && point <= block.range.end)
        else { return Ok(None); };
        if block.kind != super::super::BlockKind::Paragraph || block.range.is_empty()
            || !block.containers.is_empty() || block.quote_depth != 0 || block.markdown_html
            || self.projection().table_at(point).is_some()
            || self.markdown_edit_needs_reference_context(&[TextEdit::new(block.range.clone(), "")])
        { return Ok(None); }
        // Blank rows beside a list can still belong to its final item; an
        // unclosed HTML block can consume all later source. Neither neighbor
        // can be retained independently merely because two endings remain.
        let neighbors = block.range.start.saturating_sub(1)
            ..block.range.end.saturating_add(1).min(self.projection().text_tree().byte_len());
        if self.projection().blocks_for_region(&neighbors).iter().any(|neighbor|
            neighbor.id != block.id && (neighbor.range.is_empty() || neighbor.markdown_html
                || neighbor.kind != super::super::BlockKind::Paragraph
                || !neighbor.containers.is_empty() || neighbor.quote_depth != 0
                || neighbor.style.0 == "Code Block"))
            || markdown_html_edit::edit_changes_literal_html_grammar(
                self, &[TextEdit::new(block.range.clone(), "")])?
        { return Ok(None); }
        // Both unedited separators must remain explicit paragraph boundaries.
        for boundary in [block.range.start.checked_sub(1),
            (block.range.end < self.projection().text_tree().byte_len()).then_some(block.range.end)]
            .into_iter().flatten()
        {
            let Some(source) = self.projection().source_range(boundary..boundary + 1) else { return Ok(None); };
            if !markdown_block_styles::explicit_paragraph_separator(self, &source, patches)? {
                return Ok(None);
            }
        }
        let content = self.projection().source_range(block.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_line = self.state().source_hard_lines.line_at_offset(content.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last_line = self.state().source_hard_lines.line_at_offset(content.end.saturating_sub(1))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let trailing = self.state().source_hard_lines.get(last_line).ok_or(DocumentError::AmbiguousProjection)?;
        let start = self.state().source_hard_lines.get(first_line).ok_or(DocumentError::AmbiguousProjection)?.start;
        let end = if block.range.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.projection().source_range(block.range.end..block.range.end + 1)
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
        let inherited_styles = self.projection().style_spans_for_region(&block.range);
        if inherited_styles.iter().any(|span|
            span.range.start < block.range.start || block.range.end < span.range.end)
        { return Ok(None); }
        let local_styles = inherited_styles.into_iter().map(|mut span| {
            span.range = span.range.start - block.range.start..span.range.end - block.range.start;
            span
        }).collect::<Vec<_>>();
        // Standalone parsing must reproduce the owner's complete original
        // semantics. New list/code/HTML owners require their broader grammar.
        if old_local.text() != self.projection().text_tree().slice(block.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?
            || old_local.style_spans() != local_styles
            || old_local.blocks().len() != 1 || old_local.blocks()[0].kind != block.kind
            || old_local.blocks()[0].style != block.style
            || regional.blocks().iter().any(|block| block.markdown_html
                || !block.containers.is_empty() || matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || block.style.0 == "Code Block")
        { return Ok(None); }
        let edits = source_backed_reprojection_edits_with_patches(&old_local, &regional, patches)
            .into_iter().map(|edit| TextEdit::new(
                edit.range.start + block.range.start..edit.range.end + block.range.start,
                edit.replacement)).collect::<Vec<_>>();
        let persistent = edits.iter().map(|edit| (edit.range.clone(), edit.replacement.as_str())).collect::<Vec<_>>();
        let (target_text, text_work) = self.projection().text_tree()
            .splice_prevalidated_batch_with_stats(&persistent).map_err(DocumentError::FormattedTextStorage)?;
        let first_hard = self.projection().hard_line_at_offset(block.range.start).ok_or(DocumentError::VerificationFailed)?;
        let last_hard = self.projection().hard_line_at_offset(block.range.end).ok_or(DocumentError::VerificationFailed)?;
        let new_formatted = block.range.start..block.range.start + regional.text_tree().byte_len();
        let projected_bytes = regional.text_tree().byte_len();
        let projected_lines = regional.hard_line_count();
        let mut next_id = self.next_projected_block_id;
        let (projection, range_work) = splice_line_local_projection(self.projection(), regional, revision,
            first_hard..last_hard + 1, block.range.clone(), old_source, new_source,
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
        }, map, None, next_id, PreparedPublication::State(state)), block.range, new_formatted)))
    }
}
