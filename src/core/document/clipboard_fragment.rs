//! Owned clipboard fragments. Source remains the authority; importing a private
//! payload reparses and verifies it before any destination transaction commits.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// A versioned, immutable portable clipboard representation. Keeping the wire
/// image immutable makes register cloning cheap and avoids retaining a source
/// document or snapshot on a system clipboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardFragment(Arc<str>);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Export {
    schema_version: u32,
    plain_text: String,
    hard_breaks: Vec<usize>,
    register_kind: u32,
    is_rich: bool,
    source_text: String,
    source_bytes: Vec<u8>,
    source_format: u32,
    encoding: u32,
    file_format: u32,
    source_exact: bool,
    source_plain_text: String,
    #[serde(default)]
    inline_source_bytes: Vec<u8>,
    #[serde(default)]
    embedded_source_bytes: Vec<u8>,
    character_runs: Vec<Value>,
    paragraph_runs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_segments: Vec<SourceSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourceSegment {
    start: usize,
    end: usize,
    row: usize,
    fragment: Export,
}

impl ClipboardFragment {
    pub fn json(&self) -> &str {
        &self.0
    }

    pub(crate) fn source_mode_text(&self) -> Option<String> {
        let value: Export = serde_json::from_str(self.json()).expect("authored clipboard fragment");
        (!value.is_rich).then_some(value.source_text)
    }

