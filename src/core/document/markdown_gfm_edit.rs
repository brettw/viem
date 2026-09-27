//! Local supporting syntax for structural edits of alternate Markdown spellings.
use super::replacement::PatchComposition;
use super::*;
impl Document {
    pub(super) fn markdown_rule_text_patches(
        &self,
        edit: &TextEdit,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        if self.format() != Format::Markdown
            || !edit.range.is_empty()
            || !self
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .any(|block| block.thematic_break && block.range == edit.range)
        {
            return Ok(None);
        }
        let at = self
            .projection()
            .source_insertion_point(edit.range.start, true)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let index = self
            .state()
            .source_hard_lines
            .line_at_offset(at)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let line = self
            .state()
            .source_hard_lines
            .get(index)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let bytes = self
            .state()
            .source
            .bytes_in(line.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let decoded = self.encoding().decode_region(&bytes, line.start)?;
        let normalized = normalize(&decoded, self.file_format());
        let raw = normalized.text.trim_end_matches('\n');
        let quote = super::super::markdown_quotes::prefix(raw);
        let list = super::super::markdown_blocks::marker_prefix_length(&raw[quote..]).unwrap_or(0);
        let prefix = &raw[..quote + list];
        let blank = |index| -> Result<bool, ModelTransactionError> {
            let Some(row) = self.state().source_hard_lines.get(index) else {
                return Ok(true);
            };
            let bytes = self
                .state()
                .source
                .bytes_in(row.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let text = self.encoding().decode_region(&bytes, row.start)?.text;
            Ok(text[super::super::markdown_quotes::prefix(&text)..]
                .trim()
                .is_empty())
        };
        let ending = self.file_format().spelling();
        let mut syntax = String::new();
        if index > 0 && !blank(index - 1)? {
            syntax.push_str(&raw[..quote]);
            syntax.push_str(ending);
        }
        syntax.push_str(prefix);
        for (index, text) in edit.replacement.split('\n').enumerate() {
            if index > 0 {
                syntax.push_str(ending);
                syntax.push_str(&raw[..quote]);
                syntax.push_str(ending);
                syntax.push_str(&raw[..quote]);
            }
            let mut body = self.escape_markdown_source_text(
                line.start + self.encoding().encode_fragment(prefix)?.len(),
                text,
            )?;
            let end = body.trim_end_matches([' ', '\t']).len();
            let trailing = body[end..]
                .bytes()
                .map(|byte| if byte == b' ' { "&#32;" } else { "&#9;" })
                .collect::<String>();
            body.truncate(end);
            body.push_str(&trailing);
            syntax.push_str(&body);
        }
        if !blank(index + 1)? {
            syntax.push_str(ending);
            syntax.push_str(&raw[..quote]);
        }
        let source_end = normalized
            .endings
            .first()
            .map_or(line.end, |ending| ending.source.start);
        Ok(Some(vec![SourcePatch::primary(
            line.start..source_end,
            self.encoding().encode_fragment(&syntax)?,
        )]))
    }

    pub(super) fn prepare_markdown_supporting_patches(
        &self,
        patches: Vec<SourcePatch>,
        operation: impl FnOnce(&Document) -> Result<PreparedModelTransaction, ModelTransactionError>,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        let mut scratch = self.scratch_document();
        let mut composition = PatchComposition::new(self.source_byte_len());
        let conversion = scratch.prepare_text_edits_with_patches(Vec::new(), Some(patches))?;
        for patch in conversion.summary.source_patches.iter().rev() {
            composition.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(conversion)?;
        if scratch.text() != self.text() {
            return Err(DocumentError::VerificationFailed.into());
        }
        let prepared = operation(&scratch)?;
        let edits = prepared
            .summary
            .formatted_splices()
            .iter()
            .map(|splice| {
                let range = splice.old_range();
                let new = prepared
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
                let PreparedPublication::State(candidate) = &prepared.publication else {
                    return Err(DocumentError::VerificationFailed.into());
                };
                Ok(TextEdit::new(
                    range,
                    candidate
                        .projection
                        .text_tree()
                        .slice(new..new + splice.inserted_len())
                        .map_err(DocumentError::FormattedTextStorage)?,
                ))
            })
            .collect::<Result<Vec<_>, ModelTransactionError>>()?;
        for patch in prepared.summary.source_patches.iter().rev() {
            composition.splice(patch.range(), patch.replacement());
        }
        scratch.commit_model_transaction(prepared)?;
        self.prepare_text_edits_with_patches(edits, Some(composition.source_patches()))
    }

    pub(super) fn markdown_structural_support(
        &self,
        edits: &[TextEdit],
    ) -> Result<Vec<SourcePatch>, ModelTransactionError> {
        if self.format() != Format::Markdown {
            return Ok(Vec::new());
        }
        let mut patches = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for edit in edits {
            if !edit.replacement.contains('\n') && !self.text()[edit.range.clone()].contains('\n') {
                continue;
            }
            let range =
                edit.range.start.saturating_sub(2)..(edit.range.end + 1).min(self.text().len());
            for block in self.projection().blocks_for_region(&range) {
                if !seen.insert(block.id) {
                    continue;
                }
                if block.thematic_break {
                    if block.range == edit.range {
                        continue;
                    }
                    if let Some(support) =
                        self.markdown_rule_text_patches(&TextEdit::new(block.range.clone(), ""))?
                    {
                        patches.extend(support);
                    }
                    continue;
                }
                let super::super::BlockKind::Heading(level) = block.kind else {
                    continue;
                };
                let Some(source) = self.projection().source_range(block.range.clone()) else {
                    continue;
                };
                let Some(line) = self.state().source_hard_lines.line_at_offset(source.end) else {
                    continue;
                };
                let Some(underline) = self.state().source_hard_lines.get(line + 1) else {
                    continue;
                };
                let bytes = self
                    .state()
                    .source
                    .bytes_in(underline.clone())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let decoded = self.encoding().decode_region(&bytes, underline.start)?;
                let normalized = normalize(&decoded, self.file_format());
                let text = normalized.text.trim_end_matches('\n');
                let body = text.trim();
                if body.is_empty()
                    || !body
                        .bytes()
                        .all(|b| b == if level == 1 { b'=' } else { b'-' })
                {
                    continue;
                }
                let end = normalized
                    .endings
                    .first()
                    .map_or(underline.end, |ending| ending.source.start);
                patches.push(SourcePatch::primary(
                    source.start..source.start,
                    self.encoding()
                        .encode_fragment(&format!("{} ", "#".repeat(level as usize)))?,
                ));
                // A native ATX boundary uses the same paired separator spelling
                // as other editable paragraphs, including when Setext needed only one.
                let following = self
                    .state()
                    .source_hard_lines
                    .get(line + 2)
                    .and_then(|range| {
                        self.state()
                            .source
                            .bytes_in(range.clone())
                            .map(|bytes| (range, bytes))
                    });
                let pad = if let Some((range, bytes)) = following {
                    !self
                        .encoding()
                        .decode_region(&bytes, range.start)?
                        .text
                        .trim()
                        .is_empty()
                } else {
                    false
                };
                patches.push(SourcePatch::primary(
                    source.end..end,
                    if pad {
                        self.encoding()
                            .encode_fragment(self.file_format().spelling())?
                    } else {
                        Vec::new()
                    },
                ));
            }
        }
        Ok(patches)
    }
    pub(super) fn repair_markdown_authored_spaces(
        &self,
        edits: &[TextEdit],
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        let mut targets = Vec::new();
        for edit in edits {
            let count = edit.replacement.bytes().take_while(|b| *b == b' ').count();
            if count == 0 {
                continue;
            }
            if let Some(block) = self
                .projection()
                .blocks_for_region(&edit.range)
                .iter()
                .find(|block| {
                    matches!(block.kind, super::super::BlockKind::Heading(_))
                        && block.range.start == edit.range.start
                })
            {
                if let Some(at) = self
                    .projection()
                    .source_insertion_point(block.range.start, true)
                {
                    if let Some(row) = self
                        .state()
                        .source_hard_lines
                        .line_at_offset(at)
                        .and_then(|i| self.state().source_hard_lines.get(i))
                    {
                        targets.push((row.start, count));
                    }
                }
            }
        }
        if targets.is_empty() {
            return Ok(());
        }
        let mut ordered = patches.clone();
        validate_source_patches(&mut ordered)?;
        let candidate = apply_source_patches(&self.state().source, &ordered)?;
        let mut support = Vec::new();
        for (at, count) in targets {
            let at = rebase_source_boundary(at, &ordered, Association::BeforeInsertion)?;
            let bytes = candidate
                .bytes_in(at..(at + 1024).min(candidate.len()))
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, at)?;
            let text = &decoded.text;
            let indent = text.bytes().take_while(|b| *b == b' ').count();
            let hashes = text[indent..].bytes().take_while(|b| *b == b'#').count();
            if !(1..=6).contains(&hashes) {
                continue;
            }
            let prefix = indent + hashes;
            let spaces = text[prefix..].bytes().take_while(|b| *b == b' ').count();
            if spaces <= 1 {
                continue;
            }
            let count = count.min(spaces - 1);
            let start = at
                + self
                    .encoding()
                    .encode_fragment(&text[..prefix + spaces - count])?
                    .len();
            let end = at
                + self
                    .encoding()
                    .encode_fragment(&text[..prefix + spaces])?
                    .len();
            support.push((
                start..end,
                self.encoding().encode_fragment(&"&#32;".repeat(count))?,
            ));
        }
        let mut composition = PatchComposition::new(self.source_byte_len());
        for patch in ordered.iter().rev() {
            composition.splice(patch.range.clone(), &patch.replacement);
        }
        support.sort_by_key(|(range, _)| range.start);
        for (range, bytes) in support.into_iter().rev() {
            composition.splice(range, &bytes);
        }
        *patches = composition
            .patches()
            .into_iter()
            .map(|(range, bytes)| SourcePatch::primary(range, bytes))
            .collect();
        Ok(())
    }

    pub(super) fn repair_markdown_reference_spaces(
        &self,
        edits: &[TextEdit],
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        let mut definitions = Vec::new();
        for edit in edits {
            // Joining at the end of a definition can make newly authored
            // whitespace part of its literal spelling as well.
            let region = edit.range.start.saturating_sub(1)..edit.range.end;
            for span in self.projection().style_spans_for_region(&region) {
                if span.application == StyleApplication::Automatic("Markdown reference".into())
                    && self.text()[span.range.clone()]
                        .trim_start()
                        .starts_with('[')
                    && self.text()[span.range.clone()].contains("]:")
                {
                    if let Some(source) = self.projection().source_range(span.range) {
                        definitions.push(source);
                    }
                }
            }
        }
        if !definitions.is_empty() {
            let entity = self.encoding().encode_fragment("&#32;")?;
            for patch in patches {
                if patch.replacement == entity
                    && definitions.iter().any(|range| {
                        range.start <= patch.range.start && patch.range.start <= range.end
                    })
                {
                    patch.replacement = self.encoding().encode_fragment(" ")?;
                }
            }
        }
        Ok(())
    }
}
