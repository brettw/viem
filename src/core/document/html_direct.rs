//! Targeted direct formatting in original HTML syntax. Only touched element
//! tokens and explicitly required split delimiters are canonicalized.
use super::html::{self, Tag, TokenKind};
use super::line_endings::NormalizedText;
use super::*;
use std::{collections::BTreeSet, ops::Range};

fn attribute(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
    )
}
fn paragraph(name: &str) -> bool {
    matches!(name, "p" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}
fn inline(name: &str) -> bool {
    matches!(
        name,
        "span"
            | "b"
            | "strong"
            | "i"
            | "em"
            | "u"
            | "s"
            | "strike"
            | "del"
            | "a"
            | "font"
            | "small"
            | "big"
            | "code"
            | "sup"
            | "sub"
    )
}
fn clear_character(
    properties: &mut CharacterProperties,
    clear: &BTreeSet<StyleProperty>,
) -> Result<(), DocumentError> {
    for property in clear {
        super::style::clear_character_property(&"Direct".into(), properties, *property)
            .map_err(|_| DocumentError::UnsupportedFormatting)?;
    }
    Ok(())
}
fn tag_with_properties(
    tag: &Tag,
    clear: &BTreeSet<StyleProperty>,
    block: Option<&BlockProperties>,
) -> Result<(String, String), DocumentError> {
    let mut name = tag.name.clone();
    let mut character = CharacterProperties::default();
    let mut paragraph = BlockProperties::default();
    if let Some(css) = tag.attribute("style") {
        html::apply_css(css, &mut character, &mut paragraph);
    }
    if let Some(direction) = tag.attribute("dir").and_then(html::direction) {
        character.direction = Some(direction);
        paragraph.base_direction = Some(direction);
    }
    let replacing_direction = block.is_some_and(|block| block.base_direction != paragraph.base_direction);
    let clearing_masked_bold = clear.contains(&StyleProperty::CharacterWeight)
        && character.weight.is_some() && character.bold.is_none();
    clear_character(
        &mut character,
        &clear
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    StyleProperty::CharacterFontFamilies
                        | StyleProperty::CharacterSize
                        | StyleProperty::CharacterWeight
                        | StyleProperty::CharacterBold
                        | StyleProperty::CharacterSlant
                        | StyleProperty::CharacterForeground
                        | StyleProperty::CharacterBackground
                        | StyleProperty::CharacterUnderline
                        | StyleProperty::CharacterStrikethrough
                        | StyleProperty::CharacterLanguage
                        | StyleProperty::CharacterDirection
                        | StyleProperty::CharacterOpenTypeFeatures
                        | StyleProperty::CharacterLetterSpacing
                        | StyleProperty::CharacterScriptPosition
                )
            })
            .copied()
            .collect(),
    )?;
    if (matches!(name.as_str(), "sup" | "sub") && clear.contains(&StyleProperty::CharacterScriptPosition))
        || (matches!(name.as_str(), "b" | "strong")
        && (clear.contains(&StyleProperty::CharacterBold) || clearing_masked_bold))
        || (matches!(name.as_str(), "i" | "em") && clear.contains(&StyleProperty::CharacterSlant))
        || (name == "u" && clear.contains(&StyleProperty::CharacterUnderline))
        || (matches!(name.as_str(), "s" | "strike" | "del")
            && clear.contains(&StyleProperty::CharacterStrikethrough))
    {
        name = "span".into();
    }
    if let Some(block) = block {
        if replacing_direction { character.direction = None; }
        paragraph = block.clone();
    } else if clear.contains(&StyleProperty::CharacterDirection) && !self::paragraph(&name) {
        // An inline direction has no independent paragraph declaration. Avoid
        // recreating its removed character declaration through block CSS.
        paragraph.base_direction = None;
    }
    let mut unsupported = Vec::new();
    if let Some(css) = tag.attribute("style") {
        for (key, value) in html::declarations(css) {
            let mut c = CharacterProperties::default();
            let mut p = BlockProperties::default();
            html::apply_css(&format!("{key}:{value}"), &mut c, &mut p);
            if c == CharacterProperties::default() && p == BlockProperties::default() {
                unsupported.push(format!("{key}: {value}"));
            }
        }
    }
    let css = [
        unsupported.join("; "),
        html::character_css(&character),
        super::html_styles::block_css(&paragraph),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("; ");
    let mut out = format!("<{name}");
    let mut seen = BTreeSet::new();
    for (key, value) in &tag.attributes {
        if !seen.insert(key)
            || key == "style"
            || (key == "lang" && clear.contains(&StyleProperty::CharacterLanguage))
            || (key == "dir" && (replacing_direction || clear.contains(&StyleProperty::CharacterDirection)))
        {
            continue;
        }
        out.push_str(&format!(" {key}={}", attribute(value)));
    }
    if replacing_direction && paragraph.base_direction == Some(WritingDirection::Natural) {
        out.push_str(" dir=\"auto\"");
    }
    if !css.is_empty() {
        out.push_str(&format!(" style={}", attribute(&css)));
    }
    out.push('>');
    Ok((out, name))
}

