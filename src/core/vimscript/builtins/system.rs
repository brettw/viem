//! Environment, file and time builtins. File access is read-only and
//! bounded; scripts cannot write files, delete them or run processes.
use super::{arity, number, opt_bool, opt_number, text, vstr};
use crate::vimscript::error::{EvalResult, Fail, VimError};
use crate::vimscript::exec::{Runtime, ScriptHost};
use crate::vimscript::value::{Dict, VStr, Value};

pub(super) const NAMES: &[&str] = &[
    "getenv", "setenv", "environ", "expand", "expandcmd", "exepath", "executable", "filereadable",
    "filewritable", "isdirectory", "isabsolutepath", "getftype", "getfsize", "getftime", "getfperm",
    "glob", "globpath", "glob2regpat", "fnamemodify", "simplify", "resolve", "readfile", "readdir",
    "readblob", "getcwd", "haslocaldir", "hostname", "tempname", "mkdir", "delete", "rename",
    "writefile", "localtime", "strftime", "reltime", "reltimestr", "reltimefloat", "getpid",
    "system", "systemlist", "job_start", "libcall", "libcallnr", "timer_start",
];

const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_GLOB_RESULTS: usize = 10_000;

fn unsupported(name: &str) -> Fail {
    Fail::Error(VimError::plain(format!("Viem scripts cannot call {name}()")))
}

