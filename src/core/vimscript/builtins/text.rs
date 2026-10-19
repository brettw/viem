//! String, pattern, number and math builtins.
use super::{arity, call_callback, number, opt_bool, opt_number, text, vstr};
use crate::vimscript::error::{EvalResult, Fail, VimError};
use crate::vimscript::exec::{OptionScope, Runtime, ScriptHost};
use crate::vimscript::regex;
use crate::vimscript::value::{Blob, Dict, VStr, Value};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const NAMES: &[&str] = &[
    "strlen", "strchars", "strcharlen", "strwidth", "strdisplaywidth", "strpart", "strcharpart",
    "strgetchar", "stridx", "strridx", "strtrans", "tolower", "toupper", "tr", "trim", "escape",
    "shellescape", "fnameescape", "substitute", "match", "matchend", "matchstr", "matchstrpos",
    "matchlist", "printf", "nr2char", "char2nr", "byteidx", "byteidxcomp", "charidx", "utf16idx",
    "strutf16len", "iconv", "keytrans", "str2nr", "str2float", "float2nr", "submatch", "charclass",
    "json_encode", "json_decode", "js_encode", "js_decode", "sha256", "abs", "ceil", "floor", "round",
    "trunc", "sqrt", "pow", "exp", "log", "log10", "sin", "cos", "tan", "asin", "acos", "atan",
    "atan2", "sinh", "cosh", "tanh", "fmod", "isinf", "isnan", "and", "or", "xor", "invert", "rand",
    "srand", "repeat", "string", "base64_encode", "base64_decode", "blob2str", "str2blob",
];

