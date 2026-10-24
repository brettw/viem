//! Bounded source-column context for a partially selected Preview paragraph.
use super::*;
use unicode_width::UnicodeWidthStr;

impl Document {
    /// Count a physical line's source prefix only until it is already too wide.
    /// Later wrapping decisions cannot distinguish larger columns, and a new
    /// break resets the column. In particular, a caret near the end of a giant
    /// physical line must not make us copy that entire unselected prefix.
    pub(super) fn markdown_reflow_start_column(
        &self,
        source_range: Range<usize>,
        text_width: usize,
        tabstop: usize,
    ) -> Result<usize, DocumentError> {
        const CHUNK_BYTES: usize = 1024;
        const MAX_PREFIX_BYTES: usize = 256 * 1024;
        const MAX_GRAPHEME_BYTES: usize = 16 * 1024;
        if source_range.start > source_range.end || source_range.end > self.source_byte_len() {
            return Err(DocumentError::InvalidRange {
                start: source_range.start,
                end: source_range.end,
                length: self.source_byte_len(),
            });
        }
        if tabstop == 0 {
            return Err(DocumentError::UnsupportedFormatting);
        }
        if source_range.is_empty() {
            return Ok(0);
        }
        let index = self.physical_line_index_at_source(source_range.start)?;
        let (_, content) = self.physical_line_ranges(index)?;
        if source_range.start != content.start || source_range.end > content.end {
            return Err(DocumentError::InvalidRange {
                start: source_range.start,
                end: source_range.end,
                length: content.end,
            });
        }
        let cap = text_width.saturating_add(1);
        let mut column = 0usize;
        let mut at = source_range.start;
        let mut pending = String::new();
        while at < source_range.end {
            if at - source_range.start >= MAX_PREFIX_BYTES {
                // An enormous zero-width prefix (or an enormous configured
                // width) cannot require unbounded foreground work. Reject the
                // command atomically rather than inventing a source column.
                return Err(DocumentError::UnsupportedFormatting);
            }
            let stop = at.saturating_add(CHUNK_BYTES).min(source_range.end)
                .min(source_range.start.saturating_add(MAX_PREFIX_BYTES));
            // Four lookahead bytes cover a UTF-8 scalar or UTF-16 surrogate
            // pair. Decode the bounded window, but consume only complete
            // scalars ending before the nominal chunk edge. A diagnostic in
            // unused lookahead is reconsidered by the next chunk.
            let lookahead = stop.saturating_add(4).min(source_range.end);
            let bytes = self.source_bytes_in(at..lookahead)?;
            let decoded = self.encoding().decode_region(&bytes, at)?;
            let mut consumed_source = at;
            let mut consumed_text = 0;
            for scalar in decoded.scalar_spans() {
                if scalar.source.end > stop {
                    break;
                }
                if scalar.diagnostic.is_some() {
                    return Err(DocumentError::InvalidEncoding {
                        encoding: self.encoding(),
                        offset: scalar.source.start,
                    });
                }
                consumed_source = scalar.source.end;
                consumed_text = scalar.decoded.end;
            }
            if consumed_source == at {
                if stop < source_range.end {
                    return Err(DocumentError::UnsupportedFormatting);
                }
                return Err(DocumentError::InvalidEncoding {
                    encoding: self.encoding(),
                    offset: at,
                });
            }
            pending.push_str(&decoded.text[..consumed_text]);
            at = consumed_source;
            let final_chunk = at == source_range.end;
            let mut consumed = 0;
            let mut graphemes = pending.grapheme_indices(true).peekable();
            while let Some((start, grapheme)) = graphemes.next() {
                if !final_chunk && graphemes.peek().is_none() {
                    // Retain the final cluster until its right boundary is
                    // known. This carries combining marks, ZWJ sequences and
                    // regional-indicator pairing across source chunk seams.
                    break;
                }
                if grapheme.len() > MAX_GRAPHEME_BYTES {
                    return Err(DocumentError::UnsupportedFormatting);
                }
                let advance = if grapheme == "\t" {
                    tabstop - column % tabstop
                } else {
                    UnicodeWidthStr::width(grapheme)
                };
                column = column.saturating_add(advance).min(cap);
                if column == cap {
                    return Ok(cap);
                }
                consumed = start + grapheme.len();
            }
            drop(pending.drain(..consumed));
            if pending.len() > MAX_GRAPHEME_BYTES {
                return Err(DocumentError::UnsupportedFormatting);
            }
        }
        Ok(column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{measure_document_work, Encoding};

    fn document(prefix: &str, encoding: Encoding, bom: bool, later_line: bool) -> (Document, Range<usize>) {
        let mut bytes = if bom {
            match encoding {
                Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
                Encoding::Utf16Le => vec![0xff, 0xfe],
                Encoding::Utf16Be => vec![0xfe, 0xff],
                Encoding::Latin1 => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let text = format!("{}{prefix} selected words", if later_line { "first\n" } else { "" });
        bytes.extend(encoding.encode_fragment(&text).unwrap());
        let document = Document::from_bytes(bytes, encoding, Format::Markdown).unwrap();
        let (_, content) = document.physical_line_ranges(usize::from(later_line)).unwrap();
        let end = content.start + encoding.encoded_text_len(prefix);
        (document, content.start..end)
    }

    fn assert_width(prefix: &str, encoding: Encoding, bom: bool, later_line: bool) {
        let (document, range) = document(prefix, encoding, bom, later_line);
        assert_eq!(
            document.markdown_reflow_start_column(range, 100_000, 4).unwrap(),
            crate::document::indentation::end_column(0, prefix, 4),
            "{encoding:?}, BOM={bom}, later line={later_line}",
        );
    }

    #[test]
    fn source_chunks_keep_multibyte_scalars_and_surrogate_pairs_whole() {
        // Both the UTF-8 scalar and the UTF-16 surrogate pair straddle the
        // nominal 1 KiB source boundary, rather than merely ending there.
        assert_width(&format!("{}é\tend", "a".repeat(1023)), Encoding::Utf8, false, false);
        assert_width(&format!("{}é\tend", "a".repeat(1023)), Encoding::Latin1, false, false);
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
            assert_width(&format!("{}💖\tend", "a".repeat(511)), encoding, false, false);
        }
    }

    #[test]
    fn grapheme_context_survives_source_chunk_seams() {
        for prefix in [
            format!("{}a\u{0301} tail", "a".repeat(1023)),
            format!("{}👩‍💻 tail", "a".repeat(1021)),
            format!("{}🇺🇸🇨🇦 tail", "a".repeat(1020)),
        ] {
            // Measuring chunks independently double-counts pieces of emoji
            // sequences. Compare with the shared whole-string column policy.
            assert_width(&prefix, Encoding::Utf8, false, false);
        }
    }

    #[test]
    fn bom_and_nonzero_line_origins_do_not_become_prefix_columns() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for later_line in [false, true] {
                assert_width("alpha\tβ 👩‍💻", encoding, true, later_line);
            }
        }
    }

    #[test]
    fn an_overwidth_ascii_prefix_never_reads_its_unrelated_suffix() {
        for length in [8192, 1024 * 1024] {
            let (document, range) = document(&"a".repeat(length), Encoding::Utf8, false, false);
            let (column, work) = measure_document_work(|| {
                document.markdown_reflow_start_column(range, 80, 4).unwrap()
            });
            assert_eq!(column, 81);
            assert!(work.source_decoded_bytes <= 1028, "{length}: {work:?}");
            assert!(work.source_range_materialized_bytes <= 1032, "{length}: {work:?}");
            assert_eq!(work.source_full_materializations, 0);
        }
    }

    #[test]
    fn pathological_zero_width_prefixes_and_graphemes_have_explicit_limits() {
        for prefix in ["\u{200b}".repeat(100_000), format!("a{}", "\u{0301}".repeat(9000))] {
            let (document, range) = document(&prefix, Encoding::Utf8, false, false);
            let revision = document.revision();
            let (result, work) = measure_document_work(|| {
                document.markdown_reflow_start_column(range, 100_000, 4)
            });
            assert!(matches!(result, Err(DocumentError::UnsupportedFormatting)));
            assert!(work.source_decoded_bytes <= 258 * 1024, "{work:?}");
            assert_eq!(document.revision(), revision);
        }
    }
}
