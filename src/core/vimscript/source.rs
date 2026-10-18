//! Reading script files, line continuation and file name expansion.
use super::error::{EvalResult, Fail, VimError};
use super::exec::{FrameKind, Runtime, ScriptHost};
use super::state::SourceLine;

/// Scripts larger than this are rejected, like the existing `:source` limit.
pub const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

/// Read a script with bounded, read-only file access.
pub fn read_script_file(path: &str) -> Result<Vec<u8>, VimError> {
    let cannot_open = || VimError::new(484, format!("Can't open file {path}"));
    let metadata = std::fs::metadata(path).map_err(|_| cannot_open())?;
    if !metadata.is_file() {
        return Err(cannot_open());
    }
    if metadata.len() > MAX_SCRIPT_BYTES {
        return Err(VimError::plain(format!("{path}: script files are limited to 1 MiB")));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| cannot_open())?
        .take(MAX_SCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| cannot_open())?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err(VimError::plain(format!("{path}: script files are limited to 1 MiB")));
    }
    Ok(bytes)
}

/// Split script text into logical lines: strip a UTF-8 BOM and CR before
/// LF, join `\` continuation lines and drop `"\ ` continuation comments.
pub fn script_lines(text: &[u8]) -> Vec<SourceLine> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let mut lines: Vec<SourceLine> = Vec::new();
    let mut current: Option<(Vec<u8>, usize)> = None;
    let mut heredoc: Option<(Vec<u8>, bool)> = None;
    for (index, raw) in text.split(|b| *b == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let trimmed_start = raw.iter().position(|b| *b != b' ' && *b != b'\t').unwrap_or(raw.len());
        let rest = &raw[trimmed_start..];
        if heredoc.is_none() {
            if let Some((line, _)) = current.as_ref() {
                heredoc = heredoc_marker(line);
                if heredoc.is_some() {
                    let (line, lnum) = current.take().unwrap();
                    lines.push(SourceLine { text: line.into(), lnum });
                }
            }
        }
        if let Some((marker, trim)) = heredoc.as_ref() {
            let ended = if *trim { rest } else { raw } == marker;
            lines.push(SourceLine { text: raw.into(), lnum: index + 1 });
            if ended { heredoc = None; }
            continue;
        }
        if current.is_some() && rest.starts_with(b"\\") {
            let (line, _) = current.as_mut().unwrap();
            line.extend_from_slice(&rest[1..]);
            continue;
        }
        if current.is_some() && rest.starts_with(b"\"\\ ") {
            continue;
        }
        if let Some((line, lnum)) = current.take() {
            lines.push(SourceLine { text: line.into(), lnum });
        }
        current = Some((raw.to_vec(), index + 1));
    }
    if let Some((line, lnum)) = current {
        // A file ending in a newline has no extra empty line.
        if !(line.is_empty() && text.ends_with(b"\n")) {
            lines.push(SourceLine { text: line.into(), lnum });
        }
    }
    lines
}

