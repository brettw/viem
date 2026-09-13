//! Constant-memory line-break detection over immutable text leaves.
use super::engine::LayoutCancellationProbe;
#[cfg(test)]
use super::jobs::LayoutCancellationToken;
use super::jobs::LayoutJobError;
use super::unicode_breaks::LineBreakState;
use crate::document::FormattedTextTree;
use std::ops::Range;

const CANCELLATION_SCAN_BYTES: usize = 4096;

/// First actual line-break boundary after the start, or the range end.
/// The start must be a hard-line start or an actual Unicode break. Resuming
/// at an emitted break is exact: the first-party rule engine tests that prior
/// context cannot change subsequent opportunities after such a boundary.
/// No string, scalar vector, or break-offset list grows with the range size.
pub(super) fn first_line_break(
    tree: &FormattedTextTree,
    range: Range<usize>,
    paragraph_flow: bool,
    cancellation: &dyn LayoutCancellationProbe,
) -> Result<usize, LayoutJobError> {
    if range.start > range.end
        || range.end > tree.byte_len()
        || !tree.is_char_boundary(range.start)?
        || !tree.is_char_boundary(range.end)?
    {
        return Err(LayoutJobError::InvalidRegion(
            "line-break scan requires an ordered UTF-8 range",
        ));
    }
    let mut state = LineBreakState::new();
    let mut at = range.start;
    let mut next_checkpoint = range.start;
    while at < range.end {
        if cancellation.is_cancelled() {
            return Err(LayoutJobError::Cancelled);
        }
        let chunk = tree.byte_chunk_at(at);
        let chunk = &chunk[..chunk.len().min(range.end - at)];
        let chunk = std::str::from_utf8(chunk).expect("scalar-aligned immutable text leaf");
        for (local, character) in chunk.char_indices() {
            let offset = at + local;
            // Check consumed input even when no opportunity is emitted; the
            // scan's cancellation bound is independent of backing chunk size.
            if offset >= next_checkpoint {
                if cancellation.is_cancelled() {
                    return Err(LayoutJobError::Cancelled);
                }
                next_checkpoint = offset.saturating_add(CANCELLATION_SCAN_BYTES);
            }
            let character = if paragraph_flow && character == '\n' {
                ' '
            } else {
                character
            };
            // Unicode line-break opportunities can occur inside an extended
            // grapheme (for example after a space followed by a combining
            // mark). A captured/streamed row must end at a legal text boundary.
            if state.push(character).is_some() && tree.is_grapheme_boundary(offset)? {
                return Ok(at + local);
            }
        }
        at += chunk.len();
    }
    if cancellation.is_cancelled() {
        return Err(LayoutJobError::Cancelled);
    }
    Ok(range.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn streamed_breaks_do_not_split_a_space_with_combining_marks() {
        let tree = FormattedTextTree::try_from_text(" \u{301}x next").unwrap();
        assert_eq!(
            first_line_break(&tree, 0..tree.byte_len(), false, &LayoutCancellationToken::default()).unwrap(),
            5,
        );
    }

    #[test]
    fn giant_words_inside_paragraphs_scan_without_flat_text_or_break_vectors() {
        let length = 2 * 1024 * 1024;
        let text = format!("prefix {} tail", "a".repeat(length));
        let document = Document::new(text);
        let tree = document.projection().text_tree();
        let cancellation = LayoutCancellationToken::default();
        assert_eq!(
            first_line_break(tree, 0..tree.byte_len(), false, &cancellation).unwrap(),
            7
        );
        assert_eq!(
            first_line_break(tree, 7..tree.byte_len(), false, &cancellation).unwrap(),
            8 + length
        );
        assert_eq!(
            first_line_break(tree, 8 + length..tree.byte_len(), false, &cancellation).unwrap(),
            tree.byte_len()
        );
        assert!(!document.projection().compatibility_text_is_materialized());
        cancellation.cancel();
        assert!(matches!(
            first_line_break(tree, 0..tree.byte_len(), false, &cancellation),
            Err(LayoutJobError::Cancelled)
        ));
    }

    #[test]
    fn giant_context_runs_and_flow_have_exact_breaks() {
        for (text, expected) in [
            (format!("a{}b tail", "\u{301}".repeat(80_000)), 160_003),
            (format!("({}x tail", " ".repeat(160_000)), 160_003),
            (format!("{}z", "🇺".repeat(10_001)), 8),
        ] {
            let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
            assert_eq!(
                first_line_break(
                    &tree,
                    0..text.len(),
                    false,
                    &LayoutCancellationToken::default()
                )
                .unwrap(),
                expected
            );
        }
        let text = "abc\ndef";
        let tree = FormattedTextTree::try_from_text(text).unwrap();
        assert_eq!(
            first_line_break(
                &tree,
                0..text.len(),
                true,
                &LayoutCancellationToken::default()
            )
            .unwrap(),
            4
        );
    }

    #[test]
    fn cancellation_interrupts_a_giant_unbreakable_combining_sequence() {
        struct CancelAfterThreeChecks(std::cell::Cell<usize>);
        impl LayoutCancellationProbe for CancelAfterThreeChecks {
            fn is_cancelled(&self) -> bool {
                let count = self.0.get();
                self.0.set(count + 1);
                count >= 3
            }
        }
        let text = format!("a{}", "\u{301}".repeat(100_000));
        let tree = FormattedTextTree::try_from_text(text.as_str()).unwrap();
        assert_eq!(tree.byte_len(), 200_001);
        let cancellation = CancelAfterThreeChecks(std::cell::Cell::new(0));
        assert!(matches!(
            first_line_break(&tree, 0..tree.byte_len(), false, &cancellation),
            Err(LayoutJobError::Cancelled)
        ));
        assert_eq!(cancellation.0.get(), 4);
    }
}
