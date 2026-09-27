//! Native quote assignment changes only physical quote prefixes. The quoted
//! body keeps its existing heading/list/code and inline syntax.
use super::{
    Document, DocumentError, ModelTransactionError, PreparedModelTransaction, SourcePatch, TextEdit,
};
use crate::document::{BlockKind, BoundaryAffinity, Format};
use std::collections::BTreeSet;
use std::ops::Range;

impl Document {
    pub(super) fn markdown_quote_insertion_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if self.format() != Format::Markdown || !edit.range.is_empty() || !edit.replacement.contains('\n') {
            return Ok(None);
        }
        let Some((_, range, separator)) = self.markdown_quote_enter_syntax(edit.range.start)? else { return Ok(None); };
        let Some(block) = crate::document::edit_boundary::paragraph_at(self, edit.range.start)? else { return Ok(None); };
        if crate::document::markdown_quotes::is_fenced_block(self, &block)? { return Ok(None); }
        if let Some(patches) = super::markdown_split::patches_with_separator(self, &edit.range, &edit.replacement,
            &separator.replace(self.file_format().spelling(), "\n"), Some(range.start))? {
            return Ok(Some(patches));
        }
        let syntax = edit.replacement.split('\n')
            .map(|part| self.escape_markdown_source_text(range.start, part))
            .collect::<Result<Vec<_>, _>>()?.join(&separator);
        Ok(Some(vec![SourcePatch::primary(range, self.encoding().encode_fragment(&syntax)?)]))
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
                block.range.start <= at && at <= block.range.end && (block.style.0 == "Block quote" || block.quote_depth > 0)
            })
        else {
            return Ok(None);
        };
        // Quoted lists share item ownership and split repairs with all other
        // Markdown lists; the physical quote prefix alone is not that owner.
        if self.format() == Format::Markdown && matches!(block.kind, BlockKind::ListItem { .. })
            && !crate::document::markdown_quotes::is_fenced_block(self, &block)?
        {
            return Ok(None);
        }
        if self.format() == Format::Markdown && block.style.0 == "Code Block"
            && !crate::document::markdown_quotes::is_fenced_block(self, &block)? { return Ok(None); }
        if self.format() == Format::MarkdownSource {
            let row = self.projection().hard_line_at_offset(at)
                .and_then(|index| self.projection().hard_line_range(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let text = self.projection().text_tree().slice(row.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let prefix = crate::document::markdown_quotes::prefix(&text);
            let marker = crate::document::markdown_blocks::marker_prefix_length(&text[prefix..])
                .unwrap_or(0);
            let marker = if matches!(block.kind, BlockKind::ListItem { item_start: false, .. }) {
                marker.max(text[prefix..].len() - text[prefix..].trim_start_matches([' ', '\t']).len())
            } else { marker };
            // Visible source syntax can itself be split. Continuing its
            // container before that prefix would duplicate the original label.
            if at < row.start + prefix + marker {
                return Ok(None);
            }
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
                    let edit = if self.format() == Format::MarkdownSource {
                        let start = self
                            .projection()
                            .map_source_boundary(
                                self.revision(),
                                source_start,
                                BoundaryAffinity::Downstream,
                            )
                            .map_err(|_| DocumentError::AmbiguousProjection)?
                            .formatted_offset;
                        TextEdit::new(start..start + label_length, logical.clone())
                    } else {
                        TextEdit::new(block.range.clone(), "")
                    };
                    return Ok(Some((
                        edit,
                        source_start..source_end,
                        logical.replace('\n', self.file_format().spelling()),
                    )));
                }
            }
        }
        let logical = if let BlockKind::ListItem {
            ordered, ordinal, ..
        } = block.kind
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
                if ordered {
                    ordinal.saturating_add(1).to_string()
                } else {
                    String::new()
                },
                delimiter
            )
        } else if code {
            format!("\n{prefix}")
        } else {
            format!("\n{}\n{prefix}", prefix.trim_end_matches([' ', '\t']))
        };
        let edit = TextEdit::new(
            at..at,
            if self.format() == Format::MarkdownSource {
                logical.clone()
            } else {
                "\n".into()
            },
        );
        let source_at = if self.format() == Format::Markdown {
            crate::document::source_edit::insertion_point(self.projection(), at, None)
        } else {
            self.projection().source_insertion_point(at, at == block.range.start)
        }.ok_or(DocumentError::AmbiguousProjection)?;
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
        Ok(Some(if self.format() == Format::MarkdownSource {
            self.prepare_markdown_source_syntax_edit(edit, patches)?
        } else {
            self.preserve_markdown_edit_boundaries(std::slice::from_ref(&edit), std::iter::once(&edit.range), &mut patches)?;
            self.prepare_text_edits_with_patches(vec![edit], Some(patches))?
        }))
    }

    pub(super) fn prepare_markdown_quote_style(
        &self,
        range: Range<usize>,
        quote: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        let blocks = self.projection().blocks_for_region(&range);
        let mut lines = BTreeSet::new();
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
            let index = &self.state().source_hard_lines;
            let mut first = index
                .line_at_offset(source_start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut last = index
                .line_at_offset(source_end.saturating_sub(1).max(source_start))
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
                let body = &decoded.text[crate::document::markdown_quotes::prefix(&decoded.text)..];
                if let Some((delimiter, length)) = crate::document::projection::markdown_fence(body)
                {
                    first -= 1;
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
                }
            }
            lines.extend(first..=last);
        }
        let mut patches = Vec::new();
        let Some(first) = lines.first().copied() else {
            return Ok(self.no_op_prepared());
        };
        let last = *lines.last().unwrap();
        let band = self.state().source_hard_lines.get(first).unwrap().start
            ..self.state().source_hard_lines.get(last).unwrap().end;
        let bytes = self
            .state()
            .source
            .bytes_in(band.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, band.start)?;
        let normalized = super::normalize(&decoded, self.file_format());
        let contexts = crate::document::markdown_quotes::source_context(&normalized);
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
            let context = &contexts[index - first];
            let prefix = context.content_start.saturating_sub(line.start);
            let separator = decoded.text[crate::document::markdown_quotes::prefix(&decoded.text)..]
                .trim()
                .is_empty();
            if quote && lines.contains(&index) {
                patches.push(SourcePatch::primary(
                    line.start..line.start,
                    self.encoding().encode_fragment("> ")?,
                ));
            } else if !quote && prefix > 0 && (lines.contains(&index) || separator) {
                // Remove one quote level; outer containers remain intact.
                let quote_text = &decoded.text[..crate::document::markdown_quotes::prefix(&decoded.text)];
                let marker = quote_text.rfind('>').ok_or(DocumentError::VerificationFailed)?;
                let remove_start = self.encoding().encode_fragment(&quote_text[..marker])?.len();
                patches.push(SourcePatch::primary(
                    line.start + remove_start..line.start + prefix,
                    Vec::new(),
                ));
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
