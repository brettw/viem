//! Prompt-local register input. Expansions never dispatch document commands.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegisterInsertion { Typed, Literal }

impl CommandInterpreter {
    /// The next native key/text event selects a register, not prompt text.
    pub fn command_line_register_pending(&self) -> bool {
        self.mode == Mode::CommandLine && self.command_line_state.as_ref()
            .is_some_and(|state| state.register_insert.is_some())
    }

    pub(super) fn handle_command_line_register_text(&mut self, document: &Document, input: &str) -> CommandOutput {
        let mut output = CommandOutput::pending();
        for (at, character) in input.char_indices() {
            if !self.command_line_register_pending() {
                self.insert_command_line_register_literal(&input[at..]);
                break;
            }
            output = self.handle_command_line_register_key(document, Key::Char(character))
                .expect("pending register consumes its selector");
            if command_status_stops_compound(&output.status) { break; }
        }
        output
    }

    pub(super) fn handle_command_line_register_key(&mut self, document: &Document, key: Key) -> Option<CommandOutput> {
        self.command_line_register_key_at_depth(document, key, 0)
    }

    fn command_line_register_key_at_depth(&mut self, document: &Document, key: Key, depth: usize) -> Option<CommandOutput> {
        let state = self.command_line_state.as_mut()?;
        let pending = state.register_insert.take();
        let insertion = match (pending, key) {
            (None, Key::Ctrl('r' | 'R')) => {
                state.buffer.completion = None;
                state.register_insert = Some(RegisterInsertion::Typed);
                return Some(CommandOutput::pending());
            }
            (None, _) => return None,
            (Some(RegisterInsertion::Typed), Key::Ctrl('r' | 'R' | 'o' | 'O')) => {
                state.register_insert = Some(RegisterInsertion::Literal);
                return Some(CommandOutput::pending());
            }
            (Some(_), Key::Escape | Key::Ctrl('c' | 'C' | '[')) => return Some(CommandOutput::pending()),
            (Some(insertion), _) => insertion,
        };
        if depth >= 64 {
            return Some(CommandOutput::unsupported("command-line register expansion nesting limit"));
        }
        let text = match self.command_line_register_value(document, key) {
            Ok(text) => text,
            Err(output) => return Some(output),
        };
        Some(self.insert_command_line_register(document, &text, insertion, depth))
    }

    #[allow(clippy::result_large_err)]
    fn command_line_register_value(&self, document: &Document, key: Key) -> Result<RegisterValue, CommandOutput> {
        let failure = |message: &str| CommandOutput { status: CommandStatus::Error(message.into()), ..CommandOutput::complete() };
        let literal = |text: String| RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
            .expect("prompt objects contain literal text without structural breaks");
        match key {
            Key::Char('/') => self.last_search.as_ref().map(|(_, text)| literal(text.clone())).ok_or_else(|| failure("no previous search")),
            Key::Char(':') => self.last_command_line.clone().map(literal).ok_or_else(|| failure("no previous command line")),
            Key::Char(name) if is_valid_register(name) => self.require_register_value(document, name),
            Key::Ctrl('w' | 'W' | 'a' | 'A' | 'l' | 'L') => {
                let lines = document.hard_line_snapshot();
                let line = lines.line_at_offset(self.cursor).map_err(|error| failure(&error.to_string()))?;
                let range = line.content_range();
                let text = lines.slice_utf8(range.clone()).map_err(|error| failure(&error.to_string()))?;
                if matches!(key, Key::Ctrl('l' | 'L')) { return Ok(literal(text)); }
                let offset = self.cursor - range.start;
                let keyword = matches!(key, Key::Ctrl('w' | 'W'));
                let selected = text.grapheme_indices(true).find(|(at, grapheme)| *at >= offset && if keyword {
                    grapheme.chars().next().is_some_and(|character| character.is_alphanumeric() || character == '_')
                } else { !grapheme.chars().all(char::is_whitespace) });
                let Some((at, grapheme)) = selected else { return Err(failure("no word under cursor")); };
                if keyword {
                    let range = keyword_range(&text, at).expect("selected keyword grapheme");
                    return Ok(literal(text[range].into()));
                }
                let current = at..at + grapheme.len();
                let mut start = current.start;
                let mut end = current.end;
                for (at, grapheme) in text[..start].grapheme_indices(true).rev() {
                    if grapheme.chars().all(char::is_whitespace) { break; }
                    start = at;
                }
                for (at, grapheme) in text[end..].grapheme_indices(true) {
                    if grapheme.chars().all(char::is_whitespace) { break; }
                    end = current.end + at + grapheme.len();
                }
                Ok(literal(text[start..end].into()))
            }
            Key::Char('=') => Err(CommandOutput::unsupported("expression registers are not supported")),
            _ => Err(CommandOutput::unsupported(format!("command-line register selector {key:?}"))),
        }
    }

    fn insert_command_line_register_literal(&mut self, text: &str) {
        if text.is_empty() { return; }
        let Some(state) = self.command_line_state.as_mut() else { return; };
        state.buffer.completion = None;
        state.buffer.insert(text);
        if !is_grapheme_boundary(&state.buffer.input, state.buffer.cursor) {
            state.buffer.cursor = next_grapheme_boundary(&state.buffer.input, state.buffer.cursor)
                .unwrap_or(state.buffer.input.len());
        }
    }

    fn insert_command_line_register(&mut self, document: &Document, input: &RegisterValue, insertion: RegisterInsertion, depth: usize) -> CommandOutput {
        // Vim represents register line separators as literal CR in a prompt,
        // including the final separator of a linewise register.
        let text: String = input.text.char_indices().map(|(at, character)| {
            if character == '\n' && (input.kind == RegisterKind::Blockwise || input.hard_break_offsets().binary_search(&at).is_ok()) {
                '\r'
            } else { character }
        }).collect();
        if insertion == RegisterInsertion::Literal {
            self.insert_command_line_register_literal(&text);
            return CommandOutput::pending();
        }
        for character in text.chars() {
            let key = match character {
                '\u{8}' | '\u{7f}' => Key::Backspace,
                '\u{1}'..='\u{1a}' => Key::Ctrl(char::from(character as u8 + b'a' - 1)),
                '\u{1b}' => Key::Escape,
                other => Key::Char(other),
            };
            if self.command_line_register_pending() || matches!(key, Key::Ctrl('r')) {
                let output = self.command_line_register_key_at_depth(document, key, depth + 1)
                    .expect("register input is pending");
                if command_status_stops_compound(&output.status) { return output; }
            } else if matches!(key, Key::Backspace | Key::Ctrl('b' | 'e' | 'w' | 'u' | 'p' | 'n')) {
                // These are prompt-only editing operations. Prompt terminators,
                // tabs, and other controls remain literal register contents.
                let output = self.try_handle_controller_only_command_line_key(document, key)
                    .expect("prompt editing never submits a command");
                if command_status_stops_compound(&output.status) { return output; }
            } else {
                self.insert_command_line_register_literal(&character.to_string());
            }
        }
        CommandOutput::pending()
    }
}

#[cfg(test)]
mod tests;
