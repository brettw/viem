//! Format-owned classification of physical source breaks. The editable
//! Markdown projection normalizes prose whitespace; source views retain every
//! character and use the same classification only for optional layout flow.
use super::line_endings::{LineEnding, LogicalUnit, NormalizedText};
use super::projection::{markdown_block_prefix, markdown_fence, BlockKind};
use std::collections::BTreeSet;
use std::ops::Range;

pub(super) fn source_lines(input: &NormalizedText) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut lines = Vec::new();
    for ending in &input.endings {
        lines.push(start..ending.normalized.start);
        start = ending.normalized.end;
    }
    lines.push(start..input.text.len());
    lines
}

pub(super) fn markdown_soft_breaks(input: &NormalizedText) -> BTreeSet<usize> {
    let quotes = super::markdown_quotes::classify(input);
    if quotes.iter().all(|line| line.depth == 0) {
        return markdown_soft_breaks_without_quotes(input);
    }
    let body = super::markdown_quotes::strip(input, &quotes);
    let soft = markdown_soft_breaks_without_quotes(&body);
    input.endings.iter().enumerate().filter_map(|(index, ending)| {
        (quotes[index].depth == quotes[index + 1].depth
            && soft.contains(&body.endings[index].normalized.start))
            .then_some(ending.normalized.start)
    }).collect()
}

fn markdown_soft_breaks_without_quotes(input: &NormalizedText) -> BTreeSet<usize> {
    let lines = source_lines(input);
    let lists = super::markdown_blocks::classify(input);
    let indented = super::markdown_indented_code::classify(input);
    let syntax = super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(input));
    let mut prose = Vec::with_capacity(lines.len());
    let mut fence = None;
    for line in &lines {
        let text = &input.text[line.clone()];
        let was_fenced = fence.is_some();
        if let Some((delimiter, length)) = fence {
            if super::markdown_syntax::fence_close(text, delimiter, length) {
                fence = None;
            }
        } else {
            fence = markdown_fence(text).or_else(|| {
                super::markdown_blocks::marker_prefix_length(text)
                    .and_then(|prefix| markdown_fence(&text[prefix..]))
            });
        }
        let (_, kind) = markdown_block_prefix(&input.text, line.start, line.end);
        prose.push(
            !was_fenced
                && fence.is_none()
                && !text.trim().is_empty()
                && kind == BlockKind::Paragraph,
        );
    }
    input
        .endings
        .iter()
        .enumerate()
        .filter_map(|(i, ending)| {
            let previous = &input.text[lines[i].clone()];
            let literal = super::markdown_indented_code::containing(&indented, ending.source.start).is_some();
            let table_boundary = syntax.tables.get(syntax.tables.partition_point(|table| table.range.end <= ending.normalized.start)).is_some_and(|table| table.range.start <= ending.normalized.end);
            let block_boundary = table_boundary || syntax.blocks.get(syntax.blocks.partition_point(|block| block.range.end <= ending.normalized.start)).is_some_and(|block| {
                let touches = block.range.start <= ending.normalized.end && ending.normalized.start < block.range.end;
                touches && !(matches!(block.role, super::markdown_syntax::BlockRole::Heading(_))
                    && block.content.start <= ending.normalized.start && ending.normalized.end < block.content.end)
            }) || syntax.definitions.get(syntax.definitions.partition_point(|range| range.end <= ending.normalized.start)).is_some_and(|range| range.start <= ending.normalized.end);
            (!literal && !block_boundary && ((prose[i]
                && prose.get(i + 1) == Some(&true)
                && lists[i].is_none()
                && lists.get(i + 1).is_some_and(Option::is_none)
                || lists[i]
                    .as_ref()
                    .zip(lists.get(i + 1).and_then(Option::as_ref))
                    .is_some_and(|(a, b)| a.paragraph == b.paragraph && !a.code && !b.code))
                && !previous.ends_with("  ")
                && !previous.ends_with('\\')))
            .then_some(ending.normalized.start)
        })
        .collect()
}

