//! GFM task markers are source syntax attached to the first paragraph of an
//! item. Presentation queries read the parsed block index; only execution
//! decodes the small source prefix and prepares a verified transaction.
use super::*;
use crate::document::{Block, BlockKind, ContainerKind, ListStyle};

impl Document {
    fn checkbox_owner(&self, at: usize, _affinity: BoundaryAffinity) -> Option<Block> {
        if !self.format().is_markdown() || self.text_point(at).is_err() { return None; }
        let blocks = self.projection().list_indentation_blocks();
        let index = blocks.index_touching_point(at)?;
        let block = blocks.get(index)?;
        let owner = self.projection().list_item_first_block(&block).unwrap_or(block);
        if !Self::task_owner_eligible(&owner) { return None; }
        Some(owner)
    }

    fn task_owner_eligible(owner: &Block) -> bool {
        if !owner.checkbox_insertable || owner.markdown_html || owner.thematic_break
            || owner.containers.iter().any(|member| member.container.kind == ContainerKind::CodeBlock)
        { return false; }
        // GFM's marker belongs to the item's first prose paragraph, not a
        // nested quote block. A quote outside the item is still eligible.
        if owner.containers.iter().rposition(|member| member.container.kind == ContainerKind::ListItem)
            .is_some_and(|item| owner.containers[item + 1..].iter()
                .any(|member| member.container.kind == ContainerKind::Quote)) { return false; }
        matches!(owner.kind, BlockKind::Paragraph | BlockKind::ListItem { item_start: true, .. })
    }

    fn list_style_targets(&self, range: &Range<usize>) -> Vec<Block> {
        let selected = if range.is_empty() {
            let blocks = self.projection().list_indentation_blocks();
            blocks.index_touching_point(range.start).and_then(|index| blocks.get(index)).into_iter().collect()
        } else { structural_style::selected_blocks(self, range) };
        let mut seen = BTreeSet::new();
        selected.into_iter().map(|block|
            self.projection().list_item_first_block(&block).unwrap_or(block))
            .filter(|block| seen.insert(block.id)).collect()
    }

    /// Indexed owner lookup only; existing task items remain enabled so the
    /// same list control can toggle them off. Selection work is local to its leaves.
    pub fn can_set_task_list(&self, range: Range<usize>) -> bool {
        if !self.format().is_markdown() || !self.is_modifiable() || self.validate_range(&range).is_err() {
            return false;
        }
        let targets = self.list_style_targets(&range);
        !targets.is_empty() && targets.iter().all(Self::task_owner_eligible)
    }

    pub fn checkbox_at(&self, at: usize, affinity: BoundaryAffinity) -> Option<(u64, bool)> {
        let owner = self.checkbox_owner(at, affinity)?;
        Some((owner.id, owner.task_checkbox?))
    }

