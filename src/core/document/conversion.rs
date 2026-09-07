//! Explicit, whole-artifact semantic conversion. Ordinary edits never use this
//! serializer: conversion is a user-requested change of persistence language.
use super::{
    BlockKind, CharacterProperties, Document, DocumentError, FontSlant, Format, FormattedDocument,
    SemanticInlineStyle, StyleApplication,
};
use std::{collections::BTreeSet, ops::Range};

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
    pub warnings: Vec<ConversionWarning>,
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
    let mut output = String::new();
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
                output.push_str(&html_text(text));
                output.push_str("</code></pre>");
            } else {
                let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
                output.push_str(&fence);
                output.push('\n');
                output.push_str(text);
                output.push('\n');
                output.push_str(&fence);
            }
            index += 1;
            continue;
        }
        if to_html && matches!(block.kind, BlockKind::ListItem { .. }) {
            output.push_str(&html_list(&semantic, &blocks, &mut index, &mut losses));
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
        let rendered = inline(&semantic, body, to_html, &mut losses);
        match &block.kind {
            BlockKind::Heading(level) if to_html => output.push_str(&format!(
                "<h{level}{}>{rendered}</h{level}>",
                whitespace_attribute(&semantic.text()[block.range.clone()])
            )),
            BlockKind::Heading(level) => {
                output.push_str(&format!("{} {rendered}", "#".repeat(usize::from(*level))))
            }
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
                output.push_str(&"  ".repeat(usize::from(*level)));
                if *ordered {
                    output.push_str(&format!("{ordinal}. "));
                } else {
                    output.push_str("- ");
                }
                output.push_str(&rendered);
            }
            _ if to_html => output.push_str(&format!(
                "<p{}>{rendered}</p>",
                whitespace_attribute(&semantic.text()[block.range.clone()])
            )),
            _ => output.push_str(&rendered),
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
        source: output,
        warnings,
    })
}

fn inline(
    document: &FormattedDocument,
    range: Range<usize>,
    html: bool,
    losses: &mut BTreeSet<ConversionLoss>,
) -> String {
    let spans = document.style_spans_for_region(&range);
    let mut boundaries = BTreeSet::from([range.start, range.end]);
    for span in &spans {
        boundaries.insert(span.range.start.max(range.start));
        boundaries.insert(span.range.end.min(range.end));
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    let mut output = String::new();
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
            let mut value = html_text(text).replace('\n', "<br>");
            if code {
                value = format!("<code>{value}</code>");
            }
            if properties != CharacterProperties::default() {
                let (open, close) = super::html::character_wrapper(&properties);
                value = format!("{open}{value}{close}");
            }
            output.push_str(&value);
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
            let mut value = if code {
                let marker = "`".repeat(longest_run(text, '`') + 1);
                let pad = if text.starts_with('`')
                    || text.ends_with('`')
                    || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty())
                {
                    " "
                } else {
                    ""
                };
                format!("{marker}{pad}{text}{pad}{marker}")
            } else {
                markdown_text(text)
            };
            if !code {
                if bold && italic {
                    value = format!("***{value}***");
                } else if bold {
                    value = format!("**{value}**");
                } else if italic {
                    value = format!("*{value}*");
                }
            } else if bold || italic {
                losses.insert(ConversionLoss::Styling);
            }
            output.push_str(&value);
        }
    }
    output
}
fn longest_run(text: &str, delimiter: char) -> usize {
    text.split(|c| c != delimiter)
        .map(str::len)
        .max()
        .unwrap_or(0)
}
fn html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn markdown_text(text: &str) -> String {
    let mut output = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '#') {
            output.push('\\');
        }
        output.push(c);
    }
    output
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
) -> String {
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
    let mut output = if ordered {
        format!("<ol start=\"{ordinal}\">")
    } else {
        "<ul>".to_owned()
    };
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
        let rendered = inline(document, body.clone(), true, losses);
        let value = if ordered {
            format!(" value=\"{ordinal}\"")
        } else {
            String::new()
        };
        output.push_str(&format!(
            "<li{value}{}>{rendered}",
            whitespace_attribute(&document.text()[body])
        ));
        *index += 1;
        while *index < blocks.len()
            && matches!(blocks[*index].kind, BlockKind::ListItem {level: next, ..} if next > level)
        {
            if matches!(blocks[*index].kind, BlockKind::ListItem {level: next, ..} if next != level + 1)
            {
                losses.insert(ConversionLoss::Structure);
            }
            output.push_str(&html_list(document, blocks, index, losses));
        }
        output.push_str("</li>");
    }
    output.push_str(&format!("</{tag}>"));
    output
}
