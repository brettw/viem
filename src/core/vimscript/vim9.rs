//! Vim9 command syntax using the shared expression evaluator and runtime.
use super::exec::{CommandOutcome, FrameKind, Next, Runtime, ScriptHost};
use super::expr::{skip_white, Eval};
use super::state::{FunctionBody, FunctionFlags, Param, UserFunction};
use super::types::{self, Type};
use super::{EvalResult, VStr, Value, VimError};

#[derive(Clone, Debug)]
pub struct Signature {
    pub generics: Vec<String>,
    pub bindings: std::collections::BTreeMap<String, Type>,
    pub lexical_types: std::collections::BTreeMap<Vec<u8>, Type>,
    pub checked: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub parameters: Vec<Type>,
    pub rest: Option<(VStr, Type)>,
    pub result: Type,
    pub owner: Option<std::sync::Arc<super::objects::Class>>,
}

fn outcome(result: EvalResult<()>) -> CommandOutcome {
    CommandOutcome {
        next: Next::NextLine,
        result,
    }
}

pub(crate) fn binding_name(
    src: &[u8],
    at: &mut usize,
    kind: &[u8],
    in_function: bool,
) -> Result<Vec<u8>, VimError> {
    let start = *at;
    let scoped = src.get(*at + 1) == Some(&b':')
        && src.get(*at + 2).is_some_and(|b| !b.is_ascii_whitespace())
        && src
            .get(*at)
            .is_some_and(|b| matches!(b, b'g' | b's' | b'b' | b'w' | b't' | b'l' | b'a' | b'v'));
    if scoped {
        *at += 2;
    }
    let key_start = *at;
    while src.get(*at).is_some_and(|b| super::expr::is_name_char(*b)) {
        *at += 1;
    }
    let key = &src[key_start..*at];
    validate_binding(key)?;
    if scoped {
        match src[start] {
            b's' if !in_function => {
                return Err(VimError::new(1268, "Cannot use s: in Vim9 script"))
            }
            b'g' | b'b' | b'w' | b't' if kind != b"var" => {}
            _ => {
                return Err(VimError::new(
                    1016,
                    "Cannot declare a variable with this namespace",
                ))
            }
        }
    }
    Ok(src[start..*at].to_vec())
}

