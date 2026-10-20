//! Snapshot-bound confirmation across the stable targets of a global command.
use super::*;
use super::ex::ExCommand;
use super::ex_execute::SubstitutePreview;

const MATCH_LIMIT: usize = 100_000;
const REPLACEMENT_BYTE_LIMIT: usize = 16 * 1024 * 1024;

impl CommandInterpreter {
    pub(super) fn global_confirmation_body(&self, command: &ExCommand, depth: usize) -> bool {
        if depth >= COMPOUND_REPLAY_LIMIT { return false; }
        match &command.action {
            ExAction::Global { command, .. } => parse_ex(command).ok()
                .is_some_and(|command| self.global_confirmation_body(&command, depth + 1)),
            action => ex_execute::substitution_confirms(&self.ex_state, action),
        }
    }

    pub(super) fn prepare_global_confirmation(&mut self, document: &Document, body: &ExCommand,
        lines: &[ex_addresses::ResolvedExLine],
    ) -> CommandOutput {
        let mut resolver = self.clone();
        let mut collection = ConfirmationCollection::default();
        resolver.collect_global_confirmation(document, body, lines, &mut collection);
        if let Some(error) = collection.fatal { return ex_global::ex_error(error); }
        // Every question is tied to one original match. Overlapping invocations
        // would require asking about the result of earlier, uncommitted choices.
        // Reject that ambiguity before asking any questions or publishing edits.
        let mut matches: Vec<_> = collection.previews.iter()
            .flat_map(|preview| preview.edits.iter().map(|edit| edit.range.clone())).collect();
        matches.sort_unstable_by_key(|range| (range.start, range.end));
        if matches.windows(2).any(|pair| pair[1].start < pair[0].end
            || pair[0].is_empty() && pair[1].start == pair[0].start) {
            return CommandOutput::unsupported(
                "Confirmed :global substitutions require disjoint match ranges; use a narrower body range");
        }
        self.begin_substitute_confirmations(document, collection.previews, collection.error)
    }

    fn collect_global_confirmation(&mut self, document: &Document, command: &ExCommand,
        lines: &[ex_addresses::ResolvedExLine], collection: &mut ConfirmationCollection,
    ) {
        if self.global_replay_depth >= COMPOUND_REPLAY_LIMIT {
            collection.fatal = Some(ExExecuteError::UnsupportedGlobalCommand);
            return;
        }
        self.global_replay_depth += 1;
        for line in lines {
            if collection.fatal.is_some() { break; }
            if collection.started.elapsed() > ex_global::GLOBAL_REPLAY_TIME_BUDGET {
                collection.fatal = Some(ExExecuteError::Regex(search_regex::RegexError::RegexResourceLimit("global confirmation work")));
                break;
            }
            let result = (|| -> Result<(), ExExecuteError> {
                let request = ExNormalRequest {
                    range: ex_addresses::ResolvedExRange {
                        document: document.id(), revision: document.revision(),
                        start: line.clone(), end: line.clone(), text: line.text.clone(),
                        source: line.source.clone(), lines: vec![line.clone()],
                    }, commands: String::new(), literal: true,
                };
                let target = self.ex_normal_targets(document, &request)
                    .and_then(|targets| targets.into_iter().next().flatten())
                    .ok_or(ExExecuteError::AddressOverflow)?;
                self.prepare_ex_normal_target(document, target, target.anchor.offset());
                let addresses = self.resolve_ex_addresses(document, command, search_regex::RegexLimits::default())
                    .map_err(|error| match error {
                        ex_addresses::ExAddressResolutionError::Execute(error) => error,
                        ex_addresses::ExAddressResolutionError::Layout(_) => ExExecuteError::UnsupportedCommand(
                            "Confirmation requires exact displayed rows".into()),
                    })?;
                if let ExAction::Global { pattern, command: body, invert } = &command.action {
                    if command.range.is_some() { return Err(ExExecuteError::UnsupportedGlobalCommand); }
                    let pattern = if pattern.is_empty() {
                        self.last_search.as_ref().ok_or(ExExecuteError::NoPreviousSearch)?.1.clone()
                    } else { pattern.clone() };
                    let (_, selected) = self.select_global_targets(document, &addresses, &pattern, *invert ^ command.bang)?;
                    let body = parse_ex(body).map_err(|error| ExExecuteError::UnsupportedCommand(error.to_string()))?;
                    self.last_search = Some((SearchDirection::Forward, pattern.clone()));
                    self.ex_state.remember_global_pattern(&pattern);
                    self.collect_global_confirmation(document, &body, &selected, collection);
                    return Ok(());
                }
                let context = ExExecutionContext {
                    current_line: document.hard_line_at_offset(self.cursor).ok_or(ExExecuteError::AddressOverflow)?,
                    last_search_pattern: addresses.search.as_ref().or(self.last_search.as_ref()).map(|(_, pattern)| pattern.clone()),
                    resolved_addresses: Some(addresses), search_options: self.search_options,
                    vim_options: self.vim_options.clone(), wrap: self.wrap,
                    ..ExExecutionContext::default()
                };
                match prepare_ex(document, &self.ex_state, &context, command, &self.registers) {
                    Err(ExExecuteError::NeedsPolicy(ex_execute::ExPolicyRequest::ConfirmSubstitution(preview))) => {
                        collection.matches = collection.matches.saturating_add(preview.matches);
                        collection.bytes = collection.bytes.saturating_add(preview.edits.iter().map(|edit| edit.replacement.len()).sum::<usize>());
                        if collection.matches > MATCH_LIMIT || collection.bytes > REPLACEMENT_BYTE_LIMIT {
                            collection.fatal = Some(ExExecuteError::Regex(search_regex::RegexError::RegexResourceLimit("global confirmation matches")));
                        } else { collection.previews.push(*preview); }
                        Ok(())
                    }
                    Ok(_) => Ok(()), // An error-suppressed target without matches.
                    Err(error) => Err(error),
                }
            })();
            if let Err(error) = result {
                collection.error.get_or_insert(CommandStatus::ExError(ExCommandError::Execute(error)));
            }
        }
        self.global_replay_depth -= 1;
    }
}

struct ConfirmationCollection {
    previews: Vec<SubstitutePreview>,
    error: Option<CommandStatus>,
    fatal: Option<ExExecuteError>,
    matches: usize,
    bytes: usize,
    started: std::time::Instant,
}

impl Default for ConfirmationCollection {
    fn default() -> Self {
        Self { previews: Vec::new(), error: None, fatal: None, matches: 0, bytes: 0,
            started: std::time::Instant::now() }
    }
}
