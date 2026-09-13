//! Exact physical-line queries for pipelines with the shared line-ending stage.
use super::formatted_text::LogicalGraphemeSnapshot;
use super::*;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalSourceLine {
    pub document: DocumentId,
    pub revision: Revision,
    pub index: usize,
    pub source_range: Range<usize>,
    pub content_range: Range<usize>,
    pub text: String,
}
impl Document {
    pub fn physical_line_count(&self) -> Result<usize, DocumentError> {
        if self.format() == Format::Rtf {
            return Err(DocumentError::UnsupportedFormatting);
        }
        Ok(self.state().source_hard_lines.len())
    }
    pub fn physical_line(&self, index: usize) -> Result<PhysicalSourceLine, DocumentError> {
        self.physical_line_count()?;
        let source = self
            .state()
            .source_hard_lines
            .get(index)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, source.start)?;
        let normalized = line_endings::normalize_literal(&decoded, self.file_format());
        let end = normalized
            .endings
            .last()
            .map_or(source.end, |ending| ending.source.start);
        Ok(PhysicalSourceLine {
            document: self.id(),
            revision: self.revision(),
            index,
            source_range: source.clone(),
            content_range: source.start..end,
            text: normalized.text,
        })
    }
    pub fn physical_line_at_source(&self, at: usize) -> Result<PhysicalSourceLine, DocumentError> {
        self.physical_line_count()?;
        let index = self.state().source_hard_lines.line_at_offset(at).ok_or(
            DocumentError::InvalidRange {
                start: at,
                end: at,
                length: self.source_byte_len(),
            },
        )?;
        self.physical_line(index)
    }
    pub fn physical_line_at_text(&self, at: usize) -> Result<PhysicalSourceLine, DocumentError> {
        self.text_point(at)?;
        self.physical_line_count()?;
        let source = self
            .projection()
            .source_insertion_point(at, true)
            .or_else(|| {
                self.state()
                    .source_hard_lines
                    .get(0)
                    .map(|range| range.start)
            })
            .ok_or(DocumentError::AmbiguousProjection)?;
        let index = self
            .state()
            .source_hard_lines
            .line_at_offset(source)
            .ok_or(DocumentError::AmbiguousProjection)?;
        self.physical_line(index)
    }
    /// Explicit recovery policy for source syntax with no formatted interior:
    /// select the next visible source boundary, then the preceding one at EOF.
    pub fn visible_point_for_source(
        &self,
        source: usize,
        downstream: bool,
    ) -> Result<usize, DocumentError> {
        self.source_point(source)
            .map_err(|_| DocumentError::InvalidRange {
                start: source,
                end: source,
                length: self.source_byte_len(),
            })?;
        let projection = self.projection();
        let point = projection
            .nearest_text_boundary_for_source(source, downstream)
            .unwrap_or(0);
        if projection
            .is_logical_grapheme_boundary(point)
            .map_err(DocumentError::FormattedTextStorage)?
        {
            return Ok(point);
        }
        // Provenance may end between Unicode scalars of one grapheme. Resolve
        // only that local cluster in the persistent text tree.
        Ok(if downstream {
            projection
                .next_logical_grapheme_boundary(point)
                .map_err(DocumentError::FormattedTextStorage)?
                .unwrap_or(projection.text_tree().byte_len())
        } else {
            projection
                .previous_logical_grapheme_boundary(point)
                .map_err(DocumentError::FormattedTextStorage)?
                .unwrap_or(0)
        })
    }
    pub fn replace_physical_source(
        &mut self,
        range: Range<usize>,
        replacement: impl Into<String>,
    ) -> Result<bool, DocumentError> {
        let request = ModelRequest::ReplacePhysicalSource {
            document: self.id(),
            revision: self.revision(),
            range,
            replacement: replacement.into(),
        };
        let before = self.revision();
        self.execute_compat_request(request)?;
        Ok(self.revision() != before)
    }
}

impl Document {
    pub fn physical_source_text(&self, range: Range<usize>) -> Result<String, DocumentError> {
        self.physical_line_count()?;
        let bytes =
            self.state()
                .source
                .bytes_in(range.clone())
                .ok_or(DocumentError::InvalidRange {
                    start: range.start,
                    end: range.end,
                    length: self.source_byte_len(),
                })?;
        let decoded = self.encoding().decode_region(&bytes, range.start)?;
        Ok(line_endings::normalize_literal(&decoded, self.file_format()).text)
    }
    /// Explicit row deletion removes complete items structurally. Adapters
    /// retain the surrounding list when only part of a wrapped item is covered.
    pub fn delete_visual_text(&mut self, range: Range<usize>) -> Result<(), DocumentError> {
        self.delete_lines(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_source_lookup_after_local_edit_is_indexed_without_flattening_text() {
        let mut document = Document::new("line text\n".repeat(100_000));
        document.replace(0..1, "XX").unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
        let end = document.source_byte_len();
        for offset in end - 80..end - 10 {
            assert_eq!(
                document.visible_point_for_source(offset, true).unwrap(),
                offset
            );
            assert_eq!(
                document.visible_point_for_source(offset, false).unwrap(),
                offset
            );
            let (nodes, items) = document.projection().source_boundary_query_work(offset);
            assert!(
                nodes < 100 && items < 256,
                "late boundary visited {nodes} nodes and {items} records"
            );
        }
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn reverse_boundary_index_matches_fresh_projection_across_regional_edits() {
        for (format, original) in [
            (Format::PlainText, "one\ntwo\n"),
            (Format::MarkdownSource, "# one\nlast"),
            (Format::Markdown, "# one\n**two**"),
            (Format::Html, "<p>one</p><p>two</p>"),
        ] {
            let mut document =
                Document::from_bytes(original.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            for replacement in ["XY", "", "e\u{301}"] {
                document.replace(0..1, replacement).unwrap();
                let fresh =
                    Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
                for at in 0..=document.source_byte_len() {
                    for downstream in [false, true] {
                        assert_eq!(document.visible_point_for_source(at, downstream), fresh.visible_point_for_source(at, downstream), "{format:?}: source {at}, downstream {downstream}, replacement {replacement:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn source_ordered_lookup_handles_reordered_html_and_local_graphemes() {
        let source = "<table>before<tr><td>cell</td></tr>after</table>";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        for word in ["before", "after"] {
            assert_eq!(
                document
                    .visible_point_for_source(source.find(word).unwrap(), true)
                    .unwrap(),
                document.text().find(word).unwrap()
            );
        }
        let base = Document::new("ba");
        let reordered = projection::FormattedDocument::from_parts(
            Revision(0),
            "ba".into(),
            base.projection().blocks().to_vec(),
            Vec::new(),
            vec![
                projection::ProvenanceSpan {
                    formatted: 0..1,
                    source: 2..3,
                },
                projection::ProvenanceSpan {
                    formatted: 1..2,
                    source: 0..1,
                },
            ],
            Vec::new(),
            base.projection().style_sheet().clone(),
            0,
            3,
        );
        assert_eq!(reordered.nearest_text_boundary_for_source(0, true), Some(1));
        assert_eq!(reordered.nearest_text_boundary_for_source(2, true), Some(0));
        assert_eq!(
            reordered.nearest_text_boundary_for_source(1, false),
            Some(2)
        );
        let document = Document::new("e\u{301} tail");
        assert_eq!(document.visible_point_for_source(1, false).unwrap(), 0);
        assert_eq!(document.visible_point_for_source(1, true).unwrap(), 3);
    }
}
