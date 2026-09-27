//! CommonMark recognition with source ranges. The editor owns projection and
//! reverse edits; the grammar parser never serializes or replaces source.
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug)]
pub(super) enum InlineKind {
    Emphasis, Strong, Strike, Reference, Autolink, Html,
}
#[derive(Clone, Debug)]
pub(super) struct Inline {
    pub range: Range<usize>,
    pub inner: Range<usize>,
    pub kind: InlineKind,
}

pub(super) fn inlines(text: &str, range: Range<usize>, definitions: &str) -> BTreeMap<usize, Inline> {
    // This is a paragraph body, even if its first character resembles a block
    // marker. A harmless prefix prevents the block parser reinterpreting it.
    let source = format!("x {}{}", &text[range.clone()], definitions);
    let mut result = BTreeMap::new();
    for (event, span) in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        if span.start < 2 || span.end > range.len() + 2 { continue; }
        let raw = &source[span.clone()];
        let (kind, padding) = match event {
            Event::Start(Tag::Emphasis) => (InlineKind::Emphasis, 1),
            Event::Start(Tag::Strong) => (InlineKind::Strong, 2),
            Event::Start(Tag::Strikethrough) => (InlineKind::Strike, raw.bytes().take_while(|b| *b == b'~').count().min(2)),
            Event::Start(Tag::Image { .. }) => (InlineKind::Reference, 0),
            Event::Start(Tag::Link { link_type: LinkType::Autolink | LinkType::Email, .. }) => (InlineKind::Autolink, 1),
            Event::Start(Tag::Link { link_type: LinkType::Inline, .. }) => continue,
            Event::Start(Tag::Link { .. }) => (InlineKind::Reference, 0),
            Event::InlineHtml(_) => (InlineKind::Html, 0),
            _ => continue,
        };
        let start = range.start + span.start - 2;
        let end = range.start + span.end - 2;
        result.insert(start, Inline { range: start..end, inner: start + padding..end - padding, kind });
    }
    result
}

pub(super) fn definitions(text: &str) -> (String, Vec<Range<usize>>) {
    if !text.contains("]:") { return (String::new(), Vec::new()); }
    let parser = Parser::new(text);
    let mut ranges: Vec<_> = parser.reference_definitions().iter().map(|(_, value)| value.span.clone()).collect();
    ranges.sort_by_key(|range| range.start);
    let mut definitions = String::new();
    for range in &ranges {
        definitions.push_str("\n\n");
        definitions.push_str(&text[range.clone()]);
    }
    (definitions, ranges)
}

/// Inline destinations are self-contained. Other bracket syntax can depend on
/// a definition outside the regional projection, including a newly typed one.
pub(super) fn needs_reference_context(text: &str) -> bool {
    if !text.contains('[') { return false; }
    let mut consumed = 0;
    for (event, range) in Parser::new(text).into_offset_iter() {
        if range.start < consumed { continue; }
        if matches!(event, Event::Start(Tag::Link { link_type: LinkType::Inline, .. } | Tag::Image { link_type: LinkType::Inline, .. })) {
            if text[consumed..range.start].contains('[') { return true; }
            consumed = range.end;
        }
    }
    text[consumed..].contains('[')
}

