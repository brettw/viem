//! Passive Viem-owned HTML CSS. The frozen v1 grammar remains readable;
//! v2 uses native element selectors and ordinary declarations, with metadata
//! only for identities, inheritance, and semantics CSS cannot preserve.
use super::html::{self, TokenKind};
use super::rich_text::{overlay as overlay_character, overlay_block};
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

const OPEN: &str = "<style id=\"viem-styles\" data-viem-version=\"1\">";
const OPEN_V2: &str = "<style id=\"viem-styles\" data-viem-version=\"2\">";

#[derive(Clone)]
pub(super) struct Rule {
    pub range: Range<usize>,
    pub definition: StyleDefinitionEdit,
    version: u8,
}

impl Rule {
    fn selector<'a>(&self, text: &'a str) -> &'a str {
        text[self.range.clone()].split_once(" {\n").unwrap().0
    }
}

pub(super) struct OwnedSheet {
    pub sheet: StyleSheet,
    pub rules: Vec<Rule>,
    pub close: Option<usize>,
    empty_v2_marker: bool,
}

impl OwnedSheet {
    #[cfg(test)]
    pub(super) fn has_native_rules(&self) -> bool {
        self.rules.iter().any(|rule| {
            is_native_style(
                &self.sheet,
                rule.definition.style_id(),
                !rule.definition.is_block(),
            )
        })
    }
}

/// Native HTML carries these assignments without a class or a style sheet.
/// List selectors describe item depth, so browser list containers continue to
/// own the ordinary bullet, indentation, and marker placement.
pub(super) fn native_style_selector(
    sheet: &StyleSheet,
    id: &StyleId,
    character: bool,
) -> Option<String> {
    if character {
        return (id.0 == "Code").then(|| "code".into());
    }
    if id == &sheet.base_document {
        Some("body".into())
    } else if id == &sheet.base_paragraph {
        Some("p".into())
    } else if id.0 == "Block quote" {
        Some("blockquote".into())
    } else if let Some(level) = builtin_heading(id) {
        Some(format!("h{level}"))
    } else if let Some((ordered, level)) = id.list_family_level() {
        let ancestors = "li ".repeat(usize::from(level - 1));
        Some(format!("{ancestors}{} > li", if ordered { "ol" } else { "ul" }))
    } else if let Some(level) =
        id.0.strip_prefix("List")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|level| (1..=256).contains(level) && id.0 == format!("List{level}"))
    {
        Some(
            std::iter::repeat("li")
                .take(usize::from(level))
                .collect::<Vec<_>>()
                .join(" "),
        )
    } else {
        (id.0 == "Code Block").then(|| "pre".into())
    }
}

pub(super) fn is_native_style(sheet: &StyleSheet, id: &StyleId, character: bool) -> bool {
    native_style_selector(sheet, id, character).is_some()
        || (character && id == &sheet.base_character)
}

fn selector_v2(sheet: &StyleSheet, id: &StyleId, character: bool) -> String {
    native_style_selector(sheet, id, character)
        .unwrap_or_else(|| format!(".{}", class_name(id, character)))
}

fn native_definition(selector: &str) -> Option<(StyleSheet, StyleId, bool)> {
    let mut sheet = StyleSheet::for_format(Format::Html);
    let character = selector == "code";
    let id = match selector {
        "body" => sheet.base_document.clone(),
        "p" => sheet.base_paragraph.clone(),
        "pre" => StyleId::from("Code Block"),
        "blockquote" => StyleId::from("Block quote"),
        "code" => StyleId::from("Code"),
        _ if selector.starts_with('h') && selector.len() == 2 => {
            let id = StyleId(format!("Heading{}", &selector[1..]));
            builtin_heading(&id)?;
            id
        }
        _ if selector.ends_with("ul > li") || selector.ends_with("ol > li") => {
            let ordered = selector.ends_with("ol > li");
            let prefix = &selector[..selector.len() - 7];
            if !prefix.is_empty() && !prefix.split_terminator(' ').all(|s| s == "li") {
                return None;
            }
            let level = prefix.split_whitespace().count();
            if level > 3 {
                return None;
            }
            sheet.list_style_id(ordered, level as u8)
        }
        _ => {
            let levels = selector.split(' ').collect::<Vec<_>>();
            if levels.is_empty() || levels.len() > 256 || levels.iter().any(|s| *s != "li") {
                return None;
            }
            sheet.ensure_list_level(levels.len() as u16);
            StyleId(format!("List{}", levels.len()))
        }
    };
    Some((sheet, id, character))
}

pub(super) fn class_name(id: &StyleId, character: bool) -> String {
    let encoded =
        id.0.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
    format!("viem-{}-{encoded}", if character { "c" } else { "p" })
}

fn selector(sheet: &StyleSheet, id: &StyleId, character: bool) -> String {
    if !character {
        if id == &sheet.base_document {
            return "body".into();
        }
        if id == &sheet.base_paragraph {
            return "p".into();
        }
        if let Some(level) =
            id.0.strip_prefix("Heading")
                .filter(|v| matches!(*v, "1" | "2" | "3" | "4" | "5" | "6"))
        {
            return format!("h{level}");
        }
    }
    format!(".{}", class_name(id, character))
}

fn builtin_heading(id: &StyleId) -> Option<u8> {
    match id.0.strip_prefix("Heading")? {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        "5" => Some(5),
        "6" => Some(6),
        _ => None,
    }
}

pub(super) fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        if c.is_control() || matches!(c, '\\' | '"' | '<' | '>' | '{' | '}') {
            out.push_str(&format!("\\{:x} ", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

fn unquote(value: &str) -> Option<String> {
    let text = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c.is_control() {
            return None;
        }
        if c != '\\' {
            result.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(chars.next()?);
        }
        if hex.is_empty() {
            result.push(chars.next()?);
        } else {
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            result.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
        }
    }
    Some(result)
}

fn color_value(value: Color) -> String {
    format!(
        "{} {} {} {}",
        value.red, value.green, value.blue, value.alpha
    )
}
fn parse_color(value: &str) -> Option<Color> {
    let values = value
        .split(' ')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 4
        || values
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return None;
    }
    Some(Color {
        red: values[0],
        green: values[1],
        blue: values[2],
        alpha: values[3],
    })
}
fn direction(value: WritingDirection) -> &'static str {
    match value {
        WritingDirection::Natural => "natural",
        WritingDirection::LeftToRight => "ltr",
        WritingDirection::RightToLeft => "rtl",
    }
}
fn parse_direction(value: &str) -> Option<WritingDirection> {
    match value {
        "natural" => Some(WritingDirection::Natural),
        "ltr" => Some(WritingDirection::LeftToRight),
        "rtl" => Some(WritingDirection::RightToLeft),
        _ => None,
    }
}
fn strings(value: &[String]) -> String {
    value
        .iter()
        .map(|v| quote(v))
        .collect::<Vec<_>>()
        .join(", ")
}
fn parse_strings(value: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut inside = false;
    let mut escaped = false;
    for (at, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            inside = !inside;
        }
        if c == ',' && !inside {
            result.push(unquote(value[start..at].trim())?);
            start = at + 1;
        }
    }
    result.push(unquote(value[start..].trim())?);
    Some(result)
}

