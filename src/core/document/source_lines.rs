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
    /// An empty physical line owns source space even when its projected empty
    /// paragraph shares an ordinary typing boundary. Authoring there must not
    /// add supporting separators to preserve the disposable paragraph view.
    pub fn source_insertion_requires_materialization(&self, at: SourcePoint) -> Result<bool, DocumentError> {
        if at.document() != self.id() { return Err(DocumentError::WrongDocument); }
        if at.revision() != self.revision() { return Err(DocumentError::WrongSnapshot {
            expected: self.revision(), actual: at.revision(),
        }); }
        if at.part() != SourcePartId::PRIMARY { return Err(DocumentError::AmbiguousProjection); }
        self.source_point(at.offset()).map_err(|_| DocumentError::AmbiguousProjection)?;
        let index = self.physical_line_index_at_source(at.offset())?;
        if self.physical_line_ranges(index)?.1.is_empty() { return Ok(true); }
        let visible = self.visible_point_for_source(at.offset(), true)?;
        let point = self.text_point(visible)?;
        let Ok(insertion) = self.source_point_for_insertion(point, BoundaryAffinity::Downstream) else {
            return Ok(true);
        };
        // Prefix syntax on a visible line selects its existing semantic owner
        // (heading, emphasis, quote, etc.). Source-only lines instead recover
        // a display caret in a different line and need their exact source point.
        Ok(self.physical_line_index_at_source(insertion.offset())? != index)
    }

    /// Exact source destination used by ordinary semantic typing at a visible
    /// caret. Paragraph ownership and inline affinity both participate.
    pub fn source_point_for_insertion(
        &self,
        point: TextPoint,
        affinity: BoundaryAffinity,
    ) -> Result<SourcePoint, DocumentError> {
        if point.document() != self.id() { return Err(DocumentError::WrongDocument); }
        if point.revision() != self.revision() { return Err(DocumentError::WrongSnapshot {
            expected: point.revision(), actual: self.revision(),
        }); }
        self.text_point(point.offset())?;
        let at = super::source_edit::insertion_point(self.projection(), point.offset(), Some(affinity))
            .ok_or(DocumentError::AmbiguousProjection)?;
        self.source_point(at).map_err(|_| DocumentError::AmbiguousProjection)
    }

    /// Indexed physical bounds without decoding the line body. Only the small
    /// delimiter tail is inspected; a literal LF in a Mac-format line remains
    /// part of its content.
    pub fn physical_line_ranges(
        &self,
        index: usize,
    ) -> Result<(Range<usize>, Range<usize>), DocumentError> {
        let source = self.state().source_hard_lines.get(index)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut end = source.end;
        let spellings: &[&str] = match self.file_format() {
            FileFormat::Unix => &["\n"],
            FileFormat::Dos => &["\r\n", "\n"],
            FileFormat::Mac => &["\r"],
        };
        for spelling in spellings {
            let delimiter = self.encoding().encode_fragment(spelling)?;
            if delimiter.len() <= source.len()
                && (!matches!(self.encoding(), Encoding::Utf16Le | Encoding::Utf16Be)
                    || source.len() % 2 == 0)
                && self.state().source.bytes_in(source.end - delimiter.len()..source.end)
                    .is_some_and(|tail| tail == delimiter)
            {
                end -= delimiter.len();
                break;
            }
        }
        Ok((source.clone(), source.start..end))
    }

    /// Find a physical line's first nonblank grapheme without decoding its
    /// body. Only the space/tab prefix is read, in bounded source-rope slices.
    /// An entirely blank line keeps its initial caret, including empty lines.
    pub fn physical_first_nonblank_source(&self, index: usize) -> Result<usize, DocumentError> {
        use unicode_segmentation::UnicodeSegmentation;
        const PREFIX_CHUNK_BYTES: usize = 256;
        let (_, content) = self.physical_line_ranges(index)?;
        let (width, space, tab, scalar_bytes): (usize, &[u8], &[u8], usize) = match self.encoding() {
            Encoding::Utf8 => (1, b" ", b"\t", 4),
            Encoding::Latin1 => (1, b" ", b"\t", 1),
            Encoding::Utf16Le => (2, &[0x20, 0], &[9, 0], 4),
            Encoding::Utf16Be => (2, &[0, 0x20], &[0, 9], 4),
        };
        let mut at = content.start;
        let mut previous_blank = None;
        while at < content.end {
            let end = at.saturating_add(PREFIX_CHUNK_BYTES).min(content.end);
            let bytes = self.state().source.bytes_in(at..end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut offset = 0;
            while offset < bytes.len() {
                let unit_end = (offset + width).min(bytes.len());
                let unit = &bytes[offset..unit_end];
                if unit == space || unit == tab {
                    previous_blank = Some(at + offset);
                    offset = unit_end;
                    continue;
                }
                let nonblank = at + offset;
                if let Some(previous) = previous_blank {
                    // A combining mark can make the preceding space part of
                    // the first nonblank grapheme. One complete following
                    // scalar suffices to test that boundary; malformed bytes
                    // retain their ordinary diagnostic scalar semantics.
                    let lookahead_end = nonblank.saturating_add(scalar_bytes).min(content.end);
                    let lookahead = self.state().source.bytes_in(previous..lookahead_end)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let decoded = self.encoding().decode_region(&lookahead, previous)?;
                    if decoded.text.graphemes(true).next()
                        .is_some_and(|grapheme| !matches!(grapheme, " " | "\t")) {
                        return Ok(previous);
                    }
                }
                return Ok(nonblank);
            }
            at = end;
        }
        Ok(content.start)
    }

    pub(crate) fn code_detection_samples(&self) -> Vec<(usize, String)> {
        let count = self.state().source_hard_lines.len();
        let mut indexes = std::collections::BTreeSet::new();
        indexes.extend(0..count.min(5));
        indexes.extend(count.saturating_sub(5)..count);
        let half = super::syntax::detection::DETECTION_BYTE_LIMIT / 2;
        let length = self.source_byte_len();
        let mut decoded_budget = super::syntax::detection::DETECTION_BYTE_LIMIT;
        indexes.into_iter().filter_map(|index| {
            let range = self.state().source_hard_lines.get(index)?;
            if !((index < 5 && range.end <= half) || (index >= count.saturating_sub(5) && range.start >= length.saturating_sub(half))) {
                return None;
            }
            let line = self.physical_line(index).ok()?;
            // Latin-1 can expand in UTF-8; retain the decoded inspection cap too.
            if line.text.len() > decoded_budget { return None; }
            decoded_budget -= line.text.len();
            Some((index, line.text.trim_end_matches('\n').to_owned()))
        }).collect()
    }

    pub fn physical_line_count(&self) -> Result<usize, DocumentError> {

        Ok(self.state().source_hard_lines.len())
    }
    /// File-line numbering excludes the zero-byte caret boundary after a final
    /// terminator. Physical commands retain that editable terminal destination.
    pub fn physical_source_line_number_count(&self) -> usize {
        let lines = &self.state().source_hard_lines;
        let count = lines.len();
        if count > 1 && lines.get(count - 1).is_some_and(|range| range.is_empty()) {
            count - 1
        } else { count }
    }
    /// Indexed physical ordinals are independent of decoding and view projection.
    pub fn physical_line_index_at_source(&self, at: usize) -> Result<usize, DocumentError> {
        self.state().source_hard_lines.line_at_offset(at).ok_or(DocumentError::InvalidRange {
            start: at, end: at, length: self.source_byte_len(),
        })
    }
    /// The projected extent of one source line, queried through persistent
    /// boundary indexes without decoding its body or walking its provenance.
    pub(crate) fn physical_line_projected_extent(&self, index: usize) -> Option<Range<usize>> {
        let source = self.state().source_hard_lines.get(index)?;
        let projection = self.projection();
        let start = projection.nearest_text_boundary_for_source(source.start, true)?;
        let end = projection.nearest_text_boundary_for_source(source.end, false)?;
        Some(start.min(end)..start.max(end))
    }
    /// Ordinals with visible contributors in a local formatted range. Indivisible
    /// objects represent their first source line; hidden syntax has no glyph band.
    pub(crate) fn physical_line_indexes_in_text(&self, range: Range<usize>) -> Result<Vec<usize>, DocumentError> {
        let mut result = std::collections::BTreeSet::new();
        for span in self.projection().provenance_for_region(&range) {
            if !span.formatted.is_empty() && !span.source.is_empty() {
                result.insert(self.physical_line_index_at_source(span.source.start)?);
            }
        }
        if result.is_empty() {
            let source = self.projection().source_insertion_point(range.start, true)
                .unwrap_or_else(|| self.state().source_hard_lines.get(0).map_or(0, |line| line.start));
            result.insert(self.physical_line_index_at_source(source)?);
        }
        Ok(result.into_iter().collect())
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
    /// Literal source text with explicit semantic break identities. Reading a
    /// register must not treat every decoded LF as a physical line ending.
    pub fn physical_source_payload(
        &self,
        range: Range<usize>,
    ) -> Result<FormattedTextPayload, DocumentError> {
        let bytes = self.state().source.bytes_in(range.clone())
            .ok_or(DocumentError::InvalidRange {
                start: range.start, end: range.end, length: self.source_byte_len(),
            })?;
        let decoded = self.encoding().decode_region(&bytes, range.start)?;
        let normalized = line_endings::normalize_literal(&decoded, self.file_format());
        let breaks = normalized.endings.iter().map(|ending| ending.normalized.start).collect();
        FormattedTextPayload::new(&self.hard_line_snapshot(), normalized.text, breaks)
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)
    }

    /// A visible selection belongs to source bounds only when all of its
    /// actual contributors do. Indivisible contributors are checked whole;
    /// hidden bytes between contributors do not enlarge the selection.
    pub fn formatted_range_within_source(
        &self,
        range: Range<usize>,
        source: Range<usize>,
    ) -> Result<bool, DocumentError> {
        self.text_point(range.start)?;
        self.text_point(range.end)?;
        for at in [source.start, source.end] {
            self.source_point(at).map_err(|_| DocumentError::InvalidRange {
                start: at, end: at, length: self.source_byte_len(),
            })?;
        }
        if range.start > range.end || source.start > source.end {
            return Err(DocumentError::InvalidRange {
                start: range.start, end: range.end, length: self.projection().text_tree().byte_len(),
            });
        }
        if range.is_empty() {
            return Ok(super::source_edit::insertion_point(self.projection(), range.start, None)
                .is_some_and(|at| source.start <= at && at <= source.end));
        }
        let completed = super::source_edit::complete_contributors(
            self.projection(), &TextEdit::new(range.clone(), ""),
        )?;
        let spans = self.projection().provenance_for_region(&completed.range);
        let mut covered = range.start;
        for span in spans {
            if span.formatted.is_empty() { continue; }
            if span.source.is_empty()
                || span.source.start < source.start || source.end < span.source.end
                || covered < span.formatted.start
            { return Ok(false); }
            covered = covered.max(span.formatted.end);
        }
        Ok(covered >= range.end)
    }

    /// An explicitly physical endpoint names its source boundary. A visual
    /// endpoint is resolved inward through its own exact contributor boundary;
    /// a recovered nearby boundary is never sufficient for source mutation.
    pub fn addressed_source_range(
        &self,
        text: Range<usize>,
        physical_start: Option<usize>,
        physical_end: Option<usize>,
    ) -> Result<Range<usize>, DocumentError> {
        self.text_point(text.start)?;
        self.text_point(text.end)?;
        if text.start > text.end {
            return Err(DocumentError::InvalidRange {
                start: text.start, end: text.end,
                length: self.projection().text_tree().byte_len(),
            });
        }
        let endpoint = |at: usize, physical: Option<usize>, downstream: bool| {
            if let Some(source) = physical {
                self.source_point(source).map_err(|_| DocumentError::InvalidRange {
                    start: source, end: source, length: self.source_byte_len(),
                })?;
                return Ok(source);
            }
            let source = self.projection().source_insertion_point(at, downstream)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let affinity = if downstream { BoundaryAffinity::Downstream }
                else { BoundaryAffinity::Upstream };
            let mapped = self.projection().map_source_boundary(self.revision(), source, affinity)
                .map_err(|_| DocumentError::AmbiguousProjection)?;
            if mapped.formatted_offset != at { return Err(DocumentError::AmbiguousProjection); }
            Ok(source)
        };
        let start = endpoint(text.start, physical_start, true)?;
        let end = endpoint(text.end, physical_end, false)?;
        if start > end { return Err(DocumentError::AmbiguousProjection); }
        Ok(start..end)
    }

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
    fn physical_ranges_read_only_the_delimiter_tail_for_all_encodings() {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let text = format!("one{}two", format.spelling());
                let bytes = encoding.encode_fragment(&text).unwrap();
                let document = Document::from_bytes_with_file_format(bytes, encoding, Format::PlainText, format).unwrap();
                for index in 0..document.physical_line_count().unwrap() {
                    let line = document.physical_line(index).unwrap();
                    assert_eq!(document.physical_line_ranges(index).unwrap(),
                        (line.source_range, line.content_range));
                }
            }
        }
    }

    #[test]
    fn physical_first_nonblank_is_bounded_and_retains_grapheme_and_encoding_boundaries() {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            let source = format!("first\n \t{}\n \t\n", "x".repeat(200_000));
            let document = Document::from_bytes(encoding.encode_fragment(&source).unwrap(),
                encoding, Format::PlainText).unwrap();
            let expected = document.physical_line_ranges(1).unwrap().1.start
                + encoding.encode_fragment(" \t").unwrap().len();
            let (point, work) = measure_document_work(|| document.physical_first_nonblank_source(1).unwrap());
            assert_eq!(point, expected);
            assert_eq!(work.source_full_materializations, 0, "{work:?}");
            assert!(work.source_range_materialized_bytes <= 270, "{work:?}");
            assert!(work.source_decoded_bytes <= 6, "{work:?}");
            assert_eq!(document.physical_first_nonblank_source(2).unwrap(),
                document.physical_line_ranges(2).unwrap().1.start);
        }
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let source = format!("{}\u{301}word", " ".repeat(256));
            let document = Document::from_bytes(encoding.encode_fragment(&source).unwrap(),
                encoding, Format::PlainText).unwrap();
            assert_eq!(document.physical_first_nonblank_source(0).unwrap(),
                encoding.encode_fragment(&" ".repeat(255)).unwrap().len());
        }
        let document = Document::from_bytes(b" \t\xffsuffix".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        assert_eq!(document.physical_first_nonblank_source(0).unwrap(), 2);
        let document = Document::from_bytes(vec![0x20, 0, 0xff], Encoding::Utf16Le, Format::PlainText).unwrap();
        assert_eq!(document.physical_first_nonblank_source(0).unwrap(), 2);
    }

    #[test]
    fn physical_payload_preserves_literal_lf_in_mac_line_content() {
        let document = Document::from_bytes_with_file_format(b"one\ntwo\rthree".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Mac).unwrap();
        let payload = document.physical_source_payload(0..document.source_byte_len()).unwrap();
        assert_eq!(payload.text(), "one\ntwo\nthree");
        assert_eq!(payload.break_offsets(), &[7]);
        assert_eq!(document.physical_line_ranges(0).unwrap(), (0..8, 0..7));
    }

    #[test]
    fn visual_substitution_eligibility_checks_contributors_not_a_source_hull() {
        let document = Document::from_bytes(b"**first**\nsecond".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        assert!(document.formatted_range_within_source(0..5, 0..10).unwrap());
        assert!(!document.formatted_range_within_source(6..12, 0..10).unwrap());
        assert_eq!(document.addressed_source_range(0..8, Some(0), None).unwrap(), 0..12);
    }

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

}