pub(super) fn paragraph_patches(
    input: &NormalizedText,
    desired: &[(Range<usize>, BlockProperties)],
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let tokens = html::tokenize(&input.text);
    let mut patches = Vec::new();
    let mut selected = BTreeSet::new();
    for (range, properties) in desired {
        let mut current = None;
        for token in &tokens {
            if mapper.source_range(token.range.clone()).start >= range.start {
                break;
            }
            if let TokenKind::Tag(tag) = &token.kind {
                if paragraph(&tag.name) {
                    current = if tag.end { None } else { Some(token) };
                }
            }
        }
        let token = current.ok_or(DocumentError::UnsupportedFormatting)?;
        if !selected.insert(token.range.start) {
            continue;
        }
        let TokenKind::Tag(tag) = &token.kind else {
            unreachable!()
        };
        patches.push((
            mapper.source_range(token.range.clone()),
            tag_with_properties(tag, &BTreeSet::new(), Some(properties))?.0,
        ));
    }
    Ok(patches)
}

struct Element {
    open: usize,
    close: usize,
    visible: Option<Range<usize>>,
    changed: String,
    new_name: String,
}
pub(super) fn clear_character_patches(
    input: &NormalizedText,
    projection: &FormattedDocument,
    range: Range<usize>,
    clear: &BTreeSet<StyleProperty>,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    if range.is_empty() || clear.is_empty() {
        return Ok(Vec::new());
    }
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let tokens = html::tokenize(&input.text);
    let mut stack: Vec<usize> = Vec::new();
    let mut elements = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenKind::Tag(tag) = &token.kind {
            if tag.end
                && (inline(&tag.name)
                    || paragraph(&tag.name)
                    || matches!(
                        tag.name.as_str(),
                        "body" | "html" | "div" | "section" | "article"
                    ))
            {
                if let Some(open) = stack.pop() {
                    let TokenKind::Tag(start) = &tokens[open].kind else {
                        unreachable!()
                    };
                    if start.name != tag.name {
                        return Err(DocumentError::AmbiguousProjection);
                    }
                    let source = mapper.source_range(tokens[open].range.end..token.range.start);
                    let spans = projection
                        .provenance()
                        .iter()
                        .filter(|span| {
                            span.source.start >= source.start
                                && span.source.end <= source.end
                                && !span.formatted.is_empty()
                        })
                        .collect::<Vec<_>>();
                    let visible = spans
                        .first()
                        .zip(spans.last())
                        .map(|(first, last)| first.formatted.start..last.formatted.end);
                    let (changed, new_name) = tag_with_properties(start, clear, None)?;
                    elements.push(Element {
                        open,
                        close: index,
                        visible,
                        changed,
                        new_name,
                    });
                }
            } else if !tag.end
                && (inline(&tag.name)
                    || paragraph(&tag.name)
                    || matches!(
                        tag.name.as_str(),
                        "body" | "html" | "div" | "section" | "article"
                    ))
            {
                stack.push(index);
            }
        }
    }
    let selected_source = projection
        .source_range(range.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let mut patches = Vec::new();
    let affected = |element: &Element| {
        let TokenKind::Tag(tag) = &tokens[element.open].kind else {
            unreachable!()
        };
        let mut original = CharacterProperties::default();
        let mut block = BlockProperties::default();
        if matches!(tag.name.as_str(), "b" | "strong") {
            original.bold = Some(true);
        }
        if matches!(tag.name.as_str(), "i" | "em") {
            original.slant = Some(FontSlant::Italic);
        }
        if tag.name == "sup" {
            original.script_position = Some(ScriptPosition::Superscript);
        }
        if tag.name == "sub" {
            original.script_position = Some(ScriptPosition::Subscript);
        }
        if tag.name == "u" {
            original.underline = Some(true);
        }
        if matches!(tag.name.as_str(), "s" | "strike" | "del") {
            original.strikethrough = Some(true);
        }
        if let Some(css) = tag.attribute("style") {
            html::apply_css(css, &mut original, &mut block);
        }
        if tag.attribute("lang").is_some() {
            original.language = Some("x".into());
        }
        if tag.attribute("dir").is_some() {
            original.direction = Some(WritingDirection::LeftToRight);
        }
        original
            .declared_properties()
            .iter()
            .any(|property| clear.contains(property))
    };
    let global_clear = elements
        .iter()
        .filter(|element| {
            let TokenKind::Tag(tag) = &tokens[element.open].kind else {
                unreachable!()
            };
            !inline(&tag.name)
                && affected(element)
                && element
                    .visible
                    .as_ref()
                    .is_some_and(|visible| visible.start < range.end && range.start < visible.end)
        })
        .map(|element| element.open)
        .collect::<BTreeSet<_>>();
    for element in &elements {
        let Some(visible) = &element.visible else {
            continue;
        };
        if visible.end <= range.start || visible.start >= range.end || !affected(element) {
            continue;
        }
        if range.start <= visible.start || global_clear.contains(&element.open) {
            patches.push((
                mapper.source_range(tokens[element.open].range.clone()),
                element.changed.clone(),
            ));
        }
        let TokenKind::Tag(tag) = &tokens[element.open].kind else {
            unreachable!()
        };
        if visible.end <= range.end && tag.name != element.new_name {
            patches.push((
                mapper.source_range(tokens[element.close].range.clone()),
                format!("</{}>", element.new_name),
            ));
        }
    }
    for (boundary, entering) in [(selected_source.start, true), (selected_source.end, false)] {
        let mut active = elements
            .iter()
            .filter(|element| {
                mapper.source_range(tokens[element.open].range.clone()).end <= boundary
                    && boundary
                        <= mapper
                            .source_range(tokens[element.close].range.clone())
                            .start
            })
            .collect::<Vec<_>>();
        active.sort_by_key(|element| element.open);
        let first = active.iter().position(|element| {
            !global_clear.contains(&element.open)
                && affected(element)
                && element.visible.as_ref().is_some_and(|visible| {
                    if entering {
                        visible.start < range.start && range.start < visible.end
                    } else {
                        visible.start < range.end && range.end < visible.end
                    }
                })
        });
        let Some(first) = first else {
            continue;
        };
        let active = &active[first..];
        if active.iter().any(|element| {
            let TokenKind::Tag(tag) = &tokens[element.open].kind else {
                unreachable!()
            };
            !inline(&tag.name)
        }) {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let mut syntax = String::new();
        for element in active.iter().rev() {
            let TokenKind::Tag(tag) = &tokens[element.open].kind else {
                unreachable!()
            };
            syntax.push_str(&format!(
                "</{}>",
                if entering {
                    &tag.name
                } else {
                    &element.new_name
                }
            ));
        }
        for element in active {
            syntax.push_str(if entering {
                &element.changed
            } else {
                &input.text[tokens[element.open].range.clone()]
            });
        }
        patches.push((boundary..boundary, syntax));
    }
    // Paragraph/body declarations are inherited by every descendant. Clearing
    // them on part of the paragraph removes the ancestor declaration once and
    // restores the original sparse values only on unaffected visible runs.
    let mut runs: Vec<(Range<usize>, CharacterProperties)> = Vec::new();
    for span in projection.provenance() {
        if span.formatted.is_empty()
            || span.source.is_empty()
            || projection.text().get(span.formatted.clone()) == Some("\n")
            || (span.formatted.start < range.end && range.start < span.formatted.end)
        {
            continue;
        }
        if !elements.iter().any(|element| {
            global_clear.contains(&element.open)
                && element.visible.as_ref().is_some_and(|visible| {
                    visible.start <= span.formatted.start && span.formatted.end <= visible.end
                })
        }) {
            continue;
        }
        let mut properties = CharacterProperties::default();
        for style in projection.style_spans().iter().filter(|style| {
            style.range.start <= span.formatted.start && span.formatted.end <= style.range.end
        }) {
            if let StyleApplication::Direct(layer) = &style.application {
                super::rich_text::overlay(&mut properties, layer);
            }
        }
        let mut removed = properties.clone();
        clear_character(&mut removed, clear)?;
        let retained = removed.declared_properties();
        clear_character(&mut properties, &retained)?;
        if properties == CharacterProperties::default() {
            continue;
        }
        if let Some((last, _)) = runs
            .last_mut()
            .filter(|(last, prior)| last.end == span.source.start && *prior == properties)
        {
            last.end = span.source.end;
        } else {
            runs.push((span.source.clone(), properties));
        }
    }
    for (source, properties) in runs {
        let (open, close) = html::character_wrapper(&properties);
        patches.push((source.start..source.start, open));
        patches.push((source.end..source.end, close));
    }
    Ok(patches)
}
