//! RTF stylesheet handles are stable identities. Only changed definition
//! groups are rewritten; siblings, unknown destinations, and table indexes
//! remain byte exact.
use super::line_endings::NormalizedText;
use super::rtf::{self, Kind, Token};
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[derive(Clone)]
pub(super) struct Entry {
    pub handle: i32,
    pub character: bool,
    pub id: StyleId,
    pub range: Range<usize>,
}
pub(super) struct RtfSheet {
    pub sheet: StyleSheet,
    pub entries: Vec<Entry>,
    pub close: Option<usize>,
}
impl RtfSheet {
    pub fn id(&self, handle: i32, character: bool) -> Option<StyleId> {
        self.entries
            .iter()
            .find(|entry| entry.handle == handle && entry.character == character)
            .map(|entry| entry.id.clone())
    }
}

fn groups(tokens: &[Token]) -> Vec<(usize, usize, usize)> {
    let mut stack = Vec::new();
    let mut result = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            Kind::Open => stack.push(index),
            Kind::Close => {
                if let Some(open) = stack.pop() {
                    result.push((open, index, stack.len()));
                }
            }
            _ => {}
        }
    }
    result
}

fn style_name(tokens: &[Token]) -> String {
    let mut units = Vec::new();
    let mut skip = 0usize;
    let mut uc = 1usize;
    let mut stack = Vec::new();
    for token in tokens {
        match &token.kind {
            Kind::Open => stack.push(uc),
            Kind::Close => {
                uc = stack.pop().unwrap_or(1);
                skip = 0;
            }
            Kind::Control(control, value) if control == "uc" => {
                uc = value.unwrap_or(1).max(0) as usize
            }
            Kind::Control(control, value) if control == "u" => {
                units.push(value.unwrap_or(0) as i16 as u16);
                skip = uc;
            }
            Kind::Character(character) => {
                if skip > 0 {
                    if !matches!(character, '\r' | '\n') {
                        skip -= 1;
                    }
                } else if *character == ';' {
                    break;
                } else if !matches!(character, '\r' | '\n') {
                    let mut buffer = [0; 2];
                    units.extend_from_slice(character.encode_utf16(&mut buffer));
                }
            }
            Kind::Symbol(character) => {
                if skip > 0 {
                    skip -= 1;
                } else if *character != '*' {
                    let mut buffer = [0; 2];
                    units.extend_from_slice(character.encode_utf16(&mut buffer));
                }
            }
            Kind::Byte(byte) => {
                if skip > 0 {
                    skip -= 1;
                } else {
                    units.push(rtf::windows_1252(*byte) as u16);
                }
            }
            _ => {}
        }
    }
    String::from_utf16_lossy(&units).trim().to_owned()
}

