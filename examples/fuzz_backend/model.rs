//! Independent flat-string oracle for plain text, physical encodings, and history.
//! None of the expected text, source bytes, or line boundaries use core helpers.
use crate::support::{Recorder, Rng, RunStats};
use viem_core::document::{
    Document, Encoding, FileFormat, Format, HistoryRetentionPolicy, TextEdit,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PhysicalEncoding {
    Utf8,
    Latin1,
    Utf16Le,
    Utf16Be,
}

impl PhysicalEncoding {
    fn core(self) -> Encoding {
        match self {
            Self::Utf8 => Encoding::Utf8,
            Self::Latin1 => Encoding::Latin1,
            Self::Utf16Le => Encoding::Utf16Le,
            Self::Utf16Be => Encoding::Utf16Be,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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

    fn spelling(self) -> &'static str {
        match self {
            Self::Unix => "\n",
            Self::Dos => "\r\n",
            Self::Mac => "\r",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Initial {
    /// Decoded physical source: CR/LF spellings are intentional and preserved.
    source: String,
    encoding: PhysicalEncoding,
    endings: Endings,
    bom: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Splice {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Action {
    Init { initial: Initial },
    Splice { edit: Splice },
    Batch { edits: Vec<Splice> },
    Group { edits: Vec<Splice>, nested: bool },
    Undo,
    Redo,
    MarkSaved,
    Reopen,
}

#[derive(Clone)]
struct State {
    source: String,
    identity: u64,
}

struct Oracle {
    initial: Initial,
    states: Vec<State>,
    current: usize,
    saved: u64,
    next_identity: u64,
}

impl Oracle {
    fn new(initial: Initial) -> Self {
        Self {
            states: vec![State {
                source: initial.source.clone(),
                identity: 0,
            }],
            initial,
            current: 0,
            saved: 0,
            next_identity: 1,
        }
    }

    fn source(&self) -> &str {
        &self.states[self.current].source
    }

    fn commit(&mut self, source: String) {
        self.states.truncate(self.current + 1);
        self.states.push(State {
            source,
            identity: self.next_identity,
        });
        self.next_identity += 1;
        self.current += 1;
    }

    fn text(&self) -> String {
        normalize(self.source(), self.initial.endings).0
    }

    fn bytes(&self, source: &str) -> Result<Vec<u8>, String> {
        encode(source, self.initial.encoding, self.initial.bom)
    }

    fn open(&self, bytes: Vec<u8>) -> Result<Document, String> {
        let mut doc = Document::from_bytes_with_file_format(
            bytes,
            self.initial.encoding.core(),
            Format::PlainText,
            self.initial.endings.core(),
        )
        .map_err(|e| format!("open failed: {e:?}"))?;
        // Retention policy has its own suite: the model compares every retained
        // reference state, including the root, regardless of session length.
        doc.set_history_retention_policy(HistoryRetentionPolicy::unlimited());
        Ok(doc)
    }

    fn check_content(&self, doc: &Document, source: &str) -> Result<(), String> {
        let expected = normalize(source, self.initial.endings).0;
        if doc.text() != expected {
            return Err(format!(
                "formatted text differs: {}",
                difference(expected.as_bytes(), doc.text().as_bytes())
            ));
        }
        let bytes = self.bytes(source)?;
        if doc.source_bytes() != bytes {
            return Err(format!(
                "physical source differs: {}",
                difference(&bytes, &doc.source_bytes())
            ));
        }
        if doc.encoding() != self.initial.encoding.core()
            || doc.file_format() != self.initial.endings.core()
            || doc.has_bom() != self.initial.bom
        {
            return Err("encoding, fileformat, or BOM changed without an edit to metadata".into());
        }
        let lines: Vec<&str> = expected.split('\n').collect();
        if doc.line_count() != lines.len() {
            return Err(format!(
                "line count: expected {}, got {}",
                lines.len(),
                doc.line_count()
            ));
        }
        let mut offset = 0;
        for (index, line) in lines.iter().enumerate() {
            if doc.line_start(index) != Some(offset)
                || doc.line_end(index) != Some(offset + line.len())
            {
                return Err(format!(
                    "line {index} differs: expected {offset}..{}, got {:?}..{:?}",
                    offset + line.len(),
                    doc.line_start(index),
                    doc.line_end(index)
                ));
            }
            offset += line.len() + 1;
        }
        for boundary in boundaries(&expected) {
            doc.text_point(boundary)
                .map_err(|e| format!("legal grapheme boundary {boundary} rejected: {e:?}"))?;
        }
        Ok(())
    }

    fn check(&self, doc: &Document) -> Result<(), String> {
        self.check_content(doc, self.source())?;
        let history = doc.history_status();
        let expected_dirty = self.states[self.current].identity != self.saved;
        if history.can_undo != (self.current > 0)
            || history.can_redo != (self.current + 1 < self.states.len())
            || doc.is_dirty() != expected_dirty
            || history.is_dirty != expected_dirty
        {
            return Err(format!(
                "history differs: expected undo={} redo={} dirty={}, got undo={} redo={} dirty={}",
                self.current > 0,
                self.current + 1 < self.states.len(),
                expected_dirty,
                history.can_undo,
                history.can_redo,
                history.is_dirty
            ));
        }
        Ok(())
    }
}

/// Map each logical Unicode-scalar boundary to its decoded physical boundary.
/// Interior UTF-8 bytes deliberately have no mapping; edit validation below
/// imposes the stronger extended-grapheme rule independently of the backend.
fn normalize(source: &str, endings: Endings) -> (String, Vec<Option<usize>>) {
    let mut text = String::new();
    let mut map = vec![Some(0)];
    let mut offset = 0;
    while offset < source.len() {
        let c = source[offset..].chars().next().unwrap();
        let (emitted, consumed) = match endings {
            Endings::Dos if source[offset..].starts_with("\r\n") => ('\n', 2),
            Endings::Mac if c == '\r' => ('\n', 1),
            _ => (c, c.len_utf8()),
        };
        text.push(emitted);
        offset += consumed;
        map.resize(text.len() + 1, None);
        map[text.len()] = Some(offset);
    }
    (text, map)
}

fn encode(source: &str, encoding: PhysicalEncoding, bom: bool) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(match encoding {
            PhysicalEncoding::Utf8 => &[0xef, 0xbb, 0xbf],
            PhysicalEncoding::Utf16Le => &[0xff, 0xfe],
            PhysicalEncoding::Utf16Be => &[0xfe, 0xff],
            PhysicalEncoding::Latin1 => return Err("Latin1 has no BOM".into()),
        });
    }
    match encoding {
        PhysicalEncoding::Utf8 => bytes.extend_from_slice(source.as_bytes()),
        PhysicalEncoding::Latin1 => {
            for c in source.chars() {
                bytes.push(
                    u8::try_from(c as u32)
                        .map_err(|_| format!("unrepresentable Latin1 scalar {c:?}"))?,
                );
            }
        }
        PhysicalEncoding::Utf16Le | PhysicalEncoding::Utf16Be => {
            for unit in source.encode_utf16() {
                bytes.extend_from_slice(&match encoding {
                    PhysicalEncoding::Utf16Le => unit.to_le_bytes(),
                    _ => unit.to_be_bytes(),
                });
            }
        }
    }
    Ok(bytes)
}

fn boundaries(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect()
}

fn apply_reference(
    source: &str,
    endings: Endings,
    edits: &[Splice],
) -> Result<(String, bool), String> {
    let (text, map) = normalize(source, endings);
    let legal = boundaries(&text);
    let mut patches = Vec::new();
    let mut previous_end = None;
    for edit in edits {
        if edit.start > edit.end
            || legal.binary_search(&edit.start).is_err()
            || legal.binary_search(&edit.end).is_err()
        {
            return Err(format!(
                "invalid reference grapheme range {}..{}",
                edit.start, edit.end
            ));
        }
        if previous_end.is_some_and(|end| edit.start < end) {
            return Err("reference batch contains overlapping or unsorted ranges".into());
        }
        previous_end = Some(edit.end);
        // A semantic no-op must preserve the source's original delimiter spelling.
        if text[edit.start..edit.end] == edit.text {
            continue;
        }
        patches.push((
            map[edit.start].unwrap()..map[edit.end].unwrap(),
            edit.text.replace('\n', endings.spelling()),
        ));
    }
    let changed = !patches.is_empty();
    let mut result = source.to_owned();
    for (range, replacement) in patches.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    Ok((result, changed))
}

fn difference(expected: &[u8], actual: &[u8]) -> String {
    let index = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    format!(
        "first byte {index}; expected len={} {:?}; actual len={} {:?}",
        expected.len(),
        &expected[index.saturating_sub(16)..(index + 32).min(expected.len())],
        actual.len(),
        &actual[index.saturating_sub(16)..(index + 32).min(actual.len())]
    )
}

fn payload(rng: &mut Rng, encoding: PhysicalEncoding, max_units: usize) -> String {
    const UNICODE: &[&str] = &[
        "a",
        "z",
        "word",
        " ",
        "  ",
        "\t",
        "\n",
        "\n\n",
        "é",
        "e\u{301}",
        "\u{301}",
        "👩‍💻",
        "🇫🇷",
        "👍🏽",
        "क्‍ष",
        "אבג",
        "العربية",
        "汉字",
        "\0",
        "\u{2028}",
        "—",
        "&",
        "<",
        ">",
        "*",
        "1. ",
    ];
    const LATIN1: &[&str] = &[
        "a", "z", "word", " ", "  ", "\t", "\n", "\n\n", "é", "ö", "ÿ", "\0", "\u{85}", "\u{a0}",
        "&", "<", "*", "1. ",
    ];
    let corpus = if matches!(encoding, PhysicalEncoding::Latin1) {
        LATIN1
    } else {
        UNICODE
    };
    let mut result = String::new();
    for _ in 0..rng.usize(max_units + 1) {
        result.push_str(corpus[rng.usize(corpus.len())]);
    }
    result
}

fn generate_initial(rng: &mut Rng) -> Initial {
    let encoding = [
        PhysicalEncoding::Utf8,
        PhysicalEncoding::Latin1,
        PhysicalEncoding::Utf16Le,
        PhysicalEncoding::Utf16Be,
    ][rng.usize(4)];
    let endings = [Endings::Unix, Endings::Dos, Endings::Mac][rng.usize(3)];
    let size = if rng.chance(1, 16) { 4096 } else { 80 };
    let logical = payload(rng, encoding, size);
    let mut source = String::new();
    for c in logical.chars() {
        if c == '\n' {
            // DOS explicitly accepts mixed CRLF/bare LF. Each untouched
            // delimiter must keep its own spelling through edits and history.
            source.push_str(if matches!(endings, Endings::Dos) && rng.chance(1, 3) {
                "\n"
            } else {
                endings.spelling()
            });
        } else {
            source.push(c);
        }
    }
    Initial {
        source,
        encoding,
        endings,
        bom: !matches!(encoding, PhysicalEncoding::Latin1) && rng.chance(1, 2),
    }
}

fn generate_splice(rng: &mut Rng, text: &str, encoding: PhysicalEncoding) -> Splice {
    let legal = boundaries(text);
    let start_index = rng.usize(legal.len());
    let end_index = if text.len() > 16 * 1024 || rng.chance(2, 3) {
        (start_index + rng.usize(24)).min(legal.len() - 1)
    } else {
        start_index
    };
    let start = legal[start_index];
    let end = legal[end_index];
    let replacement = if rng.chance(1, 8) {
        text[start..end].to_owned()
    } else if text.len() > 16 * 1024 {
        String::new()
    } else {
        payload(rng, encoding, 8)
    };
    Splice {
        start,
        end,
        text: replacement,
    }
}

fn generate_action(rng: &mut Rng, oracle: &Oracle) -> Result<Action, String> {
    let text = oracle.text();
    Ok(match rng.usize(100) {
        0..=12 => Action::Undo,
        13..=21 => Action::Redo,
        22..=24 => Action::MarkSaved,
        25..=29 => Action::Reopen,
        30..=41 => {
            let mut edits: Vec<Splice> = (0..1 + rng.usize(3))
                .map(|_| generate_splice(rng, &text, oracle.initial.encoding))
                .collect();
            edits.sort_by_key(|edit| (edit.start, edit.end));
            let mut end = None;
            edits.retain(|edit| {
                // Avoid duplicate/adjacent insertion ambiguities: each batch
                // range has a distinct untouched grapheme between neighbors.
                if end.is_some_and(|previous| edit.start <= previous) {
                    return false;
                }
                end = Some(edit.end);
                true
            });
            Action::Batch { edits }
        }
        42..=54 => {
            let mut source = oracle.source().to_owned();
            let mut edits = Vec::new();
            for _ in 0..1 + rng.usize(4) {
                let text = normalize(&source, oracle.initial.endings).0;
                let edit = generate_splice(rng, &text, oracle.initial.encoding);
                source =
                    apply_reference(&source, oracle.initial.endings, std::slice::from_ref(&edit))?
                        .0;
                edits.push(edit);
            }
            Action::Group {
                edits,
                nested: rng.chance(1, 2),
            }
        }
        _ => Action::Splice {
            edit: generate_splice(rng, &text, oracle.initial.encoding),
        },
    })
}

fn execute(
    action: &Action,
    oracle: &mut Oracle,
    doc: &mut Document,
    stats: &mut RunStats,
) -> Result<(), String> {
    match action {
        Action::Init { .. } => return Err("Init may only be the first action".into()),
        Action::Splice { edit } => {
            let (source, changed) = apply_reference(
                oracle.source(),
                oracle.initial.endings,
                std::slice::from_ref(edit),
            )?;
            doc.replace(edit.start..edit.end, &edit.text)
                .map_err(|e| format!("valid splice failed: {e:?}"))?;
            if changed {
                oracle.commit(source);
            }
        }
        Action::Batch { edits } => {
            let (source, changed) =
                apply_reference(oracle.source(), oracle.initial.endings, edits)?;
            doc.apply_edits(
                edits
                    .iter()
                    .map(|edit| TextEdit::new(edit.start..edit.end, edit.text.clone()))
                    .collect(),
            )
            .map_err(|e| format!("valid batch failed: {e:?}"))?;
            if changed {
                oracle.commit(source);
            }
        }
        Action::Group { edits, nested } => {
            let mut source = oracle.source().to_owned();
            let mut changed = false;
            doc.begin_edit_group();
            if *nested {
                doc.begin_edit_group();
            }
            for edit in edits {
                let (next, edit_changed) =
                    apply_reference(&source, oracle.initial.endings, std::slice::from_ref(edit))?;
                source = next;
                changed |= edit_changed;
                doc.replace(edit.start..edit.end, &edit.text)
                    .map_err(|e| format!("valid grouped splice failed: {e:?}"))?;
                oracle.check_content(doc, &source)?;
                stats.max_source_bytes = stats.max_source_bytes.max(doc.source_bytes().len());
            }
            if *nested {
                doc.end_edit_group();
            }
            doc.end_edit_group();
            if changed {
                oracle.commit(source);
            }
        }
        Action::Undo => {
            let available = oracle.current > 0;
            if doc.undo() != available {
                return Err(format!("undo availability differs: expected {available}"));
            }
            if available {
                oracle.current -= 1;
            } else {
                stats.expected_rejections += 1;
            }
        }
        Action::Redo => {
            let available = oracle.current + 1 < oracle.states.len();
            if doc.redo() != available {
                return Err(format!("redo availability differs: expected {available}"));
            }
            if available {
                oracle.current += 1;
            } else {
                stats.expected_rejections += 1;
            }
        }
        Action::MarkSaved => {
            oracle.saved = oracle.states[oracle.current].identity;
            doc.mark_saved();
        }
        Action::Reopen => {
            let reopened = oracle.open(doc.source_bytes())?;
            oracle.check_content(&reopened, oracle.source())?;
            if reopened.is_dirty()
                || reopened.history_status().can_undo
                || reopened.history_status().can_redo
            {
                return Err("reopened document has edit history or is dirty".into());
            }
        }
    }
    oracle.check(doc)?;
    stats.actions += 1;
    stats.max_source_bytes = stats.max_source_bytes.max(doc.source_bytes().len());
    Ok(())
}

pub fn run(
    seed: u64,
    steps: usize,
    replay: Option<&[Value]>,
    recorder: &mut Recorder,
) -> Result<RunStats, String> {
    let mut rng = Rng::new(seed);
    let actions = replay
        .map(|values| {
            values
                .iter()
                .cloned()
                .map(serde_json::from_value::<Action>)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(|e| format!("invalid model replay: {e}"))?;
    let init = match actions.as_ref() {
        Some(actions) => actions.first().cloned().ok_or("empty model replay")?,
        None => Action::Init {
            initial: generate_initial(&mut rng),
        },
    };
    recorder.record(&init)?;
    let Action::Init { initial } = init else {
        return Err("model replay must begin with Init".into());
    };
    let mut oracle = Oracle::new(initial);
    let mut doc = oracle.open(oracle.bytes(oracle.source())?)?;
    oracle.check(&doc)?;
    let mut stats = RunStats {
        actions: 1,
        max_source_bytes: doc.source_bytes().len(),
        ..RunStats::default()
    };
    if let Some(actions) = actions {
        for action in actions.iter().skip(1) {
            recorder.record(action)?;
            execute(action, &mut oracle, &mut doc, &mut stats)?;
        }
    } else {
        for _ in 0..steps {
            let action = generate_action(&mut rng, &oracle)?;
            recorder.record(&action)?;
            execute(&action, &mut oracle, &mut doc, &mut stats)?;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_source_mapping_retains_mixed_delimiters_and_encodes_utf16() {
        let source = "é\r\n👩‍💻\nlast";
        let text = "é\n👩‍💻\nlast";
        assert_eq!(normalize(source, Endings::Dos).0, text);
        let (edited, changed) = apply_reference(
            source,
            Endings::Dos,
            &[Splice {
                start: 0,
                end: 2,
                text: "z\n".into(),
            }],
        )
        .unwrap();
        assert!(changed);
        assert_eq!(edited, "z\r\n\r\n👩‍💻\nlast");
        assert_eq!(
            encode("A😀", PhysicalEncoding::Utf16Le, true).unwrap(),
            vec![0xff, 0xfe, 0x41, 0, 0x3d, 0xd8, 0, 0xde]
        );
        assert_eq!(
            encode("A😀", PhysicalEncoding::Utf16Be, true).unwrap(),
            vec![0xfe, 0xff, 0, 0x41, 0xd8, 0x3d, 0xde, 0]
        );
        let (unchanged, changed) = apply_reference(
            source,
            Endings::Dos,
            &[Splice {
                start: 0,
                end: text.len(),
                text: text.into(),
            }],
        )
        .unwrap();
        assert!(!changed);
        assert_eq!(unchanged, source);
    }
}