pub(super) fn flow_ranges(input: &NormalizedText, soft: &BTreeSet<usize>) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut ranges = Vec::new();
    for ending in &input.endings {
        if !soft.contains(&ending.normalized.start) {
            ranges.push(start..ending.normalized.start);
            start = ending.normalized.end;
        }
    }
    ranges.push(start..input.text.len());
    ranges
}

/// A cooked Unicode stream still maps each output unit to exact physical
/// bytes. Each canonical pair of source endings is one mapped paragraph
/// newline; repeated pairs retain authored empty paragraphs. Deleting a
/// semantic boundary owns its complete physical separator extent.
pub(super) fn markdown(input: &NormalizedText) -> (NormalizedText, BTreeSet<usize>) {
    markdown_projection(input, false)
}

/// Source syntax remains visible, but blank paragraph-separator lines share
/// the same mapped boundary as WYSIWYG. Ordinary source endings still belong
/// to the optional per-view flow policy.
pub(super) fn markdown_source(input: &NormalizedText) -> (NormalizedText, BTreeSet<usize>) {
    markdown_projection(input, true)
}

fn markdown_projection(
    input: &NormalizedText,
    preserve_markers: bool,
) -> (NormalizedText, BTreeSet<usize>) {
    let soft = markdown_soft_breaks(input);
    let quotes = super::markdown_quotes::classify(input);
    if !preserve_markers && quotes.iter().any(|line| line.depth > 0) {
        let body = super::markdown_quotes::strip(input, &quotes);
        let body_soft = input.endings.iter().zip(&body.endings)
            .filter_map(|(before, after)| soft.contains(&before.normalized.start)
                .then_some(after.normalized.start)).collect();
        return markdown_projection_with_soft_breaks(&body, false, body_soft);
    }
    markdown_projection_with_soft_breaks(input, preserve_markers, soft)
}