fn properties(
    character: &CharacterProperties,
    block: &BlockProperties,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    macro_rules! item {
        ($key:literal,$field:expr,$convert:expr) => {
            if let Some(value) = &$field {
                out.push(($key, ($convert)(value)));
            }
        };
    }
    item!(
        "character-font-families",
        character.font_families,
        |v: &Vec<String>| strings(v)
    );
    item!("character-size", character.size, |v: &f32| v.to_string());
    item!("character-bold", character.bold, |v: &bool| v.to_string());
    item!("character-weight", character.weight, |v: &u16| v
        .to_string());
    item!("character-slant", character.slant, |v: &FontSlant| {
        match v {
            FontSlant::Upright => "normal",
            FontSlant::Italic => "italic",
            FontSlant::Oblique => "oblique",
        }
        .to_owned()
    });
    item!("character-foreground", character.foreground, |v: &Color| {
        color_value(*v)
    });
    item!("character-background", character.background, |v: &Color| {
        color_value(*v)
    });
    item!("character-underline", character.underline, |v: &bool| v
        .to_string());
    item!(
        "character-strikethrough",
        character.strikethrough,
        |v: &bool| v.to_string()
    );
    item!("character-language", character.language, |v: &String| v
        .clone());
    item!(
        "character-direction",
        character.direction,
        |v: &WritingDirection| direction(*v).to_owned()
    );
    item!(
        "character-open-type-features",
        character.open_type_features,
        |v: &BTreeMap<String, u32>| v
            .iter()
            .map(|(tag, count)| format!("{} {count}", quote(tag)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    item!(
        "character-letter-spacing",
        character.letter_spacing,
        |v: &f32| v.to_string()
    );
    item!(
        "character-baseline-shift",
        character.baseline_shift,
        |v: &f32| v.to_string()
    );
    item!(
        "paragraph-spacing-before",
        block.spacing_before,
        |v: &f32| v.to_string()
    );
    item!("paragraph-spacing-after", block.spacing_after, |v: &f32| v
        .to_string());
    item!(
        "paragraph-line-spacing",
        block.line_spacing,
        |v: &LineSpacing| match v {
            LineSpacing::Normal => "normal".into(),
            LineSpacing::Multiplier(n) => format!("multiplier {n}"),
            LineSpacing::AtLeast(n) => format!("at-least {n}"),
            LineSpacing::Exact(n) => format!("exact {n}"),
        }
    );
    item!(
        "paragraph-first-line-indent",
        block.first_line_indent,
        |v: &f32| v.to_string()
    );
    item!(
        "paragraph-leading-indent",
        block.leading_indent,
        |v: &f32| v.to_string()
    );
    item!(
        "paragraph-trailing-indent",
        block.trailing_indent,
        |v: &f32| v.to_string()
    );
    item!(
        "paragraph-alignment",
        block.alignment,
        |v: &ParagraphAlignment| match v {
            ParagraphAlignment::Start => "start",
            ParagraphAlignment::Center => "center",
            ParagraphAlignment::End => "end",
        }
        .to_owned()
    );
    item!(
        "paragraph-base-direction",
        block.base_direction,
        |v: &WritingDirection| direction(*v).to_owned()
    );
    item!("canvas-background", block.background, |v: &Color| {
        color_value(*v)
    });
    item!("canvas-padding-top", block.padding_top, |v: &f32| v
        .to_string());
    item!("canvas-padding-right", block.padding_right, |v: &f32| v
        .to_string());
    item!("canvas-padding-bottom", block.padding_bottom, |v: &f32| v
        .to_string());
    item!("canvas-padding-left", block.padding_left, |v: &f32| v
        .to_string());
    out
}

fn parse_property(
    key: &str,
    value: &str,
    c: &mut CharacterProperties,
    b: &mut BlockProperties,
) -> Option<()> {
    let float = || value.parse::<f32>().ok().filter(|v| v.is_finite());
    match key {
        "character-font-families" => c.font_families = Some(parse_strings(value)?),
        "character-size" => c.size = Some(float()?),
        "character-weight" => c.weight = Some(value.parse().ok()?),
        "character-bold" => c.bold = Some(value.parse().ok()?),
        "character-slant" => {
            c.slant = Some(match value {
                "normal" => FontSlant::Upright,
                "italic" => FontSlant::Italic,
                "oblique" => FontSlant::Oblique,
                _ => return None,
            })
        }
        "character-foreground" => c.foreground = Some(parse_color(value)?),
        "character-background" => c.background = Some(parse_color(value)?),
        "character-underline" => c.underline = Some(value.parse().ok()?),
        "character-strikethrough" => c.strikethrough = Some(value.parse().ok()?),
        "character-language" => c.language = Some(value.into()),
        "character-direction" => c.direction = Some(parse_direction(value)?),
        "character-open-type-features" => {
            let mut features = BTreeMap::new();
            if !value.is_empty() {
                for part in value.split(", ") {
                    let (tag, count) = part.rsplit_once(' ')?;
                    features.insert(unquote(tag)?, count.parse().ok()?);
                }
            }
            c.open_type_features = Some(features);
        }
        "character-letter-spacing" => c.letter_spacing = Some(float()?),
        "character-baseline-shift" => c.baseline_shift = Some(float()?),
        "paragraph-spacing-before" => b.spacing_before = Some(float()?),
        "paragraph-spacing-after" => b.spacing_after = Some(float()?),
        "paragraph-line-spacing" => {
            b.line_spacing = Some(if value == "normal" {
                LineSpacing::Normal
            } else {
                let (kind, number) = value.split_once(' ')?;
                let n = number.parse().ok()?;
                match kind {
                    "multiplier" => LineSpacing::Multiplier(n),
                    "at-least" => LineSpacing::AtLeast(n),
                    "exact" => LineSpacing::Exact(n),
                    _ => return None,
                }
            })
        }
        "paragraph-first-line-indent" => b.first_line_indent = Some(float()?),
        "paragraph-leading-indent" => b.leading_indent = Some(float()?),
        "paragraph-trailing-indent" => b.trailing_indent = Some(float()?),
        "paragraph-alignment" => {
            b.alignment = Some(match value {
                "start" => ParagraphAlignment::Start,
                "center" => ParagraphAlignment::Center,
                "end" => ParagraphAlignment::End,
                _ => return None,
            })
        }
        "paragraph-base-direction" => b.base_direction = Some(parse_direction(value)?),
        "canvas-background" => b.background = Some(parse_color(value)?),
        "canvas-padding-top" => b.padding_top = Some(float()?),
        "canvas-padding-right" => b.padding_right = Some(float()?),
        "canvas-padding-bottom" => b.padding_bottom = Some(float()?),
        "canvas-padding-left" => b.padding_left = Some(float()?),
        _ => return None,
    }
    Some(())
}

fn parse_rule(text: &str) -> Option<StyleDefinitionEdit> {
    let (selector, body) = text.split_once(" {\n")?;
    let body = body.strip_suffix("}\n")?;
    let mut values = BTreeMap::new();
    for (key, value) in html::declarations(body) {
        if key.starts_with("--viem-") {
            if values.insert(key, unquote(value)?).is_some() {
                return None;
            }
        }
    }
    let id = StyleId(values.remove("--viem-style-id")?);
    if let Some(deleted) = values.remove("--viem-style-deleted") {
        if deleted != "true"
            || !values.is_empty()
            || !StyleSheet::builtin_block(&id)
            || selector != self::selector(&StyleSheet::default(), &id, false)
        {
            return None;
        }
        return Some(StyleDefinitionEdit::DeleteBlock(id));
    }
    let metadata = StyleDefinitionMetadata {
        display_name: values.remove("--viem-style-name")?,
        origin: StyleDefinitionOrigin::SourceBacked,
    };
    let role = values.remove("--viem-style-role")?;
    let parent = values.remove("--viem-based-on").map(StyleId);
    let next = values.remove("--viem-next-style").map(StyleId);
    let mut character = CharacterProperties::default();
    let mut block = BlockProperties::default();
    for (key, value) in values {
        parse_property(
            key.strip_prefix("--viem-prop-")?,
            &value,
            &mut character,
            &mut block,
        )?;
    }
    let sheet = StyleSheet::default();
    if selector != self::selector(&sheet, &id, role == "character") {
        return None;
    }
    match role.as_str() {
        "character" if next.is_none() && block == BlockProperties::default() => {
            Some(StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id,
                    based_on: parent,
                    properties: character,
                },
                metadata,
            })
        }
        "document" | "paragraph" => Some(StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id,
                based_on: parent,
                next_paragraph_style: next,
                role: if role == "document" {
                    BlockRole::Document
                } else {
                    BlockRole::Paragraph
                },
                character,
                block,
            },
            metadata,
        }),
        _ => None,
    }
}

pub(super) fn read(text: &str) -> OwnedSheet {
    read_with_semantics(text, &super::html5_tree::tokens(text))
}

/// Source ownership follows the recovered live HTML tree. Raw tokens retain
/// exact grammar spelling, but cannot establish whether an apparent element
/// is template content, foreign/atomic content, or double-escaped script text.
fn active_source_elements(tokens: &[html::Token]) -> Vec<&html::Token> {
    let mut active = Vec::new();
    let mut stack: Vec<(&str, bool)> = Vec::new();
    for token in tokens {
        let TokenKind::Tag(tag) = &token.kind else {
            continue;
        };
        if tag.end {
            if !stack.last().is_some_and(|(_, excluded)| *excluded) {
                active.push(token);
            }
            if let Some(index) = stack.iter().rposition(|(name, _)| *name == tag.name) {
                stack.truncate(index);
            }
            continue;
        }
        let excluded = stack.last().is_some_and(|(_, excluded)| *excluded);
        if !excluded && !token.range.is_empty() {
            active.push(token);
        }
        let excluded = excluded
            || matches!(
                tag.name.as_str(),
                "script"
                    | "style"
                    | "template"
                    | "title"
                    | "noscript"
                    | "img"
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
            );
        stack.push((&tag.name, excluded));
    }
    active
}

pub(super) fn read_with_semantics(text: &str, semantic_tokens: &[html::Token]) -> OwnedSheet {
    let mut sheet = StyleSheet::for_format(super::Format::Html);
    sheet.mark_html_base_styles_source_backed();
    let tokens = html::tokenize(text);
    let eligible = active_source_elements(semantic_tokens)
        .into_iter()
        .filter_map(|token| {
            matches!(&token.kind,TokenKind::Tag(tag) if !tag.end && tag.name=="style")
                .then_some(token.range.start)
        })
        .collect::<BTreeSet<_>>();
    let mut rules = Vec::new();
    let mut close = None;
    let mut empty_v2_marker = false;
    for (index, token) in tokens.iter().enumerate() {
        if !eligible.contains(&token.range.start) {
            continue;
        }
        let version = match &text[token.range.clone()] {
            OPEN => 1,
            OPEN_V2 => 2,
            _ => continue,
        };
        let Some(end) = tokens[index + 1..]
            .iter()
            .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if tag.end&&tag.name=="style"))
        else {
            break;
        };
        if version == 2 {
            if text[token.range.end..end.range.start].trim().is_empty() {
                empty_v2_marker = true;
            } else {
                close = Some(end.range.start);
            }
        }
        let mut at = token.range.end;
        while at < end.range.start {
            if text.as_bytes()[at].is_ascii_whitespace() {
                at += 1;
                continue;
            }
            let Some(length) = text[at..end.range.start].find("}\n") else {
                break;
            };
            let range = at..at + length + 2;
            let definition = if version == 1 {
                parse_rule(&text[range.clone()])
            } else {
                parse_rule_v2(&text[range.clone()])
            };
            if let Some(definition) = definition {
                rules.push(Rule {
                    range: range.clone(),
                    definition,
                    version,
                });
            }
            at = range.end;
        }
    }
    let definitions = rules
        .iter()
        .map(|rule| rule.definition.clone())
        .collect::<Vec<_>>();
    if sheet.install_source_definitions(&definitions).is_err() {
        rules.clear();
        return OwnedSheet {
            sheet: {
                let mut s = StyleSheet::for_format(super::Format::Html);
                s.mark_html_base_styles_source_backed();
                if empty_v2_marker {
                    s.mark_html_export_definitions_source_backed();
                }
                s
            },
            rules,
            close,
            empty_v2_marker,
        };
    }
    // Only the exact declared grammar is adopted. Unknown declarations/whitespace
    // remain opaque and never become targets for rule replacement.
    rules.retain(|rule| {
        let id = rule.definition.style_id();
        let character = !rule.definition.is_block();
        let canonical = if rule.version == 1 {
            canonical_v1_spelling(&sheet, id, character)
        } else {
            write_rule_for_selector(&sheet, id, character, rule.selector(text))
        };
        canonical.as_deref() == Some(&text[rule.range.clone()])
            // Early v2 tombstones inherited the body font. Retain that exact
            // import spelling; all subsequent authoring uses Paragraph CSS.
            || (rule.version == 2
                && matches!(rule.definition, StyleDefinitionEdit::DeleteBlock(_))
                && native_style_selector(&sheet, id, false).as_deref() == Some(rule.selector(text))
                && text[rule.range.clone()] == format!(
                    "{} {{\n  --viem-style-deleted: \"true\";\n  font: inherit;\n  margin: 0;\n}}\n",
                    rule.selector(text),
                ))
    });
    let definitions = rules
        .iter()
        .map(|rule| rule.definition.clone())
        .collect::<Vec<_>>();
    let mut exact = StyleSheet::for_format(super::Format::Html);
    exact.mark_html_base_styles_source_backed();
    if exact.install_source_definitions(&definitions).is_err() {
        rules.clear();
        sheet = StyleSheet::for_format(super::Format::Html);
        sheet.mark_html_base_styles_source_backed();
    } else {
        sheet = exact;
    }
    if empty_v2_marker
        || rules.iter().any(|rule| {
            rule.version == 2
                && is_native_style(
                    &sheet,
                    rule.definition.style_id(),
                    !rule.definition.is_block(),
                )
        })
    {
        sheet.mark_html_export_definitions_source_backed();
    }
    OwnedSheet {
        sheet,
        rules,
        close,
        empty_v2_marker,
    }
}

pub(super) fn select_class(sheet: &StyleSheet, classes: &str, character: bool) -> Option<StyleId> {
    for token in classes.split_ascii_whitespace() {
        let id = if character {
            sheet
                .character_styles()
                .find(|s| class_name(&s.id, true) == token)
                .map(|s| s.id.clone())
        } else {
            sheet
                .block_styles()
                .find(|s| s.role == BlockRole::Paragraph && class_name(&s.id, false) == token)
                .map(|s| s.id.clone())
        };
        if id.is_some() {
            return id;
        }
    }
    None
}

pub(super) fn character_chain(sheet: &StyleSheet, id: &StyleId) -> CharacterProperties {
    let mut chain = Vec::new();
    let mut current = Some(id);
    while let Some(id) = current {
        let Some(style) = sheet.character_style(id) else {
            break;
        };
        chain.push(style);
        current = style.based_on.as_ref();
    }
    let mut result = CharacterProperties::default();
    for style in chain.into_iter().rev() {
        overlay_character(&mut result, &style.properties);
    }
    result
}
pub(super) fn remove_named_overrides(
    direct: &mut CharacterProperties,
    named: &CharacterProperties,
) {
    macro_rules! clear {($($field:ident),*)=>{$(if named.$field.is_some(){direct.$field=None;})*};}
    clear!(
        font_families,
        size,
        weight,
        bold,
        slant,
        foreground,
        background,
        underline,
        strikethrough,
        language,
        direction,
        open_type_features,
        letter_spacing,
        baseline_shift
    );
}

