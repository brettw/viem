//! A bounded, side-effect-free expression language for syntax-package setup.
//! Commands produced by macros/execute are sent back through the strict loader;
//! this module has no file, process, editor, or network execution primitive.
use super::{Arc, AtomicBool, BTreeMap, Ordering, VimPattern, VimRegexLimits, VimSetupContext};

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

mod builtins;
mod environment;
mod formatting;
mod functions;
mod parser;
use parser::Parser;
#[cfg(test)]
mod tests;

const MAX_DEPTH: usize = 64;
const MAX_VALUE_BYTES: usize = 256 * 1024;
const MAX_BINDINGS: usize = 1024;
const MAX_EXPRESSION_BYTES: usize = 64 * 1024;
const MAX_SETUP_STORAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Value {
    Number(i64),
    Text(String),
    List(Vec<Value>),
    Dictionary(BTreeMap<String, Value>),
    Scope(String),
}
impl Value {
    fn validate_depth(&self, depth: usize) -> Result<(), String> {
        if depth >= MAX_DEPTH {
            return Err("setup collection nesting depth budget exceeded".into());
        }
        match self {
            Self::List(values) => {
                for value in values {
                    value.validate_depth(depth + 1)?;
                }
            }
            Self::Dictionary(values) => {
                for value in values.values() {
                    value.validate_depth(depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn truth(&self) -> Result<bool, String> {
        Ok(self.number()? != 0)
    }
    fn number(&self) -> Result<i64, String> {
        match self {
            Self::Number(n) => Ok(*n),
            Self::Text(s) => Ok(s.parse().unwrap_or(0)),
            _ => Err("setup value is not a scalar".into()),
        }
    }
    pub(super) fn text(&self) -> Result<String, String> {
        match self {
            Self::Text(s) => Ok(s.clone()),
            Self::Number(n) => Ok(n.to_string()),
            _ => Err("setup value cannot be used as command text".into()),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Text(s) | Self::Scope(s) => s.len(),
            Self::Number(_) => 8,
            Self::List(v) => v.iter().map(|v| v.bytes() + 16).sum(),
            Self::Dictionary(v) => v.iter().map(|(k, v)| k.len() + v.bytes() + 64).sum(),
        }
    }
    fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Text(s) | Self::Scope(s) => s.len().saturating_mul(2),
                Self::Number(_) => 0,
                Self::List(v) => v
                    .iter()
                    .map(Value::storage_bytes)
                    .sum::<usize>()
                    .saturating_add(v.len().saturating_mul(std::mem::size_of::<Self>())),
                Self::Dictionary(v) => v
                    .iter()
                    .map(|(k, v)| k.len().saturating_mul(2) + v.storage_bytes() + 96)
                    .sum(),
            }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Value(Value),
    Variable(String),
    List(Vec<Expr>),
    Dictionary(Vec<(Expr, Expr)>),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    Member(Box<Expr>, String),
    Not(Box<Expr>),
    Negate(Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

pub(super) fn catchable_error(error: &str) -> Option<String> {
    if error.starts_with("unknown setup variable:") {
        Some(format!("E108: {error}"))
    } else if error.starts_with("unknown syntax setup variable:") {
        Some(format!("E121: {error}"))
    } else if error.starts_with("E392:") || error.starts_with("E484:") {
        Some(error.into())
    } else {
        None
    }
}

#[derive(Clone)]
struct Function {
    signature: Arc<str>,
    body: Arc<[String]>,
    compiled: Rc<RefCell<Option<Arc<CompiledFunction>>>>,
}
struct CompiledFunction {
    arguments: Vec<String>,
    variadic: bool,
    body: Arc<[functions::Statement]>,
}
#[derive(Clone)]
struct Macro {
    template: String,
    nargs: String,
    scope: Rc<RefCell<ScriptScope>>,
}
#[derive(Default)]
pub(super) struct ScriptScope {
    variables: BTreeMap<String, Value>,
    shared_collections: BTreeSet<String>,
    functions: BTreeMap<String, Function>,
}

pub(super) struct Setup<'a> {
    script: Rc<RefCell<ScriptScope>>,
    variables: BTreeMap<String, Value>,
    shared_collections: BTreeSet<String>,
    macros: BTreeMap<String, Macro>,
    global_functions: BTreeMap<String, (Rc<RefCell<ScriptScope>>, Function)>,
    prefix: String,
    input: Option<crate::document::syntax::SyntaxInputSnapshot>,
    pub(super) reads: super::super::VimSetupReads,
    filetype: String,
    filename: Option<String>,
    source_file: String,
    keyword_option: String,
    highlights: BTreeSet<String>,
    syntax_clusters: BTreeMap<String, (String, Vec<String>)>,
    emitted: Option<Vec<String>>,
    fuel: usize,
    storage_reserved: usize,
    cancel: Option<&'a AtomicBool>,
}
impl<'a> Setup<'a> {
    pub(super) fn new(context: &VimSetupContext, cancel: Option<&'a AtomicBool>) -> Self {
        let highlights = environment::initial_highlights();
        let initial_storage = highlights.iter().map(|name| name.len() + 96).sum::<usize>();
        Self {
            script: Rc::new(RefCell::new(ScriptScope::default())),
            variables: BTreeMap::new(),
            shared_collections: BTreeSet::new(),
            macros: BTreeMap::new(),
            global_functions: BTreeMap::new(),
            prefix: context.prefix.clone(),
            input: context.input.clone(),
            reads: super::super::VimSetupReads::default(),
            filetype: String::new(),
            filename: context.filename.clone(),
            source_file: String::new(),
            keyword_option: "@,48-57,_,192-255".into(),
            highlights,
            syntax_clusters: BTreeMap::new(),
            emitted: None,
            fuel: 8_000_000,
            storage_reserved: context
                .prefix
                .len()
                .saturating_mul(2)
                .saturating_add(initial_storage),
            cancel,
        }
    }
    pub(super) fn set_filetype(&mut self, filetype: &str) {
        self.filetype = filetype.to_owned();
    }
    pub(super) fn matches_exception(&mut self, source: &str, exception: &str) -> Result<bool, String> {
        if source.is_empty() { return Ok(true); }
        let (pattern, tail) = super::delimited(source)?;
        if !tail.trim().is_empty() { return Err("invalid setup catch pattern suffix".into()); }
        let pattern = VimPattern::compile(&pattern, false, VimRegexLimits::default())?;
        let cancel = self.cancel;
        pattern.is_match_text_with_control(exception, &mut self.fuel,
            &mut || cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)))
    }
    pub(super) fn update_syntax_cluster(&mut self, name: &str, groups: &[String]) -> Result<(), String> {
        let bytes = name.len() + groups.iter().map(String::len).sum::<usize>();
        self.reserve_storage(bytes.saturating_mul(2).saturating_add(128))?;
        self.charge(bytes)?;
        self.syntax_clusters.insert(name.to_ascii_lowercase(), (name.into(), groups.to_vec()));
        Ok(())
    }
    pub(super) fn append_syntax_cluster_group(&mut self, name: &str, group: &str) -> Result<(), String> {
        self.reserve_storage(name.len().saturating_add(group.len()).saturating_mul(2).saturating_add(128))?;
        self.charge(name.len() + group.len())?;
        self.syntax_clusters.entry(name.to_ascii_lowercase())
            .or_insert_with(|| (name.into(), Vec::new())).1.push(group.into());
        Ok(())
    }
    pub(super) fn clear_syntax_cluster(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            self.syntax_clusters.remove(&name.to_ascii_lowercase());
        } else {
            self.syntax_clusters.clear();
        }
    }
    pub(super) fn set_keyword_option(&mut self, value: &str) {
        self.keyword_option = value.to_owned();
    }
    pub(super) fn define_highlight(&mut self, name: &str) -> Result<(), String> {
        if name.starts_with('@') {
            return Ok(());
        }
        if name.is_empty() || name.len() > 128 {
            return Err("invalid setup highlight group name".into());
        }
        let name = name.to_ascii_lowercase();
        if !self.highlights.contains(&name) {
            self.reserve_storage(name.len().saturating_add(96))?;
            self.highlights.insert(name);
        }
        Ok(())
    }
    pub(super) fn set_source_file(&mut self, value: &str) -> String {
        std::mem::replace(&mut self.source_file, value.to_owned())
    }
    pub(super) fn set_exception(
        &mut self,
        value: Option<String>,
    ) -> Result<Option<String>, String> {
        if let Some(value) = &value {
            if value.len() > MAX_VALUE_BYTES {
                return Err("setup exception byte budget exceeded".into());
            }
            self.reserve_storage(value.len().saturating_mul(2).saturating_add(128))?;
        }
        let previous = self
            .variables
            .remove("v:exception")
            .map(|value| value.text())
            .transpose()?;
        if let Some(value) = value {
            self.variables
                .insert("v:exception".into(), Value::Text(value));
        }
        Ok(previous)
    }
    pub(super) fn environment_fingerprint(&mut self) -> Result<u64, String> {
        use std::hash::{Hash, Hasher};
        let bytes = self
            .variables
            .iter()
            .map(|(name, value)| name.len() + value.bytes())
            .sum::<usize>()
            + self
                .macros
                .iter()
                .map(|(name, value)| {
                    name.len()
                        + value.template.len()
                        + value
                            .scope
                            .borrow()
                            .variables
                            .iter()
                            .map(|(name, value)| name.len() + value.bytes())
                            .sum::<usize>()
                })
                .sum::<usize>();
        let functions_bytes = self
            .global_functions
            .iter()
            .map(|(name, (scope, function))| {
                name.len()
                    + function.signature.len()
                    + function.body.iter().map(String::len).sum::<usize>()
                    + scope
                        .borrow()
                        .variables
                        .iter()
                        .map(|(name, value)| name.len() + value.bytes())
                        .sum::<usize>()
            })
            .sum::<usize>();
        self.charge(
            bytes
                .saturating_add(functions_bytes)
                .saturating_add(self.syntax_clusters.iter().map(|(key, (name, groups))|
                    key.len() + name.len() + groups.iter().map(String::len).sum::<usize>()).sum::<usize>())
                .saturating_add(self.highlights.iter().map(String::len).sum::<usize>()),
        )?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.variables.hash(&mut hash);
        self.shared_collections.hash(&mut hash);
        self.filetype.hash(&mut hash);
        self.filename.hash(&mut hash);
        self.keyword_option.hash(&mut hash);
        self.highlights.hash(&mut hash);
        self.syntax_clusters.hash(&mut hash);
        for (name, (scope, function)) in &self.global_functions {
            name.hash(&mut hash);
            function.signature.hash(&mut hash);
            function.body.hash(&mut hash);
            scope.borrow().variables.hash(&mut hash);
        }
        for (name, value) in &self.macros {
            name.hash(&mut hash);
            value.template.hash(&mut hash);
            value.nargs.hash(&mut hash);
            value.scope.borrow().variables.hash(&mut hash);
        }
        Ok(hash.finish())
    }
    pub(super) fn begin_script(&mut self) -> Rc<RefCell<ScriptScope>> {
        std::mem::take(&mut self.script)
    }
    pub(super) fn end_script(&mut self, old: Rc<RefCell<ScriptScope>>) {
        self.script = old;
    }
    pub(super) fn replace_script(
        &mut self,
        scope: Rc<RefCell<ScriptScope>>,
    ) -> Rc<RefCell<ScriptScope>> {
        std::mem::replace(&mut self.script, scope)
    }
    fn charge(&mut self, cost: usize) -> Result<(), String> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err("syntax compilation cancelled".into());
        }
        self.fuel = self
            .fuel
            .checked_sub(cost)
            .ok_or("syntax setup work budget exceeded")?;
        Ok(())
    }
    fn reserve_storage(&mut self, bytes: usize) -> Result<(), String> {
        // Reservations are shared across nested scripts and deliberately not
        // refunded by replacement/cleanup: aliases and repeated redefinitions
        // cannot multiply retained compiler storage outside a single load cap.
        self.storage_reserved = self.storage_reserved.saturating_add(bytes);
        if self.storage_reserved > MAX_SETUP_STORAGE_BYTES {
            return Err("syntax setup aggregate storage budget exceeded".into());
        }
        Ok(())
    }
    pub(super) fn evaluate(&mut self, source: &str) -> Result<Value, String> {
        self.charge(source.len())?;
        let expression = Parser::parse(source)?;
        self.eval(&expression, &BTreeMap::new(), 0)
    }
    pub(super) fn evaluate_execute(&mut self, source: &str) -> Result<String, String> {
        self.charge(source.len())?;
        let expressions = Parser::parse_many(source)?;
        let values = self.eval_values(&expressions, &BTreeMap::new(), 0)?;
        let text = values
            .iter()
            .map(Value::text)
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        if text.len() > MAX_VALUE_BYTES {
            return Err("syntax setup execute byte budget exceeded".into());
        }
        Ok(text)
    }
    pub(super) fn call_statement(&mut self, source: &str) -> Result<Vec<String>, String> {
        if self.emitted.is_some() {
            return Err("nested syntax setup statement invocation is unsupported".into());
        }
        self.emitted = Some(Vec::new());
        let result = self.evaluate(source);
        let emitted = self.emitted.take().unwrap();
        result?;
        self.charge(emitted.iter().map(String::len).sum())?;
        super::commands::validate_function_output(&emitted)?;
        Ok(emitted)
    }
    pub(super) fn delete_function(&mut self, name: &str, force: bool) -> Result<(), String> {
        let removed = if name.starts_with("s:") {
            self.script.borrow_mut().functions.remove(name).is_some()
        } else {
            self.global_functions.remove(name).is_some()
        };
        if !removed && !force {
            return Err("unknown setup function".into());
        }
        Ok(())
    }
    pub(super) fn assign(&mut self, source: &str) -> Result<(), String> {
        let (name, expression) = source
            .split_once('=')
            .ok_or("setup assignment requires =")?;
        let name = name.trim();
        let (name, operator) = ["..", ".", "+", "-", "*", "/", "%"]
            .into_iter()
            .find_map(|operator| {
                name.strip_suffix(operator)
                    .map(|name| (name.trim_end(), Some(operator)))
            })
            .unwrap_or((name, None));
        let mut value = self.evaluate(expression.trim())?;
        if let Some(operator) = operator {
            let old = self.evaluate(name)?;
            value = self.binary(operator.trim(), old, value)?;
        }
        let mut shared = Vec::new();
        if matches!(value, Value::List(_) | Value::Dictionary(_)) {
            self.collection_references(&Parser::parse(expression.trim())?, &mut shared);
        }
        self.assign_value(name, value)?;
        if !shared.is_empty() {
            shared.push(name.to_owned());
            for name in shared {
                self.mark_shared_collection(&name);
            }
        }
        Ok(())
    }
    fn collection_references(&self, expression: &Expr, shared: &mut Vec<String>) {
        match expression {
            Expr::Variable(name) => {
                if self
                    .variable(name, &BTreeMap::new())
                    .is_ok_and(|value| matches!(value, Value::List(_) | Value::Dictionary(_)))
                {
                    shared.push(name.clone());
                }
            }
            Expr::List(values) => {
                for value in values {
                    self.collection_references(value, shared);
                }
            }
            Expr::Dictionary(values) => {
                for (_, value) in values {
                    self.collection_references(value, shared);
                }
            }
            Expr::Call(name, _)
                if matches!(
                    name.as_str(),
                    "copy" | "deepcopy" | "range" | "split" | "keys" | "items" | "matchlist"
                ) => {}
            Expr::Call(_, values) => {
                for value in values {
                    self.collection_references(value, shared);
                }
            }
            Expr::Index(value, _) | Expr::Member(value, _) | Expr::Slice(value, _, _) => {
                self.collection_references(value, shared)
            }
            Expr::Conditional(_, yes, no) => {
                self.collection_references(yes, shared);
                self.collection_references(no, shared);
            }
            Expr::Binary(_, left, right) => {
                self.collection_references(left, shared);
                self.collection_references(right, shared);
            }
            _ => {}
        }
    }
    fn mark_shared_collection(&mut self, name: &str) {
        let name = variable_name(name);
        if name.starts_with("s:") {
            self.script.borrow_mut().shared_collections.insert(name);
        } else {
            self.shared_collections.insert(name);
        }
    }
    fn require_unshared_collection(&self, name: &str) -> Result<(), String> {
        let name = variable_name(name);
        let shared = if name.starts_with("s:") {
            self.script.borrow().shared_collections.contains(&name)
        } else {
            self.shared_collections.contains(&name)
        };
        if shared {
            Err("mutation of aliased setup collections is unsupported; use copy() for independent setup values".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn assign_value(&mut self, name: &str, value: Value) -> Result<(), String> {
        value.validate_depth(0)?;
        if name.contains('[') {
            let target = Parser::parse(name)?;
            let mut indexes = Vec::new();
            let base = self.assignment_path(&target, &mut indexes)?;
            self.require_unshared_collection(&base)?;
            let mut collection = self.variable(&base, &BTreeMap::new())?;
            replace_index(&mut collection, &indexes, value)?;
            return self.assign_value(&base, collection);
        }
        validate_variable(name)?;
        // These editor options affect syntax semantics and cannot be mutated by
        // setup. Saving/restoring cpoptions is harmless: parsing is deterministic.
        if name.starts_with('&') && !matches!(name, "&cpo" | "&cpoptions") {
            return Err(format!("unsupported syntax setup option: {name}"));
        }
        if name.starts_with('&') {
            return Ok(());
        }
        self.reserve_storage(
            value
                .storage_bytes()
                .saturating_add(name.len())
                .saturating_add(128),
        )?;
        let name = variable_name(name);
        let name = name.as_str();
        let mut script = self.script.borrow_mut();
        let bindings = if name.starts_with("s:") {
            &mut script.variables
        } else {
            &mut self.variables
        };
        if bindings.len() >= MAX_BINDINGS && !bindings.contains_key(name) {
            return Err("syntax setup binding budget exceeded".into());
        }
        let retained = bindings
            .iter()
            .filter(|(key, _)| key.as_str() != name)
            .map(|(key, value)| key.len() + value.bytes())
            .sum::<usize>();
        if retained
            .saturating_add(name.len())
            .saturating_add(value.bytes())
            > MAX_VALUE_BYTES
        {
            return Err("syntax setup value byte budget exceeded".into());
        }
        bindings.insert(name.to_owned(), value);
        if name.starts_with("s:") {
            script.shared_collections.remove(name);
        } else {
            self.shared_collections.remove(name);
        }
        Ok(())
    }
    fn assignment_path(
        &mut self,
        target: &Expr,
        indexes: &mut Vec<Value>,
    ) -> Result<String, String> {
        match target {
            Expr::Variable(name) => Ok(name.clone()),
            Expr::Index(base, key) => {
                let base = self.assignment_path(base, indexes)?;
                indexes.push(self.eval(key, &BTreeMap::new(), 0)?);
                Ok(base)
            }
            _ => Err("unsupported setup assignment target".into()),
        }
    }
    pub(super) fn unlet(&mut self, names: &str, force: bool) -> Result<(), String> {
        for name in names.split_whitespace() {
            validate_variable(name)?;
            let name = variable_name(name);
            let name = name.as_str();
            let mut script = self.script.borrow_mut();
            let bindings = if name.starts_with("s:") {
                &mut script.variables
            } else {
                &mut self.variables
            };
            if bindings.remove(name).is_none() && !force {
                return Err(format!("unknown setup variable: {name}"));
            }
        }
        Ok(())
    }
    pub(super) fn define_function(
        &mut self,
        signature: &str,
        body: &[String],
    ) -> Result<(), String> {
        let (name, _) = signature
            .split_once('(')
            .ok_or("invalid setup function signature")?;
        if name.is_empty() || name.len() > 256 {
            return Err("invalid setup function name".into());
        }
        let name = name.trim();
        let bytes = body.iter().map(String::len).sum::<usize>();
        if body.len() > 1024 || bytes > MAX_EXPRESSION_BYTES {
            return Err("syntax setup function source budget exceeded".into());
        }
        self.reserve_storage(
            bytes
                .saturating_mul(2)
                .saturating_add(signature.len().saturating_mul(2))
                .saturating_add(body.len().saturating_mul(32))
                .saturating_add(256),
        )?;
        let function = Function {
            signature: signature.into(),
            body: body.to_vec().into(),
            compiled: Rc::new(RefCell::new(None)),
        };
        if name.starts_with("s:") {
            if self.script.borrow().functions.len() >= MAX_BINDINGS {
                return Err("syntax setup function budget exceeded".into());
            }
            self.script
                .borrow_mut()
                .functions
                .insert(name.into(), function);
        } else {
            if self.global_functions.len() >= MAX_BINDINGS {
                return Err("syntax setup function budget exceeded".into());
            }
            self.global_functions
                .insert(name.into(), (self.script.clone(), function));
        }
        Ok(())
    }
    pub(super) fn define_macro(&mut self, source: &str) -> Result<(), String> {
        let mut source = source.trim_start();
        let mut nargs = "0";
        while source.starts_with('-') {
            let (option, rest) = super::word(source)?;
            if let Some(value) = option.strip_prefix("-nargs=") {
                if !matches!(value, "0" | "1" | "*" | "+" | "?") {
                    return Err("unsupported syntax macro argument count".into());
                }
                nargs = value;
            } else if !matches!(option, "-bar" | "-buffer") {
                return Err(format!("unsupported syntax macro option: {option}"));
            }
            source = rest;
        }
        let (name, body) = super::word(source)?;
        if !identifier(name) || !name.starts_with(char::is_uppercase) || name.contains(':') {
            return Err("invalid syntax macro name".into());
        }
        if body.contains("<f-args>") {
            return Err("syntax macro <f-args> quoting is unsupported".into());
        }
        if self.macros.len() >= MAX_BINDINGS {
            return Err("syntax setup macro budget exceeded".into());
        }
        self.reserve_storage(
            name.len()
                .saturating_add(body.len().saturating_mul(2))
                .saturating_add(128),
        )?;
        self.macros.insert(
            name.into(),
            Macro {
                template: body.into(),
                nargs: nargs.into(),
                scope: self.script.clone(),
            },
        );
        Ok(())
    }
    pub(super) fn delete_macro(&mut self, name: &str) -> Result<(), String> {
        self.macros.remove(name).ok_or("unknown syntax macro")?;
        Ok(())
    }
    pub(super) fn expand_macro(
        &mut self,
        name: &str,
        arguments: &str,
    ) -> Result<Option<(String, Rc<RefCell<ScriptScope>>)>, String> {
        let Some(value) = self.macros.get(name).cloned() else {
            return Ok(None);
        };
        if value.nargs == "0" && !arguments.is_empty()
            || matches!(value.nargs.as_str(), "1" | "+") && arguments.is_empty()
        {
            return Err("syntax macro argument count mismatch".into());
        }
        let mut expanded = String::new();
        let mut source = value.template.as_str();
        while let Some(at) = source.find('<') {
            expanded.push_str(&source[..at]);
            source = &source[at..];
            if let Some(rest) = source.strip_prefix("<args>") {
                expanded.push_str(arguments);
                source = rest;
            } else if let Some(rest) = source.strip_prefix("<q-args>") {
                expanded.push('\'');
                expanded.push_str(&arguments.replace('\'', "''"));
                expanded.push('\'');
                source = rest;
            } else if let Some(rest) = source.strip_prefix("<lt>") {
                expanded.push('<');
                source = rest;
            } else {
                expanded.push('<');
                source = &source[1..];
            }
            if expanded.len() > MAX_VALUE_BYTES {
                return Err("syntax macro expansion budget exceeded".into());
            }
        }
        expanded.push_str(source);
        if expanded.len() > MAX_VALUE_BYTES {
            return Err("syntax macro expansion budget exceeded".into());
        }
        self.charge(expanded.len())?;
        Ok(Some((expanded, value.scope)))
    }
    fn variable(&self, name: &str, arguments: &BTreeMap<String, Value>) -> Result<Value, String> {
        if let Some(value) = arguments.get(name) {
            return Ok(value.clone());
        }
        let normalized;
        let name = if let Some(option) = name
            .strip_prefix("&l:")
            .or_else(|| name.strip_prefix("&g:"))
        {
            normalized = format!("&{option}");
            normalized.as_str()
        } else {
            name
        };
        match name {
            "v:true" => return Ok(Value::Number(1)),
            "v:false" => return Ok(Value::Number(0)),
            "v:version" | "version" => return Ok(Value::Number(902)),
            "&ft" | "&filetype" => return Ok(Value::Text(self.filetype.clone())),
            "&isk" | "&iskeyword" => return Ok(Value::Text(self.keyword_option.clone())),
            "&cpo" | "&cpoptions" | "&buftype" => return Ok(Value::Text(String::new())),
            "&pyxversion" => return Ok(Value::Number(3)),
            // The syntax engine consumes decoded UTF-8 with modern Vim grammar.
            "&enc" | "&encoding" => return Ok(Value::Text("utf-8".into())),
            "&cp" | "&compatible" => return Ok(Value::Number(0)),
            "g:" | "b:" | "s:" => return Ok(Value::Scope(name.into())),
            _ => {}
        }
        let local = if name.starts_with("l:") {
            name.to_owned()
        } else {
            format!("l:{name}")
        };
        if let Some(value) = arguments.get(&local) {
            return Ok(value.clone());
        }
        let script = self.script.borrow();
        let bindings = if name.starts_with("s:") {
            &script.variables
        } else if name.starts_with("a:") {
            arguments
        } else {
            &self.variables
        };
        let name = variable_name(name);
        bindings
            .get(&name)
            .cloned()
            .ok_or_else(|| format!("unknown syntax setup variable: {name}"))
    }
    fn eval(
        &mut self,
        expression: &Expr,
        arguments: &BTreeMap<String, Value>,
        depth: usize,
    ) -> Result<Value, String> {
        self.charge(1)?;
        if depth >= MAX_DEPTH {
            return Err("syntax setup expression depth budget exceeded".into());
        }
        let value = match expression {
            Expr::Value(v) => v.clone(),
            Expr::Variable(name) => self.variable(name, arguments)?,
            Expr::List(items) => Value::List(self.eval_values(items, arguments, depth + 1)?),
            Expr::Not(e) => Value::Number(i64::from(!self.eval(e, arguments, depth + 1)?.truth()?)),
            Expr::Negate(e) => Value::Number(
                self.eval(e, arguments, depth + 1)?
                    .number()?
                    .checked_neg()
                    .ok_or("syntax setup integer overflow")?,
            ),
            Expr::Dictionary(items) => {
                let mut values = BTreeMap::new();
                let mut bytes = 0usize;
                for (key, value) in items {
                    let key = self.eval(key, arguments, depth + 1)?.text()?;
                    let value = self.eval(value, arguments, depth + 1)?;
                    bytes = bytes
                        .saturating_add(key.len())
                        .saturating_add(value.bytes())
                        .saturating_add(64);
                    if bytes > MAX_VALUE_BYTES {
                        return Err("syntax setup dictionary byte budget exceeded".into());
                    }
                    if values.insert(key, value).is_some() {
                        return Err("duplicate setup dictionary key".into());
                    }
                }
                Value::Dictionary(values)
            }
            Expr::Index(value, key) => {
                let value = self.eval(value, arguments, depth + 1)?;
                let key = self.eval(key, arguments, depth + 1)?;
                self.lookup(&value, &key)?
                    .ok_or("setup index is out of range or key is absent")?
            }
            Expr::Slice(value, start, end) => {
                let value = self.eval(value, arguments, depth + 1)?;
                let start = start
                    .as_ref()
                    .map(|start| {
                        self.eval(start, arguments, depth + 1)
                            .and_then(|value| value.number())
                    })
                    .transpose()?;
                let end = end
                    .as_ref()
                    .map(|end| {
                        self.eval(end, arguments, depth + 1)
                            .and_then(|value| value.number())
                    })
                    .transpose()?;
                builtins::slice(value, start, end)?
            }
            Expr::Member(value, key) => {
                let value = self.eval(value, arguments, depth + 1)?;
                if matches!(value, Value::Dictionary(_) | Value::Scope(_)) {
                    self.lookup(&value, &Value::Text(key.clone()))?
                        .ok_or("setup dictionary key is absent")?
                } else {
                    let right = self.variable(key, arguments)?;
                    self.binary(".", value, right)?
                }
            }

            Expr::Conditional(condition, yes, no) => {
                let branch = if self.eval(condition, arguments, depth + 1)?.truth()? {
                    yes
                } else {
                    no
                };
                self.eval(branch, arguments, depth + 1)?
            }
            Expr::Binary(operator, left, right) => {
                let left = self.eval(left, arguments, depth + 1)?;
                if operator == "&&" && !left.truth()? {
                    return Ok(Value::Number(0));
                }
                if operator == "||" && left.truth()? {
                    return Ok(Value::Number(1));
                }
                let right = self.eval(right, arguments, depth + 1)?;
                self.binary(operator, left, right)?
            }
            Expr::Call(name, inputs) => {
                if name == "submatch" {
                    let inputs = self.eval_values(inputs, arguments, depth + 1)?;
                    let [Value::Number(index)] = inputs.as_slice() else {
                        return Err("unsupported setup submatch arguments".into());
                    };
                    return arguments
                        .get(&format!("v:setup_submatch{index}"))
                        .cloned()
                        .ok_or("submatch() requires a setup substitution expression".into());
                }
                if matches!(
                    name.as_str(),
                    "extend" | "add" | "insert" | "map" | "filter"
                ) {
                    return self.collection_call(name, inputs, arguments, depth + 1);
                }
                let inputs = self.eval_values(inputs, arguments, depth + 1)?;
                self.call(name, inputs, depth + 1)?
            }
        };
        value.validate_depth(0)?;
        if value.bytes() > MAX_VALUE_BYTES {
            return Err("syntax setup value byte budget exceeded".into());
        }
        self.charge(value.bytes())?;
        Ok(value)
    }
    fn eval_values(
        &mut self,
        expressions: &[Expr],
        arguments: &BTreeMap<String, Value>,
        depth: usize,
    ) -> Result<Vec<Value>, String> {
        let mut values = Vec::new();
        let mut bytes = 0usize;
        for expression in expressions {
            let value = self.eval(expression, arguments, depth)?;
            bytes = bytes.saturating_add(value.bytes()).saturating_add(16);
            if bytes > MAX_VALUE_BYTES {
                return Err("syntax setup value byte budget exceeded".into());
            }
            values.push(value);
        }
        Ok(values)
    }
    fn call(&mut self, name: &str, values: Vec<Value>, depth: usize) -> Result<Value, String> {
        match (name, values.as_slice()) {
            ("call", [Value::Text(name), Value::List(values)])
                if name.starts_with("s:") || self.global_functions.contains_key(name) =>
            {
                self.call(name, values.clone(), depth + 1)
            }
            ("exists", [Value::Text(name)]) => {
                let name = variable_name(name);
                let present = if name.starts_with("s:") {
                    self.script.borrow().variables.contains_key(&name)
                } else {
                    self.variables.contains_key(&name)
                };
                Ok(Value::Number(i64::from(present)))
            }
            ("has", [Value::Text(_)]) => {
                // No optional Vim editor/runtime facilities exist in this native
                // host. In particular conceal is unavailable in literal Code.
                Ok(Value::Number(0))
            }
            ("get", [Value::Scope(scope), Value::Text(key), default]) => {
                let name = format!("{scope}{key}");
                let script = self.script.borrow();
                let bindings = if scope == "s:" {
                    &script.variables
                } else {
                    &self.variables
                };
                Ok(bindings
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| default.clone()))
            }
            ("index", [Value::List(items), value]) => Ok(Value::Number(
                items
                    .iter()
                    .position(|item| item == value)
                    .map_or(-1, |n| n as i64),
            )),
            ("getline", [Value::Number(start), Value::Number(end)]) => {
                if *start < 1 || *end < *start || *end > 32 {
                    return Err("setup getline is confined to the first 32 supplied lines".into());
                }
                if self.input.is_some() {
                    let last = super::super::setup_line_count(self.input.as_ref().unwrap()) as i64;
                    return (*start..=(*end).min(last))
                        .map(|line| self.buffer_line(line).map(Value::Text))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Value::List);
                }
                Ok(Value::List(
                    self.prefix
                        .split('\n')
                        .skip(*start as usize - 1)
                        .take((*end - *start + 1) as usize)
                        .map(|s| Value::Text(s.to_owned()))
                        .collect(),
                ))
            }
            ("join", [Value::List(items), Value::Text(separator)]) => {
                let strings = items
                    .iter()
                    .map(Value::text)
                    .collect::<Result<Vec<_>, _>>()?;
                let size = strings
                    .iter()
                    .map(String::len)
                    .sum::<usize>()
                    .saturating_add(
                        separator
                            .len()
                            .saturating_mul(strings.len().saturating_sub(1)),
                    );
                if size > MAX_VALUE_BYTES {
                    return Err("syntax setup join byte budget exceeded".into());
                }
                Ok(Value::Text(strings.join(separator)))
            }
            _ if name.starts_with("s:") || self.global_functions.contains_key(name) => {
                let (scope, function) = if name.starts_with("s:") {
                    let function = self
                        .script
                        .borrow()
                        .functions
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("unknown setup function: {name}"))?;
                    (self.script.clone(), function)
                } else {
                    self.global_functions.get(name).cloned().unwrap()
                };
                let compiled = function.compiled.borrow().clone();
                let function = if let Some(compiled) = compiled {
                    compiled
                } else {
                    self.charge(function.body.iter().map(String::len).sum())?;
                    let (_, arguments, variadic, body) =
                        functions::parse(&function.signature, &function.body)?;
                    self.reserve_storage(
                        function
                            .body
                            .iter()
                            .map(String::len)
                            .sum::<usize>()
                            .saturating_mul(4),
                    )?;
                    let compiled = Arc::new(CompiledFunction {
                        arguments,
                        variadic,
                        body: body.into(),
                    });
                    *function.compiled.borrow_mut() = Some(compiled.clone());
                    compiled
                };
                if values.len() < function.arguments.len()
                    || (!function.variadic && function.arguments.len() != values.len())
                {
                    return Err("setup function argument count mismatch".into());
                }
                let extras = values[function.arguments.len()..].to_vec();
                let mut arguments: BTreeMap<String, Value> = function
                    .arguments
                    .iter()
                    .zip(values)
                    .map(|(name, value)| (format!("a:{name}"), value))
                    .collect();
                if function.variadic {
                    arguments.insert("a:0".into(), Value::Number(extras.len() as i64));
                    for (index, value) in extras.iter().enumerate() {
                        arguments.insert(format!("a:{}", index + 1), value.clone());
                    }
                    arguments.insert("a:000".into(), Value::List(extras));
                }
                let previous = std::mem::replace(&mut self.script, scope);
                let result = functions::run(self, &function.body, arguments, depth);
                self.script = previous;
                result
            }
            _ => self.builtin(name, values, depth),
        }
    }
}
fn replace_index(collection: &mut Value, indexes: &[Value], value: Value) -> Result<(), String> {
    let Some((key, rest)) = indexes.split_first() else {
        *collection = value;
        return Ok(());
    };
    match (collection, key) {
        (Value::Dictionary(items), Value::Text(key)) => {
            if rest.is_empty() {
                if items.len() >= MAX_BINDINGS && !items.contains_key(key) {
                    return Err("syntax setup dictionary item budget exceeded".into());
                }
                items.insert(key.clone(), value);
                Ok(())
            } else {
                replace_index(
                    items.get_mut(key).ok_or("setup dictionary key is absent")?,
                    rest,
                    value,
                )
            }
        }
        (Value::List(items), Value::Number(index)) => {
            let index = if *index < 0 {
                items.len() as i64 + index
            } else {
                *index
            };
            let index = usize::try_from(index).map_err(|_| "setup list index is out of range")?;
            replace_index(
                items
                    .get_mut(index)
                    .ok_or("setup list index is out of range")?,
                rest,
                value,
            )
        }
        _ => Err("setup indexed assignment requires a list or dictionary".into()),
    }
}
fn equal(left: &Value, right: &Value) -> Result<bool, String> {
    match (left, right) {
        (Value::Number(_), _) | (_, Value::Number(_)) => Ok(left.number()? == right.number()?),
        _ => Ok(left == right),
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'#'))
}
fn variable_name(name: &str) -> String {
    if name.contains(':') || name.starts_with('&') {
        name.to_owned()
    } else {
        format!("g:{name}")
    }
}
fn validate_variable(s: &str) -> Result<(), String> {
    let name = s.strip_prefix('&').unwrap_or(s);
    if !identifier(name)
        || name.len() > 128
        || name.matches(':').count() > 1
        || name.ends_with(':')
        || name.starts_with("a:")
        || name.starts_with("v:")
    {
        return Err("invalid setup variable name".into());
    }
    if name.contains(':')
        && !["s:", "b:", "g:"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    {
        return Err("unsupported setup variable scope".into());
    }
    Ok(())
}
