//! Grapheme-safe logical motions and deletion extents for Insert/Replace mode.
//!
//! The document projection supplies the current hard-line content range.
//! U+000A and carriage return scalars inside that range are ordinary content;
//! the source adapter, not this module, decides which delimiters are breaks.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use crate::document::HardLineSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InsertMotionError {
    OutOfBounds { offset: usize, length: usize },
    NotGraphemeBoundary(usize),
    InvertedSessionFloor { floor: usize, caret: usize },
}

fn validate_snapshot_boundary(snapshot: &HardLineSnapshot, offset: usize) -> Result<(), InsertMotionError> {
    if offset > snapshot.text_length() {
        return Err(InsertMotionError::OutOfBounds { offset, length: snapshot.text_length() });
    }
    if !snapshot.is_grapheme_boundary(offset) { return Err(InsertMotionError::NotGraphemeBoundary(offset)); }
    Ok(())
}

/// Inspect only the preceding whitespace/word through bounded UTF-8 chunks.
/// A single unusually large grapheme may require a larger local chunk.
pub(super) fn ctrl_w_delete_range_snapshot(
    snapshot: &HardLineSnapshot, line: Range<usize>, caret: usize, keywords: &super::options::KeywordSet,
) -> Result<Range<usize>, InsertMotionError> {
    validate_snapshot_boundary(snapshot, caret)?;
    let mut at = caret;
    let mut word_class = None;
    while at > line.start {
        let mut start = at.saturating_sub(4096).max(line.start);
        while start > line.start && snapshot.byte_chunk_at(start).first().is_some_and(|byte| byte & 0xc0 == 0x80) {
            start -= 1;
        }
        if !snapshot.is_grapheme_boundary(start) {
            start = snapshot.previous_grapheme_boundary(start).unwrap_or(line.start).max(line.start);
        }
        let text = snapshot.slice_utf8(start..at).map_err(|_| InsertMotionError::NotGraphemeBoundary(start))?;
        for (relative, grapheme) in text.grapheme_indices(true).rev() {
            let class = if grapheme.chars().all(char::is_whitespace) { GraphemeClass::Space }
                else if grapheme.chars().next().is_some_and(|c| keywords.contains(c)) { GraphemeClass::Keyword }
                else { GraphemeClass::Punctuation };
            if word_class.is_some_and(|word| word != class) { return Ok(at..caret); }
            if class != GraphemeClass::Space { word_class = Some(class); }
            at = start + relative;
        }
    }
    Ok(at..caret)
}

pub(super) fn ctrl_u_delete_range_since_snapshot(
    snapshot: &HardLineSnapshot, line: Range<usize>, caret: usize, session_floor: usize,
) -> Result<Range<usize>, InsertMotionError> {
    validate_snapshot_boundary(snapshot, caret)?;
    validate_snapshot_boundary(snapshot, session_floor)?;
    if session_floor > caret { return Err(InsertMotionError::InvertedSessionFloor { floor: session_floor, caret }); }
    Ok(line.start.max(session_floor)..caret)
}

/// `Ctrl-W` within one flat string, such as a deferred block-insert payload:
/// delete intervening whitespace and one word or punctuation run.
pub(super) fn ctrl_w_delete_range_with_keywords(
    text: &str, line: Range<usize>, caret: usize, keywords: &super::options::KeywordSet,
) -> Result<Range<usize>, InsertMotionError> {
    validate_boundary(text, caret)?;
    let mut at = caret;
    let mut word_class = None;
    for (relative, grapheme) in text[line.start..caret].grapheme_indices(true).rev() {
        let class = if grapheme.chars().all(char::is_whitespace) { GraphemeClass::Space }
            else if grapheme.chars().next().is_some_and(|c| keywords.contains(c)) { GraphemeClass::Keyword }
            else { GraphemeClass::Punctuation };
        if word_class.is_some_and(|word| word != class) { break; }
        if class != GraphemeClass::Space { word_class = Some(class); }
        at = line.start + relative;
    }
    Ok(at..caret)
}

