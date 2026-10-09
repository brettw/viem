//! Source-backed passive HTML character effects, prepared and verified atomically.
use super::replacement::PatchComposition;
use super::*;
use crate::document::{ResolvedCharacterStyle, SourceToTextError, StylePropertyValue};

pub(super) fn inline_boolean(
    properties: &CharacterProperties,
    property: StyleProperty,
) -> Option<bool> {
    match property {
        StyleProperty::CharacterUnderline => properties.underline,
        StyleProperty::CharacterSuperscript => properties.superscript,
        StyleProperty::CharacterSubscript => properties.subscript,
        _ => None,
    }
}

pub(crate) fn resolved_inline_boolean(
    properties: &ResolvedCharacterStyle,
    property: StyleProperty,
) -> bool {
    match property {
        StyleProperty::CharacterUnderline => properties.underline,
        StyleProperty::CharacterSuperscript => properties.superscript,
        StyleProperty::CharacterSubscript => properties.subscript,
        _ => false,
    }
}

fn tags(property: StyleProperty) -> Result<&'static [&'static str], DocumentError> {
    match property {
        StyleProperty::CharacterUnderline => Ok(&["ins", "u"]),
        StyleProperty::CharacterSuperscript => Ok(&["sup"]),
        StyleProperty::CharacterSubscript => Ok(&["sub"]),
        _ => Err(DocumentError::UnsupportedFormatting),
    }
}

