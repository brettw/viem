//! Source-local list continuation. Labels are source syntax and paragraph
//! decorations; the formatted intention contains only the new paragraph break.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{edit_boundary, BlockKind, ContainerKind};

/// Continue the innermost item on a row without starting its outer items again.
/// The decoded prefix ends at a proven body boundary, so inline openers are
/// excluded even when they resemble an empty list marker in isolation.
pub(super) fn sibling_prefix(
    text: &str,
    nonempty_body: bool,
    ordered: bool,
    ordinal: u64,
) -> Result<String, DocumentError> {
    let mut at = 0;
    let mut markers = Vec::new();
    loop {
        let quote = crate::document::markdown_quotes::prefix(&text[at..]);
        if quote > 0 { at += quote; continue; }
        let Some((bytes, columns)) = crate::document::markdown_blocks::marker_prefix_geometry(&text[at..]) else { break; };
        if nonempty_body && at + bytes == text.len() && !text.ends_with([' ', '\t']) { break; }
        markers.push((at..at + bytes, columns));
        at += bytes;
    }
    let Some((last, _)) = markers.last() else { return Err(DocumentError::AmbiguousProjection); };
    let marker = &text[last.clone()];
    let indentation = marker.len() - marker.trim_start_matches([' ', '\t']).len();
    let delimiter = marker[indentation..].chars().find(|ch| !ch.is_ascii_digit())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut result = String::new();
    let mut copied = 0;
    for (range, columns) in &markers[..markers.len() - 1] {
        result.push_str(&text[copied..range.start]);
        result.push_str(&" ".repeat(*columns));
        copied = range.end;
    }
    result.push_str(&text[copied..last.start + indentation]);
    if ordered { result.push_str(&ordinal.to_string()); }
    result.push(delimiter);
    result.push(' ');
    result.push_str(&text[last.end..at]);
    Ok(result)
}

