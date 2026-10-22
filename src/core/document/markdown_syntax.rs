//! CommonMark recognition with source ranges. The editor owns projection and
//! reverse edits; the grammar parser never serializes or replaces source.
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug)]
pub(super) enum InlineKind {
    Emphasis, Strong, Strike, Reference, Autolink, Html, Link,
    Image { destination: String, text: String, title: String, inline: bool },
}
#[derive(Clone, Debug)]
pub(super) struct Inline {
    pub range: Range<usize>,
    pub inner: Range<usize>,
    pub kind: InlineKind,
}

pub(super) fn inlines(text: &str, range: Range<usize>, definitions: &str) -> BTreeMap<usize, Inline> {
    parse_inlines(text, range, definitions, "x ", ParseContext::ContainerFragment)
}

/// Recognize inline pairs in physical source with its actual list and quote
/// containers. Continuation prefixes belong to those blocks, not inline text.
pub(super) fn source_inlines(text: &str) -> BTreeMap<usize, Inline> {
    parse_inlines(text, 0..text.len(), "", "", ParseContext::Interior)
}

fn parse_inlines(text: &str, range: Range<usize>, definitions: &str, prefix: &str, context: ParseContext) -> BTreeMap<usize, Inline> {
    // Paragraph bodies use a harmless prefix to suppress block recognition;
    // physical source uses its real container grammar.
    let offset = prefix.len();
    let source = format!("{prefix}{}{}", &text[range.clone()], definitions);
    let mut result = BTreeMap::new();
    let start = context.metadata_end(&source);
    let grammar = fence_grammar_text(&source[start..]);
    let mut events = Parser::new_ext(&grammar, Options::ENABLE_STRIKETHROUGH)
        .into_offset_iter().map(|(event, span)| (event, span.start + start..span.end + start));
    while let Some((event, mut span)) = events.next() {
        if span.start < offset || span.end > range.len() + offset { continue; }
        let raw = &source[span.clone()];
        let (kind, padding) = match event {
            Event::Start(Tag::Emphasis) => (InlineKind::Emphasis, 1),
            Event::Start(Tag::Strong) => (InlineKind::Strong, 2),
            Event::Start(Tag::Strikethrough) => (InlineKind::Strike, raw.bytes().take_while(|b| *b == b'~').count().min(2)),
            Event::Start(Tag::Image { dest_url, link_type, title, .. }) => {
                // pulldown's collapsed-reference interval ends at the label;
                // the consumed empty suffix still belongs to the image.
                if link_type == LinkType::Collapsed && source[span.end..].starts_with("[]") {
                    span.end += 2;
                }
                let mut alt = String::new();
                let mut depth=1;
                for (event,_) in events.by_ref() {
                    match event {
                        Event::Start(Tag::Image {..}) => depth+=1,
                        Event::End(TagEnd::Image) => {depth-=1;if depth==0 {break;}},
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        Event::SoftBreak | Event::HardBreak => alt.push(' '),
                        _ => {},
                    }
                }
                (InlineKind::Image { destination: dest_url.into_string(), text: alt, title: title.into_string(), inline: link_type == LinkType::Inline }, 0)
            },
            Event::Start(Tag::Link { link_type: LinkType::Autolink | LinkType::Email, .. }) => (InlineKind::Autolink, 1),
            Event::Start(Tag::Link { link_type: LinkType::Inline, .. }) => {
                let Some(label_end) = super::links::markdown_label_end(&source, span.start, span.end) else { continue; };
                let start = range.start + span.start - offset;
                let end = range.start + span.end - offset;
                result.insert(start, Inline { range: start..end,
                    inner: start + 1..range.start + label_end - offset, kind: InlineKind::Link });
                continue;
            },
            Event::Start(Tag::Link { .. }) => (InlineKind::Reference, 0),
            Event::InlineHtml(_) => (InlineKind::Html, 0),
            _ => continue,
        };
        let start = range.start + span.start - offset;
        let end = range.start + span.end - offset;
        result.insert(start, Inline { range: start..end, inner: start + padding..end - padding, kind });
    }
    result
}

/// The grammar depends on the origin of its input, not just its contents.
/// Container fragments have already lost source prefixes and can never own
/// document metadata, even when their first mapped byte is at the file start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParseContext {
    Document,
    Interior,
    ContainerFragment,
}

