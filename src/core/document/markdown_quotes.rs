//! Physical Markdown quote containers, retained independently of their body
//! syntax so headings, lists, code, and lazy prose keep their source spelling.
use super::line_endings::NormalizedText;
use super::projection::{markdown_block_prefix, markdown_fence, BlockKind};
use std::ops::Range;

pub(super) fn empty_insertion_patches(
    document: &super::Document,
    range: &Range<usize>,
    replacement: &str,
) -> Result<Option<Vec<(Range<usize>, String)>>, super::DocumentError> {
    if !range.is_empty() || replacement.contains('\n') {
        return Ok(None);
    }
    let Some(block) = document
        .projection()
        .blocks_for_region(range)
        .into_iter()
        .find(|block| {
            block.range == *range
                && block.style.0 == "Block quote"
                && matches!(block.kind, BlockKind::Paragraph)
        })
    else {
        return Ok(None);
    };
    if is_fenced_block(document, &block)? {
        return Ok(None);
    }
    let source_at = document
        .projection()
        .provenance_touching(range)
        .iter()
        .find(|span| span.formatted == *range)
        .map(|span| span.source.start)
        .or_else(|| {
            document
                .projection()
                .source_insertion_point(range.start, true)
        })
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    let line = document
        .state()
        .source_hard_lines
        .line_at_offset(source_at)
        .and_then(|index| document.state().source_hard_lines.get(index))
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    let bytes = document
        .state()
        .source
        .bytes_in(line.clone())
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, line.start)?;
    let prefix = prefix(&decoded.text);
    if prefix == 0 || !decoded.text[prefix..].trim().is_empty() {
        return Err(super::DocumentError::AmbiguousProjection);
    }
    let at = line.start
        + document
            .encoding()
            .encode_fragment(&decoded.text[..prefix])?
            .len();
    Ok(Some(vec![(
        at..at,
        super::projection::escape_markdown_insert_in_encoding(replacement, document.encoding()),
    )]))
}

pub(super) fn is_fenced_block(
    document: &super::Document,
    block: &super::Block,
) -> Result<bool, super::DocumentError> {
    if block.style.0 != "Block quote" && block.quote_depth == 0 {
        return Ok(false);
    }
    if !document.format().is_source_view() && !block.range.is_empty()
        && !document
            .projection()
            .style_spans_for_region(&block.range)
            .iter()
            .any(|span| {
                span.range.start == block.range.start
                    && span.application
                        == super::StyleApplication::Semantic(super::SemanticInlineStyle::Code)
            })
    {
        return Ok(false);
    }
    let at = document
        .projection()
        .source_insertion_point(block.range.start, true)
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    let lines = &document.state().source_hard_lines;
    let index = lines
        .line_at_offset(at)
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    for line in [Some(index), index.checked_sub(1).filter(|_| !document.format().is_source_view())].into_iter().flatten() {
        let source = lines
            .get(line)
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let bytes = document
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, source.start)?;
        if markdown_fence(&decoded.text[prefix(&decoded.text)..]).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Debug)]
pub(super) struct QuoteLine {
    pub range: Range<usize>,
    pub content_start: usize,
    pub depth: usize,
}

pub(super) fn prefix(text: &str) -> usize {
    prefix_with_limit(text, usize::MAX).0
}

fn prefix_with_limit(text: &str, limit: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut depth = 0;
    while depth < limit {
        let mut marker = at;
        while marker < bytes.len() && bytes[marker] == b' ' && marker - at < 3 {
            marker += 1;
        }
        if bytes.get(marker) != Some(&b'>') {
            break;
        }
        at = marker + 1;
        if matches!(bytes.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        depth += 1;
    }
    (at, depth)
}

pub(super) fn classify(input: &NormalizedText) -> Vec<QuoteLine> {
    let mut previous_depth = 0;
    let mut lazy = false;
    let mut fence: Option<(usize, u8, usize)> = None;
    super::paragraph_flow::source_lines(input)
        .into_iter()
        .map(|range| {
            let text = &input.text[range.clone()];
            let (prefix, mut depth) = if fence.is_some_and(|(depth, _, _)| depth == 0) {
                (0, 0)
            } else {
                prefix_with_limit(text, fence.map_or(usize::MAX, |(depth, _, _)| depth))
            };
            let body = &text[prefix..];
            let (_, kind) = markdown_block_prefix(body, 0, body.len());
            let prose = !body.trim().is_empty()
                && kind == BlockKind::Paragraph
                && markdown_fence(body).is_none()
                && !body.starts_with("    ")
                && !body.starts_with('\t')
                && !thematic_or_setext(body);
            if depth == 0 && previous_depth > 0 && lazy && prose && fence.is_none() {
                depth = previous_depth;
            }
            if let Some((quote_depth, delimiter, length)) = fence {
                if depth != quote_depth
                    || super::markdown_syntax::fence_close(body, delimiter, length)
                {
                    fence = None;
                }
                lazy = false;
            } else if let Some((delimiter, length)) = markdown_fence(body) {
                fence = Some((depth, delimiter, length));
                lazy = false;
            } else {
                lazy = depth > 0 && (prose || matches!(kind, BlockKind::ListItem { .. }));
            }
            previous_depth = depth;
            QuoteLine {
                content_start: range.start + prefix,
                range,
                depth,
            }
        })
        .collect()
}

fn thematic_or_setext(text: &str) -> bool {
    let body = text.trim();
    let Some(marker @ (b'-' | b'*' | b'_' | b'=')) = body.bytes().next() else {
        return false;
    };
    let count = body.bytes().filter(|byte| *byte == marker).count();
    count >= if marker == b'=' { 1 } else { 3 }
        && body
            .bytes()
            .all(|byte| byte == marker || matches!(byte, b' ' | b'\t'))
}

pub(super) fn strip(input: &NormalizedText, lines: &[QuoteLine]) -> NormalizedText {
    let mut result = NormalizedText {
        text: String::new(),
        units: Vec::with_capacity(input.units.len()),
        endings: Vec::with_capacity(input.endings.len()),
        encoding: input.encoding,
    };
    let mut line = 0;
    let mut ending = 0;
    for unit in &input.units {
        while line + 1 < lines.len() && lines[line].range.end < unit.normalized.start {
            line += 1;
        }
        if unit.normalized.start < lines[line].content_start {
            continue;
        }
        let start = result.text.len();
        result.text.push_str(&input.text[unit.normalized.clone()]);
        let mut next = unit.clone();
        next.normalized = start..result.text.len();
        while ending < input.endings.len()
            && input.endings[ending].normalized.start < unit.normalized.start
        {
            ending += 1;
        }
        if let Some(source_ending) = input
            .endings
            .get(ending)
            .filter(|ending| ending.normalized == unit.normalized)
        {
            let mut next_ending = source_ending.clone();
            next_ending.normalized = next.normalized.clone();
            result.endings.push(next_ending);
        }
        result.units.push(next);
    }
    result
}

pub(super) fn source_context(input: &NormalizedText) -> Vec<QuoteLine> {
    let source_at = |at| {
        input
            .units
            .get(
                input
                    .units
                    .partition_point(|unit| unit.normalized.start < at),
            )
            .map_or_else(
                || input.units.last().map_or(0, |unit| unit.source.end),
                |unit| unit.source.start,
            )
    };
    classify(input)
        .into_iter()
        .map(|line| QuoteLine {
            range: source_at(line.range.start)..source_at(line.range.end),
            content_start: source_at(line.content_start),
            depth: line.depth,
        })
        .collect()
}