/// GFM extended autolinks: punctuation terminates a URL, balanced parentheses
/// stay in it, and email domains must have a dot and a legal final label.
pub(super) fn autolink(text: &str, at: usize, end: usize) -> Option<(usize, String)> {
    let tail = &text[at..end];
    let previous = text[..at].chars().next_back();
    let url = tail.starts_with("http://") || tail.starts_with("https://") || tail.starts_with("www.");
    if url {
        if previous.is_some_and(|c| !c.is_whitespace() && !matches!(c, '*' | '_' | '~' | '(')) { return None; }
        let mut length = tail.find(|c: char| c.is_whitespace() || c == '<').unwrap_or(tail.len());
        loop {
            let value = &tail[..length];
            if value.ends_with(['?', '!', '.', ',', ':', '*', '_', '~']) { length -= 1; continue; }
            if value.ends_with(')') && value.bytes().filter(|c| *c == b')').count() > value.bytes().filter(|c| *c == b'(').count() { length -= 1; continue; }
            if value.ends_with(';') {
                if let Some(amp) = value.rfind('&').filter(|amp| value[*amp + 1..length - 1].bytes().all(|c| c.is_ascii_alphanumeric())) { length = amp; continue; }
            }
            break;
        }
        let value = &tail[..length];
        let host = value.strip_prefix("http://").or_else(|| value.strip_prefix("https://")).unwrap_or(value).split('/').next()?;
        let labels: Vec<_> = host.split('.').collect();
        if labels.len() < 2 || labels.iter().rev().take(2).any(|label| label.is_empty() || label.contains('_')) { return None; }
        return Some((at + length, if value.starts_with("www.") { format!("http://{value}") } else { value.to_owned() }));
    }
    if previous.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')) { return None; }
    let local = tail.bytes().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+')).count();
    if local == 0 || tail.as_bytes().get(local) != Some(&b'@') { return None; }
    let domain = &tail[local + 1..];
    let mut length = domain.bytes().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_')).count();
    while domain[..length].ends_with('.') { length -= 1; }
    let domain = &domain[..length];
    if !domain.contains('.') || !domain.ends_with(|c: char| c.is_ascii_alphabetic()) || domain.contains('_') { return None; }
    let finish = local + 1 + length;
    Some((at + finish, format!("mailto:{}", &tail[..finish])))
}

pub(super) fn fence_close(line: &str, delimiter: u8, length: usize) -> bool {
    let indent = line.bytes().take_while(|c| *c == b' ').count();
    if indent > 3 { return false; }
    let body = &line[indent..];
    let count = body.bytes().take_while(|c| *c == delimiter).count();
    count >= length && body[count..].bytes().all(|c| matches!(c, b' ' | b'\t'))
}

