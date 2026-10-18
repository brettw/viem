//! `:let`, `:const`, `:unlet`, `:lockvar` and `:unlockvar`.
use super::error::{EvalResult, Fail, VimError};
use super::exec::{CommandOutcome, Message, Next, OptionScope, Runtime, ScriptHost};
use super::expr::{index_value, is_name_char, is_white, skip_white, Eval};
use super::value::{Blob, Dict, Lock, VStr, Value};

/// An assignment target.
enum Target {
    Variable(Vec<u8>),
    Option(String, OptionScope),
    Register(char),
    Env(String),
    /// A container and the subscript to assign.
    Index { container: Value, key: Value },
    Slice { container: Value, first: Option<Value>, last: Option<Value> },
    DictKey { dict: Dict, key: VStr },
}

fn outcome(next: Next, result: EvalResult<()>) -> CommandOutcome {
    CommandOutcome { next, result }
}

fn after(src: &[u8], at: usize) -> Result<Next, VimError> {
    let at = skip_white(src, at);
    match src.get(at) {
        None => Ok(Next::NextLine),
        Some(b'|') => Ok(Next::Column(at + 1)),
        Some(b'"') => Ok(Next::NextLine),
        Some(_) => Err(VimError::trailing(&src[at..])),
    }
}

/// Parse one target starting at `eval.pos`. With `evaluate == false` only
/// the syntax is checked.
fn parse_target(eval: &mut Eval) -> EvalResult<Option<Target>> {
    eval.skip_white();
    match eval.peek() {
        b'&' => {
            eval.pos += 1;
            let scope = if eval.src[eval.pos..].starts_with(b"l:") {
                eval.pos += 2;
                OptionScope::Local
            } else if eval.src[eval.pos..].starts_with(b"g:") {
                eval.pos += 2;
                OptionScope::Global
            } else {
                OptionScope::Both
            };
            let start = eval.pos;
            if eval.src[start..].starts_with(b"t_") && eval.src.len() >= start + 4 {
                eval.pos += 4;
            } else {
                while eval.peek().is_ascii_alphanumeric() {
                    eval.pos += 1;
                }
            }
            if eval.pos == start {
                return Err(Fail::Error(VimError::new(112, "Option name missing")));
            }
            return Ok(Some(Target::Option(String::from_utf8_lossy(&eval.src[start..eval.pos]).into_owned(), scope)));
        }
        b'@' => {
            eval.pos += 1;
            let text = std::str::from_utf8(&eval.src[eval.pos..]).unwrap_or("");
            let c = text.chars().next().unwrap_or('"');
            eval.pos += c.len_utf8();
            return Ok(Some(Target::Register(if c == '@' { '"' } else { c })));
        }
        b'$' => {
            eval.pos += 1;
            let start = eval.pos;
            while is_name_char(eval.peek()) {
                eval.pos += 1;
            }
            return Ok(Some(Target::Env(String::from_utf8_lossy(&eval.src[start..eval.pos]).into_owned())));
        }
        _ => {}
    }
    let Some(name) = eval.parse_name()? else {
        return Ok(None);
    };
    if !matches!(eval.peek(), b'[' | b'.') || (eval.peek() == b'.' && !is_name_char(eval.src.get(eval.pos + 1).copied().unwrap_or(0))) {
        return Ok(Some(Target::Variable(name)));
    }
    // Subscripts: evaluate all but the last.
    let mut container = if eval.evaluate { Some(eval.rt.get_variable(eval.host, &name)?) } else { None };
    loop {
        if eval.peek() == b'.' && is_name_char(eval.src.get(eval.pos + 1).copied().unwrap_or(0)) {
            eval.pos += 1;
            let start = eval.pos;
            while is_name_char(eval.peek()) {
                eval.pos += 1;
            }
            let key = VStr::from(&eval.src[start..eval.pos]);
            let last = !matches!(eval.peek(), b'[' | b'.');
            let current = container.take();
            if !eval.evaluate {
                if last {
                    return Ok(Some(Target::Variable(Vec::new())));
                }
                continue;
            }
            let Some(Value::Dict(dict)) = current else {
                return Err(Fail::Error(VimError::new(1203, format!("Dot can only be used on a dictionary: {}", String::from_utf8_lossy(&eval.src[start - 1..])))));
            };
            if last {
                return Ok(Some(Target::DictKey { dict, key }));
            }
            container = Some(dict.get(key.as_bytes()).ok_or_else(|| Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy()))))?);
            continue;
        }
        if eval.peek() != b'[' {
            break;
        }
        eval.pos += 1;
        eval.skip_white();
        let first = if eval.peek() == b':' { None } else { Some(eval.eval1()?) };
        eval.skip_white();
        let mut range = false;
        let mut last_index = None;
        if eval.peek() == b':' {
            range = true;
            eval.pos += 1;
            eval.skip_white();
            if eval.peek() != b']' {
                last_index = Some(eval.eval1()?);
                eval.skip_white();
            }
        }
        if eval.peek() != b']' {
            return Err(Fail::Error(VimError::new(111, "Missing ']'")));
        }
        eval.pos += 1;
        let last = !matches!(eval.peek(), b'[') && !(eval.peek() == b'.' && is_name_char(eval.src.get(eval.pos + 1).copied().unwrap_or(0)));
        if !eval.evaluate {
            if last {
                return Ok(Some(Target::Variable(Vec::new())));
            }
            continue;
        }
        let current = container.take().unwrap();
        if last {
            if range {
                return Ok(Some(Target::Slice { container: current, first, last: last_index }));
            }
            return Ok(Some(Target::Index { container: current, key: first.unwrap_or_default() }));
        }
        container = Some(index_value(&current, first.as_ref(), last_index.as_ref(), range)?);
    }
    Ok(Some(Target::Variable(name)))
}

