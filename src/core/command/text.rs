use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::document::HardLineSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WordClass {
    White,
    Keyword,
    Punctuation,
}

pub(crate) fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(index, _)| index == offset)
}

pub(crate) fn floor_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index <= offset.min(text.len()))
        .last()
        .unwrap_or(0)
}

pub(crate) fn previous_grapheme_boundary(text: &str, offset: usize) -> Option<usize> {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index < offset.min(text.len() + 1))
        .last()
}

pub(crate) fn next_grapheme_boundary(text: &str, offset: usize) -> Option<usize> {
    text.grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .find(|end| *end > offset)
}

pub(crate) fn grapheme_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    if offset >= text.len() {
        return None;
    }
    let start = floor_grapheme_boundary(text, offset);
    let end = next_grapheme_boundary(text, start)?;
    Some(start..end)
}

pub(crate) fn line_start(lines: &HardLineSnapshot, offset: usize) -> usize {
    lines
        .line_at_offset(offset.min(lines.text_length()))
        .expect("a command offset is a valid UTF-8 hard-line boundary")
        .content_range()
        .start
}

pub(crate) fn line_end(lines: &HardLineSnapshot, offset: usize) -> usize {
    lines
        .line_at_offset(offset.min(lines.text_length()))
        .expect("a command offset is a valid UTF-8 hard-line boundary")
        .content_range()
        .end
}

pub(crate) fn line_range(lines: &HardLineSnapshot, offset: usize) -> Range<usize> {
    lines
        .line_at_offset(offset.min(lines.text_length()))
        .expect("a command offset is a valid UTF-8 hard-line boundary")
        .linewise_range()
}

pub(crate) fn linewise_range(
    lines: &HardLineSnapshot,
    offset: usize,
    count: usize,
) -> Range<usize> {
    let current = lines
        .line_at_offset(offset.min(lines.text_length()))
        .expect("a command offset is a valid UTF-8 hard-line boundary");
    let end = current
        .index()
        .saturating_add(count.max(1))
        .min(lines.line_count());
    lines
        .linewise_extent(current.index()..end)
        .expect("a clamped hard-line span is valid")
}

pub(crate) fn previous_line_start(lines: &HardLineSnapshot, offset: usize) -> Option<usize> {
    let current = lines.line_at_offset(offset.min(lines.text_length())).ok()?;
    lines
        .line(current.index().checked_sub(1)?)
        .map(|line| line.content_range().start)
}

pub(crate) fn next_line_start(lines: &HardLineSnapshot, offset: usize) -> Option<usize> {
    let current = lines.line_at_offset(offset.min(lines.text_length())).ok()?;
    lines
        .line(current.index().checked_add(1)?)
        .map(|line| line.content_range().start)
}

pub(crate) fn line_count(lines: &HardLineSnapshot) -> usize {
    lines.line_count()
}

pub(crate) fn nth_line_start(lines: &HardLineSnapshot, one_based: usize) -> usize {
    let index = one_based
        .max(1)
        .saturating_sub(1)
        .min(lines.line_count().saturating_sub(1));
    lines
        .line(index)
        .expect("every formatted document has at least one hard line")
        .content_range()
        .start
}

pub(crate) fn grapheme_column(text: &str, lines: &HardLineSnapshot, offset: usize) -> usize {
    let start = line_start(lines, offset);
    text[start..offset.min(line_end(lines, offset))]
        .graphemes(true)
        .count()
}

pub(crate) fn position_at_column(
    text: &str,
    lines: &HardLineSnapshot,
    line: usize,
    column: usize,
) -> usize {
    let end = line_end(lines, line);
    let mut result = line;
    for (seen, (relative, _)) in text[line..end].grapheme_indices(true).enumerate() {
        if seen == column {
            return line + relative;
        }
        result = line + relative;
    }
    if line == end {
        line
    } else {
        last_grapheme_start(text, line, end).unwrap_or(result)
    }
}

pub(crate) fn first_non_blank(text: &str, lines: &HardLineSnapshot, offset: usize) -> usize {
    let start = line_start(lines, offset);
    let end = line_end(lines, offset);
    text[start..end]
        .grapheme_indices(true)
        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .map_or(start, |(relative, _)| start + relative)
}

