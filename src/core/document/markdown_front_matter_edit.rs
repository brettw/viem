//! Preview edits cannot acquire a new metadata envelope accidentally. A space
//! before the opening rule preserves Markdown's existing block interpretation.
use super::{apply_source_patches, replacement::PatchComposition, validate_source_patches,
    Document, DocumentError, FileFormat, Format, ModelTransactionError, SourcePatch};
use crate::document::{source::SourceSnapshot, Encoding};
use std::collections::BTreeSet;

impl Document {
    pub(super) fn prevent_accidental_front_matter(
        &self,
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        if self.format() != Format::Markdown || patches.is_empty() { return Ok(()); }
        let had_metadata = self.projection().blocks_for_region(&(0..0)).iter()
            .any(|block| block.front_matter);
        let first_line = self.state().source_hard_lines.get(0)
            .ok_or(DocumentError::AmbiguousProjection)?;
        if had_metadata && patches.iter().all(|patch| patch.range.start >= first_line.end) {
            return Ok(());
        }
        validate_source_patches(patches)?;
        let candidate = apply_source_patches(&self.state().source, patches)?;
        let encoding = self.encoding();
        let format = self.file_format();
        let bom = encoding.bom_bytes();
        let content_start = if !bom.is_empty()
            && candidate.bytes_in(0..bom.len().min(candidate.len())).as_deref() == Some(bom)
        { bom.len() } else { 0 };
        let Some((body_start, true)) = delimiter_end(&candidate, content_start, encoding, format)
            else { return Ok(()); };
        let old_opener = delimiter_end(&self.state().source,
            self.projection().source_content_start(), encoding, format)
            .is_some_and(|(_, terminated)| terminated);
        let closes = if !had_metadata && old_opener {
            // No old metadata plus a valid old opener proves that no old row
            // closes it. Only changed rows can supply a closer; never scan an
            // unrelated suffix after an ordinary edit in an unclosed document.
            let mut starts = BTreeSet::new();
            let mut delta = 0isize;
            for patch in patches.iter() {
                let old_line = self.state().source_hard_lines.line_at_offset(patch.range.start)
                    .and_then(|index| self.state().source_hard_lines.get(index))
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if let Some(start) = map_retained_line_start(old_line.start, patches) {
                    starts.insert(start);
                }
                let start = patch.range.start.checked_add_signed(delta)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let end = start.checked_add(patch.replacement.len())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                // Include the code-unit seams around a replacement. Existing
                // UTF-16 repair can replace a formerly dangling half-unit.
                let width = unit_width(encoding);
                let scan_start = start.saturating_sub(width - 1).max(content_start);
                let scan_start = scan_start - (scan_start - content_start) % width;
                let scan_end = end.saturating_add(width - 1).min(candidate.len());
                for (at, unit) in EncodedUnits::new(&candidate, scan_start, scan_end, encoding) {
                    if is_ending(unit, format) { starts.insert(at + width); }
                }
                delta += patch.replacement.len() as isize - patch.range.len() as isize;
            }
            starts.into_iter().any(|start| start >= body_start
                && is_line_start(&candidate, start, content_start, encoding, format)
                && delimiter_end(&candidate, start, encoding, format).is_some())
        } else {
            // A newly exposed first rule creates a global grammar dependency.
            // Stream the source with finite scratch storage only in this case.
            has_closer(&candidate, body_start, encoding, format)
        };
        if !closes { return Ok(()); }
        let mut composition = PatchComposition::new(self.source_byte_len());
        for patch in patches.iter().rev() { composition.splice(patch.range.clone(), &patch.replacement); }
        composition.splice(content_start..content_start, &encoding.encode_fragment(" ")?);
        *patches = composition.source_patches();
        Ok(())
    }
}

fn unit_width(encoding: Encoding) -> usize {
    if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) { 2 } else { 1 }
}

fn is_ending(unit: u16, format: FileFormat) -> bool {
    unit == if format == FileFormat::Mac { b'\r' as u16 } else { b'\n' as u16 }
}

/// The first three units must match before reading any potentially long row.
/// A closer may end at EOF; an opener must own a physical line ending.
fn delimiter_end(source: &SourceSnapshot, start: usize, encoding: Encoding,
    format: FileFormat) -> Option<(usize, bool)> {
    let marker = encoding.encode_fragment("---").ok()?;
    let prefix_end = start.checked_add(marker.len())?;
    if source.bytes_in(start..prefix_end).as_deref() != Some(marker.as_slice()) { return None; }
    let width = unit_width(encoding);
    let mut units = EncodedUnits::new(source, prefix_end, source.len(), encoding);
    units.chunk_limit = 64;
    while let Some((at, unit)) = units.next() {
        if matches!(unit, 32 | 9) { continue; }
        if is_ending(unit, format) { return Some((at + width, true)); }
        if format == FileFormat::Dos && unit == b'\r' as u16 {
            return units.next().filter(|(_, unit)| *unit == b'\n' as u16)
                .map(|(at, _)| (at + width, true));
        }
        return None;
    }
    Some((source.len(), false))
}

