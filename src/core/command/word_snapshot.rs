//! Word motions over a snapshot. Only the traversed words are queried; no
//! whole-document text or per-grapheme vector is retained by a command.
use super::text::{class_for, WordClass};
use crate::document::HardLineSnapshot;

fn class(lines: &HardLineSnapshot, at: usize, big: bool) -> WordClass {
    let chunk = lines.byte_chunk_at(at);
    let width = chunk.first().map_or(0, |byte| match byte { 0..=127 => 1, 128..=223 => 2, 224..=239 => 3, _ => 4 });
    let character = std::str::from_utf8(&chunk[..width]).expect("tree leaves are UTF-8").chars().next();
    class_for(big, match character {
        None => WordClass::White,
        Some(c) if c.is_whitespace() => WordClass::White,
        Some(c) if c.is_alphanumeric() || c == '_' => WordClass::Keyword,
        Some(_) => WordClass::Punctuation,
    })
}

fn newline(lines: &HardLineSnapshot, at: usize) -> bool {
    lines.byte_chunk_at(at).first() == Some(&b'\n')
}

// Unicode alone can join a literal CR to a semantic LF, or combining/prepend
// text to an image object. Only clusters containing those spellings need the
// snapshot's semantic item query; ordinary clusters stay in the borrowed leaf.
fn needs_item_boundary<'a>(lines: &'a HardLineSnapshot, mut at: usize, mut len: usize, mut chunk: &'a [u8]) -> bool {
    while len > 0 {
        let take = len.min(chunk.len());
        let bytes = &chunk[..take];
        if bytes.contains(&b'\r') || bytes.windows(3).any(|bytes| bytes == b"\xef\xbf\xbc") { return true; }
        at += take;
        len -= take;
        if len > 0 { chunk = lines.byte_chunk_at(at); }
    }
    false
}

pub(super) fn forward(lines: &HardLineSnapshot, at: usize, big: bool, count: usize) -> usize {
    let mut items = ForwardGraphemes::new(lines, at).peekable();
    let mut preceding_newline = at == 0 || lines.previous_grapheme_boundary(at).is_some_and(|p| newline(lines, p));
    for _ in 0..count.max(1) {
        let Some(current) = items.peek().copied() else { return lines.text_length(); };
        if current.newline && preceding_newline { preceding_newline = items.next().unwrap().newline; }
        if current.class != WordClass::White {
            let kind = class_for(big, current.class);
            while items.peek().is_some_and(|item| class_for(big, item.class) == kind) {
                preceding_newline = items.next().unwrap().newline;
            }
        }
        while items.peek().is_some_and(|item| item.class == WordClass::White && !(item.newline && preceding_newline)) {
            preceding_newline = items.next().unwrap().newline;
        }
    }
    items.peek().map_or(lines.text_length(), |item| item.at)
}

pub(super) fn end(lines: &HardLineSnapshot, at: usize, big: bool, count: usize, stop: bool) -> usize {
    let len = lines.text_length();
    if len == 0 { return 0; }
    let last = lines.previous_grapheme_boundary(len).unwrap_or(0);
    let at = if at >= len { last } else { at };
    let first = if stop { at } else { lines.next_grapheme_boundary(at).unwrap_or(len) };
    let mut items = ForwardGraphemes::new(lines, first).peekable();
    let mut last_nonwhite = at;
    for _ in 0..count.max(1) {
        while items.peek().is_some_and(|item| item.class == WordClass::White) { items.next(); }
        let Some(current) = items.next() else {
            let trailing = if newline(lines, last) {
                lines.previous_grapheme_boundary(last).filter(|p| !newline(lines, *p)).unwrap_or(last)
            } else { last };
            return trailing.max(last_nonwhite);
        };
        let kind = class_for(big, current.class);
        last_nonwhite = current.last;
        while items.peek().is_some_and(|item| class_for(big, item.class) == kind) {
            last_nonwhite = items.next().unwrap().last;
        }
    }
    last_nonwhite
}

struct ForwardGraphemes<'a> {
    lines: &'a HardLineSnapshot,
    cursor: unicode_segmentation::GraphemeCursor,
    chunk: &'a str,
    start: usize,
}

impl<'a> ForwardGraphemes<'a> {
    fn new(lines: &'a HardLineSnapshot, at: usize) -> Self {
        Self { lines, cursor: unicode_segmentation::GraphemeCursor::new(at, lines.text_length(), true),
            chunk: std::str::from_utf8(lines.byte_chunk_at(at)).expect("grapheme boundary in UTF-8 leaf"), start: at }
    }
}

