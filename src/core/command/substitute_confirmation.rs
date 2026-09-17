//! Portable, snapshot-bound substitute confirmation. Decisions remain staged
//! until the interaction ends, so no edit group spans unrelated native events.
use super::ex_execute::SubstitutePreview;
use super::*;

#[derive(Clone, Debug)]
pub(super) struct SubstituteConfirmation {
    preview: SubstitutePreview,
    current: usize,
    approved: Vec<usize>,
    invocation: HistoryRestorationSnapshot,
}

impl CommandInterpreter {
    /// Plain status text, deliberately separate from an editable Ex prompt.
    pub fn substitute_confirmation_prompt(&self) -> Option<String> {
        let state = self.substitute_confirmation.as_ref()?;
        let replacement = &state.preview.edits[state.current].replacement;
        let visible: String = replacement
            .chars()
            .take(160)
            .map(|ch| {
                if ch.is_control() {
                    ch.escape_default().to_string()
                } else {
                    ch.to_string()
                }
            })
            .collect();
        Some(format!(
            "Replace with {visible}? (y/n/a/q/l) [{}/{}; {} approved]",
            state.current + 1,
            state.preview.matches,
            state.approved.len()
        ))
    }

    pub(crate) fn substitute_confirmation_history(
        &self,
        document: &Document,
    ) -> Option<&HistoryRestorationSnapshot> {
        let state = self.substitute_confirmation.as_ref()?;
        state
            .preview
            .is_current(document)
            .then_some(&state.invocation)
    }

    pub(super) fn substitute_confirmation_match(
        &self,
        document: &Document,
    ) -> Option<Range<usize>> {
        let state = self.substitute_confirmation.as_ref()?;
        state
            .preview
            .is_current(document)
            .then(|| state.preview.edits[state.current].range.clone())
    }

    pub(super) fn begin_substitute_confirmation(
        &mut self,
        document: &Document,
        preview: SubstitutePreview,
    ) -> CommandOutput {
        if self.global_replay_depth > 0 {
            return CommandOutput::unsupported(
                "Substitute confirmation is not available inside :global",
            );
        }
        if self.source_replay_depth > 0 {
            return CommandOutput {
                status: CommandStatus::ExError(ExCommandError::Execute(
                    ExExecuteError::UnsupportedCommand(
                        "Substitute confirmation is not available in sourced commands".into(),
                    ),
                )),
                ..CommandOutput::complete()
            };
        }
        let invocation = match self.capture_history_restoration(document) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::ExError(ExCommandError::Execute(
                        ExExecuteError::Document(error),
                    )),
                    ..CommandOutput::complete()
                }
            }
        };
        let old_cursor = self.cursor;
        self.cursor = normalize_normal_cursor_document(
            document,
            &document.hard_line_snapshot(),
            preview.edits[0].range.start,
        );
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.mode = Mode::Normal;
        self.substitute_confirmation = Some(Box::new(SubstituteConfirmation {
            preview,
            current: 0,
            approved: Vec::new(),
            invocation,
        }));
        CommandOutput {
            cursor_moved: old_cursor != self.cursor,
            mode_changed: true,
            ..CommandOutput::pending()
        }
    }

    pub(super) fn handle_substitute_confirmation(
        &mut self,
        document: &mut Document,
        event: InputEvent,
    ) -> CommandOutput {
        let mut state = self
            .substitute_confirmation
            .take()
            .expect("confirmation checked");
        if !state.preview.is_current(document) {
            return CommandOutput {
                status: CommandStatus::ExError(ExCommandError::Execute(
                    ExExecuteError::UnsupportedCommand(
                        "Substitution cancelled: the document changed while confirming matches"
                            .into(),
                    ),
                )),
                ..CommandOutput::complete()
            };
        }
        let choice = match &event {
            InputEvent::Key(Key::Char(choice)) => Some(*choice),
            InputEvent::Key(Key::Escape | Key::Ctrl('c' | 'C' | '[')) => Some('q'),
            InputEvent::Text(text) if text.chars().count() == 1 => text.chars().next(),
            _ => None,
        };
        let finish = match choice {
            Some('y') => {
                state.approved.push(state.current);
                state.current += 1;
                false
            }
            Some('n') => {
                state.current += 1;
                false
            }
            Some('a') => {
                state.approved.extend(state.current..state.preview.matches);
                true
            }
            Some('l') => {
                state.approved.push(state.current);
                true
            }
            Some('q') => true,
            _ => {
                self.substitute_confirmation = Some(state);
                return CommandOutput::pending();
            }
        };
        if !finish && state.current < state.preview.matches {
            self.cursor = normalize_normal_cursor_document(
                document,
                &document.hard_line_snapshot(),
                state.preview.edits[state.current].range.start,
            );
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.preferred_column = None;
            self.visual_position = None;
            self.desired_x = None;
            self.substitute_confirmation = Some(state);
            return CommandOutput {
                cursor_moved: true,
                ..CommandOutput::pending()
            };
        }
        match commit_ex(
            document,
            &mut self.ex_state,
            state.preview.approved_plan(&state.approved),
        ) {
            Ok(outcome) => {
                if let Some(pattern) = self.ex_state.previous_substitute_pattern() {
                    let direction = self
                        .last_search
                        .as_ref()
                        .map_or(SearchDirection::Forward, |(direction, _)| *direction);
                    self.last_search = Some((direction, pattern.to_owned()));
                    self.search_highlight_suppressed = false;
                }
                if let Some(ExNavigation::TextOffset(offset)) = outcome.navigation {
                    self.cursor = normalize_normal_cursor_document(
                        document,
                        &document.hard_line_snapshot(),
                        offset,
                    );
                }
                self.preferred_column = None;
                self.visual_position = None;
                self.desired_x = None;
                CommandOutput {
                    cursor_moved: outcome.document_changed,
                    document_changed: outcome.document_changed,
                    ex_outcome: Some(outcome),
                    ..CommandOutput::complete()
                }
            }
            Err(error) => CommandOutput {
                status: CommandStatus::ExError(ExCommandError::Execute(error)),
                ..CommandOutput::complete()
            },
        }
    }
}
