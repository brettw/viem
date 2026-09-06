//! Version 1 of the deliberately narrow, passive eVim-owned HTML CSS grammar.
//! Every custom value is a CSS string. Stable IDs in class selectors are
//! lowercase hexadecimal UTF-8. Standard declarations are derived output.
use super::html::{self, TokenKind};
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

const OPEN: &str = "<style id=\"evim-styles\" data-evim-version=\"1\">";

#[derive(Clone)]
pub(super) struct Rule {
    pub range: Range<usize>,
    pub definition: StyleDefinitionEdit,
}

pub(super) struct OwnedSheet {
    pub sheet: StyleSheet,
    pub rules: Vec<Rule>,
    pub close: Option<usize>,
}

pub(super) fn class_name(id: &StyleId, character: bool) -> String {
    let encoded =
        id.0.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
    format!("evim-{}-{encoded}", if character { "c" } else { "p" })
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
        if key.starts_with("--evim-") {
            if values.insert(key, unquote(value)?).is_some() {
                return None;
            }
        }
    }
    let id = StyleId(values.remove("--evim-style-id")?);
    let metadata = StyleDefinitionMetadata {
        display_name: values.remove("--evim-style-name")?,
        origin: StyleDefinitionOrigin::SourceBacked,
    };
    let role = values.remove("--evim-style-role")?;
    let parent = values.remove("--evim-based-on").map(StyleId);
    let next = values.remove("--evim-next-style").map(StyleId);
    let mut character = CharacterProperties::default();
    let mut block = BlockProperties::default();
    for (key, value) in values {
        parse_property(
            key.strip_prefix("--evim-prop-")?,
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
    let mut sheet = StyleSheet::default();
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
    for (index, token) in tokens.iter().enumerate() {
        if !eligible.contains(&token.range.start) || &text[token.range.clone()] != OPEN {
            continue;
        }
        let Some(end) = tokens[index + 1..]
            .iter()
            .find(|token| matches!(&token.kind,TokenKind::Tag(tag) if tag.end&&tag.name=="style"))
        else {
            break;
        };
        close = Some(end.range.start);
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
            if let Some(definition) = parse_rule(&text[range.clone()]) {
                rules.push(Rule {
                    range: range.clone(),
                    definition,
                });
            }
            at = range.end;
        }
        break;
    }
    let definitions = rules
        .iter()
        .map(|rule| rule.definition.clone())
        .collect::<Vec<_>>();
    if sheet.install_source_definitions(&definitions).is_err() {
        rules.clear();
        return OwnedSheet {
            sheet: {
                let mut s = StyleSheet::default();
                s.mark_html_base_styles_source_backed();
                s
            },
            rules,
            close,
        };
    }
    // Only the exact v1 grammar is adopted. Unknown declarations/whitespace
    // remain opaque and never become targets for rule replacement.
    rules.retain(|rule| {
        write_rule(
            &sheet,
            rule.definition.style_id(),
            !rule.definition.is_block(),
        )
        .as_deref()
            == Some(&text[rule.range.clone()])
    });
    let definitions = rules
        .iter()
        .map(|rule| rule.definition.clone())
        .collect::<Vec<_>>();
    let mut exact = StyleSheet::default();
    exact.mark_html_base_styles_source_backed();
    if exact.install_source_definitions(&definitions).is_err() {
        rules.clear();
        sheet = StyleSheet::default();
        sheet.mark_html_base_styles_source_backed();
    } else {
        sheet = exact;
    }
    OwnedSheet {
        sheet,
        rules,
        close,
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

fn overlay_character(target: &mut CharacterProperties, layer: &CharacterProperties) {
    macro_rules! copy {($($field:ident),*)=>{$(if layer.$field.is_some(){target.$field=layer.$field.clone();})*};}
    copy!(
        font_families,
        size,
        weight,
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
fn overlay_block(target: &mut BlockProperties, layer: &BlockProperties) {
    macro_rules! copy {($($field:ident),*)=>{$(if layer.$field.is_some(){target.$field=layer.$field.clone();})*};}
    copy!(
        spacing_before,
        spacing_after,
        line_spacing,
        first_line_indent,
        leading_indent,
        trailing_indent,
        padding_top,
        padding_right,
        padding_bottom,
        padding_left,
        background,
        alignment,
        base_direction
    );
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

pub(super) fn write_rule(sheet: &StyleSheet, id: &StyleId, character: bool) -> Option<String> {
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
        out.push_str(&format!("  --evim-{key}: {};\n", quote(value)));
    }
    if let Some(parent) = parent {
        out.push_str(&format!("  --evim-based-on: {};\n", quote(&parent.0)));
    }
    if let Some(next) = next {
        out.push_str(&format!("  --evim-next-style: {};\n", quote(&next.0)));
    }
    for (key, value) in properties(&own_character, &own_block) {
        out.push_str(&format!("  --evim-prop-{key}: {};\n", quote(&value)));
    }
    let (effective_character, mut effective_block) = if character {
        (character_chain(sheet, id), BlockProperties::default())
    } else {
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
        (c, b)
    };
    if role != "document" {
        effective_block.background = None;
        effective_block.padding_top = None;
        effective_block.padding_right = None;
        effective_block.padding_bottom = None;
        effective_block.padding_left = None;
    }
    let css = format!(
        "{}; {}",
        html::character_css(&effective_character),
        block_css(&effective_block)
    );
    for (key, value) in html::declarations(&css) {
        out.push_str(&format!("  {key}: {value};\n"));
    }
    out.push_str("}\n");
    Some(out)
}

pub(super) fn definition_patches(
    text: &str,
    before: &StyleSheet,
    after: &StyleSheet,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let owned = read(text);
    let mut patches = Vec::new();
    let mut append = String::new();
    let mut old = BTreeSet::new();
    for rule in &owned.rules {
        let key = (
            rule.definition.is_block(),
            rule.definition.style_id().clone(),
        );
        old.insert(key.clone());
        let next = write_rule(after, &key.1, !key.0).unwrap_or_default();
        if next != text[rule.range.clone()] {
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
        if metadata.is_some_and(|m| m.origin == StyleDefinitionOrigin::SourceBacked)
            && !old.contains(&(!character, id.clone()))
        {
            let changed = write_rule(before, id, character) != write_rule(after, id, character);
            if changed || owned.close.is_none() {
                append.push_str(
                    &write_rule(after, id, character)
                        .ok_or(DocumentError::UnsupportedFormatting)?,
                );
            }
        }
    }
    if !append.is_empty() {
        if let Some(at) = owned.close {
            patches.push((at..at, append));
        } else {
            let markup = format!("{OPEN}\n{append}</style>");
            let semantic_tokens = super::html5_tree::tokens(text);
            let tokens = active_source_elements(&semantic_tokens);
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
    }
    Ok(patches)
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
    fn canonical_owned_sheet_writer_reader_golden_roundtrip() {
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
        let patches =
            definition_patches("<p>Text</p>", &read("<p>Text</p>").sheet, &sheet).unwrap();
        let text = &patches[0].1;
        let parsed = read(text);
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
        assert_eq!(write_rule(&sheet,&StyleId::from("A"),true).unwrap(),
            ".evim-c-41 {\n  --evim-style-id: \"A\";\n  --evim-style-name: \"A \\22 name\\22 \";\n  --evim-style-role: \"character\";\n  --evim-based-on: \"Character\";\n  --evim-prop-character-weight: \"400\";\n  --evim-prop-character-underline: \"false\";\n  --evim-prop-character-letter-spacing: \"0\";\n  font-weight: 400;\n  text-decoration-line: none;\n  letter-spacing: 0pt;\n}\n");
    }
}

/// Replace only the first effective class attribute's value, retaining tag
/// case, other attributes, quote spelling and duplicate attributes untouched.
fn class_patch(text: &str, tag_range: Range<usize>, value: &str) -> (Range<usize>, String) {
    let tag = &text[tag_range.clone()];
    let bytes = tag.as_bytes();
    let mut at = 1;
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>' {
        at += 1;
    }
    while at < bytes.len() && bytes[at] != b'>' {
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
            if key.eq_ignore_ascii_case("class") {
                return (
                    tag_range.start + start..tag_range.start + at,
                    format!("class={}", html_attribute(value)),
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
        if key.eq_ignore_ascii_case("class") {
            return (
                tag_range.start + value_start..tag_range.start + at,
                html_attribute(value),
            );
        }
    }
    let at = tag_range.end - 1;
    (at..at, format!(" class={}", html_attribute(value)))
}

pub(super) fn paragraph_assignment_patches(
    input: &super::line_endings::NormalizedText,
    ranges: &[Range<usize>],
    sheet: &StyleSheet,
    id: &StyleId,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let tokens = html::tokenize(&input.text);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let element = if id == &sheet.base_paragraph {
        Some("p".to_owned())
    } else {
        id.0.strip_prefix("Heading")
            .filter(|level| matches!(*level, "1" | "2" | "3" | "4" | "5" | "6"))
            .map(|level| format!("h{level}"))
    };
    let class = class_name(id, false);
    let mut selected = BTreeSet::new();
    let mut patches = Vec::new();
    for source in ranges {
        let mut current = None;
        for token in &tokens {
            let raw = converter.source_range(token.range.clone());
            if raw.start > source.start {
                break;
            }
            if let TokenKind::Tag(tag) = &token.kind {
                if matches!(
                    tag.name.as_str(),
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li"
                ) {
                    if tag.end {
                        current = None;
                    } else {
                        current = Some(token);
                    }
                }
            }
        }
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
                            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "body" | "div"
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
    id: &StyleId,
    character: bool,
) -> Vec<(Range<usize>, String)> {
    let class = class_name(id, character);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    html::tokenize(&input.text)
        .into_iter()
        .filter_map(|token| {
            let TokenKind::Tag(tag) = token.kind else {
                return None;
            };
            if tag.end {
                return None;
            }
            let classes = tag
                .attribute("class")?
                .split_ascii_whitespace()
                .collect::<Vec<_>>();
            if !classes.contains(&class.as_str()) {
                return None;
            }
            let remaining = classes
                .into_iter()
                .filter(|token| *token != class)
                .collect::<Vec<_>>()
                .join(" ");
            let (range, replacement) = class_patch(&input.text, token.range, &remaining);
            Some((converter.source_range(range), replacement))
        })
        .collect()
}
