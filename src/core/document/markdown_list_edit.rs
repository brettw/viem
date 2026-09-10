//! Source-local list continuation. Labels are source syntax and paragraph
//! decorations; the formatted intention contains only the new paragraph break.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{edit_boundary, BlockKind};

impl Document {
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
            return self.prepare_text_edits_with_patches(
                vec![empty_edit],
                Some(vec![SourcePatch::primary(
                    source_at..source_at,
                    self.encoding()
                        .encode_fragment(&self.file_format().spelling().repeat(endings))?,
                )]),
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
                return self.prepare_text_edits_with_patches(
                    vec![edit],
                    Some(vec![SourcePatch::primary(
                        source_at..source_at,
                        self.encoding()
                            .encode_fragment(&self.file_format().spelling().repeat(2))?,
                    )]),
                );
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
        self.prepare_text_edits_with_patches(
            vec![edit],
            Some(vec![SourcePatch::primary(
                source,
                self.encoding().encode_fragment(&syntax)?,
            )]),
        )
    }

    pub(super) fn prepare_markdown_list_enter(
        &self,
        at: usize,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_markdown_list_split(at, false)
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
        if self.format() == crate::document::Format::MarkdownSource
            && !self
                .projection()
                .blocks_for_region(&(at..at))
                .iter()
                .any(|block| block.style.0 == "Code Block" && at < block.range.end)
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
            return self.prepare_text_edits_with_patches(
                vec![TextEdit::new(at..at, "\n")],
                Some(vec![SourcePatch::primary(
                    source_at..source_at,
                    self.encoding()
                        .encode_fragment(&self.file_format().spelling().repeat(count))?,
                )]),
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
        let origin = edit_boundary::paragraph_at(self, origin)?.ok_or(DocumentError::VerificationFailed)?;
        let block = edit_boundary::paragraph_at(self, at)?.ok_or(DocumentError::VerificationFailed)?;
        let next = self
            .projection()
            .style_sheet()
            .block_style(&origin.style)
            .and_then(|style| style.next_paragraph_style.clone())
            .unwrap_or_else(|| origin.style.clone());
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let split = if matches!(block.kind, BlockKind::ListItem { .. }) {
            if self.format() == Format::Markdown {
                scratch.prepare_markdown_list_split(at, true)?
            } else {
                scratch.prepare_rich_list_enter_with_empty_policy(at, false)?
            }
        } else if self.format() == Format::Html {
            scratch.prepare_html_paragraph_split(at, &block, &block.style)?
        } else if self.format() == Format::Markdown && block.style.0 == "Code Block" {
            scratch.prepare_markdown_code_open(at, &block, after)?
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
                ending.repeat(2)
            } else {
                format!("{ending}{closing_text}{ending}{ending}")
            };
            (closing_end..closing_end, syntax)
        } else if !after && at == block.range.start {
            (opening.start..opening.start, ending.repeat(2))
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
        let prefix_len = crate::document::markdown_blocks::marker_prefix_length(&decoded.text)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let prefix = &decoded.text[..prefix_len];
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
            "{}{indentation}{label}",
            self.file_format().spelling()
        ))?;
        let source_at = self
            .projection()
            .source_insertion_point(at, at == block.range.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        self.prepare_text_edits_with_patches(
            vec![TextEdit::new(at..at, "\n")],
            Some(vec![SourcePatch::primary(
                source_at..source_at,
                replacement,
            )]),
        )
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
