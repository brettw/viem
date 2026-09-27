//! Translate visible source-mode hard breaks through folded blank separators.
use super::{Document, DocumentError, TextEdit};

impl Document {
    pub(super) fn markdown_source_code_enter(&self, at: usize) -> Result<bool, DocumentError> {
        if self.format() != super::Format::MarkdownSource {
            return Ok(false);
        }
        let projection = self.projection();
        let line_at = |offset| {
            projection
                .hard_line_at_offset(offset)
                .and_then(|line| projection.hard_line_range(line))
                .ok_or(DocumentError::VerificationFailed)
        };
        let line = line_at(at)?;
        let text = projection
            .text_tree()
            .slice(line.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        for block in projection.blocks_for_region(&(at..at)) {
            if !super::edit_boundary::is_code_paragraph(self, &block)?
                || at < block.range.start || at > block.range.end {
                continue;
            }
            if at < block.range.end {
                return Ok(true);
            }
            let opening = line_at(block.range.start)?;
            let opener = projection
                .text_tree()
                .slice(opening.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let prefix = super::markdown_quotes::prefix;
            let closed = opening.start < line.start
                && super::projection::markdown_fence(&opener[prefix(&opener)..]).is_some_and(|(delimiter, length)| {
                    let tail = text[prefix(&text)..].trim();
                    tail.len() >= length && tail.bytes().all(|byte| byte == delimiter)
                });
            return Ok(!closed);
        }
        let indented = |line: &std::ops::Range<usize>, text: &str| {
            (text.starts_with("    ") || text.starts_with('\t'))
                && !projection
                    .blocks_for_region(line)
                    .iter()
                    .any(|block| matches!(block.kind, super::BlockKind::ListItem { .. }))
        };
        if indented(&line, &text) {
            return Ok(true);
        }
        // A first empty row after indented code still owns a literal ending.
        // Later empty prose rows retain the ordinary paragraph promotion rule.
        if line.is_empty() && line.start > 0 {
            let previous = line_at(line.start - 1)?;
            let text = projection
                .text_tree()
                .slice(previous.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            return Ok(indented(&previous, &text));
        }
        Ok(false)
    }

    /// The displayed hard breaks adjacent to `at`, walking backward when
    /// `before` is set, as (visible break count, physical source ending
    /// count). A displayed break in Markdown Source may fold one or more
    /// physical endings, so authoring a new boundary must account for the
    /// endings its neighbors already own.
    pub(super) fn markdown_source_break_neighbors(
        &self,
        mut at: usize,
        before: bool,
    ) -> Result<(usize, usize), DocumentError> {
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
    }

    /// The physical source endings Return authors at `at` so the new visible
    /// paragraph boundary and every adjacent displayed break each own a
    /// canonical pair, retaining the endings the neighbors already have.
    pub(super) fn markdown_source_enter_endings(&self, at: usize) -> Result<usize, DocumentError> {
        let (left, left_physical) = self.markdown_source_break_neighbors(at, true)?;
        let (right, right_physical) = self.markdown_source_break_neighbors(at, false)?;
        let visible = 1 + left + right;
        Ok((2 * visible).saturating_sub(left_physical + right_physical).max(1))
    }

    pub(super) fn markdown_source_replacement(
        &self,
        edit: &TextEdit,
    ) -> Result<String, DocumentError> {
        if !edit.replacement.contains('\n') {
            return Ok(edit.replacement.clone());
        }
        // Within a code paragraph the source endings are literal content.
        if edit.range.is_empty() && self.markdown_source_code_enter(edit.range.start)?
            || self
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
        let neighbor = |at, before| self.markdown_source_break_neighbors(at, before);
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

#[cfg(test)]
mod tests {
    use crate::document::{Document, Encoding, Format, ModelRequest};

    fn continue_list(document: &mut Document, at: usize) {
        document
            .apply_model_request(ModelRequest::ContinueList {
                document: document.id(),
                revision: document.revision(),
                at,
            })
            .unwrap();
    }

    fn source(document: &Document) -> String {
        String::from_utf8(document.source_bytes()).unwrap()
    }

    #[test]
    fn return_before_a_continuation_line_authors_the_endings_its_neighbor_lacks() {
        let bytes = b"first line of prose\nsecond line of prose\nthird line of prose\nfourth line\n";
        let mut document =
            Document::from_bytes(bytes.to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
        let at = "first line of prose\nsecond line of prose".len();
        continue_list(&mut document, at);
        assert_eq!(
            document.text(),
            "first line of prose\nsecond line of prose\n\nthird line of prose\nfourth line\n"
        );
        // The continuation ending already owned one ending; the new boundary
        // and that neighbor each need a canonical pair.
        assert_eq!(
            source(&document),
            "first line of prose\nsecond line of prose\n\n\n\nthird line of prose\nfourth line\n"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
    }

    #[test]
    fn return_at_a_paragraph_end_and_mid_line_keep_authoring_one_pair() {
        let mut document = Document::from_bytes(
            b"first paragraph\n\nsecond paragraph\n".to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        continue_list(&mut document, "first paragraph".len());
        assert_eq!(document.text(), "first paragraph\n\nsecond paragraph\n");
        assert_eq!(source(&document), "first paragraph\n\n\n\nsecond paragraph\n");
        assert!(document.undo());
        continue_list(&mut document, "first para".len());
        assert_eq!(document.text(), "first para\ngraph\nsecond paragraph\n");
        assert_eq!(source(&document), "first para\n\ngraph\n\nsecond paragraph\n");
    }
}
