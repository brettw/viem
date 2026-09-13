use super::encoding::{DecodedText, DecodingDiagnosticKind};
use super::Encoding;
use std::fmt;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileFormat {
    Unix,
    Dos,
    Mac,
}

impl FileFormat {
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            Self::Unix => "\n",
            Self::Dos => "\r\n",
            Self::Mac => "\r",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileFormatOrigin {
    Detected,
    Forced,
    Defaulted,
}

/// Format-independent policy used when interpreting physical line endings at
/// open time.
///
/// This is the shared equivalent of Vim's `fileformats` open policy. Plain
/// text, Markdown, and future textual adapters all pass through this one
/// component before format projection, so adapters never duplicate CR/LF
/// detection rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineEndingOpenPolicy {
    fileformats: Vec<FileFormat>,
    initial_file_format: FileFormat,
    forced_file_format: Option<FileFormat>,
}

impl Default for LineEndingOpenPolicy {
    fn default() -> Self {
        Self {
            // Unix-like Vim default, including macOS. `mac` remains
            // available when callers explicitly add it.
            fileformats: vec![FileFormat::Unix, FileFormat::Dos],
            initial_file_format: FileFormat::Unix,
            forced_file_format: None,
        }
    }
}

impl LineEndingOpenPolicy {
    /// Construct an ordered `fileformats` policy and the fallback used when
    /// that list is empty. Duplicate entries are rejected just as `:set
    /// fileformats=...` rejects them.
    pub fn new(
        fileformats: Vec<FileFormat>,
        initial_file_format: FileFormat,
    ) -> Result<Self, LineEndingPolicyError> {
        for (index, file_format) in fileformats.iter().enumerate() {
            if fileformats[..index].contains(file_format) {
                return Err(LineEndingPolicyError::DuplicateFileFormat(*file_format));
            }
        }
        Ok(Self {
            fileformats,
            initial_file_format,
            forced_file_format: None,
        })
    }

    /// Construct an explicit per-open interpretation. Detection and the
    /// ordered policy are bypassed, while the physical bytes remain unchanged.
    pub fn forced(file_format: FileFormat) -> Self {
        Self {
            forced_file_format: Some(file_format),
            ..Self::default()
        }
    }

    pub fn fileformats(&self) -> &[FileFormat] {
        &self.fileformats
    }

    pub fn initial_file_format(&self) -> FileFormat {
        self.initial_file_format
    }

    pub fn forced_file_format(&self) -> Option<FileFormat> {
        self.forced_file_format
    }

    /// Return this policy with an explicit per-open override.
    pub fn with_forced_file_format(mut self, file_format: FileFormat) -> Self {
        self.forced_file_format = Some(file_format);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineEndingPolicyError {
    DuplicateFileFormat(FileFormat),
}

impl fmt::Display for LineEndingPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateFileFormat(file_format) => {
                write!(formatter, "duplicate file format {file_format:?}")
            }
        }
    }
}

