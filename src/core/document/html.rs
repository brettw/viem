//! Passive source-preserving HTML projection. The tokenizer retains source
//! extents, spelling and duplicate attributes; semantic parsing never executes
//! code or resolves resources. Edits patch the original source, not a serializer.
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
}
impl Tag {
    pub(super) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
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
fn void(name: &str) -> bool {
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
fn paragraph(name: &str) -> bool {
    matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li")
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
fn hidden(name: &str) -> bool {
    matches!(
        name,
        "head" | "script" | "style" | "template" | "title" | "noscript"
    )
}
fn atomic(name: &str) -> bool {
    matches!(
        name,
        "img"
            | "object"
            | "iframe"
            | "embed"
            | "svg"
            | "math"
            | "pre"
            | "textarea"
            | "video"
            | "audio"
            | "canvas"
            | "table"
    )
}

pub(super) fn tokenize(input: &str) -> Vec<Token> {
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
            while at < bytes.len() && space(bytes[at]) {
                at += 1;
            }
            let mut value = String::new();
            if bytes.get(at) == Some(&b'=') {
                at += 1;
                while at < bytes.len() && space(bytes[at]) {
                    at += 1;
                }
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
            }
            if !key.is_empty() {
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
        if !end
            && matches!(
                name.as_str(),
                "script"
                    | "style"
                    | "title"
                    | "textarea"
                    | "xmp"
                    | "iframe"
                    | "noembed"
                    | "noframes"
            )
        {
            raw = Some(name.clone());
        }
        tokens.push(Token {
            range: start..at,
            kind: TokenKind::Tag(Tag {
                name,
                end,
                attributes,
            }),
        });
    }
    tokens
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
    item_paragraphs: usize,
    paragraph_style: Option<super::StyleId>,
    named_character: Option<super::StyleId>,
    preserve_whitespace: bool,
    output_start: usize,
    source_inner_start: usize,
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
            item_paragraphs: 0,
            paragraph_style: None,
            named_character: None,
            preserve_whitespace: false,
            output_start: 0,
            source_inner_start: 0,
        }
    }
}

pub(super) fn project(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
) -> FormattedDocument {
    let tokens = super::html5_tree::tokens(&input.text);
    let mut builder = Builder::new(input, revision);
    builder.style_sheet = super::html_styles::read_with_semantics(&input.text, &tokens).sheet;
    let mut stack = vec![Frame::default()];
    let mut pending_break: Option<Range<usize>> = None;
    let mut pending_space: Option<Range<usize>> = None;
    let mut pending_space_style = CharacterProperties::default();
    let mut pending_space_named = None;
    let mut paragraph_seen = false;
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
                if !frame.preserve_whitespace && value.bytes().all(space) {
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
                        }
                    }
                    continue;
                }
                if let Some(range) = pending_break.take() {
                    builder.paragraph_break(range);
                }
                builder.kind = frame.kind.clone();
                builder.paragraph = frame.paragraph.clone();
                builder.paragraph_style = frame.paragraph_style.clone();
                builder.named_character = frame.named_character.clone();
                if let Some(range) = pending_space.take() {
                    let named =
                        std::mem::replace(&mut builder.named_character, pending_space_named.take());
                    builder.emit(" ", range, &pending_space_style);
                    builder.named_character = named;
                }
                if !mapped {
                    builder.emit_read_only(&value);
                } else if frame.preserve_whitespace && value == "\n" {
                    builder.hard_break(token.range);
                } else {
                    builder.emit(&value, token.range, &frame.character);
                }
                paragraph_seen = true;
            }
            TokenKind::Tag(tag) => {
                if tag.end {
                    if let Some(index) = stack.iter().rposition(|f| f.name == tag.name) {
                        let was_hidden = stack.last().unwrap().hidden;
                        let closed = &stack[index];
                        if !closed.hidden
                            && !closed.opaque
                            && closed.output_start == builder.text.len()
                        {
                            builder.retain_empty_boundary(
                                closed.source_inner_start,
                                &closed.character,
                            );
                        }
                        stack.truncate(index.max(1));
                        if !was_hidden && block(&tag.name) {
                            pending_space = None;
                            if !builder.line_is_empty() || paragraph(&tag.name) {
                                pending_break = Some(token.range.clone());
                            }
                        }
                    }
                    continue;
                }
                let mut frame = stack.last().cloned().unwrap_or_default();
                frame.name = tag.name.clone();
                frame.hidden |= hidden(&tag.name);
                frame.opaque |= atomic(&tag.name);
                if !frame.hidden && !stack.last().unwrap().opaque && atomic(&tag.name) {
                    if let Some(range) = pending_break.take() {
                        builder.paragraph_break(range);
                    }
                    builder.emit_read_only("\u{fffc}");
                    paragraph_seen = true;
                }
                if tag.name == "br" && !frame.hidden && !frame.opaque {
                    pending_space = None;
                    pending_break = None;
                    builder.hard_break(token.range.clone());
                }
                let containing_item = if matches!(
                    tag.name.as_str(),
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    stack.iter().rposition(|frame| frame.name == "li")
                } else {
                    None
                };
                let first_item_paragraph = containing_item.is_some_and(|index| {
                    stack[index].item_paragraphs == 0
                        && stack[index].output_start == builder.text.len()
                });
                if block(&tag.name) && !frame.hidden && !frame.opaque {
                    pending_space = None;
                    if paragraph(&tag.name) {
                        if let Some(range) = pending_break.take() {
                            builder.paragraph_break(range);
                        } else if paragraph_seen && !first_item_paragraph {
                            builder.paragraph_break(token.range.clone());
                        }
                        paragraph_seen = true;
                    } else if !builder.line_is_empty() {
                        pending_break = Some(token.range.clone());
                    }
                    frame.kind = if tag.name.len() == 2 && tag.name.starts_with('h') {
                        BlockKind::Heading(tag.name.as_bytes()[1] - b'0')
                    } else if tag.name == "li" {
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
                            }
                        } else {
                            BlockKind::Paragraph
                        }
                    } else if let Some(index) = containing_item {
                        stack[index].item_paragraphs += 1;
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
                            }
                        } else {
                            BlockKind::Paragraph
                        }
                    } else {
                        BlockKind::Paragraph
                    };
                    if tag.name == "li" {
                        frame.item_paragraphs = 0;
                    }
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
                if paragraph(&tag.name) {
                    frame.paragraph_style = None;
                }
                if let Some(classes) = tag.attribute("class") {
                    if paragraph(&tag.name) {
                        frame.paragraph_style =
                            super::html_styles::select_class(&builder.style_sheet, classes, false);
                    } else if tag.name != "body" {
                        if let Some(id) =
                            super::html_styles::select_class(&builder.style_sheet, classes, true)
                        {
                            let named =
                                super::html_styles::character_chain(&builder.style_sheet, &id);
                            super::html_styles::remove_named_overrides(
                                &mut frame.character,
                                &named,
                            );
                            frame.named_character = Some(id);
                        }
                    }
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
                    apply_css(css, &mut frame.character, &mut frame.paragraph);
                    let paragraph_style =
                        frame
                            .paragraph_style
                            .clone()
                            .unwrap_or_else(|| match frame.kind {
                                BlockKind::Heading(level) => {
                                    format!("Heading{level}").as_str().into()
                                }
                                BlockKind::ListItem { level, .. } => {
                                    format!("List{}", u16::from(level) + 1).as_str().into()
                                }
                                _ => builder.style_sheet.base_paragraph.clone(),
                            });
                    let document_style = super::DocumentStyleAssignment::new(
                        builder.style_sheet.base_document.clone(),
                    );
                    if let Ok(resolved) = builder.style_sheet.resolve_assigned_paragraph_style(
                        &document_style,
                        &paragraph_style,
                        &frame.paragraph,
                        &CharacterProperties::default(),
                        frame.named_character.as_ref(),
                        &frame.character,
                    ) {
                        if let Some(shift) = relative_baseline(css, resolved.character.size) {
                            frame.character.baseline_shift = Some(shift);
                        }
                    }
                    for (name, value) in cascade_declarations(css) {
                        if name.eq_ignore_ascii_case("white-space") {
                            match value.trim().to_ascii_lowercase().as_str() {
                                "pre" | "pre-wrap" | "break-spaces" => {
                                    frame.preserve_whitespace = true
                                }
                                "normal" => frame.preserve_whitespace = false,
                                _ => {}
                            }
                        }
                    }
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
                if paragraph(&tag.name) && !frame.hidden && !frame.opaque {
                    if let Some(range) = pending_break.take() {
                        builder.paragraph_break(range);
                    }
                    builder.kind = frame.kind.clone();
                    builder.paragraph = frame.paragraph.clone();
                    if tag.name == "li" {
                        builder.marker_origin = Some(token.range.clone());
                    } else if containing_item.is_none() {
                        builder.marker_origin = None;
                    }
                    builder.emit_list_marker();
                    builder.empty_boundary_at(token.range.end);
                }
                if paragraph(&tag.name) && !frame.hidden && !frame.opaque {
                    builder.kind = frame.kind.clone();
                    builder.paragraph = frame.paragraph.clone();
                    builder.defaults = frame.character.clone();
                    builder.paragraph_style = frame.paragraph_style.clone();
                }
                frame.output_start = builder.text.len();
                frame.source_inner_start = builder.source_range(token.range.clone()).end;
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
                    if !frame.preserve_whitespace && value.bytes().all(space) {
                        if !builder.line_is_empty() && pending_break.is_none() {
                            if let Some(space) =
                                pending_space.as_mut().filter(|r| r.end == range.start)
                            {
                                space.end = range.end;
                            } else if pending_space.is_none() {
                                pending_space = Some(range);
                                pending_space_style = frame.character.clone();
                                pending_space_named = frame.named_character.clone();
                            }
                        }
                        continue;
                    }
                    if let Some(range) = pending_break.take() {
                        builder.paragraph_break(range);
                    }
                    builder.kind = frame.kind.clone();
                    builder.paragraph = frame.paragraph.clone();
                    builder.paragraph_style = frame.paragraph_style.clone();
                    builder.named_character = frame.named_character.clone();
                    if let Some(range) = pending_space.take() {
                        let named = std::mem::replace(
                            &mut builder.named_character,
                            pending_space_named.take(),
                        );
                        builder.emit(" ", range, &pending_space_style);
                        builder.named_character = named;
                    }
                    if frame.preserve_whitespace && value == "\n" {
                        builder.hard_break(range);
                    } else {
                        builder.emit(&value, range, &frame.character);
                    }
                    paragraph_seen = true;
                }
            }
        }
    }
    builder.finish(start, end)
}

