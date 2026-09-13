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

pub(super) fn html_source_runs(
    document: &Document,
    range: &Range<usize>,
) -> Result<Vec<Range<usize>>, DocumentError> {
    let mut result = Vec::new();
    for (text, _) in fragments(document, range) {
        result.extend(super::super::rich_text::text_source_runs(document, &text)?);
    }
    Ok(result)
}

pub(super) fn verify_assignment(
    before: &FormattedDocument,
    after: &FormattedDocument,
    range: &Range<usize>,
    style: &StyleId,
    preserve_code_paragraphs: bool,
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
        let preserved_paragraph = preserve_code_paragraphs && before.blocks_for_region(&(at..at + 1))
            .iter().any(|block| block.style.0 == "Code Block" && block.range.contains(&at));
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
        let prepared = self.prepare_source_only_patches(patches)?;
        if let PreparedPublication::State(candidate) = &prepared.publication {
            if !candidate
                .projection
                .has_same_hard_line_structure(self.projection())
            {
                return Err(DocumentError::VerificationFailed.into());
            }
            verify_assignment(self.projection(), &candidate.projection, &range, style, true)?;
        }
        Ok(prepared)
    }
}
