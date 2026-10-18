//! Vim key notation (`<C-W>`, `<Esc>`, `<F5>`, `<Plug>`, ...) shared by
//! double-quoted strings ("\<CR>") and the command layer's mappings.
//!
//! Keys without a character are carried inside strings as code points in
//! Supplementary Private Use Area A, the way Vim uses its K_SPECIAL bytes:
//! `SPECIAL_BASE + key` for an unmodified special key, and a modifier prefix
//! `MODIFIER_BASE + bits` before a modified key or character.

/// Shift=1, Control=2, Alt/Meta=4, Command=8, matching the input vocabulary.
pub const SHIFT: u8 = 1;
pub const CONTROL: u8 = 2;
pub const ALT: u8 = 4;
pub const COMMAND: u8 = 8;

pub const SPECIAL_BASE: u32 = 0xF0000;
pub const MODIFIER_BASE: u32 = 0xF1F00;

/// Keys without a character representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpecialKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Backspace,
    Help,
    Undo,
    /// Function key 1..=37.
    F(u8),
    Plug,
    Snr,
    Cmd,
    ScriptCmd,
    Ignore,
    Nul,
    KeypadHome,
    KeypadEnd,
    KeypadPageUp,
    KeypadPageDown,
    KeypadInsert,
    KeypadDelete,
    KeypadPlus,
    KeypadMinus,
    KeypadMultiply,
    KeypadDivide,
    KeypadEnter,
    KeypadPoint,
    Keypad(u8),
    Mouse(u8),
    Paste,
}

const MOUSE_KEYS: &[&str] = &[
    "leftmouse", "leftdrag", "leftrelease", "middlemouse", "middledrag", "middlerelease",
    "rightmouse", "rightdrag", "rightrelease", "x1mouse", "x1drag", "x1release", "x2mouse",
    "x2drag", "x2release", "scrollwheelup", "scrollwheeldown", "scrollwheelleft",
    "scrollwheelright", "mousemove", "leftmousenm", "leftreleasenm", "mouse", "netmouse",
    "decmouse", "jsbmouse", "ptermmouse", "urxvtmouse", "sgrmouse", "sgrmouserelease",
];

impl SpecialKey {
    pub fn code(self) -> u32 {
        use SpecialKey::*;
        match self {
            Up => 1,
            Down => 2,
            Left => 3,
            Right => 4,
            Home => 5,
            End => 6,
            PageUp => 7,
            PageDown => 8,
            Insert => 9,
            Delete => 10,
            Backspace => 11,
            Help => 12,
            Undo => 13,
            Plug => 14,
            Snr => 15,
            Cmd => 16,
            ScriptCmd => 17,
            Ignore => 18,
            Nul => 19,
            KeypadHome => 20,
            KeypadEnd => 21,
            KeypadPageUp => 22,
            KeypadPageDown => 23,
            KeypadInsert => 24,
            KeypadDelete => 25,
            KeypadPlus => 26,
            KeypadMinus => 27,
            KeypadMultiply => 28,
            KeypadDivide => 29,
            KeypadEnter => 30,
            KeypadPoint => 31,
            Paste => 32,
            Keypad(n) => 40 + n as u32,
            F(n) => 60 + n as u32,
            Mouse(n) => 120 + n as u32,
        }
    }
    pub fn from_code(code: u32) -> Option<Self> {
        use SpecialKey::*;
        Some(match code {
            1 => Up,
            2 => Down,
            3 => Left,
            4 => Right,
            5 => Home,
            6 => End,
            7 => PageUp,
            8 => PageDown,
            9 => Insert,
            10 => Delete,
            11 => Backspace,
            12 => Help,
            13 => Undo,
            14 => Plug,
            15 => Snr,
            16 => Cmd,
            17 => ScriptCmd,
            18 => Ignore,
            19 => Nul,
            20 => KeypadHome,
            21 => KeypadEnd,
            22 => KeypadPageUp,
            23 => KeypadPageDown,
            24 => KeypadInsert,
            25 => KeypadDelete,
            26 => KeypadPlus,
            27 => KeypadMinus,
            28 => KeypadMultiply,
            29 => KeypadDivide,
            30 => KeypadEnter,
            31 => KeypadPoint,
            32 => Paste,
            40..=49 => Keypad((code - 40) as u8),
            61..=97 => F((code - 60) as u8),
            120..=160 => Mouse((code - 120) as u8),
            _ => return None,
        })
    }
    /// Vim's canonical spelling, for listing mappings.
    pub fn name(self) -> String {
        use SpecialKey::*;
        match self {
            Up => "Up".into(),
            Down => "Down".into(),
            Left => "Left".into(),
            Right => "Right".into(),
            Home => "Home".into(),
            End => "End".into(),
            PageUp => "PageUp".into(),
            PageDown => "PageDown".into(),
            Insert => "Insert".into(),
            Delete => "Del".into(),
            Backspace => "BS".into(),
            Help => "Help".into(),
            Undo => "Undo".into(),
            F(n) => format!("F{n}"),
            Plug => "Plug".into(),
            Snr => "SNR".into(),
            Cmd => "Cmd".into(),
            ScriptCmd => "ScriptCmd".into(),
            Ignore => "Ignore".into(),
            Nul => "Nul".into(),
            KeypadHome => "kHome".into(),
            KeypadEnd => "kEnd".into(),
            KeypadPageUp => "kPageUp".into(),
            KeypadPageDown => "kPageDown".into(),
            KeypadInsert => "kInsert".into(),
            KeypadDelete => "kDel".into(),
            KeypadPlus => "kPlus".into(),
            KeypadMinus => "kMinus".into(),
            KeypadMultiply => "kMultiply".into(),
            KeypadDivide => "kDivide".into(),
            KeypadEnter => "kEnter".into(),
            KeypadPoint => "kPoint".into(),
            Keypad(n) => format!("k{n}"),
            Mouse(n) => MOUSE_KEYS.get(n as usize).map_or_else(|| "Mouse".to_string(), |name| name.to_string()),
            Paste => "Paste".into(),
        }
    }
}

