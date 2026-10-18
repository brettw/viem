//! Script state shared by every view of one buffer: variables, script-local
//! scopes, user functions, user commands, autocommands and message history.
//! Viem keeps this state buffer-wide, like registers and mappings; startup
//! initializes every buffer with the same configuration.
use super::value::{Dict, Lock, VStr, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

/// Bounded history for `:messages`, matching Vim's default of 200 lines.
pub const MESSAGE_HISTORY: usize = 200;

#[derive(Clone)]
pub struct ScriptState(Arc<StateInner>);

pub struct StateInner {
    pub globals: Dict,
    pub vim_vars: Dict,
    pub buffer_vars: Dict,
    pub tab_vars: Dict,
    window_vars: Mutex<BTreeMap<u64, Dict>>,
    scripts: Mutex<Vec<ScriptEntry>>,
    functions: Mutex<BTreeMap<VStr, Arc<UserFunction>>>,
    counters: Mutex<Counters>,
    commands: Mutex<BTreeMap<String, Arc<UserCommand>>>,
    autocmds: Mutex<super::autocmd::AutocmdTable>,
    messages: Mutex<VecDeque<String>>,
}

#[derive(Default)]
struct Counters {
    lambda: u64,
    numbered: u64,
}

#[derive(Clone, Debug)]
pub struct ScriptEntry {
    pub name: String,
    pub vars: Dict,
}

impl std::fmt::Debug for ScriptState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ScriptState")
    }
}

