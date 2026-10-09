//! Atomic insertion with view-local character overrides. The speculative
//! stages never publish history. Their local patch lists are composed against
//! the original source, then verified and committed as one model transaction.
use super::*;
use crate::document::{FontSlant, LinkEditIntent, ReplacementTypingContext, StylePropertyValue};

use super::replacement::PatchComposition;

impl Document {
    pub(crate) fn prepare_typing_after_canonical_link(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let (canonical, range) = scratch.prepare_canonical_autolink(
            edit.range.clone(),
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream),
        )?;
        for patch in canonical.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(canonical)?;
        let payload = FormattedTextPayload::new(
            &scratch.hard_line_snapshot(),
            edit.payload.text(),
            edit.payload.break_offsets().to_vec(),
        )
        .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let staged = FormattedPayloadEdit::new(range, payload).with_boundary_affinity(
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream),
        );
        let (inserted, caret, start) =
            scratch.prepare_insertion_without_link(staged, named, values, None)?;
        for patch in inserted.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(inserted)?;
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(sources.source_patches())?
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(sources.source_patches()),
                true,
            )?
        };
        self.prepared_text_point(&prepared, caret)?;
        self.prepared_text_point(&prepared, start)?;
        Ok((prepared, caret, start))
    }

    pub(crate) fn prepare_autolink_selection_removal(
        &self,
        range: std::ops::Range<usize>,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let (canonical, selected) =
            scratch.prepare_canonical_autolink(range.clone(), BoundaryAffinity::Downstream)?;
        for patch in canonical.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(canonical)?;
        let (removed, caret) = scratch.prepare_link_edit(
            scratch.id(),
            scratch.revision(),
            LinkEditIntent::RemoveSelection { range: selected },
        )?;
        for patch in removed.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(removed)?;
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(sources.source_patches())?
        } else {
            self.prepare_text_edits_with_patch_policy(
                Vec::new(),
                Some(sources.source_patches()),
                true,
            )?
        };
        self.prepared_text_point(&prepared, caret)?;
        Ok((prepared, caret))
    }
    pub(crate) fn prepare_link_after_unlinked_typing(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        destination: String,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let payload = FormattedTextPayload::new(
            &scratch.hard_line_snapshot(),
            edit.payload.text(),
            edit.payload.break_offsets().to_vec(),
        )
        .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let staged = FormattedPayloadEdit::new(edit.range.clone(), payload).with_boundary_affinity(
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream),
        );
        let (first, caret, start) =
            scratch.prepare_insertion_without_link(staged, named, values, None)?;
        for patch in first.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(first)?;
        let (linked, caret) = scratch.prepare_link_edit(
            scratch.id(),
            scratch.revision(),
            super::super::LinkEditIntent::Insert {
                range: start..caret,
                text: edit.payload.text().to_owned(),
                destination,
            },
        )?;
        for patch in linked.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(linked)?;
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(sources.source_patches())?
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(sources.source_patches()),
                true,
            )?
        };
        self.prepared_text_point(&prepared, caret)?;
        Ok((prepared, caret))
    }
    /// Compose a source-local link split with ordinary styled payload editing.
    /// The provisional character retains the original inline context and supplies
    /// an editable boundary; none of the stages
    /// stage is published, and the final transaction verifies the real payload.
    pub(crate) fn prepare_typing_after_link_split(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let placeholder = FormattedTextPayload::new(&scratch.hard_line_snapshot(), "x", Vec::new())
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let placeholder = FormattedPayloadEdit::new(edit.range.clone(), placeholder)
            .with_boundary_affinity(
                edit.boundary_affinity
                    .unwrap_or(BoundaryAffinity::Downstream),
            );
        let (first, _, staged_at) =
            scratch.prepare_insertion_with_typing_context(placeholder, None, &[], None)?;
        for patch in first.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(first)?;
        let (unlinked, staged_at) =
            scratch.prepare_unlinked_typing_boundary(staged_at..staged_at + 1)?;
        for patch in unlinked.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(unlinked)?;
        let payload = FormattedTextPayload::new(
            &scratch.hard_line_snapshot(),
            edit.payload.text(),
            edit.payload.break_offsets().to_vec(),
        )
        .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let staged = FormattedPayloadEdit::new(staged_at..staged_at, payload)
            .with_boundary_affinity(BoundaryAffinity::Downstream);
        let (next, mut caret, mut authored_start) =
            scratch.prepare_insertion_with_typing_context(staged, named, values, None)?;
        let placeholder = next
            .text_position_map()
            .map_text_point(
                scratch.text_point(staged_at)?,
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        for patch in next.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(next)?;
        let remove = scratch.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: scratch.id(),
            revision: scratch.revision(),
            edits: vec![TextEdit::new(placeholder..placeholder + 1, "")],
        })?;
        let map = remove.text_position_map();
        caret = map
            .map_text_point(
                scratch.text_point(caret)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Upstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        authored_start = map
            .map_text_point(
                scratch.text_point(authored_start)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        for patch in remove.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(remove)?;
        scratch.protect_unlinked_typing(&mut sources, &mut authored_start, &mut caret)?;
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(sources.source_patches())?
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(sources.source_patches()),
                true,
            )?
        };
        self.prepared_text_point(&prepared, caret)?;
        self.prepared_text_point(&prepared, authored_start)?;
        Ok((prepared, caret, authored_start))
    }

    pub(crate) fn prepare_typing_without_automatic_links(
        &self,
        edit: FormattedPayloadEdit,
        named: Option<&StyleId>,
        values: &[(StyleProperty, StylePropertyValue)],
        inherited: Option<&ReplacementTypingContext>,
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let payload = FormattedTextPayload::new(
            &scratch.hard_line_snapshot(),
            edit.payload.text(),
            edit.payload.break_offsets().to_vec(),
        )
        .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let staged = FormattedPayloadEdit::new(edit.range.clone(), payload).with_boundary_affinity(
            edit.boundary_affinity
                .unwrap_or(BoundaryAffinity::Downstream),
        );
        let mut inherited = inherited.cloned();
        if let Some(context) = inherited.as_mut() {
            context.link = None;
        }
        let (first, mut caret, mut start) = scratch.prepare_insertion_with_typing_context(
            staged,
            named,
            values,
            inherited.as_ref(),
        )?;
        let mut sources = PatchComposition::new(self.source_byte_len());
        for patch in first.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(first)?;
        scratch.protect_unlinked_typing(&mut sources, &mut start, &mut caret)?;
        let prepared = if self.format().is_source_view() {
            self.prepare_visible_source_patches(sources.source_patches())?
        } else {
            self.prepare_text_edits_with_patch_policy(
                vec![edit.text_edit()],
                Some(sources.source_patches()),
                true,
            )?
        };
        self.prepared_text_point(&prepared, caret)?;
        self.prepared_text_point(&prepared, start)?;
        Ok((prepared, caret, start))
    }

    /// A pending plain-link choice also protects URL/email payloads from GFM's
    /// automatic linking. Reuse ordinary link removal's verified escaping rule.
    fn protect_unlinked_typing(
        &mut self,
        sources: &mut PatchComposition,
        start: &mut usize,
        caret: &mut usize,
    ) -> Result<(), ModelTransactionError> {
        for _ in 0..512 {
            let item = self
                .projection()
                .style_spans_for_region(&(*start..*caret))
                .iter()
                .find(|span| {
                    span.application == StyleApplication::Automatic("Link".into())
                        && span.range.start < *caret
                        && *start < span.range.end
                })
                .map(|span| span.range.start);
            let Some(at) = item else {
                return Ok(());
            };
            let link = self
                .link_snapshot_at(self.text_point(at)?)?
                .filter(|link| link.editable && link.destination_source.is_none())
                .ok_or(DocumentError::UnsupportedFormatting)?;
            let (prepared, _) = self.prepare_link_edit(
                self.id(),
                self.revision(),
                LinkEditIntent::Remove { range: link.range },
            )?;
            let map = prepared.text_position_map();
            *start = map
                .map_text_point(
                    self.text_point(*start)?,
                    Association::BeforeInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            *caret = map
                .map_text_point(
                    self.text_point(*caret)?,
                    Association::AfterInsertion,
                    BoundaryAffinity::Upstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset();
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            self.commit_model_transaction(prepared)?;
        }
        Err(DocumentError::UnsupportedFormatting.into())
    }
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
            || !self.format().is_markdown()
            || !(style.0 == "Code" || style.0.is_empty())
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        Ok(())
    }

    pub fn validate_typing_named_style_at(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
        style: &StyleId,
    ) -> Result<(), DocumentError> {
        if self.is_read_only() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        self.validate_typing_named_style(style)?;
        self.validate_typing_properties_at(at, affinity, &[])
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
        if !self.format().is_markdown() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut properties = CharacterProperties::default();
        for (property, value) in values {
            match (property, value) {
                (StyleProperty::CharacterBold, StylePropertyValue::Boolean(enabled)) => {
                    properties.bold = Some(*enabled);
                }
                (StyleProperty::CharacterSlant, StylePropertyValue::FontSlant(slant)) => {
                    properties.slant = Some(*slant);
                }
                (StyleProperty::CharacterUnderline, StylePropertyValue::Boolean(enabled)) => {
                    properties.underline = Some(*enabled)
                }
                (StyleProperty::CharacterSuperscript, StylePropertyValue::Boolean(enabled)) => {
                    properties.superscript = Some(*enabled);
                    if *enabled {
                        properties.subscript = Some(false);
                    }
                }
                (StyleProperty::CharacterSubscript, StylePropertyValue::Boolean(enabled)) => {
                    properties.subscript = Some(*enabled);
                    if *enabled {
                        properties.superscript = Some(false);
                    }
                }
                (StyleProperty::CharacterStrikethrough, StylePropertyValue::Boolean(enabled)) => {
                    properties.strikethrough = Some(*enabled);
                }
                _ => return Err(DocumentError::UnsupportedFormatting),
            }
        }
        Ok(properties)
    }

    /// Native character formatting is unavailable in literal block owners.
    /// This local capability query never prepares or publishes an edit.
    pub fn validate_typing_properties_at(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
        values: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<(), DocumentError> {
        self.text_point(at)?;
        self.validate_typing_properties(values)?;
        if self.is_read_only() {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let sample = if affinity == BoundaryAffinity::Upstream && at > 0 {
            self.hard_line_snapshot()
                .previous_grapheme_boundary(at)
                .unwrap_or(at)
        } else {
            at
        };
        if self
            .projection()
            .blocks_for_region(&(sample..sample))
            .iter()
            .any(|block| {
                block.range.start <= sample
                    && sample <= block.range.end
                    && (block.style.0 == "Code Block"
                        || block.containers.iter().any(|member| {
                            member.container.kind == super::super::ContainerKind::CodeBlock
                        }))
            })
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        Ok(())
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
        let spans = self
            .projection()
            .style_spans_for_region(&(sample..sample + 1));
        let has = |application| {
            spans
                .iter()
                .any(|span| span.range.contains(&sample) && span.application == application)
        };
        p.bold.is_none_or(|enabled| {
            enabled == has(StyleApplication::Semantic(SemanticInlineStyle::Strong))
        }) && p.slant.is_none_or(|slant| {
            (slant != FontSlant::Upright)
                == has(StyleApplication::Semantic(SemanticInlineStyle::Emphasis))
        }) && [
            (StyleProperty::CharacterUnderline, p.underline),
            (StyleProperty::CharacterSuperscript, p.superscript),
            (StyleProperty::CharacterSubscript, p.subscript),
        ]
        .iter()
        .all(|(property, desired)| {
            desired.is_none_or(|enabled| {
                enabled
                    == spans.iter().any(|span| {
                        span.range.contains(&sample)
                            && matches!(&span.application, StyleApplication::Direct(value)
                    if super::inline_properties::inline_boolean(value, *property) == Some(true))
                    })
            })
        }) && p.strikethrough.is_none_or(|enabled| {
            enabled == has(StyleApplication::Automatic("Strikethrough".into()))
        })
    }

    pub(super) fn prepare_typing_markdown_style(
        &self,
        range: Range<usize>,
        style: SemanticInlineStyle,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.prepare_typing_markdown_application(range, Some(style), enabled)
    }

    pub(super) fn prepare_typing_markdown_application(
        &self,
        range: Range<usize>,
        style: Option<SemanticInlineStyle>,
        enabled: bool,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let application = style.map_or_else(
            || StyleApplication::Automatic("Strikethrough".into()),
            StyleApplication::Semantic,
        );
        let markers = style.map_or(&["~~", "~"][..], markdown_style_markers);
        let removal = |content: &Range<usize>| match style {
            Some(style) => self.markdown_style_removal_patches(content, style),
            None => self.markdown_strike_removal_patches(content),
        };
        let containing = self
            .projection()
            .style_spans_for_region(&range)
            .into_iter()
            .find(|span| {
                span.application == application
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
        let (html_open, html_close) = if style == Some(SemanticInlineStyle::Strong) {
            ("<strong>", "</strong>")
        } else if style.is_some() {
            ("<em>", "</em>")
        } else {
            ("<del>", "</del>")
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
                .find(|span| span.application == application && span.range.end == range.start);
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
                    let marker = markers
                        .iter()
                        .map(|m| self.encoding().encode_fragment(m))
                        .collect::<Result<Vec<_>, _>>()?
                        .into_iter()
                        .find(|m| bytes.starts_with(m) && bytes.ends_with(m))
                        .ok_or(DocumentError::UnsupportedFormatting)?;
                    content.end - marker.len()..content.end
                } else {
                    removal(&content)?[1].range()
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
            let marker = if style == Some(SemanticInlineStyle::Strong) {
                "**"
            } else if style.is_some() {
                "*"
            } else {
                "~~"
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
                let marker = markers
                    .iter()
                    .map(|m| self.encoding().encode_fragment(m))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .find(|m| bytes.starts_with(m) && bytes.ends_with(m))
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                let paired = style.is_some()
                    && self
                        .projection()
                        .style_spans_for_region(&span.range)
                        .iter()
                        .any(|other| {
                            other.range == span.range
                                && other.application
                                    == StyleApplication::Semantic(
                                        if style == Some(SemanticInlineStyle::Strong) {
                                            SemanticInlineStyle::Emphasis
                                        } else {
                                            SemanticInlineStyle::Strong
                                        },
                                    )
                        });
                let padding = if paired {
                    marker.len()
                        / if style == Some(SemanticInlineStyle::Strong) {
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
                let removal = removal(&content)?;
                let opening = removal[0].range();
                let closing = removal[1].range();
                let marker = self
                    .state()
                    .source
                    .bytes_in(opening.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                (content, opening, closing, marker)
            };
            source.start = source.start.max(content.start);
            source.end = source.end.min(content.end);
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
                    (P::CharacterUnderline, V::Boolean(c.underline)),
                    (P::CharacterStrikethrough, V::Boolean(c.strikethrough)),
                    (P::CharacterSuperscript, V::Boolean(c.superscript)),
                    (P::CharacterSubscript, V::Boolean(c.subscript)),
                ];

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
        if named.is_none() && inherited.is_none() && edit.payload.text().trim().is_empty() {
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
            && inherited.is_none()
        {
            // Replacing one already-matching grapheme retains its existing
            // source syntax, so no redundant wrapper edit is needed.
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
        for (style, enabled) in [
            (
                Some(SemanticInlineStyle::Emphasis),
                properties.slant.map(|s| s != FontSlant::Upright),
            ),
            (Some(SemanticInlineStyle::Strong), properties.bold),
            (None, properties.strikethrough),
        ] {
            if let Some(enabled) = enabled {
                let prepared = scratch.prepare_typing_markdown_application(
                    selection.clone(),
                    style,
                    enabled,
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
        for (property, enabled) in [
            (StyleProperty::CharacterUnderline, properties.underline),
            (StyleProperty::CharacterSuperscript, properties.superscript),
            (StyleProperty::CharacterSubscript, properties.subscript),
        ] {
            if let Some(enabled) = enabled {
                let prepared =
                    scratch.prepare_inline_property(selection.clone(), property, enabled)?;
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
        let patches = sources.source_patches();
        let edits = formatted.formatted_edits();
        let prepared = self.prepare_text_edits_with_patches(edits, Some(patches))?;
        Ok((prepared, caret, selection.start))
    }
}