pub(super) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
    Some(match name {
        "strlen" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(args[0].to_vstr()?.len() as i64))),
        "strchars" => arity(name, args, 1, 2).and_then(|()| {
            let skip = opt_bool(args, 1, false)?;
            Ok(Value::Number(char_units(&vstr(args, 0)?, skip).len() as i64))
        }),
        "strcharlen" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(char_units(&vstr(args, 0)?, true).len() as i64))),
        "strwidth" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(display_width(&text(args, 0)?, 0, 8) as i64))),
        "strdisplaywidth" => arity(name, args, 1, 2).and_then(|()| {
            let tabstop = host.get_option("tabstop", OptionScope::Both).ok().and_then(|v| v.to_number().ok()).unwrap_or(8).max(1);
            let column = opt_number(args, 1, 0)?.max(0) as usize;
            Ok(Value::Number(display_width(&text(args, 0)?, column, tabstop as usize) as i64))
        }),
        "strpart" => arity(name, args, 2, 4).and_then(|()| strpart(args)),
        "strcharpart" => arity(name, args, 2, 4).and_then(|()| {
            let units = char_units(&vstr(args, 0)?, opt_bool(args, 3, false)?);
            let mut start = number(args, 1)?;
            let mut len = match args.get(2) {
                Some(value) => value.to_number()?,
                None => units.len() as i64,
            };
            if start < 0 {
                len += start;
                start = 0;
            }
            let start = (start as usize).min(units.len());
            let end = (start + len.max(0) as usize).min(units.len());
            Ok(Value::String(units[start..end].concat().into()))
        }),
        "strgetchar" => arity(name, args, 2, 2).and_then(|()| {
            let index = number(args, 1)?;
            let text = text(args, 0)?;
            Ok(Value::Number(if index < 0 { -1 } else { text.chars().nth(index as usize).map_or(-1, |c| c as i64) }))
        }),
        "stridx" => arity(name, args, 2, 3).and_then(|()| {
            let (haystack, needle) = (vstr(args, 0)?, vstr(args, 1)?);
            let start = opt_number(args, 2, 0)?.max(0) as usize;
            if start > haystack.len() {
                return Ok(Value::Number(-1));
            }
            Ok(Value::Number(find_bytes(&haystack.as_bytes()[start..], needle.as_bytes()).map_or(-1, |at| (at + start) as i64)))
        }),
        "strridx" => arity(name, args, 2, 3).and_then(|()| {
            let (haystack, needle) = (vstr(args, 0)?, vstr(args, 1)?);
            let limit = match args.get(2) {
                Some(value) => {
                    let start = value.to_number()?;
                    if start < 0 {
                        return Ok(Value::Number(-1));
                    }
                    start as usize
                }
                None => haystack.len(),
            };
            let bytes = haystack.as_bytes();
            let needle = needle.as_bytes();
            let mut found = -1i64;
            let mut at = 0;
            while at + needle.len() <= bytes.len() && at <= limit {
                if &bytes[at..at + needle.len()] == needle {
                    found = at as i64;
                }
                at += 1;
            }
            Ok(Value::Number(found))
        }),
        "strtrans" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(strtrans(vstr(args, 0)?.as_bytes()).into()))),
        "tolower" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(text(args, 0)?.to_lowercase().into()))),
        "toupper" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(text(args, 0)?.to_uppercase().into()))),
        "tr" => arity(name, args, 3, 3).and_then(|()| {
            let (source, from, to) = (text(args, 0)?, text(args, 1)?, text(args, 2)?);
            let from: Vec<char> = from.chars().collect();
            let to: Vec<char> = to.chars().collect();
            if from.len() != to.len() {
                return Err(Fail::Error(VimError::new(475, format!("Invalid argument: {}", args[1].echo_string()))));
            }
            Ok(Value::String(source.chars().map(|c| from.iter().position(|f| *f == c).map_or(c, |i| to[i])).collect::<String>().into()))
        }),
        "trim" => arity(name, args, 1, 3).and_then(|()| {
            let source = text(args, 0)?;
            let mask: Vec<char> = match args.get(1) {
                Some(value) if !value.to_vstr()?.is_empty() => value.to_text()?.chars().collect(),
                _ => (0u8..=0x20).map(|b| b as char).chain(['\u{a0}', '\u{3000}']).collect(),
            };
            let direction = opt_number(args, 2, 0)?;
            let trimmed = match direction {
                0 => source.trim_matches(|c| mask.contains(&c)),
                1 => source.trim_start_matches(|c| mask.contains(&c)),
                2 => source.trim_end_matches(|c| mask.contains(&c)),
                _ => return Err(Fail::Error(VimError::new(475, "Invalid argument"))),
            };
            Ok(Value::String(trimmed.into()))
        }),
        "escape" => arity(name, args, 2, 2).and_then(|()| {
            let (source, chars) = (vstr(args, 0)?, text(args, 1)?);
            let mut out = Vec::new();
            for c in source.to_str_lossy().chars() {
                if chars.contains(c) {
                    out.push(b'\\');
                }
                let mut buffer = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
            }
            Ok(Value::String(out.into()))
        }),
        "shellescape" => arity(name, args, 1, 2).and_then(|()| {
            let source = text(args, 0)?;
            let special = opt_bool(args, 1, false)?;
            let mut escaped = if cfg!(windows) {
                format!("\"{}\"", source.replace('"', "\"\""))
            } else {
                format!("'{}'", source.replace('\'', "'\\''"))
            };
            if special {
                escaped = escaped.replace('!', "\\!").replace('%', "\\%").replace('#', "\\#");
            }
            Ok(Value::String(escaped.into()))
        }),
        "fnameescape" => arity(name, args, 1, 1).and_then(|()| {
            let source = text(args, 0)?;
            let mut out = String::new();
            for (index, c) in source.chars().enumerate() {
                if " \t\n*?[{`$\\%#'\"|!<".contains(c) || (index == 0 && (c == '+' || c == '>')) {
                    out.push('\\');
                }
                if index == 0 && c == '-' && source.len() > 1 {
                    out.push_str("\\");
                }
                out.push(c);
            }
            Ok(Value::String(out.into()))
        }),
        "substitute" => arity(name, args, 4, 4).and_then(|()| substitute(rt, host, args)),
        "match" | "matchend" | "matchstr" | "matchstrpos" | "matchlist" => arity(name, args, 2, 4).and_then(|()| match_function(rt, host, name, args)),
        "printf" => {
            if args.is_empty() {
                Err(Fail::Error(VimError::argument(119, name)))
            } else {
                vstr(args, 0).and_then(|spec| crate::vimscript::printf::format(spec.as_bytes(), &args[1..]).map(|bytes| Value::String(bytes.into())))
            }
        }
        "nr2char" => arity(name, args, 1, 2).and_then(|()| {
            let code = number(args, 0)?;
            if code <= 0 {
                return Ok(Value::str(""));
            }
            Ok(Value::String(char::from_u32(code as u32).map(|c| c.to_string()).unwrap_or_default().into()))
        }),
        "char2nr" => arity(name, args, 1, 2).and_then(|()| {
            let source = vstr(args, 0)?;
            Ok(Value::Number(match source.as_str() {
                Some(text) => text.chars().next().map_or(0, |c| c as i64),
                None => source.as_bytes().first().map_or(0, |b| *b as i64),
            }))
        }),
        "byteidx" | "byteidxcomp" => arity(name, args, 2, 3).and_then(|()| {
            let source = vstr(args, 0)?;
            let index = number(args, 1)?;
            let utf16 = opt_bool(args, 2, false)?;
            if index < 0 {
                return Ok(Value::Number(-1));
            }
            let text = source.to_str_lossy();
            let units: Vec<usize> = if utf16 {
                let mut offsets = Vec::new();
                for (offset, c) in text.char_indices() {
                    offsets.push(offset);
                    if c.len_utf16() == 2 {
                        offsets.push(offset);
                    }
                }
                offsets
            } else if name == "byteidx" {
                char_unit_offsets(&text, true)
            } else {
                text.char_indices().map(|(offset, _)| offset).collect()
            };
            let index = index as usize;
            Ok(Value::Number(if index < units.len() {
                units[index] as i64
            } else if index == units.len() {
                text.len() as i64
            } else {
                -1
            }))
        }),
        "charidx" | "utf16idx" => arity(name, args, 2, 4).and_then(|()| {
            let source = text(args, 0)?;
            let byte = number(args, 1)?;
            let count_composing = opt_bool(args, 2, false)?;
            if byte < 0 || byte as usize > source.len() {
                return Ok(Value::Number(-1));
            }
            let byte = byte as usize;
            let mut count = 0i64;
            if name == "utf16idx" {
                for (offset, c) in source.char_indices() {
                    if offset + c.len_utf8() > byte {
                        return Ok(Value::Number(count));
                    }
                    count += c.len_utf16() as i64;
                }
                return Ok(Value::Number(if byte == source.len() { count } else { -1 }));
            }
            let offsets = char_unit_offsets(&source, !count_composing);
            for (index, offset) in offsets.iter().enumerate() {
                let end = offsets.get(index + 1).copied().unwrap_or(source.len());
                if byte < end {
                    return Ok(Value::Number(index as i64));
                }
                let _ = offset;
            }
            Ok(Value::Number(if byte == source.len() { offsets.len() as i64 } else { -1 }))
        }),
        "strutf16len" => arity(name, args, 1, 2).and_then(|()| Ok(Value::Number(text(args, 0)?.encode_utf16().count() as i64))),
        "iconv" => arity(name, args, 3, 3).and_then(|()| iconv(args)),
        "keytrans" => arity(name, args, 1, 1).and_then(|()| {
            let source = text(args, 0)?;
            Ok(Value::String(crate::vimscript::keys::render(&crate::vimscript::keys::decode(&source)).into()))
        }),
        "str2nr" => arity(name, args, 1, 3).and_then(|()| str2nr(args)),
        "str2float" => arity(name, args, 1, 2).and_then(|()| {
            let source = text(args, 0)?;
            let source = if opt_bool(args, 1, false)? { source.replace('\'', "") } else { source };
            Ok(Value::Float(parse_float_prefix(source.trim_start())))
        }),
        "float2nr" => arity(name, args, 1, 1).and_then(|()| {
            let value = float_arg(&args[0])?;
            Ok(Value::Number(if value.is_nan() { 0 } else if value >= i64::MAX as f64 { i64::MAX } else if value <= i64::MIN as f64 { i64::MIN } else { value.trunc() as i64 }))
        }),
        "submatch" => arity(name, args, 1, 2).and_then(|()| {
            let index = number(args, 0)?;
            if !(0..=9).contains(&index) {
                return Err(Fail::Error(VimError::new(935, format!("Invalid submatch number: {index}"))));
            }
            let want_list = opt_bool(args, 1, false)?;
            let value = rt.submatches.last().and_then(|groups| groups.get(index as usize).cloned()).unwrap_or_else(|| Value::str(""));
            Ok(if want_list { Value::list(vec![value]) } else { value })
        }),
        "charclass" => arity(name, args, 1, 1).and_then(|()| {
            let source = text(args, 0)?;
            let Some(c) = source.chars().next() else { return Ok(Value::Number(0)) };
            Ok(Value::Number(if c.is_whitespace() {
                0
            } else if c.is_alphanumeric() || c == '_' {
                2
            } else if (c as u32) >= 0x1F300 && (c as u32) <= 0x1FAFF {
                3
            } else {
                1
            }))
        }),
        "json_encode" | "js_encode" => arity(name, args, 1, 1).and_then(|()| {
            let mut out = String::new();
            json_encode(&args[0], &mut out, name == "js_encode", 0)?;
            Ok(Value::String(out.into()))
        }),
        "json_decode" | "js_decode" => arity(name, args, 1, 1).and_then(|()| {
            let source = text(args, 0)?;
            if source.trim().is_empty() {
                return Ok(Value::None);
            }
            let parsed: serde_json::Value = serde_json::from_str(&source).map_err(|_| Fail::Error(VimError::new(491, format!("JSON decode error at '{source}'"))))?;
            Ok(json_to_value(parsed))
        }),
        "sha256" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(sha256_hex(vstr(args, 0)?.as_bytes()).into()))),
        "abs" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::Float(value) => Ok(Value::Float(value.abs())),
            Value::String(_) => Err(Fail::Error(VimError::new(808, "Number or Float required"))),
            other => Ok(Value::Number(other.to_number()?.wrapping_abs())),
        }),
        "ceil" | "floor" | "round" | "trunc" | "sqrt" | "exp" | "log" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
        | "sinh" | "cosh" | "tanh" => arity(name, args, 1, 1).and_then(|()| {
            let value = float_arg(&args[0])?;
            Ok(Value::Float(match name {
                "ceil" => value.ceil(),
                "floor" => value.floor(),
                "round" => value.round(),
                "trunc" => value.trunc(),
                "sqrt" => value.sqrt(),
                "exp" => value.exp(),
                "log" => value.ln(),
                "log10" => value.log10(),
                "sin" => value.sin(),
                "cos" => value.cos(),
                "tan" => value.tan(),
                "asin" => value.asin(),
                "acos" => value.acos(),
                "atan" => value.atan(),
                "sinh" => value.sinh(),
                "cosh" => value.cosh(),
                _ => value.tanh(),
            }))
        }),
        "pow" | "atan2" | "fmod" => arity(name, args, 2, 2).and_then(|()| {
            let (a, b) = (float_arg(&args[0])?, float_arg(&args[1])?);
            Ok(Value::Float(match name {
                "pow" => a.powf(b),
                "atan2" => a.atan2(b),
                _ => a % b,
            }))
        }),
        "isinf" => arity(name, args, 1, 1).and_then(|()| {
            let value = float_arg(&args[0])?;
            Ok(Value::Number(if value.is_infinite() { if value > 0.0 { 1 } else { -1 } } else { 0 }))
        }),
        "isnan" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(float_arg(&args[0])?.is_nan() as i64))),
        "and" => arity(name, args, 2, 2).and_then(|()| Ok(Value::Number(number(args, 0)? & number(args, 1)?))),
        "or" => arity(name, args, 2, 2).and_then(|()| Ok(Value::Number(number(args, 0)? | number(args, 1)?))),
        "xor" => arity(name, args, 2, 2).and_then(|()| Ok(Value::Number(number(args, 0)? ^ number(args, 1)?))),
        "invert" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(!number(args, 0)?))),
        "rand" => arity(name, args, 0, 1).and_then(|()| rand(args)),
        "srand" => arity(name, args, 0, 1).and_then(|()| {
            let seed = match args.first() {
                Some(value) => value.to_number()? as u32,
                None => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u32),
            };
            Ok(Value::list(srand_state(seed).into_iter().map(|n| Value::Number(n as i64)).collect()))
        }),
        "repeat" => arity(name, args, 2, 2).and_then(|()| {
            let count = number(args, 1)?.max(0) as usize;
            match &args[0] {
                Value::List(list) => {
                    let items = list.snapshot();
                    if items.len().saturating_mul(count) > super::MAX_LIST_ITEMS {
                        return Err(Fail::Interrupt("repeat() result too large".into()));
                    }
                    Ok(Value::list(items.iter().cloned().cycle().take(items.len() * count).collect()))
                }
                Value::Blob(blob) => {
                    let bytes = blob.bytes();
                    if bytes.len().saturating_mul(count) > super::MAX_STRING_BYTES {
                        return Err(Fail::Interrupt("repeat() result too large".into()));
                    }
                    Ok(Value::Blob(Blob::new(bytes.repeat(count))))
                },
                other => {
                    let bytes = other.to_bytes()?;
                    if bytes.len().saturating_mul(count) > super::MAX_STRING_BYTES {
                        return Err(Fail::Interrupt("repeat() result too large".into()));
                    }
                    Ok(Value::String(bytes.repeat(count).into()))
                }
            }
        }),
        "string" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(crate::vimscript::objects::display(rt, host, &args[0], true)?.into()))),
        "base64_encode" => arity(name, args, 1, 1).and_then(|()| match &args[0] {
            Value::Blob(blob) => Ok(Value::String(base64_encode(&blob.bytes()).into())),
            _ => Err(Fail::Error(VimError::new(1238, "Blob required for argument 1"))),
        }),
        "base64_decode" => arity(name, args, 1, 1).and_then(|()| {
            let source = text(args, 0)?;
            base64_decode(&source).map(|bytes| Value::Blob(Blob::new(bytes))).ok_or_else(|| Fail::Error(VimError::new(475, format!("Invalid argument: {source}"))))
        }),
        "blob2str" => arity(name, args, 1, 2).and_then(|()| match &args[0] {
            Value::Blob(blob) => Ok(Value::list(blob.bytes().split(|b| *b == b'\n').map(|line| Value::String(line.to_vec().into())).collect())),
            _ => Err(Fail::Error(VimError::new(1238, "Blob required for argument 1"))),
        }),
        "str2blob" => arity(name, args, 1, 2).and_then(|()| match &args[0] {
            Value::List(list) => {
                let mut bytes = Vec::new();
                for (index, item) in list.snapshot().iter().enumerate() {
                    if index > 0 {
                        bytes.push(b'\n');
                    }
                    bytes.extend(item.to_bytes()?);
                }
                Ok(Value::Blob(Blob::new(bytes)))
            }
            _ => Err(Fail::Error(VimError::new(1211, "List required for argument 1"))),
        }),
        _ => return None,
    })
}