pub(super) fn block_css(properties: &BlockProperties) -> String {
    let mut out = String::new();
    if let Some(background) = properties.background {
        let declarations = html::character_css(&CharacterProperties {
            background: Some(background),
            ..Default::default()
        });
        out.push_str(&declarations);
        out.push_str("; ");
    }
    macro_rules! length {
        ($key:literal,$field:ident) => {
            if let Some(v) = properties.$field {
                out.push_str(&format!("{}: {v}pt; ", $key));
            }
        };
    }
    length!("margin-block-start", spacing_before);
    length!("margin-block-end", spacing_after);
    length!("margin-inline-start", leading_indent);
    length!("margin-inline-end", trailing_indent);
    length!("text-indent", first_line_indent);
    length!("padding-top", padding_top);
    length!("padding-right", padding_right);
    length!("padding-bottom", padding_bottom);
    length!("padding-left", padding_left);
    if let Some(v) = properties.alignment {
        out.push_str(&format!(
            "text-align: {}; ",
            match v {
                ParagraphAlignment::Start => "start",
                ParagraphAlignment::Center => "center",
                ParagraphAlignment::End => "end",
            }
        ));
    }
    if let Some(v) = properties.base_direction {
        if v != WritingDirection::Natural {
            out.push_str(&format!("direction: {}; ", direction(v)));
        }
    }
    if let Some(v) = properties.line_spacing {
        out.push_str(&format!(
            "line-height: {}; ",
            match v {
                LineSpacing::Normal => "normal".into(),
                LineSpacing::Multiplier(n) => n.to_string(),
                LineSpacing::AtLeast(n) | LineSpacing::Exact(n) => format!("{n}pt"),
            }
        ));
    }
    out
}

// Import validation only: the frozen v1 grammar includes its exact fallback CSS.
// All authored definitions use write_rule and are emitted as v2.
fn canonical_v1_spelling(sheet: &StyleSheet, id: &StyleId, character: bool) -> Option<String> {
    if !character && StyleSheet::builtin_block(id) && sheet.block_style(id).is_none() {
        // An additive v1 rule persists deletion of an otherwise implicit
        // heading definition. Its ordinary CSS presents hN as the default
        // paragraph in passive readers as well as in the editable projection.
        let paragraph = canonical_v1_spelling(sheet, &sheet.base_paragraph, false)?;
        let (_, body) = paragraph.split_once(" {\n")?;
        let mut out = format!("{} {{\n  --viem-style-id: {};\n  --viem-style-deleted: \"true\";\n  font: inherit;\n  margin: 0;\n",
            selector(sheet, id, false), quote(&id.0));
        for (key, value) in html::declarations(body.strip_suffix("}\n")?) {
            if !key.starts_with("--viem-") {
                out.push_str(&format!("  {key}: {value};\n"));
            }
        }
        out.push_str("}\n");
        return Some(out);
    }
    let (metadata, parent, next, role, own_character, own_block) = if character {
        let style = sheet.character_style(id)?;
        (
            sheet.character_style_metadata(id)?,
            style.based_on.as_ref(),
            None,
            "character",
            style.properties.clone(),
            BlockProperties::default(),
        )
    } else {
        let style = sheet.block_style(id)?;
        if style.role == BlockRole::Document && id != &sheet.base_document {
            return None;
        }
        (
            sheet.block_style_metadata(id)?,
            style.based_on.as_ref(),
            style.next_paragraph_style.as_ref(),
            if style.role == BlockRole::Document {
                "document"
            } else {
                "paragraph"
            },
            style.character.clone(),
            style.block.clone(),
        )
    };
    let mut out = format!("{} {{\n", selector(sheet, id, character));
    for (key, value) in [
        ("style-id", id.0.as_str()),
        ("style-name", metadata.display_name.as_str()),
        ("style-role", role),
    ] {
        out.push_str(&format!("  --viem-{key}: {};\n", quote(value)));
    }
    if let Some(parent) = parent {
        out.push_str(&format!("  --viem-based-on: {};\n", quote(&parent.0)));
    }
    if let Some(next) = next {
        out.push_str(&format!("  --viem-next-style: {};\n", quote(&next.0)));
    }
    for (key, value) in properties(&own_character, &own_block) {
        out.push_str(&format!("  --viem-prop-{key}: {};\n", quote(&value)));
    }
    let (effective_character, effective_block) = effective_style_properties(sheet, id, character)?;
    let css = format!(
        "{}; {}",
        html::character_css(&effective_character),
        block_css(&effective_block)
    );
    for (key, value) in html::declarations(&css) {
        // Owned metadata already records sparse base weight and relative
        // emphasis. Inline-only helper declarations are neither canonical
        // metadata nor standard fallback CSS.
        if !key.starts_with("--viem-") {
            out.push_str(&format!("  {key}: {value};\n"));
        }
    }
    out.push_str("}\n");
    Some(out)
}

/// Only genuinely unrepresentable sparse information uses metadata in v2.
/// Ordinary declarations are parsed first, then the small residual restores
/// graph-dependent inheritance and distinctions such as minimum line height.
fn v2_css_properties(css: &str, character: &mut CharacterProperties, block: &mut BlockProperties) {
    html::apply_css(css, character, block);
    for (key, value) in html::declarations(css) {
        let Some(number) = value.strip_suffix("pt").and_then(|v| v.parse::<f32>().ok()) else {
            continue;
        };
        match key {
            "padding-top" => block.padding_top = Some(number),
            "padding-right" => block.padding_right = Some(number),
            "padding-bottom" => block.padding_bottom = Some(number),
            "padding-left" => block.padding_left = Some(number),
            _ => {}
        }
    }
}

fn parse_rule_v2(text: &str) -> Option<StyleDefinitionEdit> {
    let (selector, body) = text.split_once(" {\n")?;
    let body = body.strip_suffix("}\n")?;
    let mut values = BTreeMap::new();
    for (key, value) in html::declarations(body) {
        if key.starts_with("--viem-") && values.insert(key, unquote(value)?).is_some() {
            return None;
        }
    }
    let native = native_definition(selector);
    let native_selector = native.is_some();
    let (id, character, mut parent, mut next, mut name, mut c, mut b) =
        if let Some((sheet, id, character)) = native {
            if character {
                let style = sheet.character_style(&id)?;
                (
                    id.clone(),
                    true,
                    style.based_on.clone(),
                    None,
                    sheet.character_style_metadata(&id)?.display_name.clone(),
                    style.properties.clone(),
                    BlockProperties::default(),
                )
            } else {
                let style = sheet.block_style(&id)?;
                (
                    id.clone(),
                    false,
                    style.based_on.clone(),
                    style.next_paragraph_style.clone(),
                    sheet.block_style_metadata(&id)?.display_name.clone(),
                    style.character.clone(),
                    style.block.clone(),
                )
            }
        } else {
            let id = StyleId(values.remove("--viem-style-id")?);
            let character = match values.remove("--viem-style-role")?.as_str() {
                "paragraph" => false,
                "character" => true,
                _ => return None,
            };
            if selector != format!(".{}", class_name(&id, character)) {
                return None;
            }
            (
                id,
                character,
                None,
                None,
                String::new(),
                CharacterProperties::default(),
                BlockProperties::default(),
            )
        };
    if let Some(deleted) = values.remove("--viem-style-deleted") {
        return (deleted == "true"
            && values.is_empty()
            && !character
            && StyleSheet::builtin_block(&id))
        .then_some(StyleDefinitionEdit::DeleteBlock(id));
    }
    if let Some(value) = values.remove("--viem-style-name") {
        name = value;
    }
    if let Some(value) = values.remove("--viem-based-on") {
        parent = (!value.is_empty()).then_some(StyleId(value));
    }
    if let Some(value) = values.remove("--viem-next-style") {
        next = (!value.is_empty()).then_some(StyleId(value));
    }
    v2_css_properties(body, &mut c, &mut b);
    if let Some(level) = native_selector.then(|| list_style_level(&id)).flatten() {
        if html::declarations(body)
            .iter()
            .any(|(key, _)| *key == "margin-inline-start")
        {
            if let Some(indent) = &mut b.leading_indent {
                *indent += 32.0 * f32::from(level);
            }
        }
    }
    let mut props = properties(&c, &b).into_iter().collect::<BTreeMap<_, _>>();
    if let Some(clear) = values.remove("--viem-inherit") {
        for key in clear.split(' ') {
            // Validate even a clear key, so future schemas stay opaque.
            if !props.contains_key(key) {
                return None;
            }
            props.remove(key);
        }
    }
    c = CharacterProperties::default();
    b = BlockProperties::default();
    for (key, value) in props {
        parse_property(key, &value, &mut c, &mut b)?;
    }
    for (key, value) in values {
        parse_property(key.strip_prefix("--viem-prop-")?, &value, &mut c, &mut b)?;
    }
    let metadata = StyleDefinitionMetadata {
        display_name: name,
        origin: StyleDefinitionOrigin::SourceBacked,
    };
    if character {
        next.is_none()
            .then_some(StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id,
                    based_on: parent,
                    properties: c,
                },
                metadata,
            })
    } else {
        let role = if id == StyleSheet::default().base_document {
            BlockRole::Document
        } else {
            BlockRole::Paragraph
        };
        Some(StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id,
                based_on: parent,
                next_paragraph_style: next,
                role,
                character: c,
                block: b,
            },
            metadata,
        })
    }
}

fn block_chain(sheet: &StyleSheet, id: &StyleId) -> Option<(CharacterProperties, BlockProperties)> {
    let mut chain = Vec::new();
    let mut current = Some(id);
    while let Some(id) = current {
        let style = sheet.block_style(id)?;
        chain.push(style);
        current = style.based_on.as_ref();
    }
    let mut c = CharacterProperties::default();
    let mut b = BlockProperties::default();
    for style in chain.into_iter().rev() {
        overlay_character(&mut c, &style.character);
        overlay_block(&mut b, &style.block);
    }
    Some((c, b))
}

fn effective_style_properties(
    sheet: &StyleSheet,
    id: &StyleId,
    character: bool,
) -> Option<(CharacterProperties, BlockProperties)> {
    let (c, mut b) = if character {
        (character_chain(sheet, id), BlockProperties::default())
    } else {
        block_chain(sheet, id)?
    };
    if character || id != &sheet.base_document {
        b.background = None;
        b.padding_top = None;
        b.padding_right = None;
        b.padding_bottom = None;
        b.padding_left = None;
    }
    Some((c, b))
}

fn list_style_level(id: &StyleId) -> Option<u16> {
    id.list_family_level()
        .map(|(_, level)| u16::from(level))
        .or_else(|| id.legacy_list_level())
}

