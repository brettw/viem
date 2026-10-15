//! Passive HTML embedded in Markdown. Attributes cannot install CSS, execute
//! scripts, or load resources. Comments intentionally retain syntax.
use super::*;
use crate::document::html::{self, TokenKind};

#[cfg(test)]
thread_local! {
    static DIRECT_STYLE_SLOT_WRITES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn allowed(name: &str) -> bool {
    matches!(name, "a" | "abbr" | "b" | "bdi" | "bdo" | "blockquote" | "br" | "cite" | "code" | "del" | "details" | "div" | "dl" | "dt" | "dd" | "em" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "i" | "ins" | "kbd" | "li" | "mark" | "ol" | "p" | "pre" | "q" | "s" | "samp" | "small" | "span" | "strike" | "strong" | "summary" | "tt" | "u" | "sup" | "sub" | "ul" | "var" | "wbr")
}

/// Whether the admitted passive HTML still has a preformatted owner at this
/// boundary. Closing an ancestor retires nested scopes exactly as projection
/// does; unsupported tags remain literal and cannot change that ownership.
pub(super) fn open_pre(source: &str) -> bool {
    let mut stack = Vec::new();
    for token in html::tokenize(source) {
        let TokenKind::Tag(tag) = token.kind else { continue; };
        if !allowed(&tag.name) { continue; }
        if tag.end {
            if let Some(index) = stack.iter().rposition(|name| name == &tag.name) { stack.truncate(index); }
        } else if !html::void(&tag.name) { stack.push(tag.name); }
    }
    stack.iter().any(|name| name == "pre")
}

pub(super) fn pre_owners(source: &str, excluded: &[Range<usize>]) -> Vec<html::PreOwner> {
    let mut stack: Vec<(String, Option<usize>)> = Vec::new();
    let mut owners: Vec<html::PreOwner> = Vec::new();
    for token in html::tokenize_without_ranges(source, excluded) {
        let TokenKind::Tag(tag) = token.kind else { continue; };
        if !allowed(&tag.name) { continue; }
        if tag.end {
            if let Some(index) = stack.iter().rposition(|(name, _)| name == &tag.name) {
                for (_, owner) in &stack[index..] {
                    if let Some(owner) = owner { owners[*owner].range.end = token.range.end; }
                }
                stack.truncate(index);
            }
        } else if !html::void(&tag.name) {
            let owner = if tag.name == "pre" && !stack.iter().any(|(name, _)| name == "pre") {
                let owner = owners.len();
                owners.push(html::PreOwner { opening: token.range.clone(), range: token.range.start..source.len() });
                Some(owner)
            } else { None };
            stack.push((tag.name, owner));
        }
    }
    owners
}

pub(super) fn contains_table(source: &str) -> bool {
    html::tokenize(source).iter().any(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "table"))
}

