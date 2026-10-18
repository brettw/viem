//! Bridge between the command interpreter and the Vim script engine
//! (`crate::vimscript`). Every Ex command line, `:source`d file and
//! `startup.viem` runs through the engine; commands the engine does not
//! implement come back here as host commands and execute through the
//! ordinary Ex machinery.
use super::*;
use super::ex_execute::ExInfoRequest;
use crate::vimscript::{self, EvalResult, Fail, HostCommand, Message, OptionScope, Runtime, ScriptHost, ScriptState, VimError};
use crate::vimscript::value::Value;

/// One command (or editor action) staged by a running script for the
/// interpreter's command-line machinery.
#[derive(Clone, Debug)]
pub(crate) enum ScriptAction {
    Command {
        text: String,
        /// Full Vim command name, as resolved by the engine.
        name: Option<&'static str>,
        bang: bool,
        argument: String,
        /// Script id for `<SID>` in mappings.
        sid: usize,
        visual_range: Option<ex_addresses::ResolvedExRange>,
    },
    /// Move the cursor to a one-based physical source line and byte column.
    SetCursor { line: usize, column: usize },
    /// Replace physical source lines starting at `first` (one-based); lines
    /// beyond the end are appended.
    SetLines { first: usize, lines: Vec<String> },
    /// Insert lines after physical line `after` (zero inserts at the top).
    AppendLines { after: usize, lines: Vec<String> },
}

/// Immutable TextYankPost payload, retained with its replay frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct YankEvent {
    pub contents: Vec<String>, pub register: String, pub kind: String,
    pub visual: bool, pub inclusive: bool,
}
impl YankEvent {
    pub(crate) fn value(&self) -> Value {
        let contents = crate::vimscript::List::new(self.contents.iter().map(|text| Value::from(text.clone())).collect());
        contents.data().lock = crate::vimscript::value::Lock::Fixed;
        let dict = crate::vimscript::Dict::from_pairs([
            ("regcontents".into(), Value::List(contents)),
            ("regname".into(), Value::from(self.register.clone())), ("regtype".into(), Value::from(self.kind.clone())),
            ("operator".into(), Value::from("y")), ("inclusive".into(), Value::Number(self.inclusive as i64)),
            ("visual".into(), Value::Number(self.visual as i64)),
        ]);
        { let mut data = dict.data(); data.lock = crate::vimscript::value::Lock::Fixed;
          for item in data.items.values_mut() { item.lock = crate::vimscript::value::Lock::Fixed; } }
        Value::Dict(dict)
    }
}

/// Script plan emitted by a submitted command line for the coordinator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScriptPlan {
    pub text: String,
    pub visual_range: Option<ex_addresses::ResolvedExRange>,
    /// Command-line text to add to Ex history when the script succeeds.
    pub history: Option<String>,
    /// Autocommand events to fire instead of running `text`, in order, each
    /// with the name its patterns match (a file name or file type).
    pub events: Vec<(String, String)>,
    pub yank: Option<YankEvent>,
}

/// Rejected before command/option publication: native callbacks cannot await
/// another host operation while the caller is synchronously preparing a save.
pub(crate) fn native_request_error() -> CommandOutput {
    CommandOutput {
        status: CommandStatus::Error("E565: Native host and application commands are not allowed in this synchronous callback".into()),
        ..CommandOutput::complete()
    }
}

/// Convert a host command's failure into a Vim error.
pub(crate) fn status_error(status: &CommandStatus) -> Option<VimError> {
    Some(match status {
        CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled | CommandStatus::NeedsMoreLayout(_) => return None,
        CommandStatus::SearchNotFound => VimError::new(486, "Pattern not found"),
        CommandStatus::Error(message) | CommandStatus::Unsupported(message) => vim_error_from_text(message),
        CommandStatus::ExError(ExCommandError::Parse(error)) => match &error.kind {
            ex::ExParseErrorKind::UnknownCommand(command) => VimError::new(492, format!("Not an editor command: {command}")),
            _ => vim_error_from_text(&error.to_string()),
        },
        CommandStatus::ExError(ExCommandError::Execute(error)) => vim_error_from_text(&error.to_string()),
        CommandStatus::CountError(error) => VimError::plain(format!("{error:?}")),
        CommandStatus::RegisterReadError(error) => VimError::plain(format!("{error:?}")),
        CommandStatus::RegisterWriteError(error) => VimError::plain(format!("{error:?}")),
        CommandStatus::VisualBlockError(error) => VimError::plain(format!("{error:?}")),
    })
}

/// "E123: message" text keeps its number.
pub(crate) fn vim_error_from_text(text: &str) -> VimError {
    if let Some(rest) = text.strip_prefix('E') {
        if let Some((code, message)) = rest.split_once(": ") {
            if let Ok(code) = code.parse() {
                return VimError::new(code, message.to_string());
            }
        }
    }
    VimError::plain(text.to_string())
}

