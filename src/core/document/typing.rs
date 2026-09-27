//! Atomic insertion with view-local character overrides. The speculative
//! stages never publish history. Their local patch lists are composed against
//! the original source, then verified and committed as one model transaction.
use super::*;
use crate::document::{Color, FontSlant, StylePropertyValue};

use super::replacement::PatchComposition;

impl Document {
    pub(crate) fn validate_typing_payload(
        &self,
        edit: &FormattedPayloadEdit,
    ) -> Result<(), DocumentError> {
        if edit.payload.document() != self.id() {
            return Err(DocumentError::WrongDocument);
        }
        if edit.payload.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: edit.payload.revision(),
                actual: self.revision(),
            });
        }
        Ok(())
    }

    pub fn validate_typing_named_style(&self, style: &StyleId) -> Result<(), DocumentError> {
        let sheet = self.projection().style_sheet();
        if style.is_internal()
            || (!style.0.is_empty() && sheet.character_style(style).is_none())
            || !match self.format() {
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
        let sample = if at > 0
            && (at == self.projection().text_tree().byte_len()
                || affinity == BoundaryAffinity::Upstream)
        {
            at - 1
        } else {
            at
        };
        self.clean_named_character_at(sample, style, &CharacterProperties::default())
    }

    /// Resolve a chosen character style solely on the containing paragraph.
    /// Inline declarations and structural emphasis are intentionally excluded.
    pub(super) fn clean_named_character_at(
        &self,
        sample: usize,
        style: &StyleId,
        direct: &CharacterProperties,
    ) -> Result<super::super::ResolvedCharacterStyle, DocumentError> {
        let blocks = self.projection().blocks_for_region(&(sample..sample));
        let block = blocks
            .iter()
            .find(|block| block.range.contains(&sample) || block.range.start == sample)
            .or_else(|| blocks.last())
            .ok_or(DocumentError::AmbiguousProjection)?;
        self.projection()
            .style_sheet()
            .resolve_assigned_paragraph_style(
                self.projection().document_style(),
                &block.style,
                &block.direct_paragraph,
                &block.direct_default_character,
                (!style.0.is_empty()).then_some(style),
                direct,
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
        super::super::style::validate_direct_character_properties(
            &StyleId::from("Typing"),
            &properties,
        )
        .map_err(|_| DocumentError::UnsupportedFormatting)?;
        match self.format() {
            Format::Rtf => {}
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
            && p.size.map_or(true, |value| std::matches!(value, crate::document::FontSize::Points(size) if size == current.size))
            && p.weight.map_or(true, |v| v == current.base_weight)
            && matches!(bold)
            && matches!(slant)
            && matches!(foreground)
            && p.background.map_or(true, |v| {
                Some(v) == current.background || v.alpha == 0.0 && current.background.is_none()
            })
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
        let mut source = self
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        if enabled && self.format() == Format::Markdown {
            // Code keeps its interior literal. Emphasis on the complete
            // inherited Code run must enclose its delimiters, not become
            // newly visible asterisks inside the backticks.
            for span in self.projection().style_spans_for_region(&range) {
                if span.application != StyleApplication::Semantic(SemanticInlineStyle::Code)
                    || span.range.start < range.start
                    || range.end < span.range.end
                {
                    continue;
                }
                let content = self
                    .projection()
                    .source_range(span.range.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                if let Some((opening, closing)) =
                    super::super::markdown_code::delimiter_ranges(self, &content)?
                {
                    if span.range.start == range.start {
                        source.start = source.start.min(opening.start);
                    }
                    if span.range.end == range.end {
                        source.end = source.end.max(closing.end);
                    }
                }
            }
        }
        let mut patches = Vec::new();
        let (html_open, html_close) = if style == SemanticInlineStyle::Strong {
            ("<strong>", "</strong>")
        } else {
            ("<em>", "</em>")
        };
        let mut fallback = if enabled {
            vec![
                SourcePatch::primary(
                    source.start..source.start,
                    self.encoding().encode_fragment(html_open)?,
                ),
                SourcePatch::primary(
                    source.end..source.end,
                    self.encoding().encode_fragment(html_close)?,
                ),
            ]
        } else {
            Vec::new()
        };
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
                // A touching formatted run can still live inside a link or
                // another source scope. Moving its closing delimiter across
                // that hidden syntax would create crossing Markdown scopes.
                if closing.end == source.start {
                    let marker = self
                        .state()
                        .source
                        .bytes_in(closing.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    patches.push(SourcePatch::primary(closing, Vec::new()));
                    patches.push(SourcePatch::primary(source.end..source.end, marker));
                    return if self.format() == Format::MarkdownSource {
                        self.prepare_visible_source_patches(patches)
                    } else {
                        self.prepare_source_only_patches(patches).or_else(|error| {
                            if matches!(
                                error,
                                ModelTransactionError::Document(DocumentError::VerificationFailed)
                            ) {
                                self.prepare_source_only_patches(fallback)
                            } else {
                                Err(error)
                            }
                        })
                    };
                }
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
                let paired = self
                    .projection()
                    .style_spans_for_region(&span.range)
                    .iter()
                    .any(|other| {
                        other.range == span.range
                            && other.application
                                == StyleApplication::Semantic(
                                    if style == SemanticInlineStyle::Strong {
                                        SemanticInlineStyle::Emphasis
                                    } else {
                                        SemanticInlineStyle::Strong
                                    },
                                )
                    });
                let padding = if paired {
                    marker.len()
                        / if style == SemanticInlineStyle::Strong {
                            2
                        } else {
                            1
                        }
                        * 3
                } else {
                    marker.len()
                };
                (
                    full.start + padding..full.end - padding,
                    full.start + padding - marker.len()..full.start + padding,
                    full.end - padding..full.end - padding + marker.len(),
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
                fallback.push(SourcePatch::primary(opening.clone(), Vec::new()));
            } else {
                fallback.push(SourcePatch::primary(
                    opening.clone(),
                    self.encoding().encode_fragment(html_open)?,
                ));
                fallback.push(SourcePatch::primary(
                    source.start..source.start,
                    self.encoding().encode_fragment(html_close)?,
                ));
            }
            if source.end == content.end {
                fallback.push(SourcePatch::primary(closing.clone(), Vec::new()));
            } else {
                fallback.push(SourcePatch::primary(
                    source.end..source.end,
                    self.encoding().encode_fragment(html_open)?,
                ));
                fallback.push(SourcePatch::primary(
                    closing.clone(),
                    self.encoding().encode_fragment(html_close)?,
                ));
            }
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
            self.prepare_visible_source_patches(patches)
        } else {
            self.prepare_source_only_patches(patches).or_else(|error| {
                if matches!(
                    error,
                    ModelTransactionError::Document(DocumentError::VerificationFailed)
                ) {
                    self.prepare_source_only_patches(fallback)
                } else {
                    Err(error)
                }
            })
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
        self.prepare_insertion_with_typing_context(edit, named, values, None)
            .map(|(prepared, caret, _)| (prepared, caret))
    }

    /// Returns the transaction, final caret, and authored range start.
    /// Supporting whitespace is excluded from that range's typing style.
    pub(crate) fn prepare_insertion_with_typing_context(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        inherited: Option<&super::super::ReplacementTypingContext>,
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        // The captured first character supplies inheritance, while deliberate
        // pending menu choices still win. Only differing properties are later
        // written, so paragraph defaults remain sparse whenever they survive.
        let inherited_named =
            inherited.map(|context| context.named.clone().unwrap_or_else(|| StyleId::from("")));
        let effective_named = named.or(inherited_named.as_ref());
        let inherited_values = if named.is_none() {
            inherited.map(|context| {
                use StyleProperty as P;
                use StylePropertyValue as V;
                let c = &context.character;
                let mut result = vec![
                    (P::CharacterBold, V::Boolean(c.bold)),
                    (P::CharacterSlant, V::FontSlant(c.slant)),
                ];
                if self.format().is_rich_text() {
                    result.splice(
                        0..0,
                        [
                            (
                                P::CharacterFontFamilies,
                                V::FontFamilies(c.font_families.clone()),
                            ),
                            (P::CharacterSize, V::Float(c.size)),
                            (P::CharacterWeight, V::FontWeight(c.base_weight)),
                        ],
                    );
                    result.extend([
                        (P::CharacterUnderline, V::Boolean(c.underline)),
                        (P::CharacterStrikethrough, V::Boolean(c.strikethrough)),
                        (P::CharacterDirection, V::WritingDirection(c.direction)),
                        (
                            P::CharacterOpenTypeFeatures,
                            V::OpenTypeFeatures(c.open_type_features.clone()),
                        ),
                        (P::CharacterLetterSpacing, V::Float(c.letter_spacing)),
                        (
                            P::CharacterScriptPosition,
                            V::ScriptPosition(c.script_position),
                        ),
                    ]);
                    if !c.foreground_is_default {
                        result.push((P::CharacterForeground, V::Color(c.foreground)));
                    }
                    // A cleared character highlight must override a surviving
                    // paragraph's highlight. Transparent and absent are
                    // visually equivalent; context matching avoids writing a
                    // redundant override when no highlight survives.
                    result.push((
                        P::CharacterBackground,
                        V::Color(c.background.unwrap_or(Color {
                            red: 0.0,
                            green: 0.0,
                            blue: 0.0,
                            alpha: 0.0,
                        })),
                    ));
                    if let Some(v) = &c.language {
                        result.push((P::CharacterLanguage, V::Text(v.clone())));
                    }
                }
                result.retain(|(property, _)| {
                    !values.iter().any(|(explicit, _)| explicit == property)
                });
                result.extend_from_slice(values);
                result
            })
        } else {
            None
        };
        let values = inherited_values.as_deref().unwrap_or(values);
        let named = effective_named;
        if let Some(style) = named {
            self.validate_typing_named_style(style)?;
        }
        self.validate_typing_payload(&edit)?;
        let properties = self.validate_typing_properties(values)?;
        let mut at = edit.range.start;
        if named.is_none()
            && inherited.is_none()
            && self.format().is_markdown()
            && edit.payload.text().trim().is_empty()
        {
            let caret = at + edit.payload.text().len();
            return Ok((self.prepare_formatted_payload_edits(vec![edit])?, caret, at));
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
                && {
                    let sample = if at > 0
                        && (at == self.projection().text_tree().byte_len()
                            || affinity == BoundaryAffinity::Upstream)
                    {
                        at - 1
                    } else {
                        at
                    };
                    self.clean_named_character_at(sample, style, &properties)
                        .ok()
                        == crate::layout::DocumentLayoutStyles::semantic_character_at(
                            self.projection(),
                            at,
                            affinity == BoundaryAffinity::Upstream,
                        )
                        .ok()
                }
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
        let inherited_insertion = if edit.range.is_empty() {
            inherited
                .map(|context| {
                    super::markdown_typing::replacement_insertion(
                        self,
                        at,
                        affinity,
                        edit.payload.text(),
                        context,
                    )
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        let structural = if inherited_insertion.is_some() {
            inherited_insertion
        } else if edit.range.is_empty()
            && (!context_matches || self.format() == Format::MarkdownSource)
        {
            super::markdown_typing::insertion(
                self,
                at,
                edit.boundary_affinity
                    .unwrap_or(BoundaryAffinity::Downstream),
                edit.payload.text(),
                &properties,
            )?
        } else {
            None
        };
        if structural.is_none()
            && (edit.range.is_empty() || single_replacement)
            && context_matches
            && (inherited.is_none())
            && inherited.is_none_or(|context| context.paragraph.matches(self, at))
        {
            // Replacing one already-matching grapheme retains its existing
            // source-backed style, so no redundant wrapper/table edit is needed.
            let old_end = edit.range.end;
            let prepared = self.prepare_formatted_payload_edits(vec![edit])?;
            let caret = {
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
            return Ok((prepared, caret, at));
        }
        let mut caret = at + edit.payload.text().len();
        if edit.payload.text().is_empty() {
            return Ok((self.no_op_prepared(), at, at));
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
            )];
            if self.format().is_source_view() {
                caret = insertion.source_caret;
                at = caret - edit.payload.text().len();
                scratch.prepare_visible_source_patches(patches)?
            } else {
                let edits = vec![edit.text_edit()];
                scratch.prepare_text_edits_with_patches(edits, Some(patches))?
            }
        } else {
            let edits = vec![edit];
            scratch.prepare_formatted_payload_edits(edits)?
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
        if let Some(context) = inherited {
            // Clearing all content deliberately removes its source owners.
            // Replacement restores the first paragraph's assignment before
            // character traits, so headings stay headings and a later Return
            // still observes that paragraph style's following-style rule.
            let paragraph = &context.paragraph;
            let current = scratch
                .projection()
                .blocks_for_region(&(selection.start..selection.start))
                .into_iter()
                .find(|block| {
                    block.range.contains(&selection.start) || block.range.start == selection.start
                })
                .ok_or(DocumentError::AmbiguousProjection)?;
            if current.style != paragraph.style {
                let request = if let Some((ordered, _)) = paragraph.style.list_family_level() {
                    ModelRequest::SetListStyle {
                        document: scratch.id(),
                        revision: scratch.revision(),
                        range: selection.start..selection.start,
                        style: Some(if ordered {
                            super::super::ListStyle::Numbered
                        } else {
                            super::super::ListStyle::Bullet
                        }),
                    }
                } else {
                    ModelRequest::SetParagraphStyle {
                        document: scratch.id(),
                        revision: scratch.revision(),
                        range: selection.start..selection.start,
                        style: paragraph.style.clone(),
                    }
                };
                let prepared = scratch.prepare_model_request(request)?;
                apply(
                    &mut scratch,
                    prepared,
                    &mut selection,
                    &mut caret,
                    &mut sources,
                    &mut formatted,
                )?;
            }
            if self.format().is_rich_text() {
                let current = scratch
                    .projection()
                    .blocks_for_region(&(selection.start..selection.start))
                    .into_iter()
                    .find(|block| {
                        block.range.contains(&selection.start)
                            || block.range.start == selection.start
                    })
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let clear = current
                    .direct_paragraph
                    .declared_properties()
                    .difference(&paragraph.direct.declared_properties())
                    .copied()
                    .collect::<BTreeSet<_>>();
                if !clear.is_empty() {
                    let target = TextRange::new(
                        scratch.text_point(selection.start)?,
                        scratch.text_point(selection.start)?,
                    )?;
                    let prepared = scratch.prepare_persisted_style_intent(
                        PersistedStyleIntent::ClearDirectBlockProperties {
                            target: StyleBlockTarget::Paragraphs(target),
                            properties: clear,
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
                if current.direct_paragraph != paragraph.direct
                    && !paragraph.direct.declared_properties().is_empty()
                {
                    let target = TextRange::new(
                        scratch.text_point(selection.start)?,
                        scratch.text_point(selection.start)?,
                    )?;
                    let prepared = scratch.prepare_persisted_style_intent(
                        PersistedStyleIntent::SetDirectBlockProperties {
                            target: StyleBlockTarget::Paragraphs(target),
                            properties: paragraph.direct.clone(),
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
            }
        }
        if let Some(style) = named {
            let prepared =
                scratch.prepare_character_style_choice(selection.clone(), style.clone())?;
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
        let patches = sources.source_patches();
        let edits = formatted.formatted_edits();
        let prepared = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        Ok((prepared, caret, selection.start))
    }
}