pub(crate) fn last_grapheme_on_line(text: &str, lines: &HardLineSnapshot, offset: usize) -> usize {
    let start = line_start(lines, offset);
    let end = line_end(lines, offset);
    last_grapheme_start(text, start, end).unwrap_or(start)
}

pub(crate) fn last_non_blank(text: &str, lines: &HardLineSnapshot, offset: usize) -> usize {
    let start = line_start(lines, offset);
    let end = line_end(lines, offset);
    text[start..end]
        .grapheme_indices(true)
        .filter(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .last()
        .map_or(start, |(relative, _)| start + relative)
}

fn last_grapheme_start(text: &str, start: usize, end: usize) -> Option<usize> {
    text[start..end]
        .grapheme_indices(true)
        .last()
        .map(|(relative, _)| start + relative)
}

pub(crate) fn normalize_normal_cursor(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
) -> usize {
    // EOF is a boundary in its own right, including the sole boundary of a
    // terminal empty paragraph. Grapheme starts do not contain that boundary.
    let offset = if offset >= text.len() {
        text.len()
    } else {
        floor_grapheme_boundary(text, offset)
    };
    normalize_normal_cursor_snapshot(lines, offset)
}

/// Normalize an already validated logical boundary without materializing the
/// complete formatted text. Revision-bound anchors and model position maps
/// guarantee the input boundary; the persistent hard-line snapshot supplies
/// the only neighboring grapheme lookup Normal mode needs.
pub(crate) fn normalize_normal_cursor_snapshot(lines: &HardLineSnapshot, offset: usize) -> usize {
    let offset = offset.min(lines.text_length());
    let line = lines
        .line_at_offset(offset)
        .expect("a validated formatted boundary resolves to one hard line");
    let range = line.content_range();
    if range.is_empty() {
        return range.start;
    }
    if offset >= range.end {
        return lines
            .previous_grapheme_boundary(range.end)
            .unwrap_or(range.start);
    }
    offset
}

pub(crate) fn move_horizontal(
    lines: &HardLineSnapshot,
    offset: usize,
    amount: isize,
) -> usize {
    // Compatibility callers can supply a stale byte ordinal. Floor once using
    // local tree boundaries, then keep every step inside this exact hard line.
    let mut offset = offset.min(lines.text_length());
    while lines.byte_chunk_at(offset).first().is_some_and(|byte| byte & 0xc0 == 0x80) {
        offset -= 1;
    }
    if !lines.is_grapheme_boundary(offset) {
        offset = lines.previous_grapheme_boundary(offset).unwrap_or(0);
    }
    let mut position = normalize_normal_cursor_snapshot(lines, offset);
    let range = lines.line_at_offset(position).expect("normalized command boundary").content_range();
    if amount > 0 {
        for _ in 0..amount as usize {
            let Some(next) = lines.next_grapheme_boundary(position) else { break; };
            if next >= range.end { break; }
            position = next;
        }
    } else {
        for _ in 0..amount.unsigned_abs() {
            if position <= range.start { break; }
            let Some(previous) = lines.previous_grapheme_boundary(position) else { break; };
            if previous < range.start { break; }
            position = previous;
        }
    }
    position
}

pub(crate) fn move_vertical(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
    amount: isize,
) -> usize {
    let column = grapheme_column(text, lines, offset);
    let mut line = line_start(lines, offset);
    if amount > 0 {
        for _ in 0..amount as usize {
            let Some(next) = next_line_start(lines, line) else {
                break;
            };
            line = next;
        }
    } else {
        for _ in 0..amount.unsigned_abs() {
            let Some(previous) = previous_line_start(lines, line) else {
                break;
            };
            line = previous;
        }
    }
    position_at_column(text, lines, line, column)
}

fn graphemes(text: &str) -> Vec<(usize, usize, WordClass)> {
    text.grapheme_indices(true)
        .map(|(start, grapheme)| {
            let class = classify(grapheme);
            (start, start + grapheme.len(), class)
        })
        .collect()
}

fn classify(grapheme: &str) -> WordClass {
    let Some(character) = grapheme.chars().next() else {
        return WordClass::White;
    };
    if character.is_whitespace() {
        WordClass::White
    } else if character.is_alphanumeric() || character == '_' {
        WordClass::Keyword
    } else {
        WordClass::Punctuation
    }
}

fn class_for(big_word: bool, class: WordClass) -> WordClass {
    if big_word && class != WordClass::White {
        WordClass::Keyword
    } else {
        class
    }
}

pub(crate) fn move_word_forward(text: &str, offset: usize, big_word: bool, count: usize) -> usize {
    let items = graphemes(text);
    if items.is_empty() {
        return 0;
    }
    let mut index = items
        .iter()
        .position(|(start, end, _)| *start <= offset && offset < *end)
        .unwrap_or(items.len());
    for _ in 0..count.max(1).min(items.len().saturating_add(1)) {
        if index >= items.len() {
            return text.len();
        }
        let class = class_for(big_word, items[index].2);
        if class != WordClass::White {
            while index < items.len() && class_for(big_word, items[index].2) == class {
                index += 1;
            }
        }
        while index < items.len() && items[index].2 == WordClass::White {
            index += 1;
        }
    }
    items.get(index).map_or(text.len(), |item| item.0)
}

pub(crate) fn move_word_end(text: &str, offset: usize, big_word: bool, count: usize) -> usize {
    let items = graphemes(text);
    if items.is_empty() {
        return 0;
    }
    let mut index = items
        .iter()
        .position(|(start, end, _)| *start <= offset && offset < *end)
        .unwrap_or(items.len().saturating_sub(1));
    for iteration in 0..count.max(1).min(items.len().saturating_add(1)) {
        if iteration > 0 || class_for(big_word, items[index].2) == WordClass::White {
            if iteration > 0 {
                index = (index + 1).min(items.len() - 1);
            }
            while index < items.len() && items[index].2 == WordClass::White {
                index += 1;
            }
            if index >= items.len() {
                return items.last().map_or(0, |item| item.0);
            }
        }
        let class = class_for(big_word, items[index].2);
        while index + 1 < items.len() && class_for(big_word, items[index + 1].2) == class {
            index += 1;
        }
    }
    items[index].0
}

pub(crate) fn move_word_backward(text: &str, offset: usize, big_word: bool, count: usize) -> usize {
    let items = graphemes(text);
    if items.is_empty() || offset == 0 {
        return 0;
    }
    let mut index = items
        .iter()
        .rposition(|(start, _, _)| *start < offset)
        .unwrap_or(0);
    for iteration in 0..count.max(1).min(items.len().saturating_add(1)) {
        if iteration > 0 && index > 0 {
            index -= 1;
        }
        while index > 0 && items[index].2 == WordClass::White {
            index -= 1;
        }
        let class = class_for(big_word, items[index].2);
        while index > 0 && class_for(big_word, items[index - 1].2) == class {
            index -= 1;
        }
    }
    items[index].0
}

pub(crate) fn move_word_end_backward(
    text: &str,
    offset: usize,
    big_word: bool,
    count: usize,
) -> usize {
    let items = graphemes(text);
    if items.is_empty() || offset == 0 {
        return 0;
    }
    let mut current = items
        .iter()
        .position(|(start, end, _)| *start <= offset && offset < *end)
        .unwrap_or(items.len());
    let mut target = 0;
    for _ in 0..count.max(1).min(items.len().saturating_add(1)) {
        let mut candidate = current.checked_sub(1);

        // When the cursor is inside a word, `ge` first leaves that entire
        // word. On subsequent iterations the previous target is the current
        // word, so this also guarantees that a count makes progress to the
        // next earlier word end instead of rediscovering the same one.
        if current < items.len() {
            let current_class = class_for(big_word, items[current].2);
            if current_class != WordClass::White {
                while candidate
                    .is_some_and(|index| class_for(big_word, items[index].2) == current_class)
                {
                    candidate = candidate.and_then(|index| index.checked_sub(1));
                }
            }
        }

        while candidate.is_some_and(|index| items[index].2 == WordClass::White) {
            candidate = candidate.and_then(|index| index.checked_sub(1));
        }

        let Some(index) = candidate else {
            return 0;
        };
        target = index;
        current = index;
    }
    items[target].0
}

pub(crate) fn find_character(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
    needle: &str,
    forward: bool,
    till: bool,
    count: usize,
) -> Option<usize> {
    find_character_impl(text, lines, offset, needle, forward, till, count)
}

/// Repeat a prior character find. A till-find has its remembered match one
/// grapheme adjacent to the cursor, so repetition must step past that match
/// before applying its count. Initial `t`/`T` commands deliberately use
/// [`find_character`] instead because an adjacent first match is meaningful.
#[allow(dead_code)] // Wired into the interpreter after its current command-state refactor lands.
pub(crate) fn repeat_find_character(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
    needle: &str,
    forward: bool,
    till: bool,
    count: usize,
) -> Option<usize> {
    let target = find_character_impl(text, lines, offset, needle, forward, till, count)?;
    if !till || target != offset {
        return Some(target);
    }

    // A count includes the adjacent occurrence. Only retry when that lookup
    // would leave a till command at the same cursor; blindly incrementing every
    // repeat would make `2;` travel one occurrence too far.
    find_character_impl(
        text,
        lines,
        offset,
        needle,
        forward,
        till,
        count.checked_add(1)?,
    )
}

fn find_character_impl(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
    needle: &str,
    forward: bool,
    till: bool,
    count: usize,
) -> Option<usize> {
    let start = line_start(lines, offset);
    let end = line_end(lines, offset);
    let items: Vec<(usize, &str)> = text[start..end]
        .grapheme_indices(true)
        .map(|(relative, grapheme)| (start + relative, grapheme))
        .collect();
    let current = items
        .iter()
        .position(|(position, _)| *position == offset)
        .unwrap_or(0);
    let found = if forward {
        items
            .iter()
            .enumerate()
            .skip(current + 1)
            .filter(|(_, (_, grapheme))| *grapheme == needle)
            .nth(count.max(1) - 1)
            .map(|(index, _)| index)
    } else {
        items
            .iter()
            .enumerate()
            .take(current)
            .rev()
            .filter(|(_, (_, grapheme))| *grapheme == needle)
            .nth(count.max(1) - 1)
            .map(|(index, _)| index)
    }?;
    let target = if till {
        if forward {
            found.checked_sub(1)?
        } else {
            found + 1
        }
    } else {
        found
    };
    items.get(target).map(|item| item.0)
}

pub(crate) fn move_paragraph(
    lines: &HardLineSnapshot,
    offset: usize,
    forward: bool,
    count: usize,
) -> usize {
    let mut position = offset.min(lines.text_length());
    for _ in 0..count.max(1).min(lines.line_count().saturating_add(1)) {
        let previous_position = position;
        let current = lines
            .line_at_offset(position)
            .expect("a command offset is a valid hard-line boundary");
        if forward {
            let mut index = current.index();
            // Consecutive empty lines form one paragraph boundary in Vim. A
            // motion from nonblank content stops on the first empty line. A
            // motion starting in an empty run crosses the following nonblank
            // paragraph and stops at its next empty boundary.
            if current.content_range().is_empty() {
                while lines
                    .line(index.saturating_add(1))
                    .is_some_and(|line| line.content_range().is_empty())
                {
                    index += 1;
                }
                if lines.line(index.saturating_add(1)).is_some() {
                    index += 1;
                }
            }
            while lines
                .line(index.saturating_add(1))
                .is_some_and(|line| !line.content_range().is_empty())
            {
                index += 1;
            }
            position = if let Some(boundary) = lines.line(index.saturating_add(1)) {
                boundary.content_range().start
            } else {
                // At the terminal paragraph the logical exclusive boundary
                // is EOF. Normal/Visual cursor consumers clamp that boundary
                // to their final legal grapheme.
                lines.text_length()
            };
        } else {
            let mut index = current.index();
            if current.content_range().is_empty() {
                while index
                    .checked_sub(1)
                    .and_then(|at| lines.line(at))
                    .is_some_and(|line| line.content_range().is_empty())
                {
                    index -= 1;
                }
                index = index.saturating_sub(1);
            }
            while index
                .checked_sub(1)
                .and_then(|at| lines.line(at))
                .is_some_and(|line| !line.content_range().is_empty())
            {
                index -= 1;
            }
            position = index
                .checked_sub(1)
                .and_then(|at| lines.line(at))
                .filter(|line| line.content_range().is_empty())
                .or_else(|| lines.line(index))
                .expect("a paragraph motion retains one hard line")
                .content_range()
                .start;
        }
        if position == previous_position {
            break;
        }
    }
    position
}

pub(crate) fn move_sentence(
    text: &str,
    lines: &HardLineSnapshot,
    offset: usize,
    forward: bool,
    count: usize,
) -> usize {
    let items: Vec<(usize, usize, &str)> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| (start, start + grapheme.len(), grapheme))
        .collect();
    if items.is_empty() {
        return 0;
    }
    let mut position = offset.min(text.len());
    for _ in 0..count.max(1).min(items.len().saturating_add(1)) {
        let previous_position = position;
        if forward {
            let mut index = items
                .iter()
                .position(|(start, end, _)| *start <= position && position < *end)
                .unwrap_or(items.len().saturating_sub(1));
            while index < items.len() {
                if is_blank_hard_line_boundary(lines, items[index].0) {
                    index += 1;
                    while index < items.len() && items[index].2.chars().all(char::is_whitespace) {
                        index += 1;
                    }
                    break;
                }
                if let Some(after) = sentence_end_after(&items, index) {
                    index = after;
                    while index < items.len() && items[index].2.chars().all(char::is_whitespace) {
                        index += 1;
                    }
                    break;
                }
                index += 1;
            }
            position = items.get(index).map_or(text.len(), |item| item.0);
        } else {
            let mut starts = vec![0];
            let mut index = 0;
            while index < items.len() {
                let after = if is_blank_hard_line_boundary(lines, items[index].0) {
                    Some(index + 1)
                } else {
                    sentence_end_after(&items, index)
                };
                if let Some(mut next) = after {
                    while next < items.len() && items[next].2.chars().all(char::is_whitespace) {
                        next += 1;
                    }
                    if let Some(item) = items.get(next) {
                        starts.push(item.0);
                    }
                    index = next.max(index + 1);
                } else {
                    index += 1;
                }
            }
            starts.sort_unstable();
            starts.dedup();
            position = starts
                .into_iter()
                .filter(|start| *start < position)
                .last()
                .unwrap_or(0);
        }
        if position == previous_position {
            break;
        }
    }
    position
}

