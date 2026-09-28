//! Internal Markdown spelling for passive HTML edits and clipboard imports.
use super::{
    BlockKind, CharacterProperties, FontSlant, FormattedDocument, SemanticInlineStyle,
    StyleApplication,
};
use std::{collections::BTreeSet, ops::Range};
use unicode_segmentation::UnicodeSegmentation;
#[path = "markdown_serialization_containers.rs"]
pub(super) mod containers;

struct MarkdownWriter {
    source: String,
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
    Markdown,
}

impl MarkdownWriter {
    fn new() -> Self {
        Self {
            source: String::new(),
            line_prefix: MarkdownLinePrefix::Empty,
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

    fn text(&mut self, text: &str, _semantic_start: usize, spelling: TextSpelling) {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let visible = grapheme;
            match spelling {
                TextSpelling::Literal => self.push_str(visible),
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
                        let remaining = &text[offset + character_offset..];
                        if starts_block
                            || matches!(character, '\\' | '*' | '_' | '`' | '#' | '>')
                            || character == '<'
                                && super::projection::markdown_inline_break_length(remaining)
                                    .is_some()
                            || character == '&'
                                && (remaining.starts_with("&#")
                                    || super::html::reference(remaining, false).is_some())
                        {
                            self.push('\\');
                        }
                        self.push(character);
                    }
                }
            }
        }
    }
}
pub(super) fn markdown_document(document: &FormattedDocument) -> String {
    let semantic = document;
    let mut output = MarkdownWriter::new();
    let blocks = semantic.blocks();
    let has_containers = blocks.iter().any(|block| !block.containers.is_empty());
    if has_containers {
        containers::write(&semantic, blocks, &mut output);
    }
    let mut index = if has_containers { blocks.len() } else { 0 };
    while index < blocks.len() {
        let block = &blocks[index];
        if index > 0 {
            output.push('\n');
            if !matches!(block.kind, BlockKind::ListItem { .. })
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
            {
                let fence = "`".repeat(longest_run(&text, '`').max(2) + 1);
                output.push_str(&fence);
                output.push('\n');
                output.text(text, start, TextSpelling::Literal);
                output.push('\n');
                output.push_str(&fence);
            }
            index += 1;
            continue;
        }
        let body = block.range.clone();

        let quoted = block.style.0 == "Block quote";
        let (open, close) = match &block.kind {
            BlockKind::Heading(level) => (
                format!("{} ", "#".repeat(usize::from(*level))),
                String::new(),
            ),
            BlockKind::ListItem {
                ordered,
                ordinal,
                level,
                ..
            } => {
                let mut marker = "  ".repeat(usize::from(*level));
                if *ordered {
                    marker.push_str(&format!("{ordinal}. "));
                } else {
                    marker.push_str("- ");
                }
                (marker, String::new())
            }
            _ => (String::new(), String::new()),
        };
        if quoted {
            output.push_str("> ");
        }
        output.push_str(&open);
        // Container syntax (especially >) does not make its first visible
        // text safe from Markdown's paragraph/list marker grammar.
        output.line_prefix = MarkdownLinePrefix::Empty;
        inline(&semantic, body, &mut output);
        output.push_str(&close);
        index += 1;
    }
    output.source
}

fn inline(document: &FormattedDocument, range: Range<usize>, output: &mut MarkdownWriter) {
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
                    let value = document
                        .style_sheet()
                        .named_character_declarations(Some(id))
                        .expect("validated character style");
                    super::rich_text::overlay(&mut properties, &value);
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
        {
            markdown_inline_text(text, at, properties, code, output);
        }
    }
}
/// Clipboard imports reuse the Markdown writer's escaping and supported
/// inline vocabulary without creating an HTML editing document.
pub(super) fn markdown_character_fragment(
    document: &FormattedDocument,
    range: Range<usize>,
    properties: &CharacterProperties,
) -> String {
    let mut output = MarkdownWriter {
        source: String::new(),
        line_prefix: MarkdownLinePrefix::Empty,
    };
    markdown_inline_text(
        &document.text()[range.clone()],
        range.start,
        properties.clone(),
        false,
        &mut output,
    );
    output.source
}

fn markdown_inline_text(
    text: &str,
    at: usize,
    properties: CharacterProperties,
    code: bool,
    output: &mut MarkdownWriter,
) {
    let bold =
        properties.bold == Some(true) || properties.weight.is_some_and(|weight| weight >= 600);
    let italic = matches!(
        properties.slant,
        Some(FontSlant::Italic | FontSlant::Oblique)
    );
    if code {
        let visible = text;
        let marker = "`".repeat(longest_run(&visible, '`') + 1);
        let pad = if visible.starts_with('`')
            || visible.ends_with('`')
            || (visible.starts_with(' ') && visible.ends_with(' ') && !visible.trim().is_empty())
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

fn longest_run(text: &str, delimiter: char) -> usize {
    text.split(|c| c != delimiter)
        .map(str::len)
        .max()
        .unwrap_or(0)
}
