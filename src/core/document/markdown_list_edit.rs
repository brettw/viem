//! Source-local list continuation. Labels are source syntax and paragraph
//! decorations; the formatted intention contains only the new paragraph break.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{edit_boundary, BlockKind};

impl Document {
    /// A source-authoring command specifies exact syntax patches. Parsing can
    /// fold separators or change adjacent blocks, so derive that effective
    /// text change before verifying and publishing the transaction.
    pub(super) fn prepare_markdown_source_syntax_edit(
        &self,
        edit: TextEdit,
        patches: Vec<SourcePatch>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if let Some(reparsed) = self.markdown_source_reparsed_edit(&edit, &patches)? {
            match self.prepare_text_edits_with_patches(vec![reparsed], Some(patches.clone())) {
                Err(ModelTransactionError::Document(DocumentError::VerificationFailed)) => {}
                result => return result,
            }
        }
        self.prepare_reprojected_source_patches(patches)
    }


    /// Source-visible list authoring uses the parsed item's owner, including
    /// continuation paragraphs, rather than recognizing only a literal `1. `.
    /// `open` distinguishes o/O from Enter: opening never exits an empty item.
    pub(crate) fn markdown_source_list_edit(
        &self,
        at: usize,
        origin: usize,
        open: Option<bool>,
    ) -> Result<Option<TextEdit>, DocumentError> {
        if self.format() != Format::MarkdownSource || self.markdown_source_code_enter(origin)? {
            return Ok(None);
        }
        let blocks = self.projection().list_indentation_blocks(true);
        let Some(index) = blocks.index_touching_point(origin) else { return Ok(None); };
        let Some(block) = blocks.get(index) else { return Ok(None); };
        let BlockKind::ListItem { ordered, ordinal, level, .. } = block.kind else {
            return Ok(None);
        };
        let key = u16::from(level) + 1;
        let mut end = index + 1;
        let owner = loop {
            let Some(found) = blocks.find_navigation(0..end, true, |summary| {
                summary.minimum < key || summary.minimum_start <= key
            }) else { return Ok(None); };
            let Some(owner) = blocks.get(found) else { return Ok(None); };
            // Quote-only separator rows do not end an item's ownership of
            // the indented continuation paragraph after them.
            if owner.style.0 == "Block quote" && owner.kind == BlockKind::Paragraph {
                let text = self.projection().text_tree().slice(owner.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                let quote = super::super::markdown_quotes::prefix(&text);
                if quote > 0 && text[quote..].trim().is_empty() {
                    end = found;
                    continue;
                }
            }
            break owner;
        };
        if !matches!(owner.kind, BlockKind::ListItem { level: owner_level, .. } if owner_level == level) {
            return Ok(None);
        }
        let marker_line = self.projection().hard_line_at_offset(owner.range.start)
            .and_then(|index| self.projection().hard_line_range(index))
            .ok_or(DocumentError::VerificationFailed)?;
        let text = self.projection().text_tree().slice(marker_line.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let quote = super::super::markdown_quotes::prefix(&text);
        let Some(width) = super::super::markdown_blocks::marker_prefix_length(&text[quote..]) else {
            return Ok(None);
        };
        let prefix_end = quote + width;
        if open != Some(false) && at < marker_line.start + prefix_end {
            return Ok(None);
        }
        if open.is_none() && origin > marker_line.end {
            let row = self.projection().hard_line_at_offset(origin)
                .and_then(|line| self.projection().hard_line_range(line))
                .ok_or(DocumentError::VerificationFailed)?;
            let text = self.projection().text_tree().slice(row.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let quote = super::super::markdown_quotes::prefix(&text);
            if at < row.start + quote { return Ok(None); }
        }
        if open.is_none() && at <= marker_line.end && block.range.start == owner.range.start
            && text[prefix_end..].trim().is_empty()
        {
            // The quote adapter owns the supporting quote-only separator rows.
            return Ok((quote == 0).then(|| TextEdit::new(marker_line, "")));
        }
        let start = quote + text[quote..].len() - text[quote..].trim_start_matches([' ', '\t']).len();
        let digits = text[start..].bytes().take_while(u8::is_ascii_digit).count();
        let after = open.unwrap_or(true);
        let number = if after { ordinal.saturating_add(1) } else { ordinal };
        let mut marker = if ordered {
            format!("{}{number}{}", &text[..start], &text[start + digits..prefix_end])
        } else {
            text[..prefix_end].to_owned()
        };
        if !marker.ends_with([' ', '\t']) { marker.push(' '); }
        Ok(Some(TextEdit::new(at..at, if after {
            format!("\n{marker}")
        } else {
            format!("{marker}\n")
        })))
    }

    pub(crate) fn prepared_markdown_source_open_cursor(
        &self,
        prepared: &PreparedModelTransaction,
        at: usize,
        origin: usize,
        after: bool,
    ) -> Result<Option<usize>, DocumentError> {
        let list = self.markdown_source_list_edit(at, origin, Some(after))?.is_some();
        if !list && self.markdown_source_quote_code_open(at, origin, after)?.is_none() {
            return Ok(None);
        }
        let before = prepared.text_position_map().map_text_point(
            self.text_point(at)?, Association::BeforeInsertion, BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        ).map_err(|_| DocumentError::AmbiguousProjection)?
            .value().ok_or(DocumentError::AmbiguousProjection)?.offset();
        let PreparedPublication::State(state) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed);
        };
        let point = before + usize::from(after);
        let line = state.projection.hard_line_at_offset(point)
            .and_then(|index| state.projection.hard_line_range(index))
            .ok_or(DocumentError::VerificationFailed)?;
        let text = state.projection.text_tree().slice(line.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let quote = super::super::markdown_quotes::prefix(&text);
        let width = if list {
            super::super::markdown_blocks::marker_prefix_length(&text[quote..])
                .ok_or(DocumentError::VerificationFailed)?
        } else { 0 };
        Ok(Some(line.start + quote + width))
    }

    fn markdown_source_quote_code_open(
        &self,
        at: usize,
        origin: usize,
        after: bool,
    ) -> Result<Option<TextEdit>, DocumentError> {
        if self.format() != Format::MarkdownSource { return Ok(None); }
        let Some(block) = edit_boundary::paragraph_at(self, origin)? else { return Ok(None); };
        if !super::super::markdown_quotes::is_fenced_block(self, &block)? { return Ok(None); }
        let row = self.projection().hard_line_at_offset(origin)
            .and_then(|line| self.projection().hard_line_range(line))
            .ok_or(DocumentError::VerificationFailed)?;
        let text = self.projection().text_tree().slice(row)
            .map_err(DocumentError::FormattedTextStorage)?;
        let prefix = &text[..super::super::markdown_quotes::prefix(&text)];
        Ok(Some(TextEdit::new(at..at, if after { format!("\n{prefix}") } else { format!("{prefix}\n") })))
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
        let structure = self.projection().list_structure();
        let blocks = self.projection().blocks();
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
        if selected.is_empty() { return Ok(None); }
        if self.format() == Format::Markdown && !selected.iter().any(|item| {
            item.paragraph_ids.len() > 1 || !item.child_lists.is_empty()
                || self.projection().source_range(blocks[block_indices[&item.paragraph_id]].range.clone())
                    .is_some_and(|source| self.state().source_hard_lines.line_at_offset(source.start)
                        != self.state().source_hard_lines.line_at_offset(source.end.saturating_sub(1).max(source.start)))
        }) {
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
                if let Some(code) = crate::document::markdown_indented_code::source_block(self, &blocks[block_indices[id]])? {
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
                } else if code_sources.iter().any(|code| code.start <= line.start && line.start <= code.end)
                    && body.bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).any(|byte| byte == b'\t') {
                    // Moving a tab to another column changes its width. Rewrite
                    // the indentation as spaces to retain every residual column.
                    let mut columns: usize = 0;
                    let count = body.bytes().take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .inspect(|byte| columns += if *byte == b'\t' { 4 - columns % 4 } else { 1 }).count();
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
                    patches.push(SourcePatch::primary(start..start + count, self.encoding().encode_fragment(&replacement)?));
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
        let edits = if self.format().is_source_view() {
            combined.iter().filter(|patch| !patch.range.is_empty())
                .map(|patch| {
                    let boundary = |at, affinity| {
                        self.projection().map_source_boundary(self.revision(), at, affinity)
                            .map(|point| point.formatted_offset)
                            .map_err(|_| DocumentError::AmbiguousProjection)
                    };
                    let start = boundary(patch.range.start, BoundaryAffinity::Downstream)?;
                    let end = boundary(patch.range.end, BoundaryAffinity::Upstream)?;
                    Ok(TextEdit::new(start..end, self.encoding().decode_region(&patch.replacement, patch.range.start)?.text))
                }).collect::<Result<Vec<_>, DocumentError>>()?
        } else {
            Vec::new()
        };
        Ok(Some(self.prepare_text_edits_with_patches(
            edits,
            Some(combined),
        )?))
    }

    pub(crate) fn markdown_source_empty_enter_edit(
        &self,
        at: usize,
    ) -> Result<Option<TextEdit>, DocumentError> {
        if self.format() != crate::document::Format::MarkdownSource {
            return Ok(None);
        }
        let line = self
            .projection()
            .hard_line_at_offset(at)
            .and_then(|line| self.projection().hard_line_range(line))
            .ok_or(DocumentError::VerificationFailed)?;
        if !line.is_empty()
            || self
                .projection()
                .blocks_for_region(&line)
                .iter()
                .any(|block| block.style.0 == "Code Block")
        {
            return Ok(None);
        }
        let promote = at > 0 && self.markdown_source_separator_count(at - 1..at)? == 1;
        Ok(Some(TextEdit::new(at..at, if promote { "" } else { "\n" })))
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

    pub(super) fn prepare_markdown_source_list_enter(
        &self,
        at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if self.markdown_source_list_edit(at, at, None)?.is_none() {
            if let Some(prepared) = self.prepare_markdown_quote_enter(at)? {
                return Ok(prepared);
            }
        }
        let edit = self
            .list_enter_edit(at)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if let Some(empty_edit) = self.markdown_source_empty_enter_edit(at)? {
            let source_at = self
                .projection()
                .source_insertion_point(at, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let endings = if empty_edit.replacement.is_empty() {
                1
            } else {
                2
            };
            return self.prepare_markdown_source_syntax_edit(
                empty_edit,
                vec![SourcePatch::primary(
                    source_at..source_at,
                    self.encoding()
                        .encode_fragment(&self.file_format().spelling().repeat(endings))?,
                )],
            );
        }
        if edit.range.is_empty() {
            if edit.replacement == "\n" {
                // Return creates a paragraph in source mode too. Literal
                // source newlines from text input keep their separate path.
                let source_at = self
                    .projection()
                    .source_insertion_point(at, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                // A neighboring displayed break may already own one ending
                // of the pair, such as a continuation line's single ending;
                // three consecutive endings would fold back to one break.
                let endings = self.markdown_source_enter_endings(at)?;
                let patches = vec![SourcePatch::primary(
                    source_at..source_at,
                    self.encoding()
                        .encode_fragment(&self.file_format().spelling().repeat(endings))?,
                )];
                return self.prepare_markdown_source_syntax_edit(edit, patches);
            }
            return self.prepare_text_edits_with_patches(vec![edit], None);
        }
        let source = self
            .projection()
            .source_range(edit.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut physical_breaks = 0;
        let mut visible_boundaries = 0;
        let mut neighbors = Vec::new();
        if edit.range.start > 0 {
            neighbors.push(edit.range.start - 1..edit.range.start);
        }
        if edit.range.end < self.text().len() {
            neighbors.push(edit.range.end..edit.range.end + 1);
        }
        for boundary in neighbors {
            physical_breaks += self.markdown_source_separator_count(boundary)?;
            visible_boundaries += 1;
        }
        // Empty-item exit must leave an independent prose paragraph. Each
        // visible paragraph boundary owns a pair of physical source endings.
        let additional = (visible_boundaries * 2usize).saturating_sub(physical_breaks);
        let syntax = self.file_format().spelling().repeat(additional);
        self.prepare_markdown_source_syntax_edit(
            edit,
            vec![SourcePatch::primary(
                source,
                self.encoding().encode_fragment(&syntax)?,
            )],
        )
    }

    pub(super) fn prepare_markdown_list_enter(
        &self,
        at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let block = edit_boundary::paragraph_at(self, at)?
            .ok_or(DocumentError::VerificationFailed)?;
        if matches!(block.kind, BlockKind::ListItem { .. }) {
            self.prepare_markdown_list_split(at, false)
        } else {
            self.prepare_markdown_paragraph_enter(at, &block)
        }
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
        if self.format().is_wysiwyg() {
            return self.prepare_open_paragraph(at, origin, after);
        }
        if let Some(edit) = self.markdown_source_list_edit(at, origin, Some(after))? {
            return self.prepare_text_edits_with_patches(vec![edit], None);
        }
        if let Some(edit) = self.markdown_source_quote_code_open(at, origin, after)? {
            return self.prepare_text_edits_with_patches(vec![edit], None);
        }
        if self.format() == crate::document::Format::MarkdownSource
            && !self.projection().blocks_for_region(&(at..at)).iter()
                .any(|block| at < block.range.end && edit_boundary::is_code_paragraph(self, block).unwrap_or(false))
        {
            let mut physical_breaks = 0;
            let mut visible_breaks = 0usize;
            for boundary in [
                at.checked_sub(1),
                (at < self.projection().text_tree().byte_len()).then_some(at),
            ]
            .into_iter()
            .flatten()
            {
                if self
                    .projection()
                    .hard_line_at_offset(boundary)
                    .and_then(|line| self.projection().hard_line_range(line))
                    .is_some_and(|line| {
                        line.end == boundary && boundary < self.projection().text_tree().byte_len()
                    })
                {
                    physical_breaks +=
                        self.markdown_source_separator_count(boundary..boundary + 1)?;
                    visible_breaks += 1;
                }
            }
            // A new blank source row joins the adjacent separator run. Existing
            // paired (or odd-spelled) boundaries must keep their displayed rows.
            let count = if visible_breaks == 0 {
                1
            } else {
                (2 * (visible_breaks + 1))
                    .saturating_sub(physical_breaks)
                    .max(1)
            };
            let source_at = self
                .projection()
                .source_insertion_point(at, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            return self.prepare_markdown_source_syntax_edit(
                TextEdit::new(at..at, "\n"),
                vec![SourcePatch::primary(
                    source_at..source_at,
                    self.encoding()
                        .encode_fragment(&self.file_format().spelling().repeat(count))?,
                )],
            );
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
        let origin = edit_boundary::paragraph_at(self, origin)?.ok_or(DocumentError::VerificationFailed)?;
        let block = edit_boundary::paragraph_at(self, at)?.ok_or(DocumentError::VerificationFailed)?;
        let next = following_override.unwrap_or_else(|| {
            self.projection()
                .style_sheet()
                .block_style(&origin.style)
                .and_then(|style| style.next_paragraph_style.clone())
                .unwrap_or_else(|| origin.style.clone())
        });
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let split = if self.format() == Format::Markdown && block.style.0 == "Code Block" {
            scratch.prepare_markdown_code_open(at, &block, after)?
        } else if matches!(block.kind, BlockKind::ListItem { .. }) {
            if self.format() == Format::Markdown {
                scratch.prepare_markdown_list_split(at, true)?
            } else {
                scratch.prepare_rich_list_enter_with_empty_policy(at, false)?
            }
        } else if self.format() == Format::Html {
            scratch.prepare_html_paragraph_split(at, &block, &block.style)?
        } else if self.format() == Format::Markdown && block.style.0 == "Block quote" {
            scratch.prepare_markdown_quote_enter(at)?.ok_or(DocumentError::AmbiguousProjection)?
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
            [(before, block.style.clone()), (following, next.clone())]
        } else {
            [(following, block.style.clone()), (before, next.clone())]
        } {
            let actual = scratch
                .projection()
                .blocks_for_region(&(point..point))
                .into_iter()
                .find(|block| block.range.start <= point && point <= block.range.end)
                .ok_or(DocumentError::VerificationFailed)?;
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
        let patches = sources.source_patches(&scratch.state().source)?;
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
        let prepared = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        let actual = candidate.projection.blocks();
        let expected = scratch.projection().blocks();
        if actual.len() != expected.len()
            || actual.iter().zip(expected).any(|(a, b)| {
                a.range != b.range
                    || a.kind != b.kind
                    || a.style != b.style
                    || a.direct_paragraph != b.direct_paragraph
                    || a.direct_default_character != b.direct_default_character
            })
            || candidate.projection.style_spans() != scratch.projection().style_spans()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(prepared)
    }

    fn prepare_markdown_code_open(
        &self,
        at: usize,
        block: &crate::document::Block,
        after: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if crate::document::markdown_indented_code::source_block(self, block)?.is_some() {
            return self.prepare_with_fenced_indented_code(std::slice::from_ref(block), |doc| {
                let block = edit_boundary::paragraph_at(doc, at)?.ok_or(DocumentError::AmbiguousProjection)?;
                doc.prepare_markdown_code_open(at, &block, after)
            });
        }
        let fence = crate::document::markdown_code::fenced_source(self, block)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let opening = fence.opening;
        let bytes = self
            .state()
            .source
            .bytes_in(opening.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let opening_text = self.encoding().decode_region(&bytes, opening.start)?.text;
        let opening_text = opening_text.trim_end_matches(['\r', '\n']);
        let ending = self.file_format().spelling();
        let closing_end = fence.closing_content_end.unwrap_or(fence.body.end);
        let closed = fence.closing_text.is_some();
        let closing_text = fence
            .closing_text
            .unwrap_or_else(|| (fence.delimiter as char).to_string().repeat(fence.width));
        let (range, syntax) = if after && at == block.range.end {
            let syntax = if closed {
                let count = if at < self.text().len() {
                    let next = self.projection().source_insertion_point(at + 1, true)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let bytes = self.state().source.bytes_in(closing_end..next)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let decoded = self.encoding().decode_region(&bytes, closing_end)?;
                    (4usize).saturating_sub(super::normalize(&decoded, self.file_format()).endings.len()).max(1)
                } else { 2 };
                ending.repeat(count)
            } else {
                format!("{ending}{closing_text}{ending}{ending}")
            };
            (closing_end..closing_end, syntax)
        } else if !after && at == block.range.start {
            let count = if at > 0 {
                let previous = self.projection().source_insertion_point(at - 1, false)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self.state().source.bytes_in(previous..opening.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, previous)?;
                (4usize).saturating_sub(super::normalize(&decoded, self.file_format()).endings.len()).max(1)
            } else { 2 };
            (opening.start..opening.start, ending.repeat(count))
        } else {
            let source_at = self
                .projection()
                .source_insertion_point(at, at == block.range.start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prose = |range: std::ops::Range<usize>| {
                self.text()[range]
                    .split('\n')
                    .map(|line| {
                        let mut escaped = String::new();
                        for character in line.chars() {
                            if character.is_ascii_punctuation() {
                                escaped.push('\\');
                            }
                            escaped.push(character);
                        }
                        escaped
                    })
                    .collect::<Vec<_>>()
                    .join("<br>")
            };
            if after {
                (
                    source_at..closing_end,
                    format!(
                        "{ending}{closing_text}{ending}{ending}{}",
                        prose(at..block.range.end)
                    ),
                )
            } else {
                (
                    opening.start..source_at,
                    format!(
                        "{}{ending}{ending}{opening_text}{ending}",
                        prose(block.range.start..at)
                    ),
                )
            }
        };
        self.prepare_text_edits_with_patches(
            vec![TextEdit::new(at..at, "\n")],
            Some(vec![SourcePatch::primary(
                range,
                self.encoding().encode_fragment(&syntax)?,
            )]),
        )
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
        let BlockKind::ListItem {
            ordered, ordinal, ..
        } = block.kind
        else {
            return Err(DocumentError::UnsupportedFormatting.into());
        };
        if block.range.is_empty() && !force_new_item {
            return self.prepare_list_style(block.range.clone(), None);
        }
        let lists = self.projection().list_structure();
        let first_id = lists
            .lists
            .iter()
            .flat_map(|list| &list.items)
            .find(|item| item.paragraph_ids.contains(&block.id))
            .map(|item| item.paragraph_id)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let first_paragraph = self
            .projection()
            .blocks()
            .iter()
            .find(|candidate| candidate.id == first_id)
            .ok_or(DocumentError::AmbiguousProjection)?;
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
            .bytes_in(source_line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, source_line.start)?;
        let quote = crate::document::markdown_quotes::prefix(&decoded.text);
        let quote_prefix = &decoded.text[..quote];
        let prefix_len = crate::document::markdown_blocks::marker_prefix_length(&decoded.text[quote..])
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = &decoded.text[quote..quote + prefix_len];
        let indentation_len = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
        let indentation = &prefix[..indentation_len];
        let delimiter = prefix[indentation_len..]
            .chars()
            .find(|ch| !ch.is_ascii_digit())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let label = if ordered {
            format!("{}{delimiter} ", ordinal.saturating_add(1))
        } else {
            format!("{delimiter} ")
        };
        let replacement = self.encoding().encode_fragment(&format!(
            "{}{quote_prefix}{indentation}{label}",
            self.file_format().spelling()
        ))?;
        let source_at = self
            .projection()
            .source_insertion_point(at, at == block.range.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let edit = TextEdit::new(at..at, "\n");
        let mut range = source_at..source_at;
        let mut replacement = replacement;
        if at == block.range.start && block.id != first_paragraph.id {
            let current = self.state().source_hard_lines.line_at_offset(source_at)
                .and_then(|line| self.state().source_hard_lines.get(line))
                .ok_or(DocumentError::AmbiguousProjection)?;
            range.start = current.start;
            let label = if ordered { format!("{ordinal}{delimiter} ") } else { format!("{delimiter} ") };
            let mut prefix = self.encoding().encode_fragment(&format!("{quote_prefix}{indentation}{label}"))?;
            prefix.extend(replacement);
            replacement = prefix;
        }
        let mut patches = vec![SourcePatch::primary(range, replacement)];
        self.preserve_markdown_edit_boundaries(std::slice::from_ref(&edit), std::iter::once(&edit.range), &mut patches)?;
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
