//! One structural translation path for keyboard edits, selections, and payloads.
//! Adapters own source syntax; callers retain the original logical edit/map.
use super::*;

impl Document {
    /// Complete displayed rows own their block syntax. A prefix of a code
    /// paragraph owns only those body rows, so surviving code keeps its fence.
    pub(super) fn markdown_complete_row_deletion_patches(
        &self,
        range: &Range<usize>,
    ) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
        if range.is_empty() { return Ok(None); }
        let blocks = self.projection().blocks_for_region(range);
        let selected = blocks.iter().filter(|block|
            block.range.start < range.end && range.start < block.range.end).collect::<Vec<_>>();
        if selected.is_empty() || selected.iter().any(|block|
            matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || self.projection().table_at(block.range.start).is_some()) {
            return Ok(None);
        }
        let snapshot = self.hard_line_snapshot();
        let mut patches = Vec::new();
        for block in selected {
            let body = range.start.max(block.range.start)..range.end.min(block.range.end);
            let complete = body == block.range;
            if !complete && block.style.0 != "Code Block" { return Ok(None); }
            let first = snapshot.line_at_offset(body.start).map_err(|_| DocumentError::AmbiguousProjection)?;
            let last_at = snapshot.previous_grapheme_boundary(body.end).unwrap_or(body.start).max(body.start);
            let last = snapshot.line_at_offset(last_at).map_err(|_| DocumentError::AmbiguousProjection)?;
            if first.content_range().start != body.start
                || last.content_range().end != body.end && last.linewise_range().end != body.end {
                return Ok(None);
            }
            let source = self.projection().source_range(body.clone()).ok_or(DocumentError::AmbiguousProjection)?;
            let first_source = self.physical_line_at_source(source.start)?;
            let last_source = self.physical_line_at_source(source.end.saturating_sub(1).max(source.start))?;
            let mut start = first_source.content_range.start;
            let mut end = last_source.content_range.end;
            if complete && block.style.0 == "Code Block" {
                if let Some(fence) = super::super::markdown_code::fenced_source(self, block)? {
                    start = fence.opening.start;
                    end = fence.closing_content_end.unwrap_or(end);
                }
            }
            if body.end < block.range.end {
                let next_at = if last.linewise_range().end == body.end { body.end } else { body.end + 1 };
                let next = self.projection().source_insertion_point(next_at, true)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                end = self.physical_line_at_source(next)?.content_range.start;
            } else if !complete && block.style.0 == "Code Block" {
                // The last selected body row does not own the enclosing
                // closing fence or the following paragraph's separator.
                end = last_source.source_range.end;
            } else if range.end > block.range.end {
                let next_at = block.range.end + 1;
                if let Some(next) = self.projection().blocks_for_region(&(next_at..next_at)).into_iter()
                    .find(|next| next.range.start == next_at) {
                    let at = self.projection().source_insertion_point(next.range.start, true)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    end = self.physical_line_at_source(at)?.content_range.start;
                    if let Some(fence) = super::super::markdown_code::fenced_source(self, &next)? {
                        end = fence.opening.start;
                    }
                }
            }
            if range.start < block.range.start && range.start + 1 == block.range.start {
                if let Some(previous) = self.projection().blocks_for_region(&(range.start..range.start)).into_iter()
                    .find(|previous| previous.range.end == range.start) {
                    let source = self.projection().source_range(previous.range.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    start = self.physical_line_at_source(source.end.saturating_sub(1).max(source.start))?.content_range.end;
                    if let Some(fence) = super::super::markdown_code::fenced_source(self, &previous)? {
                        start = fence.closing_content_end.unwrap_or(start);
                    }
                }
            }
            patches.push(SourcePatch::primary(start..end, Vec::new()));
        }
        Ok(Some(patches))
    }

