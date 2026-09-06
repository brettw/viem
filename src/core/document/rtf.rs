//! Passive RTF group/control interpretation. Original bytes (including binary
//! payloads, unknown destinations and fallback spellings) remain authoritative.
use super::line_endings::NormalizedText;
use super::rich_text::Builder;
use super::{
    BlockProperties, CharacterProperties, Color, FontSlant, FormattedDocument, LineSpacing,
    ParagraphAlignment, Revision, WritingDirection,
};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) enum Kind {
    Open,
    Close,
    Control(String, Option<i32>),
    Symbol(char),
    Character(char),
    Byte(u8),
    Binary,
}
#[derive(Clone, Debug)]
pub(super) struct Token {
    pub range: Range<usize>,
    pub kind: Kind,
}
pub(super) fn tokenize(input: &NormalizedText) -> Vec<Token> {
    let text = &input.text;
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut tokens = Vec::new();
    while at < bytes.len() {
        let start = at;
        let c = text[at..].chars().next().unwrap();
        at += c.len_utf8();
        let kind = match c {
            '{' => Kind::Open,
            '}' => Kind::Close,
            '\\' => {
                if at == bytes.len() {
                    Kind::Symbol('\\')
                } else if bytes[at].is_ascii_alphabetic() {
                    let name_start = at;
                    while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                        at += 1;
                    }
                    let name = text[name_start..at].to_owned();
                    let number_start = at;
                    if bytes.get(at) == Some(&b'-') {
                        at += 1;
                    }
                    while at < bytes.len() && bytes[at].is_ascii_digit() {
                        at += 1;
                    }
                    let number = text[number_start..at].parse::<i32>().ok();
                    if bytes.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    if name == "bin" {
                        tokens.push(Token {
                            range: start..at,
                            kind: Kind::Control(name, number),
                        });
                        let binary_start = at;
                        let first = input.units.partition_point(|u| u.normalized.end <= at);
                        let source_start =
                            input.units.get(first).map(|u| u.source.start).unwrap_or(0);
                        let source_end =
                            source_start.saturating_add(number.unwrap_or(0).max(0) as usize);
                        let last = input.units.partition_point(|u| u.source.start < source_end);
                        at = input
                            .units
                            .get(last)
                            .map(|u| u.normalized.start)
                            .unwrap_or(text.len());
                        tokens.push(Token {
                            range: binary_start..at,
                            kind: Kind::Binary,
                        });
                        continue;
                    }
                    Kind::Control(name, number)
                } else if bytes[at] == b'\''
                    && at + 2 < bytes.len()
                    && bytes[at + 1].is_ascii_hexdigit()
                    && bytes[at + 2].is_ascii_hexdigit()
                {
                    let value = u8::from_str_radix(&text[at + 1..at + 3], 16).unwrap();
                    at += 3;
                    Kind::Byte(value)
                } else {
                    let c = text[at..].chars().next().unwrap();
                    at += c.len_utf8();
                    Kind::Symbol(c)
                }
            }
            _ => Kind::Character(c),
        };
        tokens.push(Token {
            range: start..at,
            kind,
        });
    }
    tokens
}