fn parse_operator(src: &[u8], at: usize) -> Option<(&'static str, usize)> {
    let rest = &src[at.min(src.len())..];
    for op in ["..=", "=<<", "+=", "-=", "*=", "/=", "%=", ".=", "="] {
        if rest.starts_with(op.as_bytes()) {
            // "==" is not an assignment.
            if op == "=" && rest.get(1) == Some(&b'=') {
                return None;
            }
            return Some((op, at + op.len()));
        }
    }
    None
}

/// Apply a compound operator.
fn combine(old: Value, op: &str, value: Value) -> EvalResult<Value> {
    Ok(match op {
        "+=" => match (&old, &value) {
            (Value::List(list), Value::List(more)) => {
                list.check_writable()?;
                let more = more.snapshot();
                list.data().items.extend(more);
                old
            }
            (Value::Blob(blob), Value::Blob(more)) => {
                let more = more.bytes();
                blob.data().bytes.extend(more);
                old
            }
            _ => super::expr::arithmetic(&old, &value, b'+')?,
        },
        "-=" => super::expr::arithmetic(&old, &value, b'-')?,
        "*=" => super::expr::arithmetic(&old, &value, b'*')?,
        "/=" => super::expr::arithmetic(&old, &value, b'/')?,
        "%=" => super::expr::arithmetic(&old, &value, b'%')?,
        ".=" | "..=" => {
            if matches!(old, Value::Float(_)) || matches!(value, Value::Float(_)) && false {
                return Err(Fail::Error(VimError::new(734, "Wrong variable type for .=")));
            }
            let mut bytes = old.to_bytes()?;
            bytes.extend(value.to_bytes()?);
            Value::String(bytes.into())
        }
        _ => value,
    })
}

fn current_value(eval: &mut Eval, target: &Target) -> EvalResult<Value> {
    match target {
        Target::Variable(name) => eval.rt.get_variable(eval.host, name),
        Target::Option(name, scope) => eval.host.get_option(name, *scope).map_err(Fail::Error),
        Target::Register(name) => eval.host.get_register(*name).map_err(Fail::Error),
        Target::Env(name) => Ok(Value::String(eval.rt.get_env(eval.host, name).into())),
        Target::Index { container, key } => index_value(container, Some(key), None, false),
        Target::Slice { container, first, last } => index_value(container, first.as_ref(), last.as_ref(), true),
        Target::DictKey { dict, key } => dict.get(key.as_bytes()).ok_or_else(|| Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy())))),
    }
}

