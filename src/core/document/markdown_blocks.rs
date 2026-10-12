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
    marker_with_break(text, origin, |delimiter| {
        text.bytes()
            .filter(|byte| !matches!(byte, b' ' | b'\t'))
            .all(|byte| byte == delimiter)
            && text.bytes().filter(|byte| *byte == delimiter).count() >= 3
    })
}

/// Thematic-break facts for the end of one row. Each of many list markers on
/// a row (`- - - … a`) then answers in O(1) instead of rescanning the row.
struct RowBreaks {
    start: usize,
    /// Bytes examined from the end of the row.
    scanned: usize,
    /// The last nonblank byte that differs from each of `-`, `+` and `*`.
    other: [Option<usize>; 3],
    /// The third-last nonblank byte.
    third_last: Option<usize>,
}

impl RowBreaks {
    fn new(text: &str, start: usize) -> Self {
        let mut other = [None; 3];
        let mut third_last = None;
        let mut nonblank = 0;
        let mut scanned = 0;
        for (at, byte) in text.bytes().enumerate().rev() {
            scanned += 1;
            if matches!(byte, b' ' | b'\t') {
                continue;
            }
            nonblank += 1;
            if nonblank == 3 {
                third_last = Some(start + at);
            }
            for (slot, delimiter) in other.iter_mut().zip(*b"-+*") {
                if slot.is_none() && byte != delimiter {
                    *slot = Some(start + at);
                }
            }
            if third_last.is_some() && other.iter().all(Option::is_some) {
                break;
            }
        }
        Self { start, scanned, other, third_last }
    }

    /// Whether the row from `at` is a thematic break spelled with `delimiter`.
    fn thematic(&self, at: usize, delimiter: u8) -> bool {
        let slot = b"-+*".iter().position(|candidate| *candidate == delimiter);
        slot.is_some_and(|slot| self.other[slot].is_none_or(|other| other < at))
            && self.third_last.is_some_and(|third| at <= third)
    }
}

fn marker_with_break(
    text: &str,
    origin: usize,
    thematic_break: impl FnOnce(u8) -> bool,
) -> Option<Marker> {
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
    if !ordered && thematic_break(delimiter) {
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
    // Containers arrive in source order. Successive containers on one row
    // continue from the previous column and share the row's break facts, so
    // N nested markers on a row cost O(row) rather than O(N × row).
    let mut column_from: Option<(usize, usize, usize, usize)> = None;
    let mut breaks: Option<(usize, RowBreaks)> = None;
    let content_indents: Vec<_> = syntax
        .containers
        .iter()
        .map(|container| {
            let at = normalized_at(container.range.start);
            let index = lines
                .partition_point(|line| line.end < at)
                .min(lines.len() - 1);
            let origin = quotes[index].content_start.min(at);
            let (from, mut column) = match column_from {
                Some((row, previous_origin, previous, column))
                    if row == index && previous_origin == origin && previous <= at =>
                {
                    (previous, column)
                }
                _ => (origin, 0),
            };
            for byte in input.text[from..at].bytes() {
                column += if byte == b'\t' { 4 - column % 4 } else { 1 };
            }
            column_from = Some((index, origin, at, column));
            let end = lines[index].end;
            let mut scanned = at - from;
            if !breaks
                .as_ref()
                .is_some_and(|(row, breaks)| *row == index && breaks.start <= at)
            {
                let start = lines[index].start.min(at);
                let row = RowBreaks::new(&input.text[start..end], start);
                scanned += row.scanned;
                breaks = Some((index, row));
            }
            super::work_statistics::record(|stats| {
                stats.markdown_container_prefix_bytes += scanned;
            });
            let (_, breaks) = breaks.as_ref().expect("current row breaks");
            marker_with_break(&input.text[at..end], at, |delimiter| {
                breaks.thematic(at, delimiter)
            })
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