pub(super) fn windows_1252(value: u8) -> char {
    const C1: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    if (0x80..=0x9f).contains(&value) {
        C1[(value - 0x80) as usize]
    } else {
        char::from(value)
    }
}
pub(super) fn non_body(name: &str) -> bool {
    matches!(
        name,
        "fonttbl"
            | "colortbl"
            | "stylesheet"
            | "info"
            | "pict"
            | "object"
            | "objdata"
            | "field"
            | "fldinst"
            | "fldrslt"
            | "header"
            | "headerl"
            | "headerr"
            | "headerf"
            | "footer"
            | "footerl"
            | "footerr"
            | "footerf"
            | "annotation"
            | "atnauthor"
            | "datastore"
            | "datafield"
            | "xmlnstbl"
            | "listtable"
            | "listoverridetable"
            | "generator"
            | "filetbl"
            | "revtbl"
            | "rsidtbl"
            | "themedata"
            | "colorschememapping"
            | "latentstyles"
            | "mmathPr"
            | "shp"
            | "shpinst"
            | "nonshppict"
            | "shppict"
            | "footnote"
            | "endnote"
            | "aftncn"
            | "ftncn"
            | "private"
            | "xe"
            | "tc"
            | "pntext"
            | "listtext"
    )
}
#[derive(Clone)]
pub(super) struct State {
    pub character: CharacterProperties,
    pub paragraph: BlockProperties,
    hidden: bool,
    group_start: bool,
    uc: usize,
    code_page: i32,
    font: i32,
    sl: Option<i32>,
    slmult: bool,
    list: Option<(bool, u64)>,
    modern_list: Option<i32>,
    list_level: u8,
    numbering_destination: bool,
    list_origin: Option<Range<usize>>,
    paragraph_style: Option<super::StyleId>,
    named_character: Option<super::StyleId>,
    group_output_start: usize,
}
impl Default for State {
    fn default() -> Self {
        Self {
            character: CharacterProperties::default(),
            paragraph: BlockProperties::default(),
            hidden: false,
            group_start: true,
            uc: 1,
            code_page: 1252,
            font: 0,
            sl: None,
            slmult: false,
            list: None,
            modern_list: None,
            list_level: 0,
            numbering_destination: false,
            list_origin: None,
            paragraph_style: None,
            named_character: None,
            group_output_start: 0,
        }
    }
}
#[derive(Default)]
pub(super) struct Tables {
    default_font: i32,
    default_language: Option<String>,
    fonts: BTreeMap<i32, String>,
    charsets: BTreeMap<i32, i32>,
    colors: Vec<Option<Color>>,
}
/// A recognized document header is a direct child of the outer RTF group.
/// Embedded or ignored destinations may contain arbitrary RTF-looking bytes;
/// those groups never supply the outer document's resource tables.
pub(super) fn header_group(tokens: &[Token], destination: &str) -> Option<Range<usize>> {
    let mut stack: Vec<(usize, Option<&str>)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            Kind::Open => stack.push((index, None)),
            Kind::Control(name, _) => {
                if let Some((_, first)) = stack.last_mut() {
                    if first.is_none() {
                        *first = Some(name);
                    }
                }
            }
            Kind::Close => {
                if let Some((open, first)) = stack.pop() {
                    if first == Some(destination) && stack.len() == 1 && stack[0].1 == Some("rtf") {
                        return Some(open..index + 1);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Controls belonging directly to the outer document, excluding all children.
pub(super) fn root_control(tokens: &[Token], requested: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut root = None;
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            Kind::Open => {
                if depth == 0 {
                    root = None;
                }
                depth += 1;
            }
            Kind::Close => depth = depth.saturating_sub(1),
            Kind::Control(name, _) if depth == 1 => {
                if root.is_none() {
                    root = Some(name.as_str());
                }
                if root == Some("rtf") && name == requested {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Descendant destinations do not contribute controls or name text to their
/// containing font/style definition.
pub(super) fn direct_tokens(tokens: &[Token]) -> Vec<Token> {
    let mut depth = 0usize;
    let mut direct = Vec::new();
    for token in tokens {
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth = depth.saturating_sub(1),
            _ if depth == 0 => direct.push(token.clone()),
            _ => {}
        }
    }
    direct
}

/// Style names may contain ordinary groups (including our scoped Unicode
/// escapes). Keep those while excluding nested non-body destinations.
pub(super) fn definition_tokens(tokens: &[Token]) -> Vec<Token> {
    let mut result = Vec::new();
    let mut stack = Vec::new();
    let mut hidden = false;
    for (index, token) in tokens.iter().enumerate() {
        match &token.kind {
            Kind::Open => {
                stack.push(hidden);
                let mut group_hidden = false;
                for next in &tokens[index + 1..] {
                    match &next.kind {
                        Kind::Symbol('*') => {
                            group_hidden = true;
                            break;
                        }
                        Kind::Control(name, _) => {
                            group_hidden = non_body(name);
                            break;
                        }
                        Kind::Character('\r' | '\n' | ' ') => {}
                        _ => break,
                    }
                }
                hidden |= group_hidden;
                if !hidden {
                    result.push(token.clone());
                }
            }
            Kind::Close => {
                if !hidden {
                    result.push(token.clone());
                }
                hidden = stack.pop().unwrap_or(false);
            }
            _ if !hidden => result.push(token.clone()),
            _ => {}
        }
    }
    result
}

pub(super) fn tables(tokens: &[Token]) -> Tables {
    let mut table = Tables::default();
    if let Some(index) = root_control(tokens, "deff") {
        if let Kind::Control(_, number) = &tokens[index].kind {
            table.default_font = number.unwrap_or(0);
        }
    }
    if let Some(index) = root_control(tokens, "deflang") {
        if let Kind::Control(_, number) = &tokens[index].kind {
            let mut state = State::default();
            apply_control(&mut state, "lang", *number, &table);
            table.default_language = state.character.language;
        }
    }
    if let Some(group) = header_group(tokens, "fonttbl") {
        let body = &tokens[group.start + 1..group.end - 1];
        let mut entries = vec![direct_tokens(body)];
        let mut depth = 0usize;
        let mut start = 0usize;
        for (index, token) in body.iter().enumerate() {
            match token.kind {
                Kind::Open => {
                    if depth == 0 {
                        start = index + 1;
                    }
                    depth += 1;
                }
                Kind::Close => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let direct = direct_tokens(&body[start..index]);
                        if direct.iter().find_map(|token| {
                            if let Kind::Control(name, _) = &token.kind {
                                Some(name.as_str())
                            } else {
                                None
                            }
                        }) == Some("f")
                        {
                            entries.push(direct);
                        }
                    }
                }
                _ => {}
            }
        }
        for entry in entries {
            let mut font = None;
            let mut name = String::new();
            for token in entry {
                match token.kind {
                    Kind::Control(control, number) if control == "f" => {
                        font = number;
                        name.clear();
                    }
                    Kind::Control(control, Some(charset)) if control == "fcharset" => {
                        if let Some(font) = font {
                            table.charsets.insert(font, charset);
                        }
                    }
                    Kind::Character(';') => {
                        if let Some(font) = font.take() {
                            table.fonts.insert(font, name.trim().to_owned());
                        }
                        name.clear();
                    }
                    Kind::Character(c) | Kind::Symbol(c) if !matches!(c, '\r' | '\n') => {
                        name.push(c)
                    }
                    Kind::Byte(byte) => name.push(windows_1252(byte)),
                    _ => {}
                }
            }
        }
    }
    if let Some(group) = header_group(tokens, "colortbl") {
        let mut color = [0u8; 3];
        let mut has_color = false;
        for token in direct_tokens(&tokens[group.start + 1..group.end - 1]) {
            match token.kind {
                Kind::Control(name, Some(value)) => {
                    let component = match name.as_str() {
                        "red" => Some(0),
                        "green" => Some(1),
                        "blue" => Some(2),
                        _ => None,
                    };
                    if let (Some(component), Ok(value)) = (component, u8::try_from(value)) {
                        color[component] = value;
                        has_color = true;
                    }
                }
                Kind::Character(';') => {
                    table.colors.push(has_color.then_some(Color {
                        red: f32::from(color[0]) / 255.0,
                        green: f32::from(color[1]) / 255.0,
                        blue: f32::from(color[2]) / 255.0,
                        alpha: 1.0,
                    }));
                    color = [0; 3];
                    has_color = false;
                }
                _ => {}
            }
        }
    }
    table
}
pub(super) fn default_font_name(tables: &Tables) -> Option<&str> {
    tables.fonts.get(&tables.default_font).map(String::as_str)
}
pub(super) fn apply_control(state: &mut State, name: &str, number: Option<i32>, tables: &Tables) {
    let enabled = number.unwrap_or(1) != 0;
    let twips = number.map(|n| n as f32 / 20.0);
    match name {
        "plain" => {
            state.font = tables.default_font;
            state.named_character = None;
            state.character = CharacterProperties {
                font_families: tables
                    .fonts
                    .get(&tables.default_font)
                    .map(|name| vec![name.clone()]),
                size: Some(12.0),
                weight: Some(400),
                slant: Some(FontSlant::Upright),
                foreground: Some(Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                }),
                background: Some(Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 0.0,
                }),
                underline: Some(false),
                strikethrough: Some(false),
                language: tables.default_language.clone(),
                letter_spacing: Some(0.0),
                baseline_shift: Some(0.0),
                ..Default::default()
            };
        }
        "pard" => {
            state.paragraph = BlockProperties::default();
            state.paragraph_style = None;
            state.sl = None;
            state.slmult = false;
            state.list = None;
            state.modern_list = None;
            state.list_level = 0;
        }
        "ls" => {
            state.modern_list = number.filter(|n| (1..=2000).contains(n));
            state.list = None;
        }
        "ilvl" => state.list_level = number.filter(|n| (0..9).contains(n)).unwrap_or(0) as u8,
        "pn" => {
            state.numbering_destination = true;
            state.hidden = true;
        }
        "pnlvlblt" => state.list = Some((false, 1)),
        "pnlvlbody" => state.list = None,
        "pndec" => state.list = Some((true, 1)),
        "pnstart" => {
            if let Some(n) = number.filter(|n| *n >= 0) {
                state.list.get_or_insert((true, 1)).1 = n as u64;
            }
        }
        "b" => state.character.weight = Some(if enabled { 700 } else { 400 }),
        "i" => {
            state.character.slant = Some(if enabled {
                FontSlant::Italic
            } else {
                FontSlant::Upright
            })
        }
        "ul" | "uld" | "uldash" | "uldb" | "ulw" => state.character.underline = Some(enabled),
        "ulnone" => state.character.underline = Some(false),
        "strike" | "striked" => state.character.strikethrough = Some(enabled),
        "f" => {
            if let Some(font) = number {
                state.font = font;
                if let Some(name) = tables.fonts.get(&font) {
                    state.character.font_families = Some(vec![name.clone()]);
                }
            }
        }
        "fs" => {
            if let Some(size) = number.filter(|n| *n > 0) {
                state.character.size = Some(size as f32 / 2.0);
            }
        }
        "cf" => {
            if number == Some(0) && tables.colors.first().map_or(true, Option::is_none) {
                state.character.foreground = Some(Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                });
            } else if let Some(color) = number
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|i| tables.colors.get(i))
                .copied()
                .flatten()
            {
                state.character.foreground = Some(color);
            }
        }
        "highlight" | "cb" | "chcbpat" => {
            if number == Some(0) && tables.colors.first().map_or(true, Option::is_none) {
                state.character.background = Some(Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 0.0,
                });
            } else if let Some(color) = number
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|i| tables.colors.get(i))
                .copied()
                .flatten()
            {
                state.character.background = Some(color);
            }
        }
        "expndtw" => state.character.letter_spacing = twips,
        "expnd" => state.character.letter_spacing = number.map(|n| n as f32 / 4.0),
        "up" => state.character.baseline_shift = Some(number.unwrap_or(6) as f32 / 2.0),
        "dn" => state.character.baseline_shift = Some(-(number.unwrap_or(6) as f32) / 2.0),
        "super" => {
            state.character.baseline_shift = Some(state.character.size.unwrap_or(12.0) * 0.35)
        }
        "sub" => {
            state.character.baseline_shift = Some(-state.character.size.unwrap_or(12.0) * 0.20)
        }
        "nosupersub" => state.character.baseline_shift = Some(0.0),
        "rtlch" => state.character.direction = Some(WritingDirection::RightToLeft),
        "ltrch" => state.character.direction = Some(WritingDirection::LeftToRight),
        "rtlpar" => state.paragraph.base_direction = Some(WritingDirection::RightToLeft),
        "ltrpar" => state.paragraph.base_direction = Some(WritingDirection::LeftToRight),
        "lang" => {
            if let Some(language) = match number {
                Some(1033) => Some("en-US"),
                Some(1031) => Some("de-DE"),
                Some(1036) => Some("fr-FR"),
                Some(1041) => Some("ja-JP"),
                Some(2057) => Some("en-GB"),
                Some(1034 | 3082) => Some("es-ES"),
                _ => None,
            } {
                state.character.language = Some(language.into());
            }
        }
        "li" => state.paragraph.leading_indent = twips,
        "ri" => state.paragraph.trailing_indent = twips,
        "fi" => state.paragraph.first_line_indent = twips,
        "sb" => state.paragraph.spacing_before = twips,
        "sa" => state.paragraph.spacing_after = twips,
        "ql" => state.paragraph.alignment = Some(ParagraphAlignment::Start),
        "qc" => state.paragraph.alignment = Some(ParagraphAlignment::Center),
        "qr" => state.paragraph.alignment = Some(ParagraphAlignment::End),
        "sl" => state.sl = number,
        "slmult" => state.slmult = enabled,
        _ => {}
    }
    if let Some(sl) = state.sl {
        state.paragraph.line_spacing = Some(if state.slmult {
            LineSpacing::Multiplier(sl as f32 / 240.0)
        } else if sl < 0 {
            LineSpacing::Exact(-(sl as f32) / 20.0)
        } else {
            LineSpacing::AtLeast(sl as f32 / 20.0)
        });
    }
}
fn codepage_encoding(state: &State, tables: &Tables) -> Option<&'static encoding_rs::Encoding> {
    let codepage = match tables.charsets.get(&state.font).copied() {
        Some(0 | 1) => state.code_page,
        Some(2) => return None,
        Some(128) => 932,
        Some(129) => 949,
        Some(134) => 936,
        Some(136) => 950,
        Some(204) => 1251,
        Some(238) => 1250,
        Some(161) => 1253,
        Some(162) => 1254,
        Some(177) => 1255,
        Some(178) => 1256,
        Some(186) => 1257,
        Some(163) => 1258,
        _ => state.code_page,
    };
    let label = match codepage {
        1250..=1258 => format!("windows-{codepage}"),
        874 => "windows-874".into(),
        10000 => "macintosh".into(),
        932 => "shift_jis".into(),
        936 => "gbk".into(),
        949 => "euc-kr".into(),
        950 => "big5".into(),
        65001 => "utf-8".into(),
        _ => return None,
    };
    encoding_rs::Encoding::for_label(label.as_bytes())
}
fn emit_encoded_byte(
    byte: u8,
    mut range: Range<usize>,
    tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>,
    state: &State,
    tables: &Tables,
    builder: &mut Builder<'_>,
) {
    if byte < 128 {
        builder.emit(&char::from(byte).to_string(), range, &state.character);
        return;
    }
    let Some(encoding) = codepage_encoding(state, tables) else {
        builder.emit_read_only("\u{fffd}");
        return;
    };
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut output = String::with_capacity(32);
    let mut next = byte;
    let mut invalid = false;
    loop {
        let (_, _, errors) = decoder.decode_to_string(&[next], &mut output, false);
        invalid |= errors;
        if !output.is_empty() {
            break;
        }
        let following = tokens.peek().and_then(|token| match token.kind {
            Kind::Byte(b) => Some(b),
            Kind::Character(c) if c as u32 <= 255 && !matches!(c, '\r' | '\n') => Some(c as u8),
            _ => None,
        });
        if let Some(byte) = following {
            let token = tokens.next().unwrap();
            range.end = token.range.end;
            next = byte;
        } else {
            let (_, _, errors) = decoder.decode_to_string(&[], &mut output, true);
            invalid |= errors;
            break;
        }
    }
    if invalid {
        builder.emit_read_only(&output);
    } else {
        builder.emit(&output, range, &state.character);
    }
}