    pub(super) fn prepare_task_list_style(&self, range: &Range<usize>, style: Option<ListStyle>)
        -> Result<Option<PreparedModelTransaction>, ModelTransactionError>
    {
        if !self.format().is_markdown() {
            return if style == Some(ListStyle::Task) { Err(DocumentError::UnsupportedFormatting.into()) } else { Ok(None) };
        }
        let targets = self.list_style_targets(range);
        if style != Some(ListStyle::Task) && !targets.iter().any(|block| block.task_checkbox.is_some()) {
            return Ok(None);
        }
        if style == Some(ListStyle::Task) && !self.can_set_task_list(range.clone()) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if targets.len() == 1 { return self.prepare_task_list_target(&targets[0], style).map(Some); }
        let mut scratch = self.scratch_document();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        for target in targets.iter().rev() {
            let current = scratch.list_style_targets(&target.range).into_iter()
                .find(|block| block.range.start == target.range.start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prepared = scratch.prepare_task_list_target(&current, style)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
        }
        let patches = sources.source_patches();
        if patches.is_empty() { return Ok(Some(self.no_op_prepared())); }
        self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.start..range.start, "")], Some(patches), true).map(Some)
    }

    fn prepare_task_list_target(&self, block: &Block, style: Option<ListStyle>)
        -> Result<PreparedModelTransaction, ModelTransactionError>
    {
        let Some(style) = style else {
            return self.prepare_list_style_raw(block.range.clone(), None);
        };
        if style == ListStyle::Task && block.task_checkbox.is_some() { return Ok(self.no_op_prepared()); }
        if style != ListStyle::Task && block.task_checkbox.is_none() {
            return self.prepare_list_style_raw(block.range.clone(), Some(style));
        }
        let (line_start, prefix, body) = self.checkbox_source_prefix(block)?;
        let mut scan = 0;
        let mut marker = None;
        while scan < body {
            let quote = crate::document::markdown_quotes::prefix(&prefix[scan..body]);
            if quote > 0 { scan += quote; continue; }
            let tail = &prefix[scan..body];
            let Some(length) = crate::document::markdown_blocks::marker_prefix_length(tail) else { break; };
            let indent = tail.len() - tail.trim_start_matches([' ', '\t']).len();
            marker = Some(scan + indent..scan + length);
            scan += length;
        }
        let ordered = matches!(block.kind, BlockKind::ListItem { ordered: true, .. });
        let base = match style {
            ListStyle::Task => "- ".to_owned(),
            ListStyle::Bullet if !ordered => marker.as_ref().map(|range| prefix[range.clone()].to_owned()).unwrap_or_else(|| "- ".into()),
            ListStyle::Numbered if ordered => marker.as_ref().map(|range| prefix[range.clone()].to_owned()).unwrap_or_else(|| "1. ".into()),
            ListStyle::Bullet => "- ".to_owned(),
            ListStyle::Numbered => "1. ".to_owned(),
        };
        let marker = marker.unwrap_or(body..body);
        let mut end = marker.end;
        if block.task_checkbox.is_some() {
            end += crate::document::markdown_blocks::task_prefix_length(&prefix[end..])
                .ok_or(DocumentError::AmbiguousProjection)?;
        }
        let mut replacement = base.clone();
        if style == ListStyle::Task { replacement.push_str("[ ] "); }
        let source_start = line_start + self.encoding().encode_fragment(&prefix[..marker.start])?.len();
        let source_end = line_start + self.encoding().encode_fragment(&prefix[..end])?.len();
        let mut patches = vec![SourcePatch::primary(source_start..source_end,
            self.encoding().encode_fragment(&replacement)?)];
        if block.list_level().is_some() {
            self.task_list_indent_patches(block, line_start, &prefix, &marker, &base, &mut patches)?;
        }
        if block.task_checkbox.is_some() { self.protect_removed_task_body(block, &mut patches)?; }
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(block.range.start..block.range.start, "")], Some(patches), true)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        let actual = candidate.list_style_targets(&block.range).into_iter().next()
            .ok_or(DocumentError::VerificationFailed)?;
        if actual.task_checkbox != (style == ListStyle::Task).then_some(false) {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    /// Marker width belongs to the whole item. Preserve the relative columns
    /// of its continuation paragraphs and descendants when changing families.
    fn task_list_indent_patches(&self, block: &Block, line_start: usize, prefix: &str,
        marker: &Range<usize>, replacement: &str, patches: &mut Vec<SourcePatch>)
        -> Result<(), ModelTransactionError>
    {
        let columns = |text: &str| text.bytes().fold(0usize, |column, byte|
            column + if byte == b'\t' { 4 - column % 4 } else { 1 });
        // Continuation rows spell preceding item markers as indentation, but
        // quote markers remain literal. Measure after the final quote on both
        // rows, including interleaved `- > 1000. item` containers.
        let quote = prefix[..marker.start].rfind('>').map_or(0, |at| {
            at + 1 + usize::from(prefix.as_bytes().get(at + 1).is_some_and(|byte| matches!(byte, b' ' | b'\t')))
        });
        let old_columns = columns(&prefix[quote..marker.end]);
        let new_columns = columns(&format!("{}{replacement}", &prefix[quote..marker.start]));
        let delta = new_columns as isize - old_columns as isize;
        if delta == 0 { return Ok(()); }
        let blocks = self.projection().list_indentation_blocks();
        let first = blocks.index_touching_point(block.range.start).ok_or(DocumentError::AmbiguousProjection)?;
        let key = u16::from(block.list_level().ok_or(DocumentError::AmbiguousProjection)?) + 1;
        let end = blocks.find_navigation(first + 1..blocks.len(), false, |summary|
            summary.minimum < key || summary.minimum_start <= key).unwrap_or(blocks.len());
        let last = blocks.get(end - 1).ok_or(DocumentError::AmbiguousProjection)?;
        let mut source_end = self.projection().source_range(last.range.clone())
            .map(|range| range.end).or_else(|| self.projection().source_insertion_point(last.range.start, true))
            .ok_or(DocumentError::AmbiguousProjection)?;
        if let Some(fence) = crate::document::markdown_code::fenced_source(self, &last)? {
            source_end = source_end.max(fence.closing_content_end.unwrap_or(source_end));
        }
        let lines = &self.state().source_hard_lines;
        let first_line = lines.line_at_offset(line_start).ok_or(DocumentError::AmbiguousProjection)?;
        let last_line = lines.line_at_offset(source_end.saturating_sub(1).max(line_start))
            .ok_or(DocumentError::AmbiguousProjection)?;
        for index in first_line + 1..=last_line {
            let line = lines.get(index).ok_or(DocumentError::AmbiguousProjection)?;
            let prefix_end = line.end.min(line.start + 1024);
            let bytes = self.state().source.bytes_in(line.start..prefix_end).ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, line.start)?;
            let quote = crate::document::markdown_quotes::prefix(&decoded.text);
            let count = decoded.text[quote..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
            let indent = &decoded.text[quote..quote + count];
            if columns(indent) < old_columns || decoded.text[quote..].trim().is_empty() { continue; }
            let new_width = columns(indent).checked_add_signed(delta).ok_or(DocumentError::VerificationFailed)?;
            let start = decoded.source_boundary(quote).ok_or(DocumentError::AmbiguousProjection)?;
            let end = decoded.source_boundary(quote + count).ok_or(DocumentError::AmbiguousProjection)?;
            patches.push(SourcePatch::primary(start..end, self.encoding().encode_fragment(&" ".repeat(new_width))?));
        }
        Ok(())
    }

    pub(super) fn checkbox_source_prefix(&self, block: &Block) -> Result<(usize, String, usize), ModelTransactionError> {
        let body = self.projection().source_insertion_point(block.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = self.state().source_hard_lines.line_at_offset(body)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let start = line.start.max(self.projection().source_content_start());
        let bytes = self.state().source.bytes_in(start..body).ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, start)?;
        let mut at = 0;
        loop {
            let tail = &decoded.text[at..];
            let quote = crate::document::markdown_quotes::prefix(tail);
            if quote > 0 { at += quote; continue; }
            let Some(length) = crate::document::markdown_blocks::marker_prefix_length(tail) else { break; };
            // A trailing emphasis opener is inline content, not a bare list.
            if length == tail.len() && !block.range.is_empty() && !tail.ends_with([' ', '\t']) { break; }
            at += length;
        }
        if block.list_level().is_none() {
            at += decoded.text[at..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
        }
        Ok((start, decoded.text, at))
    }

    pub(super) fn remove_checkbox_source_patches(&self, block: &Block) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let (start, prefix, offset) = self.checkbox_source_prefix(block)?;
        let tail = &prefix[offset..];
        let padding = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        let marker = offset + padding;
        if prefix.as_bytes().get(marker) != Some(&b'[') || prefix.as_bytes().get(marker + 2) != Some(&b']') {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        let end = marker + 3 + prefix[marker + 3..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
        let source_start = start + self.encoding().encode_fragment(&prefix[..marker])?.len();
        let source_end = start + self.encoding().encode_fragment(&prefix[..end])?.len();
        let mut patches = vec![SourcePatch::primary(source_start..source_end, Vec::new())];
        self.protect_removed_task_body(block, &mut patches)?;
        Ok(patches)
    }

    fn protect_removed_task_body(&self, block: &Block, patches: &mut Vec<SourcePatch>)
        -> Result<(), ModelTransactionError>
    {
        // A task marker made the first body prefix prose. Removing it can
        // activate headings, quotes, fences, or another literal task marker.
        // Reuse the local candidate proof and escape only consumed literals.
        let mut end = (block.range.start + 64).min(block.range.end);
        while !self.projection().text_tree().is_char_boundary(end)
            .map_err(DocumentError::FormattedTextStorage)? { end -= 1; }
        markdown_block_styles::preserve_retained_literals_in_regions(self,
            &[TextEdit::new(block.range.start..block.range.start, "")],
            &[block.range.start..end], patches)?;
        Ok(())
    }

    fn prepare_checkbox_patch(&self, patch: SourcePatch, at: usize, checked: bool)
        -> Result<PreparedModelTransaction, ModelTransactionError>
    {
        let prepared = self.prepare_text_edits_with_patch_policy(vec![TextEdit::new(at..at, "")], Some(vec![patch]), true)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        if prepared.summary().formatted_splices().iter().any(|splice| !splice.old_range().is_empty() || splice.inserted_len() != 0)
            || candidate.checkbox_at(at, BoundaryAffinity::Downstream).map(|(_, state)| state) != Some(checked)
        { return Err(DocumentError::VerificationFailed.into()); }
        Ok(prepared)
    }

    pub fn prepare_toggle_checkbox(&self, at: usize, block_id: u64)
        -> Result<PreparedModelTransaction, ModelTransactionError>
    {
        self.require_modifiable()?;
        let block = self.checkbox_owner(at, BoundaryAffinity::Downstream)
            .filter(|block| block.id == block_id).ok_or(DocumentError::UnsupportedFormatting)?;
        let checked = block.task_checkbox.ok_or(DocumentError::UnsupportedFormatting)?;
        let (start, prefix, offset) = self.checkbox_source_prefix(&block)?;
        let tail = &prefix[offset..];
        let padding = tail.len() - tail.trim_start_matches([' ', '\t']).len();
        let marker = offset + padding;
        if prefix.as_bytes().get(marker) != Some(&b'[')
            || prefix.as_bytes().get(marker + 2) != Some(&b']')
        { return Err(DocumentError::AmbiguousProjection.into()); }
        let source_at = start + self.encoding().encode_fragment(&prefix[..marker + 1])?.len();
        let replacement = self.encoding().encode_fragment(if checked { " " } else { "x" })?;
        self.prepare_checkbox_patch(SourcePatch::primary(source_at..source_at + replacement.len(), replacement),
            block.range.start, !checked)
    }
}