/// Return the grapheme index immediately after a sentence-ending punctuation
/// sequence at `index`. Vim permits closing punctuation between the terminal
/// and the required whitespace/end boundary; checking that boundary also
/// prevents a decimal point in `3.14` from ending a sentence.
fn sentence_end_after(items: &[(usize, usize, &str)], index: usize) -> Option<usize> {
    let terminal = items.get(index)?.2.chars().next()?;
    if !is_sentence_terminator(terminal) {
        return None;
    }
    let mut after = index + 1;
    while let Some(grapheme) = items.get(after).map(|item| item.2) {
        let Some(closer) = grapheme.chars().next() else {
            break;
        };
        if !is_sentence_closer(closer) {
            break;
        }
        after += 1;
    }
    if after == items.len()
        || items[after].2.chars().all(char::is_whitespace)
        || is_east_asian_sentence_terminator(terminal)
    {
        Some(after)
    } else {
        None
    }
}

pub(super) fn is_sentence_terminator(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '…' | '。' | '！' | '？')
}

pub(super) fn is_sentence_closer(ch: char) -> bool {
    matches!(ch, '"' | '\'' | ')' | ']' | '}' | '»' | '”' | '’')
}

pub(super) fn is_east_asian_sentence_terminator(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？')
}

fn is_blank_hard_line_boundary(lines: &HardLineSnapshot, offset: usize) -> bool {
    let Ok(current) = lines.line_at_offset(offset) else {
        return false;
    };
    if current.separator_range().as_ref().map(|range| range.start) != Some(offset) {
        return false;
    }
    lines
        .line(current.index().saturating_add(1))
        .is_some_and(|next| next.content_range().is_empty() && next.separator_range().is_some())
}