pub(super) fn quote_prefix_ranges(
    input: &NormalizedText, range: &Range<usize>, quotes: &[super::super::markdown_quotes::QuoteLine],
) -> Vec<Range<usize>> {
    if range.is_empty() { return Vec::new(); }
    let Some(Range { start: source_start, end: source_end }) = input.source_hull(range) else { return Vec::new(); };
    let normalized_at = |at| input.normalized_at(at);
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
            let last = self.input.normalized_at(source.end);
            if last <= end || last > limit { continue; }
            let excluded = quote_prefix_ranges(self.input, &(first..last), quotes);
            let removed = html::without_ranges(&self.source_text[first..last], &excluded);
            let tokens = html::tokenize(&removed.text);
            let Some(html::Token { kind: TokenKind::Tag(tag), .. }) = tokens.first() else { continue; };
            if let Some(image) = html::image_metadata(tag, first..last, source) { return Some(image); }
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
        // Source quote prefixes remain literal inside a multiline tag. Its
        // metadata was already parsed without those prefixes by the owning
        // paragraph; parsing the raw spelling again would duplicate it or
        // include quote markers in an attribute value.
        if let Some(mut image) = html::image_metadata(tag, output..output, source.clone())
            .filter(|_| self.source_syntax.recognizes_inline_html(&source)) {
            self.output.push('\u{fffc}');
            self.push_provenance(ProvenanceSpan { formatted: output..self.output.len(), source });
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
        if !tag.end && tag.name == "hr" {
            if self.output.len() > self.inline_paragraph_start && !self.output.ends_with('\n') {
                let at = self.output.len();
                self.inline_hard_breaks.push(at);
                self.output.push('\n');
                self.push_provenance(ProvenanceSpan { formatted: at..at + 1, source: source.start..source.start });
            }
            let at = self.output.len();
            self.inline_rules.push(at);
            self.push_provenance(ProvenanceSpan { formatted: at..at, source: source.clone() });
            self.push_provenance(ProvenanceSpan { formatted: at..at, source: source.end..source.end });
            self.inline_hard_breaks.push(at);
            self.output.push('\n');
            self.push_provenance(ProvenanceSpan { formatted: at..at + 1, source: source.end..source.end });
            self.inline_paragraph_start = self.output.len();
            return;
        }
        if tag.end {
            if let Some(index) = self.html_stack.iter().rposition(|(name, _, _, _)| name == &tag.name) {
                let entries: Vec<_> = self.html_stack.drain(index..).collect();
                let finish = self.output.len();
                for (_, start, application, slot) in entries {
                    self.finish_html_scope(start, finish, application, slot);
                }
            }
        } else {
            let application = match tag.name.as_str() {
                "b" | "strong" => Some(StyleApplication::Semantic(SemanticInlineStyle::Strong)),
                "i" | "em" => Some(StyleApplication::Semantic(SemanticInlineStyle::Emphasis)),
                "code" | "kbd" | "samp" | "tt" => Some(StyleApplication::Named("Code".into())),
                "del" | "s" | "strike" => Some(StyleApplication::Automatic("Strikethrough".into())),
                "a" if tag.attribute("href").is_some() => Some(StyleApplication::Automatic("Link".into())),
                "sup" => Some(StyleApplication::direct(CharacterProperties { superscript: Some(true), subscript: Some(false), ..Default::default() })),
                "sub" => Some(StyleApplication::direct(CharacterProperties { subscript: Some(true), superscript: Some(false), ..Default::default() })),
                "u" | "ins" => Some(StyleApplication::direct(CharacterProperties { underline: Some(true), ..Default::default() })),
                _ => None,
            };
            if let Some(application) = application {
                // Reserve sparse direct scopes before their children so an
                // inner script wins even when both have equal visible ranges.
                // Closing fills this slot without scanning or shifting styles
                // from preceding paragraphs or deeply nested scopes.
                let slot = if matches!(application, StyleApplication::Direct(_)) {
                    let slot = self.styles.len();
                    self.styles.push(StyleSpan { range: self.output.len()..self.output.len(), application: application.clone() });
                    Some(slot)
                } else { None };
                self.html_stack.push((tag.name.clone(), self.output.len(), application, slot));
            }
        }
    }

    pub(super) fn finish_html_scope(
        &mut self, start: usize, finish: usize, application: StyleApplication, slot: Option<usize>,
    ) {
        if let Some(slot) = slot {
            self.styles[slot].range = start..finish;
            #[cfg(test)]
            DIRECT_STYLE_SLOT_WRITES.with(|work| work.set(work.get() + 1));
        } else if start < finish {
            // Semantic scopes keep the inner-first ownership order used by
            // source-local delimiter removal.
            self.styles.push(StyleSpan { range: start..finish, application });
        }
    }

}