impl ParseContext {
    pub fn source_region(source_start: usize, content_start: usize) -> Self {
        if source_start == content_start { Self::Document } else { Self::Interior }
    }

    fn permits_metadata(self) -> bool { self == Self::Document }
    fn metadata_end(self, text: &str) -> usize {
        if self.permits_metadata() { super::markdown_front_matter::recognize(text).map_or(0, |front| front.range.end) }
        else { 0 }
    }
}

pub(super) fn definitions(text: &str, context: ParseContext) -> (String, Vec<Range<usize>>) {
    if !text.contains("]:") { return (String::new(), Vec::new()); }
    let parse_start = context.metadata_end(text);
    let grammar = fence_grammar_text(&text[parse_start..]);
    let parser = Parser::new(&grammar);
    let mut ranges: Vec<_> = parser.reference_definitions().iter()
        .map(|(_, value)| value.span.start + parse_start..value.span.end + parse_start).collect();
    ranges.sort_by_key(|range| range.start);
    let known = ranges.clone();
    let mut occupied = Vec::<Range<usize>>::new();
    for (event, range) in parser.into_offset_iter() {
        let range = range.start + parse_start..range.end + parse_start;
        if matches!(event, Event::Start(Tag::Paragraph | Tag::Heading { .. }
            | Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::Table(_))) {
            if let Some(previous) = occupied.last_mut().filter(|previous| range.start <= previous.end) {
                previous.end = previous.end.max(range.end);
            } else {
                occupied.push(range);
            }
        }
    }
    // The parser's lookup keeps only the first definition of a label. Later
    // definitions are still authored definition syntax and retain the same
    // visible reference treatment, including after a line copy.
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if offset >= parse_start && line.trim_start_matches(' ').starts_with('[') && line.contains("]:")
            && !known.get(known.partition_point(|range| range.end <= offset))
                .is_some_and(|range| range.start <= offset && offset < range.end)
            && !occupied.get(occupied.partition_point(|range| range.end <= offset))
                .is_some_and(|range| range.start <= offset && offset < range.end) {
            for (_, definition) in Parser::new(line).reference_definitions().iter() {
                ranges.push(offset + definition.span.start..offset + definition.span.end);
            }
        }
        offset += line.len();
    }
    ranges.sort_by_key(|range| range.start);
    ranges.dedup();
    let mut definitions = String::new();
    for range in &ranges {
        definitions.push_str("\n\n");
        definitions.push_str(&text[range.clone()]);
    }
    (definitions, ranges)
}

#[cfg(test)]
mod definition_tests {
    #[test]
    fn fence_grammar_shadow_preserves_other_trailing_tabs() {
        for source in ["prose ```\t\n", "![alt ```\t](image)", "```\nbody\t\n```", "plain\t\n"] {
            assert!(matches!(super::fence_grammar_text(source), std::borrow::Cow::Borrowed(_)), "{source:?}");
        }
        for source in ["```\t\n", ">   ~~~~ \t\n", "-\t```\n\t```\t\n"] {
            let shadow = super::fence_grammar_text(source);
            assert_eq!(shadow.len(), source.len());
            assert!(matches!(shadow, std::borrow::Cow::Owned(_)), "{source:?}");
        }
    }

    #[test]
    fn closing_fence_whitespace_does_not_create_an_extra_empty_paragraph() {
        for delimiter in ["```", "~~~"] {
            for suffix in ["", " ", "\t", " \t"] {
                let source = format!("{delimiter}rust title=\"keep\"\ncode\n{delimiter}{delimiter}{suffix}\n\n");
                let document = super::super::Document::from_bytes(source.as_bytes().to_vec(),
                    super::super::Encoding::Utf8, super::super::Format::Markdown).unwrap();
                assert_eq!(document.text(), "code\n", "{source:?}");
                assert_eq!(document.projection().blocks().len(), 2, "{source:?}");
                assert!(document.projection().blocks()[1].containers.is_empty(), "{source:?}");
                assert_eq!(document.source_bytes(), source.as_bytes());
            }
        }
    }

    #[test]
    fn front_matter_does_not_supply_markdown_definitions_or_inline_scopes() {
        let source = "---\n[ref]: /metadata\ntext: **literal**\n---\n\n[ref]: /document\n\n**body**";
        let (context, ranges) = super::definitions(source, super::ParseContext::Document);
        assert_eq!(ranges.len(), 1);
        assert!(context.contains("/document"));
        assert!(!context.contains("/metadata"));
        let syntax = super::Blocks::parse(source, super::ParseContext::Document);
        assert!(syntax.inline_code.is_empty());
        assert!(syntax.inline_links.is_empty());
        assert!(syntax.inline_html.is_empty());
    }

