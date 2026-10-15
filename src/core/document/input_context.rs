//! Shared prose/code context for authored input and clipboard transformations.
use super::*;

const CONTEXT_BYTES: usize = 4096;

#[derive(Clone, Copy)]
pub(super) enum QuoteContext {
    Prose,
    Code,
    Syntax,
}

impl Document {
    /// Code is a semantic style role, not a font-family heuristic.
    pub fn character_style_is_code(&self, style: &StyleId) -> bool {
        self.input_style_is_code(style, true)
    }

    fn input_style_is_code(&self, style: &StyleId, character: bool) -> bool {
        let sheet = self.projection().style_sheet();
        let expected = if character { "Code" } else { "Code Block" };
        let mut next = Some(style);
        let count = if character {
            sheet.character_style_count()
        } else {
            sheet.block_style_count()
        };
        for _ in 0..=count {
            let Some(id) = next else {
                break;
            };
            let metadata = if character {
                sheet.character_style_metadata(id)
            } else {
                sheet.block_style_metadata(id)
            };
            if id.0 == expected || metadata.is_some_and(|value| value.display_name == expected) {
                return true;
            }
            next = if character {
                sheet
                    .character_style(id)
                    .and_then(|value| value.based_on.as_ref())
            } else {
                sheet
                    .block_style(id)
                    .and_then(|value| value.based_on.as_ref())
            };
        }
        false
    }