fn minimal_style_css(
    sheet: &StyleSheet,
    id: &StyleId,
    character: bool,
    native_selector: bool,
) -> Option<String> {
    let document = !character && id == &sheet.base_document;
    let list_level = (!character && native_selector)
        .then(|| list_style_level(id))
        .flatten();
    let previous_list = list_level.filter(|level| *level > 1).and_then(|level| {
        let previous = if let Some((ordered, _)) = id.list_family_level() {
            sheet.list_style_id(ordered, (level - 2) as u8)
        } else {
            StyleId(format!("List{}", level - 1))
        };
        block_chain(sheet, &previous)
    });
    let (mut c, mut b) = effective_style_properties(sheet, id, character)?;
    let mut inherited = if document || character {
        CharacterProperties {
            weight: Some(400),
            slant: Some(FontSlant::Upright),
            underline: Some(false),
            strikethrough: Some(false),
            direction: Some(WritingDirection::Natural),
            open_type_features: Some(BTreeMap::new()),
            letter_spacing: Some(0.0),
            baseline_shift: Some(0.0),
            ..Default::default()
        }
    } else {
        block_chain(sheet, &sheet.base_document)?.0
    };
    if let Some((previous, _)) = &previous_list {
        inherited = previous.clone();
    }
    if !character && native_selector {
        if let Some(level) = builtin_heading(id) {
            inherited.weight = Some(700);
            // Browser heading sizes are intrinsic, not inherited from body.
            inherited.size = inherited
                .size
                .map(|size| size * [2.0, 1.5, 1.17, 1.0, 0.83, 0.67][usize::from(level - 1)]);
        }
    }
    if native_selector && (id.0 == "Code" || id.0 == "Code Block") {
        inherited.font_families = Some(vec!["monospace".into()]);
    }
    macro_rules! inherited { ($($field:ident),*) => {$(if c.$field == inherited.$field { c.$field = None; })*}; }
    inherited!(
        font_families,
        size,
        weight,
        slant,
        foreground,
        background,
        underline,
        strikethrough,
        direction,
        open_type_features,
        letter_spacing,
        baseline_shift
    );
    // A relative emphasis needs its base weight even when that base is inherited.
    if c.bold.is_some() && c.weight.is_none() {
        c.weight = inherited.weight;
    }
    if let Some(level) = list_level {
        // Container indentation is already supplied by ul/ol; this declaration
        // represents only a user's additional item indentation.
        if let Some(indent) = &mut b.leading_indent {
            *indent -= 32.0 * f32::from(level);
        }
    }
    let previous_css_block = previous_list
        .map(|(_, mut block)| {
            if let Some(indent) = &mut block.leading_indent {
                *indent -= 32.0 * f32::from(list_level.unwrap() - 1);
            }
            block
        })
        .unwrap_or_default();
    // The selector for a shallower item also matches deeper items. Emit a
    // default reset only when a preceding level actually changed that value.
    macro_rules! zero { ($($field:ident),*) => {$(if b.$field == Some(0.0)
        && previous_css_block.$field.unwrap_or(0.0) == 0.0 { b.$field = None; })*}; }
    zero!(
        first_line_indent,
        leading_indent,
        trailing_indent,
        padding_top,
        padding_right,
        padding_bottom,
        padding_left
    );
    if list_level.is_some() {
        zero!(spacing_before, spacing_after);
    }
    if b.line_spacing == Some(LineSpacing::Normal)
        && previous_css_block
            .line_spacing
            .unwrap_or(LineSpacing::Normal)
            == LineSpacing::Normal
    {
        b.line_spacing = None;
    }
    if b.alignment == Some(ParagraphAlignment::Start)
        && previous_css_block
            .alignment
            .unwrap_or(ParagraphAlignment::Start)
            == ParagraphAlignment::Start
    {
        b.alignment = None;
    }
    if b.base_direction == Some(WritingDirection::Natural) {
        b.base_direction = None;
    }
    Some(style_css(&c, &b))
}

fn style_css(c: &CharacterProperties, b: &BlockProperties) -> String {
    let css = format!("{}; {}", html::character_css(c), block_css(b));
    let mut result = String::new();
    for (key, value) in html::declarations(&css) {
        if !key.starts_with("--viem-") {
            let value = if key == "font-family"
                && matches!(
                    value,
                    "'monospace'" | "'serif'" | "'sans-serif'" | "'system-ui'"
                ) {
                &value[1..value.len() - 1]
            } else {
                value
            };
            result.push_str(&format!("  {key}: {value};\n"));
        }
    }
    result
}

pub(super) fn write_rule(sheet: &StyleSheet, id: &StyleId, character: bool) -> Option<String> {
    write_rule_for_selector(sheet, id, character, &selector_v2(sheet, id, character))
}

// Native class assignments imported from v1 keep their selectors. A class has
// no native HTML defaults: the same v2 grammar emits its full necessary CSS.
fn write_rule_for_selector(
    sheet: &StyleSheet,
    id: &StyleId,
    character: bool,
    selector: &str,
) -> Option<String> {
    let native = native_definition(selector);
    if !character && StyleSheet::builtin_block(id) && sheet.block_style(id).is_none() {
        let identity = if native.is_none() {
            format!(
                "  --viem-style-id: {};\n  --viem-style-role: \"paragraph\";\n",
                quote(&id.0)
            )
        } else {
            String::new()
        };
        let (c, b) = effective_style_properties(sheet, &sheet.base_paragraph, false)?;
        let fallback = style_css(&c, &b);
        return Some(format!("{selector} {{\n{identity}  --viem-style-deleted: \"true\";\n  font: inherit;\n  margin: 0;\n{fallback}}}\n"));
    }
    let (name, parent, next, c, b) = if character {
        let style = sheet.character_style(id)?;
        (
            sheet.character_style_metadata(id)?.display_name.clone(),
            style.based_on.clone(),
            None,
            style.properties.clone(),
            BlockProperties::default(),
        )
    } else {
        let style = sheet.block_style(id)?;
        if style.role == BlockRole::Document && id != &sheet.base_document {
            return None;
        }
        (
            sheet.block_style_metadata(id)?.display_name.clone(),
            style.based_on.clone(),
            style.next_paragraph_style.clone(),
            style.character.clone(),
            style.block.clone(),
        )
    };
    let mut metadata = String::new();
    if let Some((baseline, _, _)) = &native {
        let (base_name, base_parent, base_next) = if character {
            let style = baseline.character_style(id)?;
            (
                &baseline.character_style_metadata(id)?.display_name,
                &style.based_on,
                &None,
            )
        } else {
            let style = baseline.block_style(id)?;
            (
                &baseline.block_style_metadata(id)?.display_name,
                &style.based_on,
                &style.next_paragraph_style,
            )
        };
        for (key, value, changed) in [
            ("style-name", name.as_str(), name != *base_name),
            (
                "based-on",
                parent.as_ref().map_or("", |v| v.0.as_str()),
                parent != *base_parent,
            ),
            (
                "next-style",
                next.as_ref().map_or("", |v| v.0.as_str()),
                next != *base_next,
            ),
        ] {
            if changed {
                metadata.push_str(&format!("  --viem-{key}: {};\n", quote(value)));
            }
        }
    } else {
        for (key, value) in [
            ("style-id", id.0.as_str()),
            ("style-name", name.as_str()),
            (
                "style-role",
                if character { "character" } else { "paragraph" },
            ),
        ] {
            metadata.push_str(&format!("  --viem-{key}: {};\n", quote(value)));
        }
        if let Some(parent) = &parent {
            metadata.push_str(&format!("  --viem-based-on: {};\n", quote(&parent.0)));
        }
        if let Some(next) = &next {
            metadata.push_str(&format!("  --viem-next-style: {};\n", quote(&next.0)));
        }
    }
    let css = minimal_style_css(sheet, id, character, native.is_some())?;
    let provisional = format!("{selector} {{\n{metadata}{css}}}\n");
    // Generate only the residual that CSS cannot round-trip. This deliberately
    // avoids parallel, duplicate copies of ordinary font/spacing declarations.
    let parsed = parse_rule_v2(&provisional)?;
    let (parsed_c, parsed_b) = match parsed {
        StyleDefinitionEdit::InsertBlock { style, .. } => (style.character, style.block),
        StyleDefinitionEdit::InsertCharacter { style, .. } => {
            (style.properties, BlockProperties::default())
        }
        _ => return None,
    };
    let parsed_props = properties(&parsed_c, &parsed_b)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let wanted_props = properties(&c, &b).into_iter().collect::<BTreeMap<_, _>>();
    let clear = parsed_props
        .keys()
        .filter(|key| !wanted_props.contains_key(*key))
        .copied()
        .collect::<Vec<_>>();
    if !clear.is_empty() {
        metadata.push_str(&format!("  --viem-inherit: {};\n", quote(&clear.join(" "))));
    }
    for (key, value) in properties(&c, &b) {
        if parsed_props.get(key) != Some(&value) {
            metadata.push_str(&format!("  --viem-prop-{key}: {};\n", quote(&value)));
        }
    }
    Some(format!("{selector} {{\n{metadata}{css}}}\n"))
}

