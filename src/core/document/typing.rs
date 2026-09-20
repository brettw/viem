//! Atomic insertion with view-local character overrides. The speculative
//! stages never publish history. Their local patch lists are composed against
//! the original source, then verified and committed as one model transaction.
use super::*;
use crate::document::{FontSlant, StylePropertyValue};

use super::replacement::PatchComposition;

impl Document {
    pub fn validate_typing_named_style(&self, style: &StyleId) -> Result<(), DocumentError> {
        let sheet = self.projection().style_sheet();
        if style.is_internal()
            || (!style.0.is_empty() && sheet.character_style(style).is_none())
            || !match self.format() {
                Format::Html | Format::HtmlSource => true,
                Format::Rtf => style.0.is_empty() || style.0.starts_with("RtfC"),
                Format::Markdown | Format::MarkdownSource => {
                    style.0 == "Code" || style.0.is_empty()
                }
                _ => false,
            }
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        Ok(())
    }

    pub fn typing_named_style_at(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
        style: &StyleId,
    ) -> Result<super::super::ResolvedCharacterStyle, DocumentError> {
        self.text_point(at)?;
        self.validate_typing_named_style(style)?;
        let sample =
            if at > 0 && (at == self.text().len() || affinity == BoundaryAffinity::Upstream) {
                at - 1
            } else {
                at
            };
        let blocks = self.projection().blocks_for_region(&(sample..sample));
        let block = blocks
            .iter()
            .find(|block| block.range.contains(&sample) || block.range.start == sample)
            .or_else(|| blocks.last())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut paragraph_style = &block.style;
        let mut defaults = &block.direct_default_character;
        let spans = self
            .projection()
            .style_spans_for_region(&(sample..(sample + 1).min(self.text().len())));
        let mut direct = CharacterProperties::default();
        for span in &spans {
            match &span.application {
                StyleApplication::Direct(value) => {
                    super::super::rich_text::overlay(&mut direct, value)
                }
                StyleApplication::Semantic(SemanticInlineStyle::Strong) => direct.bold = Some(true),
                StyleApplication::Semantic(SemanticInlineStyle::Emphasis) => {
                    direct.slant = Some(FontSlant::Italic)
                }
                StyleApplication::SourceParagraph {
                    style,
                    defaults: value,
                } => {
                    paragraph_style = style;
                    defaults = value;
                }
                _ => {}
            }
        }
        self.projection()
            .style_sheet()
            .resolve_assigned_paragraph_style(
                self.projection().document_style(),
                paragraph_style,
                &block.direct_paragraph,
                defaults,
                (!style.0.is_empty()).then_some(style),
                &direct,
            )
            .map(|resolved| resolved.character)
            .map_err(|_| DocumentError::UnsupportedFormatting)
    }
    /// Source-visible emphasis ends can be crossed without editing their
    /// spelling. Only closing ranges certified by the current semantic spans
    /// participate; marker-like prose, escapes and code are never skipped.
    pub(crate) fn markdown_source_typing_exit(
        &self,
        at: usize,
        requested: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<Option<(usize, Vec<(StyleProperty, StylePropertyValue)>)>, DocumentError> {
        if self.format() != Format::MarkdownSource {
            return Ok(None);
        }
        self.text_point(at)?;
        let desired = self.validate_typing_properties(requested)?;
        let off = |style| match style {
            SemanticInlineStyle::Strong => desired.bold == Some(false),
            SemanticInlineStyle::Emphasis => desired.slant == Some(FontSlant::Upright),
            SemanticInlineStyle::Code => false,
        };
        let text = self.projection().text_tree();
        let spans = self.projection().style_spans_for_region(
            &(at.saturating_sub(1)..at.saturating_add(1).min(text.byte_len())),
        );
        let mut closures = Vec::new();
        for span in &spans {
            let StyleApplication::Semantic(
                style @ (SemanticInlineStyle::Strong | SemanticInlineStyle::Emphasis),
            ) = span.application
            else {
                continue;
            };
            if span.range.start >= at || span.range.end <= at {
                continue;
            }
            let width = if style == SemanticInlineStyle::Strong {
                2
            } else {
                1
            };
            let paired = spans.iter().any(|other| {
                other.range == span.range
                    && other.application
                        == StyleApplication::Semantic(if style == SemanticInlineStyle::Strong {
                            SemanticInlineStyle::Emphasis
                        } else {
                            SemanticInlineStyle::Strong
                        })
            });
            let width = if paired { 3 } else { width };
            if span.range.len() < width * 2 {
                continue;
            }
            let opening = text
                .slice(span.range.start..span.range.start + width)
                .map_err(DocumentError::FormattedTextStorage)?;
            if !opening.bytes().all(|byte| byte == b'*')
                && !opening.bytes().all(|byte| byte == b'_')
            {
                continue;
            }
            let closing = span.range.end - width..span.range.end;
            if text
                .slice(closing.clone())
                .map_err(DocumentError::FormattedTextStorage)?
                != opening
            {
                continue;
            }
            closures.push((closing, style));
        }
        closures.sort_by_key(|(range, _)| (range.start, range.end));
        let mut position = at;
        let mut exit = None;
        for (closing, style) in &closures {
            if closing.start > position {
                break;
            }
            // Equal ranges represent combined emphasis. A point in the
            // interior of a delimiter is not a legal semantic exit boundary.
            if closing.start < at || closing.end < position {
                continue;
            }
            position = closing.end;
            if off(*style) {
                exit = Some(position);
            }
        }
        let Some(exit) = exit else {
            return Ok(None);
        };
        let mut preserved = Vec::new();
        let active = |style| {
            spans.iter().any(|span| {
                span.range.start < at
                    && at < span.range.end
                    && span.application == StyleApplication::Semantic(style)
            })
        };
        if desired.bold.is_none() && active(SemanticInlineStyle::Strong) {
            preserved.push((
                StyleProperty::CharacterBold,
                StylePropertyValue::Boolean(true),
            ));
        }
        if desired.slant.is_none() && active(SemanticInlineStyle::Emphasis) {
            preserved.push((
                StyleProperty::CharacterSlant,
                StylePropertyValue::FontSlant(FontSlant::Italic),
            ));
        }
        Ok(Some((exit, preserved)))
    }

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
        if self.format().is_markdown() {
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
            && matches!(script_position)
    }

    pub(super) fn prepare_typing_markdown_style(
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
        self.prepare_insertion_with_typing_style(edit, None, values)
    }

    pub fn insert_with_typing_style(
        &mut self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<usize, ModelTransactionError> {
        let (prepared, caret) = self.prepare_insertion_with_typing_style(edit, named, values)?;
        self.commit_model_transaction(prepared)?;
        Ok(caret)
    }

    pub fn prepare_insertion_with_typing_style(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if let Some(style) = named {
            self.validate_typing_named_style(style)?;
        }
        let edit = self.normalize_typing_payload(edit)?;
        let properties = self.validate_typing_properties(values)?;
        let mut at = edit.range.start;
        if named.is_none()
            && self.format().is_markdown()
            && edit.payload.text().trim().is_empty()
        {
            let caret = at + edit.payload.text().len();
            return Ok((self.prepare_formatted_payload_edits(vec![edit])?, caret));
        }
        let single_replacement = !edit.range.is_empty()
            && self.hard_line_snapshot().next_grapheme_boundary(at) == Some(edit.range.end);
        let affinity = if single_replacement {
            BoundaryAffinity::Downstream
        } else {
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream)
        };
        let named_matches = named.map_or(true, |style| {
            self.projection()
                .selected_named_styles(at..at, affinity)
                .character
                .as_ref()
                == (!style.0.is_empty()).then_some(style)
        });
        let context_matches = named_matches
            && self.typing_context_matches(
                at,
                if single_replacement {
                    BoundaryAffinity::Downstream
                } else {
                    edit.boundary_affinity
                        .unwrap_or(BoundaryAffinity::Downstream)
                },
                &properties,
            );
        let structural = if edit.range.is_empty() && !context_matches {
            super::super::html_typing::insertion(
                self,
                at,
                edit.boundary_affinity
                    .unwrap_or(BoundaryAffinity::Downstream),
                edit.payload.text(),
                &edit.html_protective_spaces,
                &properties,
            )?
            .or(super::markdown_typing::insertion(
                self,
                at,
                edit.boundary_affinity
                    .unwrap_or(BoundaryAffinity::Downstream),
                edit.payload.text(),
                &properties,
            )?)
        } else {
            None
        };
        if structural.is_none() && (edit.range.is_empty() || single_replacement) && context_matches
        {
            // Replacing one already-matching grapheme retains its existing
            // source-backed style, so no redundant wrapper/table edit is needed.
            let old_end = edit.range.end;
            let authored_end = edit.range.start + edit.payload.text().len();
            let prepared = self.prepare_formatted_payload_edits(vec![edit])?;
            let caret = if self.format() == Format::Html {
                // A right-hand protective space can be simplified in this
                // transaction. Mapping AfterInsertion across its shared old
                // boundary would skip that space as well as the typed text.
                if let PreparedPublication::State(state) = &prepared.publication {
                    if state
                        .projection
                        .is_logical_grapheme_boundary(authored_end)
                        .map_err(DocumentError::FormattedTextStorage)?
                    {
                        authored_end
                    } else {
                        state
                            .projection
                            .next_logical_grapheme_boundary(authored_end)
                            .map_err(DocumentError::FormattedTextStorage)?
                            .ok_or(DocumentError::AmbiguousProjection)?
                    }
                } else {
                    authored_end
                }
            } else {
                prepared
                    .text_position_map()
                    .map_text_point(
                        self.text_point(old_end)?,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )?
                    .value()
                    .ok_or(DocumentError::AmbiguousProjection)?
                    .offset()
            };
            return Ok((prepared, caret));
        }
        let mut caret = at + edit.payload.text().len();
        if edit.payload.text().is_empty() {
            return Ok((self.no_op_prepared(), at));
        }
        let mut scratch = self.scratch_document();
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
        let first = if let Some(insertion) = structural {
            let patches = vec![SourcePatch::primary(
                insertion.source,
                self.encoding().encode_fragment(&insertion.syntax)?,
            )
            .with_generated_text(self.format() == Format::Html)];
            if self.format().is_source_view() {
                caret = insertion.source_caret;
                at = caret - edit.payload.text().len();
                scratch.prepare_html_source_patches(patches)?
            } else {
                scratch.prepare_text_edits_with_patches(vec![edit.text_edit()], Some(patches))?
            }
        } else {
            scratch.prepare_formatted_payload_edits(vec![edit])?
        };
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
        if let Some(style) = named {
            let range = TextRange::new(
                scratch.text_point(selection.start)?,
                scratch.text_point(selection.end)?,
            )?;
            let prepared = scratch.prepare_persisted_style_intent(
                PersistedStyleIntent::AssignCharacterStyle {
                    range,
                    style: style.clone(),
                },
            )?;
            apply(
                &mut scratch,
                prepared,
                &mut selection,
                &mut caret,
                &mut sources,
                &mut formatted,
            )?;
        }
        if self.format().is_markdown() {
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
        } else if !scratch.typing_context_matches(caret, BoundaryAffinity::Upstream, &properties) {
            let changed_values = values
                .iter()
                .filter(|value| {
                    let property = scratch
                        .validate_typing_properties(std::slice::from_ref(value))
                        .expect("the complete sparse declaration was validated");
                    !scratch.typing_context_matches(caret, BoundaryAffinity::Upstream, &property)
                })
                .cloned()
                .collect();
            let prepared =
                scratch.prepare_model_request(ModelRequest::SetDirectCharacterProperties {
                    document: scratch.id(),
                    revision: scratch.revision(),
                    range: selection.clone(),
                    values: changed_values,
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
        let patches = sources.source_patches(&scratch.state().source)?;
        let edits = formatted.formatted_edits(&scratch)?;
        let prepared = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        Ok((prepared, caret))
    }
}