/// Author list changes by replacing only paragraph delimiters and inserting
/// the required list containers. Every body byte and unrelated tag survives.
pub(super) fn list_enter_patch(
    input: &NormalizedText,
    source_at: usize,
    next_ordinal: Option<u64>,
) -> Result<(Range<usize>, String), super::DocumentError> {
    let mapper = Builder::new(input, Revision(0));
    let tokens = tokenize(&input.text);
    let mut stack: Vec<(String, String)> = Vec::new();
    for token in tokens {
        if mapper.source_range(token.range.clone()).end > source_at {
            break;
        }
        if let TokenKind::Tag(tag) = token.kind {
            if tag.end {
                if let Some(index) = stack.iter().rposition(|(name, _)| *name == tag.name) {
                    stack.truncate(index);
                }
            } else if !void(&tag.name) {
                stack.push((tag.name, input.text[token.range].to_owned()));
            }
        }
    }
    let list = stack
        .iter()
        .rposition(|(name, _)| name == "li")
        .ok_or(super::DocumentError::AmbiguousProjection)?;
    let mut syntax = String::new();
    for (name, _) in stack[list..].iter().rev() {
        syntax.push_str(&format!("</{name}>"));
    }
    for (name, opening) in &stack[list..] {
        if name == "li" {
            if let (
                Some(ordinal),
                Some(Token {
                    kind: TokenKind::Tag(tag),
                    ..
                }),
            ) = (next_ordinal, tokenize(opening).into_iter().next())
            {
                syntax.push_str("<li");
                for (name, value) in tag.attributes {
                    if name != "value" {
                        syntax.push_str(&format!(" {name}=\"{}\"", attribute_escape(&value)));
                    }
                }
                syntax.push_str(&format!(" value=\"{ordinal}\">"));
                continue;
            }
        }
        syntax.push_str(opening);
    }
    Ok((source_at..source_at, syntax))
}

