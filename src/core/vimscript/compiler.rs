//! Side-effect-free Vim9 analysis using the same expression grammar as execution.
//! Expression analysis never executes a function body. Explicit imported type
//! dependencies are linked through the host before their types are resolved.
use super::state::{FunctionBody, UserFunction};
use super::types::Type;
use super::{EvalResult, Runtime, ScriptHost, Value, VimError};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub(crate) struct Context {
    pub variables: BTreeMap<Vec<u8>, Type>,
    functions: BTreeMap<Vec<u8>, Arc<UserFunction>>,
    pub immutable: std::collections::BTreeSet<Vec<u8>>,
    pub owner: Option<Arc<super::objects::Class>>,
    dependencies: Arc<std::sync::Mutex<Vec<Arc<UserFunction>>>>,
    inferred_returns: Option<Arc<std::sync::Mutex<Vec<Type>>>>,
}

pub(crate) fn signature(function: &UserFunction) -> Type {
    function
        .signature
        .as_ref()
        .map_or(Type::Any, |s| Type::Func {
            args: s.parameters.clone(),
            optional: function
                .params
                .iter()
                .filter(|p| p.default.is_some())
                .count(),
            rest: s.rest.as_ref().map(|(_, t)| Box::new(t.clone())),
            result: Box::new(s.result.clone()),
        })
}

impl Context {
    pub(crate) fn for_class(class: Arc<super::objects::Class>, functions: &[UserFunction]) -> Self {
        Self {
            owner: Some(class),
            functions: functions
                .iter()
                .map(|f| (f.name.as_bytes().to_vec(), Arc::new(f.clone())))
                .collect(),
            ..Default::default()
        }
    }
    fn function(&self, rt: &Runtime, name: &[u8]) -> Option<Arc<UserFunction>> {
        self.functions
            .get(name)
            .cloned()
            .or_else(|| rt.state.function(name))
    }
    pub(crate) fn depend_on(&self, function: Arc<UserFunction>) {
        if function
            .signature
            .as_ref()
            .is_none_or(|s| !s.generics.is_empty())
        {
            return;
        }
        let mut dependencies = self.dependencies.lock().unwrap_or_else(|p| p.into_inner());
        if !dependencies.iter().any(|f| Arc::ptr_eq(f, &function)) {
            dependencies.push(function);
        }
    }
    pub fn lookup(&self, rt: &Runtime, name: &[u8]) -> EvalResult<Value> {
        super::vim9::validate_namespace(rt, name)?;
        if let Some(function) = self.functions.get(name) {
            return Ok(Value::Funcref(
                super::Funcref::named(function.name.clone()).specialized(function.clone()),
            ));
        }
        if let Some(ty) = self.variables.get(name) {
            return Ok(Value::type_value(ty.clone()));
        }
        if let Some(owner) = self.owner.as_ref().filter(|c| c.name.as_bytes() == name) {
            return Ok(Value::Class(owner.clone()));
        }
        if matches!(name, b"this" | b"super") {
            let owner = self.owner.as_ref().and_then(|c| {
                if name == b"super" {
                    c.base.as_ref()
                } else {
                    Some(c)
                }
            });
            if let Some(owner) = owner {
                return Ok(Value::type_value(Type::Named(owner.type_name())));
            }
        }
        if let Some(path) = rt.state.import(rt.current_sid(), name) {
            if let Some(script) = rt.state.scripts().into_iter().find(|s| s.name == path) {
                return Ok(Value::Module(script.vars));
            }
            // An autoload import is resolved by the runtime at first use.
            return Ok(Value::type_value(Type::Any));
        }
        if let Some(value) = rt
            .state
            .script_vars(rt.current_sid())
            .and_then(|vars| vars.get(name))
        {
            if let Value::Funcref(reference) = &value {
                if let Some(function) = reference
                    .specialization()
                    .or_else(|| rt.state.function(reference.name().as_bytes()))
                {
                    self.depend_on(function);
                }
            }
            return Ok(value);
        }
        if let Some(function) = rt
            .state
            .function(rt.canonical_function_name(name).as_bytes())
        {
            self.depend_on(function.clone());
            return Ok(Value::Funcref(
                super::Funcref::named(function.name.clone()).specialized(function),
            ));
        }
        match name {
            b"true" | b"false" => return Ok(Value::type_value(Type::Bool)),
            b"null" | b"null_object" | b"null_function" => return Ok(Value::type_value(Type::Any)),
            b"null_list" => return Ok(Value::type_value(Type::List(Box::new(Type::Any)))),
            b"null_dict" => return Ok(Value::type_value(Type::Dict(Box::new(Type::Any)))),
            b"null_string" => return Ok(Value::type_value(Type::String)),
            _ => {}
        }
        if name.get(1) == Some(&b':') {
            return Ok(Value::type_value(Type::Any));
        }
        if let Some(owner) = &self.owner {
            if let Some(field) = owner
                .static_fields
                .iter()
                .find(|f| f.name.as_bytes() == name)
            {
                return Ok(Value::type_value(field.ty.clone()));
            }
        }
        Err(VimError::undefined_variable(name).into())
    }
    pub fn member(&self, rt: &Runtime, value: &Value, name: &[u8]) -> EvalResult<Value> {
        let ty = Type::infer(value);
        match value {
            Value::Module(vars) => {
                vars.exported_member(name)?;
                let value = vars
                    .get(name)
                    .ok_or_else(|| VimError::new(1049, "Item not exported"))?;
                if let Value::Funcref(reference) = &value {
                    if let Some(f) = rt.state.function(reference.name().as_bytes()) {
                        self.depend_on(f);
                    }
                }
                return Ok(value);
            }
            Value::Class(class) => {
                let name = String::from_utf8_lossy(name);
                if name.starts_with("new") {
                    if class.abstract_class || class.interface || class.enumeration {
                        return Err(VimError::new(1325, "Cannot instantiate this class").into());
                    }
                    let reference = super::Funcref::constructor(class.clone(), &name);
                    let reference = if let Some(f) = class
                        .methods
                        .get(name.as_ref())
                        .and_then(|n| self.function(rt, n.as_bytes()))
                    {
                        self.depend_on(f.clone());
                        reference.specialized(f)
                    } else if name == "new" {
                        reference
                    } else {
                        return Err(VimError::new(1326, "Constructor not found").into());
                    };
                    return Ok(Value::Funcref(reference));
                }
                if let Some(field) = class.static_fields.iter().find(|f| f.name == name) {
                    return Ok(Value::type_value(field.ty.clone()));
                }
                if let Some(method) = class
                    .static_methods
                    .get(name.as_ref())
                    .and_then(|n| self.function(rt, n.as_bytes()))
                {
                    self.depend_on(method.clone());
                    return Ok(Value::Funcref(
                        super::Funcref::named(method.name.clone()).specialized(method),
                    ));
                }
            }
            _ => {}
        }
        match ty {
            Type::Any => Ok(Value::type_value(Type::Any)),
            Type::Dict(t) => Ok(Value::type_value(*t)),
            Type::Named(class_name) => {
                let class = self
                    .owner
                    .as_ref()
                    .filter(|c| c.type_name() == class_name || c.name == class_name)
                    .cloned()
                    .or_else(|| super::types::class_for_type(rt, &class_name));
                if let Some(class) = class {
                    if let Some(field) = class.fields.iter().find(|f| f.name.as_bytes() == name) {
                        return Ok(Value::type_value(field.ty.clone()));
                    }
                    if let Some(function) = class
                        .method(&String::from_utf8_lossy(name))
                        .and_then(|n| self.function(rt, n.as_bytes()))
                    {
                        self.depend_on(function.clone());
                        return Ok(Value::Funcref(
                            super::Funcref::named(function.name.clone()).specialized(function),
                        ));
                    }
                    return Err(VimError::new(
                        1326,
                        format!("Member not found: {}", String::from_utf8_lossy(name)),
                    )
                    .into());
                }
                Err(VimError::new(1010, format!("Type not recognized: {class_name}")).into())
            }
            _ => Err(VimError::new(1203, format!("Dot cannot be used on {ty}")).into()),
        }
    }
}

