//! Confirmed substitutions across the stable targets of a global command. Like
//! Vim, each target runs its own `:substitute` invocation against the text the
//! earlier invocations produced: an invocation stages its answers against the
//! revision at which it starts and publishes them when it ends, before the next
//! target is prepared. The whole command keeps one undo group, owned by the
//! invoking view between answers in the same way as Insert typing.
use super::*;
use super::ex::ExCommand;
use super::ex_addresses::ExAddressResolutionError;
use super::ex_execute::SubstitutePreview;

// Repeated invocations can grow the document geometrically (`x` to `xx` across
// the whole buffer per target), so the budgets span the whole command. Work
// counts only time spent preparing questions, never time waiting for answers.
const MATCH_LIMIT: usize = 100_000;
const REPLACEMENT_BYTE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct GlobalConfirmation {
    body: ExCommand,
    targets: Vec<Option<ExNormalTarget>>,
    next: usize,
    /// The revision the next invocation is prepared from: the original one,
    /// then the result of each published invocation.
    document: DocumentId,
    revision: Revision,
    /// Where the command started, recorded as a jump when it first asks.
    jump_origin: Option<usize>,
    matches: usize,
    bytes: usize,
    work: std::time::Duration,
    substitutions: usize,
    deferred_error: Option<CommandStatus>,
}

impl GlobalConfirmation {
    pub(super) fn is_current(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    /// Like Vim, a match that consumes a hard break joins the next line into
    /// the one before it, deleting that line's target even when the
    /// replacement is empty or supplies a new break. Returns the targets the
    /// approved matches would join, for removal once they are published.
    fn joined_targets(&self, document: &Document, preview: &SubstitutePreview, approved: &[usize]) -> Vec<usize> {
        let matches: Vec<_> = approved.iter().map(|&index| preview.edits[index].range.clone()).collect();
        (self.next..self.targets.len()).filter(|&index| {
            let Some(position) = self.targets[index].and_then(|target| ex_normal_target_position(document, target))
            else { return false; };
            // Only a target that starts a hard line follows a break in the
            // searched text. A wrapped row follows ordinary text, and a source
            // line inside folded Markdown prose follows a space, which the
            // published source edit alone decides whether to remove.
            let starts_hard_line = document.hard_line_at_offset(position)
                .and_then(|line| document.projection().hard_line_range(line))
                .is_some_and(|line| line.start == position);
            let Some(line_break) = position.checked_sub(1).filter(|_| starts_hard_line) else { return false; };
            let index = matches.partition_point(|range| range.end <= line_break);
            matches.get(index).is_some_and(|range| range.start <= line_break)
        }).collect()
    }
}

impl CommandInterpreter {
    pub(super) fn global_confirmation_body(&self, command: &ExCommand, depth: usize) -> bool {
        if depth >= COMPOUND_REPLAY_LIMIT { return false; }
        match &command.action {
            ExAction::Global { command, .. } => parse_ex(command).ok()
                .is_some_and(|command| self.global_confirmation_body(&command, depth + 1)),
            action => ex_execute::substitution_confirms(&self.ex_state, action),
        }
    }

    pub(super) fn begin_global_confirmation(&mut self, document: &mut Document, body: ExCommand,
        targets: Vec<Option<ExNormalTarget>>, origin: usize,
    ) -> CommandOutput {
        if self.global_replay_depth > 0 {
            return CommandOutput::unsupported("Substitute confirmation is not available inside :global");
        }
        if let Some(output) = self.substitute_confirmation_unavailable() { return output; }
        let session = GlobalConfirmation { body, targets, next: 0, document: document.id(),
            revision: document.revision(), jump_origin: Some(origin), matches: 0, bytes: 0,
            work: std::time::Duration::ZERO, substitutions: 0, deferred_error: None };
        document.begin_edit_group();
        let mut output = self.prepare_global_confirmation(document, std::sync::Arc::new(session),
            InputEvent::Key(Key::Enter), None);
        output.mode_changed = true;
        output
    }