impl Iterator for ForwardGraphemes<'_> {
    type Item = WordRun;
    fn next(&mut self) -> Option<Self::Item> {
        use unicode_segmentation::GraphemeIncomplete;
        let at = self.cursor.cur_cursor();
        if at >= self.lines.text_length() { return None; }
        if at == self.start + self.chunk.len() {
            self.start = at;
            self.chunk = std::str::from_utf8(self.lines.byte_chunk_at(at)).expect("UTF-8 leaf");
        }
        let tail = &self.chunk[at - self.start..];
        let mut item = WordRun::new(at, tail.chars().next().expect("following grapheme"));
        // Adjacent ASCII scalars are independent graphemes, apart from CRLF.
        // Stop before the final ASCII byte beside non-ASCII text or a leaf
        // boundary: a following combining scalar may join that last byte.
        let bytes = tail.as_bytes();
        let mut skip = 0;
        while skip + 1 < bytes.len() && bytes[skip].is_ascii() && bytes[skip + 1].is_ascii()
            && !matches!(bytes[skip], b'\r' | b'\n')
            && WordRun::new(0, bytes[skip] as char).class == item.class {
            skip += 1;
        }
        if skip > 0 {
            self.cursor.set_cursor(at + skip);
            item.last = at + skip - 1;
            return Some(item);
        }
        loop {
            match self.cursor.next_boundary(self.chunk, self.start) {
                Ok(Some(end)) => {
                    if needs_item_boundary(self.lines, at, end - at, tail.as_bytes()) {
                        let end = self.lines.next_grapheme_boundary(at).expect("following logical item");
                        *self = Self::new(self.lines, end);
                    }
                    return Some(item);
                },
                Ok(None) => return None,
                Err(GraphemeIncomplete::NextChunk) => {
                    self.start += self.chunk.len();
                    self.chunk = std::str::from_utf8(self.lines.byte_chunk_at(self.start)).expect("UTF-8 leaf");
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let bytes = self.lines.byte_chunk_before(end);
                    self.cursor.provide_context(std::str::from_utf8(bytes).expect("UTF-8 context"), end - bytes.len());
                }
                Err(_) => unreachable!("forward cursor starts at a validated grapheme boundary"),
            }
        }
    }
}

/// A reverse Unicode cursor retains one borrowed leaf rather than resolving
/// the tree again for every grapheme. GraphemeCursor requests preceding context
/// when a cluster or a regional-indicator sequence crosses a leaf boundary.
struct ReverseGraphemes<'a> {
    lines: &'a HardLineSnapshot,
    cursor: unicode_segmentation::GraphemeCursor,
    chunk: &'a str,
    start: usize,
}

#[derive(Clone, Copy)]
struct WordRun { at: usize, last: usize, class: WordClass, newline: bool }

impl WordRun {
    fn new(at: usize, character: char) -> Self {
        let class = if character.is_whitespace() { WordClass::White }
            else if character.is_alphanumeric() || character == '_' { WordClass::Keyword }
            else { WordClass::Punctuation };
        Self { at, last: at, class, newline: character == '\n' }
    }
}

impl<'a> ReverseGraphemes<'a> {
    fn new(lines: &'a HardLineSnapshot, at: usize) -> Self {
        let bytes = lines.byte_chunk_before(at);
        Self { lines, cursor: unicode_segmentation::GraphemeCursor::new(at, lines.text_length(), true),
            chunk: std::str::from_utf8(bytes).expect("grapheme boundary in UTF-8 leaf"), start: at - bytes.len() }
    }
}

