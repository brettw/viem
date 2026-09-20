//! Source-local direct declarations. Clearing removes affected controls and
//! restores their values around unselected content within the same lifetime.
use super::line_endings::NormalizedText;
use super::rtf::{self, Kind};
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

pub(super) fn character_property(name: &str, number: Option<i32>) -> Option<StyleProperty> {
    use StyleProperty::*;
    Some(match name {
        "b" => CharacterBold,
        "viemweight" if number.is_some_and(|value| (1..=1000).contains(&value)) => CharacterWeight,
        "viemfeatures" if number == Some(0) => CharacterOpenTypeFeatures,
        _ if number.is_some() && rtf::feature_control_tag(name).is_some() => {
            CharacterOpenTypeFeatures
        }
        "i" => CharacterSlant,
        "f" => CharacterFontFamilies,
        "fs" => CharacterSize,
        "cf" => CharacterForeground,
        "highlight" | "cb" | "chcbpat" => CharacterBackground,
        "ul" | "uld" | "uldash" | "uldb" | "ulw" | "ulnone" => CharacterUnderline,
        "strike" | "striked" => CharacterStrikethrough,
        "lang" => CharacterLanguage,
        "rtlch" | "ltrch" => CharacterDirection,
        "expndtw" | "expnd" => CharacterLetterSpacing,
        "super" | "sub" | "nosupersub" => CharacterScriptPosition,
        _ => return None,
    })
}

