//! Format-owned classification of physical source breaks. The editable
//! Markdown projection normalizes prose whitespace; source views retain every
//! character and use the same classification only for optional layout flow.
use super::line_endings::{LineEnding, LogicalUnit, NormalizedText};
use std::collections::BTreeSet;
use std::ops::Range;

fn source_at(input: &NormalizedText, at: usize) -> usize {
    input.units.get(input.units.partition_point(|unit| unit.normalized.start < at))
        .map_or_else(|| input.units.last().map_or(0, |unit| unit.source.end), |unit| unit.source.start)
}

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
    let syntax = super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(input)).to_source(input);
    markdown_soft_breaks_with_syntax(input, &syntax)
}

/// Only grammar-owned prose breaks may fold. Looking at the surrounding
/// physical lines invents breaks in code, HTML and empty container prefixes.
fn markdown_soft_breaks_with_syntax(input: &NormalizedText, syntax: &super::markdown_syntax::Blocks) -> BTreeSet<usize> {
    input.endings.iter().filter_map(|ending| {
        let soft = syntax.soft_breaks.get(syntax.soft_breaks.partition_point(|range| range.end <= ending.source.start))
            .is_some_and(|range| range.start <= ending.source.start && ending.source.end <= range.end);
        // Reference definitions stay visible, but their physical continuation
        // lines belong to the same deliberately literal paragraph.
        let definition = syntax.definitions.get(syntax.definitions.partition_point(|range| range.end <= ending.source.start))
            .is_some_and(|range| range.start <= ending.source.start && ending.source.end < range.end);
        (soft || definition).then_some(ending.normalized.start)
    }).collect()
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
    let syntax = super::markdown_syntax::Blocks::parse(&super::markdown_syntax::grammar_text(input)).to_source(input);
    let soft = markdown_soft_breaks_with_syntax(input, &syntax);
    let quotes = super::markdown_quotes::classify(input);
    if !preserve_markers && quotes.iter().any(|line| line.depth > 0) {
        let body = super::markdown_quotes::strip(input, &quotes);
        let body_soft = input.endings.iter().zip(&body.endings)
            .filter_map(|(before, after)| soft.contains(&before.normalized.start)
                .then_some(after.normalized.start)).collect();
        return markdown_projection_with_soft_breaks(&body, false, body_soft, &syntax);
    }
    markdown_projection_with_soft_breaks(input, preserve_markers, soft, &syntax)
}

fn markdown_projection_with_soft_breaks(
    input: &NormalizedText,
    preserve_markers: bool,
    soft: BTreeSet<usize>,
    syntax: &super::markdown_syntax::Blocks,
) -> (NormalizedText, BTreeSet<usize>) {
    let lines = source_lines(input);
    let lists = super::markdown_blocks::classify_with_syntax(input, syntax);
    let quotes = super::markdown_quotes::classify(input);
    let mut replacements: Vec<(Range<usize>, &'static str, bool)> = Vec::new();
    let mut explicit = BTreeSet::new();
    let mut i = 0;
    while i < input.endings.len() {
        let text = &input.text[quotes[i].content_start..lines[i].end];
        let ending = &input.endings[i];
        let html = syntax.blocks.get(syntax.blocks.partition_point(|block| block.range.end <= ending.source.start))
            .is_some_and(|block| {
                if !matches!(block.role, super::markdown_syntax::BlockRole::Html) || block.range.start > ending.source.start { return false; }
                if ending.source.end < block.range.end { return true; }
                let start = input.units.get(input.units.partition_point(|unit| unit.source.start < block.range.start))
                    .map_or(ending.normalized.start, |unit| unit.normalized.start);
                super::projection::markdown_html_open_pre(&input.text[start..ending.normalized.start])
            });
        if html {
            // Markdown whitespace rules do not run inside passive HTML blocks.
            i += 1;
            continue;
        }
        let inline_code = syntax.inline_code.get(syntax.inline_code.partition_point(|range| range.end <= ending.source.start))
            .is_some_and(|range| range.start <= ending.source.start && ending.source.end <= range.end);
        if inline_code {
            if !preserve_markers { replacements.push((ending.normalized.clone(), " ", false)); }
            i += 1;
            continue;
        }
        let hard = syntax.hard_breaks.get(syntax.hard_breaks.partition_point(|range| range.end <= ending.source.start))
            .is_some_and(|range| range.start <= ending.source.start && ending.source.end <= range.end);
        let code = syntax.code.get(syntax.code.partition_point(|code| code.range.end <= ending.source.start));
        if code.is_some_and(|code| code.range.start <= ending.source.start && ending.source.start < code.body_end) {
            // Literal endings follow parser-owned code scopes. Removing quote
            // prefixes must not let a fence consume a different container.
            i += 1;
            continue;
        }
        if !preserve_markers && lists[i].as_ref().is_some_and(|line|
            line.marker.is_some() && input.text[line.content_start..lines[i].end].trim().is_empty()
                && lists.get(i + 1).and_then(Option::as_ref).is_some_and(|next|
                    !next.code && matches!((&line.kind, &next.kind),
                        (super::BlockKind::ListItem { ordered: a, ordinal: b, level: c, .. },
                         super::BlockKind::ListItem { ordered: d, ordinal: e, level: f, item_start: false, .. })
                            if (a, b, c) == (d, e, f)))) {
            // An empty marker is structure before its first body, not prose
            // that contributes a leading folded space or an empty paragraph.
            replacements.push((ending.normalized.clone(), "", false));
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
                if hard {
                    explicit.insert(ending.source.end);
                }
            } else if !hard {
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
        } else if hard {
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
        let start = source_at(input, lines[index].start);
        let end = source_at(input, lines[index].end);
        // Parser ranges may begin after the container's indentation. Match
        // the whole row so that extra literal code spaces are never stripped.
        let indented = syntax.code.get(syntax.code.partition_point(|code| code.range.end <= start))
            .is_some_and(|code| !code.fenced && code.range.start <= end && start < code.range.end);
        let definition = syntax.definitions.get(syntax.definitions.partition_point(|range| range.end <= start))
            .is_some_and(|range| range.start <= end && start < range.end);
        if let Some(context) = context
            .as_ref()
            .filter(|context| !preserve_markers && context.marker.is_none() && !indented && !definition)
        {
            if context.content_start > lines[index].start
                && (index == 0 || !soft.contains(&input.endings[index - 1].normalized.start))
            {
                replacements.push((lines[index].start..context.content_start, "", false));
            }
        }
    }
    if !preserve_markers {
        for (index, line) in lines.iter().enumerate() {
            let start = source_at(input, line.start);
            let end = source_at(input, line.end);
            let definition = syntax.definitions.get(syntax.definitions.partition_point(|range| range.end <= start))
                .is_some_and(|range| range.start <= end && start < range.end);
            if !definition { continue; }
            let text = &input.text[line.clone()];
            let leading = text.len() - text.trim_start_matches([' ', '\t']).len();
            if leading > 0 && (index == 0 || !soft.contains(&input.endings[index - 1].normalized.start)) {
                replacements.push((line.start..line.start + leading, "", false));
            }
            let trailing = text.trim_end_matches([' ', '\t']).len();
            if trailing < text.len() && input.endings.get(index).is_none_or(|ending| !soft.contains(&ending.normalized.start)) {
                replacements.push((line.start + trailing..line.end, "", false));
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
