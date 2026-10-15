//! Semantic paragraph keyboard actions. Literal formats retain literal newline
//! editing; Markdown hard breaks use its native inline break syntax.
use super::*;

impl Document {
    /// A newly opened final line needs its own terminator when the source did
    /// not have one. Otherwise Escape/reopen would turn that authored empty
    /// line into the preceding line's final delimiter.
    pub(crate) fn literal_terminal_break_cursor(&self, at: usize, prefix_len: usize) -> Option<usize> {
        if !self.format().is_literal() || at != self.projection().text_tree().byte_len() {
            return None;
        }
        if at > 0 && self.hard_line_snapshot().line_at_offset(at).ok()?.content_range().start == at {
            return None;
        }
        at.checked_add(1)?.checked_add(prefix_len)
    }

    pub(crate) fn prepare_literal_terminal_break(
        &self,
        at: usize,
        prefix: &str,
    ) -> Result<Option<(PreparedModelTransaction, usize)>, ModelTransactionError> {
        let Some(cursor) = self.literal_terminal_break_cursor(at, prefix.len()) else { return Ok(None); };
        let prepared = self.prepare_text_edits(vec![TextEdit::new(at..at, format!("\n{prefix}\n"))])?;
        self.prepared_text_point(&prepared, cursor)?;
        Ok(Some((prepared, cursor)))
    }

