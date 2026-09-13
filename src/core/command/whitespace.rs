//! View-local list/listchars overrides inherit live application defaults.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleWhitespaceSetting {
    pub default_enabled: bool,
    pub default_listchars: String,
    pub enabled: Option<bool>,
    pub listchars: Option<String>,
}
impl Default for VisibleWhitespaceSetting {
    fn default() -> Self {
        let defaults = crate::layout::VisibleWhitespaceOptions::default();
        Self {
            default_enabled: defaults.enabled,
            default_listchars: defaults.listchars,
            enabled: None,
            listchars: None,
        }
    }
}
impl VisibleWhitespaceSetting {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(self.default_enabled)
    }
    pub fn listchars(&self) -> &str {
        self.listchars.as_deref().unwrap_or(&self.default_listchars)
    }
}
impl CommandInterpreter {
    pub fn set_visible_whitespace(&mut self, enabled: bool) {
        self.visible_whitespace.enabled = Some(enabled);
    }
    pub fn set_visible_whitespace_defaults(&mut self, enabled: bool, listchars: &str) {
        self.visible_whitespace.default_enabled = enabled;
        self.visible_whitespace.default_listchars = listchars.into();
    }
    pub fn visible_whitespace(&self) -> &VisibleWhitespaceSetting {
        &self.visible_whitespace
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
        for character in text.chars() {
            commands
                .handle(document, InputEvent::key(character))
                .unwrap();
        }
        let output = commands
            .handle(document, InputEvent::Key(Key::Enter))
            .unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
    }
    #[test]
    fn ex_listchars_escaped_space_is_a_space_filler_and_numeric_escapes_survive() {
        let mut document = Document::new("\t");
        let mut commands = CommandInterpreter::new();
        set(&mut commands, &mut document, ":set lcs=tab:>\\ ");
        let parsed =
            crate::layout::ListChars::parse(commands.visible_whitespace().listchars()).unwrap();
        assert_eq!(parsed.get("tab"), Some(['>', ' '].as_slice()));
        set(
            &mut commands,
            &mut document,
            r":set lcs=tab:\x3e\u002d,trail:\U0000002a",
        );
        let parsed =
            crate::layout::ListChars::parse(commands.visible_whitespace().listchars()).unwrap();
        assert_eq!(parsed.get("tab"), Some(['>', '-'].as_slice()));
        assert_eq!(parsed.get("trail"), Some(['*'].as_slice()));
        let escaped = format!(":set lcs=tab:>{0}{0}", char::from(92));
        set(&mut commands, &mut document, &escaped);
        let parsed =
            crate::layout::ListChars::parse(commands.visible_whitespace().listchars()).unwrap();
        assert_eq!(parsed.get("tab"), Some(['>', char::from(92)].as_slice()));
        assert_eq!(document.text(), "\t");
    }
    #[test]
    fn listchars_modifiers_match_native_vim_comma_option_behavior() {
        // Oracle: Vim -Nu NONE, assigning each initial value to &listchars,
        // then executing the specified :set modifier and reading &listchars.
        for (initial, modifier, expected) in [
            ("space:.,trail:*,eol:$", "-=space:.,eol:$", "space:.,trail:*,eol:$"),
            ("space:.,trail:*,eol:$", "-=space:.,trail:*", "eol:$"),
            ("space:.,trail:*,eol:$", "-=trail:*,eol:$", "space:."),
            ("space:.,trail:*,eol:$", "+=trail:*,eol:$", "space:.,trail:*,eol:$"),
            ("space:.,trail:*,eol:$", "+=space:.,eol:$", "space:.,trail:*,eol:$,space:.,eol:$"),
            ("space:.,", "+=trail:*", "space:.,trail:*"),
            ("space:.,", "+=space:.", "space:.,"),
            ("space:.", "+=space:.", "space:."),
            ("space:.,trail:*,", "-=trail:*", "space:.,"),
            ("space:.,trail:*,eol:$", "-=trail:*", "space:.,eol:$"),
            ("space:.,space:.,trail:*", "-=space:.", "space:.,trail:*"),
            ("space:.,trail:*", "^=space:.", "space:.,trail:*"),
            ("space:.,trail:*", "-=pace:.", "space:.,trail:*"),
        ] {
            let mut document = Document::new(" ");
            let mut commands = CommandInterpreter::new();
            set(&mut commands, &mut document, &format!(":set lcs={initial}"));
            set(&mut commands, &mut document, &format!(":set lcs{modifier}"));
            assert_eq!(commands.visible_whitespace().listchars(), expected, "{initial} {modifier}");
            assert_eq!(document.text(), " ");
        }
    }

}
