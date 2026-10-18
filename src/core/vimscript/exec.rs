//! The legacy Vim script runtime: frames, control flow, errors and
//! exceptions, variables, and function calls. Commands are parsed from text
//! like Vim's `do_cmdline()`; see `commands.rs` for command parsing.
//!
//! Execution is resumable at command boundaries. Script files, functions
//! called by `:call`, `:execute` strings, autocommands and user commands run
//! as frames of one explicit stack, so a host may suspend between commands
//! (for example while a file is opened) and resume later. A function called
//! from inside an expression runs synchronously to completion.
use super::error::{EvalResult, Exception, Fail, VimError};
use super::expr::Eval;
use super::state::{FunctionBody, FunctionFlags, FunctionScope, Param, ScriptState, SourceLine, UserFunction};
use super::value::{Dict, DictItem, Funcref, List, Lock, VStr, Value};
use std::sync::Arc;

/// Vim's default 'maxfuncdepth'.
pub const MAX_FUNC_DEPTH: usize = 100;
/// Limit on nested frames (`:source`, `:execute`, user commands, ...).
pub const MAX_NESTING: usize = 200;
/// Work units for one runtime; each command and expression node costs at
/// least one unit. Hosts create a runtime per top-level invocation.
pub const DEFAULT_FUEL: u64 = 20_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionScope {
    /// `&opt`, `:set`: the local value when there is one.
    Both,
    /// `&l:opt`, `:setlocal`.
    Local,
    /// `&g:opt`, `:setglobal`.
    Global,
}

/// Where an error was detected, for Vim's "Error detected while processing"
/// context and Viem's file:line diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub error: VimError,
    /// The script file, when inside (or called from) a sourced file.
    pub file: Option<String>,
    /// The function name, when inside a function.
    pub function: Option<String>,
    /// The line within the function, or else within the file (one-based).
    pub lnum: Option<usize>,
}