/// Split `:set a b c` into one command per argument, so earlier arguments
/// stay applied when a later one fails, as in Vim.
pub(crate) fn split_set_command(command: &HostCommand) -> Vec<String> {
    let name = command.name.unwrap_or("");
    if !matches!(name, "set" | "setlocal" | "setglobal") || command.argument.trim().is_empty() {
        return vec![command.text.clone()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = command.argument.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|next| *next == '\\' || next.is_whitespace()) {
            current.push(c);
            current.push(chars.next().unwrap());
        } else if c.is_whitespace() {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    let bang = if command.bang { "!" } else { "" };
    parts.into_iter().map(|part| format!("{name}{bang} {part}")).collect()
}

/// Whether a `:set` command (one argument) changes `fileformat` or
/// `fileformats`, which startup must not do before files open.
fn changes_file_policy(name: Option<&str>, text: &str) -> bool {
    if !matches!(name, Some("set" | "setlocal" | "setglobal")) {
        return false;
    }
    let Some((_, argument)) = text.split_once(char::is_whitespace) else { return false };
    let argument = argument.trim();
    let option = argument
        .trim_start_matches("no")
        .trim_start_matches("inv")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    // `fileformat` and `fileformats` are string options: a bare name shows
    // the value, like `name?`.
    let query = argument.ends_with('?') || (argument == option && option != "bomb");
    !query && ["fileformat", "ff", "fileformats", "ffs", "fileencoding", "fenc", "bomb"].iter().any(|name| option.eq_ignore_ascii_case(name))
}

/// Autocommand events a successful host command fires: `:colorscheme`
/// (ColorScheme) and file type changes (FileType), matched against the
/// scheme or file type name.
pub(crate) fn command_events(command: &HostCommand) -> Vec<(&'static str, String)> {
    if command.modifiers.noautocmd { return Vec::new(); }
    let argument = command.argument.trim();
    match command.name {
        Some("colorscheme") if !argument.is_empty() => vec![("ColorScheme", argument.to_owned())],
        Some("setfiletype") if !argument.is_empty() => vec![("FileType", argument.to_owned())],
        Some("set" | "setlocal" | "setglobal") => argument
            .split_whitespace()
            .filter_map(|part| part.split_once('='))
            .filter(|(name, _)| matches!(*name, "ft" | "filetype"))
            .map(|(_, value)| ("FileType", value.to_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Escape a value for a `:set` argument.
pub(crate) fn set_argument_value(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c == ' ' || c == '\\' || c == '|' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl CommandInterpreter {
    pub(crate) fn has_staged_script_action(&self) -> bool { self.script_action.is_some() }

    pub(crate) fn script_state(&self) -> &ScriptState {
        &self.script
    }

    /// Stage one host command or editor action from a running script on
    /// the command line; the caller then dispatches Enter.
    pub(crate) fn queue_script_action(&mut self, action: ScriptAction) -> Result<(), String> {
        if self.substitute_confirmation.is_some() {
            return Err("finish substitute confirmation before running script commands".into());
        }
        if self.mode == Mode::CommandLine {
            self.command_line_state = None;
            self.mode = Mode::Normal;
        }
        if !matches!(self.mode, Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock | Mode::Insert | Mode::Replace)
        {
            return Err("scripts run from Normal mode".into());
        }
        let return_mode = self.mode;
        self.enter_command_line(CommandLineKind::Ex);
        if let Some(state) = self.command_line_state.as_mut() {
            state.return_mode = return_mode;
            if let ScriptAction::Command { text, visual_range, .. } = &action {
                state.buffer.input = text.clone();
                state.buffer.cursor = text.len();
                state.retained_visual_range = visual_range.clone();
            }
        }
        self.script_action = Some(action);
        Ok(())
    }

    /// Effective value of an option for `&name` reads.
    pub(crate) fn script_option(&self, document: &Document, name: &str) -> Result<Value, VimError> {
        if let Some(value) = super::options::read_option(self, document, name) {
            return Ok(value);
        }
        Err(VimError::new(113, format!("Unknown option: {name}")))
    }

    pub(crate) fn script_register(&mut self, document: &Document, name: char) -> Result<Value, VimError> {
        match name {
            ':' => return Ok(Value::String(self.last_command_line.clone().unwrap_or_default().into())),
            '/' => return Ok(Value::String(self.last_search.as_ref().map(|(_, pattern)| pattern.clone()).unwrap_or_default().into())),
            '%' => return Ok(Value::str("")),
            _ => {}
        }
        match self.read_register_value_for_put(document, name) {
            Ok(Some(value)) => Ok(Value::String(register_text(&value).into())),
            Ok(None) => Ok(Value::str("")),
            Err(error) => Err(VimError::plain(format!("{error:?}"))),
        }
    }

    pub(crate) fn set_script_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError> {
        let (text, kind) = match value {
            Value::List(list) => {
                let lines: Vec<String> = list.snapshot().iter().map(|item| item.to_text()).collect::<Result<_, _>>()?;
                (lines.join("\n") + "\n", RegisterKind::Linewise)
            }
            other => {
                let text = other.to_text()?;
                let kind = if text.ends_with('\n') { RegisterKind::Linewise } else { RegisterKind::Characterwise };
                (text, kind)
            }
        };
        let name = if append { name.to_ascii_uppercase() } else { name };
        let value = RegisterValue::try_new(&text, kind, Vec::new()).map_err(|error| VimError::plain(format!("{error:?}")))?;
        self.write_script_register(name, value).map_err(|error| VimError::plain(format!("{error:?}")))
    }

    pub(crate) fn script_ignorecase(&self) -> bool {
        self.search_options.ignorecase
    }

    /// Physical one-based line of the cursor.
    pub(crate) fn script_cursor_line(&self, document: &Document) -> i64 {
        document
            .physical_line_at_text(self.cursor.min(document.projection().text_tree().byte_len()))
            .map_or(1, |line| line.index as i64 + 1)
    }

    /// One-based byte column of the cursor in its physical line.
    pub(crate) fn script_cursor_column(&self, document: &Document) -> i64 {
        let at = self.cursor.min(document.projection().text_tree().byte_len());
        let Ok(line) = document.physical_line_at_text(at) else { return 1 };
        let source = document.projection().source_insertion_point(at, true).unwrap_or(line.source_range.start);
        (source.saturating_sub(line.source_range.start) as i64) + 1
    }

    fn script_line_text(&self, document: &Document, line: i64) -> Option<String> {
        if line < 1 {
            return None;
        }
        let count = document.physical_source_line_number_count();
        if line as usize > count {
            return None;
        }
        document.physical_line(line as usize - 1).ok().map(|line| line.text.trim_end_matches(['\n', '\r']).to_string())
    }

    /// Resolve a `line()`-style position: ".", "$", "'x", "v", or a number.
    fn script_position_line(&self, document: &Document, value: &Value) -> Result<i64, VimError> {
        if let Value::Number(number) = value {
            return Ok(*number);
        }
        let text = value.to_text()?;
        let count = document.physical_source_line_number_count() as i64;
        Ok(match text.as_str() {
            "." => self.script_cursor_line(document),
            "$" => count,
            "w0" => 1,
            "w$" => count,
            "v" => self.visual_anchor.map_or_else(|| self.script_cursor_line(document), |anchor| {
                document.physical_line_at_text(anchor.min(document.projection().text_tree().byte_len())).map_or(1, |line| line.index as i64 + 1)
            }),
            mark if mark.starts_with('\'') => {
                let name = mark.chars().nth(1).unwrap_or(' ');
                match self.marks.get(&name) {
                    Some(offset) => document.physical_line_at_text((*offset).min(document.projection().text_tree().byte_len())).map_or(0, |line| line.index as i64 + 1),
                    None => 0,
                }
            }
            other => other.parse().unwrap_or(0),
        })
    }

    fn script_mode(&self) -> &'static str {
        match self.mode {
            Mode::Normal => "n",
            Mode::Insert => "i",
            Mode::Replace => "R",
            Mode::VisualCharacter => "v",
            Mode::VisualLine => "V",
            Mode::VisualBlock => "\x16",
            Mode::CommandLine => "c",
        }
    }

    /// Read-only editor builtins shared by every host.
    pub(crate) fn script_function(&mut self, document: &Document, buffer_name: Option<&str>, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
        let number = |index: usize| -> EvalResult<i64> { Ok(args.get(index).map(|value| value.to_number()).transpose()?.unwrap_or(0)) };
        Some(match name {
            "line" => args.first().map_or(Ok(Value::Number(0)), |value| Ok(Value::Number(self.script_position_line(document, value)?))),
            "col" | "virtcol" | "charcol" => (|| {
                let Some(position) = args.first() else { return Ok(Value::Number(0)) };
                let text = position.to_text().unwrap_or_default();
                Ok(Value::Number(match text.as_str() {
                    "." => self.script_cursor_column(document),
                    "$" => self.script_line_text(document, self.script_cursor_line(document)).map_or(1, |line| line.len() as i64 + 1),
                    _ => 0,
                }))
            })(),
            "getline" => (|| {
                let Some(first) = args.first() else { return Err(Fail::Error(VimError::argument(119, "getline"))) };
                let first = self.script_position_line(document, first)?;
                match args.get(1) {
                    None => Ok(Value::String(self.script_line_text(document, first).unwrap_or_default().into())),
                    Some(last) => {
                        let last = self.script_position_line(document, last)?;
                        let mut lines = Vec::new();
                        let mut line = first.max(1);
                        while line <= last {
                            match self.script_line_text(document, line) {
                                Some(text) => lines.push(Value::String(text.into())),
                                None => break,
                            }
                            line += 1;
                        }
                        Ok(Value::list(lines))
                    }
                }
            })(),
            "getpos" | "getcurpos" | "getcharpos" => (|| {
                let (line, column) = if name == "getcurpos" || args.first().is_some_and(|value| value.to_text().is_ok_and(|text| text == ".")) {
                    (self.script_cursor_line(document), self.script_cursor_column(document))
                } else if let Some(position) = args.first() {
                    (self.script_position_line(document, position)?, 1)
                } else {
                    (self.script_cursor_line(document), self.script_cursor_column(document))
                };
                let mut items = vec![Value::Number(0), line.into(), column.into(), Value::Number(0)];
                if name == "getcurpos" {
                    items.push(column.into());
                }
                Ok(Value::list(items))
            })(),
            "indent" => (|| {
                let line = match args.first() {
                    Some(value) => self.script_position_line(document, value)?,
                    None => return Err(Fail::Error(VimError::argument(119, "indent"))),
                };
                Ok(Value::Number(match self.script_line_text(document, line) {
                    Some(text) => {
                        let tabstop = self.indentation.effective().tabstop.max(1) as i64;
                        let mut width = 0i64;
                        for c in text.chars() {
                            match c {
                                ' ' => width += 1,
                                '\t' => width += tabstop - width % tabstop,
                                _ => break,
                            }
                        }
                        width
                    }
                    None => -1,
                }))
            })(),
            "nextnonblank" | "prevnonblank" => (|| {
                let mut line = match args.first() {
                    Some(value) => self.script_position_line(document, value)?,
                    None => 0,
                };
                let count = document.physical_source_line_number_count() as i64;
                while line >= 1 && line <= count {
                    if self.script_line_text(document, line).is_some_and(|text| !text.trim().is_empty()) {
                        return Ok(Value::Number(line));
                    }
                    line += if name == "nextnonblank" { 1 } else { -1 };
                }
                Ok(Value::Number(0))
            })(),
            "mode" => Ok(Value::str(self.script_mode())),
            "visualmode" => Ok(Value::str(match self.last_visual.as_ref().map(|visual| visual.mode) {
                Some(Mode::VisualLine) => "V",
                Some(Mode::VisualBlock) => "\x16",
                Some(_) => "v",
                None => "",
            })),
            "bufnr" | "winnr" | "tabpagenr" | "winbufnr" | "bufwinnr" | "tabpagewinnr" => Ok(Value::Number(1)),
            "win_getid" | "bufwinid" => Ok(Value::Number(1000)),
            "bufname" | "expand_file" => Ok(Value::String(buffer_name.unwrap_or("").into())),
            "bufexists" | "buflisted" | "bufloaded" => Ok(Value::Number(1)),
            "changenr" => Ok(Value::Number(document.history_status().current.change.as_u64() as i64)),
            "wordcount" => (|| {
                let text = document.text();
                let words = text.split_whitespace().count() as i64;
                Ok(Value::Dict(vimscript::Dict::from_pairs([
                    ("words".into(), words.into()),
                    ("chars".into(), (text.chars().count() as i64).into()),
                    ("bytes".into(), (text.len() as i64).into()),
                ])))
            })(),
            "line2byte" => (|| {
                let line = number(0)?;
                if line < 1 || line as usize > document.physical_source_line_number_count() + 1 {
                    return Ok(Value::Number(-1));
                }
                Ok(Value::Number(match document.physical_line(line as usize - 1) {
                    Ok(physical) => physical.source_range.start as i64 + 1,
                    Err(_) => document.source_byte_len() as i64 + 1,
                }))
            })(),
            "byte2line" => (|| {
                let byte = number(0)?;
                if byte < 1 {
                    return Ok(Value::Number(-1));
                }
                Ok(Value::Number(document.physical_line_index_at_source((byte as usize - 1).min(document.source_byte_len())).map_or(-1, |index| index as i64 + 1)))
            })(),
            "getreg" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or('"');
                let as_list = args.get(2).map(|value| value.to_bool()).transpose()?.unwrap_or(false);
                let value = self.script_register(document, register)?;
                if as_list {
                    let text = value.to_text()?;
                    let mut lines: Vec<Value> = text.split('\n').map(Value::from).collect();
                    if text.ends_with('\n') {
                        lines.pop();
                    }
                    return Ok(Value::list(lines));
                }
                Ok(value)
            })(),
            "getregtype" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or('"');
                Ok(Value::str(match self.read_register_value_for_put(document, register) {
                    Ok(Some(value)) => match value.kind {
                        RegisterKind::Linewise => "V",
                        RegisterKind::Blockwise => "\x16",
                        RegisterKind::Characterwise => "v",
                    },
                    _ => "",
                }))
            })(),
            "setreg" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or('"');
                let value = args.get(1).cloned().unwrap_or_default();
                let append = args.get(2).map(|value| value.to_text()).transpose()?.is_some_and(|options| options.contains('a'));
                self.set_script_register(register, &value, append).map_err(Fail::Error)?;
                Ok(Value::Number(0))
            })(),
            "hlexists" | "hlID" | "synID" | "synIDtrans" | "foldlevel" | "pumvisible" | "complete_check" | "getcmdwintype"
            | "reg_recording" | "reg_executing" | "getchangelist" | "haslocaldir" => Ok(match name {
                "reg_recording" => Value::String(self.recording.as_ref().map(|(name, _)| name.to_string()).unwrap_or_default().into()),
                "reg_executing" | "getcmdwintype" => Value::str(""),
                "getchangelist" => Value::list(vec![Value::list(Vec::new()), 0.into()]),
                _ => Value::Number(0),
            }),
            "foldclosed" | "foldclosedend" => Ok(Value::Number(-1)),
            "synIDattr" | "getcmdline" | "getcmdtype" | "state" => Ok(Value::str("")),
            "getcmdpos" => Ok(Value::Number(0)),
            "histget" => (|| {
                let history = match args.first().map(|value| value.to_text()).transpose()?.as_deref() {
                    Some("/" | "?" | "search" | "s") => &self.search_history,
                    _ => &self.ex_history,
                };
                let index = args.get(1).map(|value| value.to_number()).transpose()?.unwrap_or(-1);
                let item = if index < 0 { history.len().checked_sub(index.unsigned_abs() as usize) } else { (index as usize).checked_sub(1) };
                Ok(Value::String(item.and_then(|i| history.get(i)).cloned().unwrap_or_default().into()))
            })(),
            "histnr" => Ok(Value::Number(self.ex_history.len() as i64)),
            "histadd" => (|| {
                let entry = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                match args.first().map(|value| value.to_text()).transpose()?.as_deref() {
                    Some("/" | "?" | "search" | "s") => push_history(&mut self.search_history, entry, self.vim_options.number("history") as usize),
                    _ => push_history(&mut self.ex_history, entry, self.vim_options.number("history") as usize),
                }
                Ok(Value::Number(1))
            })(),
            "histdel" => {
                self.ex_history.clear();
                Ok(Value::Number(1))
            }
            "maparg" | "mapcheck" | "hasmapto" => Ok(self.mappings.script_query(name, args)),
            "winheight" | "winwidth" => Ok(Value::Number(if name == "winheight" { 40 } else { 80 })),
            "winline" | "wincol" => Ok(Value::Number(1)),
            "getbufvar" | "getwinvar" | "gettabvar" | "gettabwinvar" => (|| {
                let variable = args.get(if name == "gettabwinvar" { 2 } else { 1 }).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let default = args.get(if name == "gettabwinvar" { 3 } else { 2 }).cloned();
                if let Some(option) = variable.strip_prefix('&') {
                    return Ok(self.script_option(document, option).ok().or(default).unwrap_or_else(|| Value::str("")));
                }
                let dict = match name {
                    "getbufvar" => self.script.buffer_vars().clone(),
                    "gettabvar" => self.script.tab_vars().clone(),
                    _ => self.script.window_vars(0),
                };
                if variable.is_empty() {
                    return Ok(Value::Dict(dict));
                }
                Ok(dict.get(variable.as_bytes()).or(default).unwrap_or_else(|| Value::str("")))
            })(),
            "setbufvar" | "setwinvar" | "settabvar" => (|| {
                let variable = args.get(1).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let value = args.get(2).cloned().unwrap_or_default();
                if variable.starts_with('&') {
                    return Err(Fail::Error(VimError::plain("use :setlocal or let &option to change options")));
                }
                let dict = match name {
                    "setbufvar" => self.script.buffer_vars().clone(),
                    "settabvar" => self.script.tab_vars().clone(),
                    _ => self.script.window_vars(0),
                };
                dict.insert(variable, value);
                Ok(Value::Number(0))
            })(),
            "getbufline" | "getbufoneline" => (|| {
                let first = args.get(1).map(|value| self.script_position_line(document, value)).transpose()?.unwrap_or(1);
                let last = args.get(2).map(|value| self.script_position_line(document, value)).transpose()?.unwrap_or(first);
                if name == "getbufoneline" {
                    return Ok(Value::String(self.script_line_text(document, first).unwrap_or_default().into()));
                }
                let mut lines = Vec::new();
                let mut line = first.max(1);
                while line <= last {
                    match self.script_line_text(document, line) {
                        Some(text) => lines.push(Value::String(text.into())),
                        None => break,
                    }
                    line += 1;
                }
                Ok(Value::list(lines))
            })(),
            "expand" => (|| {
                let text = args.first().map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let word = self.script_word_under_cursor(document, text.starts_with("<cWORD>"));
                Ok(Value::String(match text.as_str() {
                    "<cword>" | "<cWORD>" | "<cexpr>" => word,
                    "<cfile>" => self.script_word_under_cursor(document, true),
                    _ => String::new(),
                }.into()))
            })(),
            "__viem_resolve_range" => (|| {
                let range = args.first().map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let (first, last) = self.script_resolve_range(document, &range).map_err(Fail::Error)?;
                Ok(Value::list(vec![first.into(), last.into()]))
            })(),
            _ => return None,
        })
    }

    fn script_word_under_cursor(&self, document: &Document, big: bool) -> String {
        let line = self.script_cursor_line(document);
        let Some(text) = self.script_line_text(document, line) else { return String::new() };
        let column = (self.script_cursor_column(document) - 1).max(0) as usize;
        let is_word = |c: char| if big { !c.is_whitespace() } else { c.is_alphanumeric() || c == '_' };
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let Some(index) = chars.iter().position(|(offset, c)| *offset >= column && is_word(*c)).or_else(|| chars.iter().rposition(|(offset, c)| *offset <= column && is_word(*c))) else {
            return String::new();
        };
        let mut start = index;
        while start > 0 && is_word(chars[start - 1].1) {
            start -= 1;
        }
        let mut end = index;
        while end + 1 < chars.len() && is_word(chars[end + 1].1) {
            end += 1;
        }
        chars[start..=end].iter().map(|(_, c)| c).collect()
    }

    /// Resolve an Ex range to one-based physical lines for `:call` and user
    /// commands. Numbers, ".", "$", "%", marks and simple offsets.
    pub(crate) fn script_resolve_range(&self, document: &Document, range: &str) -> Result<(i64, i64), VimError> {
        let count = document.physical_source_line_number_count() as i64;
        let current = self.script_cursor_line(document);
        if range.trim() == "%" {
            return Ok((1, count));
        }
        let resolve = |part: &str| -> Result<i64, VimError> {
            let part = part.trim();
            if part.is_empty() {
                return Ok(current);
            }
            let (base, rest) = match part.chars().next().unwrap() {
                '.' => (current, &part[1..]),
                '$' => (count, &part[1..]),
                '\'' => {
                    let name = part.chars().nth(1).unwrap_or(' ');
                    let line = if name == '<' || name == '>' {
                        let offset = self.marks.get(&name).copied();
                        offset.and_then(|offset| document.physical_line_at_text(offset.min(document.projection().text_tree().byte_len())).ok()).map(|line| line.index as i64 + 1)
                    } else {
                        self.marks.get(&name).and_then(|offset| document.physical_line_at_text((*offset).min(document.projection().text_tree().byte_len())).ok()).map(|line| line.index as i64 + 1)
                    };
                    (line.ok_or_else(|| VimError::new(20, "Mark not set"))?, &part[name.len_utf8() + 1..])
                }
                c if c.is_ascii_digit() => {
                    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
                    (digits.parse().unwrap_or(0), &part[digits.len()..])
                }
                '+' | '-' => (current, part),
                _ => return Err(VimError::new(14, format!("Invalid address: {part}"))),
            };
            let mut value = base;
            let mut chars = rest.chars().peekable();
            while let Some(sign) = chars.next() {
                let digits: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_digit)).collect();
                let amount = if digits.is_empty() { 1 } else { digits.parse().unwrap_or(0) };
                match sign {
                    '+' => value += amount,
                    '-' => value -= amount,
                    _ => {}
                }
            }
            Ok(value)
        };
        let (first, last) = match range.split_once([',', ';']) {
            Some((first, last)) => (resolve(first)?, resolve(last)?),
            None => {
                let line = resolve(range)?;
                (line, line)
            }
        };
        if first < 0 || last > count.max(1) {
            return Err(VimError::new(16, "Invalid range"));
        }
        Ok((first.min(last).max(1), first.max(last)))
    }

    /// Dynamic variables: `v:count`, `v:count1`, `v:register`, `b:changedtick`, ...
    pub(crate) fn script_dynamic_variable(&self, document: &Document, name: &[u8]) -> Option<Value> {
        Some(match name {
            b"v:count" | b"count" => Value::Number(self.script_count.unwrap_or(0) as i64),
            b"v:count1" => Value::Number(self.script_count.unwrap_or(1).max(1) as i64),
            b"v:prevcount" => Value::Number(0),
            b"v:register" => Value::String(self.requested_register.unwrap_or('"').to_string().into()),
            b"v:hlsearch" => Value::Number((self.search_options.hlsearch && !self.search_highlight_suppressed) as i64),
            b"v:searchforward" => Value::Number(self.last_search.as_ref().is_none_or(|(direction, _)| *direction == SearchDirection::Forward) as i64),
            b"v:insertmode" => Value::str(if self.mode == Mode::Replace { "r" } else { "i" }),
            b"b:changedtick" => Value::Number(document.revision().0 as i64),
            b"<cursor-line>" => Value::Number(self.script_cursor_line(document)),
            _ => return None,
        })
    }
}

