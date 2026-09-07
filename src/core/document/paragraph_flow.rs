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
    let lines = source_lines(input);
    let mut prose = Vec::with_capacity(lines.len());
    let mut fence = None;
    for line in &lines {
        let text = &input.text[line.clone()];
        let was_fenced = fence.is_some();
        if let Some((delimiter, length)) = fence {
            let tail = text.trim();
            if tail.len() >= length && tail.bytes().all(|byte| byte == delimiter) {
                fence = None;
            }
        } else {
            fence = markdown_fence(text);
        }
        let (_, kind) = markdown_block_prefix(&input.text, line.start, line.end);
        prose.push(
            !was_fenced
                && fence.is_none()
                && !text.trim().is_empty()
                && !text.starts_with("    ")
                && !text.starts_with('\t')
                && kind == BlockKind::Paragraph,
        );
    }
    input
        .endings
        .iter()
        .enumerate()
        .filter_map(|(i, ending)| {
            let previous = &input.text[lines[i].clone()];
            (prose[i]
                && prose.get(i + 1) == Some(&true)
                && !previous.ends_with("  ")
                && !previous.ends_with('\\'))
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
    let lines = source_lines(input);
    let soft = markdown_soft_breaks(input);
    let mut replacements: Vec<(Range<usize>, &'static str, bool)> = Vec::new();
    let mut explicit = BTreeSet::new();
    let mut fence = None;
    let mut i = 0;
    while i < input.endings.len() {
        let text = &input.text[lines[i].clone()];
        let was_fenced = fence.is_some();
        if let Some((delimiter, length)) = fence {
            let tail = text.trim();
            if tail.len() >= length && tail.bytes().all(|byte| byte == delimiter) {
                fence = None;
            }
        } else {
            fence = markdown_fence(text);
        }
        let ending = &input.endings[i];
        if was_fenced || fence.is_some() || text.starts_with("    ") || text.starts_with('\t') {
            // A closing fence's terminal source ending is syntax, while line
            // endings inside the code body remain explicit content.
            if was_fenced && fence.is_none() && ending.normalized.end == input.text.len() {
                replacements.push((ending.normalized.clone(), "", false));
            }
            i += 1;
            continue;
        }
        if soft.contains(&ending.normalized.start) {
            replacements.push((ending.normalized.clone(), " ", false));
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
        i += 1;
    }
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