pub(crate) fn advance_graphemes(text: &str, offset: usize, count: usize) -> usize {
    let mut result = offset.min(text.len());
    for _ in 0..count {
        let Some(next) = next_grapheme_boundary(text, result) else {
            break;
        };
        result = next;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn move_horizontal(text: &str, offset: usize, amount: isize) -> usize {
        let document = Document::new(text);
        super::move_horizontal(&document.hard_line_snapshot(), offset, amount)
    }

    fn move_vertical(text: &str, offset: usize, amount: isize) -> usize {
        let document = Document::new(text);
        super::move_vertical(text, &document.hard_line_snapshot(), offset, amount)
    }

    #[test]
    fn horizontal_motion_never_splits_a_grapheme() {
        let text = "a\u{301}bc";
        assert_eq!(move_horizontal(text, 0, 1), "a\u{301}".len());
        assert!(is_grapheme_boundary(text, move_horizontal(text, 0, 1)));
    }

    #[test]
    fn counted_horizontal_motion_scales_on_large_lines_and_clamps_without_flattening() {
        let text = format!("{}\nnext", "a\u{301}😀".repeat(50_000));
        let document = Document::new(text.clone());
        let lines = document.hard_line_snapshot();
        let last = 350_000 - "😀".len();
        assert_eq!(super::move_horizontal(&lines, 0, isize::MAX), last);
        assert_eq!(super::move_horizontal(&lines, last, isize::MIN), 0);
        assert_eq!(super::move_horizontal(&lines, 350_001, isize::MIN), 350_001);
        assert_eq!(super::move_horizontal(&lines, 0, 50_000), 175_000);
        assert_eq!(super::move_horizontal(&lines, 175_000, -50_000), 0);
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn horizontal_motion_floors_stale_bytes_and_preserves_empty_line_boundaries() {
        let document = Document::new("a\u{301}😀b\n\nend");
        let lines = document.hard_line_snapshot();
        assert_eq!(super::move_horizontal(&lines, 1, 0), 0);
        assert_eq!(super::move_horizontal(&lines, 5, 0), 3);
        assert_eq!(super::move_horizontal(&lines, 5, 1), 7);
        assert_eq!(super::move_horizontal(&lines, 9, isize::MAX), 9);
        assert_eq!(super::move_horizontal(&lines, 9, isize::MIN), 9);
        assert_eq!(super::move_horizontal(&lines, usize::MAX, 0), 12);
    }

    #[test]
    fn vertical_motion_keeps_grapheme_column() {
        let text = "aé日\nxy\n1234";
        assert_eq!(move_vertical(text, "aé".len(), 1), 8);
        assert_eq!(move_vertical(text, "aé".len(), 2), 12);
    }

    #[test]
    fn forced_mac_snapshot_keeps_literal_lf_in_horizontal_line_content() {
        let document = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let lines = document.hard_line_snapshot();
        assert_eq!(super::move_vertical(document.text(), &lines, 0, 1), 2);
        assert_eq!(super::move_horizontal(&lines, 2, 1), 3);
        assert_eq!(super::move_horizontal(&lines, 3, 1), 4);
        assert_eq!(super::last_grapheme_on_line(document.text(), &lines, 2), 4);
    }

    #[test]
    fn word_classes_distinguish_word_and_word() {
        let text = "one.two three";
        assert_eq!(move_word_forward(text, 0, false, 1), 3);
        assert_eq!(move_word_forward(text, 0, true, 1), 8);
    }

    #[test]
    fn backward_word_end_counts_reach_distinct_unicode_word_ends() {
        let text = "a\u{301}b++ γδ かな";
        let kana = text.find('か').unwrap();
        let delta = text.find('δ').unwrap();
        let last_plus = text.rfind('+').unwrap();
        let b = text.find('b').unwrap();

        assert_eq!(move_word_end_backward(text, kana, false, 1), delta);
        assert_eq!(move_word_end_backward(text, kana, false, 2), last_plus);
        assert_eq!(move_word_end_backward(text, kana, false, 3), b);
        assert_eq!(move_word_end_backward(text, kana, true, 1), delta);
        assert_eq!(move_word_end_backward(text, kana, true, 2), last_plus);
        assert_eq!(move_word_end_backward(text, kana, true, 3), 0);

        let inside_kana = kana + 'か'.len_utf8();
        let target = move_word_end_backward(text, inside_kana, false, 1);
        assert_eq!(target, delta, "ge must first leave the current word");
        assert!(is_grapheme_boundary(text, target));
    }

    #[test]
    fn repeated_unicode_till_skips_the_adjacent_remembered_match() {
        let needle = "e\u{301}";
        let text = format!("{needle}a{needle}c{needle}d{needle}");
        let document = Document::new(&text);
        let lines = document.hard_line_snapshot();
        let a = text.find('a').unwrap();
        let c = text.find('c').unwrap();
        let d = text.find('d').unwrap();

        assert_eq!(
            find_character(&text, &lines, a, needle, true, true, 1),
            Some(a),
            "an initial t accepts an adjacent match"
        );
        assert_eq!(
            repeat_find_character(&text, &lines, a, needle, true, true, 1),
            Some(c)
        );
        assert_eq!(
            repeat_find_character(&text, &lines, a, needle, true, true, 2),
            Some(c),
            "the adjacent occurrence is included in a repeat count"
        );
        assert_eq!(
            repeat_find_character(&text, &lines, a, needle, true, true, 3),
            Some(d)
        );
        assert_eq!(
            repeat_find_character(&text, &lines, c, needle, false, true, 1),
            Some(a),
            "reversing a till repeat must not stop on the adjacent match"
        );

        assert_eq!(
            find_character(&text, &lines, d, needle, false, true, 1),
            Some(d),
            "an initial T accepts an adjacent match"
        );
        assert_eq!(
            repeat_find_character(&text, &lines, d, needle, false, true, 1),
            Some(c)
        );
        assert_eq!(
            repeat_find_character(&text, &lines, d, needle, false, true, 2),
            Some(c),
            "the adjacent occurrence is included in a reverse repeat count"
        );
        assert_eq!(
            repeat_find_character(&text, &lines, d, needle, false, true, 3),
            Some(a)
        );
    }

    #[test]
    fn sentence_motion_observes_closers_and_does_not_split_decimals() {
        let text = "It is 3.14. \"Really?\" Yes!";
        let document = Document::new(text);
        let lines = document.hard_line_snapshot();
        let really = text.find("Really").unwrap();
        let yes = text.find("Yes").unwrap();

        assert_eq!(super::move_sentence(text, &lines, 0, true, 1), really - 1);
        assert_eq!(super::move_sentence(text, &lines, really - 1, true, 1), yes);
        assert_eq!(
            super::move_sentence(text, &lines, yes, false, 1),
            really - 1
        );
    }

    #[test]
    fn sentence_motion_supports_unspaced_east_asian_terminators() {
        let text = "甲。乙！丙？丁";
        let document = Document::new(text);
        let lines = document.hard_line_snapshot();
        let second = text.find('乙').unwrap();
        let third = text.find('丙').unwrap();
        let fourth = text.find('丁').unwrap();

        assert_eq!(super::move_sentence(text, &lines, 0, true, 1), second);
        assert_eq!(super::move_sentence(text, &lines, second, true, 1), third);
        assert_eq!(super::move_sentence(text, &lines, third, true, 1), fourth);
    }

    #[test]
    fn terminal_forward_paragraph_motion_exposes_exclusive_eof() {
        let text = "aaa\nbbb\nccc";
        let document = Document::new(text);
        let lines = document.hard_line_snapshot();

        assert_eq!(super::move_paragraph(&lines, 0, true, 1), text.len());
        assert_eq!(
            super::move_paragraph(&lines, text.find("bbb").unwrap(), true, 1),
            text.len()
        );
    }

    #[test]
    fn paragraph_motion_stops_on_blank_runs_with_vim_count_semantics() {
        let text = "A\n\n\nB\nC\n\nD";
        let document = Document::new(text);
        let lines = document.hard_line_snapshot();
        let first_blank = text.find("\n\n").unwrap() + 1;
        let second_boundary = text.rfind("\n\n").unwrap() + 1;
        let b = text.find('B').unwrap();
        let d = text.find('D').unwrap();

        assert_eq!(super::move_paragraph(&lines, 0, true, 1), first_blank);
        assert_eq!(super::move_paragraph(&lines, 0, true, 2), second_boundary);
        assert_eq!(
            super::move_paragraph(&lines, first_blank, true, 1),
            second_boundary
        );
        assert_eq!(super::move_paragraph(&lines, b, false, 1), first_blank + 1);
        assert_eq!(super::move_paragraph(&lines, d, false, 1), second_boundary);
        assert_eq!(super::move_paragraph(&lines, b, false, 2), 0);
    }
}
