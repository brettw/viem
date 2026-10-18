//! Vim script execution inside the compound replay runner. A submitted
//! command line that needs the engine becomes a [`ScriptFrame`]; its host
//! commands and editor actions are staged on the view's command line and
//! dispatched as ordinary replay input, so layout retries, publication,
//! undo grouping and nested replays (`:normal`) behave as for mappings.
use super::*;
use crate::command::ex_execute::ExInfoRequest;
use crate::command::scripting::{fold_script_messages, split_set_command, status_error, ScriptAction, ScriptPlan};
use crate::vimscript::value::Value;
use crate::vimscript::{EvalResult, Fail, HostCommand, Message, OptionScope, RunState, Runtime, ScriptHost, VimError};

pub(super) struct ScriptFrame {
    /// Absent only while the runtime is running.
    runtime: Option<Box<Runtime>>,
    messages: Vec<Message>,
    emitted_error: Option<CommandStatus>,
    history: Option<String>,
    /// The typed Visual range for the first host command addressing `'<,'>`.
    visual_range: Option<crate::command::ex_execute::ResolvedExRange>,
    last_failure: Option<(String, CommandStatus)>,
    /// Keys `feedkeys()` queued, with whether they may be remapped.
    fed_keys: Vec<(InputEvent, bool)>,
    pub(super) finished: bool,
    pub(super) suppress_input_events: bool,
    event_context: Option<Value>,
    previous_event_context: Option<Value>,
    started: bool,
}

impl std::fmt::Debug for ScriptFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ScriptFrame").field("finished", &self.finished).finish_non_exhaustive()
    }
}

impl ScriptFrame {
    pub(super) fn new(plan: ScriptPlan, state: crate::vimscript::ScriptState) -> Self {
        let is_event = !plan.events.is_empty();
        let event_context = plan.yank.as_ref().map(|event| event.value());
        let mut runtime = Runtime::new(state);
        if plan.events.is_empty() {
            runtime.push_command_line(&plan.text, 0);
        } else {
            // Frames are a stack: push the last event first.
            for (event, matched) in plan.events.iter().rev() {
                runtime.push_event(event, matched);
            }
        }
        Self {
            runtime: Some(Box::new(runtime)),
            messages: Vec::new(),
            emitted_error: None,
            history: plan.history,
            visual_range: plan.visual_range,
            last_failure: None,
            fed_keys: Vec::new(),
            finished: false,
            suppress_input_events: is_event,
            event_context,
            previous_event_context: None,
            started: false,
        }
    }
}

/// What the runner does after running a script frame.
pub(super) enum ScriptStep {
    /// The script stopped at a command boundary so a nested replay (for
    /// example `:normal`) can run first; it resumes when that frame pops.
    Suspended(ReplayPlan),
    /// Native effects must finish before the next script command runs.
    AwaitingHost,
    /// The script finished; a failing status stops the compound replay.
    Finished(Option<CommandStatus>),
}

struct CoreScriptHost<'a, P: TextMeasurementProvider> {
    core: &'a mut Core<P>,
    view_id: ViewId,
    clipboard: Option<&'a ClipboardCommandContext>,
    frames: &'a mut Vec<ReplayFrame>,
    accumulator: &'a mut CoreOutcomeAccumulator,
    dispatched_events: &'a mut usize,
    messages: Vec<Message>,
    emitted_error: Option<CommandStatus>,
    visual_range: Option<crate::command::ex_execute::ResolvedExRange>,
    last_failure: Option<(String, CommandStatus)>,
    fed_keys: Vec<(InputEvent, bool)>,
    /// A nested replay the last command queued; the runtime suspends.
    suspend: Option<ReplayPlan>,
    awaiting_host: bool,
    suppress_child_events: bool,
}

