//! One structural translation path for keyboard edits, selections, and payloads.
//! Adapters own source syntax; callers retain the original logical edit/map.
use super::*;

impl Document {
    /// Text and payload edits share adapter dispatch, mapping, escaping and
    /// encoding. A payload supplies explicit hard-break identity and affinity;
    /// ordinary text treats every inserted LF as a requested logical break.
    pub(super) fn translate_source_edits<'a>(
        &self,
        edits: impl IntoIterator<Item = (&'a TextEdit, Option<&'a FormattedPayloadEdit>)>,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let mut patches = Vec::new();
        let mut rich_edits = Vec::new();
        let mut logical_edits = Vec::new();
        for (edit, payload) in edits {
            logical_edits.push(edit);
            if let Some(translated) = self.translate_source_edit(edit, payload)? {
                patches.extend(translated);
            } else {
                rich_edits.push((
                    edit.clone(),
                    payload.and_then(|edit| edit.boundary_affinity),
                ));
            }
        }
        patches.extend(super::super::source_edit::rich_text_batch_patches(
            self,
            &rich_edits,
        )?);
        super::super::html_paragraph::preserve_deleted_content_boundaries(self, &logical_edits, &mut patches)?;
        Ok(patches)
    }

    /// `None` defers an ordinary HTML/RTF edit to contributor-aware batch
    /// translation, where edits sharing one source entity are combined once.
    fn translate_source_edit(
        &self,
        edit: &TextEdit,
        payload: Option<&FormattedPayloadEdit>,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if let Some(patches) = self.structural_text_patches(edit)? {
            return Ok(Some(patches));
        }
        if let Some(patches) =
            markdown_list_structure::empty_insertion_patches(self, &edit.range, &edit.replacement)?
        {
            return Ok(Some(patches));
        }
        if let Some(payload) = payload {
            if let Some(patches) = markdown_list_structure::insertion_patches(self, payload)? {
                return Ok(Some(patches));
            }
        }
        if let Some(patches) =
            super::super::markdown_code::patches(self, &edit.range, &edit.replacement)?
        {
            return patches
                .into_iter()
                .map(|(range, syntax)| {
                    self.encoding()
                        .encode_fragment(&syntax)
                        .map(|bytes| SourcePatch::primary(range, bytes))
                        .map_err(ModelTransactionError::from)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some);
        }
        let breaks = match payload {
            Some(payload) => std::borrow::Cow::Borrowed(payload.payload.break_offsets()),
            None if self.format() == Format::Markdown && edit.replacement.contains('\n') => {
                std::borrow::Cow::Owned(
                    edit.replacement
                        .match_indices('\n')
                        .map(|(at, _)| at)
                        .collect::<Vec<_>>(),
                )
            }
            None => std::borrow::Cow::Borrowed(&[][..]),
        };
        if let Some(patches) =
            self.markdown_retained_break_rewrite_patches(&edit.range, &edit.replacement, &breaks)?
        {
            return Ok(Some(patches));
        }
        if payload.is_none() || breaks.len() == edit.replacement.matches('\n').count() {
            if let Some(patches) = markdown_split::patches(self, &edit.range, &edit.replacement)? {
                return Ok(Some(patches));
            }
        }
        if self.format().is_rich_text() {
            return Ok(None);
        }
        let affinity = payload.and_then(|edit| edit.boundary_affinity);
        let source_range = if edit.range.is_empty() {
            let at = if self.format() == Format::Markdown && !edit.replacement.contains('\n') {
                // A formatted caret belongs to its visible line even when
                // its upstream side precedes hidden list or fence syntax.
                // Structural breaks retain the adapter's split-boundary rule.
                super::super::source_edit::insertion_point(
                    self.projection(),
                    edit.range.start,
                    affinity,
                )
            } else {
                self.projection().source_insertion_point(
                    edit.range.start,
                    affinity != Some(BoundaryAffinity::Upstream),
                )
            }
            .ok_or(DocumentError::AmbiguousProjection)?;
            at..at
        } else {
            self.projection()
                .source_range(edit.range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        self.reject_unsafe_opaque_mapping(&edit.range, &source_range)?;
        let line_local = match payload {
            Some(_) => {
                breaks.is_empty()
                    && !(edit.range.is_empty() && affinity == Some(BoundaryAffinity::Upstream))
            }
            None => !edit.replacement.contains('\n'),
        };
        if self.format() == Format::Markdown && line_local {
            if let Some(patches) =
                self.markdown_line_local_text_rewrite_patches(&edit.range, &edit.replacement)?
            {
                return Ok(Some(patches));
            }
        }
        let in_code = self.format() == Format::Markdown
            && self
                .projection()
                .markdown_replacement_begins_in_code(&edit.range);
        let syntax = if let Some(payload) = payload {
            if self.format() == Format::Markdown && !in_code && breaks.is_empty() {
                self.escape_markdown_source_text(source_range.start, &edit.replacement)?
            } else {
                structured_payload_syntax(
                    &payload.payload,
                    self.format(),
                    in_code,
                    self.file_format(),
                    self.encoding(),
                )
            }
        } else {
            let syntax = match self.format() {
                Format::MarkdownSource => self.markdown_source_replacement(edit)?,
                Format::PlainText | Format::Code | Format::HtmlSource => edit.replacement.clone(),
                Format::Markdown if in_code && !edit.replacement.contains('`') => {
                    edit.replacement.clone()
                }
                Format::Markdown => self
                    .escape_markdown_source_text(source_range.start, &edit.replacement)?
                    .replace('\n', "\n\n"),
                Format::Html | Format::Rtf => unreachable!("rich edits are translated as a batch"),
            };
            spell_logical_breaks(&syntax, self.file_format())
        };
        Ok(Some(vec![SourcePatch::primary(
            source_range,
            self.encoding().encode_fragment(&syntax)?,
        )]))
    }

    pub(super) fn preserve_markdown_edit_boundaries<'a>(
        &self,
        edits: &[TextEdit],
        paragraph_break_ranges: impl Iterator<Item = &'a Range<usize>>,
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        if self.format() == Format::Markdown {
            markdown_block_styles::preserve_deleted_source_prefixes(self, edits, patches)?;
            super::super::markdown_code::preserve_edited_inline_delimiters(self, edits, patches)?;
            markdown_split::remove_empty_emphasis(self, edits, patches)?;
            markdown_block_styles::remove_empty_continuation_prefixes(self, edits, patches)?;
            markdown_block_styles::preserve_deleted_boundary_spaces(self, edits, patches)?;
            markdown_block_styles::preserve_deleted_source_prefixes(self, edits, patches)?;
            markdown_block_styles::preserve_join_boundaries(self, edits, patches)?;
            markdown_block_styles::preserve_split_literals(self, edits, patches)?;
            markdown_block_styles::preserve_split_boundaries(
                self,
                paragraph_break_ranges,
                patches,
            )?;
            markdown_block_styles::preserve_retained_literals(self, edits, patches)?;
        }
        Ok(())
    }

    pub(super) fn prepare_recovered_source_edit(
        &self,
        edits: &[TextEdit],
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        self.prepare_with_recovered_source(edits, |scratch| {
            scratch.prepare_text_edits(edits.to_vec())
        })
    }

    pub(super) fn prepare_with_recovered_source(
        &self,
        edits: &[TextEdit],
        prepare: impl FnOnce(&Document) -> Result<PreparedModelTransaction, ModelTransactionError>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.format() != Format::Html {
            return Ok(None);
        }
        let patches = super::super::html_merge::recovered_source_patches(self, edits)?;
        if patches.is_empty() {
            return Ok(None);
        }
        let mut scratch = self.scratch_document();
        let materialized = scratch.prepare_source_only_patches(patches)?;
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        for patch in materialized.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(materialized)?;
        if !self
            .projection()
            .has_same_hard_line_structure(scratch.projection())
            || !same_paragraph_assignments(self.projection(), scratch.projection())
            || !super::super::rich_text::character_edit_verified(
                self.projection(),
                scratch.projection(),
                &(0..0),
                &CharacterProperties::default(),
            )
            || !super::super::html_merge::recovered_source_patches(&scratch, edits)?.is_empty()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        let edited = prepare(&scratch)?;
        if edited.is_no_op() {
            return Ok(Some(self.no_op_prepared()));
        }
        for patch in edited.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(edited)?;
        let prepared = self.prepare_text_edits_with_patches(
            edits.to_vec(),
            Some(sources.source_patches(&scratch.state().source)?),
        )?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        if !candidate
            .projection
            .has_same_hard_line_structure(scratch.projection())
            || !same_paragraph_assignments(&candidate.projection, scratch.projection())
            || !super::super::rich_text::character_edit_verified(
                scratch.projection(),
                &candidate.projection,
                &(0..0),
                &CharacterProperties::default(),
            )
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(Some(prepared))
    }

    /// Adjacent structural edits can share closing syntax. Compose them on an
    /// immutable scratch document, preparing whitespace before joins, then publish using
    /// the original logical edits. No intermediate state reaches history/views.
    pub(super) fn prepare_structural_text_batch(
        &self,
        edits: &[TextEdit],
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if !self.format().is_wysiwyg()
            || edits.len() < 2
            || !edits.iter().any(|edit| {
                !super::super::edit_boundary::merged_paragraphs(self, &edit.range).is_empty()
            })
        {
            return Ok(None);
        }
        let mut scratch = self.scratch_document();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        let is_structural = |edit: &TextEdit| {
            !super::super::edit_boundary::merged_paragraphs(self, &edit.range).is_empty()
        };
        let ordered = edits
            .iter()
            .rev()
            .filter(|edit| !is_structural(edit))
            .chain(edits.iter().rev().filter(|edit| is_structural(edit)));
        let mut map = PositionMap::identity(
            self.id(),
            super::super::PositionDomain::FormattedText,
            self.revision(),
            self.projection().text_tree().byte_len(),
        );
        for original in ordered {
            let rebase = |at, association| -> Result<usize, ModelTransactionError> {
                Ok(map
                    .map_text_point(
                        self.text_point(at)?,
                        association,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )?
                    .value()
                    .ok_or(DocumentError::AmbiguousProjection)?
                    .offset())
            };
            let mut edit = original.clone();
            edit.range = if original.range.is_empty() {
                // A point has one association. Empty edits sort before a
                // replacement at the same start, even when prepared later.
                let at = rebase(original.range.start, Association::BeforeInsertion)?;
                at..at
            } else {
                rebase(original.range.start, Association::AfterInsertion)?
                    ..rebase(original.range.end, Association::BeforeInsertion)?
            };
            // Whitespace support is applied before removing structural scopes,
            // so no intermediate paragraph loses an exposed boundary space.
            // These edits are already normalized against the complete batch.
            let patches = match scratch.structural_text_patches(&edit)? {
                Some(patches) => Some(patches),
                None if scratch.format().is_rich_text() => Some(
                    super::super::source_edit::rich_text_patches(&scratch, &edit, None)?,
                ),
                None => None,
            };
            let prepared = scratch.prepare_text_edits_with_patches(vec![edit], patches)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            map = map.then(prepared.text_position_map())?;
            scratch.commit_model_transaction(prepared)?;
        }
        self.prepare_text_edits_with_patches(
            edits.to_vec(),
            Some(sources.source_patches(&scratch.state().source)?),
        )
        .map(Some)
    }

    /// Rich fragments can supply their own inline syntax after the selection's
    /// structural boundaries have been removed by the same deletion policy.
    pub(super) fn prepare_structural_replacement(
        &self,
        range: Range<usize>,
        text: &str,
        insert: impl FnOnce(&Document, usize) -> Result<PreparedModelTransaction, ModelTransactionError>,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if super::super::edit_boundary::merged_paragraphs(self, &range).is_empty() {
            return Ok(None);
        }
        let mut scratch = self.scratch_document();
        let deleted = scratch.prepare_text_edits(vec![TextEdit::new(range.clone(), "")])?;
        let at = deleted
            .text_position_map()
            .map_text_point(
                scratch.text_point(range.start)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        for patch in deleted.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(deleted)?;
        let inserted = insert(&scratch, at)?;
        for patch in inserted.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(inserted)?;
        self.prepare_text_edits_with_patches(
            vec![TextEdit::new(range, text)],
            Some(sources.source_patches(&scratch.state().source)?),
        )
        .map(Some)
    }

    pub(super) fn structural_text_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
        if edit.range.is_empty() {
            return if self.format() == Format::Html && edit.replacement.contains('\n') {
                super::super::html_merge::anonymous_break_patches(self, edit)
            } else {
                Ok(None)
            };
        }
        if self.format() == Format::Markdown {
            if let Some(patches) =
                markdown_list_structure::joining_patches(self, &edit.range, &edit.replacement)?
            {
                return Ok(Some(patches));
            }
            return if edit.replacement.is_empty() {
                markdown_list_structure::deletion_patches(self, &edit.range, false)
            } else {
                Ok(None)
            };
        }
        if !self.format().is_rich_text() {
            return Ok(None);
        }
        if super::super::edit_boundary::merged_paragraphs(self, &edit.range).is_empty() {
            return Ok(None);
        }
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        if self.format() == Format::Rtf {
            if let Some(patches) = super::super::rtf_structure::joining_patches(self, &input, edit)?
            {
                return Ok(Some(patches));
            }
        }
        let patches = if self.format() == Format::Html {
            if let Some(patches) = super::super::html_merge::patches(self, &input, edit)? {
                Some(patches)
            } else if edit.replacement.is_empty() {
                super::super::html_paragraph::deletion_patches(self, &input, &edit.range, false)?
            } else {
                None
            }
        } else if edit.replacement.is_empty() {
            super::super::rtf_structure::deletion_patches(self, &input, &edit.range, false)?
        } else {
            None
        };
        patches
            .map(|patches| {
                patches
                    .into_iter()
                    .map(|(range, syntax)| {
                        self.encoding()
                            .encode_fragment(&syntax)
                            .map(|bytes| SourcePatch::primary(range, bytes))
                    })
                    .collect()
            })
            .transpose()
    }
}

fn same_paragraph_assignments(before: &FormattedDocument, after: &FormattedDocument) -> bool {
    before.blocks().len() == after.blocks().len()
        && before
            .blocks()
            .iter()
            .zip(after.blocks())
            .all(|(left, right)| {
                left.range == right.range
                    && left.kind == right.kind
                    && left.style == right.style
                    && left.direct_paragraph == right.direct_paragraph
                    && (!left.range.is_empty()
                        || left.direct_default_character == right.direct_default_character)
            })
}