    #[test]
    fn physical_inline_parsing_does_not_assume_a_regional_slice_is_document_leading() {
        let source = "---\ntext: **literal**\n---\n";
        let inlines = super::source_inlines(source);
        assert_eq!(inlines.len(), 1);
        assert_eq!(&source[inlines.values().next().unwrap().range.clone()], "**literal**");
    }

    #[test]
    fn front_matter_grammar_owns_literal_lines_only_at_the_document_start() {
        let source = "---\n# literal\n- item\n\n> quote\n---\n\n# Heading";
        let syntax = super::Blocks::parse(source, super::ParseContext::Document);
        assert!(syntax.front_matter.is_some());
        assert_eq!(syntax.code.len(), 1);
        assert!(syntax.containers.is_empty());
        assert_eq!(syntax.blocks.len(), 1);
        assert!(matches!(syntax.blocks[0].role, super::BlockRole::Heading(1)));
        assert_eq!(&source[syntax.blocks[0].content.clone()], "Heading");
        for context in [super::ParseContext::Interior, super::ParseContext::ContainerFragment,
            super::ParseContext::source_region(20, 3)] {
            assert!(super::Blocks::parse(source, context).front_matter.is_none(), "{context:?}");
        }
        let regional = super::Blocks::parse(source, super::ParseContext::Interior);
        assert!(super::Blocks::parse(source, super::ParseContext::source_region(3, 3)).front_matter.is_some());
        assert!(regional.code.is_empty());
        assert!(!regional.containers.is_empty());
        let (_, definitions) = super::definitions("---\n[ref]: /inside\n---", super::ParseContext::ContainerFragment);
        assert_eq!(definitions.len(), 1);
    }

    #[test]
    fn duplicate_definitions_keep_their_source_context() {
        let source = "[ref]: /first\n\n[ref]: /second\n\n```\n[code]: /hidden\n```\n\nprose\n[inline]: /literal";
        let (context, ranges) = super::definitions(source, super::ParseContext::Document);
        assert_eq!(ranges.len(), 2);
        assert!(context.contains("[ref]: /first"));
        assert!(context.contains("[ref]: /second"));
        assert!(!context.contains("/hidden"));
        assert!(!context.contains("/literal"));
    }
}

