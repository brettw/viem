//! Semantic paragraph keyboard actions. Source views retain literal newline
//! editing; WYSIWYG hard breaks use each adapter's native inline break syntax.
use super::*;

impl Document {
    /// Paragraph keyboard actions use the formatted boundary, independently
    /// of its source spelling, affinity, or how the caret arrived there.
    fn keyboard_paragraph(&self, at: usize) -> Result<Option<super::super::Block>, DocumentError> {
        if !self.format().is_wysiwyg() {
            return Ok(None);
        }
        super::super::edit_boundary::paragraph_at(self, at)
    }

    pub(crate) fn list_item_indent_key_request(
        &self,
        at: usize,
        unindent: bool,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        if self.format().is_source_view() {
            self.text_point(at)?;
            let source_blocks = self
                .projection()
                .flow_blocks_for_region(&(at..at))
                .unwrap_or_else(|| self.projection().blocks_for_region(&(at..at)));
            let in_list = source_blocks
                .iter()
                .any(|block| matches!(block.kind, super::super::BlockKind::ListItem { .. }));
            if !in_list {
                return Ok(None);
            }
            let source_at = self
                .projection()
                .source_insertion_point(at, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let semantic = Document::from_bytes_with_file_format(
                self.source_bytes(),
                self.encoding(),
                Format::Markdown,
                self.file_format(),
            )?;
            let visible_at = semantic.visible_point_for_source(source_at, true)?;
            let Some(block) = semantic.keyboard_paragraph(visible_at)? else {
                return Ok(None);
            };
            if !matches!(block.kind, super::super::BlockKind::ListItem { .. }) {
                return Ok(None);
            }
            return Ok(Some(ModelRequest::IndentList {
                document: self.id(),
                revision: self.revision(),
                range: at..at,
                unindent,
            }));
        }
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
        if !self.format().is_wysiwyg() {
            // Plain/source projections author a literal source line ending.
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