fn float_arg(value: &Value) -> EvalResult<f64> {
    match value {
        Value::Float(value) => Ok(*value),
        Value::Number(number) => Ok(*number as f64),
        _ => Err(Fail::Error(VimError::new(808, "Number or Float required"))),
    }
}

/// Characters of a string; with `combine`, composing characters stay with
/// their base character.
pub(crate) fn char_units(source: &VStr, combine: bool) -> Vec<String> {
    let text = source.to_str_lossy();
    if combine {
        text.graphemes(true).map(str::to_string).collect()
    } else {
        text.chars().map(|c| c.to_string()).collect()
    }
}

fn char_unit_offsets(text: &str, combine: bool) -> Vec<usize> {
    if combine {
        text.grapheme_indices(true).map(|(offset, _)| offset).collect()
    } else {
        text.char_indices().map(|(offset, _)| offset).collect()
    }
}

/// Display cells with tabs expanded from `column`.
pub(crate) fn display_width(text: &str, column: usize, tabstop: usize) -> usize {
    let mut width = column;
    for c in text.chars() {
        if c == '\t' {
            width += tabstop - width % tabstop;
        } else if (c as u32) < 0x20 || c == '\x7f' {
            width += 2;
        } else {
            width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        }
    }
    width - column
}

fn strtrans(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            for c in text.chars() {
                match c {
                    '\x00'..='\x1f' => {
                        out.push(b'^');
                        out.push(c as u8 + b'@');
                    }
                    '\x7f' => out.extend_from_slice(b"^?"),
                    '\u{80}'..='\u{9f}' => out.extend_from_slice(format!("<{:x}>", c as u32).as_bytes()),
                    _ => {
                        let mut buffer = [0; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                    }
                }
            }
        }
        Err(_) => {
            for &b in bytes {
                if b < 0x20 {
                    out.push(b'^');
                    out.push(b + b'@');
                } else if b >= 0x80 {
                    out.extend_from_slice(format!("<{b:02x}>").as_bytes());
                } else {
                    out.push(b);
                }
            }
        }
    }
    out
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn strpart(args: &[Value]) -> EvalResult<Value> {
    let source = vstr(args, 0)?;
    let chars = opt_bool(args, 3, false)?;
    let mut start = number(args, 1)?;
    if chars {
        let units = char_units(&source, false);
        let mut len = match args.get(2) {
            Some(value) => value.to_number()?,
            None => units.len() as i64,
        };
        if start < 0 {
            len += start;
            start = 0;
        }
        let start = (start as usize).min(units.len());
        let end = (start + len.max(0) as usize).min(units.len());
        return Ok(Value::String(units[start..end].concat().into()));
    }
    let bytes = source.as_bytes();
    let mut len = match args.get(2) {
        Some(value) => value.to_number()?,
        None => bytes.len() as i64 - start.max(0),
    };
    if start < 0 {
        len += start;
        start = 0;
    }
    let start = (start as usize).min(bytes.len());
    let end = (start as i64 + len.max(0)).min(bytes.len() as i64) as usize;
    Ok(Value::String(bytes[start..end.max(start)].to_vec().into()))
}