    pub fn is_code_at(&self, at: usize, affinity: BoundaryAffinity) -> Result<bool, DocumentError> {
        self.text_point(at)?;
        if self.format().is_code() {
            return Ok(true);
        }
        let projection = self.projection();
        let line = projection
            .hard_line_at_offset(at)
            .and_then(|line| projection.hard_line_range(line))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let outside_code = affinity == BoundaryAffinity::Downstream
            && super::super::markdown_code::closing_boundary(self, at)?.is_some();
        let sample =
            if at > line.start && (!outside_code && at == line.end || affinity == BoundaryAffinity::Upstream) {
                at - 1
            } else {
                at
            };
        let spans = projection
            .style_spans_for_region(&(sample..(sample + 1).min(projection.text_tree().byte_len())));
        for span in &spans {
            if !(span.range.contains(&sample) || span.range.is_empty() && span.range.start == at) {
                continue;
            }
            match &span.application {
                StyleApplication::Semantic(SemanticInlineStyle::Code) => return Ok(true),
                StyleApplication::Named(id) if self.character_style_is_code(id) => return Ok(true),
                _ => {}
            }
        }

        if let Some(block) = super::super::edit_boundary::paragraph_at(self, at)? {
            if self.input_style_is_code(&block.style, false) {
                return Ok(true);
            }
            if self.format().is_markdown()
                && super::super::markdown_quotes::is_fenced_block(self, &block)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn input_prose_previous(&self, at: usize) -> Result<Option<char>, DocumentError> {
        self.text_point(at)?;
        Ok(self.input_prefix(at)?.chars().next_back())
    }

    fn input_prefix(&self, at: usize) -> Result<String, DocumentError> {
        let tree = self.projection().text_tree();
        let mut start = at.saturating_sub(CONTEXT_BYTES);
        while start < at
            && !tree
                .is_char_boundary(start)
                .map_err(DocumentError::FormattedTextStorage)?
        {
            start += 1;
        }
        tree.slice(start..at)
            .map_err(DocumentError::FormattedTextStorage)
    }

    pub(super) fn quote_context(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
    ) -> Result<QuoteContext, DocumentError> {
        if self.is_code_at(at, affinity)? {
            return Ok(QuoteContext::Code);
        }
        Ok(QuoteContext::Prose)
    }

    /// Transform incoming prose while preserving target code and source syntax.
    /// Replacement graphemes use their corresponding original target context.
    pub fn transform_text_input(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
        text: &str,
        mut quote: impl FnMut(char, Option<char>) -> char,
    ) -> Result<String, DocumentError> {
        self.transform_text_input_indexed(range, affinity, text, |_, character, previous| {
            quote(character, previous)
        })
    }

    /// Like `transform_text_input`, exposing each original input byte offset
    /// so an editing intention can protect explicitly literal characters.
    pub(crate) fn transform_text_input_indexed(
        &self,
        range: Range<usize>,
        affinity: BoundaryAffinity,
        text: &str,
        mut quote: impl FnMut(usize, char, Option<char>) -> char,
    ) -> Result<String, DocumentError> {
        self.validate_range(&range)?;
        if self.format().is_code() || !text.contains(['\'', '"']) {
            return Ok(text.to_owned());
        }
        let previous = self.input_prose_previous(range.start)?;
        let old = self
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        let targets = old
            .grapheme_indices(true)
            .map(|(at, _)| range.start + at)
            .collect::<Vec<_>>();
        let incoming = text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        rewrite_quotes_indexed(
            text,
            previous,
            |offset, _| {
                let index = incoming.partition_point(|at| *at < offset);
                let at = targets
                    .get(index)
                    .or_else(|| targets.last())
                    .copied()
                    .unwrap_or(range.start);
                self.quote_context(
                    at,
                    if range.is_empty() {
                        affinity
                    } else {
                        BoundaryAffinity::Downstream
                    },
                )
            },
            &mut quote,
        )
        .map(|value| value.0)
    }

    /// Physical source paste must classify syntax at its actual byte boundary.
    pub fn transform_source_input(
        &self,
        source_range: Range<usize>,
        text: &str,
        mut quote: impl FnMut(char, Option<char>) -> char,
    ) -> Result<String, DocumentError> {
        self.transform_source_quote_input(
            source_range,
            text,
            &mut |_, character, previous| quote(character, previous),
        )
    }

    fn transform_source_quote_input(
        &self,
        source_range: Range<usize>,
        text: &str,
        quote: &mut impl FnMut(usize, char, Option<char>) -> char,
    ) -> Result<String, DocumentError> {
        if self.format().is_code() || !text.contains(['\'', '"']) {
            return Ok(text.to_owned());
        }
        let replacement = self.encoding().encode_fragment(text)?;
        let source = apply_source_patches(
            &self.state().source,
            &[SourcePatch::primary(source_range.clone(), replacement)],
        )
        .map_err(compat_document_error)?;
        let decoded = self.encoding().decode(&source.bytes())?;
        let candidate = build_state_from_decoded(
            source,
            decoded,
            self.format(),
            self.file_format(),
            self.state().file_format_origin,
            self.state().line_ending_evidence,
            self.revision(),
        )?;
        let mut preview = self.scratch_document();
        preview.history = super::super::history::History::transient(candidate);
        let formatted = |source| {
            preview
                .projection()
                .map_source_boundary(preview.revision(), source, BoundaryAffinity::Downstream)
                .map(|point| point.formatted_offset)
                .map_err(|_| DocumentError::AmbiguousProjection)
        };
        let previous = formatted(source_range.start)
            .ok()
            .and_then(|at| {
                preview
                    .input_prose_previous(at)
                    .ok()
            })
            .flatten();
        // Source input retains its own line-ending bytes. Track quote positions
        // through encoding/normalization rather than assuming start + offset.
        let mut at = source_range.start;
        let mut quote_sources = BTreeMap::new();
        for (offset, character) in text.char_indices() {
            if matches!(character, '\'' | '"') {
                quote_sources.insert(offset, at);
            }
            at += self.encoding().encode_fragment(&character.to_string())?.len();
        }
        rewrite_quotes_indexed(
            text,
            previous,
            |offset, _grapheme| {
                let Some(source_at) = quote_sources.get(&offset).copied() else {
                    return Ok(QuoteContext::Syntax);
                };
                // A quote inside hidden syntax (for example a link title) has
                // no visible prose position; it stays straight.
                let Ok(at) = formatted(source_at) else {
                    return Ok(QuoteContext::Syntax);
                };
                preview.quote_context(at, BoundaryAffinity::Downstream)
            },
            quote,
        )
        .map(|value| value.0)
    }
}

/// One scalar policy, shared by input batches and rich fragment source edits.
/// Each edit covers a complete grapheme even when a quote has combining marks.
pub(super) fn rewrite_quotes(
    text: &str,
    previous: Option<char>,
    context: impl FnMut(usize, &str) -> Result<QuoteContext, DocumentError>,
    quote: &mut impl FnMut(char, Option<char>) -> char,
) -> Result<(String, Vec<TextEdit>), DocumentError> {
    rewrite_quotes_indexed(text, previous, context, &mut |_, character, previous| {
        quote(character, previous)
    })
}

fn rewrite_quotes_indexed(
    text: &str,
    mut previous: Option<char>,
    mut context: impl FnMut(usize, &str) -> Result<QuoteContext, DocumentError>,
    quote: &mut impl FnMut(usize, char, Option<char>) -> char,
) -> Result<(String, Vec<TextEdit>), DocumentError> {
    let mut output = String::with_capacity(text.len());
    let mut edits = Vec::new();
    for (offset, grapheme) in text.grapheme_indices(true) {
        let context = if grapheme.contains(['\'', '"']) {
            context(offset, grapheme)?
        } else {
            QuoteContext::Prose
        };
        let start = output.len();
        for (relative, character) in grapheme.char_indices() {
            let replacement =
                if matches!(character, '\'' | '"') && matches!(context, QuoteContext::Prose) {
                    quote(offset + relative, character, previous)
                } else {
                    character
                };
            output.push(replacement);
            if !matches!(context, QuoteContext::Syntax) {
                previous = Some(replacement);
            }
        }
        if output[start..] != *grapheme {
            edits.push(TextEdit::new(
                offset..offset + grapheme.len(),
                &output[start..],
            ));
        }
    }
    Ok((output, edits))
}

