//! Lossless physical-line context for Markdown list paragraphs. Syntax is
//! classified before whitespace is projected, so continuation indentation and
//! lazy continuation lines remain attached to their original item.
use super::line_endings::NormalizedText;
use super::projection::{markdown_block_prefix, markdown_fence, BlockKind};
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct ListLine {
    pub kind: BlockKind,
    pub content_start: usize,
    pub marker: Option<Range<usize>>,
    pub paragraph: usize,
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
struct Item {
    indent: usize,
    content_indent: usize,
    delimiter: u8,
    ordered: bool,
    ordinal: u64,
    level: u8,
    paragraph: usize,
    after_blank: bool,
}

#[derive(Clone, Debug)]
struct Marker {
    indent: usize,
    marker_start: usize,
    content_start: usize,
    content_indent: usize,
    delimiter: u8,
    ordered: bool,
    ordinal: u64,
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
    let (width, delimiter, ordered, ordinal) = if matches!(body.first(), Some(b'-' | b'+' | b'*')) {
        (1, body[0], false, 1)
    } else if (1..=9).contains(&digits) && matches!(body.get(digits), Some(b'.' | b')')) {
        (
            digits + 1,
            body[digits],
            true,
            text[indent_bytes..indent_bytes + digits].parse().ok()?,
        )
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
        indent,
        marker_start: origin + indent_bytes,
        content_start: origin + content,
        content_indent: column.max(start_column + 1),
        delimiter,
        ordered,
        ordinal,
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
    classify_with_literal_markers(input).0
}

/// Literal list-looking lines remain prose for both ownership and soft breaks.
pub(super) fn classify_with_literal_markers(input: &NormalizedText) -> (Vec<Option<ListLine>>, Vec<bool>) {
    let lines = super::paragraph_flow::source_lines(input);
    let mut result = vec![None; lines.len()];
    let mut literal_markers = vec![false; lines.len()];
    let mut stack: Vec<Item> = Vec::new();
    let mut fence = None;
    let mut fenced_item: Option<Item> = None;
    let mut paragraph = 0;
    let mut ordinary_prose = false;
    for (index, line) in lines.iter().enumerate() {
        let text = &input.text[line.clone()];
        if let Some((delimiter, length)) = fence {
            if let Some(item) = &fenced_item {
                let mut columns = 0;
                let mut bytes = 0;
                for byte in text.bytes().take_while(|byte| matches!(byte, b' ' | b'\t')) {
                    if columns >= item.content_indent {
                        break;
                    }
                    columns += if byte == b'\t' { 4 - columns % 4 } else { 1 };
                    bytes += 1;
                }
                result[index] = Some(ListLine {
                    kind: BlockKind::ListItem {
                        ordered: item.ordered,
                        ordinal: item.ordinal,
                        level: item.level,
                        container_start: false,
                        item_start: false,
                        marker_is_decoration: false,
                    },
                    content_start: line.start + bytes,
                    marker: None,
                    paragraph: item.paragraph,
                    code: true,
                    content_indent: item.content_indent,
                });
            }
            let prefix = result[index].as_ref().map_or(0, |line| line.content_start - lines[index].start);
            if super::markdown_syntax::fence_close(&text[prefix..], delimiter, length) {
                fence = None;
                fenced_item = None;
                if let Some(item) = stack.last_mut() {
                    item.after_blank = true;
                }
            }
            ordinary_prose = false;
            continue;
        }
        let (indent_bytes, indent_columns) = indentation(text);
        let list_fence = stack
            .last()
            .filter(|item| indent_columns >= item.content_indent && indent_columns < item.content_indent + 4)
            .and_then(|item| {
                markdown_fence(&text[indent_bytes..]).map(|open| (item.clone(), open))
            });
        if let Some((mut item, open)) = list_fence {
            paragraph += 1;
            item.paragraph = paragraph;
            result[index] = Some(ListLine {
                kind: BlockKind::ListItem {
                    ordered: item.ordered,
                    ordinal: item.ordinal,
                    level: item.level,
                    container_start: false,
                    item_start: false,
                    marker_is_decoration: false,
                },
                content_start: line.start + indent_bytes,
                marker: None,
                paragraph,
                code: true,
                content_indent: item.content_indent,
            });
            fenced_item = Some(item);
            fence = Some(open);
            ordinary_prose = false;
            continue;
        } else if let Some(open) = markdown_fence(text) {
            fence = Some(open);
            stack.clear();
            ordinary_prose = false;
            continue;
        }
        if text.trim().is_empty() {
            for item in &mut stack {
                item.after_blank = true;
            }
            ordinary_prose = false;
            continue;
        }
        let candidate = marker(text, line.start);
        let interrupts_prose = candidate.as_ref().is_some_and(|marker| {
            ordinary_prose && marker.ordered && marker.ordinal != 1
                && !stack.iter().any(|item| {
                    item.indent <= marker.indent && marker.indent < item.content_indent
                })
        });
        literal_markers[index] = interrupts_prose;
        let marker = candidate.filter(|marker| {
            marker.indent <= stack.last().map_or(3, |item| item.content_indent + 3)
                && !interrupts_prose
        });
        if let Some(marker) = marker {
            while stack.last().is_some_and(|item| marker.indent < item.indent) {
                stack.pop();
            }
            let sibling = stack
                .last()
                .is_some_and(|item| marker.indent < item.content_indent);
            let previous = if sibling { stack.pop() } else { None };
            let compatible = previous.as_ref().is_some_and(|item| {
                item.ordered == marker.ordered && item.delimiter == marker.delimiter
            });
            let ordinal = if compatible && marker.ordered {
                previous.as_ref().unwrap().ordinal.saturating_add(1)
            } else {
                marker.ordinal
            };
            let level = u8::try_from(stack.len()).unwrap_or(u8::MAX);
            let item_fence = markdown_fence(&input.text[marker.content_start..line.end]);
            paragraph += 1;
            result[index] = Some(ListLine {
                kind: BlockKind::ListItem {
                    ordered: marker.ordered,
                    ordinal,
                    level,
                    container_start: !compatible,
                    item_start: true,
                    marker_is_decoration: false,
                },
                content_start: marker.content_start,
                marker: Some(marker.marker_start..marker.content_start),
                paragraph,
                code: item_fence.is_some(),
                content_indent: marker.content_indent,
            });
            stack.push(Item {
                indent: marker.indent,
                content_indent: marker.content_indent,
                delimiter: marker.delimiter,
                ordered: marker.ordered,
                ordinal,
                level,
                paragraph,
                after_blank: false,
            });
            if let Some(open) = item_fence {
                fence = Some(open);
                fenced_item = stack.last().cloned();
            }
            ordinary_prose = item_fence.is_none()
                && !input.text[marker.content_start..line.end].trim().is_empty()
                && matches!(markdown_block_prefix(&input.text, marker.content_start, line.end).1,
                    BlockKind::Paragraph);
            continue;
        }
        let (indent_bytes, columns) = indentation(text);
        while stack.len() > 1
            && stack
                .last()
                .is_some_and(|item| columns < item.content_indent && item.after_blank)
        {
            stack.pop();
        }
        let structural = !interrupts_prose && (!matches!(
            markdown_block_prefix(&input.text, line.start, line.end).1,
            BlockKind::Paragraph
        ) || markdown_fence(text).is_some());
        if let Some(item) = stack
            .last_mut()
            .filter(|item| !structural && (columns >= item.content_indent || !item.after_blank))
        {
            if item.after_blank {
                paragraph += 1;
                item.paragraph = paragraph;
            }
            item.after_blank = false;
            result[index] = Some(ListLine {
                kind: BlockKind::ListItem {
                    ordered: item.ordered,
                    ordinal: item.ordinal,
                    level: item.level,
                    container_start: false,
                    item_start: false,
                    marker_is_decoration: false,
                },
                content_start: line.start + indent_bytes,
                marker: None,
                paragraph: item.paragraph,
                code: false,
                content_indent: item.content_indent,
            });
            ordinary_prose = true;
        } else {
            stack.clear();
            ordinary_prose = !structural && columns < 4;
        }
    }
    (result, literal_markers)
}
