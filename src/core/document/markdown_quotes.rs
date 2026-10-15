//! Physical Markdown quote containers, retained independently of their body
//! syntax so headings, lists, code, and lazy prose keep their source spelling.
use super::line_endings::NormalizedText;
use super::projection::{markdown_fence, BlockKind};
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
                && (block.style.0 == "Block quote" || block.quote_depth > 0)
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
        document.escape_markdown_source_text(at, replacement)?,
    )]))
}

pub(super) fn is_fenced_block(
    document: &super::Document,
    block: &super::Block,
) -> Result<bool, super::DocumentError> {
    if block.style.0 != "Block quote" && block.quote_depth == 0 {
        return Ok(false);
    }
    if !block.range.is_empty()
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
    for line in [
        Some(index),
        index.checked_sub(1),
    ]
    .into_iter()
    .flatten()
    {
        let source = lines
            .get(line)
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let bytes = document
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let decoded = document.encoding().decode_region(&bytes, source.start)?;
        if markdown_fence(&decoded.text[container_prefix(&decoded.text)..]).is_some() {
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

/// A fenced owner may alternate list and quote containers on its opening row.
/// This is used only while identifying an already parsed code paragraph.
pub(super) fn container_prefix(text: &str) -> usize {
    let mut at = 0;
    loop {
        let before = at;
        at += prefix(&text[at..]);
        at += super::markdown_blocks::marker_prefix_length(&text[at..]).unwrap_or(0);
        at += text[at..].len() - text[at..].trim_start_matches([' ', '\t']).len();
        if at == before { return at; }
    }
}

pub(super) fn prefix_with_limit(text: &str, limit: usize) -> (usize, usize) {
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
    let syntax =
        super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(input)).to_source(input);
    classify_with_syntax(input, &syntax)
}

/// Shared grammar ranges are physical source coordinates, including after a
/// quote prefix has been removed from the normalized stream.
pub(super) fn classify_with_syntax(input: &NormalizedText, syntax: &super::markdown_syntax::Blocks) -> Vec<QuoteLine> {
    let mut active = Vec::new();
    let mut next = 0;
    super::paragraph_flow::source_lines(input)
        .into_iter()
        .map(|range| {
            let source = input.source_at(range.start)..input.source_at(range.end);
            active.retain(|container: &&super::markdown_syntax::Container| {
                source.start < container.range.end
            });
            while let Some(container) = syntax
                .containers
                .get(next)
                .filter(|container| container.range.start <= source.end)
            {
                if source.start < container.range.end {
                    active.push(container);
                }
                next += 1;
            }
            let depth = active
                .iter()
                .map(|container| container.quote_depth)
                .max()
                .unwrap_or(0);
            let mut content_start =
                range.start + prefix_with_limit(&input.text[range.clone()], depth).0;
            // A parser-owned quote inside a deep list can follow four or
            // more indentation columns. The root quote-prefix rule allows
            // only three; apply it after the enclosing list's indentation.
            // The grammar ownership is required so indented literal `>` text
            // cannot become a quote merely because it resembles a marker.
            if depth > 0 && content_start == range.start
                && active.iter().any(|container| container.list.is_some()) {
                let raw = &input.text[range.clone()];
                let indentation = raw.len() - raw.trim_start_matches([' ', '\t']).len();
                let quote = prefix_with_limit(&raw[indentation..], depth).0;
                if quote > 0 { content_start = range.start + indentation + quote; }
            }
            // A quote may begin after a list marker on the same physical row.
            // Its parser-owned start supplies the prefix that a row-only scan misses.
            // Quotes opened on one row are nested, and the innermost one's
            // walk reaches every marker that an ancestor's shorter-limited
            // walk could, so only it can extend the prefix. Rescanning each
            // ancestor made a row of N nested quote markers quadratic.
            if let Some(container) = active.iter().rev().find(|container| {
                container.list.is_none()
                    && source.start <= container.range.start
                    && container.range.start <= source.end
            }) {
                let (prefix, _) = prefix_with_limit(
                    &input.text[input.normalized_at(container.range.start)..range.end],
                    container.quote_depth,
                );
                super::work_statistics::record(|stats| {
                    stats.markdown_container_prefix_bytes += prefix;
                });
                content_start = content_start.max(input.normalized_at(container.range.start) + prefix);
            }
            QuoteLine {
                range,
                content_start,
                depth,
            }
        })
        .collect()
}

pub(super) fn strip(input: &NormalizedText, lines: &[QuoteLine]) -> NormalizedText {
    let mut result = NormalizedText::new(input.encoding);
    result.endings.reserve(input.endings.len());
    let mut line = 0;
    let mut ending = 0;
    for unit in input.units() {
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
        result.push_unit(next);
    }
    result
}

pub(super) fn source_context_with_syntax(input: &NormalizedText, syntax: &super::markdown_syntax::Blocks) -> Vec<QuoteLine> {
    let source_at = |at| input.source_at(at);
    classify_with_syntax(input, syntax)
        .into_iter()
        .map(|line| QuoteLine {
            range: source_at(line.range.start)..source_at(line.range.end),
            content_start: source_at(line.content_start),
            depth: line.depth,
        })
        .collect()
}
