//! Explicit, whole-artifact semantic conversion. Ordinary edits never use this
//! serializer: conversion is a user-requested change of persistence language.
use super::{
    BlockKind, CharacterProperties, Document, DocumentError, FontSlant, Format, FormattedDocument,
    SemanticInlineStyle, StyleApplication,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ConversionLoss {
    /// Comments, inactive content, attributes, or unsupported element semantics.
    SourceOnlyContent,
    /// A property or named definition has no equivalent in the target language.
    Styling,
    /// An opaque embedded object has only its textual placeholder available.
    EmbeddedObjects,
    /// Paragraph/list structure exceeds the target adapter's supported model.
    Structure,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversionWarning {
    FormatInformationLost {
        losses: Vec<ConversionLoss>,
    },
    UnrepresentableCharacters {
        encoding: super::Encoding,
        count: usize,
    },
}
impl ConversionWarning {
    pub fn message(&self) -> String {
        match self {
            Self::FormatInformationLost { .. } => "Warning: some information was lost during format conversion.".to_owned(),
            Self::UnrepresentableCharacters { count, .. } => format!("Warning: {count} character{} could not be represented in Latin-1 and {} replaced with '?'.", if *count == 1 { "" } else { "s" }, if *count == 1 { "was" } else { "were" }),
        }
    }
}

/// Whether a format change keeps source bytes or authors destination syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatOperation {
    Reinterpret,
    Convert,
}

pub(super) struct Conversion {
    pub source: String,
    /// Retained semantic graphemes, from original physical source bytes to
    /// their complete spelling in the generated, normalized UTF-8 source.
    /// Formatting wrappers have no correspondence. Encoding and line-ending
    /// conversion must rebase the generated ranges before using this map.
    pub source_correspondence: Vec<(Range<usize>, Range<usize>)>,
    pub warnings: Vec<ConversionWarning>,
}

struct ConversionWriter {
    source: String,
    source_correspondence: Vec<(Range<usize>, Range<usize>)>,
    semantic_sources: BTreeMap<usize, (usize, Range<usize>)>,
    object_fallbacks: BTreeMap<usize, String>,
    line_prefix: MarkdownLinePrefix,
}

#[derive(Clone, Copy)]
enum MarkdownLinePrefix {
    Empty,
    Digits,
    Other,
}

#[derive(Clone, Copy)]
enum TextSpelling {
    Literal,
    Html { hard_breaks: bool },
    Markdown,
}

impl ConversionWriter {
    fn new(document: &FormattedDocument) -> Self {
        Self {
            source: String::new(),
            source_correspondence: Vec::new(),
            object_fallbacks: BTreeMap::new(),
            line_prefix: MarkdownLinePrefix::Empty,
            semantic_sources: document
                .source_grapheme_ranges()
                .filter_map(|(offset, text, source)| {
                    source.map(|source| (offset, (offset + text.len(), source)))
                })
                .collect(),
        }
    }

    fn push(&mut self, character: char) {
        self.line_prefix = match (self.line_prefix, character) {
            (_, '\n') => MarkdownLinePrefix::Empty,
            (MarkdownLinePrefix::Empty, character) if character.is_whitespace() => {
                MarkdownLinePrefix::Empty
            }
            (MarkdownLinePrefix::Empty | MarkdownLinePrefix::Digits, '0'..='9') => {
                MarkdownLinePrefix::Digits
            }
            _ => MarkdownLinePrefix::Other,
        };
        self.source.push(character);
    }

    fn push_str(&mut self, syntax: &str) {
        for character in syntax.chars() {
            self.push(character);
        }
    }

    /// Syntax delimiters must enclose the text that will actually be emitted,
    /// including a replacement object's fallback. Ordinary text is borrowed.
    fn visible_text<'a>(&self, text: &'a str, semantic_start: usize) -> Cow<'a, str> {
        let mut result = None;
        let mut consumed = 0;
        for (at, fallback) in self
            .object_fallbacks
            .range(semantic_start..semantic_start + text.len())
        {
            let offset = at - semantic_start;
            let output = result.get_or_insert_with(String::new);
            output.push_str(&text[consumed..offset]);
            output.push_str(fallback);
            consumed = offset + '\u{fffc}'.len_utf8();
        }
        if let Some(mut output) = result {
            output.push_str(&text[consumed..]);
            Cow::Owned(output)
        } else {
            Cow::Borrowed(text)
        }
    }

    fn text(&mut self, text: &str, semantic_start: usize, spelling: TextSpelling) {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let start = self.source.len();
            let semantic_at = semantic_start + offset;
            let fallback = self.object_fallbacks.get(&semantic_at).cloned();
            let visible = fallback.as_deref().unwrap_or(grapheme);
            match spelling {
                TextSpelling::Literal => self.push_str(visible),
                TextSpelling::Html { hard_breaks } => {
                    for character in visible.chars() {
                        match character {
                            '&' => self.push_str("&amp;"),
                            '<' => self.push_str("&lt;"),
                            '>' => self.push_str("&gt;"),
                            '\n' if hard_breaks => self.push_str("<br>"),
                            _ => self.push(character),
                        }
                    }
                }
                TextSpelling::Markdown => {
                    for (character_offset, character) in visible.char_indices() {
                        if character == '\n' {
                            self.push_str("<br>");
                            continue;
                        }
                        let starts_block = matches!(character, '-' | '+' | '=')
                            && matches!(self.line_prefix, MarkdownLinePrefix::Empty)
                            || matches!(character, '.' | ')')
                                && matches!(self.line_prefix, MarkdownLinePrefix::Digits);
                        let remaining = if fallback.is_some() {
                            &visible[character_offset..]
                        } else {
                            &text[offset + character_offset..]
                        };
                        if starts_block
                            || matches!(character, '\\' | '*' | '_' | '`' | '#' | '>')
                            || character == '<'
                                && super::projection::markdown_inline_break_length(remaining).is_some()
                            || character == '&' && remaining.starts_with("&#")
                        {
                            self.push('\\');
                        }
                        self.push(character);
                    }
                }
            }
            if let Some((end, source)) = self.semantic_sources.get(&semantic_at) {
                // A style boundary may split a semantic grapheme. Only a
                // complete grapheme carries exact cross-format identity.
                if *end == semantic_at + grapheme.len() {
                    self.source_correspondence
                        .push((source.clone(), start..self.source.len()));
                }
            }
        }
    }
}
pub(super) fn convert(document: &Document, target: Format) -> Result<Conversion, DocumentError> {
    let target = target.wysiwyg();
    if target == Format::Rtf {
        return Err(DocumentError::UnsupportedFormatting);
    }
    let source_bytes = document.source_bytes();
    let decoded = document.encoding().decode(&source_bytes)?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let from = document.format().wysiwyg();
    let semantic = if from == document.format() {
        document.projection().clone()
    } else {
        let mut projected = if from == Format::Html {
            super::html::project_with_configuration(
                &input,
                document.revision(),
                decoded.bom_len,
                source_bytes.len(),
                Some(document.projection().style_sheet()),
            )
        } else {
            super::projection::project(
                &input,
                from,
                document.revision(),
                decoded.bom_len,
                source_bytes.len(),
            )
        };
        projected.install_configuration_styles(
            document.revision(),
            document.projection().style_sheet().clone(),
            document.projection().document_style().clone(),
        );
        projected
    };
    let to_html = target == Format::Html;
    let to_text = target == Format::PlainText;
    let mut losses = BTreeSet::new();
    if decoded.spans.iter().any(|span| span.diagnostic.is_some()) || from == Format::Rtf {
        losses.insert(ConversionLoss::SourceOnlyContent);
    }
    if from == Format::Html {
        for token in super::html::tokenize(&input.text) {
            match token.kind {
                super::html::TokenKind::Opaque => {
                    losses.insert(ConversionLoss::SourceOnlyContent);
                }
                super::html::TokenKind::Tag(tag) => {
                    if !matches!(
                        tag.name.as_str(),
                        "html"
                            | "body"
                            | "p"
                            | "div"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "b"
                            | "strong"
                            | "i"
                            | "em"
                            | "pre"
                            | "code"
                            | "br"
                            | "ul"
                            | "ol"
                            | "li"
                            | "blockquote"
                    ) {
                        losses.insert(ConversionLoss::SourceOnlyContent);
                    }
                    if tag
                        .attributes
                        .iter()
                        .any(|(name, _)| !matches!(name.as_str(), "start" | "value"))
                    {
                        losses.insert(ConversionLoss::SourceOnlyContent);
                    }
                }
                _ => {}
            }
        }
    }
    if from == Format::Markdown
        && input.text.lines().any(|line| {
            super::projection::markdown_fence(line)
                .is_some_and(|(_, count)| !line.trim_start()[count..].trim().is_empty())
        })
    {
        losses.insert(ConversionLoss::SourceOnlyContent);
    }
    let mut output = ConversionWriter::new(&semantic);
    if from.is_rich_text() {
        for (offset, text, source) in semantic.source_grapheme_ranges() {
            if text == "\u{fffc}" {
                if let Some(source) = source {
                    if let Some(fallback) = object_text(document, &source_bytes[source])? {
                        output.object_fallbacks.insert(offset, fallback);
                    }
                }
            }
        }
    }
    let plain_blocks;
    let blocks = if from == Format::PlainText {
        plain_blocks = plain_paragraphs(&semantic);
        plain_blocks.as_slice()
    } else {
        semantic.blocks()
    };
    let mut index = 0;
    while index < blocks.len() {
        let block = &blocks[index];
        if index > 0 {
            output.push('\n');
            if to_text
                || !to_html
                    && !matches!(block.kind, BlockKind::ListItem { .. })
                    && (matches!(blocks[index - 1].kind, BlockKind::ListItem { .. })
                        || block.kind == BlockKind::Paragraph
                            && blocks[index - 1].kind == BlockKind::Paragraph)
            {
                // A bare source ending would continue list prose or merge
                // two ordinary paragraphs under Markdown's flow rules.
                output.push('\n');
            }
        }
        if block.style.0 == "Code Block" {
            let start = block.range.start;
            // One block is one paragraph, even when a fence/pre contains
            // several hard lines. Adjacent code paragraphs stay distinct.
            let text = &semantic.text()[block.range.clone()];
            if to_text {
                output.text(text, start, TextSpelling::Literal);
                losses.insert(ConversionLoss::Styling);
            } else if to_html {
                // HTML5 ignores a literal initial LF immediately after pre.
                output.push_str("<pre><code>");
                output.text(text, start, TextSpelling::Html { hard_breaks: false });
                output.push_str("</code></pre>");
            } else {
                let fence =
                    "`".repeat(longest_run(&output.visible_text(text, start), '`').max(2) + 1);
                output.push_str(&fence);
                output.push('\n');
                output.text(text, start, TextSpelling::Literal);
                output.push('\n');
                output.push_str(&fence);
            }
            index += 1;
            continue;
        }
        if to_html && matches!(block.kind, BlockKind::ListItem { .. }) {
            html_list(&semantic, &blocks, &mut index, &mut losses, &mut output);
            continue;
        }
        let body = block.range.clone();
        if !to_html
            && (block.direct_paragraph != Default::default()
                || block.direct_default_character != Default::default())
        {
            losses.insert(ConversionLoss::Styling);
        }
        let quoted = block.style.0 == "Block quote";
        let (open, close) = match &block.kind {
            _ if to_text => {
                if block.kind != BlockKind::Paragraph || block.style.0 != "Paragraph" {
                    losses.insert(ConversionLoss::Styling);
                }
                (String::new(), String::new())
            }
            BlockKind::Heading(level) if to_html => (
                format!(
                    "<h{level}{}>",
                    whitespace_attribute(&semantic.text()[block.range.clone()])
                ),
                format!("</h{level}>"),
            ),
            BlockKind::Heading(level) => (
                format!("{} ", "#".repeat(usize::from(*level))),
                String::new(),
            ),
            BlockKind::ListItem {
                ordered,
                ordinal,
                level,
                item_start,
                ..
            } => {
                if !item_start {
                    losses.insert(ConversionLoss::Structure);
                }
                let mut marker = "  ".repeat(usize::from(*level));
                if *ordered {
                    marker.push_str(&format!("{ordinal}. "));
                } else {
                    marker.push_str("- ");
                }
                (marker, String::new())
            }
            _ if to_html => (
                format!(
                    "<p{}>",
                    whitespace_attribute(&semantic.text()[block.range.clone()])
                ),
                "</p>".to_owned(),
            ),
            _ => (String::new(), String::new()),
        };
        if quoted && !to_text {
            output.push_str(if to_html { "<blockquote>" } else { "> " });
        }
        output.push_str(&open);
        // Container syntax (especially >) does not make its first visible
        // text safe from Markdown's paragraph/list marker grammar.
        output.line_prefix = MarkdownLinePrefix::Empty;
        inline(&semantic, body, target, &mut losses, &mut output);
        output.push_str(&close);
        if quoted && to_html {
            output.push_str("</blockquote>");
        }
        index += 1;
    }
    let warnings = if losses.is_empty() {
        Vec::new()
    } else {
        vec![ConversionWarning::FormatInformationLost {
            losses: losses.into_iter().collect(),
        }]
    };
    Ok(Conversion {
        source: output.source,
        source_correspondence: output.source_correspondence,
        warnings,
    })
}