    pub fn from_json(json: &str, plain_text: &str) -> Result<Self, DocumentError> {
        // A pasteboard is external input, not a trusted in-process register.
        if json.len() > 64 * 1024 * 1024 {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let value: Export =
            serde_json::from_str(json).map_err(|_| DocumentError::UnsupportedFormatting)?;
        if value.schema_version != 1
            || value.plain_text != plain_text
            || !matches!(value.register_kind, 1..=3)
            || value.source_plain_text != value.plain_text
                && !(value.register_kind == 2
                    && value.plain_text == format!("{}\n", value.source_plain_text))
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut last = None;
        for &offset in &value.hard_breaks {
            if value.plain_text.as_bytes().get(offset) != Some(&b'\n')
                || last.is_some_and(|last| last >= offset)
            {
                return Err(DocumentError::UnsupportedFormatting);
            }
            last = Some(offset);
        }
        if value.register_kind == 3 && !value.hard_breaks.is_empty() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        for run in value.character_runs.iter().chain(&value.paragraph_runs) {
            let start = run["start"]
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or(DocumentError::UnsupportedFormatting)?;
            let end = run["end"]
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or(DocumentError::UnsupportedFormatting)?;
            if start > end
                || end > plain_text.len()
                || !plain_text.is_char_boundary(start)
                || !plain_text.is_char_boundary(end)
            {
                return Err(DocumentError::UnsupportedFormatting);
            }
        }
        let (_, encoding, _) = value.pipeline()?;
        if encoding.decode(&value.source_bytes)?.text != value.source_text {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut previous = None;
        for segment in &value.source_segments {
            if value.register_kind != 3
                || segment.start > segment.end
                || plain_text.get(segment.start..segment.end) != Some(&segment.fragment.plain_text)
                || !segment.fragment.source_segments.is_empty()
                || segment.fragment.pipeline()? != value.pipeline()?
                || previous.is_some_and(|(row, end)| row > segment.row || end > segment.start)
                || encoding.decode(&segment.fragment.source_bytes)?.text
                    != segment.fragment.source_text
            {
                return Err(DocumentError::UnsupportedFormatting);
            }
            previous = Some((segment.row, segment.end));
        }
        Ok(Self(Arc::from(json)))
    }

    pub(crate) fn register_parts(&self) -> (String, u32, Vec<usize>) {
        let value: Export =
            serde_json::from_str(self.json()).expect("validated clipboard fragment");
        (value.plain_text, value.register_kind, value.hard_breaks)
    }

    pub(crate) fn as_seen(&self) -> Self {
        let mut value: Export =
            serde_json::from_str(self.json()).expect("authored clipboard fragment");
        value.plain_text = value.source_plain_text.clone();
        if value.register_kind != 3 {
            value.register_kind = 1;
        }
        value
            .hard_breaks
            .retain(|offset| *offset < value.plain_text.len());
        Self(Arc::from(
            serde_json::to_string(&value).expect("finite clipboard properties"),
        ))
    }

    pub(crate) fn with_register(&self, text: &str, kind: u32, breaks: &[usize]) -> Self {
        let mut value: Export =
            serde_json::from_str(self.json()).expect("authored clipboard fragment");
        value.plain_text = text.to_owned();
        value.register_kind = kind;
        value.hard_breaks = breaks.to_vec();
        Self(Arc::from(
            serde_json::to_string(&value).expect("finite clipboard properties"),
        ))
    }
}

impl Export {
    fn pipeline(&self) -> Result<(Format, super::super::Encoding, FileFormat), DocumentError> {
        let format = match self.source_format {
            1 => Format::PlainText,
            2 => Format::Markdown,
            3 => Format::Html,
            4 => Format::Rtf,
            5 => Format::MarkdownSource,
            6 => Format::HtmlSource,
            _ => return Err(DocumentError::UnsupportedFormatting),
        };
        let encoding = match self.encoding {
            1 => super::super::Encoding::Utf8,
            2 => super::super::Encoding::Latin1,
            3 => super::super::Encoding::Utf16Le,
            4 => super::super::Encoding::Utf16Be,
            _ => return Err(DocumentError::UnsupportedFormatting),
        };
        let file_format = match self.file_format {
            1 => FileFormat::Unix,
            2 => FileFormat::Dos,
            3 => FileFormat::Mac,
            _ => return Err(DocumentError::UnsupportedFormatting),
        };
        Ok((format, encoding, file_format))
    }
}

impl Document {
    /// Export a legal range in this exact document snapshot. No document state,
    /// cursor, register, revision, or history is changed by this read.
    pub fn clipboard_fragment(
        &self,
        range: Range<usize>,
    ) -> Result<ClipboardFragment, DocumentError> {
        self.clipboard_fragment_with_source(range, &self.source_bytes())
    }

    fn clipboard_fragment_with_source(
        &self,
        range: Range<usize>,
        source: &[u8],
    ) -> Result<ClipboardFragment, DocumentError> {
        self.validate_range(&range)?;
        let captured = self
            .hard_line_snapshot()
            .capture(range.clone())
            .map_err(|_| DocumentError::AmbiguousProjection)?;
        let is_rich = matches!(self.format(), Format::Markdown | Format::Html | Format::Rtf);
        let source_bytes = if range == (0..self.text().len()) {
            source.to_vec()
        } else if range.is_empty() {
            Vec::new()
        } else {
            selected_source(self, &range, true, source).unwrap_or_default()
        };
        let source_text = self.encoding().decode(&source_bytes)?.text;
        let source_exact = Document::from_bytes_with_file_format(
            source_bytes.clone(),
            self.encoding(),
            self.format(),
            self.file_format(),
        )
        .is_ok_and(|source| {
            source.text() == captured.text()
                && source
                    .projection()
                    .hard_breaks_for_region(&(0..source.text().len()))
                    == captured.break_offsets()
        });
        let inline_source_bytes = if is_rich && !captured.text().contains('\n') {
            selected_source(self, &range, false, source).unwrap_or_default()
        } else {
            Vec::new()
        };
        let (character_runs, paragraph_runs) = if is_rich {
            style_runs(self.projection(), &range)?
        } else {
            (Vec::new(), Vec::new())
        };
        let export = Export {
            schema_version: 1,
            plain_text: if is_rich {
                captured.text().to_owned()
            } else {
                source_text.clone()
            },
            source_plain_text: captured.text().to_owned(),
            hard_breaks: captured.break_offsets().to_vec(),
            register_kind: 1,
            is_rich,
            source_text,
            source_bytes,
            source_format: match self.format() {
                Format::PlainText => 1,
                Format::Markdown => 2,
                Format::Html => 3,
                Format::Rtf => 4,
                Format::MarkdownSource => 5,
                Format::HtmlSource => 6,
            },
            encoding: match self.encoding() {
                super::super::Encoding::Utf8 => 1,
                super::super::Encoding::Latin1 => 2,
                super::super::Encoding::Utf16Le => 3,
                super::super::Encoding::Utf16Be => 4,
            },
            file_format: match self.file_format() {
                FileFormat::Unix => 1,
                FileFormat::Dos => 2,
                FileFormat::Mac => 3,
            },
            source_exact,
            inline_source_bytes,
            embedded_source_bytes: if is_rich {
                selected_source(self, &range, true, source).unwrap_or_default()
            } else {
                Vec::new()
            },
            character_runs,
            paragraph_runs,
            source_segments: Vec::new(),
        };
        Ok(ClipboardFragment(Arc::from(
            serde_json::to_string(&export).map_err(|_| DocumentError::UnsupportedFormatting)?,
        )))
    }

    /// Export a rectangular selection without copying text between disjoint
    /// ranges. Rows retain their visual order and each segment owns an exact
    /// independently reopenable source fragment whenever its provenance allows.
    pub fn clipboard_fragment_rows(
        &self,
        rows: &[Vec<Range<usize>>],
    ) -> Result<ClipboardFragment, DocumentError> {
        let source = self.source_bytes();
        let mut export: Export =
            serde_json::from_str(self.clipboard_fragment_with_source(0..0, &source)?.json())
                .map_err(|_| DocumentError::UnsupportedFormatting)?;
        export.register_kind = 3;
        export.source_exact = false;
        export.plain_text.clear();
        export.source_text.clear();
        export.source_bytes.clear();
        export.inline_source_bytes.clear();
        export.embedded_source_bytes.clear();
        export.character_runs.clear();
        export.paragraph_runs.clear();
        for (row_index, row) in rows.iter().enumerate() {
            if row_index != 0 {
                export.plain_text.push('\n');
                export.source_text.push('\n');
            }
            for range in row {
                if range.is_empty() {
                    self.validate_range(range)?;
                    continue;
                }
                let fragment: Export = serde_json::from_str(
                    self.clipboard_fragment_with_source(range.clone(), &source)?
                        .json(),
                )
                .map_err(|_| DocumentError::UnsupportedFormatting)?;
                let start = export.plain_text.len();
                export.plain_text.push_str(&fragment.plain_text);
                export.source_text.push_str(&fragment.source_text);
                let shift = |run: &Value| {
                    let mut run = run.clone();
                    run["start"] = json!(run["start"].as_u64().unwrap_or(0) as usize + start);
                    run["end"] = json!(run["end"].as_u64().unwrap_or(0) as usize + start);
                    run
                };
                export
                    .character_runs
                    .extend(fragment.character_runs.iter().map(shift));
                export
                    .paragraph_runs
                    .extend(fragment.paragraph_runs.iter().map(shift));
                export.source_segments.push(SourceSegment {
                    start,
                    end: export.plain_text.len(),
                    row: row_index,
                    fragment,
                });
            }
        }
        export.source_plain_text = export.plain_text.clone();
        export.source_bytes = self.encoding().encode_fragment(&export.source_text)?;
        export.hard_breaks.clear();
        Ok(ClipboardFragment(Arc::from(
            serde_json::to_string(&export).map_err(|_| DocumentError::UnsupportedFormatting)?,
        )))
    }

    /// Prepare source-preserving insertion of a compatible private fragment.
    /// `None` means that this is an ordinary/plain or different-format paste.
    /// A compatible rich payload never turns a failed verification into success.
    pub(crate) fn prepare_clipboard_fragment(
        &self,
        range: Range<usize>,
        fragment: &ClipboardFragment,
        text: &str,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        self.validate_range(&range)?;
        let export: Export = serde_json::from_str(fragment.json())
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        let (format, encoding, _) = export.pipeline()?;
        if !export.is_rich
            || format != self.format()
            || text != export.source_plain_text
            || !export.source_segments.is_empty()
        {
            return Ok(None);
        }
        if range != (0..self.text().len()) {
            if let Some(prepared) = self.prepare_with_recovered_source(
                &[TextEdit::new(range.clone(), text)],
                |scratch| {
                    scratch.prepare_clipboard_fragment(range.clone(), fragment, text)?
                        .ok_or_else(|| DocumentError::UnsupportedFormatting.into())
                },
            )? {
                return Ok(Some(prepared));
            }
            if let Some(prepared) = self.prepare_structural_replacement(range.clone(), text, |scratch, at| {
                scratch.prepare_clipboard_fragment(at..at, fragment, text)?
                    .ok_or_else(|| DocumentError::UnsupportedFormatting.into())
            })? {
                return Ok(Some(prepared));
            }
        }
        let bytes = |input: &[u8]| -> Result<Vec<u8>, DocumentError> {
            if encoding == self.encoding() {
                Ok(input.to_vec())
            } else {
                self.encoding()
                    .encode_fragment(&encoding.decode(input)?.text)
            }
        };
        let source_edit = super::super::source_edit::complete_contributors(
            self.projection(), &TextEdit::new(range.clone(), text),
        )?;
        let whole = range == (0..self.text().len());
        let plan = if whole { None } else {
            super::super::source_edit::overlapping_text_plan(self, &source_edit.range)?
        };
        let source_runs = if whole {
            vec![0..self.source_byte_len()]
        } else if let Some(plan) = &plan {
            plan.ranges.clone()
        } else { super::super::rich_text::text_source_runs(self, &source_edit.range)? };
        let insertion = plan.as_ref().map_or(source_runs[0].start, |plan| plan.insertion);
        let insertion_index = plan.as_ref().map_or(Some(0), |plan| plan.insertion_run());
        let preserved = |range: Range<usize>| -> Result<Vec<u8>, DocumentError> {
            let text = self.projection().text_tree().slice(range).map_err(DocumentError::FormattedTextStorage)?;
            let syntax = if self.format() == Format::Html {
                super::super::rich_text::escape_html_text(&text, self.encoding())
            } else {
                super::super::rtf::escape(&text)
            };
            self.encoding().encode_fragment(&syntax)
        };
        let prefix = preserved(source_edit.range.start..range.start)?;
        let suffix = preserved(range.end..source_edit.range.end)?;
        // Whole replacement reproduces the original byte image, including
        // malformed-but-supported syntax. Embedded pastes use balanced scopes
        // so an unclosed copied tag cannot restyle untouched following text.
        let candidates = if whole {
            vec![export.source_bytes.as_slice()]
        } else {
            [&export.inline_source_bytes, &export.embedded_source_bytes]
                .into_iter()
                .filter(|value| !value.is_empty())
                .map(Vec::as_slice)
                .collect()
        };
        let mut error = DocumentError::AmbiguousProjection.into();
        for candidate in candidates {
            let mut replacement = prefix.clone();
            replacement.extend(bytes(candidate)?);
            let mut patches = source_runs.iter().enumerate().map(|(index, range)| {
                let mut value = if insertion_index == Some(index) { replacement.clone() } else { Vec::new() };
                if index + 1 == source_runs.len() { value.extend_from_slice(&suffix); }
                SourcePatch::primary(range.clone(), value)
            }).collect::<Vec<_>>();
            if insertion_index.is_none() {
                patches.push(SourcePatch::primary(insertion..insertion, replacement));
            }
            let prepared = self
                .prepare_text_edits_with_patches(
                    vec![TextEdit::new(range.clone(), text)],
                    Some(patches),
                )
                .and_then(|prepared| {
                    if whole {
                        Ok(prepared)
                    } else {
                        self.isolate_clipboard_character_styles(
                            prepared, &export, range.clone(), &bytes(candidate)?,
                        )
                    }
                })
                .and_then(|prepared| {
                    verify_styles(prepared, &export, range.clone(), whole, self.projection())
                });
            match prepared {
                Ok(prepared) => return Ok(Some(prepared)),
                Err(failed) => error = failed,
            }
        }
        Err(error)
    }

    /// A balanced inline fragment can inherit different character defaults in
    /// its destination paragraph. Preserve its independently reproducible
    /// appearance with local declarations, using the normal formatting adapter.
    /// Missing source style definitions are still rejected by verification.
    fn isolate_clipboard_character_styles(
        &self,
        prepared: PreparedModelTransaction,
        export: &Export,
        replaced: Range<usize>,
        candidate: &[u8],
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if !matches!(self.format(), Format::Html | Format::Rtf) {
            return Ok(prepared);
        }
        let PreparedPublication::State(state) = &prepared.publication else {
            return Ok(prepared);
        };
        let inserted = replaced.start..replaced.start + export.source_plain_text.len();
        let (actual, _) = style_runs(&state.projection, &inserted)?;
        if styles_match(&export.character_runs, &actual) {
            return Ok(prepared);
        }
        let standalone = Document::from_bytes_with_file_format(
            candidate.to_vec(),
            self.encoding(),
            self.format(),
            self.file_format(),
        )?;
        let (independent, _) = style_runs(standalone.projection(), &(0..standalone.text().len()))?;
        if standalone.text() != export.source_plain_text
            || !styles_match(&export.character_runs, &independent)
        {
            return Ok(prepared);
        }
        let mut scratch = self.scratch_document();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        for patch in prepared.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        let inserted = scratch.prepare_text_edits_with_patches(
            vec![TextEdit::new(replaced.clone(), &export.source_plain_text)],
            Some(prepared.summary.source_patches),
        )?;
        scratch.commit_model_transaction(inserted)?;
        for expected in &export.character_runs {
            for current in &actual {
                let start = expected["start"]
                    .as_u64()
                    .unwrap_or(0)
                    .max(current["start"].as_u64().unwrap_or(0))
                    as usize;
                let end = expected["end"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(current["end"].as_u64().unwrap_or(0)) as usize;
                if start >= end {
                    continue;
                }
                let properties = |run: &Value| -> Result<CharacterProperties, DocumentError> {
                    let mut values = run.clone();
                    values["weight"] = values["base_weight"].clone();
                    if values["foreground_is_default"] == true {
                        values["foreground"] = Value::Null;
                    }
                    serde_json::from_value(values).map_err(|_| DocumentError::UnsupportedFormatting)
                };
                let mut authored = properties(expected)?;
                let current = properties(current)?;
                for property in authored.declared_properties() {
                    if !authored.changed_properties(&current).contains(&property) {
                        super::super::style::clear_character_property(
                            &StyleId::from("Clipboard"),
                            &mut authored,
                            property,
                        )?;
                    }
                }
                // A CSS face weight also controls conventional bold; retain
                // copied emphasis when changing the inherited paragraph face.
                if authored.weight.is_some() && authored.bold.is_none() {
                    authored.bold = expected["bold"].as_bool();
                }
                if authored.declared_properties().is_empty() {
                    continue;
                }
                let styled = scratch.prepare_rich_character_properties(
                    replaced.start + start..replaced.start + end,
                    authored,
                    None,
                )?;
                for patch in styled.summary.source_patches.iter().rev() {
                    sources.splice(patch.range(), patch.replacement());
                }
                scratch.commit_model_transaction(styled)?;
            }
        }
        self.prepare_text_edits_with_patches(
            vec![TextEdit::new(replaced, &export.source_plain_text)],
            Some(sources.source_patches(&scratch.state().source)?),
        )
    }
}

fn source_hull(document: &Document, range: &Range<usize>) -> Result<Range<usize>, DocumentError> {
    let spans = document.projection().provenance_for_region(range);
    let mut first = None;
    let mut end = 0;
    let mut cursor = range.start;
    for span in spans.iter().filter(|span| !span.formatted.is_empty()) {
        if span.formatted.start != cursor || span.formatted.end > range.end {
            return Err(DocumentError::AmbiguousProjection);
        }
        cursor = span.formatted.end;
        if !span.source.is_empty() {
            if first.is_some() && span.source.start < end {
                return Err(DocumentError::AmbiguousProjection);
            }
            first.get_or_insert(span.source.start);
            end = span.source.end;
        }
    }
    if cursor != range.end {
        return Err(DocumentError::AmbiguousProjection);
    }
    if first.is_none() && document.format() == Format::Html {
        return html_separator_source_hull(document, range, &spans);
    }
    if document.format() == Format::Html {
        // HTML5 can omit NULs between source whitespace contributors. The
        // reverse-edit helper already accounts for that lossless relation.
        if let Ok(runs) = super::super::rich_text::text_source_runs(document, range) {
            if let Some(tail) = runs.iter().map(|run| run.end).max() {
                end = end.max(tail);
            }
        }
    }
    first
        .map(|start| start..end)
        .ok_or(DocumentError::AmbiguousProjection)
}

fn html_separator_source_hull(
    document: &Document,
    range: &Range<usize>,
    spans: &[super::super::ProvenanceSpan],
) -> Result<Range<usize>, DocumentError> {
    let projection = document.projection();
    let separators = projection.hard_breaks_for_region(range);
    if separators != (range.start..range.end).collect::<Vec<_>>() {
        return Err(DocumentError::AmbiguousProjection);
    }
    let start = spans
        .iter()
        .find(|span| span.formatted.start == range.start)
        .filter(|span| span.source.is_empty())
        .map(|span| span.source.start)
        .ok_or(DocumentError::AmbiguousProjection)?;
    // A separator's own provenance is a recoverable source boundary. Its
    // following item supplies the other edge, including an empty paragraph.
    // Require one exact source location rather than choosing an ambiguous side.
    let following = projection.provenance_touching(&(range.end..range.end));
    let mut ends = following
        .iter()
        .filter(|span| span.formatted.start == range.end)
        .map(|span| span.source.start);
    let end = ends.next().ok_or(DocumentError::AmbiguousProjection)?;
    if end <= start
        || ends.any(|other| other != end)
        || spans.iter().any(|span| {
            !span.source.is_empty() || span.source.start < start || span.source.end > end
        })
        || spans
            .windows(2)
            .any(|pair| pair[0].source.start > pair[1].source.start)
    {
        return Err(DocumentError::AmbiguousProjection);
    }
    Ok(start..end)
}

fn selected_source(
    document: &Document,
    range: &Range<usize>,
    blocks: bool,
    source: &[u8],
) -> Result<Vec<u8>, DocumentError> {
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let hull = source_hull(document, range)?;
    if document.format() == Format::Rtf {
        return rtf_fragment(document, source, hull);
    }
    if !matches!(document.format(), Format::Markdown | Format::Html) {
        return Ok(source[hull].to_vec());
    }
    let decoded = document.encoding().decode(&source)?;
    let input = super::super::line_endings::normalize(&decoded, document.file_format());
    let builder = super::super::rich_text::Builder::new(&input, Revision(0));
    let normalized = |at: usize| -> Option<usize> {
        if at == source.len() {
            return Some(input.text.len());
        }
        input
            .units
            .iter()
            .find(|unit| unit.source.start == at)
            .map(|unit| unit.normalized.start)
    };
    let start = normalized(hull.start).ok_or(DocumentError::AmbiguousProjection)?;
    let end = normalized(hull.end).ok_or(DocumentError::AmbiguousProjection)?;
    let mut prefixes = Vec::new();
    let mut suffixes = Vec::new();
    if document.format() == Format::Html {
        let tokens = super::super::html::tokenize(&input.text);
        let opens = super::super::html_paragraph::stack_at(&tokens, start);
        let closes = super::super::html_paragraph::stack_at(&tokens, end);
        let retain = |token: &&&super::super::html::Token| match &token.kind {
            super::super::html::TokenKind::Tag(tag) => {
                blocks || !super::super::html_paragraph::structural(&tag.name)
            }
            _ => false,
        };
        for token in opens.iter().filter(retain) {
            prefixes.push(builder.source_range(token.range.clone()));
        }
        let mut output = Vec::new();
        if blocks {
            for (index, token) in tokens.iter().enumerate() {
                if matches!(&token.kind, super::super::html::TokenKind::Tag(tag) if !tag.end && tag.name == "style")
                {
                    if let Some(close) = tokens[index+1..].iter().find(|next| matches!(&next.kind, super::super::html::TokenKind::Tag(tag) if tag.end && tag.name == "style")) {
                        let region = builder.source_range(token.range.start..close.range.end);
                        if region.end <= hull.start || region.start >= hull.end { output.extend_from_slice(&source[region]); }
                    }
                }
            }
        }
        for prefix in prefixes {
            output.extend_from_slice(&source[prefix]);
        }
        output.extend_from_slice(&source[hull]);
        for token in closes.iter().filter(retain).rev() {
            let super::super::html::TokenKind::Tag(tag) = &token.kind else {
                unreachable!()
            };
            let mut depth = 0;
            let closing = tokens
                .iter()
                .filter(|next| next.range.start >= token.range.end)
                .find(|next| {
                    let super::super::html::TokenKind::Tag(next_tag) = &next.kind else {
                        return false;
                    };
                    if next_tag.name != tag.name {
                        return false;
                    }
                    if !next_tag.end {
                        depth += 1;
                        false
                    } else if depth == 0 {
                        true
                    } else {
                        depth -= 1;
                        false
                    }
                });
            if let Some(closing) = closing {
                output.extend_from_slice(&source[builder.source_range(closing.range.clone())]);
            } else {
                output.extend_from_slice(
                    &document
                        .encoding()
                        .encode_fragment(&format!("</{}>", tag.name))?,
                );
            }
        }
        if document
            .projection()
            .provenance_for_region(range)
            .iter()
            .all(|span| span.source.is_empty())
        {
            let candidate = Document::from_bytes_with_file_format(
                output.clone(),
                document.encoding(),
                document.format(),
                document.file_format(),
            )?;
            let expected_breaks = document
                .projection()
                .hard_breaks_for_region(range)
                .into_iter()
                .map(|offset| offset - range.start)
                .collect::<Vec<_>>();
            if candidate.text() != &document.text()[range.clone()]
                || candidate
                    .projection()
                    .hard_breaks_for_region(&(0..candidate.text().len()))
                    != expected_breaks
            {
                return Err(DocumentError::AmbiguousProjection);
            }
        }
        return Ok(output);
    }
    // Markdown semantic spans identify their authored delimiter runs. Copy
    // those runs at a cut edge, retaining underscore/star/backtick spelling.
    for span in document.projection().style_spans_for_region(range) {
        if !matches!(span.application, StyleApplication::Semantic(_)) {
            continue;
        }
        let Ok(body) = source_hull(document, &span.range) else {
            continue;
        };
        let (Some(a), Some(b)) = (normalized(body.start), normalized(body.end)) else {
            continue;
        };
        let mut before = a;
        while before > 0 && matches!(input.text.as_bytes()[before - 1], b'*' | b'_' | b'`') {
            before -= 1;
        }
        let mut after = b;
        while after < input.text.len() && matches!(input.text.as_bytes()[after], b'*' | b'_' | b'`')
        {
            after += 1;
        }
        if before == a || after == b {
            continue;
        }
        if span.range.start <= range.start && range.start < span.range.end {
            prefixes.push(builder.source_range(before..a));
        }
        if span.range.start < range.end && range.end <= span.range.end {
            suffixes.push(builder.source_range(b..after));
        }
    }
    if blocks {
        let line_start = input.text[..start].rfind('\n').map_or(0, |at| at + 1);
        let line_end = input.text[start..]
            .find('\n')
            .map_or(input.text.len(), |at| start + at);
        let (body, _) =
            super::super::projection::markdown_block_prefix(&input.text, line_start, line_end);
        if body <= start && body > line_start {
            prefixes.push(builder.source_range(line_start..body));
        }
    }
    prefixes.sort_by_key(|r| (r.start, r.end));
    suffixes.sort_by_key(|r| (r.start, r.end));
    let mut pieces = prefixes;
    pieces.push(hull);
    pieces.extend(suffixes);
    let mut result = Vec::new();
    let mut previous_end = 0;
    for mut piece in pieces {
        piece.start = piece.start.max(previous_end);
        if piece.start < piece.end {
            result.extend_from_slice(&source[piece.clone()]);
            previous_end = piece.end;
        }
    }
    Ok(result)
}

fn style_runs(
    projection: &FormattedDocument,
    range: &Range<usize>,
) -> Result<(Vec<Value>, Vec<Value>), DocumentError> {
    let mut characters = Vec::new();
    let mut paragraphs = Vec::new();
    for block in projection.blocks_for_region(range) {
        let selected = block.range.start.max(range.start)..block.range.end.min(range.end);
        if selected.start > selected.end {
            continue;
        }
        let paragraph = projection
            .style_sheet()
            .resolve_assigned_paragraph_style(
                projection.document_style(),
                &block.style,
                &block.direct_paragraph,
                &block.direct_default_character,
                None,
                &CharacterProperties::default(),
            )
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        // Match the layout's UAX #9 P2/P3 resolution using the full original
        // paragraph, including when the clipboard selects only a later word.
        let resolved_direction = match paragraph.base_direction {
            super::super::WritingDirection::Natural => {
                let body = projection
                    .text_tree()
                    .slice(block.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                if unicode_bidi::get_base_direction(body.as_str()) == unicode_bidi::Direction::Rtl {
                    super::super::WritingDirection::RightToLeft
                } else {
                    super::super::WritingDirection::LeftToRight
                }
            }
            direction => direction,
        };
        paragraphs.push(json!({"start":selected.start-range.start,"end":selected.end-range.start,
            "spacing_before":paragraph.spacing_before,"spacing_after":paragraph.spacing_after,
            "line_spacing":paragraph.line_spacing,"first_line_indent":paragraph.first_line_indent,
            "leading_indent":paragraph.leading_indent,"trailing_indent":paragraph.trailing_indent,
            "alignment":paragraph.alignment,"base_direction":paragraph.base_direction,"resolved_direction":resolved_direction}));
        let spans = projection.style_spans_for_region(&selected);
        let mut boundaries = BTreeSet::from([selected.start, selected.end]);
        for span in &spans {
            boundaries.insert(span.range.start.max(selected.start));
            boundaries.insert(span.range.end.min(selected.end));
        }
        let boundaries = boundaries.into_iter().collect::<Vec<_>>();
        for pair in boundaries.windows(2) {
            if pair[0] == pair[1] {
                continue;
            }
            let mut direct = CharacterProperties::default();
            let mut semantic = CharacterProperties::default();
            let mut automatic = CharacterProperties::default();
            let mut named = None;
            let mut paragraph_style = &block.style;
            let mut defaults = &block.direct_default_character;
            for span in spans.iter().filter(|span| span.range.contains(&pair[0])) {
                match &span.application {
                    StyleApplication::Named(id) => {
                        if named.is_some_and(|other| other != id) {
                            return Err(DocumentError::UnsupportedFormatting);
                        }
                        named = Some(id);
                    }
                    StyleApplication::Automatic(id) => {
                        let mut chain = Vec::new();
                        let mut next = Some(id);
                        while let Some(id) = next {
                            let style = projection
                                .style_sheet()
                                .character_style(id)
                                .ok_or(DocumentError::UnsupportedFormatting)?;
                            chain.push(&style.properties);
                            next = style.based_on.as_ref();
                        }
                        for properties in chain.into_iter().rev() {
                            super::super::rich_text::overlay(&mut automatic, properties);
                        }
                    }
                    StyleApplication::Direct(properties) => {
                        super::super::rich_text::overlay(&mut direct, properties)
                    }
                    StyleApplication::SourceParagraph {
                        style,
                        defaults: values,
                    } => {
                        paragraph_style = style;
                        defaults = values;
                    }
                    StyleApplication::Semantic(SemanticInlineStyle::Strong) => {
                        semantic.bold = Some(true)
                    }
                    StyleApplication::Semantic(SemanticInlineStyle::Emphasis) => {
                        semantic.slant = Some(super::super::FontSlant::Italic)
                    }
                    _ => {}
                }
            }
            super::super::rich_text::overlay(&mut semantic, &direct);
            super::super::rich_text::overlay(&mut semantic, &automatic);
            let style = projection
                .style_sheet()
                .resolve_assigned_paragraph_style(
                    projection.document_style(),
                    paragraph_style,
                    &block.direct_paragraph,
                    defaults,
                    named,
                    &semantic,
                )
                .map_err(|_| DocumentError::UnsupportedFormatting)?
                .character;
            characters.push(json!({"start":pair[0]-range.start,"end":pair[1]-range.start,
                "font_families":style.font_families,"size":style.size,"weight":style.weight,"base_weight":style.base_weight,
                "bold":style.bold,"slant":style.slant,"foreground":style.foreground,"foreground_is_default":style.foreground_is_default,
                "background":style.background,"underline":style.underline,"strikethrough":style.strikethrough,"language":style.language,
                "direction":style.direction,"open_type_features":style.open_type_features,"letter_spacing":style.letter_spacing,
                "baseline_shift":style.baseline_shift}));
        }
    }
    Ok((characters, paragraphs))
}

fn styles_match(expected: &[Value], actual: &[Value]) -> bool {
    expected.iter().all(|expected| {
        let start = expected["start"].as_u64().unwrap_or(0);
        let end = expected["end"].as_u64().unwrap_or(0);
        let relevant = actual
            .iter()
            .filter(|run| {
                run["start"].as_u64().unwrap_or(0) < end && run["end"].as_u64().unwrap_or(0) > start
            })
            .collect::<Vec<_>>();
        (start == end || !relevant.is_empty())
            && relevant.into_iter().all(|run| {
                expected.as_object().is_some_and(|properties| {
                    properties.iter().all(|(name, value)| {
                        name == "start"
                            || name == "end"
                            || name == "resolved_direction"
                            || run.get(name) == Some(value)
                    })
                })
            })
    })
}

fn verify_styles(
    prepared: PreparedModelTransaction,
    export: &Export,
    replaced: Range<usize>,
    paragraphs: bool,
    before: &FormattedDocument,
) -> Result<PreparedModelTransaction, ModelTransactionError> {
    let PreparedPublication::State(state) = &prepared.publication else {
        return Ok(prepared);
    };
    let (actual, paragraph_runs) = style_runs(
        &state.projection,
        &(replaced.start..replaced.start + export.source_plain_text.len()),
    )?;
    let actual_breaks = state
        .projection
        .hard_breaks_for_region(&(replaced.start..replaced.start + export.source_plain_text.len()))
        .into_iter()
        .map(|offset| offset - replaced.start)
        .collect::<Vec<_>>();
    let expected_breaks = export
        .hard_breaks
        .iter()
        .copied()
        .filter(|offset| *offset < export.source_plain_text.len())
        .collect::<Vec<_>>();
    if actual_breaks != expected_breaks
        || !styles_match(&export.character_runs, &actual)
        || paragraphs && !styles_match(&export.paragraph_runs, &paragraph_runs)
    {
        return Err(DocumentError::UnsupportedFormatting.into());
    }
    for (old_range, new_range) in [
        (0..replaced.start, 0..replaced.start),
        (
            replaced.end..before.text_tree().byte_len(),
            replaced.start + export.source_plain_text.len()
                ..state.projection.text_tree().byte_len(),
        ),
    ] {
        if old_range.is_empty() {
            continue;
        }
        let (old_character, old_paragraph) = style_runs(before, &old_range)?;
        let (new_character, new_paragraph) = style_runs(&state.projection, &new_range)?;
        if !styles_match(&old_character, &new_character) || !styles_match(&old_paragraph, &new_paragraph) {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
    }
    Ok(prepared)
}

fn rtf_fragment(
    document: &Document,
    source: &[u8],
    hull: Range<usize>,
) -> Result<Vec<u8>, DocumentError> {
    use super::super::rtf::Kind;
    let decoded = document.encoding().decode(source)?;
    let input = super::super::line_endings::normalize(&decoded, document.file_format());
    let builder = super::super::rich_text::Builder::new(&input, Revision(0));
    let tokens = super::super::rtf::tokenize(&input);
    let mut stack: Vec<(usize, Vec<Range<usize>>)> = Vec::new();
    let mut tables = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let raw = builder.source_range(token.range.clone());
        if raw.end > hull.start {
            break;
        }
        match &token.kind {
            Kind::Open => stack.push((index, vec![raw])),
            Kind::Close => {
                if let Some((open, _)) = stack.pop() {
                    if tokens[open+1..index].iter().take(2).any(|token| matches!(&token.kind, Kind::Control(name,_) if matches!(name.as_str(),"fonttbl"|"colortbl"|"stylesheet"|"listtable"|"listoverridetable"))) {
                    tables.push(builder.source_range(tokens[open].range.start..token.range.end));
                }
                }
            }
            Kind::Control(name, _)
                if !matches!(
                    name.as_str(),
                    "u" | "par"
                        | "line"
                        | "tab"
                        | "emdash"
                        | "endash"
                        | "bullet"
                        | "lquote"
                        | "rquote"
                        | "ldblquote"
                        | "rdblquote"
                        | "bin"
                ) && !super::super::rtf::non_body(name) =>
            {
                if let Some((_, pieces)) = stack.last_mut() {
                    pieces.push(raw);
                }
            }
            _ => {}
        }
    }
    let mut output = Vec::new();
    for (level, (_, pieces)) in stack.iter().enumerate() {
        for piece in pieces {
            output.extend_from_slice(&source[piece.clone()]);
        }
        if level == 0 {
            for table in &tables {
                output.extend_from_slice(&source[table.clone()]);
            }
        }
    }
    if stack.is_empty() {
        output.extend_from_slice(b"{\\rtf1 ");
    }
    output.extend_from_slice(&source[hull.clone()]);
    let mut depth = stack.len().max(1);
    for token in &tokens {
        let raw = builder.source_range(token.range.clone());
        if hull.start <= raw.start && raw.end <= hull.end {
            match token.kind {
                Kind::Open => depth += 1,
                Kind::Close => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    output.extend(std::iter::repeat(b'}').take(depth));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::super::super::Encoding;
    use super::*;

    fn open(source: &[u8], format: Format) -> Document {
        Document::from_bytes_with_file_format(
            source.to_vec(),
            Encoding::Utf8,
            format,
            FileFormat::Unix,
        )
        .unwrap()
    }
    fn export(document: &Document, text: &str) -> (ClipboardFragment, Export) {
        let start = document.text().find(text).unwrap();
        let fragment = document
            .clipboard_fragment(start..start + text.len())
            .unwrap();
        let value = serde_json::from_str(fragment.json()).unwrap();
        (fragment, value)
    }
    fn paste_empty(fragment: &ClipboardFragment, value: &Export, format: Format) -> Document {
        let mut target = open(b"", format);
        let prepared = target
            .prepare_clipboard_fragment(0..0, fragment, &value.source_plain_text)
            .unwrap()
            .unwrap();
        target.commit_model_transaction(prepared).unwrap();
        target
    }

    #[test]
    fn full_selection_retains_original_source_bytes_and_roundtrips_undo() {
        for (format, source) in [
            (
                Format::Markdown,
                b"# Title\n\n__bold__ and `code`\n".as_slice(),
            ),
            (
                Format::Html,
                b"<!DOCTYPE html><!--original--><P class='x'><B>A&amp;B</B><br>tail</P>".as_slice(),
            ),
            (
                Format::Rtf,
                br"{\rtf1\ansi {\b Bold} and \i italic\i0\par tail}".as_slice(),
            ),
        ] {
            let original = open(source, format);
            let fragment = original
                .clipboard_fragment(0..original.text().len())
                .unwrap();
            let value: Export = serde_json::from_str(fragment.json()).unwrap();
            assert_eq!(value.source_bytes, source);
            assert!(value.source_exact, "{format:?}: {:?}", value.source_text);
            let imported = ClipboardFragment::from_json(fragment.json(), original.text()).unwrap();
            let mut pasted = paste_empty(&imported, &value, format);
            assert_eq!(pasted.source_bytes(), source);
            assert!(pasted.undo());
            assert!(pasted.source_bytes().is_empty());
            assert!(pasted.redo());
            assert_eq!(pasted.source_bytes(), source);
        }
    }

    #[test]
    fn partial_markdown_preserves_authored_delimiters_and_clipped_content() {
        let original = open(b"before __bold text__ after", Format::Markdown);
        let (fragment, value) = export(&original, "old");
        assert_eq!(value.source_text, "__old__");
        assert!(value.source_exact);
        assert_eq!(value.character_runs[0]["bold"], true);
        let pasted = paste_empty(&fragment, &value, Format::Markdown);
        assert_eq!(pasted.source_bytes(), b"__old__");
    }

    #[test]
    fn partial_html_keeps_entity_spelling_tag_case_and_attributes() {
        let original = open(
            b"<P>before <B title='authored' style='color: #ff0000'>A&#38;B</B> after</P>",
            Format::Html,
        );
        let (fragment, value) = export(&original, "&B");
        assert_eq!(
            value.source_text,
            "<P><B title='authored' style='color: #ff0000'>&#38;B</B></P>"
        );
        let pasted = paste_empty(&fragment, &value, Format::Html);
        assert_eq!(pasted.text(), "&B");
        assert_eq!(pasted.source_bytes(), value.source_bytes);
        let mut existing = open(b"<p>left right</p>", Format::Html);
        let prepared = existing
            .prepare_clipboard_fragment(5..5, &fragment, "&B")
            .unwrap()
            .unwrap();
        existing.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            existing.source_bytes(),
            b"<p>left <B title='authored' style='color: #ff0000'>&#38;B</B>right</p>"
        );
    }

    #[test]
    fn partial_rtf_carries_group_controls_and_font_table() {
        let original = open(
            br"{\rtf1\ansi{\fonttbl{\f0 Helvetica;}}\f0 before {\b Bold text} after}",
            Format::Rtf,
        );
        let (fragment, value) = export(&original, "old");
        assert!(value.source_exact, "{:?}", value.source_text);
        assert!(value.source_text.contains("\\fonttbl"));
        assert!(value.source_text.contains("\\b old"));
        let pasted = paste_empty(&fragment, &value, Format::Rtf);
        assert_eq!(pasted.text(), "old");
        assert_eq!(pasted.source_bytes(), value.source_bytes);
    }

    #[test]
    fn partial_html_carries_source_owned_named_style_definitions() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        let mut paragraph = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        paragraph.character.foreground = Some(super::super::super::Color {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        });
        let metadata = sheet
            .block_style_metadata(&sheet.base_paragraph)
            .unwrap()
            .clone();
        sheet
            .install_source_definitions(&[StyleDefinitionEdit::InsertBlock {
                style: paragraph,
                metadata,
            }])
            .unwrap();
        let rule =
            super::super::super::html_styles::write_rule_v2(&sheet, &sheet.base_paragraph, false)
                .unwrap();
        let source = format!("<style id=\"viem-styles\" data-viem-version=\"2\">\n{rule}</style><p>before <b>Bold</b> after</p>");
        let original = open(source.as_bytes(), Format::Html);
        let (fragment, value) = export(&original, "Bold");
        assert!(value.source_text.starts_with("<style"));
        assert!(value.source_exact);
        assert_eq!(value.character_runs[0]["foreground"]["red"], 1.0);
        let pasted = paste_empty(&fragment, &value, Format::Html);
        assert_eq!(pasted.source_bytes(), value.source_bytes);
        let existing = open(b"<p>untouched</p>", Format::Html);
        assert!(existing
            .prepare_clipboard_fragment(3..3, &fragment, "Bold")
            .is_err());
        assert_eq!(existing.source_bytes(), b"<p>untouched</p>");
    }

    #[test]
    fn natural_paragraph_direction_uses_context_outside_the_copied_word() {
        let original = open(
            "<p style='text-align:end'>العربية Latin</p>".as_bytes(),
            Format::Html,
        );
        let (_, value) = export(&original, "Latin");
        assert_eq!(value.paragraph_runs[0]["base_direction"], "Natural");
        assert_eq!(value.paragraph_runs[0]["resolved_direction"], "RightToLeft");
        assert_eq!(value.paragraph_runs[0]["alignment"], "End");
    }

    #[test]
    fn html_separator_clipboard_retains_authored_boundary_source() {
        let multiple = "<P title='left'>A</P><!-- edge-1 --><p title='middle'></p><!-- edge-2 --><P title='right'>B</P>";
        for (source, range, expected) in [
            ("<p>A</p><p>B</p>", 1..2, "<p></p><p></p>"),
            ("<P title='left'>A</P><!-- keep --><p title='right'>B</p>", 1..2, "<P title='left'></P><!-- keep --><p title='right'></p>"),
            (multiple, 1..2, "<P title='left'></P><!-- edge-1 --><p title='middle'></p>"),
            (multiple, 2..3, "<p title='middle'></p><!-- edge-2 --><P title='right'></P>"),
            (multiple, 1..3, "<P title='left'></P><!-- edge-1 --><p title='middle'></p><!-- edge-2 --><P title='right'></P>"),
        ] {
            let original = open(source.as_bytes(), Format::Html);
            let fragment = original.clipboard_fragment(range.clone()).unwrap();
            let value: Export = serde_json::from_str(fragment.json()).unwrap();
            assert!(value.source_exact, "{source:?} {}", value.source_text);
            assert_eq!(value.source_bytes, expected.as_bytes());
            let reopened = open(&value.source_bytes, Format::Html);
            assert_eq!(reopened.text(), &original.text()[range.clone()]);
            assert_eq!(reopened.projection().hard_breaks_for_region(&(0..reopened.text().len())), (0..range.len()).collect::<Vec<_>>());
            let mut pasted = paste_empty(&fragment, &value, Format::Html);
            assert_eq!(pasted.source_bytes(), expected.as_bytes());
            assert!(pasted.undo());
            assert!(pasted.source_bytes().is_empty());
            assert!(pasted.redo());
            assert_eq!(pasted.source_bytes(), expected.as_bytes());
            assert_eq!(original.source_bytes(), source.as_bytes());
        }
    }

    #[test]
    fn rectangular_clipboard_owns_only_selected_source_segments_and_resolved_styles() {
        let empty = open(b"<p></p><!--outside-empty-->", Format::Html);
        let empty_fragment = empty.clipboard_fragment_rows(&[vec![0..0]]).unwrap();
        let empty_value: Export = serde_json::from_str(empty_fragment.json()).unwrap();
        assert!(empty_value.source_text.is_empty());
        assert!(empty_value.source_segments.is_empty());
        for (format, source) in [
            (
                Format::Html,
                "<p><b>ab</b> outside-one<br><b>cd</b> outside-two</p><!--outside-whole-->",
            ),
            (Format::Markdown, "__ab__ outside-one\n\n__cd__ outside-two"),
            (
                Format::Rtf,
                r"{\rtf1{\b ab} outside-one\line {\b cd} outside-two}",
            ),
        ] {
            let original = open(source.as_bytes(), format);
            let second = original.text().find("cd").unwrap();
            let fragment = original
                .clipboard_fragment_rows(&[vec![0..2], vec![second..second + 2]])
                .unwrap();
            let value: Export = serde_json::from_str(fragment.json()).unwrap();
            assert_eq!(value.plain_text, "ab\ncd");
            assert_eq!(value.register_kind, 3);
            assert_eq!(value.source_segments.len(), 2);
            assert!(!fragment.json().contains("outside"));
            assert!(ClipboardFragment::from_json(fragment.json(), "ab\ncd").is_ok());
            for (index, segment) in value.source_segments.iter().enumerate() {
                assert_eq!(segment.row, index);
                assert_eq!(segment.start, index * 3);
                assert_eq!(segment.end, index * 3 + 2);
                assert!(segment.fragment.source_exact, "{format:?}");
                assert_eq!(segment.fragment.character_runs[0]["bold"], true);
                let own_fragment =
                    ClipboardFragment(Arc::from(serde_json::to_string(&segment.fragment).unwrap()));
                let pasted = paste_empty(&own_fragment, &segment.fragment, format);
                assert_eq!(pasted.source_bytes(), segment.fragment.source_bytes);
                assert_eq!(pasted.text(), &value.plain_text[segment.start..segment.end]);
            }
            assert_eq!(original.source_bytes(), source.as_bytes());
        }
    }

    #[test]
    fn source_mode_exports_unformatted_original_source_spelling() {
        for format in [Format::MarkdownSource, Format::HtmlSource] {
            let source = b"**one**\r\n\r\n<p>two</p>";
            let original = Document::from_bytes_with_file_format(
                source.to_vec(),
                Encoding::Utf8,
                format,
                FileFormat::Dos,
            )
            .unwrap();
            let fragment = original
                .clipboard_fragment(0..original.text().len())
                .unwrap();
            let value: Export = serde_json::from_str(fragment.json()).unwrap();
            assert!(!value.is_rich);
            assert_eq!(value.plain_text, std::str::from_utf8(source).unwrap());
            assert_eq!(value.source_bytes, source);
            assert!(value.character_runs.is_empty());
        }
    }

    #[test]
    fn stale_plain_sidecar_and_invalid_ranges_are_rejected() {
        let original = open(b"__bold__", Format::Markdown);
        let (fragment, _) = export(&original, "bold");
        assert!(ClipboardFragment::from_json(fragment.json(), "different").is_err());
        assert!(ClipboardFragment::from_json("{}", "bold").is_err());
        assert!(original.clipboard_fragment(3..1).is_err());
        let emoji = open("👩‍💻".as_bytes(), Format::Markdown);
        assert!(emoji.clipboard_fragment(0..4).is_err());
    }
}
