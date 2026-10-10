//! Preserve unselected visible reference content when an edited definition
//! stops resolving. Definition edits have an explicit global grammar dependency.
use super::*;
use pulldown_cmark::{BrokenLink, Event, LinkType, Options, Parser, RefDefs, Tag};

#[derive(Default)]
struct ReferenceFragment {
    reference: bool,
    image_title: Option<String>,
    rendered: String,
    html: bool,
}

/// Parse only the affected label against an already parsed definition index.
/// Appending the entire definition corpus for every dependent is quadratic.
fn reference_fragment(spelling: &str, definitions: &RefDefs<'_>) -> ReferenceFragment {
    let context = format!("x {spelling}");
    let mut resolve = |link: BrokenLink<'_>| {
        definitions.get(&link.reference).map(|definition| {
            (
                definition.dest.clone(),
                definition.title.clone().unwrap_or_else(|| "".into()),
            )
        })
    };
    let mut result = ReferenceFragment::default();
    for (event, range) in Parser::new_with_broken_link_callback(
        &context,
        Options::ENABLE_STRIKETHROUGH,
        Some(&mut resolve),
    )
    .into_offset_iter()
    {
        let whole = range == (2..context.len());
        match event {
            Event::Start(Tag::Link { link_type, .. })
                if whole
                    && !matches!(
                        link_type,
                        LinkType::Inline | LinkType::Autolink | LinkType::Email
                    ) =>
            {
                result.reference = true;
            }
            Event::Start(Tag::Image { title, .. }) if whole => {
                result.image_title = Some(title.into_string());
            }
            Event::Text(text) | Event::Code(text) => {
                result.rendered.push_str(if range.start == 0 {
                    text.strip_prefix("x ").unwrap_or(&text)
                } else {
                    &text
                })
            }
            Event::SoftBreak | Event::HardBreak => result.rendered.push(' '),
            Event::InlineHtml(_) | Event::Html(_) => result.html = true,
            _ => {}
        }
    }
    result
}