/// Reuse passive parsing for an object's available fallback text. Only a
/// disposable HTML token view loses atomic treatment. Opaque RTF objects have
/// no projected text fallback; both formats retain a readable placeholder when
/// needed. Source changes only when the caller commits the conversion.
fn object_text(document: &Document, source: &[u8]) -> Result<Option<String>, DocumentError> {
    use super::html::TokenKind;
    let decoded = document.encoding().decode(source)?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    if document.format() == Format::Rtf {
        let atomic = super::rtf::tokenize(&input).iter().any(|token| {
            matches!(&token.kind, super::rtf::Kind::Control(name, _) if matches!(name.as_str(), "pict" | "object" | "field" | "shp"))
        });
        return Ok(atomic.then(|| "[Object]".to_owned()));
    }
    let mut tokens = super::html5_tree::tokens(&input.text);
    let Some(tag) = tokens.iter().find_map(|token| match &token.kind {
        TokenKind::Tag(tag) if !tag.end && super::html::atomic(&tag.name) => Some(tag),
        _ => None,
    }) else {
        return Ok(None);
    };
    let fallback = tag
        .attribute("alt")
        .or_else(|| tag.attribute("title"))
        .unwrap_or(if tag.name == "img" {
            "[Image]"
        } else {
            "[Object]"
        })
        .to_owned();
    if super::html::void(&tag.name) {
        return Ok(Some(fallback));
    }
    for token in &mut tokens {
        if let TokenKind::Tag(tag) = &mut token.kind {
            if !tag.end && super::html::void(&tag.name) && super::html::atomic(&tag.name) {
                token.kind = TokenKind::MappedText {
                    text: tag
                        .attribute("alt")
                        .or_else(|| tag.attribute("title"))
                        .unwrap_or("[Image]")
                        .to_owned(),
                    mapped: false,
                };
            } else if super::html::atomic(&tag.name) {
                tag.name = "span".into();
            } else if matches!(tag.name.as_str(), "td" | "th") {
                tag.name = "p".into();
            }
        }
    }
    let projected = super::html::project_tokens(
        &input,
        document.revision(),
        decoded.bom_len,
        source.len(),
        tokens,
    );
    Ok(Some(if projected.text().is_empty() {
        fallback
    } else {
        projected.text().to_owned()
    }))
}

