//! Named character assignments stay inside text-bearing block contents.
use super::*;

fn fragments(document: &Document, range: &Range<usize>) -> Vec<(Range<usize>, bool)> {
    let mut result = Vec::new();
    for block in document.projection().blocks_for_region(range) {
        let start = range.start.max(block.range.start);
        let end = range.end.min(block.range.end);
        if start >= end {
            continue;
        }
        let mut at = start;
        for separator in document.projection().hard_breaks_for_region(&(start..end)) {
            if at < separator {
                result.push((at..separator, block.style.0 == "Code Block"));
            }
            at = separator + 1;
        }
        if at < end {
            result.push((at..end, block.style.0 == "Code Block"));
        }
    }
    result
}

fn verify_assignment(
    before: &FormattedDocument,
    after: &FormattedDocument,
    range: &Range<usize>,
    style: &StyleId,
) -> Result<(), DocumentError> {
    let mut boundaries = BTreeSet::from([0, range.start, range.end]);
    for span in before.style_spans().iter().chain(after.style_spans()) {
        boundaries.extend([span.range.start, span.range.end]);
    }
    for block in before.blocks().iter().chain(after.blocks()) {
        boundaries.extend([block.range.start, block.range.end]);
    }
    for at in boundaries {
        if at >= before.text().len() || before.text().as_bytes()[at] == b'\n' {
            continue;
        }
        let old = before
            .selected_named_styles(at..at + 1, BoundaryAffinity::Downstream)
            .character;
        let new = after
            .selected_named_styles(at..at + 1, BoundaryAffinity::Downstream)
            .character;
        let preserved_paragraph = before
            .blocks_for_region(&(at..at + 1))
            .iter()
            .any(|block| block.style.0 == "Code Block" && block.range.contains(&at));
        let expected = if range.contains(&at) && !preserved_paragraph {
            (!style.0.is_empty()).then(|| style.clone())
        } else {
            old
        };
        if new != expected {
            return Err(DocumentError::VerificationFailed);
        }
        let direct = |projection: &FormattedDocument| {
            let mut properties = CharacterProperties::default();
            for span in projection.style_spans_for_region(&(at..at + 1)) {
                if let StyleApplication::Direct(value) = span.application {
                    super::super::rich_text::overlay(&mut properties, &value);
                }
            }
            properties
        };
        if direct(before) != direct(after) {
            return Err(DocumentError::VerificationFailed);
        }
    }
    Ok(())
}

struct CodeRun {
    text: Range<usize>,
    source: Range<usize>,
    opening: Range<usize>,
    closing: Range<usize>,
    existing: bool,
}

