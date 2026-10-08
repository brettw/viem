//! Passive HTML fragment interpretation for Markdown and clipboard interchange.
//! This is not a document format or an editing mode. Tokenization and entity
//! decoding retain source provenance; interpretation never executes code or
//! resolves resources. Shared CSS serialization also supports HTML export.
use super::line_endings::NormalizedText;
use super::rich_text::Builder;
use super::{
    BlockKind, BlockProperties, CharacterProperties, Color, FontSlant, FormattedDocument,
    LineSpacing, ParagraphAlignment, Revision, WritingDirection,
};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Tag {
    pub name: String,
    pub end: bool,
    pub attributes: Vec<(String, String)>,
    pub attribute_ranges: Vec<(String, Range<usize>)>,
}
impl Tag {
    pub(super) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
    pub(super) fn attribute_range(&self, name: &str) -> Option<Range<usize>> {
        self.attribute_ranges.iter().find(|(key, _)| key == name).map(|(_, range)| range.clone())
    }
}

/// Image attributes are passive metadata; dimensions are positive integer
/// pixel requests, whose display bounds are enforced by layout.
pub(super) fn image_metadata(
    tag: &Tag,
    range: Range<usize>,
    source: Range<usize>,
    source_view: bool,
) -> Option<super::InlineImage> {
    if tag.end || tag.name != "img" { return None; }
    let destination = tag.attribute("src")?.trim_matches(|ch: char| ch.is_ascii_whitespace());
    if destination.is_empty() { return None; }
    let dimension = |name| {
        let value = tag.attribute(name)?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
        let value = value.bytes().fold(0u32, |value, byte| value.saturating_mul(10).saturating_add(u32::from(byte - b'0')));
        (value > 0).then_some(value)
    };
    Some(super::InlineImage {
        range, text: tag.attribute("alt").unwrap_or_default().to_owned(), destination: destination.to_owned(),
        width: dimension("width"), height: dimension("height"), source_view, source, inline: false, html: true,
    })
}
#[derive(Clone, Debug)]
pub(super) enum TokenKind {
    Text,
    Tag(Tag),
    Opaque,
    MappedText { text: String, mapped: bool },
}
#[derive(Clone, Debug)]
pub(super) struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 12)
}
// HTML token syntax includes form feed, but CSS document whitespace does not.
// CR character references survive HTML preprocessing and behave as spaces.
fn css_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}
pub(super) fn void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
/// A plain paragraph or a heading: the elements which carry a paragraph style
/// and nothing else. Deliberately narrower than [`paragraph`], which also
/// admits list items and preformatted content.
pub(super) fn heading_or_paragraph(name: &str) -> bool {
    matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// Every element which bears a paragraph of its own.
pub(super) fn paragraph(name: &str) -> bool {
    heading_or_paragraph(name) || matches!(name, "li" | "pre")
}

/// The list container and item elements.
pub(super) fn list_element(name: &str) -> bool {
    matches!(name, "ul" | "ol" | "li")
}
pub(super) fn block(name: &str) -> bool {
    paragraph(name)
        || matches!(
            name,
            "div"
                | "section"
                | "article"
                | "blockquote"
                | "ul"
                | "ol"
                | "header"
                | "footer"
                | "main"
                | "nav"
                | "aside"
                | "dl"
                | "dt"
                | "dd"
        )
}
pub(super) fn hidden(name: &str) -> bool {
    matches!(
        name,
        "head" | "script" | "style" | "template" | "title" | "noscript"
    )
}
pub(super) fn raw_text(name: &str) -> bool {
    matches!(name, "script" | "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed" | "noframes")
}
pub(super) fn atomic(name: &str) -> bool {
    matches!(
        name,
        "img"
            | "object"
            | "iframe"
            | "embed"
            | "svg"
            | "math"
            | "textarea"
            | "video"
            | "audio"
            | "canvas"
            | "table"
    )
}

/// Atomic projection nodes own their complete original syntax. DOM tree
/// construction can synthesize end tags or omit a pop callback, so a DOM
/// closing token alone is not a reliable source boundary for these objects.
fn atomic_source_extents(input: &str, tokens: &[Token]) -> BTreeMap<usize, Range<usize>> {
    let mut extents = BTreeMap::new();
    let mut open = Vec::<(String, usize)>::new();
    for token in tokens {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if !atomic(&tag.name) {
            continue;
        }
        if tag.end {
            if let Some(index) = open.iter().rposition(|(name, _)| *name == tag.name) {
                for (_, start) in open.drain(index..) {
                    extents.insert(start, start..token.range.end);
                }
            }
        } else if void(&tag.name)
            || matches!(tag.name.as_str(), "svg" | "math")
                && input[token.range.clone()].trim_end().ends_with("/>")
        {
            extents.insert(token.range.start, token.range.clone());
        } else {
            open.push((tag.name.clone(), token.range.start));
        }
    }
    for (_, start) in open {
        extents.insert(start, start..input.len());
    }
    extents
}

pub(super) fn tokenize(input: &str) -> Vec<Token> {
    super::work_statistics::record(|stats| {
        stats.html_tokenization_calls += 1;
        stats.html_tokenized_bytes += input.len();
    });
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut raw: Option<String> = None;
    while at < bytes.len() {
        if let Some(name) = raw.take() {
            let mut end = at;
            while end < bytes.len() {
                if bytes[end] == b'<' && bytes.get(end + 1) == Some(&b'/') {
                    let tail = &input[end + 2..];
                    if tail
                        .get(..name.len())
                        .is_some_and(|s| s.eq_ignore_ascii_case(&name))
                        && tail
                            .as_bytes()
                            .get(name.len())
                            .is_some_and(|b| space(*b) || *b == b'>')
                    {
                        break;
                    }
                }
                end += 1;
            }
            if at < end {
                tokens.push(Token {
                    range: at..end,
                    kind: TokenKind::Opaque,
                });
            }
            at = end;
            if at == bytes.len() {
                break;
            }
        }
        let start = at;
        if bytes[at] != b'<' {
            at += 1;
            while at < bytes.len() && bytes[at] != b'<' {
                at += 1;
            }
            tokens.push(Token {
                range: start..at,
                kind: TokenKind::Text,
            });
            continue;
        }
        if input[at..].starts_with("<!--") {
            at = input[at + 4..]
                .find("-->")
                .map(|n| at + 4 + n + 3)
                .unwrap_or(bytes.len());
            tokens.push(Token {
                range: start..at,
                kind: TokenKind::Opaque,
            });
            continue;
        }
        if matches!(bytes.get(at + 1), Some(b'!' | b'?')) {
            at = input[at..]
                .find('>')
                .map(|n| at + n + 1)
                .unwrap_or(bytes.len());
            tokens.push(Token {
                range: start..at,
                kind: TokenKind::Opaque,
            });
            continue;
        }
        at += 1;
        let end = bytes.get(at) == Some(&b'/');
        if end {
            at += 1;
        }
        let name_start = at;
        if !bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
            at = start + 1;
            tokens.push(Token {
                range: start..at,
                kind: TokenKind::Text,
            });
            continue;
        }
        while at < bytes.len() && !space(bytes[at]) && !matches!(bytes[at], b'>' | b'/') {
            at += 1;
        }
        let name = input[name_start..at].to_ascii_lowercase();
        let mut attributes = Vec::new();
        let mut attribute_ranges = Vec::new();
        while at < bytes.len() && bytes[at] != b'>' {
            if space(bytes[at]) || bytes[at] == b'/' {
                at += 1;
                continue;
            }
            let key_start = at;
            while at < bytes.len() && !space(bytes[at]) && !matches!(bytes[at], b'=' | b'>') {
                at += 1;
            }
            let key = input[key_start..at].to_ascii_lowercase();
            let mut attribute_range = at..at;
            while at < bytes.len() && space(bytes[at]) {
                at += 1;
            }
            let mut value = String::new();
            if bytes.get(at) == Some(&b'=') {
                at += 1;
                while at < bytes.len() && space(bytes[at]) {
                    at += 1;
                }
                attribute_range.start = at;
                let quote = bytes.get(at).copied().filter(|q| matches!(q, b'\'' | b'"'));
                if quote.is_some() {
                    at += 1;
                }
                let value_start = at;
                while at < bytes.len()
                    && match quote {
                        Some(q) => bytes[at] != q,
                        None => !space(bytes[at]) && bytes[at] != b'>',
                    }
                {
                    at += 1;
                }
                value = decode_references(&input[value_start..at], true);
                if quote.is_some() && at < bytes.len() {
                    at += 1;
                }
                attribute_range.end = at;
            }
            if !key.is_empty() {
                attribute_ranges.push((key.clone(), attribute_range));
                attributes.push((key, value));
            }
        }
        // An unterminated tag token is ignored under HTML EOF recovery.
        if at == bytes.len() {
            tokens.push(Token {
                range: start..at,
                kind: TokenKind::Opaque,
            });
            break;
        }
        at += 1;
        if !end && raw_text(&name) {
            raw = Some(name.clone());
        }
        tokens.push(Token {
            range: start..at,
            kind: TokenKind::Tag(Tag {
                name,
                end,
                attributes,
                attribute_ranges,
            }),
        });
    }
    tokens
}

