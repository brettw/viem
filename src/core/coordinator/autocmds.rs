//! Automatic Vim events dispatched through the ordinary script replay runner.
use super::*;
use crate::command::scripting::ScriptPlan;

pub(super) fn file_type_change(outcome: &CoreOutcome) -> Option<crate::document::syntax::detection::LanguageSelection> {
    use crate::document::syntax::detection::{canonical_language, LanguageSelection};
    let effect = outcome
        .command
        .as_ref()?
        .ex_outcome
        .as_ref()?
        .vim_option_effects
        .iter()
        .rev()
        .find(|effect| matches!(effect.name.as_str(), "filetype" | "syntax"))?;
    let value = effect.new_value.text();
    Some(match value.as_str() {
        "" | "on" | "ON" => LanguageSelection::Automatic,
        "off" | "OFF" | "none" | "NONE" => LanguageSelection::None,
        name => LanguageSelection::Language(canonical_language(name)),
    })
}

/// Buffer events fired when a document's first view attaches, in Vim's
/// order for opening a file in a new GUI session.
fn empty_outcome() -> CoreOutcome {
    CoreOutcome { command: Some(crate::command::mappings::empty_mapping_output()), document_changed: false,
        position_map: None, layout_changed: false, composition_changes: Vec::new() }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Fire a native lifecycle event after its host action has reached the
    /// corresponding boundary. An omitted match uses the current filename.
    pub fn fire_autocmd(&mut self, view_id: ViewId, event: &str, matched: Option<&str>) -> Result<CoreOutcome, CoreError> {
        self.fire_autocmd_with_clipboard(view_id, event, matched, None)
    }

    pub(crate) fn fire_autocmd_with_clipboard(&mut self, view_id: ViewId, event: &str, matched: Option<&str>,
        clipboard: Option<ClipboardCommandContext>) -> Result<CoreOutcome, CoreError> {
        if !self.views.contains_key(&view_id) { return Err(CoreError::UnknownView(view_id)); }
        let name = matched.map(str::to_owned).unwrap_or_else(|| self.buffer_filename().to_owned());
        // Save must observe pre-write edits before capturing bytes and
        // post-write edits before deciding whether write-and-quit can close.
        // These callbacks have no asynchronous completion contract, so reject
        // native work before its command/option state can be published.
        let block_native = crate::vimscript::autocmd::event_index(event)
            .is_some_and(|index| matches!(crate::vimscript::autocmd::event_name(index), "BufWritePre" | "BufWritePost"))
            || self.pending_scripts.contains_key(&view_id);
        let commands = &mut self.views.get_mut(&view_id).expect("known callback view").commands;
        let previous = commands.native_requests_blocked;
        commands.native_requests_blocked |= block_native;
        let result = self.fire_named_autocmds(view_id, vec![(event.to_owned(), name)], None, clipboard)
            .map(|value| value.unwrap_or_else(empty_outcome));
        if let Some(view) = self.views.get_mut(&view_id) { view.commands.native_requests_blocked = previous; }
        result
    }

    fn fire_named_autocmds(&mut self, view_id: ViewId, events: Vec<(String, String)>, yank: Option<crate::command::scripting::YankEvent>, clipboard: Option<ClipboardCommandContext>) -> Result<Option<CoreOutcome>, CoreError> {
        if self.firing_autocmds { return Ok(None); }
        self.install_buffer_commands(view_id);
        let Some(view) = self.views.get(&view_id) else { return Ok(None) };
        let state = view.commands.script_state().clone();
        let events = {
            let table = state.autocmds();
            events.into_iter().filter(|(event, _)| crate::vimscript::autocmd::event_index(event).is_some_and(|index| table.has_event(index)))
                .collect::<Vec<_>>()
        };
        if events.is_empty() { return Ok(None); }
        let plan = ScriptPlan { text: String::new(), visual_range: None, history: None, events, yank };
        if self.replay_undo_floor.is_some() {
            // A successful mode transition or yank has no other child replay.
            if self.queued_replay.is_none() { self.queued_replay = Some(ReplayPlan::Script(plan)); }
            return Ok(None);
        }
        self.run_compound_replay(view_id, clipboard, empty_outcome(), ReplayPlan::Script(plan)).map(Some)
    }

    pub(super) fn fire_autocmds(&mut self, view_id: ViewId, events: &[&str]) -> Result<Option<CoreOutcome>, CoreError> {
        let filename = self.buffer_filename().to_owned();
        let filetype = self.reflow_language().unwrap_or_default();
        self.fire_named_autocmds(view_id, events.iter().filter(|event| **event != "FileType" || !filetype.is_empty())
            .map(|event| (event.to_string(), if *event == "FileType" { filetype.clone() } else { filename.clone() })).collect(), None, None)
    }

    /// Fire the buffer-open events once, when the first view attaches.
    pub(super) fn fire_buffer_open_events(&mut self, view_id: ViewId) {
        let first = !std::mem::replace(&mut self.buffer_opened, true);
        let new_file = self.buffer_filename().is_empty() || !std::path::Path::new(self.buffer_filename()).exists();
        let events: &[&str] = if first {
            if new_file { &["BufNewFile", "FileType", "BufEnter", "BufWinEnter"] }
            else { &["BufReadPost", "FileType", "BufEnter", "BufWinEnter"] }
        } else { &["BufEnter", "BufWinEnter"] };
        match self.fire_autocmds(view_id, events) {
            Ok(Some(outcome)) => {
                if let Some(ex) = outcome.command.and_then(|command| command.ex_outcome) {
                    self.startup_effects.extend(ex.frontend_requests);
                }
            }
            Ok(None) => {}
            Err(error) => self.record_presentation_error(view_id, error),
        }
    }

    /// Dispatch after each committed input, including the inputs inside a
    /// mapping or macro. Replay inputs queue an event frame above their caller.
    pub(super) fn fire_input_events(&mut self, view_id: ViewId, before: Mode, outcome: &mut CoreOutcome, clipboard: Option<ClipboardCommandContext>) -> Result<(), CoreError> {
        let Some(view) = self.views.get_mut(&view_id) else { return Ok(()) };
        let after = view.commands.mode();
        let yank = view.commands.pending_yank_event.take();
        if self.firing_autocmds { return Ok(()); }
        let editing = |mode: Mode| matches!(mode, Mode::Insert | Mode::Replace);
        let mut names: Vec<&str> = match (editing(before), editing(after)) {
            (false, true) => vec!["InsertEnter"],
            (true, false) => vec!["InsertLeavePre", "InsertLeave"],
            _ => Vec::new(),
        };
        if yank.is_some() { names.push("TextYankPost"); }
        if names.is_empty() { return Ok(()); }
        self.autocmd_input_generation = self.autocmd_input_generation.saturating_add(1);
        let filename = self.buffer_filename().to_owned();
        let events = names.into_iter().map(|event| (event.to_owned(), filename.clone())).collect();
        if let Some(fired) = self.fire_named_autocmds(view_id, events, yank, clipboard)? {
            let mut accumulator = CoreOutcomeAccumulator::new(std::mem::replace(outcome, empty_outcome()));
            accumulator.merge(fired)?;
            *outcome = accumulator.finish();
        }
        Ok(())
    }
}