/// A heredoc body is physical text: its backslashes and quotes do not
/// participate in the surrounding script's continuation syntax.
fn heredoc_marker(line: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut at = 0;
    let mut quote = None;
    while at < line.len() {
        let byte = line[at];
        if let Some(delimiter) = quote {
            if byte == b'\\' && delimiter == b'"' { at += 2; continue; }
            if byte == delimiter {
                if delimiter == b'\'' && line.get(at + 1) == Some(&delimiter) {
                    at += 2; continue;
                }
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if line[at..].starts_with(b"=<<") {
            let mut trim = false;
            for word in line[at + 3..].split(u8::is_ascii_whitespace).filter(|word| !word.is_empty()) {
                match word {
                    b"trim" => trim = true,
                    b"eval" => {},
                    marker if !marker.starts_with(b"\"") => return Some((marker.to_vec(), trim)),
                    _ => return None,
                }
            }
            return None;
        }
        at += 1;
    }
    None
}

/// Lines of an `:execute` string or an autocommand: NL separates commands.
/// Unlike physical script files, an evaluated string's CR is an input key
/// (not a CRLF artifact), e.g. the final Enter in `execute "normal! :set ...\<CR>"`.
pub fn split_command_lines(text: &[u8], lnum: usize) -> Vec<SourceLine> {
    text.split(|b| *b == b'\n')
        .map(|line| SourceLine { text: line.into(), lnum })
        .collect()
}

fn home() -> Option<String> {
    std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok())
}

/// The script file of the innermost script frame, for `<sfile>`/`<script>`.
pub(crate) fn current_script(rt: &Runtime) -> Option<String> {
    rt.frames.iter().rev().find_map(|frame| match &frame.kind {
        FrameKind::Script { path } => Some(path.clone()),
        _ => None,
    })
}

/// `<sfile>` inside a function names the function stack, like Vim.
fn current_sfile(rt: &Runtime) -> Option<String> {
    for frame in rt.frames.iter().rev() {
        match &frame.kind {
            FrameKind::Script { path } => return Some(path.clone()),
            FrameKind::Function => {
                let name = frame.function.as_ref().map(|function| function.name.to_str_lossy().into_owned())?;
                return Some(format!("function {name}"));
            }
            _ => {}
        }
    }
    None
}

/// Expand `~`, environment variables, `%`, `<sfile>` and file modifiers in
/// a file name argument (`:source`, `expand()`).
pub fn expand_path(rt: &Runtime, host: &dyn ScriptHost, text: &str) -> EvalResult<String> {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '\\' && at + 1 < chars.len() && matches!(chars[at + 1], ' ' | '\\' | '%' | '#' | '|' | '"') {
            out.push(chars[at + 1]);
            at += 2;
            continue;
        }
        if c == '~' && at == 0 && (chars.len() == 1 || matches!(chars[1], '/' | '\\')) {
            out.push_str(&home().unwrap_or_default());
            at += 1;
            continue;
        }
        if c == '$' {
            let (name, end) = if chars.get(at + 1) == Some(&'{') {
                let close = chars[at + 2..].iter().position(|c| *c == '}').map(|n| at + 2 + n);
                match close {
                    Some(close) => (chars[at + 2..close].iter().collect::<String>(), close + 1),
                    None => (String::new(), at + 1),
                }
            } else {
                let mut end = at + 1;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                (chars[at + 1..end].iter().collect(), end)
            };
            if !name.is_empty() {
                match host.getenv(&name) {
                    Some(value) => out.push_str(&value),
                    None if matches!(name.as_str(), "HOME") => out.push_str(&home().unwrap_or_default()),
                    None => {
                        // Vim leaves unknown variables unexpanded.
                        out.push('$');
                        out.push_str(&name);
                    }
                }
                at = end;
                continue;
            }
        }
        let rest: String = chars[at..].iter().collect();
        let special = [("<sfile>", 7), ("<script>", 8), ("<stack>", 7), ("<afile>", 7), ("<amatch>", 8), ("<abuf>", 6)]
            .into_iter()
            .find(|(tag, _)| rest.starts_with(tag));
        if (c == '%' || c == '#') || special.is_some() {
            let (base, length) = match special {
                Some(("<sfile>", length)) | Some(("<stack>", length)) => (current_sfile(rt), length),
                Some(("<script>", length)) => (current_script(rt), length),
                Some(("<abuf>", length)) => (autocmd_match(rt).map(|_| "1".to_owned()), length),
                Some((_, length)) => (autocmd_match(rt), length),
                None => {
                    if c == '#' {
                        (None, 1)
                    } else {
                        (host.buffer_name(), 1)
                    }
                }
            };
            let Some(base) = base else {
                return Err(Fail::Error(match c {
                    '#' => VimError::new(194, "No alternate file name to substitute for '#'"),
                    '%' => VimError::new(499, "Empty file name for '%' or '#', only works with \":p:h\""),
                    _ => VimError::new(498, "no :source file name to substitute for \"<sfile>\""),
                }));
            };
            at += length;
            let mut modifiers = String::new();
            while at + 1 < chars.len() && chars[at] == ':' && chars[at + 1].is_ascii_alphabetic() || at + 1 < chars.len() && chars[at] == ':' && matches!(chars[at + 1], '~' | '.') {
                modifiers.push(':');
                modifiers.push(chars[at + 1]);
                at += 2;
                if modifiers.ends_with(":s") || modifiers.ends_with(":g") {
                    // :s?pat?sub? and :gs?pat?sub? take their delimited arguments.
                    if modifiers.ends_with(":g") && chars.get(at) == Some(&'s') {
                        modifiers.push('s');
                        at += 1;
                    }
                    if let Some(&delimiter) = chars.get(at) {
                        modifiers.push(delimiter);
                        at += 1;
                        let mut seen = 0;
                        while at < chars.len() && seen < 2 {
                            if chars[at] == delimiter {
                                seen += 1;
                            }
                            modifiers.push(chars[at]);
                            at += 1;
                        }
                    }
                }
            }
            out.push_str(&apply_modifiers(&base, &modifiers));
            continue;
        }
        out.push(c);
        at += 1;
    }
    Ok(out)
}

fn autocmd_match(rt: &Runtime) -> Option<String> {
    rt.frames.iter().rev().find_map(|frame| match &frame.kind {
        FrameKind::Autocmd { matched, .. } => Some(matched.clone()),
        _ => None,
    })
}

/// Expand `~` and environment variables when an autocommand is defined.
pub fn expand_pattern(_rt: &Runtime, host: &dyn ScriptHost, pattern: &str) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        if c == '~' && first {
            out.push_str(&home().unwrap_or_default());
        } else if c == '$' && chars.peek().is_some_and(|c| c.is_ascii_alphabetic() || *c == '_') {
            let mut name = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    name.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            match host.getenv(&name) {
                Some(value) => out.push_str(&value),
                None => {
                    out.push('$');
                    out.push_str(&name);
                }
            }
        } else {
            out.push(c);
        }
        first = c == ',';
    }
    out
}

