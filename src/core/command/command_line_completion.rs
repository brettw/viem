//! Filename completion belongs to the prompt, never to document undo history.
use super::*;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FilenameCompletion {
    original: Box<CommandLineBuffer>,
    start: usize,
    candidates: Arc<[String]>,
    selected: usize,
}

impl CommandLineBuffer {
    /// Ordinary edits accept the suggestion. A slash typed at the end of an
    /// active directory suggestion accepts its existing separator instead.
    pub(super) fn accept_completion_input(&mut self, range: Range<usize>, text: &str) -> bool {
        let Some(completion) = self.completion.take() else {
            return false;
        };
        text == "/"
            && range == (self.cursor..self.cursor)
            && self
                .selection_anchor
                .map_or(true, |anchor| anchor == self.cursor)
            && completion.candidates[completion.selected].ends_with('/')
    }

    fn complete_filename(&mut self, reverse: bool) {
        let completion = if let Some(mut completion) = self.completion.take() {
            let count = completion.candidates.len();
            completion.selected = if reverse {
                (completion.selected + count - 1) % count
            } else {
                (completion.selected + 1) % count
            };
            completion
        } else {
            if self
                .selection_anchor
                .is_some_and(|anchor| anchor != self.cursor)
            {
                return;
            }
            let Ok(cwd) = std::env::current_dir() else {
                return;
            };
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from);
            // Missing, unreadable, or changing directories simply offer no
            // completion. The original command remains editable/executable.
            let Ok(Some(mut found)) =
                filename_candidates::candidates(&self.input, self.cursor, &cwd, home.as_deref())
            else {
                return;
            };
            found.values.retain(|value| {
                let mut candidate = self.input.clone();
                candidate.replace_range(found.range.clone(), value);
                is_grapheme_boundary(&candidate, found.range.start + value.len())
            });
            if found.values.is_empty() {
                return;
            }
            FilenameCompletion {
                original: Box::new(self.clone()),
                start: found.range.start,
                selected: if reverse { found.values.len() - 1 } else { 0 },
                candidates: found.values.into(),
            }
        };
        let candidate = &completion.candidates[completion.selected];
        self.input
            .replace_range(completion.start..self.cursor, candidate);
        self.cursor = completion.start + candidate.len();
        self.selection_anchor = None;
        self.detach_from_history();
        self.completion = Some(completion);
    }
}

impl CommandInterpreter {
    /// Run before ordinary key handling so acceptance and its normal action
    /// share one event in both planned and compatibility command paths.
    pub(super) fn handle_filename_completion_key(&mut self, key: Key) -> Option<CommandOutput> {
        let state = self.command_line_state.as_mut()?;
        match key {
            Key::Tab | Key::BackTab if state.kind == CommandLineKind::Ex => {
                state.buffer.complete_filename(key == Key::BackTab);
                Some(CommandOutput::pending())
            }
            Key::Ctrl('e' | 'E') if state.buffer.completion.is_some() => {
                let completion = state.buffer.completion.take().expect("active completion");
                state.buffer = *completion.original;
                Some(CommandOutput::pending())
            }
            Key::Ctrl('y' | 'Y') if state.buffer.completion.is_some() => {
                state.buffer.completion = None;
                Some(CommandOutput::pending())
            }
            Key::Char(_) => None, // Insertion handles the directory slash.
            _ => {
                state.buffer.completion = None;
                None
            }
        }
    }
}