/// Insert `lines` after physical line `after` (zero inserts at the top),
/// supplying the delimiter a final unterminated line needs.
fn append_source_lines(document: &mut Document, after: usize, lines: &[String], ending: &str) -> Result<(), DocumentError> {
    if lines.is_empty() {
        return Ok(());
    }
    let body = lines.join(ending);
    if after == 0 {
        return document.replace_physical_source(0..0, format!("{body}{ending}")).map(|_| ());
    }
    let count = document.physical_source_line_number_count();
    let physical = document.physical_line(after.min(count).max(1) - 1)?;
    if physical.source_range.end > physical.content_range.end {
        let at = physical.source_range.end;
        document.replace_physical_source(at..at, format!("{body}{ending}")).map(|_| ())
    } else {
        let at = physical.content_range.end;
        document.replace_physical_source(at..at, format!("{ending}{body}")).map(|_| ())
    }
}

fn register_text(value: &RegisterValue) -> String {
    let mut text = value.text.clone();
    if value.kind == RegisterKind::Linewise && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Runs scripts directly on one interpreter without the coordinator:
/// `startup.viem` configuration and headless interpreter use (tests).
pub(crate) struct InterpreterHost<'a> {
    pub commands: &'a mut CommandInterpreter,
    pub document: &'a mut Document,
    pub messages: Vec<Message>,
    pub buffer_name: Option<String>,
    /// Outputs of the executed host commands merged in order.
    pub merged: Option<CommandOutput>,
    /// The error text and status of the last failing host command.
    pub last_failure: Option<(String, CommandStatus)>,
    /// Running `startup.viem`: the file-opening policy cannot change and
    /// `has('vim_starting')` is true.
    pub startup: bool,
    /// Evaluating an `<expr>` mapping: commands cannot change text.
    pub textlock: bool,
}

