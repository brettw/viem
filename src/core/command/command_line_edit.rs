//! Snapshot-checked command prompt editing, independent of any native field editor.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLineSnapshot {
    pub kind: CommandLineKind,
    pub text: String,
    pub anchor: usize,
    pub active: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandLineEditAction {
    Select { anchor: usize, active: usize },
    Replace { range: Range<usize>, text: String },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLineEditRequest {
    pub document: DocumentId,
    pub revision: Revision,
    pub expected: CommandLineSnapshot,
    pub action: CommandLineEditAction,
}
impl CommandInterpreter {
    pub fn command_line_snapshot(&self) -> Option<CommandLineSnapshot> {
        let state = self.command_line_state.as_ref()?;
        Some(CommandLineSnapshot {
            kind: state.kind,
            text: state.buffer.input.clone(),
            anchor: state.buffer.selection_anchor.unwrap_or(state.buffer.cursor),
            active: state.buffer.cursor,
        })
    }
    pub(crate) fn edit_command_line(
        &mut self,
        document: &Document,
        request: CommandLineEditRequest,
    ) -> Result<CommandOutput, DocumentError> {
        if request.document != document.id() {
            return Err(DocumentError::WrongDocument);
        }
        if request.revision != document.revision()
            || self.command_line_snapshot().as_ref() != Some(&request.expected)
        {
            return Err(DocumentError::WrongSnapshot {
                expected: document.revision(),
                actual: request.revision,
            });
        }
        let input = &request.expected.text;
        let boundary = |offset| is_grapheme_boundary(input, offset);
        match request.action {
            CommandLineEditAction::Select { anchor, active } => {
                if !boundary(anchor) || !boundary(active) {
                    return Err(DocumentError::NotGraphemeBoundary(anchor.max(active)));
                }
                if anchor == active && self.recording.is_some() {
                    // A later Tab depends on this native caret move even when
                    // no native replacement occurs to record its position.
                    self.record_event(&InputEvent::Key(Key::Home));
                    for _ in input[..active].graphemes(true) {
                        self.record_event(&InputEvent::Key(Key::Right));
                    }
                }
                let buffer = &mut self
                    .command_line_state
                    .as_mut()
                    .expect("snapshot checked")
                    .buffer;
                buffer.completion = None;
                buffer.selection_anchor = (anchor != active).then_some(anchor);
                buffer.cursor = active;
            }
            CommandLineEditAction::Replace { range, text } => {
                if range.start > range.end || !boundary(range.start) || !boundary(range.end) {
                    return Err(DocumentError::NotGraphemeBoundary(range.end));
                }
                let accepted_separator = self
                    .command_line_state
                    .as_mut()
                    .expect("snapshot checked")
                    .buffer
                    .accept_completion_input(range.clone(), &text);
                if accepted_separator {
                    // Replay must accept the directory too; synthesizing caret
                    // movement first would consume completion before the slash.
                    self.record_event(&InputEvent::Text(text));
                    return Ok(CommandOutput::pending());
                }
                // Record portable editing keys, not a native range identity, so
                // replay reproduces the same edited prompt without stale offsets.
                if self.recording.is_some() && !input.is_empty() {
                    self.record_event(&InputEvent::Key(Key::Home));
                    for _ in input[..range.start].graphemes(true) {
                        self.record_event(&InputEvent::Key(Key::Right));
                    }
                }
                for _ in input[range.clone()].graphemes(true) {
                    self.record_event(&InputEvent::Key(Key::Delete));
                }
                if !text.is_empty() {
                    self.record_event(&InputEvent::Text(text.clone()));
                }
                let buffer = &mut self
                    .command_line_state
                    .as_mut()
                    .expect("snapshot checked")
                    .buffer;
                buffer.input.replace_range(range.clone(), &text);
                buffer.cursor = range.start + text.len();
                while !is_grapheme_boundary(&buffer.input, buffer.cursor) {
                    buffer.cursor = next_grapheme_boundary(&buffer.input, buffer.cursor)
                        .unwrap_or(buffer.input.len());
                }
                buffer.selection_anchor = None;
                buffer.detach_from_history();
            }
        }
        Ok(CommandOutput::pending())
    }
}