impl<P: TextMeasurementProvider> CoreScriptHost<'_, P> {
    fn commands(&self) -> &CommandInterpreter {
        &self.core.views[&self.view_id].commands
    }

    fn commands_mut(&mut self) -> &mut CommandInterpreter {
        &mut self.core.views.get_mut(&self.view_id).expect("script view remains attached").commands
    }

    /// Stage one action on the command line and dispatch Enter through the
    /// ordinary replay path.
    fn dispatch(&mut self, rt: &Runtime, action: ScriptAction) -> EvalResult<()> {
        if rt.text_locked() && matches!(action, ScriptAction::SetLines { .. } | ScriptAction::AppendLines { .. }) {
            return Err(Fail::Error(VimError::new(565, "Not allowed to change text or change window")));
        }
        if *self.dispatched_events >= self.core.compound_replay_event_limit {
            return Err(Fail::Error(VimError::plain(format!(
                "script stopped after {} commands (compound replay limit)",
                self.core.compound_replay_event_limit
            ))));
        }
        *self.dispatched_events += 1;
        self.commands_mut().queue_script_action(action).map_err(|message| Fail::Error(VimError::plain(message)))?;
        let nested = rt.is_nested();
        // A synchronous call from an expression cannot suspend, so compound
        // commands it runs (`:normal`) execute without the coordinator.
        let planned = self.commands_mut().set_plan_compound_replay(!nested);
        let mut outcome = self.core.dispatch_replay_input(self.view_id, InputEvent::Key(Key::Enter), self.clipboard);
        self.commands_mut().set_plan_compound_replay(planned);
        if rt.messages_silenced() {
            if let Some(ex) = outcome.command.as_mut().and_then(|command| command.ex_outcome.as_mut()) {
                ex.frontend_requests.retain(|request| !matches!(request, ExFrontendRequest::Info(ExInfoRequest::Message(_))));
            }
        }
        let host_action = outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref())
            .is_some_and(|ex| ex.frontend_requests.iter().any(|request|
                matches!(request, ExFrontendRequest::File(_) | ExFrontendRequest::Window(_))));
        if host_action && nested {
            // Expression evaluation cannot yield. No native request has escaped
            // yet, so reject it before publishing an external side effect.
            return Err(Fail::Error(VimError::new(565,
                "Native host commands cannot run inside an expression")));
        }
        let application_action = outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref())
            .is_some_and(|ex| ex.frontend_requests.iter().any(|request| matches!(request, ExFrontendRequest::Application(_))));
        if (host_action || application_action) && self.core.pending_scripts.contains_key(&self.view_id) {
            return Err(Fail::Error(VimError::new(565,
                "A native callback cannot start another suspended host command")));
        }
        self.awaiting_host |= host_action || application_action;
        let status = outcome.command.as_ref().map_or(CommandStatus::Complete, |command| command.status.clone());
        let history_navigation = outcome.command.as_ref().is_some_and(|command| command.history_navigation);
        let map = outcome.position_map.clone();
        // Flush earlier output before this host command's effects. Retain its
        // error status for final completion; an ordinary script error does not
        // prevent Vim's remaining commands from running.
        if !self.messages.is_empty() {
            let mut output = crate::command::mappings::empty_mapping_output();
            fold_script_messages(&mut output, std::mem::take(&mut self.messages), self.last_failure.clone());
            if crate::command::command_status_stops_compound(&output.status) {
                self.emitted_error = Some(output.status.clone());
                output.status = CommandStatus::Complete;
            }
            self.accumulator.command_mut().merge(output);
        }
        self.accumulator.merge(outcome).map_err(|error| Fail::Error(VimError::plain(format!("{error:?}"))))?;
        if let Some(map) = map.as_ref() {
            rebase_replay_ex_normal_targets(&self.core.document, self.frames, map)
                .map_err(|error| Fail::Error(VimError::plain(format!(":normal could not rebase its remaining range: {error}"))))?;
        }
        if history_navigation {
            self.core.begin_replay_undo_segment(self.view_id).map_err(|error| Fail::Error(VimError::plain(format!("{error:?}"))))?;
        }
        if let Some(nested) = self.core.queued_replay.take() {
            self.suspend = Some(nested);
        }
        if let Some(error) = status_error(&status) {
            self.last_failure = Some((error.text(), status));
            self.accumulator.command_mut().status = CommandStatus::Complete;
            return Err(Fail::Error(error));
        }
        Ok(())
    }

    fn cursor_target(args: &[Value]) -> EvalResult<Option<(usize, usize)>> {
        let number = |value: &Value| value.to_number();
        let (line, column) = match args {
            [Value::List(list), ..] => {
                let items = list.snapshot();
                (items.first().map(number).transpose()?.unwrap_or(0), items.get(1).map(number).transpose()?.unwrap_or(1))
            }
            [line, rest @ ..] => (number(line)?, rest.first().map(number).transpose()?.unwrap_or(0)),
            [] => return Ok(None),
        };
        if line < 0 || column < 0 {
            return Ok(None);
        }
        Ok(Some((line as usize, column.max(1) as usize)))
    }

    fn lines_argument(value: Option<&Value>) -> EvalResult<Vec<String>> {
        Ok(match value {
            Some(Value::List(list)) => list.snapshot().iter().map(|item| item.to_text()).collect::<Result<_, _>>()?,
            Some(value) => vec![value.to_text()?],
            None => Vec::new(),
        })
    }

    fn line_argument(&self, value: Option<&Value>) -> EvalResult<i64> {
        let Some(value) = value else { return Ok(0) };
        Ok(match value {
            Value::String(text) if text.as_bytes() == b"." => self.commands().script_cursor_line(&self.core.document),
            Value::String(text) if text.as_bytes() == b"$" => self.core.document.physical_source_line_number_count() as i64,
            other => other.to_number()?,
        })
    }

    /// `search()` and `searchpos()`: Vim regex over physical source lines.
    fn search(&mut self, rt: &Runtime, args: &[Value], positions: bool) -> EvalResult<Value> {
        let pattern = args.first().map(|value| value.to_text()).transpose()?.unwrap_or_default();
        let flags = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
        let stop = args.get(2).map(|value| value.to_number()).transpose()?.unwrap_or(0).max(0) as usize;
        let backward = flags.contains('b');
        let wrap = !flags.contains('W');
        let accept_at_cursor = flags.contains('c');
        let to_end = flags.contains('e');
        let compiled = crate::vimscript::regex::compile(pattern.as_bytes(), self.ignorecase())?;
        let document = &self.core.document;
        let commands = self.commands();
        let count = document.physical_source_line_number_count().max(1);
        let line_text = |line: usize| -> String {
            document.physical_line(line - 1).map(|physical| physical.text.trim_end_matches(['\n', '\r']).to_string()).unwrap_or_default()
        };
        let cursor_line = (commands.script_cursor_line(document).max(1) as usize).min(count);
        let cursor_column = commands.script_cursor_column(document).max(1) as usize - 1;
        let mut fuel = 50_000_000u64;
        let mut found = None;
        let lines: Vec<usize> = if backward {
            (1..=cursor_line).rev().chain(if wrap { (cursor_line..=count).rev().collect::<Vec<_>>() } else { Vec::new() }).collect()
        } else {
            (cursor_line..=count).chain(if wrap { (1..=cursor_line).collect::<Vec<_>>() } else { Vec::new() }).collect()
        };
        'lines: for (visit, line) in lines.into_iter().enumerate() {
            if stop > 0 && ((backward && line < stop) || (!backward && line > stop)) {
                break;
            }
            let text = line_text(line);
            let mut matches = Vec::new();
            let mut start = 0;
            while let Some(found) = crate::vimscript::regex::find_at(&compiled, &text, start, &mut fuel)? {
                matches.push(found.start..found.end);
                start = if found.end > found.start { found.end } else { found.start + text[found.start..].chars().next().map_or(1, char::len_utf8) };
                if start > text.len() {
                    break;
                }
            }
            let first_visit = visit == 0;
            let wrapped_visit = !first_visit && line == cursor_line;
            let candidates: Box<dyn Iterator<Item = &std::ops::Range<usize>>> =
                if backward { Box::new(matches.iter().rev()) } else { Box::new(matches.iter()) };
            for range in candidates {
                let column = if to_end { range.end.saturating_sub(1).max(range.start) } else { range.start };
                let acceptable = if first_visit {
                    if backward { column < cursor_column || (accept_at_cursor && column == cursor_column) } else { column > cursor_column || (accept_at_cursor && column == cursor_column) }
                } else if wrapped_visit {
                    if backward { column >= cursor_column } else { column <= cursor_column }
                } else {
                    true
                };
                if acceptable {
                    found = Some((line, column + 1));
                    break 'lines;
                }
            }
        }
        let _ = rt;
        let Some((line, column)) = found else {
            return Ok(if positions { Value::list(vec![0.into(), 0.into()]) } else { Value::Number(0) });
        };
        if !flags.contains('n') {
            self.dispatch(rt, ScriptAction::SetCursor { line, column })?;
        }
        Ok(if positions { Value::list(vec![(line as i64).into(), (column as i64).into()]) } else { Value::Number(line as i64) })
    }
}