fn assign(rt: &mut Runtime, host: &mut dyn ScriptHost, target: Target, value: Value, constant: bool) -> EvalResult<()> {
    match target {
        Target::Variable(name) => {
            if constant {
                if let Ok((dict, key, _)) = rt.variable_slot(host, &name) {
                    if dict.contains(&key) {
                        return Err(Fail::Error(VimError::new(995, format!("Cannot modify existing variable: {}", String::from_utf8_lossy(&name)))));
                    }
                }
                lock_value(&value, 2, true)?;
            }
            rt.set_variable(host, &name, value, constant)
        }
        Target::Option(name, scope) => host.set_option(rt, &name, scope, &value),
        Target::Register(name) => host.set_register(name, &value, false).map_err(Fail::Error),
        Target::Env(name) => {
            let text = value.to_text()?;
            host.setenv(&name, Some(&text)).map_err(Fail::Error)
        }
        Target::Index { container, key } => match container {
            Value::List(list) => {
                list.check_writable()?;
                let mut data = list.data();
                let len = data.items.len() as i64;
                let mut index = key.to_number()?;
                if index < 0 {
                    index += len;
                }
                if index < 0 || index > len {
                    return Err(Fail::Error(VimError::new(684, format!("List index out of range: {}", key.to_number()?))));
                }
                if index == len {
                    data.items.push(value);
                } else {
                    data.items[index as usize] = value;
                }
                Ok(())
            }
            Value::Dict(dict) => {
                let key = key.to_vstr()?;
                assign_dict(&dict, key, value)
            }
            Value::Blob(blob) => {
                let mut data = blob.data();
                let len = data.bytes.len() as i64;
                let mut index = key.to_number()?;
                if index < 0 {
                    index += len;
                }
                let byte = value.to_number()?;
                if !(0..=255).contains(&byte) {
                    return Err(Fail::Error(VimError::new(1239, format!("Invalid value for blob: {byte}"))));
                }
                if index == len {
                    data.bytes.push(byte as u8);
                } else if (0..len).contains(&index) {
                    data.bytes[index as usize] = byte as u8;
                } else {
                    return Err(Fail::Error(VimError::new(979, format!("Blob index out of range: {index}"))));
                }
                Ok(())
            }
            _ => Err(Fail::Error(VimError::new(689, "Can only index a List, Dictionary or Blob"))),
        },
        Target::Slice { container, first, last } => {
            let Value::List(list) = container else {
                if let Value::Blob(blob) = container {
                    let Value::Blob(source) = value else {
                        return Err(Fail::Error(VimError::new(972, "Blob value does not have the right number of bytes")));
                    };
                    let source = source.bytes();
                    let mut data = blob.data();
                    let len = data.bytes.len() as i64;
                    let start = first.map(|f| f.to_number()).transpose()?.unwrap_or(0);
                    let end = last.map(|l| l.to_number()).transpose()?.unwrap_or(len - 1);
                    if start < 0 || end >= len || end - start + 1 != source.len() as i64 {
                        return Err(Fail::Error(VimError::new(972, "Blob value does not have the right number of bytes")));
                    }
                    data.bytes[start as usize..=end as usize].copy_from_slice(&source);
                    return Ok(());
                }
                return Err(Fail::Error(VimError::new(689, "Can only index a List, Dictionary or Blob")));
            };
            let Value::List(source) = value else {
                return Err(Fail::Error(VimError::new(709, "[:] requires a List or Blob value")));
            };
            list.check_writable()?;
            let source = source.snapshot();
            let mut data = list.data();
            let len = data.items.len() as i64;
            let mut start = first.map(|f| f.to_number()).transpose()?.unwrap_or(0);
            if start < 0 {
                start += len;
            }
            if start < 0 || start > len {
                return Err(Fail::Error(VimError::new(684, format!("List index out of range: {start}"))));
            }
            let end = match last {
                Some(last) => {
                    let mut end = last.to_number()?;
                    if end < 0 {
                        end += len;
                    }
                    Some(end)
                }
                None => None,
            };
            let start = start as usize;
            match end {
                Some(end) => {
                    let count = (end - start as i64 + 1).max(0) as usize;
                    if source.len() < count {
                        return Err(Fail::Error(VimError::new(711, "List value does not have enough items")));
                    }
                    if source.len() > count {
                        return Err(Fail::Error(VimError::new(710, "List value has too many items")));
                    }
                    for (offset, item) in source.into_iter().enumerate() {
                        if start + offset < data.items.len() {
                            data.items[start + offset] = item;
                        }
                    }
                }
                None => {
                    for (offset, item) in source.into_iter().enumerate() {
                        if start + offset < data.items.len() {
                            data.items[start + offset] = item;
                        } else {
                            data.items.push(item);
                        }
                    }
                }
            }
            Ok(())
        }
        Target::DictKey { dict, key } => assign_dict(&dict, key, value),
    }
}

