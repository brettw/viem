//! Bridge between the command interpreter and the Vim script engine
//! (`crate::vimscript`). Every Ex command line, `:source`d file and
//! `startup.viem` runs through the engine; commands the engine does not
//! implement come back here as host commands and execute through the
//! ordinary Ex machinery.
use super::*;
use super::script_queries::script_window_matches;
use super::ex_execute::ExInfoRequest;
use crate::vimscript::{self, EvalResult, Fail, HostCommand, Message, OptionScope, Runtime, ScriptHost, ScriptState, VimError};
use crate::vimscript::value::Value;
use unicode_segmentation::UnicodeSegmentation;

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
        lnum: usize,
        visual_range: Option<ex_addresses::ResolvedExRange>,
    },
    /// Move the cursor to a one-based physical source line and byte column.
    SetCursor { line: usize, column: usize, want: Option<usize> },
    /// Replace physical source lines starting at `first` (one-based); lines
    /// beyond the end are appended.
    SetLines { first: usize, lines: Vec<String> },
    /// Insert lines after physical line `after` (zero inserts at the top).
    AppendLines { after: usize, lines: Vec<String> },
    DeleteLines { first: usize, last: usize },
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
            ("operator".into(), Value::from("y")), ("inclusive".into(), Value::Bool(self.inclusive)),
            ("visual".into(), Value::Bool(self.visual)),
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
pub(crate) fn command_events(command: &HostCommand, argument: &str) -> Vec<(&'static str, String)> {
    if command.modifiers.noautocmd { return Vec::new(); }
    let argument = argument.trim();
    match command.name {
        Some("colorscheme") if !argument.is_empty() => vec![("ColorScheme", argument.to_owned())],
        Some("setfiletype") if !argument.is_empty() => vec![("FileType", argument.to_owned())],
        Some("set" | "setlocal" | "setglobal") => argument.split_once('=')
            .filter(|(name, _)| matches!(*name, "ft" | "filetype"))
            .map(|(_, value)| vec![("FileType", value.to_owned())]).unwrap_or_default(),
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

    /// `@r` in an expression. Unlike `getreg('=')`, `@=` reads the expression
    /// itself rather than its result.
    pub(crate) fn script_register_variable(&mut self, document: &Document, name: char) -> Result<Value, VimError> {
        if name == '=' {
            return Ok(Value::String(self.last_expression.clone().unwrap_or_default().into()));
        }
        self.script_register(document, name)
    }

    pub(crate) fn script_register(&mut self, document: &Document, name: char) -> Result<Value, VimError> {
        match name {
            ':' => return Ok(Value::String(self.last_command_line.clone().unwrap_or_default().into())),
            '/' => return Ok(Value::String(self.last_search.as_ref().map(|(_, pattern)| pattern.clone()).unwrap_or_default().into())),
            '%' if document.artifact_binding().is_none() && self.current_artifact.is_none() => return Ok(Value::str("")),
            _ => {}
        }
        if let Some(value) = self.registers.get(name).filter(|value| std::str::from_utf8(value.script_bytes()).is_err()) {
            return Ok(Value::String(value.script_bytes().into()));
        }
        match self.read_register_value_for_put(document, name) {
            Ok(Some(value)) => Ok(Value::String(register_text(&value).into())),
            Ok(None) => Ok(Value::str("")),
            Err(error) => Err(VimError::plain(format!("{error:?}"))),
        }
    }

    /// `:let @r = value`. Unlike `setreg()`, a List is not a register value.
    pub(crate) fn set_script_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError> {
        if matches!(value, Value::List(_)) {
            return Err(VimError::new(730, "Using a List as a String"));
        }
        self.set_script_register_with_type(name, value, append, None, None)
    }

    /// `:let @r` and `setreg()`. `kind` is `setreg()`'s explicit type; without
    /// one, a List is linewise and a String is linewise only when it ends in
    /// LF or CR, as in Vim.
    fn set_script_register_with_type(
        &mut self,
        name: char,
        value: &Value,
        append: bool,
        kind: Option<RegisterKind>,
        width: Option<usize>,
    ) -> Result<(), VimError> {
        if name == '/' {
            self.last_search = Some((SearchDirection::Forward, value.to_text()?));
            self.search_highlight_suppressed = false;
            return Ok(());
        }
        if name == '=' {
            self.last_expression = Some(value.to_text()?);
            return Ok(());
        }
        // Scripts cannot write read-only or unknown registers.
        if !crate::command::registers::is_valid_register(name) || matches!(name, '.' | '%' | ':' | '#' | '~') {
            return Err(VimError::new(354, format!("Invalid register name: '{name}'")));
        }
        let append = append || name.is_ascii_uppercase();
        let name = name.to_ascii_lowercase();
        let empty_list = matches!(value, Value::List(list) if list.len() == 0);
        let result = if empty_list && !append {
            // Vim empties the register rather than storing an empty line.
            self.clear_script_register(name)
        } else if append {
            let combined = script_append_value(self.registers.get(name), value, kind, width)?;
            self.append_script_register(name, combined)
        } else {
            let value = script_register_value(value, kind)?;
            let value = if width.is_some() { value.with_block_width(width) } else { value };
            self.write_script_register(name, value)
        };
        result.map_err(|error| VimError::plain(error.to_string()))
    }

    /// `setreg()` with the Dictionary `getreginfo()` returns. Without
    /// `regcontents` the contents stay; `isunnamed` or the unnamed register's
    /// `points_to` make `"` refer to a register.
    fn set_script_register_info(&mut self, name: char, info: &vimscript::Dict) -> EvalResult<()> {
        if let Some(contents) = info.get(b"regcontents") {
            let options = match info.get(b"regtype") {
                Some(kind) => {
                    let kind = kind.to_text()?;
                    if !(kind.is_empty() || matches!(kind.as_str(), "v" | "V" | "c" | "l" | "b") || kind.starts_with('\u{16}') && kind[1..].bytes().all(|b| b.is_ascii_digit())) {
                        return Err(VimError::new(475, "Invalid value for argument value").into());
                    }
                    kind
                }
                None => String::new(),
            };
            let (append, kind, width) = setreg_options(&options).map_err(Fail::Error)?;
            self.set_script_register_with_type(name, &contents, append, kind, width).map_err(Fail::Error)?;
        }
        let target = name.to_ascii_lowercase();
        if name == '"' {
            if let Some(source) = info.get(b"points_to") {
                let source = source.to_text()?.chars().next().unwrap_or('"').to_ascii_lowercase();
                self.registers.point_unnamed(source);
            }
        } else if info.get(b"isunnamed").map(|value| value.to_bool()).transpose()?.unwrap_or(false) {
            self.registers.point_unnamed(target);
        }
        Ok(())
    }

    pub(crate) fn script_ignorecase(&self) -> bool {
        self.search_options.ignorecase
    }

    pub(crate) fn script_wrapscan(&self) -> bool {
        self.search_options.wrapscan
    }

    /// Physical one-based line of the cursor.
    pub(crate) fn script_cursor_line(&self, document: &Document) -> i64 {
        document
            .physical_line_at_text(self.cursor.min(document.projection().text_tree().byte_len()))
            .map_or(1, |line| line.index as i64 + 1)
    }

    /// The cursor's physical line and column.
    fn script_cursor_position(&self, document: &Document) -> Option<(i64, LineColumn)> {
        let at = self.cursor.min(document.projection().text_tree().byte_len());
        let line = document.physical_line_at_text(at).ok()?;
        let source = document.projection().source_insertion_point(at, true).unwrap_or(line.source_range.start);
        Some((line.index as i64 + 1, LineColumn::at_source(document, &line, source)))
    }

    /// One-based byte column of the cursor in its physical line, counted in
    /// UTF-8 as in Vim whatever the file's encoding.
    pub(crate) fn script_search_pattern(&self) -> Option<&str> { self.last_search.as_ref().map(|(_, pattern)| pattern.as_str()) }
    pub(crate) fn script_smartcase(&self) -> bool { self.search_options.smartcase }

    pub(crate) fn script_cursor_column(&self, document: &Document) -> i64 {
        self.script_cursor_position(document).map_or(1, |(_, column)| column.byte())
    }

    /// `setpos("'x", ...)`: set a mark at a one-based physical line and byte
    /// column. Unlike Vim, marks follow the text, so a line past the end
    /// fails and a column past the end of the line is clamped to it.
    pub(crate) fn script_set_mark(&mut self, document: &Document, name: char, line: usize, column: usize) -> bool {
        if !(marks::valid_mark(name) || name == '"') || matches!(name, '.' | '^') {
            return false;
        }
        let Some(index) = line.checked_sub(1).filter(|index| *index < document.physical_source_line_number_count()) else { return false };
        let Ok(physical) = document.physical_line(index) else { return false };
        let text = physical.content_text();
        let mut column = column.saturating_sub(1).min(text.len());
        while !text.is_char_boundary(column) {
            column -= 1;
        }
        let source = physical.content_range.start + document.encoding().encoded_text_len(&text[..column]);
        let Ok(offset) = document.visible_point_for_source(source.min(physical.content_range.end), true) else { return false };
        match name {
            '<' | '>' => {
                self.marks.insert(name, offset);
                self.set_line_end_mark(name, false);
                // Keep `gv` selecting what the marks now name.
                if let Some(memory) = self.last_visual.as_mut() {
                    let start_is_anchor = memory.anchor <= memory.active;
                    match (name, start_is_anchor) {
                        ('<', true) | ('>', false) => memory.anchor = offset,
                        _ => memory.active = offset,
                    }
                }
            }
            _ => self.set_named_mark_at(name, offset),
        }
        true
    }

    /// The one-based byte column of one-based character `column` on
    /// physical `line`, for `setcharpos()` and `setcursorcharpos()`.
    pub(crate) fn script_byte_column_for_character(&self, document: &Document, line: usize, column: usize) -> usize {
        let count = document.physical_source_line_number_count().max(1);
        let Ok(physical) = document.physical_line(line.clamp(1, count) - 1) else { return column };
        let text = physical.content_text();
        let skipped: usize = text.graphemes(true).take(column.saturating_sub(1)).map(str::len).sum();
        skipped + 1 + column.saturating_sub(1).saturating_sub(text.graphemes(true).count())
    }

    fn script_line_text(&self, document: &Document, line: i64) -> Option<String> {
        if line < 1 {
            return None;
        }
        let count = document.physical_source_line_number_count();
        if line as usize > count {
            return None;
        }
        document.physical_line(line as usize - 1).ok().map(|line| script_line_value(line.content_text(), document.file_format()))
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
                match self.mark_offset(name) {
                    Some(offset) => document.physical_line_at_text(offset).map_or(0, |line| line.index as i64 + 1),
                    None => 0,
                }
            }
            other => other.parse().unwrap_or(0),
        })
    }

    fn script_mark_position(&self, document: &Document, text: &str) -> Option<(i64, LineColumn)> {
        let name = text.chars().nth(1)?;
        let offset = self.mark_offset(name)?;
        let source = document.projection().source_insertion_point(offset, true)?;
        let line = document.physical_line(document.physical_line_index_at_source(source).ok()?).ok()?;
        let mut column = LineColumn::at_source(document, &line, source);
        // Vim ends a linewise Visual selection and a linewise yank at the
        // maximum column, which reads as the end of the line.
        if self.line_end_marks.contains(&name) {
            column.offset = column.text.len();
            column.line_end = true;
        }
        Some((line.index as i64 + 1, column))
    }

    /// The position `col()`, `getpos()` and their relatives read: `.`, `$`
    /// (the end of the cursor's line) or a mark.
    fn script_column_position(&self, document: &Document, text: &str) -> Option<(i64, LineColumn)> {
        match text {
            "." => self.script_cursor_position(document),
            "$" => self.script_cursor_position(document).map(|(line, mut column)| {
                column.offset = column.text.len();
                (line, column)
            }),
            mark if mark.starts_with('\'') => self.script_mark_position(document, mark),
            _ => None,
        }
    }

    fn script_tabstop(&self) -> usize {
        self.indentation.effective().tabstop.max(1) as usize
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
        if let Some(result) = self.script_buffer_query(document, buffer_name, name, args) { return Some(result); }
        let number = |index: usize| -> EvalResult<i64> { Ok(args.get(index).map(|value| value.to_number()).transpose()?.unwrap_or(0)) };
        Some(match name {
            "line" => (|| {
                let Some(value) = args.first() else { return Ok(0.into()) };
                if let Value::List(list) = value {
                    let list = list.snapshot();
                    let line = list.first().map(Value::to_number).transpose()?.unwrap_or(0);
                    return Ok(if list.len() >= 2 && line >= 1 && line as usize <= document.physical_source_line_number_count() { line } else { 0 }.into());
                }
                Ok(if matches!(value, Value::String(_)) {
                    let text = value.to_text()?;
                    if text.chars().next().is_some_and(|c| c.is_ascii_digit()) { 0.into() }
                    else { self.script_position_line(document, value)?.into() }
                } else { 0.into() })
            })(),
            "col" | "virtcol" | "charcol" => (|| {
                let Some(position) = args.first() else { return Ok(0.into()) };
                let resolved = match position {
                    Value::List(list) => {
                        let list = list.snapshot();
                        let line = list.first().map(Value::to_number).transpose()?.unwrap_or(0);
                        let Some(text) = self.script_line_text(document, line) else { return Ok(0.into()) };
                        let column = match list.get(1) {
                            Some(Value::String(value)) if value.as_bytes() == b"$" => if name == "charcol" { text.graphemes(true).count() as i64 + 1 } else { text.len() as i64 + 1 },
                            Some(value) => value.to_number()?,
                            None => 0,
                        };
                        if column < 1 || column as usize > text.len() + 1 { return Ok(0.into()) }
                        if name == "col" { return Ok(column.into()); }
                        if name == "charcol" { return Ok(if column as usize <= text.graphemes(true).count() + 1 { column } else { 0 }.into()); }
                        let mut offset = column as usize - 1;
                        while !text.is_char_boundary(offset) { offset -= 1; }
                        Some((line, LineColumn { text, offset, line_end: false }))
                    }
                    Value::String(_) => self.script_column_position(document, &position.to_text()?),
                    _ => return Err(VimError::new(1222, "String or List required for argument 1").into()),
                };
                Ok(Value::Number(resolved.map_or(0, |(_, column)| match name {
                    "col" => column.byte(), "charcol" => column.character(),
                    _ => column.virtual_end(self.script_tabstop()),
                })))
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
            "getpos" | "getcurpos" | "getcharpos" | "getcursorcharpos" => (|| {
                let current = matches!(name, "getcurpos" | "getcursorcharpos");
                if current && !script_window_matches(args.first())? {
                    return Ok(Value::list(vec![0.into(); 5]));
                }
                let text = match args.first() {
                    Some(position) if !current => position.to_text()?,
                    _ => ".".into(),
                };
                // A file mark names its buffer; Viem has only buffer 1.
                let buffer = i64::from(text.len() == 2 && text.starts_with('\'') && text.as_bytes()[1].is_ascii_uppercase());
                let (line, column) = match self.script_column_position(document, &text) {
                    Some((line, column)) if text != "$" => {
                        let number = match name {
                            _ if column.line_end && name != "getcharpos" => MAXIMUM_COLUMN,
                            // Vim reads the maximum column as the last character.
                            _ if column.line_end => column.character() - 1,
                            "getcharpos" | "getcursorcharpos" => column.character(),
                            _ => column.byte(),
                        };
                        (line, number)
                    }
                    _ if text.starts_with('\'') => (0, 0),
                    _ => match self.script_position_line(document, &Value::from(text.as_str()))? {
                        // An unknown position is all zeros.
                        0 => (0, 0),
                        line => (line, 1),
                    },
                };
                let buffer = if line == 0 { 0 } else { buffer };
                let mut items = vec![Value::Number(buffer), line.into(), column.into(), Value::Number(0)];
                if current {
                    let want = self.script_cursor_position(document).map_or(1, |(_, column)| column.cursor_want(self.script_tabstop()));
                    items.push(self.preferred_column.map_or(want, |column| if column == usize::MAX { MAXIMUM_COLUMN } else { column as i64 + 1 }).into());
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
            "changenr" => Ok(Value::Number(document.history_status().current.change.as_u64() as i64)),
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
                if byte < 1 || byte as usize > document.source_byte_len() {
                    return Ok(Value::Number(-1));
                }
                Ok(Value::Number(document.physical_line_index_at_source((byte as usize - 1).min(document.source_byte_len())).map_or(-1, |index| index as i64 + 1)))
            })(),
            "getreg" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next())
                    .unwrap_or_else(|| self.requested_register.or(self.script_register_context).unwrap_or('"'));
                let as_list = args.get(2).map(|value| value.to_bool()).transpose()?.unwrap_or(false);
                if register == '=' {
                    let value = if args.get(1).is_some_and(|value| value.to_bool().unwrap_or(false)) {
                        Value::from(self.last_expression.clone().unwrap_or_default())
                    } else { self.script_register(document, register)? };
                    return Ok(if as_list { Value::list(vec![value]) } else { value });
                }
                if as_list && !matches!(register, ':' | '/' | '%') {
                    // Preserve List items' literal LF bytes, and exact byte
                    // Strings even when no Unicode put projection exists.
                    let value = if let Some(value) = self.registers.get(register)
                        .filter(|value| std::str::from_utf8(value.script_bytes()).is_err()) {
                        Ok(Some(value.clone()))
                    } else { self.read_register_value_for_put(document, register) };
                    return Ok(match value {
                        Ok(Some(value)) => Value::list(register_lines(&value).into_iter().map(|bytes| Value::String(bytes.into())).collect()),
                        Ok(None) => Value::list(Vec::new()),
                        Err(error) => return Err(VimError::plain(format!("{error:?}")).into()),
                    });
                }
                let value = self.script_register(document, register)?;
                if as_list { return Ok(Value::list(vec![value])); }
                Ok(value)
            })(),
            "getregtype" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or_else(|| self.requested_register.or(self.script_register_context).unwrap_or('"'));
                if matches!(register, '%' | '#' | '=' | ':' | '/' | '.' | '_') {
                    // Vim reports these as characterwise even when empty.
                    return Ok(Value::str("v"));
                }
                Ok(Value::from(match self.read_register_value(document, register) {
                    Ok(Some(value)) => match value.kind {
                        RegisterKind::Linewise => "V".to_owned(),
                        RegisterKind::Blockwise => format!("\x16{}", value.block_width()),
                        RegisterKind::Characterwise => "v".to_owned(),
                    },
                    _ => String::new(),
                }))
            })(),
            "getreginfo" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or_else(|| self.requested_register.or(self.script_register_context).unwrap_or('"'));
                let register = register.to_ascii_lowercase();
                let present = match register {
                    '=' | '_' => true,
                    '/' => self.last_search.is_some(),
                    ':' => self.last_command_line.is_some(),
                    _ => matches!(self.read_register_value(document, register), Ok(Some(_))),
                };
                if !present {
                    return Ok(Value::Dict(vimscript::Dict::new()));
                }
                let name = Value::from(register.to_string());
                let contents = self.script_function(document, buffer_name, "getreg", &[name.clone(), 1.into(), 1.into()])
                    .unwrap_or_else(|| Ok(Value::list(Vec::new())))?;
                let kind = self.script_function(document, buffer_name, "getregtype", &[name])
                    .unwrap_or_else(|| Ok(Value::str("")))?;
                let source = self.registers.unnamed_source();
                let mut pairs = vec![("regcontents".into(), contents), ("regtype".into(), kind)];
                // The unnamed register says which register it points to; the
                // others say whether it points to them.
                if register == '"' {
                    if let Some(source) = source {
                        pairs.push(("points_to".into(), Value::from(source.to_string())));
                    }
                } else {
                    pairs.push(("isunnamed".into(), Value::Bool(source == Some(register))));
                }
                Ok(Value::Dict(vimscript::Dict::from_pairs(pairs)))
            })(),
            "setreg" => (|| {
                let register = args.first().map(|value| value.to_text()).transpose()?.and_then(|text| text.chars().next()).unwrap_or('"');
                if let Some(Value::Dict(info)) = args.get(1) {
                    return self.set_script_register_info(register, info).map(|()| Value::Number(0));
                }
                let value = args.get(1).cloned().unwrap_or_default();
                let options = args.get(2).map(|value| value.to_text()).transpose()?.unwrap_or_default();
                let (append, kind, width) = setreg_options(&options).map_err(Fail::Error)?;
                if let (Value::List(list), '/' | '=') = (&value, register) {
                    // These hold one line: a one-item List sets it and an
                    // empty List clears it.
                    let text = match list.snapshot().as_slice() {
                        [] => None,
                        [line] => Some(line.to_text()?),
                        _ => return Err(VimError::new(883, "Search pattern and expression register may not contain two or more lines").into()),
                    };
                    if register == '/' {
                        self.last_search = text.map(|pattern| (SearchDirection::Forward, pattern));
                        self.search_highlight_suppressed = false;
                    } else {
                        self.last_expression = text;
                    }
                    return Ok(Value::Number(0));
                }
                self.set_script_register_with_type(register, &value, append, kind, width).map_err(Fail::Error)?;
                Ok(Value::Number(0))
            })(),
            "hlexists" | "hlID" | "synID" | "synIDtrans" | "foldlevel" | "pumvisible" | "complete_check" | "getcmdwintype"
            | "reg_recording" | "reg_executing" | "getchangelist" | "haslocaldir" => Ok(match name {
                "reg_recording" => Value::String(self.recording.as_ref().map(|(name, _)| name.to_string()).unwrap_or_default().into()),
                "reg_executing" => Value::from(self.executing_macro.map(|name| name.to_string()).unwrap_or_default()),
                "getcmdwintype" => Value::str(""),
                "getchangelist" => {
                    let lines = document.hard_line_snapshot();
                    let (changes, index) = self.change_list();
                    let items = changes.iter().map(|offset| {
                        let offset = (*offset).min(lines.text_length());
                        let line = document.physical_line_at_text(offset).map_or(0, |line| line.index as i64 + 1);
                        let column = lines.line_at_offset(offset).map_or(0, |line| offset - line.content_range().start.min(offset));
                        Value::Dict(vimscript::Dict::from_pairs([
                            ("lnum".into(), line.into()),
                            ("col".into(), (column as i64).into()),
                            ("coladd".into(), 0.into()),
                        ]))
                    }).collect();
                    Value::list(vec![Value::list(items), (index as i64).into()])
                }
                _ => Value::Number(0),
            }),
            "foldclosed" | "foldclosedend" => Ok(Value::Number(-1)),
            "synIDattr" | "getcmdline" | "getcmdtype" | "state" => Ok(Value::str("")),
            "getcmdpos" => Ok(Value::Number(0)),
            "maparg" | "mapcheck" | "hasmapto" => Ok(self.mappings.script_query(name, args)),
            "getbufvar" | "getwinvar" | "gettabvar" | "gettabwinvar" => (|| {
                let valid = if name == "getbufvar" { super::script_queries::script_buffer_matches(args.first(), buffer_name)? }
                    else { super::script_queries::script_window_matches(args.first())? };
                if !valid { return Ok(args.get(if name == "gettabwinvar" { 3 } else { 2 }).cloned().unwrap_or_else(|| "".into())); }
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
        if range.trim() == "*" { return self.script_resolve_range(document, "'<,'>"); }
        if range.trim() == "%" {
            return Ok((1, count));
        }
        let resolve = |part: &str, current: i64| -> Result<i64, VimError> {
            let part = part.trim();
            if part.is_empty() {
                return Ok(current);
            }
            let (base, rest) = match part.chars().next().unwrap() {
                '.' => (current, &part[1..]),
                '$' => (count, &part[1..]),
                '\'' => {
                    let name = part.chars().nth(1).unwrap_or(' ');
                    let line = self.mark_offset(name)
                        .and_then(|offset| document.physical_line_at_text(offset).ok())
                        .map(|line| line.index as i64 + 1);
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
            Some((first, last)) => {
                let first = resolve(first, current)?;
                let last_current = if range.as_bytes().get(range.find([',', ';']).unwrap()) == Some(&b';') { first } else { current };
                (first, resolve(last, last_current)?)
            }
            None => {
                let line = resolve(range, current)?;
                (line, line)
            }
        };
        if first < 0 || last > count.max(1) {
            return Err(VimError::new(16, "Invalid range"));
        }
        Ok((first.min(last).max(1), first.max(last)))
    }

    /// Dynamic variables: `v:count`, `v:count1`, `v:register`, `b:changedtick`, ...
    pub(crate) fn script_requested_register(&self) -> Option<char> {
        self.requested_register
    }

    pub(crate) fn script_dynamic_variable(&self, document: &Document, name: &[u8]) -> Option<Value> {
        Some(match name {
            b"v:count" | b"count" => Value::Number(self.script_count.unwrap_or(0) as i64),
            b"v:count1" => Value::Number(self.script_count.unwrap_or(1).max(1) as i64),
            b"v:prevcount" => Value::Number(0),
            b"v:register" => Value::String(self.requested_register.or(self.script_register_context).unwrap_or('"').to_string().into()),
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
    let count = document.physical_source_line_number_count();
    if after == 0 {
        // An empty file's only line is empty and unterminated; inserted lines
        // before it would turn it into the end of the file, so it needs an
        // ending to remain.
        let first = document.physical_line(0)?;
        let keep = if count == 1 && first.text.is_empty() { ending } else { "" };
        return document.replace_physical_source_literal(first.source_range.start..first.source_range.start, &format!("{body}{ending}{keep}")).map(|_| ());
    }
    let physical = document.physical_line(after.min(count).max(1) - 1)?;
    if physical.source_range.end > physical.content_range.end {
        let at = physical.source_range.end;
        document.replace_physical_source_literal(at..at, &format!("{body}{ending}")).map(|_| ())
    } else {
        // After an unterminated last line the new lines stay unterminated,
        // but an empty last line needs its ending to remain a line, as for
        // `o` at the end of such a file.
        let at = physical.content_range.end;
        let keep = if lines.last().is_some_and(String::is_empty) { ending } else { "" };
        document.replace_physical_source_literal(at..at, &format!("{ending}{body}{keep}")).map(|_| ())
    }
}

/// `setreg()` options: `a` appends and `c`/`v`, `l`/`V` or `b`/CTRL-V (with
/// an optional width, which block registers derive from their rows) choose the
/// type. `u` is accepted and, as in Vim, leaves the unnamed register alone.
/// A block width is kept for `getregtype()`.
fn setreg_options(options: &str) -> Result<(bool, Option<RegisterKind>, Option<usize>), VimError> {
    let mut append = false;
    let mut kind = None;
    let mut width = None;
    let mut characters = options.chars().peekable();
    while let Some(option) = characters.next() {
        match option {
            'a' => append = true,
            'u' => {}
            'c' | 'v' => kind = Some(RegisterKind::Characterwise),
            'l' | 'V' => kind = Some(RegisterKind::Linewise),
            'b' | '\u{16}' => {
                kind = Some(RegisterKind::Blockwise);
                let mut digits = String::new();
                while let Some(digit) = characters.next_if(char::is_ascii_digit) {
                    digits.push(digit);
                }
                width = digits.parse().ok().filter(|width| *width > 0);
            }
            // Vim ignores options it does not know.
            _ => {}
        }
    }
    Ok((append, kind, width))
}

/// Vim separates register lines with line feeds. In a String every line feed
/// separates lines, so it is a semantic break; in a List each item is one
/// line and a line feed inside an item is that line's literal content.
fn script_register_value(value: &Value, kind: Option<RegisterKind>) -> Result<RegisterValue, VimError> {
    match value {
        Value::List(list) => {
            let lines: Vec<Vec<u8>> = list.snapshot().iter().map(Value::to_bytes).collect::<Result<_, _>>()?;
            register_from_lines(&lines, kind.unwrap_or(RegisterKind::Linewise))
        }
        other => {
            let mut bytes = other.to_bytes()?;
            let kind = kind.unwrap_or(if bytes.ends_with(b"\n") || bytes.ends_with(b"\r") { RegisterKind::Linewise } else { RegisterKind::Characterwise });
            if kind == RegisterKind::Blockwise && bytes.ends_with(b"\n") { bytes.pop(); }
            if kind == RegisterKind::Linewise && !bytes.ends_with(b"\n") { bytes.push(b'\n'); }
            Ok(RegisterValue::from_script_bytes(bytes, kind))
        }
    }
}

/// Each List item is literal; only the inserted delimiters are semantic breaks.
fn register_from_lines(lines: &[Vec<u8>], kind: RegisterKind) -> Result<RegisterValue, VimError> {
    if kind == RegisterKind::Blockwise && lines.iter().any(|line| line.contains(&b'\n')) {
        return Err(VimError::plain("E475: Invalid argument: a block row cannot contain a line feed"));
    }
    let mut bytes = Vec::new();
    let mut breaks = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            if kind != RegisterKind::Blockwise { breaks.push(bytes.len()); }
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(line);
    }
    if kind == RegisterKind::Linewise {
        breaks.push(bytes.len());
        bytes.push(b'\n');
    }
    RegisterValue::from_script_parts(bytes, kind, breaks).map_err(|error| VimError::plain(error.to_string()))
}

/// Script append takes the new value's type. Only a String appended to a
/// characterwise register continues the destination's last line.
fn script_append_value(existing: Option<&RegisterValue>, value: &Value, kind: Option<RegisterKind>, width: Option<usize>) -> Result<RegisterValue, VimError> {
    let incoming = script_register_value(value, kind)?;
    let kind = incoming.kind;
    let appended: Vec<Vec<u8>> = match value {
        Value::List(list) => list.snapshot().iter().map(Value::to_bytes).collect::<Result<_, _>>()?,
        other => {
            let bytes = other.to_bytes()?;
            let mut lines: Vec<Vec<u8>> = bytes.split(|byte| *byte == b'\n').map(<[u8]>::to_vec).collect();
            if kind != RegisterKind::Characterwise && bytes.ends_with(b"\n") { lines.pop(); }
            lines
        }
    };
    let width = width.or_else(|| (kind == RegisterKind::Blockwise).then(|| incoming.block_width()));
    let mut lines = existing.map(register_lines).unwrap_or_default();
    let mut appended = appended.into_iter();
    if existing.is_some_and(|existing| existing.kind == RegisterKind::Characterwise) && !matches!(value, Value::List(_)) {
        if let (Some(last), Some(first)) = (lines.last_mut(), appended.next()) { last.extend_from_slice(&first); }
    }
    lines.extend(appended);
    Ok(register_from_lines(&lines, kind)?.with_block_width(width))
}

/// Script byte rows split only at semantic delimiters or block separators.
fn register_lines(value: &RegisterValue) -> Vec<Vec<u8>> {
    let bytes = value.script_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && (value.kind == RegisterKind::Blockwise || value.script_hard_break_offsets().binary_search(&offset).is_ok()) {
            lines.push(bytes[start..offset].to_vec());
            start = offset + 1;
        }
    }
    if value.kind != RegisterKind::Linewise || start < bytes.len() { lines.push(bytes[start..].to_vec()); }
    lines
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
            let argument = if text == command.text { command.argument.as_str() } else { text.split_once(' ').map_or("", |(_, argument)| argument) };
            for (event, matched) in command_events(command, argument) {
                if event == "FileType" {
                    let matched = self.commands.script_option(self.document, "filetype")?.to_text()?;
                    rt.fire_event_now(self, event, &matched)?;
                } else {
                    rt.queue_event_name(event, matched);
                }
            }
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
            bang: false, range: String::new(), argument: argument.into(), modifiers: Default::default(), sid: rt.current_sid(), lnum: 0 };
        self.execute(rt, &command)
    }
    fn option_exists(&self, name: &str, working: bool) -> bool {
        super::options::option_exists(name, working)
    }
    fn get_register(&mut self, name: char) -> Result<Value, VimError> {
        self.commands.script_register_variable(self.document, name)
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
    "byte2line", "getreg", "getregtype", "getreginfo", "setreg", "hlexists", "hlID", "synID", "synIDtrans", "foldlevel",
    "pumvisible", "complete_check", "getcmdwintype", "reg_recording", "reg_executing", "getchangelist",
    "foldclosed", "foldclosedend", "synIDattr", "getcmdline", "getcmdtype", "state", "getcmdpos", "histget",
    "histnr", "histadd", "histdel", "maparg", "mapcheck", "hasmapto", "winheight", "winwidth", "winline", "wincol",
    "getbufvar", "getwinvar", "gettabvar", "gettabwinvar", "setbufvar", "setwinvar", "settabvar", "getbufline",
    "getbufoneline", "cursor", "setpos", "setline", "append", "winsaveview", "winrestview", "search", "searchpos",
    "feedkeys", "setcharpos", "setcursorcharpos", "getcursorcharpos", "deletebufline", "setbufline", "appendbufline",
    "searchpair", "searchpairpos", "searchcount", "searchdecl", "getmarklist", "getjumplist", "undotree",
    "getbufinfo", "getwininfo", "gettabinfo", "win_id2win",
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
    matches!(name, "@" | "undojoin") || super::options::HOST_COMMANDS.contains(&name)
        || ex::is_supported_command(name)
        || mappings::is_mapping_command(name)
        || abbreviations::is_abbreviation_command(name)
}

impl CommandInterpreter {
    /// A mapping command by full name; a listing becomes a message.
    pub(crate) fn execute_mapping_command(&mut self, name: &str, bang: bool, argument: &str, sid: usize, lnum: usize) -> CommandOutput {
        self.mapping_pending.clear();
        match self.mappings.execute_command(name, bang, argument, sid, lnum) {
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
        if command.name == Some("undojoin") {
            return if document.join_next_change() { CommandOutput::complete() }
                else { CommandOutput { status: CommandStatus::Error("E790: undojoin is not allowed after undo".into()), ..CommandOutput::complete() } };
        }
        if let Some(name) = command.name.filter(|name| mappings::is_mapping_command(name)) {
            return self.execute_mapping_command(name, command.bang, &command.argument, command.sid, command.lnum);
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

    /// Run a command line that a Normal command composed (`K` with an Ex
    /// 'keywordprg'). Script and user commands are available, and like
    /// Vim's `do_cmdline_cmd()` it adds no prompt history.
    pub(super) fn execute_composed_command_line(&mut self, document: &mut Document, command: &str) -> CommandOutput {
        if let Some(host) = vimscript::commands::single_host_command(&self.script, command) {
            if let Some(output) = super::options::execute_script_only_command(self, document, host.name, &host.argument) {
                return output;
            }
            return self.execute_ex_command_with_visual_range(document, &host.text, None);
        }
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::Script(ScriptPlan {
                text: command.to_owned(),
                visual_range: None,
                history: None,
                events: Vec::new(),
                yank: None,
            }));
            return CommandOutput::complete();
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
            ScriptAction::Command { text, name, bang, argument, sid, lnum, .. } => {
                if name == Some("undojoin") {
                    return if document.join_next_change() { CommandOutput::complete() }
                        else { CommandOutput { status: CommandStatus::Error("E790: undojoin is not allowed after undo".into()), ..CommandOutput::complete() } };
                }
                if let Some(name) = name.filter(|name| mappings::is_mapping_command(name)) {
                    return self.execute_mapping_command(name, bang, &argument, sid, lnum);
                }
                if let Some(name) = name.filter(|name| abbreviations::is_abbreviation_command(name)) {
                    return self.execute_abbreviation_command(name, &argument, sid);
                }
                if let Some(output) = super::options::execute_script_only_command(self, document, name, &argument) {
                    return output;
                }
                self.execute_ex_command_with_visual_range(document, &text, visual_range)
            }
            ScriptAction::SetCursor { line, column, want } => {
                let output = self.script_set_cursor(document, line, column);
                self.preferred_column = want;
                output
            },
            ScriptAction::SetLines { first, lines } => self.script_edit_lines(document, None, |document, ending| {
                let count = document.physical_source_line_number_count();
                let existing = count.saturating_sub(first - 1).min(lines.len());
                for (index, text) in lines[..existing].iter().enumerate() {
                    let physical = document.physical_line(first - 1 + index)?;
                    document.replace_physical_source_literal(physical.content_range, text)?;
                }
                // Lines past the end are added together, so whether the file
                // ends with a line ending depends only on the last of them.
                append_source_lines(document, first - 1 + existing, &lines[existing..], ending)
            }),
            ScriptAction::DeleteLines { first, last } => {
                let line = self.script_cursor_line(document).max(1) as usize;
                let column = self.script_cursor_column(document).max(1) as usize;
                let last = last.min(document.physical_source_line_number_count());
                let mut output = self.script_edit_lines(document, None, |document, _| {
                    let count = document.physical_source_line_number_count();
                    let first_line = document.physical_line(first - 1)?;
                    let last_line = document.physical_line(last - 1)?;
                    let mut range = first_line.source_range.start..last_line.source_range.end;
                    if last >= count && first > 1 && last_line.content_range.end == last_line.source_range.end {
                        range.start = document.physical_line(first - 2)?.content_range.end;
                    }
                    document.replace_physical_source_literal(range, "").map(|_| ())
                });
                if output.status == CommandStatus::Complete {
                    let line = if line > last { line - (last - first + 1) } else if line >= first { first } else { line };
                    output.merge(self.script_set_cursor(document, line, column));
                }
                output
            },
            ScriptAction::AppendLines { after, lines } => {
                let inserted = Some((after, lines.len()));
                self.script_edit_lines(document, inserted, |document, ending| append_source_lines(document, after, &lines, ending))
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
        // The column counts UTF-8 bytes of the decoded line, as in Vim.
        let text = physical.content_text();
        let mut column = column.saturating_sub(1).min(text.len());
        while !text.is_char_boundary(column) {
            column -= 1;
        }
        let source = (physical.content_range.start + document.encoding().encoded_text_len(&text[..column])).min(physical.content_range.end);
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
        // A layout position from before the move would win over the new
        // cursor in the next Normal command, such as `:normal! w`.
        self.visual_position = None;
        self.remember_physical_cursor(document, source);
        self.preferred_column = None;
        self.desired_x = None;
        CommandOutput { cursor_moved: self.cursor != old, ..CommandOutput::complete() }
    }

    /// Edit physical source lines, keeping the cursor on its line and
    /// column as Vim does. `inserted` is `(after, count)` when the edit adds
    /// `count` lines after line `after`; lines added above the cursor move it
    /// down with its content.
    fn script_edit_lines(
        &mut self,
        document: &mut Document,
        inserted: Option<(usize, usize)>,
        edit: impl FnOnce(&mut Document, &str) -> Result<(), DocumentError>,
    ) -> CommandOutput {
        let mut line = self.script_cursor_line(document).max(1) as usize;
        let column = self.script_cursor_column(document).max(1) as usize;
        let before = document.revision();
        let ending = document.file_format().spelling();
        if let Err(error) = edit(document, ending) {
            return CommandOutput { status: CommandStatus::Error(error.to_string()), ..CommandOutput::complete() };
        }
        let changed = document.revision() != before;
        if let Some((after, count)) = inserted.filter(|_| changed) {
            if after < line {
                line += count;
            }
        }
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

/// A position in a physical line as Vim script's column functions see it:
/// a UTF-8 offset into the decoded line, whatever the file's encoding.
struct LineColumn {
    text: String,
    offset: usize,
    /// A mark at Vim's maximum column, after the end of the line.
    line_end: bool,
}

/// Vim's `MAXCOL` as script functions report it.
const MAXIMUM_COLUMN: i64 = 2147483647;

impl LineColumn {
    fn at_source(document: &Document, line: &crate::document::PhysicalSourceLine, source: usize) -> Self {
        let text = line.content_text().to_owned();
        let encoding = document.encoding();
        let mut at = line.content_range.start;
        let mut offset = text.len();
        for (index, character) in text.char_indices() {
            if at >= source {
                offset = index;
                break;
            }
            at += encoding.scalar_source_width(character);
        }
        Self { text, offset, line_end: false }
    }

    /// `col()`: one-based UTF-8 byte column.
    fn byte(&self) -> i64 {
        self.offset as i64 + 1
    }

    /// `charcol()`: one-based character column, where a composing character
    /// belongs to its base character.
    fn character(&self) -> i64 {
        self.text[..self.offset].graphemes(true).count() as i64 + 1
    }

    /// `virtcol()`: the last screen cell of the character here, or the cell
    /// after the end of the line.
    fn virtual_end(&self, tabstop: usize) -> i64 {
        let before = vimscript::display_width(&self.text[..self.offset], 0, tabstop);
        let width = self.text[self.offset..].graphemes(true).next().map_or(1, |character| vimscript::display_width(character, before, tabstop));
        (before + width.max(1)) as i64
    }

    /// `getcurpos()`'s `curswant`: Normal mode wants the first cell of a
    /// character but the last cell of a tab, where the cursor is drawn.
    fn cursor_want(&self, tabstop: usize) -> i64 {
        if self.text[self.offset..].starts_with('\t') {
            self.virtual_end(tabstop)
        } else {
            vimscript::display_width(&self.text[..self.offset], 0, tabstop) as i64 + 1
        }
    }
}

/// A physical line as Vim script's line functions present it. Vim stores NUL
/// as LF in a line value, so a literal U+0000 reads as LF. In a Mac-format
/// file, where CR ends lines, a literal LF reads as CR.
pub(crate) fn script_line_value(content: &str, format: FileFormat) -> String {
    let content = if format == FileFormat::Mac { content.replace('\n', "\r") } else { content.to_owned() };
    content.replace('\0', "\n")
}

/// The source text for a line value from `setline()` or `append()`: an LF
/// inside one line value is Vim's spelling of NUL, never a line break, and in
/// a Mac-format file a CR is a literal LF.
pub(crate) fn script_line_source(value: &str, format: FileFormat) -> String {
    let value = value.replace('\n', "\0");
    if format == FileFormat::Mac { value.replace('\r', "\n") } else { value }
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
