//! Portable, snapshot-bound substitute confirmation. Decisions remain staged
//! until the invocation ends, so a lone invocation holds no edit group between
//! answers. A confirmed :global publishes each invocation inside one undo group
//! owned by the invoking view (see `global_confirmation`).
use super::ex_execute::SubstitutePreview;
use super::global_confirmation::GlobalConfirmation;
use super::*;

pub(super) fn scroll_motion(event: &InputEvent) -> Option<ScreenMotion> {
    match event {
        InputEvent::Key(Key::Ctrl('e' | 'E')) => Some(ScreenMotion::ScrollDown),
        InputEvent::Key(Key::Ctrl('y' | 'Y')) => Some(ScreenMotion::ScrollUp),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub(super) struct SubstituteConfirmation {
    /// The invocation being asked about. A confirmed :global has none while it
    /// waits to prepare its next invocation against exact layout.
    question: Option<SubstituteQuestion>,
    scope: ConfirmationScope,
}

#[derive(Clone, Debug)]
struct SubstituteQuestion {
    preview: std::sync::Arc<SubstitutePreview>,
    current: usize,
    approved: Vec<usize>,
}

#[derive(Clone, Debug)]
enum ConfirmationScope {
    /// A lone invocation records the view state from before its first question,
    /// so undo returns there rather than to the last match asked about.
    Single(HistoryRestorationSnapshot),
    /// The rest of a confirmed :global, which publishes this invocation before
    /// preparing the next target. Its open undo group records restoration.
    Global(std::sync::Arc<GlobalConfirmation>),
}

impl SubstituteConfirmation {
    fn is_current(&self, document: &Document) -> bool {
        match (&self.question, &self.scope) {
            (Some(question), _) => question.preview.is_current(document),
            (None, ConfirmationScope::Global(global)) => global.is_current(document),
            (None, ConfirmationScope::Single(_)) => false,
        }
    }

    fn current_question(&self, document: &Document) -> Option<&SubstituteQuestion> {
        self.question.as_ref().filter(|question| question.preview.is_current(document))
    }
}

impl CommandInterpreter {
    pub(super) fn scroll_substitute_confirmation(
        &mut self, document: &mut Document, context: &mut LayoutCommandContext<'_>, motion: ScreenMotion,
    ) -> CommandOutput {
        if !self.substitute_confirmation.as_ref().is_some_and(|state| state.current_question(document).is_some()) {
            return self.handle_substitute_confirmation(document, InputEvent::Key(Key::Escape));
        }
        // Scrolling a question never moves its match or changes an approval.
        let saved = (self.cursor, self.boundary_affinity, self.physical_cursor,
            self.visual_position, self.desired_x, self.preferred_column);
        let mut output = self.execute_screen_motion(document, context, motion, 1);
        (self.cursor, self.boundary_affinity, self.physical_cursor,
            self.visual_position, self.desired_x, self.preferred_column) = saved;
        output.cursor_moved = false;
        if output.status == CommandStatus::Complete { output.status = CommandStatus::Pending; }
        output
    }

    /// Plain status text, deliberately separate from an editable Ex prompt.
    pub fn substitute_confirmation_prompt(&self) -> Option<String> {
        let question = self.substitute_confirmation.as_ref()?.question.as_ref()?;
        let replacement = &question.preview.edits[question.current].replacement;
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
            question.current + 1,
            question.preview.matches,
            question.approved.len()
        ))
    }

    pub(crate) fn substitute_confirmation_history(
        &self,
        document: &Document,
    ) -> Option<&HistoryRestorationSnapshot> {
        let state = self.substitute_confirmation.as_ref()?;
        match &state.scope {
            ConfirmationScope::Single(invocation) if state.current_question(document).is_some() => Some(invocation),
            _ => None,
        }
    }

    pub(super) fn substitute_confirmation_match(
        &self,
        document: &Document,
    ) -> Option<Range<usize>> {
        let question = self.substitute_confirmation.as_ref()?.current_question(document)?;
        Some(question.preview.edits[question.current].range.clone())
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
        if let Some(output) = self.substitute_confirmation_unavailable() {
            return output;
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
        self.ask_substitute_confirmation(document, preview, ConfirmationScope::Single(invocation))
    }

    pub(super) fn substitute_confirmation_unavailable(&self) -> Option<CommandOutput> {
        (self.script_depth > 0).then(|| CommandOutput {
            status: CommandStatus::ExError(ExCommandError::Execute(
                ExExecuteError::UnsupportedCommand(
                    "Substitute confirmation is not available in scripts".into(),
                ),
            )),
            ..CommandOutput::complete()
        })
    }

    /// Ask about the first match of one invocation of a confirmed :global,
    /// whose undo group is already open.
    pub(super) fn ask_global_substitute_confirmation(
        &mut self,
        document: &Document,
        preview: SubstitutePreview,
        global: std::sync::Arc<GlobalConfirmation>,
    ) -> CommandOutput {
        self.ask_substitute_confirmation(document, preview, ConfirmationScope::Global(global))
    }

    /// Hold a confirmed :global between invocations until the coordinator
    /// continues it with layout of the revision it just published.
    pub(super) fn await_global_substitute_confirmation(&mut self, global: std::sync::Arc<GlobalConfirmation>) {
        self.mode = Mode::Normal;
        self.substitute_confirmation = Some(Box::new(SubstituteConfirmation {
            question: None,
            scope: ConfirmationScope::Global(global),
        }));
    }

    pub(crate) fn awaits_substitute_continuation(&self) -> bool {
        self.substitute_confirmation.as_ref().is_some_and(|state| state.question.is_none())
    }

    fn ask_substitute_confirmation(
        &mut self,
        document: &Document,
        preview: SubstitutePreview,
        scope: ConfirmationScope,
    ) -> CommandOutput {
        let old_cursor = self.cursor;
        self.cursor = normalize_normal_cursor_document(
            document,
            &document.hard_line_snapshot(),
            preview.edits[0].range.start,
        );
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.preferred_column = None;
        self.visual_position = None;
        self.desired_x = None;
        self.mode = Mode::Normal;
        self.substitute_confirmation = Some(Box::new(SubstituteConfirmation {
            question: Some(SubstituteQuestion {
                preview: std::sync::Arc::new(preview),
                current: 0,
                approved: Vec::new(),
            }),
            scope,
        }));
        CommandOutput {
            cursor_moved: old_cursor != self.cursor,
            mode_changed: true,
            ..CommandOutput::pending()
        }
    }

    /// Whether this view owns an open :global confirmation undo group. The
    /// coordinator keeps that group open between answers, as it does for
    /// Insert typing, and closes it before another writer or a save.
    pub(crate) fn holds_substitute_confirmation_group(&self, document: &Document) -> bool {
        self.substitute_confirmation.as_ref().is_some_and(|state|
            matches!(state.scope, ConfirmationScope::Global(_)) && state.is_current(document))
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
        if !state.is_current(document) {
            // Completed :global invocations stay published; only the current
            // invocation's staged answers and the remaining targets are lost.
            if matches!(state.scope, ConfirmationScope::Global(_)) {
                document.end_edit_group();
            }
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
        let Some(mut question) = state.question.take() else {
            let ConfirmationScope::Global(global) = state.scope else {
                unreachable!("only a confirmed :global waits between invocations");
            };
            return self.advance_global_confirmation(document, global, None);
        };
        let choice = match &event {
            InputEvent::Key(Key::Char(choice)) => Some(*choice),
            InputEvent::Key(Key::Escape | Key::Ctrl('c' | 'C' | '[')) => Some('q'),
            InputEvent::Text(text) if text.chars().count() == 1 => text.chars().next(),
            _ => None,
        };
        let finish = match choice {
            Some('y') => {
                question.approved.push(question.current);
                question.current += 1;
                false
            }
            Some('n') => {
                question.current += 1;
                false
            }
            Some('a') => {
                question.approved.extend(question.current..question.preview.matches);
                true
            }
            Some('l') => {
                question.approved.push(question.current);
                true
            }
            Some('q') => true,
            _ => {
                state.question = Some(question);
                self.substitute_confirmation = Some(state);
                return CommandOutput::pending();
            }
        };
        if !finish && question.current < question.preview.matches {
            self.cursor = normalize_normal_cursor_document(
                document,
                &document.hard_line_snapshot(),
                question.preview.edits[question.current].range.start,
            );
            self.boundary_affinity = BoundaryAffinity::Downstream;
            self.preferred_column = None;
            self.visual_position = None;
            self.desired_x = None;
            state.question = Some(question);
            self.substitute_confirmation = Some(state);
            return CommandOutput {
                cursor_moved: true,
                ..CommandOutput::pending()
            };
        }
        // Like Vim, a/q/l finish this :substitute invocation. A global
        // continues with its next selected line; Ctrl-C cancels the rest.
        if let ConfirmationScope::Global(global) = state.scope {
            return self.finish_global_invocation(document, global, &question.preview, &question.approved, event);
        }
        match commit_ex(document, &mut self.ex_state, question.preview.approved_plan(&question.approved)) {
            Ok(outcome) => {
                self.finish_substitute_invocation(document, &outcome);
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

    pub(super) fn finish_substitute_invocation(&mut self, document: &Document, outcome: &ExOutcome) {
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
    }
}
