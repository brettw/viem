//! Git's checkout line-ending conversion, applied by Viem itself so a git
//! base matches the working-tree file without running git's filter pipeline
//! (clean and smudge filters such as Git LFS run external programs).

use std::borrow::Cow;

/// The line-ending attributes and settings that decide git's conversion.
/// Values use `git check-attr` spellings: `set`, `unset`, `unspecified`, or
/// an assigned value such as `auto`, `lf`, `crlf` or `input`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EolAttributes {
    pub text: Option<String>,
    pub eol: Option<String>,
    /// The legacy `crlf` attribute.
    pub crlf: Option<String>,
    pub core_autocrlf: Option<String>,
    pub core_eol: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Text {
    Set,
    Unset,
    Auto,
    Unspecified,
}

fn specified(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| *value != "unspecified")
}

fn is_true(value: Option<&str>) -> bool {
    matches!(value.map(str::to_ascii_lowercase).as_deref(), Some("true" | "yes" | "on" | "1"))
}

/// Convert a stored blob to the line endings git would write on checkout.
pub fn checkout_line_endings<'a>(blob: &'a [u8], attributes: &EolAttributes) -> Cow<'a, [u8]> {
    let mut text = match specified(&attributes.text) {
        Some("set") => Text::Set,
        Some("unset") => Text::Unset,
        Some("auto") => Text::Auto,
        Some(_) => Text::Unspecified,
        None => Text::Unspecified,
    };
    let mut eol = specified(&attributes.eol).map(str::to_owned);
    // The legacy attribute maps onto the current ones when they are absent.
    if text == Text::Unspecified {
        match specified(&attributes.crlf) {
            Some("set") => text = Text::Set,
            Some("unset") => text = Text::Unset,
            Some("input") => {
                text = Text::Set;
                eol.get_or_insert_with(|| "lf".into());
            }
            _ => {}
        }
    }
    let autocrlf = specified(&attributes.core_autocrlf).map(str::to_ascii_lowercase);
    if text == Text::Unspecified {
        if eol.is_some() {
            // An eol attribute alone marks the path as text.
            text = Text::Set;
        } else if autocrlf.as_deref().is_some_and(|value| is_true(Some(value))) {
            text = Text::Auto;
        } else {
            return Cow::Borrowed(blob);
        }
    }
    if text == Text::Unset {
        return Cow::Borrowed(blob);
    }
    let crlf = match eol.as_deref() {
        Some("crlf") => true,
        Some("lf") => false,
        _ => match autocrlf.as_deref() {
            Some("input") => false,
            value if is_true(value) => true,
            _ => match specified(&attributes.core_eol).map(str::to_ascii_lowercase).as_deref() {
                Some("crlf") => true,
                Some("lf") => false,
                _ => cfg!(windows),
            },
        },
    };
    if !crlf {
        return Cow::Borrowed(blob);
    }
    if text == Text::Auto && (blob.contains(&0) || blob.windows(2).any(|pair| pair == b"\r\n")) {
        // Auto leaves binary content and blobs already stored with CRLF alone.
        return Cow::Borrowed(blob);
    }
    let mut converted = Vec::with_capacity(blob.len() + blob.len() / 32);
    for (index, &byte) in blob.iter().enumerate() {
        if byte == b'\n' && (index == 0 || blob[index - 1] != b'\r') {
            converted.push(b'\r');
        }
        converted.push(byte);
    }
    Cow::Owned(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(text: &str, eol: &str, autocrlf: &str) -> EolAttributes {
        let value = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        EolAttributes { text: value(text), eol: value(eol), crlf: None, core_autocrlf: value(autocrlf), core_eol: None }
    }

    #[test]
    fn unspecified_text_without_autocrlf_is_unchanged() {
        assert_eq!(checkout_line_endings(b"a\nb\n", &attributes("unspecified", "unspecified", "")), &b"a\nb\n"[..]);
    }

    #[test]
    fn autocrlf_converts_text_but_not_binary_or_crlf_blobs() {
        let settings = attributes("unspecified", "unspecified", "true");
        assert_eq!(checkout_line_endings(b"a\nb\n", &settings), &b"a\r\nb\r\n"[..]);
        assert_eq!(checkout_line_endings(b"a\0\nb", &settings), &b"a\0\nb"[..]);
        assert_eq!(checkout_line_endings(b"a\r\nb\n", &settings), &b"a\r\nb\n"[..]);
        assert_eq!(checkout_line_endings(b"a\nb", &attributes("unspecified", "unspecified", "input")), &b"a\nb"[..]);
    }

    #[test]
    fn eol_attribute_wins_and_implies_text() {
        assert_eq!(checkout_line_endings(b"a\nb", &attributes("unspecified", "crlf", "false")), &b"a\r\nb"[..]);
        assert_eq!(checkout_line_endings(b"a\nb", &attributes("set", "lf", "true")), &b"a\nb"[..]);
        assert_eq!(checkout_line_endings(b"a\nb", &attributes("unset", "crlf", "true")), &b"a\nb"[..]);
    }

    #[test]
    fn legacy_crlf_attribute_maps_to_text() {
        let mut settings = attributes("unspecified", "unspecified", "true");
        settings.crlf = Some("unset".into());
        assert_eq!(checkout_line_endings(b"a\nb", &settings), &b"a\nb"[..]);
    }
}