/// Interpret HTML after removing known Markdown owner prefixes. Both tag and
/// attribute value ranges still address the original decoded spelling. Callers
/// supply sorted, disjoint character-boundary ranges from source ownership.
pub(super) fn tokenize_without_ranges(input: &str, excluded: &[Range<usize>]) -> Vec<Token> {
    if excluded.is_empty() { return tokenize(input); }
    let removed = without_ranges(input, excluded);
    tokenize(&removed.text).into_iter().map(|mut token| {
        token.range = removed.original_range(token.range);
        if let TokenKind::Tag(tag) = &mut token.kind {
            for (_, range) in &mut tag.attribute_ranges { *range = removed.original_range(range.clone()); }
        }
        token
    }).collect()
}

pub(super) struct RemovedPrefixes {
    pub text: String,
    segments: Vec<(Range<usize>, usize)>,
    original_len: usize,
}
impl RemovedPrefixes {
    pub(super) fn original_range(&self, range: Range<usize>) -> Range<usize> {
        let first = self.segments.partition_point(|(segment, _)| segment.end <= range.start);
        let start = self.segments.get(first).map_or(self.original_len, |(segment, original)| original + range.start - segment.start);
        if range.is_empty() { return start..start; }
        let last = self.segments.partition_point(|(segment, _)| segment.start < range.end) - 1;
        let (segment, original) = &self.segments[last];
        start..original + range.end - segment.start
    }
}

pub(super) fn without_ranges(input: &str, excluded: &[Range<usize>]) -> RemovedPrefixes {
    let mut text = String::new();
    let mut segments = Vec::new();
    let mut at = 0;
    for range in excluded.iter().cloned().chain(std::iter::once(input.len()..input.len())) {
        assert!(at <= range.start && range.start <= range.end && range.end <= input.len());
        if at < range.start {
            let start = text.len();
            text.push_str(&input[at..range.start]);
            segments.push((start..text.len(), at));
        }
        at = range.end;
    }
    RemovedPrefixes { text, segments, original_len: input.len() }
}

#[derive(Clone)]
struct Frame {
    name: String,
    character: CharacterProperties,
    paragraph: BlockProperties,
    kind: BlockKind,
    hidden: bool,
    opaque: bool,
    list_counter: u64,
    has_list_item: bool,
    paragraph_style: Option<super::StyleId>,
    named_character: Option<super::StyleId>,
    preserve_whitespace: bool,
    preserve_newlines: bool,
    output_start: usize,
    source_inner_start: usize,
    list_container_only: bool,
    container: Option<usize>,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            name: String::new(),
            character: CharacterProperties::default(),
            paragraph: BlockProperties::default(),
            kind: BlockKind::Paragraph,
            hidden: false,
            opaque: false,
            list_counter: 0,
            has_list_item: false,
            paragraph_style: None,
            named_character: None,
            preserve_whitespace: false,
            preserve_newlines: false,
            output_start: 0,
            source_inner_start: 0,
            list_container_only: false,
            container: None,
        }
    }
}

/// A list item whose only body is a nested list supplies ancestry, not a
/// separate empty editable paragraph. A genuinely empty li still has its own
/// paragraph, cursor, and marker geometry.
fn list_container_items(tokens: &[Token]) -> std::collections::BTreeSet<usize> {
    struct Item {
        at: usize,
        depth: usize,
        has_list: bool,
        has_body: bool,
    }
    let mut items: Vec<Item> = Vec::new();
    let mut depth = 0usize;
    let mut containers = std::collections::BTreeSet::new();
    for token in tokens {
        match &token.kind {
            TokenKind::Tag(tag) if matches!(tag.name.as_str(), "ul" | "ol") => {
                if tag.end {
                    depth = depth.saturating_sub(1);
                } else {
                    if let Some(item) = items.last_mut().filter(|item| item.depth == depth) {
                        item.has_list = true;
                    }
                    depth += 1;
                }
            }
            TokenKind::Tag(tag) if tag.name == "li" => {
                if tag.end {
                    if let Some(item) = items.pop() {
                        if item.has_list && !item.has_body {
                            containers.insert(item.at);
                        }
                    }
                } else {
                    items.push(Item {
                        at: token.range.start,
                        depth,
                        has_list: false,
                        has_body: false,
                    });
                }
            }
            TokenKind::Opaque => {}
            TokenKind::MappedText { text, .. } if text.bytes().all(css_space) => {}
            _ => {
                if let Some(item) = items.last_mut().filter(|item| item.depth == depth) {
                    item.has_body = true;
                }
            }
        }
    }
    containers
}

fn emit_block_boundary(builder: &mut Builder<'_>, stack: &[Frame], range: Range<usize>) {
    if stack.iter().any(|frame| frame.name == "pre") {
        if !builder.line_is_empty() {
            builder.hard_break(range);
        }
    } else {
        builder.paragraph_break(range);
    }
}

/// Import a clipboard fragment with conventional elements and inline CSS.
/// Authored stylesheets, scripts, and external resources remain inactive.
pub(super) fn project_fragment(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
) -> FormattedDocument {
    project_tokens(
        input,
        revision,
        start,
        end,
        super::html5_tree::tokens(&input.text),
        false,
    )
}

pub(super) fn project_markdown_tokens(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
    tokens: Vec<Token>,
) -> FormattedDocument {
    project_tokens(input, revision, start, end, tokens, true)
}