fn str2nr(args: &[Value]) -> EvalResult<Value> {
    let source = text(args, 0)?;
    let base = opt_number(args, 1, 10)?;
    if !matches!(base, 2 | 8 | 10 | 16) {
        return Err(Fail::Error(VimError::new(474, "Invalid argument")));
    }
    let quoted = opt_bool(args, 2, false)?;
    let mut text = source.trim_start().to_string();
    if quoted {
        text = text.replace('\'', "");
    }
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, text.strip_prefix('+').unwrap_or(&text).to_string()),
    };
    let digits = match base {
        16 => rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")).unwrap_or(&rest).to_string(),
        2 => rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")).unwrap_or(&rest).to_string(),
        8 => rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")).unwrap_or(&rest).to_string(),
        _ => rest,
    };
    let mut value: i64 = 0;
    for c in digits.chars() {
        let Some(digit) = c.to_digit(base as u32) else { break };
        value = value.saturating_mul(base).saturating_add(digit as i64);
    }
    Ok(Value::Number(if negative { -value } else { value }))
}

/// The longest valid float at the start of `text` (strtod-like).
pub(crate) fn parse_float_prefix(text: &str) -> f64 {
    let lower = text.to_ascii_lowercase();
    for (word, value) in [("inf", f64::INFINITY), ("+inf", f64::INFINITY), ("-inf", f64::NEG_INFINITY), ("nan", f64::NAN), ("+nan", f64::NAN), ("-nan", f64::NAN)] {
        if lower.starts_with(word) {
            return value;
        }
    }
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    if end == digits_start {
        return 0.0;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                exponent += 1;
            }
            end = exponent;
        }
    }
    text[..end].trim_end_matches('.').parse().unwrap_or(0.0)
}

