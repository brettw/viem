//! Passive HTML embedded in Markdown. Attributes cannot install CSS, execute
//! scripts, or load resources. Comments intentionally retain syntax.
use super::*;
use crate::document::html::{self, TokenKind};

fn allowed(name: &str) -> bool {
    matches!(name, "a" | "abbr" | "b" | "bdi" | "bdo" | "blockquote" | "br" | "cite" | "code" | "del" | "details" | "div" | "dl" | "dt" | "dd" | "em" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "i" | "ins" | "kbd" | "li" | "mark" | "ol" | "p" | "pre" | "q" | "s" | "samp" | "small" | "span" | "strike" | "strong" | "summary" | "tt" | "u" | "ul" | "var" | "wbr")
}

pub(super) fn image_only_html(source: &str) -> bool {
    html::tokenize(source).into_iter().all(|token| match token.kind {
        TokenKind::Tag(tag) => allowed(&tag.name) && (!html::block(&tag.name) || matches!(tag.name.as_str(), "p" | "div")),
        TokenKind::Text => source[token.range].chars().all(|ch| ch == '\u{fffc}' || ch.is_whitespace()),
        _ => false,
    })
}

pub(super) fn contains_table(source: &str) -> bool {
    html::tokenize(source).iter().any(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "table"))
}

pub(super) fn quote_prefix_ranges(
    source_text: &str, units: &[LogicalUnit], range: &Range<usize>, quotes: &[super::super::markdown_quotes::QuoteLine],
) -> Vec<Range<usize>> {
    if range.is_empty() { return Vec::new(); }
    let first = units.partition_point(|unit| unit.normalized.end <= range.start);
    let last = units.partition_point(|unit| unit.normalized.start < range.end);
    if first == last { return Vec::new(); }
    let source_start = units[first].source.start;
    let source_end = units[last - 1].source.end;
    let normalized_at = |at| units.get(units.partition_point(|unit| unit.source.start < at))
        .map_or(source_text.len(), |unit| unit.normalized.start);
    quotes[quotes.partition_point(|quote| quote.range.end < source_start)..].iter()
        .take_while(|quote| quote.range.start < source_end)
        .filter_map(|quote| {
            let start = normalized_at(quote.range.start).max(range.start);
            let end = normalized_at(quote.content_start).min(range.end);
            (start < end).then(|| start - range.start..end - range.start)
        }).collect()
}

impl MarkdownBuilder<'_> {
    /// A quote nested inside a list can begin midway through a physical row.
    /// Its multiline tag still belongs to that exact owner even when the
    /// ordinary soft-row classification cannot join the leading owner row.
    pub(super) fn multiline_image(
        &self, start: usize, end: usize, limit: usize, quotes: &[super::super::markdown_quotes::QuoteLine],
    ) -> Option<InlineImage> {
        for (offset, _) in self.source_text[start..end].match_indices('<') {
            let first = start + offset;
            if !self.source_text[first..end].get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("<img")) { continue; }
            let source_start = self.unit_at(first)?.source.start;
            let Ok(index) = self.source_syntax.inline_html.binary_search_by_key(&source_start, |range| range.start) else { continue; };
            let source = self.source_syntax.inline_html[index].clone();
            let last = self.units.get(self.units.partition_point(|unit| unit.source.start < source.end))
                .map_or(self.source_text.len(), |unit| unit.normalized.start);
            if last <= end || last > limit { continue; }
            let excluded = quote_prefix_ranges(self.source_text, self.units, &(first..last), quotes);
            let removed = html::without_ranges(&self.source_text[first..last], &excluded);
            let tokens = html::tokenize(&removed.text);
            let Some(html::Token { kind: TokenKind::Tag(tag), .. }) = tokens.first() else { continue; };
            if let Some(image) = html::image_metadata(tag, first..last, source, self.preserve_markers) { return Some(image); }
        }
        None
    }

    pub(super) fn html_tag(&mut self, range: Range<usize>) {
        let tokens = html::tokenize(&self.source_text[range.clone()]);
        let Some(token) = tokens.first() else { return; };
        let TokenKind::Tag(tag) = &token.kind else { self.emit_range(range.start, range.end); return; };
        if let Some(name) = &self.html_raw_text {
            if tag.end && tag.name == *name { self.html_raw_text = None; }
            self.emit_range(range.start, range.end);
            return;
        }
        if !tag.end && html::raw_text(&tag.name) {
            self.html_raw_text = Some(tag.name.clone());
        }
        let output = self.output.len();
        let source = self.unit_at(range.start).unwrap().source.start..self.unit_at(range.end - 1).unwrap().source.end;
        if let Some(mut image) = html::image_metadata(tag, output..output, source.clone(), self.preserve_markers)
            .filter(|_| self.source_syntax.recognizes_inline_html(&source)) {
            if self.preserve_markers { self.emit_range(range.start, range.end); } else {
                self.output.push('\u{fffc}');
                self.provenance.push(ProvenanceSpan { formatted: output..self.output.len(), source });
            }
            image.range.end = self.output.len();
            self.inline_images.push(image);
            return;
        }
        if !allowed(&tag.name) {
            let start = self.output.len();
            self.emit_range(range.start, range.end);
            if tag.name == "img" { self.styles.push(StyleSpan { range: start..self.output.len(), application: StyleApplication::Automatic("Markdown reference".into()) }); }
            return;
        }
        if tag.end {
            if let Some(index) = self.html_stack.iter().rposition(|(name, _, _)| name == &tag.name) {
                let entries: Vec<_> = self.html_stack.drain(index..).collect();
                for (_, start, application) in entries { if start < self.output.len() { self.styles.push(StyleSpan { range: start..self.output.len(), application }); } }
            }
        } else {
            let application = match tag.name.as_str() {
                "b" | "strong" => Some(StyleApplication::Semantic(SemanticInlineStyle::Strong)),
                "i" | "em" => Some(StyleApplication::Semantic(SemanticInlineStyle::Emphasis)),
                "code" | "kbd" | "samp" | "tt" => Some(StyleApplication::Named("Code".into())),
                "del" | "s" | "strike" => Some(StyleApplication::Automatic("Strikethrough".into())),
                "a" if tag.attribute("href").is_some() => Some(StyleApplication::Automatic("Link".into())),
                "u" | "ins" => Some(StyleApplication::Direct(CharacterProperties { underline: Some(true), ..Default::default() })),
                _ => None,
            };
            if let Some(application) = application { self.html_stack.push((tag.name.clone(), self.output.len(), application)); }
        }
        if self.preserve_markers { self.emit_range(range.start, range.end); }
    }

    /// HTML blocks remain literal in Source, including physical newlines,
    /// while image actions and style queries use the complete tag interval.
    pub(super) fn source_html_images(&mut self, range: Range<usize>, output: usize, quotes: &[super::super::markdown_quotes::QuoteLine]) {
        let excluded = quote_prefix_ranges(self.source_text, self.units, &range, quotes);
        for token in html::tokenize_without_ranges(&self.source_text[range.clone()], &excluded) {
            let TokenKind::Tag(tag) = token.kind else { continue; };
            let start = range.start + token.range.start;
            let end = range.start + token.range.end;
            let source = self.unit_at(start).unwrap().source.start..self.unit_at(end - 1).unwrap().source.end;
            if let Some(image) = html::image_metadata(&tag,
                output + token.range.start..output + token.range.end, source, true) {
                self.inline_images.push(image);
            }
        }
    }
}