fn markdown_projection_with_soft_breaks(
    input: &NormalizedText,
    preserve_markers: bool,
    soft: BTreeSet<usize>,
) -> (NormalizedText, BTreeSet<usize>) {
    let lines = source_lines(input);
    let lists = super::markdown_blocks::classify(input);
    let indented = super::markdown_indented_code::classify(input);
    let quotes = super::markdown_quotes::classify(input);
    let mut replacements: Vec<(Range<usize>, &'static str, bool)> = Vec::new();
    let mut explicit = BTreeSet::new();
    let mut fence = None;
    let mut i = 0;
    while i < input.endings.len() {
        let text = &input.text[quotes[i].content_start..lines[i].end];
        if let Some((delimiter, length)) = fence {
            if super::markdown_syntax::fence_close(text, delimiter, length) {
                fence = None;
            }
        } else {
            fence = markdown_fence(text).or_else(|| {
                super::markdown_blocks::marker_prefix_length(text)
                    .and_then(|prefix| markdown_fence(&text[prefix..]))
            });
        }
        let ending = &input.endings[i];
        if fence.is_some()
            || lists[i]
                .as_ref()
                .zip(lists.get(i + 1).and_then(Option::as_ref))
                .is_some_and(|(line, next)| {
                    line.code && next.code && line.paragraph == next.paragraph
                })
            || super::markdown_indented_code::containing(&indented, ending.source.start).is_some_and(|block| ending.source.start < block.source.end)
        {
            // Code-body endings remain literal content; closing-fence
            // separators below still obey ordinary paragraph separation.
            i += 1;
            continue;
        }
        if soft.contains(&ending.normalized.start) {
            if preserve_markers {
                i += 1;
                continue;
            }
            let end = lists
                .get(i + 1)
                .and_then(Option::as_ref)
                .filter(|line| line.marker.is_none())
                .map_or(ending.normalized.end, |line| line.content_start);
            let end = input.text[end..].char_indices().take_while(|(_, ch)| matches!(ch, ' ' | '\t'))
                .last().map_or(end, |(offset, ch)| end + offset + ch.len_utf8());
            let start = input.text[..ending.normalized.start].trim_end_matches([' ', '\t']).len();
            replacements.push((start..end, " ", false));
        } else if lines
            .get(i + 1)
            .is_some_and(|line| input.text[line.clone()].trim().is_empty())
        {
            let mut last = i;
            while last + 1 < input.endings.len()
                && input.text[lines[last + 1].clone()].trim().is_empty()
            {
                last += 1;
            }
            if last > i {
                let count = last - i + 1;
                let pairs = count / 2;
                for pair in 0..pairs {
                    let first = i + pair * 2;
                    let end = if pair + 1 == pairs { last } else { first + 1 };
                    replacements.push((
                        input.endings[first].normalized.start..input.endings[end].normalized.end,
                        "\n",
                        true,
                    ));
                }
                i = last;
            } else if preserve_markers {
                // An ordinary terminal source ending remains editable.
                if text.ends_with("  ") || text.ends_with('\\') {
                    explicit.insert(ending.source.end);
                }
            } else if !text.ends_with("  ") && !text.ends_with('\\') {
                replacements.push((ending.normalized.clone(), "", false));
            } else {
                let count = if text.ends_with('\\') {
                    1
                } else {
                    text.len() - text.trim_end_matches(' ').len()
                };
                replacements.push((
                    ending.normalized.start - count..ending.normalized.end,
                    "\n",
                    true,
                ));
                explicit.insert(ending.source.end);
            }
        } else if text.ends_with("  ") || text.ends_with('\\') {
            if !preserve_markers {
                let count = if text.ends_with('\\') {
                    1
                } else {
                    text.len() - text.trim_end_matches(' ').len()
                };
                replacements.push((
                    ending.normalized.start - count..ending.normalized.end,
                    "\n",
                    true,
                ));
            }
            explicit.insert(ending.source.end);
        }
        i += 1;
    }
    for (index, context) in lists.iter().enumerate() {
        if let Some(context) = context
            .as_ref()
            .filter(|context| !preserve_markers && context.marker.is_none()
                && !input.units.get(input.units.partition_point(|unit| unit.normalized.start < lines[index].start))
                    .is_some_and(|unit| super::markdown_indented_code::containing(&indented, unit.source.start).is_some()))
        {
            if context.content_start > lines[index].start
                && (index == 0 || !soft.contains(&input.endings[index - 1].normalized.start))
            {
                replacements.push((lines[index].start..context.content_start, "", false));
            }
        }
    }
    replacements.sort_by_key(|(range, _, _)| range.start);
    let mut result = NormalizedText {
        text: String::new(),
        units: Vec::new(),
        endings: Vec::new(),
        encoding: input.encoding,
    };
    let mut replacement_index = 0;
    let mut unit_index = 0;
    while unit_index < input.units.len() {
        let unit = &input.units[unit_index];
        if let Some((range, value, hard)) = replacements
            .get(replacement_index)
            .filter(|(range, _, _)| range.start == unit.normalized.start)
        {
            let mut last = unit_index;
            while last + 1 < input.units.len() && input.units[last + 1].normalized.start < range.end
            {
                last += 1;
            }
            let source = unit.source.start..input.units[last].source.end;
            let normalized = result.text.len()..result.text.len() + value.len();
            result.text.push_str(value);
            if !value.is_empty() {
                result.units.push(LogicalUnit {
                    normalized: normalized.clone(),
                    source: source.clone(),
                    decoding_diagnostic: None,
                });
            }
            if *hard {
                result.endings.push(LineEnding {
                    normalized,
                    source,
                    original: input
                        .endings
                        .get(
                            input
                                .endings
                                .partition_point(|ending| ending.normalized.end < range.end),
                        )
                        .map_or(super::FileFormat::Unix, |ending| ending.original),
                });
            }
            unit_index = last + 1;
            replacement_index += 1;
        } else {
            let value = &input.text[unit.normalized.clone()];
            let normalized = result.text.len()..result.text.len() + value.len();
            result.text.push_str(value);
            let mut next = unit.clone();
            next.normalized = normalized.clone();
            result.units.push(next);
            let at = input
                .endings
                .partition_point(|ending| ending.normalized.start < unit.normalized.start);
            if let Some(ending) = input
                .endings
                .get(at)
                .filter(|ending| ending.normalized == unit.normalized)
            {
                let mut ending = ending.clone();
                ending.normalized = normalized;
                result.endings.push(ending);
            }
            unit_index += 1;
        }
    }
    (result, explicit)
}
