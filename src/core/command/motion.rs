//! Cursor motion dispatch, separate from modal grammar and edit publication.
use super::*;

impl CommandInterpreter {
    pub(super) fn move_cursor(
        &mut self,
        document: &Document,
        motion: Motion,
        count: usize,
    ) -> CommandOutput {
        self.retire_typing_context();
        let text = || document.text();
        let lines = document.hard_line_snapshot();
        let old = self.cursor;
        let in_linear_visual = matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine);
        let retain_visual_line_end = in_linear_visual && self.visual_to_line_end;
        self.cursor = match motion {
            Motion::Horizontal(amount) => {
                move_horizontal(&lines, self.cursor, directional_count(count, amount > 0))
            }
            Motion::ArrowHorizontal(amount) => {
                text::move_arrow_horizontal(&lines, self.cursor, directional_count(count, amount > 0))
            }
            Motion::InsertionHorizontal(amount) => {
                let mut position = self.cursor;
                if amount > 0 {
                    for _ in 0..count {
                        let Some(next) = lines.next_grapheme_boundary(position) else {
                            break;
                        };
                        position = next;
                    }
                } else {
                    for _ in 0..count {
                        let Some(previous) = lines.previous_grapheme_boundary(position) else {
                            break;
                        };
                        position = previous;
                    }
                }
                position
            }
            Motion::Vertical(amount) => {
                let line_position = move_vertical(
                    text(),
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                );
                if retain_visual_line_end {
                    self.preferred_column = None;
                    last_grapheme_on_line(text(), &lines, line_position)
                } else {
                    let desired = self
                        .preferred_column
                        .unwrap_or_else(|| grapheme_column(text(), &lines, self.cursor));
                    self.preferred_column = Some(desired);
                    position_at_column(text(), &lines, line_start(&lines, line_position), desired)
                }
            }
            Motion::LineStart => line_start(&lines, self.cursor),
            Motion::FirstNonBlank => first_nonblank_document(document, &lines, self.cursor),
            Motion::LineEnd => {
                let mut position = self.cursor;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, position) else {
                        break;
                    };
                    position = next;
                }
                last_grapheme_on_line(text(), &lines, position)
            }
            Motion::InsertionLineEnd => line_end(&lines, self.cursor),
            Motion::WordForward(big) => {
                let target = move_word_forward(text(), self.cursor, big, count);
                if matches!(self.mode, Mode::Insert | Mode::Replace) {
                    target
                } else {
                    normalize_normal_cursor(text(), &lines, target)
                }
            }
            Motion::WordEnd(big) => move_word_end(text(), self.cursor, big, count),
            Motion::WordBackward(big) => move_word_backward(text(), self.cursor, big, count),
            Motion::WordEndBackward(big) => move_word_end_backward(text(), self.cursor, big, count),
            Motion::LastNonBlank => {
                let mut position = self.cursor;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, position) else {
                        break;
                    };
                    position = next;
                }
                last_non_blank(text(), &lines, position)
            }
            Motion::Column(one_based) => position_at_column(
                text(),
                &lines,
                line_start(&lines, self.cursor),
                one_based.saturating_sub(1),
            ),
            Motion::LineOffsetFirstNonBlank(amount) => {
                let line = move_vertical(
                    text(),
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                );
                first_non_blank(text(), &lines, line)
            }
            Motion::Sentence(forward) => normalize_normal_cursor_snapshot(
                &lines,
                move_sentence(&lines, self.cursor, forward, count),
            ),
            Motion::Paragraph(forward) => normalize_normal_cursor_snapshot(
                &lines,
                move_paragraph(&lines, self.cursor, forward, count),
            ),
        };
        if !matches!(motion, Motion::Vertical(_)) {
            self.preferred_column = None;
        }
        self.visual_to_line_end = in_linear_visual
            && (matches!(motion, Motion::LineEnd)
                || (matches!(motion, Motion::Vertical(_)) && retain_visual_line_end));
        CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        }
    }

    pub(super) fn move_cursor_as_jump(
        &mut self,
        document: &Document,
        motion: Motion,
        count: usize,
    ) -> CommandOutput {
        let origin = self.cursor;
        let output = self.move_cursor(document, motion, count);
        self.record_successful_jump(document, origin, output)
    }

    pub(super) fn goto_line(&mut self, document: &Document, one_based: usize) -> CommandOutput {
        let old = self.cursor;
        let lines = document.hard_line_snapshot();
        self.cursor = first_nonblank_document(document, &lines, nth_line_start(&lines, one_based));
        self.preferred_column = None;
        let output = CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, old, output)
    }
}
