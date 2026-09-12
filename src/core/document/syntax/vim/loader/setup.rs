//! A bounded, side-effect-free expression language for syntax-package setup.
//! Commands produced by macros/execute are sent back through the strict loader;
//! this module has no file, process, editor, or network execution primitive.
use super::{Arc, AtomicBool, BTreeMap, Ordering, VimPattern, VimRegexLimits, VimSetupContext};

use std::{cell::RefCell, rc::Rc};

const MAX_DEPTH: usize = 64;
const MAX_VALUE_BYTES: usize = 256 * 1024;
const MAX_BINDINGS: usize = 1024;
const MAX_EXPRESSION_BYTES: usize = 64 * 1024;
const MAX_SETUP_STORAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Number(i64),
    Text(String),
    List(Vec<Value>),
    Scope(String),
}
impl Value {
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
                    .saturating_mul(2),
            }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Value(Value),
    Variable(String),
    List(Vec<Expr>),
    Not(Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Clone)]
struct Function {
    arguments: Vec<String>,
    result: Arc<Expr>,
}
#[derive(Clone)]
enum Macro {
    Arguments {
        suffix: String,
        scope: Rc<RefCell<ScriptScope>>,
    },
    ExecuteSuffix {
        expression: Arc<Expr>,
        scope: Rc<RefCell<ScriptScope>>,
    },
}
#[derive(Default)]
pub(super) struct ScriptScope {
    variables: BTreeMap<String, Value>,
    functions: BTreeMap<String, Function>,
}