impl Document {
    /// Resolve continuation syntax from the owning item, including separator
    /// paragraphs whose own physical row has no marker or indentation.
    pub(super) fn markdown_list_continuation_context(
        &self,
        at: usize,
    ) -> Result<(crate::document::Block, crate::document::encoding::DecodedText, String), ModelTransactionError> {
        let block = edit_boundary::paragraph_at(self, at)?
            .ok_or(DocumentError::AmbiguousProjection)?;
        let blocks = self.projection().list_indentation_blocks();
        let index = blocks.index_touching_point(at).ok_or(DocumentError::AmbiguousProjection)?;
        let level = block.list_level().ok_or(DocumentError::UnsupportedFormatting)?;
        let key = u16::from(level) + 1;
        let owner_index = blocks.find_navigation(0..index + 1, true, |summary| {
            summary.minimum < key || summary.minimum_start <= key
        }).ok_or(DocumentError::AmbiguousProjection)?;
        let owner = blocks.get(owner_index).filter(|owner| {
            matches!(owner.kind, BlockKind::ListItem { level: owner_level, item_start: true, .. } if owner_level == level)
        }).ok_or(DocumentError::AmbiguousProjection)?;
        let body = self.projection().source_insertion_point(owner.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = self.state().source_hard_lines.line_at_offset(body)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(line.start..body)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let mut remaining = decoded.text.as_str();
        let mut continuation = String::new();
        let mut found_marker = false;
        // Quote and list containers can interleave in either order on one
        // physical row. Keep quote syntax in place and replace every owned
        // marker with its continuation indentation.
        loop {
            let quote = crate::document::markdown_quotes::prefix(remaining);
            if quote > 0 {
                continuation.push_str(&remaining[..quote]);
                remaining = &remaining[quote..];
            } else if let Some((bytes, columns)) = crate::document::markdown_blocks::marker_prefix_geometry(remaining) {
                // This slice stops at the first visible character. A trailing
                // inline emphasis opener is not another bare list marker.
                if !owner.range.is_empty() && bytes == remaining.len()
                    && !remaining.ends_with([' ', '\t']) {
                    break;
                }
                continuation.push_str(&" ".repeat(columns));
                remaining = &remaining[bytes..];
                found_marker = true;
            } else {
                break;
            }
        }
        if !found_marker {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        // Later paragraphs can enter or leave quotes inside this item.
        // Keep the item's indentation while matching the current quote owner,
        // rather than inheriting the first paragraph's inner quote scopes.
        if block.quote_depth > owner.quote_depth {
            continuation.push_str(&"> ".repeat(block.quote_depth - owner.quote_depth));
        } else {
            for _ in block.quote_depth..owner.quote_depth {
                let quote = continuation.rfind('>').ok_or(DocumentError::AmbiguousProjection)?;
                continuation.truncate(quote);
            }
        }
        Ok((owner, decoded, continuation))
    }

    /// Whether the caret can create another paragraph in its existing item.
    /// Literal views, code bodies and table cells keep their own break actions.
    pub fn can_insert_list_paragraph(&self, at: usize) -> bool {
        self.format() == Format::Markdown
            && self.text_point(at).is_ok()
            && self.projection().table_cell_at(at).is_none()
            && edit_boundary::paragraph_at(self, at).ok().flatten().is_some_and(|block| {
                block.list_level().is_some()
                    && block.style.0 != "Code Block"
                    && !block.markdown_html
                    && !block.thematic_break
            })
    }

    pub(super) fn prepare_list_paragraph(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.text_point(at)?;
        if !self.can_insert_list_paragraph(at) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let folded_source = if self.projection().text_tree().slice(at..at.saturating_add(1)).as_deref() == Ok(" ")
            && !self.projection().style_spans_for_region(&(at..at + 1)).iter().any(|span|
                span.application == StyleApplication::Semantic(SemanticInlineStyle::Code))
        {
            if let Some(source) = self.projection().source_range(at..at + 1) {
                let bytes = self.state().source.bytes_in(source.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, source.start)?;
                (!normalize(&decoded, self.file_format()).endings.is_empty()).then_some(source)
            } else { None }
        } else { None };
        let folded_prefix_end = if let Some(source) = &folded_source {
            let next = self.projection().source_insertion_point(at + 1, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let row = self.state().source_hard_lines.line_at_offset(next)
                .and_then(|index| self.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = self.state().source.bytes_in(row.start..next)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, row.start)?;
            // Folded-space provenance can stop before an inner quote marker.
            // The new separator already carries that container prefix; consume
            // only its old structural spelling, leaving inline openers intact.
            let prefix = crate::document::markdown_quotes::container_prefix(&decoded.text);
            Some(source.end.max(decoded.source_boundary(prefix)
                .ok_or(DocumentError::AmbiguousProjection)?))
        } else { None };
        let block = edit_boundary::paragraph_at(self, at)?
            .ok_or(DocumentError::AmbiguousProjection)?;
        let (owner, decoded, continuation) = self.markdown_list_continuation_context(at)?;
        let mut separator = format!("\n{}\n{continuation}", continuation.trim_end_matches([' ', '\t']));
        if at == block.range.start {
            separator.insert_str(0, "<span></span>");
            if block.range.is_empty() && owner.id == block.id && !decoded.text.ends_with([' ', '\t']) {
                separator.insert(0, ' ');
            }
        }
        if at == block.range.end {
            // Blank indented lines alone are trivia, and cannot preserve an
            // editable continuation on save/reopen. This passive empty inline
            // body gives the paragraph an exact source-backed owner.
            separator.push_str("<span></span>");
        } else if let Some(level) = block.style.0.strip_prefix("Heading")
            .and_then(|level| level.parse::<usize>().ok()).filter(|level| (1..=6).contains(level))
        {
            separator.push_str(&"#".repeat(level));
            separator.push(' ');
        }
        let source_at = self.projection().source_insertion_point(
            at, at == block.range.start || affinity == BoundaryAffinity::Downstream && at < block.range.end,
        ).ok_or(DocumentError::AmbiguousProjection)?;
        // A folded ending contributes one visible space. Replace that exact
        // source run with the new separator instead of first deleting it in a
        // scratch document, which would require an unrelated full reprojection.
        let edit = TextEdit::new(at..at + usize::from(folded_source.is_some()), "\n");
        let mut patches = markdown_split::patches_with_separator(
            self, &edit.range, &edit.replacement, &separator, Some(source_at),
        )?.unwrap_or(vec![SourcePatch::primary(
            folded_source.clone().unwrap_or(source_at..source_at),
            self.encoding().encode_fragment(&spell_logical_breaks(&separator, self.file_format()))?,
        )]);
        if let (Some(folded), Some(end)) = (&folded_source, folded_prefix_end) {
            let patch = patches.iter_mut().find(|patch|
                patch.range.start <= folded.start && folded.end <= patch.range.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            patch.range.end = patch.range.end.max(end);
        }
        if block.range.is_empty() && block.kind == BlockKind::Paragraph {
            self.preserve_empty_list_body_prefix(source_at, &continuation, &mut patches)?;
        }
        self.preserve_markdown_edit_boundaries(
            std::slice::from_ref(&edit), std::iter::once(&edit.range), &mut patches,
        )?;
        let prepared = self.prepare_text_edits_with_patches(vec![edit], Some(patches))?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let before = candidate.projection.blocks_for_region(&(at..at)).into_iter()
            .find(|paragraph| paragraph.range.start <= at && at <= paragraph.range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        let after = candidate.projection.blocks_for_region(&(at + 1..at + 1)).into_iter()
            .find(|paragraph| paragraph.range.start == at + 1)
            .ok_or(DocumentError::VerificationFailed)?;
        let BlockKind::ListItem { ordered, ordinal, level, .. } = owner.kind else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let mut retained_kind = block.kind.clone();
        if matches!(retained_kind, BlockKind::Paragraph) {
            retained_kind = owner.kind.clone();
            if let BlockKind::ListItem { item_start, container_start, .. } = &mut retained_kind {
                *item_start = false;
                *container_start = false;
            }
        }
        if !matches!(after.kind, BlockKind::ListItem {
            ordered: next_ordered, ordinal: next_ordinal, level: next_level, item_start: false, ..
        } if next_ordered == ordered && next_ordinal == ordinal && next_level == level)
            || before.kind != retained_kind
            || before.list_item_owner() != after.list_item_owner()
            || before.quote_depth != block.quote_depth || after.quote_depth != block.quote_depth
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    /// A separator caret can follow existing blank-row indentation. Reusing
    /// that prefix avoids doubling it when the empty body gains inline syntax.
    fn preserve_empty_list_body_prefix(
        &self,
        source_at: usize,
        continuation: &str,
        patches: &mut [SourcePatch],
    ) -> Result<(), ModelTransactionError> {
        let line = self.state().source_hard_lines.line_at_offset(source_at)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self.state().source.bytes_in(line.start..source_at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        if decoded.text == continuation { return Ok(()); }
        if !decoded.text.chars().all(|ch| matches!(ch, ' ' | '\t' | '>')) {
            return Err(DocumentError::AmbiguousProjection.into());
        }
        // Equivalent tab spelling already supplies the required columns.
        // Preserve it instead of rewriting untouched blank-row trivia.
        let expanded = |prefix: &str| {
            let mut result = String::new();
            for ch in prefix.chars() {
                if ch == '\t' {
                    for _ in 0..4 - result.len() % 4 { result.push(' '); }
                } else { result.push(ch); }
            }
            result
        };
        if expanded(&decoded.text) == expanded(continuation) { return Ok(()); }
        let patch = patches.iter_mut().find(|patch| patch.range.start == source_at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut replacement = self.encoding().encode_fragment(continuation)?;
        replacement.extend_from_slice(&patch.replacement);
        patch.range.start = line.start;
        patch.replacement = replacement;
        Ok(())
    }

    /// Removing an item's label releases its continuation paragraphs too.
    /// Their source indentation belongs to the item, not to the visible body.
    pub(super) fn prepare_markdown_list_as_prose(
        &self,
        range: &std::ops::Range<usize>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !self.format().is_markdown() {
            return Ok(None);
        }
        // Only the lists around the edit, without a whole-store copy.
        let window = self.projection().list_window(range);
        let structure = &window.structure;
        let blocks = &window.blocks;
        let block_indices = blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, index))
            .collect::<std::collections::HashMap<_, _>>();
        let list_indices = structure
            .lists
            .iter()
            .enumerate()
            .map(|(index, list)| (list.id, index))
            .collect::<std::collections::HashMap<_, _>>();
        let selected = structure
            .lists
            .iter()
            .flat_map(|list| &list.items)
            .filter(|item| {
                block_indices.get(&item.paragraph_id).is_some_and(|index| {
                    let first = &blocks[*index];
                    if range.is_empty() {
                        first.range.start <= range.start && range.start <= first.range.end
                    } else {
                        first.range.start < range.end && range.start < first.range.end
                    }
                })
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(None);
        }
        if self.format() == Format::Markdown
            && !selected.iter().any(|item| {
                item.paragraph_ids.len() > 1
                    || !item.child_lists.is_empty()
                    || self
                        .projection()
                        .source_range(blocks[block_indices[&item.paragraph_id]].range.clone())
                        .is_some_and(|source| {
                            self.state().source_hard_lines.line_at_offset(source.start)
                                != self
                                    .state()
                                    .source_hard_lines
                                    .line_at_offset(source.end.saturating_sub(1).max(source.start))
                        })
            })
        {
            return Ok(None);
        }
        let mut patches = Vec::new();
        for item in selected {
            let first = &blocks[block_indices[&item.paragraph_id]];
            let mut ids = item.paragraph_ids.clone();
            let mut children = item.child_lists.clone();
            while let Some(id) = children.pop() {
                if let Some(index) = list_indices.get(&id) {
                    let list = &structure.lists[*index];
                    for child in &list.items {
                        ids.extend_from_slice(&child.paragraph_ids);
                        children.extend_from_slice(&child.child_lists);
                    }
                }
            }
            let last_index = ids
                .iter()
                .filter_map(|id| block_indices.get(id))
                .max()
                .unwrap();
            let last = &blocks[*last_index];
            let source_at = self
                .projection()
                .source_insertion_point(first.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_end = if last.range.is_empty() {
                self.projection()
                    .source_insertion_point(last.range.start, true)
            } else {
                self.projection()
                    .source_range(last.range.clone())
                    .map(|source| source.end.saturating_sub(1))
            }
            .ok_or(DocumentError::AmbiguousProjection)?;
            let lines = &self.state().source_hard_lines;
            let begin = lines
                .line_at_offset(source_at)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let end = lines
                .line_at_offset(source_end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut code_sources = Vec::new();
            for id in &ids {
                if let Some(code) = crate::document::markdown_indented_code::source_block(
                    self,
                    &blocks[block_indices[id]],
                )? {
                    code_sources.push(code.source);
                }
            }
            let mut item_indent = 0;
            for index in begin..=end {
                let line = lines.get(index).ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self
                    .state()
                    .source
                    .bytes_in(line.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, line.start)?;
                let quote = super::super::markdown_quotes::prefix(&decoded.text);
                let body = &decoded.text[quote..];
                let mut replacement = String::new();
                let remove = if index == begin {
                    let prefix = super::super::markdown_blocks::marker_prefix_length(body)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    item_indent = body[..prefix].bytes().fold(0, |column, byte| {
                        column + if byte == b'\t' { 4 - column % 4 } else { 1 }
                    });
                    prefix
                } else if code_sources
                    .iter()
                    .any(|code| code.start <= line.start && line.start <= code.end)
                    && body
                        .bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .any(|byte| byte == b'\t')
                {
                    // Moving a tab to another column changes its width. Rewrite
                    // the indentation as spaces to retain every residual column.
                    let mut columns: usize = 0;
                    let count = body
                        .bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .inspect(|byte| columns += if *byte == b'\t' { 4 - columns % 4 } else { 1 })
                        .count();
                    replacement = " ".repeat(columns.saturating_sub(item_indent));
                    count
                } else {
                    let mut column = 0;
                    body.bytes()
                        .take_while(|byte| {
                            if column >= item_indent || !matches!(byte, b' ' | b'\t') {
                                return false;
                            }
                            column += if *byte == b'\t' { 4 - column % 4 } else { 1 };
                            true
                        })
                        .count()
                };
                if remove > 0 {
                    let start = line.start
                        + self
                            .encoding()
                            .encode_fragment(&decoded.text[..quote])?
                            .len();
                    let count = self.encoding().encode_fragment(&body[..remove])?.len();
                    patches.push(SourcePatch::primary(
                        start..start + count,
                        self.encoding().encode_fragment(&replacement)?,
                    ));
                }
            }
            patches.extend(super::markdown_block_styles::support_patches(
                self,
                &(first.range.start..last.range.end),
                false,
                true,
            )?);
        }
        if patches.is_empty() {
            return Ok(None);
        }
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        patches.dedup();
        let mut combined: Vec<SourcePatch> = Vec::new();
        for patch in patches {
            if let Some(previous) = combined.last_mut().filter(|previous| {
                previous.replacement.is_empty()
                    && patch.replacement.is_empty()
                    && patch.range.start <= previous.range.end
            }) {
                // A selected parent removes child indentation; a selected
                // child additionally removes its own label at the same source.
                previous.range.end = previous.range.end.max(patch.range.end);
            } else {
                combined.push(patch);
            }
        }
        let edits = Vec::new();
        Ok(Some(
            self.prepare_text_edits_with_patches(edits, Some(combined))?,
        ))
    }

    pub(in crate::document) fn markdown_source_separator_count(
        &self,
        boundary: std::ops::Range<usize>,
    ) -> Result<usize, DocumentError> {
        let spans = self.projection().provenance_for_region(&boundary);
        let first = spans
            .iter()
            .find(|span| !span.formatted.is_empty())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let last = spans
            .iter()
            .rev()
            .find(|span| !span.formatted.is_empty())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self
            .state()
            .source
            .bytes_in(first.source.start..last.source.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, first.source.start)?;
        Ok(super::normalize(&decoded, self.file_format()).endings.len())
    }

    pub(super) fn prepare_markdown_list_enter(
        &self,
        at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.projection().table_cell_at(at).is_some() {return self.prepare_text_edits(vec![TextEdit::new(at..at,"\n")]);}
        let block =
            edit_boundary::paragraph_at(self, at)?.ok_or(DocumentError::VerificationFailed)?;
        if block.list_level().is_some() && !self.is_trailing_blank_list_paragraph(&block)? {
            self.prepare_markdown_list_split(at, false)
        } else {
            self.prepare_markdown_paragraph_enter(at, &block)
        }
    }

    /// Enter on an empty nested item lifts it one level, as Backspace does;
    /// only an empty top-level item exits the list.
    pub(super) fn prepare_empty_nested_item_lift(
        &self,
        at: usize,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let Some(block) = edit_boundary::paragraph_at(self, at)? else { return Ok(None) };
        if !block.range.is_empty() || !matches!(block.kind, BlockKind::ListItem { level: 1.., item_start: true, .. }) {
            return Ok(None);
        }
        Ok(self.prepare_list_indent(at..at, true).ok())
    }

    /// An empty paragraph spelled only by blank lines after a list's last
    /// item: exiting an item leaves it, and the parser still assigns it to
    /// that item. It is an independent paragraph, not an item continuation.
    fn is_trailing_blank_list_paragraph(&self, block: &crate::document::Block) -> Result<bool, DocumentError> {
        if !block.range.is_empty() || block.kind != BlockKind::Paragraph {
            return Ok(false);
        }
        let projection = self.projection();
        let next = block.range.end + 1;
        if next <= projection.text_tree().byte_len()
            && projection.blocks_for_region(&(next..next)).iter()
                .any(|following| following.range.start == next && following.list_level().is_some()
                    && !matches!(following.kind, BlockKind::ListItem { container_start: true, level: 0, .. }))
        {
            return Ok(false);
        }
        let Some(source) = projection.source_insertion_point(block.range.start, true) else { return Ok(false) };
        let lines = &self.state().source_hard_lines;
        let Some(line) = lines.line_at_offset(source).and_then(|index| lines.get(index)) else { return Ok(false) };
        let bytes = self.state().source.bytes_in(line.clone()).ok_or(DocumentError::AmbiguousProjection)?;
        Ok(self.encoding().decode_region(&bytes, line.start)?.text.trim().is_empty())
    }

    fn prepare_markdown_paragraph_enter(
        &self,
        at: usize,
        block: &crate::document::Block,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let following_override = (at < block.range.end).then(|| block.style.clone());
        self.prepare_open_paragraph_with_following(at, at, true, following_override)
    }

    pub(super) fn prepare_open_line(
        &self,
        at: usize,
        origin: usize,
        after: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&(at..at))?;
        self.validate_range(&(origin..origin))?;
        if let Some((prepared, _)) = self.prepare_literal_terminal_break(at, "")? {
            return Ok(prepared);
        }
        if self.format().is_markdown() {
            // Table cells and code (including front matter) take Enter's
            // in-owner hard break: a new row inside the cell or a literal line
            // inside the code, never a following paragraph outside it.
            let code = edit_boundary::paragraph_at(self, origin)?
                .map(|block| edit_boundary::is_code_paragraph(self, &block))
                .transpose()?
                .unwrap_or(false);
            if code || self.projection().table_cell_at(origin).is_some() {
                let payload = FormattedTextPayload::new(&self.hard_line_snapshot(), "\n", vec![0])
                    .map_err(|_| DocumentError::VerificationFailed)?;
                return self.prepare_formatted_payload_edits(vec![FormattedPayloadEdit::new(at..at, payload)]);
            }
            return self.prepare_open_paragraph(at, origin, after);
        }
        self.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], None)
    }

    /// A line-open is one paragraph intention: split in the native format,
    /// then assign the originating paragraph's following style to the new side.
    /// Scratch composition keeps both steps one verified, local source edit.
    fn prepare_open_paragraph(
        &self,
        at: usize,
        origin: usize,
        after: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_open_paragraph_with_following(at, origin, after, None)
    }

    fn prepare_open_paragraph_with_following(
        &self,
        at: usize,
        origin: usize,
        after: bool,
        following_override: Option<StyleId>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let block =
            edit_boundary::paragraph_at(self, at)?.ok_or(DocumentError::VerificationFailed)?;
        if self.format() == Format::Markdown
            && matches!(block.kind, BlockKind::ListItem { ordered: true, ordinal: 999_999_999.., .. })
        {
            // Once the generated decimal spelling becomes literal, the
            // retained body takes its parsed paragraph treatment. Applying an
            // old generated list style to it would recreate the retired list.
            return self.prepare_markdown_list_split(at, true);
        }
        let heading_start = if self.format() == Format::Markdown
            && at == block.range.start
            && block.quote_depth == 0
        {
            match block.kind { BlockKind::Heading(level) => Some(level), _ => None }
        } else { None };
        let support = if heading_start.is_some() {
            Vec::new()
        } else {
            self.markdown_structural_support(&[TextEdit::new(at..at, "\n")])?
        };
        if !support.is_empty() {
            return self.prepare_markdown_supporting_patches(support, |doc| {
                doc.prepare_open_paragraph_with_following(at, origin, after, following_override)
            });
        }
        let origin =
            edit_boundary::paragraph_at(self, origin)?.ok_or(DocumentError::VerificationFailed)?;
        // Raw separator paragraphs acquire the generated item style when an
        // explicit empty body is materialized, without changing their owner.
        let materialized_style = |paragraph: &crate::document::Block| -> Result<StyleId, DocumentError> {
            if paragraph.range.is_empty() && paragraph.kind == BlockKind::Paragraph && paragraph.list_level().is_some()
                && !self.is_trailing_blank_list_paragraph(paragraph)? {
                if let Some(level) = paragraph.list_level() {
                    if let Some(ordered) = paragraph.containers.iter().rev().find_map(|member| {
                        if let ContainerKind::List { ordered } = member.container.kind { Some(ordered) }
                        else { None }
                    }) {
                        return Ok(self.projection().style_sheet().list_style_id(ordered, level));
                    }
                }
            }
            Ok(paragraph.style.clone())
        };
        let retained_style = materialized_style(&block)?;
        let origin_style = materialized_style(&origin)?;
        let next = following_override.unwrap_or_else(|| {
            self.projection()
                .style_sheet()
                .next_paragraph_style(&origin_style)
                .cloned()
                .unwrap_or_else(|_| origin_style.clone())
        });
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let split = if let Some(level) = heading_start {
            let source_at =
                crate::document::source_edit::insertion_point(scratch.projection(), at, None)
                    .ok_or(DocumentError::AmbiguousProjection)?;
            let source_line = scratch
                .state()
                .source_hard_lines
                .line_at_offset(source_at)
                .and_then(|index| scratch.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            // Opening before a heading must precede its complete source
            // owner. Inserting after its prefix would expose the retained
            // closing hashes or leave a Setext underline on the wrong body.
            let syntax = format!(
                "{} {}",
                "#".repeat(level as usize),
                scratch.file_format().spelling().repeat(2)
            );
            scratch.prepare_text_edits_with_patches(
                vec![TextEdit::new(at..at, "\n")],
                Some(vec![SourcePatch::primary(
                    source_line.start..source_line.start,
                    scratch.encoding().encode_fragment(&syntax)?,
                )]),
            )?
        } else if block.range.is_empty() && block.kind == BlockKind::Paragraph
            && block.list_level().is_some() && !self.is_trailing_blank_list_paragraph(&block)? {
            scratch.prepare_list_paragraph(at, BoundaryAffinity::Downstream)?
        } else if matches!(block.kind, BlockKind::ListItem { .. }) {
            if self.format() == Format::Markdown {
                scratch.prepare_markdown_list_split(at, true)?
            } else {
                scratch.prepare_structural_list_enter(at)?
            }
        } else if self.format() == Format::Markdown
            && (block.style.0 == "Block quote" || block.quote_depth > 0)
        {
            scratch
                .prepare_markdown_quote_enter(at)?
                .ok_or(DocumentError::AmbiguousProjection)?
        } else {
            scratch.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], None)?
        };
        let before = split
            .text_position_map()
            .map_text_point(
                scratch.text_point(at)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        let inserted = split
            .summary()
            .formatted_splices()
            .iter()
            .find(|splice| splice.old_range() == (at..at))
            .ok_or(DocumentError::VerificationFailed)?
            .inserted_len();
        let following = before + inserted;
        let split_splices = split.summary().formatted_splices().to_vec();
        record_open_paragraph_step(&mut scratch, split, &mut sources)?;
        // The opposite side retains the original paragraph's style even when
        // an adapter's ordinary split defaults a heading to body text.
        for (point, style) in if after {
            [(before, retained_style.clone()), (following, next.clone())]
        } else {
            [(following, retained_style.clone()), (before, next.clone())]
        } {
            let actual = scratch
                .projection()
                .blocks_for_region(&(point..point))
                .into_iter()
                .find(|block| block.range.start <= point && point <= block.range.end)
                .ok_or(DocumentError::VerificationFailed)?;
            // Image is derived from the retained object, not an authored
            // paragraph assignment for the empty side of a split.
            let style = if style.0 == "Image" {
                if actual.style.0 == "Image" { actual.style.clone() }
                else { scratch.projection().style_sheet().base_paragraph.clone() }
            } else { style };
            if actual.style != style {
                let styled = scratch.prepare_markdown_paragraph_style(actual.range, style)?;
                if let PreparedPublication::State(candidate) = &styled.publication {
                    if candidate.projection.text() != scratch.text() {
                        return Err(DocumentError::VerificationFailed.into());
                    }
                }
                record_open_paragraph_step(&mut scratch, styled, &mut sources)?;
            }
        }
        let patches = sources.source_patches();
        // Keep the authored break distinct from adjacent support replacements.
        // Coalescing them makes BeforeInsertion cross the break for O when an
        // old collapsed space is replaced by NBSP at that same boundary.
        let mut delta = 0isize;
        let edits = split_splices
            .iter()
            .map(|splice| {
                let range = splice.old_range();
                let start = range
                    .start
                    .checked_add_signed(delta)
                    .ok_or(DocumentError::VerificationFailed)?;
                let replacement = scratch
                    .projection()
                    .text_tree()
                    .slice(start..start + splice.inserted_len())
                    .map_err(DocumentError::FormattedTextStorage)?;
                delta += splice.inserted_len() as isize - range.len() as isize;
                Ok(TextEdit::new(range, replacement))
            })
            .collect::<Result<Vec<_>, DocumentError>>()?;
        let prepared = self.prepare_text_edits_with_patches(edits.clone(), Some(patches))?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        // Both results retain the untouched blocks and styles as the same
        // shifted structure, so compare the edited extent with its complete
        // neighboring blocks, and the totals, rather than the whole document.
        let (actual_projection, expected_projection) = (&candidate.projection, scratch.projection());
        let delta = edits.iter().map(|edit| edit.replacement.len() as isize - edit.range.len() as isize).sum::<isize>();
        let changed = edits.first().map_or(at, |edit| edit.range.start)
            ..edits.last().map_or(at, |edit| edit.range.end).checked_add_signed(delta)
                .ok_or(DocumentError::VerificationFailed)?;
        let length = actual_projection.text_tree().byte_len();
        let around = changed.start.saturating_sub(1)..changed.end.saturating_add(1).min(length);
        let actual = actual_projection.blocks_for_region(&around);
        let expected = expected_projection.blocks_for_region(&around);
        let extent = actual.first().map_or(around.start, |block| block.range.start.min(around.start))
            ..actual.last().map_or(around.end, |block| block.range.end.max(around.end));
        if length != expected_projection.text_tree().byte_len()
            || actual_projection.block_count() != expected_projection.block_count()
            || actual_projection.style_span_count() != expected_projection.style_span_count()
            || actual.len() != expected.len()
            || actual.iter().zip(&expected).any(|(a, b)| {
                a.range != b.range
                    || a.kind != b.kind
                    || a.style != b.style
                    || a.direct_paragraph != b.direct_paragraph
                    || a.direct_default_character != b.direct_default_character
            })
            || actual_projection.style_spans_touching(&extent) != expected_projection.style_spans_touching(&extent)
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    fn prepare_markdown_list_split(
        &self,
        at: usize,
        force_new_item: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let block = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| block.range.start <= at && at <= block.range.end)
            .ok_or(DocumentError::VerificationFailed)?;
        let window = self.projection().list_window(&(at..at));
        let lists = &window.structure;
        let item = lists.lists.iter().flat_map(|list| &list.items)
            .find(|item| item.paragraph_ids.contains(&block.id))
            .ok_or(DocumentError::AmbiguousProjection)?;
        if block.range.is_empty() && !force_new_item
            && matches!(block.kind, BlockKind::ListItem { item_start: true, .. })
            && item.paragraph_ids.len() == 1 && item.child_lists.is_empty()
        {
            return self.prepare_list_style(block.range.clone(), None);
        }
        let first_id = item.paragraph_id;
        let first_paragraph = window
            .blocks
            .iter()
            .find(|candidate| candidate.id == first_id)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let BlockKind::ListItem { ordered, ordinal, .. } = first_paragraph.kind else {
            return Err(DocumentError::UnsupportedFormatting.into());
        };
        let body_source = self
            .projection()
            .source_insertion_point(first_paragraph.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let source_line = self
            .state()
            .source_hard_lines
            .line_at_offset(body_source)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self
            .state()
            .source
            .bytes_in(source_line.start..body_source)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, source_line.start)?;
        let prefix = sibling_prefix(&decoded.text, !first_paragraph.range.is_empty(), ordered, ordinal.saturating_add(1))?;
        let replacement = self.encoding().encode_fragment(&format!(
            "{}{prefix}",
            self.file_format().spelling()
        ))?;
        let source_at = self
            .projection()
            .source_insertion_point(at, at == block.range.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let edit = TextEdit::new(at..at, "\n");
        let mut range = source_at..source_at;
        let mut replacement = replacement;
        let inner_quote = block.containers.iter().rposition(|member|
            member.container.kind == ContainerKind::ListItem)
            .is_some_and(|item| block.containers[item + 1..].iter().any(|member|
                member.container.kind == ContainerKind::Quote));
        if inner_quote {
            // Bare quote syntax within the item has no paragraph leaf.
            // Materialize either empty side of the split without changing
            // its list or quote ownership or adding visible content.
            if at == block.range.start && block.id == first_paragraph.id {
                let mut body = self.encoding().encode_fragment("<span></span>")?;
                body.extend(replacement);
                replacement = body;
            }
            if at == block.range.end {
                replacement.extend(self.encoding().encode_fragment("<span></span>")?);
            }
        }
        let mut materializes_empty_body = block.range.is_empty() && block.kind == BlockKind::Paragraph;
        if at == block.range.start && block.id != first_paragraph.id {
            if matches!(block.kind, BlockKind::Paragraph) {
                // An editable raw separator has no source body to retain.
                // Keep its body before appending the new item's marker.
                let mut support = self.encoding().encode_fragment("<span></span>")?;
                support.extend(replacement);
                replacement = support;
            } else if block.range.is_empty() {
                // Keep an existing passive empty body with the old item.
                // A physically blank quoted continuation needs such a body
                // too; otherwise its old paragraph disappears on the split.
                let current = self.state().source_hard_lines.line_at_offset(source_at)
                    .and_then(|line| self.state().source_hard_lines.get(line))
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self.state().source.bytes_in(current.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, current.start)?;
                let end = decoded.text.trim_end_matches(['\r', '\n']).len();
                let at = decoded.source_boundary(end).ok_or(DocumentError::AmbiguousProjection)?;
                range = at..at;
                if crate::document::markdown_quotes::container_prefix(&decoded.text[..end]) == end {
                    materializes_empty_body = true;
                    let mut support = self.encoding().encode_fragment("<span></span>")?;
                    support.extend(replacement);
                    replacement = support;
                }
            } else {
                let mut support = self.encoding().encode_fragment("<span></span>")?;
                support.extend(replacement);
                replacement = support;
            }
        }
        let mut patches = vec![SourcePatch::primary(range, replacement)];
        if materializes_empty_body {
            let (_, _, continuation) = self.markdown_list_continuation_context(at)?;
            let body_at = patches[0].range.start;
            self.preserve_empty_list_body_prefix(body_at, &continuation, &mut patches)?;
        }
        if at == block.range.end && !block.range.is_empty() {
            let next = self.projection().blocks_for_region(&(at + 1..at + 1))
                .into_iter().find(|next| next.range.start == at + 1);
            if next.is_some_and(|next| matches!(next.kind, BlockKind::ListItem { item_start: false, .. })) {
                // A blank line after a newly empty marker ends that item in
                // GFM. Its following body already supplies the next paragraph,
                // so retain one ending and keep that body within the new item.
                if let Some(row) = self.state().source_hard_lines.line_at_offset(source_at)
                    .and_then(|index| self.state().source_hard_lines.get(index + 1)) {
                    let bytes = self.state().source.bytes_in(row.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let decoded = self.encoding().decode_region(&bytes, row.start)?;
                    if decoded.text.trim_matches(['\r', '\n']).is_empty() {
                        patches.push(SourcePatch::primary(row, Vec::new()));
                    }
                }
            }
        }
        self.preserve_markdown_edit_boundaries(
            std::slice::from_ref(&edit),
            std::iter::once(&edit.range),
            &mut patches,
        )?;
        if ordered && ordinal >= 999_999_999 {
            // The next decimal spelling is ordinary CommonMark text. Its
            // visible marker and continuation ownership are intentionally
            // determined by the same source parser used on reopen.
            return self.prepare_reprojected_source_patches(patches);
        }
        self.prepare_text_edits_with_patches(vec![edit], Some(patches))
    }
}

fn record_open_paragraph_step(
    scratch: &mut Document,
    prepared: PreparedModelTransaction,
    sources: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    scratch.commit_model_transaction(prepared)?;
    Ok(())
}