impl<'a> InterpreterHost<'a> {
    pub(crate) fn new(commands: &'a mut CommandInterpreter, document: &'a mut Document) -> Self {
        Self { commands, document, messages: Vec::new(), buffer_name: None, merged: None, last_failure: None, startup: false, textlock: false }
    }
}

impl ScriptHost for InterpreterHost<'_> {
    fn execute(&mut self, rt: &mut Runtime, command: &HostCommand) -> EvalResult<()> {
        if (self.textlock || rt.text_locked()) && !matches!(command.name, Some("set" | "setlocal" | "setglobal" | "nohlsearch")) && !command.name.is_some_and(mappings::is_mapping_command) {
            return Err(Fail::Error(VimError::new(565, "Not allowed to change text or change window")));
        }
        if self.startup && command.name == Some("nohlsearch") {
            self.commands.search_highlight_suppressed = true;
            return Ok(());
        }
        if command.name == Some("colorscheme") && !command.argument.trim().is_empty() && !command.modifiers.noautocmd {
            rt.fire_event_now(self, "ColorSchemePre", command.argument.trim())?;
        }
        for text in split_set_command(command) {
            if self.startup && changes_file_policy(command.name, &text) {
                return Err(Fail::Error(VimError::plain("startup cannot change source encoding, BOM, fileformat or the fileformats opening policy")));
            }
            let mut output = self.commands.execute_host_command(self.document, &text, command);
            if rt.messages_silenced() {
                if let Some(outcome) = output.ex_outcome.as_mut() {
                    outcome.frontend_requests.retain(|request| !matches!(request, ExFrontendRequest::Info(ExInfoRequest::Message(_))));
                }
            }
            let error = status_error(&output.status);
            if let Some(error) = &error {
                self.last_failure = Some((error.text(), output.status.clone()));
                output.status = CommandStatus::Complete;
            }
            match self.merged.as_mut() {
                Some(merged) => merged.merge(output),
                None => self.merged = Some(output),
            }
            if let Some(error) = error {
                return Err(Fail::Error(error));
            }
        }
        for (event, matched) in command_events(command) {
            rt.queue_event_name(event, matched);
        }
        Ok(())
    }
    fn get_option(&mut self, name: &str, _scope: OptionScope) -> Result<Value, VimError> {
        self.commands.script_option(self.document, name)
    }
    fn set_option(&mut self, rt: &mut Runtime, name: &str, scope: OptionScope, value: &Value) -> EvalResult<()> {
        let text = super::options::set_command_for(self.commands, self.document, name, scope, value)?;
        let (verb, argument) = text.split_once(' ').unwrap_or((&text, ""));
        let command = HostCommand { text: text.clone(), name: crate::vimscript::excmds::lookup(verb),
            bang: false, range: String::new(), argument: argument.into(), modifiers: Default::default(), sid: rt.current_sid() };
        self.execute(rt, &command)
    }
    fn option_exists(&self, name: &str, working: bool) -> bool {
        super::options::option_exists(name, working)
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        self.commands.script_register(self.document, name)
    }
    fn set_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError> {
        self.commands.set_script_register(name, value, append)
    }
    fn call_function(&mut self, _rt: &mut Runtime, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
        let buffer_name = self.buffer_name.clone();
        self.commands.script_function(self.document, buffer_name.as_deref(), name, args)
    }
    fn has_function(&self, name: &str) -> bool {
        SCRIPT_FUNCTIONS.contains(&name)
    }
    fn textlock(&self) -> bool {
        self.textlock
    }
    fn feature(&self, name: &str) -> Option<bool> {
        if name == "vim_starting" {
            return Some(self.startup);
        }
        script_feature(name)
    }
    fn command_exists(&self, name: &str) -> bool {
        host_command_exists(name)
    }
    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
    fn ignorecase(&self) -> bool {
        self.commands.search_options.ignorecase
    }
    fn dynamic_variable(&self, name: &[u8]) -> Option<Value> {
        self.commands.script_dynamic_variable(self.document, name)
    }
    fn buffer_name(&self) -> Option<String> {
        self.buffer_name.clone()
    }
}

