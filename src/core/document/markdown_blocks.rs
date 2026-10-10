//! Lossless physical-line context for Markdown list paragraphs. Syntax is
//! classified before whitespace is projected, so continuation indentation and
//! lazy continuation lines remain attached to their original item.
use super::line_endings::NormalizedText;
use super::projection::BlockKind;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct ListLine {
    pub kind: BlockKind,
    pub content_start: usize,
    pub marker: Option<Range<usize>>,
    pub code: bool,
    pub content_indent: usize,
}

pub(super) fn source_context(input: &NormalizedText) -> Vec<(Range<usize>, ListLine)> {
    let lines = super::paragraph_flow::source_lines(input);
    let source_at = |at: usize| {
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
        .enumerate()
        .filter_map(|(index, line)| {
            line.map(|mut line| {
                line.content_start = source_at(line.content_start);
                line.marker = line
                    .marker
                    .map(|range| source_at(range.start)..source_at(range.end));
                (
                    source_at(lines[index].start)..source_at(lines[index].end),
                    line,
                )
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
struct Marker {
    start: usize,
    content_start: usize,
    content_indent: usize,
}

fn indentation(text: &str) -> (usize, usize) {
    let mut columns = 0;
    let mut bytes = 0;
    for byte in text.bytes() {
        match byte {
            b' ' => columns += 1,
            b'\t' => columns += 4 - columns % 4,
            _ => break,
        }
        bytes += 1;
    }
    (bytes, columns)
}

fn marker(text: &str, origin: usize) -> Option<Marker> {
    let (indent_bytes, indent) = indentation(text);
    let body = &text.as_bytes()[indent_bytes..];
    let digits = body.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let (width, delimiter, ordered) = if matches!(body.first(), Some(b'-' | b'+' | b'*')) {
        (1, body[0], false)
    } else if (1..=9).contains(&digits) && matches!(body.get(digits), Some(b'.' | b')')) {
        (digits + 1, body[digits], true)
    } else {
        return None;
    };
    if body
        .get(width)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t'))
    {
        return None;
    }
    if !ordered
        && text
            .bytes()
            .filter(|byte| !matches!(byte, b' ' | b'\t'))
            .all(|byte| byte == delimiter)
        && body.iter().filter(|byte| **byte == delimiter).count() >= 3
    {
        return None;
    }
    let mut content = indent_bytes + width;
    let mut column = indent + width;
    let start_column = column;
    while let Some(byte @ (b' ' | b'\t')) = text.as_bytes().get(content) {
        column += if *byte == b'\t' { 4 - column % 4 } else { 1 };
        content += 1;
    }
    if column - start_column > 4 {
        content = indent_bytes + width + 1;
        column = start_column + 1;
    }
    Some(Marker {
        start: origin + indent_bytes,
        content_start: origin + content,
        content_indent: column.max(start_column + 1),
    })
}

pub(super) fn marker_prefix_length(text: &str) -> Option<usize> {
    marker_prefix_geometry(text).map(|(bytes, _)| bytes)
}

pub(super) fn marker_prefix_geometry(text: &str) -> Option<(usize, usize)> {
    // Physical source-line slices may retain their ending. A bare marker such
    // as `1.` is still an empty item when followed by that line ending.
    marker(text.trim_end_matches(['\r', '\n']), 0)
        .map(|marker| (marker.content_start, marker.content_indent))
}

pub(super) fn classify(input: &NormalizedText) -> Vec<Option<ListLine>> {
    let syntax =
        super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(input))
            .to_source(input);
    classify_with_syntax(input, &syntax)
}

/// Reuse the original grammar ownership after quote prefixes or whitespace
/// have been projected. Source identities, rather than the cooked spelling,
/// decide which item owns each physical row.
pub(super) fn classify_with_syntax(
    input: &NormalizedText,
    syntax: &super::markdown_syntax::Blocks,
) -> Vec<Option<ListLine>> {
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
    let normalized_at = |at| {
        input
            .units
            .get(input.units.partition_point(|unit| unit.source.start < at))
            .map_or(input.text.len(), |unit| unit.normalized.start)
    };
    let lines = super::paragraph_flow::source_lines(input);
    let quotes = super::markdown_quotes::classify(input);
    let content_indents: Vec<_> = syntax
        .containers
        .iter()
        .map(|container| {
            let at = normalized_at(container.range.start);
            let index = lines
                .partition_point(|line| line.end < at)
                .min(lines.len() - 1);
            let origin = quotes[index].content_start.min(at);
            let mut column = 0;
            for byte in input.text[origin..at].bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            marker(&input.text[at..lines[index].end], at)
                .map_or(0, |marker| column + marker.content_indent)
        })
        .collect();
    let mut active = Vec::new();
    let mut next = 0;
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let start = source_at(line.start);
            let end = source_at(line.end);
            active.retain(|container: &&super::markdown_syntax::Container| {
                start < container.range.end
            });
            while let Some(container) = syntax
                .containers
                .get(next)
                .filter(|container| container.range.start <= end)
            {
                if start < container.range.end {
                    active.push(container);
                }
                next += 1;
            }
            let code_scope = syntax
                .code
                .get(syntax.code.partition_point(|code| code.range.end <= start))
                .filter(|code| code.range.start <= end && start < code.range.end);
            // Surplus separator rows are editable empty paragraphs. Literal code
            // rows still need their item's prefix to preserve the empty body.
            if code_scope.is_none() && input.text[line.clone()].trim().is_empty() {
                return None;
            }
            let container = active
                .iter()
                .copied()
                .filter(|container| container.list.is_some())
                .max_by_key(|container| match container.list {
                    Some(BlockKind::ListItem { level, .. }) => (level, container.range.start),
                    _ => unreachable!(),
                })?;
            let mut kind = container.list.clone().unwrap();
            let item_start = start <= container.range.start && container.range.start <= end;
            if let BlockKind::ListItem {
                item_start: first,
                container_start,
                ..
            } = &mut kind
            {
                *first = item_start;
                *container_start &= item_start;
            }
            let marker_start = normalized_at(container.range.start)
                .max(line.start)
                .min(line.end);
            let marker = item_start
                .then(|| marker(&input.text[marker_start..line.end], marker_start))
                .flatten();
            let container_index = syntax
                .containers
                .partition_point(|entry| entry.range.start < container.range.start);
            let content_indent = syntax.containers[container_index..]
                .iter()
                .zip(&content_indents[container_index..])
                .find(|(entry, _)| std::ptr::eq(*entry, container))
                .map_or(0, |(_, indent)| *indent);
            let origin = quotes[index].content_start;
            let mut bytes = 0;
            let mut columns = 0;
            for byte in input.text[origin..line.end]
                .bytes()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
            {
                if code_scope.is_some_and(|code| code.fenced) && columns >= content_indent {
                    break;
                }
                columns += if byte == b'\t' { 4 - columns % 4 } else { 1 };
                bytes += 1;
            }
            let content_start = marker
                .as_ref()
                .map_or(origin + bytes, |marker| marker.content_start);
            let code = code_scope.is_some();
            Some(ListLine {
                kind,
                content_start,
                marker: marker.map(|marker| marker.start..content_start),
                code,
                content_indent,
            })
        })
        .collect()
}
