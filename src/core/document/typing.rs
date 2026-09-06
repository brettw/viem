//! Atomic insertion with view-local character overrides. The speculative
//! stages never publish history. Their local patch lists are composed against
//! the original source, then verified and committed as one model transaction.
use super::*;
use crate::document::{FontSlant, StylePropertyValue};

use super::replacement::PatchComposition;

impl Document {
    /// Validate a sparse typing declaration without creating source syntax.
    pub fn validate_typing_properties(
        &self,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<CharacterProperties, DocumentError> {
        let mut properties = CharacterProperties::default();
        for (property, value) in values {
            super::super::style::set_character_property(
                &StyleId::from("Typing"),
                &mut properties,
                *property,
                value,
            )
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        }
        super::super::style::validate_character_properties(&StyleId::from("Typing"), &properties)
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        match self.format() {
            Format::Html | Format::HtmlSource | Format::Rtf => {}
            Format::Markdown | Format::MarkdownSource
                if values.iter().all(|(p, _)| {
                    matches!(
                        p,
                        StyleProperty::CharacterBold | StyleProperty::CharacterSlant
                    )
                }) => {}
            _ => return Err(DocumentError::UnsupportedFormatting),
        }
        Ok(properties)
    }

    fn typing_context_matches(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
        p: &CharacterProperties,
    ) -> bool {
        let len = self.projection().text_tree().byte_len();
        if len == 0 {
            return false;
        }
        let sample = if at == len || at > 0 && affinity == BoundaryAffinity::Upstream {
            at - 1
        } else {
            at
        };
        if matches!(self.format(), Format::Markdown | Format::MarkdownSource) {
            let spans = self
                .projection()
                .style_spans_for_region(&(sample..sample + 1));
            let has = |style| {
                spans.iter().any(|s| {
                    s.range.contains(&sample) && s.application == StyleApplication::Semantic(style)
                })
            };
            return p
                .bold
                .map_or(true, |b| b == has(SemanticInlineStyle::Strong))
                && p.slant.map_or(true, |s| {
                    (s != FontSlant::Upright) == has(SemanticInlineStyle::Emphasis)
                });
        }
        let Some(current) =
            super::super::rich_text::resolved_character_at(self.projection(), sample)
        else {
            return false;
        };
        macro_rules! matches {
            ($field:ident) => {
                p.$field.as_ref().map_or(true, |v| v == &current.$field)
            };
        }
        matches!(font_families)
            && matches!(size)
            && p.weight.map_or(true, |v| v == current.base_weight)
            && matches!(bold)
            && matches!(slant)
            && matches!(foreground)
            && p.background.map_or(true, |v| Some(v) == current.background)
            && matches!(underline)
            && matches!(strikethrough)
            && p.language
                .as_ref()
                .map_or(true, |v| Some(v) == current.language.as_ref())
            && matches!(direction)
            && matches!(open_type_features)
            && matches!(letter_spacing)
            && matches!(baseline_shift)
    }

    fn prepare_typing_markdown_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let containing = self
            .projection()
            .style_spans_for_region(&range)
            .into_iter()
            .find(|span| {
                span.application == StyleApplication::Semantic(style)
                    && span.range.start <= range.start
                    && span.range.end >= range.end
            });
        if enabled && containing.is_some() || !enabled && containing.is_none() {
            return Ok(self.no_op_prepared());
        }
        let source = self
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut patches = Vec::new();
        if enabled {
            let preceding = self
                .projection()
                .style_spans_for_region(&(range.start.saturating_sub(1)..range.start))
                .into_iter()
                .find(|span| {
                    span.application == StyleApplication::Semantic(style)
                        && span.range.end == range.start
                });
            if let Some(previous) = preceding {
                let content = self
                    .projection()
                    .source_range(previous.range)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let closing = if self.format() == Format::MarkdownSource {
                    let bytes = self
                        .state()
                        .source
                        .bytes_in(content.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let marker = markdown_style_markers(style)
                        .iter()
                        .map(|m| self.encoding().encode_fragment(m))
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .find(|m| bytes.starts_with(m) && bytes.ends_with(m))
                        .ok_or(DocumentError::UnsupportedFormatting)?;
                    content.end - marker.len()..content.end
                } else {
                    self.markdown_style_removal_patches(&content, style)?[1].range()
                };
                let marker = self
                    .state()
                    .source
                    .bytes_in(closing.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                patches.push(SourcePatch::primary(closing, Vec::new()));
                patches.push(SourcePatch::primary(source.end..source.end, marker));
                return if self.format() == Format::MarkdownSource {
                    self.prepare_html_source_patches(patches)
                } else {
                    self.prepare_source_only_patches(patches)
                };
            }
        }
        if enabled {
            let selected = self
                .projection()
                .text_tree()
                .slice(range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            let marker = if style == SemanticInlineStyle::Strong {
                "**"
            } else {
                "*"
            };
            if selected.contains(marker) {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            let bytes = self.encoding().encode_fragment(marker)?;
            patches.push(SourcePatch::primary(
                source.start..source.start,
                bytes.clone(),
            ));
            patches.push(SourcePatch::primary(source.end..source.end, bytes));
        } else {
            let span = containing.expect("checked containing");
            let (content, opening, closing, marker) = if self.format() == Format::MarkdownSource {
                let full = self
                    .projection()
                    .source_range(span.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let bytes = self
                    .state()
                    .source
                    .bytes_in(full.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let marker = markdown_style_markers(style)
                    .iter()
                    .map(|m| self.encoding().encode_fragment(m))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .find(|m| bytes.starts_with(m) && bytes.ends_with(m))
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                (
                    full.start + marker.len()..full.end - marker.len(),
                    full.start..full.start + marker.len(),
                    full.end - marker.len()..full.end,
                    marker,
                )
            } else {
                let content = self
                    .projection()
                    .source_range(span.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let removal = self.markdown_style_removal_patches(&content, style)?;
                let opening = removal[0].range();
                let closing = removal[1].range();
                let marker = self
                    .state()
                    .source
                    .bytes_in(opening.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                (content, opening, closing, marker)
            };
            if source.start == content.start {
                patches.push(SourcePatch::primary(opening, Vec::new()));
            } else {
                patches.push(SourcePatch::primary(
                    source.start..source.start,
                    marker.clone(),
                ));
            }
            if source.end == content.end {
                patches.push(SourcePatch::primary(closing, Vec::new()));
            } else {
                patches.push(SourcePatch::primary(source.end..source.end, marker));
            }
        }
        if self.format() == Format::MarkdownSource {
            self.prepare_html_source_patches(patches)
        } else {
            self.prepare_source_only_patches(patches)
        }
    }

    /// Returns the exact caret after the authored content, before generated
    /// closing syntax in a source-visible view.
    pub fn insert_with_typing_properties(
        &mut self,
        edit: FormattedPayloadEdit,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<usize, ModelTransactionError> {
        let (prepared, caret) = self.prepare_insertion_with_typing_properties(edit, values)?;
        self.commit_model_transaction(prepared)?;
        Ok(caret)
    }

    pub fn prepare_insertion_with_typing_properties(
        &self,
        edit: FormattedPayloadEdit,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let properties = self.validate_typing_properties(values)?;
        let at = edit.range.start;
        if matches!(self.format(), Format::Markdown | Format::MarkdownSource)
            && edit.payload.text().trim().is_empty()
        {
            let caret = at + edit.payload.text().len();
            return Ok((self.prepare_formatted_payload_edits(vec![edit])?, caret));
        }
        let single_replacement = !edit.range.is_empty()
            && self.hard_line_snapshot().next_grapheme_boundary(at) == Some(edit.range.end);
        if (edit.range.is_empty() || single_replacement)
            && self.typing_context_matches(
                at,
                if single_replacement {
                    BoundaryAffinity::Downstream
                } else {
                    edit.boundary_affinity
                        .unwrap_or(BoundaryAffinity::Downstream)
                },
                &properties,
            )
        {
            // Replacing one already-matching grapheme retains its existing
            // source-backed style, so no redundant wrapper/table edit is needed.
            let old_end = edit.range.end;
            let prepared = self.prepare_formatted_payload_edits(vec![edit])?;
            let caret = prepared
                .text_position_map()
                .map_text_point(
                    self.text_point(old_end)?,
                    Association::AfterInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            return Ok((prepared, caret));
        }
        let mut caret = at + edit.payload.text().len();
        if edit.payload.text().is_empty() {
            return Ok((self.no_op_prepared(), at));
        }
        let mut scratch = Document {
            id: self.id,
            history: super::super::new_document_history(self.state().clone()),
            open_work: self.open_work,
            next_revision: self.next_revision,
            next_projected_block_id: self.next_projected_block_id,
            edit_group_depth: 0,
            edit_group_generation: 0,
            position_map_capture: None,
            artifact_binding: None,
            pending_artifact_writes: Default::default(),
            next_artifact_write_token: 1,
            next_save_sequence: 1,
            last_successful_save_sequence: 0,
            read_only: false,
            recovered_dirty: false,
        };
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let publish = |scratch: &mut Document,
                       prepared: PreparedModelTransaction,
                       sources: &mut PatchComposition,
                       formatted: &mut PatchComposition|
         -> Result<(), ModelTransactionError> {
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            if let PreparedPublication::State(state) = &prepared.publication {
                let mut delta = 0isize;
                let replacements = prepared
                    .summary
                    .formatted_splices
                    .iter()
                    .map(|splice| {
                        let range = splice.old_range();
                        let start = (range.start as isize + delta) as usize;
                        let length = splice.inserted_len();
                        delta += length as isize - range.len() as isize;
                        Ok((
                            range,
                            state
                                .projection
                                .text_tree()
                                .slice(start..start + length)
                                .map_err(DocumentError::FormattedTextStorage)?
                                .into_bytes(),
                        ))
                    })
                    .collect::<Result<Vec<_>, DocumentError>>()?;
                for (range, bytes) in replacements.into_iter().rev() {
                    formatted.splice(range, &bytes);
                }
            }
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };
        let first = scratch.prepare_formatted_payload_edits(vec![edit])?;
        publish(&mut scratch, first, &mut sources, &mut formatted)?;
        let projection = scratch.projection();
        let start = if projection
            .is_logical_grapheme_boundary(at)
            .map_err(DocumentError::FormattedTextStorage)?
        {
            at
        } else {
            projection
                .previous_logical_grapheme_boundary(at)
                .map_err(DocumentError::FormattedTextStorage)?
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        if !projection
            .is_logical_grapheme_boundary(caret)
            .map_err(DocumentError::FormattedTextStorage)?
        {
            caret = projection
                .next_logical_grapheme_boundary(caret)
                .map_err(DocumentError::FormattedTextStorage)?
                .ok_or(DocumentError::AmbiguousProjection)?;
        }
        let mut selection = start..caret;
        let apply = |scratch: &mut Document,
                     prepared: PreparedModelTransaction,
                     selection: &mut Range<usize>,
                     caret: &mut usize,
                     sources: &mut PatchComposition,
                     formatted: &mut PatchComposition|
         -> Result<(), ModelTransactionError> {
            let map = prepared.text_position_map();
            let start = scratch.text_anchor(
                scratch.text_point(selection.start)?,
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?;
            let end = scratch.text_anchor(
                scratch.text_point(*caret)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Upstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?;
            let new_start = map
                .map_text_anchor(start)?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            *caret = map
                .map_text_anchor(end)?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            *selection = new_start..*caret;
            publish(scratch, prepared, sources, formatted)
        };
        if matches!(self.format(), Format::Markdown | Format::MarkdownSource) {
            for (style, enabled) in [
                (
                    SemanticInlineStyle::Emphasis,
                    properties.slant.map(|s| s != FontSlant::Upright),
                ),
                (SemanticInlineStyle::Strong, properties.bold),
            ] {
                if let Some(enabled) = enabled {
                    let prepared =
                        scratch.prepare_typing_markdown_style(selection.clone(), style, enabled)?;
                    apply(
                        &mut scratch,
                        prepared,
                        &mut selection,
                        &mut caret,
                        &mut sources,
                        &mut formatted,
                    )?;
                }
            }
        } else {
            let prepared =
                scratch.prepare_model_request(ModelRequest::SetDirectCharacterProperties {
                    document: scratch.id(),
                    revision: scratch.revision(),
                    range: selection.clone(),
                    values: values.to_vec(),
                })?;
            apply(
                &mut scratch,
                prepared,
                &mut selection,
                &mut caret,
                &mut sources,
                &mut formatted,
            )?;
        }
        let patches = sources
            .patches()
            .into_iter()
            .map(|(range, bytes)| SourcePatch::primary(range, bytes))
            .collect();
        let edits = formatted
            .patches()
            .into_iter()
            .map(|(range, bytes)| {
                TextEdit::new(
                    range,
                    String::from_utf8(bytes)
                        .expect("formatted patches retain validated UTF-8 boundaries"),
                )
            })
            .collect();
        let prepared = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        Ok((prepared, caret))
    }
}