impl std::error::Error for LineEndingPolicyError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineEndingEvidence {
    pub crlf: usize,
    pub bare_lf: usize,
    pub bare_cr: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct NormalizedText {
    pub(crate) text: String,
    pub(crate) units: Vec<LogicalUnit>,
    pub(crate) endings: Vec<LineEnding>,
    pub(crate) encoding: Encoding,
}

#[derive(Clone, Debug)]
pub(crate) struct LogicalUnit {
    pub(crate) normalized: Range<usize>,
    pub(crate) source: Range<usize>,
    pub(crate) decoding_diagnostic: Option<DecodingDiagnosticKind>,
}

#[derive(Clone, Debug)]
pub(crate) struct LineEnding {
    pub(crate) normalized: Range<usize>,
    pub(crate) source: Range<usize>,
    pub(crate) original: FileFormat,
}

pub(crate) fn detect(decoded: &str) -> (FileFormat, LineEndingEvidence) {
    detect_with_policy(decoded, &LineEndingOpenPolicy::default())
}

pub(crate) fn detect_with_policy(
    decoded: &str,
    policy: &LineEndingOpenPolicy,
) -> (FileFormat, LineEndingEvidence) {
    let (file_format, _, evidence) = open_interpretation(decoded, policy);
    (file_format, evidence)
}

pub(crate) fn open_interpretation(
    decoded: &str,
    policy: &LineEndingOpenPolicy,
) -> (FileFormat, FileFormatOrigin, LineEndingEvidence) {
    let evidence = line_ending_evidence(decoded);
    if let Some(file_format) = policy.forced_file_format {
        return (file_format, FileFormatOrigin::Forced, evidence);
    }
    if policy.fileformats.is_empty() {
        return (
            policy.initial_file_format,
            FileFormatOrigin::Defaulted,
            evidence,
        );
    }
    if policy.fileformats.len() == 1 {
        return (policy.fileformats[0], FileFormatOrigin::Defaulted, evidence);
    }

    let permits = |candidate| policy.fileformats.contains(&candidate);
    let has_lf = evidence.crlf > 0 || evidence.bare_lf > 0;
    let all_endings_are_crlf = evidence.crlf > 0 && evidence.bare_lf == 0 && evidence.bare_cr == 0;
    let detected = if all_endings_are_crlf && permits(FileFormat::Dos) {
        Some(FileFormat::Dos)
    } else if permits(FileFormat::Mac) && prefer_mac_interpretation(decoded) {
        Some(FileFormat::Mac)
    } else if has_lf && permits(FileFormat::Unix) {
        Some(FileFormat::Unix)
    } else if evidence.bare_cr > 0 && permits(FileFormat::Mac) {
        Some(FileFormat::Mac)
    } else {
        None
    };
    match detected {
        Some(file_format) => (file_format, FileFormatOrigin::Detected, evidence),
        None => (policy.fileformats[0], FileFormatOrigin::Defaulted, evidence),
    }
}

fn line_ending_evidence(decoded: &str) -> LineEndingEvidence {
    let bytes = decoded.as_bytes();
    let mut evidence = LineEndingEvidence::default();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                evidence.crlf += 1;
                i += 2;
            }
            b'\r' => {
                evidence.bare_cr += 1;
                i += 1;
            }
            b'\n' => {
                evidence.bare_lf += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    evidence
}

/// Vim permits a legacy Mac preference when a bare CR precedes the first LF
/// and CRs dominate an initial bounded sample. Keep this deliberately bounded
/// so choosing an interpretation never needs a second whole-document pass.
fn prefer_mac_interpretation(decoded: &str) -> bool {
    const SAMPLE_BYTES: usize = 64 * 1024;

    let bytes = decoded.as_bytes();
    let sample_end = bytes.len().min(SAMPLE_BYTES);
    let mut first_bare_cr = None;
    let mut first_lf = None;
    let mut bare_cr_count = 0usize;
    let mut lf_count = 0usize;
    let mut offset = 0usize;
    while offset < sample_end {
        match bytes[offset] {
            b'\r' if offset + 1 < sample_end && bytes[offset + 1] == b'\n' => {
                first_lf.get_or_insert(offset + 1);
                lf_count += 1;
                offset += 2;
            }
            b'\r' => {
                first_bare_cr.get_or_insert(offset);
                bare_cr_count += 1;
                offset += 1;
            }
            b'\n' => {
                first_lf.get_or_insert(offset);
                lf_count += 1;
                offset += 1;
            }
            _ => offset += 1,
        }
    }
    first_bare_cr.is_some_and(|cr| first_lf.map_or(true, |lf| cr < lf)) && bare_cr_count > lf_count
}

pub(crate) fn normalize(decoded: &DecodedText, format: FileFormat) -> NormalizedText {
    normalize_with_mapping(decoded, format, false)
}

/// Literal adapters retain bounded identity/conversion runs rather than one
/// record per scalar. Rich parsers can still request the scalar iterator above.
pub(crate) fn normalize_literal(decoded: &DecodedText, format: FileFormat) -> NormalizedText {
    normalize_with_mapping(decoded, format, true)
}

fn normalize_with_mapping(decoded: &DecodedText, format: FileFormat, compact: bool) -> NormalizedText {
    let mut text = String::with_capacity(decoded.text.len());
    let mut units: Vec<LogicalUnit> = Vec::with_capacity(decoded.spans.len());
    let mut endings = Vec::new();
    let mut spans = decoded.scalar_spans();
    let mut previous_regular = false;

    while let Some(span) = spans.next() {
        let offset = span.decoded.start;
        let ch = decoded.text[offset..]
            .chars()
            .next()
            .expect("character boundary");
        let len = ch.len_utf8();
        let is_crlf = ch == '\r' && decoded.text[offset + len..].starts_with('\n');
        let is_ending = match format {
            FileFormat::Unix => ch == '\n',
            FileFormat::Dos => ch == '\n' || is_crlf,
            FileFormat::Mac => ch == '\r',
        };

        if is_ending {
            let source_start = span.source.start;
            let source_end = if is_crlf && format == FileFormat::Dos {
                spans.next().expect("CRLF has a following scalar").source.end
            } else { span.source.end };
            let normalized_start = text.len();
            text.push('\n');
            let normalized = normalized_start..text.len();
            let regular = source_end - source_start == decoded.encoding.scalar_source_width('\n');
            if let Some(previous) = units.last_mut().filter(|previous| {
                compact && previous_regular && regular
                    && text.len() - previous.normalized.start <= super::encoding::MAPPING_CHUNK_BYTES
            }) {
                previous.normalized.end = text.len();
                previous.source.end = source_end;
            } else {
                units.push(LogicalUnit {
                    normalized: normalized.clone(),
                    source: source_start..source_end,
                    decoding_diagnostic: None,
                });
            }
            endings.push(LineEnding {
                normalized,
                source: source_start..source_end,
                original: if is_crlf {
                    FileFormat::Dos
                } else if ch == '\r' {
                    FileFormat::Mac
                } else {
                    FileFormat::Unix
                },
            });
            previous_regular = regular;
            continue;
        }

        let normalized_start = text.len();
        text.push(ch);
        let regular = span.diagnostic.is_none();
        if let Some(previous) = units.last_mut().filter(|previous| {
            compact && previous_regular && regular
                && text.len() - previous.normalized.start <= super::encoding::MAPPING_CHUNK_BYTES
        }) {
            previous.normalized.end = text.len();
            previous.source.end = span.source.end;
        } else {
            units.push(LogicalUnit {
                normalized: normalized_start..text.len(),
                source: span.source,
                decoding_diagnostic: span.diagnostic,
            });
        }
        previous_regular = regular;
    }

    NormalizedText {
        text,
        units,
        endings,
        encoding: decoded.encoding,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn literal_normalization_keeps_long_lines_compact_and_matches_scalar_oracle() {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let text = format!("{}\r\n{}\r{}\n", "éa".repeat(10_000), "b".repeat(10_000), "c".repeat(10_000));
                let bytes = encoding.encode_fragment(&text).unwrap();
                let decoded = encoding.decode(&bytes).unwrap();
                let dense = normalize(&decoded, format);
                let compact = normalize_literal(&decoded, format);
                assert_eq!(compact.text, dense.text);
                assert!(compact.units.len() < 32);
                assert_eq!(compact.endings.iter().map(|e| (&e.normalized, &e.source)).collect::<Vec<_>>(),
                    dense.endings.iter().map(|e| (&e.normalized, &e.source)).collect::<Vec<_>>());
                for run in &compact.units {
                    let start = dense.units.partition_point(|unit| unit.normalized.start < run.normalized.start);
                    let end = dense.units.partition_point(|unit| unit.normalized.end <= run.normalized.end);
                    assert_eq!(dense.units[start].source.start, run.source.start);
                    assert_eq!(dense.units[end - 1].source.end, run.source.end);
                }
            }
        }
    }