fn assign_dict(dict: &Dict, key: VStr, value: Value) -> EvalResult<()> {
    let mut data = dict.data();
    match data.items.get_mut(&key) {
        Some(item) => {
            if item.lock.is_locked() {
                return Err(Fail::Error(VimError::new(741, format!("Value is locked: {}", key.to_str_lossy()))));
            }
            item.value = value;
        }
        None => {
            if data.lock.is_locked() {
                return Err(Fail::Error(VimError::new(741, "Value is locked")));
            }
            data.items.insert(key, super::value::DictItem::new(value));
        }
    }
    Ok(())
}

/// Lock (or unlock) a value's containers to `depth` levels.
pub(crate) fn lock_value(value: &Value, depth: i64, lock: bool) -> Result<(), VimError> {
    lock_value_inner(value, depth, lock, depth == i64::MAX,
        &mut std::collections::HashMap::new(), &mut std::collections::HashSet::new())
}

fn lock_value_inner(value: &Value, depth: i64, lock: bool, recursive: bool,
    visited: &mut std::collections::HashMap<(bool, usize), i64>,
    active: &mut std::collections::HashSet<(bool, usize)>) -> Result<(), VimError> {
    if depth <= 0 { return Ok(()); }
    let identity = match value {
        Value::List(list) => Some((false, list.id())),
        Value::Dict(dict) => Some((true, dict.id())),
        _ => None,
    };
    if let Some(identity) = identity {
        if active.len() >= 100 || recursive && active.contains(&identity) {
            return Err(VimError::new(743, "Variable nested too deep for (un)lock"));
        }
        if visited.get(&identity).is_some_and(|previous| *previous >= depth) { return Ok(()); }
        visited.insert(identity, depth);
        active.insert(identity);
    }
    let state = if lock { Lock::Locked } else { Lock::Unlocked };
    match value {
        Value::List(list) => {
            let items = {
                let mut data = list.data();
                if data.lock != Lock::Fixed { data.lock = state; }
                data.items.clone()
            };
            for item in items {
                lock_value_inner(&item, depth - 1, lock, recursive, visited, active)?;
            }
        }
        Value::Dict(dict) => {
            let values = {
                let mut data = dict.data();
                if data.lock != Lock::Fixed { data.lock = state; }
                for item in data.items.values_mut() {
                    if item.lock != Lock::Fixed && depth > 1 { item.lock = state; }
                }
                data.items.values().map(|item| item.value.clone()).collect::<Vec<_>>()
            };
            for item in values {
                lock_value_inner(&item, depth - 1, lock, recursive, visited, active)?;
            }
        }
        Value::Blob(blob) => {
            let mut data = blob.data();
            if data.lock != Lock::Fixed { data.lock = state; }
        }
        _ => {}
    }
    if let Some(identity) = identity { active.remove(&identity); }
    Ok(())
}

