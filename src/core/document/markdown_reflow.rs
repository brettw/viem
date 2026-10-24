//! Preview reflow changes source spelling, never the visible paragraph. The
//! command supplies a visible extent; rounding it to source lines would expand
//! a two-row motion to an entire paragraph stored on one long physical line.
use super::*;
use crate::document::{Block, BlockKind, SemanticInlineStyle};
use unicode_segmentation::UnicodeSegmentation;

struct Gap {
    source: Range<usize>,
    normalized: Range<usize>,
    had_break: bool,
}

impl Document {
    /// Rewrap only the source whitespace belonging to the selected Preview
    /// fragment. Protected owners and empty paragraphs are successful no-ops.
    /// Width is measured in source columns, including hidden Markdown syntax;
    /// the caller's visual rows determine scope, not the output width.
    pub fn reflow_markdown(
        &mut self,
        range: Range<usize>,
        text_width: u32,
        tabstop: u32,
    ) -> Result<bool, DocumentError> {
        self.require_modifiable()?;
        self.validate_range(&range)?;
        if !self.format().is_markdown() || text_width == 0 || tabstop == 0 {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut patches = Vec::new();
        let mut regions = Vec::new();
        for block in self.projection().blocks_for_region(&range) {
            if block.range.is_empty() || block.markdown_html || block.thematic_break
                || block.style.0 == "Code Block" || matches!(block.kind, BlockKind::Heading(_))
                || block.containers.iter().any(|member| member.container.kind == crate::document::ContainerKind::CodeBlock)
                || self.projection().table_at(block.range.start).is_some()
            {
                continue;
            }
            let selected = range.start.max(block.range.start)..range.end.min(block.range.end);
            if selected.start >= selected.end { continue; }
            let planned = self.markdown_reflow_paragraph(&block, selected, text_width, tabstop)
                .map_err(compat_document_error)?;
            if !planned.is_empty() {
                regions.push(block.range.clone());
                patches.extend(planned);
            }
        }
        if patches.is_empty() { return Ok(false); }
        let prepared = self.prepare_markdown_reflow_patches(patches, &regions)
            .map_err(compat_document_error)?;
        self.commit_model_transaction(prepared).map_err(compat_document_error)?;
        Ok(true)
    }

    fn markdown_reflow_paragraph(
        &self,
        block: &Block,
        selected: Range<usize>,
        text_width: u32,
        tabstop: u32,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let visible = self.projection().text_tree().slice(selected.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let mut protected = self.projection().style_spans_for_region(&selected).into_iter()
            .filter(|span| match &span.application {
                StyleApplication::Semantic(SemanticInlineStyle::Code) => true,
                StyleApplication::Automatic(name) => matches!(name.0.as_str(),
                    "Code" | "Link" | "Markdown reference" | "Comment"),
                _ => false,
            }).map(|span| span.range).collect::<Vec<_>>();
        protected.sort_by_key(|range| range.start);
        let mut protected_end = 0;
        let mut protected_index = 0;
        let Some(extent) = self.projection().source_range(selected.clone()) else {
            return Ok(Vec::new());
        };
        // An upstream visible boundary stops before a closing emphasis marker
        // or a link destination. Include that hidden suffix when measuring the
        // final word, but never make it (or the next visible character) editable.
        let measurement_end = if selected.end == self.projection().text_tree().byte_len() {
            self.source_byte_len()
        } else {
            self.next_grapheme_boundary(selected.end)
                .and_then(|end| self.projection().source_range(selected.end..end))
                .map_or(extent.end, |next| next.start)
        }.max(extent.end);
        let first_line = self.state().source_hard_lines.line_at_offset(extent.start)
            .and_then(|index| self.state().source_hard_lines.get(index))
            .ok_or(DocumentError::AmbiguousProjection)?;
        // The prefix before a partial selection contributes to its first source
        // column. Read it as context only: none of its whitespace is editable.
        let starting_column = self.markdown_reflow_start_column(first_line.start..extent.start,
            text_width as usize, tabstop as usize)?;
        let source = extent.start..measurement_end;
        let bytes = self.source_bytes_in(source.clone())?;
        let decoded = self.encoding().decode_region(&bytes, source.start)?;
        if decoded.spans.iter().any(|span| span.diagnostic.is_some()) {
            return Ok(Vec::new());
        }
        let input = normalize(&decoded, self.file_format());
        let mut gaps = Vec::new();
        for (offset, _) in visible.grapheme_indices(true).filter(|(_, grapheme)| *grapheme == " ") {
            let at = selected.start + offset;
            while protected.get(protected_index).is_some_and(|range| range.start <= at) {
                protected_end = protected_end.max(protected[protected_index].end);
                protected_index += 1;
            }
            // Only a single ordinary visible space can become a soft break.
            // Runs of spaces, tabs, entities and explicit hard breaks retain
            // their exact visible and source meaning. Both words must belong
            // to the selection; its edges are not implicit break requests.
            if offset == 0 || offset + 1 == visible.len()
                || visible[..offset].ends_with(char::is_whitespace)
                || visible[offset + 1..].starts_with(char::is_whitespace)
                || at < protected_end
            { continue; }
            let Some(mut gap) = self.projection().source_range(at..at + 1) else { continue; };
            let start = input.normalized_at(gap.start);
            let mut end = input.normalized_at(gap.end);
            if start >= end || end > input.text.len() { continue; }
            let had_break = input.text[start..end].contains('\n');
            if had_break {
                // Folded-space provenance can end before a nested quote's
                // hidden prefix. Consume only that already-owned prefix, never
                // the next word's emphasis opener or other inline syntax.
                let Some(next) = self.projection().source_insertion_point(at + 1, true) else { continue; };
                let next = input.normalized_at(next);
                let line_start = input.text[..next].rfind('\n').map_or(0, |at| at + 1);
                let prefix = crate::document::markdown_quotes::container_prefix(&input.text[line_start..next]);
                end = end.max(line_start + prefix);
                gap.end = input.source_at(end);
                let raw = &input.text[start..end];
                if raw.matches('\n').count() != 1
                    || !raw.chars().all(|ch| matches!(ch, ' ' | '\t' | '\n' | '>')) {
                    continue;
                }
            } else if &input.text[start..end] != " " {
                continue;
            }
            gaps.push(Gap { source: gap, normalized: start..end, had_break });
        }
        if gaps.is_empty() { return Ok(Vec::new()); }
        let continuation = if block.list_level().is_some() {
            self.markdown_list_continuation_context(block.range.start)?.2
        } else if block.quote_depth > 0 {
            // Lazy quote continuations may have no physical prefix. Ownership
            // supplies the required depth even when the selected row omits it.
            "> ".repeat(block.quote_depth)
        } else {
            let body = self.projection().source_insertion_point(block.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let row = self.state().source_hard_lines.line_at_offset(body)
                .and_then(|index| self.state().source_hard_lines.get(index))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let prefix = self.source_bytes_in(row.start..body)?;
            let prefix = self.encoding().decode_region(&prefix, row.start)?;
            prefix.text.chars().take_while(|ch| matches!(ch, ' ' | '\t')).collect()
        };
        let tabstop = tabstop as usize;
        let mut column = starting_column;
        let mut cursor = 0;
        let mut patches = Vec::new();
        for (index, gap) in gaps.iter().enumerate() {
            column = source_column(column, &input, cursor..gap.normalized.start, tabstop);
            let end = gaps.get(index + 1).map_or(input.normalized_at(measurement_end), |next| next.normalized.start);
            let first_ending = input.endings.partition_point(|ending| ending.normalized.start < gap.normalized.end);
            let end = input.endings.get(first_ending)
                .map_or(end, |ending| end.min(ending.normalized.start));
            let first_line = &input.text[gap.normalized.end..end.max(gap.normalized.end)];
            let wrap = crate::document::indentation::end_column(column.saturating_add(1), first_line, tabstop)
                > text_width as usize
                && safe_line_start(&input.text[gap.normalized.end..]);
            let replacement = if wrap {
                column = crate::document::indentation::end_column(0, &continuation, tabstop);
                if gap.had_break {
                    // Keeping a break also keeps its authored CR/LF spelling
                    // and container spelling. Only new breaks use fileformat.
                    cursor = gap.normalized.end;
                    column = source_column(0, &input, gap.normalized.clone(), tabstop);
                    continue;
                }
                format!("{}{continuation}", self.file_format().spelling())
            } else {
                column = column.saturating_add(1);
                " ".to_owned()
            };
            let replacement = self.encoding().encode_fragment(&replacement)?;
            if self.source_bytes_in(gap.source.clone())? != replacement {
                patches.push(SourcePatch::primary(gap.source.clone(), replacement));
            }
            cursor = gap.normalized.end;
        }
        Ok(patches)
    }
}

fn source_column(
    start: usize,
    input: &crate::document::line_endings::NormalizedText,
    range: Range<usize>,
    tabstop: usize,
) -> usize {
    // Normalized text also contains literal LF under fileformat=mac. Only
    // tagged source endings reset columns; splitting on the scalar would
    // measure (and wrap) a different physical source line than the user owns.
    let first = input.endings.partition_point(|ending| ending.normalized.start < range.start);
    let last = input.endings[first..].iter()
        .take_while(|ending| ending.normalized.end <= range.end).last();
    let (column, at) = last.map_or((start, range.start), |ending| (0, ending.normalized.end));
    crate::document::indentation::end_column(column, &input.text[at..range.end], tabstop)
}

/// A soft break must not turn prose into block syntax. Prefer an overlong
/// source line to an escape-spelling change or an accidental heading/list/HTML
/// block. Candidate verification still checks the complete semantic result.
fn safe_line_start(tail: &str) -> bool {
    let tail = tail.trim_start_matches([' ', '\t']);
    let Some(first) = tail.as_bytes().first().copied() else { return false; };
    let run = tail.bytes().take_while(|byte| *byte == first).count();
    let after = tail.as_bytes().get(run).copied();
    let blank = after.is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\n'));
    match first {
        b'>' | b'<' => return false,
        b'#' if run <= 6 && blank => return false,
        b'`' | b'~' if run >= 3 => return false,
        b'+' if run == 1 && blank => return false,
        b'-' | b'*' | b'_' if blank => return false,
        b'=' if blank => return false,
        // Inline links and reference uses may start a line, but moving a
        // definition-shaped label there would change its role. Markdown caps
        // reference labels, so this lookahead never scans a whole paragraph.
        b'[' if tail.as_bytes()[..tail.len().min(1002)]
            .windows(2).any(|pair| pair == b"]:") => return false,
        _ => {}
    }
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    !(digits > 0 && digits <= 9 && tail.as_bytes().get(digits).is_some_and(|ch| matches!(ch, b'.' | b')'))
        && tail[digits + 1..].starts_with([' ', '\t']))
}