    #[test]
    fn detects_dos_only_when_all_breaks_are_crlf() {
        assert_eq!(detect("a\r\nb\r\n").0, FileFormat::Dos);
        assert_eq!(detect("a\r\nb\n").0, FileFormat::Unix);
        // The Unix-like default does not include the legacy `mac` format.
        assert_eq!(detect("a\rb\r").0, FileFormat::Unix);
    }

    #[test]
    fn ordered_open_policy_handles_forced_empty_single_and_fallback_cases() {
        let empty = LineEndingOpenPolicy::new(Vec::new(), FileFormat::Mac).unwrap();
        assert_eq!(detect_with_policy("a\nb", &empty).0, FileFormat::Mac);

        let single = LineEndingOpenPolicy::new(vec![FileFormat::Dos], FileFormat::Unix).unwrap();
        assert_eq!(detect_with_policy("a\nb", &single).0, FileFormat::Dos);

        let fallback =
            LineEndingOpenPolicy::new(vec![FileFormat::Dos, FileFormat::Mac], FileFormat::Unix)
                .unwrap();
        assert_eq!(
            detect_with_policy("no endings", &fallback).0,
            FileFormat::Dos
        );

        let forced = fallback.with_forced_file_format(FileFormat::Unix);
        assert_eq!(
            detect_with_policy("a\r\nb\r\n", &forced).0,
            FileFormat::Unix
        );
    }