pub(super) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
    Some(match name {
        "getenv" => arity(name, args, 1, 1).and_then(|()| Ok(match host.getenv(&text(args, 0)?) {
            Some(value) => Value::String(value.into()),
            None => Value::Null,
        })),
        "setenv" => arity(name, args, 2, 2).and_then(|()| {
            let name = text(args, 0)?;
            let value = match &args[1] {
                Value::Null => None,
                other => Some(other.to_text()?),
            };
            host.setenv(&name, value.as_deref()).map(|()| Value::Number(0)).map_err(Fail::Error)
        }),
        "environ" => arity(name, args, 0, 0).map(|()| Value::Dict(Dict::from_pairs(std::env::vars().map(|(key, value)| (VStr::from(key), Value::String(value.into())))))),
        "expand" => arity(name, args, 1, 3).and_then(|()| expand(rt, host, args)),
        "expandcmd" => arity(name, args, 1, 2).and_then(|()| {
            let source = text(args, 0)?;
            let mut out = Vec::new();
            for word in source.split(' ') {
                out.push(if word.starts_with(['%', '#', '<', '~', '$']) { crate::vimscript::source::expand_path(rt, host, word)? } else { word.to_string() });
            }
            Ok(Value::String(out.join(" ").into()))
        }),
        "exepath" | "executable" => arity(name, args, 1, 1).and_then(|()| {
            let program = text(args, 0)?;
            let found = find_executable(host, &program);
            Ok(if name == "exepath" { Value::String(found.unwrap_or_default().into()) } else { Value::Number(found.is_some() as i64) })
        }),
        "filereadable" => arity(name, args, 1, 1).and_then(|()| {
            let path = expand_name(rt, host, &text(args, 0)?)?;
            let readable = std::fs::metadata(&path).is_ok_and(|m| m.is_file()) && std::fs::File::open(&path).is_ok();
            Ok(Value::Number(readable as i64))
        }),
        "filewritable" => arity(name, args, 1, 1).and_then(|()| {
            let path = expand_name(rt, host, &text(args, 0)?)?;
            Ok(Value::Number(match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => 2,
                Ok(metadata) => (!metadata.permissions().readonly()) as i64,
                Err(_) => 0,
            }))
        }),
        "isdirectory" => arity(name, args, 1, 1).and_then(|()| {
            let path = expand_name(rt, host, &text(args, 0)?)?;
            Ok(Value::Number(std::path::Path::new(&path).is_dir() as i64))
        }),
        "isabsolutepath" => arity(name, args, 1, 1).and_then(|()| Ok(Value::Number(std::path::Path::new(&text(args, 0)?).is_absolute() as i64))),
        "getftype" => arity(name, args, 1, 1).and_then(|()| {
            let path = text(args, 0)?;
            Ok(Value::String(match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => "link",
                Ok(metadata) if metadata.is_dir() => "dir",
                Ok(metadata) if metadata.is_file() => "file",
                Ok(_) => "other",
                Err(_) => "",
            }.into()))
        }),
        "getfsize" => arity(name, args, 1, 1).and_then(|()| {
            let path = text(args, 0)?;
            Ok(Value::Number(match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => 0,
                Ok(metadata) => metadata.len().min(i64::MAX as u64) as i64,
                Err(_) => -1,
            }))
        }),
        "getftime" => arity(name, args, 1, 1).and_then(|()| {
            let path = text(args, 0)?;
            Ok(Value::Number(
                std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(-1, |d| d.as_secs() as i64),
            ))
        }),
        "getfperm" => arity(name, args, 1, 1).and_then(|()| {
            let path = text(args, 0)?;
            Ok(Value::String(match std::fs::metadata(&path) {
                Ok(metadata) => permissions(&metadata),
                Err(_) => String::new(),
            }.into()))
        }),
        "glob" => arity(name, args, 1, 4).and_then(|()| {
            let pattern = expand_name(rt, host, &text(args, 0)?)?;
            let as_list = opt_bool(args, 2, false)?;
            let matches = glob(rt, &pattern, &mut GlobBudget::default())?;
            Ok(if as_list { Value::list(matches.into_iter().map(Value::from).collect()) } else { Value::String(matches.join("\n").into()) })
        }),
        "globpath" => arity(name, args, 2, 5).and_then(|()| {
            let paths = text(args, 0)?;
            let pattern = text(args, 1)?;
            let as_list = opt_bool(args, 3, false)?;
            let mut matches = Vec::new();
            let mut budget = GlobBudget::default();
            for directory in paths.split(',').filter(|d| !d.is_empty()) {
                let directory = expand_name(rt, host, directory)?;
                matches.extend(glob(rt, &format!("{}/{}", directory.trim_end_matches('/'), pattern), &mut budget)?);
            }
            Ok(if as_list { Value::list(matches.into_iter().map(Value::from).collect()) } else { Value::String(matches.join("\n").into()) })
        }),
        "glob2regpat" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(glob2regpat(&text(args, 0)?).into()))),
        "fnamemodify" => arity(name, args, 2, 2).and_then(|()| {
            let path = text(args, 0)?;
            let modifiers = text(args, 1)?;
            let path = if path.starts_with('~') { crate::vimscript::source::expand_path(rt, host, &path)? } else { path };
            Ok(Value::String(crate::vimscript::source::apply_modifiers(&path, &modifiers).into()))
        }),
        "simplify" => arity(name, args, 1, 1).and_then(|()| Ok(Value::String(simplify(&text(args, 0)?).into()))),
        "resolve" => arity(name, args, 1, 1).and_then(|()| {
            let path = text(args, 0)?;
            Ok(Value::String(std::fs::canonicalize(&path).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path).into()))
        }),
        "readfile" | "readblob" => arity(name, args, 1, 3).and_then(|()| {
            let path = expand_name(rt, host, &text(args, 0)?)?;
            let metadata = std::fs::metadata(&path).map_err(|_| Fail::Error(VimError::new(484, format!("Can't open file {path}"))))?;
            if !metadata.is_file() {
                return Err(Fail::Error(VimError::new(484, format!("Not a regular file: {path}"))));
            }
            if metadata.len() > MAX_READ_BYTES {
                return Err(Fail::Error(VimError::plain(format!("{path}: file too large for {name}()"))));
            }
            use std::io::Read;
            let file = std::fs::File::open(&path).map_err(|_| Fail::Error(VimError::new(484, format!("Can't open file {path}"))))?;
            if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
                return Err(Fail::Error(VimError::new(484, format!("Not a regular file: {path}"))));
            }
            let mut bytes = Vec::new();
            file.take(MAX_READ_BYTES + 1).read_to_end(&mut bytes).map_err(|_| Fail::Error(VimError::new(484, format!("Can't read file {path}"))))?;
            if bytes.len() as u64 > MAX_READ_BYTES {
                return Err(Fail::Error(VimError::plain(format!("{path}: file too large for {name}()"))));
            }
            if name == "readblob" {
                return Ok(Value::Blob(crate::vimscript::value::Blob::new(bytes)));
            }
            let binary = matches!(args.get(1), Some(Value::String(flags)) if flags.as_bytes().contains(&b'b'));
            let limit = opt_number(args, 2, i64::MAX)?;
            let mut lines: Vec<Value> = Vec::new();
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            let mut parts: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
            if !binary && bytes.ends_with(b"\n") {
                parts.pop();
            }
            for part in parts {
                let part = if binary { part } else { part.strip_suffix(b"\r").unwrap_or(part) };
                lines.push(Value::String(part.to_vec().into()));
            }
            if limit >= 0 && (limit as usize) < lines.len() {
                lines.truncate(limit as usize);
            } else if limit < 0 {
                let keep = limit.unsigned_abs().min(usize::MAX as u64) as usize;
                if keep < lines.len() {
                    lines.drain(..lines.len() - keep);
                }
            }
            Ok(Value::list(lines))
        }),
        "readdir" => arity(name, args, 1, 3).and_then(|()| {
            let path = expand_name(rt, host, &text(args, 0)?)?;
            let mut names: Vec<String> = std::fs::read_dir(&path)
                .map_err(|_| Fail::Error(VimError::new(484, format!("Can't open file {path}"))))?
                .filter_map(|entry| entry.ok().map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .take(MAX_GLOB_RESULTS)
                .collect();
            names.sort();
            Ok(Value::list(names.into_iter().map(Value::from).collect()))
        }),
        "getcwd" => arity(name, args, 0, 2).map(|()| Value::String(std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default().into())),
        "haslocaldir" => arity(name, args, 0, 2).map(|()| Value::Number(0)),
        "hostname" => arity(name, args, 0, 0).map(|()| Value::String(hostname().into())),
        "tempname" => arity(name, args, 0, 0).map(|()| {
            let directory = std::env::temp_dir();
            let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            Value::String(directory.join(format!("viem{unique}")).to_string_lossy().into_owned().into())
        }),
        // Scripts cannot create directories; report failure without an
        // error, so guarded setup like `silent! call mkdir(...)` stays quiet.
        "mkdir" => arity(name, args, 1, 3).map(|()| Value::Number(0)),
        "delete" | "rename" | "writefile" | "system" | "systemlist" | "job_start" | "libcall" | "libcallnr" | "timer_start" => Err(unsupported(name)),
        "localtime" => arity(name, args, 0, 0).map(|()| Value::Number(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64))),
        "strftime" => arity(name, args, 1, 2).and_then(|()| {
            let format = text(args, 0)?;
            let time = match args.get(1) {
                Some(value) => value.to_number()?,
                None => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64),
            };
            Ok(Value::String(strftime(&format, time).into()))
        }),
        "reltime" => arity(name, args, 0, 2).and_then(|()| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let value = |d: std::time::Duration| Value::list(vec![(d.as_secs() as i64).into(), (d.subsec_micros() as i64).into()]);
            let read = |value: &Value| -> EvalResult<std::time::Duration> {
                let Value::List(list) = value else { return Ok(std::time::Duration::ZERO) };
                let items = list.snapshot();
                let seconds = items.first().map(|v| v.to_number()).transpose()?.unwrap_or(0);
                let micros = items.get(1).map(|v| v.to_number()).transpose()?.unwrap_or(0);
                Ok(std::time::Duration::from_secs(seconds.max(0) as u64) + std::time::Duration::from_micros(micros.max(0) as u64))
            };
            Ok(match args.len() {
                0 => value(now),
                1 => value(now.saturating_sub(read(&args[0])?)),
                _ => value(read(&args[1])?.saturating_sub(read(&args[0])?)),
            })
        }),
        "reltimestr" | "reltimefloat" => arity(name, args, 1, 1).and_then(|()| {
            let Value::List(list) = &args[0] else { return Ok(Value::str("")) };
            let items = list.snapshot();
            let seconds = items.first().map(|v| v.to_number()).transpose()?.unwrap_or(0);
            let micros = items.get(1).map(|v| v.to_number()).transpose()?.unwrap_or(0);
            let float = seconds as f64 + micros as f64 / 1_000_000.0;
            Ok(if name == "reltimefloat" { Value::Float(float) } else { Value::String(format!("{float:12.6}").trim_start().to_string().into()) })
        }),
        "getpid" => arity(name, args, 0, 0).map(|()| Value::Number(std::process::id() as i64)),
        _ => return None,
    })
}

