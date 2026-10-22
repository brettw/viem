//! Leave a literal code owner by replacing its two final empty lines with an
//! ordinary paragraph. The retained body and authored fence stay byte-exact.
use super::*;

impl Document {
    fn code_block_exit_tail(&self, at: usize) -> Result<Option<(super::super::Block, Range<usize>)>, DocumentError> {
        if self.format() != Format::Markdown { return Ok(None); }
        let Some(block) = super::super::edit_boundary::paragraph_at(self, at)? else { return Ok(None) };
        if at != block.range.end || !block.code_kind.treatment_removable()
            || block.markdown_html || !super::super::edit_boundary::is_code_paragraph(self, &block)? {
            return Ok(None);
        }
        let lines = self.hard_line_snapshot();
        let current = lines.line_at_offset(at).map_err(|_| DocumentError::AmbiguousProjection)?;
        let Some(previous) = current.index().checked_sub(1).and_then(|index| lines.line(index)) else { return Ok(None) };
        // Both empty rows must belong to this code owner. The separator before
        // them is consumed too, leaving the retained final code line intact.
        if previous.content_range().start < block.range.start { return Ok(None); }
        for line in [&current, &previous] {
            let range = line.content_range();
            let mut offset = range.start;
            while offset < range.end {
                let bytes = self.projection().text_tree().byte_chunk_at(offset);
                let length = bytes.len().min(range.end - offset);
                if length == 0 { return Err(DocumentError::AmbiguousProjection); }
                if !bytes[..length].iter().all(|byte| matches!(byte, b' ' | b'\t')) { return Ok(None); }
                offset += length;
            }
        }
        let tail = previous.content_range().start.saturating_sub(1).max(block.range.start)..at;
        Ok(Some((block, tail)))
    }

    pub(crate) fn code_block_exit_request(&self, at: usize) -> Result<Option<ModelRequest>, DocumentError> {
        Ok(self.code_block_exit_tail(at)?.map(|_| ModelRequest::ExitCodeBlock {
            document: self.id(), revision: self.revision(), at,
        }))
    }

    pub(super) fn prepare_code_block_exit(&self, at: usize) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let (block, tail) = self.code_block_exit_tail(at)?.ok_or(DocumentError::UnsupportedFormatting)?;
        let fence = super::super::markdown_code::fenced_source(self, &block)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let source_tail = self.projection().source_range(tail.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let ending = self.file_format().spelling();
        let continuation = if block.list_level().is_some() {
            fence.body_prefix.clone()
        } else {
            fence.body_prefix[..super::super::markdown_quotes::prefix(&fence.body_prefix)].to_owned()
        };
        let separator = if continuation.is_empty() {
            ending.repeat(2)
        } else {
            format!("{ending}{}{ending}{continuation}<span></span>", continuation.trim_end_matches([' ', '\t']))
        };
        let (after, syntax) = if let Some(end) = fence.closing_content_end {
            (end, separator)
        } else {
            // An unclosed fence at EOF gains only its missing closer. Keep the
            // original marker kind and width and the retained body bytes.
            let marker = (fence.delimiter as char).to_string().repeat(fence.width);
            (fence.body.end, format!("{ending}{}{marker}{separator}", fence.body_prefix))
        };
        let patches = vec![
            SourcePatch::primary(source_tail, Vec::new()),
            SourcePatch::primary(after..after, self.encoding().encode_fragment(&syntax)?),
        ];
        let cursor = tail.start + 1;
        let prepared = if tail.len() == 1 && self.projection().text_tree().byte_chunk_at(tail.start).first() == Some(&b'\n') {
            // An initially empty block has only one internal break to replace
            // with a paragraph boundary. Its flat text stays the same while
            // its hard-line ownership deliberately changes. Verify that
            // structural intention explicitly, retaining an identity caret map.
            let mut prepared = self.prepare_reprojected_source_patches(patches)?;
            let PreparedPublication::State(candidate) = &prepared.publication else {
                return Err(DocumentError::VerificationFailed.into());
            };
            let old = self.projection().text_tree();
            let new = candidate.projection.text_tree();
            if let Some((before, after)) = old.changed_extent(new) {
                if old.slice(before).map_err(DocumentError::FormattedTextStorage)?
                    != new.slice(after).map_err(DocumentError::FormattedTextStorage)? {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
            prepared.text_position_map = PositionMap::for_text_snapshots(
                self.id(), self.revision(), candidate.revision,
                self.projection(), &candidate.projection, Vec::new(),
            )?;
            prepared.summary.formatted_splices.clear();
            prepared
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![TextEdit::new(tail.clone(), "\n")], Some(patches), true,
            )?
        };
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let blocks = candidate.projection.blocks_for_region(&(tail.start..cursor));
        let retained = blocks.iter().find(|b| b.range.start == block.range.start && b.range.end == tail.start);
        let paragraph = blocks.iter().find(|b| b.range == (cursor..cursor));
        if !retained.is_some_and(|b| b.style == block.style && b.code_language == block.code_language
            && b.code_kind == block.code_kind && b.quote_depth == block.quote_depth && b.list_level() == block.list_level())
            || !paragraph.is_some_and(|b| b.style.0 != "Code Block" && b.quote_depth == block.quote_depth
                && b.list_level() == block.list_level()
                && b.list_item_owner() == retained.and_then(|retained| retained.list_item_owner())
                && b.containers.iter().all(|member| member.container.kind != super::super::ContainerKind::CodeBlock)) {
            return Err(DocumentError::VerificationFailed.into());
        }
        self.prepared_text_point(&prepared, cursor)?;
        Ok(prepared)
    }
}