    #[test]
    fn multi_format_policy_uses_content_rules_not_list_order() {
        let policy = LineEndingOpenPolicy::new(
            vec![FileFormat::Mac, FileFormat::Dos, FileFormat::Unix],
            FileFormat::Unix,
        )
        .unwrap();
        assert_eq!(detect_with_policy("a\r\nb\r\n", &policy).0, FileFormat::Dos);
        assert_eq!(detect_with_policy("a\r\nb\n", &policy).0, FileFormat::Unix);
        assert_eq!(detect_with_policy("a\rb\r", &policy).0, FileFormat::Mac);
    }

    #[test]
    fn leading_cr_dominance_can_select_legacy_mac_but_is_sample_bounded() {
        let policy =
            LineEndingOpenPolicy::new(vec![FileFormat::Unix, FileFormat::Mac], FileFormat::Unix)
                .unwrap();
        assert_eq!(detect_with_policy("a\rb\rc\nd", &policy).0, FileFormat::Mac);
        assert_eq!(detect_with_policy("a\nb\rc\r", &policy).0, FileFormat::Unix);
    }

    #[test]
    fn policy_rejects_duplicate_entries() {
        assert_eq!(
            LineEndingOpenPolicy::new(
                vec![FileFormat::Unix, FileFormat::Dos, FileFormat::Unix],
                FileFormat::Mac,
            ),
            Err(LineEndingPolicyError::DuplicateFileFormat(FileFormat::Unix))
        );
    }

    #[test]
    fn unix_leaves_cr_as_content() {
        let decoded = Encoding::Utf8.decode(b"a\r\nb").unwrap();
        let normalized = normalize(&decoded, FileFormat::Unix);
        assert_eq!(normalized.text, "a\r\nb");
        assert_eq!(normalized.endings.len(), 1);
        assert_eq!(normalized.endings[0].source, 2..3);
    }

    #[test]
    fn dos_combines_crlf_into_one_logical_break() {
        let decoded = Encoding::Utf8.decode(b"a\r\nb").unwrap();
        let normalized = normalize(&decoded, FileFormat::Dos);
        assert_eq!(normalized.text, "a\nb");
        assert_eq!(normalized.endings[0].source, 1..3);
    }

    #[test]
    fn mac_tags_only_cr_as_a_logical_break_even_when_lf_is_present() {
        let decoded = Encoding::Utf8.decode(b"a\rb\nc").unwrap();
        let normalized = normalize(&decoded, FileFormat::Mac);
        // Both code points are U+000A in the UTF-8 value stream, but only the
        // CR-derived unit is a logical source-line-break token. Downstream
        // adapters must consume `endings`, not infer structure from bytes.
        assert_eq!(normalized.text, "a\nb\nc");
        assert_eq!(normalized.endings.len(), 1);
        assert_eq!(normalized.endings[0].normalized, 1..2);
        assert_eq!(normalized.endings[0].source, 1..2);
        assert_eq!(normalized.units[3].normalized, 3..4);
        assert_eq!(normalized.units[3].source, 3..4);
    }

    #[test]
    fn many_short_lines_normalize_without_per_scalar_linear_lookup() {
        const LINES: usize = 50_000;
        let source = "x\r\n".repeat(LINES);
        let decoded = Encoding::Utf8.decode(source.as_bytes()).unwrap();
        let normalized = normalize(&decoded, FileFormat::Dos);
        assert_eq!(normalized.text.len(), LINES * 2);
        assert_eq!(normalized.endings.len(), LINES);
        assert_eq!(normalized.units.len(), LINES * 2);
        assert_eq!(
            normalized.endings.last().unwrap().source,
            source.len() - 2..source.len()
        );
    }
}
