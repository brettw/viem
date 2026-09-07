//! Source-preserving projection oracle. Trace offsets belong to the snapshot
//! produced by the preceding action; replay never generates a new random choice.
use crate::support::{Recorder, Rng, RunStats};
use evim_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, ModelRequest, ModelTransactionError,
    SemanticInlineStyle, StyleApplication, StyleSpan, StyleTransactionError, TextEdit,
};
use evim_core::layout::DocumentLayoutStyles;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceFormat {
    Plain,
    Markdown,
    MarkdownSource,
    Html,
    HtmlSource,
    Rtf,
}
impl SourceFormat {
    fn core(self) -> Format {
        match self {
            Self::Plain => Format::PlainText,
            Self::Markdown => Format::Markdown,
            Self::MarkdownSource => Format::MarkdownSource,
            Self::Html => Format::Html,
            Self::HtmlSource => Format::HtmlSource,
            Self::Rtf => Format::Rtf,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceEncoding {
    Utf8,
    Latin1,
    Utf16Le,
    Utf16Be,
}
impl SourceEncoding {
    fn core(self) -> Encoding {
        match self {
            Self::Utf8 => Encoding::Utf8,
            Self::Latin1 => Encoding::Latin1,
            Self::Utf16Le => Encoding::Utf16Le,
            Self::Utf16Be => Encoding::Utf16Be,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Endings {
    Unix,
    Dos,
    Mac,
}
impl Endings {
    fn core(self) -> FileFormat {
        match self {
            Self::Unix => FileFormat::Unix,
            Self::Dos => FileFormat::Dos,
            Self::Mac => FileFormat::Mac,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Init {
        bytes: Vec<u8>,
        format: SourceFormat,
        encoding: SourceEncoding,
        endings: Endings,
    },
    Replace {
        start: usize,
        end: usize,
        text: String,
    },
    SourceReplace {
        start: usize,
        end: usize,
        text: String,
    },
    Style {
        start: usize,
        end: usize,
        strong: bool,
        enabled: bool,
    },
    Undo,
    Redo,
    NoOpFormat,
    Check,
}

#[derive(Clone, Debug, PartialEq)]
struct Saved {
    bytes: Vec<u8>,
    text: String,
    encoding: Encoding,
    format: Format,
    endings: FileFormat,
}
impl Saved {
    fn capture(document: &Document) -> Self {
        Self {
            bytes: document.source_bytes(),
            text: document.text().to_owned(),
            encoding: document.encoding(),
            format: document.format(),
            endings: document.file_format(),
        }
    }
}

#[derive(Default)]
struct Oracle {
    document: Option<Document>,
    history: Vec<Saved>,
    history_at: usize,
}

pub fn run(
    seed: u64,
    steps: usize,
    replay: Option<&[Value]>,
    recorder: &mut Recorder,
) -> Result<RunStats, String> {
    let mut rng = Rng::new(seed);
    let mut oracle = Oracle::default();
    let mut stats = RunStats::default();
    for index in 0..replay.map_or(steps, |actions| actions.len()) {
        let action = match replay {
            Some(actions) => serde_json::from_value(actions[index].clone())
                .map_err(|error| format!("projection action {index}: {error}"))?,
            None => generate(&oracle, &mut rng, index),
        };
        recorder.record(&action)?;
        let prior = oracle.document.as_ref().map(Saved::capture);
        let rejected = oracle.execute(&action).map_err(|error| {
            format!(
                "projection action {index} {action:?}: {error}\nprior snapshot: {}",
                format!("{prior:?}").chars().take(5000).collect::<String>()
            )
        })?;
        stats.actions += 1;
        stats.expected_rejections += usize::from(rejected);
        if let Some(document) = &oracle.document {
            stats.max_source_bytes = stats.max_source_bytes.max(document.source_byte_len());
        }
    }
    Ok(stats)
}

impl Oracle {
    fn execute(&mut self, action: &Action) -> Result<bool, String> {
        if let Action::Init {
            bytes,
            format,
            encoding,
            endings,
        } = action
        {
            let document = Document::from_bytes_with_file_format(
                bytes.clone(),
                encoding.core(),
                format.core(),
                endings.core(),
            )
            .map_err(|error| format!("opening preserved fixture: {error:?}"))?;
            same("no-op source identity", &document.source_bytes(), bytes)?;
            self.history = vec![Saved::capture(&document)];
            self.history_at = 0;
            self.document = Some(document);
        } else {
            let document = self.document.as_mut().ok_or("trace must begin with init")?;
            let before = Saved::capture(document);
            let revision = document.revision();
            match action {
                Action::Undo | Action::Redo => {
                    let undo = matches!(action, Action::Undo);
                    let expected = if undo {
                        self.history_at > 0
                    } else {
                        self.history_at + 1 < self.history.len()
                    };
                    same(
                        "history availability",
                        &(if undo {
                            document.undo()
                        } else {
                            document.redo()
                        }),
                        &expected,
                    )?;
                    if expected {
                        if undo {
                            self.history_at -= 1;
                        } else {
                            self.history_at += 1;
                        }
                    }
                    same(
                        "exact history restoration",
                        &Saved::capture(document),
                        &self.history[self.history_at],
                    )?;
                }
                Action::Check => {}
                _ => {
                    let request = match action {
                        Action::Replace { start, end, text } => ModelRequest::ApplyTextEdits {
                            document: document.id(),
                            revision,
                            edits: vec![TextEdit::new(*start..*end, text.clone())],
                        },
                        Action::SourceReplace { start, end, text } => {
                            ModelRequest::ReplacePhysicalSource {
                                document: document.id(),
                                revision,
                                range: *start..*end,
                                replacement: text.clone(),
                            }
                        }
                        Action::Style {
                            start,
                            end,
                            strong,
                            enabled,
                        } => ModelRequest::SetSemanticStyle {
                            document: document.id(),
                            revision,
                            range: *start..*end,
                            style: if *strong {
                                SemanticInlineStyle::Strong
                            } else {
                                SemanticInlineStyle::Emphasis
                            },
                            enabled: *enabled,
                        },
                        Action::NoOpFormat => ModelRequest::SetFormat {
                            document: document.id(),
                            revision,
                            target: document.format(),
                        },
                        _ => unreachable!(),
                    };
                    let prepared = match document.prepare_model_request(request) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            same(
                                "rejected edit source/text atomicity",
                                &Saved::capture(document),
                                &before,
                            )?;
                            same(
                                "rejected edit revision atomicity",
                                &document.revision(),
                                &revision,
                            )?;
                            if !expected_rejection(&error) {
                                return Err(format!("unexpected model failure: {error:?}"));
                            }
                            compare_fresh(document)?;
                            return Ok(true);
                        }
                    };
                    // The patch audit is independent of the piece tree publication.
                    // Any byte not covered by a declared patch must remain identical.
                    let mut expected_bytes = Vec::new();
                    let mut frontier = 0;
                    for patch in prepared.summary().source_patches() {
                        let range = patch.range();
                        if range.start < frontier
                            || range.end < range.start
                            || range.end > before.bytes.len()
                        {
                            return Err(format!(
                                "invalid/overlapping declared source patch {range:?}"
                            ));
                        }
                        expected_bytes.extend_from_slice(&before.bytes[frontier..range.start]);
                        expected_bytes.extend_from_slice(patch.replacement());
                        frontier = range.end;
                    }
                    expected_bytes.extend_from_slice(&before.bytes[frontier..]);
                    document
                        .commit_model_transaction(prepared)
                        .map_err(|error| format!("prepared commit failed: {error:?}"))?;
                    same(
                        "declared source patch locality",
                        &document.source_bytes(),
                        &expected_bytes,
                    )?;
                    if let Action::Replace { start, end, text } = action {
                        let mut expected = before.text.clone();
                        expected.replace_range(*start..*end, text);
                        same(
                            "requested formatted replacement",
                            &document.text(),
                            &expected.as_str(),
                        )?;
                    }
                    if let Action::SourceReplace { start, end, text } = action {
                        let spelling = match before.endings {
                            FileFormat::Unix => "\n",
                            FileFormat::Dos => "\r\n",
                            FileFormat::Mac => "\r",
                        };
                        let text = text.replace('\n', spelling);
                        let encoded: Vec<u8> = match before.encoding {
                            Encoding::Utf8 => text.into_bytes(),
                            Encoding::Latin1 => text
                                .chars()
                                .map(|ch| {
                                    u8::try_from(ch as u32)
                                        .expect("successful Latin1 request is representable")
                                })
                                .collect(),
                            Encoding::Utf16Le => {
                                text.encode_utf16().flat_map(u16::to_le_bytes).collect()
                            }
                            Encoding::Utf16Be => {
                                text.encode_utf16().flat_map(u16::to_be_bytes).collect()
                            }
                        };
                        let mut expected = before.bytes.clone();
                        expected.splice(*start..*end, encoded);
                        same(
                            "requested physical source replacement",
                            &document.source_bytes(),
                            &expected,
                        )?;
                    }
                    if matches!(action, Action::NoOpFormat) {
                        same(
                            "no-op format source/text identity",
                            &Saved::capture(document),
                            &before,
                        )?;
                        same(
                            "no-op format revision identity",
                            &document.revision(),
                            &revision,
                        )?;
                    }
                    if document.revision() != revision {
                        self.history.truncate(self.history_at + 1);
                        self.history.push(Saved::capture(document));
                        self.history_at += 1;
                    }
                }
            }
        }
        compare_fresh(self.document.as_ref().unwrap())?;
        Ok(false)
    }
}

fn expected_rejection(error: &ModelTransactionError) -> bool {
    matches!(
        error,
        ModelTransactionError::Document(
            DocumentError::AmbiguousProjection
                | DocumentError::UnsupportedFormatting
                | DocumentError::OverlappingFormatting
                | DocumentError::FormattedPayloadCannotReproject
                | DocumentError::UnrepresentableCharacter { .. }
                | DocumentError::UnrepresentableFormattedCharacter { .. }
                | DocumentError::OpaqueDecodingConflict { .. }
        ) | ModelTransactionError::Style(StyleTransactionError::Unsupported { .. })
    )
}

fn compare_fresh(document: &Document) -> Result<(), String> {
    let bytes = document.source_bytes();
    let fresh = Document::from_bytes_with_file_format(
        bytes.clone(),
        document.encoding(),
        document.format(),
        document.file_format(),
    )
    .map_err(|error| format!("fresh reopen failed: {error:?}"))?;
    same("fresh byte identity", &fresh.source_bytes(), &bytes)?;
    same("incremental/fresh text", &document.text(), &fresh.text())?;
    let left = document.projection();
    let right = fresh.projection();
    let blocks = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .cloned()
            .map(|mut block| {
                block.id = 0;
                block
            })
            .collect::<Vec<_>>()
    };
    same(
        "incremental/fresh paragraph structure",
        &blocks(document),
        &blocks(&fresh),
    )?;
    let lines = |document: &Document| {
        (0..document.projection().hard_line_count())
            .map(|line| document.projection().hard_line_range(line))
            .collect::<Vec<_>>()
    };
    same(
        "incremental/fresh hard lines",
        &lines(document),
        &lines(&fresh),
    )?;
    for flow in [false, true] {
        let ranges = |document: &Document| {
            (0..document.projection().presentation_line_count(flow))
                .map(|line| document.projection().presentation_line_range(line, flow))
                .collect::<Vec<_>>()
        };
        same(
            "incremental/fresh flow boundaries",
            &ranges(document),
            &ranges(&fresh),
        )?;
    }
    // Compare application coverage rather than span segmentation. Regional
    // splicing may legitimately split one equivalent adjacent style run.
    same(
        "incremental/fresh style applications",
        &style_coverage(left.style_spans()),
        &style_coverage(right.style_spans()),
    )?;
    let diagnostic = |document: &Document| {
        document
            .decoding_diagnostics()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.encoding,
                    diagnostic.kind,
                    diagnostic.source_range.clone(),
                    diagnostic.formatted_range.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    same(
        "incremental/fresh decoding diagnostics",
        &diagnostic(document),
        &diagnostic(&fresh),
    )?;
    // Both parsers emit scalar provenance. Empty anchors and hidden syntax
    // boundaries are intentional parts of that relation and must agree too.
    if left.provenance() != right.provenance() {
        let first = left
            .provenance()
            .iter()
            .zip(right.provenance())
            .position(|(a, b)| a != b)
            .unwrap_or(left.provenance().len().min(right.provenance().len()));
        return Err(format!("incremental/fresh provenance at span {first} differs ({} vs {} spans): {:?} vs {:?}; source {:?}",
            left.provenance().len(), right.provenance().len(), left.provenance().get(first), right.provenance().get(first),
            bytes.iter().copied().take(800).collect::<Vec<_>>()));
    }
    let resolved = |document: &Document| -> Result<DocumentLayoutStyles, String> {
        let mut styles = DocumentLayoutStyles::resolve(document.projection())
            .map_err(|error| format!("style resolution: {error:?}"))?;
        styles.style_sheet_revision.0 = 0;
        for paragraph in &mut styles.paragraphs {
            paragraph.block_id = 0;
        }
        Ok(styles)
    };
    same(
        "incremental/fresh resolved layout styles",
        &resolved(document)?,
        &resolved(&fresh)?,
    )?;
    Ok(())
}

fn style_coverage(spans: &[StyleSpan]) -> Vec<(std::ops::Range<usize>, Vec<StyleApplication>)> {
    let mut boundaries = spans
        .iter()
        .flat_map(|span| [span.range.start, span.range.end])
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut result: Vec<(std::ops::Range<usize>, Vec<StyleApplication>)> = Vec::new();
    for pair in boundaries.windows(2) {
        let mut applications = spans
            .iter()
            .filter(|span| span.range.start <= pair[0] && span.range.end >= pair[1])
            .map(|span| span.application.clone())
            .collect::<Vec<_>>();
        // Grammar annotations do not participate in the ordered style cascade.
        // Their relative position beside a direct declaration is immaterial.
        let mut annotations = Vec::new();
        applications.retain(|application| {
            if matches!(
                application,
                StyleApplication::SourceSyntax
                    | StyleApplication::SourceRawText
                    | StyleApplication::SourcePreservedWhitespace
            ) {
                if !annotations.contains(application) {
                    annotations.push(application.clone());
                }
                false
            } else {
                true
            }
        });
        annotations.sort_by_key(|annotation| format!("{annotation:?}"));
        annotations.extend(applications);
        let applications = annotations;
        if applications.is_empty() {
            continue;
        }
        if let Some(previous) = result.last_mut() {
            if previous.0.end == pair[0] && previous.1 == applications {
                previous.0.end = pair[1];
                continue;
            }
        }
        result.push((pair[0]..pair[1], applications));
    }
    result
}

fn same<T: std::fmt::Debug + PartialEq>(
    label: &str,
    actual: &T,
    expected: &T,
) -> Result<(), String> {
    if actual == expected {
        return Ok(());
    }
    let abbreviated = |value: &T| format!("{value:?}").chars().take(1600).collect::<String>();
    Err(format!(
        "{label} differs\nactual: {}\nexpected: {}",
        abbreviated(actual),
        abbreviated(expected)
    ))
}

fn generate(oracle: &Oracle, rng: &mut Rng, index: usize) -> Action {
    if oracle.document.is_none() || index % 40 == 0 {
        return fixture(rng);
    }
    let document = oracle.document.as_ref().unwrap();
    match rng.usize(16) {
        0 => Action::Undo,
        1 => Action::Redo,
        2 => Action::NoOpFormat,
        3 => Action::Check,
        kind => {
            let mut boundaries = document
                .text()
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .collect::<Vec<_>>();
            boundaries.push(document.text().len());
            if kind == 4 && document.format() != Format::Rtf {
                let bytes = document.source_bytes();
                boundaries = match document.encoding() {
                    Encoding::Utf16Le | Encoding::Utf16Be => (0..=bytes.len()).step_by(2).collect(),
                    Encoding::Utf8 => match std::str::from_utf8(&bytes) {
                        Ok(text) => text
                            .char_indices()
                            .map(|(offset, _)| offset)
                            .chain(std::iter::once(bytes.len()))
                            .collect(),
                        Err(_) => (0..=bytes.len()).collect(),
                    },
                    Encoding::Latin1 => (0..=bytes.len()).collect(),
                };
            }
            let from = rng.usize(boundaries.len());
            let to = (from + rng.usize(4)).min(boundaries.len() - 1);
            let (start, end) = (boundaries[from], boundaries[to]);
            if kind == 5 {
                return Action::Style {
                    start,
                    end,
                    strong: rng.chance(1, 2),
                    enabled: rng.chance(1, 2),
                };
            }
            let choices = [
                "",
                "x",
                "word",
                " ",
                "\n",
                "\n\n",
                "é",
                "e\u{301}",
                "猫",
                "👩🏽‍💻",
                "אב",
                "**",
                "_",
                "&amp;",
                "<",
                "}",
                "\\",
                "1. ",
                "- ",
                "\t",
                "\r",
            ];
            let text = choices[rng.usize(choices.len())].to_owned();
            if kind == 4 && document.format() != Format::Rtf {
                Action::SourceReplace { start, end, text }
            } else {
                Action::Replace { start, end, text }
            }
        }
    }
}

fn fixture(rng: &mut Rng) -> Action {
    use SourceFormat::*;
    let fixtures = [
        (Plain, ""),
        (Plain, "one é\ntwo e\u{301}\n👩🏽‍💻 end\n"),
        (
            Markdown,
            "# Title\n\nA **bold** and _italic_ paragraph.\nA continuation.\n\nTail.\n",
        ),
        (
            MarkdownSource,
            "# Title\n\nA **bold** and _italic_ paragraph.\nA continuation.\n\nTail.\n",
        ),
        (
            Markdown,
            "1. first\n   continuation\n2. **second**\n   - nested\n\nTail.",
        ),
        (MarkdownSource, "1. first\n2. second\n3. "),
        (Markdown, "```rs\none\n\n**code**\n```\n\nTail.\n"),
        (MarkdownSource, "```\nline one\n\nline two\n```\n\nTail."),
        (
            Markdown,
            "Malformed *one **two_ [link](unfinished\n\n`code\n",
        ),
        (
            Html,
            "<p>A <b data-keep='yes'>bold</b> &amp; é.</p><!--keep--><p>Tail</p>",
        ),
        (
            HtmlSource,
            "<p>A <b>bold</b> &amp; é.</p>\n<!--keep-->\n<p>Tail</p>",
        ),
        (
            Html,
            "<ol start='3'><li>One<p>Second paragraph</p></li><li><i>Two</i></li></ol><p>Tail</p>",
        ),
        (Html, "<pre>one\n\ntwo &lt; é</pre><p>after<br>break</p>"),
        (
            HtmlSource,
            "<script>if (a < b) x='&amp;';</script><p style='color:red'>Text</p>",
        ),
        (Html, "<p><b>unclosed<i>nest</b> &bogus; <tag x='unfinished"),
        (
            Rtf,
            r"{\rtf1\ansi First {\b bold} and {\i italic}.\par Tail.}",
        ),
        (
            Rtf,
            r"{\rtf1{\fonttbl{\f0 Helvetica;}}{\colortbl;\red128\green30\blue60;}\f0\fs28 Text {\cf1 colored}\line more\par Tail}",
        ),
        (
            Rtf,
            r"{\rtf1{\stylesheet{\s0 Normal;}{\s1\sbasedon0\b Heading1;}{\*\cs2\i Accent;}}\s1 Title\par\s0 Body {\cs2 accent}\par Tail}",
        ),
        (
            Rtf,
            r"{\rtf1\uc1 \u233? {\b word}\par {\*\unknown opaque {nested}} tail",
        ),
    ];
    let (format, source) = fixtures[rng.usize(fixtures.len())];
    let encoding = [
        SourceEncoding::Utf8,
        SourceEncoding::Latin1,
        SourceEncoding::Utf16Le,
        SourceEncoding::Utf16Be,
    ][rng.usize(4)];
    let endings = [Endings::Unix, Endings::Dos, Endings::Mac][rng.usize(3)];
    let source = source.replace(
        '\n',
        match endings {
            Endings::Unix => "\n",
            Endings::Dos => "\r\n",
            Endings::Mac => "\r",
        },
    );
    let mut bytes = match encoding {
        SourceEncoding::Utf8 => source.into_bytes(),
        SourceEncoding::Latin1 => source
            .chars()
            .map(|ch| if (ch as u32) <= 255 { ch as u8 } else { b'?' })
            .collect(),
        SourceEncoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        SourceEncoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    };
    if rng.chance(1, 4) {
        let bom: &[u8] = match encoding {
            SourceEncoding::Utf8 => &[0xef, 0xbb, 0xbf],
            SourceEncoding::Utf16Le => &[0xff, 0xfe],
            SourceEncoding::Utf16Be => &[0xfe, 0xff],
            SourceEncoding::Latin1 => &[],
        };
        bytes.splice(0..0, bom.iter().copied());
    }
    if matches!(format, Plain) && rng.chance(1, 3) {
        bytes.extend_from_slice(match encoding {
            SourceEncoding::Utf8 => &[0xff, 0xe2, 0x82],
            SourceEncoding::Utf16Le => &[0x00, 0xd8, 0x20],
            SourceEncoding::Utf16Be => &[0xd8, 0x00, 0x20],
            SourceEncoding::Latin1 => &[0x80, 0xff],
        });
    }
    Action::Init {
        bytes,
        format,
        encoding,
        endings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_json_round_trip_is_a_complete_replay() {
        let mut first = Oracle::default();
        let mut replayed = Oracle::default();
        let actions = [
            Action::Init {
                bytes: b"<p>word</p><p>tail</p>".to_vec(),
                format: SourceFormat::Html,
                encoding: SourceEncoding::Utf8,
                endings: Endings::Unix,
            },
            Action::Replace {
                start: 1,
                end: 2,
                text: "é".into(),
            },
            Action::Style {
                start: 0,
                end: 1,
                strong: true,
                enabled: true,
            },
            Action::SourceReplace {
                start: 0,
                end: 0,
                text: "<!--preserved-->".into(),
            },
            Action::NoOpFormat,
            Action::Check,
            Action::Undo,
            Action::Redo,
        ];
        for (index, action) in actions.into_iter().enumerate() {
            let json = serde_json::to_value(&action).unwrap();
            let replay = serde_json::from_value(json).unwrap();
            let a = first
                .execute(&action)
                .unwrap_or_else(|error| panic!("{index} {action:?}: {error}"));
            let b = replayed.execute(&replay).unwrap();
            assert_eq!(a, b);
            assert_eq!(
                Saved::capture(first.document.as_ref().unwrap()),
                Saved::capture(replayed.document.as_ref().unwrap())
            );
        }
    }
    #[test]
    fn generation_and_json_are_deterministic_without_suppressing_findings() {
        let mut a = Rng::new(0x5151);
        let mut b = Rng::new(0x5151);
        let mut oracle = Oracle::default();
        oracle
            .execute(&Action::Init {
                bytes: "é猫\nword".as_bytes().to_vec(),
                format: SourceFormat::Plain,
                encoding: SourceEncoding::Utf8,
                endings: Endings::Unix,
            })
            .unwrap();
        // The replay mechanism is tested independently of whether a generated
        // action exposes a product defect. Campaign findings must remain failures.
        for index in 0..120 {
            let first = serde_json::to_value(generate(&oracle, &mut a, index)).unwrap();
            let second = serde_json::to_value(generate(&oracle, &mut b, index)).unwrap();
            assert_eq!(first, second);
            let action: Action = serde_json::from_value(first.clone()).unwrap();
            assert_eq!(serde_json::to_value(action).unwrap(), first);
        }
        assert!(!expected_rejection(&ModelTransactionError::Document(
            DocumentError::VerificationFailed
        )));
    }
    #[test]
    fn oracle_observes_corrupted_expected_source_history() {
        let mut oracle = Oracle::default();
        oracle
            .execute(&Action::Init {
                bytes: b"word".to_vec(),
                format: SourceFormat::Plain,
                encoding: SourceEncoding::Utf8,
                endings: Endings::Unix,
            })
            .unwrap();
        oracle
            .execute(&Action::Replace {
                start: 1,
                end: 2,
                text: "é".into(),
            })
            .unwrap();
        oracle.history[0].bytes[0] = b'X';
        assert!(oracle
            .execute(&Action::Undo)
            .unwrap_err()
            .contains("exact history restoration"));
    }
}