pub(crate) fn call(
    rt: &Runtime,
    context: &Context,
    name: &[u8],
    args: &[Value],
) -> EvalResult<Value> {
    if let Ok(value) = context.lookup(rt, name) {
        return invoke(rt, &Type::infer(&value), args);
    }
    let name = String::from_utf8_lossy(name);
    if !super::builtins::is_vim_builtin(&name) {
        return Err(VimError::unknown_function(name.as_bytes()).into());
    }
    Ok(Value::type_value(super::builtins::signatures::check(
        rt, &name, args, true,
    )?))
}

pub(crate) fn invoke(rt: &Runtime, ty: &Type, args: &[Value]) -> EvalResult<Value> {
    let Type::Func {
        args: parameters,
        optional,
        rest,
        result,
    } = ty
    else {
        return if *ty == Type::Any {
            Ok(Value::type_value(Type::Any))
        } else {
            Err(VimError::new(1085, "Not a callable type").into())
        };
    };
    if args.len() < parameters.len().saturating_sub(*optional) {
        return Err(VimError::new(119, "Not enough arguments").into());
    }
    if rest.is_none() && args.len() > parameters.len() {
        return Err(VimError::new(118, "Too many arguments").into());
    }
    for (index, arg) in args.iter().enumerate() {
        let expected = parameters
            .get(index)
            .or_else(|| {
                rest.as_deref()
                    .map(|t| if let Type::List(t) = t { t.as_ref() } else { t })
            })
            .unwrap_or(&Type::Any);
        expected.require_in(&Type::infer(arg), rt)?;
    }
    Ok(Value::type_value(result.as_ref().clone()))
}

