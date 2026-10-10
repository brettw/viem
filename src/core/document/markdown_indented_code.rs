//! GFM indented code classification, before paragraph whitespace is folded.
//! Source coordinates keep the same ownership in both Markdown presentations.
use super::line_endings::NormalizedText;
use super::projection::{markdown_block_prefix, markdown_fence, BlockKind};
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct CodeLine {
    pub source: Range<usize>,
    pub content_start: usize,
    pub padding: usize,
}

#[derive(Clone, Debug)]
pub(super) struct CodeBlock {
    pub source: Range<usize>,
    pub lines: Vec<CodeLine>,
    pub container_indent: usize,
}

/// Consume indentation in columns. A tab crossing the boundary leaves spaces
/// in the literal body; its physical byte remains their shared contributor.
fn strip_indent(text: &str, target: usize) -> Option<(usize, usize)> {
    strip_indent_from(text, target, 0)
}

fn strip_indent_from(text: &str, target: usize, mut column: usize) -> Option<(usize, usize)> {
    for (at, byte) in text.bytes().enumerate() {
        column += match byte {
            b' ' => 1,
            b'\t' => 4 - column % 4,
            _ => return None,
        };
        if column >= target {
            return Some((at + 1, column - target));
        }
    }
    None
}

pub(super) fn classify(input: &NormalizedText) -> Vec<CodeBlock> {
    let quotes = super::markdown_quotes::classify(input);
    let body = super::markdown_quotes::strip(input, &quotes);
    let lines = super::paragraph_flow::source_lines(&body);
    let lists = super::markdown_blocks::classify(&body);
    let original = super::paragraph_flow::source_lines(input);
    let source_at = |input: &NormalizedText, at: usize| {
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
    let mut result = Vec::new();
    let mut index = 0;
    let mut prose = false;
    let mut fence = None;
    while index < lines.len() {
        let line = &lines[index];
        let text = &body.text[line.clone()];
        let context = lists[index].as_ref();
        if index > 0 && quotes[index].depth != quotes[index - 1].depth {
            // A fence cannot survive the end of its quote container. The next
            // line can independently begin an indented code block.
            fence = None;
        }
        if index > 0
            && (quotes[index].depth != quotes[index - 1].depth
                || context
                    .zip(lists[index - 1].as_ref())
                    .is_some_and(|(a, b)| a.paragraph != b.paragraph))
        {
            prose = false;
        }
        if let Some((delimiter, width)) = fence {
            let prefix = context.map_or(0, |line| line.content_start - lines[index].start);
            if super::markdown_syntax::fence_close(&text[prefix..], delimiter, width) {
                fence = None;
            }
            index += 1;
            prose = false;
            continue;
        }
        if text.trim().is_empty() {
            prose = false;
            index += 1;
            continue;
        }
        let marker = context.and_then(|line| line.marker.as_ref());
        let base = context.map_or(0, |line| line.content_indent);
        let start = marker.map_or(line.start, |marker| marker.end);
        let indent = strip_indent_from(
            &body.text[start..line.end],
            base + 4,
            if marker.is_some() { base } else { 0 },
        );
        if let Some((skip, padding)) = indent.filter(|_| !prose || marker.is_some()) {
            let depth = quotes[index].depth;
            let mut code_lines = Vec::new();
            let mut next = index;
            let mut last_nonblank = index;
            while next < lines.len() && quotes[next].depth == depth {
                let row = &lines[next];
                let raw = &body.text[row.clone()];
                let blank = raw.trim().is_empty();
                let content = if next == index {
                    Some((start + skip, padding))
                } else if blank {
                    Some(
                        strip_indent(raw, base + 4)
                            .map_or((row.end, 0), |(skip, padding)| (row.start + skip, padding)),
                    )
                } else if lists[next]
                    .as_ref()
                    .is_some_and(|line| line.marker.is_some())
                {
                    None
                } else {
                    strip_indent(raw, base + 4).map(|(skip, padding)| (row.start + skip, padding))
                };
                let Some((content, padding)) = content else {
                    break;
                };
                code_lines.push(CodeLine {
                    source: source_at(input, original[next].start)
                        ..source_at(input, original[next].end),
                    content_start: source_at(&body, content),
                    padding,
                });
                if !blank {
                    last_nonblank = next;
                }
                next += 1;
            }
            code_lines.truncate(last_nonblank - index + 1);
            result.push(CodeBlock {
                source: code_lines[0].source.start..code_lines.last().unwrap().source.end,
                lines: code_lines,
                container_indent: base,
            });
            index = last_nonblank + 1;
            prose = false;
            continue;
        }
        let semantic = context.map_or(text, |context| &body.text[context.content_start..line.end]);
        if let Some(open) = markdown_fence(semantic) {
            fence = Some(open);
            prose = false;
        } else {
            prose = !semantic.trim().is_empty()
                && markdown_block_prefix(semantic, 0, semantic.len()).1 == BlockKind::Paragraph;
        }
        index += 1;
    }
    result
}

pub(super) fn source_block(
    document: &super::Document,
    block: &super::Block,
) -> Result<Option<CodeBlock>, super::DocumentError> {
    if block.style.0 != "Code Block" {
        return Ok(None);
    }
    let at = document
        .projection()
        .source_insertion_point(block.range.start, true)
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    if !document.format().is_source_view() {
        let row = document
            .state()
            .source_hard_lines
            .line_at_offset(at)
            .and_then(|index| document.state().source_hard_lines.get(index))
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let bytes = document
            .state()
            .source
            .bytes_in(row.start..at)
            .ok_or(super::DocumentError::AmbiguousProjection)?;
        let prefix = document.encoding().decode_region(&bytes, row.start)?.text;
        let quote = super::markdown_quotes::prefix(&prefix);
        let marker = super::markdown_blocks::marker_prefix_length(&prefix[quote..]).unwrap_or(0);
        if strip_indent(&prefix[quote + marker..], 4).is_none() {
            return Ok(None);
        }
    }
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    Ok(classify(&input)
        .into_iter()
        .find(|code| code.source.start <= at && at <= code.source.end))
}