/// Editor builtins the command layer provides.
pub(crate) const SCRIPT_FUNCTIONS: &[&str] = &[
    "line", "col", "virtcol", "charcol", "getline", "getpos", "getcurpos", "getcharpos", "indent", "nextnonblank",
    "prevnonblank", "mode", "visualmode", "bufnr", "winnr", "tabpagenr", "winbufnr", "bufwinnr", "tabpagewinnr",
    "win_getid", "bufwinid", "bufname", "bufexists", "buflisted", "bufloaded", "changenr", "wordcount", "line2byte",
    "byte2line", "getreg", "getregtype", "setreg", "hlexists", "hlID", "synID", "synIDtrans", "foldlevel",
    "pumvisible", "complete_check", "getcmdwintype", "reg_recording", "reg_executing", "getchangelist",
    "foldclosed", "foldclosedend", "synIDattr", "getcmdline", "getcmdtype", "state", "getcmdpos", "histget",
    "histnr", "histadd", "histdel", "maparg", "mapcheck", "hasmapto", "winheight", "winwidth", "winline", "wincol",
    "getbufvar", "getwinvar", "gettabvar", "gettabwinvar", "setbufvar", "setwinvar", "settabvar", "getbufline",
    "getbufoneline", "cursor", "setpos", "setline", "append", "winsaveview", "winrestview", "search", "searchpos",
    "feedkeys", "setcharpos", "setcursorcharpos",
];

