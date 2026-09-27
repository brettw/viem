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
    for block in document.projection().blocks_for_region(range) {
        if block.range.start <= range.start && range.end <= block.range.end {
            if let Some(code) = super::markdown_indented_code::source_block(document, &block)? {
                let source = if range.is_empty() {
                    let at = super::source_edit::insertion_point(document.projection(), range.start, None)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    at..at
                } else {
                    let start = super::source_edit::insertion_point(document.projection(), range.start, None)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let end = super::source_edit::insertion_point(document.projection(), range.end, None)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    start..end
                };
                let bytes = document.state().source.bytes_in(code.source.start..code.lines[0].content_start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let prefix = document.encoding().decode_region(&bytes, code.source.start)?.text;
                let quote = super::markdown_quotes::prefix(&prefix);
                let marker = super::markdown_blocks::marker_prefix_length(&prefix[quote..]).unwrap_or(0);
                let prefix = if marker > 0 { format!("{}{}{}", &prefix[..quote], " ".repeat(marker), &prefix[quote + marker..]) } else { prefix };
                return Ok(Some(vec![(source, replacement.replace('\n', &format!("{}{prefix}", document.file_format().spelling())))]));
            }
        }
    }
    if let Some(patches) = super::markdown_quotes::empty_insertion_patches(document, range, replacement)? {
        return Ok(Some(patches));
    }
    if let Some(patches) = join_following_paragraph(document, range, replacement)? {
        return Ok(Some(patches));
    }
    let block = document
        .projection()
        .blocks_for_region(range)
        .into_iter()
        .find(|block| block.range.start <= range.start && range.end <= block.range.end);
    let quoted_code = block.as_ref().map(|block| super::markdown_quotes::is_fenced_block(document, block))
        .transpose()?.unwrap_or(false);
    let code_block = quoted_code || block
        .as_ref()
        .is_some_and(|block| block.style.0 == "Code Block");
    if code_block && block.as_ref().is_some_and(|block| block.range.is_empty()) {
        return empty_body_patches(document, block.as_ref().unwrap(), replacement, quoted_code).map(Some);
    }
    if code_block && !replacement.contains(['`', '~']) {
        let block = block.as_ref().unwrap();
        if block.range.is_empty() {
            return Ok(None);
        }
        // The final code-body boundary is inside its fence, even when the
        // typing caret's ordinary downstream side would be after that syntax.
        // Keep this common path local: no full code-body or source capture.
        if !range.is_empty() || replacement.contains('\n') || range.end == block.range.end {
            let source = if range.is_empty() {
                let at = document
                    .projection()
                    .source_insertion_point(range.start, range.start != block.range.end)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                at..at
            } else {
                let mut source = document
                    .projection()
                    .source_range(range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if document.projection().hard_line_at_offset(range.end)
                    .and_then(|index| document.projection().hard_line_range(index))
                    .is_some_and(|line| line.start == range.end)
                {
                    source.end = super::source_edit::insertion_point(document.projection(), range.end, None)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                }
                source
            };
            let mut separator = document.file_format().spelling().to_owned();
            if replacement.contains('\n') && (quoted_code || matches!(block.kind, super::BlockKind::ListItem { .. }))
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
                if quoted_code || prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
                    separator.push_str(&prefix);
                }
            }
            return Ok(Some(vec![(source, replacement.replace('\n', &separator))]));
        }
        return Ok(None);
    }
    if !replacement.contains(['`', '~']) {
        // EOF has no following inline context. Its insertion point is inside
        // the last span's closing delimiter, so text typed there stays literal.
        // Paragraph breaks still go through structural split translation.
        if range.is_empty() && range.start == document.projection().text_tree().byte_len() && !replacement.contains('\n')
            && document.projection().style_spans_for_region(&(range.start.saturating_sub(1)..range.end)).iter().any(|span|
                span.range.end == range.start && span.application == StyleApplication::Semantic(SemanticInlineStyle::Code))
        {
            let at = super::source_edit::insertion_point(document.projection(), range.start, None)
                .ok_or(DocumentError::AmbiguousProjection)?;
            return Ok(Some(vec![(at..at, replacement.to_owned())]));
        }
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
    let input = if quoted_code {
        super::markdown_quotes::strip(&input, &super::markdown_quotes::classify(&input))
    } else { input };
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
                    && super::markdown_syntax::fence_close(plain, delimiter, length)
                {
                    closing = Some(offset..offset + plain.len());
                    break;
                }
                if offset + line.len() <= start
                    && super::markdown_syntax::fence_close(plain, delimiter, length)
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

/// An empty code body may already own a blank physical line, or its insertion
/// seed may be on the closing fence (or after an unterminated opener). Preserve
/// that distinction and the container prefix when materializing its text.
fn empty_body_patches(
    document: &Document,
    block: &super::Block,
    replacement: &str,
    quoted: bool,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let at = super::source_edit::insertion_point(document.projection(), block.range.start, None)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let original = super::line_endings::normalize(&decoded, document.file_format());
    let input = if quoted {
        super::markdown_quotes::strip(&original, &super::markdown_quotes::classify(&original))
    } else { original.clone() };
    let mapper = super::rich_text::Builder::new(&input, document.revision());
    let position = input.units.iter().find(|unit| unit.source.start == at)
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let mut opening = None;
    let mut closing = None;
    let mut offset = 0;
    for line in input.text.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        if let Some((_, delimiter, length)) = opening.as_ref() {
            if super::markdown_syntax::fence_close(body, *delimiter, *length) {
                if offset >= position {
                    closing = Some(offset..offset + body.len());
                    break;
                }
                opening = None;
            }
        } else if let Some((delimiter, length)) = super::projection::markdown_fence(body) {
            let indent = body.len() - body.trim_start_matches(' ').len();
            opening = Some((offset + indent..offset + indent + length, delimiter, length));
        }
        offset += line.len();
    }
    let (opening, delimiter, length) = opening.ok_or(DocumentError::AmbiguousProjection)?;
    let source_open = mapper.source_range(opening.clone());
    let physical = &document.state().source_hard_lines;
    let prefix_at = if position > 0 && !input.text[..position].ends_with('\n') {
        source_open.start
    } else { at };
    let prefix_line = physical.line_at_offset(prefix_at).and_then(|index| physical.get(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let prefix_bytes = document.state().source.bytes_in(prefix_line.start..prefix_at)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let prefix = document.encoding().decode_region(&prefix_bytes, prefix_line.start)?.text;
    let prefix = if prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'>')) { prefix } else { String::new() };
    let separator = format!("{}{prefix}", document.file_format().spelling());
    let mut syntax = String::new();
    if position > 0 && !input.text[..position].ends_with('\n') {
        syntax.push_str(&separator);
    }
    syntax.push_str(&replacement.replace('\n', &separator));
    if position < input.text.len() && !input.text[position..].starts_with('\n') {
        syntax.push_str(&separator);
    }
    let mut patches = vec![(at..at, syntax)];
    let needed = replacement.lines().filter_map(|line| {
        let line = line.trim();
        (!line.is_empty() && line.bytes().all(|byte| byte == delimiter)).then_some(line.len() + 1)
    }).max().unwrap_or(0).max(length);
    if needed > length {
        let marker = (delimiter as char).to_string().repeat(needed);
        patches.push((source_open, marker.clone()));
        if let Some(closing) = closing {
            patches.push((mapper.source_range(closing), marker));
        }
    }
    Ok(patches)
}