impl Default for ScriptState {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StateInner {
    fn drop(&mut self) {
        // Lists and dictionaries may reference each other; break the cycles.
        self.globals.clear_recursive();
        self.buffer_vars.clear_recursive();
        self.tab_vars.clear_recursive();
        for (_, dict) in std::mem::take(&mut *self.window_vars.lock().unwrap_or_else(|p| p.into_inner())) {
            dict.clear_recursive();
        }
        for script in std::mem::take(&mut *self.scripts.lock().unwrap_or_else(|p| p.into_inner())) {
            script.vars.clear_recursive();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl ScriptState {
    pub fn new() -> Self {
        let state = Self(Arc::new(StateInner {
            globals: Dict::new(),
            vim_vars: Dict::new(),
            buffer_vars: Dict::new(),
            tab_vars: Dict::new(),
            window_vars: Mutex::new(BTreeMap::new()),
            scripts: Mutex::new(Vec::new()),
            functions: Mutex::new(BTreeMap::new()),
            counters: Mutex::new(Counters::default()),
            commands: Mutex::new(BTreeMap::new()),
            autocmds: Mutex::new(Default::default()),
            messages: Mutex::new(VecDeque::new()),
        }));
        super::vvars::initialize(&state.0.vim_vars);
        state
    }
    pub fn inner(&self) -> &StateInner {
        &self.0
    }
    pub fn ptr_eq(&self, other: &ScriptState) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn globals(&self) -> &Dict {
        &self.0.globals
    }
    pub fn vim_vars(&self) -> &Dict {
        &self.0.vim_vars
    }
    pub fn buffer_vars(&self) -> &Dict {
        &self.0.buffer_vars
    }
    pub fn tab_vars(&self) -> &Dict {
        &self.0.tab_vars
    }
    pub fn window_vars(&self, view: u64) -> Dict {
        lock(&self.0.window_vars).entry(view).or_default().clone()
    }
    pub fn forget_window(&self, view: u64) {
        if let Some(dict) = lock(&self.0.window_vars).remove(&view) {
            dict.clear_recursive();
        }
    }

    /// Set a `v:` variable, bypassing its read-only lock.
    pub fn set_vvar(&self, name: &str, value: Value) {
        let mut data = self.0.vim_vars.data();
        let key: VStr = name.into();
        match data.items.get_mut(&key) {
            Some(item) => item.value = value,
            None => {
                data.items.insert(key, super::value::DictItem { value, lock: Lock::Fixed });
            }
        }
    }
    pub fn vvar(&self, name: &str) -> Value {
        self.0.vim_vars.get_str(name).unwrap_or_default()
    }

    /// The script ID for `name`, creating an entry on first use. IDs start at 1.
    pub fn script_id(&self, name: &str) -> usize {
        let mut scripts = lock(&self.0.scripts);
        if let Some(index) = scripts.iter().position(|script| script.name == name) {
            return index + 1;
        }
        scripts.push(ScriptEntry { name: name.to_string(), vars: Dict::new() });
        scripts.len()
    }
    /// A fresh script context that never matches a file, e.g. for the
    /// command line or an `:execute` without a script.
    pub fn script_vars(&self, sid: usize) -> Option<Dict> {
        lock(&self.0.scripts).get(sid.checked_sub(1)?).map(|script| script.vars.clone())
    }
    pub fn script_name(&self, sid: usize) -> Option<String> {
        lock(&self.0.scripts).get(sid.checked_sub(1)?).map(|script| script.name.clone())
    }
    pub fn scripts(&self) -> Vec<ScriptEntry> {
        lock(&self.0.scripts).clone()
    }

    pub fn function(&self, name: &[u8]) -> Option<Arc<UserFunction>> {
        lock(&self.0.functions).get(name).cloned()
    }
    pub fn define_function(&self, function: UserFunction) {
        lock(&self.0.functions).insert(function.name.clone(), Arc::new(function));
    }
    pub fn delete_function(&self, name: &[u8]) -> bool {
        lock(&self.0.functions).remove(name).is_some()
    }
    pub fn function_names(&self) -> Vec<VStr> {
        lock(&self.0.functions).keys().cloned().collect()
    }
    pub fn next_lambda_name(&self) -> VStr {
        let mut counters = lock(&self.0.counters);
        counters.lambda += 1;
        format!("<lambda>{}", counters.lambda).into()
    }
    pub fn next_numbered_name(&self) -> VStr {
        let mut counters = lock(&self.0.counters);
        counters.numbered += 1;
        counters.numbered.to_string().into()
    }

    pub fn user_command(&self, name: &str) -> Option<Arc<UserCommand>> {
        lock(&self.0.commands).get(name).cloned()
    }
    pub fn user_commands(&self) -> Vec<Arc<UserCommand>> {
        lock(&self.0.commands).values().cloned().collect()
    }
    /// Commands whose names start with `prefix`, for unique abbreviations.
    pub fn user_commands_with_prefix(&self, prefix: &str) -> Vec<Arc<UserCommand>> {
        lock(&self.0.commands)
            .range(prefix.to_string()..)
            .take_while(|(name, _)| name.starts_with(prefix))
            .map(|(_, command)| command.clone())
            .collect()
    }
    pub fn define_user_command(&self, command: UserCommand) {
        lock(&self.0.commands).insert(command.name.clone(), Arc::new(command));
    }
    pub fn delete_user_command(&self, name: &str) -> bool {
        lock(&self.0.commands).remove(name).is_some()
    }
    pub fn clear_user_commands(&self) {
        lock(&self.0.commands).clear();
    }

    pub fn autocmds(&self) -> MutexGuard<'_, super::autocmd::AutocmdTable> {
        lock(&self.0.autocmds)
    }

    pub fn record_message(&self, text: &str) {
        let mut messages = lock(&self.0.messages);
        for line in text.split('\n') {
            if messages.len() >= MESSAGE_HISTORY {
                messages.pop_front();
            }
            messages.push_back(line.to_string());
        }
    }
    pub fn messages(&self) -> Vec<String> {
        lock(&self.0.messages).iter().cloned().collect()
    }
    pub fn clear_messages(&self, keep: Option<usize>) {
        let mut messages = lock(&self.0.messages);
        let keep = keep.unwrap_or(0).min(messages.len());
        let drop = messages.len() - keep;
        messages.drain(..drop);
    }
}

/// A parameter of a user function.
#[derive(Clone, Debug)]
pub struct Param {
    pub name: VStr,
    /// Default value expression (`arg = expr`), evaluated at call time.
    pub default: Option<VStr>,
}

/// One logical line of a script after line continuation.
#[derive(Clone, Debug)]
pub struct SourceLine {
    pub text: VStr,
    /// One-based physical line number of the first contributing line.
    pub lnum: usize,
}

#[derive(Clone, Debug)]
pub enum FunctionBody {
    Lines(Arc<[SourceLine]>),
    /// A legacy lambda `{args -> expr}`.
    Lambda(VStr),
}

#[derive(Clone, Debug, Default)]
pub struct FunctionFlags {
    pub range: bool,
    pub abort: bool,
    pub dict: bool,
    pub closure: bool,
}

/// A user-defined function or lambda.
#[derive(Clone, Debug)]
pub struct UserFunction {
    /// Internal name: `Foo`, `<SNR>3_Foo`, `dict.fn` numbered `17`, `<lambda>4`.
    pub name: VStr,
    pub params: Vec<Param>,
    pub varargs: bool,
    pub body: FunctionBody,
    pub flags: FunctionFlags,
    pub sid: usize,
    /// Line of the `:function` command in its script, for messages.
    pub def_lnum: usize,
    /// Captured scope of the defining function for closures and lambdas.
    pub outer: Option<Arc<FunctionScope>>,
}

/// The `l:` and `a:` scopes of one function invocation.
#[derive(Debug)]
pub struct FunctionScope {
    pub locals: Dict,
    pub args: Dict,
    pub self_dict: Option<Dict>,
    pub outer: Option<Arc<FunctionScope>>,
    pub function: VStr,
}

/// `:command -nargs=...` argument count rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandArgs {
    Zero,
    One,
    Any,
    ZeroOrOne,
    OneOrMore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRange {
    None,
    /// `-range`: default current line.
    Line,
    /// `-range=%`: default whole file.
    File,
    /// `-range=N`: a count with default N, given before the name.
    Count(i64),
    /// `-count=N`: a count given before the name or as the first argument.
    CountArg(i64),
}

/// A user-defined Ex command.
#[derive(Clone, Debug)]
pub struct UserCommand {
    pub name: String,
    pub nargs: CommandArgs,
    pub range: CommandRange,
    pub bang: bool,
    pub bar: bool,
    pub register: bool,
    pub buffer: bool,
    pub keepscript: bool,
    pub complete: Option<String>,
    pub addr: Option<String>,
    pub replacement: VStr,
    pub sid: usize,
}
