//! Builtin functions. Pure functions live here; editor functions such as
//! `getline()` are provided by the host. Functions Vim has but Viem does not
//! implement report "Unknown function ... (not supported by Viem)".
use super::error::{EvalResult, Fail, VimError};
use super::exec::{Runtime, ScriptHost};
use super::value::{VStr, Value};

mod collections;
mod script;
mod system;
mod text;

// Bound individual generated containers before allocating their contents.
pub(crate) const MAX_LIST_ITEMS: usize = 1_000_000;
pub(crate) const MAX_STRING_BYTES: usize = 64 * 1024 * 1024;

pub(crate) mod signatures;

pub fn is_vim_builtin(name: &str) -> bool {
    signatures::contains(name)
}

/// Base argument position for `base->name()` method calls (0-based).
pub fn method_base_position(name: &[u8]) -> Result<usize, VimError> {
    signatures::method(&String::from_utf8_lossy(name))
}

pub(crate) fn arity(name: &str, args: &[Value], min: usize, max: usize) -> EvalResult<()> {
    if args.len() < min {
        return Err(Fail::Error(VimError::argument(119, name)));
    }
    if args.len() > max {
        return Err(Fail::Error(VimError::argument(118, name)));
    }
    Ok(())
}

pub(crate) fn number(args: &[Value], index: usize) -> EvalResult<i64> {
    match args.get(index) {
        Some(value) => Ok(value.to_number()?),
        None => Ok(0),
    }
}

pub(crate) fn opt_number(args: &[Value], index: usize, default: i64) -> EvalResult<i64> {
    match args.get(index) {
        Some(value) => Ok(value.to_number()?),
        None => Ok(default),
    }
}

pub(crate) fn vstr(args: &[Value], index: usize) -> EvalResult<VStr> {
    match args.get(index) {
        Some(value) => Ok(value.to_vstr()?),
        None => Ok(VStr::empty()),
    }
}

pub(crate) fn text(args: &[Value], index: usize) -> EvalResult<String> {
    Ok(vstr(args, index)?.to_str_lossy().into_owned())
}

pub(crate) fn opt_bool(args: &[Value], index: usize, default: bool) -> EvalResult<bool> {
    match args.get(index) {
        Some(value) => Ok(value.to_bool()?),
        None => Ok(default),
    }
}

/// Call a callback given as a Funcref or a function name.
pub(crate) fn call_callback(rt: &mut Runtime, host: &mut dyn ScriptHost, callback: &Value, args: Vec<Value>) -> EvalResult<Value> {
    match callback {
        Value::Funcref(funcref) => rt.call_funcref(host, funcref, args, None),
        Value::String(name) => {
            let funcref = rt.resolve_function_name(host, name.as_bytes())?;
            rt.call_funcref(host, &funcref, args, None)
        }
        _ => Err(Fail::Error(VimError::new(921, "Invalid callback argument"))),
    }
}

/// Like Vim, an error in a builtin's callback is reported at once and the
/// builtin stops early but still returns its result. Inside `:try` the
/// error propagates to become an exception.
pub(crate) fn callback_failed(rt: &mut Runtime, host: &mut dyn ScriptHost, fail: Fail) -> EvalResult<()> {
    match fail {
        Fail::Error(error) if rt.trylevel == 0 && rt.emsg_silent == 0 => {
            rt.report(host, error);
            Ok(())
        }
        Fail::Error(error) if rt.emsg_silent > 0 => {
            rt.state.set_vvar("errmsg", error.text().into());
            rt.did_emsg = true;
            Ok(())
        }
        other => Err(other),
    }
}

/// Dispatch an engine builtin. None means `name` is not one.
pub(crate) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, mut args: Vec<Value>) -> Option<EvalResult<Value>> {
    if args.iter().any(|v|matches!(v, Value::Void)) {return Some(Err(VimError::new(1031, "Cannot use void value").into()));}
    // Null strings and tuples retain identity for comparisons, but read-only
    // builtin operations see their empty contents.
    if !matches!(name, "type" | "typename" | "string" | "copy" | "deepcopy" | "islocked" | "id") {
        for value in &mut args {
            if let Value::NullTyped(ty) = value {
                match ty.as_ref() {
                    super::types::Type::String => *value = Value::str(""),
                    super::types::Type::Tuple(..) => *value = Value::Tuple(std::sync::Arc::new(vec![])),
                    _ => {},
                }
            }
        }
    }
    if let Some(result) = text::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = collections::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = script::call(rt, host, name, &args) {
        return Some(result);
    }
    if let Some(result) = system::call(rt, host, name, &args) {
        return Some(result);
    }
    None
}

/// Whether the engine implements `name` (for `exists('*name')`).
pub fn is_implemented(name: &str) -> bool {
    text::NAMES.contains(&name) || collections::NAMES.contains(&name) || script::NAMES.contains(&name) || system::NAMES.contains(&name)
}