/// Lines of a `=<<` heredoc after the current line; returns them and the
/// position after the end marker.
fn heredoc(rt: &Runtime, src: &[u8], at: usize) -> Result<(Vec<Vec<u8>>, usize, bool, bool), VimError> {
    let mut words = String::from_utf8_lossy(&src[at..]).split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let mut trim = false;
    let mut eval = false;
    while let Some(first) = words.first() {
        if first == "trim" {
            trim = true;
            words.remove(0);
        } else if first == "eval" {
            eval = true;
            words.remove(0);
        } else {
            break;
        }
    }
    let Some(marker) = words.first().cloned() else {
        return Err(VimError::new(172, "Missing marker"));
    };
    if marker.starts_with('"') {
        return Err(VimError::new(172, "Missing marker"));
    }
    let frame = rt.current_frame().expect("let runs in a frame");
    let mut lines = Vec::new();
    let mut index = frame.line + 1;
    // With "trim", indentation of the :let line is removed from the marker.
    let let_indent: Vec<u8> = frame.lines[frame.line].text.as_bytes().iter().take_while(|b| is_white(**b)).copied().collect();
    while index < frame.lines.len() {
        let text = frame.lines[index].text.as_bytes();
        let candidate = if trim { text.strip_prefix(let_indent.as_slice()).unwrap_or(text) } else { text };
        let candidate = if trim { &candidate[candidate.iter().take_while(|b| is_white(**b)).count()..] } else { candidate };
        if candidate == marker.as_bytes() {
            break;
        }
        lines.push(text.to_vec());
        index += 1;
    }
    if index >= frame.lines.len() {
        return Err(VimError::new(990, format!("Missing end marker '{marker}'")));
    }
    if trim {
        let indent = lines
            .iter()
            .find(|line| !line.is_empty())
            .map(|line| line.iter().take_while(|b| is_white(**b)).count())
            .unwrap_or(0);
        for line in &mut lines {
            let strip = line.iter().take(indent).take_while(|b| is_white(**b)).count();
            line.drain(..strip);
        }
    }
    Ok((lines, index, trim, eval))
}

