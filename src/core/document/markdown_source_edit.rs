//! Translate visible source-mode hard breaks through folded blank separators.
use super::{Document, DocumentError, TextEdit};

impl Document {
    pub(super) fn markdown_source_replacement(
        &self,
        edit: &TextEdit,
    ) -> Result<String, DocumentError> {
        if !edit.replacement.contains('\n') {
            return Ok(edit.replacement.clone());
        }
        // Within a code paragraph the source endings are literal content.
        if self
            .projection()
            .blocks_for_region(&edit.range)
            .iter()
            .any(|block| {
                block.style.0 == "Code Block"
                    && block.range.start <= edit.range.start
                    && edit.range.end < block.range.end
            })
        {
            return Ok(edit.replacement.clone());
        }
        let neighbor = |mut at: usize, before: bool| -> Result<(usize, usize), DocumentError> {
            let mut visible = 0;
            let mut physical = 0;
            loop {
                let boundary = if before { at.checked_sub(1) } else { Some(at) };
                let Some(boundary) =
                    boundary.filter(|&point| point < self.projection().text_tree().byte_len())
                else {
                    break;
                };
                let is_break = self
                    .projection()
                    .hard_line_at_offset(boundary)
                    .and_then(|line| self.projection().hard_line_range(line))
                    .is_some_and(|line| line.end == boundary);
                if !is_break {
                    break;
                }
                physical += self.markdown_source_separator_count(boundary..boundary + 1)?;
                visible += 1;
                at = if before { boundary } else { boundary + 1 };
            }
            Ok((visible, physical))
        };
        let text = &edit.replacement;
        let mut result = String::with_capacity(text.len());
        let mut at = 0;
        while let Some(relative) = text[at..].find('\n') {
            let start = at + relative;
            result.push_str(&text[at..start]);
            let count = text[start..]
                .bytes()
                .take_while(|&byte| byte == b'\n')
                .count();
            let end = start + count;
            let (left, left_physical) = if start == 0 {
                neighbor(edit.range.start, true)?
            } else {
                (0, 0)
            };
            let (right, right_physical) = if end == text.len() {
                neighbor(edit.range.end, false)?
            } else {
                (0, 0)
            };
            let visible = count + left + right;
            // An isolated source ending remains one hard break. Two or more
            // displayed adjacent breaks need canonical pairs; retain existing
            // neighboring source delimiters and author only the missing ones.
            let inserted = if visible == 1 {
                1
            } else {
                (2 * visible)
                    .saturating_sub(left_physical + right_physical)
                    .max(count)
            };
            result.extend(std::iter::repeat('\n').take(inserted));
            at = end;
        }
        result.push_str(&text[at..]);
        Ok(result)
    }
}