pub(super) struct Setup<'a> {
    script: Rc<RefCell<ScriptScope>>,
    variables: BTreeMap<String, Value>,
    macros: BTreeMap<String, Macro>,
    prefix: String,
    fuel: usize,
    storage_reserved: usize,
    cancel: Option<&'a AtomicBool>,
}
impl<'a> Setup<'a> {
    pub(super) fn new(context: &VimSetupContext, cancel: Option<&'a AtomicBool>) -> Self {
        Self {
            script: Rc::new(RefCell::new(ScriptScope::default())),
            variables: BTreeMap::new(),
            macros: BTreeMap::new(),
            prefix: context.prefix.clone(),
            fuel: 8_000_000,
            storage_reserved: context.prefix.len().saturating_mul(2),
            cancel,
        }
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
    pub(super) fn assign(&mut self, source: &str) -> Result<(), String> {
        let (name, expression) = source
            .split_once('=')
            .ok_or("setup assignment requires =")?;
        let name = name.trim();
        validate_variable(name)?;
        // These editor options affect syntax semantics and cannot be mutated by
        // setup. Saving/restoring cpoptions is harmless: parsing is deterministic.
        if name.starts_with('&') && !matches!(name, "&cpo" | "&cpoptions") {
            return Err(format!("unsupported syntax setup option: {name}"));
        }
        let value = self.evaluate(expression.trim())?;
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
        Ok(())
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
        let (name, args) = signature
            .split_once('(')
            .ok_or("invalid setup function signature")?;
        let args = args
            .strip_suffix(')')
            .ok_or("unsupported setup function attributes")?;
        if !name.starts_with("s:") || !identifier(name) {
            return Err("only script-local setup functions are supported".into());
        }
        let arguments = if args.trim().is_empty() {
            vec![]
        } else {
            args.split(',')
                .map(|a| a.trim().to_owned())
                .collect::<Vec<_>>()
        };
        if arguments.len() > 16 || arguments.iter().any(|a| !identifier(a) || a.contains(':')) {
            return Err("invalid setup function parameters".into());
        }
        let [statement] = body else {
            return Err(
                "setup functions require exactly one side-effect-free return expression".into(),
            );
        };
        let result = Parser::parse(
            statement
                .strip_prefix("return ")
                .ok_or("setup functions only support return")?,
        )?;
        validate_pure_expression(&result, 0)?;
        self.reserve_storage(
            expression_storage(&result)
                .saturating_add(signature.len().saturating_mul(2))
                .saturating_add(256),
        )?;
        if self.script.borrow().functions.len() >= MAX_BINDINGS {
            return Err("syntax setup function budget exceeded".into());
        }
        self.script.borrow_mut().functions.insert(
            name.into(),
            Function {
                arguments,
                result: Arc::new(result),
            },
        );
        Ok(())
    }
    pub(super) fn define_macro(&mut self, source: &str) -> Result<(), String> {
        let source = source
            .strip_prefix("-nargs=*")
            .ok_or("only -nargs=* syntax macros are supported")?
            .trim_start();
        let (name, body) = super::word(source)?;
        if !identifier(name) || !name.starts_with(char::is_uppercase) || name.contains(':') {
            return Err("invalid syntax macro name".into());
        }
        let value = if let Some(suffix) = body.strip_prefix("<args>") {
            if !matches!(suffix.trim(), "" | "fold") {
                return Err("syntax macro supports only literal fold suffix".into());
            }
            Macro::Arguments {
                suffix: suffix.trim().into(),
                scope: self.script.clone(),
            }
        } else if let Some(expression) = body.strip_prefix("execute <q-args> ") {
            let expression = Parser::parse(expression)?;
            validate_pure_expression(&expression, 0)?;
            Macro::ExecuteSuffix {
                expression: Arc::new(expression),
                scope: self.script.clone(),
            }
        } else {
            return Err("unsupported syntax command macro body".into());
        };
        if self.macros.len() >= MAX_BINDINGS {
            return Err("syntax setup macro budget exceeded".into());
        }
        self.reserve_storage(name.len().saturating_add(128).saturating_add(match &value {
            Macro::Arguments { suffix, .. } => suffix.len().saturating_mul(2),
            Macro::ExecuteSuffix { expression, .. } => expression_storage(expression),
        }))?;
        self.macros.insert(name.into(), value);
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
        let (suffix, scope) = match value {
            Macro::Arguments { suffix, scope } => (suffix, scope),
            Macro::ExecuteSuffix { expression, scope } => {
                let caller = std::mem::replace(&mut self.script, scope.clone());
                let result = self.eval(&expression, &BTreeMap::new(), 0);
                self.script = caller;
                (result?.text()?, scope)
            }
        };
        self.charge(arguments.len().saturating_add(suffix.len()))?;
        if arguments.len().saturating_add(suffix.len()) > MAX_VALUE_BYTES {
            return Err("syntax macro expansion budget exceeded".into());
        }
        Ok(Some((format!("{arguments} {suffix}"), scope)))
    }
    fn variable(&self, name: &str, arguments: &BTreeMap<String, Value>) -> Result<Value, String> {
        match name {
            "v:true" => return Ok(Value::Number(1)),
            "v:false" => return Ok(Value::Number(0)),
            "&cpo" | "&cpoptions" | "&buftype" => return Ok(Value::Text(String::new())),
            "&pyxversion" => return Ok(Value::Number(3)),
            "g:" | "b:" | "s:" => return Ok(Value::Scope(name.into())),
            _ => {}
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
                match operator.as_str() {
                    "&&" | "||" => Value::Number(i64::from(right.truth()?)),
                    ".." | "." => Value::Text(format!("{}{}", left.text()?, right.text()?)),
                    "==" | "==#" => Value::Number(i64::from(equal(&left, &right)?)),
                    "!=" | "!=#" => Value::Number(i64::from(!equal(&left, &right)?)),
                    ">" => Value::Number(i64::from(left.number()? > right.number()?)),
                    ">=" => Value::Number(i64::from(left.number()? >= right.number()?)),
                    "<" => Value::Number(i64::from(left.number()? < right.number()?)),
                    "<=" => Value::Number(i64::from(left.number()? <= right.number()?)),
                    "=~#" | "!~#" | "=~" | "!~" => {
                        let pattern =
                            VimPattern::compile(&right.text()?, false, VimRegexLimits::default())?;
                        let cancel = self.cancel;
                        let found = pattern.is_match_text_with_control(
                            &left.text()?,
                            &mut self.fuel,
                            &mut || cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)),
                        )?;
                        Value::Number(i64::from(found ^ operator.starts_with('!')))
                    }
                    _ => return Err("unsupported setup binary operator".into()),
                }
            }
            Expr::Call(name, inputs) => {
                let inputs = self.eval_values(inputs, arguments, depth + 1)?;
                self.call(name, inputs, depth + 1)?
            }
        };
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
            _ if name.starts_with("s:") => {
                let function = self
                    .script
                    .borrow()
                    .functions
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown setup function: {name}"))?;
                if function.arguments.len() != values.len() {
                    return Err("setup function argument count mismatch".into());
                }
                let arguments = function
                    .arguments
                    .iter()
                    .zip(values)
                    .map(|(name, value)| (format!("a:{name}"), value))
                    .collect();
                self.eval(&function.result, &arguments, depth)
            }
            _ => Err(format!(
                "unsupported syntax setup function or arguments: {name}"
            )),
        }
    }
}
fn expression_storage(expression: &Expr) -> usize {
    std::mem::size_of::<Expr>()
        + match expression {
            Expr::Value(v) => v.storage_bytes(),
            Expr::Variable(name) => name.len().saturating_mul(2),
            Expr::List(values) => values
                .iter()
                .map(expression_storage)
                .sum::<usize>()
                .saturating_mul(2),
            Expr::Not(e) => expression_storage(e),
            Expr::Binary(operator, left, right) => {
                operator.len().saturating_mul(2)
                    + expression_storage(left)
                    + expression_storage(right)
            }
            Expr::Conditional(condition, yes, no) => {
                expression_storage(condition) + expression_storage(yes) + expression_storage(no)
            }
            Expr::Call(name, values) => {
                name.len().saturating_mul(2)
                    + values
                        .iter()
                        .map(expression_storage)
                        .sum::<usize>()
                        .saturating_mul(2)
            }
        }
}
fn validate_pure_expression(expression: &Expr, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("syntax setup expression depth budget exceeded".into());
    }
    match expression {
        Expr::Call(name, expressions) => {
            if !matches!(
                name.as_str(),
                "exists" | "has" | "get" | "index" | "getline" | "join"
            ) && !name.starts_with("s:")
            {
                return Err(format!("unsupported syntax setup function: {name}"));
            }
            for e in expressions {
                validate_pure_expression(e, depth + 1)?;
            }
        }
        Expr::List(expressions) => {
            for e in expressions {
                validate_pure_expression(e, depth + 1)?;
            }
        }
        Expr::Not(e) => validate_pure_expression(e, depth + 1)?,
        Expr::Binary(_, left, right) => {
            validate_pure_expression(left, depth + 1)?;
            validate_pure_expression(right, depth + 1)?;
        }
        Expr::Conditional(condition, yes, no) => {
            validate_pure_expression(condition, depth + 1)?;
            validate_pure_expression(yes, depth + 1)?;
            validate_pure_expression(no, depth + 1)?;
        }
        Expr::Value(_) | Expr::Variable(_) => {}
    }
    Ok(())
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
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':'))
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Name(String),
    String(String),
    Number(i64),
    Symbol(String),
    End,
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
}
impl Parser {
    fn parse(source: &str) -> Result<Expr, String> {
        if source.len() > MAX_EXPRESSION_BYTES {
            return Err("syntax setup expression byte budget exceeded".into());
        }
        let mut parser = Self {
            tokens: lex(source)?,
            at: 0,
        };
        let expression = parser.expression(0, 0)?;
        if parser.peek() != &Token::End {
            return Err("unsupported trailing setup expression".into());
        }
        Ok(expression)
    }
    fn peek(&self) -> &Token {
        self.tokens.get(self.at).unwrap_or(&Token::End)
    }
    fn consume(&mut self, symbol: &str) -> bool {
        if self.peek() == &Token::Symbol(symbol.into()) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, symbol: &str) -> Result<(), String> {
        if self.consume(symbol) {
            Ok(())
        } else {
            Err(format!("expected setup token: {symbol}"))
        }
    }
    fn expression(&mut self, minimum: u8, depth: usize) -> Result<Expr, String> {
        if depth >= MAX_DEPTH {
            return Err("syntax setup parse depth budget exceeded".into());
        }
        let mut left = if self.consume("!") {
            Expr::Not(Box::new(self.expression(6, depth + 1)?))
        } else if self.consume("(") {
            let e = self.expression(0, depth + 1)?;
            self.expect(")")?;
            e
        } else if self.consume("[") {
            Expr::List(self.sequence("]", depth + 1)?)
        } else {
            let token = self.peek().clone();
            self.at += 1;
            match token {
                Token::String(s) => Expr::Value(Value::Text(s)),
                Token::Number(n) => Expr::Value(Value::Number(n)),
                Token::Name(name) if self.consume("(") => {
                    Expr::Call(name, self.sequence(")", depth + 1)?)
                }
                Token::Name(name) => Expr::Variable(name),
                _ => return Err("unsupported setup expression atom".into()),
            }
        };
        loop {
            if minimum <= 7 && self.consume("->") {
                let Token::Name(name) = self.peek().clone() else {
                    return Err("setup method name required".into());
                };
                self.at += 1;
                self.expect("(")?;
                let mut values = vec![left];
                values.extend(self.sequence(")", depth + 1)?);
                left = Expr::Call(name, values);
                continue;
            }
            let Token::Symbol(operator) = self.peek().clone() else {
                break;
            };
            let precedence = match operator.as_str() {
                "?" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" | "==#" | "!=#" | "<" | "<=" | ">" | ">=" | "=~#" | "!~#" | "=~"
                | "!~" => 4,
                ".." | "." => 5,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.at += 1;
            if operator == "?" {
                let yes = self.expression(0, depth + 1)?;
                self.expect(":")?;
                let no = self.expression(precedence, depth + 1)?;
                left = Expr::Conditional(Box::new(left), Box::new(yes), Box::new(no));
            } else {
                let right = self.expression(precedence + 1, depth + 1)?;
                left = Expr::Binary(operator, Box::new(left), Box::new(right));
            }
        }
        Ok(left)
    }
    fn sequence(&mut self, end: &str, depth: usize) -> Result<Vec<Expr>, String> {
        let mut values = vec![];
        if self.consume(end) {
            return Ok(values);
        }
        loop {
            if values.len() >= 1024 {
                return Err("syntax setup list budget exceeded".into());
            }
            values.push(self.expression(0, depth)?);
            if self.consume(end) {
                break;
            }
            self.expect(",")?;
        }
        Ok(values)
    }
}
fn lex(source: &str) -> Result<Vec<Token>, String> {
    let mut chars = source.char_indices().peekable();
    let mut tokens = vec![];
    while let Some((at, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if tokens.len() >= 4096 {
            return Err("syntax setup token budget exceeded".into());
        }
        if ch == '\'' || ch == '"' {
            let mut value = String::new();
            let mut closed = false;
            while let Some((_, c)) = chars.next() {
                if c == ch {
                    if ch == '\'' && chars.peek().is_some_and(|(_, c)| *c == '\'') {
                        chars.next();
                        value.push('\'');
                        continue;
                    }
                    closed = true;
                    break;
                }
                if c == '\\' && ch == '"' {
                    let (_, next) = chars.next().ok_or("unterminated setup string escape")?;
                    value.push(match next {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{8}',
                        'e' => '\u{1b}',
                        '\\' => '\\',
                        '"' => '"',
                        _ => return Err(format!("unsupported setup string escape: \\{next}")),
                    });
                } else {
                    value.push(c);
                }
            }
            if !closed {
                return Err("unterminated setup string".into());
            }
            tokens.push(Token::String(value));
            continue;
        }
        if ch.is_ascii_digit() || ch == '-' && chars.peek().is_some_and(|(_, c)| c.is_ascii_digit())
        {
            let mut end = at + ch.len_utf8();
            while chars.peek().is_some_and(|(_, c)| c.is_ascii_digit()) {
                let (at, c) = chars.next().unwrap();
                end = at + c.len_utf8();
            }
            tokens.push(Token::Number(
                source[at..end]
                    .parse()
                    .map_err(|_| "invalid setup number")?,
            ));
            continue;
        }
        if ch.is_ascii_alphabetic()
            || ch == '_'
            || ch == '&' && chars.peek().is_some_and(|(_, c)| c.is_ascii_alphabetic())
        {
            let mut end = at + ch.len_utf8();
            while chars
                .peek()
                .is_some_and(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '_' | ':'))
            {
                let (at, c) = chars.next().unwrap();
                end = at + c.len_utf8();
            }
            tokens.push(Token::Name(source[at..end].into()));
            continue;
        }
        let operator = [
            "=~#", "!~#", "==#", "!=#", "||", "&&", "==", "!=", ">=", "<=", "=~", "!~", "..", "->",
        ]
        .into_iter()
        .find(|s| source[at..].starts_with(s));
        if let Some(operator) = operator {
            for _ in 1..operator.len() {
                chars.next();
            }
            tokens.push(Token::Symbol(operator.into()));
        } else if "!?():,.[]<>".contains(ch) {
            tokens.push(Token::Symbol(ch.to_string()));
        } else {
            return Err(format!("unsupported syntax setup token: {ch}"));
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}