pub(super) fn definition_patches_with_policy(
    text: &str,
    after: &StyleSheet,
    include_builtin_definitions: bool,
    file_format: FileFormat,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    // Application settings are deliberately absent from the canonical source
    // grammar when native style export is disabled. In particular a saved
    // Paragraph default may be represented by a sparse projection layer; that
    // implementation detail must not change the spelling of custom CSS or make
    // the same rule unreadable on a machine without those settings.
    let canonical;
    let after = if include_builtin_definitions {
        after
    } else {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.mark_html_base_styles_source_backed();
        let definitions = after
            .block_styles()
            .filter(|style| !is_native_style(after, &style.id, false))
            .filter_map(|style| {
                let metadata = after.block_style_metadata(&style.id)?;
                (metadata.origin == StyleDefinitionOrigin::SourceBacked).then_some(
                    StyleDefinitionEdit::InsertBlock {
                        style: style.clone(),
                        metadata: metadata.clone(),
                    },
                )
            })
            .chain(
                after
                    .character_styles()
                    .filter(|style| !is_native_style(after, &style.id, true))
                    .filter_map(|style| {
                        let metadata = after.character_style_metadata(&style.id)?;
                        (metadata.origin == StyleDefinitionOrigin::SourceBacked).then_some(
                            StyleDefinitionEdit::InsertCharacter {
                                style: style.clone(),
                                metadata: metadata.clone(),
                            },
                        )
                    }),
            )
            .collect::<Vec<_>>();
        sheet
            .install_source_definitions(&definitions)
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
        canonical = sheet;
        &canonical
    };
    let owned = read(text);
    let mut patches = Vec::new();
    let mut append = String::new();
    let mut old = BTreeSet::new();
    for rule in &owned.rules {
        let key = (
            rule.definition.is_block(),
            rule.definition.style_id().clone(),
        );
        let selector = rule.selector(text);
        old.insert((key.0, key.1.clone(), selector.to_owned()));
        if !include_builtin_definitions && is_native_style(after, &key.1, !key.0) {
            patches.push((rule.range.clone(), String::new()));
            continue;
        }
        let mut next = write_rule_for_selector(after, &key.1, !key.0, selector).unwrap_or_default();
        if next.ends_with(" {\n}\n") {
            next.clear();
        }
        if rule.version == 1 || next != text[rule.range.clone()] {
            patches.push((rule.range.clone(), next));
        }
    }
    for (character, id) in after
        .block_styles()
        .map(|s| (false, &s.id))
        .chain(after.character_styles().map(|s| (true, &s.id)))
    {
        let metadata = if character {
            after.character_style_metadata(id)
        } else {
            after.block_style_metadata(id)
        };
        let native = is_native_style(after, id, character);
        if !include_builtin_definitions && native {
            continue;
        }
        if character
            && id == &after.base_character
            && after.character_style(id).is_some_and(|style| {
                style.based_on.is_none() && style.properties == CharacterProperties::default()
            })
            && after
                .character_style_metadata(id)
                .is_some_and(|m| m.display_name == "Base Character")
        {
            continue;
        }
        if (include_builtin_definitions && native
            || metadata.is_some_and(|m| m.origin == StyleDefinitionOrigin::SourceBacked))
            && !old.contains(&(!character, id.clone(), selector_v2(after, id, character)))
        {
            let rule =
                write_rule(after, id, character).ok_or(DocumentError::UnsupportedFormatting)?;
            if !rule.ends_with(" {\n}\n") {
                append.push_str(&rule);
            }
        }
    }
    for id in after.deleted_source_blocks() {
        if !include_builtin_definitions && is_native_style(after, id, false) {
            continue;
        }
        if !old.contains(&(true, id.clone(), selector_v2(after, id, false))) {
            append.push_str(
                &write_rule(after, id, false).ok_or(DocumentError::UnsupportedFormatting)?,
            );
        }
    }
    let mut new_markup = String::new();
    if !append.is_empty() {
        if let Some(at) = owned.close {
            patches.push((at..at, append));
        } else {
            new_markup = format!("{OPEN_V2}\n{append}</style>");
        }
    }
    let has_native_output = after
        .block_styles()
        .map(|style| (false, &style.id))
        .chain(after.character_styles().map(|style| (true, &style.id)))
        .any(|(character, id)| {
            native_style_selector(after, id, character).is_some()
                && write_rule(after, id, character).is_some_and(|rule| !rule.ends_with(" {\n}\n"))
        })
        || after.deleted_source_blocks().next().is_some();
    if include_builtin_definitions && !has_native_output && !owned.empty_v2_marker {
        new_markup.push_str(&format!("{OPEN_V2}</style>"));
    }
    let semantic_tokens = super::html5_tree::tokens(text);
    let active = active_source_elements(&semantic_tokens);
    if !new_markup.is_empty() {
        let markup = new_markup;
        let tokens = &active;
        let head = tokens
            .iter()
            .find(|t| matches!(&t.kind,TokenKind::Tag(tag)if!tag.end&&tag.name=="head"));
        if let Some(head) = head {
            let mut at = head.range.end;
            for token in tokens.iter().filter(|t| t.range.start >= head.range.end) {
                if matches!(&token.kind,TokenKind::Tag(tag)if tag.end&&tag.name=="head") {
                    break;
                }
                if matches!(&token.kind,TokenKind::Tag(tag)if!tag.end&&tag.name=="meta"&&(tag.attribute("charset").is_some()||tag.attribute("http-equiv").is_some()))
                {
                    at = token.range.end;
                }
            }
            patches.push((at..at, markup));
        } else if let Some(html) = tokens
            .iter()
            .find(|t| matches!(&t.kind,TokenKind::Tag(tag)if!tag.end&&tag.name=="html"))
        {
            let at = html.range.end;
            patches.push((at..at, format!("<head>{markup}</head>")));
        } else {
            patches.push((0..0, markup));
        }
    }
    // Removing all owned definitions also removes the now-empty owned element.
    // Any unknown declaration, rule, or comment keeps its original wrapper.
    let tokens = html::tokenize(text);
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(&text[token.range.clone()], OPEN | OPEN_V2) {
            continue;
        }
        if !active.iter().any(|item| {
            item.range.start == token.range.start
                && matches!(&item.kind, TokenKind::Tag(tag) if !tag.end && tag.name == "style")
        }) {
            continue;
        }
        let Some(end) = tokens[index + 1..].iter().find(
            |token| matches!(&token.kind, TokenKind::Tag(tag) if tag.end && tag.name == "style"),
        ) else {
            continue;
        };
        let body = token.range.end..end.range.start;
        if &text[token.range.clone()] == OPEN {
            migrate_legacy_sheet(text, token.range.clone(), end.range.clone(), &mut patches);
        }
        let mut changes = patches
            .iter()
            .filter(|(range, _)| body.start <= range.start && range.end <= body.end)
            .collect::<Vec<_>>();
        let remove_empty_marker = !include_builtin_definitions
            && &text[token.range.clone()] == OPEN_V2
            && text[body.clone()].trim().is_empty();
        if changes.is_empty() && !remove_empty_marker {
            continue;
        }
        changes.sort_by_key(|(range, _)| (range.start, range.end));
        let mut remaining = text[body.clone()].to_owned();
        for (range, replacement) in changes.into_iter().rev() {
            remaining.replace_range(
                range.start - body.start..range.end - body.start,
                replacement,
            );
        }
        if remaining.trim().is_empty() {
            patches.retain(|(range, _)| !(body.start <= range.start && range.end <= body.end));
            patches.push((token.range.start..end.range.end, String::new()));
        }
    }
    // Generated CSS follows the document's physical line endings. Mixing LF
    // into a DOS file can change detection on reopen and invalidate old rules.
    for (_, replacement) in &mut patches {
        *replacement = replacement.replace('\n', file_format.spelling());
    }
    Ok(patches)
}

/// Change only adopted rules in their original cascade positions. An entirely
/// owned sheet needs one version-token edit. Opaque gaps retain v1 wrappers;
/// adjacent authored runs get v2 wrappers without moving across those gaps.
fn migrate_legacy_sheet(
    text: &str,
    opening: Range<usize>,
    closing: Range<usize>,
    patches: &mut Vec<(Range<usize>, String)>,
) {
    let body = opening.end..closing.start;
    let (mut changes, other): (Vec<_>, Vec<_>) = std::mem::take(patches)
        .into_iter()
        .partition(|(range, _)| body.start <= range.start && range.end <= body.end);
    *patches = other;
    changes.sort_by_key(|(range, _)| (range.start, range.end));
    let mut runs: Vec<(Range<usize>, String)> = Vec::new();
    for (range, replacement) in changes {
        if let Some((previous, output)) = runs
            .last_mut()
            .filter(|(previous, _)| text[previous.end..range.start].trim().is_empty())
        {
            output.push_str(&text[previous.end..range.start]);
            output.push_str(&replacement);
            previous.end = range.end;
        } else {
            runs.push((range, replacement));
        }
    }
    if runs.len() == 1
        && text[body.start..runs[0].0.start].trim().is_empty()
        && text[runs[0].0.end..body.end].trim().is_empty()
    {
        if !runs[0].1.trim().is_empty() {
            patches.push((opening, OPEN_V2.into()));
        }
        patches.extend(runs);
        return;
    }
    for (mut range, replacement) in runs {
        if replacement.trim().is_empty() {
            patches.push((range, replacement));
            continue;
        }
        let mut output = if text[body.start..range.start].trim().is_empty() {
            let prefix = format!("{OPEN_V2}{}", &text[body.start..range.start]);
            range.start = opening.start;
            prefix
        } else {
            format!("</style>{OPEN_V2}\n")
        };
        output.push_str(&replacement);
        if text[range.end..body.end].trim().is_empty() {
            output.push_str(&text[range.end..body.end]);
            output.push_str(&text[closing.clone()]);
            range.end = closing.end;
        } else {
            output.push_str(&format!("</style>{OPEN}"));
        }
        patches.push((range, output));
    }
}

