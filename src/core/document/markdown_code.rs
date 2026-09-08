//! Source-local code-body insertion and literal delimiter edits. Fence growth
//! is a supporting syntax patch in the same transaction as authored text.
use super::{Document, DocumentError, Format, Revision, SemanticInlineStyle, StyleApplication};
use std::ops::Range;

pub(super) fn patches(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    if document.format() != Format::Markdown {
        return Ok(None);
    }
    if let Some(patches) = join_following_paragraph(document, range, replacement)? {
        return Ok(Some(patches));
    }
    let block = document
        .projection()
        .blocks_for_region(range)
        .into_iter()
        .find(|block| block.range.start <= range.start && range.end <= block.range.end);
    let code_block = block
        .as_ref()
        .is_some_and(|block| block.style.0 == "Code Block");
    if code_block && !replacement.contains(['`', '~']) {
        let block = block.as_ref().unwrap();
        if block.range.is_empty() {
            return Ok(None);
        }
        // The final code-body boundary is inside its fence, even when the
        // typing caret's ordinary downstream side would be after that syntax.
        // Keep this common path local: no full code-body or source capture.
        if replacement.contains('\n') || range.is_empty() && range.end == block.range.end {
            let source = if range.is_empty() {
                let at = document
                    .projection()
                    .source_insertion_point(range.start, range.start != block.range.end)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                at..at
            } else {
                document
                    .projection()
                    .source_range(range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?
            };
            let mut separator = document.file_format().spelling().to_owned();
            if replacement.contains('\n') && matches!(block.kind, super::BlockKind::ListItem { .. })
            {
                let first = document
                    .projection()
                    .source_insertion_point(block.range.start, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let line = document
                    .state()
                    .source_hard_lines
                    .line_at_offset(first)
                    .and_then(|index| document.state().source_hard_lines.get(index))
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let prefix = document
                    .state()
                    .source
                    .bytes_in(line.start..first)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let prefix = document.encoding().decode_region(&prefix, line.start)?.text;
                if prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
                    separator.push_str(&prefix);
                }
            }
            return Ok(Some(vec![(source, replacement.replace('\n', &separator))]));
        }
        return Ok(None);
    }
    if !replacement.contains(['`', '~']) {
        return Ok(None);
    }
    let content = if code_block {
        block.unwrap().range
    } else {
        let Some(span) = document
            .projection()
            .style_spans_for_region(
                &(range.start.saturating_sub(1)..(range.end + 1).min(document.text().len())),
            )
            .into_iter()
            .find(|span| {
                span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                    && span.range.start <= range.start
                    && range.end <= span.range.end
            })
        else {
            return Ok(None);
        };
        span.range
    };
    if content.is_empty() {
        return Ok(None);
    }
    let source_content = document
        .projection()
        .source_range(content.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let source_edit = if range.is_empty() && range.start == content.end {
        source_content.end..source_content.end
    } else if range.is_empty() && range.start == content.start {
        source_content.start..source_content.start
    } else {
        document
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?
    };
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let builder = super::rich_text::Builder::new(&input, Revision(0));
    let start = input
        .units
        .iter()
        .find(|unit| unit.source.start == source_content.start)
        .map(|unit| unit.normalized.start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let end = input
        .units
        .iter()
        .find(|unit| unit.source.end == source_content.end)
        .map(|unit| unit.normalized.end)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut body = document.text()[content.clone()].to_owned();
    body.replace_range(
        range.start - content.start..range.end - content.start,
        replacement,
    );
    let mut result = vec![(
        source_edit,
        replacement.replace('\n', document.file_format().spelling()),
    )];
    if code_block {
        let mut offset = 0;
        let mut fence = None;
        let mut closing = None;
        for line in input.text.split_inclusive('\n') {
            let plain = line.trim_end_matches('\n');
            if let Some((open, delimiter, length)) = fence.clone() {
                if offset > start
                    && plain.trim().len() >= length
                    && plain.trim().bytes().all(|c| c == delimiter)
                {
                    closing = Some(offset..offset + plain.len());
                    break;
                }
                if offset + line.len() <= start
                    && plain.trim().len() >= length
                    && plain.trim().bytes().all(|c| c == delimiter)
                {
                    fence = None;
                } else {
                    fence = Some((open, delimiter, length));
                }
            } else if let Some((delimiter, length)) = super::projection::markdown_fence(plain) {
                let indent = plain.len() - plain.trim_start_matches(' ').len();
                fence = Some((offset + indent..offset + indent + length, delimiter, length));
            }
            offset += line.len();
        }
        let Some((open, delimiter, old_length)) = fence else {
            return Err(DocumentError::AmbiguousProjection);
        };
        let needed = body
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                (!line.is_empty() && line.bytes().all(|c| c == delimiter)).then_some(line.len() + 1)
            })
            .max()
            .unwrap_or(0)
            .max(old_length);
        if needed > old_length {
            let marker = (delimiter as char).to_string().repeat(needed);
            result.push((builder.source_range(open), marker.clone()));
            if let Some(close) = closing {
                result.push((builder.source_range(close), marker));
            }
        }
    } else {
        if body.contains('\n') {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let bytes = input.text.as_bytes();
        let mut left = start;
        let mut right = end;
        if left > 0 && bytes[left - 1] == b' ' {
            left -= 1;
        }
        if bytes.get(right) == Some(&b' ') {
            right += 1;
        }
        let open_end = left;
        while left > 0 && bytes[left - 1] == b'`' {
            left -= 1;
        }
        let close_start = right;
        while bytes.get(right) == Some(&b'`') {
            right += 1;
        }
        let old_length = open_end - left;
        if old_length == 0 || right - close_start != old_length {
            return Err(DocumentError::AmbiguousProjection);
        }
        let needed = body
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(old_length);
        let marker = "`".repeat(needed);
        let pad = if body.starts_with('`')
            || body.ends_with('`')
            || (body.starts_with(' ') && body.ends_with(' ') && !body.trim().is_empty())
        {
            " "
        } else {
            ""
        };
        let open = format!("{marker}{pad}");
        let close = format!("{pad}{marker}");
        if input.text[left..start] != open {
            result.push((builder.source_range(left..start), open));
        }
        if input.text[end..right] != close {
            result.push((builder.source_range(end..right), close));
        }
    }
    Ok(Some(result))
}

/// Joining the last code line owns its closing fence. Move that fence after
/// the following paragraph's visible content; otherwise removing only the
/// projected separator leaves source grammar between the joined characters.
fn join_following_paragraph(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    if range.is_empty() || replacement.contains('\n') {
        return Ok(None);
    }
    let projection = document.projection();
    let blocks = projection.blocks_for_region(range);
    let Some(code) = blocks.iter().find(|block| {
        block.range.end == range.start
            && block.style.0 == "Code Block"
            && block.kind == super::BlockKind::Paragraph
            && !block.range.is_empty()
    }) else {
        return Ok(None);
    };
    let Some(following) = blocks.iter().find(|block| {
        block.range.start == range.start + 1
            && range.end <= block.range.end
            && block.style.0 != "Code Block"
            && !block.range.is_empty()
    }) else {
        return Ok(None);
    };
    let code_source = projection
        .source_range(code.range.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let following_source = projection
        .source_range(following.range.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let source_lines = &document.state().source_hard_lines;
    let last_code_line = source_lines
        .line_at_offset(code_source.end)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let Some(closing) = source_lines.get(last_code_line + 1) else {
        return Ok(None);
    };
    let bytes = document
        .state()
        .source
        .bytes_in(closing.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, closing.start)?;
    let normalized = super::line_endings::normalize(&decoded, document.file_format());
    let closing_text = normalized.text.trim_end_matches('\n');
    let Some((delimiter, closing_width)) = super::projection::markdown_fence(closing_text) else {
        return Ok(None);
    };
    if !closing_text.trim().bytes().all(|byte| byte == delimiter) {
        return Ok(None);
    }
    let first_code_line = source_lines
        .line_at_offset(code_source.start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let Some(opening) = first_code_line
        .checked_sub(1)
        .and_then(|index| source_lines.get(index))
    else {
        return Ok(None);
    };
    let bytes = document
        .state()
        .source
        .bytes_in(opening.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let opening_decoded = document.encoding().decode_region(&bytes, opening.start)?;
    let Some((opening_delimiter, opening_width)) =
        super::projection::markdown_fence(&opening_decoded.text)
    else {
        return Ok(None);
    };
    if opening_delimiter != delimiter {
        return Ok(None);
    }
    let end_line = source_lines
        .line_at_offset(following_source.end.saturating_sub(1))
        .and_then(|index| source_lines.get(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let bytes = document
        .state()
        .source
        .bytes_in(end_line.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, end_line.start)?;
    let normalized = super::line_endings::normalize(&decoded, document.file_format());
    let end = normalized
        .endings
        .last()
        .filter(|ending| ending.source.end == end_line.end)
        .map_or(end_line.end, |ending| ending.source.start);
    let tail = projection
        .text_tree()
        .slice(range.end..following.range.end)
        .map_err(DocumentError::FormattedTextStorage)?;
    let mut appended = replacement.to_owned();
    appended.push_str(&tail);
    let last_line = projection
        .hard_line_at_offset(code.range.end)
        .and_then(|index| projection.hard_line_range(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut joined_tail = projection
        .text_tree()
        .slice(last_line.start..code.range.end)
        .map_err(DocumentError::FormattedTextStorage)?;
    joined_tail.push_str(&appended);
    let needed = joined_tail
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            (!line.is_empty() && line.bytes().all(|byte| byte == delimiter))
                .then_some(line.len() + 1)
        })
        .max()
        .unwrap_or(opening_width)
        .max(opening_width);
    let mut patches = Vec::new();
    if needed > opening_width {
        let indent =
            opening_decoded.text.len() - opening_decoded.text.trim_start_matches(' ').len();
        let start = opening.start
            + document
                .encoding()
                .encode_fragment(&opening_decoded.text[..indent])?
                .len();
        let end = start
            + document
                .encoding()
                .encode_fragment(&(delimiter as char).to_string().repeat(opening_width))?
                .len();
        let marker = (delimiter as char).to_string().repeat(needed);
        patches.push((start..end, marker));
    }
    let closing_text = if needed > closing_width {
        (delimiter as char).to_string().repeat(needed)
    } else {
        closing_text.to_owned()
    };
    appended.push('\n');
    appended.push_str(&closing_text);
    patches.push((
        code_source.end..end,
        appended.replace('\n', document.file_format().spelling()),
    ));
    Ok(Some(patches))
}

pub(super) fn delimiter_ranges(
    document: &Document,
    source: &Range<usize>,
) -> Result<Option<(Range<usize>, Range<usize>)>, DocumentError> {
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let Some(start) = input
        .units
        .iter()
        .find(|unit| unit.source.start == source.start)
        .map(|unit| unit.normalized.start)
    else {
        return Ok(None);
    };
    let Some(end) = input
        .units
        .iter()
        .find(|unit| unit.source.end == source.end)
        .map(|unit| unit.normalized.end)
    else {
        return Ok(None);
    };
    let bytes = input.text.as_bytes();
    let mut left = start;
    let mut right = end;
    if left > 0 && bytes[left - 1] == b' ' {
        left -= 1;
    }
    if bytes.get(right) == Some(&b' ') {
        right += 1;
    }
    let open_end = left;
    let close_start = right;
    while left > 0 && bytes[left - 1] == b'`' {
        left -= 1;
    }
    while bytes.get(right) == Some(&b'`') {
        right += 1;
    }
    if open_end == left || open_end - left != right - close_start {
        return Ok(None);
    }
    let builder = super::rich_text::Builder::new(&input, Revision(0));
    Ok(Some((
        builder.source_range(left..start),
        builder.source_range(end..right),
    )))
}

/// Remove code appearance from a subrange while retaining literal code on both
/// sides. Only the old delimiters and selected source extent are patched.
pub(super) fn clear_patches(
    document: &Document,
    span: &Range<usize>,
    selected: &Range<usize>,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let content = document
        .projection()
        .source_range(span.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let source = document
        .projection()
        .source_range(selected.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let (opening, closing) =
        delimiter_ranges(document, &content)?.ok_or(DocumentError::UnsupportedFormatting)?;
    let bytes = document.source_bytes();
    let decoded = document
        .encoding()
        .decode_region(&bytes[opening.clone()], opening.start)?;
    let marker = "`".repeat(
        decoded
            .text
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count(),
    );
    let prefix = &document.text()[span.start..selected.start];
    let suffix = &document.text()[selected.end..span.end];
    let pad = |text: &str| {
        if text.starts_with('`')
            || text.ends_with('`')
            || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty())
        {
            " "
        } else {
            ""
        }
    };
    let open = if prefix.is_empty() {
        String::new()
    } else {
        format!("{marker}{}", pad(prefix))
    };
    let close = if suffix.is_empty() {
        String::new()
    } else {
        format!("{}{marker}", pad(suffix))
    };
    let prefix_end = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}{marker}", pad(prefix))
    };
    let suffix_start = if suffix.is_empty() {
        String::new()
    } else {
        format!("{marker}{}", pad(suffix))
    };
    let text = super::projection::escape_markdown_insert(&document.text()[selected.clone()]);
    Ok(vec![
        (opening, open),
        (source, format!("{prefix_end}{text}{suffix_start}")),
        (closing, close),
    ])
}