/// One key of Vim notation: a character or special key with modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotationKey {
    Char(char),
    Special(SpecialKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Notation {
    pub key: NotationKey,
    pub modifiers: u8,
}

fn special(name: &str) -> Option<NotationKey> {
    use SpecialKey::*;
    let key = match name {
        "nul" => Special(Nul),
        "bs" | "backspace" => Special(Backspace),
        "tab" => return Some(NotationKey::Char('\t')),
        "nl" | "newline" | "linefeed" | "lf" => return Some(NotationKey::Char('\n')),
        "ff" | "formfeed" => return Some(NotationKey::Char('\x0c')),
        "cr" | "return" | "enter" => return Some(NotationKey::Char('\r')),
        "esc" => return Some(NotationKey::Char('\x1b')),
        "space" => return Some(NotationKey::Char(' ')),
        "lt" => return Some(NotationKey::Char('<')),
        "bslash" => return Some(NotationKey::Char('\\')),
        "bar" => return Some(NotationKey::Char('|')),
        "del" | "delete" => Special(Delete),
        "csi" => return Some(NotationKey::Char('\u{9b}')),
        "eol" => return Some(NotationKey::Char('\r')),
        "up" => Special(Up),
        "down" => Special(Down),
        "left" => Special(Left),
        "right" => Special(Right),
        "help" => Special(Help),
        "undo" => Special(Undo),
        "insert" | "ins" => Special(Insert),
        "home" => Special(Home),
        "end" => Special(End),
        "pageup" => Special(PageUp),
        "pagedown" => Special(PageDown),
        "khome" => Special(KeypadHome),
        "kend" => Special(KeypadEnd),
        "kpageup" => Special(KeypadPageUp),
        "kpagedown" => Special(KeypadPageDown),
        "kinsert" => Special(KeypadInsert),
        "kdel" => Special(KeypadDelete),
        "kplus" => Special(KeypadPlus),
        "kminus" => Special(KeypadMinus),
        "kmultiply" => Special(KeypadMultiply),
        "kdivide" => Special(KeypadDivide),
        "kenter" => Special(KeypadEnter),
        "kpoint" => Special(KeypadPoint),
        "plug" => Special(Plug),
        "snr" => Special(Snr),
        "cmd" => Special(Cmd),
        "scriptcmd" => Special(ScriptCmd),
        "ignore" => Special(Ignore),
        "paste" | "pastestart" | "pasteend" => Special(Paste),
        "xup" => Special(Up),
        "xdown" => Special(Down),
        "xleft" => Special(Left),
        "xright" => Special(Right),
        "xhome" | "zhome" => Special(Home),
        "xend" | "zend" => Special(End),
        _ => {
            if let Some(number) = name.strip_prefix('k').and_then(|n| n.parse::<u8>().ok()).filter(|n| *n <= 9) {
                return Some(Special(Keypad(number)));
            }
            let function = name.strip_prefix("xf").or_else(|| name.strip_prefix('f'));
            if let Some(number) = function.and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=37).contains(n)) {
                return Some(Special(F(number)));
            }
            if let Some(index) = MOUSE_KEYS.iter().position(|mouse| *mouse == name) {
                return Some(Special(Mouse(index as u8)));
            }
            if let Some(code) = name.strip_prefix("char-") {
                let value = if let Some(hex) = code.strip_prefix("0x").or_else(|| code.strip_prefix("0X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else if code.len() > 1 && code.starts_with('0') {
                    u32::from_str_radix(&code[1..], 8).ok()
                } else {
                    code.parse().ok()
                };
                return value.and_then(char::from_u32).map(NotationKey::Char);
            }
            return None;
        }
    };
    Some(key)
}
use NotationKey::Special;