fn iconv(args: &[Value]) -> EvalResult<Value> {
    let source = vstr(args, 0)?;
    let normalize = |name: String| name.to_ascii_lowercase().replace('_', "-");
    let (from, to) = (normalize(text(args, 1)?), normalize(text(args, 2)?));
    let utf8 = |name: &str| matches!(name, "utf-8" | "utf8" | "ucs-2" | "ucs2");
    let latin = |name: &str| matches!(name, "latin1" | "iso-8859-1" | "latin-1" | "cp1252" | "iso8859-1");
    let bytes = source.as_bytes();
    if utf8(&from) && utf8(&to) || from == to {
        return Ok(Value::String(source));
    }
    if latin(&from) && utf8(&to) {
        return Ok(Value::String(bytes.iter().map(|b| *b as char).collect::<String>().into()));
    }
    if utf8(&from) && latin(&to) {
        let Ok(text) = std::str::from_utf8(bytes) else { return Ok(Value::str("")) };
        let mut out = Vec::new();
        for c in text.chars() {
            if (c as u32) > 0xff {
                return Ok(Value::str(""));
            }
            out.push(c as u8);
        }
        return Ok(Value::String(out.into()));
    }
    Ok(Value::str(""))
}

fn substitute(rt: &mut Runtime, host: &mut dyn ScriptHost, args: &[Value]) -> EvalResult<Value> {
    let source = vstr(args, 0)?;
    let pattern = vstr(args, 1)?;
    let flags = text(args, 3)?;
    let global = flags.contains('g');
    let text = source.to_str_lossy().into_owned();
    let pattern = regex::compile(pattern.as_bytes(), false)?;
    let mut out = String::new();
    // Vim's do_string_sub(): an empty match is skipped when it repeats at
    // the position of the previous empty match, and matching stops once a
    // match reaches the end of the string.
    let mut tail = 0usize;
    let mut zero_width: Option<usize> = None;
    while let Some(found) = regex::find_at(&pattern, &text, tail, &mut rt.fuel)? {
        if found.start == found.end {
            if zero_width == Some(found.start) {
                let step = text[tail..].chars().next().map_or(1, char::len_utf8);
                if tail >= text.len() {
                    break;
                }
                out.push_str(&text[tail..tail + step]);
                tail += step;
                continue;
            }
            zero_width = Some(found.start);
        }
        out.push_str(&text[tail..found.start]);
        let replacement = match &args[2] {
            Value::Funcref(_) => {
                let groups: Vec<Value> = found.groups.iter().map(|group| Value::String(group.clone().map_or_else(VStr::empty, |range| text[range].into()))).collect();
                call_callback(rt, host, &args[2], vec![Value::list(groups)])?.to_text()?
            }
            other => {
                let sub = other.to_text()?;
                if let Some(expression) = sub.strip_prefix("\\=") {
                    let groups: Vec<Value> = found.groups.iter().map(|group| Value::String(group.clone().map_or_else(VStr::empty, |range| text[range].into()))).collect();
                    rt.submatches.push(groups);
                    let sid = rt.current_sid();
                    let value = rt.eval_expression(host, expression.as_bytes(), sid);
                    rt.submatches.pop();
                    match value? {
                        Value::List(list) => list.snapshot().iter().map(|item| item.to_text()).collect::<Result<Vec<_>, _>>()?.join("\n"),
                        other => other.to_text()?,
                    }
                } else {
                    regex::expand_replacement(&sub, &text, &found)
                }
            }
        };
        out.push_str(&replacement);
        tail = found.end;
        if tail >= text.len() || !global {
            break;
        }
    }
    if tail < text.len() {
        out.push_str(&text[tail..]);
    }
    Ok(Value::String(out.into()))
}