fn project_tokens(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
    tokens: Vec<Token>,
    markdown_references: bool,
) -> FormattedDocument {
    let lexical_tokens = tokenize(&input.text);
    // Reparenting requires explicit, balanced list delimiters. Recovery may
    // synthesize missing list tags for display, but cannot advertise an edit
    // whose original source has no such boundary.
    let mut list_stack = Vec::new();
    let mut list_editable = true;
    for token in &lexical_tokens {
        if let TokenKind::Tag(tag) = &token.kind {
            if list_element(&tag.name) {
                if tag.end {
                    list_editable &= list_stack.pop() == Some(tag.name.as_str());
                } else {
                    list_stack.push(tag.name.as_str());
                }
            }
        }
    }
    list_editable &= list_stack.is_empty();
    let mut builder = Builder::new(input, revision);
    let mut inline_images = Vec::new();
    builder.list_indent_support = Some((list_editable, list_editable));
    let mut stack = vec![Frame::default()];
    let mut owners: Vec<super::containers::SourceContainer> = Vec::new();
    let mut pending_break: Option<Range<usize>> = None;
    let mut pending_space: Option<Range<usize>> = None;
    let mut pending_space_style = CharacterProperties::default();
    let mut pending_space_named = None;
    let mut pending_space_is_segment_break = false;
    let mut paragraph_seen = false;
    let container_items = list_container_items(&tokens);
    let atomic_extents = if tokens
        .iter()
        .any(|token| matches!(&token.kind, TokenKind::Tag(tag) if !tag.end && atomic(&tag.name)))
    {
        atomic_source_extents(&input.text, &lexical_tokens)
    } else {
        BTreeMap::new()
    };
    for token in tokens {
        match token.kind {
            TokenKind::Opaque => {}
            TokenKind::MappedText {
                text: value,
                mapped,
            } => {
                let frame = stack.last().unwrap();
                if frame.hidden || frame.opaque {
                    continue;
                }
                let value = if value == "\r" { " ".into() } else { value };
                let hard_break = frame.preserve_newlines && value == "\n";
                if !frame.preserve_whitespace
                    && !hard_break
                    && value.bytes().all(css_space)
                    && !(markdown_references && input.text[token.range.clone()].starts_with("&#"))
                {
                    if !builder.line_is_empty() && pending_break.is_none() {
                        if let Some(space) = pending_space
                            .as_mut()
                            .filter(|range| range.end == token.range.start)
                        {
                            space.end = token.range.end;
                        } else if pending_space.is_none() {
                            pending_space = Some(token.range);
                            pending_space_style = frame.character.clone();
                            pending_space_named = frame.named_character.clone();
                            pending_space_is_segment_break = false;
                        }
                        if value == "\n" && !pending_space_is_segment_break {
                            // Spaces surrounding a segment break disappear
                            // before that break becomes a space. The surviving
                            // space therefore has the break's inline style.
                            pending_space_style = frame.character.clone();
                            pending_space_named = frame.named_character.clone();
                            pending_space_is_segment_break = true;
                        }
                    }
                    continue;
                }
                if let Some(range) = pending_break.take() {
                    emit_block_boundary(&mut builder, &stack, range);
                }
                let paragraph_frame = stack
                    .iter()
                    .find(|frame| frame.name == "pre")
                    .unwrap_or(frame);
                builder.kind = paragraph_frame.kind.clone();
                builder.paragraph = paragraph_frame.paragraph.clone();
                builder.paragraph_style = paragraph_frame.paragraph_style.clone();
                builder.defaults = stack
                    .iter()
                    .rev()
                    .find(|ancestor| paragraph(&ancestor.name))
                    .map(|ancestor| ancestor.character.clone())
                    .unwrap_or_default();
                builder.named_character = frame.named_character.clone();
                if hard_break {
                    // Collapsible spaces adjacent to a preserved segment
                    // break disappear even across an inline element boundary.
                    pending_space = None;
                }
                if let Some(range) = pending_space.take() {
                    let named =
                        std::mem::replace(&mut builder.named_character, pending_space_named.take());
                    builder.emit(" ", range, &pending_space_style);
                    builder.named_character = named;
                }
                if !mapped {
                    builder.emit_read_only_with_boundaries(&value, token.range, &frame.character);
                } else if hard_break {
                    builder.hard_break(token.range);
                } else {
                    builder.emit(&value, token.range, &frame.character);
                }
                paragraph_seen = true;
            }
            TokenKind::Tag(tag) => {
                if tag.end {
                    if let Some(index) = stack.iter().rposition(|f| f.name == tag.name) {
                        for closed in &stack[index..] {
                            if let Some(owner) = closed.container {
                                owners[owner].range.end =
                                    builder.source_range(token.range.clone()).end;
                            }
                        }
                        let was_hidden = stack.last().unwrap().hidden;
                        let was_opaque = stack.last().unwrap().opaque;
                        let closed = &stack[index];
                        let closed_paragraph = paragraph(&tag.name);
                        if !closed.hidden
                            && !closed.opaque
                            // An empty element beyond a deferred paragraph
                            // boundary has no editable content in this line.
                            // Anchoring here would make later typing realize
                            // that boundary and unexpectedly add a newline.
                            && pending_break.is_none()
                            && !matches!(closed.name.as_str(), "html" | "head" | "body")
                            && !closed.list_container_only
                            && closed.output_start == builder.text.len()
                        {
                            builder.retain_empty_boundary(
                                closed.source_inner_start,
                                &closed.character,
                            );
                        }
                        stack.truncate(index.max(1));
                        if !was_hidden && !was_opaque && block(&tag.name) {
                            pending_space = None;
                            if !builder.line_is_empty()
                                || closed_paragraph
                                || tag.name == "blockquote"
                            {
                                pending_break = Some(token.range.clone());
                            }
                        }
                    }
                    continue;
                }
                let mut frame = stack.last().cloned().unwrap_or_default();
                let inherited_paragraph_direction = frame.paragraph.base_direction;
                frame.name = tag.name.clone();
                frame.container = None;
                if block(&tag.name) {
                    frame.paragraph.clear_box();
                }
                let container_kind = match tag.name.as_str() {
                    "blockquote" => Some(super::ContainerKind::Quote),
                    "pre" => Some(super::ContainerKind::CodeBlock),
                    "ul" => Some(super::ContainerKind::List { ordered: false }),
                    "ol" => Some(super::ContainerKind::List { ordered: true }),
                    "li" => Some(super::ContainerKind::ListItem),
                    _ => None,
                };
                frame.list_container_only =
                    tag.name == "li" && container_items.contains(&token.range.start);
                frame.hidden |= hidden(&tag.name);
                frame.opaque |= atomic(&tag.name);
                if !frame.hidden && !stack.last().unwrap().opaque && atomic(&tag.name) {
                    if let Some(range) = pending_break.take() {
                        emit_block_boundary(&mut builder, &stack, range);
                    }
                    if let Some(range) = pending_space.take() {
                        let named = std::mem::replace(
                            &mut builder.named_character,
                            pending_space_named.take(),
                        );
                        builder.emit(" ", range, &pending_space_style);
                        builder.named_character = named;
                    }
                    let output_start = builder.text.len();
                    builder.emit(
                        "\u{fffc}",
                        atomic_extents
                            .get(&token.range.start)
                            .cloned()
                            .unwrap_or(token.range.clone()),
                        &stack.last().unwrap().character,
                    );
                    if markdown_references {
                        if let Some(image) = image_metadata(&tag, output_start..builder.text.len(),
                            builder.source_range(token.range.clone()), false) {
                            inline_images.push(image);
                        }
                    }
                    paragraph_seen = true;
                }
                if tag.name == "br" && !frame.hidden && !frame.opaque {
                    pending_space = None;
                    pending_break = None;
                    builder.hard_break(token.range.clone());
                }
                let containing_item = if block(&tag.name) && !list_element(&tag.name) {
                    stack.iter().rposition(|frame| frame.name == "li")
                } else {
                    None
                };
                let first_item_paragraph = containing_item.is_some_and(|index| {
                    stack[index].output_start == builder.text.len() && pending_break.is_none()
                });
                let paragraph_element =
                    (paragraph(&tag.name) || tag.name == "blockquote" || containing_item.is_some())
                        && !frame.list_container_only;
                let inside_pre = stack.iter().any(|frame| frame.name == "pre");
                if block(&tag.name) && !frame.hidden && !frame.opaque {
                    pending_space = None;
                    if paragraph_element {
                        if let Some(range) = pending_break.take() {
                            emit_block_boundary(&mut builder, &stack, range);
                        } else if paragraph_seen && !first_item_paragraph {
                            emit_block_boundary(&mut builder, &stack, token.range.clone());
                        }
                        // A quote is a container: its first explicit child
                        // paragraph does not create an extra empty paragraph.
                        paragraph_seen = tag.name != "blockquote";
                    } else if !builder.line_is_empty() {
                        pending_break = Some(token.range.clone());
                    }
                    frame.kind = if tag.name == "li" {
                        let level = stack
                            .iter()
                            .filter(|f| matches!(f.name.as_str(), "ul" | "ol"))
                            .count()
                            .saturating_sub(1)
                            .min(255) as u8;
                        let list = stack
                            .iter_mut()
                            .rev()
                            .find(|f| matches!(f.name.as_str(), "ul" | "ol"));
                        if let Some(list) = list {
                            let container_start = !list.has_list_item;
                            list.has_list_item = true;
                            list.list_counter = tag
                                .attribute("value")
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or_else(|| list.list_counter.saturating_add(1));
                            BlockKind::ListItem {
                                ordered: list.name == "ol",
                                ordinal: list.list_counter,
                                level,
                                container_start,
                                item_start: true,
                                marker_is_decoration: true,
                            }
                        } else {
                            BlockKind::Paragraph
                        }
                    } else if let Some(index) = containing_item {
                        if let BlockKind::ListItem {
                            ordered,
                            ordinal,
                            level,
                            container_start,
                            ..
                        } = stack[index].kind
                        {
                            BlockKind::ListItem {
                                ordered,
                                ordinal,
                                level,
                                container_start: container_start && first_item_paragraph,
                                item_start: first_item_paragraph,
                                marker_is_decoration: true,
                            }
                        } else {
                            BlockKind::Paragraph
                        }
                    } else if tag.name.len() == 2 && tag.name.starts_with('h') {
                        BlockKind::Heading(tag.name.as_bytes()[1] - b'0')
                    } else {
                        BlockKind::Paragraph
                    };
                }
                match tag.name.as_str() {
                    "b" | "strong" => frame.character.bold = Some(true),
                    "i" | "em" => frame.character.slant = Some(FontSlant::Italic),
                    "u" => frame.character.underline = Some(true),
                    "s" | "strike" | "del" => frame.character.strikethrough = Some(true),
                    "ol" => {
                        frame.list_counter = tag
                            .attribute("start")
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(1)
                            .saturating_sub(1);
                        frame.has_list_item = false;
                    }
                    "ul" => {
                        frame.list_counter = 0;
                        frame.has_list_item = false;
                    }
                    _ => {}
                }
                if paragraph_element {
                    frame.paragraph_style = None;
                }
                if tag.name == "pre" {
                    frame.preserve_whitespace = true;
                    frame.preserve_newlines = true;
                    frame.paragraph_style = Some("Code Block".into());
                } else if tag.name == "code" {
                    frame.named_character = Some("Code".into());
                } else if containing_item.is_some()
                    && tag.name.len() == 2
                    && tag.name.starts_with('h')
                {
                    frame.paragraph_style = Some(
                        format!("Heading{}", tag.name.as_bytes()[1] - b'0')
                            .as_str()
                            .into(),
                    );
                }
                if frame.paragraph_style.is_none() {
                    if let BlockKind::Heading(level) = frame.kind {
                        if builder
                            .style_sheet
                            .block_style(&format!("Heading{level}").as_str().into())
                            .is_none()
                        {
                            frame.paragraph_style =
                                Some(builder.style_sheet.base_paragraph.clone());
                        }
                    }
                }
                if let Some(css) = tag.attribute("style") {
                    if block(&tag.name) {
                        apply_css(css, &mut frame.character, &mut frame.paragraph);
                    } else {
                        // Inline declarations never change the containing block's box.
                        apply_css(css, &mut frame.character, &mut BlockProperties::default());
                    }
                    for (name, value) in cascade_declarations(css) {
                        if name.eq_ignore_ascii_case("white-space") {
                            match value.trim().to_ascii_lowercase().as_str() {
                                "pre" | "pre-wrap" | "break-spaces" => {
                                    frame.preserve_whitespace = true;
                                    frame.preserve_newlines = true;
                                }
                                "normal" | "nowrap" | "initial" => {
                                    frame.preserve_whitespace = false;
                                    frame.preserve_newlines = false;
                                }
                                "pre-line" => {
                                    frame.preserve_whitespace = false;
                                    frame.preserve_newlines = true;
                                }
                                "inherit" | "unset" => {
                                    let parent = stack.last().unwrap();
                                    frame.preserve_whitespace = parent.preserve_whitespace;
                                    frame.preserve_newlines = parent.preserve_newlines;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if block(&tag.name) && frame.paragraph.background.is_some() {
                    frame.character.background =
                        stack.last().and_then(|parent| parent.character.background);
                }
                if let Some(kind) = container_kind.filter(|_| !frame.hidden && !frame.opaque) {
                    let mut owner = super::containers::SourceContainer::new(
                        builder.source_range(token.range.clone()).start..end,
                        kind,
                    );
                    owner.owns_empty_end = true;
                    let mut own_character = CharacterProperties::default();
                    let mut own_paragraph = BlockProperties::default();
                    if let Some(css) = tag.attribute("style") {
                        apply_css(css, &mut own_character, &mut own_paragraph);
                    }
                    own_character.background = None;
                    owner.direct_formatting =
                        super::BlockDirectFormatting::shared(own_paragraph, own_character);
                    frame.container = Some(owners.len());
                    owners.push(owner);
                    // Box declarations belong to this owner, not to each of its
                    // paragraph descendants. Inherited text properties remain.
                    frame.paragraph.clear_box();
                    frame.paragraph.leading_indent = None;
                    frame.paragraph.trailing_indent = None;
                }
                if let Some(lang) = tag.attribute("lang").filter(|s| {
                    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }) {
                    frame.character.language = Some(lang.to_owned());
                }
                if let Some(dir) = tag.attribute("dir").and_then(direction) {
                    frame.character.direction = Some(dir);
                    frame.paragraph.base_direction = Some(dir);
                }
                if !block(&tag.name) {
                    // Inline bidi scopes affect characters, never the containing
                    // paragraph's base direction, including its first text run.
                    frame.paragraph.base_direction = inherited_paragraph_direction;
                }
                // HTML5 drops exactly one initial LF in pre, including a
                // character reference or a preprocessed CR/CRLF. An empty
                // paragraph's typing boundary belongs after that ignored
                // source: inserting before it would turn it into a new visible
                // line. Keep the original spelling untouched.
                let mut inner_start = token.range.end;
                if tag.name == "pre" {
                    let tail = &input.text[inner_start..];
                    let ignored = if tail.starts_with("\r\n") {
                        2
                    } else if tail.starts_with(['\n', '\r']) {
                        1
                    } else {
                        reference(tail, false)
                            .filter(|(value, _)| value == "\n")
                            .map_or(0, |(_, length)| length)
                    };
                    inner_start += ignored;
                }
                if paragraph_element && !inside_pre && !frame.hidden && !frame.opaque {
                    if let Some(range) = pending_break.take() {
                        emit_block_boundary(&mut builder, &stack, range);
                    }
                    builder.kind = frame.kind.clone();
                    builder.paragraph = frame.paragraph.clone();
                    builder.empty_boundary_at(inner_start);
                }
                if paragraph_element && !inside_pre && !frame.hidden && !frame.opaque {
                    builder.kind = frame.kind.clone();
                    builder.paragraph = frame.paragraph.clone();
                    builder.defaults = frame.character.clone();
                    builder.paragraph_style = frame.paragraph_style.clone();
                }
                frame.output_start = builder.text.len();
                frame.source_inner_start = builder.source_range(inner_start..inner_start).start;
                if !void(&tag.name) {
                    stack.push(frame);
                }
            }
            TokenKind::Text => {
                let frame = stack.last().unwrap();
                if frame.hidden || frame.opaque {
                    continue;
                }
                let mut at = token.range.start;
                while at < token.range.end {
                    let (value, consumed) = reference(&input.text[at..token.range.end], false)
                        .unwrap_or_else(|| {
                            let c = input.text[at..].chars().next().unwrap();
                            (
                                if c == '\0' {
                                    "\u{fffd}".into()
                                } else {
                                    c.to_string()
                                },
                                c.len_utf8(),
                            )
                        });
                    let range = at..at + consumed;
                    at += consumed;
                    let value = if value == "\r" { " ".into() } else { value };
                    let hard_break = frame.preserve_newlines && value == "\n";
                    if !frame.preserve_whitespace
                        && !hard_break
                        && value.bytes().all(css_space)
                        && !(markdown_references && input.text[range.clone()].starts_with("&#"))
                    {
                        if !builder.line_is_empty() && pending_break.is_none() {
                            if let Some(space) =
                                pending_space.as_mut().filter(|r| r.end == range.start)
                            {
                                space.end = range.end;
                            } else if pending_space.is_none() {
                                pending_space = Some(range);
                                pending_space_style = frame.character.clone();
                                pending_space_named = frame.named_character.clone();
                                pending_space_is_segment_break = false;
                            }
                            if value == "\n" && !pending_space_is_segment_break {
                                pending_space_style = frame.character.clone();
                                pending_space_named = frame.named_character.clone();
                                pending_space_is_segment_break = true;
                            }
                        }
                        continue;
                    }
                    if let Some(range) = pending_break.take() {
                        emit_block_boundary(&mut builder, &stack, range);
                    }
                    let paragraph_frame = stack
                        .iter()
                        .find(|frame| frame.name == "pre")
                        .unwrap_or(frame);
                    builder.kind = paragraph_frame.kind.clone();
                    builder.paragraph = paragraph_frame.paragraph.clone();
                    builder.paragraph_style = paragraph_frame.paragraph_style.clone();
                    builder.defaults = stack
                        .iter()
                        .rev()
                        .find(|ancestor| paragraph(&ancestor.name))
                        .map(|ancestor| ancestor.character.clone())
                        .unwrap_or_default();
                    builder.named_character = frame.named_character.clone();
                    if hard_break {
                        pending_space = None;
                    }
                    if let Some(range) = pending_space.take() {
                        let named = std::mem::replace(
                            &mut builder.named_character,
                            pending_space_named.take(),
                        );
                        builder.emit(" ", range, &pending_space_style);
                        builder.named_character = named;
                    }
                    if hard_break {
                        builder.hard_break(range);
                    } else {
                        builder.emit(&value, range, &frame.character);
                    }
                    paragraph_seen = true;
                }
            }
        }
    }
    let mut result = builder.finish(start, end);
    result.install_inline_images(inline_images);
    result.install_source_containers(owners);
    super::links::style_html_links(&mut result, input, &lexical_tokens);
    result
}

pub(super) fn reference(input: &str, attribute: bool) -> Option<(String, usize)> {
    let input = input.strip_prefix('&')?;
    if let Some(numeric) = input.strip_prefix('#') {
        let (radix, digits, prefix) = if numeric.starts_with(['x', 'X']) {
            (16, &numeric[1..], 2)
        } else {
            (10, numeric, 1)
        };
        let len = digits
            .bytes()
            .take_while(|c| {
                if radix == 16 {
                    c.is_ascii_hexdigit()
                } else {
                    c.is_ascii_digit()
                }
            })
            .count();
        if len == 0 {
            return None;
        }
        let code = u32::from_str_radix(&digits[..len], radix)
            .ok()
            .unwrap_or(0xfffd);
        let code = if (0x80..=0x9f).contains(&code) {
            windows_1252(code as u8) as u32
        } else {
            code
        };
        let c = char::from_u32(code)
            .filter(|c| *c != '\0')
            .unwrap_or('\u{fffd}');
        return Some((
            c.to_string(),
            1 + prefix + len + usize::from(digits.as_bytes().get(len) == Some(&b';')),
        ));
    }
    // Reuse the WHATWG table already maintained by our HTML parser. Its
    // zero-valued entries are prefix sentinels, not complete references.
    let limit = input
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b';')
        .take(33)
        .count();
    for length in (1..=limit).rev() {
        let candidate = &input[..length];
        if let Some(&(first, second)) = html5ever::data::NAMED_ENTITIES.get(candidate) {
            if first == 0 {
                continue;
            }
            if attribute
                && !candidate.ends_with(';')
                && input
                    .as_bytes()
                    .get(length)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'=')
            {
                return None;
            }
            let value = [first, second]
                .into_iter()
                .filter(|&code| code != 0)
                .map(|code| char::from_u32(code).expect("WHATWG references contain valid scalars"))
                .collect();
            return Some((value, length + 1));
        }
    }
    None
}
fn decode_references(input: &str, attribute: bool) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < input.len() {
        if let Some((value, len)) = reference(&input[at..], attribute) {
            out.push_str(&value);
            at += len;
        } else {
            let c = input[at..].chars().next().unwrap();
            out.push(c);
            at += c.len_utf8();
        }
    }
    out
}
pub(super) fn direction(value: &str) -> Option<WritingDirection> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ltr" => Some(WritingDirection::LeftToRight),
        "rtl" => Some(WritingDirection::RightToLeft),
        "auto" => Some(WritingDirection::Natural),
        _ => None,
    }
}
fn length(value: &str) -> Option<f32> {
    let v = value.trim();
    let (number, scale) = if let Some(n) = v.strip_suffix("pt") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("px") {
        (n, 0.75)
    } else if v == "0" {
        (v, 1.0)
    } else {
        return None;
    };
    number
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n * scale)
}
fn color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(body) = value
        .strip_prefix("color(srgb ")
        .and_then(|s| s.strip_suffix(')'))
    {
        let values = body
            .split_ascii_whitespace()
            .filter(|s| *s != "/")
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if !matches!(values.len(), 3 | 4)
            || (values.len() == 4) != (body.matches('/').count() == 1)
            || body.matches('/').count() > 1
            || values
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return None;
        }
        return Some(Color {
            red: values[0],
            green: values[1],
            blue: values[2],
            alpha: values.get(3).copied().unwrap_or(1.0),
        });
    }
    if let Some(body) = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let values = body
            .split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !matches!(values.len(), 3 | 4) {
            return None;
        }
        if body.contains(',') {
            if body.contains('/') || body.split(',').count() != values.len() {
                return None;
            }
        } else if (values.len() == 4) != (body.matches('/').count() == 1)
            || body.matches('/').count() > 1
        {
            return None;
        }
        let component = |at: usize| -> Option<f32> {
            let value = values[at];
            let percentage = value.ends_with('%');
            let raw = value.trim_end_matches('%').parse::<f32>().ok()?;
            let scale = if percentage {
                100.0
            } else if at < 3 {
                255.0
            } else {
                1.0
            };
            raw.is_finite().then_some((raw / scale).clamp(0.0, 1.0))
        };
        return Some(Color {
            red: component(0)?,
            green: component(1)?,
            blue: component(2)?,
            alpha: if values.len() == 4 {
                component(3)?
            } else {
                1.0
            },
        });
    }
    let hex = match value.as_str() {
        "black" => "000000",
        "white" => "ffffff",
        "red" => "ff0000",
        "green" => "008000",
        "blue" => "0000ff",
        "yellow" => "ffff00",
        "gray" | "grey" => "808080",
        "silver" => "c0c0c0",
        "maroon" => "800000",
        "purple" => "800080",
        "fuchsia" => "ff00ff",
        "lime" => "00ff00",
        "olive" => "808000",
        "navy" => "000080",
        "teal" => "008080",
        "aqua" => "00ffff",
        "transparent" => {
            return Some(Color {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 0.0,
            })
        }
        _ => value.strip_prefix('#')?,
    };
    let expanded;
    let hex = if hex.len() == 3 || hex.len() == 4 {
        expanded = hex.chars().flat_map(|c| [c, c]).collect::<String>();
        expanded.as_str()
    } else {
        hex
    };
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at| {
        u8::from_str_radix(&hex[at..at + 2], 16)
            .ok()
            .map(|n| f32::from(n) / 255.0)
    };
    Some(Color {
        red: channel(0)?,
        green: channel(2)?,
        blue: channel(4)?,
        alpha: if hex.len() == 8 { channel(6)? } else { 1.0 },
    })
}
fn split_css(css: &str, separator: char) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut depth = 0;
    let mut escaped = false;
    for (at, c) in css.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q || matches!(c, '\n' | '\r' | '\x0c') {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            c if c == separator && depth == 0 => {
                chunks.push(&css[start..at]);
                start = at + c.len_utf8();
            }
            _ => {}
        }
    }
    chunks.push(&css[start..]);
    chunks
}
fn css_unescape(value: &str) -> Option<String> {
    let value = value.trim();
    let quote = value.chars().next().filter(|c| matches!(c, '\'' | '"'));
    let value = if let Some(quote) = quote {
        value.strip_prefix(quote)?.strip_suffix(quote)?
    } else {
        value
    };
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            if quote.is_some() && (quote == Some(c) || matches!(c, '\n' | '\r' | '\x0c')) {
                return None;
            }
            result.push(c);
            continue;
        }
        let first = chars.next()?;
        if first.is_ascii_hexdigit() {
            let mut hex = first.to_string();
            while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
                hex.push(chars.next()?);
            }
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            result.push(
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)
                    .filter(|c| *c != '\0')
                    .unwrap_or('\u{fffd}'),
            );
        } else if first == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else if !matches!(first, '\n' | '\x0c') {
            result.push(first);
        }
    }
    Some(result)
}
fn css_without_comments(css: &str) -> String {
    let mut output = String::new();
    let mut at = 0;
    let mut quote = None;
    let mut escaped = false;
    while at < css.len() {
        let c = css[at..].chars().next().unwrap();
        if quote.is_none() && css[at..].starts_with("/*") {
            at = css[at + 2..]
                .find("*/")
                .map(|n| at + 2 + n + 2)
                .unwrap_or(css.len());
            output.push(' ');
            continue;
        }
        output.push(c);
        at += c.len_utf8();
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        }
    }
    output
}
pub(super) fn declarations(css: &str) -> Vec<(&str, &str)> {
    split_css(css, ';')
        .into_iter()
        .filter_map(|s| s.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}
pub(super) fn cascade_declarations(css: &str) -> Vec<(String, String)> {
    let css = css_without_comments(css);
    let mut declarations = declarations(&css)
        .into_iter()
        .filter_map(|(key, value)| {
            let key = css_unescape(key)?.to_ascii_lowercase();
            if !key.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-') {
                return None;
            }
            let pieces = split_css(value, '!');
            let important = pieces.len() == 2 && pieces[1].trim().eq_ignore_ascii_case("important");
            Some((
                key,
                if important {
                    pieces[0].trim().to_owned()
                } else {
                    value.to_owned()
                },
                important,
            ))
        })
        .collect::<Vec<_>>();
    declarations.sort_by_key(|(_, _, important)| *important);
    declarations
        .into_iter()
        .map(|(key, value, _)| (key, value))
        .collect()
}
fn font_families(value: &str) -> Option<Vec<String>> {
    split_css(value, ',')
        .into_iter()
        .map(|part| {
            let part = part.trim();
            if part.starts_with(['\'', '"']) {
                return css_unescape(part).filter(|name| !name.is_empty());
            }
            let name = css_unescape(part)?;
            if name.is_empty()
                || name.chars().any(|c| {
                    !(c.is_alphanumeric() || c == '-' || c == '_' || c.is_ascii_whitespace())
                })
            {
                return None;
            }
            if part
                .split_ascii_whitespace()
                .any(|word| word.as_bytes().first().is_some_and(u8::is_ascii_digit))
            {
                return None;
            }
            let name = name.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
            ) {
                return None;
            }
            Some(name)
        })
        .collect()
}
fn font_features(value: &str) -> Option<BTreeMap<String, u32>> {
    let mut features = BTreeMap::new();
    if value.eq_ignore_ascii_case("normal") {
        return Some(features);
    }
    for part in split_css(value, ',') {
        let part = part.trim();
        let quote = part.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let mut escaped = false;
        let mut end = None;
        for (at, c) in part.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == quote {
                end = Some(at + 1);
                break;
            }
        }
        let end = end?;
        let tag = css_unescape(&part[..end])?;
        if tag.len() != 4 || !tag.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return None;
        }
        let count = match part[end..].trim().to_ascii_lowercase().as_str() {
            "" | "on" => 1,
            "off" => 0,
            value => value.parse::<u32>().ok()?,
        };
        features.insert(tag, count);
    }
    Some(features)
}
fn font_axis_settings(value: &str) -> Option<BTreeMap<String, f32>> {
    let mut axes = BTreeMap::new();
    if value.eq_ignore_ascii_case("normal") {
        return Some(axes);
    }
    for part in split_css(value, ',') {
        let part = part.trim();
        let quote = part.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let mut escaped = false;
        let mut end = None;
        for (at, c) in part.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == quote {
                end = Some(at + 1);
                break;
            }
        }
        let end = end?;
        let tag = css_unescape(&part[..end])?;
        if tag.len() != 4 || !tag.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return None;
        }
        let count = part[end..]
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())?;
        if axes.len() >= 64 && !axes.contains_key(&tag) {
            return None;
        }
        axes.insert(tag, count);
    }
    Some(axes)
}
fn css_box_sides<T: Clone>(values: &[T]) -> Option<[T; 4]> {
    Some(match values {
        [all] => [all.clone(), all.clone(), all.clone(), all.clone()],
        [vertical, horizontal] => [
            vertical.clone(),
            horizontal.clone(),
            vertical.clone(),
            horizontal.clone(),
        ],
        [top, horizontal, bottom] => [
            top.clone(),
            horizontal.clone(),
            bottom.clone(),
            horizontal.clone(),
        ],
        [top, right, bottom, left] => [top.clone(), right.clone(), bottom.clone(), left.clone()],
        _ => return None,
    })
}