fn expand_name(rt: &mut Runtime, host: &mut dyn ScriptHost, text: &str) -> EvalResult<String> {
    if text.starts_with(['~', '$', '%', '<']) || text.contains("$") {
        crate::vimscript::source::expand_path(rt, host, text)
    } else {
        Ok(text.to_string())
    }
}

fn expand(rt: &mut Runtime, host: &mut dyn ScriptHost, args: &[Value]) -> EvalResult<Value> {
    let source = text(args, 0)?;
    let as_list = opt_bool(args, 2, false)?;
    let trimmed = source.trim();
    // Words under the cursor and similar editor values come from the host.
    let lower = trimmed.to_ascii_lowercase();
    if ["<cword>", "<cword>", "<cfile>", "<cexpr>", "<slnum>", "<sflnum>"].iter().any(|tag| lower.starts_with(tag)) || trimmed.starts_with("<cWORD>") {
        if let Some(result) = host.call_function(rt, "expand", &[Value::String(trimmed.into())]) {
            return result;
        }
        return Ok(Value::str(""));
    }
    let expanded = match crate::vimscript::source::expand_path(rt, host, trimmed) {
        Ok(expanded) => expanded,
        Err(Fail::Error(_)) if trimmed.starts_with(['%', '#']) => String::new(),
        Err(Fail::Error(error)) if error.code == 498 => String::new(),
        Err(other) => return Err(other),
    };
    let wildcard = expanded.contains(['*', '?', '[']);
    let results = if wildcard { glob(rt, &expanded, &mut GlobBudget::default())? } else { vec![expanded] };
    Ok(if as_list { Value::list(results.into_iter().map(Value::from).collect()) } else { Value::String(results.join("\n").into()) })
}