    /// Publish a finished invocation, then prepare the next one. Ctrl-C and
    /// failures end the command; completed invocations stay published.
    pub(super) fn finish_global_invocation(&mut self, document: &mut Document,
        mut global: std::sync::Arc<GlobalConfirmation>, preview: &SubstitutePreview, approved: &[usize],
        event: InputEvent,
    ) -> CommandOutput {
        let session = std::sync::Arc::make_mut(&mut global);
        let joined = session.joined_targets(document, preview, approved);
        let committed = commit_ex(document, &mut self.ex_state, preview.approved_plan(approved));
        let mut fatal = None;
        let mut published = None;
        match committed {
            Ok(mut outcome) => {
                for index in joined { session.targets[index] = None; }
                if let Some(transaction) = outcome.model_transactions().last() {
                    let remaining = &mut session.targets[session.next..];
                    if let Err(error) = rebase_ex_normal_targets(document, remaining, transaction.text_position_map()) {
                        fatal = Some(ExExecuteError::UnsupportedCommand(
                            format!(":global could not rebase its remaining lines: {error}")));
                    }
                }
                self.finish_substitute_invocation(document, &outcome);
                // The command reports its total once, when it ends.
                session.substitutions = session.substitutions.saturating_add(outcome.substitutions);
                outcome.substitutions = 0;
                published = Some(outcome);
            }
            Err(error) => {
                session.deferred_error.get_or_insert(CommandStatus::ExError(ExCommandError::Execute(error)));
            }
        }
        if fatal.is_some() || matches!(event, InputEvent::Key(Key::Ctrl('c' | 'C'))) {
            return self.end_global_confirmation(document, session, fatal, published);
        }
        (session.document, session.revision) = (document.id(), document.revision());
        let mut output = self.prepare_global_confirmation(document, global, event, published);
        output.cursor_moved = true;
        output
    }

    /// Prepare the next invocation. Displayed rows must come from layout of the
    /// revision just published, so the coordinator continues with the same key
    /// once that publication is visible. A headless caller continues now, with
    /// hard lines in place of any layout it supplied for an older revision.
    fn prepare_global_confirmation(&mut self, document: &mut Document,
        global: std::sync::Arc<GlobalConfirmation>, event: InputEvent, published: Option<ExOutcome>,
    ) -> CommandOutput {
        if self.plan_compound_replay {
            self.await_global_substitute_confirmation(global);
            self.pending_replay = Some(ReplayPlan::ContinueAfterPublication(event));
            return CommandOutput {
                document_changed: published.as_ref().is_some_and(|outcome| outcome.document_changed),
                ex_outcome: published,
                ..CommandOutput::pending()
            };
        }
        let stale = self.line_layout.take_if(|snapshot| snapshot.document_revision != document.revision());
        let output = self.advance_global_confirmation(document, global, published);
        if stale.is_some() { self.line_layout = stale; }
        output
    }

    /// Ask about the next remaining target whose invocation has matches, or
    /// end the command when none remains.
    pub(super) fn advance_global_confirmation(&mut self, document: &mut Document,
        mut global: std::sync::Arc<GlobalConfirmation>, published: Option<ExOutcome>,
    ) -> CommandOutput {
        let waiting = global.clone();
        let session = std::sync::Arc::make_mut(&mut global);
        match self.next_global_confirmation(document, session) {
            Ok(Some(preview)) => {
                let origin = session.jump_origin.take();
                let mut output = self.ask_global_substitute_confirmation(document, preview, global);
                if let Some(origin) = origin { self.record_jump(document, origin, self.cursor); }
                output.document_changed = published.as_ref().is_some_and(|outcome| outcome.document_changed);
                output.ex_outcome = published;
                output
            }
            Ok(None) => self.end_global_confirmation(document, session, None, published),
            // Nothing was published in this event, so the coordinator can
            // extend layout and repeat the same preparation.
            Err(ExAddressResolutionError::Layout(error)) => {
                debug_assert!(published.is_none());
                self.await_global_substitute_confirmation(waiting);
                layout_error(error)
            }
            Err(ExAddressResolutionError::Execute(error)) => self.end_global_confirmation(document, session, Some(error), published),
        }
    }

    /// Close the command's undo group and report its total substitutions.
    fn end_global_confirmation(&mut self, document: &mut Document, session: &mut GlobalConfirmation,
        error: Option<ExExecuteError>, published: Option<ExOutcome>,
    ) -> CommandOutput {
        document.end_edit_group();
        let status = match error {
            Some(error) => CommandStatus::ExError(ExCommandError::Execute(error)),
            None => session.deferred_error.take().unwrap_or(CommandStatus::Complete),
        };
        let mut outcome = published.unwrap_or_default();
        outcome.substitutions = session.substitutions;
        CommandOutput {
            status,
            cursor_moved: true,
            document_changed: outcome.document_changed,
            ex_outcome: Some(outcome),
            ..CommandOutput::complete()
        }
    }

