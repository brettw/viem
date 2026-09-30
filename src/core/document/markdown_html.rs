//! Passive HTML embedded in Markdown. Attributes cannot install CSS, execute
//! scripts, or load resources. Comments/images intentionally retain syntax.
use super::*;
use crate::document::html::{self, TokenKind};

fn allowed(name: &str) -> bool {
    matches!(name, "a" | "abbr" | "b" | "bdi" | "bdo" | "blockquote" | "br" | "cite" | "code" | "del" | "details" | "div" | "dl" | "dt" | "dd" | "em" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "i" | "ins" | "kbd" | "li" | "mark" | "ol" | "p" | "pre" | "q" | "s" | "samp" | "small" | "span" | "strike" | "strong" | "summary" | "tt" | "u" | "ul" | "var" | "wbr")
}

impl MarkdownBuilder<'_> {
    pub(super) fn html_tag(&mut self, range: Range<usize>) {
        let tokens = html::tokenize(&self.source_text[range.clone()]);
        let Some(token) = tokens.first() else { return; };
        let TokenKind::Tag(tag) = &token.kind else { self.emit_range(range.start, range.end); return; };
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
