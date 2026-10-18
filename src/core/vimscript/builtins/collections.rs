//! List, Dictionary and Blob builtins.
use super::{arity, call_callback, callback_failed, number, opt_bool, opt_number, text, vstr};
use crate::vimscript::error::{EvalResult, Fail, VimError};
use crate::vimscript::exec::{Runtime, ScriptHost};
use crate::vimscript::value::{compare_bytes, values_equal, Blob, Dict, List, Lock, VStr, Value};

pub(super) const NAMES: &[&str] = &[
    "add", "insert", "remove", "extend", "extendnew", "filter", "map", "mapnew", "foreach", "reduce",
    "sort", "uniq", "reverse", "index", "indexof", "count", "join", "split", "range", "min", "max",
    "flatten", "flattennew", "slice", "keys", "values", "items", "has_key", "get", "len", "empty",
    "copy", "deepcopy", "list2str", "str2list", "list2blob", "blob2list", "islocked", "id",
    "garbagecollect", "type", "typename",
];

fn list_required(index: usize) -> Fail {
    Fail::Error(VimError::new(1211, format!("List required for argument {}", index + 1)))
}

pub(super) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
    Some(match name {
        "add" => arity(name, args, 2, 2).and_then(|()| match &args[0] {
            Value::List(list) => {
                list.check_argument(name)?;
                list.push(args[1].clone());
                Ok(args[0].clone())
            }
            Value::Blob(blob) => {
                let byte = args[1].to_number()?;
                blob.data().bytes.push(byte as u8);
                Ok(args[0].clone())
            }
            _ => Err(Fail::Error(VimError::new(897, "List or Blob required"))),
        }),
        "insert" => arity(name, args, 2, 3).and_then(|()| match &args[0] {
            Value::List(list) => {
                list.check_argument(name)?;
                let len = list.len() as i64;
                let mut index = opt_number(args, 2, 0)?;
                if index < 0 {
                    index += len;
                }
                if index < 0 || index > len {
                    return Err(Fail::Error(VimError::new(684, format!("List index out of range: {}", opt_number(args, 2, 0)?))));
                }
                list.data().items.insert(index as usize, args[1].clone());
                Ok(args[0].clone())
            }
            Value::Blob(blob) => {
                let len = blob.len() as i64;
                let index = opt_number(args, 2, 0)?;
                if index < 0 || index > len {
                    return Err(Fail::Error(VimError::new(475, "Invalid argument")));
                }
                blob.data().bytes.insert(index as usize, args[1].to_number()? as u8);
                Ok(args[0].clone())
            }
            _ => Err(Fail::Error(VimError::new(899, "Argument of insert() must be a List or Blob"))),
        }),
        "remove" => arity(name, args, 2, 3).and_then(|()| remove(args)),
        "extend" | "extendnew" => arity(name, args, 2, 3).and_then(|()| extend(args, name == "extendnew")),
        "filter" | "map" | "mapnew" | "foreach" => arity(name, args, 2, 2).and_then(|()| map_filter(rt, host, name, args)),
        "reduce" => arity(name, args, 2, 3).and_then(|()| {
            let items: Vec<Value> = match &args[0] {
                Value::List(list) => list.snapshot(),
                Value::Blob(blob) => blob.bytes().into_iter().map(|b| Value::Number(b as i64)).collect(),
                Value::String(text) => super::text::char_units(text, true).into_iter().map(Value::from).collect(),
                _ => return Err(Fail::Error(VimError::new(1098, "String, List or Blob required"))),
            };
            let mut items = items.into_iter();
            let mut accumulator = match args.get(2) {
                Some(initial) => initial.clone(),
                None => items.next().ok_or_else(|| Fail::Error(VimError::new(998, "Reduce of an empty List with no initial value")))?,
            };
            for item in items {
                accumulator = call_callback(rt, host, &args[1], vec![accumulator, item])?;
            }
            Ok(accumulator)
        }),
        "sort" | "uniq" => arity(name, args, 1, 3).and_then(|()| sort_uniq(rt, host, name, args)),
        "reverse" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::List(list) => {
                list.check_writable()?;
                list.data().items.reverse();
                Ok(args[0].clone())
            }
            Value::Blob(blob) => {
                blob.data().bytes.reverse();
                Ok(args[0].clone())
            }
            Value::String(text) => Ok(Value::String(super::text::char_units(text, true).into_iter().rev().collect::<String>().into())),
            _ => Ok(Value::Number(0)),
        }),
        "index" => arity(name, args, 2, 4).and_then(|()| {
            let ignore_case = opt_bool(args, 3, false)?;
            match &args[0] {
                Value::List(list) => {
                    let items = list.snapshot();
                    let len = items.len() as i64;
                    let mut start = opt_number(args, 2, 0)?;
                    if start < 0 {
                        start = (start + len).max(0);
                    }
                    for (index, item) in items.iter().enumerate().skip(start as usize) {
                        if values_equal(item, &args[1], ignore_case, 0)? {
                            return Ok(Value::Number(index as i64));
                        }
                    }
                    Ok(Value::Number(-1))
                }
                Value::Blob(blob) => {
                    let byte = args[1].to_number()?;
                    let start = opt_number(args, 2, 0)?.max(0) as usize;
                    Ok(Value::Number(blob.bytes().iter().enumerate().skip(start).find(|(_, b)| **b as i64 == byte).map_or(-1, |(i, _)| i as i64)))
                }
                _ => Err(Fail::Error(VimError::new(897, "List or Blob required"))),
            }
        }),
        "indexof" => arity(name, args, 2, 3).and_then(|()| {
            let Value::List(list) = &args[0] else { return Ok(Value::Number(-1)) };
            let start = match args.get(2) {
                Some(Value::Dict(options)) => options.get_str("startidx").map(|v| v.to_number()).transpose()?.unwrap_or(0),
                _ => 0,
            };
            let items = list.snapshot();
            let start = if start < 0 { (start + items.len() as i64).max(0) } else { start } as usize;
            for (index, item) in items.iter().enumerate().skip(start) {
                let keep = match &args[1] {
                    Value::String(expression) => {
                        rt.state.set_vvar("key", (index as i64).into());
                        rt.state.set_vvar("val", item.clone());
                        let sid = rt.current_sid();
                        rt.eval_expression(host, expression.as_bytes(), sid)?.to_bool()?
                    }
                    callback => call_callback(rt, host, callback, vec![(index as i64).into(), item.clone()])?.to_bool()?,
                };
                if keep {
                    return Ok(Value::Number(index as i64));
                }
            }
            Ok(Value::Number(-1))
        }),
        "count" => arity(name, args, 2, 4).and_then(|()| {
            let ignore_case = opt_bool(args, 2, false)?;
            match &args[0] {
                Value::List(list) => {
                    let start = opt_number(args, 3, 0)?;
                    let items = list.snapshot();
                    let start = if start < 0 { (start + items.len() as i64).max(0) } else { start } as usize;
                    let mut count = 0;
                    for item in items.iter().skip(start) {
                        if values_equal(item, &args[1], ignore_case, 0)? {
                            count += 1;
                        }
                    }
                    Ok(Value::Number(count))
                }
                Value::Dict(dict) => {
                    let mut count = 0;
                    for (_, item) in dict.snapshot() {
                        if values_equal(&item, &args[1], ignore_case, 0)? {
                            count += 1;
                        }
                    }
                    Ok(Value::Number(count))
                }
                Value::String(text) => {
                    let needle = args[1].to_vstr()?;
                    if needle.is_empty() {
                        return Ok(Value::Number(0));
                    }
                    let (haystack, needle) = if ignore_case {
                        (text.to_str_lossy().to_lowercase(), needle.to_str_lossy().to_lowercase())
                    } else {
                        (text.to_str_lossy().into_owned(), needle.to_str_lossy().into_owned())
                    };
                    Ok(Value::Number(haystack.matches(needle.as_str()).count() as i64))
                }
                _ => Err(Fail::Error(VimError::new(706, "Argument of count() must be a List or Dictionary"))),
            }
        }),
        "join" => arity(name, args, 1, 2).and_then(|()| {
            let Value::List(list) = &args[0] else { return Err(Fail::Error(VimError::new(714, "List required"))) };
            let separator = match args.get(1) {
                Some(value) => value.to_bytes()?,
                None => b" ".to_vec(),
            };
            let mut out = Vec::new();
            for (index, item) in list.snapshot().iter().enumerate() {
                if index > 0 {
                    out.extend_from_slice(&separator);
                }
                match item {
                    Value::List(_) | Value::Dict(_) | Value::Funcref(_) | Value::Blob(_) => out.extend(item.string_bytes()),
                    other => out.extend(other.to_bytes()?),
                }
            }
            Ok(Value::String(out.into()))
        }),
        "split" => arity(name, args, 1, 3).and_then(|()| split(rt, args)),
        "range" => arity(name, args, 1, 3).and_then(|()| {
            let (start, end) = match args.get(1) {
                Some(end) => (i128::from(number(args, 0)?), i128::from(end.to_number()?)),
                None => (0, i128::from(number(args, 0)?) - 1),
            };
            let stride = i128::from(opt_number(args, 2, 1)?);
            if stride == 0 {
                return Err(Fail::Error(VimError::new(726, "Stride is zero")));
            }
            if (stride > 0 && end + 1 < start) || (stride < 0 && end > start + 1) {
                return Err(Fail::Error(VimError::new(727, "Start past end")));
            }
            let count = if (stride > 0 && start > end) || (stride < 0 && start < end) {
                0
            } else if stride > 0 {
                (end - start) / stride + 1
            } else {
                (start - end) / -stride + 1
            };
            if count > super::MAX_LIST_ITEMS as i128 {
                return Err(Fail::Interrupt("range() result too large".into()));
            }
            rt.charge(count as u64)?;
            let mut items = Vec::with_capacity(count as usize);
            for index in 0..count {
                // Widened arithmetic also handles i64::MIN strides and endpoints.
                items.push(Value::Number((start + index * stride) as i64));
            }
            Ok(Value::list(items))
        }),
        "min" | "max" => arity(name, args, 1, 1).and_then(|()| {
            let items: Vec<Value> = match &args[0] {
                Value::List(list) => list.snapshot(),
                Value::Dict(dict) => dict.snapshot().into_iter().map(|(_, v)| v).collect(),
                Value::String(text) if text.is_empty() => Vec::new(),
                _ => return Err(Fail::Error(VimError::new(712, format!("Argument of {name}() must be a List or Dictionary")))),
            };
            let mut best: Option<i64> = None;
            for item in items {
                let value = item.to_number()?;
                best = Some(match best {
                    None => value,
                    Some(current) if name == "min" => current.min(value),
                    Some(current) => current.max(value),
                });
            }
            Ok(Value::Number(best.unwrap_or(0)))
        }),
        "flatten" | "flattennew" => arity(name, args, 1, 2).and_then(|()| {
            let Value::List(list) = &args[0] else { return Err(list_required(0)) };
            let depth = opt_number(args, 1, 999_999)?;
            if depth < 0 {
                return Err(Fail::Error(VimError::new(900, "maxdepth must be non-negative number")));
            }
            let flat = flatten(rt, list, depth)?;
            if name == "flatten" {
                list.check_writable()?;
                list.data().items = flat;
                Ok(args[0].clone())
            } else {
                Ok(Value::list(flat))
            }
        }),
        "slice" => arity(name, args, 2, 3).and_then(|()| {
            let start = number(args, 1)?;
            match &args[0] {
                Value::List(list) => {
                    let items = list.snapshot();
                    let (start, end) = slice_bounds(start, args.get(2), items.len())?;
                    Ok(Value::list(items[start..end].to_vec()))
                }
                Value::Blob(blob) => {
                    let bytes = blob.bytes();
                    let (start, end) = slice_bounds(start, args.get(2), bytes.len())?;
                    Ok(Value::Blob(Blob::new(bytes[start..end].to_vec())))
                }
                other => {
                    let units = super::text::char_units(&other.to_vstr()?, false);
                    let (start, end) = slice_bounds(start, args.get(2), units.len())?;
                    Ok(Value::String(units[start..end].concat().into()))
                }
            }
        }),
        "keys" | "values" | "items" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::Dict(dict) => Ok(Value::list(
                dict.snapshot()
                    .into_iter()
                    .map(|(key, value)| match name {
                        "keys" => Value::String(key),
                        "values" => value,
                        _ => Value::list(vec![Value::String(key), value]),
                    })
                    .collect(),
            )),
            Value::List(list) if name == "items" => Ok(Value::list(list.snapshot().into_iter().enumerate().map(|(index, value)| Value::list(vec![(index as i64).into(), value])).collect())),
            Value::String(text) if name == "items" => Ok(Value::list(
                super::text::char_units(text, true).into_iter().enumerate().map(|(index, c)| Value::list(vec![(index as i64).into(), c.into()])).collect(),
            )),
            _ => Err(Fail::Error(VimError::new(1206, "Dictionary required for argument 1"))),
        }),
        "has_key" => arity(name, args, 2, 2).and_then(|()| match &args[0] {
            Value::Dict(dict) => Ok(Value::Number(dict.contains(args[1].to_vstr()?.as_bytes()) as i64)),
            _ => Err(Fail::Error(VimError::new(1206, "Dictionary required for argument 1"))),
        }),
        "get" => arity(name, args, 2, 3).and_then(|()| {
            let default = args.get(2).cloned().unwrap_or(Value::Number(0));
            match &args[0] {
                Value::List(list) => {
                    let len = list.len() as i64;
                    let mut index = args[1].to_number()?;
                    if index < 0 {
                        index += len;
                    }
                    Ok(if (0..len).contains(&index) { list.get(index as usize).unwrap() } else { default })
                }
                Value::Blob(blob) => {
                    let bytes = blob.bytes();
                    let mut index = args[1].to_number()?;
                    if index < 0 {
                        index += bytes.len() as i64;
                    }
                    Ok(bytes.get(index as usize).map_or(default, |b| Value::Number(*b as i64)))
                }
                Value::Dict(dict) => Ok(dict.get(args[1].to_vstr()?.as_bytes()).unwrap_or(default)),
                Value::Funcref(funcref) => {
                    let what = args[1].to_text()?;
                    Ok(match what.as_str() {
                        "name" => Value::String(funcref.name().clone()),
                        "func" => Value::Funcref(crate::vimscript::value::Funcref::named(funcref.name().clone())),
                        "dict" => funcref.dict().map_or(Value::Number(0), |dict| Value::Dict(dict.clone())),
                        "args" => Value::list(funcref.args().to_vec()),
                        "arity" => {
                            let function = rt.state.function(funcref.name().as_bytes());
                            let (required, optional, varargs) = function.map_or((0, 0, false), |f| {
                                let required = f.params.iter().filter(|p| p.default.is_none()).count();
                                (required, f.params.len() - required, f.varargs)
                            });
                            Value::Dict(Dict::from_pairs([
                                ("required".into(), (required as i64).into()),
                                ("optional".into(), (optional as i64).into()),
                                ("varargs".into(), Value::Bool(varargs)),
                            ]))
                        }
                        _ => return Err(Fail::Error(VimError::new(475, format!("Invalid argument: {what}")))),
                    })
                }
                _ => Err(Fail::Error(VimError::new(896, "Argument of get() must be a List, Dictionary or Blob"))),
            }
        }),
        "len" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::List(list) => Ok(Value::Number(list.len() as i64)),
            Value::Dict(dict) => Ok(Value::Number(dict.len() as i64)),
            Value::Blob(blob) => Ok(Value::Number(blob.len() as i64)),
            Value::String(text) => Ok(Value::Number(text.len() as i64)),
            Value::Number(number) => Ok(Value::Number(number.to_string().len() as i64)),
            _ => Err(Fail::Error(VimError::new(701, "Invalid type for len()"))),
        }),
        "empty" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(args[0].is_empty_value() as i64))),
        "copy" => arity(name, args, 1, 1).and_then(|()| Ok(args[0].copy(false))),
        "deepcopy" => arity(name, args, 1, 2).and_then(|()| Ok(args[0].copy(true))),
        "list2str" => arity(name, args, 1, 2).and_then(|()| {
            let Value::List(list) = &args[0] else { return Err(list_required(0)) };
            let mut out = String::new();
            for item in list.snapshot() {
                if let Some(c) = char::from_u32(item.to_number()? as u32) {
                    if c != '\0' {
                        out.push(c);
                    }
                }
            }
            Ok(Value::String(out.into()))
        }),
        "str2list" => arity(name, args, 1, 2).and_then(|()| Ok(Value::list(text(args, 0)?.chars().map(|c| Value::Number(c as i64)).collect()))),
        "list2blob" => arity(name, args, 1, 1).and_then(|()| {
            let Value::List(list) = &args[0] else { return Err(list_required(0)) };
            let mut bytes = Vec::new();
            for item in list.snapshot() {
                let value = item.to_number()?;
                if !(0..=255).contains(&value) {
                    return Err(Fail::Error(VimError::new(1051, format!("Invalid argument: {value}"))));
                }
                bytes.push(value as u8);
            }
            Ok(Value::Blob(Blob::new(bytes)))
        }),
        "blob2list" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::Blob(blob) => Ok(Value::list(blob.bytes().into_iter().map(|b| Value::Number(b as i64)).collect())),
            _ => Err(Fail::Error(VimError::new(1238, "Blob required for argument 1"))),
        }),
        "islocked" => arity(name, args, 1, 1).and_then(|()| {
            let name = vstr(args, 0)?;
            let (dict, key, _) = rt.variable_slot(host, name.as_bytes())?;
            let data = dict.data();
            let Some(item) = data.items.get(key.as_slice()) else {
                return Err(Fail::Error(VimError::new(108, format!("No such variable: \"{}\"", name.to_str_lossy()))));
            };
            let locked = item.lock.is_locked()
                || match &item.value {
                    Value::List(list) => list.data().lock.is_locked(),
                    Value::Dict(dict) => dict.data().lock.is_locked(),
                    _ => false,
                };
            Ok(Value::Number(locked as i64))
        }),
        "id" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(match &args[0] {
            Value::List(list) => format!("0x{:x}", list.id()),
            Value::Dict(dict) => format!("0x{:x}", dict.id()),
            _ => String::new(),
        }.into()))),
        "garbagecollect" => arity(name, args, 0, 1).map(|()| Value::Number(0)),
        "type" => arity(name, args, 1, 1).map(|()| Value::Number(args[0].type_number())),
        "typename" => arity(name, args, 1, 1).map(|()| Value::String(typename(&args[0]).into())),
        _ => return None,
    })
}

