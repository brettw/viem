//! Source-line transformations preserve delimiters and unrelated malformed bytes.
use super::*;
use crate::document::{BlockKind, IndentationOptions, leading_whitespace_len};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalSourceTransform {
    Lowercase,
    Uppercase,
    ToggleCase,
    Indent { columns: usize, options: IndentationOptions },
    Outdent { columns: usize, options: IndentationOptions },
    Reindent { options: IndentationOptions },
}

impl Document {
    /// Independent ordinary paragraphs can be removed from one snapshot. The
    /// exact contributor ranges prove the local source patches; a single
    /// candidate verifies the complete requested text before publication.
    pub(crate) fn prepare_independent_paragraph_deletions(
        &self, ranges: Vec<Range<usize>>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !self.format().is_wysiwyg() || !self.projection().style_spans().is_empty()
            || self.projection().blocks().iter().any(|block| block.style.0 != "Paragraph"
                || block.kind != BlockKind::Paragraph || block.quote_depth != 0
                || !block.containers.is_empty() || block.markdown_html || block.thematic_break) {
            return Ok(None);
        }
        let mut patches = Vec::new();
        let mut edits = Vec::new();
        for range in ranges {
            self.validate_range(&range)?;
            let Some(source) = self.projection().source_range(range.clone()) else { return Ok(None); };
            patches.push(SourcePatch::primary(source, Vec::new()));
            edits.push(TextEdit::new(range, ""));
        }
        self.prepare_text_edits_with_patch_policy(edits, Some(patches), true).map(Some)
    }

