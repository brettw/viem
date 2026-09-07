//! Source-local list continuation. Labels are source syntax and paragraph
//! decorations; the formatted intention contains only the new paragraph break.
use super::{
    Document, DocumentError, ModelTransactionError, PreparedModelTransaction, SourcePatch, TextEdit,
};
use crate::document::BlockKind;

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
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&(at..at))?;
        if self.format() == crate::document::Format::MarkdownSource
            && !self.projection().blocks_for_region(&(at..at)).iter().any(|block| block.style.0 == "Code Block" && at < block.range.end)
        {
            let mut physical_breaks = 0;
            let mut visible_breaks = 0usize;
            for boundary in [at.checked_sub(1), (at < self.projection().text_tree().byte_len()).then_some(at)].into_iter().flatten() {
                if self.projection().hard_line_at_offset(boundary)
                    .and_then(|line| self.projection().hard_line_range(line))
                    .is_some_and(|line| line.end == boundary && boundary < self.projection().text_tree().byte_len())
                {
                    physical_breaks += self.markdown_source_separator_count(boundary..boundary + 1)?;
                    visible_breaks += 1;
                }
            }
            // A new blank source row joins the adjacent separator run. Existing
            // paired (or odd-spelled) boundaries must keep their displayed rows.
            let count = if visible_breaks == 0 { 1 } else {
                (2 * (visible_breaks + 1)).saturating_sub(physical_breaks).max(1)
            };
            let source_at = self.projection().source_insertion_point(at, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            return self.prepare_text_edits_with_patches(
                vec![TextEdit::new(at..at, "\n")],
                Some(vec![SourcePatch::primary(source_at..source_at,
                    self.encoding().encode_fragment(&self.file_format().spelling().repeat(count))?)]),
            );
        }
        if self.format() == crate::document::Format::Markdown
            && self.projection().blocks_for_region(&(at..at)).iter().any(|block| {
                block.style.0 != "Code Block" && matches!(block.kind, BlockKind::ListItem { .. })
            })
        {
            return self.prepare_markdown_list_split(at, true);
        }
        self.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], None)
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
