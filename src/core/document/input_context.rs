//! Shared prose/code context for authored input and clipboard transformations.
use super::*;

const CONTEXT_BYTES: usize = 4096;

#[derive(Clone, Copy)]
pub(super) enum QuoteContext {
    Prose,
    SourceProse(Option<char>),
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
        if self.format().is_code() { return Ok(true); }
        let projection = self.projection();
        let line = projection
            .hard_line_at_offset(at)
            .and_then(|line| projection.hard_line_range(line))
            .ok_or(DocumentError::AmbiguousProjection)?;
        let sample =
            if at > line.start && (at == line.end || affinity == BoundaryAffinity::Upstream) {
                at - 1
            } else {
                at
            };
        let spans = projection
            .style_spans_for_region(&(sample..(sample + 1).min(projection.text_tree().byte_len())));
        let mut source_paragraph = None;
        for span in &spans {
            if !(span.range.contains(&sample) || span.range.is_empty() && span.range.start == at) {
                continue;
            }
            match &span.application {
                StyleApplication::Semantic(SemanticInlineStyle::Code) => return Ok(true),
                StyleApplication::Named(id) if self.character_style_is_code(id) => return Ok(true),
                StyleApplication::SourceParagraph { style, .. } => {
                    source_paragraph = Some(self.input_style_is_code(style, false))
                }
                _ => {}
            }
        }
        if let Some(code) = source_paragraph {
            return Ok(code);
        }
        if self.format() == Format::HtmlSource {
            return self.empty_html_code_context(at);
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
            if self.format() == Format::Html
                && (block.range.is_empty()
                    || self
                        .projection()
                        .provenance_touching(&(at..at))
                        .iter()
                        .any(|span| span.formatted.is_empty() && span.source.is_empty()))
            {
                return self.empty_html_code_context(at);
            }
        }
        Ok(false)
    }

    fn empty_html_code_context(&self, at: usize) -> Result<bool, DocumentError> {
        let source_at = super::super::rich_text::text_source_range(self, &(at..at))?.start;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let offset = input
            .units
            .get(
                input
                    .units
                    .partition_point(|unit| unit.source.end <= source_at),
            )
            .map_or(input.text.len(), |unit| unit.normalized.start);
        let tokens = super::super::html::tokenize(&input.text);
        Ok(super::super::html_paragraph::stack_at(&tokens, offset).iter().any(|token| {
            matches!(&token.kind, super::super::html::TokenKind::Tag(tag) if matches!(tag.name.as_str(), "pre" | "code"))
        }))
    }

    pub fn input_prose_previous(
        &self,
        at: usize,
        _affinity: BoundaryAffinity,
    ) -> Result<Option<char>, DocumentError> {
        self.text_point(at)?;
        if self.format() == Format::HtmlSource {
            return Ok(self
                .html_source_prose_prefix(at, CONTEXT_BYTES)?
                .and_then(|text| text.chars().next_back()));
        }
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
        character: bool,
    ) -> Result<QuoteContext, DocumentError> {
        if self.format() == Format::HtmlSource {
            let spans = self
                .projection()
                .style_spans_for_region(&(at..(at + 1).min(self.text().len())));
            if character
                && spans.iter().any(|span| {
                    span.range.contains(&at)
                        && matches!(
                            span.application,
                            StyleApplication::SourceSyntax | StyleApplication::SourceRawText
                        )
                })
                || !character && !self.html_source_prose_at(at, affinity)?
            {
                return Ok(QuoteContext::Syntax);
            }
        }
        if self.is_code_at(at, affinity)? {
            return Ok(QuoteContext::Code);
        }
        if self.format() == Format::MarkdownSource
            && markdown_source_literal(&self.input_prefix(at)?)
        {
            return Ok(QuoteContext::Syntax);
        }
        Ok(if self.format() == Format::HtmlSource {
            QuoteContext::SourceProse(self.input_prose_previous(at, affinity)?)
        } else {
            QuoteContext::Prose
        })
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
        self.validate_range(&range)?;
        if self.format().is_code() || !text.contains(['\'', '"']) {
            return Ok(text.to_owned());
        }
        if self.format().is_source_view()
            && text.chars().nth(1).is_some()
        {
            let mut preview = self.scratch_document();
            if preview.replace(range.clone(), text).is_ok()
                && preview
                    .projection()
                    .text_tree()
                    .slice(range.start..range.start + text.len())
                    .ok()
                    .as_deref()
                    == Some(text)
            {
                let previous = self.input_prose_previous(range.start, affinity)?;
                return rewrite_quotes(
                    text,
                    previous,
                    |offset, _| {
                        preview.quote_context(
                            range.start + offset,
                            BoundaryAffinity::Downstream,
                            true,
                        )
                    },
                    &mut quote,
                )
                .map(|value| value.0);
            }
            let source = self
                .projection()
                .source_range(range)
                .ok_or(DocumentError::AmbiguousProjection)?;
            return self.transform_source_quote_input(source, text, true, &mut quote);
        }
        let previous = self.input_prose_previous(range.start, affinity)?;
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
        rewrite_quotes(
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
                    false,
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
        self.transform_source_quote_input(source_range, text, false, &mut quote)
    }

    fn transform_source_quote_input(
        &self,
        source_range: Range<usize>,
        text: &str,
        logical_breaks: bool,
        quote: &mut impl FnMut(char, Option<char>) -> char,
    ) -> Result<String, DocumentError> {
        if self.format().is_code() || !text.contains(['\'', '"']) {
            return Ok(text.to_owned());
        }
        let source_format = match self.format().wysiwyg() {
            Format::Html => Format::HtmlSource,
            Format::Markdown => Format::MarkdownSource,
            value => value,
        };
        let spelling = if logical_breaks {
            spell_logical_breaks(text, self.file_format())
        } else {
            text.to_owned()
        };
        let replacement = self.encoding().encode_fragment(&spelling)?;
        let source = apply_source_patches(
            &self.state().source,
            &[SourcePatch::primary(source_range.clone(), replacement)],
        )
        .map_err(compat_document_error)?;
        let decoded = self.encoding().decode(&source.bytes())?;
        let candidate = build_state_from_decoded_with_configuration(
            source,
            decoded,
            source_format,
            self.file_format(),
            self.state().file_format_origin,
            self.state().line_ending_evidence,
            self.revision(),
            Some(self.projection().style_sheet()),
        )?;
        let mut preview = self.scratch_document();
        preview.history = super::super::new_document_history(candidate);
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
                    .input_prose_previous(at, BoundaryAffinity::Downstream)
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
            let scalar = character.to_string();
            let spelling = if logical_breaks && character == '\n' {
                self.file_format().spelling()
            } else {
                &scalar
            };
            at += self.encoding().encode_fragment(spelling)?.len();
        }
        let rtf_visible = if source_format == Format::Rtf {
            preview
                .projection()
                .provenance_for_region(&(0..preview.text().len()))
                .into_iter()
                .filter(|span| !span.formatted.is_empty())
                .map(|span| (span.source.start, (span.source.end, span.formatted.start)))
                .collect::<BTreeMap<_, _>>()
        } else {
            BTreeMap::new()
        };
        rewrite_quotes(
            text,
            previous,
            |offset, grapheme| {
                let Some(source_at) = quote_sources.get(&offset).copied() else {
                    return Ok(QuoteContext::Syntax);
                };
                if source_format == Format::Rtf {
                    let source_end = source_at + self.encoding().encode_fragment(grapheme)?.len();
                    return rtf_visible
                        .get(&source_at)
                        .filter(|(end, _)| *end == source_end)
                        .map_or(Ok(QuoteContext::Syntax), |(_, at)| {
                            preview.quote_context(*at, BoundaryAffinity::Downstream, true)
                        });
                }
                preview.quote_context(formatted(source_at)?, BoundaryAffinity::Downstream, true)
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
    mut previous: Option<char>,
    mut context: impl FnMut(usize, &str) -> Result<QuoteContext, DocumentError>,
    quote: &mut impl FnMut(char, Option<char>) -> char,
) -> Result<(String, Vec<TextEdit>), DocumentError> {
    let mut output = String::with_capacity(text.len());
    let mut edits = Vec::new();
    let mut previous_quote = None;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let context = if grapheme.contains(['\'', '"']) {
            context(offset, grapheme)?
        } else {
            QuoteContext::Prose
        };
        let start = output.len();
        for character in grapheme.chars() {
            if let QuoteContext::SourceProse(value) = context {
                if matches!(character, '\'' | '"') {
                    previous = previous_quote
                        .filter(|(original, _)| Some(*original) == value)
                        .map(|(_, replacement)| replacement)
                        .or(value);
                }
            }
            let replacement = if matches!(character, '\'' | '"')
                && matches!(context, QuoteContext::Prose | QuoteContext::SourceProse(_))
            {
                quote(character, previous)
            } else {
                character
            };
            output.push(replacement);
            if !matches!(context, QuoteContext::Syntax) {
                previous = Some(replacement);
                if matches!(character, '\'' | '"') {
                    previous_quote = (replacement != character).then_some((character, replacement));
                }
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

fn markdown_source_literal(prefix: &str) -> bool {
    let line = prefix.rsplit('\n').next().unwrap_or(prefix);
    line.chars()
        .rev()
        .take_while(|value| *value == '\\')
        .count()
        % 2
        == 1
        || unfinished_code_span(line)
        || line
            .rfind("](")
            .is_some_and(|start| line.rfind(')').map_or(true, |end| start > end))
        || line.rfind('<').is_some_and(|start| {
            line.rfind('>').map_or(true, |end| start > end)
                && line[start + 1..]
                    .chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_alphabetic() || matches!(value, '/' | '!'))
        })
}

// A source caret can be inside an unfinished span before the lossless parser
// has a closing delimiter. Escapes matter only outside literal code bodies.
fn unfinished_code_span(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut at = 0;
    let mut delimiter = None;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' if delimiter.is_none() => {
                at += 2;
            }
            b'`' => {
                let start = at;
                while bytes.get(at) == Some(&b'`') {
                    at += 1;
                }
                let length = at - start;
                if delimiter == Some(length) {
                    delimiter = None;
                } else if delimiter.is_none() {
                    delimiter = Some(length);
                }
            }
            _ => at += 1,
        }
    }
    delimiter.is_some()
}
