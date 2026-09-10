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
            let in_list = source_blocks.iter().any(|block| {
                matches!(block.kind, super::super::BlockKind::ListItem { .. })
            }) || self.projection().style_spans_touching(&(at..at)).iter().any(|span| {
                matches!(&span.application, StyleApplication::SourceParagraph { style, .. } if style.is_internal_list())
            });
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
                if self.format() == Format::HtmlSource {
                    Format::Html
                } else {
                    Format::Markdown
                },
                self.file_format(),
            )?;
            let visible_at = semantic.visible_point_for_source(source_at, true)?;
            let Some(block) = semantic.keyboard_paragraph(visible_at)? else {
                return Ok(None);
            };
            if visible_at != block.range.start
                || !matches!(
                    block.kind,
                    super::super::BlockKind::ListItem {
                        item_start: true,
                        ..
                    }
                )
            {
                return Ok(None);
            }
            // In source mode, the actual body start remains a structural
            // boundary even when opening inline syntax precedes it. Markdown
            // additionally exposes its literal marker and marker/body boundary.
            let at_body_start = semantic
                .projection()
                .source_insertion_point(visible_at, true)
                == Some(source_at);
            let at_marker = self.format() == Format::MarkdownSource
                && source_blocks.iter().any(|block| {
                    self.projection()
                        .list_marker_range_for_block(block)
                        .is_some_and(|marker| at == marker.start || at == marker.end)
                });
            return Ok(
                (at_body_start || at_marker).then(|| ModelRequest::IndentList {
                    document: self.id(),
                    revision: self.revision(),
                    range: at..at,
                    unindent,
                }),
            );
        }
        let Some(block) = self.keyboard_paragraph(at)? else {
            return Ok(None);
        };
        Ok((at == block.range.start
            && matches!(
                block.kind,
                super::super::BlockKind::ListItem {
                    item_start: true,
                    ..
                }
            ))
        .then(|| ModelRequest::IndentList {
            document: self.id(),
            revision: self.revision(),
            range: block.range,
            unindent,
        }))
    }

    pub(crate) fn paragraph_boundary_reset_request(
        &self,
        at: usize,
        empty_quote_only: bool,
    ) -> Result<Option<ModelRequest>, DocumentError> {
        let Some(block) = self.keyboard_paragraph(at)? else {
            return Ok(None);
        };
        let quote = block.style == StyleId::from("Block quote");
        if empty_quote_only {
            if !quote || !self.text()[block.range.clone()].trim().is_empty() {
                return Ok(None);
            }
        } else if at != block.range.start {
            return Ok(None);
        }
        if empty_quote_only || super::super::edit_boundary::is_code_paragraph(self, &block)?
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
        let block = super::super::edit_boundary::paragraph_at(self, at)?
            .ok_or(DocumentError::AmbiguousProjection)?;
        if self.format() == Format::Markdown
            && block.style == StyleId::from("Block quote")
            && super::super::markdown_quotes::is_fenced_block(self, &block)?
        {
            let prepared = self
                .prepare_markdown_quote_enter(at)?
                .ok_or(DocumentError::AmbiguousProjection)?;
            return self.verify_hard_break_paragraph(prepared, &block);
        }
        if self.format() == Format::Html
            || self.format() == Format::Markdown && block.style == StyleId::from("Code Block")
        {
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
        let inline_html = self.format() == Format::Markdown
            && (matches!(block.kind, super::super::BlockKind::Heading(_))
                || self.text()[block.range.clone()].trim().is_empty());
        let syntax = if self.format() == Format::Rtf {
            "\\line ".to_owned()
        } else if inline_html {
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
        let patches = patches.unwrap_or(vec![SourcePatch::primary(
            source_at..source_at,
            self.encoding()
                .encode_fragment(&spell_logical_breaks(&syntax, self.file_format()))?,
        )]);
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