fn match_function(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> EvalResult<Value> {
    let _ = host;
    let pattern = regex::compile(vstr(args, 1)?.as_bytes(), host.ignorecase())?;
    let count = opt_number(args, 3, 1)?.max(1);
    if let Value::List(list) = &args[0] {
        let items = list.snapshot();
        let mut index = opt_number(args, 2, 0)?;
        if index < 0 {
            index = 0;
        }
        let mut remaining = count;
        for (position, item) in items.iter().enumerate().skip(index as usize) {
            let text = item.to_text().unwrap_or_default();
            if let Some(found) = regex::find_at(&pattern, &text, 0, &mut rt.fuel)? {
                remaining -= 1;
                if remaining > 0 {
                    continue;
                }
                return Ok(match name {
                    "match" => Value::Number(position as i64),
                    "matchend" => Value::Number(found.end as i64),
                    "matchstr" => Value::String(text[found.start..found.end].into()),
                    "matchstrpos" => Value::list(vec![Value::String(text[found.start..found.end].into()), (position as i64).into(), (found.start as i64).into(), (found.end as i64).into()]),
                    _ => groups_list(&text, &found),
                });
            }
        }
        return Ok(no_match(name, true));
    }
    let text = args[0].to_text()?;
    let mut start = opt_number(args, 2, 0)?;
    if start < 0 {
        start = 0;
    }
    if start as usize > text.len() {
        return Ok(no_match(name, false));
    }
    let mut at = start as usize;
    let mut remaining = count;
    loop {
        let Some(found) = regex::find_at(&pattern, &text, at, &mut rt.fuel)? else {
            return Ok(no_match(name, false));
        };
        remaining -= 1;
        if remaining == 0 {
            return Ok(match name {
                "match" => Value::Number(found.start as i64),
                "matchend" => Value::Number(found.end as i64),
                "matchstr" => Value::String(text[found.start..found.end].into()),
                "matchstrpos" => Value::list(vec![Value::String(text[found.start..found.end].into()), (found.start as i64).into(), (found.end as i64).into()]),
                _ => groups_list(&text, &found),
            });
        }
        at = if found.end > found.start { found.end } else { found.end + text[found.end..].chars().next().map_or(1, char::len_utf8) };
        if at > text.len() {
            return Ok(no_match(name, false));
        }
    }
}

fn groups_list(text: &str, found: &regex::Match) -> Value {
    Value::list(found.groups.iter().map(|group| Value::String(group.clone().map_or_else(VStr::empty, |range| text[range].into()))).collect())
}

fn no_match(name: &str, list: bool) -> Value {
    match name {
        "match" | "matchend" => Value::Number(-1),
        "matchstr" => Value::str(""),
        "matchstrpos" => {
            if list {
                Value::list(vec![Value::str(""), (-1).into(), (-1).into(), (-1).into()])
            } else {
                Value::list(vec![Value::str(""), (-1).into(), (-1).into()])
            }
        }
        _ => Value::list(Vec::new()),
    }
}

fn json_encode(value: &Value, out: &mut String, js: bool, depth: usize) -> EvalResult<()> {
    if depth > 100 {
        return Err(Fail::Error(VimError::new(724, "Variable nested too deep for displaying")));
    }
    match value {
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::Float(float) => {
            if float.is_nan() {
                out.push_str("NaN");
            } else if float.is_infinite() {
                out.push_str(if *float > 0.0 { "Infinity" } else { "-Infinity" });
            } else {
                out.push_str(&crate::vimscript::value::format_float(*float));
            }
        }
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Null | Value::NullTyped(_) => out.push_str("null"),
        Value::None => {
            if !js {
                out.push_str("null");
            }
        }
        Value::String(text) => out.push_str(&serde_json::to_string(&text.to_str_lossy()).unwrap_or_default()),
        Value::Tuple(_) | Value::List(_) => {
            let items = match value {Value::Tuple(items) => items.as_ref().clone(), Value::List(list) => list.snapshot(), _ => unreachable!()};
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                json_encode(item, out, js, depth + 1)?;
            }
            out.push(']');
        }
        Value::Dict(dict) => {
            out.push('{');
            for (index, (key, item)) in dict.snapshot().iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let key = key.to_str_lossy();
                if js && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !key.is_empty() {
                    out.push_str(&key);
                } else {
                    out.push_str(&serde_json::to_string(&key).unwrap_or_default());
                }
                out.push(':');
                json_encode(item, out, js, depth + 1)?;
            }
            out.push('}');
        }
        Value::Blob(blob) => {
            out.push('[');
            for (index, byte) in blob.bytes().iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&byte.to_string());
            }
            out.push(']');
        }
        Value::TypeAlias(_) | Value::Void | Value::Funcref(_) | Value::Class(_) | Value::Object(_) | Value::Module(_) | Value::Type(_) => return Err(Fail::Error(VimError::new(474, "Invalid argument"))),
    }
    Ok(())
}