impl<P: TextMeasurementProvider> ScriptHost for CoreScriptHost<'_, P> {
    fn execute(&mut self, rt: &mut Runtime, command: &HostCommand) -> EvalResult<()> {
        if rt.text_locked() && !matches!(command.name, Some("set" | "setlocal" | "setglobal" | "nohlsearch"))
            && !command.name.is_some_and(crate::command::mappings::is_mapping_command) {
            return Err(Fail::Error(VimError::new(565, "Not allowed to change text or change window")));
        }
        self.suppress_child_events = command.modifiers.noautocmd || !rt.automatic_events_allowed();
        if command.name == Some("colorscheme") && !command.argument.trim().is_empty() && !command.modifiers.noautocmd {
            rt.fire_event_now(self, "ColorSchemePre", command.argument.trim())?;
        }
        for text in split_set_command(command) {
            let visual_range = if command.range.starts_with("'<") { self.visual_range.take() } else { None };
            let argument = if text == command.text { command.argument.clone() } else { text.split_once(' ').map_or(String::new(), |(_, argument)| argument.to_owned()) };
            self.dispatch(rt, ScriptAction::Command { text, name: command.name, bang: command.bang, argument, sid: command.sid, visual_range })?;
            if let Some(yank) = self.commands_mut().pending_yank_event.take() {
                if !self.suppress_child_events {
                    let previous = rt.state.vvar("event");
                    rt.state.set_vvar("event", yank.value());
                    let filename = self.core.buffer_filename().to_owned();
                    let fired = rt.fire_event_now(self, "TextYankPost", &filename);
                    rt.state.set_vvar("event", previous);
                    fired?;
                }
            }
            if self.suspend.is_some() {
                break;
            }
        }
        for (event, matched) in crate::command::scripting::command_events(command) {
            rt.queue_event_name(event, matched);
        }
        Ok(())
    }
    fn get_option(&mut self, name: &str, _scope: OptionScope) -> Result<Value, VimError> {
        self.commands().script_option(&self.core.document, name)
    }
    fn set_option(&mut self, rt: &mut Runtime, name: &str, scope: OptionScope, value: &Value) -> EvalResult<()> {
        let text = crate::command::options::set_command_for(self.commands(), &self.core.document, name, scope, value)?;
        let (command_name, argument) = text.split_once(' ').unwrap_or((text.as_str(), ""));
        let name = crate::vimscript::excmds::lookup(command_name);
        self.dispatch(rt, ScriptAction::Command { text: text.clone(), name, bang: false, argument: argument.to_owned(), sid: rt.current_sid(), visual_range: None })?;
        if let Ok(value) = self.commands().script_option(&self.core.document, "filetype") {
            if text.contains("filetype=") || text.contains("ft=") {
                rt.queue_event_name("FileType".into(), value.to_text()?);
            }
        }
        Ok(())
    }
    fn option_exists(&self, name: &str, working: bool) -> bool {
        crate::command::options::option_exists(name, working)
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        let view_id = self.view_id;
        let core = &mut *self.core;
        core.views.get_mut(&view_id).expect("script view").commands.script_register(&core.document, name)
    }
    fn set_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError> {
        self.commands_mut().set_script_register(name, value, append)?;
        self.core.publish_buffer_commands(self.view_id);
        Ok(())
    }
    fn call_function(&mut self, rt: &mut Runtime, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
        let result = match name {
            "cursor" | "setcharpos" | "setcursorcharpos" | "setpos" => (|| {
                let args = if name == "setpos" || name == "setcharpos" {
                    let target = args.first().map(|value| value.to_text()).transpose()?.unwrap_or_default();
                    if target != "." {
                        return Ok(Value::Number(-1));
                    }
                    match args.get(1) {
                        Some(Value::List(list)) => list.snapshot().into_iter().skip(1).collect::<Vec<_>>(),
                        _ => return Ok(Value::Number(-1)),
                    }
                } else {
                    args.to_vec()
                };
                let Some((line, column)) = Self::cursor_target(&args)? else { return Ok(Value::Number(-1)) };
                let line = if line == 0 { self.commands().script_cursor_line(&self.core.document).max(1) as usize } else { line };
                self.dispatch(rt, ScriptAction::SetCursor { line, column })?;
                Ok(Value::Number(0))
            })(),
            "winrestview" => (|| {
                let Some(Value::Dict(view)) = args.first() else { return Err(Fail::Error(VimError::new(1206, "Dictionary required for argument 1"))) };
                let current = self.commands().script_cursor_line(&self.core.document);
                let line = view.get_str("lnum").map(|value| value.to_number()).transpose()?.unwrap_or(current).max(1) as usize;
                let column = view.get_str("col").map(|value| value.to_number()).transpose()?.unwrap_or(0).max(0) as usize + 1;
                self.dispatch(rt, ScriptAction::SetCursor { line, column })?;
                Ok(Value::Number(0))
            })(),
            "winsaveview" => (|| {
                let document = &self.core.document;
                let commands = self.commands();
                let line = commands.script_cursor_line(document);
                let column = commands.script_cursor_column(document) - 1;
                Ok(Value::Dict(crate::vimscript::Dict::from_pairs([
                    ("lnum".into(), line.into()),
                    ("col".into(), column.into()),
                    ("coladd".into(), 0.into()),
                    ("curswant".into(), column.into()),
                    ("topline".into(), 1.into()),
                    ("topfill".into(), 0.into()),
                    ("leftcol".into(), 0.into()),
                    ("skipcol".into(), 0.into()),
                ])))
            })(),
            "setline" | "append" => (|| {
                if self.textlock() {
                    return Err(Fail::Error(VimError::new(565, "Not allowed to change text or change window")));
                }
                let line = self.line_argument(args.first())?;
                let lines = Self::lines_argument(args.get(1))?;
                let count = self.core.document.physical_source_line_number_count() as i64;
                if name == "setline" {
                    if line < 1 || line > count + 1 {
                        return Ok(Value::Number(1));
                    }
                    self.dispatch(rt, ScriptAction::SetLines { first: line as usize, lines })?;
                } else {
                    if line < 0 || line > count {
                        return Ok(Value::Number(1));
                    }
                    self.dispatch(rt, ScriptAction::AppendLines { after: line as usize, lines })?;
                }
                Ok(Value::Number(0))
            })(),
            "search" | "searchpos" => self.search(rt, args, name == "searchpos"),
            "feedkeys" => (|| {
                let keys = args.first().map(|value| value.to_bytes()).transpose()?.unwrap_or_default();
                let mode = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let remap = !mode.contains('n');
                for key in crate::command::mappings::script_keys(&keys) {
                    self.fed_keys.push((InputEvent::Key(key), remap));
                }
                Ok(Value::Number(0))
            })(),
            _ => {
                let view_id = self.view_id;
                let core = &mut *self.core;
                let buffer = core.buffer_filename().to_owned();
                let buffer = (!buffer.is_empty()).then_some(buffer);
                return core.views.get_mut(&view_id).expect("script view").commands.script_function(&core.document, buffer.as_deref(), name, args);
            }
        };
        Some(result)
    }
    fn has_function(&self, name: &str) -> bool {
        crate::command::scripting::SCRIPT_FUNCTIONS.contains(&name)
    }
    fn feature(&self, name: &str) -> Option<bool> {
        crate::command::scripting::script_feature(name)
    }
    fn command_exists(&self, name: &str) -> bool {
        crate::command::scripting::host_command_exists(name)
    }
    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
    fn ignorecase(&self) -> bool {
        self.commands().script_ignorecase()
    }
    fn dynamic_variable(&self, name: &[u8]) -> Option<Value> {
        self.commands().script_dynamic_variable(&self.core.document, name)
    }
    fn view_id(&self) -> u64 {
        self.view_id.0
    }
    fn buffer_name(&self) -> Option<String> {
        let name = self.core.buffer_filename();
        (!name.is_empty()).then(|| name.to_owned())
    }
    fn should_suspend(&mut self) -> bool {
        self.suspend.is_some() || self.awaiting_host
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// A script frame abandoned before it finished (an outer replay stopped)
    /// releases its hold on the view's script depth.
    pub(super) fn release_script_frame(&mut self, view_id: ViewId, frame: &ScriptFrame) {
        if frame.started && !frame.finished {
            if let Some(view) = self.views.get_mut(&view_id) {
                if let Some(previous) = &frame.previous_event_context {
                    view.commands.script_state().set_vvar("event", previous.clone());
                }
                view.commands.script_depth -= 1;
            }
        }
    }

    pub(super) fn release_replay_frame(&mut self, view_id: ViewId, frame: Option<ReplayFrame>) {
        if let Some(ReplayFrame::Script(script)) = frame {
            self.release_script_frame(view_id, &script);
        }
    }

    /// Run the script frame on top of `frames` until it finishes or must
    /// suspend for a nested replay.
    pub(super) fn run_script_frame(
        &mut self,
        view_id: ViewId,
        clipboard: Option<&ClipboardCommandContext>,
        frames: &mut Vec<ReplayFrame>,
        accumulator: &mut CoreOutcomeAccumulator,
        dispatched_events: &mut usize,
    ) -> Result<ScriptStep, CoreError> {
        let Some(ReplayFrame::Script(frame)) = frames.last_mut() else {
            unreachable!("run_script_frame requires a script frame on top");
        };
        let mut runtime = frame.runtime.take().expect("a script frame owns its runtime between runs");
        if !std::mem::replace(&mut frame.started, true) {
            self.views.get_mut(&view_id).expect("script view").commands.script_depth += 1;
            if let Some(context) = &frame.event_context {
                frame.previous_event_context = Some(runtime.state.vvar("event"));
                runtime.state.set_vvar("event", context.clone());
            }
        }
        let messages = std::mem::take(&mut frame.messages);
        let emitted_error = frame.emitted_error.take();
        let visual_range = frame.visual_range.take();
        let last_failure = frame.last_failure.take();
        let fed_keys = std::mem::take(&mut frame.fed_keys);
        let mut host = CoreScriptHost {
            core: &mut *self,
            view_id,
            clipboard,
            frames: &mut *frames,
            accumulator: &mut *accumulator,
            dispatched_events,
            messages,
            emitted_error,
            visual_range,
            last_failure,
            fed_keys,
            suspend: None,
            awaiting_host: false,
            suppress_child_events: false,
        };
        let state = runtime.run(&mut host);
        let CoreScriptHost { messages, emitted_error, visual_range, last_failure, fed_keys, suspend, awaiting_host, suppress_child_events, .. } = host;
        let Some(ReplayFrame::Script(frame)) = frames.last_mut() else {
            unreachable!("nested replays are pushed only after the script frame suspends");
        };
        frame.messages = messages;
        frame.emitted_error = emitted_error;
        frame.visual_range = visual_range;
        frame.last_failure = last_failure;
        frame.fed_keys = fed_keys;
        frame.runtime = Some(runtime);
        frame.suppress_input_events = suppress_child_events;
        if state == RunState::Suspended {
            if awaiting_host { return Ok(ScriptStep::AwaitingHost); }
            if let Some(plan) = suspend {
                return Ok(ScriptStep::Suspended(plan));
            }
        }
        frame.finished = true;
        if let Some(previous) = frame.previous_event_context.take() {
            self.views[&view_id].commands.script_state().set_vvar("event", previous);
        }
        self.views.get_mut(&view_id).expect("script view").commands.script_depth -= 1;
        let messages = std::mem::take(&mut frame.messages);
        let failure = frame.last_failure.take();
        let history = frame.history.take();
        let fed = std::mem::take(&mut frame.fed_keys);
        let new_errors = messages.iter().any(|message| matches!(message, Message::Error(_)));
        let errors = new_errors || frame.emitted_error.is_some();
        let command = accumulator.command_mut();
        fold_script_messages(command, messages, failure);
        if !new_errors {
            if let Some(error) = frame.emitted_error.take() { command.status = error; }
        }
        let status = command.status.clone();
        if !errors {
            if let Some(history) = history {
                self.views.get_mut(&view_id).expect("script view").commands.push_script_history(history);
            }
        }
        if errors && crate::command::command_status_stops_compound(&status) {
            return Ok(ScriptStep::Finished(Some(status)));
        }
        if !fed.is_empty() {
            return Ok(ScriptStep::Suspended(ReplayPlan::Mapping(crate::command::mappings::MappingReplayPlan {
                events: fed,
                restore_select: false,
            })));
        }
        Ok(ScriptStep::Finished(None))
    }
}