/// `fnamemodify()` modifiers: `:p :h :t :r :e :~ :. :s?a?b? :gs?a?b? :S`.
pub fn apply_modifiers(path: &str, modifiers: &str) -> String {
    let mut result = path.to_string();
    let mut rest = modifiers.to_string();
    while let Some(after) = rest.strip_prefix(':').map(str::to_string) {
        let mut chars = after.chars();
        let Some(modifier) = chars.next() else { break };
        rest = chars.as_str().to_string();
        match modifier {
            'p' => {
                let absolute = std::path::Path::new(&result);
                if !absolute.is_absolute() {
                    if let Ok(cwd) = std::env::current_dir() {
                        result = cwd.join(&result).to_string_lossy().into_owned();
                    }
                }
                result = normalize_path(&result);
                if std::path::Path::new(&result).is_dir() && !result.ends_with('/') && !result.ends_with('\\') {
                    result.push(std::path::MAIN_SEPARATOR);
                }
            }
            'h' => {
                let trimmed = if result.len() > 1 { result.trim_end_matches(['/', '\\']) } else { &result };
                result = match trimmed.rfind(['/', '\\']) {
                    Some(0) => "/".to_string(),
                    Some(index) => trimmed[..index].to_string(),
                    None => if trimmed.is_empty() { String::new() } else { ".".to_string() },
                };
            }
            't' => {
                let trimmed = result.trim_end_matches(['/', '\\']);
                result = trimmed.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            }
            'r' => {
                let tail_start = result.rfind(['/', '\\']).map_or(0, |n| n + 1);
                if let Some(dot) = result[tail_start..].rfind('.') {
                    if dot > 0 {
                        result.truncate(tail_start + dot);
                    }
                }
            }
            'e' => {
                let tail_start = result.rfind(['/', '\\']).map_or(0, |n| n + 1);
                result = match result[tail_start..].rfind('.') {
                    Some(dot) if dot > 0 => result[tail_start + dot + 1..].to_string(),
                    _ => String::new(),
                };
            }
            '~' => {
                if let Some(home) = home() {
                    if let Some(stripped) = result.strip_prefix(&home) {
                        if stripped.is_empty() || stripped.starts_with(['/', '\\']) {
                            result = format!("~{stripped}");
                        }
                    }
                }
            }
            '.' => {
                if let Ok(cwd) = std::env::current_dir() {
                    let cwd = cwd.to_string_lossy().into_owned();
                    if let Some(stripped) = result.strip_prefix(&cwd) {
                        if let Some(stripped) = stripped.strip_prefix(['/', '\\']) {
                            result = stripped.to_string();
                        }
                    }
                }
            }
            'S' => result = format!("'{}'", result.replace('\'', "'\\''")),
            's' | 'g' => {
                let global = modifier == 'g';
                if global {
                    if let Some(stripped) = rest.strip_prefix('s') {
                        rest = stripped.to_string();
                    }
                }
                let mut chars = rest.chars();
                let Some(delimiter) = chars.next() else { break };
                let body: String = chars.collect();
                let mut parts = body.splitn(3, delimiter);
                let pattern = parts.next().unwrap_or("").to_string();
                let replacement = parts.next().unwrap_or("").to_string();
                let remainder = parts.next().unwrap_or("").to_string();
                result = if global { result.replace(&pattern, &replacement) } else { result.replacen(&pattern, &replacement, 1) };
                rest = if remainder.starts_with(':') { remainder } else { String::new() };
            }
            _ => {}
        }
    }
    result
}

fn normalize_path(path: &str) -> String {
    let separator = if path.contains('\\') && !path.contains('/') { '\\' } else { '/' };
    let absolute = path.starts_with(['/', '\\']);
    let prefix_end = if path.len() >= 2 && path.as_bytes()[1] == b':' { 2 } else { 0 };
    let mut parts: Vec<&str> = Vec::new();
    for part in path[prefix_end..].split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = path[..prefix_end].to_string();
    if absolute || prefix_end > 0 {
        out.push(separator);
    }
    out.push_str(&parts.join(&separator.to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_lines_join_and_comments_drop() {
        let lines = script_lines(b"let x = [\n      \\ 1,\n      \"\\ comment\n      \\ 2]\necho x\r\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text.as_bytes(), b"let x = [ 1, 2]");
        assert_eq!(lines[0].lnum, 1);
        assert_eq!(lines[1].text.as_bytes(), b"echo x");
        assert_eq!(lines[1].lnum, 5);
    }

    #[test]
    fn file_modifiers_follow_fnamemodify() {
        assert_eq!(apply_modifiers("/a/b/c.txt", ":h"), "/a/b");
        assert_eq!(apply_modifiers("/a/b/c.txt", ":t"), "c.txt");
        assert_eq!(apply_modifiers("/a/b/c.txt", ":t:r"), "c");
        assert_eq!(apply_modifiers("/a/b/c.tar.gz", ":e"), "gz");
        assert_eq!(apply_modifiers("/a/b/c.tar.gz", ":r:r"), "/a/b/c");
        assert_eq!(apply_modifiers("c.txt", ":h"), ".");
        assert_eq!(apply_modifiers("/a/b/c.txt", ":s?b?x?"), "/a/x/c.txt");
        assert_eq!(apply_modifiers("/a/b/c.txt", ":gs?/?\\\\?"), "\\\\a\\\\b\\\\c.txt");
    }
}