fn validate_boundary(text: &str, offset: usize) -> Result<(), InsertMotionError> {
    if offset > text.len() {
        return Err(InsertMotionError::OutOfBounds {
            offset,
            length: text.len(),
        });
    }
    if offset != text.len()
        && !text
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
    {
        return Err(InsertMotionError::NotGraphemeBoundary(offset));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GraphemeClass {
    Space,
    Keyword,
    Punctuation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::options::KeywordSet;
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn ctrl_w_in(document: &Document, caret: usize) -> Result<Range<usize>, InsertMotionError> {
        let snapshot = document.hard_line_snapshot();
        let line = snapshot.line_at_offset(caret.min(snapshot.text_length())).unwrap().content_range();
        ctrl_w_delete_range_snapshot(&snapshot, line, caret, &KeywordSet::default())
    }

    fn ctrl_u_in(document: &Document, caret: usize, floor: usize) -> Result<Range<usize>, InsertMotionError> {
        let snapshot = document.hard_line_snapshot();
        let line = snapshot.line_at_offset(caret.min(snapshot.text_length())).unwrap().content_range();
        ctrl_u_delete_range_since_snapshot(&snapshot, line, caret, floor)
    }

    fn ctrl_w(text: &str, caret: usize) -> Result<Range<usize>, InsertMotionError> {
        ctrl_w_in(&Document::new(text), caret)
    }

    fn ctrl_u(text: &str, caret: usize, floor: usize) -> Result<Range<usize>, InsertMotionError> {
        ctrl_u_in(&Document::new(text), caret, floor)
    }

    #[test]
    fn snapshot_word_deletion_matches_the_flat_string_scan_across_chunk_boundaries() {
        for text in ["one e\u{301} 🇺🇸👩🏽‍💻 !!  ".repeat(300),
            format!("{}{}{}", "a".repeat(9000), " ".repeat(5000), "β_ζ".repeat(3000))] {
            let document = Document::new(&text);
            let snapshot = document.hard_line_snapshot();
            for caret in text.grapheme_indices(true).map(|(at, _)| at)
                .step_by(97).chain(std::iter::once(text.len())) {
                assert_eq!(ctrl_w_delete_range_snapshot(&snapshot, 0..text.len(), caret, &KeywordSet::default()),
                    ctrl_w_delete_range_with_keywords(&text, 0..text.len(), caret, &KeywordSet::default()));
                assert_eq!(ctrl_u_delete_range_since_snapshot(&snapshot, 0..text.len(), caret, 0), Ok(0..caret));
            }
            assert!(!document.projection().compatibility_text_is_materialized());
        }
    }

    #[test]
    fn ctrl_w_deletes_a_word_and_intervening_whitespace() {
        let text = "one two   ";
        let range = ctrl_w(text, text.len()).unwrap();
        assert_eq!(&text[range], "two   ");
    }

    #[test]
    fn ctrl_w_uses_separate_keyword_and_punctuation_runs() {
        let text = "word...";
        let punctuation = ctrl_w(text, text.len()).unwrap();
        assert_eq!(&text[punctuation.clone()], "...");
        let word = ctrl_w(text, punctuation.start).unwrap();
        assert_eq!(&text[word], "word");
    }

    #[test]
    fn ctrl_w_word_scan_stops_at_the_supplied_hard_line() {
        let text = "previous\n   ";
        let range = ctrl_w(text, text.len()).unwrap();
        assert_eq!(&text[range], "   ");
        assert_eq!(ctrl_w(text, 9).unwrap(), 9..9);
    }

    #[test]
    fn ctrl_w_is_unicode_grapheme_safe() {
        let text = "a\u{301}bc 😀";
        let emoji = ctrl_w(text, text.len()).unwrap();
        assert_eq!(&text[emoji], "😀");
        let word_end = text.find(' ').unwrap();
        assert_eq!(&text[ctrl_w(text, word_end).unwrap()], "a\u{301}bc");
    }

    #[test]
    fn ctrl_u_deletes_to_current_hard_line_start() {
        let text = "keep\ndelete this";
        assert_eq!(&text[ctrl_u(text, text.len(), 0).unwrap()], "delete this");
    }

    #[test]
    fn ctrl_u_session_floor_clips_only_on_current_line() {
        let text = "old inserted";
        assert_eq!(ctrl_u(text, text.len(), 4).unwrap(), 4..text.len());
        let text = "floor\nnew";
        assert_eq!(ctrl_u(text, text.len(), 0).unwrap(), 6..text.len());
    }

    #[test]
    fn empty_lines_produce_empty_deletion_ranges() {
        let text = "one\n\ntwo";
        let empty = text.find("\n\n").unwrap() + 1;
        assert_eq!(ctrl_w(text, empty).unwrap(), empty..empty);
        assert_eq!(ctrl_u(text, empty, 0).unwrap(), empty..empty);
    }

    #[test]
    fn mac_hard_line_keeps_literal_lf_inside_insert_extents() {
        let document = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(ctrl_u_in(&document, 5, 0).unwrap(), 2..5);
        assert_eq!(
            ctrl_w_in(&document, 5).unwrap(),
            4..5,
            "literal LF is whitespace within the line, not a stopping boundary"
        );
    }

    #[test]
    fn carriage_return_is_content_not_a_hard_line() {
        let text = "left\rright";
        assert_eq!(ctrl_u(text, text.len(), 0).unwrap(), 0..text.len());
    }

    #[test]
    fn invalid_boundaries_and_floors_are_structured_errors() {
        let text = "a\u{301}b";
        assert_eq!(ctrl_w(text, 1), Err(InsertMotionError::NotGraphemeBoundary(1)));
        assert_eq!(
            ctrl_w(text, 99),
            Err(InsertMotionError::OutOfBounds { offset: 99, length: text.len() })
        );
        assert_eq!(
            ctrl_u("abc", 1, 2),
            Err(InsertMotionError::InvertedSessionFloor { floor: 2, caret: 1 })
        );
    }
}
