use super::{DocumentError, Revision};
use std::ops::Range;

/// Encoding of the authoritative textual source artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    Latin1,
    Utf16Le,
    Utf16Be,
}

/// Policy used when a document is created or explicitly changes its BOM.
/// Opening with [`BomPolicy::Preserve`] never changes the original bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BomPolicy {
    Preserve,
    Always,
    Never,
}

/// Why a source byte range could not be decoded as a Unicode scalar value.
///
/// Malformed input is not rejected or normalized away. Each diagnostic is
/// projected as one visible U+FFFD item whose provenance covers the exact
/// opaque source bytes described by the diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodingDiagnosticKind {
    InvalidUtf8Sequence,
    TruncatedUtf8Sequence,
    UnpairedUtf16HighSurrogate,
    UnpairedUtf16LowSurrogate,
    TruncatedUtf16CodeUnit,
}

/// A read-only decoding diagnostic expressed in both source-byte and current
/// formatted-text coordinates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodingDiagnostic {
    pub revision: Revision,
    pub encoding: Encoding,
    pub kind: DecodingDiagnosticKind,
    pub source_range: Range<usize>,
    pub formatted_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct DecodedText {
    pub(crate) text: String,
    pub(crate) spans: Vec<DecodedSpan>,
    pub(crate) bom_len: usize,
    pub(crate) encoding: Encoding,
}

#[derive(Clone, Debug)]
pub(crate) struct DecodedSpan {
    pub(crate) decoded: Range<usize>,
    pub(crate) source: Range<usize>,
    pub(crate) diagnostic: Option<DecodingDiagnosticKind>,
}

impl Encoding {
    /// Select the initial textual encoding without interpreting or rewriting
    /// the authoritative source bytes.
    ///
    /// Supported BOMs take precedence, including when their following payload
    /// is malformed. Without a BOM, a wholly valid UTF-8 source selects UTF-8;
    /// every other byte sequence uses the editor's existing ISO-8859-1
    /// fallback. Deliberately no BOM-less UTF-16 or statistical charset
    /// guessing is performed.
    pub fn detect(bytes: &[u8]) -> Self {
        detected_bom(bytes).unwrap_or_else(|| {
            if std::str::from_utf8(bytes).is_ok() {
                Self::Utf8
            } else {
                Self::Latin1
            }
        })
    }

    /// Detect and construct the initial decoded projection without validating
    /// a valid BOM-less UTF-8 source twice. A malformed BOM-less candidate may
    /// necessarily scan its valid prefix before the whole source is decoded as
    /// Latin-1, but it never constructs and discards a UTF-8 projection.
    pub(crate) fn detect_and_decode(bytes: &[u8]) -> Result<DecodedText, DocumentError> {
        if let Some(encoding) = detected_bom(bytes) {
            return encoding.decode(bytes);
        }
        match std::str::from_utf8(bytes) {
            Ok(valid) => Ok(decode_known_valid_utf8(valid)),
            Err(_) => Ok(decode_latin1(bytes)),
        }
    }

    pub(crate) fn decode(self, bytes: &[u8]) -> Result<DecodedText, DocumentError> {
        match self {
            Self::Utf8 => decode_utf8(bytes, true),
            Self::Latin1 => Ok(decode_latin1(bytes)),
            Self::Utf16Le => decode_utf16(bytes, Endian::Little, true),
            Self::Utf16Be => decode_utf16(bytes, Endian::Big, true),
        }
    }

    /// Decode one source-rope slice whose endpoints were established by the
    /// existing encoding/line-ending projection. A regional slice never owns
    /// the artifact BOM: BOM-looking bytes at its first boundary are ordinary
    /// content, and emitted source ranges remain snapshot-absolute.
    pub(crate) fn decode_region(
        self,
        bytes: &[u8],
        source_origin: usize,
    ) -> Result<DecodedText, DocumentError> {
        let mut decoded = match self {
            Self::Utf8 => decode_utf8(bytes, false)?,
            Self::Latin1 => decode_latin1(bytes),
            Self::Utf16Le => decode_utf16(bytes, Endian::Little, false)?,
            Self::Utf16Be => decode_utf16(bytes, Endian::Big, false)?,
        };
        for span in &mut decoded.spans {
            span.source.start += source_origin;
            span.source.end += source_origin;
        }
        decoded.bom_len = source_origin;
        Ok(decoded)
    }

