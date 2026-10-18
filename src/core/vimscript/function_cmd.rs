//! `:function` and `:delfunction`.
use super::error::{EvalResult, Fail, VimError};
use super::exec::{CommandOutcome, FrameKind, Message, Next, Runtime, ScriptHost};
use super::expr::{is_name_char, is_white, skip_white, Eval};
use super::state::{FunctionBody, FunctionFlags, Param, SourceLine, UserFunction};
use super::value::{Funcref, VStr, Value};

fn outcome(next: Next, result: EvalResult<()>) -> CommandOutcome {
    CommandOutcome { next, result }
}

/// Where a function name will be stored.
enum Definition {
    Named(VStr),
    /// `dict.key` / `dict['key']`: a numbered function stored in a Dictionary.
    Member { dict: super::value::Dict, key: VStr },
}

/// Find the body of a function defined on line `start` of the current
/// frame: returns the body lines and the index of the `:endfunction` line.
fn collect_body(rt: &Runtime, start: usize) -> Result<(Vec<SourceLine>, usize), VimError> {
    let frame = rt.current_frame().expect("function runs in a frame");
    let mut depth = 1usize;
    let mut body = Vec::new();
    let mut index = start + 1;
    let mut heredoc: Option<(Vec<u8>, bool)> = None;
    while index < frame.lines.len() {
        let line = &frame.lines[index];
        let text = line.text.as_bytes();
        if let Some((marker, trim)) = &heredoc {
            let candidate = if *trim { &text[text.iter().take_while(|b| is_white(**b)).count()..] } else { text };
            if candidate == marker.as_slice() {
                heredoc = None;
            }
            body.push(SourceLine { text: line.text.clone(), lnum: body.len() + 1 });
            index += 1;
            continue;
        }
        if let Some((name, after, _)) = super::commands::line_command_name(text) {
            match name {
                "endfunction" => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((body, index));
                    }
                }
                "function" => {
                    // Only a definition (name followed by '(') nests.
                    let rest = &text[after..];
                    if rest.contains(&b'(') {
                        depth += 1;
                    }
                }
                "let" | "const" => {
                    if let Some(at) = find_subslice(text, b"=<<") {
                        let words: Vec<&[u8]> = text[at + 3..].split(|b| is_white(*b)).filter(|w| !w.is_empty()).collect();
                        let trim = words.iter().any(|w| *w == b"trim");
                        if let Some(marker) = words.iter().find(|w| **w != b"trim" && **w != b"eval") {
                            if !marker.starts_with(b"\"") {
                                heredoc = Some((marker.to_vec(), trim));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        body.push(SourceLine { text: line.text.clone(), lnum: body.len() + 1 });
        index += 1;
    }
    Err(VimError::new(126, "Missing :endfunction"))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

pub(crate) fn function_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, bang: bool, executing: bool) -> CommandOutcome {
    let at = skip_white(src, at);
    if at >= src.len() {
        if executing {
            list_functions(rt, host, None);
        }
        return outcome(Next::NextLine, Ok(()));
    }
    if src[at] == b'/' {
        if executing {
            let pattern = src[at + 1..].strip_suffix(b"/").unwrap_or(&src[at + 1..]).to_vec();
            list_functions(rt, host, Some(&pattern));
        }
        return outcome(Next::NextLine, Ok(()));
    }
    // Parse the name (scopes, <SID>, curly braces, dict members).
    let mut eval = Eval::new(rt, host, src, at, executing);
    let name = match eval.parse_name() {
        Ok(Some(name)) => name,
        Ok(None) => return outcome(Next::NextLine, Err(Fail::Error(VimError::new(129, "Function name required")))),
        Err(fail) => return outcome(Next::NextLine, if executing { Err(fail) } else { Ok(()) }),
    };
    let mut members: Vec<VStr> = Vec::new();
    loop {
        if eval.peek() == b'.' && eval.src.get(eval.pos + 1).is_some_and(|b| is_name_char(*b)) {
            eval.pos += 1;
            let start = eval.pos;
            while is_name_char(eval.peek()) {
                eval.pos += 1;
            }
            members.push(VStr::from(&eval.src[start..eval.pos]));
        } else if eval.peek() == b'[' {
            eval.pos += 1;
            let key = eval.eval1();
            eval.skip_white();
            if eval.peek() == b']' {
                eval.pos += 1;
            }
            match key {
                Ok(key) if executing => match key.to_vstr() {
                    Ok(key) => members.push(key),
                    Err(error) => return outcome(Next::NextLine, Err(Fail::Error(error))),
                },
                Ok(_) => members.push(VStr::empty()),
                Err(fail) => return outcome(Next::NextLine, if executing { Err(fail) } else { Ok(()) }),
            }
        } else {
            break;
        }
    }
    let after_name = eval.pos;
    let paren = skip_white(src, after_name);
    if src.get(paren) != Some(&b'(') {
        // `:function Name` lists the function.
        if executing {
            let canonical = rt.canonical_function_name(&name);
            match rt.state.function(canonical.as_bytes()) {
                Some(function) => {
                    let text = describe_function(&function, true);
                    rt.emit(host, Message::Output(text));
                }
                None => return outcome(Next::NextLine, Err(Fail::Error(VimError::new(123, format!("Undefined function: {}", String::from_utf8_lossy(&name)))))),
            }
        }
        return outcome(Next::NextLine, Ok(()));
    }
    // Parameters.
    let mut params = Vec::new();
    let mut varargs = false;
    let mut position = paren + 1;
    let parse_error = |message: String| outcome(Next::NextLine, Err(Fail::Error(VimError::new(125, message))));
    loop {
        position = skip_white(src, position);
        match src.get(position) {
            Some(b')') => {
                position += 1;
                break;
            }
            None => return parse_error("Missing ')'".into()),
            _ => {}
        }
        if src[position..].starts_with(b"...") {
            varargs = true;
            position = skip_white(src, position + 3);
            if src.get(position) != Some(&b')') {
                return parse_error(format!("Illegal argument: {}", String::from_utf8_lossy(&src[position..])));
            }
            continue;
        }
        let start = position;
        while position < src.len() && is_name_char(src[position]) {
            position += 1;
        }
        let param = &src[start..position];
        if param.is_empty() || param[0].is_ascii_digit() || matches!(param, b"firstline" | b"lastline") {
            return parse_error(format!("Illegal argument: {}", String::from_utf8_lossy(&src[start..])));
        }
        if params.iter().any(|p: &Param| p.name.as_bytes() == param) {
            return outcome(Next::NextLine, Err(Fail::Error(VimError::new(853, format!("Duplicate argument name: {}", String::from_utf8_lossy(param))))));
        }
        position = skip_white(src, position);
        let mut default = None;
        if src.get(position) == Some(&b'=') {
            position = skip_white(src, position + 1);
            let mut eval = Eval::new(rt, host, src, position, false);
            if eval.eval1().is_err() {
                return parse_error(format!("Illegal argument: {}", String::from_utf8_lossy(&src[start..])));
            }
            let end = eval.pos;
            default = Some(VStr::from(trim_end(&src[position..end])));
            position = end;
        } else if params.iter().any(|p: &Param| p.default.is_some()) {
            return outcome(Next::NextLine, Err(Fail::Error(VimError::new(989, format!("Non-default argument follows default argument: {}", String::from_utf8_lossy(param))))));
        }
        params.push(Param { name: VStr::from(param), default });
        position = skip_white(src, position);
        if src.get(position) == Some(&b',') {
            position += 1;
        }
    }
    // Attributes.
    let mut flags = FunctionFlags::default();
    loop {
        position = skip_white(src, position);
        let start = position;
        while position < src.len() && src[position].is_ascii_alphabetic() {
            position += 1;
        }
        match &src[start..position] {
            b"" => break,
            b"range" => flags.range = true,
            b"abort" => flags.abort = true,
            b"dict" => flags.dict = true,
            b"closure" => flags.closure = true,
            other => {
                return outcome(Next::NextLine, Err(Fail::Error(VimError::new(488, format!("Trailing characters: {}", String::from_utf8_lossy(other))))));
            }
        }
    }
    let position = skip_white(src, position);
    if position < src.len() && src[position] != b'"' {
        return outcome(Next::NextLine, Err(Fail::Error(VimError::trailing(&src[position..]))));
    }
    // The body follows on the next lines of this frame.
    let current_line = rt.current_frame().map_or(0, |frame| frame.line);
    let interactive = matches!(rt.current_frame().map(|frame| &frame.kind), Some(FrameKind::CommandLine)) && rt.current_frame().is_some_and(|frame| frame.lines.len() == 1);
    let (body, end_index) = match collect_body(rt, current_line) {
        Ok(found) => found,
        Err(error) => {
            let error = if interactive { VimError::plain("Viem cannot define functions interactively; put the definition in a script and :source it") } else { error };
            return outcome(Next::Position(usize::MAX / 2, 0), if executing { Err(Fail::Error(error)) } else { Ok(()) });
        }
    };
    let next = Next::Position(end_index + 1, 0);
    if !executing {
        return outcome(next, Ok(()));
    }
    let def_lnum = rt.current_frame().and_then(|frame| frame.lines.get(current_line)).map_or(0, |line| line.lnum);
    let result = define(rt, host, &name, members, params, varargs, body, flags, bang, def_lnum);
    outcome(next, result)
}

fn trim_end(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().rposition(|b| !is_white(*b)).map_or(0, |end| end + 1);
    &bytes[..end]
}

#[allow(clippy::too_many_arguments)]
fn define(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    name: &[u8],
    members: Vec<VStr>,
    params: Vec<Param>,
    varargs: bool,
    body: Vec<SourceLine>,
    mut flags: FunctionFlags,
    bang: bool,
    def_lnum: usize,
) -> EvalResult<()> {
    let definition = if members.is_empty() {
        let canonical = rt.canonical_function_name(name);
        let bytes = canonical.as_bytes();
        let valid = bytes.first().is_some_and(|b| b.is_ascii_uppercase())
            || bytes.starts_with(b"<SNR>")
            || bytes.contains(&b'#')
            || (name.len() > 2 && name[1] == b':' && name[0] == b's');
        if !valid {
            return Err(Fail::Error(VimError::new(128, format!("Function name must start with a capital or \"s:\": {}", String::from_utf8_lossy(name)))));
        }
        if rt.state.function(bytes).is_some() && !bang {
            return Err(Fail::Error(VimError::new(122, format!("Function {} already exists, add ! to replace it", canonical.to_str_lossy()))));
        }
        // A variable holding a Funcref with the same name would be ambiguous.
        if let Ok(Value::Funcref(_)) = rt.get_variable(host, bytes) {
            if !bytes.starts_with(b"<SNR>") {
                return Err(Fail::Error(VimError::new(707, format!("Function name conflicts with variable: {}", canonical.to_str_lossy()))));
            }
        }
        Definition::Named(canonical)
    } else {
        let mut container = rt.get_variable(host, name)?;
        let (last, path) = members.split_last().unwrap();
        for member in path {
            let Value::Dict(dict) = &container else {
                return Err(Fail::Error(VimError::new(1203, "Dot can only be used on a dictionary")));
            };
            container = dict.get(member.as_bytes()).ok_or_else(|| Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", member.to_str_lossy()))))?;
        }
        let Value::Dict(dict) = container else {
            return Err(Fail::Error(VimError::new(1203, "Dot can only be used on a dictionary")));
        };
        if dict.contains(last.as_bytes()) && !bang {
            return Err(Fail::Error(VimError::new(717, format!("Dictionary entry already exists: {}", last.to_str_lossy()))));
        }
        flags.dict = true;
        Definition::Member { dict, key: last.clone() }
    };
    let outer = if flags.closure { rt.current_scope() } else { None };
    let sid = rt.current_sid();
    match definition {
        Definition::Named(canonical) => {
            rt.state.define_function(UserFunction { name: canonical, params, varargs, body: FunctionBody::Lines(body.into()), flags, sid, def_lnum, outer });
        }
        Definition::Member { dict, key } => {
            // Replacing reuses the existing numbered function name.
            let existing = match dict.get(key.as_bytes()) {
                Some(Value::Funcref(funcref)) if funcref.name().as_bytes().iter().all(u8::is_ascii_digit) => Some(funcref.name().clone()),
                _ => None,
            };
            let numbered = existing.unwrap_or_else(|| rt.state.next_numbered_name());
            rt.state.define_function(UserFunction { name: numbered.clone(), params, varargs, body: FunctionBody::Lines(body.into()), flags, sid, def_lnum, outer });
            if dict.data().lock.is_locked() {
                return Err(Fail::Error(VimError::new(741, "Value is locked")));
            }
            dict.insert(key, Value::Funcref(Funcref::named(numbered)));
        }
    }
    Ok(())
}

pub(crate) fn delfunction(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8], force: bool) -> EvalResult<()> {
    if argument.is_empty() {
        return Err(Fail::Error(VimError::new(471, "Argument required")));
    }
    let mut eval = Eval::new(rt, host, argument, 0, true);
    let Some(name) = eval.parse_name()? else {
        return Err(Fail::Error(VimError::new(129, "Function name required")));
    };
    if eval.peek() == b'.' || eval.peek() == b'[' {
        let mut container = eval.rt.get_variable(eval.host, &name)?;
        let mut last = None;
        while eval.peek() == b'.' || eval.peek() == b'[' {
            let key = if eval.peek() == b'.' {
                eval.pos += 1;
                let start = eval.pos;
                while is_name_char(eval.peek()) {
                    eval.pos += 1;
                }
                VStr::from(&eval.src[start..eval.pos])
            } else {
                eval.pos += 1;
                let key = eval.eval1()?.to_vstr()?;
                eval.skip_white();
                eval.pos += 1;
                key
            };
            if let Some((dict, previous)) = last.take() {
                let _: (super::value::Dict, VStr) = (dict, previous);
            }
            let Value::Dict(dict) = &container else {
                return Err(Fail::Error(VimError::new(1203, "Dot can only be used on a dictionary")));
            };
            let next = dict.get(key.as_bytes());
            last = Some((dict.clone(), key));
            if let Some(next) = next {
                container = next;
            }
        }
        let Some((dict, key)) = last else { return Ok(()) };
        match dict.remove(key.as_bytes()) {
            Some(Value::Funcref(funcref)) => {
                eval.rt.state.delete_function(funcref.name().as_bytes());
                Ok(())
            }
            Some(other) => {
                dict.insert(key, other);
                Err(Fail::Error(VimError::new(130, "Unknown function")))
            }
            None if force => Ok(()),
            None => Err(Fail::Error(VimError::new(130, format!("Unknown function: {}", key.to_str_lossy())))),
        }
    } else {
        let canonical = eval.rt.canonical_function_name(&name);
        if !eval.rt.state.delete_function(canonical.as_bytes()) && !force {
            return Err(Fail::Error(VimError::new(130, format!("Unknown function: {}", canonical.to_str_lossy()))));
        }
        Ok(())
    }
}

fn describe_function(function: &UserFunction, body: bool) -> String {
    let mut params: Vec<String> = function
        .params
        .iter()
        .map(|param| match &param.default {
            Some(default) => format!("{} = {}", param.name.to_str_lossy(), default.to_str_lossy()),
            None => param.name.to_str_lossy().into_owned(),
        })
        .collect();
    if function.varargs {
        params.push("...".into());
    }
    let mut attributes = String::new();
    for (enabled, word) in [(function.flags.range, " range"), (function.flags.abort, " abort"), (function.flags.dict, " dict"), (function.flags.closure, " closure")] {
        if enabled {
            attributes.push_str(word);
        }
    }
    let mut text = format!("   function {}({}){}", function.name.to_str_lossy(), params.join(", "), attributes);
    if body {
        match &function.body {
            FunctionBody::Lines(lines) => {
                for (index, line) in lines.iter().enumerate() {
                    text.push_str(&format!("\n{:<3}{}", index + 1, line.text.to_str_lossy()));
                }
            }
            FunctionBody::Lambda(expression) => text.push_str(&format!("\n1  return {}", expression.to_str_lossy())),
        }
        text.push_str("\n   endfunction");
    }
    text
}

fn list_functions(rt: &mut Runtime, host: &mut dyn ScriptHost, pattern: Option<&[u8]>) {
    let mut lines = Vec::new();
    let mut fuel = 1_000_000u64;
    for name in rt.state.function_names() {
        if name.as_bytes().starts_with(b"<lambda>") || name.as_bytes().iter().all(u8::is_ascii_digit) {
            continue;
        }
        if let Some(pattern) = pattern {
            if !super::regex::string_matches(name.as_bytes(), pattern, false, &mut fuel).unwrap_or(false) {
                continue;
            }
        }
        if let Some(function) = rt.state.function(name.as_bytes()) {
            lines.push(describe_function(&function, false).trim_start().to_string());
        }
    }
    rt.emit(host, Message::Output(lines.join("\n")));
}