fn typename(value: &Value) -> String {
    match value {
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Float(_) => "float".into(),
        Value::Bool(_) => "bool".into(),
        Value::None | Value::Null => "special".into(),
        Value::Blob(_) => "blob".into(),
        Value::Funcref(_) => "func".into(),
        Value::List(list) => {
            let items = list.snapshot();
            let mut kinds: Vec<String> = items.iter().map(typename).collect();
            kinds.dedup();
            match kinds.as_slice() {
                [] => "list<any>".into(),
                [one] => format!("list<{one}>"),
                _ => "list<any>".into(),
            }
        }
        Value::Dict(dict) => {
            let mut kinds: Vec<String> = dict.snapshot().iter().map(|(_, value)| typename(value)).collect();
            kinds.dedup();
            match kinds.as_slice() {
                [] => "dict<any>".into(),
                [one] => format!("dict<{one}>"),
                _ => "dict<any>".into(),
            }
        }
    }
}

fn slice_bounds(start: i64, end: Option<&Value>, len: usize) -> EvalResult<(usize, usize)> {
    let len_i = len as i64;
    let start = if start < 0 { (start + len_i).max(0) } else { start.min(len_i) } as usize;
    let end = match end {
        Some(end) => {
            let end = end.to_number()?;
            (if end < 0 { (end + len_i).max(0) } else { end.min(len_i) }) as usize
        }
        None => len,
    };
    Ok((start, end.max(start)))
}