    pub(crate) fn encode_fragment(self, text: &str) -> Result<Vec<u8>, DocumentError> {
        match self {
            Self::Utf8 => Ok(text.as_bytes().to_vec()),
            Self::Latin1 => {
                let mut result = Vec::with_capacity(text.len());
                for ch in text.chars() {
                    let value = ch as u32;
                    if value > 0xff {
                        return Err(DocumentError::UnrepresentableCharacter {
                            encoding: self,
                            character: ch,
                        });
                    }
                    result.push(value as u8);
                }
                Ok(result)
            }
            Self::Utf16Le | Self::Utf16Be => {
                let mut result = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    let bytes = if self == Self::Utf16Le {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    };
                    result.extend_from_slice(&bytes);
                }
                Ok(result)
            }
        }
    }

    pub(crate) fn bom_bytes(self) -> &'static [u8] {
        match self {
            Self::Utf8 => &[0xef, 0xbb, 0xbf],
            Self::Latin1 => &[],
            Self::Utf16Le => &[0xff, 0xfe],
            Self::Utf16Be => &[0xfe, 0xff],
        }
    }
}

fn detected_bom(bytes: &[u8]) -> Option<Encoding> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some(Encoding::Utf8)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        Some(Encoding::Utf16Le)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some(Encoding::Utf16Be)
    } else {
        None
    }
}

fn decode_known_valid_utf8(valid: &str) -> DecodedText {
    let mut text = String::with_capacity(valid.len());
    let mut spans = Vec::new();
    push_valid_utf8(&mut text, &mut spans, 0, valid);
    DecodedText {
        text,
        spans,
        bom_len: 0,
        encoding: Encoding::Utf8,
    }
}

impl DecodedText {
    /// Map an exact decoded UTF-8 boundary back into source bytes. At the
    /// beginning this deliberately returns the first byte after a BOM.
    pub(crate) fn source_boundary(&self, decoded: usize) -> Option<usize> {
        if decoded == 0 {
            return Some(self.bom_len);
        }
        if decoded == self.text.len() {
            return self
                .spans
                .last()
                .map(|span| span.source.end)
                .or(Some(self.bom_len));
        }
        let following = self
            .spans
            .partition_point(|span| span.decoded.start < decoded);
        if self
            .spans
            .get(following)
            .is_some_and(|span| span.decoded.start == decoded)
        {
            return Some(self.spans[following].source.start);
        }
        following
            .checked_sub(1)
            .and_then(|index| self.spans.get(index))
            .filter(|span| span.decoded.end == decoded)
            .map(|span| span.source.end)
    }
}

