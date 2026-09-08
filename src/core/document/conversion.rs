//! Explicit, whole-artifact semantic conversion. Ordinary edits never use this
//! serializer: conversion is a user-requested change of persistence language.
use super::{
    BlockKind, CharacterProperties, Document, DocumentError, FontSlant, Format, FormattedDocument,
    SemanticInlineStyle, StyleApplication,
};
use std::{
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

pub(super) fn crosses_markup_family(from: Format, to: Format) -> bool {
    matches!(
        (from, to),
        (
            Format::Html | Format::HtmlSource,
            Format::Markdown | Format::MarkdownSource
        ) | (
            Format::Markdown | Format::MarkdownSource,
            Format::Html | Format::HtmlSource
        )
    )
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
            semantic_sources: document
                .source_grapheme_ranges()
                .filter_map(|(offset, text, source)| {
                    source.map(|source| (offset, (offset + text.len(), source)))
                })
                .collect(),
        }
    }

    fn push(&mut self, character: char) {
        self.source.push(character);
    }

    fn push_str(&mut self, syntax: &str) {
        self.source.push_str(syntax);
    }

    fn text(&mut self, text: &str, semantic_start: usize, spelling: TextSpelling) {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let start = self.source.len();
            match spelling {
                TextSpelling::Literal => self.source.push_str(grapheme),
                TextSpelling::Html { hard_breaks } => {
                    for character in grapheme.chars() {
                        match character {
                            '&' => self.source.push_str("&amp;"),
                            '<' => self.source.push_str("&lt;"),
                            '>' => self.source.push_str("&gt;"),
                            '\n' if hard_breaks => self.source.push_str("<br>"),
                            _ => self.source.push(character),
                        }
                    }
                }
                TextSpelling::Markdown => {
                    for character in grapheme.chars() {
                        if matches!(character, '\\' | '*' | '_' | '`' | '#')
                            || character == '<' && super::projection::markdown_inline_break_length(&text[offset..]).is_some()
                        {
                            self.source.push('\\');
                        }
                        self.source.push(character);
                    }
                }
            }
            let semantic_at = semantic_start + offset;
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
    let decoded = document.encoding().decode(&document.source_bytes())?;
    if let Some(span) = decoded.spans.iter().find(|span| span.diagnostic.is_some()) {
        return Err(DocumentError::OpaqueDecodingConflict {
            source_range: span.source.clone(),
        });
    }
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let from_html = matches!(document.format(), Format::Html | Format::HtmlSource);
    let semantic = super::projection::project(
        &input,
        if from_html {
            Format::Html
        } else {
            Format::Markdown
        },
        document.revision(),
        decoded.bom_len,
        document.source_bytes().len(),
    );
    let to_html = matches!(target, Format::Html | Format::HtmlSource);
    let mut losses = BTreeSet::new();
    if from_html {
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
    if !from_html
        && input.text.lines().any(|line| {
            super::projection::markdown_fence(line)
                .is_some_and(|(_, count)| !line.trim_start()[count..].trim().is_empty())
        })
    {
        losses.insert(ConversionLoss::SourceOnlyContent);
    }
    let mut output = ConversionWriter::new(&semantic);
    let blocks = semantic.blocks();
    let mut index = 0;
    while index < blocks.len() {
        let block = &blocks[index];
        if index > 0 {
            output.push('\n');
            if !to_html
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
            let mut end = block.range.end;
            while index + 1 < blocks.len() && blocks[index + 1].style.0 == "Code Block" {
                index += 1;
                end = blocks[index].range.end;
            }
            let text = &semantic.text()[start..end];
            if to_html {
                // HTML5 ignores a literal initial LF immediately after pre.
                output.push_str("<pre><code>");
                output.text(text, start, TextSpelling::Html { hard_breaks: false });
                output.push_str("</code></pre>");
            } else {
                let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
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
        let body = list_body(&semantic, block);
        if !to_html && semantic.text()[body.clone()].contains('\n') {
            losses.insert(ConversionLoss::Structure);
        }
        if !to_html
            && (block.direct_paragraph != Default::default()
                || block.direct_default_character != Default::default())
        {
            losses.insert(ConversionLoss::Styling);
        }
        let (open, close) = match &block.kind {
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
        output.push_str(&open);
        inline(&semantic, body, to_html, &mut losses, &mut output);
        output.push_str(&close);
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

fn inline(
    document: &FormattedDocument,
    range: Range<usize>,
    html: bool,
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
        if html {
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
                let marker = "`".repeat(longest_run(text, '`') + 1);
                let pad = if text.starts_with('`')
                    || text.ends_with('`')
                    || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty())
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
fn list_body(_document: &FormattedDocument, block: &super::Block) -> Range<usize> {
    // Conversion always consumes a WYSIWYG projection. List labels are layout
    // decorations, so the complete block range is body content.
    block.range.clone()
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
        let body = list_body(document, block);
        let value = if ordered {
            format!(" value=\"{ordinal}\"")
        } else {
            String::new()
        };
        output.push_str(&format!(
            "<li{value}{}>",
            whitespace_attribute(&document.text()[body.clone()])
        ));
        inline(document, body, true, losses, output);
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
}