impl ScriptError {
    /// "file.vim:3: E492: ..." or "function Foo, line 2: E..." context.
    pub fn display(&self) -> String {
        match (&self.file, &self.function, self.lnum) {
            (_, Some(function), Some(lnum)) => format!("function {function}, line {lnum}: {}", self.error.text()),
            (Some(file), None, Some(lnum)) => format!("{file}:{lnum}: {}", self.error.text()),
            _ => self.error.text(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// `:echo` text; a new message line.
    Echo(String),
    /// `:echon`: continues the current line.
    Echon(String),
    /// `:echomsg`, also kept in `:messages`.
    EchoMsg(String),
    /// `:echoerr` and errors.
    Error(ScriptError),
    /// Output of listing commands such as `:let`, `:function`, `:messages`.
    Output(String),
}

/// The complete text of a command the engine does not implement itself.
#[derive(Clone, Debug)]
pub struct HostCommand {
    /// Range, name, bang and argument, without modifiers, `|` or comment.
    pub text: String,
    /// The resolved full Vim name, when it is a known Vim command.
    pub name: Option<&'static str>,
    pub bang: bool,
    pub range: String,
    pub argument: String,
    pub modifiers: Modifiers,
    /// Script context for `<SID>` and script-local mappings.
    pub sid: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub silent: bool,
    pub silent_errors: bool,
    pub unsilent: bool,
    pub verbose: Option<i64>,
    pub noautocmd: bool,
    pub sandbox: bool,
    pub vertical: bool,
    pub horizontal: bool,
    pub keepjumps: bool,
    pub keeppatterns: bool,
    pub keepmarks: bool,
    pub keepalt: bool,
    pub lockmarks: bool,
    pub confirm: bool,
    pub browse: bool,
    /// Window placement modifiers Viem cannot honor (`:topleft`, `:tab`, ...).
    pub placement: Vec<String>,
}

/// Services provided to scripts. Editor hosts run on the buffer's serial owner;
/// syntax workers own private script state and immutable document queries.
/// No host may hold a lock while calling back into the runtime.
pub trait ScriptHost {
    /// A confined host may replace builtins that otherwise inspect ambient
    /// files, environment or editor state. Checked before engine dispatch.
    fn override_builtin(&mut self, _rt: &mut Runtime, _name: &str, _args: &[Value]) -> Option<EvalResult<Value>> {
        None
    }
    /// Syntax hosts resolve runtime imports within their pinned resources.
    fn handles_runtime_commands(&self) -> bool { false }
    /// Load an autoload namespace before resolving a missing global or function.
    /// Ordinary editor hosts may decline; confined hosts account for the import.
    fn autoload(&mut self, _rt: &mut Runtime, _name: &[u8]) -> EvalResult<bool> { Ok(false) }
    /// Built-in runtime events supplied by a host without sourcing editor plugins.
    fn runtime_event(&mut self, _rt: &mut Runtime, _event: &str, _matched: &str) -> EvalResult<()> { Ok(()) }
    /// Execute an Ex command that is not part of the scripting language.
    fn execute(&mut self, rt: &mut Runtime, command: &HostCommand) -> EvalResult<()>;
    fn get_option(&mut self, name: &str, scope: OptionScope) -> Result<Value, VimError>;
    /// `:let &opt = value`.
    fn set_option(&mut self, rt: &mut Runtime, name: &str, scope: OptionScope, value: &Value) -> EvalResult<()>;
    /// `exists('&name')` (`working == false`) and `exists('+name')`.
    fn option_exists(&self, name: &str, working: bool) -> bool;
    fn get_register(&mut self, name: char) -> Result<Value, VimError>;
    fn set_register(&mut self, name: char, value: &Value, append: bool) -> Result<(), VimError>;
    /// Editor builtins such as `getline()`; None if `name` is not one.
    fn call_function(&mut self, rt: &mut Runtime, name: &str, args: &[Value]) -> Option<EvalResult<Value>>;
    fn has_function(&self, name: &str) -> bool;
    /// `has()` host answers; None falls back to the engine's table.
    fn feature(&self, name: &str) -> Option<bool>;
    /// Whether a full Ex command name is implemented by the host.
    fn command_exists(&self, name: &str) -> bool;
    fn message(&mut self, message: Message);
    fn ignorecase(&self) -> bool;
    /// Dynamic `v:` and `b:` values such as `v:count` and `b:changedtick`.
    fn dynamic_variable(&self, _name: &[u8]) -> Option<Value> {
        None
    }
    fn view_id(&self) -> u64 {
        0
    }
    /// The bound file path of the current buffer, for `%` expansion.
    fn buffer_name(&self) -> Option<String> {
        None
    }
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, VimError> {
        super::source::read_script_file(path)
    }
    /// Checked between commands; true asks the runtime to stop at the next
    /// command boundary where it can resume later.
    fn should_suspend(&mut self) -> bool {
        false
    }
    fn interrupted(&mut self) -> bool {
        false
    }
    /// Set while evaluating `<expr>` mappings: text and windows cannot change.
    fn textlock(&self) -> bool {
        false
    }
    fn getenv(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
    fn setenv(&mut self, _name: &str, _value: Option<&str>) -> Result<(), VimError> {
        Err(VimError::plain("Viem scripts cannot change environment variables"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    /// All frames finished.
    Complete,
    /// Stopped at a command boundary at the host's request; call `run`
    /// again to continue.
    Suspended,
}

#[derive(Clone, Debug)]
pub enum FrameKind {
    Script { path: String },
    Function,
    Execute,
    Autocmd { event: usize, matched: String },
    UserCommand { name: String },
    CommandLine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CondKind {
    If,
    While,
    For,
    Try,
}

#[derive(Clone, Debug)]
pub(crate) enum ForItems {
    List(List),
    Chars(Vec<VStr>),
    Blob(Vec<u8>),
}

#[derive(Clone, Debug)]
pub(crate) struct ForState {
    pub items: ForItems,
    pub index: usize,
    pub targets: Vec<VStr>,
    pub rest: Option<VStr>,
}

/// An action postponed until a `:finally` block finishes.
#[derive(Clone, Debug)]
pub(crate) enum Pending {
    Exception(Box<Exception>),
    Return(Value),
    Break,
    Continue,
    Finish,
    Aborted,
}

#[derive(Clone, Debug)]
pub(crate) struct Cond {
    pub kind: CondKind,
    /// Commands inside the current block execute.
    pub active: bool,
    /// For `:if`: a branch was chosen (or the conditional is skipped).
    pub taken: bool,
    pub had_else: bool,
    /// Position of the `:while`/`:for` command.
    pub start: (usize, usize),
    pub looping: bool,
    pub broken: bool,
    pub continued: bool,
    pub for_state: Option<ForState>,
    /// The enclosing block was active when this conditional began.
    pub outer_active: bool,
    pub in_catch: bool,
    pub in_finally: bool,
    /// The pending exception was caught, or catch clauses are skipped.
    pub caught: bool,
    pub pending: Option<Pending>,
}

impl Cond {
    pub fn new(kind: CondKind, active: bool, outer_active: bool) -> Self {
        Self {
            kind,
            active,
            taken: active,
            had_else: false,
            start: (0, 0),
            looping: false,
            broken: false,
            continued: false,
            for_state: None,
            outer_active,
            in_catch: false,
            in_finally: false,
            caught: false,
            pending: None,
        }
    }
    /// A `:try` that converts errors into exceptions while it is open.
    pub fn counts_for_trylevel(&self) -> bool {
        self.kind == CondKind::Try && self.outer_active
    }
}

pub(crate) struct Frame {
    pub kind: FrameKind,
    pub lines: Arc<[SourceLine]>,
    pub line: usize,
    pub col: usize,
    /// The line of the command being executed, for error context.
    pub command_line: usize,
    pub cstack: Vec<Cond>,
    pub sid: usize,
    pub scope: Option<Arc<FunctionScope>>,
    pub function: Option<Arc<UserFunction>>,
    pub result: Option<Value>,
    pub abort_on_error: bool,
    pub had_error: bool,
    /// Started by a synchronous call; the nested run stops when it is popped.
    pub boundary: bool,
    /// Set when the frame finished because of `:finish` or `:return`.
    pub finished: bool,
    /// The `:source` file that pushed this frame, for SourcePost.
    pub source_post: Option<String>,
    /// `:silent` / `:silent!` modifiers in effect for this frame's lifetime.
    pub silent: bool,
    pub silent_errors: bool,
    pub autocmd: bool,
    pub autocmd_nested: bool,
    pub autocmd_level: usize,
    pub noautocmd: bool,
}

impl Frame {
    pub(crate) fn new(kind: FrameKind, lines: Arc<[SourceLine]>, sid: usize) -> Self {
        let autocmd = matches!(kind, FrameKind::Autocmd { .. });
        Self {
            kind,
            lines,
            line: 0,
            col: 0,
            command_line: 0,
            cstack: Vec::new(),
            sid,
            scope: None,
            function: None,
            result: None,
            abort_on_error: false,
            had_error: false,
            boundary: false,
            finished: false,
            source_post: None,
            silent: false,
            silent_errors: false,
            autocmd,
            autocmd_nested: false,
            autocmd_level: 0,
            noautocmd: false,
        }
    }
    pub(crate) fn executing(&self) -> bool {
        self.cstack.last().is_none_or(|cond| cond.active)
    }
    fn is_function(&self) -> bool {
        matches!(self.kind, FrameKind::Function)
    }
    fn active_trys(&self, from: usize) -> usize {
        self.cstack[from.min(self.cstack.len())..].iter().filter(|cond| cond.counts_for_trylevel()).count()
    }
}

/// Output captured by `execute()`.
#[derive(Default)]
pub(crate) struct Capture {
    pub text: Vec<u8>,
    pub silent: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum Redirect {
    Variable { name: Vec<u8>, append: bool, text: Vec<u8> },
    Register { name: char, append: bool, text: Vec<u8> },
}

pub struct Runtime {
    pub state: ScriptState,
    /// Implicit script scope for the syntax host's Vim9 declaration profile.
    /// Ordinary legacy editor scripts retain their global default scope.
    script_local_sids: std::collections::BTreeSet<usize>,
    pub(crate) frames: Vec<Frame>,
    /// Synchronous nesting depth; suspension is impossible while positive.
    pub(crate) nested: usize,
    pub(crate) nested_result: Option<EvalResult<Value>>,
    pub(crate) call_depth: usize,
    pub(crate) source_depth: usize,
    /// Open `:try` conditionals across all frames.
    pub(crate) trylevel: usize,
    pub(crate) emsg_silent: usize,
    pub(crate) msg_silent: usize,
    pub(crate) fuel: u64,
    /// Every error reported during this runtime's life, in order.
    pub errors: Vec<ScriptError>,
    /// Preserve an uncatchable failure's origin before synchronous frames unwind.
    pub(crate) interrupt_context: Option<ScriptError>,
    pub(crate) captures: Vec<Capture>,
    pub(crate) redirect: Option<Redirect>,
    /// The command currently executing, for "Vim(cmd):E..." exceptions.
    pub(crate) current_command: String,
    /// Autocommand events queued by host commands, fired after the command.
    pub(crate) queued_events: Vec<(usize, String)>,
    pub(crate) autocmd_depth: usize,
    pub(crate) caught_exceptions: Vec<Exception>,
    /// An error was reported during the current command (Vim's did_emsg):
    /// the rest of the line is skipped and `abort` functions return.
    pub(crate) did_emsg: bool,
    /// The last synchronous user function call ended because of an error.
    pub(crate) call_aborted: bool,
    /// Match groups for `submatch()` inside a `\=` substitute() expression.
    pub(crate) submatches: Vec<Vec<Value>>,
    /// Startup configuration: commands that edit text or act on files and
    /// windows are rejected.
    pub restricted: bool,
    /// Set after an uncaught exception or interrupt aborted the run.
    pub aborted: bool,
}

impl Runtime {
    pub fn new(state: ScriptState) -> Self {
        Self {
            state,
            script_local_sids: Default::default(),
            frames: Vec::new(),
            nested: 0,
            nested_result: None,
            call_depth: 0,
            source_depth: 0,
            trylevel: 0,
            emsg_silent: 0,
            msg_silent: 0,
            fuel: DEFAULT_FUEL,
            errors: Vec::new(),
            interrupt_context: None,
            captures: Vec::new(),
            redirect: None,
            current_command: String::new(),
            queued_events: Vec::new(),
            autocmd_depth: 0,
            caught_exceptions: Vec::new(),
            submatches: Vec::new(),
            did_emsg: false,
            call_aborted: false,
            restricted: false,
            aborted: false,
        }
    }

    /// Inside a synchronous call from an expression, where the runtime
    /// cannot suspend.
    pub fn is_nested(&self) -> bool {
        self.nested > 0
    }

    /// Messages are suppressed by `:silent`.
    pub fn messages_silenced(&self) -> bool {
        self.msg_silent > 0
    }

    pub fn is_idle(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn set_fuel(&mut self, fuel: u64) {
        self.fuel = fuel;
    }

    pub(crate) fn charge(&mut self, cost: u64) -> EvalResult<()> {
        if self.fuel < cost {
            self.fuel = 0;
            return Err(Fail::Interrupt("script work budget exhausted".into()));
        }
        self.fuel -= cost;
        Ok(())
    }

    // ----- entry points ------------------------------------------------

    /// Queue a script file's text as a new frame; `run` executes it.
    pub fn push_script(&mut self, path: &str, text: &[u8]) -> Result<(), VimError> {
        if self.frames.len() >= MAX_NESTING {
            return Err(VimError::new(169, "Command too recursive"));
        }
        let lines = super::source::script_lines(text);
        let sid = self.state.script_id(path);
        self.frames.push(Frame::new(FrameKind::Script { path: path.to_string() }, lines.into(), sid));
        Ok(())
    }

    /// Execute an already decoded script synchronously while preserving its
    /// physical source locations and the calling runtime's scopes and budget.
    pub(crate) fn source_lines_now(&mut self, host: &mut dyn ScriptHost, path: &str, lines: Vec<SourceLine>, script_local: bool) -> EvalResult<()> {
        if self.nested == 0 { self.interrupt_context = None; }
        if self.frames.len() >= MAX_NESTING {
            return Err(Fail::Error(VimError::new(169, "Command too recursive")));
        }
        let sid = self.state.script_id(path);
        if script_local {self.script_local_sids.insert(sid);} else {self.script_local_sids.remove(&sid);}
        let mut frame = Frame::new(FrameKind::Script { path: path.to_owned() }, lines.into(), sid);
        frame.boundary = true;
        let base = self.frames.len();
        self.frames.push(frame);
        self.run_nested(host, base).map(|_| ())
    }

    /// Queue one command line (as typed after `:`) as a frame.
    pub fn push_command_line(&mut self, text: &str, sid: usize) {
        let lines: Arc<[SourceLine]> = super::source::split_command_lines(text.as_bytes(), 1).into();
        self.frames.push(Frame::new(FrameKind::CommandLine, lines, sid));
    }

    /// Run frames until none remain or the host asks to suspend.
    pub fn run(&mut self, host: &mut dyn ScriptHost) -> RunState {
        loop {
            if self.frames.is_empty() {
                return RunState::Complete;
            }
            if self.nested == 0 && host.should_suspend() {
                return RunState::Suspended;
            }
            if host.interrupted() {
                self.abort_with(host, Fail::Interrupt("Interrupted".into()));
                continue;
            }
            if let Err(fail) = self.step(host) {
                self.abort_with(host, fail);
            }
        }
    }

    /// Discard all frames, e.g. when the host cancels a suspended script.
    pub fn cancel(&mut self) {
        while !self.frames.is_empty() {
            self.pop_frame_silently();
        }
    }

    /// Execute a command line synchronously to completion (no suspension).
    pub fn execute_now(&mut self, host: &mut dyn ScriptHost, text: &str, sid: usize) -> EvalResult<()> {
        self.push_command_line(text, sid);
        let base = self.frames.len() - 1;
        self.frames[base].boundary = true;
        self.run_nested(host, base).map(|_| ())
    }

    /// Run until the boundary frame at `base` is popped, returning its value.
    pub(crate) fn run_nested(&mut self, host: &mut dyn ScriptHost, base: usize) -> EvalResult<Value> {
        self.nested += 1;
        let result = loop {
            if self.frames.len() <= base {
                break self.nested_result.take().unwrap_or(Ok(Value::Number(0)));
            }
            if host.interrupted() {
                self.unwind_to(base);
                break Err(Fail::Interrupt("Interrupted".into()));
            }
            if let Err(fail) = self.step(host) {
                if matches!(fail, Fail::Interrupt(_)) && self.interrupt_context.is_none() {
                    self.interrupt_context = Some(self.error_context());
                }
                self.unwind_to(base);
                self.nested_result = None;
                break Err(fail);
            }
        };
        self.nested -= 1;
        result
    }

    fn unwind_to(&mut self, base: usize) {
        while self.frames.len() > base {
            self.pop_frame_silently();
        }
    }

    /// Evaluate one expression, e.g. for `<expr>` mappings or `eval()`.
    pub fn eval_expression(&mut self, host: &mut dyn ScriptHost, text: &[u8], sid: usize) -> EvalResult<Value> {
        let scope = self.frames.last().and_then(|frame| frame.scope.clone());
        self.with_context(scope, sid, |rt| {
            let mut eval = Eval::new(rt, host, text, 0, true);
            eval.skip_white();
            let value = eval.eval1()?;
            eval.skip_white();
            if !eval.at_end() {
                return Err(Fail::Error(VimError::trailing(&text[eval.pos..])));
            }
            Ok(value)
        })
    }

    /// Run `f` with a placeholder frame supplying a script context and scope.
    pub(crate) fn with_context<T>(&mut self, scope: Option<Arc<FunctionScope>>, sid: usize, f: impl FnOnce(&mut Self) -> EvalResult<T>) -> EvalResult<T> {
        let lines: Arc<[SourceLine]> = Vec::new().into();
        let mut frame = Frame::new(FrameKind::Execute, lines, sid);
        frame.scope = scope;
        frame.boundary = true;
        self.frames.push(frame);
        let base = self.frames.len() - 1;
        self.nested += 1;
        let result = f(self);
        self.nested -= 1;
        self.unwind_to(base);
        self.nested_result = None;
        result
    }

    // ----- the command loop ----------------------------------------------

    /// Abort everything after an uncaught failure, reporting it.
    fn abort_with(&mut self, host: &mut dyn ScriptHost, fail: Fail) {
        match fail {
            Fail::Exception(exception) => {
                let error = uncaught_error(&exception);
                self.report(host, error);
            }
            Fail::Interrupt(reason) => self.report(host, VimError::plain(reason)),
            Fail::Error(error) => self.report(host, error),
            Fail::Aborted => {}
        }
        self.cancel();
        self.aborted = true;
    }

    /// Execute the next command of the top frame. Returns failures that
    /// escaped every frame (uncaught exceptions, interrupts).
    fn step(&mut self, host: &mut dyn ScriptHost) -> EvalResult<()> {
        self.charge(1)?;
        let index = self.frames.len() - 1;
        let frame = &mut self.frames[index];
        if frame.line >= frame.lines.len() {
            return self.end_of_frame(host);
        }
        let line = frame.lines[frame.line].text.clone();
        let col = frame.col;
        frame.command_line = frame.line;
        if col >= line.len() {
            frame.line += 1;
            frame.col = 0;
            return Ok(());
        }
        let executing = frame.executing();
        self.did_emsg = false;
        let CommandOutcome { next, result } = super::commands::execute_command(self, host, &line, col, executing);
        if index < self.frames.len() {
            let frame = &mut self.frames[index];
            match next {
                Next::Column(col) => frame.col = col,
                Next::NextLine => {
                    frame.line += 1;
                    frame.col = 0;
                }
                Next::Position(line, col) => {
                    frame.line = line;
                    frame.col = col;
                }
                Next::Unchanged => {}
            }
        }
        let result = result.and_then(|()| self.fire_queued_events(host));
        let result = match result {
            Ok(()) => Ok(()),
            Err(fail) => self.handle_failure(host, index, fail),
        };
        if self.did_emsg && result.is_ok() {
            self.after_error(index)?;
        }
        result
    }

    /// After a command reported an error: skip the rest of its line, and
    /// return from a function defined with `abort` (or a lambda).
    fn after_error(&mut self, index: usize) -> EvalResult<()> {
        self.skip_rest_of_line(index);
        let Some(frame) = self.frames.get_mut(index) else { return Ok(()) };
        frame.had_error = true;
        if frame.abort_on_error && self.emsg_silent == 0 {
            while self.frames.len() > index + 1 {
                self.pop_frame_silently();
            }
            return self.unwind(None, Unwind::Aborted);
        }
        Ok(())
    }

    /// Errors become exceptions inside `:try`, are reported otherwise, and
    /// exceptions unwind to the nearest matching `:catch`.
    pub(crate) fn handle_failure(&mut self, host: &mut dyn ScriptHost, index: usize, fail: Fail) -> EvalResult<()> {
        match fail {
            Fail::Error(error) => {
                if self.emsg_silent > 0 {
                    // A silenced error does not set did_emsg, so the rest of
                    // the line still runs unless the command itself stopped
                    // it (`:call` failures, unknown commands).
                    self.state.set_vvar("errmsg", error.text().into());
                    return Ok(());
                }
                if self.trylevel > 0 {
                    let exception = self.error_exception(&error);
                    return self.throw_from(host, index, exception);
                }
                self.report(host, error);
                Ok(())
            }
            Fail::Aborted => {
                self.mark_line_error(index);
                if self.emsg_silent == 0 {
                    self.did_emsg = true;
                }
                Ok(())
            }
            Fail::Exception(exception) => self.throw_from(host, index, *exception),
            Fail::Interrupt(reason) => Err(Fail::Interrupt(reason)),
        }
    }

    fn mark_line_error(&mut self, index: usize) {
        self.skip_rest_of_line(index);
    }

    /// Skip the remaining commands of the frame's current line. A frame at
    /// column zero already moved past the failing line.
    pub(crate) fn skip_rest_of_line(&mut self, index: usize) {
        if let Some(frame) = self.frames.get_mut(index) {
            if frame.col > 0 {
                frame.line += 1;
                frame.col = 0;
            }
        }
    }

    /// Throw from the frame at `index`, discarding frames it pushed.
    fn throw_from(&mut self, host: &mut dyn ScriptHost, index: usize, exception: Exception) -> EvalResult<()> {
        while self.frames.len() > index + 1 {
            self.pop_frame_silently();
        }
        self.unwind(Some(host), Unwind::Exception(Box::new(exception)))
    }

    pub(crate) fn error_exception(&self, error: &VimError) -> Exception {
        let command = if self.current_command.is_empty() { String::new() } else { format!("({})", self.current_command) };
        Exception {
            value: Value::String(format!("Vim{command}:{}", error.text()).into()),
            throwpoint: self.throwpoint(),
            from_error: true,
            interrupt: false,
        }
    }

    pub(crate) fn throwpoint(&self) -> String {
        for frame in self.frames.iter().rev() {
            let lnum = frame.lines.get(frame.command_line.min(frame.lines.len().saturating_sub(1))).map_or(0, |line| line.lnum);
            match &frame.kind {
                FrameKind::Function => {
                    let name = frame.function.as_ref().map_or_else(String::new, |function| function.name.to_str_lossy().into_owned());
                    return format!("function {name}, line {lnum}");
                }
                FrameKind::Script { path } => return format!("{path}, line {lnum}"),
                _ => {}
            }
        }
        String::new()
    }

    /// The context of the innermost function or script frame.
    pub(crate) fn error_context(&self) -> ScriptError {
        let mut context = ScriptError { error: VimError::plain(""), file: None, function: None, lnum: None };
        for frame in self.frames.iter().rev() {
            let lnum = frame.lines.get(frame.command_line.min(frame.lines.len().saturating_sub(1))).map(|line| line.lnum);
            match &frame.kind {
                FrameKind::Function if context.function.is_none() && context.file.is_none() => {
                    context.function = frame.function.as_ref().map(|function| function.name.to_str_lossy().into_owned());
                    context.lnum = lnum;
                }
                FrameKind::Script { path } if context.file.is_none() => {
                    context.file = Some(path.clone());
                    if context.function.is_none() {
                        context.lnum = lnum;
                    }
                    break;
                }
                _ => {}
            }
        }
        context
    }

    pub(crate) fn report(&mut self, host: &mut dyn ScriptHost, error: VimError) {
        self.did_emsg = true;
        self.state.set_vvar("errmsg", error.text().into());
        let mut context = self.error_context();
        context.error = error;
        self.state.record_message(&context.error.text());
        self.errors.push(context.clone());
        if let Some(frame) = self.frames.last_mut() {
            frame.had_error = true;
        }
        self.emit(host, Message::Error(context));
    }

    /// Send a message to the host, an active capture or a redirection.
    pub(crate) fn emit(&mut self, host: &mut dyn ScriptHost, message: Message) {
        let (newline, text) = match &message {
            Message::Echo(text) | Message::EchoMsg(text) | Message::Output(text) => (true, text.clone()),
            Message::Echon(text) => (false, text.clone()),
            Message::Error(error) => (true, error.error.text()),
        };
        if let Some(Redirect::Variable { text: buffer, .. } | Redirect::Register { text: buffer, .. }) = &mut self.redirect {
            if newline {
                buffer.push(b'\n');
            }
            buffer.extend_from_slice(text.as_bytes());
        }
        if let Some(capture) = self.captures.last_mut() {
            if newline {
                capture.text.push(b'\n');
            }
            capture.text.extend_from_slice(text.as_bytes());
            if !matches!(message, Message::Error(_)) || capture.silent {
                return;
            }
        }
        if self.msg_silent > 0 && !matches!(message, Message::Error(_)) {
            return;
        }
        host.message(message);
    }

    // ----- frames and unwinding -------------------------------------------

    fn end_of_frame(&mut self, host: &mut dyn ScriptHost) -> EvalResult<()> {
        let index = self.frames.len() - 1;
        if let Some(kind) = self.frames[index].cstack.last().map(|cond| cond.kind) {
            if !self.frames[index].finished {
                let error = match kind {
                    CondKind::If => VimError::new(171, "Missing :endif"),
                    CondKind::While => VimError::new(170, "Missing :endwhile"),
                    CondKind::For => VimError::new(170, "Missing :endfor"),
                    CondKind::Try => VimError::new(600, "Missing :endtry"),
                };
                self.frames[index].command_line = self.frames[index].lines.len().saturating_sub(1);
                self.report(host, error);
            }
            let trys = self.frames[index].active_trys(0);
            self.trylevel = self.trylevel.saturating_sub(trys);
            self.frames[index].cstack.clear();
        }
        self.pop_frame(Some(host));
        Ok(())
    }

    /// Pop the top frame normally, delivering a boundary frame's value.
    pub(crate) fn pop_frame(&mut self, host: Option<&mut dyn ScriptHost>) {
        let Some(frame) = self.frames.pop() else { return };
        self.release_frame(&frame);
        if frame.boundary && self.nested_result.is_none() {
            self.nested_result = Some(Ok(frame.result.clone().unwrap_or(Value::Number(0))));
        }
        if let Some(path) = &frame.source_post {
            if let Some(host) = host {
                self.queue_event_name("SourcePost", path.clone());
                let _ = self.fire_queued_events(host);
            }
        }
    }

    fn pop_frame_silently(&mut self) {
        if let Some(frame) = self.frames.pop() {
            self.release_frame(&frame);
        }
    }

    /// Finish the exception of a `:catch` clause that is being left.
    pub(crate) fn leave_catch(&mut self) {
        self.caught_exceptions.pop();
        let (value, throwpoint) = match self.caught_exceptions.last() {
            Some(exception) => (exception.value.clone(), exception.throwpoint.clone()),
            None => (Value::str(""), String::new()),
        };
        self.state.set_vvar("exception", value);
        self.state.set_vvar("throwpoint", throwpoint.into());
    }

    fn release_frame(&mut self, frame: &Frame) {
        for _ in frame.cstack.iter().filter(|cond| cond.kind == CondKind::Try && cond.in_catch) {
            self.leave_catch();
        }
        self.trylevel = self.trylevel.saturating_sub(frame.active_trys(0));
        if frame.is_function() {
            self.call_depth = self.call_depth.saturating_sub(1);
        }
        if frame.source_post.is_some() {
            self.source_depth = self.source_depth.saturating_sub(1);
        }
        if frame.silent {
            self.msg_silent = self.msg_silent.saturating_sub(1);
        }
        if frame.silent_errors {
            self.emsg_silent = self.emsg_silent.saturating_sub(1);
        }
        if frame.autocmd {
            self.autocmd_depth = self.autocmd_depth.saturating_sub(1);
        }
    }

    /// Remove the conditionals of frame `index` above `keep`.
    fn truncate_conds(&mut self, index: usize, keep: usize) {
        let trys = self.frames[index].active_trys(keep);
        self.trylevel = self.trylevel.saturating_sub(trys);
        let catches = self.frames[index].cstack[keep.min(self.frames[index].cstack.len())..]
            .iter()
            .filter(|cond| cond.kind == CondKind::Try && cond.in_catch)
            .count();
        for _ in 0..catches {
            self.leave_catch();
        }
        self.frames[index].cstack.truncate(keep);
    }

    /// Pop a `:try` at `endtry`.
    pub(crate) fn pop_try(&mut self) {
        let Some(frame) = self.frames.last_mut() else { return };
        if let Some(cond) = frame.cstack.pop() {
            if cond.counts_for_trylevel() {
                self.trylevel = self.trylevel.saturating_sub(1);
            }
        }
    }

    /// Unwind control flow: `:break`, `:continue`, `:return`, `:finish`,
    /// exceptions and aborted functions. `:finally` blocks run first.
    pub(crate) fn unwind(&mut self, mut host: Option<&mut dyn ScriptHost>, mut action: Unwind) -> EvalResult<()> {
        loop {
            let Some(index) = self.frames.len().checked_sub(1) else {
                return match action {
                    Unwind::Exception(exception) => Err(Fail::Exception(exception)),
                    _ => Ok(()),
                };
            };
            let mut position = self.frames[index].cstack.len();
            let mut resolved = false;
            while position > 0 {
                position -= 1;
                let cond = &self.frames[index].cstack[position];
                let kind = cond.kind;
                let handles = match (&action, kind) {
                    (Unwind::Break | Unwind::Continue, CondKind::While | CondKind::For) => true,
                    (_, CondKind::Try) => cond.outer_active && !cond.in_finally,
                    _ => false,
                };
                if !handles {
                    continue;
                }
                // Deactivate everything inside this conditional; the skipped
                // :endif/:endwhile/:endtry commands still pop them.
                for inner in &mut self.frames[index].cstack[position + 1..] {
                    inner.active = false;
                    inner.taken = true;
                    inner.broken = true;
                    if inner.kind == CondKind::Try {
                        // An exception from a :finally discards its pending action.
                        inner.pending = None;
                        inner.caught = true;
                    }
                }
                if matches!(kind, CondKind::While | CondKind::For) {
                    let cond = &mut self.frames[index].cstack[position];
                    if matches!(action, Unwind::Break) {
                        cond.broken = true;
                    } else {
                        cond.continued = true;
                    }
                    cond.active = false;
                    resolved = true;
                    break;
                }
                // A :try: catch exceptions from its body; otherwise run :finally.
                let cond = &mut self.frames[index].cstack[position];
                let from_body = !cond.in_catch;
                let pending = match std::mem::replace(&mut action, Unwind::Finish) {
                    Unwind::Exception(exception) => {
                        if from_body {
                            cond.caught = false;
                        } else {
                            cond.caught = true;
                        }
                        Pending::Exception(exception)
                    }
                    Unwind::Return(value) => Pending::Return(value),
                    Unwind::Break => Pending::Break,
                    Unwind::Continue => Pending::Continue,
                    Unwind::Finish => Pending::Finish,
                    Unwind::Aborted => Pending::Aborted,
                };
                if !matches!(pending, Pending::Exception(_)) {
                    cond.caught = true;
                }
                cond.pending = Some(pending);
                cond.active = false;
                let left_catch = std::mem::replace(&mut cond.in_catch, false);
                if left_catch {
                    self.leave_catch();
                }
                resolved = true;
                break;
            }
            if resolved {
                return Ok(());
            }
            // The action leaves this frame.
            match action {
                Unwind::Break => return Err(Fail::Error(VimError::new(587, ":break without :while or :for"))),
                Unwind::Continue => return Err(Fail::Error(VimError::new(586, ":continue without :while or :for"))),
                Unwind::Return(value) => {
                    let frame = &mut self.frames[index];
                    if !frame.is_function() {
                        // :return inside :execute (or a user command) in a function.
                        frame.finished = true;
                        let boundary = frame.boundary;
                        self.truncate_conds(index, 0);
                        self.pop_frame(host.as_mut().map(|host| -> &mut dyn ScriptHost { &mut **host }));
                        if boundary {
                            return Ok(());
                        }
                        action = Unwind::Return(value);
                        continue;
                    }
                    self.truncate_conds(index, 0);
                    let frame = &mut self.frames[index];
                    frame.result = Some(value);
                    frame.finished = true;
                    frame.line = frame.lines.len();
                    return Ok(());
                }
                Unwind::Finish => {
                    self.truncate_conds(index, 0);
                    let frame = &mut self.frames[index];
                    frame.finished = true;
                    if matches!(frame.kind, FrameKind::Script { .. }) {
                        frame.line = frame.lines.len();
                        return Ok(());
                    }
                    let boundary = frame.boundary;
                    self.pop_frame(host.as_mut().map(|host| -> &mut dyn ScriptHost { &mut **host }));
                    if boundary {
                        return Ok(());
                    }
                    action = Unwind::Finish;
                }
                Unwind::Aborted => {
                    self.truncate_conds(index, 0);
                    let frame = &mut self.frames[index];
                    frame.finished = true;
                    let boundary = frame.boundary;
                    let function = frame.is_function();
                    self.frames.pop().map(|frame| self.release_frame(&frame));
                    if boundary {
                        // The caller's expression continues with -1.
                        self.nested_result = Some(Ok(Value::Number(-1)));
                        self.call_aborted = true;
                        return Ok(());
                    }
                    if function {
                        if let Some(caller) = self.frames.len().checked_sub(1) {
                            self.skip_rest_of_line(caller);
                            let caller = &mut self.frames[caller];
                            caller.had_error = true;
                            if caller.abort_on_error {
                                action = Unwind::Aborted;
                                continue;
                            }
                        }
                    }
                    return Ok(());
                }
                Unwind::Exception(exception) => {
                    self.truncate_conds(index, 0);
                    let boundary = self.frames[index].boundary;
                    self.frames.pop().map(|frame| self.release_frame(&frame));
                    if boundary {
                        self.nested_result = Some(Err(Fail::Exception(exception)));
                        return Ok(());
                    }
                    if self.frames.is_empty() {
                        return Err(Fail::Exception(exception));
                    }
                    action = Unwind::Exception(exception);
                }
            }
        }
    }

    pub(crate) fn current_frame(&self) -> Option<&Frame> {
        self.frames.last()
    }

    pub(crate) fn current_frame_mut(&mut self) -> Option<&mut Frame> {
        self.frames.last_mut()
    }

    /// The script ID of the innermost frame.
    pub fn current_sid(&self) -> usize {
        self.frames.last().map_or(0, |frame| frame.sid)
    }

    pub(crate) fn current_scope(&self) -> Option<Arc<FunctionScope>> {
        self.frames.last().and_then(|frame| frame.scope.clone())
    }

    /// Mark frames pushed since `before` with the silent modifiers of the
    /// command that pushed them, keeping the counters raised until they pop.
    pub(crate) fn transfer_silence(&mut self, before: usize, silent: bool, silent_errors: bool) -> bool {
        if self.frames.len() <= before {
            return false;
        }
        let frame = &mut self.frames[before];
        frame.silent |= silent;
        frame.silent_errors |= silent_errors;
        true
    }

    // ----- autocommands ---------------------------------------------------

    pub fn text_locked(&self) -> bool {
        self.frames.iter().any(|frame| matches!(&frame.kind, FrameKind::Autocmd { event, .. }
            if super::autocmd::event_name(*event) == "TextYankPost"))
    }

    /// Whether editor work caused by the current command may trigger events.
    /// Ordinary autocommands suppress recursion unless declared ++nested.
    pub fn automatic_events_allowed(&self) -> bool {
        !self.frames.iter().any(|frame| frame.noautocmd)
            && self.frames.iter().rev().find(|frame| frame.autocmd)
                .is_none_or(|frame| frame.autocmd_nested && frame.autocmd_level < 10)
    }

    pub fn queue_event(&mut self, event: usize, matched: impl Into<String>) {
        if !self.frames.iter().any(|frame| frame.noautocmd) {
            self.queued_events.push((event, matched.into()));
        }
    }

    pub fn queue_event_name(&mut self, event: &str, matched: impl Into<String>) {
        if let Some(event) = super::autocmd::event_index(event) {
            self.queue_event(event, matched);
        }
    }

    /// Push the autocommands for queued events as frames; they run before
    /// the next command of the current frame.
    pub(crate) fn fire_queued_events(&mut self, _host: &mut dyn ScriptHost) -> EvalResult<()> {
        if self.queued_events.is_empty() {
            return Ok(());
        }
        let events = std::mem::take(&mut self.queued_events);
        let mut frames = Vec::new();
        for (event, matched) in events {
            frames.extend(self.autocmd_frames(event, &matched, None));
        }
        for frame in frames.into_iter().rev() {
            if self.frames.len() >= MAX_NESTING {
                return Err(Fail::Error(VimError::new(218, "Autocommand nesting too deep")));
            }
            self.autocmd_depth += 1;
            self.frames.push(frame);
        }
        Ok(())
    }

    pub(crate) fn autocmd_frames(&mut self, event: usize, matched: &str, group: Option<usize>) -> Vec<Frame> {
        if self.frames.iter().any(|frame| frame.noautocmd) { return Vec::new(); }
        // Before the first command runs, frames are only queued siblings.
        let parent = (!self.current_command.is_empty()).then(|| self.frames.iter().rev().find(|frame| frame.autocmd)).flatten();
        let level = parent.map_or(0, |frame| frame.autocmd_level);
        if level >= 10 || parent.is_some_and(|frame| !frame.autocmd_nested) { return Vec::new(); }
        let file_event = !matches!(
            super::autocmd::event_name(event),
            "FileType" | "Syntax" | "ColorScheme" | "ColorSchemePre" | "User" | "OptionSet"
                | "SourcePre" | "SourcePost" | "TextYankPost" | "InsertEnter" | "InsertLeave"
                | "InsertLeavePre" | "VimEnter" | "GUIEnter" | "FocusGained" | "FocusLost"
        ) || matches!(super::autocmd::event_name(event), "SourcePre" | "SourcePost");
        let matching = self.state.autocmds().matching(event, group, matched, file_event);
        let mut frames = Vec::new();
        for autocmd in matching {
            if autocmd.once {
                self.state.autocmds().remove_id(autocmd.id);
            }
            let lines: Arc<[SourceLine]> = super::source::split_command_lines(autocmd.command.as_bytes(), 1).into();
            let mut frame = Frame::new(FrameKind::Autocmd { event, matched: matched.to_string() }, lines, autocmd.sid);
            frame.autocmd_nested = autocmd.nested;
            frame.autocmd_level = level + 1;
            frames.push(frame);
        }
        frames
    }

    /// Fire an event and run its autocommands synchronously to completion.
    pub fn fire_event_now(&mut self, host: &mut dyn ScriptHost, event: &str, matched: &str) -> EvalResult<()> {
        let Some(event) = super::autocmd::event_index(event) else { return Ok(()) };
        let frames = self.autocmd_frames(event, matched, None);
        for mut frame in frames {
            frame.boundary = true;
            self.autocmd_depth += 1;
            self.frames.push(frame);
            let base = self.frames.len() - 1;
            match self.run_nested(host, base) {
                Ok(_) | Err(Fail::Aborted) => {}
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }

    /// Queue an event's autocommands as resumable frames.
    pub fn push_event(&mut self, event: &str, matched: &str) -> bool {
        let Some(event) = super::autocmd::event_index(event) else { return false };
        let frames = self.autocmd_frames(event, matched, None);
        let pushed = !frames.is_empty();
        for frame in frames.into_iter().rev() {
            self.autocmd_depth += 1;
            self.frames.push(frame);
        }
        pushed
    }

    // ----- variables ------------------------------------------------------

    /// Resolve a variable name (with optional scope) to its scope dictionary
    /// and key. Unprefixed names are local in functions, global elsewhere.
    pub(crate) fn variable_slot(&self, host: &dyn ScriptHost, name: &[u8]) -> Result<(Dict, Vec<u8>, char), VimError> {
        let illegal = || VimError::new(461, format!("Illegal variable name: {}", String::from_utf8_lossy(name)));
        if name.len() >= 2 && name[1] == b':' {
            let key = name[2..].to_vec();
            let dict = match name[0] {
                b'g' => self.state.globals().clone(),
                b'v' => self.state.vim_vars().clone(),
                b'b' => self.state.buffer_vars().clone(),
                b't' => self.state.tab_vars().clone(),
                b'w' => self.state.window_vars(host.view_id()),
                b's' => self.state.script_vars(self.current_sid()).ok_or_else(illegal)?,
                b'l' | b'a' => match self.current_scope() {
                    Some(scope) => if name[0] == b'l' { scope.locals.clone() } else { scope.args.clone() },
                    None => return Err(VimError::undefined_variable(name)),
                },
                _ => return Err(illegal()),
            };
            return Ok((dict, key, name[0] as char));
        }
        match self.current_scope() {
            Some(scope) => Ok((scope.locals.clone(), name.to_vec(), 'l')),
            None if self.script_local_sids.contains(&self.current_sid()) => Ok((self.state.script_vars(self.current_sid()).ok_or_else(illegal)?, name.to_vec(), 's')),
            None => Ok((self.state.globals().clone(), name.to_vec(), 'g')),
        }
    }

    pub fn get_variable(&mut self, host: &mut dyn ScriptHost, name: &[u8]) -> EvalResult<Value> {
        self.lookup_variable(host, name, true)
    }
    pub(crate) fn lookup_variable(&mut self, host: &mut dyn ScriptHost, name: &[u8], autoload: bool) -> EvalResult<Value> {
        if let Some(value) = host.dynamic_variable(name) {
            return Ok(value);
        }
        if name.len() == 2 && name[1] == b':' {
            let (dict, _, _) = self.variable_slot(host, name)?;
            return Ok(Value::Dict(dict));
        }
        if name == b"self" {
            if let Some(dict) = self.current_scope().and_then(|scope| scope.self_dict.clone()) {
                return Ok(Value::Dict(dict));
            }
        }
        let explicit = name.len() >= 2 && name[1] == b':';
        let (dict, key, scope) = self.variable_slot(host, name)?;
        if let Some(value) = dict.get(&key) {
            return Ok(value);
        }
        if autoload && scope == 'g' && key.contains(&b'#') && host.autoload(self, &key)? {
            if let Some(value) = dict.get(&key) { return Ok(value); }
        }
        // Old names for v: variables ("count") come before closure scopes.
        if !explicit {
            if let Some(value) = compat_vvar(name).and_then(|vname| self.state.vim_vars().get(vname.as_bytes())) {
                return Ok(value);
            }
        }
        // Closures see the variables of their defining functions.
        if scope == 'l' || scope == 'a' {
            let mut outer = self.current_scope().and_then(|scope| scope.outer.clone());
            while let Some(found) = outer {
                let dict = if scope == 'a' { &found.args } else { &found.locals };
                if let Some(value) = dict.get(&key) {
                    return Ok(value);
                }
                outer = found.outer.clone();
            }
        }
        Err(Fail::Error(VimError::undefined_variable(name)))
    }

    /// Assign a variable, honoring locks, read-only `v:` variables and the
    /// Funcref naming rule. `lock` makes the binding constant (`:const`).
    pub fn set_variable(&mut self, host: &mut dyn ScriptHost, name: &[u8], value: Value, lock: bool) -> EvalResult<()> {
        let (mut dict, key, mut scope) = self.variable_slot(host, name)?;
        let explicit = name.len() >= 2 && name[1] == b':';
        if !explicit && !dict.contains(&key) {
            if let Some(vname) = compat_vvar(name) {
                // "count" names v:count, which is read-only.
                if !super::vvars::writable(vname) {
                    return Err(Fail::Error(VimError::new(46, format!("Cannot change read-only variable \"{}\"", String::from_utf8_lossy(name)))));
                }
                return self.set_variable(host, format!("v:{vname}").as_bytes(), value, lock);
            }
            // An existing variable of a defining function is shared.
            if scope == 'l' {
                let mut outer = self.current_scope().and_then(|scope| scope.outer.clone());
                while let Some(found) = outer {
                    if found.locals.contains(&key) {
                        dict = found.locals.clone();
                        break;
                    }
                    if found.args.contains(&key) {
                        dict = found.args.clone();
                        scope = 'a';
                        break;
                    }
                    outer = found.outer.clone();
                }
            }
        }
        if key.is_empty() || !key.iter().all(|b| super::expr::is_name_char(*b) || *b == b'#') || key[0].is_ascii_digit() && scope != 'a' {
            return Err(Fail::Error(VimError::new(461, format!("Illegal variable name: {}", String::from_utf8_lossy(name)))));
        }
        if scope == 'a' {
            return Err(Fail::Error(VimError::new(46, format!("Cannot change read-only variable \"{}\"", String::from_utf8_lossy(name)))));
        }
        if scope == 'v' {
            let key_text = String::from_utf8_lossy(&key).into_owned();
            if !super::vvars::writable(&key_text) {
                return Err(Fail::Error(VimError::new(46, format!("Cannot change read-only variable \"{}\"", String::from_utf8_lossy(name)))));
            }
            self.state.set_vvar(&key_text, value);
            return Ok(());
        }
        if let Value::Funcref(_) = &value {
            check_funcref_name(name, scope)?;
            if matches!(scope, 'g' | 'l') && self.state.function(&key).is_some() {
                return Err(Fail::Error(VimError::new(705, format!("Variable name conflicts with existing function: {}", String::from_utf8_lossy(name)))));
            }
        }
        let mut data = dict.data();
        match data.items.get_mut(key.as_slice()) {
            Some(item) => {
                if item.lock.is_locked() {
                    return Err(Fail::Error(VimError::new(741, format!("Value is locked: {}", String::from_utf8_lossy(name)))));
                }
                item.value = value;
                if lock {
                    item.lock = Lock::Fixed;
                }
            }
            None => {
                if data.lock.is_locked() {
                    return Err(Fail::Error(VimError::new(741, format!("Value is locked: {}", String::from_utf8_lossy(name)))));
                }
                data.items.insert(VStr::from(key), DictItem { value, lock: if lock { Lock::Fixed } else { Lock::Unlocked } });
            }
        }
        Ok(())
    }

    pub fn get_env(&mut self, host: &mut dyn ScriptHost, name: &str) -> String {
        host.getenv(name).unwrap_or_default()
    }

    // ----- functions --------------------------------------------------------

    pub(crate) fn define_lambda(&mut self, params: Vec<Param>, varargs: bool, body: VStr) -> VStr {
        let name = self.state.next_lambda_name();
        let function = UserFunction {
            name: name.clone(),
            params,
            varargs,
            body: FunctionBody::Lambda(body),
            flags: FunctionFlags { closure: true, ..Default::default() },
            sid: self.current_sid(),
            def_lnum: 0,
            outer: self.current_scope(),
        };
        self.state.define_function(function);
        name
    }

    /// Resolve a function name used in a call: a Funcref variable, a script
    /// function, a user function or a builtin.
    pub fn resolve_function_name(&mut self, host: &mut dyn ScriptHost, name: &[u8]) -> EvalResult<Funcref> {
        let scoped = name.len() > 2 && name[1] == b':';
        let first = if scoped { name[2] } else { name[0] };
        let could_be_variable = !name.starts_with(b"<SNR>")
            && !name.contains(&b'#')
            && (first.is_ascii_uppercase() || (scoped && matches!(name[0], b'l' | b'a' | b'g' | b's' | b'b' | b'w' | b't' | b'v')));
        if could_be_variable {
            if let Ok(Value::Funcref(funcref)) = self.get_variable(host, name) {
                return Ok(funcref);
            }
        }
        Ok(Funcref::named(self.canonical_function_name(name)))
    }

    /// `s:Foo` and `<SID>Foo` become `<SNR>{sid}_Foo`; `g:Foo` becomes `Foo`.
    pub fn canonical_function_name(&self, name: &[u8]) -> VStr {
        if name.len() > 2 && name[1] == b':' {
            match name[0] {
                b's' => return format!("<SNR>{}_{}", self.current_sid(), String::from_utf8_lossy(&name[2..])).into(),
                b'g' => return VStr::from(&name[2..]),
                _ => {}
            }
        }
        if name.len() > 5 && name[..5].eq_ignore_ascii_case(b"<SID>") {
            return format!("<SNR>{}_{}", self.current_sid(), String::from_utf8_lossy(&name[5..])).into();
        }
        VStr::from(name)
    }

    /// Call a Funcref (user function, lambda or builtin) synchronously.
    pub fn call_funcref(&mut self, host: &mut dyn ScriptHost, funcref: &Funcref, args: Vec<Value>, self_dict: Option<Dict>) -> EvalResult<Value> {
        let mut all = funcref.args().to_vec();
        all.extend(args);
        let dict = match (funcref.dict(), self_dict) {
            (Some(bound), _) if !funcref.auto_bound() => Some(bound.clone()),
            (_, Some(dict)) => Some(dict),
            (Some(bound), None) => Some(bound.clone()),
            (None, None) => None,
        };
        self.call_function(host, funcref.name().as_bytes(), all, dict)
    }

    pub fn call_function(&mut self, host: &mut dyn ScriptHost, name: &[u8], args: Vec<Value>, self_dict: Option<Dict>) -> EvalResult<Value> {
        self.charge(10)?;
        if name.contains(&b'#') && self.state.function(name).is_none() {
            host.autoload(self, name)?;
        }
        if let Some(function) = self.state.function(name) {
            return self.call_user_function(host, function, args, self_dict, None);
        }
        if is_builtin_name(name) {
            let text = String::from_utf8_lossy(name).into_owned();
            if let Some(result) = host.override_builtin(self, &text, &args) {
                return result;
            }
            if let Some(result) = super::builtins::call(self, host, &text, args.clone()) {
                return result;
            }
            if let Some(result) = host.call_function(self, &text, &args) {
                return result;
            }
            if super::builtins::is_vim_builtin(&text) {
                return Err(Fail::Error(VimError::new(117, format!("Unknown function: {text} (not supported by Viem)"))));
            }
        }
        Err(Fail::Error(VimError::unknown_function(name)))
    }

    /// Whether `name` refers to a callable function (for `exists('*name')`).
    pub fn function_exists(&self, host: &dyn ScriptHost, name: &[u8]) -> bool {
        let canonical = self.canonical_function_name(name);
        if self.state.function(canonical.as_bytes()).is_some() {
            return true;
        }
        if is_builtin_name(name) {
            let text = String::from_utf8_lossy(name);
            return super::builtins::is_implemented(&text) || host.has_function(&text);
        }
        false
    }

    /// Prepare the scope for a user function call.
    pub(crate) fn prepare_call(
        &mut self,
        host: &mut dyn ScriptHost,
        function: &Arc<UserFunction>,
        args: Vec<Value>,
        self_dict: Option<Dict>,
        range: Option<(i64, i64)>,
    ) -> EvalResult<Arc<FunctionScope>> {
        let name = function.name.to_str_lossy().into_owned();
        if function.flags.dict && self_dict.is_none() {
            return Err(Fail::Error(VimError::new(725, format!("Calling dict function without Dictionary: {name}"))));
        }
        let required = function.params.iter().filter(|param| param.default.is_none()).count();
        if args.len() < required {
            return Err(Fail::Error(VimError::argument(119, &name)));
        }
        if args.len() > function.params.len() && !function.varargs {
            return Err(Fail::Error(VimError::argument(118, &name)));
        }
        if self.call_depth >= MAX_FUNC_DEPTH {
            return Err(Fail::Error(VimError::new(132, "Function call depth is higher than 'maxfuncdepth'")));
        }
        let is_lambda = matches!(function.body, FunctionBody::Lambda(_));
        let locals = Dict::new();
        let arguments = Dict::new();
        let scope = Arc::new(FunctionScope {
            locals,
            args: arguments.clone(),
            self_dict: if function.flags.dict || is_lambda || self_dict.is_some() { self_dict } else { None },
            outer: if function.flags.closure || is_lambda { function.outer.clone() } else { None },
            function: function.name.clone(),
        });
        {
            let (first, last) = match range {
                Some(range) => range,
                None => {
                    let line = host.dynamic_variable(b"<cursor-line>").and_then(|v| v.to_number().ok()).unwrap_or(1);
                    (line, line)
                }
            };
            arguments.insert("firstline", first.into());
            arguments.insert("lastline", last.into());
        }
        let mut args = args.into_iter();
        // Lambda arguments are local variables; a: holds only a:0/a:000.
        let parameters = if is_lambda { scope.locals.clone() } else { arguments.clone() };
        for param in &function.params {
            let value = match args.next() {
                Some(Value::None) | None if param.default.is_some() => {
                    let default = param.default.clone().unwrap();
                    self.with_context(Some(scope.clone()), function.sid, |rt| {
                        let mut eval = Eval::new(rt, host, default.as_bytes(), 0, true);
                        eval.eval1()
                    })?
                }
                Some(value) => value,
                None => Value::Number(0),
            };
            parameters.insert(param.name.clone(), value);
        }
        let rest: Vec<Value> = args.collect();
        {
            arguments.insert("0", (rest.len() as i64).into());
            for (index, value) in rest.iter().enumerate() {
                arguments.insert(format!("{}", index + 1), value.clone());
            }
            arguments.insert("000", Value::list(rest));
        }
        {
            let mut data = arguments.data();
            data.lock = Lock::Fixed;
            for item in data.items.values_mut() {
                item.lock = Lock::Fixed;
            }
        }
        Ok(scope)
    }

    /// Build the frame for a user function call.
    pub(crate) fn function_frame(&mut self, function: Arc<UserFunction>, scope: Arc<FunctionScope>) -> Frame {
        let lines = match &function.body {
            FunctionBody::Lines(lines) => lines.clone(),
            FunctionBody::Lambda(body) => {
                let mut text = b"return ".to_vec();
                text.extend_from_slice(body.as_bytes());
                vec![SourceLine { text: text.into(), lnum: 1 }].into()
            }
        };
        let mut frame = Frame::new(FrameKind::Function, lines, function.sid);
        frame.scope = Some(scope);
        frame.abort_on_error = function.flags.abort || matches!(function.body, FunctionBody::Lambda(_));
        frame.function = Some(function);
        frame
    }

    /// Call a user function synchronously and return its value.
    pub fn call_user_function(
        &mut self,
        host: &mut dyn ScriptHost,
        function: Arc<UserFunction>,
        args: Vec<Value>,
        self_dict: Option<Dict>,
        range: Option<(i64, i64)>,
    ) -> EvalResult<Value> {
        let scope = self.prepare_call(host, &function, args, self_dict, range)?;
        let mut frame = self.function_frame(function, scope);
        frame.boundary = true;
        self.call_depth += 1;
        self.frames.push(frame);
        let base = self.frames.len() - 1;
        let saved = (self.did_emsg, self.call_aborted);
        self.call_aborted = false;
        let result = self.run_nested(host, base);
        // Errors inside a function without abort do not reach the caller.
        self.did_emsg = saved.0 || self.call_aborted;
        self.call_aborted = saved.1;
        result
    }

    /// Push a user function call as a resumable frame (statement-level `:call`).
    pub(crate) fn push_function_call(
        &mut self,
        host: &mut dyn ScriptHost,
        function: Arc<UserFunction>,
        args: Vec<Value>,
        self_dict: Option<Dict>,
        range: Option<(i64, i64)>,
    ) -> EvalResult<()> {
        let scope = self.prepare_call(host, &function, args, self_dict, range)?;
        let frame = self.function_frame(function, scope);
        self.call_depth += 1;
        self.frames.push(frame);
        Ok(())
    }
}

/// The message for an exception that nothing caught: errors show their
/// original text, thrown values E605.
pub fn uncaught_error(exception: &Exception) -> VimError {
    if exception.from_error {
        let text = exception.text();
        let message = match text.find(':') {
            Some(colon) if text.starts_with("Vim") => &text[colon + 1..],
            _ => text.as_str(),
        };
        if let Some(rest) = message.strip_prefix('E') {
            if let Some((code, body)) = rest.split_once(": ") {
                if let Ok(code) = code.parse() {
                    return VimError::new(code, body.to_string());
                }
            }
        }
        return VimError::plain(message.to_string());
    }
    VimError::new(605, format!("Exception not caught: {}", exception.text()))
}

pub(crate) fn is_builtin_name(name: &[u8]) -> bool {
    name.first().is_some_and(|b| b.is_ascii_lowercase()) && name.iter().all(|b| super::expr::is_name_char(*b))
}

/// Non-local exits handled by `Runtime::unwind`.
#[derive(Clone, Debug)]
pub(crate) enum Unwind {
    Break,
    Continue,
    Return(Value),
    Finish,
    Exception(Box<Exception>),
    Aborted,
}

/// Where the next command starts after executing one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Next {
    Column(usize),
    NextLine,
    Position(usize, usize),
    Unchanged,
}

pub(crate) struct CommandOutcome {
    pub next: Next,
    pub result: EvalResult<()>,
}

/// `count`, `version` and other old names that work without `v:`.
fn compat_vvar(name: &[u8]) -> Option<&'static str> {
    match name {
        b"count" => Some("count"),
        b"errmsg" => Some("errmsg"),
        b"shell_error" => Some("shell_error"),
        b"this_session" => Some("this_session"),
        b"version" => Some("version"),
        _ => None,
    }
}

fn check_funcref_name(name: &[u8], scope: char) -> Result<(), Fail> {
    if matches!(scope, 's' | 'w' | 'b' | 't') && name.len() > 2 && name[1] == b':' {
        return Ok(());
    }
    let first = if name.len() > 2 && name[1] == b':' { name[2] } else { name[0] };
    if !first.is_ascii_uppercase() {
        return Err(Fail::Error(VimError::new(704, format!("Funcref variable name must start with a capital: {}", String::from_utf8_lossy(name)))));
    }
    Ok(())
}