impl Document {
    pub(super) fn preserve_edited_reference_content(
        &self,
        edits: &[TextEdit],
        patches: &mut Vec<SourcePatch>,
    ) -> Result<bool, ModelTransactionError> {
        if self.format() != Format::Markdown || !self.markdown_edit_needs_reference_context(edits) {
            return Ok(false);
        }
        let input = normalize(
            &self.encoding().decode(&self.source_bytes())?,
            self.file_format(),
        );
        let (_, definitions) = super::super::markdown_syntax::definitions(&input.text);
        let mapper = super::super::rich_text::Builder::new(&input, self.revision());
        let literal_definition_edits = edits.iter().all(|edit| {
            !edit.replacement.contains('\n')
                && !self.text()[edit.range.clone()].contains('\n')
                && (if edit.range.is_empty() {
                    super::super::source_edit::insertion_point(
                        self.projection(),
                        edit.range.start,
                        None,
                    )
                    .map(|at| at..at)
                } else {
                    self.projection().source_range(edit.range.clone())
                })
                .is_some_and(|source| {
                    definitions.iter().any(|definition| {
                        let definition = mapper.source_range(definition.clone());
                        definition.start <= source.start && source.end <= definition.end
                    })
                })
        });
        let mut support = Vec::new();
        for definition in definitions {
            let source = mapper.source_range(definition.clone());
            if !patches
                .iter()
                .any(|patch| source.start <= patch.range.start && patch.range.end <= source.end)
            {
                continue;
            }
            // Whitespace authored beside a folded definition ending must add
            // a visible space, rather than merge into its source indentation.
            // Retire just this definition and protect the new whitespace; its
            // untouched continuation endings can keep folding as prose.
            let force_prose = edits.iter().any(|edit| {
                edit.range.is_empty() && edit.replacement.chars().any(|ch| matches!(ch, ' ' | '\t'))
                    && self.projection().source_insertion_point(edit.range.start, true)
                        .is_some_and(|at| source.start <= at && at <= source.end)
                    && (self.projection().source_insertion_point(edit.range.start, true) == Some(source.start)
                        || self.projection().provenance_touching(&edit.range).iter().any(|span| {
                            source.start <= span.source.start && span.source.end <= source.end
                                && self.text().get(span.formatted.clone()) == Some(" ")
                                && self.state().source.bytes_in(span.source.clone())
                                    .and_then(|bytes| self.encoding().decode_region(&bytes, span.source.start).ok())
                                    .is_some_and(|decoded| decoded.text.contains(['\n', '\r']))
                        }))
            });
            if force_prose {
                for patch in patches.iter_mut().filter(|patch| patch.range.is_empty()
                    && source.start <= patch.range.start && patch.range.start <= source.end) {
                    let replacement = self.encoding().decode_region(&patch.replacement, patch.range.start)?.text;
                    if replacement.chars().all(|ch| matches!(ch, ' ' | '\t')) {
                        patch.replacement = self.encoding().encode_fragment(&replacement.chars()
                            .map(|ch| if ch == ' ' { "&#32;" } else { "&#9;" }).collect::<String>())?;
                    }
                }
            }
            let mut bytes = self
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let mut local: Vec<_> = patches
                .iter()
                .filter(|patch| source.start <= patch.range.start && patch.range.end <= source.end)
                .collect();
            local.sort_by_key(|patch| (patch.range.start, patch.range.end));
            for patch in local.into_iter().rev() {
                bytes.splice(
                    patch.range.start - source.start..patch.range.end - source.start,
                    patch.replacement.clone(),
                );
            }
            let candidate = normalize(
                &self.encoding().decode_region(&bytes, source.start)?,
                self.file_format(),
            );
            let (_, retained_definitions) =
                super::super::markdown_syntax::definitions(&candidate.text);
            let structural_prefix = candidate.text.lines().any(|line| {
                let body = line.trim_start_matches([' ', '\t']);
                body.starts_with("* ")
                    || body.starts_with("*\t")
                    || body.starts_with("- ")
                    || body.starts_with("-\t")
                    || body.starts_with("+ ")
                    || body.starts_with("+\t")
            });
            if !force_prose && !structural_prefix
                && retained_definitions.iter().any(|range| {
                    candidate.text[..range.start]
                        .chars()
                        .all(|ch| matches!(ch, ' ' | '\t'))
                        && range.end >= candidate.text.trim_end_matches('\n').len()
                })
            {
                continue;
            }
            if force_prose || !retained_definitions.is_empty() || structural_prefix {
                // A shortened definition would expose its previously literal
                // destination/title as interpreted prose. Retire that partial
                // definition with one escape, keeping its literal content.
                if let Some(offset) = input.text[definition.clone()].find('[') {
                    let opener = mapper
                        .source_range(definition.start + offset..definition.start + offset + 1);
                    if !patches.iter().any(|patch| {
                        patch.range.start < opener.end && opener.start < patch.range.end
                    }) {
                        support.push(SourcePatch::primary(
                            opener, self.encoding().encode_fragment("\\[")?,
                        ));
                    }
                }
            }
            // Once definition syntax retires, authored row-leading whitespace
            // and markers have ordinary prose rules. Protect only the new
            // scalar spelling; retained source and malformed bytes stay put.
            for patch in patches.iter_mut().filter(|patch| {
                patch.range.is_empty()
                    && source.start <= patch.range.start
                    && patch.range.end <= source.end
            }) {
                let before = self
                    .state()
                    .source
                    .bytes_in(source.start..patch.range.start)
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let before = self.encoding().decode_region(&before, source.start)?;
                let before = normalize(&before, self.file_format());
                if !before
                    .text
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .all(|ch| matches!(ch, ' ' | '\t'))
                {
                    continue;
                }
                let replacement = self
                    .encoding()
                    .decode_region(&patch.replacement, patch.range.start)?
                    .text;
                let mut protected = String::new();
                for ch in replacement.chars() {
                    match ch {
                        ' ' => protected.push_str("&#32;"),
                        '\t' => protected.push_str("&#9;"),
                        '*' | '-' | '+' => {
                            protected.push('\\');
                            protected.push(ch);
                        }
                        ch => protected.push(ch),
                    }
                }
                patch.replacement = self.encoding().encode_fragment(&protected)?;
            }

        }
        patches.extend(support);
        Ok(literal_definition_edits)
    }