pub(crate) fn skip_let(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> Next {
    let mut eval = Eval::new(rt, host, src, at, false);
    let start = eval.pos;
    let parsed = (|| -> EvalResult<Option<usize>> {
        eval.skip_white();
        if eval.peek() == b'[' {
            // List unpacking.
            while !eval.at_end() && eval.peek() != b']' {
                eval.pos += 1;
            }
            eval.pos += 1;
        } else if parse_target(&mut eval)?.is_none() {
            return Ok(None);
        }
        eval.skip_white();
        let Some((op, after_op)) = parse_operator(eval.src, eval.pos) else { return Ok(None) };
        if op == "=<<" {
            return Ok(Some(usize::MAX - after_op));
        }
        eval.pos = after_op;
        eval.skip_white();
        eval.eval1()?;
        Ok(Some(eval.pos))
    })();
    let _ = start;
    match parsed {
        Ok(Some(end)) if end > src.len() => {
            let after_op = usize::MAX - end;
            match heredoc(rt, src, after_op) {
                Ok((_, index, _, _)) => Next::Position(index + 1, 0),
                Err(_) => Next::NextLine,
            }
        }
        Ok(Some(end)) => after(src, end).unwrap_or(Next::NextLine),
        _ => Next::NextLine,
    }
}

pub(crate) fn let_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, constant: bool) -> CommandOutcome {
    let at = skip_white(src, at);
    if at >= src.len() || src[at] == b'|' || src[at] == b'"' {
        let next = after(src, at).unwrap_or(Next::NextLine);
        list_variables(rt, host, &[]);
        return outcome(next, Ok(()));
    }
    let mut eval = Eval::new(rt, host, src, at, true);
    // List unpacking: [a, b; rest]
    if eval.peek() == b'[' {
        eval.pos += 1;
        let mut targets = Vec::new();
        let mut rest = None;
        loop {
            eval.skip_white();
            if eval.peek() == b']' {
                eval.pos += 1;
                break;
            }
            if eval.peek() == b';' {
                eval.pos += 1;
                match parse_target(&mut eval) {
                    Ok(Some(target)) => rest = Some(target),
                    Ok(None) => return outcome(Next::NextLine, Err(Fail::Error(VimError::new(475, "Invalid argument")))),
                    Err(fail) => return outcome(Next::NextLine, Err(fail)),
                }
                continue;
            }
            match parse_target(&mut eval) {
                Ok(Some(target)) => targets.push(target),
                Ok(None) => return outcome(Next::NextLine, Err(Fail::Error(VimError::new(475, format!("Invalid argument: {}", String::from_utf8_lossy(&src[at..])))))),
                Err(fail) => return outcome(Next::NextLine, Err(fail)),
            }
            eval.skip_white();
            if eval.peek() == b',' {
                eval.pos += 1;
            }
        }
        eval.skip_white();
        let Some((op, after_op)) = parse_operator(src, eval.pos) else {
            return outcome(Next::NextLine, Err(Fail::Error(VimError::new(15, format!("Invalid expression: \"{}\"", String::from_utf8_lossy(&src[eval.pos..]))))));
        };
        eval.pos = after_op;
        eval.skip_white();
        let value = eval.eval1();
        let end = eval.pos;
        let next = match after(src, end) {
            Ok(next) => next,
            Err(err) => return outcome(Next::NextLine, Err(Fail::Error(err))),
        };
        let value = match value {
            Ok(value) => value,
            Err(fail) => return outcome(next, Err(fail)),
        };
        let Value::List(list) = value else {
            return outcome(next, Err(Fail::Error(VimError::new(714, "List required"))));
        };
        let items = list.snapshot();
        if items.len() < targets.len() {
            return outcome(next, Err(Fail::Error(VimError::new(688, "More targets than List items"))));
        }
        if items.len() > targets.len() && rest.is_none() {
            return outcome(next, Err(Fail::Error(VimError::new(687, "Less targets than List items"))));
        }
        let result = (|| {
            let count = targets.len();
            for (target, item) in targets.into_iter().zip(items.iter().cloned()) {
                let value = if op == "=" {
                    item
                } else {
                    let mut eval = Eval::new(rt, host, src, 0, true);
                    combine(current_value(&mut eval, &target)?, op, item)?
                };
                assign(rt, host, target, value, constant)?;
            }
            if let Some(rest) = rest {
                assign(rt, host, rest, Value::list(items[count..].to_vec()), constant)?;
            }
            Ok(())
        })();
        return outcome(next, result);
    }
    let target = match parse_target(&mut eval) {
        Ok(Some(target)) => target,
        Ok(None) => return outcome(Next::NextLine, Err(Fail::Error(VimError::new(475, format!("Invalid argument: {}", String::from_utf8_lossy(&src[at..])))))),
        Err(fail) => return outcome(Next::NextLine, Err(fail)),
    };
    eval.skip_white();
    let Some((op, after_op)) = parse_operator(src, eval.pos) else {
        // `:let var ...` lists variables.
        if eval.at_end() || matches!(eval.peek(), b'|' | b'"') || is_name_char(eval.peek()) {
            let names: Vec<Vec<u8>> = String::from_utf8_lossy(&src[at..])
                .split('|')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(|name| name.as_bytes().to_vec())
                .collect();
            let next = after(src, eval.pos).unwrap_or(Next::NextLine);
            list_variables(rt, host, &names);
            return outcome(next, Ok(()));
        }
        return outcome(Next::NextLine, Err(Fail::Error(VimError::new(15, format!("Invalid expression: \"{}\"", String::from_utf8_lossy(&src[eval.pos..]))))));
    };
    if op == "=<<" {
        return let_heredoc(rt, host, src, after_op, target, constant);
    }
    eval.pos = after_op;
    eval.skip_white();
    if eval.at_end() {
        return outcome(Next::NextLine, Err(Fail::Error(VimError::new(15, "Invalid expression: \"\""))));
    }
    let value = eval.eval1();
    let end = eval.pos;
    let value = match value {
        Ok(value) => value,
        Err(fail) => return outcome(super::commands::next_after_failure(src, end), Err(fail)),
    };
    let next = match after(src, end) {
        Ok(next) => next,
        Err(err) => return outcome(Next::NextLine, Err(Fail::Error(err))),
    };
    let result = (|| {
        let value = if op == "=" {
            value
        } else {
            let mut eval = Eval::new(rt, host, src, 0, true);
            let old = current_value(&mut eval, &target)?;
            combine(old, op, value)?
        };
        assign(rt, host, target, value, constant)
    })();
    outcome(next, result)
}