fn flatten(rt: &mut Runtime, list: &List, depth: i64) -> EvalResult<Vec<Value>> {
    // An explicit stack prevents nested/cyclic Lists from consuming the Rust stack.
    // Read one item at a time rather than retaining a snapshot of each ancestor.
    let mut stack = vec![(list.clone(), 0usize, depth)];
    let mut active = std::collections::HashSet::from([list.id()]);
    let mut out = Vec::new();
    while let Some((list, index, depth)) = stack.last_mut() {
        rt.charge(1)?;
        let Some(item) = list.get(*index) else {
            active.remove(&list.id());
            stack.pop();
            continue;
        };
        *index += 1;
        match item {
            Value::List(child) if *depth > 0 => {
                let next_depth = *depth - 1;
                if stack.len() >= 256 || !active.insert(child.id()) {
                    return Err(Fail::Interrupt("flatten() nesting or cycle limit exceeded".into()));
                }
                stack.push((child, 0, next_depth));
            }
            other => {
                if out.len() >= super::MAX_LIST_ITEMS {
                    return Err(Fail::Interrupt("flatten() result too large".into()));
                }
                out.push(other);
            }
        }
    }
    Ok(out)
}

fn remove(args: &[Value]) -> EvalResult<Value> {
    match &args[0] {
        Value::List(list) => {
            list.check_writable()?;
            let len = list.len() as i64;
            let mut start = args[1].to_number()?;
            if start < 0 {
                start += len;
            }
            if start < 0 || start >= len {
                return Err(Fail::Error(VimError::new(684, format!("List index out of range: {}", args[1].to_number()?))));
            }
            match args.get(2) {
                None => Ok(list.data().items.remove(start as usize)),
                Some(end) => {
                    let mut end = end.to_number()?;
                    if end < 0 {
                        end += len;
                    }
                    if end < start || end >= len {
                        return Err(Fail::Error(VimError::new(16, "Invalid range")));
                    }
                    let removed: Vec<Value> = list.data().items.drain(start as usize..=end as usize).collect();
                    Ok(Value::list(removed))
                }
            }
        }
        Value::Dict(dict) => {
            if args.len() > 2 {
                return Err(Fail::Error(VimError::argument(118, "remove")));
            }
            if dict.data().lock.is_locked() {
                return Err(Fail::Error(VimError::new(741, "Value is locked")));
            }
            let key = args[1].to_vstr()?;
            dict.remove(key.as_bytes()).ok_or_else(|| Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy()))))
        }
        Value::Blob(blob) => {
            let len = blob.len() as i64;
            let mut start = args[1].to_number()?;
            if start < 0 {
                start += len;
            }
            if start < 0 || start >= len {
                return Err(Fail::Error(VimError::new(979, format!("Blob index out of range: {start}"))));
            }
            match args.get(2) {
                None => Ok(Value::Number(blob.data().bytes.remove(start as usize) as i64)),
                Some(end) => {
                    let mut end = end.to_number()?;
                    if end < 0 {
                        end += len;
                    }
                    let removed: Vec<u8> = blob.data().bytes.drain(start as usize..=(end.min(len - 1)) as usize).collect();
                    Ok(Value::Blob(Blob::new(removed)))
                }
            }
        }
        _ => Err(Fail::Error(VimError::new(896, "Argument of remove() must be a List, Dictionary or Blob"))),
    }
}

