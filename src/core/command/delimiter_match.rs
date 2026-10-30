//! Vim 9.2's `findmatchlimit()` for paired delimiters, shared by `%`, `[(`, `[{`,
//! the method motions and the block text objects. Unless 'cpoptions' contains
//! `%`, a candidate counts only while an even number of unescaped double
//! quotes separate it from the start, and `'x'` or `'\x'` character literals
//! are skipped. Quotes toggle only in lines with an even quote count, and a
//! line ending in a backslash continues its quote state. Unless 'cpoptions'
//! contains `M`, a candidate must be escaped by an odd number of backslashes
//! exactly when the search requires it. Positions are logical graphemes,
//! compared by their first scalar; quote counts read only the lines that need
//! them, through borrowed leaf chunks.
use crate::document::{HardLineInfo, HardLineSnapshot};
use std::ops::Range;

/// The 'cpoptions' flags that change delimiter matching.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MatchRules {
    /// `%`: Vi-compatible matching without quote or literal heuristics.
    pub vi_compatible: bool,
    /// `M`: backslashes before delimiters are ignored.
    pub ignore_backslashes: bool,
}

impl MatchRules {
    pub(super) fn from_cpoptions(cpoptions: &str) -> Self {
        Self { vi_compatible: cpoptions.contains('%'), ignore_backslashes: cpoptions.contains('M') }
    }
}