    pub(super) fn repair_markdown_reference_dependents(
        &self,
        edits: &[TextEdit],
        patches: &mut Vec<SourcePatch>,
    ) -> Result<(), ModelTransactionError> {
        if self.format() != Format::Markdown || !self.markdown_edit_needs_reference_context(edits) {
            return Ok(());
        }
        let old = normalize(
            &self.encoding().decode(&self.source_bytes())?,
            self.file_format(),
        );
        let old_parser = Parser::new(&old.text);
        let old_definitions = old_parser.reference_definitions();
        let mapper = super::super::rich_text::Builder::new(&old, self.revision());
        if !old_definitions.iter().any(|(_, definition)| {
            let source = mapper.source_range(definition.span.clone());
            patches
                .iter()
                .any(|patch| patch.range.start <= source.end && source.start <= patch.range.end)
        }) {
            return Ok(());
        }
        let mut ordered = patches.clone();
        validate_source_patches(&mut ordered)?;
        let candidate = apply_source_patches(&self.state().source, &ordered)?;
        let decoded = self.encoding().decode(&candidate.bytes())?;
        let candidate = normalize(&decoded, self.file_format());
        let candidate_parser = Parser::new(&candidate.text);
        let definitions = candidate_parser.reference_definitions();
        let untouched = |range: &Range<usize>| {
            !ordered.iter().any(|patch| {
                patch.range.start < range.end && range.start < patch.range.end
                    || patch.range.is_empty()
                        && range.start < patch.range.start
                        && patch.range.start < range.end
            })
        };
        let mut support = Vec::new();
        for span in self.projection().style_spans().iter().filter(|span| {
            span.application == StyleApplication::Automatic("Markdown reference".into())
        }) {
            let Some(source) = self.projection().source_range(span.range.clone()) else {
                continue;
            };
            if !untouched(&source) {
                continue;
            }
            let bytes = self
                .state()
                .source
                .bytes_in(source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let spelling = self.encoding().decode_region(&bytes, source.start)?.text;
            if !reference_fragment(&spelling, old_definitions).reference {
                continue;
            }
            let current = reference_fragment(&spelling, definitions);
            if current.reference {
                continue;
            }
            let visible = self
                .projection()
                .text_tree()
                .slice(span.range.clone())
                .map_err(DocumentError::FormattedTextStorage)?;
            if current.rendered != visible || current.html {
                // Insert protection beside the original punctuation. Never
                // re-encode the surrounding label or its malformed bytes.
                let decoded = self.encoding().decode_region(&bytes, source.start)?;
                for scalar in decoded.scalar_spans() {
                    let text = &decoded.text[scalar.decoded];
                    if scalar.diagnostic.is_none() && escape_markdown_insert(text) != text {
                        support.push(SourcePatch::primary(
                            scalar.source.start..scalar.source.start,
                            self.encoding().encode_fragment("\\")?,
                        ));
                    }
                }
            }
        }
        for image in self
            .projection()
            .inline_images_for_region(&(0..self.projection().text_tree().byte_len()))
        {
            if image.inline || image.html || !untouched(&image.source) {
                continue;
            }
            let bytes = self
                .state()
                .source
                .bytes_in(image.source.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let decoded = self.encoding().decode_region(&bytes, image.source.start)?;
            let spelling = &decoded.text;
            if reference_fragment(spelling, definitions)
                .image_title
                .is_some()
            {
                continue;
            }
            // Keep the original object and title when its definition disappears.
            let title = reference_fragment(spelling, old_definitions)
                .image_title
                .unwrap_or_default();
            let title = if title.is_empty() {
                String::new()
            } else {
                format!(
                    " \"{}\"",
                    title
                        .replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('\\', "\\\\")
                )
            };
            let label_end = super::super::links::markdown_label_end(spelling, 1, spelling.len())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let suffix = decoded
                .source_boundary(label_end + 1)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let syntax = format!(
                "(<{}>{title})",
                super::super::links::escape_destination(&image.destination)
            );
            support.push(SourcePatch::primary(
                suffix..image.source.end,
                self.encoding().encode_fragment(&syntax)?,
            ));
        }
        patches.extend(support);
        Ok(())
    }
}