fn extend(args: &[Value], new: bool) -> EvalResult<Value> {
    match (&args[0], &args[1]) {
        (Value::List(target), Value::List(source)) => {
            let target = if new { List::new(target.snapshot()) } else { target.clone() };
            target.check_writable()?;
            let source = source.snapshot();
            let len = target.len() as i64;
            let index = match args.get(2) {
                Some(index) => {
                    let mut index = index.to_number()?;
                    if index < 0 {
                        index += len;
                    }
                    if index < 0 || index > len {
                        return Err(Fail::Error(VimError::new(684, format!("List index out of range: {index}"))));
                    }
                    index as usize
                }
                None => len as usize,
            };
            let mut data = target.data();
            let tail = data.items.split_off(index);
            data.items.extend(source);
            data.items.extend(tail);
            drop(data);
            Ok(Value::List(target))
        }
        (Value::Dict(target), Value::Dict(source)) => {
            let target = if new { Dict::from_pairs(target.snapshot()) } else { target.clone() };
            let how = match args.get(2) {
                Some(how) => how.to_text()?,
                None => "force".into(),
            };
            if !matches!(how.as_str(), "force" | "keep" | "error") {
                return Err(Fail::Error(VimError::new(475, format!("Invalid argument: {how}"))));
            }
            if target.data().lock.is_locked() {
                return Err(Fail::Error(VimError::new(741, "Value is locked")));
            }
            for (key, value) in source.snapshot() {
                if target.contains(key.as_bytes()) {
                    match how.as_str() {
                        "keep" => continue,
                        "error" => return Err(Fail::Error(VimError::new(737, format!("Key already exists: {}", key.to_str_lossy())))),
                        _ => {}
                    }
                }
                target.insert(key, value);
            }
            Ok(Value::Dict(target))
        }
        (Value::Blob(target), Value::Blob(source)) => {
            let target = if new { Blob::new(target.bytes()) } else { target.clone() };
            let more = source.bytes();
            target.data().bytes.extend(more);
            Ok(Value::Blob(target))
        }
        _ => Err(Fail::Error(VimError::new(712, "Argument of extend() must be a List or Dictionary"))),
    }
}