impl Iterator for ReverseGraphemes<'_> {
    type Item = WordRun;
    fn next(&mut self) -> Option<Self::Item> {
        use unicode_segmentation::GraphemeIncomplete;
        let end = self.cursor.cur_cursor();
        let local = end - self.start;
        let bytes = self.chunk.as_bytes();
        if local >= 2 && bytes[local - 1].is_ascii() && bytes[local - 2].is_ascii()
            && !matches!(bytes[local - 1], b'\r' | b'\n') {
            let mut item = WordRun::new(end - 1, bytes[local - 1] as char);
            let mut first = local - 1;
            // Keep an ASCII predecessor as proof that the new run boundary
            // is not inside a grapheme beginning in preceding Unicode text.
            while first >= 2 && bytes[first - 2].is_ascii() && bytes[first - 1].is_ascii()
                && !matches!(bytes[first - 1], b'\r' | b'\n')
                && WordRun::new(0, bytes[first - 1] as char).class == item.class {
                first -= 1;
            }
            item.at = self.start + first;
            self.cursor.set_cursor(item.at);
            return Some(item);
        }
        loop {
            match self.cursor.prev_boundary(self.chunk, self.start) {
                Ok(Some(at)) => {
                    let tail = if at == self.start + self.chunk.len() {
                        std::str::from_utf8(self.lines.byte_chunk_at(at)).expect("UTF-8 leaf")
                    } else { &self.chunk[at - self.start..] };
                    if needs_item_boundary(self.lines, at, end - at, tail.as_bytes()) {
                        let at = self.lines.previous_grapheme_boundary(end).expect("preceding logical item");
                        *self = Self::new(self.lines, at);
                        let tail = std::str::from_utf8(self.lines.byte_chunk_at(at)).expect("UTF-8 leaf");
                        return Some(WordRun::new(at, tail.chars().next().expect("preceding logical item")));
                    }
                    let character = tail.chars().next().expect("preceding grapheme");
                    return Some(WordRun::new(at, character));
                }
                Ok(None) => return None,
                Err(GraphemeIncomplete::PrevChunk) => {
                    let bytes = self.lines.byte_chunk_before(self.start);
                    self.start -= bytes.len();
                    self.chunk = std::str::from_utf8(bytes).expect("UTF-8 leaf");
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let bytes = self.lines.byte_chunk_before(end);
                    self.cursor.provide_context(std::str::from_utf8(bytes).expect("UTF-8 context"), end - bytes.len());
                }
                Err(_) => unreachable!("reverse cursor starts at a validated grapheme boundary"),
            }
        }
    }
}

pub(super) fn backward(lines: &HardLineSnapshot, at: usize, big: bool, count: usize) -> usize {
    let mut previous = ReverseGraphemes::new(lines, at).peekable();
    let Some(mut current) = previous.next() else { return 0; };
    for iteration in 0..count.max(1) {
        if iteration > 0 {
            let Some(next) = previous.next() else { return 0; };
            current = next;
        }
        while current.at > 0 && current.class == WordClass::White {
            if current.newline && previous.peek().is_some_and(|item| item.newline) { break; }
            current = previous.next().expect("nonzero boundary has a preceding grapheme");
        }
        if current.newline && (current.at == 0 || previous.peek().is_some_and(|item| item.newline)) {
            if current.at == 0 { return 0; }
            continue;
        }
        let current_class = class_for(big, current.class);
        while previous.peek().is_some_and(|item| class_for(big, item.class) == current_class) {
            current = previous.next().unwrap();
        }
        if current.at == 0 { return 0; }
    }
    current.at
}

pub(super) fn end_backward(lines: &HardLineSnapshot, mut at: usize, big: bool, count: usize) -> usize {
    let mut previous = ReverseGraphemes::new(lines, at);
    let mut current_class = if at < lines.text_length() { class(lines, at, big) } else { WordClass::White };
    for _ in 0..count.max(1) {
        let mut candidate = previous.next();
        if current_class != WordClass::White {
            while candidate.is_some_and(|item| class_for(big, item.class) == current_class) {
                candidate = previous.next();
            }
        }
        while candidate.is_some_and(|item| item.class == WordClass::White) { candidate = previous.next(); }
        let Some(item) = candidate else { return 0; };
        at = item.last;
        current_class = class_for(big, item.class);
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::text;
    use crate::document::{Document, Encoding, Format};
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn leaf_cursors_match_word_motion_reference() {
        let cases = [
            String::new(), "a".into(), "one two ++ three\n\nfour\n".into(), " \t\n\n \n".into(),
            "a\u{301}b café—漢字 👩🏽‍💻!! end\n\nnext".into(), "\u{600}ab !! 🇺🇸🇨🇦 next".into(),
            format!("start {}é{} end", "a".repeat(4093), "\u{301}".repeat(20)),
            format!("start {} end", "🇺".repeat(1101)),
        ];
        for source in cases {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
            let lines = document.hard_line_snapshot();
            let mut points = source.grapheme_indices(true).map(|(at, _)| at).collect::<Vec<_>>();
            points.push(source.len());
            for &at in points.iter().step_by((points.len() / 32).max(1)) {
                for big in [false, true] { for count in [0, 1, 2, 3, 40] {
                    assert_eq!(forward(&lines, at, big, count), text::move_word_forward(&source, at, big, count), "w {at} {big} {count}: {source:?}");
                    assert_eq!(end(&lines, at, big, count, false), text::move_word_end(&source, at, big, count), "e {at} {big} {count}: {source:?}");
                    assert_eq!(backward(&lines, at, big, count), text::move_word_backward(&source, at, big, count), "b {at} {big} {count}: {source:?}");
                    assert_eq!(end_backward(&lines, at, big, count), text::move_word_end_backward(&source, at, big, count), "ge {at} {big} {count}: {source:?}");
                } }
            }
        }
    }
}