fn html_attribute(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_preserves_the_frozen_style_graph() {
        let mut sheet = StyleSheet::default();
        sheet.mark_html_base_styles_source_backed();
        let style = BlockStyle {
            id: "First".into(),
            based_on: Some("Paragraph".into()),
            next_paragraph_style: Some("Paragraph".into()),
            role: BlockRole::Paragraph,
            character: CharacterProperties {
                size: Some(18.0),
                ..Default::default()
            },
            block: Default::default(),
        };
        let metadata = StyleDefinitionMetadata {
            display_name: "First".into(),
            origin: StyleDefinitionOrigin::SourceBacked,
        };
        sheet.insert_block_style(style, metadata).unwrap();
        // Freeze the complete legacy import contract independently of the
        // current export policy and v2 writer.
        let rules = sheet
            .block_styles()
            .filter(|style| {
                sheet
                    .block_style_metadata(&style.id)
                    .is_some_and(|m| m.origin == StyleDefinitionOrigin::SourceBacked)
            })
            .map(|style| canonical_v1_spelling(&sheet, &style.id, false).unwrap())
            .collect::<String>();
        let text = format!("{OPEN}\n{rules}</style>");
        let parsed = read(&text);
        for style in sheet.block_styles() {
            assert_eq!(
                parsed.sheet.block_style(&style.id),
                Some(style),
                "source={text}"
            );
            assert_eq!(
                parsed.sheet.block_style_metadata(&style.id),
                sheet.block_style_metadata(&style.id),
                "source={text}"
            );
        }
    }

    #[test]
    fn migration_preserves_unchanged_custom_styles_and_legacy_native_definitions() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.mark_html_export_definitions_source_backed();
        sheet
            .insert_block_style(
                BlockStyle {
                    id: "First".into(),
                    based_on: Some("Heading1".into()),
                    next_paragraph_style: Some("Paragraph".into()),
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(18.0),
                        ..Default::default()
                    },
                    block: Default::default(),
                },
                StyleDefinitionMetadata {
                    display_name: "First".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            )
            .unwrap();
        let rules = sheet
            .block_styles()
            .map(|style| canonical_v1_spelling(&sheet, &style.id, false).unwrap())
            .collect::<String>();
        let body = "<h1>Head</h1><p class='viem-p-4669727374'>Words</p><!--keep-->";
        let source = format!("{OPEN}\n{rules}</style>{body}");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        assert!(document.include_style_definitions_in_file());
        let before = document.projection().style_sheet().clone();
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                    origin: StyleDefinitionOrigin::SourceBacked,
                    edit: StyleDefinitionEdit::InsertCharacter {
                        style: CharacterStyle {
                            id: "Added".into(),
                            based_on: Some("Character".into()),
                            properties: CharacterProperties {
                                slant: Some(FontSlant::Italic),
                                ..Default::default()
                            },
                        },
                        metadata: StyleDefinitionMetadata {
                            display_name: "Added".into(),
                            origin: StyleDefinitionOrigin::SourceBacked,
                        },
                    },
                }),
            ))
            .unwrap();
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(!saved.contains(OPEN), "{saved}");
        assert!(saved.ends_with(body));
        let reopened = Document::from_bytes(saved.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), "Head\nWords");
        assert_eq!(
            reopened.projection().blocks()[1].style,
            StyleId::from("First")
        );
        for style in before.block_styles() {
            assert_eq!(
                reopened.projection().style_sheet().block_style(&style.id),
                Some(style)
            );
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn migrated_native_classes_and_new_native_assignments_share_one_v2_definition() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.ensure_list_level(1);
        sheet.mark_html_export_definitions_source_backed();
        let rules = sheet
            .block_styles()
            .map(|style| canonical_v1_spelling(&sheet, &style.id, false).unwrap())
            .chain(
                sheet
                    .character_styles()
                    .map(|style| canonical_v1_spelling(&sheet, &style.id, true).unwrap()),
            )
            .collect::<String>();
        let code_class = class_name(&"Code Block".into(), false);
        let quote_class = class_name(&"Block quote".into(), false);
        let list_class = class_name(&"List1".into(), false);
        let inline_class = class_name(&"Code".into(), true);
        let body = format!("<p class='{code_class} keep' data-keep='yes'>Old  words</p><p class='{quote_class}'>Quote</p><div class='{list_class}'>List</div><p><span class='{inline_class}'>Inline</span></p><p>New</p>");
        let source = format!("{OPEN}\n{rules}</style>{body}");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let before_text = document.text().to_owned();
        let before_blocks = document
            .projection()
            .blocks()
            .iter()
            .map(|block| (block.kind.clone(), block.style.clone()))
            .collect::<Vec<_>>();
        for size in [20.0, 24.0] {
            let mut style = document
                .projection()
                .style_sheet()
                .block_style(&"Code Block".into())
                .unwrap()
                .clone();
            style.character.size = Some(size);
            document
                .apply_style_request(StyleModelRequest::new(
                    document.id(),
                    document.revision(),
                    StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                        origin: StyleDefinitionOrigin::SourceBacked,
                        edit: StyleDefinitionEdit::UpdateBlock(style),
                    }),
                ))
                .unwrap();
            let saved = String::from_utf8(document.source_bytes()).unwrap();
            assert!(!saved.contains(OPEN), "{saved}");
            assert!(saved.ends_with(&body));
            assert_eq!(document.text(), before_text);
            assert_eq!(
                document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (block.kind.clone(), block.style.clone()))
                    .collect::<Vec<_>>(),
                before_blocks
            );
            let parsed = read(&saved);
            let css = |selector: &str| {
                let rule = parsed
                    .rules
                    .iter()
                    .find(|rule| rule.selector(&saved) == selector)
                    .unwrap();
                &saved[rule.range.clone()]
            };
            assert!(css(&format!(".{code_class}")).contains("font-family: monospace;"));
            assert!(css(&format!(".{code_class}")).contains(&format!("font-size: {size}pt;")));
            assert!(css("pre").contains(&format!("font-size: {size}pt;")));
            assert!(css(&format!(".{inline_class}")).contains("font-family: monospace;"));
            assert!(css(&format!(".{quote_class}")).contains("margin-inline-start: 32pt;"));
            assert!(css(&format!(".{list_class}")).contains("margin-inline-start: 32pt;"));
            let reopened =
                Document::from_bytes(saved.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(reopened.text(), before_text);
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style(&"Code Block".into())
                    .unwrap()
                    .character
                    .size,
                Some(size)
            );
        }
        let range = TextRange::new(
            document.text_point(before_text.len() - 3).unwrap(),
            document.text_point(before_text.len()).unwrap(),
        )
        .unwrap();
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::AssignBlockStyle {
                    target: StyleBlockTarget::Paragraphs(range),
                    style: "Code Block".into(),
                }),
            ))
            .unwrap();
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(saved.ends_with("<pre>New</pre>"), "{saved}");
        assert_eq!(document.text(), before_text);
        assert_eq!(
            document.projection().blocks().last().unwrap().style,
            StyleId::from("Code Block")
        );
        assert!(document.undo());
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn deleted_native_rules_migrate_and_follow_full_paragraph_fallback() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.mark_html_base_styles_source_backed();
        let mut body = sheet.block_style(&sheet.base_document).unwrap().clone();
        body.character.letter_spacing = Some(2.0);
        let mut paragraph = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        paragraph.character.size = Some(20.0);
        paragraph.character.letter_spacing = Some(0.0);
        paragraph.block.spacing_before = Some(12.0);
        paragraph.block.spacing_after = Some(8.0);
        for edit in [
            StyleDefinitionEdit::UpdateBlock(body),
            StyleDefinitionEdit::UpdateBlock(paragraph.clone()),
            StyleDefinitionEdit::DeleteBlock("Heading1".into()),
        ] {
            sheet
                .apply_source_edit(&edit, StyleSheetRevision(sheet.revision.0 + 1), false)
                .unwrap();
        }
        let old_v2 = "h1 {\n  --viem-style-deleted: \"true\";\n  font: inherit;\n  margin: 0;\n}\n";
        let content = "<h1 data-keep='yes'>Head</h1><p>Tail</p><!--keep-->";
        for (opening, version_one) in [(OPEN, true), (OPEN_V2, false)] {
            let writer = if version_one {
                canonical_v1_spelling
            } else {
                write_rule
            };
            let rules = [&sheet.base_document, &sheet.base_paragraph]
                .into_iter()
                .map(|id| writer(&sheet, id, false).unwrap())
                .collect::<String>();
            let deletion = if version_one {
                canonical_v1_spelling(&sheet, &"Heading1".into(), false).unwrap()
            } else {
                old_v2.to_owned()
            };
            let source = format!("<style>h1 {{ text-align: center; margin-inline-start: 16pt }}</style>{opening}\n{rules}{deletion}</style>{content}");
            for format in [Format::Html, Format::HtmlSource] {
                let mut document =
                    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document
                    .projection()
                    .style_sheet()
                    .block_style(&"Heading1".into())
                    .is_none());
                let apply = |document: &mut Document, edit| {
                    document
                        .apply_style_request(StyleModelRequest::new(
                            document.id(),
                            document.revision(),
                            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                                origin: StyleDefinitionOrigin::SourceBacked,
                                edit,
                            }),
                        ))
                        .unwrap()
                };
                let unchanged = apply(
                    &mut document,
                    StyleDefinitionEdit::UpdateBlock(paragraph.clone()),
                );
                assert_eq!(unchanged.summary().kind(), ModelChangeKind::NoOp);
                assert_eq!(document.source_bytes(), source.as_bytes());
                let mut changed = paragraph.clone();
                changed.character.size = Some(22.0);
                for (size, edit) in [
                    (
                        20.0,
                        StyleDefinitionEdit::UpdateMetadata {
                            namespace: StyleNamespace::Block,
                            id: "Paragraph".into(),
                            metadata: StyleDefinitionMetadata {
                                display_name: "Body copy".into(),
                                origin: StyleDefinitionOrigin::SourceBacked,
                            },
                        },
                    ),
                    (22.0, StyleDefinitionEdit::UpdateBlock(changed)),
                ] {
                    apply(&mut document, edit);
                    let saved = String::from_utf8(document.source_bytes()).unwrap();
                    assert!(saved.ends_with(content));
                    assert!(!saved.contains(OPEN));
                    let parsed = read(&saved);
                    let rule = parsed.rules.iter().find(|rule| matches!(&rule.definition, StyleDefinitionEdit::DeleteBlock(id) if id.0 == "Heading1")).unwrap();
                    let css = &saved[rule.range.clone()];
                    for declaration in [
                        format!("font-size: {size}pt;"),
                        "margin-block-start: 12pt;".into(),
                        "margin-block-end: 8pt;".into(),
                        "text-align: start;".into(),
                        "margin-inline-start: 0pt;".into(),
                        "letter-spacing: 0pt;".into(),
                    ] {
                        assert!(css.contains(&declaration), "{css}");
                    }
                    assert!(!css.contains("padding-"));
                    let reopened =
                        Document::from_bytes(saved.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
                    assert_eq!(reopened.text(), "Head\nTail");
                    let resolved = crate::layout::DocumentLayoutStyles::semantic_character_at(
                        reopened.projection(), 0, false,
                    ).unwrap();
                    assert_eq!(resolved.size, size);
                    assert_eq!(resolved.letter_spacing, 0.0);
                    assert_eq!(
                        reopened.projection().blocks()[0].style,
                        StyleId::from("Paragraph")
                    );
                    assert_eq!(
                        reopened
                            .projection()
                            .style_sheet()
                            .block_style(&"Paragraph".into())
                            .unwrap()
                            .character
                            .size,
                        Some(size)
                    );
                }
                assert!(document.undo());
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
            }
        }
    }

    #[test]
    fn old_v2_deletion_import_rejects_unknown_declarations_and_spelling() {
        let old = "h1 {\n  --viem-style-deleted: \"true\";\n  font: inherit;\n  margin: 0;\n}\n";
        for rule in [
            old.replace("  margin: 0;", "  margin: 0;\n  color: red;"),
            old.replace("font: inherit", "font:  inherit"),
            old.replace("h1 {", "h01 {"),
        ] {
            let source = format!("{OPEN_V2}\n{rule}</style><h1>Head</h1>");
            let parsed = read(&source);
            assert!(parsed.rules.is_empty());
            assert!(parsed.sheet.block_style(&"Heading1".into()).is_some());
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert_eq!(
                document.projection().blocks()[0].style,
                StyleId::from("Heading1")
            );
        }
    }

    #[test]
    fn version_one_character_rule_has_a_fixed_golden_spelling() {
        let mut sheet = StyleSheet::default();
        sheet
            .insert_character_style(
                CharacterStyle {
                    id: "A".into(),
                    based_on: Some("Character".into()),
                    properties: CharacterProperties {
                        weight: Some(400),
                        underline: Some(false),
                        letter_spacing: Some(0.0),
                        ..Default::default()
                    },
                },
                StyleDefinitionMetadata {
                    display_name: "A \"name\"".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            )
            .unwrap();
        assert_eq!(canonical_v1_spelling(&sheet,&StyleId::from("A"),true).unwrap(),
            ".viem-c-41 {\n  --viem-style-id: \"A\";\n  --viem-style-name: \"A \\22 name\\22 \";\n  --viem-style-role: \"character\";\n  --viem-based-on: \"Character\";\n  --viem-prop-character-weight: \"400\";\n  --viem-prop-character-underline: \"false\";\n  --viem-prop-character-letter-spacing: \"0\";\n  font-weight: 400;\n  text-decoration-line: none;\n  letter-spacing: 0pt;\n}\n");
    }

    #[test]
    fn version_two_native_rules_have_simple_css_and_round_trip_exact_definitions() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.ensure_list_level(3);
        assert_eq!(
            write_rule(&sheet, &sheet.base_document, false).unwrap(),
            "body {\n  font-family: 'SF Pro';\n  font-size: 14pt;\n}\n"
        );
        assert_eq!(
            write_rule(&sheet, &"List1".into(), false).unwrap(),
            "li {\n}\n"
        );
        let rules = sheet
            .block_styles()
            .map(|s| write_rule(&sheet, &s.id, false).unwrap())
            .chain(
                sheet
                    .character_styles()
                    .map(|s| write_rule(&sheet, &s.id, true).unwrap()),
            )
            .collect::<String>();
        assert!(!rules.contains("text-indent:"), "{rules}");
        assert!(!rules.contains("letter-spacing:"), "{rules}");
        assert!(!rules.contains("vertical-align:"), "{rules}");
        assert!(!rules.contains("--viem-prop-"), "{rules}");
        let parsed = read(&format!("{OPEN_V2}\n{rules}</style>"));
        for style in sheet.block_styles() {
            assert_eq!(parsed.sheet.block_style(&style.id), Some(style), "{rules}");
        }
        for style in sheet.character_styles() {
            assert_eq!(
                parsed.sheet.character_style(&style.id),
                Some(style),
                "{rules}"
            );
        }
        assert_eq!(
            parsed.rules.len(),
            sheet.block_style_count() + sheet.character_style_count()
        );
    }

    #[test]
    fn version_two_css_is_authoritative_and_metadata_only_restores_sparse_residuals() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.ensure_list_level(3);
        let mut style = sheet.block_style(&"List1".into()).unwrap().clone();
        style.block.leading_indent = Some(48.0);
        style.character.letter_spacing = Some(1.25);
        style.character.size = Some(18.0);
        sheet
            .install_source_definitions(&[StyleDefinitionEdit::InsertBlock {
                style: style.clone(),
                metadata: StyleDefinitionMetadata {
                    display_name: "List Level 1".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            }])
            .unwrap();
        let rule = write_rule(&sheet, &style.id, false).unwrap();
        assert!(rule.contains("margin-inline-start: 16pt;"), "{rule}");
        assert!(rule.contains("font-size: 18pt;"), "{rule}");
        assert!(rule.contains("letter-spacing: 1.25pt;"), "{rule}");
        assert!(!rule.contains("--viem-prop-"), "{rule}");
        let parsed = read(&format!("{OPEN_V2}\n{rule}</style>"));
        assert_eq!(parsed.sheet.block_style(&style.id), Some(&style));

        let mut paragraph = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        paragraph.block.spacing_before = None;
        paragraph.character.bold = Some(true);
        paragraph.block.line_spacing = Some(LineSpacing::AtLeast(19.0));
        sheet
            .install_source_definitions(&[StyleDefinitionEdit::InsertBlock {
                style: paragraph.clone(),
                metadata: StyleDefinitionMetadata {
                    display_name: "Base Paragraph".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            }])
            .unwrap();
        let rule = write_rule(&sheet, &paragraph.id, false).unwrap();
        let parsed = read(&format!("{OPEN_V2}\n{rule}</style>"));
        assert_eq!(
            parsed.sheet.block_style(&paragraph.id),
            Some(&paragraph),
            "{rule}"
        );
        assert!(rule.contains("--viem-inherit:"), "{rule}");
        assert!(
            rule.contains("--viem-prop-paragraph-line-spacing:"),
            "{rule}"
        );
        assert!(!rule.contains("--viem-prop-character-weight:"), "{rule}");
    }

    #[test]
    fn deeper_list_css_resets_only_properties_changed_by_the_shallower_selector() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        sheet.ensure_list_level(3);
        let mut first = sheet.block_style(&"List1".into()).unwrap().clone();
        first.character.size = Some(18.0);
        first.block.leading_indent = Some(48.0);
        first.block.first_line_indent = Some(3.0);
        sheet
            .install_source_definitions(&[StyleDefinitionEdit::InsertBlock {
                style: first,
                metadata: StyleDefinitionMetadata {
                    display_name: "List Level 1".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            }])
            .unwrap();
        let first = write_rule(&sheet, &"List1".into(), false).unwrap();
        let second = write_rule(&sheet, &"List2".into(), false).unwrap();
        assert!(second.contains("font-size: 14pt;"), "{second}");
        assert!(second.contains("margin-inline-start: 0pt;"), "{second}");
        assert!(second.contains("text-indent: 0pt;"), "{second}");
        assert!(!second.contains("letter-spacing:"), "{second}");
        let third = write_rule(&sheet, &"List3".into(), false).unwrap();
        assert_eq!(third, "li li li {\n}\n");
        let parsed = read(&format!("{OPEN_V2}\n{first}{second}{third}</style>"));
        assert_eq!(parsed.rules.len(), 3);
        assert_eq!(
            parsed.sheet.block_style(&"List2".into()),
            sheet.block_style(&"List2".into())
        );
    }

    fn patched(text: &str, mut patches: Vec<(Range<usize>, String)>) -> String {
        patches.sort_by_key(|(range, _)| (range.start, range.end));
        let mut result = text.to_owned();
        for (range, replacement) in patches.into_iter().rev() {
            result.replace_range(range, &replacement);
        }
        result
    }

    #[test]
    fn export_policy_preserves_custom_opaque_and_legacy_bytes() {
        let mut sheet = StyleSheet::for_format(Format::Html);
        let metadata = StyleDefinitionMetadata {
            display_name: "Callout".into(),
            origin: StyleDefinitionOrigin::SourceBacked,
        };
        sheet
            .insert_block_style(
                BlockStyle {
                    id: "Callout".into(),
                    based_on: Some(sheet.base_paragraph.clone()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(18.0),
                        ..Default::default()
                    },
                    block: Default::default(),
                },
                metadata,
            )
            .unwrap();
        let original = "<!--keep--><p>Words</p><style>.unknown { color: red }</style>";
        let source = patched(
            original,
            definition_patches_with_policy(original, &sheet, false, FileFormat::Unix).unwrap(),
        );
        assert!(source.ends_with(original), "{source}");
        assert!(!source.contains("body {"), "{source}");
        assert!(!source.contains("p {"), "{source}");
        assert!(source.contains(".viem-p-43616c6c6f7574 {"), "{source}");
        let enabled = patched(
            &source,
            definition_patches_with_policy(&source, &sheet, true, FileFormat::Unix).unwrap(),
        );
        assert!(read(&enabled).has_native_rules());
        let disabled = patched(
            &enabled,
            definition_patches_with_policy(&enabled, &sheet, false, FileFormat::Unix).unwrap(),
        );
        assert_eq!(disabled, source);

        let legacy = format!(
            "{OPEN}\n{}</style>{original}",
            canonical_v1_spelling(&sheet, &sheet.base_paragraph, false).unwrap()
        );
        let migrated = patched(
            &legacy,
            definition_patches_with_policy(&legacy, &sheet, true, FileFormat::Unix).unwrap(),
        );
        assert!(!migrated.contains(OPEN), "{migrated}");
        assert!(migrated.contains(OPEN_V2), "{migrated}");
        assert!(migrated.ends_with(original));
        let removed = patched(
            &legacy,
            definition_patches_with_policy(&legacy, &sheet, false, FileFormat::Unix).unwrap(),
        );
        assert!(!removed.contains("data-viem-version=\"1\""), "{removed}");
        assert!(removed.ends_with(original));
    }
}

