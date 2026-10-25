//! Authoring visible HTML comments through the character-style menu.
use super::replacement::PatchComposition;
use super::*;

fn is_comment(span: &StyleSpan) -> bool {
    matches!(&span.application, StyleApplication::Automatic(id) if id.0 == "Comment")
}

impl Document {
    fn unfinished_comment_contains(&self, range: &Range<usize>) -> bool {
        let text = self.projection().text_tree();
        let styled = self
            .projection()
            .style_spans_for_region(range)
            .iter()
            .any(|span| {
                if !is_comment(span)
                    || span.range.start > range.start
                    || span.range.end < range.end
                    || span.range.len() < 4
                    || text
                        .slice(span.range.start..span.range.start + 4)
                        .as_deref()
                        != Ok("<!--")
                {
                    return false;
                }
                // Comment block styles include trailing text on the closing line.
                // Search for an actual closer instead of testing the span's suffix.
                // Read rope chunks so a long malformed comment needs no flat copy.
                let mut at = span.range.start + 4;
                let mut previous = [0u8; 2];
                while at < span.range.end {
                    let chunk = text.byte_chunk_at(at);
                    let chunk = &chunk[..chunk.len().min(span.range.end - at)];
                    for &byte in chunk {
                        if previous == [b'-', b'-'] && byte == b'>' {
                            return false;
                        }
                        previous = [previous[1], byte];
                    }
                    at += chunk.len();
                }
                true
            });
        styled
            || self
                .unfinished_inline_comment_contains(range)
                .unwrap_or(false)
    }