/// Host answers for `has()`.
pub(crate) fn script_feature(name: &str) -> Option<bool> {
    match name {
        "gui_running" | "gui" | "viem" => Some(true),
        _ => None,
    }
}

/// Whether the host implements the full Vim command `name`.
pub(crate) fn host_command_exists(name: &str) -> bool {
    super::options::HOST_COMMANDS.contains(&name)
        || ex::is_supported_command(name)
        || mappings::is_mapping_command(name)
        || abbreviations::is_abbreviation_command(name)
}

impl CommandInterpreter {
    /// A mapping command by full name; a listing becomes a message.
    pub(crate) fn execute_mapping_command(&mut self, name: &str, bang: bool, argument: &str, sid: usize) -> CommandOutput {
        self.mapping_pending.clear();
        match self.mappings.execute_command(name, bang, argument, sid) {
            Ok(None) => CommandOutput::complete(),
            Ok(Some(listing)) => {
                let mut outcome = ExOutcome::default();
                outcome.frontend_requests.push(ExFrontendRequest::Info(ExInfoRequest::Message(listing)));
                CommandOutput { ex_outcome: Some(outcome), ..CommandOutput::complete() }
            }
            Err(message) => CommandOutput { status: CommandStatus::Error(message), ..CommandOutput::complete() },
        }
    }

