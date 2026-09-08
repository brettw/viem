//! Native quote assignment changes only physical quote prefixes. The quoted
//! body keeps its existing heading/list/code and inline syntax.
use super::{
    Document, DocumentError, ModelTransactionError, PreparedModelTransaction, SourcePatch, TextEdit,
};
use crate::document::{BlockKind, BoundaryAffinity, Format};
use std::collections::BTreeSet;
use std::ops::Range;

impl Document {
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
        if !matches!(self.format(), Format::Markdown | Format::MarkdownSource) {
            return Ok(None);
        }
        let Some(block) = self
            .projection()
            .blocks_for_region(&(at..at))
            .into_iter()
            .find(|block| {
                block.range.start <= at && at <= block.range.end && block.style.0 == "Block quote"
            })
        else {
            return Ok(None);
        };
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
        let source_at = self
            .projection()
            .source_insertion_point(at, at == block.range.start)
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
        Ok(Some(self.prepare_text_edits_with_patches(
            vec![edit],
            Some(vec![SourcePatch::primary(
                source_range,
                self.encoding().encode_fragment(&syntax)?,
            )]),
        )?))
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
            (quote || block.style.0 == "Block quote")
                && if range.is_empty() {
                    block.range.start <= range.start && range.start <= block.range.end
                } else {
                    block.range.start < range.end && range.start < block.range.end
                }
        }) {
            if quote && block.style.0 == "Block quote" {
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
                patches.push(SourcePatch::primary(
                    line.start..line.start + prefix,
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
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }
}