/// Adjacent closing/opening backticks would form one longer delimiter run.
/// Join only spans whose complete source boundaries touch, so hidden emphasis
/// or other intervening syntax continues to own its original text.
fn code_run_patches(
    document: &Document,
    existing: &[StyleSpan],
    added: Vec<(Range<usize>, Range<usize>)>,
) -> Result<Vec<SourcePatch>, ModelTransactionError> {
    let mut runs = added
        .into_iter()
        .map(|(text, source)| CodeRun {
            opening: source.start..source.start,
            closing: source.end..source.end,
            text,
            source,
            existing: false,
        })
        .collect::<Vec<_>>();
    if runs.is_empty() {
        return Ok(Vec::new());
    }
    for span in existing {
        let source = document
            .projection()
            .source_range(span.range.clone())
            .ok_or(DocumentError::AmbiguousProjection)?;
        let (opening, closing) = super::super::markdown_code::delimiter_ranges(document, &source)?
            .ok_or(DocumentError::AmbiguousProjection)?;
        runs.push(CodeRun {
            text: span.range.clone(),
            source,
            opening,
            closing,
            existing: true,
        });
    }
    runs.sort_by_key(|run| (run.text.start, run.text.end));
    let mut patches = Vec::new();
    let mut first = 0;
    while first < runs.len() {
        let mut end = first + 1;
        while end < runs.len()
            && runs[end - 1].text.end == runs[end].text.start
            && runs[end - 1].closing.end == runs[end].opening.start
        {
            end += 1;
        }
        let group = &runs[first..end];
        first = end;
        if group.iter().all(|run| run.existing) {
            continue;
        }
        let body = &document.text()[group[0].text.start..group.last().unwrap().text.end];
        let mut width = body
            .split(|character| character != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            + 1;
        for run in group.iter().filter(|run| run.existing) {
            let bytes = document
                .state()
                .source
                .bytes_in(run.opening.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let spelling = document
                .encoding()
                .decode_region(&bytes, run.opening.start)?
                .text;
            width = width.max(spelling.bytes().take_while(|byte| *byte == b'`').count());
        }
        let marker = "`".repeat(width);
        let padding = if body.starts_with('`')
            || body.ends_with('`')
            || body.starts_with(' ') && body.ends_with(' ') && !body.trim().is_empty()
        {
            " "
        } else {
            ""
        };
        for (range, syntax) in [
            (group[0].opening.clone(), format!("{marker}{padding}")),
            (
                group.last().unwrap().closing.clone(),
                format!("{padding}{marker}"),
            ),
        ] {
            let replacement = document.encoding().encode_fragment(&syntax)?;
            if document.state().source.bytes_in(range.clone()).as_deref()
                != Some(replacement.as_slice())
            {
                patches.push(SourcePatch::primary(range, replacement));
            }
        }
        // A retained literal backtick beside a newly authored delimiter would
        // merge with its run and make the new Code syntax visible. Protect
        // only that adjacent literal contributor; existing Code runs above
        // have already been joined with their own delimiter widths.
        for edge in [group[0].text.start, group.last().unwrap().text.end] {
            let adjacent = if edge == group[0].text.start {
                document.previous_grapheme_boundary(edge).map(|start| start..edge)
            } else { document.next_grapheme_boundary(edge).map(|end| edge..end) };
            if let Some(adjacent) = adjacent.filter(|range| &document.text()[range.clone()] == "`") {
                if let Some(source) = document.projection().source_range(adjacent) {
                    if document.state().source.bytes_in(source.clone()) == Some(document.encoding().encode_fragment("`")?) {
                        patches.push(SourcePatch::primary(source, document.encoding().encode_fragment("\\`")?));
                    }
                }
            }
        }
        for pair in group.windows(2) {
            for range in [&pair[0].closing, &pair[1].opening] {
                if !range.is_empty() {
                    patches.push(SourcePatch::primary(range.clone(), Vec::new()));
                }
            }
        }
        for run in group.iter().filter(|run| !run.existing) {
            let body = &document.text()[run.text.clone()];
            let raw = document
                .state()
                .source
                .bytes_in(run.source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            if document
                .encoding()
                .decode_region(&raw, run.source.start)?
                .text
                != body
            {
                patches.push(SourcePatch::primary(
                    run.source.clone(),
                    document.encoding().encode_fragment(body)?,
                ));
            }
        }
    }
    Ok(patches)
}

impl Document {
    /// A user style choice replaces inline formatting as one source transaction.
    /// The lower-level persisted assignment remains available to translators.
    pub(super) fn prepare_character_style_choice(
        &self,
        mut range: Range<usize>,
        style: StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        self.validate_typing_named_style(&style)?;
        if range.is_empty() {
            return Ok(self.no_op_prepared());
        }

        if self.format().is_source_view() {
            // Source syntax participates in source-view style spans, while
            // clearing a named choice operates on its semantic body. Resolve
            // those bodies through the shared source artifact, then publish
            // the verified local patches in the original Source projection.
            let source = self.projection().source_range(range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let blocks = self.projection().blocks_for_region(&range);
            let isolated = blocks.as_slice().first().filter(|block| {
                blocks.len() == 1 && block.range.start <= range.start && range.end <= block.range.end
                    && block.quote_depth == 0 && block.containers.is_empty() && !block.markdown_html
                    && block.style.0 != "Code Block"
                    && self.projection().table_cell_at(range.start).is_none()
            }).and_then(|block| self.projection().source_range(block.range.clone()));
            let (semantic, base) = if let Some(owner) = isolated {
                // Ordinary prose and headings are independent inline owners.
                // Most typing choices need only this small source region,
                // even when they occur near the end of a large document.
                let mut local = Document::from_bytes_with_file_format(
                    self.source_bytes_in(owner.clone())?, self.encoding(), Format::Markdown, self.file_format())?;
                let mut state = local.state().clone();
                state.projection.install_configuration_styles(state.revision,
                    self.projection().style_sheet().clone(), self.projection().document_style().clone());
                local.history.initialize_projection(state);
                local.configuration = self.configuration.clone();
                local.next_revision = local.next_revision.max(self.next_revision);
                (local, owner.start)
            } else {
                // Structural owners can depend on enclosing table/list/HTML
                // context. Preserve that context when selections cross it.
                let mut semantic = self.scratch_document();
                let view = semantic.prepare_view_format_change(Format::Markdown)?;
                semantic.commit_model_transaction(view)?;
                (semantic, 0)
            };
            let first = semantic.visible_point_for_source(source.start - base, true)?;
            let last = semantic.visible_point_for_source(source.end - base, false)?;
            if first < last {
                let prepared = semantic.prepare_character_style_choice(first..last, style)?;
                let patches = prepared.summary.source_patches.into_iter().map(|patch|
                    SourcePatch::primary(base + patch.range.start..base + patch.range.end, patch.replacement)).collect();
                return self.prepare_visible_source_patches_with_policy(patches, true);
            }
            // A selection containing only exposed delimiters has no semantic
            // character assignment. Code still accepts that literal source
            // text, exactly as authoring its surrounding backticks would.
            if style.0 == "Code" {
                let selected = self.projection().text_tree().slice(range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                let marker = "`".repeat(selected.split(|ch| ch != '`').map(str::len).max().unwrap_or(0) + 1);
                let padding = if selected.starts_with('`') || selected.ends_with('`') { " " } else { "" };
                return self.prepare_visible_source_patches_with_policy(vec![
                    SourcePatch::primary(source.start..source.start,
                        self.encoding().encode_fragment(&format!("{marker}{padding}"))?),
                    SourcePatch::primary(source.end..source.end,
                        self.encoding().encode_fragment(&format!("{padding}{marker}"))?),
                ], true);
            }
            return Ok(self.no_op_prepared());
        }

        let original_range = range.clone();
        let mut scratch = self.scratch_document();
        let mut sources = super::replacement::PatchComposition::new(self.source_byte_len());
        let mut formatted =
            super::replacement::PatchComposition::new(self.projection().text_tree().byte_len());
        let publish = |scratch: &mut Document,
                       prepared: PreparedModelTransaction,
                       range: &mut Range<usize>,
                       sources: &mut super::replacement::PatchComposition,
                       formatted: &mut super::replacement::PatchComposition|
         -> Result<(), ModelTransactionError> {
            let mapped = |at, association| -> Result<usize, ModelTransactionError> {
                Ok(prepared
                    .text_position_map()
                    .map_text_point(
                        scratch.text_point(at)?,
                        association,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )?
                    .value()
                    .ok_or(DocumentError::AmbiguousProjection)?
                    .offset())
            };
            *range = mapped(range.start, Association::AfterInsertion)?
                ..mapped(range.end, Association::BeforeInsertion)?;
            for patch in prepared.summary.source_patches.iter().rev() {
                sources.splice(patch.range(), patch.replacement());
            }
            formatted.record_formatted(&prepared)?;
            scratch.commit_model_transaction(prepared)?;
            Ok(())
        };
        if self.format().is_markdown() {
            let mut retained_traits = Vec::new();
            for trait_style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
                let mut remaining = scratch.projection().style_spans_for_region(&range)
                    .iter().filter(|span| span.application == StyleApplication::Semantic(trait_style))
                    .count();
                loop {
                    let Some(span) = scratch
                        .projection()
                        .style_spans_for_region(&range)
                        .into_iter()
                        .find(|span| {
                            span.application == StyleApplication::Semantic(trait_style)
                                && span.range.start < range.end
                                && range.start < span.range.end
                        })
                    else {
                        break;
                    };
                    if remaining == 0 {
                        return Err(DocumentError::UnsupportedFormatting.into());
                    }
                    remaining -= 1;
                    let selected = range.start.max(span.range.start)..range.end.min(span.range.end);
                    let clear = if self.format() == Format::Markdown {
                        span.range.clone()
                    } else {
                        selected.clone()
                    };
                    let prepared =
                        scratch.prepare_typing_markdown_style(clear, trait_style, false)?;
                    if prepared.summary.source_patches.is_empty() {
                        return Err(DocumentError::UnsupportedFormatting.into());
                    }
                    publish(
                        &mut scratch,
                        prepared,
                        &mut range,
                        &mut sources,
                        &mut formatted,
                    )?;
                    if self.format() == Format::Markdown {
                        for retained in [
                            span.range.start..selected.start,
                            selected.end..span.range.end,
                        ] {
                            if retained.is_empty() {
                                continue;
                            }
                            retained_traits.push((retained, trait_style));
                        }
                    }
                }
            }
            // Restore retained flanks after removing every overlapping trait;
            // intermediate mixed runs of '*' otherwise have ambiguous nesting.
            for (retained, trait_style) in retained_traits {
                let prepared =
                    scratch.prepare_typing_markdown_style(retained, trait_style, true)?;
                publish(
                    &mut scratch,
                    prepared,
                    &mut range,
                    &mut sources,
                    &mut formatted,
                )?;
            }
        }
        if self.format().is_markdown() {
            let cleared = scratch.prepare_typing_markdown_application(range.clone(), None, false)?;
            publish(&mut scratch, cleared, &mut range, &mut sources, &mut formatted)?;
            for property in [StyleProperty::CharacterUnderline, StyleProperty::CharacterSuperscript,
                StyleProperty::CharacterSubscript]
            {
                let cleared = scratch.prepare_inline_property(range.clone(), property, false)?;
                publish(&mut scratch, cleared, &mut range, &mut sources, &mut formatted)?;
            }
        }
        let target = TextRange::new(
            scratch.text_point(range.start)?,
            scratch.text_point(range.end)?,
        )?;
        let intent = PersistedStyleIntent::AssignCharacterStyle {
            range: target,
            style: style.clone(),
        };
        let prepared = { scratch.prepare_persisted_style_intent(intent)? };
        publish(
            &mut scratch,
            prepared,
            &mut range,
            &mut sources,
            &mut formatted,
        )?;
        if self.format().is_wysiwyg() {
            // Check the final combination, including paragraph inheritance and
            // all unselected formatting, before publishing any source patches.
            let mut boundaries = BTreeSet::from([0, original_range.start, original_range.end]);
            for projection in [self.projection(), scratch.projection()] {
                for span in projection.style_spans() {
                    boundaries.extend([span.range.start, span.range.end]);
                }
                for block in projection.blocks() {
                    boundaries.extend([block.range.start, block.range.end]);
                }
            }
            for at in boundaries
                .into_iter()
                .filter(|at| *at < self.text().len() && self.text().as_bytes()[*at] != b'\n')
            {
                let code_paragraph = self.format().is_markdown()
                    && self
                        .projection()
                        .blocks_for_region(&(at..at + 1))
                        .iter()
                        .any(|block| block.style.0 == "Code Block" && block.range.contains(&at));
                let expected = if original_range.contains(&at) && !code_paragraph {
                    self.clean_named_character_at(at, &style, &CharacterProperties::default())?
                } else {
                    crate::layout::DocumentLayoutStyles::semantic_character_at(
                        self.projection(),
                        at,
                        false,
                    )
                    .map_err(|_| DocumentError::VerificationFailed)?
                };
                let actual = crate::layout::DocumentLayoutStyles::semantic_character_at(
                    scratch.projection(),
                    at,
                    false,
                )
                .map_err(|_| DocumentError::VerificationFailed)?;
                if expected != actual {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        let patches = sources.source_patches();
        let edits = formatted.formatted_edits();
        self.prepare_text_edits_with_patches(edits, Some(patches))
    }

    pub(super) fn prepare_markdown_named_character(
        &self,
        range: Range<usize>,
        style: &StyleId,
    ) -> Result<PreparedModelTransaction, ModelTransactionError> {
        self.validate_range(&range)?;
        self.validate_typing_named_style(style)?;
        if range.is_empty() {
            return Ok(self.no_op_prepared());
        }
        let code = style.0 == "Code";
        if self.format() == Format::MarkdownSource {
            if !code
                && !self
                    .projection()
                    .style_spans_for_region(&range)
                    .iter()
                    .any(|span| {
                        span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                    })
            {
                return Ok(self.no_op_prepared());
            }
            return self.prepare_semantic_style(range, SemanticInlineStyle::Code, code);
        }
        let mut patches = Vec::new();
        for (fragment, block_code) in fragments(self, &range) {
            if block_code {
                // Fences assign the paragraph's Code Block style. There is no
                // named character assignment to remove from their body.
                continue;
            }
            let context = fragment.start.saturating_sub(1)
                ..fragment.end.saturating_add(1).min(self.text().len());
            let code_spans = self
                .projection()
                .style_spans_for_region(&context)
                .into_iter()
                .filter(|span| {
                    span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                })
                .collect::<Vec<_>>();
            if !code {
                for span in code_spans {
                    let selected =
                        span.range.start.max(fragment.start)..span.range.end.min(fragment.end);
                    if selected.is_empty() {
                        continue;
                    }
                    for (source, syntax) in
                        super::super::markdown_code::clear_patches(self, &span.range, &selected)?
                    {
                        patches.push(SourcePatch::primary(
                            source,
                            self.encoding().encode_fragment(&syntax)?,
                        ));
                    }
                }
                continue;
            }
            if self.projection().blocks_for_region(&fragment).iter().any(|block| block.markdown_html) {
                let source = self.projection().source_range(fragment.clone()).ok_or(DocumentError::AmbiguousProjection)?;
                patches.push(SourcePatch::primary(source.start..source.start,
                    self.encoding().encode_fragment("<code>")?));
                patches.push(SourcePatch::primary(source.end..source.end,
                    self.encoding().encode_fragment("</code>")?));
                continue;
            }
            // Split at hidden inline syntax as well as structural boundaries.
            // This retains existing emphasis delimiters around each code span.
            let mut runs: Vec<(Range<usize>, Range<usize>)> = Vec::new();
            for span in self
                .projection()
                .provenance_for_region(&fragment)
                .into_iter()
                .filter(|span| !span.formatted.is_empty())
            {
                if code_spans.iter().any(|code| {
                    code.range.start <= span.formatted.start && span.formatted.end <= code.range.end
                }) {
                    continue;
                }
                if span.source.is_empty() {
                    return Err(DocumentError::AmbiguousProjection.into());
                }
                if let Some((text, source)) = runs.last_mut().filter(|(text, source)| {
                    text.end == span.formatted.start && source.end == span.source.start
                }) {
                    text.end = span.formatted.end;
                    source.end = span.source.end;
                } else {
                    runs.push((span.formatted, span.source));
                }
            }
            patches.extend(code_run_patches(self, &code_spans, runs)?);
        }
        if patches.is_empty() {
            return Ok(self.no_op_prepared());
        }
        super::markdown_block_styles::preserve_retained_literals(self,
            &[TextEdit::new(range.clone(), self.text()[range.clone()].to_owned())], &mut patches)?;
        let prepared = self.prepare_source_only_patches(patches)?;
        if let PreparedPublication::State(candidate) = &prepared.publication {
            if !candidate
                .projection
                .has_same_hard_line_structure(self.projection())
            {
                return Err(DocumentError::VerificationFailed.into());
            }
            verify_assignment(self.projection(), &candidate.projection, &range, style)?;
        }
        Ok(prepared)
    }
}