pub(super) fn block(input: &NormalizedText, range: Range<usize>, revision: Revision) -> FormattedDocument {
    let mut fragment = NormalizedText::new(input.encoding);
    fragment.text = input.text[range.clone()].to_owned();
    for mut unit in input.units_from(input.unit_partition_point(|u| u.normalized.start < range.start))
        .take_while(|u| u.normalized.start < range.end) {
        unit.normalized = unit.normalized.start - range.start..unit.normalized.end - range.start; fragment.push_unit(unit);
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
            TokenKind::Tag(tag) if html::image_metadata(tag, 0..0, 0..0).is_some() => {
                tag.attributes.retain(|(name, _)| matches!(name.as_str(), "src" | "alt" | "width" | "height"));
            }
            TokenKind::Tag(tag) if allowed(&tag.name) => {
                let pre = tag.name == "pre";
                tag.attributes.retain(|(name, _)| matches!(name.as_str(), "href" | "title" | "start" | "value" | "dir" | "align") || pre && name == "lang");
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
    let start = fragment.first_unit().map_or(0, |unit| unit.source.start);
    let end = fragment.last_unit().map_or(start, |unit| unit.source.end);
    let mut result = html::project_markdown_tokens(&fragment, revision, start, end, tokens);
    let mut styles = Vec::new();
    for (ranges, name) in [(comments, "Comment"), (references, "Markdown reference")] {
        for range in ranges {
            let first = fragment.unit_partition_point(|u| u.normalized.start < range.start);
            let last = fragment.unit_partition_point(|u| u.normalized.start < range.end);
            if first == last { continue; }
            let source = fragment.unit(first).unwrap().source.start..fragment.unit(last - 1).unwrap().source.end;
            for span in result.provenance().iter().filter(|span| source.start <= span.source.start && span.source.end <= source.end && !span.formatted.is_empty()) {
                styles.push(StyleSpan { range: span.formatted.clone(), application: StyleApplication::Automatic(name.into()) });
            }
        }
    }
    result.append_link_styles(styles);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, Encoding};

    #[test]
    fn passive_pre_owner_ranges_follow_ancestor_closure_and_original_prefixes() {
        let source = "> <div>\n> <PRE LANG='c&#43;&#43;' lang=rust>\n> <pre lang=python>x</pre>\n> </div>\n> <pre lang=unknown>y";
        let excluded: Vec<_> = source.match_indices("> ").map(|(at, _)| at..at + 2).collect();
        let owners = pre_owners(source, &excluded);
        assert_eq!(owners.len(), 2);
        assert_eq!(&source[owners[0].opening.clone()], "<PRE LANG='c&#43;&#43;' lang=rust>");
        assert_eq!(owners[0].range.end, source.find("</div>").unwrap() + "</div>".len());
        let tag = |owner: &html::PreOwner| match &html::tokenize(&source[owner.opening.clone()])[0].kind {
            TokenKind::Tag(tag) => tag.clone(),
            _ => panic!("pre owner opens with a tag"),
        };
        assert_eq!(html::pre_language(&tag(&owners[0])).as_deref(), Some("cpp"));
        assert_eq!(owners[1].range.end, source.len());
        assert_eq!(html::pre_language(&tag(&owners[1])).as_deref(), Some("unknown"));
        let attribute = tag(&owners[0]).attribute_range("lang").unwrap();
        let attribute = owners[0].opening.start + attribute.start..owners[0].opening.start + attribute.end;
        assert_eq!(&source[attribute], "'c&#43;&#43;'");
    }

    #[test]
    fn scripted_paragraphs_and_deep_inline_scopes_complete_in_linear_style_work() {
        const COUNT: usize = 10_000;
        for source in [
            "<sup>x</sup>\n\n".repeat(COUNT),
            format!("{}x{}", "<sup>".repeat(COUNT), "</sup>".repeat(COUNT)),
        ] {
            DIRECT_STYLE_SLOT_WRITES.with(|work| work.set(0));
            let document =
                Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(DIRECT_STYLE_SLOT_WRITES.with(std::cell::Cell::get), COUNT);
            assert_eq!(
                document
                    .projection()
                    .style_spans()
                    .iter()
                    .filter(|span| matches!(span.application, StyleApplication::Direct(_)))
                    .count(),
                COUNT
            );
            assert_eq!(
                document.text().chars().filter(|ch| *ch == 'x').count(),
                if document.text().contains('\n') {
                    COUNT
                } else {
                    1
                }
            );
        }

        let empty =
            Document::from_bytes(b"a<sup></sup>b".to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(empty.text(), "ab");
        assert!(empty
            .projection()
            .style_spans()
            .iter()
            .all(|span| !span.range.is_empty()));
    }
}
