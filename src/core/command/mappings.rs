//! Vim mapping commands, portable key notation, and bounded input expansion.
use super::*;
use crate::vimscript::keys::Notation;
use crate::vimscript::Value;
use std::sync::Arc;

const NORMAL: u8 = 1;
const VISUAL: u8 = 2;
const OPERATOR: u8 = 4;
const INSERT: u8 = 8;
const COMMAND_LINE: u8 = 16;
const SELECT: u8 = 32;
/// `:lmap`: recorded, but only active with 'iminsert', which Viem lacks.
const LANGUAGE: u8 = 64;
/// `:tmap`: recorded; Viem has no terminal windows.
const TERMINAL: u8 = 128;
const MODES: [u8; 8] = [NORMAL, VISUAL, SELECT, OPERATOR, INSERT, COMMAND_LINE, LANGUAGE, TERMINAL];

/// Buffer-wide mappings and abbreviations.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyMappings(Arc<Vec<Mapping>>, pub(crate) super::abbreviations::Abbreviations);

/// One mode's mapping. A command naming several modes stores one entry per
/// mode, so a later command can replace or remove one mode independently.
#[derive(Clone, Debug)]
struct Mapping {
    mode: u8,
    lhs: Vec<Key>,
    rhs: Vec<Key>,
    /// The sides as written, after `<Leader>` expansion, for listings and
    /// `maparg()`; an `<expr>` mapping evaluates `rhs_text`.
    lhs_text: String,
    rhs_text: String,
    recursive: bool,
    /// `<script>`: only script-local (`<SNR>`) mappings are remapped.
    script: bool,
    expr: bool,
    silent: bool,
    nowait: bool,
    buffer: bool,
    sid: usize,
    select_as_visual: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MappingReplayPlan {
    pub events: Vec<(InputEvent, bool)>,
    pub restore_select: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Map,
    NoRemap,
    Unmap,
    Clear,
}

#[derive(Default)]
struct Attributes {
    silent: bool,
    nowait: bool,
    unique: bool,
    buffer: bool,
    expr: bool,
    script: bool,
}

/// Modes and operation of a full mapping command name.
fn command_kind(name: &str, bang: bool) -> Option<(u8, Operation)> {
    let (prefix, base) = match name {
        "map" | "noremap" | "unmap" | "mapclear" => ("", name),
        _ => name.split_at_checked(1)?,
    };
    let operation = match base {
        "map" => Operation::Map,
        "noremap" => Operation::NoRemap,
        "unmap" => Operation::Unmap,
        "mapclear" => Operation::Clear,
        _ => return None,
    };
    let mode = match (prefix, bang) {
        ("", false) => NORMAL | VISUAL | SELECT | OPERATOR,
        ("", true) => INSERT | COMMAND_LINE,
        (_, true) => return None,
        ("n", _) => NORMAL,
        ("v", _) => VISUAL | SELECT,
        ("x", _) => VISUAL,
        ("s", _) => SELECT,
        ("o", _) => OPERATOR,
        ("i", _) => INSERT,
        ("c", _) => COMMAND_LINE,
        ("l", _) => LANGUAGE,
        ("t", _) => TERMINAL,
        _ => return None,
    };
    Some((mode, operation))
}

/// Whether `name` (a full Vim command name) is a mapping command.
pub(crate) fn is_mapping_command(name: &str) -> bool {
    let (name, bang) = match name.strip_suffix('!') {
        Some(name) => (name, true),
        None => (name, false),
    };
    command_kind(name, bang).is_some()
}

/// `<buffer>`, `<silent>`, ... before the left-hand side, in any order.
fn parse_attributes(mut text: &str) -> (Attributes, &str) {
    let mut attributes = Attributes::default();
    // `<special>` only matters with 'cpoptions' flag `<`, which Viem lacks.
    let mut special = false;
    loop {
        text = text.trim_start_matches([' ', '\t']);
        let Some(end) = text.starts_with('<').then(|| text.find('>')).flatten() else { break };
        let flag = match text[1..end].to_ascii_lowercase().as_str() {
            "buffer" => &mut attributes.buffer,
            "silent" => &mut attributes.silent,
            "nowait" => &mut attributes.nowait,
            "unique" => &mut attributes.unique,
            "expr" => &mut attributes.expr,
            "script" => &mut attributes.script,
            "special" => &mut special,
            _ => break,
        };
        *flag = true;
        text = &text[end + 1..];
    }
    (attributes, text)
}

/// Split `{lhs} {rhs}`: the left side ends at white space not quoted by
/// CTRL-V; the right side keeps trailing white space.
fn split_sides(text: &str) -> (&str, &str) {
    let mut chars = text.char_indices();
    while let Some((at, character)) = chars.next() {
        if character == '\u{16}' {
            chars.next();
        } else if character == ' ' || character == '\t' {
            return (&text[..at], text[at..].trim_start_matches([' ', '\t']));
        }
    }
    (text, "")
}

/// Keys for mapping text: Vim notation (`<C-W>`, `<Leader>` already
/// expanded, `<Plug>`, `<M-x>`, ...), CTRL-V quoting, and key codes from
/// double-quoted script strings. Unknown `<names>` stay literal, as in Vim.
pub(crate) fn parse_map_keys(text: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(character) = rest.chars().next() {
        if character == '\u{16}' {
            rest = &rest[1..];
            if let Some(quoted) = rest.chars().next() {
                keys.push(Key::Char(quoted));
                rest = &rest[quoted.len_utf8()..];
            }
            continue;
        }
        if character == '<' {
            if let Some(end) = rest.find('>') {
                let name = &rest[1..end];
                if let Some(bracketed) = bracket_keys(name) {
                    keys.extend(bracketed);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let code = character as u32;
        if (crate::vimscript::keys::SPECIAL_BASE..crate::vimscript::keys::SPECIAL_BASE + 0x2000).contains(&code) {
            // Key codes from "\<...>" strings, with any modifier prefix.
            let mut length = character.len_utf8();
            if code >= crate::vimscript::keys::MODIFIER_BASE {
                length += rest[length..].chars().next().map_or(0, char::len_utf8);
            }
            keys.extend(crate::vimscript::keys::decode(&rest[..length]).iter().filter_map(notation_key));
            rest = &rest[length..];
            continue;
        }
        keys.extend(notation_key(&Notation { key: crate::vimscript::keys::NotationKey::Char(character), modifiers: 0 }));
        rest = &rest[character.len_utf8()..];
    }
    keys
}

/// The keys one `<name>` spells, or None when it is not key notation.
fn bracket_keys(name: &str) -> Option<Vec<Key>> {
    if name.eq_ignore_ascii_case("nop") {
        return Some(Vec::new());
    }
    if let Some(key) = viem_bracket_key(name) {
        return Some(vec![key]);
    }
    let notation = crate::vimscript::keys::parse_notation(name)?;
    Some(notation_key(&notation).into_iter().collect())
}

/// Viem's native navigation spellings and their established meanings.
fn viem_bracket_key(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    let simple = match lower.as_str() {
        "lt" => Some(Key::Char('<')),
        "space" => Some(Key::Char(' ')),
        "bar" => Some(Key::Char('|')),
        "bslash" => Some(Key::Char('\\')),
        "esc" => Some(Key::Escape),
        "cr" | "enter" | "return" => Some(Key::Enter),
        "tab" => Some(Key::Tab),
        "s-tab" => Some(Key::BackTab),
        "s-cr" | "s-enter" => Some(Key::ShiftEnter),
        "a-cr" | "a-enter" | "m-cr" | "m-enter" => Some(Key::AltEnter),
        "bs" | "backspace" => Some(Key::Backspace),
        "del" | "delete" => Some(Key::Delete),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "c-left" => Some(Key::WordLeft),
        "c-right" => Some(Key::WordRight),
        "paragraphstart" | "c-up" | "a-up" | "m-up" => Some(Key::ParagraphStart),
        "paragraphend" | "a-down" | "m-down" => Some(Key::ParagraphEnd),
        "nextparagraph" | "c-down" => Some(Key::NextParagraph),
        "c-home" => Some(Key::DocumentStart),
        "c-end" => Some(Key::DocumentEnd),
        _ => None,
    };
    if simple.is_some() {
        return simple;
    }
    let mut modifiers = 0;
    let mut tail = lower.as_str();
    while tail.len() > 2 && tail.as_bytes()[1] == b'-' {
        modifiers |= match tail.as_bytes()[0] {
            b's' => 1,
            b'c' => 2,
            b'a' | b'm' => 4,
            b'd' => 8,
            _ => break,
        };
        tail = &tail[2..];
    }
    if modifiers == 0 {
        return None;
    }
    let navigation = match tail {
        "left" => Some(if modifiers & 6 != 0 { NavigationKey::WordLeft } else { NavigationKey::Left }),
        "right" => Some(if modifiers & 6 != 0 { NavigationKey::WordRight } else { NavigationKey::Right }),
        "up" => Some(if modifiers & 6 != 0 { NavigationKey::ParagraphStart } else { NavigationKey::Up }),
        "down" => Some(if modifiers & 4 != 0 {
            NavigationKey::ParagraphEnd
        } else if modifiers & 2 != 0 {
            NavigationKey::NextParagraph
        } else {
            NavigationKey::Down
        }),
        "paragraphstart" => Some(NavigationKey::ParagraphStart),
        "paragraphend" => Some(NavigationKey::ParagraphEnd),
        "nextparagraph" => Some(NavigationKey::NextParagraph),
        "home" => Some(if modifiers & 2 != 0 { NavigationKey::DocumentStart } else { NavigationKey::Home }),
        "end" => Some(if modifiers & 2 != 0 { NavigationKey::DocumentEnd } else { NavigationKey::End }),
        "pageup" => Some(NavigationKey::PageUp),
        "pagedown" => Some(NavigationKey::PageDown),
        _ => None,
    };
    if let Some(key) = navigation {
        return Some(canonical_mapping_key(Key::ModifiedNavigation { key, modifiers }));
    }
    if modifiers == 2 && tail.chars().count() == 1 {
        return Some(canonical_mapping_key(Key::Ctrl(tail.chars().next().unwrap())));
    }
    if modifiers == 1 && tail.chars().count() == 1 {
        return Some(Key::Char(tail.chars().next().unwrap().to_ascii_uppercase()));
    }
    None
}

/// Strict Vim notation for callers that must reject unknown key names.
pub fn parse_key_notation(input: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = input;
    while let Some(character) = rest.chars().next() {
        if character != '<' {
            keys.push(Key::Char(character));
            rest = &rest[character.len_utf8()..];
            continue;
        }
        let end = rest.find('>').ok_or("unterminated key notation; use <lt> for a literal <")?;
        let name = &rest[1..end];
        keys.extend(bracket_keys(name).ok_or_else(|| format!("unsupported key notation <{name}>"))?);
        rest = &rest[end + 1..];
    }
    Ok(keys)
}

/// Vim's mode characters for a listing line.
fn mode_chars(mode: u8) -> String {
    let mut chars = String::new();
    if mode & (INSERT | COMMAND_LINE) == INSERT | COMMAND_LINE {
        chars.push('!');
    } else if mode & INSERT != 0 {
        chars.push('i');
    } else if mode & LANGUAGE != 0 {
        chars.push('l');
    } else if mode & COMMAND_LINE != 0 {
        chars.push('c');
    } else if mode & (NORMAL | VISUAL | SELECT | OPERATOR) == NORMAL | VISUAL | SELECT | OPERATOR {
        chars.push(' ');
    } else {
        if mode & NORMAL != 0 {
            chars.push('n');
        }
        if mode & OPERATOR != 0 {
            chars.push('o');
        }
        if mode & TERMINAL != 0 {
            chars.push('t');
        }
        if mode & (VISUAL | SELECT) == VISUAL | SELECT {
            chars.push('v');
        } else {
            if mode & VISUAL != 0 {
                chars.push('x');
            }
            if mode & SELECT != 0 {
                chars.push('s');
            }
        }
    }
    chars
}

impl Mapping {
    fn same_definition(&self, other: &Mapping) -> bool {
        self.lhs == other.lhs
            && self.rhs_text == other.rhs_text
            && self.recursive == other.recursive
            && self.script == other.script
            && self.expr == other.expr
            && self.silent == other.silent
            && self.buffer == other.buffer
    }
}

impl KeyMappings {
    /// Execute raw mapping command text (`nnoremap <silent> x y`). None
    /// means the text is not a mapping command.
    pub(crate) fn execute(&mut self, input: &str) -> Option<Result<(), String>> {
        let input = input.trim_start();
        let input = input.strip_prefix(':').unwrap_or(input).trim_start();
        let name_end = input.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(input.len());
        let full = crate::vimscript::excmds::lookup(&input[..name_end])?;
        let rest = &input[name_end..];
        let (bang, rest) = match rest.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        command_kind(full, bang)?;
        Some(self.execute_command(full, bang, rest.trim_start_matches([' ', '\t']), 0).map(|_| ()))
    }

    /// Execute a mapping command given its full name. `Ok(Some(text))` is
    /// a listing to show.
    pub(crate) fn execute_command(&mut self, name: &str, bang: bool, argument: &str, sid: usize) -> Result<Option<String>, String> {
        let (mode, operation) = command_kind(name, bang).ok_or_else(|| format!("E492: Not an editor command: {name}"))?;
        let (attributes, rest) = parse_attributes(argument);
        if operation == Operation::Clear {
            if !rest.trim().is_empty() {
                return Err(format!("E474: Invalid argument: {}", rest.trim()));
            }
            Arc::make_mut(&mut self.0).retain(|mapping| mapping.mode & mode == 0 || mapping.buffer != attributes.buffer);
            return Ok(None);
        }
        let (lhs_text, rhs_text) = split_sides(rest);
        if lhs_text.is_empty() {
            return Ok(Some(self.listing(mode, None, attributes.buffer)));
        }
        let lhs = parse_map_keys(lhs_text);
        if operation == Operation::Unmap {
            if !rhs_text.trim().is_empty() {
                return Err(format!("E474: Invalid argument: {}", rest.trim()));
            }
            let before = self.0.len();
            Arc::make_mut(&mut self.0).retain(|mapping| mapping.mode & mode == 0 || mapping.buffer != attributes.buffer || mapping.lhs != lhs);
            if self.0.len() == before {
                return Err("E31: No such mapping".into());
            }
            return Ok(None);
        }
        if rhs_text.is_empty() {
            return Ok(Some(self.listing(mode, Some(&lhs), attributes.buffer)));
        }
        if lhs.is_empty() {
            return Err(format!("E474: Invalid argument: {lhs_text}"));
        }
        if attributes.unique && self.0.iter().any(|mapping| mapping.mode & mode != 0 && mapping.lhs == lhs && (attributes.buffer || !mapping.buffer)) {
            return Err(format!("E227: Mapping already exists for {lhs_text}"));
        }
        let rhs = if attributes.expr { Vec::new() } else { parse_map_keys(rhs_text) };
        let mappings = Arc::make_mut(&mut self.0);
        for selected in MODES.into_iter().filter(|selected| selected & mode != 0) {
            mappings.retain(|mapping| mapping.mode != selected || mapping.buffer != attributes.buffer || mapping.lhs != lhs);
            mappings.push(Mapping {
                mode: selected,
                lhs: lhs.clone(),
                rhs: rhs.clone(),
                lhs_text: lhs_text.to_owned(),
                rhs_text: rhs_text.to_owned(),
                recursive: operation == Operation::Map,
                script: attributes.script,
                expr: attributes.expr,
                silent: attributes.silent,
                nowait: attributes.nowait,
                buffer: attributes.buffer,
                sid,
                select_as_visual: mode & SELECT != 0 && mode & VISUAL != 0,
            });
        }
        Ok(None)
    }

    /// Vim's `:map` listing for `mode`, optionally only left sides starting
    /// with `prefix`. Identical definitions in several modes share a line.
    fn listing(&self, mode: u8, prefix: Option<&[Key]>, buffer_only: bool) -> String {
        let mut groups: Vec<(u8, &Mapping)> = Vec::new();
        for mapping in self.0.iter().filter(|mapping| {
            mapping.mode & mode != 0
                && prefix.is_none_or(|prefix| mapping.lhs.starts_with(prefix))
                && (!buffer_only || mapping.buffer)
        }) {
            match groups.iter_mut().find(|(_, first)| first.same_definition(mapping)) {
                Some((modes, _)) => *modes |= mapping.mode,
                None => groups.push((mapping.mode, mapping)),
            }
        }
        if groups.is_empty() {
            return "No mapping found".into();
        }
        groups
            .iter()
            .map(|(modes, mapping)| {
                let mut line = format!("{:<3}{}", mode_chars(*modes), mapping.lhs_text);
                let width = 3 + 12;
                while line.chars().count() < width {
                    line.push(' ');
                }
                if !line.ends_with(' ') {
                    line.push(' ');
                }
                line.push(if !mapping.recursive && !mapping.script { '*' } else if mapping.script { '&' } else { ' ' });
                line.push(if mapping.buffer { '@' } else { ' ' });
                line.push_str(if mapping.rhs_text.is_empty() { "<Nop>" } else { &mapping.rhs_text });
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `maparg()`, `mapcheck()` and `hasmapto()`.
    pub(crate) fn script_query(&self, function: &str, args: &[Value]) -> Value {
        let text = |index: usize| args.get(index).and_then(|value| value.to_text().ok()).unwrap_or_default();
        let flag = |index: usize| args.get(index).and_then(|value| value.to_number().ok()).unwrap_or(0) != 0;
        if flag(2) { return self.1.script_query(function, args); }
        let mode = mode_bits(&text(1));
        match function {
            "hasmapto" => {
                let wanted = parse_map_keys(&text(0));
                let found = self.0.iter().any(|mapping| {
                    mapping.mode & mode != 0 && !wanted.is_empty() && mapping.rhs.windows(wanted.len()).any(|window| window == wanted)
                });
                Value::Number(found as i64)
            }
            _ => {
                let lhs = parse_map_keys(&text(0));
                let check = function == "mapcheck";
                let found = self.0.iter().filter(|mapping| {
                    mapping.mode & mode != 0
                        && if check {
                            !lhs.is_empty() && (mapping.lhs.starts_with(&lhs) || lhs.starts_with(&mapping.lhs))
                        } else {
                            mapping.lhs == lhs
                        }
                }).max_by_key(|mapping| mapping.buffer);
                if function == "maparg" && flag(3) {
                    let Some(mapping) = found else { return Value::Dict(crate::vimscript::Dict::new()) };
                    let modes = self
                        .0
                        .iter()
                        .filter(|other| other.same_definition(mapping))
                        .fold(0, |modes, other| modes | other.mode);
                    return Value::Dict(crate::vimscript::Dict::from_pairs([
                        ("lhs".into(), Value::str(&mapping.lhs_text)),
                        ("lhsraw".into(), Value::str(&mapping.lhs_text)),
                        ("rhs".into(), Value::str(&mapping.rhs_text)),
                        ("silent".into(), Value::Number(mapping.silent as i64)),
                        ("noremap".into(), Value::Number(!mapping.recursive as i64)),
                        ("script".into(), Value::Number(mapping.script as i64)),
                        ("expr".into(), Value::Number(mapping.expr as i64)),
                        ("buffer".into(), Value::Number(mapping.buffer as i64)),
                        ("nowait".into(), Value::Number(mapping.nowait as i64)),
                        ("mode".into(), Value::String(mode_chars(modes).into())),
                        ("sid".into(), Value::Number(mapping.sid as i64)),
                        ("scriptversion".into(), Value::Number(1)),
                        ("lnum".into(), Value::Number(0)),
                        ("abbr".into(), Value::Number(0)),
                        ("mode_bits".into(), Value::Number(modes as i64)),
                    ]));
                }
                Value::String(found.map(|mapping| mapping.rhs_text.clone()).unwrap_or_default().into())
            }
        }
    }
}

/// The input key for one key of Vim notation. Keys Viem's input vocabulary
/// cannot express (`<Plug>`, `<SNR>`, `<Insert>`, keypad and mouse keys)
/// become private-use characters, so they still match mapping sequences
/// exactly; `<Ignore>` and `<Nul>` produce no key.
pub(crate) fn notation_key(notation: &crate::vimscript::keys::Notation) -> Option<Key> {
    use crate::vimscript::keys::{NotationKey, SpecialKey, CONTROL, SHIFT, SPECIAL_BASE};
    let modifiers = notation.modifiers;
    let key = match notation.key {
        NotationKey::Char(character) => {
            let key = match character {
                '\r' => Key::Enter,
                '\x1b' => Key::Escape,
                '\t' if modifiers & SHIFT != 0 => Key::BackTab,
                '\t' => Key::Tab,
                '\x7f' => Key::Backspace,
                '\n' => Key::Ctrl('j'),
                c if (c as u32) < 0x20 => Key::Ctrl(((c as u8) + b'`') as char),
                c if modifiers & CONTROL != 0 => Key::Ctrl(c.to_ascii_lowercase()),
                c => Key::Char(c),
            };
            return Some(canonical_mapping_key(key));
        }
        NotationKey::Special(special) => special,
    };
    let navigation = |plain: Key, navigation: NavigationKey| {
        if modifiers == 0 { plain } else { canonical_mapping_key(Key::ModifiedNavigation { key: navigation, modifiers }) }
    };
    Some(match key {
        SpecialKey::Up => navigation(Key::Up, NavigationKey::Up),
        SpecialKey::Down => navigation(Key::Down, NavigationKey::Down),
        SpecialKey::Left => navigation(Key::Left, NavigationKey::Left),
        SpecialKey::Right => navigation(Key::Right, NavigationKey::Right),
        SpecialKey::Home | SpecialKey::KeypadHome => navigation(Key::Home, NavigationKey::Home),
        SpecialKey::End | SpecialKey::KeypadEnd => navigation(Key::End, NavigationKey::End),
        SpecialKey::PageUp | SpecialKey::KeypadPageUp => navigation(Key::PageUp, NavigationKey::PageUp),
        SpecialKey::PageDown | SpecialKey::KeypadPageDown => navigation(Key::PageDown, NavigationKey::PageDown),
        SpecialKey::Delete | SpecialKey::KeypadDelete => Key::Delete,
        SpecialKey::Backspace => Key::Backspace,
        SpecialKey::KeypadEnter => Key::Enter,
        SpecialKey::KeypadPlus => Key::Char('+'),
        SpecialKey::KeypadMinus => Key::Char('-'),
        SpecialKey::KeypadMultiply => Key::Char('*'),
        SpecialKey::KeypadDivide => Key::Char('/'),
        SpecialKey::KeypadPoint => Key::Char('.'),
        SpecialKey::Keypad(digit) => Key::Char((b'0' + digit) as char),
        SpecialKey::F(number) if number <= 35 => Key::Function { number, modifiers },
        SpecialKey::Ignore | SpecialKey::Nul => return None,
        other => Key::Char(char::from_u32(SPECIAL_BASE + other.code()).expect("special keys are private-use scalars")),
    })
}

/// Input keys for a script string with Vim key codes (`feedkeys()`).
pub(crate) fn script_keys(bytes: &[u8]) -> Vec<Key> {
    crate::vimscript::keys::decode(&String::from_utf8_lossy(bytes)).iter().filter_map(notation_key).collect()
}

/// Mode bits for a `maparg()` mode string ("" is Normal, Visual, Select
/// and Operator-pending, as for `:map`).
fn mode_bits(mode: &str) -> u8 {
    if mode.is_empty() {
        return NORMAL | VISUAL | SELECT | OPERATOR;
    }
    mode.chars().fold(0, |bits, flag| {
        bits | match flag {
            'n' => NORMAL,
            'v' => VISUAL | SELECT,
            'x' => VISUAL,
            's' => SELECT,
            'o' => OPERATOR,
            'i' => INSERT,
            'c' => COMMAND_LINE,
            'l' => LANGUAGE,
            't' => TERMINAL,
            '!' => INSERT | COMMAND_LINE,
            _ => 0,
        }
    })
}

/// Vim's display spelling of a key sequence (`maparg()`, `:map` listing).
pub(crate) fn render_key_notation(keys: &[Key]) -> String {
    let mut text = String::new();
    for key in keys {
        match key {
            Key::Char('<') => text.push_str("<lt>"),
            Key::Char(' ') => text.push_str("<Space>"),
            Key::Char(character) => text.push(*character),
            Key::Escape => text.push_str("<Esc>"),
            Key::Enter => text.push_str("<CR>"),
            Key::Tab => text.push_str("<Tab>"),
            Key::BackTab => text.push_str("<S-Tab>"),
            Key::Backspace => text.push_str("<BS>"),
            Key::Delete => text.push_str("<Del>"),
            Key::Left => text.push_str("<Left>"),
            Key::Right => text.push_str("<Right>"),
            Key::Up => text.push_str("<Up>"),
            Key::Down => text.push_str("<Down>"),
            Key::Home => text.push_str("<Home>"),
            Key::End => text.push_str("<End>"),
            Key::PageUp => text.push_str("<PageUp>"),
            Key::PageDown => text.push_str("<PageDown>"),
            Key::Ctrl(character) => text.push_str(&format!("<C-{}>", character.to_ascii_uppercase())),
            Key::Function { number, modifiers: 0 } => text.push_str(&format!("<F{number}>")),
            other => text.push_str(&format!("{other:?}")),
        }
    }
    text
}

fn canonical_mapping_key(key: Key) -> Key {
    match key {
        Key::ModifiedNavigation { key: key @ (NavigationKey::WordLeft | NavigationKey::WordRight | NavigationKey::ParagraphStart | NavigationKey::ParagraphEnd | NavigationKey::NextParagraph | NavigationKey::DocumentStart | NavigationKey::DocumentEnd), modifiers: 2 | 4 | 8 } => key.key(),
        Key::Ctrl('i' | 'I') => Key::Tab,
        Key::Ctrl('m' | 'M') => Key::Enter,
        Key::Ctrl('[') => Key::Escape,
        Key::Ctrl(character) => Key::Ctrl(character.to_ascii_lowercase()),
        key => key,
    }
}

fn input_key(event: &InputEvent) -> Option<Key> {
    match event {
        InputEvent::Key(key) => Some(canonical_mapping_key(*key)),
        InputEvent::Text(text) if text.chars().count() == 1 => text.chars().next().map(Key::Char),
        _ => None,
    }
}

impl CommandInterpreter {
    fn mapping_mode(&self) -> Option<u8> {
        if self.substitute_confirmation.is_some()
            || self.mapping_suppressed
            || self.cmd_key.is_some()
            || self.literal_input_pending()
            || self.register_pending
            || self.select_register_pending
            || self.command_line_register_pending()
            || self.insert_control_g_pending()
            || matches!(
                self.pending,
                Pending::ReplaceCharacter { .. }
                    | Pending::ReplaceVisual
                    | Pending::ReplaceVisualBlock
                    | Pending::Find { .. }
                    | Pending::OperatorFind { .. }
                    | Pending::Bracket { .. }
                    | Pending::OperatorBracket { .. }
                    | Pending::SetMark
                    | Pending::JumpMark { .. }
                    | Pending::OperatorMark { .. }
                    | Pending::MacroRecord
                    | Pending::MacroPlay { .. }
            )
        {
            return None;
        }
        if self.is_native_selection() { return None; }
        Some(match self.mode {
            Mode::Normal if matches!(self.pending, Pending::Operator(_)) => OPERATOR,
            Mode::Normal => NORMAL,
            Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock => if self.is_select_mode() { SELECT } else { VISUAL },
            Mode::Insert | Mode::Replace => INSERT,
            Mode::CommandLine => COMMAND_LINE,
        })
    }

    pub(crate) fn mapping_applies(&self, event: &InputEvent) -> bool {
        if matches!(event, InputEvent::Key(Key::PasteClipboard | Key::PasteClipboardMatchingStyle)) { return false; }
        let Some(mode) = self.mapping_mode() else {
            return false;
        };
        let Some(key) = input_key(event) else {
            return false;
        };
        if self.recording.is_some()
            && self.mode == Mode::Normal
            && self.pending == Pending::None
            && self.mapping_pending.is_empty()
            && key == Key::Char('q')
        {
            return false;
        }
        !self.mapping_pending.is_empty()
            || self
                .mappings
                .0
                .iter()
                .any(|mapping| mapping.mode == mode && mapping.lhs.first() == Some(&key))
    }

    pub(crate) fn has_pending_mapping(&self) -> bool {
        !self.mapping_pending.is_empty()
    }
    pub(crate) fn set_mapping_suppressed(&mut self, suppressed: bool) -> bool {
        std::mem::replace(&mut self.mapping_suppressed, suppressed)
    }

    pub(crate) fn flush_mapping(&mut self, document: &mut Document) -> Option<MappingReplayPlan> {
        if self.mapping_pending.is_empty() {
            return None;
        }
        let mode = self.mapping_mode()?;
        Some(
            self.resolve_mapping(document, mode, true)
                .expect("flushing always releases the prefix"),
        )
    }

    fn resolve_mapping(&mut self, document: &mut Document, mode: u8, flush: bool) -> Option<MappingReplayPlan> {
        let pending = &self.mapping_pending;
        let complete_nowait = self.mappings.0.iter().any(|mapping| mapping.mode == mode && mapping.nowait && *pending == mapping.lhs);
        if !flush
            && !complete_nowait
            && self.mappings.0.iter().any(|mapping| {
                mapping.mode == mode
                    && mapping.lhs.len() > pending.len()
                    && mapping.lhs.starts_with(pending)
            })
        {
            return None;
        }
        let selected = self
            .mappings
            .0
            .iter()
            .filter(|mapping| mapping.mode == mode && pending.starts_with(&mapping.lhs))
            .max_by_key(|mapping| (mapping.buffer, mapping.lhs.len()))
            .cloned();
        let restore_select = self.is_select_mode() && selected.as_ref().is_some_and(|mapping| mapping.select_as_visual);
        let mut events = Vec::new();
        let pending = self.mapping_pending.clone();
        let consumed = if let Some(mapping) = selected {
            let rhs = if mapping.expr { self.evaluate_expr_mapping(document, &mapping) } else { mapping.rhs.clone() };
            // `<script>` remaps only script-local mappings, which begin with
            // the private `<SNR>` key.
            let snr = Key::Char(char::from_u32(crate::vimscript::keys::SPECIAL_BASE + crate::vimscript::keys::SpecialKey::Snr.code()).expect("private-use key"));
            let remap = mapping.recursive || (mapping.script && rhs.contains(&snr));
            // Vim does not remap a RHS's initial complete copy of its own LHS.
            let self_prefix = rhs.starts_with(&mapping.lhs);
            events.extend(rhs.iter().enumerate().map(|(index, key)| {
                (InputEvent::Key(*key), remap && !(self_prefix && index < mapping.lhs.len()))
            }));
            mapping.lhs.len()
        } else {
            events.push((InputEvent::Key(pending[0]), false));
            1
        };
        events.extend(
            pending[consumed..]
                .iter()
                .map(|key| (InputEvent::Key(*key), true)),
        );
        self.mapping_pending.clear();
        if restore_select { self.selection_behavior = SelectionBehavior::Visual; }
        Some(MappingReplayPlan { events, restore_select })
    }

    pub(super) fn handle_mapping(
        &mut self,
        document: &mut Document,
        event: &InputEvent,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if !self.mapping_applies(event) {
            return Ok(None);
        }
        let key = input_key(event).expect("mapping input has a key");
        if matches!(key, Key::Escape | Key::Ctrl('c')) && !self.mapping_pending.is_empty() {
            self.mapping_pending.clear();
            return Ok(None);
        }
        self.record_event(event);
        let mode = self.mapping_mode().unwrap();
        self.mapping_pending.push(key);
        let Some(plan) = self.resolve_mapping(document, mode, false) else {
            return Ok(Some(CommandOutput::pending()));
        };
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::Mapping(plan));
            return Ok(Some(CommandOutput::complete()));
        }
        if !self.begin_replay_frame() {
            return Ok(Some(CommandOutput {
                status: CommandStatus::Error("mapping recursion limit reached".into()),
                ..CommandOutput::complete()
            }));
        }
        let old_depth = document.edit_group_depth();
        document.begin_edit_group();
        let result = (|| {
            let mut output = CommandOutput::complete();
            for (event, remap) in plan.events {
                let old = self.set_mapping_suppressed(!remap);
                let next = self.handle(document, event);
                self.set_mapping_suppressed(old);
                let next = next?;
                let stop = command_status_stops_compound(&next.status);
                output.merge(next);
                if stop {
                    break;
                }
            }
            Ok(Some(output))
        })();
        document.restore_edit_group_depth(old_depth + 1);
        document.end_edit_group();
        self.end_replay_frame();
        if plan.restore_select { self.finish_select_mapping(); }
        result
    }
}

impl CommandInterpreter {
    /// Evaluate an `<expr>` mapping's right side with text locked; its
    /// string value is used as typed keys. Errors produce no keys.
    fn evaluate_expr_mapping(&mut self, document: &mut Document, mapping: &Mapping) -> Vec<Key> {
        let saved_count = self.script_count.replace(self.count.unwrap_or(0));
        let mut runtime = crate::vimscript::Runtime::new(self.script.clone());
        let mut host = super::scripting::InterpreterHost::new(self, document);
        host.textlock = true;
        let value = runtime.eval_expression(&mut host, mapping.rhs_text.as_bytes(), mapping.sid);
        drop(host);
        self.script_count = saved_count;
        match value.and_then(|value| value.to_bytes().map_err(crate::vimscript::Fail::Error)) {
            Ok(bytes) => script_keys(&bytes),
            Err(_) => Vec::new(),
        }
    }

    /// Whether `event` belongs to a `<Cmd>` mapping command line.
    pub(super) fn cmd_key_applies(&self, event: &InputEvent) -> bool {
        self.cmd_key.is_some()
            || matches!(event, InputEvent::Key(Key::Char(character)) if [crate::vimscript::keys::SpecialKey::Cmd, crate::vimscript::keys::SpecialKey::ScriptCmd]
                .iter()
                .any(|key| char::from_u32(crate::vimscript::keys::SPECIAL_BASE + key.code()) == Some(*character)))
    }

    /// `<Cmd>…<CR>` in a mapping runs the command line without changing
    /// mode and without remapping its text.
    pub(super) fn handle_cmd_key(&mut self, document: &mut Document, event: &InputEvent) -> Option<CommandOutput> {
        let special = |key: crate::vimscript::keys::SpecialKey| {
            char::from_u32(crate::vimscript::keys::SPECIAL_BASE + key.code()).expect("private-use key")
        };
        let starts = |character: &char| {
            *character == special(crate::vimscript::keys::SpecialKey::Cmd)
                || *character == special(crate::vimscript::keys::SpecialKey::ScriptCmd)
        };
        let Some(text) = self.cmd_key.as_mut() else {
            return match event {
                InputEvent::Key(Key::Char(character)) if starts(character) => {
                    self.cmd_key = Some(String::new());
                    Some(CommandOutput::pending())
                }
                _ => None,
            };
        };
        match event {
            InputEvent::Key(Key::Enter) => {
                let text = std::mem::take(text);
                self.cmd_key = None;
                Some(self.submit_ex_line(document, &text, None))
            }
            InputEvent::Key(Key::Escape) => {
                self.cmd_key = None;
                Some(CommandOutput { status: CommandStatus::Error("E1255: <Cmd> mapping must end with <CR>".into()), ..CommandOutput::complete() })
            }
            InputEvent::Key(Key::Char(character)) => {
                text.push(*character);
                Some(CommandOutput::pending())
            }
            InputEvent::Text(more) => {
                text.push_str(more);
                Some(CommandOutput::pending())
            }
            InputEvent::Key(other) => {
                text.push_str(&render_key_notation(&[*other]));
                Some(CommandOutput::pending())
            }
        }
    }
}

pub(crate) fn empty_mapping_output() -> CommandOutput {
    CommandOutput::complete()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{Core, CoreEvent, ViewId};
    use crate::layout::MockTextMeasurementProvider;

    fn setup(source: &str, startup: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
        let mut core = Core::new(Document::new(source));
        assert_eq!(core.initialize_startup(startup), vec![]);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        (core, view)
    }
    fn keys(
        core: &mut Core<MockTextMeasurementProvider>,
        view: ViewId,
        text: &str,
    ) -> CommandOutput {
        let mut result = CommandOutput::complete();
        for key in parse_key_notation(text).unwrap() {
            result = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap()
                .command
                .unwrap();
            assert!(
                !command_status_stops_compound(&result.status),
                "{text}: {:?}",
                result.status
            );
        }
        result
    }
    #[test]
    fn requested_yank_mapping_preserves_count_register_and_hard_line_extent() {
        let (mut core, view) = setup("alpha beta\ngamma delta\nlast", "map Y y$\nnmap a d");
        keys(&mut core, view, "l\"a2Y");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            "lpha beta\ngamma delta"
        );
        assert_eq!(core.document().text(), "alpha beta\ngamma delta\nlast");
    }
    #[test]
    fn function_mapping_prefills_prompt_and_explicit_return_executes_split() {
        let (mut core, view) = setup("abc", "map <C-F2> :sp\nmap <S-C-F3> :sp<CR>");
        keys(&mut core, view, "<C-F2>");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::CommandLine);
        assert_eq!(
            core.command_state(view).unwrap().command_line().unwrap(),
            "sp"
        );
        keys(&mut core, view, "<Esc>");
        let output = core
            .handle_with_layout(
                view,
                CoreEvent::Input(InputEvent::Key(Key::Function {
                    number: 3,
                    modifiers: 3,
                })),
            )
            .unwrap()
            .command
            .unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.ex_outcome.unwrap().frontend_requests.len() > 0);
    }
    #[test]
    fn modes_recursive_nonrecursive_operator_and_literal_operands() {
        let (mut core, view) = setup(
            "abc def ghi",
            "map Q w\nnmap Z Q\nnnoremap X Q\nomap K e\nnmap b l",
        );
        keys(&mut core, view, "Z");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, "0fb");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        keys(&mut core, view, "0dK");
        assert_eq!(core.document().text(), " def ghi");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "abc def ghi");
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::key('X')))
            .unwrap()
            .command
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Unsupported(_)));
    }
    #[test]
    fn mapped_edit_is_one_undo_group_and_dot_keeps_last_semantic_edit() {
        let (mut core, view) = setup("abcdef", "nnoremap Q xx");
        keys(&mut core, view, "Q");
        assert_eq!(core.document().text(), "cdef");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "abcdef");
        keys(&mut core, view, "<C-r>.");
        assert_eq!(core.document().text(), "def");
    }
    #[test]
    fn multi_key_mismatch_and_timeout_choose_longest_complete_prefix() {
        let (mut core, view) = setup("abc def ghi", "nmap g l\nnmap gg w\nnmap jk w");
        keys(&mut core, view, "g");
        assert!(core.has_pending_mapping(view));
        core.flush_mapping_prefix(view).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        keys(&mut core, view, "gg");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, "gl");
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
        keys(&mut core, view, "j<Esc>");
        assert!(!core.has_pending_mapping(view));
    }
    #[test]
    fn mappings_apply_across_views_and_interactive_unmap_is_shared() {
        let (mut core, a) = setup("abc def", "map Y y$");
        let b = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        keys(&mut core, b, "lY");
        assert_eq!(
            core.command_state(b).unwrap().register('"').unwrap().text,
            "bc def"
        );
        keys(&mut core, a, ":unmap Y<CR>");
        keys(&mut core, b, "Y");
        assert_eq!(
            core.command_state(b).unwrap().register('"').unwrap().kind,
            RegisterKind::Linewise
        );
    }
    #[test]
    fn recursive_cycles_terminate_and_self_prefix_keeps_vim_behavior() {
        let (mut core, view) = setup("abc def", "map Q Z\nmap Z Q\nmap Y Y");
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::key('Q')))
            .unwrap()
            .command
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Error(_)));
        keys(&mut core, view, "Y");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('"')
                .unwrap()
                .kind,
            RegisterKind::Linewise
        );
    }
    #[test]
    fn insert_mapping_and_literal_function_quotation() {
        let (mut core, view) = setup("abc", "imap jj <Esc>\nimap <C-F2> word");
        keys(&mut core, view, "i<C-F2>jj");
        assert_eq!(core.document().text(), "wordabc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        keys(&mut core, view, "i<C-v><C-F2><Esc>");
        assert!(core.document().text().contains("<C-F2>"));
    }
    #[test]
    fn startup_reports_line_errors_continues_and_cannot_change_source() {
        let mut core: Core<MockTextMeasurementProvider> = Core::new(Document::new("abc\ndef"));
        let source = core.document().source_bytes();
        let revision = core.document().revision();
        // As in Vim, unknown key notation maps its literal characters.
        let diagnostics = core.initialize_startup("\" comment\nset nowrap ignorecase\nset ff=dos\nnormal dd\nmap <Unknown> x\nmap Y y$\nset tw=72");
        assert_eq!(
            diagnostics.iter().map(|d| d.line).collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        let a = core.add_view(MockTextMeasurementProvider::new(), 100., 100.);
        let b = core.add_view(MockTextMeasurementProvider::new(), 100., 100.);
        assert!(!core.layout(a).unwrap().wrap());
        assert!(!core.layout(b).unwrap().wrap());
        assert_eq!(core.initialize_startup("map Y dd").len(), 1);
        keys(&mut core, a, "Y");
        assert_eq!(core.document().source_bytes(), source);
    }
    #[test]
    fn headless_mapping_and_key_notation_parser() {
        let mut document = Document::new("abc def");
        let mut commands = CommandInterpreter::new();
        assert!(commands
            .configure_startup(&mut document, "map Y y$")
            .is_empty());
        commands
            .handle(&mut document, InputEvent::key('Y'))
            .unwrap();
        assert_eq!(commands.register('"').unwrap().text, "abc def");
        assert_eq!(
            parse_key_notation("<D-M-C-S-F35><lt><Space><Nop>").unwrap(),
            [
                Key::Function {
                    number: 35,
                    modifiers: 15
                },
                Key::Char('<'),
                Key::Char(' ')
            ]
        );
        assert!(parse_key_notation("<F38>").is_err());
        assert!(parse_key_notation("<Unknown>").is_err());
        assert_eq!(parse_map_keys("<Unknown>"), "<Unknown>".chars().map(Key::Char).collect::<Vec<_>>());
    }
    #[test]
    fn visual_mapping_and_macro_recording_share_semantic_replay() {
        let (mut core, view) = setup("abcdef", "nmap Q x\nvmap Q e");
        keys(&mut core, view, "qaQq");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            "Q"
        );
        keys(&mut core, view, "@a");
        assert_eq!(core.document().text(), "cdef");
        keys(&mut core, view, "vQd");
        assert_eq!(core.document().text(), "");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "cdef");
    }
    #[test]
    fn normal_bang_bypasses_mappings_and_plain_normal_expands_them() {
        let (mut core, view) = setup("abc def ghi", "nmap l w");
        keys(&mut core, view, ":normal l<CR>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, ":normal! l<CR>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
    }
    #[test]
    fn startup_protects_file_policies_case_insensitively_and_atomically() {
        let (mut core, _) = setup("abc\ndef", "");
        let mut fresh: Core<MockTextMeasurementProvider> = Core::new(Document::new("abc\ndef"));
        let before = fresh.document().source_bytes();
        let diagnostics = fresh.initialize_startup("set FF=dos\nset FILEFORMAT&\nset tw=19 ff=dos\nset FFS=mac\nset fileformats=dos\nset tw=31");
        assert_eq!(
            diagnostics.iter().map(|item| item.line).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(fresh.document().source_bytes(), before);
        assert_eq!(fresh.text_width().effective(), 31);
        assert_eq!(core.initialize_startup("set FF=dos").len(), 1);
    }
    #[test]
    fn insert_mapping_keeps_the_entire_typing_session_in_one_undo_group() {
        let (mut core, view) = setup("", "imap <F2> MID\nimap jj <Esc>");
        keys(&mut core, view, "ibefore<F2>afterjj");
        assert_eq!(core.document().text(), "beforeMIDafter");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "");
    }

    #[test]
    fn control_aliases_match_the_same_physical_keys() {
        let (mut core, view) = setup("abc def", "nmap <C-M> l\nnmap <C-I> l\nnmap <C-[> l");
        keys(&mut core, view, "<CR><Tab><Esc>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        for key in [Key::Ctrl('m'), Key::Ctrl('I'), Key::Ctrl('[')] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
    }
    #[test]
    fn supplied_layout_mapping_requires_coordinator_and_validates_snapshot_first() {
        use crate::layout::{LayoutEngine, ViewLayout};
        let mut document = Document::new("abc def");
        let mut commands = CommandInterpreter::new();
        assert!(commands
            .configure_startup(&mut document, "map Q gj")
            .is_empty());
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(40., 200.);
        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap().clone();
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0., 200.).unwrap());
        let output = commands
            .handle_with_layout(&mut document, InputEvent::key('Q'), &mut context)
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Unsupported(_)));
        let mut other = Document::new("other");
        let output = commands
            .handle_with_layout(&mut other, InputEvent::key('Q'), &mut context)
            .unwrap();
        assert_eq!(
            output.status,
            CommandStatus::Error("stale layout context".into())
        );
    }
}