/// Apply the physical box subset supported by the native block layout. Unsupported
/// CSS remains in source but never receives an invented interpretation.
fn apply_box_css(key: &str, value: &str, block: &mut BlockProperties) -> bool {
    let words = split_css(value, ' ')
        .into_iter()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    let nonnegative = |value: &str| length(value).filter(|v| *v >= 0.0);
    macro_rules! sides {
        ($field:ident, $top:ident, $right:ident, $bottom:ident, $left:ident, $parse:expr) => {
            if let Some(values) = words
                .iter()
                .map(|value| ($parse)(value))
                .collect::<Option<Vec<_>>>()
                .and_then(|values| css_box_sides(&values))
            {
                [block.$top, block.$right, block.$bottom, block.$left] = values.map(Some);
            }
        };
    }
    match key {
        "margin" => sides!(
            margin,
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
            length
        ),
        "padding" => sides!(
            padding,
            padding_top,
            padding_right,
            padding_bottom,
            padding_left,
            nonnegative
        ),
        "border-width" => sides!(
            border,
            border_top_width,
            border_right_width,
            border_bottom_width,
            border_left_width,
            nonnegative
        ),
        "border-color" => sides!(
            border,
            border_top_color,
            border_right_color,
            border_bottom_color,
            border_left_color,
            color
        ),
        "margin-top" => {
            if let Some(v) = length(value) {
                block.margin_top = Some(v);
            }
        }
        "padding-top" => {
            if let Some(v) = nonnegative(value) {
                block.padding_top = Some(v);
            }
        }
        "border-top-width" => {
            if let Some(v) = nonnegative(value) {
                block.border_top_width = Some(v);
            }
        }
        "border-top-color" => {
            if value == "currentcolor" {
                block.border_top_color = None;
            } else if let Some(v) = color(value) {
                block.border_top_color = Some(v);
            }
        }
        "border-top-style" if matches!(value, "none" | "hidden") => {
            block.border_top_width = Some(0.0)
        }
        "margin-right" => {
            if let Some(v) = length(value) {
                block.margin_right = Some(v);
            }
        }
        "padding-right" => {
            if let Some(v) = nonnegative(value) {
                block.padding_right = Some(v);
            }
        }
        "border-right-width" => {
            if let Some(v) = nonnegative(value) {
                block.border_right_width = Some(v);
            }
        }
        "border-right-color" => {
            if value == "currentcolor" {
                block.border_right_color = None;
            } else if let Some(v) = color(value) {
                block.border_right_color = Some(v);
            }
        }
        "border-right-style" if matches!(value, "none" | "hidden") => {
            block.border_right_width = Some(0.0)
        }
        "margin-bottom" => {
            if let Some(v) = length(value) {
                block.margin_bottom = Some(v);
            }
        }
        "padding-bottom" => {
            if let Some(v) = nonnegative(value) {
                block.padding_bottom = Some(v);
            }
        }
        "border-bottom-width" => {
            if let Some(v) = nonnegative(value) {
                block.border_bottom_width = Some(v);
            }
        }
        "border-bottom-color" => {
            if value == "currentcolor" {
                block.border_bottom_color = None;
            } else if let Some(v) = color(value) {
                block.border_bottom_color = Some(v);
            }
        }
        "border-bottom-style" if matches!(value, "none" | "hidden") => {
            block.border_bottom_width = Some(0.0)
        }
        "margin-left" => {
            if let Some(v) = length(value) {
                block.margin_left = Some(v);
            }
        }
        "padding-left" => {
            if let Some(v) = nonnegative(value) {
                block.padding_left = Some(v);
            }
        }
        "border-left-width" => {
            if let Some(v) = nonnegative(value) {
                block.border_left_width = Some(v);
            }
        }
        "border-left-color" => {
            if value == "currentcolor" {
                block.border_left_color = None;
            } else if let Some(v) = color(value) {
                block.border_left_color = Some(v);
            }
        }
        "border-left-style" if matches!(value, "none" | "hidden") => {
            block.border_left_width = Some(0.0)
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let none = words
                .iter()
                .any(|value| matches!(*value, "none" | "hidden"));
            let solid = words.contains(&"solid");
            if !none && !solid {
                return true;
            }
            let width = if none {
                Some(0.0)
            } else {
                words.iter().find_map(|value| nonnegative(value))
            };
            let color = words.iter().find_map(|value| color(value));
            for side in ["top", "right", "bottom", "left"] {
                if key != "border" && key != format!("border-{side}") {
                    continue;
                }
                match side {
                    "top" => {
                        block.border_top_width = width;
                        block.border_top_color = color;
                    }
                    "right" => {
                        block.border_right_width = width;
                        block.border_right_color = color;
                    }
                    "bottom" => {
                        block.border_bottom_width = width;
                        block.border_bottom_color = color;
                    }
                    "left" => {
                        block.border_left_width = width;
                        block.border_left_color = color;
                    }
                    _ => unreachable!(),
                }
            }
        }
        "border-style" if matches!(value, "none" | "hidden") => {
            block.border_top_width = Some(0.0);
            block.border_right_width = Some(0.0);
            block.border_bottom_width = Some(0.0);
            block.border_left_width = Some(0.0);
        }
        _ => return false,
    }
    true
}