/// One search from a start position, which is never itself a candidate.
#[derive(Clone, Copy, Debug)]
pub(super) struct DelimiterSearch {
    /// Ends the search at depth zero.
    pub target: char,
    /// Deepens the search.
    pub nested: char,
    pub forward: bool,
    /// Whether candidates must be escaped; `%` copies its start's escaping.
    pub escaped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartInQuotes {
    Maybe,
    Yes,
    No,
}

pub(super) fn find_delimiter(
    lines: &HardLineSnapshot,
    origin: usize,
    search: DelimiterSearch,
    rules: MatchRules,
) -> Option<usize> {
    let mut line = lines.line_at_offset(origin.min(lines.text_length())).ok()?;
    let mut range = line.content_range();
    let mut at = origin.min(range.end);
    let mut depth = 0usize;
    let mut inquote = false;
    let mut start_in_quotes = StartInQuotes::Maybe;
    // Whether quotes toggle in the current line: None until the first quote
    // needs it, so a line without quotes or continuations is never counted.
    let mut toggles: Option<bool> = None;
    let mut entered = true;
    loop {
        if search.forward {
            if at >= range.end {
                line = lines.line(line.index() + 1)?;
                range = line.content_range();
                at = range.start;
                entered = true;
            } else {
                at = lines.next_grapheme_boundary(at)?.min(range.end);
            }
        } else if at <= range.start {
            line = lines.line(line.index().checked_sub(1)?)?;
            range = line.content_range();
            at = range.end;
            entered = true;
        } else {
            at = lines.previous_grapheme_boundary(at)?.max(range.start);
        }
        if entered {
            entered = false;
            toggles = None;
            if rules.vi_compatible {
                toggles = Some(false);
            } else {
                let ends_continued = last_byte(lines, &range) == Some(b'\\');
                let follows_continued = line.index() > 0
                    && lines.line(line.index() - 1).is_some_and(|previous| last_byte(lines, &previous.content_range()) == Some(b'\\'));
                if ends_continued || follows_continued {
                    // Vim's position for the start's quote parity: the step
                    // itself forward, one byte after it backward.
                    let probe = (start_in_quotes == StartInQuotes::Maybe)
                        .then(|| if search.forward { at } else { at + 1 });
                    let (even, at_start) = count_quotes(lines, &range, probe);
                    toggles = Some(even);
                    if !even {
                        inquote = false;
                        // An unscanned start position counts as inside quotes.
                        let at_start = at_start.unwrap_or(true);
                        if ends_continued {
                            toggles = Some(true);
                            // Vim 9.2 assumes a start in a continued line is
                            // quoted; later Vim uses `at_start` here too.
                            if start_in_quotes == StartInQuotes::Maybe {
                                inquote = true;
                                start_in_quotes = StartInQuotes::Yes;
                            } else if !search.forward {
                                inquote = true;
                            }
                        }
                        if follows_continued {
                            toggles = Some(true);
                            if start_in_quotes == StartInQuotes::Maybe {
                                inquote = at_start;
                                if inquote { start_in_quotes = StartInQuotes::Yes; }
                            } else if search.forward {
                                inquote = true;
                            }
                        }
                    }
                }
                // Otherwise an uneven count resets `inquote`, which entering
                // a line without a continuation has already done.
            }
            if start_in_quotes == StartInQuotes::Maybe {
                start_in_quotes = StartInQuotes::No;
            }
        }
        let Some(character) = (at < range.end).then(|| first_char(lines, at)).flatten() else {
            // A line end without a trailing backslash resets the quote state.
            if last_byte(lines, &range) != Some(b'\\') {
                inquote = false;
                start_in_quotes = StartInQuotes::No;
            }
            continue;
        };
        match character {
            '"' => {
                let toggles = *toggles.get_or_insert_with(|| count_quotes(lines, &range, None).0);
                if toggles && backslashes_before(lines, at, range.start) % 2 == 0 {
                    inquote = !inquote;
                    start_in_quotes = StartInQuotes::No;
                }
            }
            '\'' => {
                if !rules.vi_compatible {
                    if let Some(literal) = character_literal(lines, at, &range, search.forward) {
                        at = literal;
                    }
                }
            }
            character if character == search.target || character == search.nested => {
                if inquote && start_in_quotes != StartInQuotes::Yes {
                    continue;
                }
                if !rules.ignore_backslashes && (backslashes_before(lines, at, range.start) % 2 == 1) != search.escaped {
                    continue;
                }
                if character == search.nested {
                    depth += 1;
                } else if depth == 0 {
                    return Some(at);
                } else {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
}

/// Vim's skip over `'x'` and `'\x'` from either quote, compared bytewise as
/// Vim does. Returns the other quote, where the scan resumes.
fn character_literal(lines: &HardLineSnapshot, at: usize, range: &Range<usize>, forward: bool) -> Option<usize> {
    let byte = |offset: usize| (range.start <= offset && offset < range.end)
        .then(|| lines.byte_chunk_at(offset).first().copied()).flatten();
    let target = if forward {
        byte(at + 1)?;
        if byte(at + 1) == Some(b'\\') && byte(at + 2).is_some() && byte(at + 3) == Some(b'\'') {
            at + 3
        } else if byte(at + 2) == Some(b'\'') {
            at + 2
        } else {
            return None;
        }
    } else {
        if at - range.start <= 1 {
            return None;
        }
        if byte(at - 2) == Some(b'\'') {
            at - 2
        } else if byte(at - 2) == Some(b'\\') && at - range.start > 2 && byte(at - 3) == Some(b'\'') {
            at - 3
        } else {
            return None;
        }
    };
    // A byte jump can only resume at a logical item.
    lines.is_grapheme_boundary(target).then_some(target)
}

/// Whether the line holds an even number of double quotes, skipping `\"`,
/// `\\` and `'"'`, and that parity just before `probe` when the scan reaches it.
fn count_quotes(lines: &HardLineSnapshot, range: &Range<usize>, probe: Option<usize>) -> (bool, Option<bool>) {
    // Vim starts this count at -1, so an odd running count means even quotes.
    let mut count = -1i64;
    let mut at_start = None;
    let mut previous = None::<u8>;
    let mut quote_after_apostrophe = false;
    let mut skip = false;
    let mut at = range.start;
    while at < range.end {
        let chunk = lines.byte_chunk_at(at);
        let length = chunk.len().min(range.end - at);
        if length == 0 {
            break;
        }
        for (offset, &byte) in chunk[..length].iter().enumerate() {
            let position = at + offset;
            if quote_after_apostrophe {
                // A quote between apostrophes, as in '"', is not counted.
                if byte != b'\'' { count += 1; }
                quote_after_apostrophe = false;
            }
            if skip {
                skip = false;
            } else {
                if probe == Some(position) { at_start = Some(count & 1 == 1); }
                if byte == b'"' {
                    if previous == Some(b'\'') { quote_after_apostrophe = true; } else { count += 1; }
                } else if byte == b'\\' {
                    skip = true;
                }
            }
            previous = Some(byte);
        }
        at += length;
    }
    if quote_after_apostrophe { count += 1; }
    (count & 1 == 1, at_start)
}

fn backslashes_before(lines: &HardLineSnapshot, mut at: usize, start: usize) -> usize {
    let mut count = 0;
    while at > start {
        let chunk = lines.byte_chunk_before(at);
        let available = chunk.len().min(at - start);
        let run = chunk[chunk.len() - available..].iter().rev().take_while(|byte| **byte == b'\\').count();
        count += run;
        if run < available || available == 0 {
            break;
        }
        at -= run;
    }
    count
}

fn last_byte(lines: &HardLineSnapshot, range: &Range<usize>) -> Option<u8> {
    (!range.is_empty()).then(|| lines.byte_chunk_before(range.end).last().copied()).flatten()
}

/// Leaves are scalar-aligned, so the first scalar is within the chunk.
fn first_char(lines: &HardLineSnapshot, at: usize) -> Option<char> {
    let chunk = lines.byte_chunk_at(at);
    let prefix = &chunk[..chunk.len().min(4)];
    let valid = std::str::from_utf8(prefix).map_or_else(|error| error.valid_up_to(), |_| prefix.len());
    std::str::from_utf8(&prefix[..valid]).ok()?.chars().next()
}

/// The pair containing `delimiter`, its partner and whether it opens.
pub(super) fn delimiter_pair(delimiter: char) -> Option<(char, char, bool)> {
    Some(match delimiter {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        '}' => ('{', '}', false),
        _ => return None,
    })
}

/// Vim's `%` from a delimiter at `at`: the partner with the same escaping.
pub(super) fn matching_delimiter(lines: &HardLineSnapshot, at: usize, rules: MatchRules) -> Option<usize> {
    let line: HardLineInfo = lines.line_at_offset(at).ok()?;
    let (opening, closing, opens) = delimiter_pair(first_char(lines, at)?)?;
    let escaped = !rules.ignore_backslashes && backslashes_before(lines, at, line.content_range().start) % 2 == 1;
    let (target, nested) = if opens { (closing, opening) } else { (opening, closing) };
    find_delimiter(lines, at, DelimiterSearch { target, nested, forward: opens, escaped }, rules)
}

#[cfg(test)]
#[path = "delimiter_match_tests.rs"]
mod tests;
