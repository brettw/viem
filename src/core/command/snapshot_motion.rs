//! Sentence and paired-delimiter motions query immutable logical items. Scans
//! stop at the requested destination; unrelated prefixes/suffixes stay in the
//! persistent document tree rather than becoming a flat string or item array.
use super::text::{
    is_blank_hard_line_boundary, is_east_asian_sentence_terminator, is_sentence_closer,
    is_sentence_terminator, line_end,
};
use crate::document::HardLineSnapshot;

struct Item {
    end: usize,
    first: char,
    whitespace: bool,
}
struct Scan<'a> {
    lines: &'a HardLineSnapshot,
    #[cfg(test)]
    inspected: usize,
}
impl<'a> Scan<'a> {
    fn new(lines: &'a HardLineSnapshot) -> Self {
        Self {
            lines,
            #[cfg(test)]
            inspected: 0,
        }
    }
    fn item(&mut self, at: usize) -> Option<Item> {
        let end = self.lines.next_grapheme_boundary(at)?;
        let first = std::str::from_utf8(self.lines.byte_chunk_at(at))
            .ok()?
            .chars()
            .next()?;
        #[cfg(test)]
        {
            self.inspected += 1;
        }
        let whitespace = first.is_whitespace()
            && self
                .lines
                .slice_utf8(at..end)
                .ok()?
                .chars()
                .all(char::is_whitespace);
        Some(Item {
            end,
            first,
            whitespace,
        })
    }
    fn skip_whitespace(&mut self, mut at: usize) -> usize {
        while let Some(item) = self.item(at) {
            if !item.whitespace {
                break;
            }
            at = item.end;
        }
        at
    }
    fn after_closers(&mut self, mut at: usize) -> (usize, bool) {
        while let Some(item) = self.item(at) {
            if !is_sentence_closer(item.first) {
                return (at, item.whitespace);
            }
            at = item.end;
        }
        (at, true)
    }
    fn forward_sentence(&mut self, mut at: usize) -> usize {
        while let Some(item) = self.item(at) {
            if is_blank_hard_line_boundary(self.lines, at) {
                return self.skip_whitespace(item.end);
            }
            if is_sentence_terminator(item.first) {
                let (after, boundary) = self.after_closers(item.end);
                if boundary || is_east_asian_sentence_terminator(item.first) {
                    return self.skip_whitespace(after);
                }
            }
            at = item.end;
        }
        self.lines.text_length()
    }
    fn backward_sentence(&mut self, origin: usize) -> usize {
        if origin == 0 {
            return 0;
        }
        // Carry the right-hand context while scanning backwards. In particular,
        // a long whitespace/closing-punctuation run is traversed only once.
        // Destinations at or after origin cannot win. Treat that suffix as a
        // sentinel instead of inspecting arbitrarily long trailing whitespace.
        let mut next_nonwhite = origin;
        let mut boundary = false;
        let mut start_after_closers = origin;
        let mut at = origin;
        while let Some(previous) = self.lines.previous_grapheme_boundary(at) {
            at = previous;
            let item = self.item(at).expect("a preceding logical item exists");
            let next = if is_blank_hard_line_boundary(self.lines, at) {
                Some(next_nonwhite)
            } else if is_sentence_terminator(item.first)
                && (boundary || is_east_asian_sentence_terminator(item.first))
            {
                Some(start_after_closers)
            } else {
                None
            };
            if let Some(next) = next.filter(|next| *next < origin) {
                return next;
            }
            if !item.whitespace {
                next_nonwhite = at;
            }
            if !is_sentence_closer(item.first) {
                boundary = item.whitespace;
                start_after_closers = next_nonwhite;
            }
        }
        0
    }
    fn sentence(&mut self, offset: usize, forward: bool, count: usize) -> usize {
        let mut at = offset;
        for _ in 0..count.max(1) {
            let next = if forward {
                self.forward_sentence(at)
            } else {
                self.backward_sentence(at)
            };
            if next == at {
                break;
            }
            at = next;
        }
        at
    }
    fn matching_pair(&mut self, offset: usize) -> Option<usize> {
        let limit = line_end(self.lines, offset);
        let mut at = offset;
        let (opening, closing, forward) = loop {
            let item = self.item(at)?;
            let pair = match item.first {
                '(' => Some(('(', ')', true)),
                '[' => Some(('[', ']', true)),
                '{' => Some(('{', '}', true)),
                ')' => Some(('(', ')', false)),
                ']' => Some(('[', ']', false)),
                '}' => Some(('{', '}', false)),
                _ => None,
            };
            if let Some(pair) = pair {
                break pair;
            }
            at = item.end;
            if at >= limit {
                return None;
            }
        };
        let mut depth = 0usize;
        loop {
            let item = self.item(at)?;
            if item.first == if forward { opening } else { closing } {
                depth += 1;
            } else if item.first == if forward { closing } else { opening } {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            at = if forward {
                item.end
            } else {
                self.lines.previous_grapheme_boundary(at)?
            };
        }
    }
}

pub(super) fn move_sentence(
    lines: &HardLineSnapshot,
    offset: usize,
    forward: bool,
    count: usize,
) -> usize {
    Scan::new(lines).sentence(offset, forward, count)
}
pub(super) fn matching_pair(lines: &HardLineSnapshot, offset: usize) -> Option<usize> {
    Scan::new(lines).matching_pair(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, Format};
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn sentence_scans_match_reference_at_every_boundary_in_both_directions() {
        let mut cases = vec![
            "Hello. Next! (Really?) Yes… Final。次！終？".to_owned(),
            "one\n\ntwo\n\n\nthree".into(),
            "3.14 isn't.\u{301} A sentence?\u{301} End".into(),
            " \u{301}white.\n\t\nword。👩‍💻".into(),
            "x.)))    y\nx...)q\n".into(),
        ];
        let alphabet = ["a", " ", "\n", ".", "?", "！", ")", "\"", "\u{301}", "👩‍💻"];
        let mut seed = 17u32;
        for _ in 0..100 {
            cases.push(
                (0..40)
                    .map(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        alphabet[seed as usize % alphabet.len()]
                    })
                    .collect(),
            );
        }
        for text in cases {
            let document = Document::new(&text);
            let lines = document.hard_line_snapshot();
            for at in text
                .grapheme_indices(true)
                .map(|(at, _)| at)
                .chain([text.len()])
            {
                for forward in [false, true] {
                    for count in [1, 2, 4] {
                        assert_eq!(
                            move_sentence(&lines, at, forward, count),
                            super::super::text::move_sentence(&text, &lines, at, forward, count),
                            "{text:?} at={at} forward={forward} count={count}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nearby_motion_work_is_independent_of_unrelated_document_size() {
        for repeat in [10, 100_000] {
            let prefix = "unrelated line\n".repeat(repeat);
            let local = "First. (Nested [pair]). Last. ";
            let mut document = Document::from_bytes(
                format!("{prefix}{local}{}", "unrelated line\n".repeat(repeat)).into_bytes(),
                Encoding::Utf8,
                Format::Code,
            )
            .unwrap();
            document.replace(0..0, "X").unwrap();
            let start = prefix.len() + 1;
            let lines = document.hard_line_snapshot();
            let mut scan = Scan::new(&lines);
            assert_eq!(scan.sentence(start, true, 1), start + 7);
            assert_eq!(scan.sentence(start + 25, false, 1), start + 24);
            assert_eq!(scan.matching_pair(start + 7), Some(start + 21));
            assert_eq!(scan.matching_pair(start + 21), Some(start + 7));
            assert!(scan.inspected < 100, "{} items", scan.inspected);
            assert!(!document.projection().compatibility_text_is_materialized());
        }
    }
    #[test]
    fn backward_sentence_does_not_scan_whitespace_after_the_cursor() {
        let text = format!("Earlier. Current.{}Later.", " ".repeat(1_000_000));
        let document = Document::new(&text);
        let lines = document.hard_line_snapshot();
        let mut scan = Scan::new(&lines);
        assert_eq!(scan.sentence(17, false, 1), 9);
        assert!(scan.inspected < 20);
    }
}
