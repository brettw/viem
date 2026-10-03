//! Resolution of Vim text objects into checked, half-open UTF-8 ranges.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::document::HardLineSnapshot;

use super::text::{is_east_asian_sentence_terminator, is_sentence_closer, is_sentence_terminator};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextObjectScope {
    Inner,
    Around,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextObjectKind {
    Word,
    BigWord,
    Sentence,
    Paragraph,
    Quote(QuoteKind),
    Pair(PairKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuoteKind {
    Double,
    Single,
    Backtick,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairKind {
    Parentheses,
    Brackets,
    Braces,
    Angles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextObject {
    pub scope: TextObjectScope,
    pub kind: TextObjectKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextObjectError {
    ZeroCount,
    CursorOutOfBounds(usize),
    CursorNotGraphemeBoundary(usize),
    ResultNotGraphemeBoundary(usize),
    NotFound,
}

impl TextObjectKind {
    /// Translate the key following `i` or `a` in Vim's text-object grammar.
    pub fn from_vim_key(key: char) -> Option<Self> {
        match key {
            'w' => Some(Self::Word),
            'W' => Some(Self::BigWord),
            's' => Some(Self::Sentence),
            'p' => Some(Self::Paragraph),
            '"' => Some(Self::Quote(QuoteKind::Double)),
            '\'' => Some(Self::Quote(QuoteKind::Single)),
            '`' => Some(Self::Quote(QuoteKind::Backtick)),
            '(' | ')' | 'b' => Some(Self::Pair(PairKind::Parentheses)),
            '[' | ']' => Some(Self::Pair(PairKind::Brackets)),
            '{' | '}' | 'B' => Some(Self::Pair(PairKind::Braces)),
            '<' | '>' => Some(Self::Pair(PairKind::Angles)),
            _ => None,
        }
    }
}

/// Resolve an object in one immutable projection. Both the cursor and returned
/// range use formatted UTF-8 byte offsets and legal grapheme boundaries.
pub fn resolve_text_object(
    text: &str,
    lines: &HardLineSnapshot,
    cursor: usize,
    object: TextObject,
    count: usize,
) -> Result<Range<usize>, TextObjectError> {
    if count == 0 {
        return Err(TextObjectError::ZeroCount);
    }
    validate_cursor(text, cursor)?;
    let range = match object.kind {
        TextObjectKind::Word => resolve_word(text, cursor, object.scope, count, false),
        TextObjectKind::BigWord => resolve_word(text, cursor, object.scope, count, true),
        TextObjectKind::Sentence => resolve_sentence(text, lines, cursor, object.scope, count),
        TextObjectKind::Paragraph => resolve_paragraph(text, lines, cursor, object.scope, count),
        TextObjectKind::Quote(kind) => {
            resolve_quote(text, lines, cursor, object.scope, count, kind)
        }
        TextObjectKind::Pair(kind) => resolve_pair(text, cursor, object.scope, count, kind),
    }
    .ok_or(TextObjectError::NotFound)?;
    validate_result(text, range)
}

fn validate_cursor(text: &str, cursor: usize) -> Result<(), TextObjectError> {
    if cursor > text.len() {
        return Err(TextObjectError::CursorOutOfBounds(cursor));
    }
    if !is_grapheme_boundary(text, cursor) {
        return Err(TextObjectError::CursorNotGraphemeBoundary(cursor));
    }
    Ok(())
}

fn validate_result(text: &str, range: Range<usize>) -> Result<Range<usize>, TextObjectError> {
    if !is_grapheme_boundary(text, range.start) {
        return Err(TextObjectError::ResultNotGraphemeBoundary(range.start));
    }
    if !is_grapheme_boundary(text, range.end) {
        return Err(TextObjectError::ResultNotGraphemeBoundary(range.end));
    }
    Ok(range)
}

fn is_grapheme_boundary(text: &str, at: usize) -> bool {
    at == text.len()
        || text
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == at)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WordClass {
    Space,
    Keyword,
    Punctuation,
}

fn word_class(grapheme: &str, big: bool) -> WordClass {
    if grapheme.chars().all(char::is_whitespace) {
        WordClass::Space
    } else if big || grapheme.chars().next().is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
        WordClass::Keyword
    } else {
        WordClass::Punctuation
    }
}

/// Pointer word selection visits only the hit word, not the entire document
/// or hard line. A line's trailing boundary selects its final word.
pub(super) fn pointer_word_range(lines: &HardLineSnapshot, offset: usize) -> Option<Range<usize>> {
    let line = lines.line_at_offset(offset).ok()?.content_range();
    if line.is_empty() { return Some(line); }
    let at = if offset == line.end { lines.previous_grapheme_boundary(offset)? } else { offset };
    let class_at = |at| {
        let range = lines.grapheme_range_at(at)?;
        Some(word_class(&lines.slice_utf8(range).ok()?, false))
    };
    let class = class_at(at)?;
    let mut start = at;
    while start > line.start {
        let previous = lines.previous_grapheme_boundary(start)?;
        if class_at(previous)? != class { break; }
        start = previous;
    }
    let mut end = lines.next_grapheme_boundary(at)?;
    while end < line.end && class_at(end)? == class {
        end = lines.next_grapheme_boundary(end)?;
    }
    Some(start..end)
}

#[derive(Clone, Debug)]
struct WordRun {
    range: Range<usize>,
    class: WordClass,
}

fn resolve_word(
    text: &str,
    cursor: usize,
    scope: TextObjectScope,
    count: usize,
    big: bool,
) -> Option<Range<usize>> {
    let runs = word_runs(text, big);
    if runs.is_empty() {
        return None;
    }
    let current = run_at_cursor(&runs, cursor, text.len())?;
    if runs[current].class == WordClass::Space {
        return resolve_word_from_space(&runs, current, scope, count);
    }
    let mut last = current;
    for _ in 1..count {
        last = next_nonspace(&runs, last + 1)?;
    }
    let start = runs[current].range.start;
    let end = runs[last].range.end;
    if scope == TextObjectScope::Inner {
        return Some(start..end);
    }
    if let Some(after) = runs
        .get(last + 1)
        .filter(|run| run.class == WordClass::Space)
    {
        Some(start..after.range.end)
    } else if current > 0 && runs[current - 1].class == WordClass::Space {
        Some(runs[current - 1].range.start..end)
    } else {
        Some(start..end)
    }
}

fn resolve_word_from_space(
    runs: &[WordRun],
    current: usize,
    scope: TextObjectScope,
    count: usize,
) -> Option<Range<usize>> {
    if scope == TextObjectScope::Inner && count == 1 {
        return Some(runs[current].range.clone());
    }
    if let Some(first_word) = next_nonspace(runs, current + 1) {
        let mut last = first_word;
        for _ in 1..count {
            last = next_nonspace(runs, last + 1)?;
        }
        let mut end = runs[last].range.end;
        if scope == TextObjectScope::Around {
            if let Some(after) = runs
                .get(last + 1)
                .filter(|run| run.class == WordClass::Space)
            {
                end = after.range.end;
            }
        }
        return Some(runs[current].range.start..end);
    }
    let previous = (0..current)
        .rev()
        .find(|index| runs[*index].class != WordClass::Space)?;
    Some(runs[previous].range.start..runs[current].range.end)
}

fn word_runs(text: &str, big: bool) -> Vec<WordRun> {
    let mut result: Vec<WordRun> = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        let class = word_class(grapheme, big);
        if let Some(last) = result.last_mut().filter(|last| last.class == class) {
            last.range.end = end;
        } else {
            result.push(WordRun {
                range: start..end,
                class,
            });
        }
    }
    result
}

fn run_at_cursor(runs: &[WordRun], cursor: usize, text_len: usize) -> Option<usize> {
    if cursor == text_len {
        return runs.len().checked_sub(1);
    }
    runs.iter()
        .position(|run| run.range.start <= cursor && cursor < run.range.end)
}

fn next_nonspace(runs: &[WordRun], from: usize) -> Option<usize> {
    (from..runs.len()).find(|index| runs[*index].class != WordClass::Space)
}

#[derive(Clone, Debug)]
struct ParagraphSpan {
    content: Range<usize>,
    before: Range<usize>,
    after: Range<usize>,
}

fn resolve_paragraph(
    text: &str,
    lines: &HardLineSnapshot,
    cursor: usize,
    scope: TextObjectScope,
    count: usize,
) -> Option<Range<usize>> {
    let paragraphs = paragraph_spans(text, lines);
    let ranges: Vec<_> = paragraphs
        .iter()
        .map(|paragraph| paragraph.content.clone())
        .collect();
    let first = object_at_or_after(&ranges, cursor)?;
    let last = first.checked_add(count - 1)?;
    let last_paragraph = paragraphs.get(last)?;
    let first_paragraph = &paragraphs[first];
    if scope == TextObjectScope::Inner {
        return Some(first_paragraph.content.start..last_paragraph.content.end);
    }
    if !last_paragraph.after.is_empty() {
        Some(first_paragraph.content.start..last_paragraph.after.end)
    } else if !first_paragraph.before.is_empty() {
        Some(first_paragraph.before.start..last_paragraph.content.end)
    } else {
        Some(first_paragraph.content.start..last_paragraph.content.end)
    }
}

fn paragraph_spans(text: &str, hard_lines: &HardLineSnapshot) -> Vec<ParagraphSpan> {
    #[derive(Clone)]
    struct Line {
        start: usize,
        content_end: usize,
        blank: bool,
    }
    let lines = hard_lines
        .lines(0..hard_lines.line_count())
        .expect("the complete authoritative hard-line range is valid")
        .into_iter()
        .map(|line| {
            let content = line.content_range();
            Line {
                start: content.start,
                content_end: content.end,
                // Preserve the existing whitespace-only paragraph behavior,
                // but do not mistake a literal LF inside a forced-Mac hard
                // line for a projected blank-line boundary.
                blank: text[content]
                    .chars()
                    .all(|character| character != '\n' && character.is_whitespace()),
            }
        })
        .collect::<Vec<_>>();
    let mut raw = Vec::<Range<usize>>::new();
    let mut line = 0;
    while line < lines.len() {
        while line < lines.len() && lines[line].blank {
            line += 1;
        }
        if line == lines.len() {
            break;
        }
        let paragraph_start = lines[line].start;
        let mut end = lines[line].content_end;
        while line < lines.len() && !lines[line].blank {
            end = lines[line].content_end;
            line += 1;
        }
        raw.push(paragraph_start..end);
    }
    raw.iter()
        .enumerate()
        .map(|(index, content)| {
            let before_start = index
                .checked_sub(1)
                .map(|previous| raw[previous].end)
                .unwrap_or(content.start);
            let after_end = raw
                .get(index + 1)
                .map(|next| next.start)
                .unwrap_or(content.end);
            ParagraphSpan {
                content: content.clone(),
                before: before_start..content.start,
                after: content.end..after_end,
            }
        })
        .collect()
}

fn resolve_sentence(
    text: &str,
    lines: &HardLineSnapshot,
    cursor: usize,
    scope: TextObjectScope,
    count: usize,
) -> Option<Range<usize>> {
    let paragraphs = paragraph_spans(text, lines);
    let mut sentences = Vec::new();
    for paragraph in &paragraphs {
        sentences.extend(sentence_ranges(text, paragraph.content.clone()));
    }
    let first = object_at_or_after(&sentences, cursor)?;
    let last = first.checked_add(count - 1)?;
    let last_range = sentences.get(last)?;
    let first_range = &sentences[first];
    if scope == TextObjectScope::Inner {
        return Some(first_range.start..last_range.end);
    }
    let paragraph = paragraphs.iter().find(|paragraph| {
        paragraph.content.start <= first_range.start && first_range.end <= paragraph.content.end
    })?;
    let following_boundary = sentences
        .get(last + 1)
        .filter(|next| next.start <= paragraph.content.end)
        .map(|next| next.start)
        .unwrap_or(paragraph.content.end);
    if last_range.end < following_boundary {
        Some(first_range.start..following_boundary)
    } else {
        let preceding_boundary = first
            .checked_sub(1)
            .and_then(|previous| sentences.get(previous))
            .filter(|previous| paragraph.content.start <= previous.end)
            .map(|previous| previous.end)
            .unwrap_or(paragraph.content.start);
        Some(preceding_boundary..last_range.end)
    }
}

fn sentence_ranges(text: &str, paragraph: Range<usize>) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut at = skip_whitespace_forward(text, paragraph.start, paragraph.end);
    while at < paragraph.end {
        let start = at;
        let mut end = paragraph.end;
        while at < paragraph.end {
            let ch = text[at..].chars().next().unwrap();
            at += ch.len_utf8();
            if is_sentence_terminator(ch) {
                while at < paragraph.end {
                    let closer = text[at..].chars().next().unwrap();
                    if !is_sentence_closer(closer) {
                        break;
                    }
                    at += closer.len_utf8();
                }
                if at == paragraph.end
                    || text[at..].chars().next().is_some_and(char::is_whitespace)
                    || is_east_asian_sentence_terminator(ch)
                {
                    end = at;
                    break;
                }
            }
        }
        end = trim_whitespace_end(text, start, end);
        if start < end {
            result.push(start..end);
        }
        at = skip_whitespace_forward(text, at.max(end), paragraph.end);
    }
    result
}

fn skip_whitespace_forward(text: &str, mut at: usize, end: usize) -> usize {
    while at < end {
        let ch = text[at..].chars().next().unwrap();
        if !ch.is_whitespace() {
            break;
        }
        at += ch.len_utf8();
    }
    at
}

fn trim_whitespace_end(text: &str, start: usize, mut end: usize) -> usize {
    while start < end {
        let ch = text[..end].chars().next_back().unwrap();
        if !ch.is_whitespace() {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

fn object_at_or_after(objects: &[Range<usize>], cursor: usize) -> Option<usize> {
    objects
        .iter()
        .position(|range| range.start <= cursor && cursor < range.end)
        .or_else(|| objects.iter().position(|range| cursor <= range.start))
        .or_else(|| objects.len().checked_sub(1))
}

fn resolve_quote(
    text: &str,
    lines: &HardLineSnapshot,
    cursor: usize,
    scope: TextObjectScope,
    count: usize,
    kind: QuoteKind,
) -> Option<Range<usize>> {
    let delimiter = match kind {
        QuoteKind::Double => '"',
        QuoteKind::Single => '\'',
        QuoteKind::Backtick => '`',
    };
    let line = lines
        .line_at_offset(cursor)
        .expect("a validated text-object cursor resolves to one hard line");
    let line = line.content_range();
    let markers = quote_markers(text, line.clone(), delimiter);
    let exact = markers.iter().position(|marker| marker.start == cursor);
    let (open, close) = if let Some(index) = exact {
        if index % 2 == 0 {
            (markers.get(index)?, markers.get(index + 1)?)
        } else {
            (markers.get(index - 1)?, markers.get(index)?)
        }
    } else {
        let next = markers.iter().position(|marker| cursor < marker.start)?;
        if next == 0 {
            (markers.first()?, markers.get(1)?)
        } else {
            (markers.get(next - 1)?, markers.get(next)?)
        }
    };
    match scope {
        TextObjectScope::Inner if count == 1 => Some(open.end..close.start),
        // Vim treats a count greater than one on an inner quote object as a
        // request to include the delimiters.  It does not advance to a later
        // quoted string.
        TextObjectScope::Inner => Some(open.start..close.end),
        TextObjectScope::Around => Some(quote_around_range(text, line, open.start..close.end)),
    }
}

fn quote_markers(text: &str, line: Range<usize>, delimiter: char) -> Vec<Range<usize>> {
    let mut markers = Vec::new();
    for (relative, ch) in text[line.clone()].char_indices() {
        if ch != delimiter {
            continue;
        }
        let at = line.start + relative;
        if is_escaped(text, at, line.start) {
            continue;
        }
        markers.push(at..at + ch.len_utf8());
    }
    markers
}

fn quote_around_range(text: &str, line: Range<usize>, quoted: Range<usize>) -> Range<usize> {
    let mut end = quoted.end;
    while end < line.end {
        let ch = text[end..].chars().next().unwrap();
        if !matches!(ch, ' ' | '\t') {
            break;
        }
        end += ch.len_utf8();
    }
    if end != quoted.end {
        return quoted.start..end;
    }

    let mut start = quoted.start;
    while line.start < start {
        let ch = text[..start].chars().next_back().unwrap();
        if !matches!(ch, ' ' | '\t') {
            break;
        }
        start -= ch.len_utf8();
    }
    start..quoted.end
}

#[derive(Clone, Debug)]
struct DelimiterPair {
    open: Range<usize>,
    close: Range<usize>,
}

fn resolve_pair(
    text: &str,
    cursor: usize,
    scope: TextObjectScope,
    count: usize,
    kind: PairKind,
) -> Option<Range<usize>> {
    let (open, close) = match kind {
        PairKind::Parentheses => ('(', ')'),
        PairKind::Brackets => ('[', ']'),
        PairKind::Braces => ('{', '}'),
        PairKind::Angles => ('<', '>'),
    };
    let pairs = delimiter_pairs(text, open, close);
    let mut enclosing: Vec<&DelimiterPair> = pairs
        .iter()
        .filter(|pair| pair.open.start <= cursor && cursor <= pair.close.start)
        .collect();
    enclosing.sort_by_key(|pair| pair.close.end - pair.open.start);
    let pair = if let Some(pair) = enclosing.get(count - 1) {
        *pair
    } else if enclosing.is_empty() {
        forward_delimiter_pair(text, cursor, open, close, &pairs, count)?
    } else {
        return None;
    };
    match scope {
        // Keep the empty half-open interior representable.  Operator-pending
        // yank can observe it even though Vim rejects a fresh Visual `vi(`;
        // that context-specific distinction belongs to the caller.
        TextObjectScope::Inner => Some(pair.open.end..pair.close.start),
        TextObjectScope::Around => Some(pair.open.start..pair.close.end),
    }
}

fn forward_delimiter_pair<'a>(
    text: &str,
    cursor: usize,
    open: char,
    close: char,
    pairs: &'a [DelimiterPair],
    count: usize,
) -> Option<&'a DelimiterPair> {
    let next_open = text[cursor..].char_indices().find_map(|(relative, ch)| {
        let at = cursor + relative;
        (ch == open && !is_escaped(text, at, 0)).then_some(at)
    })?;
    let outer = pairs.iter().find(|pair| pair.open.start == next_open)?;
    if count == 1 {
        return Some(outer);
    }

    let mut depth = 0usize;
    for (relative, ch) in text[outer.open.start..outer.close.end].char_indices() {
        let at = outer.open.start + relative;
        if is_escaped(text, at, 0) {
            continue;
        }
        if ch == open {
            depth += 1;
            if depth == count {
                return pairs.iter().find(|pair| pair.open.start == at);
            }
        } else if ch == close {
            depth = depth.saturating_sub(1);
        }
    }
    None
}

fn delimiter_pairs(text: &str, open: char, close: char) -> Vec<DelimiterPair> {
    let mut stack = Vec::<Range<usize>>::new();
    let mut pairs = Vec::new();
    for (at, ch) in text.char_indices() {
        if ch != open && ch != close {
            continue;
        }
        if is_escaped(text, at, 0) {
            continue;
        }
        let marker = at..at + ch.len_utf8();
        if ch == open {
            stack.push(marker);
        } else if let Some(opening) = stack.pop() {
            pairs.push(DelimiterPair {
                open: opening,
                close: marker,
            });
        }
    }
    pairs
}

fn is_escaped(text: &str, at: usize, lower_bound: usize) -> bool {
    let mut slashes = 0;
    let mut before = at;
    while before > lower_bound && text.as_bytes()[before - 1] == b'\\' {
        slashes += 1;
        before -= 1;
    }
    slashes % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn resolve_text_object(
        text: &str,
        cursor: usize,
        object: TextObject,
        count: usize,
    ) -> Result<Range<usize>, TextObjectError> {
        let document = Document::new(text);
        super::resolve_text_object(text, &document.hard_line_snapshot(), cursor, object, count)
    }

    fn inner(kind: TextObjectKind) -> TextObject {
        TextObject {
            scope: TextObjectScope::Inner,
            kind,
        }
    }

    fn around(kind: TextObjectKind) -> TextObject {
        TextObject {
            scope: TextObjectScope::Around,
            kind,
        }
    }

    fn selected(text: &str, range: Result<Range<usize>, TextObjectError>) -> &str {
        &text[range.unwrap()]
    }

    #[test]
    fn vim_keys_include_closing_delimiters_and_aliases() {
        assert_eq!(
            TextObjectKind::from_vim_key(')'),
            Some(TextObjectKind::Pair(PairKind::Parentheses))
        );
        assert_eq!(
            TextObjectKind::from_vim_key('b'),
            Some(TextObjectKind::Pair(PairKind::Parentheses))
        );
        assert_eq!(
            TextObjectKind::from_vim_key('B'),
            Some(TextObjectKind::Pair(PairKind::Braces))
        );
        assert_eq!(TextObjectKind::from_vim_key('x'), None);
    }

    #[test]
    fn inner_and_around_word_choose_whitespace_like_vim() {
        let text = "one two";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 1, inner(TextObjectKind::Word), 1)
            ),
            "one"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 1, around(TextObjectKind::Word), 1)
            ),
            "one "
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 5, around(TextObjectKind::Word), 1)
            ),
            " two"
        );
    }

    #[test]
    fn word_and_big_word_treat_punctuation_differently() {
        let text = "alpha.beta next";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, inner(TextObjectKind::Word), 2)
            ),
            "alpha."
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, inner(TextObjectKind::BigWord), 1)
            ),
            "alpha.beta"
        );
    }

    #[test]
    fn whitespace_inner_word_and_counts_are_deterministic() {
        let text = "one   two three";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 3, inner(TextObjectKind::Word), 1)
            ),
            "   "
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 3, inner(TextObjectKind::Word), 2)
            ),
            "   two three"
        );
    }

    #[test]
    fn words_never_split_unicode_graphemes() {
        let text = "a\u{301}bc 😀 ok";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, inner(TextObjectKind::Word), 1)
            ),
            "a\u{301}bc"
        );
        let emoji = text.find('😀').unwrap();
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, emoji, inner(TextObjectKind::Word), 1)
            ),
            "😀"
        );
        assert_eq!(
            resolve_text_object(text, 1, inner(TextObjectKind::Word), 1),
            Err(TextObjectError::CursorNotGraphemeBoundary(1))
        );
    }

    #[test]
    fn sentences_include_closers_and_ignore_decimal_periods() {
        let text = "It is 3.14. \"Really?\" Yes!";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 3, inner(TextObjectKind::Sentence), 1)
            ),
            "It is 3.14."
        );
        let really = text.find("Really").unwrap();
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, really, inner(TextObjectKind::Sentence), 1)
            ),
            "\"Really?\""
        );
    }

    #[test]
    fn sentences_split_after_unspaced_east_asian_terminators() {
        let text = "甲。乙！丙？丁";
        let second = text.find('乙').unwrap();
        let third = text.find('丙').unwrap();
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, second, inner(TextObjectKind::Sentence), 1)
            ),
            "乙！"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, third, inner(TextObjectKind::Sentence), 1)
            ),
            "丙？"
        );
    }

    #[test]
    fn sentence_count_and_around_include_intervening_space() {
        let text = "One.  Two? Three!";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, inner(TextObjectKind::Sentence), 2)
            ),
            "One.  Two?"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, around(TextObjectKind::Sentence), 1)
            ),
            "One.  "
        );
    }

    #[test]
    fn paragraphs_use_blank_lines_and_counts() {
        let text = "first\nline\n\n  \nsecond\n\nthird";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 2, inner(TextObjectKind::Paragraph), 1)
            ),
            "first\nline"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 2, around(TextObjectKind::Paragraph), 1)
            ),
            "first\nline\n\n  \n"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 2, inner(TextObjectKind::Paragraph), 2)
            ),
            "first\nline\n\n  \nsecond"
        );
    }

    #[test]
    fn cursor_on_blank_selects_following_paragraph() {
        let text = "one\n\nnext";
        let blank = text.find("\n\n").unwrap() + 1;
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, blank, inner(TextObjectKind::Paragraph), 1)
            ),
            "next"
        );
    }

    #[test]
    fn quotes_honor_escaping_and_vim_around_whitespace() {
        let text = "x \"a \\\"quoted\\\" value\" y\n\"next\"";
        let value = text.find("value").unwrap();
        assert_eq!(
            selected(
                text,
                resolve_text_object(
                    text,
                    value,
                    inner(TextObjectKind::Quote(QuoteKind::Double)),
                    1
                )
            ),
            "a \\\"quoted\\\" value"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(
                    text,
                    value,
                    around(TextObjectKind::Quote(QuoteKind::Double)),
                    1
                )
            ),
            "\"a \\\"quoted\\\" value\" "
        );
    }

    #[test]
    fn quote_objects_use_cursor_relative_delimiters_and_do_not_search_backward() {
        let text = "aa  \"one\"  bb  \"two\"  cc";
        let kind = TextObjectKind::Quote(QuoteKind::Double);
        let first_open = text.find("\"one").unwrap();
        let first_close = text.find("\"  bb").unwrap();
        let second_open = text.find("\"two").unwrap();
        let second_close = text.find("\"  cc").unwrap();

        for cursor in [0, first_open, text.find("one").unwrap(), first_close] {
            assert_eq!(
                selected(text, resolve_text_object(text, cursor, inner(kind), 1)),
                "one"
            );
            assert_eq!(
                selected(text, resolve_text_object(text, cursor, around(kind), 1)),
                "\"one\"  "
            );
        }

        let between = text.find("bb").unwrap();
        assert_eq!(
            selected(text, resolve_text_object(text, between, inner(kind), 1)),
            "  bb  "
        );
        assert_eq!(
            selected(text, resolve_text_object(text, between, around(kind), 1)),
            "\"  bb  \""
        );

        for cursor in [second_open, text.find("two").unwrap(), second_close] {
            assert_eq!(
                selected(text, resolve_text_object(text, cursor, inner(kind), 1)),
                "two"
            );
        }
        for cursor in [second_close + 1, text.find("cc").unwrap()] {
            assert_eq!(
                resolve_text_object(text, cursor, inner(kind), 1),
                Err(TextObjectError::NotFound)
            );
        }
    }

    #[test]
    fn quote_counts_follow_vim_and_do_not_advance_to_another_string() {
        let text = "aa  \"one\"  bb  \"two\"  cc";
        let kind = TextObjectKind::Quote(QuoteKind::Double);
        let one = text.find("one").unwrap();
        assert_eq!(
            selected(text, resolve_text_object(text, one, inner(kind), 1)),
            "one"
        );
        for count in [2, 3, usize::MAX] {
            assert_eq!(
                selected(text, resolve_text_object(text, one, inner(kind), count)),
                "\"one\""
            );
            assert_eq!(
                selected(text, resolve_text_object(text, one, around(kind), count)),
                "\"one\"  "
            );
        }

        let between = text.find("bb").unwrap();
        assert_eq!(
            selected(text, resolve_text_object(text, between, inner(kind), 2)),
            "\"  bb  \""
        );
    }

    #[test]
    fn around_quote_prefers_trailing_ascii_blank_then_leading_ascii_blank() {
        let kind = TextObjectKind::Quote(QuoteKind::Double);
        let trailing = "x \"one\"\t  y";
        assert_eq!(
            selected(
                trailing,
                resolve_text_object(trailing, trailing.find("one").unwrap(), around(kind), 1)
            ),
            "\"one\"\t  "
        );

        let leading = "x\t  \"one\"";
        assert_eq!(
            selected(
                leading,
                resolve_text_object(leading, leading.find("one").unwrap(), around(kind), 1)
            ),
            "\t  \"one\""
        );

        let non_breaking = "x \"one\"\u{a0}y";
        assert_eq!(
            selected(
                non_breaking,
                resolve_text_object(
                    non_breaking,
                    non_breaking.find("one").unwrap(),
                    around(kind),
                    1
                )
            ),
            " \"one\""
        );
    }

    #[test]
    fn quote_delimiter_variants_share_pairing_and_escape_rules() {
        for (text, kind, expected) in [
            (
                "x 'a \\'quoted\\' value' y",
                QuoteKind::Single,
                "a \\'quoted\\' value",
            ),
            (
                "x `a \\`quoted\\` value` y",
                QuoteKind::Backtick,
                "a \\`quoted\\` value",
            ),
        ] {
            assert_eq!(
                selected(
                    text,
                    resolve_text_object(
                        text,
                        text.find("value").unwrap(),
                        inner(TextObjectKind::Quote(kind)),
                        1
                    )
                ),
                expected
            );
        }
    }

    #[test]
    fn quote_objects_stop_at_semantic_hard_lines() {
        let text = "x \"open\nclose\"";
        assert_eq!(
            resolve_text_object(
                text,
                text.find("open").unwrap(),
                inner(TextObjectKind::Quote(QuoteKind::Double)),
                1
            ),
            Err(TextObjectError::NotFound)
        );
    }

    #[test]
    fn quote_object_crosses_literal_lf_inside_a_forced_mac_hard_line() {
        let document = Document::from_bytes_with_file_format(
            b"a\r\"b\nc\"".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.text(), "a\n\"b\nc\"");
        let cursor = document.text().find('c').unwrap();
        let range = super::resolve_text_object(
            document.text(),
            &document.hard_line_snapshot(),
            cursor,
            inner(TextObjectKind::Quote(QuoteKind::Double)),
            1,
        )
        .unwrap();
        assert_eq!(&document.text()[range], "b\nc");
    }

    #[test]
    fn quote_objects_search_forward_and_ignore_around_count() {
        let text = "before 'one' and 'two'";
        assert_eq!(
            selected(
                text,
                resolve_text_object(text, 0, around(TextObjectKind::Quote(QuoteKind::Single)), 2)
            ),
            "'one' "
        );
    }

    #[test]
    fn pair_objects_search_forward_from_before_the_next_opening() {
        for (text, kind, expected) in [
            ("x (one) y", PairKind::Parentheses, ("one", "(one)")),
            ("x [one] y", PairKind::Brackets, ("one", "[one]")),
            ("x {one} y", PairKind::Braces, ("one", "{one}")),
            ("x <one> y", PairKind::Angles, ("one", "<one>")),
        ] {
            let kind = TextObjectKind::Pair(kind);
            let open = text.find(|ch| matches!(ch, '(' | '[' | '{' | '<')).unwrap();
            let close = text.find(|ch| matches!(ch, ')' | ']' | '}' | '>')).unwrap();
            for cursor in [0, open, text.find("one").unwrap(), close] {
                assert_eq!(
                    selected(text, resolve_text_object(text, cursor, inner(kind), 1)),
                    expected.0
                );
                assert_eq!(
                    selected(text, resolve_text_object(text, cursor, around(kind), 1)),
                    expected.1
                );
            }
            assert_eq!(
                resolve_text_object(text, text.find('y').unwrap(), inner(kind), 1),
                Err(TextObjectError::NotFound)
            );
        }
    }

    #[test]
    fn counted_forward_pair_descends_but_counted_enclosing_pair_climbs() {
        let text = "x (a (b (c) d) e) y";
        let kind = TextObjectKind::Pair(PairKind::Parentheses);
        for (count, expected) in [(1, "a (b (c) d) e"), (2, "b (c) d"), (3, "c")] {
            assert_eq!(
                selected(text, resolve_text_object(text, 0, inner(kind), count)),
                expected
            );
        }
        assert_eq!(
            resolve_text_object(text, 0, inner(kind), 4),
            Err(TextObjectError::NotFound)
        );

        let c = text.find('c').unwrap();
        assert_eq!(
            selected(text, resolve_text_object(text, c, inner(kind), 2)),
            "b (c) d"
        );
        assert_eq!(
            selected(text, resolve_text_object(text, c, inner(kind), 3)),
            "a (b (c) d) e"
        );

        let sequential = "x (a) (b) y";
        assert_eq!(
            resolve_text_object(sequential, 0, inner(kind), 2),
            Err(TextObjectError::NotFound)
        );
    }

    #[test]
    fn nested_pair_count_climbs_outward() {
        let text = "outer(a + (b * c)) tail";
        let cursor = text.find('b').unwrap();
        assert_eq!(
            selected(
                text,
                resolve_text_object(
                    text,
                    cursor,
                    inner(TextObjectKind::Pair(PairKind::Parentheses)),
                    1
                )
            ),
            "b * c"
        );
        assert_eq!(
            selected(
                text,
                resolve_text_object(
                    text,
                    cursor,
                    around(TextObjectKind::Pair(PairKind::Parentheses)),
                    2
                )
            ),
            "(a + (b * c))"
        );
    }

    #[test]
    fn escaped_pair_delimiters_do_not_participate() {
        let text = "\\(ignored\\) then (chosen)";
        assert_eq!(
            selected(
                text,
                resolve_text_object(
                    text,
                    0,
                    around(TextObjectKind::Pair(PairKind::Parentheses)),
                    1
                )
            ),
            "(chosen)"
        );
    }

    #[test]
    fn unmatched_and_insufficient_nesting_fail_cleanly() {
        let text = "before (inside) after";
        assert_eq!(
            resolve_text_object(
                text,
                text.find("inside").unwrap(),
                inner(TextObjectKind::Pair(PairKind::Parentheses)),
                2
            ),
            Err(TextObjectError::NotFound)
        );
        assert_eq!(
            resolve_text_object(
                "unmatched ( text",
                12,
                inner(TextObjectKind::Pair(PairKind::Parentheses)),
                1
            ),
            Err(TextObjectError::NotFound)
        );
    }

    #[test]
    fn delimiters_that_split_graphemes_are_rejected() {
        let text = "(\u{301}x)";
        assert_eq!(
            resolve_text_object(
                text,
                text.find('x').unwrap(),
                inner(TextObjectKind::Pair(PairKind::Parentheses)),
                1
            ),
            Err(TextObjectError::ResultNotGraphemeBoundary(1))
        );
    }

    #[test]
    fn invalid_count_and_cursor_are_explicit() {
        assert_eq!(
            resolve_text_object("word", 0, inner(TextObjectKind::Word), 0),
            Err(TextObjectError::ZeroCount)
        );
        assert_eq!(
            resolve_text_object("word", 9, inner(TextObjectKind::Word), 1),
            Err(TextObjectError::CursorOutOfBounds(9))
        );
    }
}
