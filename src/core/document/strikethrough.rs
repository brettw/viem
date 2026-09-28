//! Markdown strikethrough edits, composed as one verified transaction.
use super::replacement::PatchComposition;
use super::*;

impl Document {
    pub(super) fn prepare_strikethrough(
        &self,
        range: Range<usize>,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if range.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let application = StyleApplication::Automatic("Strikethrough".into());
        let spans = self.projection().style_spans_for_region(&range);
        let mut boundaries = BTreeSet::from([range.start, range.end]);
        for span in spans.iter().filter(|span| span.application == application) {
            boundaries.insert(span.range.start.max(range.start));
            boundaries.insert(span.range.end.min(range.end));
        }
        let boundaries = boundaries.into_iter().collect::<Vec<_>>();
        let segments = boundaries
            .windows(2)
            .filter(|pair| {
                pair[0] < pair[1]
                    && spans.iter().any(|span| {
                        span.application == application
                            && span.range.start <= pair[0]
                            && pair[1] <= span.range.end
                    }) != enabled
            })
            .map(|pair| pair[0]..pair[1])
            .collect::<Vec<_>>();
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        // Later source edits leave earlier snapshot-local segments in place.
        for segment in segments.into_iter().rev() {
            let prepared = scratch.prepare_typing_markdown_application(segment, None, enabled)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            formatted.record_formatted(&prepared)?;
            scratch.commit_model_transaction(prepared)?;
        }
        self.prepare_text_edits_with_patches(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
        )
    }

    pub(super) fn markdown_strike_removal_patches(
        &self,
        content: &Range<usize>,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        for (opening, closing) in [
            ("~~", "~~"),
            ("~", "~"),
            ("<del>", "</del>"),
            ("<s>", "</s>"),
            ("<strike>", "</strike>"),
        ] {
            let first = self.encoding().encode_fragment(opening)?;
            let last = self.encoding().encode_fragment(closing)?;
            let Some(start) = content.start.checked_sub(first.len()) else {
                continue;
            };
            let open = start..content.start;
            let close = content.end..content.end + last.len();
            if self.state().source.bytes_in(open.clone()).as_ref() == Some(&first)
                && self.state().source.bytes_in(close.clone()).as_ref() == Some(&last)
            {
                return Ok(vec![
                    SourcePatch::primary(open, Vec::new()),
                    SourcePatch::primary(close, Vec::new()),
                ]);
            }
        }
        Err(DocumentError::UnsupportedFormatting.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, FormattedTextPayload, StylePropertyValue};

    fn strike(document: &mut Document, range: Range<usize>, enabled: bool) {
        let request = ModelRequest::SetStrikethrough {
            document: document.id(),
            revision: document.revision(),
            range,
            enabled,
        };
        let prepared = document.prepare_model_request(request).unwrap();
        document.commit_model_transaction(prepared).unwrap();
    }

    #[test]
    fn markdown_strike_selection_persists_and_undoes_in_both_views() {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut document =
                Document::from_bytes(b"word".to_vec(), Encoding::Utf8, format).unwrap();
            strike(&mut document, 0..4, true);
            assert_eq!(document.source_bytes(), b"~~word~~");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), b"word");
            assert!(document.redo());
            let range = if format.is_source_view() { 0..8 } else { 0..4 };
            strike(&mut document, range, false);
            assert_eq!(document.source_bytes(), b"word");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), b"~~word~~");
        }
    }

    #[test]
    fn markdown_strike_typing_and_turning_off_preserve_caret_and_history() {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut document = Document::from_bytes(Vec::new(), Encoding::Utf8, format).unwrap();
            let payload =
                FormattedTextPayload::new(&document.hard_line_snapshot(), "word", vec![]).unwrap();
            let at = document
                .insert_with_typing_properties(
                    FormattedPayloadEdit::new(0..0, payload),
                    &[(
                        StyleProperty::CharacterStrikethrough,
                        StylePropertyValue::Boolean(true),
                    )],
                )
                .unwrap();
            assert_eq!(document.source_bytes(), b"~~word~~");
            assert_eq!(at, if format.is_source_view() { 6 } else { 4 });
            let payload =
                FormattedTextPayload::new(&document.hard_line_snapshot(), "X", vec![]).unwrap();
            document
                .insert_with_typing_properties(
                    FormattedPayloadEdit::new(at..at, payload),
                    &[(
                        StyleProperty::CharacterStrikethrough,
                        StylePropertyValue::Boolean(false),
                    )],
                )
                .unwrap();
            assert_eq!(document.source_bytes(), b"~~word~~X", "{format:?}");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), b"~~word~~");
            assert!(document.undo());
            assert!(document.source_bytes().is_empty());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), b"~~word~~");
        }
    }

    #[test]
    fn partial_and_mixed_strike_selections_preserve_unselected_text() {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut document =
                Document::from_bytes(b"~~word~~".to_vec(), Encoding::Utf8, format).unwrap();
            let range = if format.is_source_view() { 3..5 } else { 1..3 };
            strike(&mut document, range, false);
            assert_eq!(document.source_bytes(), b"~~w~~or~~d~~", "{format:?}");
            let partial = document.source_bytes();
            let end = document.text().len();
            strike(&mut document, 0..end, false);
            assert_eq!(document.source_bytes(), b"word", "{format:?}");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), partial);
        }
    }

    #[test]
    fn strikethrough_rejects_literal_formats_without_mutation() {
        for format in [Format::PlainText, Format::Code] {
            let document = Document::from_bytes(b"word".to_vec(), Encoding::Utf8, format).unwrap();
            let history = document.history_status();
            let request = ModelRequest::SetStrikethrough {
                document: document.id(),
                revision: document.revision(),
                range: 0..4,
                enabled: true,
            };
            assert!(document.prepare_model_request(request).is_err());
            assert_eq!(document.source_bytes(), b"word");
            assert_eq!(document.history_status(), history);
        }
    }
}