impl Document {
    fn inline_scope_delimiters(
        &self,
        scope: &Range<usize>,
        names: &[&str],
    ) -> Result<(Range<usize>, Range<usize>, Vec<u8>, Vec<u8>), ModelTransactionError> {
        let full = self
            .projection()
            .source_range(scope.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut delimiters = None;
        for name in names {
            let open = self.encoding().encode_fragment(&format!("<{name}>"))?;
            let close = self.encoding().encode_fragment(&format!("</{name}>"))?;
            let (opening, closing) = if self.format().is_source_view() {
                (
                    full.start..full.start + open.len(),
                    full.end.saturating_sub(close.len())..full.end,
                )
            } else {
                (
                    full.start.saturating_sub(open.len())..full.start,
                    full.end..full.end + close.len(),
                )
            };
            if self.state().source.bytes_in(opening.clone()).as_ref() == Some(&open)
                && self.state().source.bytes_in(closing.clone()).as_ref() == Some(&close)
            {
                delimiters = Some((opening, closing, open, close));
                break;
            }
        }
        if delimiters.is_none() {
            let boundary = |at, affinity| match self.projection().map_source_boundary(
                self.revision(),
                at,
                affinity,
            ) {
                Ok(point) => Some(point.formatted_offset),
                Err(SourceToTextError::InteriorHiddenSyntax {
                    upstream_formatted,
                    downstream_formatted,
                    ..
                }) if upstream_formatted == downstream_formatted => upstream_formatted,
                _ => None,
            };
            let scan = |mut first: usize, last: usize| -> Result<_, ModelTransactionError> {
                if matches!(
                    self.encoding(),
                    super::super::Encoding::Utf16Le | super::super::Encoding::Utf16Be
                ) {
                    first -= first % 2;
                }
                let bytes = self
                    .state()
                    .source
                    .bytes_in(first..last)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                Ok(self.encoding().decode_region(&bytes, first)?)
            };
            let before = scan(
                full.start.saturating_sub(4096),
                if self.format().is_source_view() {
                    full.end.min(full.start.saturating_add(4096))
                } else {
                    full.start
                },
            )?;
            let after = scan(
                if self.format().is_source_view() {
                    full.start.max(full.end.saturating_sub(4096))
                } else {
                    full.end
                },
                self.source_byte_len().min(full.end.saturating_add(4096)),
            )?;
            let opening = super::super::html::tokenize(&before.text)
                .into_iter()
                .rev()
                .find_map(|token| {
                    let super::super::html::TokenKind::Tag(tag) = token.kind else {
                        return None;
                    };
                    if tag.end || !names.contains(&tag.name.as_str()) {
                        return None;
                    }
                    let range = before.source_boundary(token.range.start)?
                        ..before.source_boundary(token.range.end)?;
                    if self.format().is_source_view() && range.start != full.start
                        || !self.format().is_source_view()
                            && boundary(range.end, BoundaryAffinity::Downstream)
                                != Some(scope.start)
                    {
                        return None;
                    }
                    Some((tag.name, range))
                });
            if let Some((name, opening)) = opening {
                let closing = super::super::html::tokenize(&after.text)
                    .into_iter()
                    .find_map(|token| {
                        let super::super::html::TokenKind::Tag(tag) = token.kind else {
                            return None;
                        };
                        if !tag.end || tag.name != name {
                            return None;
                        }
                        let range = after.source_boundary(token.range.start)?
                            ..after.source_boundary(token.range.end)?;
                        if self.format().is_source_view() && range.end != full.end
                            || !self.format().is_source_view()
                                && boundary(range.start, BoundaryAffinity::Upstream)
                                    != Some(scope.end)
                        {
                            return None;
                        }
                        Some(range)
                    });
                if let Some(closing) = closing {
                    let open = self
                        .state()
                        .source
                        .bytes_in(opening.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let close = self
                        .state()
                        .source
                        .bytes_in(closing.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    delimiters = Some((opening, closing, open, close));
                }
            }
        }
        delimiters.ok_or_else(|| DocumentError::UnsupportedFormatting.into())
    }

    pub(super) fn prepare_inline_property(
        &self,
        range: Range<usize>,
        property: StyleProperty,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        tags(property)?;
        if !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        if range.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        // Breaks are paragraph ownership boundaries, never part of an inline scope.
        let text = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let mut start = range.start;
        let mut segments = Vec::new();
        for (offset, ch) in text.char_indices() {
            if ch == '\n' {
                if start < range.start + offset {
                    segments.push(start..range.start + offset);
                }
                start = range.start + offset + 1;
            }
        }
        if start < range.end {
            segments.push(start..range.end);
        }
        for segment in segments.into_iter().rev() {
            let spans = scratch.projection().style_spans_for_region(&segment);
            let mut boundaries = BTreeSet::from([segment.start, segment.end]);
            for span in &spans {
                if matches!(&span.application, StyleApplication::Direct(p) if inline_boolean(p, property) == Some(true))
                {
                    boundaries.insert(span.range.start.max(segment.start));
                    boundaries.insert(span.range.end.min(segment.end));
                }
            }
            let boundaries: Vec<_> = boundaries.into_iter().collect();
            for pair in boundaries.windows(2).rev() {
                if pair[0] == pair[1] {
                    continue;
                }
                let mut selected = pair[0]..pair[1];
                // Enabling a script replaces the other script in the selection.
                if enabled {
                    let opposite = match property {
                        StyleProperty::CharacterSuperscript => {
                            Some(StyleProperty::CharacterSubscript)
                        }
                        StyleProperty::CharacterSubscript => {
                            Some(StyleProperty::CharacterSuperscript)
                        }
                        _ => None,
                    };
                    if let Some(opposite) = opposite {
                        let prepared =
                            scratch.prepare_inline_property(selected.clone(), opposite, false)?;
                        for patch in prepared.summary.source_patches.iter().rev() {
                            sources.splice(patch.range(), patch.replacement());
                        }
                        formatted.record_formatted(&prepared)?;
                        let map = prepared.text_position_map();
                        let mapped =
                            |at, association, affinity| -> Result<usize, ModelTransactionError> {
                                let anchor = scratch.text_anchor(
                                    scratch.text_point(at)?,
                                    association,
                                    affinity,
                                    DeletionRecovery::PreferFollowingThenPreceding,
                                )?;
                                Ok(map
                                    .map_text_anchor(anchor)?
                                    .value()
                                    .ok_or(DocumentError::AmbiguousProjection)?
                                    .offset())
                            };
                        selected = mapped(
                            selected.start,
                            Association::AfterInsertion,
                            BoundaryAffinity::Downstream,
                        )?
                            ..mapped(
                                selected.end,
                                Association::BeforeInsertion,
                                BoundaryAffinity::Upstream,
                            )?;
                        scratch.commit_model_transaction(prepared)?;
                    }
                }
                let depth = scratch
                    .projection()
                    .style_spans_for_region(&selected)
                    .len()
                    .saturating_add(1)
                    .min(64);
                for _ in 0..depth {
                    let prepared = scratch.prepare_typing_inline_property(
                        selected.clone(),
                        property,
                        enabled,
                    )?;
                    if prepared.summary.source_patches.is_empty() {
                        break;
                    }
                    for patch in prepared.summary.source_patches.iter().rev() {
                        sources.splice(patch.range(), patch.replacement());
                    }
                    formatted.record_formatted(&prepared)?;
                    let map = prepared.text_position_map();
                    let mapped =
                        |at, association, affinity| -> Result<usize, ModelTransactionError> {
                            let anchor = scratch.text_anchor(
                                scratch.text_point(at)?,
                                association,
                                affinity,
                                DeletionRecovery::PreferFollowingThenPreceding,
                            )?;
                            Ok(map
                                .map_text_anchor(anchor)?
                                .value()
                                .ok_or(DocumentError::AmbiguousProjection)?
                                .offset())
                        };
                    selected = mapped(
                        selected.start,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                    )?
                        ..mapped(
                            selected.end,
                            Association::BeforeInsertion,
                            BoundaryAffinity::Upstream,
                        )?;
                    scratch.commit_model_transaction(prepared)?;
                    if enabled {
                        break;
                    }
                }
                if !enabled && scratch.projection().style_spans_for_region(&selected).iter().any(|span|
                    matches!(&span.application, StyleApplication::Direct(p) if inline_boolean(p, property) == Some(true))) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
            }
        }
        self.prepare_text_edits_with_patch_policy(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
            true,
        )
    }

    pub(super) fn prepare_typing_inline_property(
        &self,
        range: Range<usize>,
        property: StyleProperty,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let names = tags(property)?;
        let containing = self.projection().style_spans_for_region(&range).into_iter().rev().find(|span| {
            span.range.start <= range.start && range.end <= span.range.end
                && matches!(&span.application, StyleApplication::Direct(p) if inline_boolean(p, property) == Some(true))
        });
        if enabled == containing.is_some() {
            return Ok(self.no_op_prepared());
        }
        self.validate_typing_properties_at(
            range.start,
            BoundaryAffinity::Downstream,
            &[(property, StylePropertyValue::Boolean(enabled))],
        )?;
        let mut source = self
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let mut patches = Vec::new();
        if enabled {
            let open = self
                .encoding()
                .encode_fragment(&format!("<{}>", names[0]))?;
            let close = self
                .encoding()
                .encode_fragment(&format!("</{}>", names[0]))?;
            patches.push(SourcePatch::primary(source.start..source.start, open));
            patches.push(SourcePatch::primary(source.end..source.end, close));
        } else {
            let span = containing.expect("checked containing scope");
            let scope = span.range;
            let (opening, closing, open, close) = self.inline_scope_delimiters(&scope, names)?;
            source.start = source.start.max(opening.end);
            source.end = source.end.min(closing.start);
            let mut inner = Vec::new();
            for span in self.projection().style_spans_for_region(&range) {
                let delimiters = match &span.application {
                    StyleApplication::Direct(p) => [
                        StyleProperty::CharacterUnderline,
                        StyleProperty::CharacterSuperscript,
                        StyleProperty::CharacterSubscript,
                    ]
                    .into_iter()
                    .find(|property| inline_boolean(p, *property) == Some(true))
                    .and_then(|property| {
                        self.inline_scope_delimiters(&span.range, tags(property).ok()?)
                            .ok()
                    }),
                    StyleApplication::Semantic(style) => {
                        let raw = self
                            .projection()
                            .source_range(span.range.clone())
                            .ok_or(DocumentError::AmbiguousProjection)?;
                        self.markdown_style_removal_patches(&raw, *style)
                            .ok()
                            .and_then(|p| {
                                let first = p[0].range();
                                let last = p[1].range();
                                Some((
                                    first.clone(),
                                    last.clone(),
                                    self.state().source.bytes_in(first)?,
                                    self.state().source.bytes_in(last)?,
                                ))
                            })
                            .or_else(|| {
                                self.inline_scope_delimiters(
                                    &span.range,
                                    match style {
                                        SemanticInlineStyle::Strong => &["b", "strong"],
                                        SemanticInlineStyle::Emphasis => &["i", "em"],
                                        SemanticInlineStyle::Code => &["code", "kbd", "samp", "tt"],
                                    },
                                )
                                .ok()
                            })
                    }
                    StyleApplication::Named(id) if id.0 == "Code" => self
                        .inline_scope_delimiters(&span.range, &["code", "kbd", "samp", "tt"])
                        .ok(),
                    StyleApplication::Automatic(id) if id.0 == "Strikethrough" => self
                        .inline_scope_delimiters(&span.range, &["del", "s", "strike"])
                        .ok(),
                    StyleApplication::Automatic(id) if id.0 == "Link" => {
                        self.inline_scope_delimiters(&span.range, &["a"]).ok()
                    }
                    _ => None,
                };
                let Some((first, last, left, right)) = delimiters else {
                    continue;
                };
                if first.start <= opening.start
                    || last.end >= closing.end
                    || inner.iter().any(|(start, _, _, _)| *start == first)
                {
                    continue;
                }
                if range.start == span.range.start {
                    source.start = source.start.min(first.start);
                }
                if range.end == span.range.end {
                    source.end = source.end.max(last.end);
                }
                inner.push((first, last, left, right));
            }
            inner.sort_by_key(|(first, _, _, _)| first.start);
            if source.start > source.end {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            if range.start == scope.start || source.start == opening.end {
                patches.push(SourcePatch::primary(opening, Vec::new()));
            } else {
                let active = inner
                    .iter()
                    .filter(|(first, last, _, _)| {
                        first.end <= source.start && source.start < last.start
                    })
                    .collect::<Vec<_>>();
                let mut boundary = Vec::new();
                for (_, _, _, right) in active.iter().rev() {
                    boundary.extend_from_slice(right);
                }
                boundary.extend_from_slice(&close);
                for (_, _, left, _) in &active {
                    boundary.extend_from_slice(left);
                }
                patches.push(SourcePatch::primary(source.start..source.start, boundary));
            }
            if range.end == scope.end || source.end == closing.start {
                patches.push(SourcePatch::primary(closing, Vec::new()));
            } else {
                let active = inner
                    .iter()
                    .filter(|(first, last, _, _)| {
                        first.end < source.end && source.end <= last.start
                    })
                    .collect::<Vec<_>>();
                let mut boundary = Vec::new();
                for (_, _, _, right) in active.iter().rev() {
                    boundary.extend_from_slice(right);
                }
                boundary.extend_from_slice(&open);
                for (_, _, left, _) in &active {
                    boundary.extend_from_slice(left);
                }
                patches.push(SourcePatch::primary(source.end..source.end, boundary));
            }
        }
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(patches)?
        } else {
            self.prepare_text_edits_with_patch_policy(Vec::new(), Some(patches), true)?
        };
        if !self.format().is_source_view() {
            if let PreparedPublication::State(candidate) = &prepared.publication {
                let spans = candidate.projection.style_spans_for_region(&range);
                let mut boundaries = BTreeSet::from([range.start, range.end]);
                for span in &spans {
                    boundaries.insert(span.range.start.max(range.start));
                    boundaries.insert(span.range.end.min(range.end));
                }
                let before = self.projection().style_spans_for_region(&range);
                for span in before {
                    boundaries.insert(span.range.start.max(range.start));
                    boundaries.insert(span.range.end.min(range.end));
                }
                let boundaries: Vec<_> = boundaries.into_iter().collect();
                for pair in boundaries.windows(2).filter(|pair| pair[0] < pair[1]) {
                    let old = crate::layout::DocumentLayoutStyles::semantic_character_at(
                        self.projection(),
                        pair[0],
                        false,
                    )
                    .map_err(|_| DocumentError::VerificationFailed)?;
                    let mut new = crate::layout::DocumentLayoutStyles::semantic_character_at(
                        &candidate.projection,
                        pair[0],
                        false,
                    )
                    .map_err(|_| DocumentError::VerificationFailed)?;
                    match property {
                        StyleProperty::CharacterUnderline => new.underline = old.underline,
                        StyleProperty::CharacterSuperscript => new.superscript = old.superscript,
                        StyleProperty::CharacterSubscript => new.subscript = old.subscript,
                        _ => {}
                    }
                    if old != new {
                        return Err(DocumentError::VerificationFailed.into());
                    }
                }
                if enabled
                    && boundaries.windows(2).any(|pair| {
                        pair[0] < pair[1]
                            && spans.iter().any(|span| {
                                span.range.contains(&pair[0])
                                    && matches!(&span.application,
                        StyleApplication::Direct(p) if inline_boolean(p, property) == Some(true))
                            }) != enabled
                    })
                {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        Ok(prepared)
    }
}