/// Replace only the first effective class attribute's value, retaining tag
/// case, other attributes, quote spelling and duplicate attributes untouched.
pub(super) fn class_patch(text: &str, tag_range: Range<usize>, value: &str) -> (Range<usize>, String) {
    attribute_patch(text, tag_range, "class", value)
}

fn attribute_patch(text: &str, tag_range: Range<usize>, name: &str, value: &str) -> (Range<usize>, String) {
    let tag = &text[tag_range.clone()];
    let bytes = tag.as_bytes();
    let mut at = 1;
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>' {
        at += 1;
    }
    while at < bytes.len() && bytes[at] != b'>' {
        let attribute_start = at;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while at < bytes.len()
            && !bytes[at].is_ascii_whitespace()
            && !matches!(bytes[at], b'=' | b'>')
        {
            at += 1;
        }
        let key = &tag[start..at];
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes.get(at) != Some(&b'=') {
            if key.eq_ignore_ascii_case(name) {
                if value.is_empty() {
                    return (
                        tag_range.start + attribute_start..tag_range.start + at,
                        String::new(),
                    );
                }
                return (
                    tag_range.start + start..tag_range.start + at,
                    format!("{name}={}", html_attribute(value)),
                );
            }
            if bytes.get(at) == Some(&b'>') {
                break;
            }
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let value_start = at;
        let quote = bytes.get(at).copied().filter(|c| matches!(c, b'"' | b'\''));
        if quote.is_some() {
            at += 1;
        }
        while at < bytes.len()
            && match quote {
                Some(q) => bytes[at] != q,
                None => !bytes[at].is_ascii_whitespace() && bytes[at] != b'>',
            }
        {
            at += 1;
        }
        if quote.is_some() && at < bytes.len() {
            at += 1;
        }
        if key.eq_ignore_ascii_case(name) {
            if value.is_empty() {
                return (
                    tag_range.start + attribute_start..tag_range.start + at,
                    String::new(),
                );
            }
            return (
                tag_range.start + value_start..tag_range.start + at,
                html_attribute(value),
            );
        }
    }
    let at = tag_range.end - 1;
    (at..at, format!(" {name}={}", html_attribute(value)))
}

/// Built-in list styles are structural HTML. Existing ordered lists keep their
/// numbering; a paragraph which was not in a list starts a bullet list.
fn list_assignment_patches(
    input: &super::line_endings::NormalizedText,
    ranges: &[Range<usize>],
    sheet: &StyleSheet,
    level: u16,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let mut targets = Vec::new();
    let mut patches = Vec::new();
    let mut selected = BTreeSet::new();
    for source in ranges {
        let at = tokens
            .iter()
            .find(|token| converter.source_range(token.range.clone()).end > source.start)
            .map(|token| token.range.start)
            .unwrap_or(input.text.len());
        let stack = super::html_paragraph::stack_at(&tokens, at);
        let lists = stack.iter().filter(|token| matches!(&token.kind, TokenKind::Tag(tag) if matches!(tag.name.as_str(), "ul" | "ol"))).collect::<Vec<_>>();
        if let Some(item) = stack
            .iter()
            .rev()
            .find(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "li"))
        {
            if !selected.insert(item.range.start) {
                continue;
            }
            let TokenKind::Tag(tag) = &item.kind else {
                unreachable!()
            };
            let classes = tag
                .attribute("class")
                .unwrap_or("")
                .split_ascii_whitespace()
                .filter(|class| select_class(sheet, class, false).is_none())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if lists.len() != usize::from(level) {
                return Err(DocumentError::UnsupportedFormatting);
            }
            if tag.attribute("class").unwrap_or("") != classes.join(" ") {
                let (range, text) =
                    class_patch(&input.text, item.range.clone(), &classes.join(" "));
                patches.push((converter.source_range(range), text));
            }
        } else {
            targets.push((source.clone(), Some(ListStyle::Bullet), 1, None));
        }
    }
    for (range, mut text) in html::list_patches(input, &targets)? {
        let generated = html::tokenize(&text);
        let mut edits = Vec::new();
        for token in generated {
            if let TokenKind::Tag(tag) = &token.kind {
                if !tag.end && tag.name == "li" {
                    let classes = tag
                        .attribute("class")
                        .unwrap_or("")
                        .split_ascii_whitespace()
                        .filter(|class| select_class(sheet, class, false).is_none())
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    if tag.attribute("class").unwrap_or("") != classes.join(" ") {
                        edits.push(class_patch(&text, token.range.clone(), &classes.join(" ")));
                    }
                }
            }
        }
        for (range, replacement) in edits.into_iter().rev() {
            text.replace_range(range, &replacement);
        }
        if level > 1 {
            if let Some(at) = text.find("<ul>") {
                text.insert_str(at, &"<ul><li>".repeat(usize::from(level - 1)));
            }
            if let Some(at) = text.rfind("</ul>") {
                text.insert_str(
                    at + "</ul>".len(),
                    &"</li></ul>".repeat(usize::from(level - 1)),
                );
            }
        }
        patches.push((range, text));
    }
    Ok(patches)
}