/// Explicit item values restart numbering and bound the effects of an insertion.
pub(super) fn list_item_has_explicit_value(input: &NormalizedText, source_at: usize) -> bool {
    let mapper = Builder::new(input, Revision(0));
    let mut stack: Vec<Tag> = Vec::new();
    for token in tokenize(&input.text) {
        if mapper.source_range(token.range).end > source_at {
            break;
        }
        if let TokenKind::Tag(tag) = token.kind {
            if tag.end {
                if let Some(at) = stack.iter().rposition(|open| open.name == tag.name) {
                    stack.truncate(at);
                }
            } else if !void(&tag.name) {
                stack.push(tag);
            }
        }
    }
    stack
        .iter()
        .rev()
        .find(|tag| tag.name == "li")
        .is_some_and(|tag| {
            tag.attribute("value")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some()
        })
}

pub(super) fn list_patches(
    input: &NormalizedText,
    targets: &[(Range<usize>, Option<super::ListStyle>, u64, Option<u64>)],
) -> Result<Vec<(Range<usize>, String)>, super::DocumentError> {
    let tokens = tokenize(&input.text);
    let mapper = Builder::new(input, Revision(0));
    let mut stack: Vec<usize> = Vec::new();
    let mut nodes: Vec<(usize, usize, Option<usize>)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if tag.end {
            if let Some(at) = stack.iter().rposition(|open| matches!(&tokens[*open].kind, TokenKind::Tag(open_tag) if open_tag.name == tag.name)) {
                let open = stack[at];
                let parent_list = stack[..at].iter().rev().copied().find(|parent| matches!(&tokens[*parent].kind, TokenKind::Tag(t) if matches!(t.name.as_str(), "ul" | "ol")));
                nodes.push((open, index, parent_list));
                stack.truncate(at);
            }
        } else if !void(&tag.name) {
            stack.push(index);
        }
    }
    let mut result = Vec::new();
    let owners = targets
        .iter()
        .map(|(source, _, _, _)| {
            nodes
                .iter()
                .filter(|(open, close, _)| {
                    matches!(&tokens[*open].kind,TokenKind::Tag(tag) if tag.name=="li")
                        && mapper.source_range(tokens[*open].range.clone()).end <= source.start
                        && source.end <= mapper.source_range(tokens[*close].range.clone()).start
                })
                .min_by_key(|(open, close, _)| tokens[*close].range.end - tokens[*open].range.start)
                .map(|(open, _, _)| *open)
        })
        .collect::<Vec<_>>();
    let mut handled = std::collections::BTreeSet::new();
    // A whole-list change owns the container delimiters, so retain every item
    // delimiter and its original attributes rather than splitting the list.
    for (parent, close, _) in &nodes {
        let TokenKind::Tag(parent_tag) = &tokens[*parent].kind else {
            continue;
        };
        if !matches!(parent_tag.name.as_str(), "ul" | "ol") {
            continue;
        }
        let children = nodes
            .iter()
            .filter(|(open, _, owner)| {
                *owner == Some(*parent)
                    && matches!(&tokens[*open].kind,TokenKind::Tag(tag) if tag.name=="li")
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            continue;
        }
        let selected = children
            .iter()
            .filter_map(|(open, _, _)| {
                targets
                    .iter()
                    .enumerate()
                    .find(|(index, _)| owners[*index] == Some(*open))
            })
            .collect::<Vec<_>>();
        if selected.len() != children.len() {
            continue;
        }
        let Some(style) = selected[0].1 .1 else {
            continue;
        };
        if selected.iter().any(|(_, target)| target.1 != Some(style)) {
            continue;
        }
        let name = if style == super::ListStyle::Numbered {
            "ol"
        } else {
            "ul"
        };
        let mut opening = input.text[tokens[*parent].range.clone()].to_owned();
        opening.replace_range(1..1 + parent_tag.name.len(), name);
        if style == super::ListStyle::Numbered {
            opening.insert_str(1 + name.len(), &format!(" start=\"{}\"", selected[0].1 .2));
        }
        let mut closing = input.text[tokens[*close].range.clone()].to_owned();
        closing.replace_range(2..2 + parent_tag.name.len(), name);
        result.push((mapper.source_range(tokens[*parent].range.clone()), opening));
        result.push((mapper.source_range(tokens[*close].range.clone()), closing));
        handled.extend(selected.into_iter().map(|(index, _)| index));
    }
    let mut previous_plain: Option<(usize, usize, String)> = None;
    for (target_index, (source, target, ordinal, original_ordinal)) in targets.iter().enumerate() {
        if handled.contains(&target_index) {
            previous_plain = None;
            continue;
        }
        let node = nodes.iter().filter(|(open, close, _)| {
            matches!(&tokens[*open].kind, TokenKind::Tag(tag) if if original_ordinal.is_some(){tag.name=="li"}else{paragraph(&tag.name)})
                && mapper.source_range(tokens[*open].range.clone()).end <= source.start
                && source.end <= mapper.source_range(tokens[*close].range.clone()).start
        }).min_by_key(|(open, close, _)| tokens[*close].range.end - tokens[*open].range.start);
        let list_name = match target {
            Some(super::ListStyle::Bullet) => Some("ul"),
            Some(super::ListStyle::Numbered) => Some("ol"),
            None => None,
        };
        let mut wrap_open = list_name
            .map(|name| {
                if name == "ol" {
                    format!("<ol start=\"{ordinal}\">")
                } else {
                    "<ul>".to_owned()
                }
            })
            .unwrap_or_default();
        let wrap_close = list_name
            .map(|name| format!("</{name}>"))
            .unwrap_or_default();
        if let Some((open, close, parent)) = node {
            let TokenKind::Tag(tag) = &tokens[*open].kind else {
                unreachable!()
            };
            let contains_paragraph = tokens[*open+1..*close].iter().any(|token| matches!(&token.kind,TokenKind::Tag(tag) if !tag.end && paragraph(&tag.name)));
            let desired_tag = if target.is_some() {
                "li"
            } else if contains_paragraph {
                "div"
            } else {
                "p"
            };
            let original_open = &input.text[tokens[*open].range.clone()];
            let mut opening = original_open.to_owned();
            opening.replace_range(1..1 + tag.name.len(), desired_tag);
            let original_close = &input.text[tokens[*close].range.clone()];
            let mut closing = original_close.to_owned();
            closing.replace_range(2..2 + tag.name.len(), desired_tag);
            let (leave_parent, resume_parent) = if tag.name == "li" {
                let parent = parent.ok_or(super::DocumentError::AmbiguousProjection)?;
                let TokenKind::Tag(parent_tag) = &tokens[parent].kind else {
                    unreachable!()
                };
                let mut resume = input.text[tokens[parent].range.clone()].to_owned();
                if parent_tag.name == "ol" {
                    resume.insert_str(
                        3,
                        &format!(
                            " start=\"{}\"",
                            original_ordinal.unwrap_or(*ordinal).saturating_add(1)
                        ),
                    );
                }
                (format!("</{}>", parent_tag.name), resume)
            } else {
                (String::new(), String::new())
            };
            if tag.name != "li" {
                if let (Some(name), Some((previous, end, previous_name))) =
                    (list_name, &previous_plain)
                {
                    let gap = &input.text[*end..tokens[*open].range.start];
                    let trivia = tokenize(gap).iter().all(|token| match token.kind {
                        TokenKind::Text => gap[token.range.clone()].trim().is_empty(),
                        TokenKind::Opaque => gap[token.range.clone()].starts_with("<!--"),
                        _ => false,
                    });
                    if name == previous_name && trivia {
                        let length = result[*previous].1.len() - wrap_close.len();
                        result[*previous].1.truncate(length);
                        wrap_open.clear();
                    }
                }
            }
            result.push((
                mapper.source_range(tokens[*open].range.clone()),
                format!("{leave_parent}{wrap_open}{opening}"),
            ));
            result.push((
                mapper.source_range(tokens[*close].range.clone()),
                format!("{closing}{wrap_close}{resume_parent}"),
            ));
            previous_plain = if tag.name != "li" {
                list_name.map(|name| (result.len() - 1, tokens[*close].range.end, name.to_owned()))
            } else {
                None
            };
        } else if target.is_some() {
            previous_plain = None;
            result.push((source.start..source.start, format!("{wrap_open}<li>")));
            result.push((source.end..source.end, format!("</li>{wrap_close}")));
        }
    }
    Ok(result)
}

