//! One structural translation path for keyboard edits, selections, and payloads.
//! Adapters own source syntax; callers retain the original logical edit/map.
use super::*;

impl Document {
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
        let mut scratch = structural_style::scratch_document(self);
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
        if !matches!(self.format(), Format::Html | Format::Rtf | Format::Markdown)
            || edits.len() < 2
            || !edits.iter().any(|edit| {
                !super::super::edit_boundary::merged_paragraphs(self, &edit.range).is_empty()
            })
        {
            return Ok(None);
        }
        let mut scratch = structural_style::scratch_document(self);
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
                None if matches!(scratch.format(), Format::Html | Format::Rtf) => Some(
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
        let mut scratch = structural_style::scratch_document(self);
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
        if !matches!(self.format(), Format::Html | Format::Rtf) {
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
