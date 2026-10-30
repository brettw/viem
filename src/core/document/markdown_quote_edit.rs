//! Native quote assignment changes only physical quote prefixes. The quoted
//! body keeps its existing heading/list/code and inline syntax.
use super::{
    structural_style, Document, DocumentError, ModelTransactionError, PreparedModelTransaction, SourcePatch, TextEdit,
};
use crate::document::{BlockKind, BoundaryAffinity, Format};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

impl Document {
    pub(super) fn markdown_quote_insertion_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if self.format() != Format::Markdown
            || !edit.range.is_empty()
            || !edit.replacement.contains('\n')
        {
            return Ok(None);
        }
        let Some((_, range, separator)) = self.markdown_quote_enter_syntax(edit.range.start)?
        else {
            return Ok(None);
        };
        let Some(block) = crate::document::edit_boundary::paragraph_at(self, edit.range.start)?
        else {
            return Ok(None);
        };
        if crate::document::markdown_quotes::is_fenced_block(self, &block)? {
            return Ok(None);
        }
        if let Some(patches) = super::markdown_split::patches_with_separator(
            self,
            &edit.range,
            &edit.replacement,
            &separator.replace(self.file_format().spelling(), "\n"),
            Some(range.start),
        )? {
            return Ok(Some(patches));
        }
        // Each later part begins a quoted paragraph after its prefix.
        let mut syntax = String::new();
        for (index, part) in edit.replacement.split('\n').enumerate() {
            let escaped = self.escape_markdown_source_text(range.start, part)?;
            if index == 0 {
                syntax.push_str(&escaped);
            } else {
                syntax.push_str(&separator);
                super::protect_markdown_line_start(&escaped, &mut syntax);
            }
        }
        Ok(Some(vec![SourcePatch::primary(
            range,
            self.encoding().encode_fragment(&syntax)?,
        )]))
    }

    pub(crate) fn markdown_quote_enter_edit(
        &self,
        at: usize,
    ) -> Result<Option<TextEdit>, DocumentError> {
        Ok(self
            .markdown_quote_enter_syntax(at)?
            .map(|(edit, _, _)| edit))
    }

    fn markdown_quote_enter_syntax(
        &self,
        at: usize,
    ) -> Result<Option<(TextEdit, Range<usize>, String)>, DocumentError> {
        if !self.format().is_markdown() {
            return Ok(None);
        }
        let Some(block) = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| {
                block.range.start <= at
                    && at <= block.range.end
                    && (block.style.0 == "Block quote" || block.quote_depth > 0)
            })
        else {
            return Ok(None);
        };
        // Quoted lists share item ownership and split repairs with all other
        // Markdown lists; the physical quote prefix alone is not that owner.
        if matches!(block.kind, BlockKind::ListItem { .. })
            && !crate::document::markdown_quotes::is_fenced_block(self, &block)?
        {
            return Ok(None);
        }
        if block.style.0 == "Code Block"
            && !crate::document::markdown_quotes::is_fenced_block(self, &block)?
        {
            return Ok(None);
        }
        let source_start = self
            .projection()
            .provenance_touching(&(block.range.start..block.range.start))
            .iter()
            .find(|span| span.formatted == block.range && block.range.is_empty())
            .map(|span| span.source.start)
            .or_else(|| {
                self.projection()
                    .source_insertion_point(block.range.start, true)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        let index = self
            .state()
            .source_hard_lines
            .line_at_offset(source_start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = self
            .state()
            .source_hard_lines
            .get(index)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self
            .state()
            .source
            .bytes_in(line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let length = crate::document::markdown_quotes::prefix(&decoded.text);
        let prefix = &decoded.text[..length];
        if prefix.is_empty() {
            return Err(DocumentError::AmbiguousProjection);
        }
        let body = &decoded.text[length..];
        let code = crate::document::markdown_quotes::is_fenced_block(self, &block)?;
        if matches!(
            block.kind,
            BlockKind::ListItem {
                item_start: true,
                ..
            }
        ) {
            if let Some(label_length) = crate::document::markdown_blocks::marker_prefix_length(body)
            {
                if body[label_length..].trim().is_empty() {
                    let current = self
                        .projection()
                        .hard_line_at_offset(at)
                        .and_then(|index| self.projection().hard_line_range(index))
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let mut boundaries = 0;
                    let mut endings = 0;
                    for boundary in [
                        current.start.checked_sub(1),
                        (current.end < self.text().len()).then_some(current.end),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        boundaries += 1;
                        endings += self.markdown_source_separator_count(boundary..boundary + 1)?;
                    }
                    let extra = (2usize * boundaries).saturating_sub(endings);
                    let logical = if extra == 0 {
                        String::new()
                    } else {
                        format!(
                            "{}\n{prefix}",
                            format!("\n{}", prefix.trim_end_matches([' ', '\t'])).repeat(extra - 1)
                        )
                    };
                    let source_start = line.start + self.encoding().encode_fragment(prefix)?.len();
                    let source_end = source_start
                        + self
                            .encoding()
                            .encode_fragment(&body[..label_length])?
                            .len();
                    return Ok(Some((
                        TextEdit::new(block.range.clone(), ""),
                        source_start..source_end,
                        logical.replace('\n', self.file_format().spelling()),
                    )));
                }
            }
        }
        let code_prefix = if code {
            crate::document::markdown_code::fenced_source(self, &block)?
                .map(|fence| fence.body_prefix)
        } else { None };
        let continuation_prefix = code_prefix.as_deref().unwrap_or(prefix);
        let list_kind = (!code).then_some(&block.kind);
        let logical = if let Some(BlockKind::ListItem {
            ordered, ordinal, ..
        }) = list_kind
        {
            let label_length = crate::document::markdown_blocks::marker_prefix_length(body)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let label = &body[..label_length];
            let indentation = label.len() - label.trim_start_matches([' ', '\t']).len();
            let delimiter = label[indentation..]
                .chars()
                .find(|ch| !ch.is_ascii_digit())
                .ok_or(DocumentError::AmbiguousProjection)?;
            format!(
                "\n{prefix}{}{}{} ",
                &label[..indentation],
                if *ordered {
                    ordinal.saturating_add(1).to_string()
                } else {
                    String::new()
                },
                delimiter
            )
        } else if code {
            // Code lines continue the fence body with one ending; a prose
            // paragraph separator would add a quote-only row.
            format!("\n{continuation_prefix}")
        } else {
            format!("\n{}\n{continuation_prefix}", prefix.trim_end_matches([' ', '\t']))
        };
        let edit = TextEdit::new(at..at, "\n");
        let source_at = crate::document::source_edit::insertion_point(self.projection(), at, None)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let syntax = logical.replace('\n', self.file_format().spelling());
        Ok(Some((edit, source_at..source_at, syntax)))
    }

    pub(super) fn prepare_markdown_quote_enter(
        &self,
        at: usize,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        let Some((edit, source_range, syntax)) = self.markdown_quote_enter_syntax(at)? else {
            return Ok(None);
        };
        let mut patches = vec![SourcePatch::primary(
            source_range,
            self.encoding().encode_fragment(&syntax)?,
        )];
        self.preserve_markdown_edit_boundaries(
            std::slice::from_ref(&edit),
            std::iter::once(&edit.range),
            &mut patches,
        )?;
        Ok(Some(self.prepare_text_edits_with_patches(vec![edit], Some(patches))?))
    }

    /// Removing quote treatment from an alert's first paragraph also removes
    /// its hidden marker, which would otherwise become an empty alert.
    pub(super) fn prepare_markdown_quote_style(
        &self,
        range: Range<usize>,
        quote: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if !quote {
            let patches = self.unquoted_alert_marker_patches(&range)?;
            if !patches.is_empty() {
                return self.prepare_markdown_supporting_patches(patches, |document|
                    document.prepare_markdown_quote_style_raw(range.clone(), false));
            }
        }
        self.prepare_markdown_quote_style_raw(range, quote)
    }

    pub(super) fn prepare_markdown_quote_style_raw(
        &self,
        range: Range<usize>,
        quote: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if quote && self.format().is_markdown() {
            // A nested item's quote is inserted inside its list marker. Its
            // first leaf becomes a quote, so a formerly hidden task marker
            // would become literal text. Outer quotes around top-level items
            // keep their task ownership and their authored marker spelling.
            let patches = structural_style::selected_blocks(self, &range).iter()
                .filter(|block| block.task_checkbox.is_some() && block.quote_depth == 0
                    && matches!(block.kind, BlockKind::ListItem { level: 1.., .. }))
                .map(|block| self.remove_checkbox_source_patches(block))
                .collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect::<Vec<_>>();
            if !patches.is_empty() {
                return self.prepare_markdown_supporting_patches(patches, |document|
                    document.prepare_markdown_quote_style_raw(range.clone(), quote));
            }
        }
        let blocks = self.projection().blocks_for_region(&range);
        let mut lines = BTreeSet::new();
        let mut quote_depths = BTreeMap::new();
        let mut code_list_prefixes = BTreeMap::new();
        let mut patches = Vec::new();
        for block in blocks.iter().filter(|block| {
            (quote || (block.style.0 == "Block quote" || block.quote_depth > 0))
                && if range.is_empty() {
                    block.range.start <= range.start && range.start <= block.range.end
                } else {
                    block.range.start < range.end && range.start < block.range.end
                }
        }) {
            if quote && (block.style.0 == "Block quote" || block.quote_depth > 0) {
                continue;
            }
            let provenance = self.projection().provenance_for_region(&block.range);
            let source_start = provenance
                .first()
                .map(|span| span.source.start)
                .or_else(|| {
                    self.projection()
                        .source_insertion_point(block.range.start, true)
                })
                .ok_or(DocumentError::AmbiguousProjection)?;
            let source_end = provenance
                .last()
                .map_or(source_start, |span| span.source.end);
            // A final hard break leaves an editable empty body row. Its
            // zero-width provenance is outside this half-open block query,
            // but that row still belongs to the quote, before the closing fence.
            let empty_terminal_body = !block.range.is_empty()
                && self.projection().hard_breaks_for_region(&(block.range.end - 1..block.range.end))
                    .contains(&(block.range.end - 1));
            let index = &self.state().source_hard_lines;
            let mut first = index
                .line_at_offset(source_start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut last = index
                .line_at_offset(if empty_terminal_body {
                    source_end
                } else {
                    source_end.saturating_sub(1).max(source_start)
                })
                .ok_or(DocumentError::AmbiguousProjection)?;
            // Fenced code's delimiters are outside its visible body but belong
            // to the same containing paragraph when its quote container moves.
            if self.format() == Format::Markdown && first > 0 {
                let before = index
                    .get(first - 1)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self
                    .state()
                    .source
                    .bytes_in(before.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, before.start)?;
                let opening_prefix = crate::document::markdown_quotes::container_prefix(&decoded.text);
                let body = &decoded.text[opening_prefix..];
                if let Some((delimiter, length)) = crate::document::projection::markdown_fence(body)
                {
                    first -= 1;
                    let list_columns = matches!(block.kind, BlockKind::ListItem { .. }).then(|| {
                        decoded.text[..opening_prefix].chars().fold(0usize, |columns, ch|
                            if ch == '\t' { columns + 4 - columns % 4 } else { columns + 1 })
                    });
                    if let Some(after) = index.get(last + 1) {
                        let bytes = self
                            .state()
                            .source
                            .bytes_in(after.clone())
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        let decoded = self.encoding().decode_region(&bytes, after.start)?;
                        let tail = decoded.text
                            [crate::document::markdown_quotes::prefix(&decoded.text)..]
                            .trim();
                        if tail.len() >= length && tail.bytes().all(|byte| byte == delimiter) {
                            last += 1;
                        }
                    }
                    if let Some(columns) = list_columns {
                        for line in first..=last { code_list_prefixes.insert(line, (first, opening_prefix, columns)); }
                    }
                }
            }
            // A Setext underline belongs to the heading even though its
            // bytes have no visible provenance. Move its container prefix
            // with the heading body so it cannot become a separate rule.
            if block.style.0.starts_with("Heading") {
                let opening = index.get(first).ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self.state().source.bytes_in(opening.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, opening.start)?;
                let body = &decoded.text[crate::document::markdown_quotes::container_prefix(&decoded.text)..];
                if !body.starts_with('#') {
                    if let Some(underline) = index.get(last + 1) {
                        let bytes = self.state().source.bytes_in(underline.clone())
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        let decoded = self.encoding().decode_region(&bytes, underline.start)?;
                        let body = decoded.text[crate::document::markdown_quotes::prefix(&decoded.text)..].trim();
                        if !body.is_empty() && (body.bytes().all(|byte| byte == b'=')
                            || body.bytes().all(|byte| byte == b'-')) {
                            last += 1;
                        }
                    }
                }
            }
            if quote && self.format() == Format::Markdown && block.style.0 == "Paragraph"
                && range.end <= block.range.end {
                if let Some(following) = block.range.end.checked_add(1) {
                    if let Some(table) = self.projection().table_at(following)
                        .filter(|table| table.range.start == following) {
                        if let Some(next) = index.get(last + 1)
                            .filter(|next| next.start == table.source_range.start) {
                            // An unquoted table header can lazily continue the
                            // new quote paragraph. Keep the already separate
                            // table outside it with one supporting separator.
                            patches.push(SourcePatch::primary(next.start..next.start,
                                self.encoding().encode_fragment(self.file_format().spelling())?));
                        }
                    }
                }
            }
            if quote && matches!(block.kind, BlockKind::ListItem { level: 1.., .. })
                && !code_list_prefixes.contains_key(&first) {
                let row = index.get(first).ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self.state().source.bytes_in(row.clone()).ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, row.start)?;
                let prefix = crate::document::markdown_quotes::container_prefix(&decoded.text);
                let columns = decoded.text[..prefix].chars().fold(0usize, |column, ch|
                    if ch == '\t' { column + 4 - column % 4 } else { column + 1 });
                for line in first..=last { code_list_prefixes.insert(line, (first, prefix, columns)); }
            }
            if !quote {
                for line in first..=last { quote_depths.insert(line, block.quote_depth.max(1)); }
            }
            lines.extend(first..=last);
        }
        let Some(first) = lines.first().copied() else {
            return Ok(self.no_op_prepared());
        };
        let last = *lines.last().unwrap();
        for index in first..=last {
            let line = self
                .state()
                .source_hard_lines
                .get(index)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = self
                .state()
                .source
                .bytes_in(line.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, line.start)?;
            let separator = decoded.text[crate::document::markdown_quotes::prefix(&decoded.text)..]
                .trim()
                .is_empty();
            if quote && lines.contains(&index) {
                let (offset, padding) = if let Some(&(opening, opening_prefix, columns)) = code_list_prefixes.get(&index) {
                    if index == opening { (opening_prefix, 0) } else {
                        let mut width = 0;
                        let mut bytes = 0;
                        for ch in decoded.text.chars() {
                            if width >= columns || !matches!(ch, ' ' | '\t') { break; }
                            width += if ch == '\t' { 4 - width % 4 } else { 1 };
                            bytes += ch.len_utf8();
                        }
                        (bytes, columns.saturating_sub(width))
                    }
                } else { (0, 0) };
                let source_offset = line.start + self.encoding().encode_fragment(&decoded.text[..offset])?.len();
                patches.push(SourcePatch::primary(
                    source_offset..source_offset,
                    self.encoding().encode_fragment(&format!("{}> ", " ".repeat(padding)))?,
                ));
            } else if !quote && (lines.contains(&index) || separator) {
                // The quote may be inside a list, after its marker. Its
                // current paragraph owner supplies the structural depth:
                // code body text can contain further literal `>` markers.
                let prefix = crate::document::markdown_quotes::container_prefix(&decoded.text);
                let quote_text = &decoded.text[..prefix];
                let depth = quote_depths.range(..=index).next_back()
                    .map(|(_, depth)| *depth).unwrap_or(1);
                if let Some((marker, _)) = quote_text.match_indices('>').nth(depth - 1) {
                    let end = marker + 1 + usize::from(quote_text.as_bytes().get(marker + 1)
                        .is_some_and(|byte| matches!(byte, b' ' | b'\t')));
                    let start = decoded.source_boundary(marker).ok_or(DocumentError::AmbiguousProjection)?;
                    let end = decoded.source_boundary(end).ok_or(DocumentError::AmbiguousProjection)?;
                    patches.push(SourcePatch::primary(start..end, Vec::new()));
                }
            }
        }
        if patches.is_empty() {
            return Ok(self.no_op_prepared());
        }
        if self.format() == Format::Markdown {
            if !quote {
                patches.extend(super::markdown_block_styles::support_patches(
                    self, &range, false, true,
                )?);
            }
            return self.prepare_text_edits_with_patches(Vec::new(), Some(patches));
        }
        let edits = patches
            .iter()
            .map(|patch| {
                let start = self
                    .projection()
                    .map_source_boundary(
                        self.revision(),
                        patch.range.start,
                        BoundaryAffinity::Downstream,
                    )
                    .map_err(|_| DocumentError::AmbiguousProjection)?
                    .formatted_offset;
                let end = self
                    .projection()
                    .map_source_boundary(
                        self.revision(),
                        patch.range.end,
                        BoundaryAffinity::Upstream,
                    )
                    .map_err(|_| DocumentError::AmbiguousProjection)?
                    .formatted_offset;
                let replacement = self
                    .encoding()
                    .decode_region(&patch.replacement, patch.range.start)?
                    .text;
                Ok(TextEdit::new(start..end, replacement))
            })
            .collect::<Result<Vec<_>, DocumentError>>()?;
        if !quote {
            patches.extend(super::markdown_block_styles::support_patches(
                self, &range, false, true,
            )?);
        }
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }
}
