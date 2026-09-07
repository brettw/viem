//! Paragraph direction from logical text, independent of resolved run levels.
//!
//! Digits in a right-to-left paragraph commonly have an even embedding level.
//! That level describes their run, not the paragraph's leading edge. UAX #9
//! P2/P3 instead uses the first strong character outside directional isolates.

use super::ParagraphLayoutStyle;
use crate::document::WritingDirection;
use unicode_bidi::{get_base_direction, Direction};

/// Inspect only the context already captured for this layout operation. A
/// resumed long-line slice uses its existing checkpoint direction instead;
/// this helper never retrieves or scans an uncaptured document prefix.
pub(super) fn paragraph_is_right_to_left(
    paragraph: &ParagraphLayoutStyle,
    context: &str,
    context_origin: usize,
) -> bool {
    match paragraph.base_direction {
        WritingDirection::LeftToRight => false,
        WritingDirection::RightToLeft => true,
        WritingDirection::Natural => {
            let body_start = paragraph
                .list_marker_range
                .as_ref()
                .map_or(0, |marker| marker.end.saturating_sub(context_origin));
            // Generated label text is paragraph furniture, not prose context.
            // A neutral captured prefix follows P3's left-to-right fallback.
            context
                .get(body_start..)
                .is_some_and(|body| get_base_direction(body) == Direction::Rtl)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::layout::DocumentLayoutStyles;

    fn paragraph() -> ParagraphLayoutStyle {
        DocumentLayoutStyles::resolve(Document::new("text").projection())
            .unwrap()
            .paragraphs
            .remove(0)
    }

    #[test]
    fn natural_direction_uses_first_strong_not_numeric_or_neutral_prefix() {
        let paragraph = paragraph();
        for (text, rtl) in [
            ("2026 שלום עולם", true),
            ("١٢٣ مرحبا", true),
            ("  !2026 مرحبا hello", true),
            ("2026 hello שלום", false),
            ("2026 ...", false),
            ("\u{200f}2026 hello", true),
            ("\u{200e}2026 שלום", false),
            ("\u{061c}2026 hello", true),
        ] {
            assert_eq!(
                paragraph_is_right_to_left(&paragraph, text, 0),
                rtl,
                "{text}"
            );
        }
    }

    #[test]
    fn directional_isolates_do_not_choose_the_surrounding_paragraph_direction() {
        let paragraph = paragraph();
        for (text, rtl) in [
            ("\u{2066}English\u{2069} 2026 שלום", true),
            ("\u{2067}שלום\u{2069} English", false),
            ("\u{2068}שלום\u{2069} English", false),
            ("\u{2066}one\u{2067}שלום\u{2069}two\u{2069} مرحبا", true),
            ("\u{2067}שלום", false),
        ] {
            assert_eq!(
                paragraph_is_right_to_left(&paragraph, text, 0),
                rtl,
                "{text}"
            );
        }
    }

    #[test]
    fn list_labels_and_explicit_directions_are_independent() {
        let mut paragraph = paragraph();
        paragraph.list_marker_range = Some(100..103);
        assert!(paragraph_is_right_to_left(&paragraph, "A. 2026 שלום", 100));
        assert!(paragraph_is_right_to_left(&paragraph, "2026 שלום", 103));
        assert!(!paragraph_is_right_to_left(&paragraph, "A.", 100));
        paragraph.base_direction = WritingDirection::LeftToRight;
        assert!(!paragraph_is_right_to_left(&paragraph, "שלום", 103));
        paragraph.base_direction = WritingDirection::RightToLeft;
        assert!(paragraph_is_right_to_left(&paragraph, "English", 103));
    }
}