pub(super) fn project(
    input: &NormalizedText,
    revision: Revision,
    start: usize,
    end: usize,
) -> FormattedDocument {
    let tokens = tokenize(input);
    let tables = tables(&tokens);
    let list_tables = super::rtf_lists::ListTables::read(&tokens);
    let mut numbering = super::rtf_lists::Numbering::default();
    let mut paragraph_list = None;
    let mut paragraph_started = false;
    let mut paragraph_source_start = 0usize;
    let mut builder = Builder::new(input, revision);
    let styles = super::rtf_styles::read(input);
    builder.style_sheet = styles.sheet.clone();
    let mut stack = Vec::new();
    let mut state = State::default();
    state.font = tables.default_font;
    let mut skip = 0usize;
    let mut pending_unicode: Option<(u16, Range<usize>, CharacterProperties)> = None;
    let flush = |pending: &mut Option<(u16, Range<usize>, CharacterProperties)>,
                 builder: &mut Builder<'_>| {
        if let Some((_, range, style)) = pending.take() {
            builder.emit("\u{fffd}", range, &style);
        }
    };
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        if !state.hidden
            && matches!(
                &token.kind,
                Kind::Character(c) if !matches!(c, '\r' | '\n')
            )
            || (!state.hidden
                && matches!(
                    &token.kind,
                    Kind::Byte(_) | Kind::Symbol('\\' | '{' | '}' | '~' | '_' | '-')
                ))
            || (!state.hidden
                && matches!(&token.kind, Kind::Control(name, _) if matches!(name.as_str(), "tab" | "u" | "bullet" | "emdash" | "endash" | "lquote" | "rquote" | "ldblquote" | "rdblquote") || (builder.line_is_empty() && matches!(name.as_str(), "par" | "line"))))
            || (!state.hidden
                && !paragraph_started
                && matches!(token.kind, Kind::Close)
                && state.modern_list.is_some()
                && state
                    .list_origin
                    .as_ref()
                    .is_some_and(|origin| origin.end >= paragraph_source_start))
        {
            if !paragraph_started {
                paragraph_started = true;
                paragraph_list = state.modern_list.and_then(|id| {
                    numbering.next(&list_tables, id, state.list_level).map(
                        |(ordered, ordinal, first)| (id, state.list_level, ordered, ordinal, first),
                    )
                });
                if paragraph_list.is_none() {
                    numbering.leave();
                }
            }
            builder.kind = paragraph_list
                .map(
                    |(_, level, ordered, ordinal, first)| super::BlockKind::ListItem {
                        ordered,
                        ordinal,
                        level,
                        container_start: first,
                        item_start: true,
                    },
                )
                .unwrap_or_else(|| {
                    state
                        .list
                        .map(|(ordered, ordinal)| super::BlockKind::ListItem {
                            ordered,
                            ordinal,
                            level: 0,
                            container_start: false,
                            item_start: true,
                        })
                        .unwrap_or(super::BlockKind::Paragraph)
                });
            builder.marker_origin = if paragraph_list.is_some() {
                state
                    .list_origin
                    .clone()
                    .or(Some(token.range.start..token.range.start))
            } else {
                state.list_origin.clone()
            };
            builder.paragraph_style = state.paragraph_style.clone();
            builder.named_character = state.named_character.clone();
            builder.paragraph = state.paragraph.clone();
            if let Some((id, level, ..)) = paragraph_list {
                if let Some(defaults) = list_tables.level(id, level) {
                    builder.paragraph.leading_indent = builder
                        .paragraph
                        .leading_indent
                        .or(defaults.paragraph.leading_indent);
                    builder.paragraph.first_line_indent = builder
                        .paragraph
                        .first_line_indent
                        .or(defaults.paragraph.first_line_indent);
                }
                if matches!(token.kind, Kind::Close) {
                    builder.emit_list_marker();
                }
            }
            if state.list.is_none() && paragraph_list.is_none() {
                if let Some(level) = state
                    .paragraph_style
                    .as_ref()
                    .and_then(|style| builder.style_sheet.block_style_metadata(style))
                    .and_then(|metadata| {
                        let name = metadata.display_name.to_ascii_lowercase();
                        let matches = styles
                            .entries
                            .iter()
                            .filter(|entry| !entry.character)
                            .filter(|entry| {
                                builder
                                    .style_sheet
                                    .block_style_metadata(&entry.id)
                                    .is_some_and(|other| {
                                        other.display_name.eq_ignore_ascii_case(&name)
                                    })
                            })
                            .count();
                        (matches == 1)
                            .then(|| {
                                name.strip_prefix("heading ")
                                    .and_then(|level| level.parse::<u8>().ok())
                            })
                            .flatten()
                    })
                    .filter(|level| (1..=6).contains(level))
                {
                    builder.kind = super::BlockKind::Heading(level);
                }
            }
        }
        if skip > 0 {
            match token.kind {
                Kind::Character('\r' | '\n') => continue,
                Kind::Character(_) | Kind::Byte(_) | Kind::Symbol(_) => {
                    skip -= 1;
                    if let Some((_, range, _)) = pending_unicode.as_mut() {
                        range.end = token.range.end;
                    } else {
                        let source_end = builder.source_range(token.range.clone()).end;
                        if let Some(span) = builder.provenance.last_mut() {
                            span.source.end = source_end;
                        }
                    }
                    continue;
                }
                Kind::Open | Kind::Close => skip = 0,
                _ => {
                    skip -= 1;
                    continue;
                }
            }
        }
        match token.kind {
            Kind::Open => {
                stack.push(state.clone());
                state.group_start = true;
                state.numbering_destination = false;
                state.group_output_start = builder.text.len();
            }
            Kind::Close => {
                flush(&mut pending_unicode, &mut builder);
                if !state.hidden
                    && builder.line_is_empty()
                    && state.list.is_some()
                    && state
                        .list_origin
                        .as_ref()
                        .is_some_and(|origin| origin.end >= paragraph_source_start)
                {
                    builder.emit_list_marker();
                    builder.retain_empty_boundary(
                        builder
                            .source_range(token.range.start..token.range.start)
                            .start,
                    );
                }
                if !state.hidden
                    && (state.group_output_start == builder.text.len() || builder.line_is_empty())
                {
                    builder.retain_empty_boundary(
                        builder
                            .source_range(token.range.start..token.range.start)
                            .start,
                    );
                }
                let list = state.numbering_destination.then_some(state.list);
                state = stack.pop().unwrap_or_default();
                if let Some(list) = list {
                    state.list = list;
                    state.modern_list = None;
                    state.list_origin = Some(token.range.clone());
                    if builder.line_is_empty() {
                        builder.kind = list
                            .map(|(ordered, ordinal)| super::BlockKind::ListItem {
                                ordered,
                                ordinal,
                                level: 0,
                                container_start: false,
                                item_start: true,
                            })
                            .unwrap_or(super::BlockKind::Paragraph);
                        builder.marker_origin = Some(token.range.clone());
                        if list.is_none() {
                            builder.empty_boundary_at(token.range.end);
                        }
                    }
                }
            }
            Kind::Binary => {}
            Kind::Symbol('*') if state.group_start => state.hidden = true,
            Kind::Control(name, number) => {
                if state.group_start && non_body(&name) {
                    if !state.hidden && matches!(name.as_str(), "pict" | "object" | "field" | "shp")
                    {
                        builder.emit_read_only("\u{fffc}");
                    }
                    state.hidden = true;
                }
                if !matches!(name.as_str(), "rtf" | "ansi" | "mac" | "pc" | "pca") {
                    state.group_start = false;
                }
                match name.as_str() {
                    "ls" | "ilvl" if !state.hidden => {
                        apply_control(&mut state, &name, number, &tables);
                        state.list_origin = Some(token.range.clone());
                    }
                    "s" if !state.hidden => {
                        if let Some(id) = number.and_then(|handle| {
                            styles.id(handle, false).or_else(|| {
                                (handle == 0).then(|| super::StyleId::from("Paragraph"))
                            })
                        }) {
                            state.paragraph_style = Some(id.clone());
                            state.character = CharacterProperties::default();
                            state.paragraph = BlockProperties::default();
                            if builder.line_is_empty() {
                                builder.paragraph_style = Some(id);
                                builder.kind = super::BlockKind::Paragraph;
                            }
                        }
                    }
                    "cs" if !state.hidden => {
                        if let Some(id) = number.and_then(|handle| styles.id(handle, true)) {
                            state.named_character = Some(id);
                            state.character = CharacterProperties::default();
                        } else if number == Some(0) {
                            state.named_character = None;
                            state.character = CharacterProperties::default();
                        }
                    }
                    "uc" => state.uc = number.unwrap_or(1).clamp(0, 32767) as usize,
                    "ansicpg" => state.code_page = number.unwrap_or(1252),
                    "u" if !state.hidden => {
                        let unit = number.unwrap_or(0) as i16 as u16;
                        if (0xd800..=0xdbff).contains(&unit) {
                            flush(&mut pending_unicode, &mut builder);
                            pending_unicode =
                                Some((unit, token.range.clone(), state.character.clone()));
                        } else if (0xdc00..=0xdfff).contains(&unit) {
                            if let Some((high, range, style)) = pending_unicode.take() {
                                let code = 0x10000
                                    + ((u32::from(high) - 0xd800) << 10)
                                    + (u32::from(unit) - 0xdc00);
                                builder.emit(
                                    &char::from_u32(code).unwrap_or('\u{fffd}').to_string(),
                                    range.start..token.range.end,
                                    &style,
                                );
                            } else {
                                builder.emit("\u{fffd}", token.range.clone(), &state.character);
                            }
                        } else {
                            flush(&mut pending_unicode, &mut builder);
                            builder.emit(
                                &char::from_u32(u32::from(unit))
                                    .unwrap_or('\u{fffd}')
                                    .to_string(),
                                token.range.clone(),
                                &state.character,
                            );
                        }
                        skip = state.uc;
                    }
                    "par" | "line" if !state.hidden => {
                        flush(&mut pending_unicode, &mut builder);
                        builder.paragraph = state.paragraph.clone();
                        if let Some((id, level, ..)) = paragraph_list {
                            if let Some(defaults) = list_tables.level(id, level) {
                                builder.paragraph.leading_indent = builder
                                    .paragraph
                                    .leading_indent
                                    .or(defaults.paragraph.leading_indent);
                                builder.paragraph.first_line_indent = builder
                                    .paragraph
                                    .first_line_indent
                                    .or(defaults.paragraph.first_line_indent);
                            }
                        }
                        if name == "par" {
                            paragraph_source_start = token.range.end;
                            builder.paragraph_break(token.range);
                            builder.kind = super::BlockKind::Paragraph;
                            paragraph_started = false;
                            paragraph_list = None;
                        } else {
                            builder.hard_break(token.range);
                        }
                    }
                    "tab" if !state.hidden => {
                        flush(&mut pending_unicode, &mut builder);
                        builder.emit("\t", token.range, &state.character);
                    }
                    "emdash" | "endash" | "bullet" | "lquote" | "rquote" | "ldblquote"
                    | "rdblquote"
                        if !state.hidden =>
                    {
                        flush(&mut pending_unicode, &mut builder);
                        let value = match name.as_str() {
                            "emdash" => "—",
                            "endash" => "–",
                            "bullet" => "•",
                            "lquote" => "‘",
                            "rquote" => "’",
                            "ldblquote" => "“",
                            _ => "”",
                        };
                        builder.emit(value, token.range, &state.character);
                    }
                    _ => apply_control(&mut state, &name, number, &tables),
                }
            }
            Kind::Character('\r' | '\n') => {}
            Kind::Character(c) if !state.hidden => {
                state.group_start = false;
                flush(&mut pending_unicode, &mut builder);
                builder.paragraph = state.paragraph.clone();
                if let Some((id, level, ..)) = paragraph_list {
                    if let Some(defaults) = list_tables.level(id, level) {
                        builder.paragraph.leading_indent = builder
                            .paragraph
                            .leading_indent
                            .or(defaults.paragraph.leading_indent);
                        builder.paragraph.first_line_indent = builder
                            .paragraph
                            .first_line_indent
                            .or(defaults.paragraph.first_line_indent);
                    }
                }
                if c as u32 <= 255 {
                    emit_encoded_byte(
                        c as u8,
                        token.range,
                        &mut tokens,
                        &state,
                        &tables,
                        &mut builder,
                    );
                } else {
                    builder.emit(&c.to_string(), token.range, &state.character);
                }
            }
            Kind::Byte(byte) if !state.hidden => {
                state.group_start = false;
                flush(&mut pending_unicode, &mut builder);
                builder.paragraph = state.paragraph.clone();
                if let Some((id, level, ..)) = paragraph_list {
                    if let Some(defaults) = list_tables.level(id, level) {
                        builder.paragraph.leading_indent = builder
                            .paragraph
                            .leading_indent
                            .or(defaults.paragraph.leading_indent);
                        builder.paragraph.first_line_indent = builder
                            .paragraph
                            .first_line_indent
                            .or(defaults.paragraph.first_line_indent);
                    }
                }
                emit_encoded_byte(
                    byte,
                    token.range,
                    &mut tokens,
                    &state,
                    &tables,
                    &mut builder,
                );
            }
            Kind::Symbol(c) if !state.hidden => {
                state.group_start = false;
                flush(&mut pending_unicode, &mut builder);
                let value = match c {
                    '\\' => Some("\\"),
                    '{' => Some("{"),
                    '}' => Some("}"),
                    '~' => Some("\u{a0}"),
                    '_' => Some("\u{2011}"),
                    '-' => Some("\u{ad}"),
                    _ => None,
                };
                if let Some(value) = value {
                    builder.emit(value, token.range, &state.character);
                }
            }
            _ => {}
        }
    }
    flush(&mut pending_unicode, &mut builder);
    builder.finish(start, end)
}

