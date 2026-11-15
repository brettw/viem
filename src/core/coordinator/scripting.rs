//! Vim script execution inside the compound replay runner. A submitted
//! command line that needs the engine becomes a [`ScriptFrame`]; its host
//! commands and editor actions are staged on the view's command line and
//! dispatched as ordinary replay input, so layout retries, publication,
//! undo grouping and nested replays (`:normal`) behave as for mappings.
use super::*;
use crate::command::ex_execute::ExInfoRequest;
use crate::command::scripting::{fold_script_messages, script_line_source, split_set_command, status_error, ScriptAction, ScriptPlan};
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
    /// The failure of the nested replay (a `:normal`) this script waited for,
    /// delivered to the script when it resumes.
    pub(super) resume_failure: Option<CommandStatus>,
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
            resume_failure: None,
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
        if rt.text_locked() && matches!(action, ScriptAction::SetLines { .. } | ScriptAction::AppendLines { .. } | ScriptAction::DeleteLines { .. }) {
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
        if !self.messages.is_empty() && (host_action || application_action
            || outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).is_some_and(|ex|
                ex.frontend_requests.iter().any(|request| matches!(request, ExFrontendRequest::Info(ExInfoRequest::Message(_)))))) {
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

    fn cursor_target(&self, args: &[Value]) -> EvalResult<Option<(usize, usize, Option<usize>)>> {
        let items = match args.first() {
            Some(Value::List(list)) => list.snapshot(),
            _ => args.to_vec(),
        };
        if items.len() < 2 { return Ok(None); }
        let line = self.line_argument(items.first())?;
        let column = match &items[1] {
            Value::String(text) if text.as_bytes() == b"$" => usize::MAX as i64,
            value => value.to_number()?,
        };
        if line < 0 || column < 0 && !matches!(&items[1], Value::String(text) if text.as_bytes() == b"$") {
            return Err(VimError::new(475, format!("Invalid argument: {}", line.min(column))).into());
        }
        let want = items.get(3).map(Value::to_number).transpose()?.filter(|want| *want >= 0).map(|want| want.saturating_sub(1).max(0) as usize);
        let column = if matches!(&items[1], Value::String(text) if text.as_bytes() == b"$") { usize::MAX }
            else if column == 0 { self.commands().script_cursor_column(&self.core.document).max(1) as usize }
            else { column as usize };
        Ok(Some((line as usize, column, want)))
    }

    /// Line values for `setline()` and `append()`, as source text.
    fn lines_argument(value: Option<&Value>, format: crate::document::FileFormat) -> EvalResult<Vec<String>> {
        let line = |value: &Value| -> Result<String, VimError> {
            let text = match value { Value::Dict(_) => Ok(value.echo_string()), _ => value.to_text() }?;
            Ok(script_line_source(&text, format))
        };
        Ok(match value {
            Some(Value::List(list)) => list.snapshot().iter().map(line).collect::<Result<_, _>>()?,
            Some(value) => vec![line(value)?],
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

    fn searchpair(&mut self, rt: &mut Runtime, args: &[Value], positions: bool) -> EvalResult<Value> {
        let text = |index| args.get(index).map(Value::to_text).transpose().map(|value| value.unwrap_or_default());
        let patterns = [text(0)?, text(1)?, text(2)?];
        let flags = text(3)?;
        if flags.chars().any(|flag| !"bcnrswWm".contains(flag)) || flags.contains('n') && flags.contains('s') {
            return Err(VimError::new(475, format!("Invalid argument: {flags}")).into());
        }
        let backward = flags.contains('b');
        let origin = (self.commands().script_cursor_line(&self.core.document) as usize,
            self.commands().script_cursor_column(&self.core.document) as usize);
        let stop = args.get(5).cloned().unwrap_or(0.into());
        let timeout = args.get(6).cloned().unwrap_or(0.into());
        let started = std::time::Instant::now();
        let timeout_ms = timeout.to_number()?.max(0) as u128;
        let mut depth = 1i64; let mut count = 0; let mut last = None;
        let mut visited = std::collections::BTreeSet::new();
        let result: EvalResult<()> = (|| {
            for iteration in 0..10_000 {
                if timeout_ms != 0 && started.elapsed().as_millis() >= timeout_ms { break; }
                let mut candidates = Vec::new();
                for (kind, pattern) in patterns.iter().enumerate() {
                    if pattern.is_empty() || kind == 1 && depth != 1 { continue; }
                    let mut search_flags = String::from("n");
                    for flag in flags.chars().filter(|flag| "bwW".contains(*flag)) { search_flags.push(flag); }
                    if iteration == 0 && flags.contains('c') { search_flags.push('c'); }
                    let value = self.search(rt, &[pattern.clone().into(), search_flags.into(), stop.clone(), timeout.clone()], true)?;
                    let Value::List(list) = value else { unreachable!() };
                    let items = list.snapshot();
                    let line = items[0].to_number()? as usize; let column = items[1].to_number()? as usize;
                    if line != 0 { candidates.push(((line, column), kind)); }
                }
                candidates.sort_by_key(|(position, _)| *position);
                let Some((position, kind)) = (if backward { candidates.pop() } else { candidates.into_iter().next() }) else { break };
                if !visited.insert(position) { break; }
                self.dispatch(rt, ScriptAction::SetCursor { line: position.0, column: position.1, want: None })?;
                let skip = match args.get(4) {
                    None | Some(Value::Number(0)) => false,
                    Some(Value::String(text)) if text.as_bytes().is_empty() => false,
                    Some(Value::Funcref(function)) => rt.call_funcref(self, function, Vec::new(), None)?.to_number()? != 0,
                    Some(value) => {
                        let sid = rt.current_sid();
                        rt.eval_expression(self, value.to_vstr()?.as_bytes(), sid)?.to_number()? != 0
                    }
                };
                if skip { continue; }
                let opens = kind == if backward { 2 } else { 0 };
                if opens { depth += 1; } else { depth -= 1; }
                if depth == 0 {
                    count += 1; last = Some(position);
                    if !flags.contains('r') { break; }
                    depth = 1;
                }
            }
            Ok(())
        })();
        if result.is_err() || flags.contains('n') || last.is_none() {
            self.dispatch(rt, ScriptAction::SetCursor { line: origin.0, column: origin.1, want: None })?;
        }
        result?;
        if !flags.contains('n') {
            if let Some((line, column)) = last {
                if flags.contains('s') {
                    let view = self.view_id;
                    self.core.views.get_mut(&view).expect("script view").commands.script_set_mark(
                        &self.core.document, '\'', origin.0, origin.1);
                }
                self.dispatch(rt, ScriptAction::SetCursor { line, column, want: None })?;
            }
        }
        Ok(if positions {
            let (line, column) = last.unwrap_or((0, 0)); Value::list(vec![(line as i64).into(), (column as i64).into()])
        } else { if flags.contains('m') { count } else { last.map_or(0, |position| position.0 as i64) }.into() })
    }

    fn searchcount(&mut self, args: &[Value]) -> EvalResult<Value> {
        let options = match args.first() { Some(Value::Dict(options)) => options.clone(), None => crate::vimscript::Dict::new(),
            _ => return Err(VimError::new(1206, "Dictionary required for argument 1").into()) };
        let pattern = options.get_str("pattern").map(|value| value.to_text()).transpose()?
            .or_else(|| self.commands().script_search_pattern().map(str::to_owned)).unwrap_or_default();
        if pattern.is_empty() { return Ok(Value::Dict(crate::vimscript::Dict::new())); }
        let ignorecase = self.ignorecase() && !(self.commands().script_smartcase() && pattern.chars().any(char::is_uppercase));
        let compiled = crate::vimscript::regex::compile(pattern.as_bytes(), ignorecase)?;
        let maxcount = options.get_str("maxcount").map(|value| value.to_number()).transpose()?.unwrap_or(99).max(0);
        let timeout = options.get_str("timeout").map(|value| value.to_number()).transpose()?.unwrap_or(40).max(0);
        let started = std::time::Instant::now();
        let mut cursor = (self.commands().script_cursor_line(&self.core.document), self.commands().script_cursor_column(&self.core.document));
        if let Some(Value::List(pos)) = options.get_str("pos") {
            let pos = pos.snapshot();
            if pos.len() < 2 { return Err(VimError::new(475, "Invalid argument: pos").into()); }
            cursor = (pos[0].to_number()?, pos[1].to_number()?);
        }
        let mut fuel = 50_000_000; let mut total = 0i64; let mut current = 0i64; let mut exact = 0; let mut incomplete = 0;
        'lines: for index in 0..self.core.document.physical_source_line_number_count() {
            let line = self.core.document.physical_line(index).map_err(|error| VimError::plain(error.to_string()))?;
            let text = line.content_text(); let mut start = 0;
            while let Some(found) = crate::vimscript::regex::find_at(&compiled, text, start, &mut fuel)? {
                total += 1;
                if (index as i64 + 1, found.start as i64 + 1) <= cursor { current = total; }
                if index as i64 + 1 == cursor.0 && (found.start as i64) < cursor.1 && cursor.1 <= found.end.max(found.start + 1) as i64 { exact = 1; }
                if maxcount != 0 && total > maxcount { incomplete = 2; break 'lines; }
                if timeout != 0 && started.elapsed().as_millis() >= timeout as u128 { incomplete = 1; break 'lines; }
                start = if found.end > found.start { found.end } else { found.start + text[found.start..].chars().next().map_or(1, char::len_utf8) };
                if start > text.len() { break; }
            }
        }
        Ok(Value::Dict(crate::vimscript::Dict::from_pairs([
            ("current".into(), current.into()), ("total".into(), total.into()), ("exact_match".into(), exact.into()),
            ("incomplete".into(), incomplete.into()), ("maxcount".into(), maxcount.into()),
        ])))
    }

    fn searchdecl(&mut self, rt: &mut Runtime, args: &[Value]) -> EvalResult<Value> {
        let identifier = args.first().map(Value::to_text).transpose()?.unwrap_or_default();
        if identifier.is_empty() { return Ok(1.into()); }
        let global = args.get(1).map(Value::to_number).transpose()?.unwrap_or(0) != 0;
        let thisblock = args.get(2).map(Value::to_number).transpose()?.unwrap_or(0) != 0;
        let line = self.commands().script_cursor_line(&self.core.document).max(1) as usize;
        let column = self.commands().script_cursor_column(&self.core.document).max(1) as usize;
        let mut first = 0;
        if !global {
            let mut closed = 0;
            'scope: for index in (0..line).rev() {
                let physical = self.core.document.physical_line(index).map_err(|error| VimError::plain(error.to_string()))?;
                let text = physical.content_text();
                if text.trim_start().starts_with("//") || text.trim_start().starts_with("/*") { continue; }
                let end = if index + 1 == line { column.saturating_sub(1).min(text.len()) } else { text.len() };
                for &byte in text.as_bytes()[..end].iter().rev() {
                    match byte {
                        b'}' => closed += 1,
                        b'{' if closed > 0 => closed -= 1,
                        b'{' => { first = index; if thisblock { break 'scope; } },
                        _ => {}
                    }
                }
            }
        }
        let escaped = identifier.replace('\\', "\\\\");
        let pattern = format!("\\V\\<{}\\>", escaped);
        let compiled = crate::vimscript::regex::compile(pattern.as_bytes(), self.ignorecase())?;
        let mut fuel = 50_000_000;
        for index in first..line {
            let physical = self.core.document.physical_line(index).map_err(|error| VimError::plain(error.to_string()))?;
            let text = physical.content_text();
            if text.trim_start().starts_with("//") || text.trim_start().starts_with("/*") { continue; }
            if let Some(found) = crate::vimscript::regex::find_at(&compiled, text, 0, &mut fuel)? {
                if index + 1 == line && found.start + 1 >= column { break; }
                self.dispatch(rt, ScriptAction::SetCursor { line: index + 1, column: found.start + 1, want: None })?;
                return Ok(0.into());
            }
        }
        Ok(1.into())
    }

    /// `search()` and `searchpos()`: Vim regex over physical source lines.
    fn search(&mut self, rt: &mut Runtime, args: &[Value], positions: bool) -> EvalResult<Value> {
        let mut pattern = args.first().map(|value| value.to_text()).transpose()?.unwrap_or_default();
        if pattern.is_empty() {
            pattern = self.commands().script_search_pattern().ok_or_else(|| VimError::new(35, "No previous regular expression"))?.to_owned();
        }
        let flags = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
        if flags.chars().any(|flag| !"bcenpswWz".contains(flag)) || flags.contains('n') && flags.contains('s') {
            return Err(VimError::new(475, format!("Invalid argument: {flags}")).into());
        }
        let stop = args.get(2).map(|value| value.to_number()).transpose()?.unwrap_or(0).max(0) as usize;
        let skip = match args.get(4) {
            Some(Value::String(text)) if text.as_bytes().iter().all(u8::is_ascii_whitespace) => None,
            Some(Value::Number(0)) | None => None,
            Some(skip) => Some(skip.clone()),
        };
        let backward = flags.contains('b');
        // Without `w` or `W`, 'wrapscan' decides, as for `/`.
        let wrap = if flags.contains('W') { false } else { flags.contains('w') || self.commands().script_wrapscan() };
        let accept_at_cursor = flags.contains('c');
        let to_end = flags.contains('e');
        let ignorecase = self.ignorecase() && !(self.commands().script_smartcase() && pattern.chars().any(char::is_uppercase));
        let compiled = crate::vimscript::regex::compile(pattern.as_bytes(), ignorecase)?;
        // Multiline patterns share one bounded normalized source window. The
        // legacy matcher already limits string inputs to 256 KiB; do not copy
        // a larger document or silently treat truncated context as a miss.
        let multiline = pattern.contains("\\n") || pattern.contains("\\_");
        let joined = if multiline {
            let mut text = String::new(); let mut starts = Vec::new();
            for index in 0..self.core.document.physical_source_line_number_count() {
                let (range, _) = self.core.document.physical_line_ranges(index).map_err(|error| VimError::plain(error.to_string()))?;
                if range.len() > 256 * 1024 || text.len() + range.len() > 256 * 1024 {
                    return Err(VimError::new(363, "Multiline search input budget exceeded").into());
                }
                let line = self.core.document.physical_line(index).map_err(|error| VimError::plain(error.to_string()))?;
                starts.push(text.len()); text.push_str(line.content_text());
                if index + 1 < self.core.document.physical_source_line_number_count() { text.push('\n'); }
            }
            Some((text, starts))
        } else { None };
        let (count, cursor_line, cursor_column) = {
            let document = &self.core.document;
            let commands = self.commands();
            let count = document.physical_source_line_number_count().max(1);
            let cursor_line = (commands.script_cursor_line(document).max(1) as usize).min(count);
            (count, cursor_line, commands.script_cursor_column(document).max(1) as usize)
        };
        let lines: Vec<usize> = if backward {
            (1..=cursor_line).rev().chain(if wrap { (cursor_line..=count).rev().collect::<Vec<_>>() } else { Vec::new() }).collect()
        } else {
            (cursor_line..=count).chain(if wrap { (1..=cursor_line).collect::<Vec<_>>() } else { Vec::new() }).collect()
        };
        let mut fuel = 50_000_000u64;
        let mut found = None;
        'lines: for (visit, line) in lines.into_iter().enumerate() {
            if stop > 0 && ((backward && line < stop) || (!backward && line > stop)) {
                break;
            }
            let local = self.core.document.physical_line(line - 1).map(|physical| physical.content_text().to_string()).unwrap_or_default();
            let text = joined.as_ref().map_or(local.as_str(), |(text, starts)| &text[starts[line - 1]..]);
            let mut matches = Vec::new();
            let mut start = 0;
            while let Some(found) = crate::vimscript::regex::find_at(&compiled, &text, start, &mut fuel)? {
                if found.start > local.len() { break; }
                let subpattern = found.groups.iter().skip(1).position(Option::is_some).map_or(1, |index| index + 2);
                matches.push((found.start..found.end, subpattern));
                start = if found.end > found.start { found.end } else { found.start + text[found.start..].chars().next().map_or(1, char::len_utf8) };
                if start > text.len() {
                    break;
                }
            }
            if backward {
                matches.reverse();
            }
            let first_visit = visit == 0;
            let wrapped_visit = !first_visit && line == cursor_line;
            let cursor = cursor_column - 1;
            for (range, subpattern) in matches {
                let column = if to_end { text[..range.end].char_indices().last().map_or(range.start, |(at, _)| at).max(range.start) } else { range.start };
                let acceptable = if first_visit {
                    if backward { column < cursor || (accept_at_cursor && column == cursor) } else { column > cursor || (accept_at_cursor && column == cursor) }
                } else if wrapped_visit {
                    if backward { column >= cursor } else { column <= cursor }
                } else {
                    true
                };
                if !acceptable {
                    continue;
                }
                if let Some(skip) = &skip {
                    // Vim evaluates the skip expression with the cursor on the match.
                    self.dispatch(rt, ScriptAction::SetCursor { line, column: column + 1, want: None })?;
                    let skipped = match skip {
                        Value::Funcref(funcref) => rt.call_funcref(self, funcref, Vec::new(), None),
                        other => {
                            let text = other.to_vstr()?;
                            let sid = rt.current_sid();
                            rt.eval_expression(self, text.as_bytes(), sid)
                        }
                    };
                    let skipped = match skipped.and_then(|value| Ok(value.to_number()?)) {
                        Ok(value) => value != 0,
                        Err(error) => {
                            // The error propagates and the cursor stays put.
                            self.dispatch(rt, ScriptAction::SetCursor { line: cursor_line, column: cursor_column, want: None })?;
                            return Err(error);
                        }
                    };
                    if skipped {
                        continue;
                    }
                }
                let (line, column) = if let Some((_, starts)) = &joined {
                    let offset = starts[line - 1] + column;
                    let row = starts.partition_point(|start| *start <= offset).saturating_sub(1);
                    (row + 1, offset - starts[row] + 1)
                } else { (line, column + 1) };
                found = Some((line, column, subpattern));
                break 'lines;
            }
        }
        let moved_for_skip = skip.is_some();
        let Some((line, column, subpattern)) = found else {
            if moved_for_skip {
                self.dispatch(rt, ScriptAction::SetCursor { line: cursor_line, column: cursor_column, want: None })?;
            }
            return Ok(if positions { Value::list(vec![0.into(); if flags.contains('p') { 3 } else { 2 }]) } else { Value::Number(0) });
        };
        if flags.contains('n') {
            if moved_for_skip {
                self.dispatch(rt, ScriptAction::SetCursor { line: cursor_line, column: cursor_column, want: None })?;
            }
        } else {
            if flags.contains('s') {
                let view = self.view_id;
                self.core.views.get_mut(&view).expect("script view").commands.script_set_mark(
                    &self.core.document, '\'', cursor_line, cursor_column);
            }
            self.dispatch(rt, ScriptAction::SetCursor { line, column, want: None })?;
        }
        Ok(if positions {
            let mut result = vec![(line as i64).into(), (column as i64).into()];
            if flags.contains('p') { result.push((subpattern as i64).into()); }
            Value::list(result)
        } else { Value::Number(if flags.contains('p') { subpattern as i64 } else { line as i64 }) })
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
            self.suppress_child_events = command.modifiers.noautocmd || !rt.automatic_events_allowed();
            let visual_range = if command.range.starts_with("'<") { self.visual_range.take() } else { None };
            let argument = if text == command.text { command.argument.clone() } else { text.split_once(' ').map_or(String::new(), |(_, argument)| argument.to_owned()) };
            let events = crate::command::scripting::command_events(command, &argument);
            self.dispatch(rt, ScriptAction::Command { text, name: command.name, bang: command.bang, argument, sid: command.sid, lnum: command.lnum, visual_range })?;
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
            for (event, matched) in events {
                if event == "FileType" {
                    let matched = self.commands().script_option(&self.core.document, "filetype")?.to_text()?;
                    rt.fire_event_now(self, event, &matched)?;
                } else {
                    rt.queue_event_name(event, matched);
                }
            }
            if self.suspend.is_some() {
                break;
            }
        }
        Ok(())
    }
    fn get_option(&mut self, name: &str, _scope: OptionScope) -> Result<Value, VimError> {
        self.commands().script_option(&self.core.document, name)
    }
    fn set_option(&mut self, rt: &mut Runtime, name: &str, scope: OptionScope, value: &Value) -> EvalResult<()> {
        let text = crate::command::options::set_command_for(self.commands(), &self.core.document, name, scope, value)?;
        let (command_name, argument) = text.split_once(' ').unwrap_or((text.as_str(), ""));
        let command_name = crate::vimscript::excmds::lookup(command_name);
        self.dispatch(rt, ScriptAction::Command { text: text.clone(), name: command_name, bang: false, argument: argument.to_owned(), sid: rt.current_sid(), lnum: 0, visual_range: None })?;
        if matches!(name, "filetype" | "ft") {
            let value = self.commands().script_option(&self.core.document, "filetype")?.to_text()?;
            rt.fire_event_now(self, "FileType", &value)?;
        }
        Ok(())
    }
    fn option_exists(&self, name: &str, working: bool) -> bool {
        crate::command::options::option_exists(name, working)
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        let view_id = self.view_id;
        let core = &mut *self.core;
        core.views.get_mut(&view_id).expect("script view").commands.script_register_variable(&core.document, name)
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
                    let position = match args.get(1) {
                        Some(Value::List(list)) if list.len() >= 3 => list.snapshot().into_iter().skip(1).collect::<Vec<_>>(),
                        _ => return Ok(Value::Number(-1)),
                    };
                    let mark = target.strip_prefix('\'').filter(|name| name.chars().count() == 1).and_then(|name| name.chars().next());
                    if let Some(mark) = mark {
                        let Some((line, column, _)) = self.cursor_target(&position)? else { return Ok(Value::Number(-1)) };
                        let column = if name == "setcharpos" {
                            self.commands().script_byte_column_for_character(&self.core.document, line, column)
                        } else {
                            column
                        };
                        let view_id = self.view_id;
                        let core = &mut *self.core;
                        let set = core.views.get_mut(&view_id).expect("script view").commands.script_set_mark(&core.document, mark, line, column);
                        if set {
                            core.publish_buffer_commands(view_id);
                        }
                        return Ok(Value::Number(if set { 0 } else { -1 }));
                    }
                    if target != "." {
                        return Ok(Value::Number(-1));
                    }
                    position
                } else {
                    args.to_vec()
                };
                let Some((line, column, want)) = self.cursor_target(&args)? else { return Ok(Value::Number(-1)) };
                let line = if line == 0 { self.commands().script_cursor_line(&self.core.document).max(1) as usize } else { line };
                let column = if matches!(name, "setcharpos" | "setcursorcharpos") {
                    self.commands().script_byte_column_for_character(&self.core.document, line, column)
                } else {
                    column
                };
                self.dispatch(rt, ScriptAction::SetCursor { line, column, want })?;
                Ok(Value::Number(0))
            })(),
            "winrestview" => (|| {
                let Some(Value::Dict(view)) = args.first() else { return Err(Fail::Error(VimError::new(1206, "Dictionary required for argument 1"))) };
                let current = self.commands().script_cursor_line(&self.core.document);
                let line = view.get_str("lnum").map(|value| value.to_number()).transpose()?.unwrap_or(current).max(1) as usize;
                let column = view.get_str("col").map(|value| value.to_number()).transpose()?.unwrap_or(0).max(0) as usize + 1;
                self.dispatch(rt, ScriptAction::SetCursor { line, column, want: None })?;
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
            "setline" | "append" | "setbufline" | "appendbufline" | "deletebufline" => (|| {
                if self.textlock() {
                    return Err(Fail::Error(VimError::new(565, "Not allowed to change text or change window")));
                }
                let buffered = matches!(name, "setbufline" | "appendbufline" | "deletebufline");
                if buffered && !crate::command::script_queries::script_buffer_matches(args.first(), Some(self.core.buffer_filename()))? {
                    return Ok(1.into());
                }
                let args = if buffered { &args[1..] } else { args };
                let line = self.line_argument(args.first())?;
                if name == "deletebufline" {
                    let last = if args.len() > 1 { self.line_argument(args.get(1))? } else { line };
                    let count = self.core.document.physical_source_line_number_count() as i64;
                    if line < 1 || line > count || last < line { return Ok(1.into()); }
                    self.dispatch(rt, ScriptAction::DeleteLines { first: line as usize, last: last.min(count) as usize })?;
                    return Ok(0.into());
                }
                let lines = Self::lines_argument(args.get(1), self.core.document.file_format())?;
                let count = self.core.document.physical_source_line_number_count() as i64;
                if matches!(name, "setline" | "setbufline") {
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
            "searchpair" | "searchpairpos" => self.searchpair(rt, args, name == "searchpairpos"),
            "searchcount" => self.searchcount(args),
            "searchdecl" => self.searchdecl(rt, args),
            "getbufvar" | "getwinvar" | "gettabvar" | "gettabwinvar"
            | "setbufvar" | "setwinvar" | "settabvar" => (|| {
                let set = name.starts_with("set");
                let tabwin = name == "gettabwinvar";
                let valid = if name.contains("buf") {
                    crate::command::script_queries::script_buffer_matches(args.first(), Some(self.core.buffer_filename()))?
                } else {
                    crate::command::script_queries::script_window_matches(args.first())?
                        && (!tabwin || crate::command::script_queries::script_window_matches(args.get(1))?)
                };
                let index = if tabwin { 2 } else { 1 };
                let variable = args.get(index).map(Value::to_text).transpose()?.unwrap_or_default();
                let value = args.get(index + 1).cloned().unwrap_or_else(|| "".into());
                if !valid { return Ok(if set { 0.into() } else { value }); }
                if let Some(option) = variable.strip_prefix('&') {
                    if set { self.set_option(rt, option, OptionScope::Local, &value)?; return Ok(0.into()); }
                    return Ok(self.get_option(option, OptionScope::Local).unwrap_or(value));
                }
                let dict = if name.contains("buf") { rt.state.buffer_vars().clone() }
                    else if name.contains("tab") && !tabwin { rt.state.tab_vars().clone() }
                    else { rt.state.window_vars(self.view_id.0) };
                if set {
                    let scope = if name.contains("buf") { "b" } else if name.contains("tab") { "t" } else { "w" };
                    rt.set_variable(self, format!("{scope}:{variable}").as_bytes(), value, false)?;
                    Ok(0.into())
                }
                else if variable.is_empty() { Ok(Value::Dict(dict)) }
                else { Ok(dict.get(variable.as_bytes()).unwrap_or(value)) }
            })(),
            "feedkeys" => (|| {
                let keys = args.first().map(|value| value.to_bytes()).transpose()?.unwrap_or_default();
                let mode = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let remap = !mode.contains('n');
                for key in crate::command::mappings::script_keys(&keys) {
                    self.fed_keys.push((InputEvent::Key(key), remap));
                }
                if mode.contains('i') {
                    let count = crate::command::mappings::script_keys(&keys).len();
                    let split = self.fed_keys.len().saturating_sub(count);
                    self.fed_keys.rotate_left(split);
                }
                if mode.contains('x') {
                    let events = std::mem::take(&mut self.fed_keys);
                    for (event, remap) in events {
                        if *self.dispatched_events >= self.core.compound_replay_event_limit {
                            return Err(VimError::plain("feedkeys event budget exceeded").into());
                        }
                        *self.dispatched_events += 1;
                        let planned = self.commands_mut().set_plan_compound_replay(false);
                        let suppressed = self.commands_mut().set_mapping_suppressed(!remap);
                        let outcome = self.core.dispatch_replay_input(self.view_id, event, self.clipboard);
                        self.commands_mut().set_mapping_suppressed(suppressed);
                        self.commands_mut().set_plan_compound_replay(planned);
                        let error = outcome.command.as_ref().and_then(|command| status_error(&command.status));
                        self.accumulator.merge(outcome).map_err(|error| VimError::plain(format!("{error:?}")))?;
                        if let Some(error) = error { return Err(error.into()); }
                    }
                    if !mode.contains('!') && matches!(self.commands().mode(), Mode::Insert | Mode::Replace) {
                        let outcome = self.core.dispatch_replay_input(self.view_id, InputEvent::Key(Key::Escape), self.clipboard);
                        self.accumulator.merge(outcome).map_err(|error| VimError::plain(format!("{error:?}")))?;
                    }
                }
                Ok(Value::Number(0))
            })(),
            _ => {
                let view_id = self.view_id;
                let core = &mut *self.core;
                let buffer = core.buffer_filename().to_owned();
                let buffer = (!buffer.is_empty()).then_some(buffer);
                let result = core.views.get_mut(&view_id).expect("script view").commands.script_function(&core.document, buffer.as_deref(), name, args);
                if name == "getwininfo" {
                    if let Some(Ok(Value::List(items))) = &result {
                        for item in items.snapshot() {
                            if let Value::Dict(item) = item { item.insert("variables", Value::Dict(rt.state.window_vars(view_id.0))); }
                        }
                    }
                }
                // Like `:let @r`, a function that changes registers or history
                // publishes it, so a later command in the same run sees it.
                if matches!(name, "setreg" | "histadd" | "histdel") && result.as_ref().is_some_and(Result::is_ok) {
                    core.publish_buffer_commands(view_id);
                }
                return result;
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

    /// A command inside replays a script started (its `:normal`) failed.
    /// Like Vim, end only those replays and let the script handle the error;
    /// without a script frame the caller stops the whole replay.
    pub(super) fn fail_into_script(
        &mut self,
        view_id: ViewId,
        clipboard: Option<&ClipboardCommandContext>,
        frames: &mut Vec<ReplayFrame>,
        accumulator: &mut CoreOutcomeAccumulator,
        status: CommandStatus,
    ) -> Result<Option<CommandStatus>, CoreError> {
        let Some(script) = frames.iter().rposition(|frame| matches!(frame, ReplayFrame::Script(_))) else {
            return Ok(Some(status));
        };
        if script + 1 == frames.len() {
            return Ok(Some(status));
        }
        while frames.len() > script + 1 {
            let frame = frames.pop();
            if matches!(&frame, Some(ReplayFrame::ExNormal { plan, .. }) if plan.global) {
                self.views.get_mut(&view_id).expect("replay view").commands.global_replay_depth -= 1;
            }
            self.release_replay_frame(view_id, frame);
            self.views.get_mut(&view_id).expect("replay view").commands.end_replay_frame();
        }
        self.queued_replay = None;
        if self.views.get(&view_id).expect("replay view").commands.replay_needs_abort() {
            let cleanup = self.dispatch_replay_input(view_id, InputEvent::Key(Key::Escape), clipboard);
            accumulator.merge(cleanup)?;
        }
        accumulator.command_mut().status = CommandStatus::Complete;
        if let Some(ReplayFrame::Script(frame)) = frames.last_mut() {
            frame.resume_failure = Some(status);
        }
        Ok(None)
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
        let resume_failure = frame.resume_failure.take();
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
        if let Some(error) = resume_failure.as_ref().and_then(status_error) {
            host.last_failure = Some((error.text(), resume_failure.clone().unwrap_or(CommandStatus::Complete)));
            runtime.fail_suspended_command(&mut host, Fail::Error(error));
        }
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
                register: None,
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
        self.focus_view(view);
        if !self.views.contains_key(&view) { return Err(CoreError::UnknownView(view)); }
        if self.pending_scripts.get(&view).is_none_or(|pending| pending.token != token) {
            return Err(CoreError::StaleScriptContinuation);
        }
        let initial = CoreOutcome {
 command: Some(crate::command::mappings::empty_mapping_output()),
            ..CoreOutcome::default()
        };
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