/// Inline destinations are self-contained. Other bracket syntax can depend on
/// a definition outside the regional projection, including a newly typed one.
pub(super) fn needs_reference_context(text: &str) -> bool {
    if !text.contains('[') { return false; }
    let mut consumed = 0;
    let grammar = fence_grammar_text(text);
    for (event, range) in Parser::new(&grammar).into_offset_iter() {
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
        let mut excess_closes = tail[..length].bytes().fold(0isize, |balance, byte| {
            balance + isize::from(byte == b')') - isize::from(byte == b'(')
        });
        loop {
            let value = &tail[..length];
            if value.ends_with(['?', '!', '.', ',', ':', '*', '_', '~']) { length -= 1; continue; }
            if value.ends_with(')') && excess_closes > 0 { length -= 1; excess_closes -= 1; continue; }
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
    if !domain.contains('.') || domain.ends_with(['-', '_']) || domain.split('.').any(str::is_empty) { return None; }
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

/// pulldown-cmark's closing-fence scanner accepts trailing spaces but not
/// tabs. Both are permitted by Markdown and by our source fence recognizer.
/// Use an offset-preserving grammar view so that its container ranges agree
/// without changing any authored whitespace or literal code text.
fn fence_grammar_text(text: &str) -> std::borrow::Cow<'_, str> {
    let mut replacement = None;
    if text.contains('\t') {
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let body = content.trim_end_matches([' ', '\t']);
            let suffix = &content[body.len()..];
            let fence = &content[super::markdown_quotes::container_prefix(content)..];
            if suffix.contains('\t')
                && fence.as_bytes().first().is_some_and(|delimiter| {
                    matches!(delimiter, b'`' | b'~')
                        && fence_close(fence, *delimiter, 3)
                }) {
                let bytes = replacement.get_or_insert_with(|| text.as_bytes().to_vec());
                for byte in &mut bytes[start + body.len()..start + content.len()] {
                    if *byte == b'\t' { *byte = b' '; }
                }
            }
            start += line.len();
        }
    }
    replacement.map_or(std::borrow::Cow::Borrowed(text),
        |bytes| std::borrow::Cow::Owned(String::from_utf8(bytes).unwrap()))
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
#[derive(Clone, Debug)]
pub(super) struct CodeSyntax {
    pub range: Range<usize>,
    /// Includes internal body endings, but excludes a closing fence or an
    /// indented block's final paragraph separator.
    pub body_end: usize,
    pub fenced: bool,
}
#[derive(Default)]
pub(super) struct Blocks {
    pub front_matter: Option<super::markdown_front_matter::FrontMatter>,
    pub paragraphs: Vec<Range<usize>>,
    pub soft_breaks: Vec<Range<usize>>,
    pub hard_breaks: Vec<Range<usize>>,
    pub inline_code: Vec<Range<usize>>,
    pub inline_links: Vec<Range<usize>>,
    pub blocks: Vec<BlockSyntax>,
    pub containers: Vec<Container>,
    pub owners: Vec<super::containers::SourceContainer>,
    pub definitions: Vec<Range<usize>>,
    pub tables: Vec<super::markdown_tables::TableSyntax>,
    pub code: Vec<CodeSyntax>,
    pub inline_html: Vec<Range<usize>>,
}
impl Blocks {
    pub fn parse(text: &str, context: ParseContext) -> Self {
        super::work_statistics::record(|work| {
            work.markdown_block_parse_calls += 1;
            work.markdown_block_parsed_bytes += text.len();
        });
        use pulldown_cmark::TagEnd;
        let mut result = Self::default();
        result.front_matter = context.permits_metadata().then(|| super::markdown_front_matter::recognize(text)).flatten();
        let parse_start = result.front_matter.as_ref().map_or(0, |front| front.range.end);
        let grammar = fence_grammar_text(&text[parse_start..]);
        let parser = Parser::new_ext(&grammar, Options::ENABLE_TABLES);
        let mut tables = super::markdown_tables::TableParser::default();
        result.definitions = definitions(text, context).1;
        if let Some(front) = &result.front_matter {
            result.owners.push(super::containers::SourceContainer::new(front.range.clone(), super::ContainerKind::CodeBlock));
            result.code.push(CodeSyntax { range: front.range.clone(), body_end: front.body.end, fenced: true });
        }
        let mut quotes = 0;
        let mut lists: Vec<(Range<usize>, Option<u64>, bool, usize)> = Vec::new();
        let mut items: Vec<usize> = Vec::new();
        let mut active = None;
        let mut active_code = None;
        for (event, range) in parser.into_offset_iter() {
            let range = range.start + parse_start..range.end + parse_start;
            tables.event(text, &event, &range);
            if matches!(event, Event::InlineHtml(_)) { result.inline_html.push(range.clone()); }
            match &event {
                Event::Start(Tag::Paragraph) => result.paragraphs.push(range.clone()),
                Event::SoftBreak => result.soft_breaks.push(range.clone()),
                Event::HardBreak => result.hard_breaks.push(range.clone()),
                Event::Code(_) => result.inline_code.push(range.clone()),
                Event::Start(Tag::Link { link_type: LinkType::Inline, .. }) => result.inline_links.push(range.clone()),
                _ => {}
            }
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
                Event::Start(Tag::CodeBlock(kind)) => {
                    result.owners.push(super::containers::SourceContainer::new(range.clone(), super::ContainerKind::CodeBlock));
                    active_code = Some(result.code.len());
                    let body_end = text[range.clone()].find('\n').map_or(range.end, |at| range.start + at + 1);
                    result.code.push(CodeSyntax { range, body_end, fenced: matches!(kind, pulldown_cmark::CodeBlockKind::Fenced(_)) });
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some(index) = active_code.take() {
                        let code = &mut result.code[index];
                        if !code.fenced {
                            if let Some(body) = text[..code.body_end].strip_suffix('\n') {
                                code.body_end = body.len();
                            }
                        }
                    }
                }
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
                    if let Some(index) = active_code {
                        if matches!(event, Event::Text(_)) { result.code[index].body_end = range.end; }
                    }
                    if let Some(index) = active {
                        let block = &mut result.blocks[index];
                        block.content.start = block.content.start.min(range.start);
                        block.content.end = block.content.end.max(range.end);
                    }
                }
            }
        }
        for block in &mut result.blocks {
            // Text-event offsets may omit the escape before the first visible
            // character. That backslash belongs to the heading body relation.
            if matches!(block.role, BlockRole::Heading(_))
                && block.content.start < block.content.end {
                while block.content.start > block.range.start
                    && text.as_bytes().get(block.content.start - 1) == Some(&b'\\') {
                    block.content.start -= 1;
                }
            }
            if block.content.start > block.content.end {
                let raw = text[block.range.clone()].trim_end_matches(['\n', '\r']);
                // Empty ATX headings still have a body boundary. Keep it
                // after the opening marker, before any optional closing
                // sequence, so typing does not turn that sequence into text.
                let indent = raw.len() - raw.trim_start_matches([' ', '\t']).len();
                let hashes = raw[indent..].bytes().take_while(|byte| *byte == b'#').count();
                let prefix = if (1..=6).contains(&hashes) {
                    let end = indent + hashes;
                    end + usize::from(raw.as_bytes().get(end).is_some_and(|byte| matches!(byte, b' ' | b'\t')))
                } else {
                    raw.len()
                };
                let at = block.range.start + prefix;
                block.content = at..at;
            }
        }
        result.blocks.sort_by_key(|block| block.range.start);
        result.tables = tables.tables;
        result.containers.sort_by_key(|container| container.range.start);
        result.definitions.sort_by_key(|range| range.start);
        result
    }
    pub fn to_source(mut self, input: &super::line_endings::NormalizedText) -> Self {
        let at = |at| input.source_at(at);
        if let Some(front) = &mut self.front_matter {
            front.range = at(front.range.start)..at(front.range.end);
            front.body = at(front.body.start)..at(front.body.end);
        }
        self.tables = self.tables.into_iter().map(|table| table.to_source(input)).collect();
        for block in &mut self.blocks { block.range = at(block.range.start)..at(block.range.end); block.content = at(block.content.start)..at(block.content.end); }
        for owner in &mut self.owners { owner.range = at(owner.range.start)..at(owner.range.end); }
        for code in &mut self.code { code.range = at(code.range.start)..at(code.range.end); code.body_end = at(code.body_end); }
        for container in &mut self.containers { container.range = at(container.range.start)..at(container.range.end); }
        for range in &mut self.definitions { *range = at(range.start)..at(range.end); }
        for range in &mut self.inline_html { *range = at(range.start)..at(range.end); }
        for ranges in [&mut self.paragraphs, &mut self.soft_breaks, &mut self.hard_breaks,
            &mut self.inline_code, &mut self.inline_links] {
            for range in ranges { *range = at(range.start)..at(range.end); }
        }
        self
    }

    pub(super) fn recognizes_inline_html(&self, source: &Range<usize>) -> bool {
        self.inline_html.binary_search_by_key(&source.start, |range| range.start).ok()
            .is_some_and(|index| self.inline_html[index] == *source)
    }

    pub(super) fn recognizes_inline_link(&self, source: &Range<usize>) -> bool {
        self.inline_links.binary_search_by_key(&source.start, |range| range.start).ok()
            .is_some_and(|index| self.inline_links[index] == *source)
    }
}

/// Line-ending characters that the open policy keeps as content are text, not
/// grammar line breaks: literal LF under Mac endings, and any CR left after
/// normalization (recognized endings all become LF). Otherwise the grammar
/// splits a line that every other classifier sees whole. Preserve byte
/// offsets while classifying blocks.
pub(super) fn grammar_text(input: &super::line_endings::NormalizedText) -> std::borrow::Cow<'_, str> {
    let mut replacement = None;
    for (at, _) in input.text.match_indices('\n') {
        if input.endings.get(input.endings.partition_point(|ending| ending.normalized.start < at))
            .is_none_or(|ending| ending.normalized.start != at) {
            replacement.get_or_insert_with(|| input.text.as_bytes().to_vec())[at] = 1;
        }
    }
    for (at, _) in input.text.match_indices('\r') {
        replacement.get_or_insert_with(|| input.text.as_bytes().to_vec())[at] = 1;
    }
    replacement.map_or(std::borrow::Cow::Borrowed(&input.text), |bytes| std::borrow::Cow::Owned(String::from_utf8(bytes).unwrap()))
}