fn json_to_value(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(value),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(value) => Value::Number(value),
            None => Value::Float(number.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(text) => Value::String(text.into()),
        serde_json::Value::Array(items) => Value::list(items.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(map) => Value::Dict(Dict::from_pairs(map.into_iter().map(|(key, value)| (VStr::from(key), json_to_value(value))))),
    }
}

thread_local! {
    static RAND: std::cell::Cell<Option<[u32; 4]>> = const { std::cell::Cell::new(None) };
}

fn srand_state(seed: u32) -> [u32; 4] {
    // splitmix32, as Vim's init_srand().
    let mut x = seed;
    let mut next = || {
        x = x.wrapping_add(0x9e3779b9);
        let mut z = x;
        z = (z ^ (z >> 16)).wrapping_mul(0x85ebca6b);
        z = (z ^ (z >> 13)).wrapping_mul(0xc2b2ae35);
        z ^ (z >> 16)
    };
    [next(), next(), next(), next()]
}

fn xoshiro(state: &mut [u32; 4]) -> u32 {
    let result = state[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    let t = state[1] << 9;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = state[3].rotate_left(11);
    result
}

fn rand(args: &[Value]) -> EvalResult<Value> {
    if let Some(Value::List(list)) = args.first() {
        let items = list.snapshot();
        if items.len() != 4 {
            return Err(Fail::Error(VimError::new(475, "Invalid argument")));
        }
        let mut state = [0u32; 4];
        for (slot, item) in state.iter_mut().zip(&items) {
            *slot = item.to_number()? as u32;
        }
        let result = xoshiro(&mut state);
        let mut data = list.data();
        for (index, value) in state.iter().enumerate() {
            data.items[index] = Value::Number(*value as i64);
        }
        return Ok(Value::Number(result as i64));
    }
    Ok(Value::Number(RAND.with(|cell| {
        let mut state = cell.get().unwrap_or_else(|| srand_state(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u32)));
        let result = xoshiro(&mut state);
        cell.set(Some(state));
        result as i64
    })))
}

fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (slot, value) in h.iter_mut().zip(v) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(BASE64[(n >> 18) as usize & 63] as char);
        out.push(BASE64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { BASE64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        if c == b'=' {
            break;
        }
        let value = BASE64.iter().position(|b| *b == c)? as u32;
        buffer = buffer << 6 | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn floats_parse_like_strtod() {
        assert_eq!(parse_float_prefix("1.5e3x"), 1500.0);
        assert_eq!(parse_float_prefix("1e3"), 1000.0);
        assert_eq!(parse_float_prefix("abc"), 0.0);
        assert!(parse_float_prefix("inf").is_infinite());
    }

    #[test]
    fn base64_round_trips() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    }
}