    fn unfinished_inline_comment_contains(
        &self,
        range: &Range<usize>,
    ) -> Result<bool, DocumentError> {
        let Some(selected) = self.projection().source_range(range.clone()) else {
            return Ok(false);
        };
        let line = self.physical_line_source_range(selected.start)?;
        // This is only a failure diagnostic. Keep malformed, unstyled inline
        // syntax inspection bounded rather than flattening an arbitrarily long line.
        if line.len() > 64 * 1024 {
            return Ok(false);
        }
        let bytes = self
            .state()
            .source
            .bytes_in(line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let last_closer = decoded.text.rfind("-->");
        for (at, _) in decoded.text.match_indices("<!--") {
            let Some(source_at) = decoded.source_boundary(at) else {
                continue;
            };
            if source_at >= selected.start {
                break;
            }
            if decoded.text[..at]
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count()
                % 2
                != 0
            {
                continue;
            }
            if last_closer.is_none_or(|closer| closer < at) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn comment_replace_target(&self, at: usize) -> Option<usize> {
        let comment = self.comment_containing(&(at..at))?;
        (at + 4 == comment.end
            && self
                .projection()
                .text_tree()
                .slice(at..comment.end)
                .as_deref()
                == Ok(" -->"))
        .then_some(comment.end)
    }

    pub(super) fn prepare_comment_replacement(
        &self,
        caret: usize,
        edit: FormattedPayloadEdit,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        let deleted = scratch.prepare_text_edits(vec![TextEdit::new(edit.range.clone(), "")])?;
        record(&mut scratch, deleted, &mut sources, &mut formatted)?;
        let insertion = FormattedPayloadEdit::new(caret..caret, edit.payload)
            .with_boundary_affinity(BoundaryAffinity::Upstream);
        let (inserted, after, _) = scratch.prepare_comment_typing(insertion)?;
        record(&mut scratch, inserted, &mut sources, &mut formatted)?;
        let prepared = self.prepare_text_edits_with_patch_policy(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
            true,
        )?;
        Ok((prepared, after))
    }

    fn comment_containing(&self, range: &Range<usize>) -> Option<Range<usize>> {
        self.projection()
            .style_spans_for_region(range)
            .into_iter()
            .find(|span| {
                is_comment(span) && span.range.start <= range.start && range.end <= span.range.end
            })
            .map(|span| span.range)
            .filter(|span| {
                span.len() >= 7
                    && self
                        .projection()
                        .text_tree()
                        .slice(span.start..span.start + 4)
                        .as_deref()
                        == Ok("<!--")
                    && self
                        .projection()
                        .text_tree()
                        .slice(span.end - 3..span.end)
                        .as_deref()
                        == Ok("-->")
            })
    }

    pub(super) fn prepare_comment_choice(
        &self,
        range: Range<usize>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        if range.is_empty() {
            return self
                .prepare_empty_comment(range.start)
                .map(|(prepared, _)| prepared);
        }
        if self.comment_containing(&range).is_some() {
            return Ok(self.no_op_prepared());
        }
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        // Reuse the same clearing policy as Code. Plain selections need no
        // intermediate transaction or whole-projection style inspection.
        if self
            .projection()
            .style_spans_for_region(&range)
            .iter()
            .any(|span| {
                matches!(
                    span.application,
                    StyleApplication::Semantic(_)
                        | StyleApplication::Named(_)
                        | StyleApplication::Direct(_)
                )
            })
        {
            let clear = scratch.prepare_character_style_choice(range.clone(), "".into())?;
            record(&mut scratch, clear, &mut sources, &mut formatted)?;
        }
        let wrapped = scratch.prepare_comment_wrapper(range)?;
        record(&mut scratch, wrapped, &mut sources, &mut formatted)?;
        self.prepare_text_edits_with_patch_policy(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
            true,
        )
    }

    fn prepare_comment_wrapper(
        &self,
        range: Range<usize>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let body = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        self.prepare_comment_wrapper_body(range, &body)
    }

    pub(crate) fn prepare_empty_comment(
        &self,
        at: usize,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        self.validate_typing_named_style(&"Comment".into())?;
        let mut scratch = self.scratch_document();
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        // Establish an unstyled source insertion boundary, including at the
        // edge of a code span, before replacing this temporary character with syntax.
        // Only the composed comment transaction is ever published.
        let payload = FormattedTextPayload::new(&scratch.hard_line_snapshot(), "x", vec![])
            .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?;
        let (inserted, end, start) = scratch.prepare_insertion_with_typing_context(
            FormattedPayloadEdit::new(at..at, payload),
            Some(&"".into()),
            &[],
            None,
        )?;
        record(&mut scratch, inserted, &mut sources, &mut formatted)?;
        let wrapped = scratch.prepare_comment_wrapper_body(start..end, "")?;
        record(&mut scratch, wrapped, &mut sources, &mut formatted)?;
        let prepared = self.prepare_text_edits_with_patch_policy(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
            true,
        )?;
        Ok((prepared, start + 5))
    }

    fn prepare_comment_wrapper_body(
        &self,
        range: Range<usize>,
        body: &str,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        if body.contains("-->") || body.contains("<!--") || body.contains('\u{fffc}') {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let blocks = self.projection().blocks_for_region(&range);
        if blocks
            .iter()
            .any(|block| block.style.0 == "Code Block" || block.is_front_matter())
        {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let source = self
            .projection()
            .source_range(range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let visible = format!("<!-- {body} -->");
        // A comment at a physical line start otherwise makes the rest of that
        // line an HTML block. Keep any unselected suffix in its prose context.
        let source_line_start = self
            .state()
            .source_hard_lines
            .line_at_offset(source.start)
            .and_then(|line| self.state().source_hard_lines.get(line))
            .is_some_and(|line| line.start == source.start);
        let inline = blocks.iter().any(|block| {
            block.quote_depth > 0
                || matches!(block.kind, super::super::BlockKind::ListItem { .. })
                || (source_line_start || block.range.start == range.start)
                    && range.end < block.range.end
        });
        let syntax = if inline {
            format!("<span>{visible}</span>")
        } else {
            visible.clone()
        };
        let bytes = self
            .encoding()
            .encode_fragment(&syntax.replace('\n', self.file_format().spelling()))?;
        let prepared = self.prepare_text_edits_with_patch_policy(
            vec![TextEdit::new(range.clone(), visible.clone())],
            Some(vec![SourcePatch::primary(source, bytes)]),
            true,
        )?;
        if let PreparedPublication::State(candidate) = &prepared.publication {
            let target = range.start..range.start + visible.len();
            if !candidate
                .projection
                .style_spans_for_region(&target)
                .iter()
                .any(|span| is_comment(span) && span.range == target)
            {
                if self.unfinished_comment_contains(&range) {
                    return Err(DocumentError::UnfinishedComment.into());
                }
                return Err(DocumentError::UnsupportedFormatting.into());
            }
        }
        Ok(prepared)
    }

    pub(super) fn prepare_comment_typing(
        &self,
        edit: FormattedPayloadEdit,
    ) -> Result<(PreparedModelTransaction, usize, usize), ModelTransactionError> {
        let at = edit.range.start;
        if let Some(comment) = self.comment_containing(&edit.range) {
            // Delimiters are visible text, but pending Comment typing continues
            // only inside their body. Replacing a whole comment creates a new one.
            if comment.start + 4 <= at && edit.range.end + 3 <= comment.end {
                let end = edit.range.end;
                let prepared = self.prepare_formatted_payload_edits(vec![edit])?;
                let caret = prepared
                    .text_position_map()
                    .map_text_point(
                        self.text_point(end)?,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )?
                    .value()
                    .ok_or(DocumentError::AmbiguousProjection)?
                    .offset();
                return Ok((prepared, caret, at));
            }
        }
        if edit.payload.text().is_empty() {
            return Ok((self.no_op_prepared(), at, at));
        }
        let mut scratch = self.scratch_document();
        let (inserted, caret, start) =
            scratch.prepare_insertion_with_typing_context(edit, Some(&"".into()), &[], None)?;
        let mut sources = PatchComposition::new(self.source_byte_len());
        let mut formatted = PatchComposition::new(self.projection().text_tree().byte_len());
        record(&mut scratch, inserted, &mut sources, &mut formatted)?;
        let wrapped = scratch.prepare_comment_wrapper(start..caret)?;
        record(&mut scratch, wrapped, &mut sources, &mut formatted)?;
        let prepared = self.prepare_text_edits_with_patch_policy(
            formatted.formatted_edits(),
            Some(sources.source_patches()),
            true,
        )?;
        Ok((prepared, caret + 5, start + 5))
    }
}

fn record(
    document: &mut Document,
    prepared: PreparedModelTransaction,
    sources: &mut PatchComposition,
    formatted: &mut PatchComposition,
) -> Result<(), ModelTransactionError> {
    for patch in prepared.summary.source_patches.iter().rev() {
        sources.splice(patch.range(), patch.replacement());
    }
    formatted.record_formatted(&prepared)?;
    document.commit_model_transaction(prepared)?;
    Ok(())
}