fn has_closer(source: &SourceSnapshot, start: usize, encoding: Encoding, format: FileFormat) -> bool {
    if delimiter_end(source, start, encoding, format).is_some() { return true; }
    let width = unit_width(encoding);
    EncodedUnits::new(source, start, source.len(), encoding).any(|(at, unit)|
        is_ending(unit, format) && delimiter_end(source, at + width, encoding, format).is_some())
}

fn is_line_start(source: &SourceSnapshot, at: usize, content_start: usize,
    encoding: Encoding, format: FileFormat) -> bool {
    let width = unit_width(encoding);
    if at < content_start + width || at > source.len() || (at - content_start) % width != 0 { return false; }
    EncodedUnits::new(source, at - width, at, encoding).next()
        .is_some_and(|(_, unit)| is_ending(unit, format))
}

fn map_retained_line_start(at: usize, patches: &[SourcePatch]) -> Option<usize> {
    let mut delta = 0isize;
    for patch in patches {
        if patch.range.start < at && at < patch.range.end { return None; }
        if patch.range.end <= at && patch.range.start < at {
            delta += patch.replacement.len() as isize - patch.range.len() as isize;
        }
    }
    at.checked_add_signed(delta)
}

/// ASCII recognition over encoded source units needs neither whole-source
/// decoding nor replacement of malformed bytes. Scratch storage stays 4 KiB.
struct EncodedUnits<'a> {
    source: &'a SourceSnapshot,
    at: usize,
    end: usize,
    encoding: Encoding,
    bytes: Vec<u8>,
    index: usize,
    chunk_limit: usize,
}
impl<'a> EncodedUnits<'a> {
    fn new(source: &'a SourceSnapshot, at: usize, end: usize, encoding: Encoding) -> Self {
        Self { source, at, end, encoding, bytes: Vec::new(), index: 0, chunk_limit: 4096 }
    }
}
impl Iterator for EncodedUnits<'_> {
    type Item = (usize, u16);
    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.end { return None; }
        if self.index >= self.bytes.len() {
            self.bytes = self.source.bytes_in(self.at..self.at.saturating_add(self.chunk_limit).min(self.end))?;
            self.index = 0;
        }
        let width = unit_width(self.encoding);
        let at = self.at;
        let byte = self.bytes[self.index];
        let unit = match self.encoding {
            Encoding::Utf16Le | Encoding::Utf16Be => match self.bytes.get(self.index + 1) {
                Some(&next) if self.encoding == Encoding::Utf16Le => u16::from_le_bytes([byte, next]),
                Some(&next) => u16::from_be_bytes([byte, next]),
                None => u16::MAX,
            },
            _ => byte as u16,
        };
        self.at += width;
        self.index += width;
        Some((at, unit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accidental_envelopes_preserve_encoded_boms_and_endings() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                for bom in [false, true] {
                    if bom && encoding == Encoding::Latin1 { continue; }
                    for (source, removed) in [
                        ("o---\n\nname: example\n---", "o"),
                        ("---\n\n```yaml\n----\n```\n\nTail", "----"),
                        ("---\nname: example\n---\n\n---\n\nTail\n\n---", "---\nname: example\n---\n\n"),
                    ] {
                        let source = source.replace('\n', format.spelling());
                        let removed = removed.replace('\n', format.spelling());
                        let body = encoding.encode_fragment(&source).unwrap();
                        let mut original = if bom { encoding.bom_bytes().to_vec() } else { Vec::new() };
                        let content_start = original.len();
                        original.extend_from_slice(&body);
                        let document = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, format).unwrap();
                        let at = source.find(&removed).unwrap();
                        let start = content_start + encoding.encode_fragment(&source[..at]).unwrap().len();
                        let end = start + encoding.encode_fragment(&removed).unwrap().len();
                        let replacement = if removed == "----" { "---" } else { "" };
                        let mut patches = vec![SourcePatch::primary(start..end, encoding.encode_fragment(replacement).unwrap())];
                        document.prevent_accidental_front_matter(&mut patches).unwrap();
                        let candidate = apply_source_patches(&document.state().source, &patches).unwrap();
                        let mut expected_body = source.clone();
                        expected_body.replace_range(at..at + removed.len(), replacement);
                        expected_body.insert(0, ' ');
                        let mut expected = original[..content_start].to_vec();
                        expected.extend(encoding.encode_fragment(&expected_body).unwrap());
                        assert_eq!(candidate.bytes(), expected, "{encoding:?} {format:?} bom={bom}");
                        let reopened = Document::from_bytes_with_file_format(expected, encoding, Format::Markdown, format).unwrap();
                        assert!(!reopened.projection().blocks_for_region(&(0..0)).iter().any(|block| block.front_matter));
                        assert_eq!(document.source_bytes(), original);
                    }
                }
            }
        }
    }

    #[test]
    fn checking_unclosed_metadata_reads_only_changed_rows() {
        let source = format!("---\n\n{}", "Unrelated prose\n\n".repeat(100_000));
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = source.len() - "prose\n\n".len();
        let mut patches = vec![SourcePatch::primary(at..at + 5, b"words".to_vec())];
        let before = patches.clone();
        let (result, work) = crate::document::measure_document_work(|| document.prevent_accidental_front_matter(&mut patches));
        result.unwrap();
        assert_eq!(patches, before);
        assert_eq!(work.source_full_materializations, 0);
        assert!(work.source_range_materialized_bytes < 1024, "{work:?}");
    }
}