fn let_heredoc(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, target: Target, constant: bool) -> CommandOutcome {
    let (lines, end_index, _trim, eval_lines) = match heredoc(rt, src, at) {
        Ok(parsed) => parsed,
        Err(err) => return outcome(Next::NextLine, Err(Fail::Error(err))),
    };
    let next = Next::Position(end_index + 1, 0);
    let result = (|| {
        let mut items = Vec::new();
        for line in lines {
            if eval_lines {
                let mut eval = Eval::new(rt, host, &line, 0, true);
                items.push(eval.interpolate_heredoc()?);
            } else {
                items.push(Value::String(line.into()));
            }
        }
        assign(rt, host, target, Value::list(items), constant)
    })();
    outcome(next, result)
}

fn list_variables(rt: &mut Runtime, host: &mut dyn ScriptHost, names: &[Vec<u8>]) {
    let mut lines = Vec::new();
    let describe = |name: &str, value: &Value| -> String {
        let marker = match value {
            Value::Number(_) | Value::Bool(_) => '#',
            Value::Funcref(_) => '*',
            _ => ' ',
        };
        format!("{name:<24}{marker}{}", value.echo_string())
    };
    if names.is_empty() {
        for (prefix, dict) in [("", rt.state.globals().clone()), ("b:", rt.state.buffer_vars().clone()), ("w:", rt.state.window_vars(host.view_id())), ("t:", rt.state.tab_vars().clone()), ("v:", rt.state.vim_vars().clone())] {
            for (key, value) in dict.snapshot() {
                lines.push(describe(&format!("{prefix}{}", key.to_str_lossy()), &value));
            }
        }
    } else {
        for name in names {
            match rt.get_variable(host, name) {
                Ok(value) => lines.push(describe(&String::from_utf8_lossy(name), &value)),
                Err(_) => {
                    let error = VimError::undefined_variable(name);
                    rt.report(host, error);
                }
            }
        }
    }
    rt.emit(host, Message::Output(lines.join("\n")));
}