#[derive(Clone, Debug)]
pub(super) enum BlockRole { Heading(u8), Html, Rule }
#[derive(Clone, Debug)]
pub(super) struct BlockSyntax {
    pub range: Range<usize>,
    pub content: Range<usize>,
    pub role: BlockRole,
}
#[derive(Clone, Debug)]
pub(super) struct Container {
    pub range: Range<usize>,
    pub quote_depth: usize,
    pub list: Option<super::BlockKind>,
    pub loose: bool,
}
#[derive(Default)]
pub(super) struct Blocks {
    pub blocks: Vec<BlockSyntax>,
    pub containers: Vec<Container>,
    pub owners: Vec<super::containers::SourceContainer>,
    pub definitions: Vec<Range<usize>>,
}
impl Blocks {
    pub fn parse(text: &str) -> Self {
        use pulldown_cmark::TagEnd;
        let parser = Parser::new(text);
        let mut result = Self::default();
        result.definitions = parser.reference_definitions().iter().map(|(_, definition)| definition.span.clone()).collect();
        let mut quotes = 0;
        let mut lists: Vec<(Range<usize>, Option<u64>, bool, usize)> = Vec::new();
        let mut items: Vec<usize> = Vec::new();
        let mut active = None;
        for (event, range) in parser.into_offset_iter() {
            match event {
                Event::Start(Tag::BlockQuote(_)) => {
                    quotes += 1;
                    result.owners.push(super::containers::SourceContainer::new(range.clone(), super::ContainerKind::Quote));
                    result.containers.push(Container { range, quote_depth: quotes, list: None, loose: false });
                }
                Event::End(TagEnd::BlockQuote(_)) => quotes -= 1,
                Event::Start(Tag::List(ordinal)) => {
                    result.owners.push(super::containers::SourceContainer::new(range.clone(), super::ContainerKind::List { ordered: ordinal.is_some() }));
                    lists.push((range, ordinal, false, result.containers.len()));
                }
                Event::Start(Tag::CodeBlock(_)) => result.owners.push(super::containers::SourceContainer::new(range, super::ContainerKind::CodeBlock)),
                Event::End(TagEnd::List(_)) => {
                    let (range, _, loose, first) = lists.pop().unwrap();
                    if loose {
                        for container in &mut result.containers[first..] {
                            if range.start <= container.range.start && container.range.end <= range.end
                                && matches!(container.list, Some(super::BlockKind::ListItem { level, .. }) if level as usize == lists.len()) {
                                container.loose = true;
                            }
                        }
                    }
                }
                Event::Start(Tag::Item) => {
                    result.owners.push(super::containers::SourceContainer::new(range.clone(), super::ContainerKind::ListItem));
                    let level = lists.len().saturating_sub(1).min(255) as u8;
                    let (list_range, ordinal, _, _) = lists.last().unwrap();
                    let kind = super::BlockKind::ListItem { ordered: ordinal.is_some(), ordinal: ordinal.unwrap_or(1), level,
                        container_start: list_range.start == range.start, item_start: true, marker_is_decoration: false };
                    items.push(result.containers.len());
                    result.containers.push(Container { range, quote_depth: quotes, list: Some(kind), loose: false });
                }
                Event::End(TagEnd::Item) => {
                    items.pop();
                    if let Some((_, Some(ordinal), _, _)) = lists.last_mut() { *ordinal = ordinal.saturating_add(1); }
                }
                Event::Start(Tag::Paragraph) => { if items.last().is_some_and(|index| result.containers[*index].quote_depth == quotes) { if let Some((_, _, loose, _)) = lists.last_mut() { *loose = true; } } }
                Event::Start(Tag::Heading { level, .. }) => {
                    active = Some(result.blocks.len());
                    result.blocks.push(BlockSyntax { content: range.end..range.start, range, role: BlockRole::Heading(level as u8) });
                }
                Event::End(TagEnd::Heading(_)) => { active = None; }
                Event::Start(Tag::HtmlBlock) => {
                    result.blocks.push(BlockSyntax { content: range.clone(), range, role: BlockRole::Html });
                }
                Event::Rule => result.blocks.push(BlockSyntax { content: range.clone(), range, role: BlockRole::Rule }),
                _ => {
                    if let Some(index) = active {
                        let block = &mut result.blocks[index];
                        block.content.start = block.content.start.min(range.start);
                        block.content.end = block.content.end.max(range.end);
                    }
                }
            }
        }
        for block in &mut result.blocks {
            if block.content.start > block.content.end {
                let raw = text[block.range.clone()].trim_end_matches(['\n', '\r']);
                let at = block.range.start + raw.len();
                block.content = at..at;
            }
        }
        result.blocks.sort_by_key(|block| block.range.start);
        result.containers.sort_by_key(|container| container.range.start);
        result.definitions.sort_by_key(|range| range.start);
        result
    }
    pub fn to_source(mut self, input: &super::line_endings::NormalizedText) -> Self {
        let at = |at| input.units.get(input.units.partition_point(|unit| unit.normalized.start < at))
            .map_or_else(|| input.units.last().map_or(0, |unit| unit.source.end), |unit| unit.source.start);
        for block in &mut self.blocks { block.range = at(block.range.start)..at(block.range.end); block.content = at(block.content.start)..at(block.content.end); }
        for owner in &mut self.owners { owner.range = at(owner.range.start)..at(owner.range.end); }
        for container in &mut self.containers { container.range = at(container.range.start)..at(container.range.end); }
        for range in &mut self.definitions { *range = at(range.start)..at(range.end); }
        self
    }
}

/// Literal LF in a document explicitly opened with Mac CR endings is text,
/// not a grammar line break. Preserve byte offsets while classifying blocks.
pub(super) fn grammar_text(input: &super::line_endings::NormalizedText) -> std::borrow::Cow<'_, str> {
    let mut replacement = None;
    for (at, _) in input.text.match_indices('\n') {
        if input.endings.get(input.endings.partition_point(|ending| ending.normalized.start < at))
            .is_none_or(|ending| ending.normalized.start != at) {
            replacement.get_or_insert_with(|| input.text.as_bytes().to_vec())[at] = 1;
        }
    }
    replacement.map_or(std::borrow::Cow::Borrowed(&input.text), |bytes| std::borrow::Cow::Owned(String::from_utf8(bytes).unwrap()))
}