fn find_executable(host: &dyn ScriptHost, program: &str) -> Option<String> {
    if program.is_empty() {
        return None;
    }
    let is_executable = |path: &std::path::Path| -> bool {
        let Ok(metadata) = std::fs::metadata(path) else { return false };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if program.contains(['/', '\\']) {
        let path = std::path::Path::new(program);
        return is_executable(path).then(|| path.to_string_lossy().into_owned());
    }
    let search = host.getenv("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ';' } else { ':' };
    let extensions: Vec<String> = if cfg!(windows) {
        host.getenv("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into()).split(';').map(str::to_string).collect()
    } else {
        vec![String::new()]
    };
    for directory in search.split(separator).filter(|d| !d.is_empty()) {
        for extension in &extensions {
            let candidate = std::path::Path::new(directory).join(format!("{program}{extension}"));
            if is_executable(&candidate) {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn permissions(metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        let bits = ['r', 'w', 'x'];
        (0..9).map(|i| if mode & (1 << (8 - i)) != 0 { bits[i % 3] } else { '-' }).collect()
    }
    #[cfg(not(unix))]
    {
        if metadata.permissions().readonly() { "r--r--r--".into() } else { "rw-rw-rw-".into() }
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|text| text.trim().to_string()))
        .unwrap_or_else(|| "localhost".into())
}

/// One budget is shared by every root of globpath(), including failed matches.
struct GlobBudget {
    work: usize,
    results: usize,
    bytes: usize,
}

impl Default for GlobBudget {
    fn default() -> Self {
        Self { work: 100_000, results: MAX_GLOB_RESULTS, bytes: super::MAX_STRING_BYTES }
    }
}

impl GlobBudget {
    fn visit(&mut self, rt: &mut Runtime) -> EvalResult<()> {
        rt.charge(1)?;
        self.work = self.work.checked_sub(1).ok_or_else(|| Fail::Interrupt("glob() traversal limit exceeded".into()))?;
        Ok(())
    }

    fn add(&mut self, path: &str, results: &mut Vec<String>) -> EvalResult<()> {
        self.results = self.results.checked_sub(1).ok_or_else(|| Fail::Interrupt("glob() result limit exceeded".into()))?;
        self.bytes = self.bytes.checked_sub(path.len().saturating_add(1)).ok_or_else(|| Fail::Interrupt("glob() result too large".into()))?;
        results.push(path.to_string());
        Ok(())
    }
}

/// Filesystem glob with bounded results, entries scanned and traversal depth.
fn glob(rt: &mut Runtime, pattern: &str, budget: &mut GlobBudget) -> EvalResult<Vec<String>> {
    if !pattern.contains(['*', '?', '[']) {
        budget.visit(rt)?;
        let mut results = Vec::new();
        if std::path::Path::new(pattern).exists() {
            budget.add(pattern, &mut results)?;
        }
        return Ok(results);
    }
    let absolute = pattern.starts_with('/');
    let components: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).take(129).collect();
    if components.len() > 128 {
        return Err(Fail::Interrupt("glob() nesting limit exceeded".into()));
    }
    let mut results = Vec::new();
    let start = if absolute { "/" } else { "" };
    glob_walk(rt, start, &components, &mut results, budget, 0)?;
    results.sort();
    Ok(results)
}

fn glob_walk(rt: &mut Runtime, base: &str, components: &[&str], results: &mut Vec<String>, budget: &mut GlobBudget, depth: usize) -> EvalResult<()> {
    budget.visit(rt)?;
    if depth > 128 {
        return Err(Fail::Interrupt("glob() nesting limit exceeded".into()));
    }
    let Some((first, rest)) = components.split_first() else {
        if !base.is_empty() {
            budget.add(base.trim_end_matches('/'), results)?;
        }
        return Ok(());
    };
    let join = |name: &str| if base.is_empty() { name.to_string() } else if base.ends_with('/') { format!("{base}{name}") } else { format!("{base}/{name}") };
    if *first == "**" {
        glob_walk(rt, base, rest, results, budget, depth + 1)?;
        let directory = if base.is_empty() { "." } else { base };
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries {
                budget.visit(rt)?;
                let Ok(entry) = entry else { continue };
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.starts_with('.') {
                        glob_walk(rt, &join(&name), components, results, budget, depth + 1)?;
                    }
                }
            }
        }
        return Ok(());
    }
    if !first.contains(['*', '?', '[']) {
        let next = join(first);
        if std::path::Path::new(&next).exists() {
            glob_walk(rt, &next, rest, results, budget, depth + 1)?;
        }
        return Ok(());
    }
    let directory = if base.is_empty() { "." } else { base };
    let Ok(entries) = std::fs::read_dir(directory) else { return Ok(()) };
    let mut names = Vec::new();
    for entry in entries {
        budget.visit(rt)?;
        if let Ok(entry) = entry {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    for name in names {
        if name.starts_with('.') && !first.starts_with('.') {
            continue;
        }
        if crate::vimscript::autocmd::glob_match(first, &name, false) {
            glob_walk(rt, &join(&name), rest, results, budget, depth + 1)?;
        }
    }
    Ok(())
}

fn glob2regpat(glob: &str) -> String {
    let mut out = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '.' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '{' => out.push_str("\\("),
            '}' => out.push_str("\\)"),
            ',' => out.push_str("\\|"),
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push('\\');
                    out.push(next);
                }
            }
            other => out.push(other),
        }
    }
    out.push('$');
    out
}