pub(crate) fn binary(left: &Value, right: &Value, op: &str) -> EvalResult<Value> {
    let (a, b) = (Type::infer(left), Type::infer(right));
    if op == ".." {
        return Ok(Value::type_value(Type::String));
    }
    if op == "+" && a == b && matches!(a, Type::List(_) | Type::Blob | Type::Tuple(..)) {
        return Ok(Value::type_value(a));
    }
    if !matches!(a, Type::Any | Type::Number | Type::Float)
        || !matches!(b, Type::Any | Type::Number | Type::Float)
    {
        return Err(VimError::new(1036, format!("Invalid operands for {op}: {a} and {b}")).into());
    }
    Ok(Value::type_value(if a == Type::Float || b == Type::Float {
        Type::Float
    } else if a == Type::Any || b == Type::Any {
        Type::Any
    } else {
        Type::Number
    }))
}

fn expression(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    source: &[u8],
    context: &Context,
) -> EvalResult<(Type, usize)> {
    let mut eval = super::expr::Eval::analyze(rt, host, source, 0, context);
    let value = eval.eval1()?;
    Ok((Type::infer(&value), eval.pos))
}

pub(crate) fn analyze_lambda(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    function: &mut UserFunction,
    mut context: Context,
    infer_result: bool,
) -> EvalResult<()> {
    let returns = Arc::new(std::sync::Mutex::new(vec![]));
    context.inferred_returns = infer_result.then(|| returns.clone());
    check_with_context(rt, host, function, context)?;
    if infer_result {
        let types = std::mem::take(&mut *returns.lock().unwrap_or_else(|p| p.into_inner()));
        let result = types.first().cloned().unwrap_or(Type::Void);
        let result = if types.iter().all(|ty| ty == &result) {
            result
        } else {
            Type::Any
        };
        if !matches!(result, Type::Any | Type::Void) {
            if let FunctionBody::Lines(lines) = &function.body {
                if !returns_on_all_paths(&reachable_lines(rt, host, statements(lines.to_vec()))) {
                    return Err(VimError::new(1027, "Missing return statement").into());
                }
            }
        }
        function.signature.as_mut().unwrap().result = result;
    }
    Ok(())
}