/// Parse the inside of `<...>` (e.g. "C-W", "S-F5", "Leader" is not handled
/// here). Returns None for unknown names.
pub fn parse_notation(name: &str) -> Option<Notation> {
    let lower = name.to_ascii_lowercase();
    let mut modifiers = 0u8;
    let mut rest = lower.as_str();
    let mut original = name;
    // Modifier prefixes: S- C- A- M- D- T-; a single trailing character may
    // itself be '-' ("<C-->").
    loop {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3 && bytes[1] == b'-' {
            let bit = match bytes[0] {
                b's' => SHIFT,
                b'c' => CONTROL,
                b'a' | b'm' => ALT,
                b'd' => COMMAND,
                b't' => ALT,
                _ => break,
            };
            modifiers |= bit;
            rest = &rest[2..];
            original = &original[2..];
            continue;
        }
        break;
    }
    if let Some(key) = special(rest) {
        return Some(Notation { key, modifiers });
    }
    // A single character after modifiers; keep its original case.
    let mut chars = original.chars();
    let c = chars.next()?;
    if chars.next().is_some() || modifiers == 0 {
        return None;
    }
    Some(Notation { key: NotationKey::Char(c), modifiers })
}

/// Bytes for "\<name>" inside a double-quoted string: control characters
/// for `<C-x>`, UTF-8 for characters, private-use code points for keys
/// without characters.
pub fn notation_bytes(name: &str) -> Option<Vec<u8>> {
    // "\<*C-w>" puts the modifier separately.
    let (separate, name) = match name.strip_prefix('*') {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    let notation = parse_notation(name)?;
    let mut out = String::new();
    match notation.key {
        NotationKey::Char(c) => {
            let mut modifiers = notation.modifiers;
            let mut c = c;
            if modifiers & CONTROL != 0 && !separate {
                if let Some(control) = control_char(c) {
                    c = control;
                    modifiers &= !CONTROL;
                }
            }
            if modifiers & SHIFT != 0 && c.is_ascii_alphabetic() {
                c = c.to_ascii_uppercase();
                modifiers &= !SHIFT;
            }
            if modifiers != 0 {
                out.push(char::from_u32(MODIFIER_BASE + modifiers as u32).unwrap());
            }
            out.push(c);
        }
        NotationKey::Special(key) => {
            if notation.modifiers != 0 {
                out.push(char::from_u32(MODIFIER_BASE + notation.modifiers as u32).unwrap());
            }
            out.push(char::from_u32(SPECIAL_BASE + key.code()).unwrap());
        }
    }
    Some(out.into_bytes())
}

/// The ASCII control character for Ctrl plus `c`, if there is one.
pub fn control_char(c: char) -> Option<char> {
    match c {
        '@' => None,
        'a'..='z' => Some(((c as u8) - b'a' + 1) as char),
        'A'..='Z' => Some(((c as u8) - b'A' + 1) as char),
        '[' => Some('\x1b'),
        '\\' => Some('\x1c'),
        ']' => Some('\x1d'),
        '^' => Some('\x1e'),
        '_' => Some('\x1f'),
        '?' => Some('\x7f'),
        _ => None,
    }
}

/// Decode a string into keys: private-use code points become special keys
/// or modifiers, everything else characters.
pub fn decode(text: &str) -> Vec<Notation> {
    let mut keys = Vec::new();
    let mut modifiers = 0u8;
    for c in text.chars() {
        let code = c as u32;
        if (MODIFIER_BASE..MODIFIER_BASE + 16).contains(&code) {
            modifiers = (code - MODIFIER_BASE) as u8;
            continue;
        }
        if (SPECIAL_BASE..SPECIAL_BASE + 0x1000).contains(&code) {
            if let Some(key) = SpecialKey::from_code(code - SPECIAL_BASE) {
                keys.push(Notation { key: NotationKey::Special(key), modifiers });
                modifiers = 0;
                continue;
            }
        }
        keys.push(Notation { key: NotationKey::Char(c), modifiers });
        modifiers = 0;
    }
    keys
}

/// Render keys in Vim notation for listings (`:map`, `maparg()`).
pub fn render(keys: &[Notation]) -> String {
    let mut out = String::new();
    for key in keys {
        let prefix = modifier_prefix(key.modifiers);
        match key.key {
            NotationKey::Special(special) => out.push_str(&format!("<{prefix}{}>", special.name())),
            NotationKey::Char(c) if key.modifiers != 0 => out.push_str(&format!("<{prefix}{c}>")),
            NotationKey::Char(c) => match c {
                '\r' => out.push_str("<CR>"),
                '\n' => out.push_str("<NL>"),
                '\x1b' => out.push_str("<Esc>"),
                '\t' => out.push_str("<Tab>"),
                ' ' => out.push_str("<Space>"),
                '\x7f' => out.push_str("<Del>"),
                '<' => out.push_str("<lt>"),
                c if (c as u32) < 0x20 => out.push_str(&format!("<C-{}>", ((c as u8) + b'@') as char)),
                c => out.push(c),
            },
        }
    }
    out
}

fn modifier_prefix(modifiers: u8) -> String {
    let mut prefix = String::new();
    if modifiers & SHIFT != 0 {
        prefix.push_str("S-");
    }
    if modifiers & CONTROL != 0 {
        prefix.push_str("C-");
    }
    if modifiers & ALT != 0 {
        prefix.push_str("M-");
    }
    if modifiers & COMMAND != 0 {
        prefix.push_str("D-");
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_notation_matches_vim_bytes_for_characters() {
        assert_eq!(notation_bytes("C-W"), Some(vec![0x17]));
        assert_eq!(notation_bytes("CR"), Some(b"\r".to_vec()));
        assert_eq!(notation_bytes("Esc"), Some(vec![0x1b]));
        assert_eq!(notation_bytes("lt"), Some(b"<".to_vec()));
        assert_eq!(notation_bytes("S-a"), Some(b"A".to_vec()));
        assert!(notation_bytes("NoSuchKey").is_none());
        let left = String::from_utf8(notation_bytes("Left").unwrap()).unwrap();
        assert_eq!(decode(&left), vec![Notation { key: NotationKey::Special(SpecialKey::Left), modifiers: 0 }]);
        let shifted = String::from_utf8(notation_bytes("S-F5").unwrap()).unwrap();
        assert_eq!(decode(&shifted), vec![Notation { key: NotationKey::Special(SpecialKey::F(5)), modifiers: SHIFT }]);
        let command = String::from_utf8(notation_bytes("D-s").unwrap()).unwrap();
        assert_eq!(decode(&command), vec![Notation { key: NotationKey::Char('s'), modifiers: COMMAND }]);
    }

    #[test]
    fn rendering_round_trips_common_keys() {
        let keys = decode("\x17\r ");
        assert_eq!(render(&keys), "<C-W><CR><Space>");
    }
}