pub(super) fn block(input: &NormalizedText, range: Range<usize>, revision: Revision) -> FormattedDocument {
    let mut fragment = NormalizedText { text: input.text[range.clone()].to_owned(), units: Vec::new(), endings: Vec::new(), encoding: input.encoding };
    for unit in &input.units[input.units.partition_point(|u| u.normalized.start < range.start)..input.units.partition_point(|u| u.normalized.start < range.end)] {
        let mut unit = unit.clone(); unit.normalized = unit.normalized.start - range.start..unit.normalized.end - range.start; fragment.units.push(unit);
    }
    for ending in input.endings.iter().filter(|ending| range.start <= ending.normalized.start && ending.normalized.end <= range.end) {
        let mut ending = ending.clone(); ending.normalized = ending.normalized.start - range.start..ending.normalized.end - range.start; fragment.endings.push(ending);
    }
    let mut comments = Vec::new();
    let mut references = Vec::new();
    let mut tokens = html::tokenize(&fragment.text);
    for token in &mut tokens {
        match &mut token.kind {
            TokenKind::Opaque if fragment.text[token.range.clone()].starts_with("<!--") => {
                comments.push(token.range.clone());
                token.kind = TokenKind::MappedText { text: fragment.text[token.range.clone()].to_owned(), mapped: true };
            }
            TokenKind::Opaque => { token.kind = TokenKind::MappedText { text: fragment.text[token.range.clone()].to_owned(), mapped: true }; }
            TokenKind::Tag(tag) if html::image_metadata(tag, 0..0, 0..0, false).is_some() => {
                tag.attributes.retain(|(name, _)| matches!(name.as_str(), "src" | "alt" | "width" | "height"));
            }
            TokenKind::Tag(tag) if allowed(&tag.name) => {
                tag.attributes.retain(|(name, _)| matches!(name.as_str(), "href" | "title" | "start" | "value" | "dir" | "align"));
            }
            TokenKind::Tag(tag) => {
                if tag.name == "img" { references.push(token.range.clone()); }
                token.kind = TokenKind::MappedText { text: fragment.text[token.range.clone()].to_owned(), mapped: true };
            }
            _ => {}
        }
    }
    let tokens = tokens.into_iter().flat_map(|token| {
        if let TokenKind::MappedText { ref text, mapped: true } = token.kind {
            if text == &fragment.text[token.range.clone()] {
                return text.char_indices().map(|(offset, ch)| html::Token {
                    range: token.range.start + offset..token.range.start + offset + ch.len_utf8(),
                    kind: TokenKind::MappedText { text: ch.to_string(), mapped: true },
                }).collect::<Vec<_>>();
            }
        }
        vec![token]
    }).collect();
    let start = fragment.units.first().map_or(0, |unit| unit.source.start);
    let end = fragment.units.last().map_or(start, |unit| unit.source.end);
    let mut result = html::project_markdown_tokens(&fragment, revision, start, end, tokens);
    let mut styles = Vec::new();
    for (ranges, name) in [(comments, "Comment"), (references, "Markdown reference")] {
        for range in ranges {
            let first = fragment.units.partition_point(|u| u.normalized.start < range.start);
            let last = fragment.units.partition_point(|u| u.normalized.start < range.end);
            if first == last { continue; }
            let source = fragment.units[first].source.start..fragment.units[last - 1].source.end;
            for span in result.provenance().iter().filter(|span| source.start <= span.source.start && span.source.end <= source.end && !span.formatted.is_empty()) {
                styles.push(StyleSpan { range: span.formatted.clone(), application: StyleApplication::Automatic(name.into()) });
            }
        }
    }
    result.append_link_styles(styles);
    result
}