fn simplify(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if path.ends_with('/') && !out.ends_with('/') && !out.is_empty() {
        out.push('/');
    }
    if out.is_empty() {
        out = if absolute { "/".into() } else { ".".into() };
    }
    out
}

/// `strftime()` in UTC with the common conversions.
fn strftime(format: &str, time: i64) -> String {
    let days = time.div_euclid(86_400);
    let seconds = time.rem_euclid(86_400);
    let (hour, minute, second) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let weekday = (days + 4).rem_euclid(7);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let cumulative = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = cumulative[(month - 1) as usize] + day + if leap && month > 2 { 1 } else { 0 };
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&year.to_string()),
            Some('y') => out.push_str(&format!("{:02}", year % 100)),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('e') => out.push_str(&format!("{day:2}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('I') => out.push_str(&format!("{:02}", if hour % 12 == 0 { 12 } else { hour % 12 })),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('p') => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            Some('a') => out.push_str(&DAYS[weekday as usize][..3]),
            Some('A') => out.push_str(DAYS[weekday as usize]),
            Some('b') | Some('h') => out.push_str(&MONTHS[(month - 1) as usize][..3]),
            Some('B') => out.push_str(MONTHS[(month - 1) as usize]),
            Some('j') => out.push_str(&format!("{yday:03}")),
            Some('w') => out.push_str(&weekday.to_string()),
            Some('u') => out.push_str(&(if weekday == 0 { 7 } else { weekday }).to_string()),
            Some('Z') => out.push_str("UTC"),
            Some('z') => out.push_str("+0000"),
            Some('s') => out.push_str(&time.to_string()),
            Some('F') => out.push_str(&format!("{year}-{month:02}-{day:02}")),
            Some('T') => out.push_str(&format!("{hour:02}:{minute:02}:{second:02}")),
            Some('R') => out.push_str(&format!("{hour:02}:{minute:02}")),
            Some('D') | Some('x') => out.push_str(&format!("{month:02}/{day:02}/{:02}", year % 100)),
            Some('X') => out.push_str(&format!("{hour:02}:{minute:02}:{second:02}")),
            Some('c') => out.push_str(&format!("{} {} {day:2} {hour:02}:{minute:02}:{second:02} {year}", &DAYS[weekday as usize][..3], &MONTHS[(month - 1) as usize][..3])),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    let _ = (number(&[], 0), vstr(&[], 0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_work_and_read_limits() {
        use crate::vimscript::{state::ScriptState, tests::MemoryHost};
        let directory = std::env::temp_dir().join(format!("viem-script-limits-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let _cleanup = Cleanup(directory.clone());
        for index in 0..8 {
            std::fs::write(directory.join(format!("{index}.txt")), b"a\nb\n").unwrap();
        }
        let base = directory.to_str().unwrap();
        let mut rt = Runtime::new(ScriptState::new());
        let mut results = Vec::new();
        // Scanning is bounded even when no name matches and no result is emitted.
        let mut budget = GlobBudget { work: 4, ..GlobBudget::default() };
        assert!(matches!(glob_walk(&mut rt, base, &["*missing"], &mut results, &mut budget, 0), Err(Fail::Interrupt(_))));
        assert!(results.is_empty());
        let mut budget = GlobBudget { results: 1, ..GlobBudget::default() };
        assert_eq!(glob(&mut rt, base, &mut budget).unwrap(), vec![base]);
        assert!(matches!(glob(&mut rt, base, &mut budget), Err(Fail::Interrupt(_))));
        let found = glob(&mut rt, &format!("{base}/*.txt"), &mut GlobBudget::default()).unwrap();
        assert_eq!(found.len(), 8);
        assert!(found.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(matches!(glob_walk(&mut rt, base, &["**", "missing"], &mut results, &mut GlobBudget::default(), 129), Err(Fail::Interrupt(_))));

        let mut host = MemoryHost::new();
        let read = call(&mut rt, &mut host, "readfile", &[directory.join("0.txt").to_str().unwrap().into(), "".into(), i64::MIN.into()]).unwrap().unwrap();
        assert_eq!(read.string_bytes(), b"['a', 'b']");
        assert!(matches!(call(&mut rt, &mut host, "readblob", &[base.into()]).unwrap(), Err(Fail::Error(_))));
        let large = directory.join("large");
        std::fs::File::create(&large).unwrap().set_len(MAX_READ_BYTES + 1).unwrap();
        assert!(matches!(call(&mut rt, &mut host, "readblob", &[large.to_str().unwrap().into()]).unwrap(), Err(Fail::Error(_))));
    }

    #[test]
    fn strftime_formats_utc() {
        assert_eq!(strftime("%Y-%m-%d %H:%M:%S %a %j", 0), "1970-01-01 00:00:00 Thu 001");
        assert_eq!(strftime("%Y-%m-%d", 951_782_400), "2000-02-29");
    }

    #[test]
    fn simplify_and_glob_patterns() {
        assert_eq!(simplify("/a/./b/../c"), "/a/c");
        assert_eq!(simplify("a/../../b"), "../b");
        assert_eq!(glob2regpat("*.txt"), "^.*\\.txt$");
    }
}