/// Ex payloads own their bar rules; expression statements use quoted strings
/// and balanced containers. Keep physical locations on the resulting commands.
fn statements(lines: Vec<super::state::SourceLine>) -> Vec<super::state::SourceLine> {
    fn expression_command(source: &[u8]) -> bool {
        let source = source.trim_ascii_start();
        let end = source
            .iter()
            .position(|b| !b.is_ascii_alphabetic())
            .unwrap_or(source.len());
        let word = &source[..end];
        matches!(
            word,
            b"var"
                | b"final"
                | b"const"
                | b"return"
                | b"if"
                | b"elseif"
                | b"else"
                | b"endif"
                | b"while"
                | b"endwhile"
                | b"for"
                | b"endfor"
                | b"try"
                | b"finally"
                | b"endtry"
                | b"break"
                | b"continue"
                | b"echo"
                | b"echomsg"
                | b"echon"
                | b"echoerr"
                | b"throw"
                | b"execute"
                | b"eval"
                | b"call"
                | b"defer"
        ) || source.get(end) == Some(&b'(')
            || assignment_operator(source).is_some()
    }
    let mut result = vec![];
    let mut heredoc: Option<Vec<u8>> = None;
    for line in lines {
        let source = line.text.as_bytes();
        if let Some(marker) = &heredoc {
            if source.trim_ascii() == marker {
                heredoc = None;
            }
            result.push(line);
            continue;
        }
        if source.windows(3).any(|s| s == b"=<<") {
            heredoc = source
                .split(u8::is_ascii_whitespace)
                .filter(|s| !s.is_empty())
                .last()
                .map(|s| s.to_vec());
            result.push(line);
            continue;
        }
        let mut start = 0;
        let mut at = 0;
        let mut depth = 0usize;
        let mut quote = None;
        let mut split = expression_command(source);
        while at < source.len() && split {
            let byte = source[at];
            if let Some(q) = quote {
                if byte == b'\\' && q == b'"' {
                    at += 2;
                    continue;
                }
                if byte == q {
                    if q == b'\'' && source.get(at + 1) == Some(&q) {
                        at += 2;
                        continue;
                    }
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if matches!(byte, b'(' | b'[' | b'{') {
                depth += 1;
            } else if matches!(byte, b')' | b']' | b'}') {
                depth = depth.saturating_sub(1);
            } else if byte == b'#' && (at == 0 || source[at - 1].is_ascii_whitespace()) {
                break;
            } else if byte == b'|' && depth == 0 {
                if source.get(at + 1) == Some(&b'|') {
                    at += 2;
                    continue;
                }
                result.push(super::state::SourceLine {
                    text: source[start..at].into(),
                    lnum: line.lnum,
                });
                start = at + 1;
                split = expression_command(&source[start..]);
            }
            at += 1;
        }
        result.push(super::state::SourceLine {
            text: source[start..].into(),
            lnum: line.lnum,
        });
    }
    result
}

fn reachable_lines(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    lines: Vec<super::state::SourceLine>,
) -> Vec<super::state::SourceLine> {
    // Vim omits branches with a compile-time constant condition. Unknown
    // conditions retain both arms for validation, without executing either.
    let mut branches: Vec<(bool, bool, bool)> = vec![];
    let mut result = vec![];
    let mut heredoc: Option<(String, bool)> = None;
    for line in lines {
        if let Some((marker, active)) = &heredoc {
            let ended = line.text.to_str_lossy().trim() == marker;
            if *active {
                result.push(line);
            }
            if ended {
                heredoc = None;
            }
            continue;
        }
        let text = line.text.to_str_lossy().into_owned();
        let text = super::source::vim9_comment_prefix(&text).trim();
        let (command, argument) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let active = branches.last().is_none_or(|b| b.2);
        let constant = if matches!(command, "if" | "elseif") && active {
            let mut eval = super::expr::Eval::constant(rt, host, argument.as_bytes());
            eval.eval1()
                .ok()
                .filter(|_| eval.at_end())
                .and_then(|value| super::vim9::truth(&value).ok())
        } else {
            None
        };
        if let Some((_, body)) = text.split_once("=<<") {
            if let Some(marker) = body.split_whitespace().last() {
                heredoc = Some((marker.to_string(), active));
            }
        }
        let line = if let Some(value) = constant {
            super::state::SourceLine {
                text: format!("{command} {}", if value { "true" } else { "false" }).into(),
                lnum: line.lnum,
            }
        } else {
            line
        };
        match command {
            "if" => {
                branches.push((
                    active,
                    constant == Some(true),
                    active && constant != Some(false),
                ));
                if active {
                    result.push(line);
                }
            }
            "elseif" => {
                if let Some(branch) = branches.last_mut() {
                    branch.2 = branch.0 && !branch.1 && constant != Some(false);
                    branch.1 |= constant == Some(true);
                    if branch.0 {
                        result.push(line);
                    }
                }
            }
            "else" => {
                if let Some(branch) = branches.last_mut() {
                    branch.2 = branch.0 && !branch.1;
                    branch.1 = true;
                    if branch.0 {
                        result.push(line);
                    }
                }
            }
            "endif" => {
                if branches.pop().is_some_and(|b| b.0) {
                    result.push(line);
                }
            }
            _ if active => result.push(line),
            _ => {}
        }
    }
    result
}

/// Block pairing is syntax, including in compile-time discarded branches.
/// Bodies of nested functions are checked in their own compilation context.
fn structure(lines: &[super::state::SourceLine]) -> EvalResult<()> {
    let mut blocks: Vec<(&str, bool)> = vec![];
    let mut nested: Vec<bool> = vec![];
    let mut heredoc: Option<String> = None;
    for line in lines {
        let text = line.text.to_str_lossy();
        let text = super::source::vim9_comment_prefix(&text).trim();
        if let Some(marker) = &heredoc {
            if text == marker {
                heredoc = None;
            }
            continue;
        }
        if let Some((_, rest)) = text.split_once("=<<") {
            heredoc = rest.split_whitespace().last().map(str::to_owned);
            continue;
        }
        let command = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('!');
        if command == "def"
            || super::excmds::lookup(command) == Some("function") && text.contains('(')
        {
            nested.push(command == "def");
            continue;
        }
        if nested.last().is_some_and(|typed| {
            if *typed {
                command == "enddef"
            } else {
                super::excmds::lookup(command) == Some("endfunction")
            }
        }) {
            nested.pop();
            continue;
        }
        if !nested.is_empty() {
            continue;
        }
        match command {
            "if" => blocks.push(("if", false)),
            "try" => blocks.push(("try", false)),
            "for" | "while" => {
                if blocks
                    .iter()
                    .filter(|(kind, _)| matches!(*kind, "for" | "while"))
                    .count()
                    >= 10
                {
                    return Err(VimError::new(1306, "Loop nesting too deep").into());
                }
                blocks.push((if command == "for" { "for" } else { "while" }, false));
            }
            "{" => blocks.push(("{", false)),
            "else" | "elseif" => {
                let Some(("if", seen_else)) = blocks.last_mut() else {
                    return Err(VimError::new(581, "Conditional branch without if").into());
                };
                if *seen_else {
                    return Err(VimError::new(583, "Branch after else").into());
                }
                *seen_else = command == "else";
            }
            "catch" | "finally" => {
                let Some(("try", seen_finally)) = blocks.last_mut() else {
                    return Err(VimError::new(603, "Exception branch without try").into());
                };
                if *seen_finally {
                    return Err(VimError::new(607, "Branch after finally").into());
                }
                *seen_finally = command == "finally";
            }
            "endif" | "endfor" | "endwhile" | "endtry" | "}" => {
                let expected = if command == "}" { "{" } else { &command[3..] };
                if blocks.pop().is_none_or(|(kind, _)| kind != expected) {
                    return Err(VimError::new(171, format!("Unexpected {command}")).into());
                }
            }
            "break" | "continue"
                if !blocks
                    .iter()
                    .any(|(kind, _)| matches!(*kind, "for" | "while")) =>
            {
                return Err(VimError::new(587, format!("{command} without a loop")).into())
            }
            _ => {}
        }
    }
    if let Some((kind, _)) = blocks.last() {
        return Err(VimError::new(171, format!("Missing end for {kind}")).into());
    }
    Ok(())
}

/// Validate a body without executing its statements or sampling callbacks.
/// Explicit module dependencies may run their initializers through the host
/// while their exported types are linked.
pub(crate) fn check_with_context(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    function: &UserFunction,
    context: Context,
) -> EvalResult<()> {
    if rt.compile_depth >= 50 {
        return Err(VimError::new(132, "Function definition nesting too deep").into());
    }
    rt.compile_depth += 1;
    let result = check_inner(rt, host, function, context);
    rt.compile_depth -= 1;
    result
}

fn check_inner(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    function: &UserFunction,
    mut context: Context,
) -> EvalResult<()> {
    let Some(sig) = &function.signature else {
        return Ok(());
    };
    context.owner = sig.owner.clone();
    context.variables.extend(sig.lexical_types.clone());
    for ty in &sig.parameters {
        super::types::validate_storage(ty, rt)?;
    }
    if let Some((_, ty)) = &sig.rest {
        super::types::validate_storage(ty, rt)?;
    }
    super::types::validate(&sig.result, rt)?;
    let mut outer = function.outer.clone();
    while let Some(scope) = outer {
        for (name, value) in scope.locals.snapshot() {
            context
                .variables
                .entry(name.as_bytes().to_vec())
                .or_insert_with(|| Type::infer(&value));
        }
        outer = scope.outer.clone();
    }
    for (param, ty) in function.params.iter().zip(&sig.parameters) {
        if param.name.as_bytes() != b"_"
            && (context.variables.contains_key(param.name.as_bytes())
                || rt
                    .state
                    .script_vars(function.sid)
                    .is_some_and(|vars| vars.contains(param.name.as_bytes())))
        {
            return Err(VimError::new(1168, "Argument already declared in the script").into());
        }
        if let Some(default) = &param.default {
            ty.require_in(&expression(rt, host, default.as_bytes(), &context)?.0, rt)?;
        }
        context
            .variables
            .insert(param.name.as_bytes().to_vec(), ty.clone());
    }
    if let Some((name, ty)) = &sig.rest {
        context
            .variables
            .insert(name.as_bytes().to_vec(), ty.clone());
    }
    let lines = match &function.body {
        FunctionBody::Lambda(expr) => vec![super::state::SourceLine {
            text: format!("return {}", expr.to_str_lossy()).into(),
            lnum: 1,
        }],
        FunctionBody::Lines(lines) => lines.to_vec(),
    };
    let lines = statements(lines);
    structure(&lines)?;
    let lines = reachable_lines(rt, host, lines);
    let mut scopes: Vec<Context> = vec![];
    let mut at = 0;
    while at < lines.len() {
        rt.charge(1)?;
        let source = lines[at].text.to_str_lossy();
        let source = super::source::vim9_comment_prefix(&source).trim();
        at += 1;
        if source.is_empty() {
            continue;
        }
        let (command, argument) = source
            .split_once(char::is_whitespace)
            .unwrap_or((source, ""));
        match command {
            "var" | "const" | "final" => {
                let src = argument.as_bytes();
                let mut end = 0;
                if src.starts_with(b"[") {
                    let close = src
                        .iter()
                        .position(|b| *b == b']')
                        .ok_or_else(|| VimError::new(111, "Missing ]"))?;
                    let value_at = super::expr::skip_white(src, close + 1);
                    if src.get(value_at) != Some(&b'=') {
                        return Err(VimError::new(1022, "Initialization required").into());
                    }
                    let ty = expression(rt, host, &src[value_at + 1..], &context)?.0;
                    bind_pattern(
                        rt,
                        host,
                        &mut context,
                        &src[1..close],
                        &ty,
                        command != "var",
                    )?;
                    continue;
                }
                let name = super::vim9::binding_name(src, &mut end, command.as_bytes(), true)?;
                if context.variables.contains_key(&name) {
                    return Err(VimError::new(1017, "Variable already declared").into());
                }
                let ty = if src.get(end) == Some(&b':') {
                    end += 1;
                    Some(super::types::parse_active(src, &mut end, rt, host)?)
                } else {
                    None
                };
                if let Some(ty) = &ty {
                    super::types::validate_storage(ty, rt)?;
                }
                end = super::expr::skip_white(src, end);
                if src.get(end) != Some(&b'=') && (ty.is_none() || command != "var") {
                    return Err(VimError::new(1022, "Type or initialization required").into());
                }
                let actual = if src.get(end..).is_some_and(|s| s.starts_with(b"=<<")) {
                    let marker = argument.split_whitespace().last().unwrap_or("");
                    while at < lines.len() && lines[at].text.to_str_lossy().trim() != marker {
                        at += 1;
                    }
                    if at == lines.len() {
                        return Err(VimError::new(990, "Missing heredoc end marker").into());
                    }
                    at += 1;
                    Type::List(Box::new(Type::String))
                } else if src.get(end) == Some(&b'=') {
                    expression(rt, host, &src[end + 1..], &context)?.0
                } else {
                    ty.clone().unwrap_or(Type::Any)
                };
                actual.require_value()?;
                if actual == Type::Void {
                    return Err(VimError::new(1031, "Cannot use void value").into());
                }
                if let Some(ty) = &ty {
                    ty.require_in(&actual, rt)?;
                }
                context.variables.insert(name.clone(), ty.unwrap_or(actual));
                if command != "var" {
                    context.immutable.insert(name);
                }
            }
            "if" | "elseif" | "while" => {
                if command == "elseif" {
                    if let Some(scope) = scopes.last() {
                        context = scope.clone();
                    }
                }
                let ty = expression(rt, host, argument.as_bytes(), &context)?.0;
                Type::Bool.require(&ty)?;
                if command != "elseif" {
                    scopes.push(context.clone());
                }
            }
            "else" | "catch" | "finally" => {
                if let Some(scope) = scopes.last() {
                    context = scope.clone();
                }
            }
            "endif" | "endwhile" | "endfor" | "endtry" => {
                if let Some(scope) = scopes.pop() {
                    context = scope;
                }
            }
            "try" | "{" => scopes.push(context.clone()),
            "}" => {
                if let Some(scope) = scopes.pop() {
                    context = scope;
                }
            }
            "for" => {
                let Some((name, value)) = argument.split_once(" in ") else {
                    return Err(VimError::new(690, "Missing in after for").into());
                };
                let ty = expression(rt, host, value.as_bytes(), &context)?.0;
                let item = match ty {
                    Type::List(t) => *t,
                    Type::String => Type::String,
                    Type::Blob => Type::Number,
                    Type::Tuple(..) | Type::Any => Type::Any,
                    _ => return Err(VimError::new(1098, "Iterable required").into()),
                };
                scopes.push(context.clone());
                let name = name.trim();
                if let Some(pattern) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
                    bind_pattern(rt, host, &mut context, pattern.as_bytes(), &item, true)?;
                } else {
                    super::vim9::validate_binding(
                        name.strip_prefix("g:").unwrap_or(name).as_bytes(),
                    )?;
                    if name.as_bytes().get(1) != Some(&b':')
                        && context.lookup(rt, name.as_bytes()).is_ok()
                    {
                        return Err(VimError::new(
                            1254,
                            "Cannot use declared variable in for loop",
                        )
                        .into());
                    }
                    context.variables.insert(name.as_bytes().to_vec(), item);
                    if name.as_bytes().get(1) != Some(&b':') {
                        context.immutable.insert(name.as_bytes().to_vec());
                    }
                }
            }
            "return" => {
                let ty = if argument.is_empty() {
                    Type::Void
                } else {
                    expression(rt, host, argument.as_bytes(), &context)?.0
                };
                if let Some(returns) = &context.inferred_returns {
                    returns.lock().unwrap_or_else(|p| p.into_inner()).push(ty);
                } else {
                    sig.result.require_in(&ty, rt)?;
                }
            }
            "function" | "function!" | "func" | "func!" => {
                let name = argument.split('(').next().unwrap_or("").as_bytes().to_vec();
                at = super::source::definition_end(&lines, at - 1, "endfunction")?;
                context.variables.insert(
                    name,
                    Type::Func {
                        args: vec![],
                        optional: 0,
                        rest: Some(Box::new(Type::Any)),
                        result: Box::new(Type::Any),
                    },
                );
                at += 1;
            }
            "def" => {
                let start = at;
                at = super::source::definition_end(&lines, at - 1, "enddef")?;
                let mut nested = super::vim9::parse_function(
                    rt,
                    host,
                    argument.as_bytes(),
                    lines[start..at].to_vec(),
                    lines[start - 1].lnum,
                )?;
                nested.signature.as_mut().unwrap().lexical_types = context.variables.clone();
                let name = argument
                    .split(['(', '<'])
                    .next()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec();
                let generic = !nested.signature.as_ref().unwrap().generics.is_empty();
                context.functions.insert(name, Arc::new(nested.clone()));
                if !generic {
                    let mut nested_context = context.clone();
                    nested_context.inferred_returns = None;
                    check_with_context(rt, host, &nested, nested_context)?;
                }
                at += 1;
            }
            "echo" | "echomsg" | "echon" | "echoerr" | "execute" | "throw" | "call" | "defer"
            | "eval" => {
                let mut rest = argument.as_bytes();
                while !rest.is_empty() {
                    let (_, end) = expression(rt, host, rest, &context)?;
                    if end == 0 {
                        break;
                    }
                    rest = &rest[end..];
                }
            }
            "let" => return Err(VimError::new(1126, "Cannot use :let in Vim9 script").into()),
            "break" | "continue" | "finish" => {}
            _ => {
                if let Some((position, op)) = assignment_operator(source.as_bytes()) {
                    let target = source[..position].trim();
                    if context.immutable.contains(target.as_bytes()) {
                        return Err(VimError::new(1018, "Cannot assign to a constant").into());
                    }
                    let expected = expression(rt, host, target.as_bytes(), &context)?.0;
                    let actual = expression(
                        rt,
                        host,
                        &source.as_bytes()[position + op.len()..],
                        &context,
                    )?
                    .0;
                    if op == "=" {
                        expected.require_in(&actual, rt)?;
                    } else {
                        let result = binary(
                            &Value::type_value(expected.clone()),
                            &Value::type_value(actual),
                            &op[..op.len() - 1],
                        )?;
                        expected.require_in(&Type::infer(&result), rt)?;
                    }
                } else if let Some(target) = source
                    .strip_prefix("++")
                    .or_else(|| source.strip_prefix("--"))
                {
                    if context.immutable.contains(target.as_bytes()) {
                        return Err(VimError::new(1018, "Cannot assign to a constant").into());
                    }
                    Type::Number.require(&expression(rt, host, target.as_bytes(), &context)?.0)?;
                } else if source.contains('(')
                    && !matches!(command, "normal" | "normal!" | "command" | "autocmd")
                {
                    expression(rt, host, source.as_bytes(), &context)?;
                } else if !matches!(command, "legacy" | "vim9cmd")
                    && super::excmds::lookup(command.trim_end_matches('!')).is_none()
                    && rt.state.user_command(command).is_none()
                {
                    return Err(
                        VimError::new(492, format!("Not an editor command: {source}")).into(),
                    );
                }
            }
        }
    }
    if !matches!(sig.result, Type::Void | Type::Any) && !returns_on_all_paths(&lines) {
        return Err(VimError::new(1027, "Missing return statement").into());
    }
    Ok(())
}

fn bind_pattern(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    context: &mut Context,
    pattern: &[u8],
    ty: &Type,
    immutable: bool,
) -> EvalResult<()> {
    let mut at = 0;
    let mut index = 0;
    let mut rest = false;
    while at < pattern.len() {
        at = super::expr::skip_white(pattern, at);
        let start = at;
        while pattern
            .get(at)
            .is_some_and(|b| super::expr::is_name_char(*b))
        {
            at += 1;
        }
        if start == at {
            return Err(VimError::new(1017, "Variable name required").into());
        }
        let name = pattern[start..at].to_vec();
        if context.variables.contains_key(&name) {
            return Err(VimError::new(1017, "Variable already declared").into());
        }
        let declared = if pattern.get(at) == Some(&b':') {
            at += 1;
            Some(super::types::parse_active(pattern, &mut at, rt, host)?)
        } else {
            None
        };
        let item = match ty {
            Type::List(t) => {
                if rest {
                    ty.clone()
                } else {
                    t.as_ref().clone()
                }
            }
            Type::Tuple(items, tail) => {
                if rest {
                    let common = items
                        .get(index)
                        .cloned()
                        .or_else(|| tail.as_deref().cloned())
                        .unwrap_or(Type::Any);
                    Type::List(Box::new(
                        if items.iter().skip(index).all(|t| *t == common) {
                            common
                        } else {
                            Type::Any
                        },
                    ))
                } else {
                    items
                        .get(index)
                        .cloned()
                        .or_else(|| tail.as_deref().cloned())
                        .ok_or_else(|| VimError::new(688, "More targets than tuple items"))?
                }
            }
            Type::Any => Type::Any,
            _ => return Err(VimError::new(714, "List or Tuple required").into()),
        };
        if let Some(declared) = &declared {
            declared.require_in(&item, rt)?;
        }
        context
            .variables
            .insert(name.clone(), declared.unwrap_or(item));
        if immutable {
            context.immutable.insert(name);
        }
        index += 1;
        at = super::expr::skip_white(pattern, at);
        if at == pattern.len() {
            break;
        }
        match pattern[at] {
            b',' if !rest => {}
            b';' if !rest => rest = true,
            _ => return Err(VimError::new(475, "Invalid destructuring pattern").into()),
        }
        at += 1;
    }
    Ok(())
}

/// Locate statement assignment without confusing quoted text, subscripts,
/// comparisons or lambda arrows with an lvalue's assignment operator.
fn assignment_operator(source: &[u8]) -> Option<(usize, &'static str)> {
    let mut at = 0;
    let mut depth = 0usize;
    let mut quote = None;
    while at < source.len() {
        let byte = source[at];
        if let Some(q) = quote {
            if byte == b'\\' && q == b'"' {
                at += 2;
                continue;
            }
            if byte == q {
                if q == b'\'' && source.get(at + 1) == Some(&q) {
                    at += 2;
                    continue;
                }
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if matches!(byte, b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(byte, b')' | b']' | b'}') {
            depth = depth.saturating_sub(1);
        } else if depth == 0 {
            for op in ["..=", "+=", "-=", "*=", "/=", "%=", "="] {
                if source[at..].starts_with(op.as_bytes())
                    && !(op == "="
                        && (source
                            .get(at + 1)
                            .is_some_and(|b| matches!(b, b'=' | b'>' | b'~'))
                            || at > 0 && matches!(source[at - 1], b'!' | b'<' | b'>' | b'=')))
                {
                    return Some((at, op));
                }
            }
        }
        at += 1;
    }
    None
}

/// Track normal fallthrough independently of expression analysis. A return in
/// only one conditional arm cannot satisfy a function's declared result type.
fn returns_on_all_paths(lines: &[super::state::SourceLine]) -> bool {
    fn block(lines: &[super::state::SourceLine], at: &mut usize, depth: usize) -> bool {
        if depth > 50 {
            return false;
        }
        let mut terminated = false;
        while *at < lines.len() {
            if let Some((marker, trim)) = super::source::heredoc_marker(lines[*at].text.as_bytes())
            {
                *at += 1;
                while *at < lines.len() {
                    let bytes = lines[*at].text.as_bytes();
                    *at += 1;
                    if (if trim {
                        bytes.trim_ascii_start()
                    } else {
                        bytes
                    }) == marker
                    {
                        break;
                    }
                }
                continue;
            }
            let text = lines[*at].text.to_str_lossy();
            let text = super::source::vim9_comment_prefix(&text).trim();
            let (command, argument) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
            if matches!(
                command,
                "elseif"
                    | "else"
                    | "endif"
                    | "endwhile"
                    | "endfor"
                    | "catch"
                    | "finally"
                    | "endtry"
                    | "}"
            ) {
                break;
            }
            *at += 1;
            match command {
                "return" | "throw" => terminated = true,
                "def" | "function" | "function!" | "func" | "func!" => {
                    *at = match super::source::definition_end(
                        lines,
                        *at - 1,
                        if command == "def" {
                            "enddef"
                        } else {
                            "endfunction"
                        },
                    ) {
                        Ok(end) => end + 1,
                        Err(_) => return false,
                    };
                }
                "if" => {
                    let mut condition = argument.trim().to_string();
                    let mut all = true;
                    let mut exhaustive = false;
                    loop {
                        let arm = block(lines, at, depth + 1);
                        if !matches!(condition.as_str(), "false" | "0") && !exhaustive {
                            all &= arm;
                        }
                        exhaustive |= matches!(condition.as_str(), "true" | "1");
                        if *at >= lines.len() {
                            return false;
                        }
                        let next = lines[*at].text.to_str_lossy();
                        let next = super::source::vim9_comment_prefix(&next).trim();
                        *at += 1;
                        if let Some(expr) = next.strip_prefix("elseif ") {
                            condition = expr.trim().into();
                        } else if next == "else" {
                            let arm = block(lines, at, depth + 1);
                            if !exhaustive {
                                all &= arm;
                            }
                            exhaustive = true;
                            *at += 1;
                            break;
                        } else {
                            break;
                        }
                    }
                    terminated |= all && exhaustive;
                }
                "while" | "for" => {
                    block(lines, at, depth + 1);
                    *at += 1;
                    // Vim still requires a return after a loop, including
                    // `while true` with an unconditional return in its body.
                }
                "try" => {
                    let mut all = block(lines, at, depth + 1);
                    let mut finally = false;
                    while *at < lines.len() {
                        let text = lines[*at].text.to_str_lossy();
                        let text = text.trim();
                        *at += 1;
                        if text.starts_with("catch") {
                            all &= block(lines, at, depth + 1);
                        } else if text == "finally" {
                            finally = block(lines, at, depth + 1);
                        } else {
                            break;
                        }
                    }
                    terminated |= all || finally;
                }
                "{" => {
                    terminated |= block(lines, at, depth + 1);
                    *at += 1;
                }
                _ => {}
            }
        }
        terminated
    }
    block(lines, &mut 0, 0)
}

pub(crate) fn ensure(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    function: &UserFunction,
) -> EvalResult<()> {
    let Some(signature) = &function.signature else {
        return Ok(());
    };
    if signature.owner.as_ref().is_some_and(|c| {
        c.interface
            || c.abstract_methods.contains(
                function
                    .name
                    .to_str_lossy()
                    .rsplit('.')
                    .next()
                    .unwrap_or(""),
            )
    }) {
        return Ok(());
    }
    if signature.checked.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    if rt.compile_depth >= 50 {
        return Err(VimError::new(132, "Compilation nesting too deep").into());
    }
    let root = rt.compile_seen.is_empty();
    let body_id = match &function.body {
        FunctionBody::Lines(lines) => lines.as_ptr() as usize,
        FunctionBody::Lambda(body) => body.as_bytes().as_ptr() as usize,
    };
    let key = format!(
        "{body_id}:{}:{:?}:{:?}",
        function.name.to_str_lossy(),
        signature.parameters,
        signature.bindings
    );
    if !rt.compile_seen.insert(key) {
        return Ok(());
    }
    rt.compile_depth += 1;
    let context = Context::default();
    let dependencies = context.dependencies.clone();
    let result = rt
        .with_context(function.outer.clone(), function.sid, |rt| {
            let frame = rt.current_frame_mut().unwrap();
            frame.vim9 = true;
            frame.function = Some(Arc::new(function.clone()));
            check_with_context(rt, host, function, context)
        })
        .and_then(|()| {
            let dependencies = dependencies
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            for dependency in dependencies {
                ensure(rt, host, &dependency)?;
            }
            Ok(())
        });
    rt.compile_depth -= 1;
    if result.is_ok() {
        rt.compile_pending.push(signature.checked.clone());
    }
    if root {
        // Recursive groups become compiled together. A failed dependency cannot
        // leave a partially validated function eligible to run side effects.
        if result.is_ok() {
            for checked in &rt.compile_pending {
                checked.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        rt.compile_pending.clear();
        rt.compile_seen.clear();
    }
    result
}