    /// Find the next remaining target whose invocation has matches, leaving
    /// the cursor unchanged when there is none.
    fn next_global_confirmation(&mut self, document: &Document, session: &mut GlobalConfirmation,
    ) -> Result<Option<SubstitutePreview>, ExAddressResolutionError> {
        let started = std::time::Instant::now();
        let saved = (self.cursor, self.boundary_affinity, self.physical_cursor,
            self.visual_position, self.desired_x, self.preferred_column);
        self.global_replay_depth += 1;
        let result = self.scan_global_confirmation(document, session, started);
        self.global_replay_depth -= 1;
        session.work = session.work.saturating_add(started.elapsed());
        if !matches!(result, Ok(Some(_))) {
            (self.cursor, self.boundary_affinity, self.physical_cursor,
                self.visual_position, self.desired_x, self.preferred_column) = saved;
        }
        result
    }

    fn scan_global_confirmation(&mut self, document: &Document, session: &mut GlobalConfirmation,
        started: std::time::Instant,
    ) -> Result<Option<SubstitutePreview>, ExAddressResolutionError> {
        while let Some(slot) = session.targets.get(session.next).copied() {
            if session.work.saturating_add(started.elapsed()) > ex_global::GLOBAL_REPLAY_TIME_BUDGET {
                return Err(ExExecuteError::Regex(search_regex::RegexError::RegexResourceLimit("global confirmation work")).into());
            }
            session.next += 1;
            // Deleted targets were removed while rebasing; they never collapse
            // onto a surviving line.
            let Some((target, position)) = slot
                .and_then(|target| ex_normal_target_position(document, target).map(|position| (target, position)))
            else { continue; };
            self.prepare_ex_normal_target(document, target, position);
            match self.prepare_global_confirmation_target(document, &session.body, 0) {
                Ok(Some(preview)) => {
                    session.matches = session.matches.saturating_add(preview.matches);
                    session.bytes = session.bytes.saturating_add(
                        preview.edits.iter().map(|edit| edit.replacement.len()).sum::<usize>());
                    if session.matches > MATCH_LIMIT || session.bytes > REPLACEMENT_BYTE_LIMIT {
                        return Err(ExExecuteError::Regex(search_regex::RegexError::RegexResourceLimit("global confirmation matches")).into());
                    }
                    return Ok(Some(preview));
                }
                Ok(None) => {}
                Err(ExAddressResolutionError::Layout(error)) => return Err(ExAddressResolutionError::Layout(error)),
                // Like unconfirmed :global, report a failed target and continue.
                Err(ExAddressResolutionError::Execute(error)) => {
                    session.deferred_error.get_or_insert(CommandStatus::ExError(ExCommandError::Execute(error)));
                }
            }
        }
        Ok(None)
    }

    /// Prepare the body at the cursor's line against the current revision. A
    /// nested :global without a range tests only that line.
    fn prepare_global_confirmation_target(&mut self, document: &Document, command: &ExCommand, depth: usize,
    ) -> Result<Option<SubstitutePreview>, ExAddressResolutionError> {
        if depth >= COMPOUND_REPLAY_LIMIT { return Err(ExExecuteError::UnsupportedGlobalCommand.into()); }
        let addresses = self.resolve_ex_addresses(document, command, search_regex::RegexLimits::default())?;
        if let ExAction::Global { pattern, command: body, invert } = &command.action {
            if command.range.is_some() { return Err(ExExecuteError::UnsupportedGlobalCommand.into()); }
            let pattern = if pattern.is_empty() {
                self.last_search.as_ref().ok_or(ExExecuteError::NoPreviousSearch)?.1.clone()
            } else { pattern.clone() };
            let (_, selected) = self.select_global_targets(document, &addresses, &pattern, *invert ^ command.bang)?;
            let body = parse_ex(body).map_err(|error| ExExecuteError::UnsupportedCommand(error.to_string()))?;
            self.last_search = Some((SearchDirection::Forward, pattern.clone()));
            self.ex_state.remember_global_pattern(&pattern);
            if selected.is_empty() { return Ok(None); }
            return self.prepare_global_confirmation_target(document, &body, depth + 1);
        }
        let context = ExExecutionContext {
            current_line: document.hard_line_at_offset(self.cursor).ok_or(ExExecuteError::AddressOverflow)?,
            last_search_pattern: addresses.search.as_ref().or(self.last_search.as_ref()).map(|(_, pattern)| pattern.clone()),
            resolved_addresses: Some(addresses), search_options: self.search_options,
            vim_options: self.vim_options.clone(), wrap: self.wrap,
            ..ExExecutionContext::default()
        };
        match prepare_ex(document, &self.ex_state, &context, command, &self.registers) {
            Err(ExExecuteError::NeedsPolicy(ex_execute::ExPolicyRequest::ConfirmSubstitution(preview))) => Ok(Some(*preview)),
            Ok(_) => Ok(None), // An error-suppressed target without matches.
            Err(error) => Err(error.into()),
        }
    }
}