fn decode_utf8(bytes: &[u8], recognize_bom: bool) -> Result<DecodedText, DocumentError> {
    let bom_len = usize::from(recognize_bom && bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut source = bom_len;

    while source < bytes.len() {
        match std::str::from_utf8(&bytes[source..]) {
            Ok(valid) => {
                push_valid_utf8(&mut text, &mut spans, source, valid);
                source = bytes.len();
            }
            Err(error) => {
                let valid_len = error.valid_up_to();
                if valid_len != 0 {
                    let valid = std::str::from_utf8(&bytes[source..source + valid_len])
                        .expect("valid_up_to is valid UTF-8");
                    push_valid_utf8(&mut text, &mut spans, source, valid);
                    source += valid_len;
                }

                // `None` means the remaining bytes are a truncated sequence.
                // They form one opaque projected item; otherwise Rust reports
                // the exact ill-formed prefix length for this maximal subpart.
                let (invalid_len, kind) = match error.error_len() {
                    Some(length) => (length, DecodingDiagnosticKind::InvalidUtf8Sequence),
                    None => (
                        bytes.len() - source,
                        DecodingDiagnosticKind::TruncatedUtf8Sequence,
                    ),
                };
                push_opaque(&mut text, &mut spans, source..source + invalid_len, kind);
                source += invalid_len;
            }
        }
    }

    Ok(DecodedText {
        text,
        spans,
        bom_len,
        encoding: Encoding::Utf8,
    })
}

fn push_valid_utf8(
    text: &mut String,
    spans: &mut Vec<DecodedSpan>,
    source_start: usize,
    valid: &str,
) {
    for (relative, ch) in valid.char_indices() {
        let decoded_start = text.len();
        text.push(ch);
        spans.push(DecodedSpan {
            decoded: decoded_start..text.len(),
            source: source_start + relative..source_start + relative + ch.len_utf8(),
            diagnostic: None,
        });
    }
}

fn push_opaque(
    text: &mut String,
    spans: &mut Vec<DecodedSpan>,
    source: Range<usize>,
    kind: DecodingDiagnosticKind,
) {
    let decoded_start = text.len();
    text.push('\u{fffd}');
    spans.push(DecodedSpan {
        decoded: decoded_start..text.len(),
        source,
        diagnostic: Some(kind),
    });
}

fn decode_latin1(bytes: &[u8]) -> DecodedText {
    let mut text = String::with_capacity(bytes.len());
    let mut spans = Vec::with_capacity(bytes.len());
    for (source_start, byte) in bytes.iter().copied().enumerate() {
        let decoded_start = text.len();
        text.push(char::from(byte));
        spans.push(DecodedSpan {
            decoded: decoded_start..text.len(),
            source: source_start..source_start + 1,
            diagnostic: None,
        });
    }
    DecodedText {
        text,
        spans,
        bom_len: 0,
        encoding: Encoding::Latin1,
    }
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn decode_utf16(
    bytes: &[u8],
    endian: Endian,
    recognize_bom: bool,
) -> Result<DecodedText, DocumentError> {
    let expected_bom = match endian {
        Endian::Little => [0xff, 0xfe],
        Endian::Big => [0xfe, 0xff],
    };
    let opposite_bom = match endian {
        Endian::Little => [0xfe, 0xff],
        Endian::Big => [0xff, 0xfe],
    };
    if recognize_bom && bytes.starts_with(&opposite_bom) {
        return Err(DocumentError::MismatchedBom);
    }
    let bom_len = usize::from(recognize_bom && bytes.starts_with(&expected_bom)) * 2;
    let read = |offset: usize| -> u16 {
        let pair = [bytes[offset], bytes[offset + 1]];
        match endian {
            Endian::Little => u16::from_le_bytes(pair),
            Endian::Big => u16::from_be_bytes(pair),
        }
    };
    let encoding = match endian {
        Endian::Little => Encoding::Utf16Le,
        Endian::Big => Encoding::Utf16Be,
    };

    let mut text = String::new();
    let mut spans = Vec::new();
    let mut source = bom_len;
    while source < bytes.len() {
        if source + 1 == bytes.len() {
            push_opaque(
                &mut text,
                &mut spans,
                source..source + 1,
                DecodingDiagnosticKind::TruncatedUtf16CodeUnit,
            );
            source += 1;
            continue;
        }

        let first = read(source);
        let (ch, width) = if (0xd800..=0xdbff).contains(&first) {
            if source + 3 >= bytes.len() || !(0xdc00..=0xdfff).contains(&read(source + 2)) {
                push_opaque(
                    &mut text,
                    &mut spans,
                    source..source + 2,
                    DecodingDiagnosticKind::UnpairedUtf16HighSurrogate,
                );
                source += 2;
                continue;
            }
            let second = read(source + 2);
            let scalar = 0x10000 + (((first as u32 - 0xd800) << 10) | (second as u32 - 0xdc00));
            (char::from_u32(scalar).expect("valid surrogate pair"), 4)
        } else if (0xdc00..=0xdfff).contains(&first) {
            push_opaque(
                &mut text,
                &mut spans,
                source..source + 2,
                DecodingDiagnosticKind::UnpairedUtf16LowSurrogate,
            );
            source += 2;
            continue;
        } else {
            (
                char::from_u32(first as u32).expect("non-surrogate UTF-16 unit"),
                2,
            )
        };

        let decoded_start = text.len();
        text.push(ch);
        spans.push(DecodedSpan {
            decoded: decoded_start..text.len(),
            source: source..source + width,
            diagnostic: None,
        });
        source += width;
    }

    Ok(DecodedText {
        text,
        spans,
        bom_len,
        encoding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_is_iso_8859_1_not_windows_1252() {
        let decoded = Encoding::Latin1.decode(&[0x41, 0x80, 0xe9, 0xff]).unwrap();
        assert_eq!(decoded.text, "A\u{80}\u{e9}\u{ff}");
        assert_eq!(
            Encoding::Latin1.encode_fragment(&decoded.text).unwrap(),
            [0x41, 0x80, 0xe9, 0xff]
        );
    }

    #[test]
    fn detection_is_bom_first_then_strict_utf8_then_latin1() {
        assert_eq!(Encoding::detect(&[]), Encoding::Utf8);
        assert_eq!(Encoding::detect(b"plain UTF-8"), Encoding::Utf8);
        assert_eq!(Encoding::detect("café 😀".as_bytes()), Encoding::Utf8);
        assert_eq!(
            Encoding::detect(&[0xef, 0xbb, 0xbf, 0xff]),
            Encoding::Utf8,
            "a supported BOM owns detection even when its payload is malformed"
        );
        assert_eq!(Encoding::detect(&[0xff, 0xfe, b'A', 0]), Encoding::Utf16Le);
        assert_eq!(Encoding::detect(&[0xfe, 0xff, 0, b'A']), Encoding::Utf16Be);
        assert_eq!(Encoding::detect(b"caf\xe9"), Encoding::Latin1);
        assert_eq!(
            Encoding::detect(&[0xf0, 0x28, 0x8c, 0x28]),
            Encoding::Latin1
        );
        assert_eq!(
            Encoding::detect(&[b'A', 0, b'B', 0]),
            Encoding::Utf8,
            "BOM-less UTF-16 is intentionally not guessed"
        );
    }

    #[test]
    fn utf16_both_endiannesses_and_surrogates() {
        let le = [0xff, 0xfe, 0x41, 0x00, 0x3d, 0xd8, 0x00, 0xde];
        let be = [0xfe, 0xff, 0x00, 0x41, 0xd8, 0x3d, 0xde, 0x00];
        assert_eq!(Encoding::Utf16Le.decode(&le).unwrap().text, "A😀");
        assert_eq!(Encoding::Utf16Be.decode(&be).unwrap().text, "A😀");
    }

    #[test]
    fn malformed_utf8_is_visible_and_keeps_exact_source_extents() {
        let decoded = Encoding::Utf8
            .decode(&[b'a', 0xf0, 0x9f, b'b', 0xe2, 0x82])
            .unwrap();
        assert_eq!(decoded.text, "a\u{fffd}b\u{fffd}");
        let opaque: Vec<_> = decoded
            .spans
            .iter()
            .filter(|span| span.diagnostic.is_some())
            .map(|span| (span.diagnostic.unwrap(), span.source.clone()))
            .collect();
        assert_eq!(
            opaque,
            [
                (DecodingDiagnosticKind::InvalidUtf8Sequence, 1..3),
                (DecodingDiagnosticKind::TruncatedUtf8Sequence, 4..6),
            ]
        );
    }

    #[test]
    fn malformed_utf16_units_are_visible_and_keep_exact_source_extents() {
        // BOM, unpaired high surrogate, A, unpaired low surrogate, odd byte.
        let bytes = [0xff, 0xfe, 0x00, 0xd8, 0x41, 0x00, 0x00, 0xdc, 0xab];
        let decoded = Encoding::Utf16Le.decode(&bytes).unwrap();
        assert_eq!(decoded.text, "\u{fffd}A\u{fffd}\u{fffd}");
        let opaque: Vec<_> = decoded
            .spans
            .iter()
            .filter(|span| span.diagnostic.is_some())
            .map(|span| span.source.clone())
            .collect();
        assert_eq!(opaque, [2..4, 6..8, 8..9]);
    }
}