    /// Text and payload edits share adapter dispatch, mapping, escaping and
    /// encoding. A payload supplies explicit hard-break identity and affinity;
    /// ordinary text treats every inserted LF as a requested logical break.
    pub(in crate::document) fn translate_source_edits<'a>(
        &self,
        edits: impl IntoIterator<Item = (&'a TextEdit, Option<&'a FormattedPayloadEdit>)>,
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        let mut patches = Vec::new();
        let mut rich_edits = Vec::new();
        for (edit, payload) in edits {
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
        Ok(patches)
    }

    /// `None` defers an ordinary rich edit to contributor-aware batch
    /// translation, where edits sharing one source entity are combined once.
    fn translate_source_edit(
        &self,
        edit: &TextEdit,
        payload: Option<&FormattedPayloadEdit>,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        let affinity = payload.and_then(|edit| edit.boundary_affinity);
        if self.format() == Format::Markdown && edit.range.is_empty()
            && edit.replacement.ends_with('\n')
            && payload.is_none_or(|payload| payload.payload.break_offsets().iter().copied()
                .eq(edit.replacement.match_indices('\n').map(|(at, _)| at)))
        {
            if let Some(block) = self.projection().blocks_for_region(&edit.range).into_iter()
                .find(|block| block.range.start == edit.range.start && block.quote_depth == 0
                    && block.containers.is_empty() && matches!(block.kind, super::super::BlockKind::Heading(_)))
            {
                if self.projection().style_spans_for_region(&edit.range).is_empty() {
                    let super::super::BlockKind::Heading(level) = block.kind else { unreachable!() };
                    let body = self.projection().source_insertion_point(block.range.start, true)
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let at = self.physical_line_at_source(body)?.content_range.start;
                    let mut syntax = String::new();
                    for paragraph in edit.replacement[..edit.replacement.len() - 1].split('\n') {
                        syntax.push_str(&"#".repeat(level as usize));
                        syntax.push(' ');
                        let leading = paragraph.len() - paragraph.trim_start_matches([' ', '\t']).len();
                        let trailing = paragraph.trim_end_matches([' ', '\t']).len();
                        for character in paragraph[..leading].chars() {
                            syntax.push_str(if character == ' ' { "&#32;" } else { "&#9;" });
                        }
                        if leading < trailing {
                            syntax.push_str(&escape_markdown_insert_in_encoding(&paragraph[leading..trailing], self.encoding()));
                        }
                        for character in paragraph[trailing.max(leading)..].chars() {
                            syntax.push_str(if character == ' ' { "&#32;" } else { "&#9;" });
                        }
                        syntax.push_str(&self.file_format().spelling().repeat(2));
                    }
                    // Opening before the heading keeps its complete authored
                    // syntax and gives each inserted paragraph the same role.
                    return Ok(Some(vec![SourcePatch::primary(at..at, self.encoding().encode_fragment(&syntax)?)]));
                }
            }
        }
        if edit.range.is_empty() && affinity == Some(BoundaryAffinity::Downstream)
            && !edit.replacement.contains('\n')
        {
            if let Some(closing) = super::super::markdown_code::closing_boundary(self, edit.range.start)? {
                let syntax = self.escape_markdown_source_text(closing.end, &edit.replacement)?;
                return Ok(Some(vec![SourcePatch::primary(closing.end..closing.end,
                    self.encoding().encode_fragment(&syntax)?)]));
            }
        }
        if let Some(patches) = self.table_text_patches(edit, affinity)? {return Ok(Some(patches));}
        if self.format() == Format::Markdown && !edit.range.is_empty() && edit.replacement.is_empty() {
            if let Some(link) = self.link_snapshot_at(self.text_point(edit.range.start)?)?
                .filter(|link| link.editable && link.range == edit.range) {
                return Ok(Some(vec![SourcePatch::primary(link.source, Vec::new())]));
            }
        }
        if self.format() == Format::Markdown
            && !edit.replacement.contains('\n')
            && super::super::source_edit::complete_contributors(self.projection(), edit)?.range
                != edit.range
        {
            return Ok(None);
        }
        if let Some(patches) = self.markdown_rule_text_patches(edit)? {
            return Ok(Some(patches));
        }
        if self.format() == Format::Markdown && !edit.replacement.contains('\n') {
            let nearby = edit.range.start.saturating_sub(1)..(edit.range.end + 1).min(self.projection().text_tree().byte_len());
            let literal = self.projection().style_spans_for_region(&nearby).iter().any(|span| {
                span.range.start <= edit.range.start && edit.range.end <= span.range.end
                    && matches!(&span.application, StyleApplication::Automatic(id) if matches!(id.0.as_str(), "Markdown reference" | "Comment"))
            });
            let html = self
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| {
                    block.markdown_html
                        && block.range.start <= edit.range.start
                        && edit.range.end <= block.range.end
                });
            if literal || html {
                let range = if edit.range.is_empty() {
                    let at = super::super::source_edit::insertion_point(
                        self.projection(),
                        edit.range.start,
                        None,
                    )
                    .ok_or(DocumentError::AmbiguousProjection)?;
                    at..at
                } else {
                    self.projection()
                        .source_range(edit.range.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?
                };
                let outside_comment = edit.range.is_empty()
                    && self.projection().style_spans_for_region(&nearby).iter().any(|span|
                        span.application == StyleApplication::Automatic("Comment".into())
                            && (span.range.start == edit.range.start
                                || span.range.end == edit.range.start
                                    && self.projection().text_tree().slice(span.range.clone())
                                        .is_ok_and(|text| text.ends_with("-->"))));
                let replacement = if outside_comment {
                    self.escape_markdown_source_text(range.start, &edit.replacement)?
                } else if html && !literal {
                    super::markdown_html_edit::literal_html(&edit.replacement, self.encoding())
                } else {
                    edit.replacement.clone()
                };
                return Ok(Some(vec![SourcePatch::primary(
                    range,
                    self.encoding().encode_fragment(&replacement)?,
                )]));
            }
        }
        if let Some(patches) = self.structural_text_patches(edit)? {
            return Ok(Some(patches));
        }
        if let Some(patches) =
            markdown_list_structure::empty_insertion_patches(self, &edit.range, &edit.replacement)?
        {
            return Ok(Some(patches));
        }
        if let Some(patches) = self.markdown_empty_heading_insertion_patches(edit)? {
            return Ok(Some(patches));
        }
        if let Some(payload) = payload {
            if let Some(patches) = markdown_list_structure::insertion_patches(self, payload)? {
                return Ok(Some(patches));
            }
        } else if let Some(patches) = markdown_list_structure::text_insertion_patches(self, edit)? {
            return Ok(Some(patches));
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
            if let Some(patches) = self.markdown_quote_insertion_patches(edit)? {
                return Ok(Some(patches));
            }
            if let Some(patches) = markdown_split::patches(self, &edit.range, &edit.replacement)? {
                return Ok(Some(patches));
            }
        }

        let source_range = if edit.range.is_empty() {
            let at = if self.format() == Format::Markdown && !edit.replacement.contains('\n') {
                // A formatted caret belongs to its visible line even when
                // its upstream side precedes hidden list or fence syntax.
                // Structural breaks retain the adapter's split-boundary rule.
                super::super::source_edit::document_insertion_point(
                    self,
                    edit.range.start,
                    affinity,
                )?
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
            && match affinity.filter(|_| edit.range.is_empty()) {
                Some(affinity) => self.is_code_at(edit.range.start, affinity)?,
                None => self.projection().markdown_replacement_begins_in_code(&edit.range),
            };
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
                Format::PlainText | Format::Code => edit.replacement.clone(),
                Format::Markdown if in_code && !edit.replacement.contains('`') => {
                    edit.replacement.clone()
                }
                Format::Markdown => self
                    .escape_markdown_source_text(source_range.start, &edit.replacement)?
                    .replace('\n', "\n\n"),

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
            if self.preserve_edited_reference_content(edits, patches)? {
                markdown_block_styles::preserve_deleted_boundary_spaces(self, edits, patches)?;
                markdown_block_styles::preserve_edited_paragraph_boundaries(self, edits, patches)?;
                markdown_block_styles::preserve_split_literals(self, edits, patches)?;
                markdown_block_styles::preserve_split_boundaries(self, paragraph_break_ranges, patches)?;
                markdown_block_styles::preserve_retained_literals(self, edits, patches)?;
                self.repair_markdown_reference_spaces(edits, patches)?;
                return Ok(());
            }
            if edits.iter().all(|edit|self.projection().table_at(edit.range.start).is_some_and(|table|edit.range==table.range)) {return Ok(());}
            if edits.iter().all(|edit|self.projection().table_cell_at(edit.range.start).is_some_and(|(_,_,cell)|edit.range.end<=cell.range.end)) {
                super::super::markdown_code::preserve_edited_inline_delimiters(self,edits,patches)?;
                markdown_split::remove_empty_emphasis(self,edits,patches)?;
                markdown_split::repair_flanking(self,edits,patches)?;
                return Ok(());
            }
            if edits.iter().all(|edit| {
                !edit.replacement.contains('\n')
                    && !self.text()[edit.range.clone()].contains('\n')
                    && self
                        .projection()
                        .style_spans_for_region(&edit.range)
                        .iter()
                        .any(|span| {
                            span.range.start <= edit.range.start
                                && edit.range.end <= span.range.end
                                && span.application
                                    == StyleApplication::Automatic("Markdown reference".into())
                                && self.text()[span.range.clone()]
                                    .trim_start()
                                    .starts_with('[')
                                && self.text()[span.range.clone()].contains("]:")
                        })
            }) {
                return Ok(());
            }
            markdown_block_styles::preserve_deleted_source_prefixes(self, edits, patches)?;
            super::super::markdown_code::preserve_edited_inline_delimiters(self, edits, patches)?;
            markdown_split::remove_empty_emphasis(self, edits, patches)?;
            markdown_block_styles::remove_empty_continuation_prefixes(self, edits, patches)?;
            markdown_block_styles::preserve_empty_continuation_paragraphs(self, edits, patches)?;
            markdown_block_styles::preserve_deleted_boundary_spaces(self, edits, patches)?;
            markdown_block_styles::preserve_deleted_source_prefixes(self, edits, patches)?;
            markdown_block_styles::preserve_join_boundaries(self, edits, patches)?;
            markdown_block_styles::preserve_edited_paragraph_boundaries(self, edits, patches)?;
            markdown_block_styles::preserve_split_literals(self, edits, patches)?;
            markdown_block_styles::preserve_split_boundaries(
                self,
                paragraph_break_ranges,
                patches,
            )?;
            markdown_block_styles::preserve_retained_literals(self, edits, patches)?;
            markdown_split::repair_flanking(self, edits, patches)?;
            self.repair_markdown_authored_spaces(edits, patches)?;
            self.repair_markdown_reference_spaces(edits, patches)?;
            markdown_list_structure::preserve_empty_item_boundaries(self, edits, patches)?;
        }
        Ok(())
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
                None => None,
            };
            let prepared = scratch.prepare_text_edits_with_patches(vec![edit], patches)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            map = map.then(prepared.text_position_map())?;
            scratch.commit_model_transaction(prepared)?;
        }
        self.prepare_text_edits_with_patches(edits.to_vec(), Some(sources.source_patches()))
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
            Some(sources.source_patches()),
        )
        .map(Some)
    }

    pub(super) fn structural_text_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, DocumentError> {
        if edit.range.is_empty() {
            return Ok(None);
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
        Ok(None)
    }
}