pub(super) fn apply_css(
    css: &str,
    character: &mut CharacterProperties,
    paragraph: &mut BlockProperties,
) {
    let mut base_axes = None;
    for (key, value) in cascade_declarations(css) {
        let value = value.as_str();
        let lower = value.to_ascii_lowercase();
        if apply_box_css(key.as_str(), &lower, paragraph) {
            continue;
        }
        match key.as_str() {
            "font-family" => {
                if let Some(values) = font_families(value) {
                    character.font_families = Some(values);
                }
            }
            "--viem-font-face" => {
                character.font_face = css_unescape(value).filter(|name| name.len() <= 1024 && !name.chars().any(char::is_control));
            }
            "font-size" => {
                if let Some(size) = length(&lower).filter(|n| *n > 0.0) {
                    character.size = Some(size.into());
                }
            }
            "--viem-bold" => character.bold = value.parse().ok(),
            "--viem-base-weight" => {
                character.weight = value.parse::<u16>().ok().filter(|n| (1..=1000).contains(n))
            }
            "font-weight" => {
                if let Some(weight) = match lower.as_str() {
                    "normal" => Some(400),
                    "bold" => Some(700),
                    _ => lower.parse::<u16>().ok().filter(|n| (1..=1000).contains(n)),
                } {
                    character.weight = Some(weight);
                    character.bold = None;
                }
            }
            "font-style" => {
                if let Some(slant) = match lower.as_str() {
                    "normal" => Some(FontSlant::Upright),
                    "italic" => Some(FontSlant::Italic),
                    "oblique" => Some(FontSlant::Oblique),
                    _ => None,
                } {
                    character.slant = Some(slant);
                }
            }
            "color" => {
                if let Some(c) = color(value) {
                    character.foreground = Some(c);
                }
            }
            "background-color" | "background" => {
                if let Some(c) = color(value) {
                    character.background = Some(c);
                    paragraph.background = Some(c);
                }
            }
            "text-decoration" | "text-decoration-line" => {
                let words = lower.split_ascii_whitespace().collect::<Vec<_>>();
                if !words.is_empty()
                    && (words == ["none"]
                        || words
                            .iter()
                            .all(|s| matches!(*s, "underline" | "line-through")))
                    && words
                        .iter()
                        .enumerate()
                        .all(|(index, word)| !words[..index].contains(word))
                {
                    character.underline = Some(words.contains(&"underline"));
                    character.strikethrough = Some(words.contains(&"line-through"));
                }
            }
            "letter-spacing" => {
                if let Some(n) = if lower == "normal" {
                    Some(0.0)
                } else {
                    length(&lower)
                } {
                    character.letter_spacing = Some(n);
                }
            }
            "direction" => {
                if let Some(d) =
                    direction(&lower).filter(|direction| *direction != WritingDirection::Natural)
                {
                    character.direction = Some(d);
                    paragraph.base_direction = Some(d);
                }
            }
            "margin-block-start" => {
                if let Some(n) = length(&lower) {
                    paragraph.margin_top = Some(n);
                }
            }
            "margin-block-end" => {
                if let Some(n) = length(&lower) {
                    paragraph.margin_bottom = Some(n);
                }
            }
            "margin-inline-start" => {
                if let Some(n) = length(&lower) {
                    paragraph.leading_indent = Some(n);
                }
            }
            "margin-inline-end" => {
                if let Some(n) = length(&lower) {
                    paragraph.trailing_indent = Some(n);
                }
            }
            "text-indent" => {
                if let Some(n) = length(&lower) {
                    paragraph.first_line_indent = Some(n);
                }
            }
            "text-align" => {
                if let Some(n) = match lower.as_str() {
                    "start" => Some(ParagraphAlignment::Start),
                    "end" => Some(ParagraphAlignment::End),
                    "center" => Some(ParagraphAlignment::Center),
                    _ => None,
                } {
                    paragraph.alignment = Some(n);
                }
            }
            "line-height" => {
                if let Some(n) = if lower == "normal" {
                    Some(LineSpacing::Normal)
                } else if let Some(n) = length(&lower).filter(|n| *n >= 0.0) {
                    Some(LineSpacing::Exact(n))
                } else {
                    lower
                        .parse::<f32>()
                        .ok()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .map(LineSpacing::Multiplier)
                } {
                    paragraph.line_spacing = Some(n);
                }
            }
            "font-variation-settings" => {
                if let Some(axes) = font_axis_settings(value) {
                    character.font_axes = Some(axes);
                }
            }
            "--viem-base-font-axes" => base_axes = font_axis_settings(value),
            "font-feature-settings" => {
                if let Some(features) = font_features(value) {
                    character.open_type_features = Some(features);
                }
            }
            _ => {}
        }
    }
    if let Some(axes) = base_axes {
        character.font_axes = Some(axes);
    }
}