/// Joining the last code line owns its closing fence. Move that fence after
/// the following paragraph's visible content; otherwise removing only the
/// projected separator leaves source grammar between the joined characters.
fn join_following_paragraph(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    if range.is_empty() {
        return Ok(None);
    }
    if !replacement.contains('\n') {
        if let Some(patches) = join_code_into_previous(document, range, replacement)? {
            return Ok(Some(patches));
        }
    }
    let projection = document.projection();
    let blocks = projection.blocks_for_region(range);
    let Some(code) = blocks.iter().find(|block| {
        block.range.start <= range.start
            && range.start <= block.range.end
            && block.range.end < range.end
    }) else {
        return Ok(None);
    };
    if !is_fenced_paragraph(document, code)? { return Ok(None); }
    let Some(following) = blocks.iter().find(|block| {
        code.range.end < block.range.start
            && block.range.start <= range.end
            && range.end <= block.range.end
    }) else {
        return Ok(None);
    };
    let Some(fence) = fenced_source(document, code)? else { return Ok(None); };
    let Some(closing_text) = fence.closing_text.as_deref() else { return Ok(None); };
    let code_source = fence.body.clone();
    let delimiter = fence.delimiter;
    let opening_width = fence.width;
    let closing_width = closing_text.trim().len();
    let following_source = projection.source_range(following.range.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let source_lines = &document.state().source_hard_lines;
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
    let mut end = normalized
        .endings
        .last()
        .filter(|ending| ending.source.end == end_line.end)
        .map_or(end_line.end, |ending| ending.source.start);
    if is_fenced_paragraph(document, following)? {
        let Some(following_fence) = fenced_source(document, following)? else { return Ok(None); };
        end = following_fence.closing_content_end.unwrap_or(following_fence.body.end);
    } else if following.range.is_empty() {
        end = following_source.end;
    }
    let tail = projection
        .text_tree()
        .slice(range.end..following.range.end)
        .map_err(DocumentError::FormattedTextStorage)?;
    let mut appended = replacement.to_owned();
    appended.push_str(&tail);
    let last_line = projection
        .hard_line_at_offset(range.start)
        .and_then(|index| projection.hard_line_range(index))
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut joined_tail = projection
        .text_tree()
        .slice(last_line.start..range.start)
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
        let start = fence.opening_marker;
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
    let edit_start = if range.start == code.range.start {
        code_source.start
    } else {
        projection.source_insertion_point(range.start, false)
            .ok_or(DocumentError::AmbiguousProjection)?
    };
    patches.push((
        edit_start..end,
        appended.replace('\n', &format!("{}{}", document.file_format().spelling(), fence.body_prefix)),
    ));
    Ok(Some(patches))
}

/// Once a code paragraph joins preceding prose, its literal body needs prose
/// escaping and inline hard breaks. The selected contributors and delimiters
/// are separate patches so surrounding inline syntax remains intact.
fn join_code_into_previous(
    document: &Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    let projection = document.projection();
    let blocks = projection.blocks_for_region(range);
    let Some(first) = blocks.iter().find(|block| {
        block.range.start <= range.start && range.start <= block.range.end
    }) else {
        return Ok(None);
    };
    if is_fenced_paragraph(document, first)? { return Ok(None); }
    let Some(code) = blocks.iter().find(|block| {
        first.range.end < block.range.start && block.range.start <= range.end
            && range.end <= block.range.end
    }) else {
        return Ok(None);
    };
    if !is_fenced_paragraph(document, code)? { return Ok(None); }
    let Some(fence) = fenced_source(document, code)? else { return Ok(None); };
    let body = fence.body;
    let mut ranges = super::source_edit::visible_runs(projection, range)?
        .into_iter().map(|run| run.source).collect::<Vec<_>>();
    ranges.extend(selected_inline_delimiters(document, range)?);
    let separator = projection.source_range(first.range.end..first.range.end + 1)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let closing_range = fence.closing.map(|closing| body.end..closing.end);
    let tail = projection.text_tree().slice(range.end..code.range.end)
        .map_err(DocumentError::FormattedTextStorage)?;
    let tail_source = projection.source_insertion_point(range.end, range.end != code.range.end)
        .ok_or(DocumentError::AmbiguousProjection)?..body.end;
    ranges.push(separator.start..tail_source.start);
    let mut tail_syntax = tail.split('\n').map(|text| super::projection::escape_markdown_insert_in_encoding(text, document.encoding()))
        .collect::<Vec<_>>().join("<br>");
    if let Some(closing) = &closing_range {
        let following = code.range.end < projection.text_tree().byte_len();
        if following {
            tail_syntax.push_str(document.file_format().spelling());
            tail_syntax.push_str(document.file_format().spelling());
        }
        // The body suffix and its closing fence form one explicit conversion
        // patch; keep the following paragraph's source untouched.
        ranges.push(tail_source.start..closing.end);
    } else {
        ranges.push(tail_source);
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut().filter(|previous| range.start <= previous.end) {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let last = merged.len() - 1;
    Ok(Some(merged.into_iter().enumerate().map(|(index, source)| {
        let mut syntax = if index == 0 {
            super::projection::escape_markdown_insert_in_encoding(replacement, document.encoding())
        } else { String::new() };
        if index == last { syntax.push_str(&tail_syntax); }
        (source, syntax)
    }).collect()))
}

pub(super) struct FencedSource {
    pub(super) body: Range<usize>,
    pub(super) opening: Range<usize>,
    pub(super) closing: Option<Range<usize>>,
    pub(super) closing_content_end: Option<usize>,
    pub(super) closing_text: Option<String>,
    pub(super) delimiter: u8,
    pub(super) width: usize,
    opening_marker: usize,
    body_prefix: String,
}

fn is_fenced_paragraph(document: &Document, block: &super::Block) -> Result<bool, DocumentError> {
    Ok(block.style.0 == "Code Block" || super::markdown_quotes::is_fenced_block(document, block)?)
}

/// Fence syntax belongs to the paragraph even when its body has no characters.
/// Inspect only neighboring physical source lines around the projected body.
pub(super) fn fenced_source(document: &Document, block: &super::Block) -> Result<Option<FencedSource>, DocumentError> {
    let body = document.projection().source_range(block.range.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let lines = &document.state().source_hard_lines;
    let first = lines.line_at_offset(body.start).ok_or(DocumentError::AmbiguousProjection)?;
    let Some(opening) = lines.get(first.saturating_sub(1)) else {
        return Ok(None);
    };
    let read = |line: &Range<usize>| {
        let bytes = document.state().source.bytes_in(line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        document.encoding().decode_region(&bytes, line.start)
    };
    let text = read(&opening)?;
    let quote = super::markdown_quotes::prefix(&text.text);
    let mut prefix = quote + super::markdown_blocks::marker_prefix_length(&text.text[quote..]).unwrap_or(0);
    prefix += text.text[prefix..].len() - text.text[prefix..].trim_start_matches([' ', '\t']).len();
    let Some((delimiter, width)) = super::projection::markdown_fence(&text.text[prefix..]) else {
        return Ok(None);
    };
    let opening_marker = opening.start + document.encoding().encode_fragment(&text.text[..prefix])?.len();
    let first_line = lines.get(first).ok_or(DocumentError::AmbiguousProjection)?;
    let prefix_bytes = document.state().source.bytes_in(first_line.start..body.start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let body_prefix = document.encoding().decode_region(&prefix_bytes, first_line.start)?.text;
    let last = lines.line_at_offset(body.end).ok_or(DocumentError::AmbiguousProjection)?;
    let mut result = FencedSource { body, opening, closing: None, closing_content_end: None,
        closing_text: None, delimiter, width, opening_marker, body_prefix };
    for index in [last, last + 1] {
        let Some(line) = lines.get(index) else { continue; };
        if line.start < result.body.end { continue; }
        let decoded = read(&line)?;
        let normalized = super::line_endings::normalize(&decoded, document.file_format());
        let quote = super::markdown_quotes::prefix(&normalized.text);
        let line_body = normalized.text[quote..].trim_end_matches('\n');
        let container = if matches!(block.kind, super::BlockKind::ListItem { .. }) { result.body_prefix[super::markdown_quotes::prefix(&result.body_prefix)..].len() } else { 0 };
        let trim = line_body.bytes().take(container).take_while(|b| *b == b' ').count();
        let value = &line_body[trim..];
        if super::markdown_syntax::fence_close(value, delimiter, width) {
            result.closing_content_end = Some(normalized.endings.last()
                .filter(|ending| ending.source.end == line.end)
                .map_or(line.end, |ending| ending.source.start));
            result.closing_text = Some(value.to_owned());
            result.closing = Some(line);
            break;
        }
    }
    Ok(Some(result))
}

pub(super) fn delimiter_ranges(
    document: &Document,
    source: &Range<usize>,
) -> Result<Option<(Range<usize>, Range<usize>)>, DocumentError> {
    let tick = document.encoding().encode_fragment("`")?;
    let space = document.encoding().encode_fragment(" ")?;
    let slash = document.encoding().encode_fragment("\\")?;
    let width = tick.len();
    let matches = |at: usize, spelling: &[u8]| {
        document
            .state()
            .source
            .bytes_in(at..at + spelling.len())
            .as_deref()
            == Some(spelling)
    };
    let mut left = source.start;
    let mut right = source.end;
    if left >= width && matches(left - width, &space) {
        left -= width;
    }
    if matches(right, &space) {
        right += width;
    }
    let open_end = left;
    let close_start = right;
    while left >= width && matches(left - width, &tick) {
        left -= width;
    }
    let mut escaped = left;
    while escaped >= width && matches(escaped - width, &slash) {
        escaped -= width;
    }
    if left < open_end && (left - escaped) / width % 2 == 1 {
        left += width;
    }
    while matches(right, &tick) {
        right += width;
    }
    if open_end == left || open_end - left != right - close_start {
        return Ok(None);
    }
    Ok(Some((left..source.start, source.end..right)))
}

/// An entirely consumed inline code span also owns its delimiters. Keeping an
/// empty backtick pair would create literal characters after a paragraph join.
pub(super) fn selected_inline_delimiters(
    document: &Document,
    selected: &Range<usize>,
) -> Result<Vec<Range<usize>>, DocumentError> {
    let projection = document.projection();
    let mut ranges = Vec::new();
    for span in projection.style_spans_for_region(selected) {
        if span.application != StyleApplication::Semantic(SemanticInlineStyle::Code)
            || span.range.is_empty()
            || span.range.start < selected.start
            || selected.end < span.range.end
        {
            continue;
        }
        let source = projection
            .source_range(span.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        if let Some((opening, closing)) = delimiter_ranges(document, &source)? {
            ranges.extend([opening, closing]);
        }
    }
    Ok(ranges)
}

/// Backtick syntax must remain paired after every text/payload translation.
/// Remove empty spans, join newly adjacent spans, and update required padding;
/// retained body bytes stay outside these supporting syntax patches.
pub(super) fn preserve_edited_inline_delimiters(
    document: &Document,
    edits: &[super::TextEdit],
    patches: &mut Vec<super::SourcePatch>,
) -> Result<(), DocumentError> {
    struct Run {
        opening: Range<usize>,
        closing: Range<usize>,
        body: String,
        width: usize,
    }
    let mut spans = edits
        .iter()
        .flat_map(|edit| {
            let band = edit.range.start.saturating_sub(1)
                ..(edit.range.end + 1).min(document.projection().text_tree().byte_len());
            document.projection().style_spans_for_region(&band)
        })
        .filter(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Code))
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, span.range.end));
    spans.dedup_by(|left, right| left.range == right.range);
    let mut runs = Vec::new();
    let mut support = Vec::new();
    for span in spans {
        if span.range.is_empty() {
            continue;
        }
        let content = document
            .projection()
            .source_range(span.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let Some((opening, closing)) = delimiter_ranges(document, &content)? else {
            continue;
        };
        // Fence conversions and explicit code-typing rewrites own their syntax.
        if patches.iter().any(|patch| {
            !patch.replacement().is_empty()
                && (patch.range().start < content.start && opening.start < patch.range().end
                    || patch.range().start < closing.end && content.end < patch.range().end)
        }) {
            continue;
        }
        let body = edited_source_fragment(document, &content, patches)?;
        if body.is_empty() {
            super::source_edit::append_uncovered_deletions(&opening, patches, &mut support);
            super::source_edit::append_uncovered_deletions(&closing, patches, &mut support);
            continue;
        }
        let body = document
            .encoding()
            .decode_region(&body, content.start)?
            .text;
        let body = body.replace(document.file_format().spelling(), " ");
        let bytes = document
            .state()
            .source
            .bytes_in(opening.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let marker = document
            .encoding()
            .decode_region(&bytes, opening.start)?
            .text;
        let width = marker.bytes().take_while(|byte| *byte == b'`').count();
        runs.push(Run {
            opening,
            closing,
            body,
            width,
        });
    }
    // Empty-span cleanup can be the only syntax between two surviving spans.
    patches.extend(support);
    let mut support = Vec::new();
    let mut first = 0;
    while first < runs.len() {
        let mut end = first + 1;
        while end < runs.len() {
            let gap = runs[end - 1].closing.end..runs[end].opening.start;
            if gap.start > gap.end || !edited_source_fragment(document, &gap, patches)?.is_empty() {
                break;
            }
            end += 1;
        }
        let group = &runs[first..end];
        first = end;
        let body = group
            .iter()
            .map(|run| run.body.as_str())
            .collect::<String>();
        if group.len() == 1 {
            let run = &group[0];
            let read = |range: &Range<usize>| {
                let bytes = document
                    .state()
                    .source
                    .bytes_in(range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                document
                    .encoding()
                    .decode_region(&bytes, range.start)
                    .map(|decoded| decoded.text)
            };
            let opening = read(&run.opening)?;
            let closing = read(&run.closing)?;
            let left_pad = opening.trim_start_matches('`');
            let right_pad = closing.trim_end_matches('`');
            let raw = format!("{left_pad}{body}{right_pad}");
            let projected = if raw.starts_with(' ')
                && raw.ends_with(' ')
                && !raw.chars().all(|character| character == ' ')
            {
                &raw[1..raw.len() - 1]
            } else {
                &raw
            };
            if projected == body
                && !raw.starts_with('`')
                && !raw.ends_with('`')
                && !raw
                    .split(|character| character != '`')
                    .any(|ticks| ticks.len() == run.width)
            {
                // Optional original padding and a longer marker are preserved
                // whenever the surviving body still has its exact spelling.
                continue;
            }
        }
        let width = group.iter().map(|run| run.width).max().unwrap().max(
            body.split(|character| character != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let marker = "`".repeat(width);
        let pad = if body.starts_with('`')
            || body.ends_with('`')
            || body.starts_with(' ') && body.ends_with(' ') && !body.trim().is_empty()
        {
            " "
        } else {
            ""
        };
        for (range, syntax) in [
            (&group[0].opening, format!("{marker}{pad}")),
            (&group.last().unwrap().closing, format!("{pad}{marker}")),
        ] {
            let replacement = document.encoding().encode_fragment(&syntax)?;
            if document.state().source.bytes_in(range.clone()).as_deref()
                != Some(replacement.as_slice())
            {
                support.push(super::SourcePatch::primary(range.clone(), replacement));
            }
        }
        for pair in group.windows(2) {
            super::source_edit::append_uncovered_deletions(&pair[0].closing, patches, &mut support);
            super::source_edit::append_uncovered_deletions(&pair[1].opening, patches, &mut support);
        }
    }
    patches.extend(support);
    Ok(())
}

pub(super) fn edited_source_fragment(
    document: &Document,
    content: &Range<usize>,
    patches: &[super::SourcePatch],
) -> Result<Vec<u8>, DocumentError> {
    let mut body = document
        .state()
        .source
        .bytes_in(content.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut relevant = patches
        .iter()
        .filter(|patch| {
            patch.range().start < content.end && content.start < patch.range().end
                || patch.range().is_empty()
                    && content.start <= patch.range().start
                    && patch.range().start <= content.end
        })
        .collect::<Vec<_>>();
    relevant.sort_by_key(|patch| (patch.range().start, patch.range().end));
    for patch in relevant.into_iter().rev() {
        let start = patch.range().start.max(content.start) - content.start;
        let end = patch.range().end.min(content.end) - content.start;
        body.splice(start..end, patch.replacement().iter().copied());
    }
    Ok(body)
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
    let text = super::projection::escape_markdown_insert_in_encoding(&document.text()[selected.clone()], document.encoding());
    Ok(vec![
        (opening, open),
        (source, format!("{prefix_end}{text}{suffix_start}")),
        (closing, close),
    ])
}