/// Canonical text spelling. Whitespace runs which could collapse at a new
/// boundary are explicitly reversible inline pre-wrap spans. Ordinary interior
/// single spaces retain compact character-reference spelling.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < text.len() {
        let c = text[at..].chars().next().unwrap();
        if matches!(c, ' ' | '\t' | '\r') {
            let start = at;
            while at < text.len()
                && text[at..]
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, ' ' | '\t' | '\r'))
            {
                at += text[at..].chars().next().unwrap().len_utf8();
            }
            let run = &text[start..at];
            let preserved = start == 0 || at == text.len() || run.len() > 1 || run != " ";
            if preserved {
                out.push_str("<span style=\"white-space: pre-wrap\">");
            }
            for c in run.chars() {
                out.push_str(match c {
                    ' ' => "&#32;",
                    '\t' => "&#9;",
                    _ => "&#13;",
                });
            }
            if preserved {
                out.push_str("</span>");
            }
            continue;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(c),
        }
        at += c.len_utf8();
    }
    out
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
            super::rtf::windows_1252(code as u8) as u32
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
    // The WHATWG named character table is generated as sorted immutable data.
    let limit = input
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b';')
        .take(33)
        .count();
    for length in (1..=limit).rev() {
        let candidate = &input[..length];
        if let Ok(index) = entities::NAMED.binary_search_by_key(&candidate, |(name, _)| *name) {
            if attribute
                && !candidate.ends_with(';')
                && input
                    .as_bytes()
                    .get(length)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'=')
            {
                return None;
            }
            return Some((entities::NAMED[index].1.to_owned(), length + 1));
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
#[path = "html_entities.rs"]
mod entities;

fn direction(value: &str) -> Option<WritingDirection> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ltr" => Some(WritingDirection::LeftToRight),
        "rtl" => Some(WritingDirection::RightToLeft),
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
fn cascade_declarations(css: &str) -> Vec<(String, String)> {
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
fn relative_baseline(css: &str, size: f32) -> Option<f32> {
    let mut result = None;
    for (key, value) in cascade_declarations(css) {
        if key != "vertical-align" {
            continue;
        }
        match value.to_ascii_lowercase().as_str() {
            "super" => result = Some(size / 3.0),
            "sub" => result = Some(-size / 5.0),
            "baseline" => result = Some(0.0),
            value if length(value).is_some() => result = None,
            _ => {}
        }
    }
    result
}
pub(super) fn apply_css(
    css: &str,
    character: &mut CharacterProperties,
    paragraph: &mut BlockProperties,
) {
    for (key, value) in cascade_declarations(css) {
        let value = value.as_str();
        let lower = value.to_ascii_lowercase();
        match key.as_str() {
            "font-family" => {
                if let Some(values) = font_families(value) {
                    character.font_families = Some(values);
                }
            }
            "font-size" => {
                if let Some(size) = length(&lower).filter(|n| *n > 0.0) {
                    character.size = Some(size);
                }
            }
            "--evim-bold" => character.bold = value.parse().ok(),
            "--evim-base-weight" => {
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
            "background-color" => {
                if let Some(c) = color(value) {
                    character.background = Some(c);
                }
            }
            "text-decoration-line" => {
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
            "vertical-align" => {
                if let Some(n) = length(&lower) {
                    character.baseline_shift = Some(n);
                }
            }
            "direction" => {
                if let Some(d) = direction(&lower) {
                    character.direction = Some(d);
                    paragraph.base_direction = Some(d);
                }
            }
            "margin-block-start" => {
                if let Some(n) = length(&lower) {
                    paragraph.spacing_before = Some(n);
                }
            }
            "margin-block-end" => {
                if let Some(n) = length(&lower) {
                    paragraph.spacing_after = Some(n);
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
            "font-feature-settings" => {
                if let Some(features) = font_features(value) {
                    character.open_type_features = Some(features);
                }
            }
            _ => {}
        }
    }
    if let Some(shift) = relative_baseline(css, character.size.unwrap_or(14.0)) {
        character.baseline_shift = Some(shift);
    }
}

/// Version-one canonical character declarations. All absolute lengths are
/// points; property order follows the normalized schema. Unknown declarations
/// are never copied into a newly owned formatting construct.
pub(super) fn character_css(properties: &CharacterProperties) -> String {
    let mut declarations = Vec::new();
    if let Some(families) = &properties.font_families {
        declarations.push(format!(
            "font-family: {}",
            families
                .iter()
                .map(|name| format!(
                    "'{}'",
                    name.replace('\\', "\\\\")
                        .replace('\'', "\\'")
                        .replace('\n', "\\a ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(size) = properties.size {
        declarations.push(format!("font-size: {size}pt"));
    }
    if let Some(bold) = properties.bold {
        let base = properties.weight.unwrap_or(400);
        let weight = if bold {
            base.saturating_add(300).min(1000)
        } else {
            base
        };
        declarations.push(format!("font-weight: {weight}"));
        declarations.push(format!("--evim-base-weight: {base}"));
        declarations.push(format!("--evim-bold: {bold}"));
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
    if let Some(direction) = properties.direction {
        declarations.push(format!(
            "direction: {}",
            match direction {
                WritingDirection::RightToLeft => "rtl",
                _ => "ltr",
            }
        ));
    }
    if let Some(features) = &properties.open_type_features {
        declarations.push(format!(
            "font-feature-settings: {}",
            if features.is_empty() {
                "normal".into()
            } else {
                features
                    .iter()
                    .map(|(tag, value)| format!("'{tag}' {value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
    }
    if let Some(spacing) = properties.letter_spacing {
        declarations.push(format!("letter-spacing: {spacing}pt"));
    }
    if let Some(shift) = properties.baseline_shift {
        declarations.push(format!("vertical-align: {shift}pt"));
    }
    declarations.join("; ")
}
fn attribute_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}
pub(super) fn character_wrapper(properties: &CharacterProperties) -> (String, String) {
    let mut rest = properties.clone();
    let tag = if rest.bold == Some(true) && rest.weight.is_none() {
        rest.bold = None;
        "b"
    } else if rest.slant == Some(FontSlant::Italic) {
        rest.slant = None;
        "i"
    } else if rest.underline == Some(true) {
        if rest.strikethrough.is_none() {
            rest.underline = None;
        }
        "u"
    } else if rest.strikethrough == Some(true) {
        if rest.underline.is_none() {
            rest.strikethrough = None;
        }
        "s"
    } else {
        "span"
    };
    let css = character_css(&rest);
    let mut opening = format!("<{tag}");
    if !css.is_empty() {
        opening.push_str(&format!(" style=\"{}\"", attribute_escape(&css)));
    }
    if let Some(language) = &properties.language {
        opening.push_str(&format!(" lang=\"{}\"", attribute_escape(language)));
    }
    opening.push('>');
    (opening, format!("</{tag}>"))
}

/// When exactly one conventional element contributed the toggled property,
/// its two tags are the declared canonicalization boundary. Descendant text
/// and all unrelated source constructs remain outside these two patches.
pub(super) fn exact_conventional_removal(
    input: &NormalizedText,
    source: &Range<usize>,
    bold: bool,
) -> Option<Vec<(Range<usize>, String)>> {
    let builder = Builder::new(input, Revision(0));
    let tokens = tokenize(&input.text);
    let names = if bold { ["b", "strong"] } else { ["i", "em"] };
    let opening = tokens.iter().find_map(|token| {
        let TokenKind::Tag(tag) = &token.kind else {
            return None;
        };
        let range = builder.source_range(token.range.clone());
        (!tag.end && names.contains(&tag.name.as_str()) && range.end == source.start)
            .then_some((range, tag))
    })?;
    let closing = tokens.iter().find_map(|token| {
        let TokenKind::Tag(tag) = &token.kind else {
            return None;
        };
        let range = builder.source_range(token.range.clone());
        (tag.end && tag.name == opening.1.name && range.start == source.end).then_some(range)
    })?;
    let mut properties = CharacterProperties::default();
    let mut paragraph = BlockProperties::default();
    if let Some(css) = opening.1.attribute("style") {
        apply_css(css, &mut properties, &mut paragraph);
    }
    if let Some(language) = opening.1.attribute("lang") {
        properties.language = Some(language.to_owned());
    }
    if let Some(direction) = opening.1.attribute("dir").and_then(direction) {
        properties.direction = Some(direction);
    }
    // The requested false value is explicit if another inherited layer exists.
    // The transaction verifier detects that case and falls back to an override.
    if bold {
        properties.weight = None;
    } else {
        properties.slant = None;
    }
    let wrappers = if properties == CharacterProperties::default() {
        (String::new(), String::new())
    } else {
        character_wrapper(&properties)
    };
    Some(vec![(opening.0, wrappers.0), (closing, wrappers.1)])
}

pub(super) fn empty_insertion_point(input: &NormalizedText) -> Option<usize> {
    let builder = Builder::new(input, Revision(0));
    let mut stack = Vec::<String>::new();
    let mut candidate = None;
    let mut document_close = None;
    for token in tokenize(&input.text) {
        let TokenKind::Tag(tag) = token.kind else {
            continue;
        };
        if tag.end {
            if tag.name == "html" {
                document_close = Some(builder.source_range(token.range.clone()).start);
            }
            if let Some(at) = stack.iter().rposition(|name| name == &tag.name) {
                stack.truncate(at);
            }
            continue;
        }
        let visible = !stack.iter().any(|name| hidden(name) || atomic(name));
        if visible
            && !hidden(&tag.name)
            && !atomic(&tag.name)
            && !void(&tag.name)
            && tag.name != "html"
        {
            candidate = Some(builder.source_range(token.range.clone()).end);
        }
        if !void(&tag.name) {
            stack.push(tag.name);
        }
    }
    candidate
        .or(document_close)
        .or_else(|| input.units.last().map(|u| u.source.end))
        .or(Some(0))
}