/// Evaluate a map()/filter() callback (expression string or Funcref).
fn apply_callback(rt: &mut Runtime, host: &mut dyn ScriptHost, callback: &Value, key: Value, value: Value) -> EvalResult<Value> {
    match callback {
        Value::String(expression) => {
            let saved = (rt.state.vvar("key"), rt.state.vvar("val"));
            rt.state.set_vvar("key", key);
            rt.state.set_vvar("val", value);
            let sid = rt.current_sid();
            let result = rt.eval_expression(host, expression.as_bytes(), sid);
            rt.state.set_vvar("key", saved.0);
            rt.state.set_vvar("val", saved.1);
            result
        }
        other => call_callback(rt, host, other, vec![key, value]),
    }
}

fn map_filter(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> EvalResult<Value> {
    let callback = &args[1];
    match &args[0] {
        Value::List(list) => {
            if name != "mapnew" && name != "foreach" {
                list.check_writable()?;
            }
            let items = list.snapshot();
            let mut results = Vec::with_capacity(items.len());
            let total = items.len();
            for (index, item) in items.iter().cloned().enumerate() {
                let result = match apply_callback(rt, host, callback, (index as i64).into(), item.clone()) {
                    Ok(result) => result,
                    Err(fail) => {
                        callback_failed(rt, host, fail)?;
                        // Keep the remaining items unchanged.
                        if !matches!(name, "foreach") {
                            if name == "filter" {
                                results.extend(items[index..total].iter().cloned());
                            } else {
                                results.extend(items[index..total].iter().cloned());
                            }
                        }
                        break;
                    }
                };
                match name {
                    "filter" => {
                        if result.to_bool()? {
                            results.push(item);
                        }
                    }
                    "foreach" => {}
                    _ => results.push(result),
                }
            }
            match name {
                "mapnew" => Ok(Value::list(results)),
                "foreach" => Ok(args[0].clone()),
                _ => {
                    list.data().items = results;
                    Ok(args[0].clone())
                }
            }
        }
        Value::Dict(dict) => {
            let target = if name == "mapnew" { Dict::new() } else { dict.clone() };
            for (key, value) in dict.snapshot() {
                let result = match apply_callback(rt, host, callback, Value::String(key.clone()), value.clone()) {
                    Ok(result) => result,
                    Err(fail) => {
                        callback_failed(rt, host, fail)?;
                        break;
                    }
                };
                match name {
                    "filter" => {
                        if !result.to_bool()? {
                            dict.remove(key.as_bytes());
                        }
                    }
                    "foreach" => {}
                    _ => target.insert(key, result),
                }
            }
            Ok(if name == "mapnew" { Value::Dict(target) } else { args[0].clone() })
        }
        Value::String(text) => {
            let mut out = String::new();
            for (index, unit) in super::text::char_units(text, true).into_iter().enumerate() {
                let result = apply_callback(rt, host, callback, (index as i64).into(), unit.clone().into())?;
                match name {
                    "filter" => {
                        if result.to_bool()? {
                            out.push_str(&unit);
                        }
                    }
                    "foreach" => {}
                    _ => out.push_str(&result.to_text()?),
                }
            }
            Ok(if name == "foreach" { args[0].clone() } else { Value::String(out.into()) })
        }
        Value::Blob(blob) => {
            let bytes = blob.bytes();
            let mut out = Vec::new();
            for (index, byte) in bytes.into_iter().enumerate() {
                let result = apply_callback(rt, host, callback, (index as i64).into(), (byte as i64).into())?;
                match name {
                    "filter" => {
                        if result.to_bool()? {
                            out.push(byte);
                        }
                    }
                    "foreach" => {}
                    _ => out.push(result.to_number()? as u8),
                }
            }
            match name {
                "mapnew" => Ok(Value::Blob(Blob::new(out))),
                "foreach" => Ok(args[0].clone()),
                _ => {
                    blob.data().bytes = out;
                    Ok(args[0].clone())
                }
            }
        }
        _ => Err(Fail::Error(VimError::new(1250, format!("Argument of {name}() must be a List, String, Dictionary or Blob")))),
    }
}

fn sort_uniq(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> EvalResult<Value> {
    let Value::List(list) = &args[0] else {
        return Err(Fail::Error(VimError::new(686, format!("Argument of {name}() must be a List"))));
    };
    list.check_writable()?;
    let mut items = list.snapshot();
    #[derive(Clone, Copy)]
    enum How {
        String { ignore_case: bool },
        Numeric,
        NumericStrings,
        Float,
    }
    let mut callback = None;
    let how = match args.get(1) {
        None => How::String { ignore_case: false },
        Some(Value::Number(1)) => How::String { ignore_case: true },
        Some(Value::Number(0)) => How::String { ignore_case: false },
        Some(Value::String(text)) => match text.as_bytes() {
            b"" => How::String { ignore_case: false },
            b"i" => How::String { ignore_case: true },
            b"l" => How::String { ignore_case: false },
            b"n" => How::Numeric,
            b"N" => How::NumericStrings,
            b"f" => How::Float,
            _ => {
                callback = Some(args[1].clone());
                How::String { ignore_case: false }
            }
        },
        Some(Value::Funcref(_)) => {
            callback = Some(args[1].clone());
            How::String { ignore_case: false }
        }
        Some(_) => return Err(Fail::Error(VimError::new(474, "Invalid argument"))),
    };
    let self_dict = match args.get(2) {
        Some(Value::Dict(dict)) => Some(dict.clone()),
        Some(_) => return Err(Fail::Error(VimError::new(715, "Dictionary required"))),
        None => None,
    };
    let mut failure: Option<Fail> = None;
    let mut compare = |rt: &mut Runtime, host: &mut dyn ScriptHost, a: &Value, b: &Value| -> std::cmp::Ordering {
        if failure.is_some() {
            return std::cmp::Ordering::Equal;
        }
        if let Some(callback) = &callback {
            let result = match callback {
                Value::Funcref(funcref) => rt.call_funcref(host, funcref, vec![a.clone(), b.clone()], self_dict.clone()),
                Value::String(name) => match rt.resolve_function_name(host, name.as_bytes()) {
                    Ok(funcref) => rt.call_funcref(host, &funcref, vec![a.clone(), b.clone()], self_dict.clone()),
                    Err(fail) => Err(fail),
                },
                _ => Ok(Value::Number(0)),
            };
            return match result.and_then(|value| value.to_number().map_err(Fail::Error)) {
                Ok(value) => value.cmp(&0),
                Err(fail) => {
                    failure = Some(fail);
                    std::cmp::Ordering::Equal
                }
            };
        }
        match how {
            How::String { ignore_case } => {
                let text = |value: &Value| match value {
                    Value::String(text) => text.as_bytes().to_vec(),
                    other => other.string_bytes(),
                };
                compare_bytes(&text(a), &text(b), ignore_case)
            }
            How::Numeric => {
                let number = |value: &Value| match value {
                    Value::Number(n) => *n,
                    Value::Float(f) => *f as i64,
                    Value::Bool(b) => *b as i64,
                    _ => 0,
                };
                number(a).cmp(&number(b))
            }
            How::NumericStrings => {
                let number = |value: &Value| value.to_number().unwrap_or(0);
                number(a).cmp(&number(b))
            }
            How::Float => {
                let float = |value: &Value| value.to_float().unwrap_or(0.0);
                float(a).partial_cmp(&float(b)).unwrap_or(std::cmp::Ordering::Equal)
            }
        }
    };
    if name == "sort" {
        // Stable merge sort calling back into the runtime.
        let mut sorted: Vec<Value> = Vec::with_capacity(items.len());
        for item in items.drain(..) {
            let position = {
                let mut low = 0;
                let mut high = sorted.len();
                while low < high {
                    let middle = (low + high) / 2;
                    if compare(rt, host, &sorted[middle], &item) == std::cmp::Ordering::Greater {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                low
            };
            sorted.insert(position, item);
            if sorted.len() % 64 == 0 {
                rt.charge(sorted.len() as u64)?;
            }
        }
        if let Some(fail) = failure {
            return Err(fail);
        }
        list.data().items = sorted;
    } else {
        let mut unique: Vec<Value> = Vec::new();
        for item in items {
            if let Some(last) = unique.last() {
                if compare(rt, host, last, &item) == std::cmp::Ordering::Equal && (callback.is_some() || values_equal(last, &item, matches!(how, How::String { ignore_case: true }), 0).unwrap_or(false) || !matches!(how, How::String { .. })) {
                    continue;
                }
            }
            unique.push(item);
        }
        if let Some(fail) = failure {
            return Err(fail);
        }
        list.data().items = unique;
    }
    let _ = Lock::Unlocked;
    Ok(args[0].clone())
}

fn split(rt: &mut Runtime, args: &[Value]) -> EvalResult<Value> {
    let source = vstr(args, 0)?;
    let text = source.to_str_lossy().into_owned();
    let keep_empty = opt_bool(args, 2, false)?;
    let pattern = match args.get(1) {
        Some(value) => value.to_vstr()?,
        None => VStr::from("\\s\\+"),
    };
    if pattern.is_empty() {
        return Ok(Value::list(text.chars().map(|c| Value::String(c.to_string().into())).collect()));
    }
    let compiled = crate::vimscript::regex::compile(pattern.as_bytes(), false)?;
    // Vim's f_split(): an empty piece is kept only between items, and an
    // empty match never matches twice at the same position.
    let mut items: Vec<Value> = Vec::new();
    let mut start = 0usize;
    let mut skip = 0usize;
    while start < text.len() || keep_empty {
        let found = if start >= text.len() {
            None
        } else {
            crate::vimscript::regex::find_at(&compiled, &text, start + skip, &mut rt.fuel)?
        };
        let end = found.as_ref().map_or(text.len(), |found| found.start);
        let nonempty_match = found.as_ref().is_some_and(|found| found.end > found.start);
        if keep_empty || end > start || (!items.is_empty() && start < text.len() && found.is_some() && nonempty_match) {
            items.push(Value::String(text[start..end].into()));
        }
        let Some(found) = found else { break };
        skip = if found.end > start { 0 } else { text[found.end..].chars().next().map_or(1, char::len_utf8) };
        start = found.end;
        if start >= text.len() && !keep_empty {
            break;
        }
    }
    Ok(Value::list(items))
}