/// Version-one canonical character declarations. All absolute lengths are
/// points; property order follows the normalized schema. Unknown declarations
/// are never copied into a newly owned formatting construct.
pub(super) fn character_css(properties: &CharacterProperties) -> String {
    let mut declarations = Vec::new();
    let css_string = |value: &str| {
        value
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\a ")
            .replace('\r', "\\d ")
            .replace('\u{c}', "\\c ")
    };
    if let Some(families) = &properties.font_families {
        declarations.push(format!(
            "font-family: {}",
            families
                .iter()
                .map(|name| {
                    // CSS generic families are keywords. Quoting them would
                    // request a literal installed face and lose the fallback.
                    if matches!(
                        name.to_ascii_lowercase().as_str(),
                        "serif"
                            | "sans-serif"
                            | "monospace"
                            | "cursive"
                            | "fantasy"
                            | "system-ui"
                            | "ui-serif"
                            | "ui-sans-serif"
                            | "ui-monospace"
                            | "ui-rounded"
                            | "emoji"
                            | "math"
                            | "fangsong"
                    ) {
                        name.clone()
                    } else {
                        format!("'{}'", css_string(name))
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(size) = properties.size {
        declarations.push(match size {
            super::FontSize::Points(value) => format!("font-size: {value}pt"),
            super::FontSize::Percentage(value) => format!("font-size: {value}%"),
        });
    }
    if let Some(bold) = properties.bold {
        let base = properties.weight.unwrap_or(400);
        let weight = if bold {
            base.saturating_add(300).min(1000)
        } else {
            base
        };
        declarations.push(format!("font-weight: {weight}"));
        declarations.push(format!("--viem-base-weight: {base}"));
        declarations.push(format!("--viem-bold: {bold}"));
    } else if let Some(weight) = properties.weight {
        declarations.push(format!("font-weight: {weight}"));
    }
    if let Some(slant) = properties.slant {
        declarations.push(format!(
            "font-style: {}",
            match slant {
                FontSlant::Upright => "normal",
                FontSlant::Italic => "italic",
                FontSlant::Oblique => "oblique",
            }
        ));
    }
    let css_color = |color: Color| {
        let channels = [color.red, color.green, color.blue, color.alpha];
        if channels
            .iter()
            .all(|value| ((value * 255.0).round() / 255.0) == *value)
        {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                (color.red * 255.0).round() as u8,
                (color.green * 255.0).round() as u8,
                (color.blue * 255.0).round() as u8,
                (color.alpha * 255.0).round() as u8
            )
        } else {
            format!(
                "color(srgb {} {} {} / {})",
                color.red, color.green, color.blue, color.alpha
            )
        }
    };
    if let Some(color) = properties.foreground {
        declarations.push(format!("color: {}", css_color(color)));
    }
    if let Some(color) = properties.background {
        declarations.push(format!("background-color: {}", css_color(color)));
    }
    if properties.underline.is_some() || properties.strikethrough.is_some() {
        let mut values = Vec::new();
        if properties.underline == Some(true) {
            values.push("underline");
        }
        if properties.strikethrough == Some(true) {
            values.push("line-through");
        }
        if values.is_empty() {
            values.push("none");
        }
        declarations.push(format!("text-decoration-line: {}", values.join(" ")));
    }
    if let Some(direction) = properties
        .direction
        .filter(|value| *value != WritingDirection::Natural)
    {
        declarations.push(format!(
            "direction: {}",
            match direction {
                WritingDirection::RightToLeft => "rtl",
                _ => "ltr",
            }
        ));
    }
    if let Some(face) = &properties.font_face {
        declarations.push(format!("--viem-font-face: '{}'", css_string(face)));
    }
    if let Some(axes) = &properties.font_axes {
        let settings = |values: &BTreeMap<String, f32>| {
            if values.is_empty() {
                "normal".to_owned()
            } else {
                values
                    .iter()
                    .map(|(tag, value)| format!("'{}' {value}", css_string(tag)))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let mut rendered = axes.clone();
        if properties.bold == Some(true) {
            if let Some(weight) = rendered.get_mut("wght") {
                *weight = (*weight + 300.0).min(1000.0);
            }
        }
        if properties.slant.is_some_and(|s| s != FontSlant::Upright) {
            if let Some(italic) = rendered.get_mut("ital") {
                *italic = 1.0;
            } else if let Some(slant) = rendered.get_mut("slnt") {
                *slant = slant.min(-12.0);
            }
        }
        declarations.push(format!("font-variation-settings: {}", settings(&rendered)));
        if rendered != *axes {
            declarations.push(format!("--viem-base-font-axes: {}", settings(axes)));
        }
    }
    if let Some(features) = &properties.open_type_features {
        declarations.push(format!(
            "font-feature-settings: {}",
            if features.is_empty() {
                "normal".into()
            } else {
                features
                    .iter()
                    .map(|(tag, value)| format!("'{}' {value}", css_string(tag)))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
    }
    if let Some(spacing) = properties.letter_spacing {
        declarations.push(format!("letter-spacing: {spacing}pt"));
    }
    declarations.join("; ")
}
#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    fn variable_axes_css_preserves_base_coordinates_and_emphasis() {
        let properties = CharacterProperties {
            font_face: Some("Condensed Light".into()),
            font_axes: Some(BTreeMap::from([
                ("wght".into(), 450.25),
                ("wdth".into(), 87.5),
                ("slnt".into(), -4.0),
            ])),
            weight: Some(450),
            bold: Some(true),
            slant: Some(FontSlant::Italic),
            ..Default::default()
        };
        let css = character_css(&properties);
        assert!(css.contains("'wght' 750.25"));
        assert!(css.contains("'slnt' -12"));
        let mut parsed = CharacterProperties::default();
        apply_css(&css, &mut parsed, &mut BlockProperties::default());
        assert_eq!(parsed.font_face, properties.font_face);
        assert_eq!(parsed.font_axes, properties.font_axes);
        assert_eq!(parsed.bold, properties.bold);
        assert_eq!(parsed.weight, properties.weight);
        for invalid in ["'bad' 1", "'wght' NaN", "'wght' inf", "'wght' on"] {
            assert!(font_axis_settings(invalid).is_none());
        }
        assert_eq!(font_axis_settings("normal"), Some(BTreeMap::new()));
    }

    #[test]
    fn named_references_retain_the_complete_imported_mapping() {
        let mut entries = html5ever::data::NAMED_ENTITIES
            .entries()
            .filter(|(_, value)| value.0 != 0)
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        entries.sort_unstable();
        assert_eq!(entries.len(), 2231);
        let mut serialized = String::new();
        for name in entries {
            let (value, consumed) = reference(&format!("&{name}"), false).unwrap();
            assert_eq!(consumed, name.len() + 1);
            serialized.push_str(name);
            serialized.push('\0');
            serialized.push_str(&value);
            serialized.push('\0');
        }
        // Frozen from the former local table, including legacy names and
        // references which decode to two scalars. A dependency update must
        // preserve these mappings unless the behavior change is intentional.
        assert_eq!(
            super::super::SourceArtifactDigest::from_bytes(serialized.as_bytes()).as_bytes(),
            &[
                0xfe, 0xcb, 0x04, 0x24, 0x88, 0x4c, 0x51, 0x7f, 0xaf, 0x8d, 0xcd, 0xac, 0xa4, 0x58,
                0x3b, 0x3c, 0x9e, 0xf9, 0x2a, 0x53, 0x0c, 0x54, 0x04, 0xe0, 0x80, 0x97, 0xc1, 0x8c,
                0xc0, 0xb8, 0x0b, 0x97,
            ],
        );
    }

    #[test]
    fn named_reference_prefixes_and_attribute_rules_remain_distinct() {
        assert_eq!(reference("&notin;tail", false), Some(("∉".into(), 7)));
        assert_eq!(reference("&notinX", false), Some(("¬".into(), 4)));
        assert_eq!(reference("&notinX", true), None);
        assert_eq!(reference("&amp=", true), None);
        assert_eq!(reference("&amp;=", true), Some(("&".into(), 5)));
        assert_eq!(reference("&CounterClockwise", false), None);
        assert_eq!(reference("&apos", false), None);
    }
}

fn windows_1252(value: u8) -> char {
    const C1: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    if (0x80..=0x9f).contains(&value) {
        C1[(value - 0x80) as usize]
    } else {
        char::from(value)
    }
}
