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

    /// Decode external HTML into the shared passive fragment model. Only its
    /// normalized text and resolved formatting cross the clipboard boundary;
    /// HTML never becomes an editable document format.
    pub fn from_html_utf8(bytes: &[u8]) -> Result<(Self, String), DocumentError> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let decoded = super::super::Encoding::Utf8.decode(bytes)?;
        let input = super::super::line_endings::normalize(&decoded, FileFormat::Unix);
        let projection =
            super::super::html::project_fragment(&input, Revision(0), decoded.bom_len, bytes.len());
        let text = projection.text().to_owned();
        let range = 0..text.len();
        let (character_runs, paragraph_runs) = style_runs(&projection, &range)?;
        let export = Export {
            schema_version: 1,
            plain_text: text.clone(),
            source_plain_text: text.clone(),
            hard_breaks: projection.hard_breaks_for_region(&range),
            register_kind: 1,
            is_rich: true,
            source_text: text.clone(),
            source_bytes: text.as_bytes().to_vec(),
            source_format: 1,
            encoding: 1,
            file_format: 1,
            inline_source_bytes: Vec::new(),
            embedded_source_bytes: Vec::new(),
            character_runs,
            paragraph_runs,
            source_segments: Vec::new(),
        };
        let json =
            serde_json::to_string(&export).map_err(|_| DocumentError::UnsupportedFormatting)?;
        Ok((Self(Arc::from(json)), text))
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

    /// Whether this value follows a source-preserving or normalized formatted paste path.
    /// Candidate verification can still reject unsupported imported syntax.
    pub fn can_insert_rich_source(&self, format: Format, text: &str) -> bool {
        serde_json::from_str::<Export>(self.json())
            .is_ok_and(|export| export.can_insert_rich_source(format, text))
    }

    /// Transform only the fragment's prose and retain its source and resolved
    /// style image. Clipboard/register values remain immutable.
    pub fn transform_quotes(
        &self,
        previous: Option<char>,
        mut quote: impl FnMut(char, Option<char>) -> char,
    ) -> Result<(String, Self), DocumentError> {
        let original: Export =
            serde_json::from_str(self.json()).map_err(|_| DocumentError::UnsupportedFormatting)?;
        if !original.plain_text.contains(['\'', '"']) {
            return Ok((original.plain_text, self.clone()));
        }
        let (format, encoding, file_format) = original.pipeline()?;
        let mut document = Document::from_bytes_with_file_format(
            original.source_bytes.clone(),
            encoding,
            format,
            file_format,
        )?;
        if document.text() != original.source_plain_text || !original.source_segments.is_empty() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let (text, edits) = super::input_context::rewrite_quotes(
            document.text(),
            previous,
            |at, _| document.quote_context(at, BoundaryAffinity::Downstream),
            &mut quote,
        )?;
        if edits.is_empty() {
            return Ok((original.plain_text, self.clone()));
        }
        let mapped = original
            .source_plain_text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(original.source_plain_text.len()))
            .zip(
                text.char_indices()
                    .map(|(at, _)| at)
                    .chain(std::iter::once(text.len())),
            )
            .collect::<BTreeMap<_, _>>();
        document.apply_edits(edits)?;
        let fragment = document.clipboard_fragment(0..document.text().len())?;
        let mut transformed: Export = serde_json::from_str(fragment.json())
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        // Configuration-only typography need not occur in copied source.
        // Retain the original resolved runs while rebasing their text extents.
        let remap_runs = |runs: Vec<Value>| -> Result<Vec<Value>, DocumentError> {
            runs.into_iter()
                .map(|mut run| {
                    for key in ["start", "end"] {
                        let at = run[key]
                            .as_u64()
                            .and_then(|at| usize::try_from(at).ok())
                            .ok_or(DocumentError::UnsupportedFormatting)?;
                        run[key] = json!(mapped
                            .get(&at)
                            .ok_or(DocumentError::UnsupportedFormatting)?);
                    }
                    Ok(run)
                })
                .collect()
        };
        transformed.character_runs = remap_runs(original.character_runs)?;
        transformed.paragraph_runs = remap_runs(original.paragraph_runs)?;
        transformed.hard_breaks = original
            .hard_breaks
            .iter()
            .copied()
            .filter(|&at| at < original.source_plain_text.len())
            .map(|at| {
                mapped
                    .get(&at)
                    .copied()
                    .ok_or(DocumentError::UnsupportedFormatting)
            })
            .collect::<Result<_, _>>()?;
        transformed.register_kind = original.register_kind;
        transformed.is_rich = original.is_rich;
        let appended_line_break = original.register_kind == 2
            && original.plain_text == format!("{}\n", original.source_plain_text);
        if original.plain_text == original.source_plain_text || appended_line_break {
            // Source-view yanks use normalized visible text even when the
            // fragment's persisted source retains CRLF or CR spelling.
            transformed.plain_text = transformed.source_plain_text.clone();
        }
        if appended_line_break {
            transformed.hard_breaks.push(transformed.plain_text.len());
            transformed.plain_text.push('\n');
        }
        let text = transformed.plain_text.clone();
        Ok((
            text,
            Self(Arc::from(
                serde_json::to_string(&transformed)
                    .map_err(|_| DocumentError::UnsupportedFormatting)?,
            )),
        ))
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
    fn can_insert_rich_source(&self, format: Format, text: &str) -> bool {
        self.is_rich
            && format.is_wysiwyg()
            && self
                .pipeline()
                .is_ok_and(|(source, _, _)| source == format || source == Format::PlainText)
            && text == self.source_plain_text
            && self.source_segments.is_empty()
    }

    fn pipeline(&self) -> Result<(Format, super::super::Encoding, FileFormat), DocumentError> {
        let format = match self.source_format {
            1 => Format::PlainText,
            2 => Format::Markdown,
            5 => Format::MarkdownSource,
            7 => Format::Code,
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
        let is_rich = self.format().is_wysiwyg();
        let source_bytes = if range == (0..self.text().len()) {
            source.to_vec()
        } else if range.is_empty() {
            Vec::new()
        } else {
            selected_source(self, &range, true, source).unwrap_or_default()
        };
        let source_text = self.encoding().decode(&source_bytes)?.text;
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
                Format::MarkdownSource => 5,
                Format::Code => 7,
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

    /// Preserve compatible source fragments or replay normalized external styles.
    /// `None` means that this payload uses ordinary text insertion.
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
        if export.is_rich
            && export.source_format == 1
            && self.format().is_wysiwyg()
            && text == export.source_plain_text
            && export.source_segments.is_empty()
        {
            return match self.prepare_normalized_clipboard_fragment(range, &export) {
                Ok(prepared) => Ok(Some(prepared)),
                // External CSS can exceed a destination's persisted style
                // vocabulary. Retain ordinary text paste in that case; private
                // same-format source fragments still verify strictly below.
                Err(ModelTransactionError::Document(DocumentError::UnsupportedFormatting)) => {
                    Ok(None)
                }
                Err(error) => Err(error),
            };
        }
        let (_, encoding, _) = export.pipeline()?;
        if !export.can_insert_rich_source(self.format(), text) {
            return Ok(None);
        }
        if range != (0..self.text().len()) {
            if let Some(prepared) =
                self.prepare_structural_replacement(range.clone(), text, |scratch, at| {
                    scratch
                        .prepare_clipboard_fragment(at..at, fragment, text)?
                        .ok_or_else(|| DocumentError::UnsupportedFormatting.into())
                })?
            {
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
            self.projection(),
            &TextEdit::new(range.clone(), text),
        )?;
        let whole = range == (0..self.text().len());
        let plan = if whole {
            None
        } else {
            super::super::source_edit::overlapping_text_plan(self, &source_edit.range)?
        };
        let source_runs = if whole {
            vec![0..self.source_byte_len()]
        } else if let Some(plan) = &plan {
            plan.ranges.clone()
        } else {
            super::super::rich_text::text_source_runs(self, &source_edit.range)?
        };
        let insertion = plan
            .as_ref()
            .map_or(source_runs[0].start, |plan| plan.insertion);
        let insertion_index = plan.as_ref().map_or(Some(0), |plan| plan.insertion_run());
        let preserved = |range: Range<usize>| -> Result<Vec<u8>, DocumentError> {
            let text = self
                .projection()
                .text_tree()
                .slice(range)
                .map_err(DocumentError::FormattedTextStorage)?;
            let syntax = self.escape_markdown_source_text(insertion, &text)?;
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
            let mut patches = source_runs
                .iter()
                .enumerate()
                .map(|(index, range)| {
                    let mut value = if insertion_index == Some(index) {
                        replacement.clone()
                    } else {
                        Vec::new()
                    };
                    if index + 1 == source_runs.len() {
                        value.extend_from_slice(&suffix);
                    }
                    SourcePatch::primary(range.clone(), value)
                })
                .collect::<Vec<_>>();
            if insertion_index.is_none() {
                patches.push(SourcePatch::primary(insertion..insertion, replacement));
            }
            let prepared = self
                .prepare_text_edits_with_patches(
                    vec![TextEdit::new(range.clone(), text)],
                    Some(patches),
                )
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

    /// Replay external HTML's normalized formatting through the destination's
    /// existing editing adapters. The composed source patches publish as one
    /// undoable paste; unsupported Markdown properties follow its vocabulary.
    fn prepare_normalized_clipboard_fragment(
        &self,
        replaced: Range<usize>,
        export: &Export,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        let publish = |scratch: &mut Document,
                       prepared: PreparedModelTransaction,
                       sources: &mut replacement::PatchComposition|
         -> Result<(), ModelTransactionError> {
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };

        let payload = super::super::FormattedTextPayload::new(
            &scratch.hard_line_snapshot(),
            export.source_plain_text.clone(),
            export.hard_breaks.clone(),
        )
        .map_err(|_| DocumentError::UnsupportedFormatting)?;
        let inserted = scratch.prepare_formatted_payload_edits(vec![FormattedPayloadEdit::new(
            replaced.clone(),
            payload,
        )])?;
        publish(&mut scratch, inserted, &mut sources)?;

        let mut markdown_runs: Vec<(Range<usize>, CharacterProperties)> = Vec::new();
        for run in &export.character_runs {
            let start = replaced.start
                + run["start"]
                    .as_u64()
                    .ok_or(DocumentError::UnsupportedFormatting)? as usize;
            let end = replaced.start
                + run["end"]
                    .as_u64()
                    .ok_or(DocumentError::UnsupportedFormatting)? as usize;
            if start == end {
                continue;
            }
            let mut values = run.clone();
            values["weight"] = values["base_weight"].clone();
            if values["direction"] == "Natural" {
                values["direction"] = Value::Null;
            }
            if values["foreground_is_default"] == true {
                values["foreground"] = Value::Null;
            }
            let properties: CharacterProperties =
                serde_json::from_value(values).map_err(|_| DocumentError::UnsupportedFormatting)?;
            {
                let semantic = CharacterProperties {
                    bold: Some(
                        properties.bold == Some(true)
                            || properties.weight.is_some_and(|weight| weight >= 600),
                    ),
                    slant: Some(
                        if matches!(
                            properties.slant,
                            Some(
                                super::super::FontSlant::Italic | super::super::FontSlant::Oblique
                            )
                        ) {
                            super::super::FontSlant::Italic
                        } else {
                            super::super::FontSlant::Upright
                        },
                    ),
                    ..CharacterProperties::default()
                };
                if let Some((range, _)) = markdown_runs
                    .last_mut()
                    .filter(|(range, previous)| range.end == start && *previous == semantic)
                {
                    range.end = end;
                } else {
                    markdown_runs.push((start..end, semantic));
                }
            }
        }
        for (range, properties) in markdown_runs {
            if properties.bold != Some(true)
                && properties.slant != Some(super::super::FontSlant::Italic)
            {
                continue;
            }
            let syntax = super::super::markdown_serialization::markdown_character_fragment(
                scratch.projection(),
                range.clone(),
                &properties,
            );
            let source = scratch
                .projection()
                .source_range(range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let styled = scratch.prepare_source_only_patches(vec![SourcePatch::primary(
                source,
                self.encoding().encode_fragment(&syntax)?,
            )])?;
            publish(&mut scratch, styled, &mut sources)?;
        }
        self.prepare_text_edits_with_patches(
            vec![TextEdit::new(replaced, &export.source_plain_text)],
            Some(sources.source_patches()),
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
    first
        .map(|start| start..end)
        .ok_or(DocumentError::AmbiguousProjection)
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

    if document.format() != Format::Markdown {
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
    let mut prefixes = Vec::new();
    let mut suffixes = Vec::new();
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
            "margin_top":paragraph.margin_top,"margin_bottom":paragraph.margin_bottom,
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
            let mut link_defaults = CharacterProperties::default();
            let mut named = None;
            let paragraph_style = &block.style;
            let defaults = &block.direct_default_character;
            for span in spans.iter().filter(|span| span.range.contains(&pair[0])) {
                match &span.application {
                    StyleApplication::Named(id) => {
                        if named.is_some_and(|other| other != id) {
                            return Err(DocumentError::UnsupportedFormatting);
                        }
                        named = Some(id);
                    }
                    StyleApplication::Automatic(id) => {
                        if id.0 == "Link" {
                            let properties = projection
                                .style_sheet()
                                .automatic_character_properties(id)
                                .map_err(|_| DocumentError::UnsupportedFormatting)?;
                            super::super::rich_text::overlay(&mut link_defaults, &properties);
                        } else {
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
                    }
                    StyleApplication::Direct(properties) => {
                        super::super::rich_text::overlay(&mut direct, properties)
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
            let mut defaults = defaults.clone();
            super::super::rich_text::overlay(&mut defaults, &link_defaults);
            let style = projection
                .style_sheet()
                .resolve_assigned_paragraph_style(
                    projection.document_style(),
                    paragraph_style,
                    &block.direct_paragraph,
                    &defaults,
                    named,
                    &semantic,
                )
                .map_err(|_| DocumentError::UnsupportedFormatting)?
                .character;
            characters.push(json!({"start":pair[0]-range.start,"end":pair[1]-range.start,
                "font_families":style.font_families,"font_axes":style.font_axes,"size":style.size,"weight":style.weight,"base_weight":style.base_weight,
                "bold":style.bold,"slant":style.slant,"foreground":style.foreground,"foreground_is_default":style.foreground_is_default,
                "background":style.background,"underline":style.underline,"strikethrough":style.strikethrough,"language":style.language,
                "direction":style.direction,"open_type_features":style.open_type_features,"letter_spacing":style.letter_spacing}));
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
        if !styles_match(&old_character, &new_character)
            || !styles_match(&old_paragraph, &new_paragraph)
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
    }
    Ok(prepared)
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

        ] {
            let original = open(source, format);
            let fragment = original
                .clipboard_fragment(0..original.text().len())
                .unwrap();
            let value: Export = serde_json::from_str(fragment.json()).unwrap();
            assert_eq!(value.source_bytes, source);
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
        assert_eq!(value.character_runs[0]["bold"], true);
        let pasted = paste_empty(&fragment, &value, Format::Markdown);
        assert_eq!(pasted.source_bytes(), b"__old__");
    }

    #[test]
    fn rectangular_clipboard_owns_only_selected_source_segments_and_resolved_styles() {
        let empty = open(b"", Format::Markdown);
        let empty_fragment = empty.clipboard_fragment_rows(&[vec![0..0]]).unwrap();
        let empty_value: Export = serde_json::from_str(empty_fragment.json()).unwrap();
        assert!(empty_value.source_text.is_empty());
        assert!(empty_value.source_segments.is_empty());
        for (format, source) in [
            (Format::Markdown, "__ab__ outside-one\n\n__cd__ outside-two"),

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
        for format in [Format::MarkdownSource] {
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
    fn passive_html_import_retains_inline_styles_without_html_document_mode() {
        let (fragment, text) = ClipboardFragment::from_html_utf8(
            b"<style>p{color:green}</style><script>bad()</script><p><b>A&amp;B</b> <span style='color:#ff0000'>red</span></p>",
        ).unwrap();
        assert_eq!(text, "A&B red");
        let value: Export = serde_json::from_str(fragment.json()).unwrap();
        assert!(value.is_rich);
        assert_eq!(value.source_format, 1);
        assert_eq!(value.source_bytes, text.as_bytes());
        assert_eq!(value.character_runs[0]["bold"], true);
        assert!(value
            .character_runs
            .iter()
            .any(|run| run["foreground"]["red"] == 1.0 && run["foreground"]["green"] == 0.0));
        assert!(ClipboardFragment::from_json(fragment.json(), &text).is_ok());
        assert!(fragment.can_insert_rich_source(Format::Markdown, &text));
        assert!(!fragment.can_insert_rich_source(Format::PlainText, &text));
    }

    #[test]
    fn normalized_html_markdown_reuses_inline_writer_and_retains_breaks() {
        let (fragment, plain) = ClipboardFragment::from_html_utf8(
            b"<p><b><i>A * B &amp; C</i></b><br>next</p><p>last</p>",
        )
        .unwrap();
        assert_eq!(plain, "A * B & C\nnext\nlast");
        let mut document = open(b"", Format::Markdown);
        let prepared = document
            .prepare_clipboard_fragment(0..0, &fragment, &plain)
            .unwrap()
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        let saved = document.source_bytes();
        assert!(std::str::from_utf8(&saved)
            .unwrap()
            .contains(r"***A * B & C***"));
        let reopened = open(&saved, Format::Markdown);
        for document in [&document, &reopened] {
            assert_eq!(document.text(), plain);
            assert_eq!(
                document
                    .projection()
                    .hard_breaks_for_region(&(0..plain.len())),
                vec![9, 14]
            );
            let style = crate::layout::DocumentLayoutStyles::semantic_character_at(
                document.projection(),
                0,
                false,
            )
            .unwrap();
            assert!(style.bold);
            assert_eq!(style.slant, super::super::super::FontSlant::Italic);
        }
    }

    #[test]
    fn normalized_html_quote_transform_retains_break_offsets_and_styles() {
        let (fragment, _) =
            ClipboardFragment::from_html_utf8(b"<p><b>\"bold\"</b><br>next</p><p>last</p>")
                .unwrap();
        let (text, transformed) = fragment
            .transform_quotes(
                None,
                |character, _| {
                    if character == '\"' {
                        '“'
                    } else {
                        character
                    }
                },
            )
            .unwrap();
        let value: Export = serde_json::from_str(transformed.json()).unwrap();
        assert!(value.is_rich);
        assert_eq!(text, "“bold“\nnext\nlast");
        assert_eq!(value.hard_breaks, vec![10, 15]);
        assert_eq!(value.character_runs[0]["bold"], true);
        assert_eq!(value.character_runs[0]["end"], 10);
    }

    #[test]
    fn passive_html_import_resolves_natural_paragraph_direction() {
        let (fragment, _) = ClipboardFragment::from_html_utf8(
            "<p style='text-align:end'>العربية Latin</p>".as_bytes(),
        )
        .unwrap();
        let value: Export = serde_json::from_str(fragment.json()).unwrap();
        assert_eq!(value.paragraph_runs[0]["base_direction"], "Natural");
        assert_eq!(value.paragraph_runs[0]["resolved_direction"], "RightToLeft");
        assert_eq!(value.paragraph_runs[0]["alignment"], "End");
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