/// All replay state stays with its original view while a native request runs.
/// It retains the same fuel/event budgets through any number of resumptions.
pub(super) struct PendingReplay {
    pub token: u64,
    pub frames: Vec<ReplayFrame>,
    pub dispatched_events: usize,
    pub mapping_replay: bool,
    pub global_error: Option<CommandStatus>,
    pub global_started: Option<std::time::Instant>,
    pub completed_global_targets: usize,
}

static NEXT_CONTINUATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub(super) fn allocate_continuation_token() -> Option<u64> {
    NEXT_CONTINUATION.fetch_update(std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed, |token| token.checked_add(1)).ok()
}

impl<P: TextMeasurementProvider> Core<P> {
    pub(super) fn cancel_pending_script(&mut self, view: ViewId) {
        let Some(mut pending) = self.pending_scripts.remove(&view) else { return };
        while let Some(frame) = pending.frames.pop() {
            if let ReplayFrame::Script(script) = &frame { self.release_script_frame(view, script); }
            if let Some(target) = self.views.get_mut(&view) {
                if matches!(&frame, ReplayFrame::Mapping { plan, .. } if plan.restore_select) {
                    target.commands.finish_select_mapping();
                }
                if matches!(&frame, ReplayFrame::ExNormal { plan, .. } if plan.global) {
                    target.commands.global_replay_depth -= 1;
                }
                target.commands.end_replay_frame();
            }
        }
    }