fn inline(
    document: &FormattedDocument,
    range: Range<usize>,
    target: Format,
    losses: &mut BTreeSet<ConversionLoss>,
    output: &mut ConversionWriter,
) {
    let spans = document.style_spans_for_region(&range);
    let mut boundaries = BTreeSet::from([range.start, range.end]);
    for span in &spans {
        boundaries.insert(span.range.start.max(range.start));
        boundaries.insert(span.range.end.min(range.end));
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    for pair in boundaries.windows(2) {
        let at = pair[0];
        let mut properties = CharacterProperties::default();
        let mut code = false;
        for span in spans.iter().filter(|span| span.range.contains(&at)) {
            match &span.application {
                StyleApplication::Direct(value) => {
                    super::rich_text::overlay(&mut properties, value)
                }
                StyleApplication::Named(id) if id.0 == "Code" => code = true,
                StyleApplication::Named(id) => {
                    let value = super::html_styles::character_chain(document.style_sheet(), id);
                    super::rich_text::overlay(&mut properties, &value);
                    losses.insert(ConversionLoss::Styling);
                }
                StyleApplication::Semantic(SemanticInlineStyle::Strong) => {
                    properties.bold = Some(true)
                }
                StyleApplication::Semantic(SemanticInlineStyle::Emphasis) => {
                    properties.slant = Some(FontSlant::Italic)
                }
                StyleApplication::Semantic(SemanticInlineStyle::Code) => code = true,
                _ => {}
            }
        }
        let text = &document.text()[at..pair[1]];
        if text.contains('\u{fffc}') {
            losses.insert(ConversionLoss::EmbeddedObjects);
        }
        if target == Format::PlainText {
            if code || properties != CharacterProperties::default() {
                losses.insert(ConversionLoss::Styling);
            }
            output.text(text, at, TextSpelling::Literal);
        } else if target == Format::Html {
            let wrapper = (properties != CharacterProperties::default())
                .then(|| super::html::character_wrapper(&properties));
            if let Some((open, _)) = &wrapper {
                output.push_str(open);
            }
            if code {
                output.push_str("<code>");
            }
            output.text(text, at, TextSpelling::Html { hard_breaks: true });
            if code {
                output.push_str("</code>");
            }
            if let Some((_, close)) = &wrapper {
                output.push_str(close);
            }
        } else {
            let bold = properties.bold == Some(true)
                || properties.weight.is_some_and(|weight| weight >= 600);
            let italic = matches!(
                properties.slant,
                Some(FontSlant::Italic | FontSlant::Oblique)
            );
            let mut supported = CharacterProperties::default();
            supported.bold = properties.bold;
            supported.weight = properties.weight;
            supported.slant = properties.slant;
            if properties != supported || properties.weight.is_some() {
                losses.insert(ConversionLoss::Styling);
            }
            if code {
                let visible = output.visible_text(text, at);
                let marker = "`".repeat(longest_run(&visible, '`') + 1);
                let pad = if visible.starts_with('`')
                    || visible.ends_with('`')
                    || (visible.starts_with(' ')
                        && visible.ends_with(' ')
                        && !visible.trim().is_empty())
                {
                    " "
                } else {
                    ""
                };
                output.push_str(&marker);
                output.push_str(pad);
                output.text(text, at, TextSpelling::Literal);
                output.push_str(pad);
                output.push_str(&marker);
                if bold || italic {
                    losses.insert(ConversionLoss::Styling);
                }
            } else {
                let marker = if bold && italic {
                    "***"
                } else if bold {
                    "**"
                } else if italic {
                    "*"
                } else {
                    ""
                };
                output.push_str(marker);
                output.text(text, at, TextSpelling::Markdown);
                output.push_str(marker);
            }
        }
    }
}
fn longest_run(text: &str, delimiter: char) -> usize {
    text.split(|c| c != delimiter)
        .map(str::len)
        .max()
        .unwrap_or(0)
}
fn whitespace_attribute(text: &str) -> &'static str {
    if text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains("  ")
        || text.contains('\t')
    {
        " style=\"white-space: pre-wrap\""
    } else {
        ""
    }
}
/// Plain source uses blank physical lines to separate paragraphs. Single
/// line endings remain hard breaks within the paragraph's visible text.
fn plain_paragraphs(document: &FormattedDocument) -> Vec<super::Block> {
    let text = document.text();
    let template = document.blocks().into_iter().next().unwrap();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut scan = 0;
    while let Some(relative) = text[scan..].find('\n') {
        let at = scan + relative;
        let mut end = at + 1;
        if let Some(next) = text[end..].find('\n') {
            if text[end..end + next].trim().is_empty() {
                end += next + 1;
            }
        }
        if end > at + 1 {
            ranges.push(start..at);
            start = end;
        }
        scan = end;
    }
    ranges.push(start..text.len());
    ranges
        .into_iter()
        .map(|range| super::Block {
            range,
            ..template.clone()
        })
        .collect()
}