    /// Return at a folded source ending replaces its displayed space with the
    /// paragraph boundary. Literal spaces, hard breaks, and Source retain their
    /// ordinary editing rules.
    pub(super) fn prepare_enter_at_folded_ending(
        &self,
        at: usize,
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.format() != Format::Markdown
            || self.projection().text_tree().slice(at..at.saturating_add(1)).as_deref() != Ok(" ")
            || self.projection().table_cell_at(at).is_some()
        {
            return Ok(None);
        }
        let range = at..at + 1;
        let Some(source) = self.projection().source_range(range.clone()) else {
            return Ok(None);
        };
        let bytes = self.state().source.bytes_in(source.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, source.start)?;
        if normalize(&decoded, self.file_format()).endings.is_empty()
            || self.projection().style_spans_for_region(&range).iter().any(|span|
                span.application == StyleApplication::Semantic(SemanticInlineStyle::Code))
        {
            return Ok(None);
        }
        if self.projection().blocks_for_region(&range).iter().any(|block|
            block.kind == super::super::BlockKind::Paragraph && block.quote_depth == 0
                && block.containers.is_empty() && !block.markdown_html)
        {
            // Ordinary prose has no container continuation policy. Express the
            // replacement directly so projection work stays paragraph-local.
            return self.prepare_text_edits(vec![TextEdit::new(range, "\n")]).map(Some);
        }
        let mut scratch = self.scratch_document();
        let mut sources = replacement::PatchComposition::new(self.source_byte_len());
        let mut formatted = replacement::PatchComposition::new(self.projection().text_tree().byte_len());
        let deletion = scratch.prepare_text_edits(vec![TextEdit::new(range, "")])?;
        formatted.record_formatted(&deletion)?;
        for patch in deletion.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(deletion)?;
        let split = scratch.prepare_structural_list_enter(at)?;
        formatted.record_formatted(&split)?;
        for patch in split.summary.source_patches.iter().rev() {
            sources.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(split)?;
        let prepared = self.prepare_text_edits_with_patches(
            formatted.formatted_edits(), Some(sources.source_patches()),
        )?;
        let PreparedPublication::State(candidate) = &prepared.publication else {
            return Err(DocumentError::VerificationFailed.into());
        };
        if candidate.projection.text() != scratch.text()
            || !candidate.projection.has_same_hard_line_structure(scratch.projection())
            || candidate.projection.style_spans() != scratch.projection().style_spans()
        {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok(Some(prepared))
    }

    /// Paragraph keyboard actions use the formatted boundary, independently
    /// of its source spelling, affinity, or how the caret arrived there.
    fn keyboard_paragraph(&self, at: usize) -> Result<Option<super::super::Block>, DocumentError> {
        if !self.format().is_markdown() {
            return Ok(None);
        }
        super::super::edit_boundary::paragraph_at(self, at)
    }

    pub(crate) fn list_item_indent_key_request(
        &self,
        at: usize,
        unindent: bool,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        let Some(block) = self.keyboard_paragraph(at)? else {
            return Ok(None);
        };
        Ok(
            matches!(block.kind, super::super::BlockKind::ListItem { .. }).then(|| {
                ModelRequest::IndentList {
                    document: self.id(),
                    revision: self.revision(),
                    range: block.range,
                    unindent,
                }
            }),
        )
    }

    pub(crate) fn paragraph_boundary_reset_request(
        &self,
        at: usize,
        empty_quote_only: bool,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        let Some(block) = self.keyboard_paragraph(at)? else {
            return Ok(None);
        };
        let quote = block.style == StyleId::from("Block quote") || block.quote_depth > 0;
        if empty_quote_only {
            // Literal code owns its blank lines even inside a quotation.
            if super::super::edit_boundary::is_code_paragraph(self, &block)? {
                return Ok(None);
            }
            if !quote || !self.text()[block.range.clone()].trim().is_empty() {
                return Ok(None);
            }
            // An empty quoted list item leaves its list one level at a time
            // before the quote itself is removed.
            if matches!(block.kind, super::super::BlockKind::ListItem { item_start: true, .. }) {
                return Ok(None);
            }
        } else if at != block.range.start {
            return Ok(None);
        }
        if !empty_quote_only
            && matches!(
                block.kind,
                super::super::BlockKind::ListItem {
                    item_start: false,
                    ..
                }
            )
        {
            // A continuation paragraph has no label to remove. Backspace
            // joins its body to the preceding paragraph of the same item.
            return Ok(None);
        }
        if !empty_quote_only
            && matches!(
                block.kind,
                super::super::BlockKind::ListItem {
                    level: 1..,
                    item_start: true,
                    ..
                }
            )
        {
            // A nested item first moves out by one structural level. Only a
            // top-level item loses its list treatment at the body boundary.
            let range = block.range.clone();
            return Ok(Some(ModelRequest::IndentList {
                document: self.id(),
                revision: self.revision(),
                range,
                unindent: true,
            }));
        }
        if empty_quote_only
            || super::super::edit_boundary::is_code_paragraph(self, &block)?
            || matches!(block.kind, super::super::BlockKind::ListItem { .. })
        {
            return Ok(Some(ModelRequest::SetParagraphStyle {
                document: self.id(),
                revision: self.revision(),
                range: block.range,
                style: self.projection().style_sheet().base_paragraph.clone(),
            }));
        }
        Ok(None)
    }

    pub(super) fn prepare_intra_paragraph_break(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.text_point(at)?;
        if !self.format().is_markdown() {
            // Plain/source projections author a literal source line ending.
            if let Some((prepared, _)) = self.prepare_literal_terminal_break(at, "")? {
                return Ok(prepared);
            }
            return self.prepare_text_edits(vec![TextEdit::new(at..at, "\n")]);
        }
        if self.projection().table_cell_at(at).is_some() {return self.prepare_text_edits(vec![TextEdit::new(at..at,"\n")]);}
        let block = super::super::edit_boundary::paragraph_at(self, at)?
            .ok_or(DocumentError::AmbiguousProjection)?;
        if self.format() == Format::Markdown
            && self.projection().style_spans_for_region(&(at..at)).iter().any(|span|
                span.application == StyleApplication::Automatic("Comment".into())
                    && span.range.start < at && at < span.range.end)
        {
            // Visible comment contents are literal, so backslash and <br>
            // would appear as text. Its own source newline is the hard break.
            let prepared = self.prepare_text_edits(vec![TextEdit::new(at..at, "\n")])?;
            return self.verify_hard_break_paragraph(prepared, &block);
        }
        if self.format() == Format::Markdown {
            // An empty overlap query excludes a span's edges. Include both
            // neighboring sides so comment starts and ends remain reachable.
            let nearby = at.saturating_sub(1)..at.saturating_add(1).min(self.text().len());
            if let Some(comment) = self.projection().style_spans_for_region(&nearby).iter().find(|span|
                span.application == StyleApplication::Automatic("Comment".into())
                    && (span.range.start == at || span.range.end == at))
            {
                // A break beside a block comment must join its existing
                // literal lines in the same paragraph. Retire that opener
                // locally, protecting the comment's existing hard breaks.
                let mut patches = self.literal_comment_support(comment.range.clone())?;
                let source = self.projection().source_insertion_point(at, at == comment.range.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                patches.push(SourcePatch::primary(source..source, self.encoding().encode_fragment("<br>")?));
                let prepared = self.prepare_text_edits_with_patches(
                    vec![TextEdit::new(at..at, "\n")], Some(patches),
                )?;
                return self.verify_hard_break_paragraph(prepared, &block);
            }
        }
        if self.format() == Format::Markdown && block.thematic_break {
            // Typing at the rule's editable boundary replaces its furniture.
            // Retire the rule first so the requested break belongs to ordinary
            // prose, with the same verified supporting edit as printable text.
            if let Some(patches) = self.markdown_rule_text_patches(&TextEdit::new(at..at, ""))? {
                return self.prepare_markdown_supporting_patches(patches, |document| {
                    document.prepare_intra_paragraph_break(at, affinity)
                });
            }
        }
        if self.format() == Format::Markdown
            && (block.style == StyleId::from("Block quote") || block.quote_depth > 0)
            && super::super::markdown_quotes::is_fenced_block(self, &block)?
        {
            let prepared = self
                .prepare_markdown_quote_enter(at)?
                .ok_or(DocumentError::AmbiguousProjection)?;
            return self.verify_hard_break_paragraph(prepared, &block);
        }
        if self.format() == Format::Markdown && block.style == StyleId::from("Code Block") {
            // HTML's ordinary text insertion writes <br> with its verified
            // whitespace protections. Verify paragraph ownership as well as
            // the requested text, including at empty and terminal boundaries.
            let prepared = self.prepare_text_edits(vec![TextEdit::new(at..at, "\n")])?;
            return self.verify_hard_break_paragraph(prepared, &block);
        }
        let source_at = self
            .projection()
            .source_insertion_point(
                at,
                at == block.range.start
                    || affinity == BoundaryAffinity::Downstream && at < self.text().len(),
            )
            .ok_or(DocumentError::AmbiguousProjection)?;
        let before_folded_ending = at < block.range.end
            && self.projection().text_tree().slice(at..at + 1).as_deref() == Ok(" ")
            && self.projection().source_range(at..at + 1).is_some_and(|range| {
                self.state().source.bytes_in(range.clone())
                    .and_then(|bytes| self.encoding().decode_region(&bytes, range.start).ok())
                    .is_some_and(|decoded| !normalize(&decoded, self.file_format()).endings.is_empty())
            });
        let inline_html = self.format() == Format::Markdown
            && (matches!(block.kind, super::super::BlockKind::Heading(_))
                || at == block.range.end
                || before_folded_ending
                || self.text()[block.range.clone()].trim().is_empty());
        let mut syntax = if inline_html {
            // An ATX heading cannot span physical lines. Bare br is also
            // stable in an empty quote/item, where blank continuation lines
            // otherwise acquire paragraph or empty-item structure.
            "<br>".to_owned()
        } else {
            let source_start = self
                .projection()
                .source_insertion_point(block.range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let line_index = self
                .state()
                .source_hard_lines
                .line_at_offset(source_start)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let line = self
                .state()
                .source_hard_lines
                .get(line_index)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let bytes = self
                .state()
                .source
                .bytes_in(line.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let text = self.encoding().decode_region(&bytes, line.start)?.text;
            let quote_length = super::super::markdown_quotes::prefix(&text);
            let quote = &text[..quote_length];
            let indentation = if matches!(block.kind, super::super::BlockKind::ListItem { .. }) {
                super::super::markdown_blocks::marker_prefix_length(&text[quote_length..])
                    .unwrap_or_else(|| {
                        text[quote_length..].len()
                            - text[quote_length..].trim_start_matches([' ', '\t']).len()
                    })
            } else {
                0
            };
            format!("\\\n{quote}{}", " ".repeat(indentation))
        };
        if inline_html && block.range.is_empty() {
            if let Some(line) = self.state().source_hard_lines.line_at_offset(source_at)
                .and_then(|index| self.state().source_hard_lines.get(index)) {
                let bytes = self.state().source.bytes_in(line.start..source_at)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let prefix = self.encoding().decode_region(&bytes, line.start)?.text;
                if !prefix.ends_with([' ', '\t'])
                    && (block.style.0.starts_with("Heading")
                        || matches!(block.kind, super::super::BlockKind::ListItem { .. })) {
                    syntax.insert(0, ' ');
                }
            }
        }
        let in_code = self.projection().style_spans_for_region(
            &(at.saturating_sub(1)..at.saturating_add(1).min(self.text().len())),
        ).into_iter().any(|span| span.range.start <= at && at <= span.range.end
            && (matches!(span.application, StyleApplication::Semantic(SemanticInlineStyle::Code))
                || matches!(span.application, StyleApplication::Named(ref id) if id.0 == "Code")));
        let patches = if self.format() == Format::Markdown && (!inline_html || in_code) {
            markdown_split::hard_break_patches(self, at, source_at, &syntax)?
        } else {
            None
        };
        let mut patches = patches.unwrap_or(vec![SourcePatch::primary(
            source_at..source_at,
            self.encoding()
                .encode_fragment(&spell_logical_breaks(&syntax, self.file_format()))?,
        )]);
        self.preserve_edited_reference_content(&[TextEdit::new(at..at, "\n")], &mut patches)?;
        markdown_block_styles::preserve_split_boundaries(self, std::iter::once(&(at..at)), &mut patches)?;
        let prepared =
            self.prepare_text_edits_with_patches(vec![TextEdit::new(at..at, "\n")], Some(patches))?;
        self.verify_hard_break_paragraph(prepared, &block)
    }

    fn verify_hard_break_paragraph(
        &self,
        prepared: PreparedModelTransaction,
        block: &super::super::Block,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if let PreparedPublication::State(candidate) = &prepared.publication {
            let map = |at, association| -> Result<usize, ModelTransactionError> {
                Ok(prepared
                    .text_position_map()
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
            let start = map(block.range.start, Association::BeforeInsertion)?;
            let end = map(block.range.end, Association::AfterInsertion)?;
            let target = candidate
                .projection
                .blocks_for_region(&(start..end))
                .into_iter()
                .find(|next| next.range == (start..end))
                .ok_or(DocumentError::VerificationFailed)?;
            if target.style != block.style
                || target.kind != block.kind
                || target.direct_paragraph != block.direct_paragraph
                || target.direct_default_character != block.direct_default_character
            {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        Ok(prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_literal_break_preparation_only_reads_local_source() {
        let source = format!("{}tail", "line\n".repeat(100_000));
        let mut document = Document::new(source.clone());
        let (result, work) = crate::document::measure_document_work(|| {
            let (prepared, cursor) = document.prepare_literal_terminal_break(source.len(), "").unwrap().unwrap();
            document.commit_model_transaction(prepared).unwrap();
            cursor
        });
        assert_eq!(result, source.len() + 1);
        assert_eq!(document.addressable_hard_line_count(), 100_002);
        assert_eq!(document.hard_line_snapshot().slice_utf8(source.len()..source.len() + 2).unwrap(), "\n\n");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
        assert!(work.source_range_materialized_bytes < 4096, "{work:?}");
    }
}