    /// Execute one host command from a script directly (no coordinator).
    pub(crate) fn execute_host_command(&mut self, document: &mut Document, text: &str, command: &HostCommand) -> CommandOutput {
        if let Some(name) = command.name.filter(|name| mappings::is_mapping_command(name)) {
            return self.execute_mapping_command(name, command.bang, &command.argument, command.sid);
        }
        if let Some(name) = command.name.filter(|name| abbreviations::is_abbreviation_command(name)) {
            return self.execute_abbreviation_command(name, &command.argument, command.sid);
        }
        if let Some(output) = super::options::execute_script_only_command(self, document, command.name, &command.argument) {
            return output;
        }
        let saved = self.plan_compound_replay;
        self.plan_compound_replay = false;
        let output = self.execute_ex_command(document, text);
        self.plan_compound_replay = saved;
        output
    }

    pub(crate) fn set_plan_compound_replay(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.plan_compound_replay, enabled)
    }

    /// Add a successful script command line to the Ex history.
    pub(crate) fn push_script_history(&mut self, entry: String) {
        push_history(&mut self.ex_history, entry, self.vim_options.number("history") as usize);
    }

    /// Run a submitted Ex command line. A single plain command executes
    /// directly; anything else runs through the script engine, planned for
    /// the coordinator when it owns compound execution.
    pub(super) fn submit_ex_line(
        &mut self,
        document: &mut Document,
        command: &str,
        visual_range: Option<&ex_addresses::ResolvedExRange>,
    ) -> CommandOutput {
        if let Some(host) = vimscript::commands::single_host_command(&self.script, command) {
            if let Some(output) = super::options::execute_script_only_command(self, document, host.name, &host.argument) {
                return output;
            }
            return self.execute_ex_command_with_visual_range(document, &host.text, visual_range);
        }
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::Script(ScriptPlan {
                text: command.to_owned(),
                visual_range: visual_range.cloned(),
                history: (self.global_replay_depth == 0).then(|| command.to_owned()),
                events: Vec::new(),
                yank: None,
            }));
            return CommandOutput { mode_changed: true, ..CommandOutput::complete() };
        }
        self.run_script_headless(document, command)
    }

    /// Execute a host command or editor action staged by a running script.
    pub(super) fn execute_script_action(
        &mut self,
        document: &mut Document,
        action: ScriptAction,
        visual_range: Option<&ex_addresses::ResolvedExRange>,
    ) -> CommandOutput {
        match action {
            ScriptAction::Command { text, name, bang, argument, sid, .. } => {
                if let Some(name) = name.filter(|name| mappings::is_mapping_command(name)) {
                    return self.execute_mapping_command(name, bang, &argument, sid);
                }
                if let Some(name) = name.filter(|name| abbreviations::is_abbreviation_command(name)) {
                    return self.execute_abbreviation_command(name, &argument, sid);
                }
                if let Some(output) = super::options::execute_script_only_command(self, document, name, &argument) {
                    return output;
                }
                self.execute_ex_command_with_visual_range(document, &text, visual_range)
            }
            ScriptAction::SetCursor { line, column } => self.script_set_cursor(document, line, column),
            ScriptAction::SetLines { first, lines } => self.script_edit_lines(document, |document, ending| {
                let count = document.physical_source_line_number_count();
                for (index, text) in lines.iter().enumerate() {
                    let line = first - 1 + index;
                    if line < count {
                        let physical = document.physical_line(line)?;
                        document.replace_physical_source(physical.content_range, text.clone())?;
                    } else {
                        append_source_lines(document, line, std::slice::from_ref(text), ending)?;
                    }
                }
                Ok(())
            }),
            ScriptAction::AppendLines { after, lines } => {
                self.script_edit_lines(document, |document, ending| append_source_lines(document, after, &lines, ending))
            }
        }
    }

    /// `cursor()`, `setpos('.')` and `winrestview()`: a one-based physical
    /// line and byte column, clamped like Vim.
    fn script_set_cursor(&mut self, document: &Document, line: usize, column: usize) -> CommandOutput {
        let count = document.physical_source_line_number_count().max(1);
        let Ok(physical) = document.physical_line(line.clamp(1, count) - 1) else {
            return CommandOutput { status: CommandStatus::Error("E966: Invalid line number".into()), ..CommandOutput::complete() };
        };
        let mut column = column.saturating_sub(1).min(physical.text.len());
        while !physical.text.is_char_boundary(column) {
            column -= 1;
        }
        let source = (physical.content_range.start + column).min(physical.content_range.end);
        let Ok(cursor) = document.visible_point_for_source(source, true) else {
            return CommandOutput { status: CommandStatus::Error("cursor position is not editable".into()), ..CommandOutput::complete() };
        };
        let old = self.cursor;
        self.cursor = if matches!(self.mode, Mode::Normal) {
            normalize_normal_cursor_document(document, &document.hard_line_snapshot(), cursor)
        } else {
            cursor
        };
        self.boundary_affinity = BoundaryAffinity::Downstream;
        self.remember_physical_cursor(document, source);
        self.preferred_column = None;
        self.desired_x = None;
        CommandOutput { cursor_moved: self.cursor != old, ..CommandOutput::complete() }
    }

    /// Edit physical source lines, keeping the cursor on its line and
    /// column as Vim does.
    fn script_edit_lines(
        &mut self,
        document: &mut Document,
        edit: impl FnOnce(&mut Document, &str) -> Result<(), DocumentError>,
    ) -> CommandOutput {
        let line = self.script_cursor_line(document).max(1) as usize;
        let column = self.script_cursor_column(document).max(1) as usize;
        let before = document.revision();
        let ending = document.file_format().spelling();
        if let Err(error) = edit(document, ending) {
            return CommandOutput { status: CommandStatus::Error(error.to_string()), ..CommandOutput::complete() };
        }
        let changed = document.revision() != before;
        let mut output = self.script_set_cursor(document, line, column);
        output.document_changed = changed;
        output.cursor_moved = true;
        output
    }

    /// Run a command line through the script engine without the coordinator.
    pub(crate) fn run_script_headless(&mut self, document: &mut Document, text: &str) -> CommandOutput {
        let state = self.script.clone();
        let mut runtime = Runtime::new(state);
        runtime.push_command_line(text, 0);
        self.script_depth += 1;
        let mut host = InterpreterHost::new(self, document);
        runtime.run(&mut host);
        host.commands.script_depth -= 1;
        let mut output = host.merged.take().unwrap_or_else(CommandOutput::complete);
        let messages = std::mem::take(&mut host.messages);
        let failure = host.last_failure.take();
        fold_script_messages(&mut output, messages, failure);
        output
    }
}

