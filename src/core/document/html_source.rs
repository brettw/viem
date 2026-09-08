//! A source-visible HTML projection. Bytes remain in source order; semantic
//! character context is borrowed from the recovered passive HTML projection.
use super::html::{self, TokenKind};
use super::line_endings::NormalizedText;
use super::*;
use std::collections::BTreeSet;

fn source_range(input: &NormalizedText, raw: &Range<usize>) -> Option<Range<usize>> {
    let first = input
        .units
        .partition_point(|unit| unit.source.end <= raw.start);
    let last = input
        .units
        .partition_point(|unit| unit.source.start < raw.end);
    (first < last)
        .then(|| input.units[first].normalized.start..input.units[last - 1].normalized.end)
}

pub(super) fn syntax_spans(text: &str) -> Vec<StyleSpan> {
    let mut spans = Vec::new();
    fn add(spans: &mut Vec<StyleSpan>, range: Range<usize>, name: &str) {
        if !range.is_empty() {
            spans.push(StyleSpan {
                range,
                application: StyleApplication::Automatic(StyleId(format!("* HTML {name}"))),
            });
        }
    }
    let bytes = text.as_bytes();
    let mut raw_next = false;
    for token in html::tokenize(text) {
        let raw_body = raw_next;
        raw_next = matches!(&token.kind, TokenKind::Tag(tag) if !tag.end && matches!(tag.name.as_str(),"script"|"style"|"title"|"textarea"|"xmp"|"iframe"|"noembed"|"noframes"));
        match token.kind {
            TokenKind::Tag(_) => {
                spans.push(StyleSpan {
                    range: token.range.clone(),
                    application: StyleApplication::SourceSyntax,
                });
                let mut at = token.range.start + 1;
                if bytes.get(at) == Some(&b'/') {
                    at += 1;
                }
                add(&mut spans, token.range.start..at, "Brackets");
                let begin = at;
                while at < token.range.end
                    && !bytes[at].is_ascii_whitespace()
                    && !matches!(bytes[at], b'/' | b'>')
                {
                    at += 1;
                }
                add(&mut spans, begin..at, "Tag name");
                while at < token.range.end {
                    if bytes[at].is_ascii_whitespace() {
                        at += 1;
                        continue;
                    }
                    if matches!(bytes[at], b'/' | b'>') {
                        add(&mut spans, at..at + 1, "Brackets");
                        at += 1;
                        continue;
                    }
                    let begin = at;
                    while at < token.range.end
                        && !bytes[at].is_ascii_whitespace()
                        && !matches!(bytes[at], b'=' | b'>')
                    {
                        at += 1;
                    }
                    add(&mut spans, begin..at, "Attribute key");
                    while at < token.range.end && bytes[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    if bytes.get(at) != Some(&b'=') {
                        continue;
                    }
                    add(&mut spans, at..at + 1, "Equals");
                    at += 1;
                    while at < token.range.end && bytes[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    let begin = at;
                    let quote = bytes.get(at).copied().filter(|c| matches!(c, b'\'' | b'"'));
                    if quote.is_some() {
                        at += 1;
                    }
                    while at < token.range.end
                        && match quote {
                            Some(q) => bytes[at] != q,
                            None => !bytes[at].is_ascii_whitespace() && bytes[at] != b'>',
                        }
                    {
                        at += 1;
                    }
                    if quote.is_some() && at < token.range.end {
                        at += 1;
                    }
                    add(&mut spans, begin..at, "Attribute value");
                }
            }
            TokenKind::Opaque => {
                if raw_body {
                    spans.push(StyleSpan {
                        range: token.range.clone(),
                        application: StyleApplication::SourceRawText,
                    });
                }
                spans.push(StyleSpan {
                    range: token.range.clone(),
                    application: StyleApplication::SourceSyntax,
                });
                add(&mut spans, token.range, "Uninterpreted");
            }
            TokenKind::Text | TokenKind::MappedText { .. } => {
                let mut at = token.range.start;
                while at < token.range.end {
                    if bytes[at] == b'&' {
                        if let Some((_, consumed)) =
                            html::reference(&text[at..token.range.end], false)
                        {
                            let end = at + consumed;
                            spans.push(StyleSpan {
                                range: at..end,
                                application: StyleApplication::SourceSyntax,
                            });
                            add(&mut spans, at..end, "Entity");
                            at = end;
                            continue;
                        }
                    }
                    at += 1;
                }
            }
        }
    }
    spans
}

pub(super) fn project(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
) -> FormattedDocument {
    project_with_configuration(input, revision, start, end, None)
}

pub(super) fn project_with_configuration(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
    configuration: Option<&StyleSheet>,
) -> FormattedDocument {
    let recovered = super::html5_tree::tokens(&input.text);
    let mut pre_ranges = Vec::new();
    let mut open_pre = Vec::new();
    let mut hidden_stack = Vec::new();
    for token in &recovered {
        if let TokenKind::Tag(tag) = &token.kind {
            if tag.end {
                let hidden = hidden_stack.pop().unwrap_or(false);
                if tag.name == "pre" && !hidden {
                    if let Some(start) = open_pre.pop() {
                        if open_pre.is_empty() {
                            pre_ranges.push(start..token.range.end);
                        }
                    }
                }
            } else {
                let hidden = hidden_stack.last().copied().unwrap_or(false)
                    || html::hidden(&tag.name)
                    || html::atomic(&tag.name);
                hidden_stack.push(hidden);
                if tag.name == "pre" && !hidden {
                    open_pre.push(token.range.start);
                }
            }
        }
    }
    if let Some(start) = open_pre.first() {
        pre_ranges.push(*start..input.text.len());
    }
    // HTML5 foster parenting can reorder recovered nodes. Source paragraphs
    // always follow the authoritative source order.
    pre_ranges.sort_by_key(|range| range.start);
    let semantic = html::project_tokens_with_configuration(
        input,
        revision,
        start,
        end,
        recovered,
        configuration,
    );
    let plain = super::projection::project_plain(input, revision, start, end);
    let mut sheet = semantic.style_sheet().clone();
    sheet.install_html_source_styles();
    let mut contexts: Vec<(Range<usize>, Vec<StyleApplication>)> = Vec::new();
    for provenance in semantic.provenance() {
        if provenance.source.is_empty() || provenance.formatted.is_empty() {
            continue;
        }
        let Some(range) = source_range(input, &provenance.source) else {
            continue;
        };
        let index = semantic
            .blocks()
            .partition_point(|block| block.range.start <= provenance.formatted.start)
            .saturating_sub(1);
        let block = &semantic.blocks()[index];
        let mut applications = vec![StyleApplication::SourceParagraph {
            style: block.style.clone(),
            defaults: block.direct_default_character.clone(),
        }];
        applications.extend(
            semantic
                .style_spans_for_region(&provenance.formatted)
                .into_iter()
                .filter(|span| {
                    span.range.start <= provenance.formatted.start
                        && provenance.formatted.end <= span.range.end
                        && span.application != StyleApplication::SourcePreservedWhitespace
                })
                .map(|span| span.application),
        );
        if let Some((old, values)) = contexts
            .last_mut()
            .filter(|(old, values)| old.end == range.start && *values == applications)
        {
            old.end = range.end;
            let _ = values;
        } else {
            contexts.push((range, applications));
        }
    }
    contexts.sort_by_key(|(range, _)| range.start);
    let tokens = html::tokenize(&input.text);
    let mut boundaries = BTreeSet::from([0, input.text.len()]);
    for (range, _) in &contexts {
        boundaries.extend([range.start, range.end]);
    }
    for token in &tokens {
        boundaries.extend([token.range.start, token.range.end]);
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    let mut styles = Vec::new();
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        if range.is_empty() {
            continue;
        }
        let next = contexts.partition_point(|(context, _)| context.end <= range.start);
        let inside = contexts
            .get(next)
            .filter(|(context, _)| context.start <= range.start);
        let token = tokens.get(tokens.partition_point(|token| token.range.end <= range.start));
        let prefer_previous = token.map_or(true, |token| {
            matches!(&token.kind,TokenKind::Tag(tag)if tag.end)
                || !matches!(&token.kind, TokenKind::Tag(_))
        });
        let selected = inside.or_else(|| {
            if prefer_previous {
                next.checked_sub(1)
                    .and_then(|i| contexts.get(i))
                    .or_else(|| contexts.get(next))
            } else {
                contexts
                    .get(next)
                    .or_else(|| next.checked_sub(1).and_then(|i| contexts.get(i)))
            }
        });
        if let Some((_, applications)) = selected {
            for application in applications {
                styles.push(StyleSpan {
                    range: range.clone(),
                    application: application.clone(),
                });
            }
        }
    }
    styles.extend(syntax_spans(&input.text));
    // A physical source hard line is a cache unit. Keep every decoration
    // interval inside one such unit, including the real newline item.
    let mut line_boundaries = input
        .text
        .match_indices('\n')
        .flat_map(|(at, _)| [at, at + 1])
        .collect::<Vec<_>>();
    // Adjacent line breaks share a boundary. Splitting twice there would
    // manufacture an empty style span, which is invalid layout input.
    line_boundaries.dedup();
    let mut split = Vec::new();
    for span in styles {
        // These intervals describe a complete lexical token. Splitting them
        // at physical lines would invent editable token starts inside comments
        // or multiline attributes. Paint intervals may still split freely.
        if matches!(
            span.application,
            StyleApplication::SourceSyntax | StyleApplication::SourceRawText
        ) {
            split.push(span);
            continue;
        }
        let mut start = span.range.start;
        for &end in &line_boundaries[line_boundaries.partition_point(|at| *at <= start)
            ..line_boundaries.partition_point(|at| *at < span.range.end)]
        {
            split.push(StyleSpan {
                range: start..end,
                application: span.application.clone(),
            });
            start = end;
        }
        if start < span.range.end {
            split.push(StyleSpan {
                range: start..span.range.end,
                application: span.application,
            });
        }
    }
    let mut styles = split;
    // Hard-line separators have no painted glyph. Keeping semantic font
    // layers off them also makes adjacent source lines independent cache
    // units; literal-context markers remain for script/style input policy.
    styles.retain(|span| {
        &input.text[span.range.clone()] != "\n"
            || matches!(
                span.application,
                StyleApplication::SourceSyntax | StyleApplication::SourceRawText
            )
    });
    styles.sort_by_key(|span| span.range.start);
    let mut result = FormattedDocument::from_parts(
        revision,
        input.text.clone(),
        plain.blocks().to_vec(),
        styles,
        plain.provenance().to_vec(),
        plain.decoding_diagnostics().to_vec(),
        sheet,
        start,
        end,
    );
    // Source keeps every character and physical hard line. A pre owns one
    // paragraph across those lines, so paragraph spacing applies only at its
    // outside edges. Mixed first/last source lines remain intact cache units.
    let mut code_lines: Vec<Range<usize>> = Vec::new();
    for range in pre_ranges {
        let first = plain.hard_line_at_offset(range.start).unwrap_or(0);
        let last = plain
            .hard_line_at_offset(range.end.saturating_sub(1).max(range.start))
            .unwrap_or(first);
        let lines = first..last + 1;
        if let Some(previous) = code_lines.last_mut().filter(|old| lines.start < old.end) {
            previous.end = previous.end.max(lines.end);
        } else {
            code_lines.push(lines);
        }
    }
    if !code_lines.is_empty() {
        let mut paragraphs = Vec::new();
        let mut line = 0;
        for code in code_lines {
            paragraphs.extend_from_slice(&plain.blocks()[line..code.start]);
            let mut paragraph = plain.blocks()[code.start].clone();
            paragraph.range.end = plain.blocks()[code.end - 1].range.end;
            let quoted = result
                .style_spans_for_region(&paragraph.range)
                .iter()
                .any(|span| matches!(&span.application,
                    StyleApplication::SourceParagraph { style, .. } if style.0 == "Block quote"));
            let code_style = StyleId::from(if quoted { "Block quote" } else { "Code Block" });
            paragraph.style = if result.style_sheet().block_style(&code_style).is_some() {
                code_style
            } else {
                result.style_sheet().base_paragraph.clone()
            };
            paragraphs.push(paragraph);
            line = code.end;
        }
        paragraphs.extend_from_slice(&plain.blocks()[line..]);
        result.install_paragraph_partition(paragraphs);
    }
    let mut soft = BTreeSet::new();
    let mut prose_extents = Vec::new();
    for block in semantic.blocks() {
        if block.style.0 == "Code Block" {
            continue;
        }
        let spans = semantic.provenance_for_region(&block.range);
        if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
            if first.source.start < last.source.end {
                prose_extents.push(first.source.start..last.source.end);
            }
        }
    }
    prose_extents.sort_by_key(|range| range.start);
    let mut protected_depth = 0usize;
    let mut token_index = 0;
    for ending in &input.endings {
        while token_index < tokens.len() && tokens[token_index].range.end <= ending.normalized.start
        {
            if let TokenKind::Tag(tag) = &tokens[token_index].kind {
                if matches!(
                    tag.name.as_str(),
                    "pre" | "code" | "script" | "style" | "textarea"
                ) {
                    if tag.end {
                        protected_depth = protected_depth.saturating_sub(1);
                    } else {
                        protected_depth += 1;
                    }
                }
            }
            token_index += 1;
        }
        let token_is_prose = tokens.get(token_index).map_or(true, |token| {
            token.range.start > ending.normalized.start
                || matches!(token.kind, TokenKind::Text | TokenKind::MappedText { .. })
        });
        let extent = prose_extents.partition_point(|range| range.start <= ending.source.start);
        if protected_depth == 0
            && token_is_prose
            && extent
                .checked_sub(1)
                .and_then(|index| prose_extents.get(index))
                .is_some_and(|range| ending.source.end <= range.end)
        {
            soft.insert(ending.normalized.start);
        }
    }
    result.install_flow_ranges(super::paragraph_flow::flow_ranges(input, &soft));
    result
}

impl Document {
    pub fn html_source_prose_at(
        &self,
        offset: usize,
        affinity: BoundaryAffinity,
    ) -> Result<bool, DocumentError> {
        self.text_point(offset)?;
        if self.format() != Format::HtmlSource {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let spans = self.projection().style_spans_for_region(
            &(offset.saturating_sub(1)..(offset + 1).min(self.projection().text_tree().byte_len())),
        );
        if spans.iter().any(|span| {
            span.application == StyleApplication::SourceSyntax
                && span.range.start < offset
                && offset < span.range.end
        }) {
            return Ok(false);
        }
        if spans.iter().any(|span| {
            span.application == StyleApplication::SourceRawText && span.range.contains(&offset)
        }) {
            return Ok(false);
        }
        if affinity == BoundaryAffinity::Downstream {
            // A boundary before existing markup belongs to the adjacent prose;
            // only its interior, and raw element content, are literal syntax.
            return Ok(true);
        }
        // At the end of a delimiter the insertion is already outside it.
        if spans.iter().any(|span| {
            span.application == StyleApplication::SourceSyntax && span.range.end == offset
        }) {
            return Ok(true);
        }
        Ok(true)
    }

    /// A bounded decoded prose context for authored quote direction. `None`
    /// means no preceding prose or paragraph boundary could be established
    /// within the budget; callers should retain literal input in that case.
    /// Syntax eligibility remains the separate `html_source_prose_at` query.
    pub fn html_source_prose_prefix(
        &self,
        at: usize,
        max_bytes: usize,
    ) -> Result<Option<String>, DocumentError> {
        self.text_point(at)?;
        if self.format() != Format::HtmlSource {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let tree = self.projection().text_tree();
        let mut start = at.saturating_sub(max_bytes);
        while start < at
            && !tree
                .is_char_boundary(start)
                .map_err(DocumentError::FormattedTextStorage)?
        {
            start += 1;
        }
        let spans = self.projection().style_spans_for_region(&(start..at));
        let raw = spans
            .iter()
            .filter(|span| span.application == StyleApplication::SourceRawText)
            .map(|span| span.range.clone())
            .collect::<Vec<_>>();
        let syntax = spans
            .iter()
            .filter(|span| span.application == StyleApplication::SourceSyntax);
        let mut position = start;
        let mut output = String::new();
        let mut established = start == 0;
        for span in syntax {
            if span.range.start > position {
                output.push_str(
                    &tree
                        .slice(position..span.range.start.min(at))
                        .map_err(DocumentError::FormattedTextStorage)?,
                );
                established = true;
            }
            position = position.max(span.range.end.min(at));
            if raw
                .iter()
                .any(|range| range.start <= span.range.start && span.range.end <= range.end)
            {
                continue;
            }
            // Token metadata gives its complete extent even when our bounded
            // window starts inside it. Inspect at most a small tag-name prefix;
            // never materialize an arbitrarily long attribute or comment.
            let mut prefix_end = (span.range.start + 32).min(span.range.end);
            while prefix_end > span.range.start
                && !tree
                    .is_char_boundary(prefix_end)
                    .map_err(DocumentError::FormattedTextStorage)?
            {
                prefix_end -= 1;
            }
            let prefix = tree
                .slice(span.range.start..prefix_end)
                .map_err(DocumentError::FormattedTextStorage)?;
            if prefix.starts_with('&') {
                if span.range.len() > max_bytes || span.range.end > at {
                    continue;
                }
                let encoded = tree
                    .slice(span.range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?;
                if let Some((decoded, _)) = html::reference(&encoded, false) {
                    output.push_str(&decoded);
                    established = true;
                }
            } else if let Some(tag) = prefix.strip_prefix('<') {
                let tag = tag.strip_prefix('/').unwrap_or(tag);
                let name = tag
                    .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '/' | '>'))
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if html::block(&name) || name == "br" {
                    output.push('\n');
                    established = true;
                }
            }
        }
        if position < at {
            output.push_str(
                &tree
                    .slice(position..at)
                    .map_err(DocumentError::FormattedTextStorage)?,
            );
            established = true;
        }
        Ok(established.then_some(output))
    }

    pub fn html_source_styled_insertion(
        &self,
        at: usize,
        affinity: BoundaryAffinity,
        text: &str,
        properties: &CharacterProperties,
    ) -> Result<(String, usize), DocumentError> {
        if !self.html_source_prose_at(at, affinity)? {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let css = html::character_css(properties);
        if css.is_empty() {
            return Ok((text.to_owned(), text.len()));
        }
        let prefix = format!(
            "<span style=\"{}\">",
            css.replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
        );
        let caret = prefix.len() + text.len();
        Ok((format!("{prefix}{text}</span>"), caret))
    }
}

pub(super) struct TranslatedStyle {
    pub patches: Vec<SourcePatch>,
    pub style_sheet: StyleSheet,
}

pub(super) fn translate_style(
    document: &Document,
    intent: &PersistedStyleIntent,
) -> Result<TranslatedStyle, ModelTransactionError> {
    let mut visible = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        Format::Html,
        document.file_format(),
    )?;
    // Source and WYSIWYG are two projections of the same styled document.
    // Opening the temporary projection from bytes alone would discard saved
    // defaults and native definitions intentionally kept outside the file.
    let mut visible_state =
        visible.reproject_html_configuration(document.projection().style_sheet())?;
    visible_state.include_style_definitions_in_file = document.include_style_definitions_in_file();
    visible_state.projection.install_configuration_styles(
        visible_state.revision,
        document.projection().style_sheet().clone(),
        document.projection().document_style().clone(),
    );
    visible.history.initialize_projection(visible_state);
    visible.next_revision = document.next_revision;
    let map = |range: TextRange| -> Result<TextRange, ModelTransactionError> {
        map_range(document, &visible, range)
    };
    let target = |target: StyleBlockTarget| -> Result<StyleBlockTarget, ModelTransactionError> {
        Ok(match target {
            StyleBlockTarget::DocumentRoot => target,
            StyleBlockTarget::Paragraphs(range) => StyleBlockTarget::Paragraphs(map(range)?),
        })
    };
    let translated = match intent {
        PersistedStyleIntent::AssignBlockStyle { target: t, style } => {
            PersistedStyleIntent::AssignBlockStyle {
                target: target(*t)?,
                style: style.clone(),
            }
        }
        PersistedStyleIntent::AssignCharacterStyle { range, style } => {
            if style.is_internal() {
                return Err(DocumentError::UnsupportedFormatting.into());
            }
            PersistedStyleIntent::AssignCharacterStyle {
                range: map(*range)?,
                style: style.clone(),
            }
        }
        PersistedStyleIntent::SetDirectCharacterProperties { range, properties } => {
            PersistedStyleIntent::SetDirectCharacterProperties {
                range: map(*range)?,
                properties: properties.clone(),
            }
        }
        PersistedStyleIntent::ClearDirectCharacterProperties { range, properties } => {
            PersistedStyleIntent::ClearDirectCharacterProperties {
                range: map(*range)?,
                properties: properties.clone(),
            }
        }
        PersistedStyleIntent::SetDirectBlockProperties {
            target: t,
            properties,
        } => PersistedStyleIntent::SetDirectBlockProperties {
            target: target(*t)?,
            properties: properties.clone(),
        },
        PersistedStyleIntent::ClearDirectBlockProperties {
            target: t,
            properties,
        } => PersistedStyleIntent::ClearDirectBlockProperties {
            target: target(*t)?,
            properties: properties.clone(),
        },
        PersistedStyleIntent::EditStyleDefinition { .. } => intent.clone(),
    };
    let request = StyleModelRequest::new(
        visible.id(),
        visible.revision(),
        StyleModelIntent::Persisted(translated),
    );
    let committed = visible.apply_style_request(request)?;
    Ok(TranslatedStyle {
        patches: committed.summary().source_patches().to_vec(),
        style_sheet: visible.projection().style_sheet().clone(),
    })
}

/// A strict restart checkpoint for independent source paragraphs. Stateful or
/// malformed fragments use the complete recovered HTML parser instead.
pub(super) fn independent_fragment(text: &str) -> bool {
    let mut stack = Vec::new();
    let mut saw_root = false;
    for token in html::tokenize(text) {
        match token.kind {
            TokenKind::Tag(tag) => {
                if !matches!(
                    tag.name.as_str(),
                    "p" | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "b"
                        | "strong"
                        | "i"
                        | "em"
                        | "u"
                        | "s"
                        | "strike"
                        | "sub"
                        | "sup"
                        | "code"
                        | "span"
                        | "font"
                        | "small"
                        | "big"
                        | "br"
                        | "wbr"
                ) {
                    return false;
                }
                if tag.end {
                    if stack.pop().as_ref() != Some(&tag.name) {
                        return false;
                    }
                } else if !matches!(tag.name.as_str(), "br" | "wbr") {
                    if stack.is_empty() {
                        if !matches!(
                            tag.name.as_str(),
                            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                        ) {
                            return false;
                        }
                        saw_root = true;
                    }
                    stack.push(tag.name);
                }
            }
            TokenKind::Opaque => {
                if !text[token.range].starts_with("<!--") {
                    return false;
                }
            }
            _ => {
                if stack.is_empty() && !text[token.range].trim().is_empty() {
                    return false;
                }
            }
        }
    }
    saw_root && stack.is_empty()
}

fn map_range(
    document: &Document,
    visible: &Document,
    range: TextRange,
) -> Result<TextRange, ModelTransactionError> {
    document.validate_style_text_range(range)?;
    let raw = document
        .projection()
        .source_range(range.start().offset()..range.end().offset())
        .or_else(|| {
            document
                .projection()
                .source_insertion_point(range.start().offset(), true)
                .map(|at| at..at)
        })
        .ok_or(DocumentError::AmbiguousProjection)?;
    let start = visible.visible_point_for_source(raw.start, true)?;
    let end = if range.is_empty() {
        start
    } else {
        visible.visible_point_for_source(raw.end, false)?
    };
    if end < start {
        return Err(DocumentError::AmbiguousProjection.into());
    }
    Ok(TextRange::new(
        visible.text_point(start)?,
        visible.text_point(end)?,
    )?)
}

pub(super) fn translate_list(
    document: &Document,
    range: Range<usize>,
    style: Option<ListStyle>,
) -> Result<Vec<SourcePatch>, ModelTransactionError> {
    let mut visible = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        Format::Html,
        document.file_format(),
    )?;
    let range = TextRange::new(
        document.text_point(range.start)?,
        document.text_point(range.end)?,
    )?;
    let mapped = map_range(document, &visible, range)?;
    let committed = visible.apply_model_request(ModelRequest::SetListStyle {
        document: visible.id(),
        revision: visible.revision(),
        range: mapped.start().offset()..mapped.end().offset(),
        style,
    })?;
    Ok(committed.summary().source_patches().to_vec())
}

/// Ordinary prose edits cannot alter HTML parser state when neither side
/// crosses a delimiter/reference, and some non-whitespace prose remains.
/// This permits bounded typing even inside paragraphs spanning source lines.
pub(super) fn can_inherit_literal_context(
    previous: &FormattedDocument,
    region: &Range<usize>,
    edit: &TextEdit,
) -> bool {
    if edit.replacement.contains(['<', '>', '&', '\r', '\n']) {
        return false;
    }
    let Ok(old) = previous.text_tree().slice(edit.range.clone()) else {
        return false;
    };
    if old.contains(['<', '>', '&', '\r', '\n']) {
        return false;
    }
    let spans = previous.style_spans_for_region(region);
    if spans
        .iter()
        .any(|span| span.range.start < region.start || span.range.end > region.end)
    {
        return false;
    }
    let mut left = region.start;
    let mut right = region.end;
    for span in spans
        .iter()
        .filter(|span| span.application == StyleApplication::SourceSyntax)
    {
        if span.range.end <= edit.range.start {
            left = left.max(span.range.end);
        } else if span.range.start >= edit.range.end {
            right = right.min(span.range.start);
        } else {
            return false;
        }
    }
    if left > edit.range.start || right < edit.range.end || left == right {
        return false;
    }
    let Ok(before) = previous.text_tree().slice(left..edit.range.start) else {
        return false;
    };
    let Ok(after) = previous.text_tree().slice(edit.range.end..right) else {
        return false;
    };
    // Malformed tag/reference prefixes can be ordinary projected prose, yet
    // their interpretation depends on neighboring bytes. Inserting immediately
    // after a literal '<' can change an ignored end tag into visible text and
    // alter which source newline belongs to a prose paragraph.
    if before.contains(['<', '&']) || after.contains(['<', '&']) {
        return false;
    }
    if before.trim().is_empty() && after.trim().is_empty() {
        return false;
    }
    // Source text can retain different resolved contexts around collapsed
    // whitespace. Do not transplant one style across such a real boundary.
    !spans.iter().any(|span| {
        span.range.start > edit.range.start && span.range.start < edit.range.end
            || span.range.end > edit.range.start && span.range.end < edit.range.end
    })
}

pub(super) fn inherited_literal_projection(
    previous: &FormattedDocument,
    input: &NormalizedText,
    revision: Revision,
    region: &Range<usize>,
    edit: &TextEdit,
    start: usize,
    end: usize,
) -> FormattedDocument {
    let plain = super::projection::project_plain(input, revision, start, end);
    let sampled = if edit.range.start < region.end {
        edit.range.start
    } else {
        edit.range.start.saturating_sub(1)
    };
    let syntax = previous.style_spans_for_region(&(sampled..sampled + 1));
    let sampled = if syntax.iter().any(|span| {
        span.application == StyleApplication::SourceSyntax && span.range.contains(&sampled)
    }) {
        sampled.saturating_sub(1)
    } else {
        sampled
    };
    let shift = |value: usize| value - edit.range.len() + edit.replacement.len();
    let mut styles = Vec::new();
    for span in previous.style_spans_for_region(region) {
        let inherits = span.range.contains(&sampled);
        let lower = if span.range.start >= edit.range.end {
            if edit.range.is_empty() && span.range.start == edit.range.start && inherits {
                span.range.start
            } else {
                shift(span.range.start)
            }
        } else {
            span.range.start.min(edit.range.start)
        };
        let upper =
            if span.range.end > edit.range.end || span.range.end == edit.range.end && inherits {
                shift(span.range.end)
            } else {
                span.range.end.min(edit.range.start)
            };
        if lower < upper {
            styles.push(StyleSpan {
                range: lower - region.start..upper - region.start,
                application: span.application,
            });
        }
    }
    styles.sort_by_key(|span| span.range.start);
    let mut blocks = plain.blocks().to_vec();
    if let Some(code) = previous
        .blocks_for_region(region)
        .into_iter()
        .find(|block| {
            matches!(block.style.0.as_str(), "Code Block" | "Block quote")
                && block.range.start <= region.start
                && region.end <= block.range.end
        })
    {
        for block in &mut blocks {
            block.style = code.style.clone();
            block.direct_paragraph = code.direct_paragraph.clone();
            block.direct_default_character = code.direct_default_character.clone();
        }
    }
    FormattedDocument::from_parts(
        revision,
        input.text.clone(),
        blocks,
        styles,
        plain.provenance().to_vec(),
        plain.decoding_diagnostics().to_vec(),
        previous.style_sheet().clone(),
        start,
        end,
    )
}