pub(crate) fn unlet_command(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8], force: bool) -> EvalResult<()> {
    if argument.is_empty() {
        return Err(Fail::Error(VimError::new(471, "Argument required")));
    }
    let mut at = 0;
    while at < argument.len() {
        at = skip_white(argument, at);
        if at >= argument.len() {
            break;
        }
        let start = at;
        let mut eval = Eval::new(rt, host, argument, at, true);
        let target = parse_target(&mut eval)?;
        at = eval.pos;
        let Some(target) = target else {
            return Err(Fail::Error(VimError::new(475, format!("Invalid argument: {}", String::from_utf8_lossy(&argument[start..])))));
        };
        let missing = |name: &[u8]| Fail::Error(VimError::new(108, format!("No such variable: \"{}\"", String::from_utf8_lossy(name))));
        match target {
            Target::Variable(name) => {
                if name.len() >= 2 && name[1] == b':' && matches!(name[0], b'a' | b'v') {
                    return Err(Fail::Error(VimError::new(795, format!("Cannot delete variable {}", String::from_utf8_lossy(&name)))));
                }
                let (dict, key, _) = rt.variable_slot(host, &name)?;
                let mut data = dict.data();
                match data.items.get(key.as_slice()) {
                    // lockvar protects the value, not the variable's binding.
                    // Vim permits unlet of a locked variable; :const and v:
                    // bindings remain fixed.
                    Some(item) if item.lock == Lock::Fixed => return Err(Fail::Error(VimError::new(741, format!("Value is locked: {}", String::from_utf8_lossy(&name))))),
                    Some(_) => {
                        data.items.remove(key.as_slice());
                    }
                    None if force => {}
                    None => return Err(missing(&name)),
                }
            }
            Target::Env(name) => host.setenv(&name, None).map_err(Fail::Error)?,
            Target::DictKey { dict, key } => {
                if dict.remove(key.as_bytes()).is_none() && !force {
                    return Err(Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy()))));
                }
            }
            Target::Index { container, key } => match container {
                Value::List(list) => {
                    list.check_writable()?;
                    let mut data = list.data();
                    let len = data.items.len() as i64;
                    let mut index = key.to_number()?;
                    if index < 0 {
                        index += len;
                    }
                    if index < 0 || index >= len {
                        return Err(Fail::Error(VimError::new(684, format!("List index out of range: {}", key.to_number()?))));
                    }
                    data.items.remove(index as usize);
                }
                Value::Dict(dict) => {
                    let key = key.to_vstr()?;
                    if dict.remove(key.as_bytes()).is_none() && !force {
                        return Err(Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy()))));
                    }
                }
                Value::Blob(blob) => {
                    let mut data = blob.data();
                    let len = data.bytes.len() as i64;
                    let mut index = key.to_number()?;
                    if index < 0 {
                        index += len;
                    }
                    if index < 0 || index >= len {
                        return Err(Fail::Error(VimError::new(979, format!("Blob index out of range: {index}"))));
                    }
                    data.bytes.remove(index as usize);
                }
                _ => return Err(Fail::Error(VimError::new(689, "Can only index a List, Dictionary or Blob"))),
            },
            Target::Slice { container, first, last } => {
                let Value::List(list) = container else {
                    if let Value::Blob(blob) = container {
                        let mut data = blob.data();
                        let len = data.bytes.len() as i64;
                        let start = first.map(|f| f.to_number()).transpose()?.unwrap_or(0).max(0) as usize;
                        let end = last.map(|l| l.to_number()).transpose()?.unwrap_or(len - 1);
                        let end = if end < 0 { end + len } else { end }.min(len - 1);
                        if (start as i64) <= end {
                            data.bytes.drain(start..=end as usize);
                        }
                        let _ = Blob::default();
                        continue;
                    }
                    return Err(Fail::Error(VimError::new(689, "Can only index a List, Dictionary or Blob")));
                };
                list.check_writable()?;
                let mut data = list.data();
                let len = data.items.len() as i64;
                let mut start = first.map(|f| f.to_number()).transpose()?.unwrap_or(0);
                if start < 0 {
                    start += len;
                }
                let mut end = last.map(|l| l.to_number()).transpose()?.unwrap_or(len - 1);
                if end < 0 {
                    end += len;
                }
                let end = end.min(len - 1);
                if start >= 0 && start <= end {
                    data.items.drain(start as usize..=end as usize);
                }
            }
            Target::Option(..) | Target::Register(_) => {
                return Err(Fail::Error(VimError::new(475, format!("Invalid argument: {}", String::from_utf8_lossy(&argument[start..])))));
            }
        }
    }
    Ok(())
}

pub(crate) fn lock_command(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8], lock: bool, force: bool) -> EvalResult<()> {
    let mut at = skip_white(argument, 0);
    let mut depth = 2i64;
    if argument.get(at).is_some_and(u8::is_ascii_digit) {
        let start = at;
        while argument.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        depth = String::from_utf8_lossy(&argument[start..at]).parse().unwrap_or(2);
    }
    if force {
        depth = i64::MAX;
    }
    while at < argument.len() {
        at = skip_white(argument, at);
        if at >= argument.len() {
            break;
        }
        let mut eval = Eval::new(rt, host, argument, at, true);
        let target = parse_target(&mut eval)?;
        at = eval.pos;
        let Some(Target::Variable(name)) = target else {
            continue;
        };
        let (dict, key, _) = rt.variable_slot(host, &name)?;
        let value = {
            let mut data = dict.data();
            let Some(item) = data.items.get_mut(key.as_slice()) else {
                return Err(Fail::Error(VimError::new(108, format!("No such variable: \"{}\"", String::from_utf8_lossy(&name)))));
            };
            if item.lock != Lock::Fixed {
                item.lock = if lock { Lock::Locked } else { Lock::Unlocked };
            }
            item.value.clone()
        };
        lock_value(&value, depth, lock)?;
    }
    Ok(())
}