pub(crate) fn validate_binding(name: &[u8]) -> Result<(), VimError> {
    if !name.first().is_some_and(|b| super::expr::is_name_start(*b))
        || !name.iter().all(|b| super::expr::is_name_char(*b))
    {
        return Err(VimError::new(1017, "Invalid variable name"));
    }
    if matches!(
        name,
        b"true"
            | b"false"
            | b"this"
            | b"super"
            | b"null"
            | b"null_blob"
            | b"null_dict"
            | b"null_function"
            | b"null_list"
            | b"null_partial"
            | b"null_string"
            | b"null_object"
            | b"null_class"
            | b"null_job"
            | b"null_channel"
            | b"null_tuple"
    ) {
        return Err(VimError::new(
            1034,
            format!(
                "Cannot use reserved name: {}",
                String::from_utf8_lossy(name)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_namespace(rt: &Runtime, name: &[u8]) -> Result<(), VimError> {
    if rt.vim9() {
        if name.starts_with(b"l:") || name.starts_with(b"a:") {
            return Err(VimError::new(
                1075,
                format!("Namespace not supported: {}", String::from_utf8_lossy(name)),
            ));
        }
        if name.starts_with(b"s:") && rt.state.script_is_vim9(rt.current_sid()) {
            return Err(VimError::new(1268, "Cannot use s: in Vim9 script"));
        }
    }
    Ok(())
}

pub(crate) fn truth(value: &Value) -> Result<bool, VimError> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::Number(n @ (0 | 1)) => Ok(*n != 0),
        _ => Err(VimError::new(
            1023,
            format!("Using a {} as a Bool", value.type_name()),
        )),
    }
}

pub(crate) fn string_index(
    text: &VStr,
    first: Option<&Value>,
    second: Option<&Value>,
    range: bool,
) -> EvalResult<Value> {
    // Vim9 indexes characters together with their composing marks. Invalid
    // UTF-8 bytes retain one index each, without replacement decoding.
    let bytes = text.as_bytes();
    let mut offsets = vec![];
    let mut at = 0;
    while at < bytes.len() {
        let width = match bytes[at] {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let character = bytes
            .get(at..at + width)
            .and_then(|s| std::str::from_utf8(s).ok());
        static MARKS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let composing = character.is_some_and(|s| {
            MARKS
                .get_or_init(|| regex::Regex::new(r"^\p{M}$").unwrap())
                .is_match(s)
        });
        if offsets.is_empty() || !composing {
            offsets.push(at);
        }
        at += if character.is_some() { width } else { 1 };
    }
    let len = offsets.len() as i64;
    offsets.push(bytes.len());
    let normalize = |n: i64| if n < 0 { len + n } else { n };
    let from = normalize(first.map(Value::to_number).transpose()?.unwrap_or(0));
    let through = if range {
        normalize(second.map(Value::to_number).transpose()?.unwrap_or(-1))
    } else {
        from
    };
    let from = from.max(0);
    let through = through.min(len - 1);
    if from >= len || through < from {
        return Ok(Value::str(""));
    }
    Ok(Value::String(
        bytes[offsets[from as usize]..offsets[through as usize + 1]].into(),
    ))
}

pub(crate) fn execute(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    line: &VStr,
    col: usize,
    executing: bool,
) -> Option<CommandOutcome> {
    let normalized;
    let line = if rt.vim9() {
        normalized =
            VStr::from(&line.as_bytes()[..super::source::vim9_comment_end(line.as_bytes())]);
        &normalized
    } else {
        line
    };
    let raw = line.as_bytes();
    if col >= raw.len() {
        return if rt.vim9() {
            Some(outcome(Ok(())))
        } else {
            None
        };
    }
    let mut at = skip_white(raw, col);
    while raw.get(at) == Some(&b':') {
        at = skip_white(raw, at + 1);
    }
    let text = &raw[at..];
    let word_end = text
        .iter()
        .position(|b| b.is_ascii_whitespace() || *b == b'!')
        .unwrap_or(text.len());
    let word = &text[..word_end];
    let word = if word.len() >= 4 && b"defcompile".starts_with(word) {
        b"defcompile".as_slice()
    } else {
        word
    };
    let arg = skip_white(raw, at + word_end);
    if matches!(
        word,
        b"vim9script"
            | b"def"
            | b"defcompile"
            | b"import"
            | b"export"
            | b"type"
            | b"class"
            | b"interface"
            | b"abstract"
            | b"enum"
            | b"defer"
    ) || rt.vim9() && matches!(word, b"var" | b"final" | b"const")
    {
        rt.current_command = String::from_utf8_lossy(word).into_owned();
    }
    if word == b"vim9script" {
        return Some(outcome(if !executing {
            Ok(())
        } else {
            start_script(rt, &raw[arg..])
        }));
    }
    if word.len() >= 4 && b"vim9cmd".starts_with(word)
        || word.len() >= 3 && b"legacy".starts_with(word)
    {
        let mode = b"vim9cmd".starts_with(word);
        let previous = rt.vim9();
        if arg == raw.len() {
            return Some(outcome(Err(VimError::new(
                if mode { 1164 } else { 1234 },
                "Command required",
            )
            .into())));
        }
        let index = rt.frames.len().checked_sub(1)?;
        rt.frames[index].vim9 = mode;
        let result = super::commands::execute_command(rt, host, line, arg, executing);
        if let Some(frame) = rt.frames.get_mut(index) {
            frame.vim9 = previous;
        }
        return Some(result);
    }
    if word == b"defer" {
        return Some(outcome(if executing {
            (|| {
                let mut eval = Eval::new(rt, host, raw, arg, true);
                let callee = eval
                    .parse_callee()?
                    .ok_or_else(|| VimError::new(1298, "Function call required after defer"))?;
                if skip_white(raw, eval.pos) != raw.len() {
                    return Err(VimError::trailing(&raw[eval.pos..]).into());
                }
                let frame = rt
                    .frames
                    .iter_mut()
                    .rev()
                    .find(|f| matches!(f.kind, FrameKind::Function))
                    .ok_or_else(|| VimError::new(1300, "defer outside a function"))?;
                if frame.deferred.len() >= 10000 {
                    return Err(VimError::new(132, "Too many deferred calls").into());
                }
                frame.deferred.push(callee);
                Ok(())
            })()
        } else {
            Ok(())
        }));
    }
    if word == b"def" {
        return Some(define(rt, host, raw, arg, executing));
    }
    if word == b"import" {
        return Some(outcome(if executing {
            import(rt, host, raw, arg)
        } else {
            Ok(())
        }));
    }
    if !rt.vim9() {
        return None;
    }
    if text.is_empty() || text.starts_with(b"#") {
        return Some(outcome(Ok(())));
    }
    if text.starts_with(b"++") || text.starts_with(b"--") {
        if !executing {
            return Some(outcome(Ok(())));
        }
        let mut assignment = text[2..].to_vec();
        assignment.extend_from_slice(if text[0] == b'+' { b" += 1" } else { b" -= 1" });
        return Some(super::let_cmd::let_command(rt, host, &assignment, 0, false));
    }
    if matches!(text, b"{" | b"}") {
        let result = if text == b"{" {
            let frame = rt.current_frame_mut().unwrap();
            if frame.cstack.len() >= 50 {
                Err(VimError::new(579, "Block nesting too deep").into())
            } else {
                frame.cstack.push(super::exec::Cond::new(
                    super::exec::CondKind::Block,
                    executing,
                    executing,
                ));
                Ok(())
            }
        } else {
            let frame = rt.current_frame_mut().unwrap();
            if frame
                .cstack
                .last()
                .is_some_and(|c| c.kind == super::exec::CondKind::Block)
            {
                frame.cstack.pop();
                Ok(())
            } else {
                Err(VimError::new(1025, "Using } outside a block").into())
            }
        };
        return Some(outcome(result));
    }
    if matches!(text.first(), Some(b'&' | b'@' | b'$')) && text.contains(&b'=')
        || text.starts_with(b"[") && text.windows(2).any(|s| s == b"] ") && text.contains(&b'=')
    {
        return Some(if executing {
            super::let_cmd::let_command(rt, host, raw, at, false)
        } else {
            outcome(Ok(()))
        });
    }
    if matches!(word, b"class" | b"interface" | b"abstract" | b"enum") {
        return Some(super::objects::define(
            rt,
            host,
            &raw[arg..],
            word,
            executing,
        ));
    }
    if word == b"type" {
        return Some(outcome(if executing {
            (|| {
                if rt.current_scope().is_some() {
                    return Err(VimError::new(
                        1399,
                        "Type aliases can only be defined at script level",
                    )
                    .into());
                }
                let text = String::from_utf8_lossy(&raw[arg..]);
                let (name, source) = text
                    .split_once('=')
                    .ok_or_else(|| VimError::new(1395, "Type alias requires ="))?;
                let name = name.trim();
                if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                    || !name.bytes().all(super::expr::is_name_char)
                {
                    return Err(VimError::new(1395, "Invalid type alias name").into());
                }
                let mut end = 0;
                let ty = types::parse_active(source.as_bytes(), &mut end, rt, host)?;
                if !source[end..].trim().is_empty() {
                    return Err(VimError::trailing(&source.as_bytes()[end..]).into());
                }
                types::validate(&ty, rt)?;
                rt.state
                    .define_type(rt.current_sid(), name.into(), ty.clone())?;
                rt.declaration_scope().insert(
                    name,
                    Value::TypeAlias(std::sync::Arc::new(super::value::TypeAlias {
                        name: name.into(),
                        ty,
                    })),
                );
                Ok(())
            })()
        } else {
            Ok(())
        }));
    }
    if word == b"import" {
        return Some(outcome(if executing {
            import(rt, host, raw, arg)
        } else {
            Ok(())
        }));
    }
    if word == b"export" {
        if rt.current_scope().is_some() {
            return Some(outcome(Err(VimError::new(
                1042,
                "Export can only be used at script level",
            )
            .into())));
        }
        let item = String::from_utf8_lossy(&raw[arg..]);
        let mut words = item.split_whitespace();
        let command = words.next().unwrap_or("");
        let abstract_class = command == "abstract" && words.next() == Some("class");
        let name = words
            .next()
            .unwrap_or("")
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("")
            .to_string();
        if !abstract_class
            && !matches!(
                command,
                "var" | "final" | "const" | "def" | "class" | "interface" | "enum" | "type"
            )
        {
            return Some(outcome(Err(VimError::new(
                1043,
                "Invalid command after export",
            )
            .into())));
        }
        let sid = rt.current_sid();
        let result = super::commands::execute_command(rt, host, line, arg, executing);
        if executing && result.result.is_ok() {
            if let Some(vars) = rt.state.script_vars(sid) {
                if command == "def" {
                    let internal = format!("<SNR>{sid}_{name}");
                    let reference = super::Funcref::named(internal.clone());
                    let reference = rt
                        .state
                        .function(internal.as_bytes())
                        .map_or(reference.clone(), |f| {
                            reference.with_type(super::compiler::signature(&f))
                        });
                    vars.insert(name.clone(), Value::Funcref(reference));
                }
                vars.data()
                    .exports
                    .get_or_insert_with(Default::default)
                    .insert(name.into());
            }
        }
        return Some(result);
    }
    if matches!(word, b"var" | b"final" | b"const") {
        return Some(if executing {
            declare(rt, host, raw, arg, word)
        } else {
            let next = raw
                .windows(3)
                .position(|s| s == b"=<<")
                .and_then(|at| super::let_cmd::heredoc(rt, raw, at + 3).ok())
                .map_or(Next::NextLine, |(_, end, _, _)| Next::Position(end + 1, 0));
            CommandOutcome {
                next,
                result: Ok(()),
            }
        });
    }
    if word == b"let" {
        return Some(outcome(if executing {
            Err(VimError::new(1126, "Cannot use :let in Vim9 script").into())
        } else {
            Ok(())
        }));
    }
    if word == b"defcompile" {
        return Some(outcome(if executing {
            compile(rt, host, &raw[arg..])
        } else {
            Ok(())
        }));
    }
    if word == b"enddef" {
        return Some(outcome(Err(
            VimError::new(1157, ":enddef without :def").into()
        )));
    }
    // Commands beginning with expressions include assignments and function
    // calls. The legacy command table is still the sole Ex dispatcher.
    let mut eval = Eval::new(rt, host, raw, at, false);
    if let Ok(Some(_)) = eval.parse_name() {
        let mut pos = skip_white(raw, eval.pos);
        // Follow lvalue members/indexes without evaluating their expressions.
        loop {
            if raw.get(pos) == Some(&b'.')
                && raw
                    .get(pos + 1)
                    .is_some_and(|b| super::expr::is_name_char(*b))
            {
                pos += 1;
                while raw.get(pos).is_some_and(|b| super::expr::is_name_char(*b)) {
                    pos += 1;
                }
            } else if raw.get(pos) == Some(&b'[') {
                let mut depth = 1;
                pos += 1;
                while pos < raw.len() && depth > 0 {
                    match raw[pos] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        _ => {}
                    }
                    pos += 1;
                }
            } else {
                break;
            }
        }
        pos = skip_white(raw, pos);
        if raw.get(pos..).is_some_and(|rest| {
            ["=", "+=", "-=", "*=", "/=", "%=", "..="]
                .iter()
                .any(|op| rest.starts_with(op.as_bytes()))
        }) {
            return Some(if executing {
                super::let_cmd::let_command(rt, host, raw, at, false)
            } else {
                outcome(Ok(()))
            });
        }
        if matches!(raw.get(pos), Some(b'(' | b'<'))
            && pos > 0
            && !raw[pos - 1].is_ascii_whitespace()
            || raw.get(pos..).is_some_and(|s| s.starts_with(b"->"))
        {
            return Some(expression_statement(rt, host, raw, at, executing));
        }
    }
    if matches!(text.first(), Some(b'\'' | b'"' | b'[' | b'(')) {
        return Some(expression_statement(rt, host, raw, at, executing));
    }
    None
}

pub(crate) fn specialize(
    rt: &Runtime,
    reference: &super::Funcref,
    types: Vec<Type>,
) -> EvalResult<super::Funcref> {
    let original = reference
        .specialization()
        .or_else(|| rt.state.function(reference.name().as_bytes()))
        .ok_or_else(|| {
            VimError::new(
                if reference.class().is_some() {
                    1560
                } else {
                    1558
                },
                "Generic function not found",
            )
        })?;
    let mut function = (*original).clone();
    let signature = function
        .signature
        .as_mut()
        .ok_or_else(|| VimError::new(1560, "Not a generic function"))?;
    if signature.generics.is_empty() {
        return Err(VimError::new(1560, "Not a generic function").into());
    }
    if signature.generics.len() != types.len() {
        return Err(VimError::new(
            if types.len() > signature.generics.len() {
                1556
            } else {
                1557
            },
            "Wrong number of type arguments",
        )
        .into());
    }
    signature
        .bindings
        .extend(signature.generics.iter().cloned().zip(types));
    signature.parameters = signature
        .parameters
        .iter()
        .map(|t| types::substitute(t, &signature.bindings))
        .collect();
    signature.result = types::substitute(&signature.result, &signature.bindings);
    signature.rest = signature
        .rest
        .as_ref()
        .map(|(n, t)| (n.clone(), types::substitute(t, &signature.bindings)));
    signature.generics.clear();
    signature.checked = Default::default();
    Ok(reference.specialized(std::sync::Arc::new(function)))
}

fn compile(rt: &mut Runtime, host: &mut dyn ScriptHost, arg: &[u8]) -> EvalResult<()> {
    if let Some(open) = arg.iter().position(|b| *b == b'<') {
        let mut at = open + 1;
        let mut types = vec![];
        loop {
            types.push(types::parse_active(arg, &mut at, rt, host)?);
            at = skip_white(arg, at);
            if arg.get(at) == Some(&b'>') {
                at += 1;
                break;
            }
            if arg.get(at) != Some(&b',') {
                return Err(VimError::new(1556, "Missing >").into());
            }
            at += 1;
        }
        if skip_white(arg, at) != arg.len() {
            return Err(VimError::trailing(&arg[at..]).into());
        }
        let reference = rt.resolve_function_name(host, &arg[..open])?;
        let reference = specialize(rt, &reference, types)?;
        return super::compiler::ensure(rt, host, &reference.specialization().unwrap());
    }
    let functions = if arg.is_empty() {
        rt.state
            .function_names()
            .into_iter()
            .filter_map(|name| rt.state.function(name.as_bytes()))
            .filter(|f| f.sid == rt.current_sid() && f.signature.is_some())
            .collect::<Vec<_>>()
    } else if let Ok(Value::Class(class)) = rt.lookup_variable(host, arg, false) {
        class
            .methods
            .values()
            .chain(class.static_methods.values())
            .filter_map(|name| rt.state.function(name.as_bytes()))
            .collect()
    } else {
        vec![rt
            .state
            .function(rt.canonical_function_name(arg).as_bytes())
            .ok_or_else(|| VimError::unknown_function(arg))?]
    };
    for function in functions {
        if function
            .signature
            .as_ref()
            .is_some_and(|s| !s.generics.is_empty())
        {
            continue;
        }
        if !arg.is_empty()
            && function
                .signature
                .as_ref()
                .is_some_and(|s| s.checked.load(std::sync::atomic::Ordering::Relaxed))
        {
            let name = function.name.to_str_lossy();
            let name = name
                .strip_prefix("<SNR>")
                .and_then(|s| s.split_once('_'))
                .map_or(name.as_ref(), |(_, name)| name);
            let name = name
                .split_once('#')
                .and_then(|(class, rest)| {
                    rest.split_once('.')
                        .map(|(_, method)| format!("{class}.{method}"))
                })
                .unwrap_or_else(|| name.to_string());
            rt.emit(
                host,
                super::exec::Message::Output(format!("Function {name} does not need compiling")),
            );
        } else {
            super::compiler::ensure(rt, host, &function)?;
        }
    }
    Ok(())
}

fn import(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    mut at: usize,
) -> EvalResult<()> {
    if rt.current_scope().is_some() {
        return Err(VimError::new(1094, "Import can only be used at script level").into());
    }
    let lazy = src[at..].starts_with(b"autoload ");
    if lazy {
        at = skip_white(src, at + 8);
    }
    let mut eval = Eval::new(rt, host, src, at, true);
    let filename = eval.eval1()?.to_text()?;
    at = skip_white(src, eval.pos);
    let alias = if src.get(at..).is_some_and(|s| s.starts_with(b"as ")) {
        String::from_utf8_lossy(&src[skip_white(src, at + 3)..])
            .trim()
            .to_string()
    } else {
        if at != src.len() {
            return Err(VimError::trailing(&src[at..]).into());
        }
        std::path::Path::new(&filename)
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_suffix(".vim"))
            .ok_or_else(|| VimError::new(1257, "Import requires 'as name'"))?
            .to_string()
    };
    if alias.is_empty()
        || !super::expr::is_name_start(alias.as_bytes()[0])
        || !alias.bytes().all(super::expr::is_name_char)
    {
        return Err(VimError::new(1047, "Invalid import name").into());
    }
    if rt.lookup_variable(host, alias.as_bytes(), false).is_ok() {
        return Err(VimError::new(1073, "Name already defined").into());
    }
    let from = rt.state.script_name(rt.current_sid()).unwrap_or_default();
    if let Some(path) = host.resolve_import(&from, &filename, lazy) {
        let path = path?;
        if !lazy {
            module(rt, host, &path)?;
        }
        return rt
            .state
            .add_import(rt.current_sid(), alias.into(), path)
            .map_err(super::Fail::from);
    }
    let path = std::path::Path::new(&filename);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else if filename.starts_with('.') {
        std::path::Path::new(&rt.state.script_name(rt.current_sid()).unwrap_or_default())
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .join(path)
    } else {
        let rtp = host
            .get_option("runtimepath", super::OptionScope::Global)?
            .to_text()?;
        let sub = if lazy { "autoload" } else { "import" };
        rtp.split(',')
            .map(|base| std::path::Path::new(base).join(sub).join(path))
            .find(|p| p.is_file())
            .ok_or_else(|| VimError::new(1053, format!("Could not import {filename}")))?
    };
    // Lexical normalization also works with virtual/in-memory hosts. Actual
    // reads and symlink confinement are owned by the host.
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if normalized.file_name().is_some() => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let path = normalized.to_string_lossy().into_owned();
    if !lazy {
        module(rt, host, &path)?;
    }
    rt.state.add_import(rt.current_sid(), alias.into(), path)?;
    Ok(())
}

pub(crate) fn module(rt: &mut Runtime, host: &mut dyn ScriptHost, path: &str) -> EvalResult<Value> {
    let sid = rt.state.script_id(path);
    let (loaded, loading) = rt.state.module_status(sid);
    if loading {
        return Ok(Value::Module(rt.state.script_vars(sid).unwrap()));
    }
    let vars = rt.state.script_vars(sid).unwrap();
    if !loaded {
        rt.state.module_loading(sid, true, false);
        vars.data().exports = Some(Default::default());
        let errors = rt.errors.len();
        let result = if let Some(result) = host.import_module(rt, path) {
            result
        } else {
            host.read_file(path)
                .map_err(super::Fail::from)
                .and_then(|bytes| {
                    rt.source_lines_now(host, path, super::source::script_lines(&bytes))
                })
        };
        let success = result.is_ok() && rt.errors.len() == errors;
        rt.state.module_loading(sid, false, success);
        result?;
        if !success {
            return Err(VimError::new(1053, format!("Could not import {path}")).into());
        }
        if !rt.state.script_is_vim9(sid) {
            return Err(
                VimError::new(1048, "Item not found in script: vim9script required").into(),
            );
        }
    }
    Ok(Value::Module(vars))
}

pub(crate) fn start_script(rt: &mut Runtime, arguments: &[u8]) -> EvalResult<()> {
    let Some(frame) = rt.current_frame() else {
        return Err(VimError::new(1038, "vim9script can only be used in a script").into());
    };
    if !matches!(frame.kind, FrameKind::Script { .. }) {
        return Err(VimError::new(1038, "vim9script can only be used in a script").into());
    }
    if frame.vim9
        || frame.lines[..frame.line].iter().any(|line| {
            let s = line.text.to_str_lossy();
            let s = s.trim();
            !s.is_empty() && !s.starts_with('"') && !s.starts_with('#')
        })
    {
        return Err(VimError::new(1039, "vim9script must be the first command in a script").into());
    }
    let arguments = String::from_utf8_lossy(arguments);
    let arguments = arguments.split('#').next().unwrap_or("").trim();
    if !matches!(arguments, "" | "noclear") {
        return Err(VimError::trailing(arguments.as_bytes()).into());
    }
    let sid = rt.current_sid();
    if arguments != "noclear" {
        rt.state.reset_vim9_script(sid);
    }
    rt.state.set_script_vim9(sid, true);
    let frame = rt.current_frame_mut().unwrap();
    frame.vim9 = true;
    frame.abort_on_error = true;
    Ok(())
}

fn expression_statement(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    at: usize,
    executing: bool,
) -> CommandOutcome {
    if !executing {
        return outcome(Ok(()));
    }
    let mut eval = Eval::new(rt, host, src, at, true);
    let value = eval.eval1();
    let end = skip_white(src, eval.pos);
    let next = if src.get(end) == Some(&b'|') {
        Next::Column(end + 1)
    } else {
        Next::NextLine
    };
    CommandOutcome {
        next,
        result: value.and_then(|_| {
            if end < src.len() && !matches!(src[end], b'#' | b'|') {
                Err(VimError::trailing(&src[end..]).into())
            } else {
                Ok(())
            }
        }),
    }
}

fn declare(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    mut at: usize,
    kind: &[u8],
) -> CommandOutcome {
    if src.get(at) == Some(&b'[') {
        return declare_many(rt, host, src, at, kind);
    }
    let mut next = Next::NextLine;
    let result = (|| {
        let name = binding_name(src, &mut at, kind, rt.current_scope().is_some())?;
        let name = name.as_slice();
        let ty = if src.get(at) == Some(&b':') {
            at += 1;
            Some(types::parse_active(src, &mut at, rt, host)?)
        } else {
            None
        };
        if let Some(ty) = &ty {
            types::validate_storage(ty, rt)?;
        }
        at = skip_white(src, at);
        let (dict, key) = if name.get(1) == Some(&b':') {
            let (dict, key, _) = rt.variable_slot(host, name)?;
            (dict, key)
        } else {
            (rt.declaration_scope(), name.to_vec())
        };
        if rt.lookup_variable(host, name, false).is_ok() {
            return Err(VimError::new(
                1041,
                format!("Redefining script item: {}", String::from_utf8_lossy(name)),
            )
            .into());
        }
        if src.get(at) == Some(&b'=')
            && (at == 0
                || !src[at - 1].is_ascii_whitespace()
                || !src
                    .get(at + 1)
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'<'))
        {
            return Err(VimError::new(1004, "White space required around =").into());
        }
        let value = if src.get(at..).is_some_and(|s| s.starts_with(b"=<<")) {
            let (value, end) = super::let_cmd::heredoc_value(rt, host, src, at + 3)?;
            next = Next::Position(end + 1, 0);
            value
        } else if src.get(at) == Some(&b'=') {
            at += 1;
            let mut eval = Eval::new(rt, host, src, skip_white(src, at), true);
            let value = eval.eval1()?;
            let end = skip_white(src, eval.pos);
            if end < src.len() && !matches!(src[end], b'#' | b'|') {
                return Err(VimError::trailing(&src[end..]).into());
            }
            if src.get(end) == Some(&b'|') {
                next = Next::Column(end + 1);
            }
            value
        } else if at == src.len() || src[at] == b'#' {
            if kind != b"var" {
                return Err(VimError::new(1021, "Constant requires a value").into());
            }
            ty.as_ref()
                .ok_or_else(|| VimError::new(1022, "Type or initialization required"))?
                .default_value()
        } else {
            return Err(VimError::trailing(&src[at..]).into());
        };
        let ty = ty.unwrap_or_else(|| Type::infer(&value));
        if ty != Type::Any && !matches!(value, Value::NullTyped(_)) {
            types::validate(&ty, rt)?;
        }
        let value = ty.check(value)?;
        if matches!(ty, Type::Func { .. }) && !name.first().is_some_and(u8::is_ascii_uppercase) {
            return Err(
                VimError::new(704, "Funcref variable name must start with a capital").into(),
            );
        }
        ty.attach(&value);
        if kind == b"const" {
            super::let_cmd::lock_value(&value, i64::MAX, true)?;
        }
        dict.data()
            .declarations
            .insert(key.clone().into(), ty.clone());
        // Publish after both type validation and any recursive locking succeeded.
        dict.insert(key.clone(), value);
        if kind != b"var" {
            dict.data().items.get_mut(key.as_slice()).unwrap().lock = super::value::Lock::Fixed;
        }
        Ok(())
    })();
    CommandOutcome { next, result }
}

fn declare_many(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    mut at: usize,
    kind: &[u8],
) -> CommandOutcome {
    outcome((|| {
        at += 1;
        let mut names = vec![];
        let mut rest = false;
        loop {
            at = skip_white(src, at);
            if src.get(at) == Some(&b']') {
                at += 1;
                break;
            }
            let start = at;
            while src.get(at).is_some_and(|b| super::expr::is_name_char(*b)) {
                at += 1;
            }
            if at == start {
                return Err(VimError::new(1017, "Variable name required").into());
            }
            let name = VStr::from(&src[start..at]);
            if names.iter().any(|(n, _)| *n == name)
                || rt.lookup_variable(host, name.as_bytes(), false).is_ok()
            {
                return Err(VimError::new(1041, "Variable already defined").into());
            }
            let ty = if src.get(at) == Some(&b':') {
                at += 1;
                Some(types::parse_active(src, &mut at, rt, host)?)
            } else {
                None
            };
            names.push((name, ty));
            at = skip_white(src, at);
            match src.get(at) {
                Some(b',') if !rest => at += 1,
                Some(b';') if !rest => {
                    rest = true;
                    at += 1;
                }
                Some(b']') => {}
                _ => return Err(VimError::new(475, "Invalid destructuring declaration").into()),
            }
        }
        at = skip_white(src, at);
        if src.get(at) != Some(&b'=') {
            return Err(VimError::new(1022, "Initialization required").into());
        }
        let mut eval = Eval::new(rt, host, src, skip_white(src, at + 1), true);
        let value = eval.eval1()?;
        if skip_white(src, eval.pos) != src.len() {
            return Err(VimError::trailing(&src[eval.pos..]).into());
        }
        let items = match value {
            Value::List(list) => list.snapshot(),
            Value::Tuple(items) => items.as_ref().clone(),
            _ => return Err(VimError::new(714, "List or Tuple required").into()),
        };
        let required = names.len().saturating_sub(rest as usize);
        if items.len() < required {
            return Err(VimError::new(688, "More targets than items").into());
        }
        if !rest && items.len() != required {
            return Err(VimError::new(687, "Less targets than items").into());
        }
        let mut prepared = vec![];
        for (index, (name, ty)) in names.into_iter().enumerate() {
            let value = if rest && index == required {
                Value::list(items[index..].to_vec())
            } else {
                items[index].clone()
            };
            let ty = ty.unwrap_or_else(|| Type::infer(&value));
            let value = ty.check(value)?;
            prepared.push((name, ty, value));
        }
        let dict = rt.declaration_scope();
        for (name, ty, value) in prepared {
            ty.attach(&value);
            if kind == b"const" {
                super::let_cmd::lock_value(&value, i64::MAX, true)?;
            }
            dict.data().declarations.insert(name.clone(), ty);
            dict.insert(name.clone(), value);
            if kind != b"var" {
                dict.data().items.get_mut(name.as_bytes()).unwrap().lock =
                    super::value::Lock::Fixed;
            }
        }
        Ok(())
    })())
}

fn define(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    mut at: usize,
    executing: bool,
) -> CommandOutcome {
    let force = src.get(at) == Some(&b'!');
    if force {
        at = skip_white(src, at + 1);
    }
    let start = rt.current_frame().map_or(0, |f| f.line);
    let Some(frame) = rt.current_frame() else {
        return outcome(Err(VimError::new(1057, "Missing :enddef").into()));
    };
    let end = match super::source::definition_end(&frame.lines, start, "enddef") {
        Ok(end) => end,
        Err(error) => return outcome(Err(error.into())),
    };
    let body = frame.lines[start + 1..end].to_vec();
    let next = Next::Position(end + 1, 0);
    if !executing {
        return CommandOutcome {
            next,
            result: Ok(()),
        };
    }
    let local_name = src[at..]
        .split(|b| matches!(b, b'(' | b'<'))
        .next()
        .unwrap_or_default();
    if !local_name
        .strip_prefix(b"g:")
        .or_else(|| local_name.strip_prefix(b"s:"))
        .unwrap_or(local_name)
        .first()
        .is_some_and(u8::is_ascii_uppercase)
    {
        return CommandOutcome {
            next,
            result: Err(VimError::new(1267, "Function name must start with a capital").into()),
        };
    }
    let result = parse_function(rt, host, &src[at..], body, start + 1).and_then(|mut function| {
        if rt.current_scope().is_some() {
            let local_name = src[at..]
                .split(|b| matches!(b, b'(' | b'<'))
                .next()
                .unwrap_or_default();
            if !local_name.starts_with(b"g:") {
                let scope = rt.declaration_scope();
                if scope.contains(local_name) {
                    return Err(VimError::new(1073, "Name already defined").into());
                }
                function.name = rt.state.next_lambda_name();
                let value = Value::Funcref(
                    super::Funcref::named(function.name.clone())
                        .with_type(super::compiler::signature(&function)),
                );
                rt.state.define_function(function)?;
                scope.insert(local_name, value);
                return Ok(());
            }
        }
        if !force && rt.state.function(function.name.as_bytes()).is_some() {
            return Err(VimError::new(1073, "Name already defined").into());
        }
        rt.state.define_function(function)?;
        Ok(())
    });
    CommandOutcome { next, result }
}

pub(crate) fn parse_function(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    src: &[u8],
    body: Vec<super::state::SourceLine>,
    lnum: usize,
) -> EvalResult<UserFunction> {
    let mut at = 0;
    let name_start = at;
    while src
        .get(at)
        .is_some_and(|c| super::expr::is_name_char(*c) || matches!(c, b':' | b'#'))
    {
        at += 1;
    }
    let name = &src[name_start..at];
    let mut generics = vec![];
    if src.get(at) == Some(&b'<') {
        at += 1;
        if src.get(skip_white(src, at)) == Some(&b'>') {
            return Err(VimError::new(1555, "Empty type parameter list").into());
        }
        loop {
            at = skip_white(src, at);
            let start = at;
            while src.get(at).is_some_and(|b| super::expr::is_name_char(*b)) {
                at += 1;
            }
            let name = String::from_utf8_lossy(&src[start..at]).into_owned();
            if !name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                || generics.contains(&name)
            {
                return Err(VimError::new(1552, "Invalid or duplicate type parameter").into());
            }
            generics.push(name);
            at = skip_white(src, at);
            if src.get(at) == Some(&b'>') {
                at += 1;
                break;
            }
            if src.get(at) != Some(&b',') {
                return Err(VimError::new(1553, "Missing > after type parameters").into());
            }
            at += 1;
        }
    }
    if name.is_empty() || src.get(at) != Some(&b'(') {
        return Err(VimError::new(1056, "Expected a function name followed by (").into());
    }
    at += 1;
    let mut params: Vec<Param> = vec![];
    let mut parameters: Vec<Type> = vec![];
    let mut rest = None;
    while src.get(skip_white(src, at)) != Some(&b')') {
        at = skip_white(src, at);
        let variadic = src.get(at..).is_some_and(|s| s.starts_with(b"..."));
        if variadic {
            at += 3;
        }
        let start = at;
        while src.get(at).is_some_and(|c| super::expr::is_name_char(*c)) {
            at += 1;
        }
        let name = VStr::from(&src[start..at]);
        if name.is_empty() {
            return Err(VimError::new(1055, "Missing argument name").into());
        }
        validate_binding(name.as_bytes())?;
        if name.as_bytes() != b"_" && rt.lookup_variable(host, name.as_bytes(), false).is_ok() {
            return Err(VimError::new(1168, "Argument already declared in the script").into());
        }
        let annotated = src.get(at) == Some(&b':');
        let mut ty = if annotated {
            at += 1;
            types::parse_active(src, &mut at, rt, host)?
        } else {
            Type::Any
        };
        at = skip_white(src, at);
        let default = if src.get(at) == Some(&b'=') {
            at = skip_white(src, at + 1);
            let start = at;
            let mut eval = Eval::new(rt, host, src, at, false);
            eval.eval1()?;
            at = eval.pos;
            Some(VStr::from(&src[start..at]))
        } else {
            None
        };
        if !annotated && name.as_bytes() != b"_" {
            let source = default
                .as_ref()
                .ok_or_else(|| VimError::new(1077, "Missing argument type"))?;
            let mut context = super::compiler::Context::default();
            for (param, ty) in params.iter().zip(&parameters) {
                context
                    .variables
                    .insert(param.name.as_bytes().to_vec(), ty.clone());
            }
            ty = Type::infer(&Eval::analyze(rt, host, source.as_bytes(), 0, &context).eval1()?);
        }
        if rest.is_some() {
            return Err(VimError::new(1055, "Varargs must be last").into());
        }
        if variadic {
            if default.is_some() {
                return Err(VimError::new(1055, "Varargs cannot have a default value").into());
            }
            if !matches!(ty, Type::List(_)) {
                return Err(VimError::new(1013, "Varargs type must be a list").into());
            }
            rest = Some((name, ty));
        } else {
            if name.as_bytes() != b"_" && params.iter().any(|p: &Param| p.name == name) {
                return Err(VimError::new(853, "Duplicate argument name").into());
            }
            if default.is_none() && params.iter().any(|p: &Param| p.default.is_some()) {
                return Err(
                    VimError::new(989, "Non-default argument follows default argument").into(),
                );
            }
            params.push(Param { name, default });
            parameters.push(ty);
        }
        at = skip_white(src, at);
        if src.get(at) == Some(&b',') {
            at += 1;
        } else if src.get(at) != Some(&b')') {
            return Err(VimError::new(110, "Missing ')'").into());
        }
    }
    at = skip_white(src, at) + 1;
    at = skip_white(src, at);
    let result = if src.get(at) == Some(&b':') {
        at += 1;
        types::parse_active(src, &mut at, rt, host)?
    } else {
        Type::Void
    };
    at = skip_white(src, at);
    if at < src.len() && src[at] != b'#' {
        return Err(VimError::trailing(&src[at..]).into());
    }
    let name = if name.starts_with(b"g:") {
        VStr::from(&name[2..])
    } else {
        let name = name.strip_prefix(b"s:").unwrap_or(name);
        VStr::from(format!(
            "<SNR>{}_{}",
            rt.current_sid(),
            String::from_utf8_lossy(name)
        ))
    };
    Ok(UserFunction {
        name,
        params,
        varargs: rest.is_some(),
        body: FunctionBody::Lines(body.into()),
        flags: FunctionFlags {
            abort: true,
            closure: true,
            ..Default::default()
        },
        sid: rt.current_sid(),
        def_lnum: lnum,
        outer: rt.capture_scope(),
        signature: Some(Signature {
            generics,
            lexical_types: Default::default(),
            bindings: rt
                .current_frame()
                .and_then(|f| f.function.as_ref())
                .and_then(|f| f.signature.as_ref())
                .map_or_else(Default::default, |s| s.bindings.clone()),
            checked: Default::default(),
            parameters,
            rest,
            result,
            owner: rt
                .current_frame()
                .and_then(|f| f.function.as_ref())
                .and_then(|f| f.signature.as_ref())
                .and_then(|s| s.owner.clone()),
        }),
    })
}

#[cfg(test)]
mod tests {
    use crate::vimscript::tests::run_script;

    #[test]
    fn invalid_class_methods_report_the_primary_vim_error() {
        // Vim may subsequently execute rejected definition bodies while
        // recovering. Check the primary diagnostic, not recovery cascades.
        for (definition, code) in [
            (
                "class C\n  def len(): string\n    return 'bad'\n  enddef\nendclass",
                1383,
            ),
            (
                "class C\n  static def string(): string\n    return 'bad'\n  enddef\nendclass",
                1413,
            ),
            ("class C\n  def lowercase()\n  enddef\nendclass", 1267),
            (
                "class C\n  def new(): string\n    return 'bad'\n  enddef\nendclass",
                1365,
            ),
            ("abstract class C\n  def new()\n  enddef\nendclass", 1359),
        ] {
            let output = run_script(&format!("vim9script\n{definition}\n"));
            assert!(
                output.contains(&format!("E{code}:")),
                "{definition}: {output}"
            );
        }
    }

    #[test]
    fn object_string_hooks_share_exception_and_recursion_limits() {
        let output = run_script("vim9script\nclass Recursive\n  def string(): string\n    return string(this)\n  enddef\nendclass\ntry\n  echo [Recursive.new()]\ncatch /E132:/\n  echo 'bounded'\nendtry\necho 'resumed'\n");
        assert_eq!(output, "\nbounded\nresumed");
    }

    #[test]
    fn reloading_a_class_preserves_old_object_identity_and_methods() {
        use crate::vimscript::{
            tests::{run_with, MemoryHost},
            RunState,
        };
        let mut host = MemoryHost::new();
        let mut runtime = run_with(&mut host, "vim9script\nclass Box\n  def Value(): number\n    return 1\n  enddef\nendclass\ng:old = Box.new()\n");
        runtime.push_script("test.vim", b"vim9script\nclass Box\n  def Value(): number\n    return 2\n  enddef\nendclass\ntry\n  var current: Box = g:old\ncatch\n  echo 'distinct'\nendtry\necho g:old.Value() Box.new().Value()\n").unwrap();
        assert_eq!(runtime.run(&mut host), RunState::Complete);
        assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
        assert_eq!(host.output, "\ndistinct\n1 2");
    }

    #[test]
    fn lazy_imported_class_types_resolve_through_the_host() {
        use crate::vimscript::tests::{run_with, MemoryHost};
        let mut host = MemoryHost::new();
        host.files.insert(
            "types.vim".into(),
            b"vim9script\nexport class Box\n  var number: number\nendclass\n".to_vec(),
        );
        let runtime = run_with(&mut host, "vim9script\nimport autoload './types.vim' as types\ndef Read(value: types.Box): number\n  return value.number\nenddef\necho Read(types.Box.new(9))\n");
        assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
        assert_eq!(host.output, "\n9");
    }

    #[test]
    fn lambda_types_load_explicit_imports_only_after_recognizing_a_lambda() {
        use crate::vimscript::tests::{run_with, MemoryHost};
        let mut host = MemoryHost::new();
        host.files.insert(
            "types.vim".into(),
            b"vim9script\necho 'loaded'\nexport class Box\n  var n: number\nendclass\n".to_vec(),
        );
        let runtime = run_with(&mut host, "vim9script\nimport autoload './types.vim' as types\nvar Read = (value: types.Box): number => value.n\necho Read(types.Box.new(9))\necho true ? (1) : 2\n");
        assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
        assert_eq!(host.output, "\nloaded\n9\n1");
    }

    #[test]
    fn teardown_releases_reloaded_enum_and_orphaned_closure_cycles() {
        use crate::vimscript::{
            tests::{run_with, MemoryHost},
            RunState, Value,
        };
        let mut host = MemoryHost::new();
        let mut runtime = run_with(&mut host, "vim9script\nenum Color\n  Red, Blue\nendenum\ndef Make(): func\n  var values: list<any> = []\n  var Ref = () => values\n  add(values, Ref)\n  return Ref\nenddef\nvar Ref = Make()\n");
        assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
        let vars = runtime.state.script_vars(1).unwrap();
        let Some(Value::Class(class)) = vars.get_str("Color") else {
            panic!("missing enum")
        };
        let class_weak = std::sync::Arc::downgrade(&class);
        let Some(Value::Funcref(reference)) = vars.get_str("Ref") else {
            panic!("missing closure")
        };
        let function = runtime.state.function(reference.name().as_bytes()).unwrap();
        let captured = function.outer.as_ref().unwrap().locals.downgrade();
        drop((class, reference, function, vars));
        runtime.push_script("test.vim", b"vim9script\n").unwrap();
        assert_eq!(runtime.run(&mut host), RunState::Complete);
        drop(runtime);
        assert!(
            class_weak.upgrade().is_none(),
            "enum cycle retained after state teardown"
        );
        assert!(
            captured.upgrade().is_none(),
            "orphaned closure retained after state teardown"
        );
    }

    #[test]
    fn compilation_checks_uncalled_branches_without_running_effects() {
        let output = run_script("vim9script\nvar calls = 0\ndef Effect(): number\n  calls += 1\n  return calls\nenddef\ndef Valid(): number\n  return Effect()\nenddef\ndefcompile Valid\necho calls\ndef Invalid(): number\n  if g:condition\n    var wrong: number = 'bad'\n  endif\n  return Effect()\nenddef\ntry\n  defcompile Invalid\ncatch /E1012:/\n  echo 'type error' calls\nendtry\n");
        assert_eq!(output, "\n0\ntype error 0");
    }

    #[test]
    fn const_locks_nested_values_and_defer_runs_after_exceptions() {
        let output = run_script("vim9script\nconst values = [[1]]\ntry\n  add(values[0], 2)\ncatch /E74/\n  echo values\nendtry\nvar log: list<string> = []\ndef Record(message: string)\n  add(log, message)\nenddef\ndef Fail()\n  defer Record('cleanup')\n  throw 'failure'\nenddef\ntry\n  Fail()\ncatch /failure/\n  echo log\nendtry\n");
        assert_eq!(output, "\n[[1]]\n['cleanup']");
    }

    #[test]
    fn classes_share_function_execution_and_enforce_field_access() {
        let output = run_script("vim9script\nclass Counter\n  var value: number = 1\n  public var label: string = 'count'\n  def new(this.value)\n  enddef\n  def Add(n: number): number\n    this.value += n\n    return this.value\n  enddef\nendclass\nvar c = Counter.new(4)\necho c.Add(3) c.value c.label\ntry\n  c.value = 9\ncatch /E1335:/\n  echo c.value\nendtry\nc.label = 'total'\necho c.label\n");
        assert_eq!(output, "\n7 7 count\n7\ntotal");
    }

    #[test]
    fn interfaces_inheritance_super_and_enum_objects() {
        let output = run_script("vim9script\ninterface HasValue\n  def Value(): number\nendinterface\nclass Base implements HasValue\n  def Value(): number\n    return 2\n  enddef\nendclass\nclass Child extends Base\n  def Value(): number\n    return super.Value() + 1\n  enddef\nendclass\nvar x: HasValue = Child.new()\necho x.Value()\nenum Color\n  Red, Green, Blue\nendenum\necho Color.Green.name Color.Green.ordinal len(Color.values)\n");
        assert_eq!(output, "\n3\nGreen 1 3");
    }

    #[test]
    fn typed_functions_and_script_scope_share_the_runtime() {
        assert_eq!(run_script("vim9script\nvar base: number = 4\ndef Sum(a: number, b: number = 3): number\n  return base + a + b\nenddef\necho Sum(2)\nbase += 1\necho Sum(2, 1)\n"), "\n9\n8");
    }

    #[test]
    fn expressions_use_vim9_values_and_character_indexing() {
        assert_eq!(run_script("vim9script\necho true false 'a' == 'A'\nvar d = {name: 'éclair'}\necho d.name[0] d.name[1 : 2] d.name[-1]\n"), "\ntrue false false\né cl r");
    }

    #[test]
    fn type_errors_do_not_publish_the_assignment() {
        let output = run_script(
            "vim9script\nvar n: number = 3\ntry\n  n = 'wrong'\ncatch /E1012:/\n  echo n\nendtry\n",
        );
        assert_eq!(output, "\n3");
    }

    #[test]
    fn legacy_and_vim9_functions_can_call_each_other() {
        assert_eq!(run_script("def s:Modern(n: number): string\n  return string(n + 1)\nenddef\necho s:Modern(4)\n"), "\n5");
    }

    #[test]
    fn block_scopes_and_loop_closures_retain_each_iteration() {
        let output = run_script("vim9script\nvar callbacks: list<any> = []\nfor i in [1, 2]\n  var x = i * 10\n  add(callbacks, () => x)\nendfor\necho callbacks[0]() callbacks[1]() exists('x') exists('i')\n");
        assert_eq!(output, "\n10 20 0 0");
    }

    #[test]
    fn typed_collections_enforce_alias_mutation_and_final_is_shallow() {
        let output = run_script("vim9script\nfinal xs: list<number> = [1]\nvar alias = xs\nadd(xs, 2)\ntry\n  add(alias, 'bad')\ncatch /E1012:/\n  echo xs\nendtry\ntry\n  alias[0] = 'bad'\ncatch /E1012:/\n  echo xs\nendtry\n");
        assert_eq!(output, "\n[1, 2]\n[1, 2]");
    }

    #[test]
    fn implicit_continuation_and_typed_lambdas_use_shared_expressions() {
        let output = run_script("vim9script\nvar numbers = [\n  2,\n  3,\n]\nvar Twice = (n: number): number => n * 2\necho Twice(numbers[0])\necho 'ab'\n  .. 'cd'\n");
        assert_eq!(output, "\n4\nabcd");
    }

    #[test]
    fn imports_are_cached_live_namespaces_with_export_boundaries() {
        use crate::vimscript::tests::{run_with, MemoryHost};
        let mut host = MemoryHost::new();
        host.files.insert("dep.vim".into(), b"vim9script\nexport var count = 1\nvar secret = 9\nexport def Bump(): number\n  count += 1\n  return count\nenddef\n".to_vec());
        let runtime = run_with(&mut host, "vim9script\nimport './dep.vim' as dep\necho dep.count dep.Bump() dep.count\ntry\n  echo dep.secret\ncatch /E1049:/\n  echo 'private'\nendtry\n");
        assert!(runtime.errors.is_empty(), "{:?}", runtime.errors);
        assert_eq!(host.output, "\n1 2 2\nprivate");
    }
}
