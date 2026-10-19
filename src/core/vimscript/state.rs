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
    /// Weak ownership inventory includes cycles orphaned by reload/unlet.
    /// It does not keep an otherwise unused container alive.
    cyclic_scopes: Mutex<BTreeMap<usize, std::sync::Weak<Mutex<super::value::DictData>>>>,
    /// Type aliases and compiled signatures may outlive a reloaded namespace.
    /// Retain their class identities until this bounded script state closes.
    classes: Mutex<BTreeMap<String, Arc<super::objects::Class>>>,
    counters: Mutex<Counters>,
    commands: Mutex<BTreeMap<String, Arc<UserCommand>>>,
    autocmds: Mutex<super::autocmd::AutocmdTable>,
    messages: Mutex<VecDeque<String>>,
}

#[derive(Default)]
struct Counters {
    lambda: u64,
    numbered: u64,
    class: u64,
}

#[derive(Clone, Debug)]
pub struct ScriptEntry {
    pub name: String,
    pub vars: Dict,
    pub vim9: bool,
    pub imports: BTreeMap<VStr, String>,
    pub types: BTreeMap<String, super::types::Type>,
    pub loaded: bool,
    pub loading: bool,
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
        // Drain registry locks before walking values. A single iterative walk
        // handles shared graphs and does not consume the native call stack.
        let windows = std::mem::take(self.window_vars.get_mut().unwrap_or_else(|p|p.into_inner()));
        let scripts = std::mem::take(self.scripts.get_mut().unwrap_or_else(|p|p.into_inner()));
        let functions = std::mem::take(self.functions.get_mut().unwrap_or_else(|p|p.into_inner()));
        let scopes = std::mem::take(self.cyclic_scopes.get_mut().unwrap_or_else(|p|p.into_inner()));
        let classes = std::mem::take(self.classes.get_mut().unwrap_or_else(|p|p.into_inner()));
        let mut values = vec![Value::Dict(self.globals.clone()), Value::Dict(self.vim_vars.clone()), Value::Dict(self.buffer_vars.clone()), Value::Dict(self.tab_vars.clone())];
        values.extend(windows.into_values().map(Value::Dict));
        values.extend(scripts.into_iter().map(|s|Value::Dict(s.vars)));
        values.extend(scopes.into_values().filter_map(|d|d.upgrade()).map(|d|Value::Dict(Dict::from_shared(d))));
        values.extend(functions.into_values().map(|f|Value::Funcref(super::Funcref::named(f.name.clone()).specialized(f))));
        values.extend(classes.into_values().map(Value::Class));
        Value::clear_values(values);
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
            cyclic_scopes: Mutex::new(BTreeMap::new()),
            classes: Mutex::new(BTreeMap::new()),
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
        scripts.push(ScriptEntry { name: name.to_string(), vars: Dict::new(), vim9: false, imports: BTreeMap::new(), types: BTreeMap::new(), loaded: false, loading: false });
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
    pub(crate) fn type_alias(&self, sid: usize, name: &str) -> Option<super::types::Type> {
        sid.checked_sub(1).and_then(|i|lock(&self.0.scripts).get(i).and_then(|s|s.types.get(name).cloned()))
    }
    pub(crate) fn define_type(&self, sid: usize, name: String, ty: super::types::Type) -> Result<(), super::VimError> {
        let mut scripts = lock(&self.0.scripts);
        let script = sid.checked_sub(1).and_then(|i|scripts.get_mut(i)).ok_or_else(||super::VimError::new(1094, "Type outside script"))?;
        if script.types.contains_key(&name) {return Err(super::VimError::new(1041, "Redefining type"));} script.types.insert(name, ty); Ok(())
    }
    pub(crate) fn reset_vim9_script(&self, sid: usize) {
        let vars = {
            let mut scripts = lock(&self.0.scripts);
            let Some(script) = sid.checked_sub(1).and_then(|i|scripts.get_mut(i)) else {return;};
            script.types.clear(); script.imports.clear(); script.vars.clone()
        };
        {let mut data = vars.data(); data.items.clear(); data.declarations.clear(); if let Some(exports) = &mut data.exports {exports.clear();}}
        lock(&self.0.functions).retain(|name, function| function.sid != sid || !name.as_bytes().starts_with(b"<SNR>") || function.signature.as_ref().is_some_and(|s|s.owner.is_some()));
    }
    pub fn script_is_vim9(&self, sid: usize) -> bool {
        sid.checked_sub(1).and_then(|i|lock(&self.0.scripts).get(i).map(|s|s.vim9)).unwrap_or(false)
    }
    pub fn set_script_vim9(&self, sid: usize, value: bool) {
        if let Some(index) = sid.checked_sub(1) {
            if let Some(script) = lock(&self.0.scripts).get_mut(index) {script.vim9 = value;}
        }
    }
    pub fn module_status(&self, sid: usize) -> (bool, bool) {
        lock(&self.0.scripts).get(sid - 1).map_or((false, false), |s|(s.loaded, s.loading))
    }
    pub fn module_loading(&self, sid: usize, loading: bool, loaded: bool) {
        if let Some(s) = lock(&self.0.scripts).get_mut(sid - 1) {s.loading = loading; s.loaded = loaded;}
    }
    pub fn import(&self, sid: usize, alias: &[u8]) -> Option<String> {
        sid.checked_sub(1).and_then(|i|lock(&self.0.scripts).get(i).and_then(|s|s.imports.get(alias).cloned()))
    }
    pub fn add_import(&self, sid: usize, alias: VStr, path: String) -> Result<(), super::VimError> {
        let mut scripts = lock(&self.0.scripts);
        let script = sid.checked_sub(1).and_then(|i|scripts.get_mut(i)).ok_or_else(||super::VimError::new(1094, "Import outside a script"))?;
        if script.imports.contains_key(&alias) || script.imports.values().any(|p|p == &path) {
            return Err(super::VimError::new(1088, "Script already imported"));
        }
        script.imports.insert(alias, path); Ok(())
    }