pub(super) fn clear_character_patches(
    input: &NormalizedText,
    projection: &FormattedDocument,
    range: Range<usize>,
    clear: &BTreeSet<StyleProperty>,
) -> Result<Vec<(Range<usize>, String)>, DocumentError> {
    let tokens = rtf::tokenize(input);
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let visible = projection
        .provenance()
        .iter()
        .filter(|span| !span.formatted.is_empty() && !span.source.is_empty())
        .collect::<Vec<_>>();
    let mut current: BTreeMap<StyleProperty, Vec<usize>> = BTreeMap::new();
    let mut stack = Vec::new();
    let mut selected: BTreeSet<usize> = BTreeSet::new();
    let mut outside = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            Kind::Open => stack.push(current.clone()),
            Kind::Close => current = stack.pop().unwrap_or_default(),
            Kind::Control(name, _) if matches!(name.as_str(), "s" | "cs") => current.clear(),
            Kind::Control(name, _) if name == "plain" => {
                for property in clear {
                    current.entry(*property).or_default().push(index);
                }
            }
            Kind::Control(name, number) => {
                if let Some(property) =
                    character_property(name, *number).filter(|property| clear.contains(property))
                {
                    current.entry(property).or_default().push(index);
                }
            }
            _ => {}
        }
        let source = mapper.source_range(token.range.clone());
        let start = visible.partition_point(|span| span.source.end <= source.start);
        for span in visible[start..]
            .iter()
            .take_while(|span| span.source.start < source.end)
        {
            let inside = range.start <= span.formatted.start && span.formatted.end <= range.end;
            let overlaps = range.start < span.formatted.end && span.formatted.start < range.end;
            if overlaps && !inside {
                return Err(DocumentError::AmbiguousProjection);
            }
            if inside {
                for controls in current.values() {
                    selected.extend(controls.iter().copied());
                }
            } else if projection.text().get(span.formatted.clone()) != Some("\n") {
                outside.push((span.source.clone(), current.clone()));
            }
        }
    }
    let mut result = Vec::new();
    let mut plain = rtf::State::default();
    rtf::apply_control(&mut plain, "plain", None, &rtf::tables(&tokens));
    let default_properties = plain.character;
    let mut controls_for = |properties: &CharacterProperties| -> Result<String, DocumentError> {
        if properties == &CharacterProperties::default() {
            return Ok(String::new());
        }
        let mut authored =
            rtf::character_patches(input, &(usize::MAX - 1..usize::MAX), properties)?;
        authored.pop();
        let (_, prefix) = authored.pop().ok_or(DocumentError::UnsupportedFormatting)?;
        for patch in authored {
            if !result.contains(&patch) {
                result.push(patch);
            }
        }
        Ok(prefix.trim_start_matches('{').trim_end().to_owned())
    };
    let mut replacements = Vec::new();
    for index in &selected {
        let index = *index;
        let token = &tokens[index];
        if matches!(&token.kind,Kind::Control(name,_) if name=="plain") {
            if super::rtf_styles::read(input).id(0, true).is_some() {
                return Err(DocumentError::UnsupportedFormatting);
            }
            let mut remaining = default_properties.clone();
            for property in clear {
                super::style::clear_character_property(&"Direct".into(), &mut remaining, *property)
                    .map_err(|_| DocumentError::UnsupportedFormatting)?;
            }
            let controls = controls_for(&remaining)?;
            replacements.push((
                mapper.source_range(token.range.clone()),
                format!("\\cs0{controls} "),
            ));
            continue;
        }
        // A removed control may have delimited a preceding control word. Keep
        // that lexical delimiter without introducing a visible space.
        let delimiter = if index > 0
            && !selected.contains(&(index - 1))
            && matches!(&tokens[index - 1].kind, Kind::Control(_, _))
            && tokens[index - 1].range.end == token.range.start
            && !input.text[..token.range.start].ends_with(' ')
        {
            " "
        } else {
            ""
        };
        replacements.push((
            mapper.source_range(token.range.clone()),
            delimiter.to_owned(),
        ));
    }
    let mut runs: Vec<(Range<usize>, String)> = Vec::new();
    for (source, properties) in outside {
        let mut controls = String::new();
        for (property, chain) in &properties {
            if !chain.iter().any(|index| selected.contains(index)) {
                continue;
            }
            if *property == StyleProperty::CharacterOpenTypeFeatures {
                // Each tag is a separate control in one sparse map. Restore
                // its full scoped sequence, including any font-default reset.
                for index in chain {
                    let token = &tokens[*index];
                    if matches!(&token.kind, Kind::Control(name, _) if name == "plain") {
                        controls.push_str("\\viemfeatures0");
                    } else {
                        controls.push_str(input.text[token.range.clone()].trim_end());
                    }
                }
                continue;
            }
            let token = &tokens[*chain.last().unwrap()];
            if matches!(&token.kind,Kind::Control(name,_) if name=="plain") {
                let mut selected_default = default_properties.clone();
                for other in default_properties
                    .declared_properties()
                    .into_iter()
                    .filter(|other| {
                        other != property
                            && !(*property == StyleProperty::CharacterWeight
                                && *other == StyleProperty::CharacterBold)
                    })
                {
                    super::style::clear_character_property(
                        &"Direct".into(),
                        &mut selected_default,
                        other,
                    )
                    .map_err(|_| DocumentError::UnsupportedFormatting)?;
                }
                controls.push_str(&controls_for(&selected_default)?);
            } else {
                controls.push_str(input.text[token.range.clone()].trim_end());
            }
        }
        if controls.is_empty() {
            continue;
        }
        if let Some((last, previous)) = runs
            .last_mut()
            .filter(|(last, previous)| last.end == source.start && *previous == controls)
        {
            let _ = previous;
            last.end = source.end;
        } else {
            runs.push((source, controls));
        }
    }
    drop(controls_for);
    result.extend(replacements);
    for (source, controls) in runs {
        result.push((source.start..source.start, format!("{{{controls} ")));
        result.push((source.end..source.end, "}".to_owned()));
    }
    result.sort_by_key(|(range, _)| (range.start, range.end));
    Ok(result)
}

/// Preserve the existing modern list selector when a paragraph reset is needed
/// to remove direct paragraph declarations. Table destinations are scoped, so
/// their internal handles cannot become body state after their closing brace.
pub(super) fn modern_list_at(input: &NormalizedText, source_at: usize) -> Option<(i32, i32)> {
    let mapper = super::rich_text::Builder::new(input, Revision(0));
    let mut state = (None, 0, false);
    let mut stack = Vec::new();
    for token in rtf::tokenize(input) {
        if mapper.source_range(token.range.clone()).start >= source_at {
            break;
        }
        match token.kind {
            Kind::Open => {
                stack.push(state);
                state.2 = false;
            }
            Kind::Close => {
                let numbering = state.2;
                state = stack.pop().unwrap_or((None, 0, false));
                if numbering {
                    state.0 = None;
                }
            }
            Kind::Control(name, value) => match name.as_str() {
                "ls" => state.0 = value.filter(|value| *value > 0),
                "ilvl" => state.1 = value.unwrap_or(0),
                "pard" => {
                    state.0 = None;
                    state.1 = 0;
                }
                "pn" => state.2 = true,
                _ => {}
            },
            _ => {}
        }
    }
    state.0.map(|handle| (handle, state.1))
}
