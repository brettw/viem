//! Snapshot-bound substitutions. Captures are read-only scalar ranges; only the
//! replaced ranges are editing boundaries and therefore require grapheme stops.
use super::replacement::PatchComposition;
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplacementFragment {
    Literal(FormattedTextPayload),
    Capture(Range<usize>),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FragmentEdit {
    pub range: Range<usize>,
    pub fragments: Vec<ReplacementFragment>,
}
#[derive(Clone)]
struct CapturedStyle {
    range: Range<usize>,
    properties: CharacterProperties,
    named: Option<StyleId>,
    code: bool,
}
impl Document {
    pub(super) fn prepare_fragment_edits(
        &self,
        mut edits: Vec<FragmentEdit>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        for edit in &edits {
            self.validate_range(&edit.range)?;
        }
        for pair in edits.windows(2) {
            if pair[0].range.end > pair[1].range.start
                || (pair[0].range.is_empty() && pair[0].range == pair[1].range)
            {
                return Err(DocumentError::OverlappingEdits.into());
            }
        }
        let snapshot = self.hard_line_snapshot();
        let mut payloads = Vec::new();
        let mut captures = Vec::new();
        let mut capture_groups = Vec::new();
        let keep_styles = self.format().is_wysiwyg();
        for edit in &edits {
            let capture_start = captures.len();
            let mut text = String::new();
            let mut breaks = Vec::new();
            for fragment in &edit.fragments {
                let start = text.len();
                match fragment {
                    ReplacementFragment::Literal(payload) => {
                        if payload.document() != self.id() {
                            return Err(DocumentError::WrongDocument.into());
                        }
                        if payload.revision() != self.revision() {
                            return Err(DocumentError::WrongSnapshot {
                                expected: self.revision(),
                                actual: payload.revision(),
                            }
                            .into());
                        }
                        text.push_str(payload.text());
                        breaks.extend(payload.break_offsets().iter().map(|offset| start + offset));
                    }
                    ReplacementFragment::Capture(range) => {
                        if range.start > range.end
                            || range.end > self.text().len()
                            || !self.text().is_char_boundary(range.start)
                            || !self.text().is_char_boundary(range.end)
                        {
                            return Err(DocumentError::InvalidRange {
                                start: range.start,
                                end: range.end,
                                length: self.text().len(),
                            }
                            .into());
                        }
                        text.push_str(&self.text()[range.clone()]);
                        breaks.extend(
                            self.projection()
                                .hard_breaks_for_region(range)
                                .iter()
                                .map(|offset| start + offset - range.start),
                        );
                        if keep_styles {
                            for mut style in self.capture_style_runs(range.clone())? {
                                style.range = start + style.range.start - range.start
                                    ..start + style.range.end - range.start;
                                captures.push((edit.range.start, style));
                            }
                        }
                    }
                }
            }
            let payload = FormattedTextPayload::new(&snapshot, text.clone(), breaks)
                .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
            payloads.push(FormattedPayloadEdit::new(edit.range.clone(), payload));
            capture_groups.push((text, capture_start..captures.len()));
        }

        for ((text, capture_range), normalized) in capture_groups.into_iter().zip(&payloads) {
            if normalized.payload.text() != text {
                // HTML normalization preserves scalar order and may prepend a
                // previously generated protected space while simplifying it.
                // Capture boundaries are scalar boundaries, not byte counts.
                let extra = normalized
                    .payload
                    .text()
                    .chars()
                    .count()
                    .checked_sub(text.chars().count())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let boundaries = text
                    .char_indices()
                    .map(|(at, _)| at)
                    .chain(std::iter::once(text.len()))
                    .zip(
                        normalized
                            .payload
                            .text()
                            .char_indices()
                            .map(|(at, _)| at)
                            .chain(std::iter::once(normalized.payload.text().len()))
                            .skip(extra),
                    )
                    .collect::<BTreeMap<_, _>>();
                for (origin, capture) in &mut captures[capture_range] {
                    *origin = normalized.range.start;
                    capture.range = *boundaries
                        .get(&capture.range.start)
                        .ok_or(DocumentError::AmbiguousProjection)?
                        ..*boundaries
                            .get(&capture.range.end)
                            .ok_or(DocumentError::AmbiguousProjection)?;
                }
            }
        }
        payloads.retain(|edit| {
            snapshot
                .capture(edit.range.clone())
                .map_or(true, |current| {
                    current.text() != edit.payload.text()
                        || current.break_offsets() != edit.payload.break_offsets()
                })
        });
        if !keep_styles || captures.is_empty() {
            return self.prepare_formatted_payload_edits(payloads);
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let prepared = scratch.prepare_formatted_payload_edits(payloads)?;
        let captures = captures
            .into_iter()
            .map(
                |(origin, mut capture)| -> Result<_, ModelTransactionError> {
                    let destination = prepared
                        .text_position_map()
                        .map_text_point(
                            self.text_point(origin)?,
                            Association::BeforeInsertion,
                            BoundaryAffinity::Downstream,
                            DeletionRecovery::PreferFollowingThenPreceding,
                        )?
                        .value()
                        .ok_or(DocumentError::AmbiguousProjection)?
                        .offset();
                    capture.range =
                        destination + capture.range.start..destination + capture.range.end;
                    Ok(capture)
                },
            )
            .collect::<Result<Vec<_>, _>>()?;
        publish(&mut scratch, prepared, &mut sources, &mut formatted)?;
        for mut capture in captures {
            if capture.range.is_empty() {
                continue;
            }
            // Scalars from separate capture fragments may combine into one
            // grapheme. Appearance follows its containing item; no text endpoint
            // is changed or expanded by this styling-only operation.
            if !scratch
                .projection()
                .is_logical_grapheme_boundary(capture.range.start)
                .map_err(DocumentError::FormattedTextStorage)?
            {
                capture.range.start = scratch
                    .projection()
                    .previous_logical_grapheme_boundary(capture.range.start)
                    .map_err(DocumentError::FormattedTextStorage)?
                    .ok_or(DocumentError::AmbiguousProjection)?;
            }
            if !scratch
                .projection()
                .is_logical_grapheme_boundary(capture.range.end)
                .map_err(DocumentError::FormattedTextStorage)?
            {
                capture.range.end = scratch
                    .projection()
                    .next_logical_grapheme_boundary(capture.range.end)
                    .map_err(DocumentError::FormattedTextStorage)?
                    .ok_or(DocumentError::AmbiguousProjection)?;
            }
            if scratch
                .capture_style_runs(capture.range.clone())?
                .iter()
                .all(|current| {
                    current.properties == capture.properties
                        && current.named == capture.named
                        && current.code == capture.code
                })
            {
                continue;
            }
            if self.format() == Format::Markdown {
                if !capture.code {
                    let code_spans = scratch
                        .projection()
                        .style_spans_for_region(&capture.range)
                        .into_iter()
                        .filter(|span| {
                            span.application
                                == StyleApplication::Semantic(SemanticInlineStyle::Code)
                        })
                        .collect::<Vec<_>>();
                    for span in code_spans {
                        let selected = span.range.start.max(capture.range.start)
                            ..span.range.end.min(capture.range.end);
                        let patches = super::super::markdown_code::clear_patches(
                            &scratch,
                            &span.range,
                            &selected,
                        )?
                        .into_iter()
                        .map(|(range, syntax)| {
                            scratch
                                .encoding()
                                .encode_fragment(&syntax)
                                .map(|bytes| SourcePatch::primary(range, bytes))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                        let prepared = scratch.prepare_source_only_patches(patches)?;
                        publish(&mut scratch, prepared, &mut sources, &mut formatted)?;
                    }
                }
                for (style, enabled) in [
                    (
                        SemanticInlineStyle::Emphasis,
                        capture.properties.slant != Some(super::super::FontSlant::Upright),
                    ),
                    (
                        SemanticInlineStyle::Strong,
                        capture.properties.bold == Some(true),
                    ),
                ] {
                    let prepared = scratch.prepare_typing_markdown_style(
                        capture.range.clone(),
                        style,
                        enabled,
                    )?;
                    publish(&mut scratch, prepared, &mut sources, &mut formatted)?;
                }
                if capture.code
                    && !scratch
                        .projection()
                        .style_spans_for_region(&capture.range)
                        .iter()
                        .any(|span| {
                            span.application
                                == StyleApplication::Semantic(SemanticInlineStyle::Code)
                                && span.range.start <= capture.range.start
                                && span.range.end >= capture.range.end
                        })
                {
                    let prepared = scratch.prepare_semantic_style(
                        capture.range.clone(),
                        SemanticInlineStyle::Code,
                        true,
                    )?;
                    publish(&mut scratch, prepared, &mut sources, &mut formatted)?;
                }
            }
        }
        let patches = sources.source_patches();
        let text_edits = formatted.formatted_edits();
        self.prepare_text_edits_with_patches(text_edits, Some(patches))
    }
    fn capture_style_runs(
        &self,
        range: Range<usize>,
    ) -> Result<Vec<CapturedStyle>, ModelTransactionError> {
        let spans = self.projection().style_spans_for_region(&range);
        let mut boundaries = BTreeSet::from([range.start, range.end]);
        for span in &spans {
            boundaries.insert(span.range.start.max(range.start));
            boundaries.insert(span.range.end.min(range.end));
        }
        for offset in self.projection().hard_breaks_for_region(&range).iter() {
            boundaries.insert(*offset);
            boundaries.insert(*offset + 1);
        }
        let boundaries = boundaries.into_iter().collect::<Vec<_>>();
        let mut runs: Vec<CapturedStyle> = Vec::new();
        for pair in boundaries.windows(2) {
            let at = pair[0];
            if at == pair[1]
                || self
                    .projection()
                    .hard_breaks_for_region(&(at..at + 1))
                    .contains(&at)
            {
                continue;
            }
            let Some(resolved) =
                super::super::rich_text::resolved_character_at(self.projection(), at)
            else {
                continue;
            };
            let mut properties = CharacterProperties {
                font_families: Some(resolved.font_families),
                size: Some(resolved.size.into()),
                weight: Some(resolved.base_weight),
                bold: Some(resolved.bold),
                slant: Some(resolved.slant),
                foreground: (!resolved.foreground_is_default).then_some(resolved.foreground),
                background: resolved.background,
                underline: Some(resolved.underline),
                strikethrough: Some(resolved.strikethrough),
                language: resolved.language,
                direction: (resolved.direction != super::super::WritingDirection::Natural)
                    .then_some(resolved.direction),
                font_face: Some(resolved.font_face),
                font_axes: Some(resolved.font_axes),
                open_type_features: Some(resolved.open_type_features),
                letter_spacing: Some(resolved.letter_spacing),
            };
            let mut named = None;
            let mut code = false;
            for span in spans.iter().filter(|span| span.range.contains(&at)) {
                match &span.application {
                    StyleApplication::Named(id) => {
                        named = Some(id.clone());
                        code |= id.0 == "Code";
                    }
                    StyleApplication::Semantic(SemanticInlineStyle::Strong) => {
                        properties.bold = Some(true)
                    }
                    StyleApplication::Semantic(SemanticInlineStyle::Emphasis) => {
                        properties.slant = Some(super::super::FontSlant::Italic)
                    }
                    StyleApplication::Semantic(SemanticInlineStyle::Code) => code = true,
                    _ => {}
                }
            }
            if let Some(last) = runs.last_mut().filter(|last| {
                last.range.end == at
                    && last.properties == properties
                    && last.named == named
                    && last.code == code
            }) {
                last.range.end = pair[1];
            } else {
                runs.push(CapturedStyle {
                    range: at..pair[1],
                    properties,
                    named,
                    code,
                    });
            }
        }
        Ok(runs)
    }
}
fn publish(
    document: &mut Document,
    prepared: PreparedModelTransaction,
    sources: &mut PatchComposition,
    formatted: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    formatted.record_formatted(&prepared)?;
    document.commit_model_transaction(prepared)?;
    Ok(())
}
