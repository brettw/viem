//! Cursor motion dispatch, separate from modal grammar and edit publication.
use super::*;

impl CommandInterpreter {
    /// Home's hard-line motion; see `smart_home_reaches_line_start`.
    pub(super) fn home_motion(&self, document: &Document) -> Motion {
        if self.home_reaches_hard_line_start(document) { Motion::LineStart } else { Motion::FirstNonBlank }
    }

    pub(super) fn home_reaches_hard_line_start(&self, document: &Document) -> bool {
        if !self.smart_home() {
            return false;
        }
        let lines = document.hard_line_snapshot();
        let mut first = first_nonblank_document(document, &lines, self.cursor);
        if !matches!(self.mode, Mode::Insert | Mode::Replace) {
            first = normalize_normal_cursor_document(document, &lines, first);
        }
        first == self.cursor
    }

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
        let last_addressable_line_start = if matches!(self.mode, Mode::Insert | Mode::Replace) {
            None
        } else {
            let count = document.addressable_hard_line_count();
            (count < lines.line_count()).then(|| nth_line_start(&lines, count))
        };
        // Resolve the destination line before applying its column semantics. Clamping
        // the final cursor instead would turn a downward motion into a line-end motion.
        let addressable_line_position = |position| {
            if position == lines.text_length() {
                last_addressable_line_start.unwrap_or(position)
            } else {
                position
            }
        };
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
                let line_position = addressable_line_position(move_vertical(
                    text(),
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                ));
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
                last_grapheme_on_line(text(), &lines, addressable_line_position(position))
            }
            Motion::InsertionLineEnd => line_end(&lines, self.cursor),
            Motion::WordForward(big) => {
                let target = word_snapshot::forward_with_keywords(&lines, self.cursor, big, count, self.vim_options.keywords());
                if matches!(self.mode, Mode::Insert | Mode::Replace) {
                    target
                } else {
                    normalize_normal_cursor_snapshot(&lines, target)
                }
            }
            Motion::WordEnd(big) => word_snapshot::end_with_keywords(&lines, self.cursor, big, count, false, self.vim_options.keywords()),
            Motion::WordBackward(big) => word_snapshot::backward_with_keywords(&lines, self.cursor, big, count, self.vim_options.keywords()),
            Motion::WordEndBackward(big) => word_snapshot::end_backward_with_keywords(&lines, self.cursor, big, count, self.vim_options.keywords()),
            Motion::LastNonBlank => {
                let mut position = self.cursor;
                for _ in 1..count {
                    let Some(next) = next_line_start(&lines, position) else {
                        break;
                    };
                    position = next;
                }
                last_non_blank(text(), &lines, addressable_line_position(position))
            }
            Motion::Column(one_based) => position_at_column(
                text(),
                &lines,
                line_start(&lines, self.cursor),
                one_based.saturating_sub(1),
            ),
            Motion::LineOffsetFirstNonBlank(amount) => {
                let line = addressable_line_position(move_vertical(
                    text(),
                    &lines,
                    self.cursor,
                    directional_count(count, amount > 0),
                ));
                first_non_blank(text(), &lines, line)
            }
            Motion::Sentence(forward) => normalize_normal_cursor_snapshot(
                &lines,
                move_sentence(&lines, self.cursor, forward, count),
            ),
            Motion::Paragraph(forward) => normalize_normal_cursor_snapshot(
                &lines,
                text::move_document_paragraph(document, self.cursor, forward, count),
            ),
        };
        if !matches!(self.mode, Mode::Insert | Mode::Replace) {
            self.cursor = normalize_normal_cursor_document(document, &lines, self.cursor);
        }
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
        let target = nth_line_start(&lines, one_based.min(document.addressable_hard_line_count()));
        self.cursor = if self.vim_options.flag("startofline") { first_nonblank_document(document, &lines, target) }
            else { options::at_logical_column(&lines, target, options::logical_column(&lines, old)) };
        self.preferred_column = None;
        let output = CommandOutput {
            cursor_moved: old != self.cursor,
            ..CommandOutput::complete()
        };
        self.record_successful_jump(document, old, output)
    }
}