/// Canonical list controls cover the selected paragraph and its terminator,
/// with supporting scopes retaining neighboring body state and original bytes.
pub(super) fn list_patches(
    input: &NormalizedText,
    targets: &[(Range<usize>, Option<super::ListStyle>, u64, Option<u64>)],
    projection: &FormattedDocument,
) -> Result<Vec<(Range<usize>, String)>, super::DocumentError> {
    let mut patches = Vec::new();
    let mapper = Builder::new(input, Revision(0));
    let mut opens = Vec::new();
    let mut groups = Vec::new();
    for token in tokenize(input) {
        match token.kind {
            Kind::Open => opens.push(mapper.source_range(token.range).start),
            Kind::Close => {
                if let Some(start) = opens.pop() {
                    groups.push(start..mapper.source_range(token.range).end);
                }
            }
            _ => {}
        }
    }
    for (source, style, ordinal, _) in targets {
        let mut source = source.clone();
        // The delimiter carries the same paragraph formatting as the body in
        // external RTF readers. Preserve its bytes while including it in the
        // new scope; a following paragraph's controls remain outside.
        if let Some(next) = projection.provenance().iter().find(|span| {
            !span.formatted.is_empty() && !span.source.is_empty() && span.source.start >= source.end
        }) {
            if projection.text().get(next.formatted.clone()) == Some("\n") {
                source.end = next.source.end;
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
        let prefix = match style {
            Some(super::ListStyle::Bullet) => "{\\ls0\\li400\\fi-200{\\pntext\\bullet\\tab}{\\*\\pn\\pnlvlblt\\pnf0{\\pntxtb\\bullet}}".to_owned(),
            Some(super::ListStyle::Numbered) => format!("{{\\ls0\\li400\\fi-200{{\\pntext {ordinal}.\\tab}}{{\\*\\pn\\pnlvlbody\\pndec\\pnstart{ordinal}{{\\pntxta .}}}}"),
            None => "{\\ls0\\li0\\fi0{\\*\\pn\\pnlvlbody}".to_owned(),
        };
        if source.is_empty() {
            patches.push((source.clone(), format!("{prefix}}}")));
        } else {
            patches.push((source.start..source.start, prefix));
            patches.push((source.end..source.end, "}".to_owned()));
        }
    }
    // Adjacent paragraphs share the boundary after the first \par. Close its
    // scope before opening the next scope in one insertion at that boundary.
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    let mut merged: Vec<(Range<usize>, String)> = Vec::new();
    for (range, syntax) in patches {
        if let Some((_, previous)) = merged
            .last_mut()
            .filter(|(previous, _)| previous.is_empty() && *previous == range)
        {
            previous.push_str(&syntax);
        } else {
            merged.push((range, syntax));
        }
    }
    Ok(merged)
}

/// Inserted RTF scopes its Unicode fallback count so surrounding \uc state
/// cannot change reopening or subsequent text.
pub(super) fn escape(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = String::from("{\\uc1 ");
    for c in text.chars() {
        match c {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\par "),
            '\t' => out.push_str("\\tab "),
            c if c.is_ascii() && !c.is_control() => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out.push('}');
    out
}

pub(super) fn character_patches(
    input: &NormalizedText,
    range: &Range<usize>,
    properties: &CharacterProperties,
) -> Result<Vec<(Range<usize>, String)>, super::DocumentError> {
    use super::DocumentError::UnsupportedFormatting;
    let tokens = tokenize(input);
    let mut tables = tables(&tokens);
    let builder = Builder::new(input, Revision(0));
    let mut control = String::new();
    let mut patches = Vec::new();
    let mut font_additions = String::new();
    let mut color_additions = String::new();
    if let Some(families) = &properties.font_families {
        if families.len() != 1 || families[0].contains([';', '\n', '\r']) || !families[0].is_ascii()
        {
            return Err(UnsupportedFormatting);
        }
        let family = &families[0];
        let handle =
            if let Some((handle, _)) = tables.fonts.iter().find(|(_, name)| *name == family) {
                *handle
            } else {
                let handle = tables
                    .fonts
                    .keys()
                    .copied()
                    .max()
                    .unwrap_or(-1)
                    .checked_add(1)
                    .ok_or(UnsupportedFormatting)?;
                let escaped = family
                    .replace('\\', "\\\\")
                    .replace('{', "\\{")
                    .replace('}', "\\}");
                font_additions.push_str(&format!("{{\\f{handle}\\fnil {escaped};}}"));
                tables.fonts.insert(handle, family.clone());
                handle
            };
        control.push_str(&format!("\\f{handle}"));
    }
    let exact_scaled = |value: f32, scale: f32| -> Result<i32, super::DocumentError> {
        let scaled = value * scale;
        if !scaled.is_finite()
            || scaled.fract() != 0.0
            || scaled < i32::MIN as f32
            || scaled > i32::MAX as f32
        {
            return Err(UnsupportedFormatting);
        }
        Ok(scaled as i32)
    };
    if let Some(size) = properties.size {
        control.push_str(&format!("\\fs{}", exact_scaled(size, 2.0)?));
    }
    if let Some(weight) = properties.weight {
        control.push_str(match weight {
            400 => "\\b0",
            700 => "\\b",
            _ => return Err(UnsupportedFormatting),
        });
    }
    if let Some(slant) = properties.slant {
        control.push_str(match slant {
            FontSlant::Upright => "\\i0",
            FontSlant::Italic => "\\i",
            _ => return Err(UnsupportedFormatting),
        });
    }
    for (color, name) in [
        (properties.foreground, "cf"),
        (properties.background, "highlight"),
    ] {
        if let Some(color) = color {
            if name == "highlight"
                && color
                    == (Color {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 0.0,
                    })
                && tables.colors.first().map_or(true, Option::is_none)
            {
                control.push_str("\\highlight0");
                continue;
            }
            if name == "cf"
                && color
                    == (Color {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    })
                && tables.colors.first().map_or(true, Option::is_none)
            {
                control.push_str("\\cf0");
                continue;
            }
            if color.alpha != 1.0 {
                return Err(UnsupportedFormatting);
            }
            let handle = if let Some(handle) =
                tables.colors.iter().position(|entry| *entry == Some(color))
            {
                handle
            } else {
                let red = exact_scaled(color.red, 255.0)?;
                let green = exact_scaled(color.green, 255.0)?;
                let blue = exact_scaled(color.blue, 255.0)?;
                if [red, green, blue].iter().any(|v| !(0..=255).contains(v)) {
                    return Err(UnsupportedFormatting);
                }
                if tables.colors.is_empty() {
                    tables.colors.push(None);
                    color_additions.push(';');
                }
                let handle = tables.colors.len();
                tables.colors.push(Some(color));
                color_additions.push_str(&format!("\\red{red}\\green{green}\\blue{blue};"));
                handle
            };
            control.push_str(&format!("\\{name}{handle}"));
        }
    }
    if let Some(value) = properties.underline {
        control.push_str(if value { "\\ul" } else { "\\ulnone" });
    }
    if let Some(value) = properties.strikethrough {
        control.push_str(if value { "\\strike" } else { "\\strike0" });
    }
    if let Some(language) = &properties.language {
        let code = match language.as_str() {
            "en-US" => 1033,
            "en-GB" => 2057,
            "de-DE" => 1031,
            "fr-FR" => 1036,
            "ja-JP" => 1041,
            "es-ES" => 3082,
            _ => return Err(UnsupportedFormatting),
        };
        control.push_str(&format!("\\lang{code}"));
    }
    if let Some(direction) = properties.direction {
        control.push_str(match direction {
            WritingDirection::LeftToRight => "\\ltrch",
            WritingDirection::RightToLeft => "\\rtlch",
            _ => return Err(UnsupportedFormatting),
        });
    }
    if properties
        .open_type_features
        .as_ref()
        .is_some_and(|f| !f.is_empty())
    {
        return Err(UnsupportedFormatting);
    }
    if let Some(spacing) = properties.letter_spacing {
        control.push_str(&format!("\\expndtw{}", exact_scaled(spacing, 20.0)?));
    }
    if let Some(shift) = properties.baseline_shift {
        control.push_str(&format!("\\up{}", exact_scaled(shift, 2.0)?));
    }
    let group_end = |destination: &str| {
        header_group(&tokens, destination).map(|group| {
            builder
                .source_range(tokens[group.end - 1].range.clone())
                .start
        })
    };
    let header_insertion = root_control(&tokens, "rtf")
        .map(|index| builder.source_range(tokens[index].range.clone()).end)
        .ok_or(UnsupportedFormatting)?;
    let mut new_tables = String::new();
    for (destination, addition) in [("fonttbl", font_additions), ("colortbl", color_additions)] {
        if addition.is_empty() {
            continue;
        }
        if let Some(at) = group_end(destination) {
            patches.push((at..at, addition));
        } else {
            new_tables.push_str(&format!("{{\\{destination}{addition}}}"));
        }
    }
    if !new_tables.is_empty() {
        patches.push((header_insertion..header_insertion, new_tables));
    }
    patches.push((range.start..range.start, format!("{{{control} ")));
    patches.push((range.end..range.end, "}".into()));
    Ok(patches)
}

pub(super) fn empty_insertion_point(input: &NormalizedText) -> Option<usize> {
    let builder = Builder::new(input, Revision(0));
    tokenize(input).iter().rev().find_map(|token| {
        matches!(token.kind, Kind::Close).then(|| builder.source_range(token.range.clone()).start)
    })
}