    /// Acknowledge exactly the outstanding native request. A stale token cannot
    /// cancel or resume a newer script. Closing the view discards its frames.
    pub fn resume_script(&mut self, view: ViewId, token: u64, success: bool,
        clipboard: Option<ClipboardCommandContext>) -> Result<CoreOutcome, CoreError> {
        if !self.views.contains_key(&view) { return Err(CoreError::UnknownView(view)); }
        if self.pending_scripts.get(&view).is_none_or(|pending| pending.token != token) {
            return Err(CoreError::StaleScriptContinuation);
        }
        let initial = CoreOutcome { command: Some(crate::command::mappings::empty_mapping_output()),
            document_changed: false, position_map: None, layout_changed: false, composition_changes: Vec::new() };
        if !success {
            self.cancel_pending_script(view);
            self.install_buffer_commands(view);
            // Cancellation completes the same Normal command as a successful
            // replay. Finish Ctrl-O's deferred return and reopen its typing
            // undo/restoration group without executing abandoned frames.
            return self.run_compound_replay_state(view, clipboard, initial, None, None);
        }
        let pending = self.pending_scripts.remove(&view).expect("validated continuation");
        self.install_buffer_commands(view);
        self.run_compound_replay_state(view, clipboard, initial, None, Some(pending))
    }
}
