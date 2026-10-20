//! Exact source insertion for a physical-line change whose empty destination
//! has no visible caret yet. Preparation remains isolated and revision-bound.
use super::*;
use super::replacement::PatchComposition;
use crate::document::{StylePropertyValue, TextPoint};

impl Document {
    pub fn prepare_insertion_at_source(
        &self,
        at: SourcePoint,
        payload: FormattedTextPayload,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, TextPoint, SourcePoint), ModelTransactionError> {
        self.require_modifiable()?;
        if at.document() != self.id() {
            return Err(ModelTransactionError::WrongDocument {
                expected: self.id(), actual: at.document(),
            });
        }
        if at.revision() != self.revision() {
            return Err(ModelTransactionError::StaleRevision {
                expected: at.revision(), actual: self.revision(),
            });
        }
        if at.part() != SourcePartId::PRIMARY {
            return Err(PositionError::WrongSourcePart {
                expected: SourcePartId::PRIMARY, actual: at.part(),
            }.into());
        }
        self.source_point(at.offset())?;
        if payload.document() != self.id() { return Err(DocumentError::WrongDocument.into()); }
        if payload.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: payload.revision(), actual: self.revision(),
            }.into());
        }
        if let Some(style) = named { self.validate_typing_named_style(style)?; }
        if !values.is_empty() { self.validate_typing_properties(values)?; }

        // Visible destinations retain the ordinary semantic typing path,
        // including paragraph splits, code owners and inline inheritance.
        if let Ok(mapped) = self.map_source_point(at, BoundaryAffinity::Downstream) {
            let point = mapped.text().offset();
            if !self.physical_line_ranges(self.physical_line_index_at_source(at.offset())?)?.1.is_empty()
                && self.source_point_for_insertion(mapped.text(), BoundaryAffinity::Downstream)
                    .is_ok_and(|point| point.offset() == at.offset())
            {
                let edit = FormattedPayloadEdit::new(point..point, payload)
                    .with_boundary_affinity(BoundaryAffinity::Downstream);
                let (mut prepared, caret) = if self.format().is_markdown() {
                    self.prepare_insertion_with_typing_style(edit, named, values)?
                } else {
                    let prepared = self.prepare_formatted_payload_request(
                        FormattedPayloadEditRequest::new(self.id(), self.revision(), vec![edit]),
                    )?;
                    let caret = prepared.text_position_map().map_text_point(
                        self.text_point(point)?, Association::AfterInsertion,
                        BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding,
                    )?.value().ok_or(DocumentError::AmbiguousProjection)?.offset();
                    (prepared, caret)
                };
                self.attach_prepared_source_map(&mut prepared)?;
                let candidate = self.prepared_candidate_document(&prepared)?;
                let source = candidate.projection().source_insertion_point(caret, false)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                return Ok((prepared, candidate.text_point(caret)?, candidate.source_point(source)?));
            }
        }

        let mut syntax = if self.format().is_markdown() {
            self.escape_markdown_source_text(at.offset(), payload.text())?
        } else { payload.text().to_owned() };
        if self.format().is_markdown() && syntax.contains('\n') {
            // Marked breaks retain their semantic identity. An unmarked LF
            // is literal text, so protect it from Markdown's ending fold.
            let offsets: Vec<_> = payload.text().match_indices('\n').map(|(offset, _)| offset).collect();
            let mut parts = syntax.split('\n');
            let mut converted = parts.next().unwrap_or("").to_owned();
            for (index, part) in parts.enumerate() {
                if payload.break_offsets().binary_search(&offsets[index]).is_ok() {
                    // Match ordinary semantic prose insertion: a complete
                    // paragraph ending also remains valid before an ATX
                    // heading, where a terminal backslash may become literal.
                    converted.push_str(self.file_format().spelling());
                    converted.push_str(self.file_format().spelling());
                } else {
                    converted.push_str("&#10;");
                }
                converted.push_str(part);
            }
            syntax = converted;
        } else if !payload.break_offsets().is_empty() {
            let mut converted = String::with_capacity(syntax.len());
            let mut start = 0;
            for &offset in payload.break_offsets() {
                converted.push_str(&syntax[start..offset]);
                converted.push_str(self.file_format().spelling());
                start = offset + 1;
            }
            converted.push_str(&syntax[start..]);
            syntax = converted;
        }
        let encoded = self.encoding().encode_fragment(&syntax)?;
        let source_end = at.offset() + encoded.len();
        let mut scratch = self.scratch_document();
        let initial = scratch.prepare_reprojected_source_patches(vec![
            SourcePatch::primary(at.offset()..at.offset(), encoded),
        ])?;
        let mut composed = PatchComposition::new(self.source_byte_len());
        for patch in initial.summary().source_patches().iter().rev() {
            composed.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(initial)?;
        let start = scratch.visible_point_for_source(at.offset(), true)?;
        let mut caret = scratch.visible_point_for_source(source_end, false)?;
        if start > caret { return Err(DocumentError::FormattedPayloadCannotReproject.into()); }
        let captured = scratch.hard_line_snapshot().capture(start..caret)
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        if captured.text() != payload.text() || captured.break_offsets() != payload.break_offsets() {
            return Err(DocumentError::FormattedPayloadCannotReproject.into());
        }
        let mut source_caret = scratch.source_point(source_end)?;
        if named.is_some() || !values.is_empty() {
            // Restyling the authored selection shares ordinary verified typing
            // rules. Compose its patches with the exact original insertion.
            let rebound = FormattedTextPayload::new(&scratch.hard_line_snapshot(),
                payload.text(), payload.break_offsets().to_vec())
                .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
            let (styled, new_caret) = scratch.prepare_insertion_with_typing_style(
                FormattedPayloadEdit::new(start..caret, rebound), named, values,
            )?;
            let candidate = scratch.prepared_candidate_document(&styled)?;
            source_caret = candidate.source_point(candidate.projection()
                .source_insertion_point(new_caret, false)
                .ok_or(DocumentError::AmbiguousProjection)?)?;
            for patch in styled.summary().source_patches().iter().rev() {
                composed.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(styled)?;
            caret = new_caret;
        }
        let mut prepared = self.prepare_reprojected_source_patches(composed.source_patches())?;
        self.attach_prepared_source_map(&mut prepared)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        let text = candidate.text_point(caret)?;
        let source = candidate.source_point(source_caret.offset())?;
        Ok((prepared, text, source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    fn payload(document: &Document, text: &str) -> FormattedTextPayload {
        FormattedTextPayload::new(&document.hard_line_snapshot(), text, Vec::new()).unwrap()
    }

    #[test]
    fn typing_materializes_the_exact_hidden_physical_destination() {
        let mut document = Document::from_bytes(b"\nlast".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let revision = document.revision();
        let (prepared, caret, source) = document.prepare_insertion_at_source(
            document.source_point(0).unwrap(), payload(&document, "X"), None, &[],
        ).unwrap();
        assert_eq!(document.source_bytes(), b"\nlast");
        assert_eq!(document.revision(), revision);
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), b"X\nlast");
        assert_eq!(document.text(), "X last");
        assert_eq!(caret.offset(), 1);
        assert_eq!(source.offset(), 1);
        assert_eq!(caret.revision(), document.revision());
        assert_eq!(source.revision(), document.revision());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"\nlast");
    }

    #[test]
    fn hidden_source_typing_escapes_new_markup_and_applies_pending_style() {
        let mut document = Document::from_bytes(b"\nlast".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let (prepared, caret, source) = document.prepare_insertion_at_source(
            document.source_point(0).unwrap(), payload(&document, "X"), None,
            &[(StyleProperty::CharacterBold, StylePropertyValue::Boolean(true))],
        ).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), b"**X**\nlast");
        assert_eq!(document.text(), "X last");
        assert_eq!(caret.offset(), 1);
        assert_eq!(source.offset(), 3);

        let document = Document::from_bytes(b"\nlast".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let (prepared, _, _) = document.prepare_insertion_at_source(
            document.source_point(0).unwrap(), payload(&document, "#"), None, &[],
        ).unwrap();
        let candidate = document.prepared_candidate_document(&prepared).unwrap();
        assert_eq!(candidate.source_bytes(), b"\\#\nlast");
        assert_eq!(candidate.text(), "# last");
    }

    #[test]
    fn source_typing_rejects_stale_points_without_changing_source() {
        let mut document = Document::from_bytes(b"\nlast".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let stale = document.source_point(0).unwrap();
        document.replace_physical_source(1..2, "L").unwrap();
        let before = document.source_bytes();
        assert!(matches!(document.prepare_insertion_at_source(stale,
            payload(&document, "X"), None, &[]), Err(ModelTransactionError::StaleRevision { .. })));
        assert_eq!(document.source_bytes(), before);
    }

    #[test]
    fn source_typing_preserves_unmarked_lf_under_mac_fileformat() {
        let mut document = Document::from_bytes_with_file_format(b"\rlast".to_vec(),
            Encoding::Utf8, Format::PlainText, FileFormat::Mac).unwrap();
        let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "one\ntwo", Vec::new()).unwrap();
        let (prepared, _, _) = document.prepare_insertion_at_source(
            document.source_point(0).unwrap(), payload, None, &[],
        ).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), b"one\ntwo\rlast");
        assert_eq!(document.physical_source_line_number_count(), 2);

        let mut document = Document::from_bytes_with_file_format(b"\rlast".to_vec(),
            Encoding::Utf8, Format::Markdown, FileFormat::Mac).unwrap();
        let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "one\ntwo", Vec::new()).unwrap();
        let (prepared, _, _) = document.prepare_insertion_at_source(
            document.source_point(0).unwrap(), payload, None, &[],
        ).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), b"one&#10;two\rlast");
        assert!(document.hard_line_snapshot().capture(0..7).unwrap().break_offsets().is_empty());
    }

    #[test]
    fn exact_source_insertion_honors_a_hidden_heading_prefix_boundary() {
        let mut document = Document::from_bytes(b"first\n# second".to_vec(),
            Encoding::Utf8, Format::Markdown).unwrap();
        let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "X\n", vec![1]).unwrap();
        let (prepared, _, _) = document.prepare_insertion_at_source(
            document.source_point(6).unwrap(), payload, None, &[],
        ).unwrap();
        assert!(prepared.summary().source_patches().iter()
            .all(|patch| patch.range() == (6..6)));
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), b"first\nX\n\n# second");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"first\n# second");
    }
}