    pub(super) fn prepare_physical_source_transform(
        &self, range: Range<usize>, transform: PhysicalSourceTransform,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.source_point(range.start)?;
        self.source_point(range.end)?;
        if range.start > range.end {
            return Err(DocumentError::InvalidRange {
                start: range.start, end: range.end, length: self.source_byte_len(),
            }.into());
        }
        if range.is_empty() { return Ok(self.no_op_prepared()); }
        let first = self.physical_line_index_at_source(range.start)?;
        let last = self.physical_line_index_at_source(range.end - 1)?;
        let mut patches: Vec<SourcePatch> = Vec::new();
        let mut previous = 0;
        if let PhysicalSourceTransform::Reindent { options } = transform {
            for index in (first.saturating_sub(512)..first).rev() {
                let line = self.physical_line(index)?;
                let prefix = leading_whitespace_len(&line.text);
                if prefix < line.text.len() {
                    previous = options.columns(&line.text[..prefix]);
                    break;
                }
            }
        }
        for index in first..=last {
            let (_, content) = self.physical_line_ranges(index)?;
            let content = content.start.max(range.start)..content.end.min(range.end);
            if content.start > content.end { continue; }
            let bytes = self.source_bytes_in(content.clone())?;
            let decoded = self.encoding().decode_region(&bytes, content.start)?;
            match transform {
                PhysicalSourceTransform::Indent { options, .. }
                | PhysicalSourceTransform::Outdent { options, .. }
                | PhysicalSourceTransform::Reindent { options } => {
                    if decoded.text.is_empty() { continue; }
                    let prefix = leading_whitespace_len(&decoded.text);
                    let old_columns = options.columns(&decoded.text[..prefix]);
                    let columns = match transform {
                        PhysicalSourceTransform::Indent { columns, .. } => old_columns.checked_add(columns)
                            .ok_or(DocumentError::UnsupportedFormatting)?,
                        PhysicalSourceTransform::Outdent { columns, .. } => old_columns.saturating_sub(columns),
                        _ => if prefix == decoded.text.len() { 0 } else { previous },
                    };
                    if prefix < decoded.text.len() { previous = columns; }
                    let replacement = options.try_whitespace(0, columns).map_err(|_| DocumentError::UnsupportedFormatting)?;
                    if replacement != decoded.text[..prefix] {
                        let end = decoded.source_boundary(prefix).ok_or(DocumentError::AmbiguousProjection)?;
                        patches.push(SourcePatch::primary(content.start..end, self.encoding().encode_fragment(&replacement)?));
                    }
                }
                PhysicalSourceTransform::Lowercase | PhysicalSourceTransform::Uppercase
                    | PhysicalSourceTransform::ToggleCase => {
                    let transformed = match transform {
                        PhysicalSourceTransform::Lowercase => decoded.text.to_lowercase(),
                        PhysicalSourceTransform::Uppercase => decoded.text.to_uppercase(),
                        PhysicalSourceTransform::ToggleCase => decoded.text.chars().flat_map(|ch|
                            if ch.is_lowercase() { ch.to_uppercase().collect::<Vec<_>>() }
                            else { ch.to_lowercase().collect::<Vec<_>>() }).collect(),
                        _ => unreachable!(),
                    };
                    let mut output = transformed.chars();
                    for span in decoded.scalar_spans() {
                        let original = &decoded.text[span.decoded.clone()];
                        let ch = original.chars().next().ok_or(DocumentError::AmbiguousProjection)?;
                        let count = if matches!(transform, PhysicalSourceTransform::Uppercase)
                            || (matches!(transform, PhysicalSourceTransform::ToggleCase) && ch.is_lowercase()) {
                            ch.to_uppercase().count()
                        } else { ch.to_lowercase().count() };
                        let replacement: String = output.by_ref().take(count).collect();
                        if replacement == original { continue; }
                        let encoded = self.encoding().encode_fragment(&replacement)?;
                        if let Some(previous) = patches.last_mut().filter(|patch| patch.range.end == span.source.start) {
                            previous.range.end = span.source.end;
                            previous.replacement.extend(encoded);
                        } else { patches.push(SourcePatch::primary(span.source, encoded)); }
                    }
                    if output.next().is_some() { return Err(DocumentError::VerificationFailed.into()); }
                }
            }
        }
        self.prepare_physical_source_patches(patches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn source_transforms_preserve_mixed_endings_literal_lf_and_malformed_bytes() {
        let original = b"one\r\ntwo\nthree";
        let mut document = Document::from_bytes_with_file_format(original.to_vec(), Encoding::Utf8,
            Format::Code, FileFormat::Dos).unwrap();
        document.apply_model_request(ModelRequest::TransformPhysicalSource {
            document: document.id(), revision: document.revision(), range: 0..original.len(),
            transform: PhysicalSourceTransform::Uppercase,
        }).unwrap();
        assert_eq!(document.source_bytes(), b"ONE\r\nTWO\nTHREE");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);

        let mut document = Document::from_bytes_with_file_format(b"one\ntwo\rthree".to_vec(),
            Encoding::Utf8, Format::PlainText, FileFormat::Mac).unwrap();
        document.apply_model_request(ModelRequest::TransformPhysicalSource {
            document: document.id(), revision: document.revision(), range: 0..7,
            transform: PhysicalSourceTransform::Indent { columns: 2, options: IndentationOptions::default() },
        }).unwrap();
        assert_eq!(document.source_bytes(), b"  one\ntwo\rthree");

        let mut document = Document::from_bytes(b"a\xffb".to_vec(), Encoding::Utf8, Format::Code).unwrap();
        document.apply_model_request(ModelRequest::TransformPhysicalSource {
            document: document.id(), revision: document.revision(), range: 0..3,
            transform: PhysicalSourceTransform::Uppercase,
        }).unwrap();
        assert_eq!(document.source_bytes(), b"A\xffB");
    }

    #[test]
    fn physical_source_move_preserves_source_and_formatted_identities_in_folded_prose() {
        let mut document = Document::from_bytes(b"one\n**two**\nthree".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let source = document.source_point(7).unwrap();
        let text = document.text_point(5).unwrap();
        let neighbor = document.text_point(1).unwrap();
        let folded_ending = document.text_point(3).unwrap();
        let neighbor_anchor = document.text_anchor(neighbor, Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap();
        let committed = document.apply_model_request(ModelRequest::MovePhysicalSource {
            document: document.id(), revision: document.revision(),
            patches: vec![SourcePatch::primary(0..0, b"**two**\n".to_vec()),
                SourcePatch::primary(4..12, Vec::new())],
            source: 4..12, destination: 0,
        }).unwrap();
        assert_eq!(document.source_bytes(), b"**two**\none\nthree");
        assert_eq!(document.text(), "two one three");
        let map = committed.text_position_map();
        let mapped_source = map.source_position_map().unwrap().map_source_point(source,
            Association::AfterInsertion, DeletionRecovery::Unresolvable).unwrap();
        assert_eq!(mapped_source.value().unwrap().offset(), 3);
        let mapped_text = map.map_text_point(text, Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap();
        assert_eq!(mapped_text.value().unwrap().offset(), 1);
        assert_eq!(map.map_text_point(neighbor, Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 5);
        assert_eq!(document.resolve_text_anchor(neighbor_anchor).unwrap().value().unwrap().offset(), 5);
        assert_eq!(map.map_text_point(folded_ending, Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 7);
        let undo = document.apply_model_request(ModelRequest::NavigateHistory {
            document: document.id(), revision: document.revision(), navigation: HistoryNavigationRequest::Undo,
        }).unwrap();
        assert_eq!(document.source_bytes(), b"one\n**two**\nthree");
        assert_eq!(document.resolve_text_anchor(neighbor_anchor).unwrap().value().unwrap().offset(), 1);
        assert_eq!(undo.text_position_map().source_position_map().unwrap().map_source_point(
            *mapped_source.value().unwrap(), Association::AfterInsertion,
            DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 7);
        assert_eq!(undo.text_position_map().map_text_point(*mapped_text.value().unwrap(),
            Association::AfterInsertion, BoundaryAffinity::Downstream,
            DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 5);
    }

    #[test]
    fn moved_source_only_destinations_compose_and_reject_invalid_identity_claims() {
        let mut document = Document::from_bytes(b"a\n\nb".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let target = document.source_point(2).unwrap();
        let moved = document.apply_model_request(ModelRequest::MovePhysicalSource {
            document: document.id(), revision: document.revision(),
            patches: vec![SourcePatch::primary(0..0, b"\n".to_vec()), SourcePatch::primary(2..3, Vec::new())],
            source: 2..3, destination: 0,
        }).unwrap();
        let inserted = document.apply_model_request(ModelRequest::ReplacePhysicalSource {
            document: document.id(), revision: document.revision(), range: 0..0, replacement: "x".into(),
        }).unwrap();
        let composed = moved.text_position_map().then(inserted.text_position_map()).unwrap();
        assert_eq!(composed.source_position_map().unwrap().map_source_point(target,
            Association::AfterInsertion, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 1);
        let before = document.source_bytes();
        let revision = document.revision();
        assert!(document.prepare_model_request(ModelRequest::MovePhysicalSource {
            document: document.id(), revision, patches: vec![SourcePatch::primary(0..0, b"q".to_vec())],
            source: 0..1, destination: 0,
        }).is_err());
        assert_eq!(document.source_bytes(), before);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn source_reordering_retains_empty_line_start_identity_and_compact_range_composition() {
        let mut document = Document::from_bytes(b"b\n\na".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        let empty = document.source_point(2).unwrap();
        let word = document.text_point(0).unwrap();
        let sorted = document.apply_model_request(ModelRequest::ReorderPhysicalSource {
            document: document.id(), revision: document.revision(),
            patches: vec![SourcePatch::primary(0..4, b"\na\nb".to_vec())],
            relocations: vec![(0..1, 3), (2..2, 0), (3..4, 1)],
        }).unwrap();
        assert_eq!(sorted.text_position_map().source_position_map().unwrap().map_source_point(empty,
            Association::AfterInsertion, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 0);
        assert_eq!(sorted.text_position_map().map_text_point(word, Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 3);
        let inserted = document.apply_model_request(ModelRequest::ReplacePhysicalSource {
            document: document.id(), revision: document.revision(), range: 0..0, replacement: "x".into(),
        }).unwrap();
        let composed = sorted.text_position_map().then(inserted.text_position_map()).unwrap();
        assert_eq!(composed.source_position_map().unwrap().map_source_point(empty,
            Association::AfterInsertion, DeletionRecovery::Unresolvable).unwrap().value().unwrap().offset(), 1);
        assert_eq!(composed.preserved_content_ranges(0..1).unwrap(), Some(vec![4..5]));
        assert!(composed.replacements().all(|(old, new)| old.start <= old.end && new.start <= new.end));
    }

    #[test]
    fn empty_literal_paragraph_anchors_follow_source_sort_and_move_with_undo() {
        for format in [Format::PlainText, Format::Code] {
            for moving in [false, true] {
                let mut document = Document::from_bytes(b"z\n\na".to_vec(), Encoding::Utf8, format).unwrap();
                let id = document.projection().hard_line_id(1).unwrap();
                let anchor = document.text_anchor(document.text_point(2).unwrap(), Association::AfterInsertion,
                    BoundaryAffinity::Downstream, DeletionRecovery::Unresolvable).unwrap();
                let request = if moving {
                    ModelRequest::MovePhysicalSource {
                        document: document.id(), revision: document.revision(),
                        patches: vec![SourcePatch::primary(0..0, b"\n".to_vec()),
                            SourcePatch::primary(2..3, Vec::new())],
                        source: 2..3, destination: 0,
                    }
                } else {
                    ModelRequest::ReorderPhysicalSource {
                        document: document.id(), revision: document.revision(),
                        patches: vec![SourcePatch::primary(0..4, b"\na\nz".to_vec())],
                        relocations: vec![(0..1, 3), (2..2, 0), (3..4, 1)],
                    }
                };
                document.apply_model_request(request).unwrap();
                assert_eq!(document.projection().hard_line_id(0), Some(id), "{format:?} move={moving}");
                assert_eq!(document.resolve_text_anchor(anchor).unwrap().value().unwrap().offset(), 0,
                    "{format:?} move={moving}");
                let ids: std::collections::HashSet<_> = document.projection().blocks().iter().map(|block| block.id).collect();
                assert_eq!(ids.len(), document.projection().blocks().len());
                assert!(document.undo());
                assert_eq!(document.source_bytes(), b"z\n\na");
                assert_eq!(document.resolve_text_anchor(anchor).unwrap().value().unwrap().offset(), 2);
                assert!(document.redo());
                assert_eq!(document.resolve_text_anchor(anchor).unwrap().value().unwrap().offset(), 0);
            }
        }
    }
}