fn html_list(
    document: &FormattedDocument,
    blocks: &[super::Block],
    index: &mut usize,
    losses: &mut BTreeSet<ConversionLoss>,
    output: &mut ConversionWriter,
) {
    let BlockKind::ListItem {
        ordered,
        ordinal,
        level,
        ..
    } = blocks[*index].kind
    else {
        unreachable!()
    };
    let tag = if ordered { "ol" } else { "ul" };
    if ordered {
        output.push_str(&format!("<ol start=\"{ordinal}\">"));
    } else {
        output.push_str("<ul>");
    }
    while *index < blocks.len() {
        let block = &blocks[*index];
        let BlockKind::ListItem {
            ordered: item_ordered,
            ordinal,
            level: item_level,
            item_start,
            ..
        } = block.kind
        else {
            break;
        };
        if item_level != level || item_ordered != ordered {
            break;
        }
        if !item_start {
            losses.insert(ConversionLoss::Structure);
        }
        let body = block.range.clone();
        let value = if ordered {
            format!(" value=\"{ordinal}\"")
        } else {
            String::new()
        };
        output.push_str(&format!(
            "<li{value}{}>",
            whitespace_attribute(&document.text()[body.clone()])
        ));
        inline(document, body, Format::Html, losses, output);
        *index += 1;
        while *index < blocks.len()
            && matches!(blocks[*index].kind, BlockKind::ListItem {level: next, ..} if next > level)
        {
            if matches!(blocks[*index].kind, BlockKind::ListItem {level: next, ..} if next != level + 1)
            {
                losses.insert(ConversionLoss::Structure);
            }
            html_list(document, blocks, index, losses, output);
        }
        output.push_str("</li>");
    }
    output.push_str(&format!("</{tag}>"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    fn assert_spelling(
        original: &[u8],
        encoding: Encoding,
        converted: &Conversion,
        original_spelling: &str,
        generated_spelling: &str,
    ) {
        let original_spelling = encoding.encode_fragment(original_spelling).unwrap();
        assert!(
            converted.source_correspondence.iter().any(|(from, to)| {
                original[from.clone()] == original_spelling
                    && &converted.source[to.clone()] == generated_spelling
            }),
            "missing retained spelling {generated_spelling:?}"
        );
    }

    #[test]
    fn tracked_markdown_conversion_preserves_spelling_and_physical_unicode_provenance() {
        let source = "<h2>Head &amp; 👩‍💻e\u{301}</h2>\r\n<p><b>same</b> # tail</p>\r\n<pre><code>a &lt; b\r\n  c</code></pre>";
        let expected = "## Head & 👩‍💻e\u{301}\n**same** \\# tail\n\n```\na < b\n  c\n```";
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let mut bytes = encoding.bom_bytes().to_vec();
            bytes.extend(encoding.encode_fragment(source).unwrap());
            for format in [Format::Html, Format::HtmlSource] {
                let document = Document::from_bytes(bytes.clone(), encoding, format).unwrap();
                let converted = convert(&document, Format::MarkdownSource).unwrap();
                assert_eq!(converted.source, expected);
                for (old, new) in [
                    ("&amp;", "&"),
                    ("👩‍💻", "👩‍💻"),
                    ("e\u{301}", "e\u{301}"),
                    ("#", "\\#"),
                    ("&lt;", "<"),
                ] {
                    assert_spelling(&bytes, encoding, &converted, old, new);
                }
                assert_eq!(document.source_bytes(), bytes);
            }
        }
    }

    #[test]
    fn tracked_html_conversion_keeps_nested_list_and_inline_wrapper_offsets() {
        let source = "## Head *soft* &\n\n- one\n  - two\n\n```\na < b\n```";
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let converted = convert(&document, Format::Html).unwrap();
        assert_eq!(
            converted.source,
            "<h2>Head <i>soft</i> &amp;</h2>\n<ul><li>one<ul><li>two</li></ul></li></ul>\n<pre><code>a &lt; b</code></pre>"
        );
        assert_spelling(source.as_bytes(), Encoding::Utf8, &converted, "&", "&amp;");
        assert_spelling(source.as_bytes(), Encoding::Utf8, &converted, "<", "&lt;");
        let second_item = source.find("two").unwrap();
        let generated = converted.source.find("two").unwrap();
        assert!(converted
            .source_correspondence
            .contains(&(second_item..second_item + 1, generated..generated + 1)));
    }

    #[test]
    fn tracked_inline_code_padding_and_hard_break_are_not_confused_with_wrappers() {
        let source = "<p><code>`x`</code> tail</p>";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let converted = convert(&document, Format::Markdown).unwrap();
        assert_eq!(converted.source, "`` `x` `` tail");
        assert_spelling(source.as_bytes(), Encoding::Utf8, &converted, "`", "`");

        let source = "a  \nb";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let converted = convert(&document, Format::Html).unwrap();
        assert_eq!(converted.source, "<p>a<br>b</p>");
        assert!(converted.source_correspondence.iter().any(|(from, to)| {
            source[from.clone()].contains('\n') && &converted.source[to.clone()] == "<br>"
        }));
    }
    #[test]
    fn source_view_conversion_uses_the_configured_html_semantics() {
        use crate::document::{
            PersistedStyleIntent, StyleDefinitionEdit, StyleDefinitionOrigin, StyleModelIntent,
            StyleModelRequest,
        };
        let mut outputs = Vec::new();
        for format in [Format::Html, Format::HtmlSource] {
            let source = "<p><span style='vertical-align:super'>Word</span></p>";
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let mut paragraph = document
                .projection()
                .style_sheet()
                .block_style(&"Paragraph".into())
                .unwrap()
                .clone();
            paragraph.character.size = Some(30.);
            document
                .apply_style_request(StyleModelRequest::new(
                    document.id(),
                    document.revision(),
                    StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                        origin: StyleDefinitionOrigin::SourceBacked,
                        edit: StyleDefinitionEdit::UpdateBlock(paragraph),
                    }),
                ))
                .unwrap();
            assert_eq!(document.source_bytes(), source.as_bytes());
            let converted = convert(&document, Format::Html).unwrap();
            assert!(
                converted.source.contains("vertical-align: 10pt"),
                "{}",
                converted.source
            );
            outputs.push(converted.source);
        }
        assert_eq!(outputs[0], outputs[1]);
    }

}