/// Show a failed command line's error text in the frontend's output area:
/// only typed `ExError`s were converted there before, but Vim reports
/// every command-line error.
pub(crate) fn surface_error_text(output: &mut CommandOutput) {
    let (CommandStatus::Error(text) | CommandStatus::Unsupported(text)) = &output.status else { return };
    let text = text.clone();
    let requests = &mut output.ex_outcome.get_or_insert_with(Default::default).frontend_requests;
    if !requests.iter().any(|request| matches!(request, ExFrontendRequest::Info(ExInfoRequest::Message(existing)) if *existing == text)) {
        requests.push(ExFrontendRequest::Info(ExInfoRequest::Message(text)));
    }
}

/// Fold script messages into a command output: echo text becomes an
/// informational message and any error makes the command an error. When
/// the only message is the error of the last failing host command, that
/// command's own status (`failure`) is kept, so a single failing command
/// reports exactly as it would without the engine.
pub(crate) fn fold_script_messages(output: &mut CommandOutput, messages: Vec<Message>, failure: Option<(String, CommandStatus)>) {
    if let (Some((text, status)), [Message::Error(error)]) = (&failure, messages.as_slice()) {
        if error.error.text() == *text && error.file.is_none() && error.function.is_none() {
            output.status = status.clone();
            return;
        }
    }
    let mut text = String::new();
    let mut error = false;
    for message in messages {
        match message {
            Message::Echo(line) | Message::EchoMsg(line) | Message::Output(line) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&line);
            }
            Message::Echon(more) => text.push_str(&more),
            Message::Error(script_error) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&script_error.display());
                error = true;
            }
        }
    }
    if error {
        output.status = CommandStatus::Error(text);
        surface_error_text(output);
    } else if !text.is_empty() {
        output
            .ex_outcome
            .get_or_insert_with(Default::default)
            .frontend_requests
            .push(ExFrontendRequest::Info(ExInfoRequest::Message(text)));
    }
}
