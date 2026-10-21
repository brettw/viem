//! Keypad identity survives mapping lookup and macro recording; fallback is ordinary input.
use super::*;
use crate::vimscript::keys::SpecialKey;
pub fn valid_keypad_code(code: u8) -> bool {
    matches!(code, 20..=31 | 40..=49)
}
pub(super) fn fallback(code: u8, modifiers: u8) -> Key {
    let key = match code {
        20 => Key::Home,
        21 => Key::End,
        22 => Key::PageUp,
        23 => Key::PageDown,
        24 => Key::Char(
            char::from_u32(crate::vimscript::keys::SPECIAL_BASE + SpecialKey::Insert.code())
                .unwrap(),
        ),
        25 => Key::Delete,
        26 => Key::Char('+'),
        27 => Key::Char('-'),
        28 => Key::Char('*'),
        29 => Key::Char('/'),
        30 if modifiers == 4 => Key::AltEnter,
        30 if modifiers & 1 != 0 => Key::ShiftEnter,
        30 => Key::Enter,
        31 => Key::Char('.'),
        40..=49 => Key::Char(char::from(b'0' + code - 40)),
        _ => unreachable!("validated keypad code"),
    };
    if let Some(key) = NavigationKey::from_key(key) {
        if modifiers != 0 {
            return Key::ModifiedNavigation { key, modifiers };
        }
    }
    key
}
pub(super) fn notation(code: u8, modifiers: u8) -> String {
    let name = SpecialKey::from_code(code as u32)
        .expect("validated keypad code")
        .name();
    let mut text = String::from("<");
    for (bit, name) in [(2, "C-"), (1, "S-"), (4, "A-"), (8, "D-")] {
        if modifiers & bit != 0 {
            text.push_str(name);
        }
    }
    text.push_str(&name);
    text.push('>');
    text
}