    pub fn function(&self, name: &[u8]) -> Option<Arc<UserFunction>> {
        lock(&self.0.functions).get(name).cloned()
    }
    pub(crate) fn next_class_identity(&self) -> Result<u64, super::VimError> {
        let mut counters = lock(&self.0.counters);
        counters.class = counters.class.checked_add(1).ok_or_else(||super::VimError::new(342, "Class identity budget exhausted"))?;
        Ok(counters.class)
    }
    pub(crate) fn register_class(&self, class: Arc<super::objects::Class>) -> Result<(), super::VimError> {
        let mut classes = lock(&self.0.classes);
        if classes.len() >= 10_000 {return Err(super::VimError::new(342, "Script class budget exhausted"));}
        classes.insert(class.type_name(), class);
        Ok(())
    }
    pub(crate) fn class(&self, identity: &str) -> Option<Arc<super::objects::Class>> {lock(&self.0.classes).get(identity).cloned()}
    pub(crate) fn track_scope(&self, dict: &Dict) -> Result<(), super::VimError> {
        const MAX_SCOPES: usize = 100_000;
        let mut scopes = lock(&self.0.cyclic_scopes);
        if scopes.contains_key(&dict.id()) {scopes.insert(dict.id(), dict.downgrade()); return Ok(());}
        if scopes.len() >= MAX_SCOPES {
            scopes.retain(|_, scope|scope.strong_count() != 0);
            if scopes.len() >= MAX_SCOPES {return Err(super::VimError::new(342, "Script scope budget exhausted"));}
        }
        scopes.insert(dict.id(), dict.downgrade());
        Ok(())
    }
    pub fn define_function(&self, function: UserFunction) -> Result<(), super::VimError> {
        let mut outer = function.outer.clone();
        while let Some(scope) = outer {
            self.track_scope(&scope.locals)?;
            self.track_scope(&scope.args)?;
            outer = scope.outer.clone();
        }
        {
            let functions = lock(&self.0.functions);
            if functions.len() >= 100_000 && !functions.contains_key(&function.name) {return Err(super::VimError::new(342, "Script function budget exhausted"));}
        }
        lock(&self.0.functions).insert(function.name.clone(), Arc::new(function));
        Ok(())
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
    pub signature: Option<super::vim9::Signature>,
}

/// The `l:` and `a:` scopes of one function invocation.
#[derive(Clone, Debug)]
pub struct FunctionScope {
    pub self_object: Option<Arc<super::objects::Object>>,
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
