//! Grapheme-safe logical motions and deletion extents for Insert/Replace mode.
//!
//! The document projection supplies the current hard-line content range.
//! U+000A and carriage return scalars inside that range are ordinary content;
//! the source adapter, not this module, decides which delimiters are breaks.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InsertMotionError {
    OutOfBounds { offset: usize, length: usize },
    NotGraphemeBoundary(usize),
    InvertedSessionFloor { floor: usize, caret: usize },
}

/// Vim Insert-mode `Ctrl-W`: delete intervening whitespace and one preceding
/// word/punctuation run, stopping at the supplied hard-line boundary.
pub fn ctrl_w_delete_range(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<Range<usize>, InsertMotionError> {
    validate_boundary(text, caret)?;
    Ok(previous_word_boundary_unchecked(text, line.start, caret)..caret)
}

/// Vim Insert-mode `Ctrl-U` logical extent when no Insert-session floor is
/// available. It never consumes the preceding projected hard-line boundary.
pub fn ctrl_u_delete_range(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<Range<usize>, InsertMotionError> {
    validate_boundary(text, caret)?;
    Ok(line.start..caret)
}

/// `Ctrl-U` extent clipped to the beginning of the current Insert session.
/// A floor on another line has no effect; a floor after the caret is an error.
pub fn ctrl_u_delete_range_since(
    text: &str,
    line: Range<usize>,
    caret: usize,
    session_floor: usize,
) -> Result<Range<usize>, InsertMotionError> {
    validate_boundary(text, caret)?;
    validate_boundary(text, session_floor)?;
    if session_floor > caret {
        return Err(InsertMotionError::InvertedSessionFloor {
            floor: session_floor,
            caret,
        });
    }
    let start = if session_floor >= line.start {
        session_floor
    } else {
        line.start
    };
    Ok(start..caret)
}

pub fn insert_home(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<usize, InsertMotionError> {
    validate_boundary(text, caret)?;
    Ok(line.start)
}

pub fn insert_end(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<usize, InsertMotionError> {
    validate_boundary(text, caret)?;
    Ok(line.end)
}

/// Start of the preceding Vim-style small word. Whitespace immediately before
/// the caret is traversed first; the result never crosses a hard line.
pub fn previous_word_boundary(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<usize, InsertMotionError> {
    validate_boundary(text, caret)?;
    Ok(previous_word_boundary_unchecked(text, line.start, caret))
}

/// Start of the next small word/punctuation run on the current hard line.
/// From whitespace this skips to the next non-whitespace run; from a run it
/// consumes that class and following whitespace.
pub fn next_word_boundary(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<usize, InsertMotionError> {
    validate_boundary(text, caret)?;
    let line_end = line.end;
    if caret == line_end {
        return Ok(caret);
    }
    let graphemes = classified_graphemes(text, caret..line_end);
    let Some(first) = graphemes.first() else {
        return Ok(caret);
    };
    let mut index = 0;
    if first.class != GraphemeClass::Space {
        let class = first.class;
        while index < graphemes.len() && graphemes[index].class == class {
            index += 1;
        }
    }
    while index < graphemes.len() && graphemes[index].class == GraphemeClass::Space {
        index += 1;
    }
    Ok(graphemes
        .get(index)
        .map(|grapheme| grapheme.range.start)
        .unwrap_or(line_end))
}

/// End of the current (or next, when starting on whitespace) small word run.
pub fn word_end_boundary(
    text: &str,
    line: Range<usize>,
    caret: usize,
) -> Result<usize, InsertMotionError> {
    validate_boundary(text, caret)?;
    let line_end = line.end;
    let graphemes = classified_graphemes(text, caret..line_end);
    let mut index = 0;
    while index < graphemes.len() && graphemes[index].class == GraphemeClass::Space {
        index += 1;
    }
    let Some(first) = graphemes.get(index) else {
        return Ok(line_end);
    };
    let class = first.class;
    let mut end = first.range.end;
    while index < graphemes.len() && graphemes[index].class == class {
        end = graphemes[index].range.end;
        index += 1;
    }
    Ok(end)
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

fn previous_word_boundary_unchecked(text: &str, line_start: usize, caret: usize) -> usize {
    if caret == line_start {
        return caret;
    }
    let graphemes = classified_graphemes(text, line_start..caret);
    let mut index = graphemes.len();
    while index > 0 && graphemes[index - 1].class == GraphemeClass::Space {
        index -= 1;
    }
    if index == 0 {
        return line_start;
    }
    let class = graphemes[index - 1].class;
    while index > 0 && graphemes[index - 1].class == class {
        index -= 1;
    }
    graphemes
        .get(index)
        .map(|grapheme| grapheme.range.start)
        .unwrap_or(line_start)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GraphemeClass {
    Space,
    Keyword,
    Punctuation,
}

#[derive(Clone, Debug)]
struct ClassifiedGrapheme {
    range: Range<usize>,
    class: GraphemeClass,
}

fn classified_graphemes(text: &str, range: Range<usize>) -> Vec<ClassifiedGrapheme> {
    text[range.clone()]
        .grapheme_indices(true)
        .map(|(relative, grapheme)| {
            let start = range.start + relative;
            let first = grapheme.chars().next().expect("nonempty grapheme");
            let class = if grapheme.chars().all(char::is_whitespace) {
                GraphemeClass::Space
            } else if first.is_alphanumeric() || first == '_' {
                GraphemeClass::Keyword
            } else {
                GraphemeClass::Punctuation
            };
            ClassifiedGrapheme {
                range: start..start + grapheme.len(),
                class,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn hard_line(text: &str, caret: usize) -> Range<usize> {
        let document = Document::new(text);
        document
            .hard_line_snapshot()
            .line_at_offset(caret.min(text.len()))
            .unwrap()
            .content_range()
    }

    fn ctrl_w_delete_range(text: &str, caret: usize) -> Result<Range<usize>, InsertMotionError> {
        super::ctrl_w_delete_range(text, hard_line(text, caret), caret)
    }

    fn ctrl_u_delete_range(text: &str, caret: usize) -> Result<Range<usize>, InsertMotionError> {
        super::ctrl_u_delete_range(text, hard_line(text, caret), caret)
    }

    fn ctrl_u_delete_range_since(
        text: &str,
        caret: usize,
        floor: usize,
    ) -> Result<Range<usize>, InsertMotionError> {
        super::ctrl_u_delete_range_since(text, hard_line(text, caret), caret, floor)
    }

    fn insert_home(text: &str, caret: usize) -> Result<usize, InsertMotionError> {
        super::insert_home(text, hard_line(text, caret), caret)
    }

    fn insert_end(text: &str, caret: usize) -> Result<usize, InsertMotionError> {
        super::insert_end(text, hard_line(text, caret), caret)
    }

    fn previous_word_boundary(text: &str, caret: usize) -> Result<usize, InsertMotionError> {
        super::previous_word_boundary(text, hard_line(text, caret), caret)
    }

    fn next_word_boundary(text: &str, caret: usize) -> Result<usize, InsertMotionError> {
        super::next_word_boundary(text, hard_line(text, caret), caret)
    }

    fn word_end_boundary(text: &str, caret: usize) -> Result<usize, InsertMotionError> {
        super::word_end_boundary(text, hard_line(text, caret), caret)
    }

    #[test]
    fn ctrl_w_deletes_a_word_and_intervening_whitespace() {
        let text = "one two   ";
        let range = ctrl_w_delete_range(text, text.len()).unwrap();
        assert_eq!(&text[range], "two   ");
    }

    #[test]
    fn ctrl_w_uses_separate_keyword_and_punctuation_runs() {
        let text = "word...";
        let punctuation = ctrl_w_delete_range(text, text.len()).unwrap();
        assert_eq!(&text[punctuation.clone()], "...");
        let word = ctrl_w_delete_range(text, punctuation.start).unwrap();
        assert_eq!(&text[word], "word");
    }

    #[test]
    fn ctrl_w_never_crosses_a_hard_line() {
        let text = "previous\n   ";
        let range = ctrl_w_delete_range(text, text.len()).unwrap();
        assert_eq!(&text[range], "   ");
        assert_eq!(ctrl_w_delete_range(text, 9).unwrap(), 9..9);
    }

    #[test]
    fn ctrl_w_is_unicode_grapheme_safe() {
        let text = "a\u{301}bc 😀";
        let emoji = ctrl_w_delete_range(text, text.len()).unwrap();
        assert_eq!(&text[emoji], "😀");
        let word_end = text.find(' ').unwrap();
        assert_eq!(
            &text[ctrl_w_delete_range(text, word_end).unwrap()],
            "a\u{301}bc"
        );
    }

    #[test]
    fn ctrl_u_deletes_to_current_hard_line_start() {
        let text = "keep\ndelete this";
        assert_eq!(
            &text[ctrl_u_delete_range(text, text.len()).unwrap()],
            "delete this"
        );
    }

    #[test]
    fn ctrl_u_session_floor_clips_only_on_current_line() {
        let text = "old inserted";
        assert_eq!(
            ctrl_u_delete_range_since(text, text.len(), 4).unwrap(),
            4..text.len()
        );
        let text = "floor\nnew";
        assert_eq!(
            ctrl_u_delete_range_since(text, text.len(), 0).unwrap(),
            6..text.len()
        );
    }

    #[test]
    fn empty_lines_produce_empty_deletion_ranges() {
        let text = "one\n\ntwo";
        let empty = text.find("\n\n").unwrap() + 1;
        assert_eq!(ctrl_w_delete_range(text, empty).unwrap(), empty..empty);
        assert_eq!(ctrl_u_delete_range(text, empty).unwrap(), empty..empty);
        assert_eq!(insert_home(text, empty).unwrap(), empty);
        assert_eq!(insert_end(text, empty).unwrap(), empty);
    }

    #[test]
    fn home_and_end_use_hard_lines() {
        let text = "first\nsecond\nthird";
        let caret = text.find("cond").unwrap();
        assert_eq!(insert_home(text, caret).unwrap(), 6);
        assert_eq!(insert_end(text, caret).unwrap(), 12);
    }

    #[test]
    fn supplied_mac_hard_line_keeps_literal_lf_inside_insert_extents() {
        let document = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let text = document.text();
        let line = document
            .hard_line_snapshot()
            .line_at_offset(4)
            .unwrap()
            .content_range();
        assert_eq!(super::insert_home(text, line.clone(), 4).unwrap(), 2);
        assert_eq!(super::insert_end(text, line.clone(), 2).unwrap(), 5);
        assert_eq!(
            super::ctrl_u_delete_range(text, line.clone(), 5).unwrap(),
            2..5
        );
        assert_eq!(
            super::ctrl_w_delete_range(text, line, 5).unwrap(),
            4..5,
            "literal LF is whitespace within the line, not a stopping boundary"
        );
    }

    #[test]
    fn carriage_return_is_content_not_a_hard_line() {
        let text = "left\rright";
        assert_eq!(insert_home(text, text.len()).unwrap(), 0);
        assert_eq!(insert_end(text, 0).unwrap(), text.len());
        assert_eq!(
            ctrl_u_delete_range(text, text.len()).unwrap(),
            0..text.len()
        );
    }

    #[test]
    fn previous_and_next_word_boundaries_observe_classes() {
        let text = "alpha...  beta";
        assert_eq!(previous_word_boundary(text, text.len()).unwrap(), 10);
        assert_eq!(next_word_boundary(text, 0).unwrap(), 5);
        assert_eq!(next_word_boundary(text, 5).unwrap(), 10);
        assert_eq!(next_word_boundary(text, 8).unwrap(), 10);
        assert_eq!(next_word_boundary(text, text.len()).unwrap(), text.len());
    }

    #[test]
    fn word_end_skips_whitespace_then_consumes_one_class() {
        let text = "  café!!!";
        assert_eq!(word_end_boundary(text, 0).unwrap(), "  café".len());
        assert_eq!(word_end_boundary(text, "  café".len()).unwrap(), text.len());
    }

    #[test]
    fn invalid_boundaries_and_floors_are_structured_errors() {
        let text = "a\u{301}b";
        assert_eq!(
            ctrl_w_delete_range(text, 1),
            Err(InsertMotionError::NotGraphemeBoundary(1))
        );
        assert_eq!(
            insert_end(text, 99),
            Err(InsertMotionError::OutOfBounds {
                offset: 99,
                length: text.len(),
            })
        );
        assert_eq!(
            ctrl_u_delete_range_since("abc", 1, 2),
            Err(InsertMotionError::InvertedSessionFloor { floor: 2, caret: 1 })
        );
    }
}