pub(super) fn paragraph_assignment_patches(
    input: &super::line_endings::NormalizedText,
    ranges: &[Range<usize>],
    sheet: &StyleSheet,
    id: &StyleId,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    if id.0 == "Block quote" {
        return super::html_quotes::wrap_patches(input, ranges);
    }
    if id == &sheet.base_paragraph {
        if let Some(patches) = super::html_quotes::remove_patches(input, ranges)? {
            return Ok(patches);
        }
    }
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    if let Some(level) = list_style_level(id) {
        return list_assignment_patches(input, ranges, sheet, level);
    }
    let element = if id == &sheet.base_paragraph {
        Some("p".to_owned())
    } else if id.0 == "Code Block" {
        Some("pre".to_owned())
    } else {
        id.0.strip_prefix("Heading")
            .filter(|level| matches!(*level, "1" | "2" | "3" | "4" | "5" | "6"))
            .map(|level| format!("h{level}"))
    };
    let class = class_name(id, false);
    if let Some(element) = &element {
        let (items, plain): (Vec<_>, Vec<_>) = ranges.iter().cloned().partition(|source| {
            let at = tokens
                .iter()
                .find(|token| converter.source_range(token.range.clone()).end > source.start)
                .map(|token| token.range.start)
                .unwrap_or(input.text.len());
            super::html_paragraph::stack_at(&tokens, at)
                .iter()
                .any(|token| matches!(&token.kind, TokenKind::Tag(tag) if tag.name == "li"))
        });
        if !items.is_empty() {
            let targets = items
                .into_iter()
                .map(|source| (source, None, 1, Some(1)))
                .collect::<Vec<_>>();
            let mut patches = html::list_patches(input, &targets)?;
            for (_, text) in &mut patches {
                let mut edits = Vec::new();
                for token in html::tokenize(text) {
                    if let TokenKind::Tag(tag) = &token.kind {
                        if tag.name == "p" {
                            let name_start = token.range.start + if tag.end { 2 } else { 1 };
                            if element != "p" {
                                edits.push((name_start..name_start + 1, element.clone()));
                            }
                            if !tag.end {
                                let classes = tag
                                    .attribute("class")
                                    .unwrap_or("")
                                    .split_ascii_whitespace()
                                    .filter(|name| select_class(sheet, name, false).is_none())
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                if tag.attribute("class").unwrap_or("") != classes {
                                    edits.push(class_patch(text, token.range.clone(), &classes));
                                }
                            }
                        }
                    }
                }
                edits.sort_by_key(|(range, _)| range.start);
                for (range, replacement) in edits.into_iter().rev() {
                    text.replace_range(range, &replacement);
                }
            }
            patches.extend(paragraph_assignment_patches(input, &plain, sheet, id)?);
            return Ok(patches);
        }
    }
    let mut selected = BTreeSet::new();
    let mut patches = Vec::new();
    for source in ranges {
        let at = tokens
            .iter()
            .find(|token| converter.source_range(token.range.clone()).end > source.start)
            .map(|token| token.range.start)
            .unwrap_or(input.text.len());
        let open = super::html_paragraph::stack_at(&tokens, at);
        let current = open.iter().rev().find(|token| matches!(&token.kind, TokenKind::Tag(tag) if matches!(tag.name.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "pre" | "blockquote"))).copied();
        let Some(token) = current else {
            // Anonymous paragraph text has no authoring element yet. Include
            // its enclosing phrasing wrappers, keeping structural containers,
            // a leading owned stylesheet, and trailing trivia outside the new
            // paragraph whenever their balance permits it.
            let mut open: Vec<&html::Token> = Vec::new();
            for token in &tokens {
                if converter.source_range(token.range.clone()).start >= source.end {
                    break;
                }
                if let TokenKind::Tag(tag) = &token.kind {
                    if tag.end {
                        if let Some(index) = open.iter().rposition(
                            |token| matches!(&token.kind,TokenKind::Tag(t)if t.name==tag.name),
                        ) {
                            open.truncate(index);
                        }
                    } else if !matches!(
                        tag.name.as_str(),
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
                    ) {
                        open.push(token);
                    }
                }
            }
            let structural = |name: &str| {
                matches!(
                    name,
                    "html"
                        | "head"
                        | "body"
                        | "div"
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
            };
            let mut extent = source.clone();
            let mut before: Vec<&html::Token> = Vec::new();
            for token in &tokens {
                let raw = converter.source_range(token.range.clone());
                if raw.start >= source.start {
                    break;
                }
                if let TokenKind::Tag(tag) = &token.kind {
                    if tag.end {
                        if let Some(index) = before.iter().rposition(
                            |token| matches!(&token.kind,TokenKind::Tag(t)if t.name==tag.name),
                        ) {
                            before.truncate(index);
                        }
                    } else if structural(&tag.name) {
                        before.clear();
                    } else if !matches!(
                        tag.name.as_str(),
                        "meta" | "link" | "br" | "hr" | "img" | "input" | "wbr"
                    ) {
                        before.push(token);
                    }
                }
            }
            if let Some(first) = before.first() {
                extent.start = converter.source_range(first.range.clone()).start;
            }
            for token in tokens
                .iter()
                .filter(|token| converter.source_range(token.range.clone()).start >= source.end)
            {
                match &token.kind {
                    TokenKind::Tag(tag) if tag.end && !structural(&tag.name) => {
                        if let Some(index) = open.iter().rposition(
                            |token| matches!(&token.kind,TokenKind::Tag(t)if t.name==tag.name),
                        ) {
                            open.truncate(index);
                            extent.end = converter.source_range(token.range.clone()).end;
                        } else {
                            break;
                        }
                    }
                    TokenKind::Opaque => continue,
                    TokenKind::Text
                        if input.text[token.range.clone()]
                            .bytes()
                            .all(|b| b.is_ascii_whitespace()) =>
                    {
                        continue
                    }
                    _ => break,
                }
            }
            if !selected.insert(extent.start) {
                continue;
            }
            let name = element.as_deref().unwrap_or("p");
            let opening = if element.is_some() {
                format!("<{name}>")
            } else {
                format!("<p class=\"{class}\">")
            };
            if extent.is_empty() {
                patches.push((extent, format!("{opening}</{name}>")));
            } else {
                patches.push((extent.start..extent.start, opening));
                patches.push((extent.end..extent.end, format!("</{name}>")));
            }
            continue;
        };
        if !selected.insert(token.range.start) {
            continue;
        }
        let TokenKind::Tag(tag) = &token.kind else {
            unreachable!()
        };
        let mut classes = tag
            .attribute("class")
            .unwrap_or("")
            .split_ascii_whitespace()
            .filter(|name| select_class(sheet, name, false).is_none())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if let Some(element) = &element {
            if tag.name == "pre" && element != "pre" && {
                let body_end = tokens.iter().find(|next| next.range.start >= token.range.end
                    && matches!(&next.kind, TokenKind::Tag(tag) if tag.end && tag.name == "pre"))
                    .map_or(input.text.len(), |token| token.range.start);
                let body = &input.text[token.range.end..body_end];
                body.contains(['\n', '\r', '\t', '&']) || body.trim() != body
                    || body.contains("  ") || body.contains('<') && body.contains(' ')
            } {
                // The code style is replaced, while its literal spaces and
                // hard breaks remain content through standard CSS whitespace.
                let existing = tag.attribute("style").unwrap_or("");
                let css = if existing.is_empty() { "white-space: pre-wrap".into() }
                    else { format!("{existing};white-space: pre-wrap") };
                let (range, replacement) = attribute_patch(
                    &input.text, token.range.clone(), "style", &css,
                );
                patches.push((converter.source_range(range), replacement));
            }
            if &tag.name != element {
                patches.push((
                    converter.source_range(
                        token.range.start + 1..token.range.start + 1 + tag.name.len(),
                    ),
                    element.clone(),
                ));
                let following = tokens
                    .iter()
                    .filter(|next| next.range.start >= token.range.end);
                let mut boundary = None;
                for next in following {
                    if let TokenKind::Tag(next_tag) = &next.kind {
                        if next_tag.end && next_tag.name == tag.name {
                            patches.push((
                                converter.source_range(
                                    next.range.start + 2
                                        ..next.range.start + 2 + next_tag.name.len(),
                                ),
                                element.clone(),
                            ));
                            boundary = Some(None);
                            break;
                        }
                        if matches!(
                            next_tag.name.as_str(),
                            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "body" | "div" | "blockquote"
                        ) {
                            boundary = Some(Some(next.range.start));
                            break;
                        }
                    }
                }
                if let Some(at) = boundary.unwrap_or(Some(input.text.len())) {
                    patches.push((converter.source_range(at..at), format!("</{element}>")));
                }
            }
            if tag
                .attribute("class")
                .is_some_and(|value| classes.join(" ") != value)
            {
                let (range, replacement) =
                    class_patch(&input.text, token.range.clone(), &classes.join(" "));
                patches.push((converter.source_range(range), replacement));
            }
        } else {
            classes.insert(0, class.clone());
            let (range, replacement) =
                class_patch(&input.text, token.range.clone(), &classes.join(" "));
            patches.push((converter.source_range(range), replacement));
        }
    }
    Ok(patches)
}

pub(super) fn remove_assignment_patches(
    input: &super::line_endings::NormalizedText,
    sheet: &StyleSheet,
    id: &StyleId,
    character: bool,
) -> Vec<(Range<usize>, String)> {
    let class = class_name(id, character);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let tokens = super::html5_tree::tokens(&input.text);
    let mut seen = BTreeSet::new();
    let mut list_kinds = Vec::new();
    active_source_elements(&tokens)
        .into_iter()
        .filter_map(|token| {
            let TokenKind::Tag(tag) = &token.kind else {
                return None;
            };
            if matches!(tag.name.as_str(), "ul" | "ol") {
                if tag.end {
                    list_kinds.pop();
                } else {
                    list_kinds.push(tag.name == "ol");
                }
            }
            if tag.end || token.range.is_empty() || !seen.insert(token.range.start) {
                return None;
            }
            let classes = tag
                .attribute("class")
                .unwrap_or("")
                .split_ascii_whitespace()
                .collect::<Vec<_>>();
            let assigned = select_class(sheet, &classes.join(" "), character);
            let implicit_heading = !character
                && builtin_heading(id).is_some_and(|level| tag.name == format!("h{level}"))
                && assigned.is_none();
            let implicit_list = !character
                && tag.name == "li"
                && assigned.is_none()
                && (id.0 == format!("List{}", list_kinds.len().max(1))
                    || id.list_family_level().is_some_and(|(ordered, level)| {
                        list_kinds.last() == Some(&ordered)
                            && usize::from(level) == list_kinds.len().max(1).min(4)
                    }));
            if assigned.as_ref() != Some(id) && !implicit_heading && !implicit_list {
                return None;
            }
            let mut remaining = classes
                .into_iter()
                .filter(|token| *token != class)
                .collect::<Vec<_>>()
                .join(" ");
            let fallback = class_name(
                &StyleId::from(if character { "Character" } else { "Paragraph" }),
                character,
            );
            if !remaining.is_empty() {
                remaining.insert(0, ' ');
            }
            remaining.insert_str(0, &fallback);
            let (range, replacement) = class_patch(&input.text, token.range.clone(), &remaining);
            Some((converter.source_range(range), replacement))
        })
        .collect()
}