pub(super) fn read(input: &NormalizedText) -> RtfSheet {
    let tokens = rtf::tokenize(input);
    let groups = groups(&tokens);
    let tables = rtf::tables(&tokens);
    let header = rtf::header_group(&tokens, "stylesheet");
    let stylesheet = groups.iter().find(|(open, close, _)| {
        header
            .as_ref()
            .is_some_and(|header| header.start == *open && header.end == *close + 1)
    });
    let mut sheet = StyleSheet::default();
    sheet.set_intrinsic_character_defaults(CharacterProperties {
        size: Some(12.0.into()),
        ..Default::default()
    });
    let mut document = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
    document.character.size = Some(12.0.into());
    document.character.font_families = rtf::default_font_name(&tables)
        .map(|name| vec![name.to_owned()])
        .or(document.character.font_families);
    sheet
        .install_source_definitions(&[StyleDefinitionEdit::InsertBlock {
            style: document,
            metadata: StyleDefinitionMetadata::generated("Base Paragraph"),
        }])
        .expect("RTF document defaults have valid native properties");
    sheet.record_source_character_defaults(
        sheet.base_paragraph.clone(),
        CharacterProperties {
            font_families: rtf::default_font_name(&tables).map(|name| vec![name.to_owned()]),
            ..Default::default()
        },
    );
    let mut result = RtfSheet {
        sheet,
        entries: Vec::new(),
        close: stylesheet.map(|(_, close, _)| tokens[*close].range.start),
    };
    let Some((sheet_open, sheet_close, sheet_depth)) = stylesheet else {
        return result;
    };
    struct Raw {
        entry: Entry,
        name: String,
        parent: Option<i32>,
        next: Option<i32>,
        character: CharacterProperties,
        paragraph: BlockProperties,
    }
    let mut raw = Vec::new();
    let mut handles = BTreeSet::new();
    for (open, close, depth) in &groups {
        if *depth != sheet_depth + 1 || open <= sheet_open || close >= sheet_close {
            continue;
        }
        let body = rtf::definition_tokens(&tokens[open + 1..*close]);
        let first_control = body
            .iter()
            .find(|token| matches!(token.kind, Kind::Control(_, _)));
        let Some(Token {
            kind: Kind::Control(name, Some(handle)),
            ..
        }) = first_control
        else {
            continue;
        };
        if !matches!(name.as_str(), "s" | "cs") || *handle < 0 {
            continue;
        }
        let (character, handle) = (name == "cs", *handle);
        if !handles.insert((character, handle)) {
            continue;
        }
        let name = style_name(&body);
        if name.is_empty() {
            continue;
        }
        let mut state = rtf::State::default();
        let mut parent = None;
        let mut next = None;
        for token in &body {
            if let Kind::Control(control, value) = &token.kind {
                match control.as_str() {
                    "sbasedon" => parent = *value,
                    "snext" => next = *value,
                    _ => rtf::apply_control(&mut state, control, *value, &tables),
                }
            }
        }
        let id = if !character && handle == 0 {
            StyleId::from("Paragraph")
        } else {
            StyleId(format!(
                "Rtf{}{}",
                if character { "C" } else { "P" },
                handle
            ))
        };
        raw.push(Raw {
            entry: Entry {
                handle,
                character,
                id,
                range: tokens[*open].range.start..tokens[*close].range.end,
            },
            name,
            parent,
            next,
            character: state.character,
            paragraph: state.paragraph,
        });
    }
    let ids = raw
        .iter()
        .map(|raw| {
            (
                (raw.entry.character, raw.entry.handle),
                raw.entry.id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let definitions = raw
        .iter()
        .map(|raw| {
            let metadata = StyleDefinitionMetadata {
                display_name: raw.name.clone(),
                origin: StyleDefinitionOrigin::SourceBacked,
            };
            let parent = raw
                .parent
                .and_then(|handle| ids.get(&(raw.entry.character, handle)))
                .cloned();
            if raw.entry.character {
                StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: raw.entry.id.clone(),
                        based_on: parent,
                        properties: raw.character.clone(),
                    },
                    metadata,
                }
            } else {
                StyleDefinitionEdit::InsertBlock {
                    style: BlockStyle {
                        id: raw.entry.id.clone(),
                        based_on: if raw.entry.handle == 0 { None }
                            else { parent.or_else(|| Some("Paragraph".into())) },
                        next_paragraph_style: raw
                            .next
                            .and_then(|handle| ids.get(&(false, handle)))
                            .cloned()
                            .or_else(|| (raw.next == Some(0)).then(|| "Paragraph".into())),
                        role: BlockRole::Paragraph,
                        character: if raw.entry.handle == 0 {
                            let mut properties = raw.character.clone();
                            // The document's authored default-font table still
                            // applies when Normal omits its own font selector.
                            if properties.font_families.is_none() {
                                properties.font_families = rtf::default_font_name(&tables)
                                    .map(|name| vec![name.to_owned()]);
                            }
                            properties
                        } else {
                            raw.character.clone()
                        },
                        block: raw.paragraph.clone(),
                    },
                    metadata,
                }
            }
        })
        .collect::<Vec<_>>();
    if result
        .sheet
        .install_source_definitions(&definitions)
        .is_ok()
    {
        result.entries = raw.into_iter().map(|raw| raw.entry).collect();
    }
    result
}

pub(super) fn handle(sheet: &RtfSheet, id: &StyleId, character: bool) -> Option<i32> {
    sheet
        .entries
        .iter()
        .find(|entry| entry.character == character && &entry.id == id)
        .map(|entry| entry.handle)
        .or_else(|| (!character && id.0 == "Paragraph").then_some(0))
        .or_else(|| {
            id.0.strip_prefix(if character { "RtfC" } else { "RtfP" })
                .and_then(|suffix| suffix.parse().ok())
        })
}

pub(super) fn paragraph_controls(properties: &BlockProperties) -> Result<String, DocumentError> {
    let mut result = String::new();
    let twips = |value: f32| {
        if value.is_finite() && (value * 20.0).fract() == 0.0 {
            Ok((value * 20.0) as i32)
        } else {
            Err(DocumentError::UnsupportedFormatting)
        }
    };
    for (control, value) in [
        ("li", properties.leading_indent),
        ("ri", properties.trailing_indent),
        ("fi", properties.first_line_indent),
        ("sb", properties.spacing_before),
        ("sa", properties.spacing_after),
    ] {
        if let Some(value) = value {
            result.push_str(&format!("\\{control}{}", twips(value)?));
        }
    }
    if let Some(alignment) = properties.alignment {
        result.push_str(match alignment {
            ParagraphAlignment::Start => "\\ql",
            ParagraphAlignment::Center => "\\qc",
            ParagraphAlignment::End => "\\qr",
        });
    }
    if let Some(direction) = properties.base_direction {
        result.push_str(match direction {
            WritingDirection::LeftToRight => "\\ltrpar",
            WritingDirection::RightToLeft => "\\rtlpar",
            WritingDirection::Natural => return Err(DocumentError::UnsupportedFormatting),
        });
    }
    if let Some(spacing) = properties.line_spacing {
        result.push_str(&match spacing {
            LineSpacing::Normal => "\\sl0\\slmult0".into(),
            LineSpacing::Multiplier(value) if (value * 240.0).fract() == 0.0 => {
                format!("\\sl{}\\slmult1", (value * 240.0) as i32)
            }
            LineSpacing::Exact(value) => format!("\\sl{}\\slmult0", -twips(value)?),
            LineSpacing::AtLeast(value) => format!("\\sl{}\\slmult0", twips(value)?),
            _ => return Err(DocumentError::UnsupportedFormatting),
        });
    }
    if properties.padding_top.is_some()
        || properties.padding_right.is_some()
        || properties.padding_bottom.is_some()
        || properties.padding_left.is_some()
        || properties.background.is_some()
    {
        return Err(DocumentError::UnsupportedFormatting);
    }
    Ok(result)
}

pub(super) fn definition_patches(
    input: &NormalizedText,
    before: &StyleSheet,
    after: &StyleSheet,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let native = read(input);
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let mut patches = Vec::new();
    let mut additions = String::new();
    let mut ids = BTreeSet::new();
    for sheet in [before, after] {
        for style in sheet.block_styles() {
            if sheet
                .block_style_metadata(&style.id)
                .is_some_and(|metadata| metadata.origin == StyleDefinitionOrigin::SourceBacked)
            {
                ids.insert((false, style.id.clone()));
            }
        }
        for style in sheet.character_styles() {
            if sheet
                .character_style_metadata(&style.id)
                .is_some_and(|metadata| metadata.origin == StyleDefinitionOrigin::SourceBacked)
            {
                ids.insert((true, style.id.clone()));
            }
        }
    }
    for (character, id) in ids {
        let old = if character {
            before
                .character_style(&id)
                .map(|style| format!("{style:?}{:?}", before.character_style_metadata(&id)))
        } else {
            before
                .block_style(&id)
                .map(|style| format!("{style:?}{:?}", before.block_style_metadata(&id)))
        };
        let new = if character {
            after
                .character_style(&id)
                .map(|style| format!("{style:?}{:?}", after.character_style_metadata(&id)))
        } else {
            after
                .block_style(&id)
                .map(|style| format!("{style:?}{:?}", after.block_style_metadata(&id)))
        };
        let relative_paragraph_size = |sheet: &StyleSheet| -> Option<f32> {
            if character || !matches!(sheet.block_style(&id)?.character.size, Some(FontSize::Percentage(_))) {
                return None;
            }
            sheet.resolve_paragraph_style(&sheet.base_paragraph, &id, None,
                &BlockProperties::default(), &CharacterProperties::default())
                .ok().map(|style| style.character.size)
        };
        let fallback_size = relative_paragraph_size(after);
        if old == new && relative_paragraph_size(before) == fallback_size {
            continue;
        }
        let existing = native
            .entries
            .iter()
            .find(|entry| entry.character == character && entry.id == id);
        if new.is_none() {
            if let Some(existing) = existing {
                patches.push((mapper.source_range(existing.range.clone()), String::new()));
            }
            continue;
        }
        let number = handle(&native, &id, character).ok_or(DocumentError::UnsupportedFormatting)?;
        if number < 0
            || (existing.is_none()
                && native
                    .entries
                    .iter()
                    .any(|entry| entry.character == character && entry.handle == number))
        {
            return Err(DocumentError::UnsupportedFormatting);
        }
        let (properties, paragraph, parent, next, metadata) = if character {
            let style = after.character_style(&id).unwrap();
            (
                &style.properties,
                None,
                style.based_on.as_ref(),
                None,
                after.character_style_metadata(&id).unwrap(),
            )
        } else {
            let style = after.block_style(&id).unwrap();
            (
                &style.character,
                Some(&style.block),
                style.based_on.as_ref(),
                style.next_paragraph_style.as_ref(),
                after.block_style_metadata(&id).unwrap(),
            )
        };
        let mut controls = format!("{}{}", if character { "\\*\\cs" } else { "\\s" }, number);
        if let Some(parent) =
            parent.filter(|id| !(character && id.0.is_empty()))
        {
            controls.push_str(&format!(
                "\\sbasedon{}",
                handle(&native, parent, character).ok_or(DocumentError::UnsupportedFormatting)?
            ));
        }
        if let Some(next) = next {
            controls.push_str(&format!(
                "\\snext{}",
                handle(&native, next, false).ok_or(DocumentError::UnsupportedFormatting)?
            ));
        }
        // RTF has no relative-size control. Other readers receive a snapshot
        // in half-points; Viem's following extension retains the declaration.
        if let Some(size) = fallback_size {
            controls.push_str(&format!("\\fs{}", ((size * 2.0).round() as i32).max(1)));
        }
        let mut property_patches =
            rtf::character_patches(input, &(usize::MAX - 1..usize::MAX), properties)?;
        property_patches.pop();
        let (_, prefix) = property_patches
            .pop()
            .ok_or(DocumentError::UnsupportedFormatting)?;
        controls.push_str(prefix.trim_start_matches('{').trim_end());
        if let Some(paragraph) = paragraph {
            controls.push_str(&paragraph_controls(paragraph)?);
        }
        patches.extend(property_patches);
        let name = rtf::escape(&metadata.display_name).replace(';', "\\u59?");
        let syntax = format!("{{{controls} {name};}}");
        if let Some(existing) = existing {
            patches.push((mapper.source_range(existing.range.clone()), syntax));
        } else {
            additions.push_str(&syntax);
        }
    }
    if !additions.is_empty() {
        if let Some(close) = native.close {
            let at = mapper.source_range(close..close).start;
            patches.push((at..at, additions));
        } else {
            let tokens = rtf::tokenize(input);
            let token = &tokens
                [rtf::root_control(&tokens, "rtf").ok_or(DocumentError::UnsupportedFormatting)?];
            let at = mapper.source_range(token.range.clone()).end;
            patches.push((at..at, format!("{{\\stylesheet{additions}}}")));
        }
    }
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    // New resource tables and a new stylesheet can share the header gap.
    let mut merged: Vec<(Range<usize>, String)> = Vec::new();
    for (range, syntax) in patches {
        if let Some((last, text)) = merged
            .last_mut()
            .filter(|(last, _)| last.is_empty() && *last == range)
        {
            let _ = last;
            text.push_str(&syntax);
        } else {
            merged.push((range, syntax));
        }
    }
    Ok(merged)
}

/// Character assignments scope each visible source run. Crossing a source group
/// never expands the selection; existing sparse direct overrides follow cs.
pub(super) fn character_assignment_patches(
    input: &NormalizedText,
    projection: &FormattedDocument,
    range: &Range<usize>,
    id: &StyleId,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let native = read(input);
    let clearing = id.0.is_empty();
    let reset_through_paragraph = clearing && native.id(0, true).is_some();
    let number = if clearing {
        0
    } else {
        handle(&native, id, true).ok_or(DocumentError::UnsupportedFormatting)?
    };
    let mut runs: Vec<(Range<usize>, CharacterProperties, String)> = Vec::new();
    let mut at = range.start;
    for span in projection.provenance_for_region(range) {
        if span.formatted.is_empty() {
            continue;
        }
        if span.formatted.start != at || span.formatted.end > range.end {
            return Err(DocumentError::AmbiguousProjection);
        }
        at = span.formatted.end;
        let text = projection
            .text_tree()
            .slice(span.formatted.clone())
            .map_err(DocumentError::FormattedTextStorage)?;
        if text == "\n" {
            continue;
        }
        if span.source.is_empty() {
            return Err(DocumentError::AmbiguousProjection);
        }
        let mut properties = CharacterProperties::default();
        for style in projection.style_spans_for_region(&span.formatted) {
            if let StyleApplication::Direct(value) = style.application {
                super::rich_text::overlay(&mut properties, &value);
            }
        }
        let selector = if reset_through_paragraph {
            let block = projection
                .blocks_for_region(&span.formatted)
                .into_iter()
                .find(|block| block.range.start <= span.formatted.start
                    && span.formatted.end <= block.range.end)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let paragraph = handle(&native, &block.style, false)
                .ok_or(DocumentError::UnsupportedFormatting)?;
            // A real cs0 definition cannot represent "no character style".
            // Plain clears the named assignment, and restoring this same
            // paragraph immediately removes Plain's direct font resets. Keep
            // the paragraph's sparse direct declarations in this local scope.
            format!("\\plain\\s{paragraph}{}", paragraph_controls(&block.direct_paragraph)?)
        } else {
            format!("\\cs{number}")
        };
        if let Some((previous, _, _)) = runs
            .last_mut()
            .filter(|(previous, value, previous_selector)| {
                previous.end == span.source.start && *value == properties
                    && *previous_selector == selector
            })
        {
            previous.end = span.source.end;
        } else {
            runs.push((span.source, properties, selector));
        }
    }
    if at != range.end {
        return Err(DocumentError::AmbiguousProjection);
    }
    let mut patches = Vec::new();
    for (source, properties, selector) in runs {
        let mut direct = rtf::character_patches(input, &source, &properties)?;
        let (end, _) = direct.pop().ok_or(DocumentError::UnsupportedFormatting)?;
        let (start, prefix) = direct.pop().ok_or(DocumentError::UnsupportedFormatting)?;
        patches.extend(direct);
        patches.push((
            start,
            format!("{{{selector}{}", prefix.trim_start_matches('{')),
        ));
        patches.push((end, "}".to_owned()));
    }
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    patches.dedup();
    Ok(patches)
}

pub(super) fn assignment_patches(
    input: &NormalizedText,
    projection: &FormattedDocument,
    sources: &[Range<usize>],
    id: &StyleId,
    character: bool,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let native = read(input);
    let number = handle(&native, id, character).ok_or(DocumentError::UnsupportedFormatting)?;
    let tokens = rtf::tokenize(input);
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let groups = groups(&tokens)
        .into_iter()
        .map(|(open, close, _)| {
            mapper.source_range(tokens[open].range.start..tokens[close].range.end)
        })
        .collect::<Vec<_>>();
    let mut patches = Vec::new();
    for source in sources {
        let mut source = source.clone();
        let empty_paragraph = !character && source.is_empty();
        if empty_paragraph {
            // Empty inline scopes can close between the caret seed and its
            // paragraph delimiter. The next visible contributor must be that
            // delimiter; hidden destinations cannot supply it and following
            // text/objects cannot be crossed to find one.
            if let Some(next) = projection.provenance().iter().find(|span| {
                !span.formatted.is_empty()
                    && !span.source.is_empty()
                    && span.source.start >= source.start
            }) {
                if projection.text().get(next.formatted.clone()) == Some("\n")
                    && tokens.iter().any(|token| {
                        matches!(&token.kind, Kind::Control(name, _) if name == "par")
                            && mapper.source_range(token.range.clone()) == next.source
                    })
                {
                    source.end = next.source.end;
                }
            }
        }
        loop {
            let before = source.clone();
            for group in &groups {
                if group.start < source.start && source.start < group.end && group.end <= source.end
                {
                    source.start = group.start;
                }
                if source.start <= group.start && group.start < source.end && source.end < group.end
                {
                    source.end = group.end;
                }
            }
            if source == before {
                break;
            }
        }
        let control = if !character && number == 0 && native.id(0, false).is_none() {
            "\\pard".to_owned()
        } else {
            format!("\\{}{number}", if character { "cs" } else { "s" })
        };
        if empty_paragraph && !source.is_empty() {
            // A retained caret inside an empty group must keep this style
            // when text is later inserted there. Retarget inner paragraph
            // selectors; otherwise they would override the new outer scope.
            // Their following direct character controls remain authoritative.
            let has_text = projection.provenance().iter().any(|span| {
                !span.formatted.is_empty()
                    && span.source.start < source.end
                    && source.start < span.source.end
                    && projection.text().get(span.formatted.clone()) != Some("\n")
            });
            if !has_text {
                for token in rtf::definition_tokens(&tokens) {
                    let range = mapper.source_range(token.range.clone());
                    if source.start <= range.start && range.end <= source.end {
                        let replacement = match &token.kind {
                            Kind::Control(name, handle)
                                if name == "s" && *handle != Some(number) =>
                            {
                                Some(format!("{control} "))
                            }
                            Kind::Control(name, _) if name == "pard" && control != "\\pard" => {
                                Some(format!("\\pard{control} "))
                            }
                            _ => None,
                        };
                        if let Some(value) = replacement {
                            patches.push((range, value));
                        }
                    }
                }
            }
        }
        if source.is_empty() {
            patches.push((source, format!("{{{control} }}")));
        } else {
            patches.push((source.start..source.start, format!("{{{control} ")));
            patches.push((source.end..source.end, "}".to_owned()));
        }
    }
    Ok(patches)
}

pub(super) fn remove_assignment_patches(
    input: &NormalizedText,
    id: &StyleId,
    character: bool,
    replacement: Option<&StyleId>,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let native = read(input);
    let number = handle(&native, id, character).ok_or(DocumentError::UnsupportedFormatting)?;
    let replacement = replacement.and_then(|id| handle(&native, id, character));
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    Ok(rtf::tokenize(input)
        .into_iter()
        .filter(|token| {
            !native.entries.iter().any(|entry| {
                entry.range.start <= token.range.start && token.range.end <= entry.range.end
            })
        })
        .filter_map(|token| match &token.kind {
            Kind::Control(name, Some(handle))
                if name == if character { "cs" } else { "s" } && *handle == number =>
            {
                let syntax = replacement
                    .map(|handle| format!("\\{}{handle} ", if character { "cs" } else { "s" }))
                    .unwrap_or_else(|| {
                        if character {
                            "\\plain ".to_owned()
                        } else {
                            "\\pard ".to_owned()
                        }
                    });
                Some((mapper.source_range(token.range), syntax))
            }
            _ => None,
        })
        .collect())
}
