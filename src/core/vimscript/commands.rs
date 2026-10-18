//! Parsing and executing one Ex command, like Vim's `do_one_cmd()`.
//! Script commands are implemented here; other commands are separated from
//! their `|` successors and comments and passed to the host.
use super::autocmd::{self, Autocmd, AutocmdPattern};
use super::error::{EvalResult, Exception, Fail, VimError};
use super::exec::{
    CommandOutcome, Cond, CondKind, ForItems, ForState, Frame, FrameKind, HostCommand, Message, Modifiers, Next,
    Pending, Runtime, ScriptHost, Unwind,
};
use super::expr::{is_name_char, is_white, skip_white, Eval};
use super::state::{CommandArgs, UserCommand};
use super::value::{VStr, Value};
use std::sync::Arc;

const MAX_SOURCE_DEPTH: usize = 16;

fn outcome(next: Next, result: EvalResult<()>) -> CommandOutcome {
    CommandOutcome { next, result }
}

fn error(next: Next, error: VimError) -> CommandOutcome {
    outcome(next, Err(Fail::Error(error)))
}

/// Vim's `skip_range()`: the end of an optional line range.
pub fn skip_range(src: &[u8], mut at: usize) -> usize {
    loop {
        at = skip_white(src, at);
        let Some(&byte) = src.get(at) else { return at };
        match byte {
            b'0'..=b'9' | b'.' | b'$' | b'%' | b'+' | b'-' | b',' | b';' => at += 1,
            b'\'' => {
                at += 1;
                if at < src.len() {
                    at += 1;
                }
            }
            b'/' | b'?' => {
                let delimiter = byte;
                at += 1;
                while at < src.len() && src[at] != delimiter {
                    if src[at] == b'\\' && at + 1 < src.len() {
                        at += 1;
                    }
                    at += 1;
                }
                if at < src.len() {
                    at += 1;
                }
            }
            b'\\' if matches!(src.get(at + 1), Some(b'/' | b'?' | b'&')) => at += 2,
            _ => return at,
        }
    }
}

/// Split a TRLBAR command's argument at an unescaped `|` (and a comment when
/// allowed). Returns the cleaned argument and the next command position.
pub fn split_bar(src: &[u8], start: usize, comment: bool, keep_escapes: bool) -> (Vec<u8>, Next) {
    let mut out = Vec::new();
    let mut at = start;
    while at < src.len() {
        let byte = src[at];
        if byte == 0x16 {
            // Ctrl-V: the next character is literal.
            if keep_escapes {
                out.push(byte);
            }
            if at + 1 < src.len() {
                out.push(src[at + 1]);
            }
            at += 2;
            continue;
        }
        if byte == b'|' || (byte == b'"' && comment) {
            if at > start && src[at - 1] == b'\\' {
                // "\|" and "\"" are literal; the backslash is removed.
                out.pop();
                out.push(byte);
                at += 1;
                continue;
            }
            // Commands without trailing comments (maps) keep trailing
            // white space, as Vim's separate_nextcmd() does.
            if comment {
                trim_trailing(&mut out);
            }
            if byte == b'|' {
                return (out, Next::Column(at + 1));
            }
            return (out, Next::NextLine);
        }
        out.push(byte);
        at += 1;
    }
    if comment {
        trim_trailing(&mut out);
    }
    (out, Next::NextLine)
}

fn trim_trailing(bytes: &mut Vec<u8>) {
    while bytes.last().is_some_and(|b| is_white(*b)) {
        // Keep a white space escaped by a backslash.
        if bytes.len() >= 2 && bytes[bytes.len() - 2] == b'\\' {
            break;
        }
        bytes.pop();
    }
}

/// After an expression: accept the end, `|`, or a comment when allowed.
fn after_expression(src: &[u8], at: usize, comment: bool) -> Result<Next, VimError> {
    let at = skip_white(src, at);
    match src.get(at) {
        None => Ok(Next::NextLine),
        Some(b'|') => Ok(Next::Column(at + 1)),
        Some(b'"') if comment => Ok(Next::NextLine),
        Some(_) => Err(VimError::trailing(&src[at..])),
    }
}

/// Where to continue after an expression failed at `at`: Vim's
/// set_nextcmd() finds a next command only when `|` directly follows the
/// failure point.
pub(crate) fn next_after_failure(src: &[u8], at: usize) -> Next {
    let at = skip_white(src, at);
    if src.get(at) == Some(&b'|') { Next::Column(at + 1) } else { Next::NextLine }
}

/// Scan to the next unquoted `|` without evaluating; used when an
/// expression cannot be parsed in a skipped block.
fn skip_to_bar(src: &[u8], mut at: usize) -> Next {
    let mut quote: Option<u8> = None;
    while at < src.len() {
        let byte = src[at];
        match quote {
            Some(q) => {
                if byte == b'\\' && q == b'"' {
                    at += 1;
                } else if byte == q {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'|' => return Next::Column(at + 1),
                _ => {}
            },
        }
        at += 1;
    }
    Next::NextLine
}

/// Modifiers recognized before a command name: (full name, minimum length).
const MODIFIERS: &[(&str, usize)] = &[
    ("aboveleft", 3), ("belowright", 3), ("botright", 2), ("browse", 3), ("confirm", 4),
    ("filter", 4), ("hide", 3), ("horizontal", 3), ("keepalt", 5), ("keepjumps", 5),
    ("keepmarks", 3), ("keeppatterns", 5), ("leftabove", 5), ("legacy", 3), ("lockmarks", 3),
    ("noautocmd", 3), ("noswapfile", 3), ("rightbelow", 6), ("sandbox", 3), ("silent", 3),
    ("tab", 3), ("topleft", 2), ("unsilent", 3), ("verbose", 4), ("vertical", 4), ("vim9cmd", 4),
];

fn word_at(src: &[u8], at: usize) -> &[u8] {
    let mut end = at;
    while end < src.len() && src[end].is_ascii_alphabetic() {
        end += 1;
    }
    &src[at..end]
}

fn matches_abbreviation(word: &[u8], full: &str, minimum: usize) -> bool {
    word.len() >= minimum && full.as_bytes().starts_with(word)
}

/// Parse command modifiers at `at`, returning their position after them.
fn parse_modifiers(src: &[u8], mut at: usize, modifiers: &mut Modifiers) -> Result<usize, VimError> {
    loop {
        at = skip_white(src, at);
        while src.get(at) == Some(&b':') {
            at = skip_white(src, at + 1);
        }
        // A count before :verbose / :tab.
        let count_start = at;
        while src.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let count = (at > count_start).then(|| String::from_utf8_lossy(&src[count_start..at]).parse::<i64>().unwrap_or(1));
        let word = word_at(src, at);
        let after = at + word.len();
        let Some(&(full, _)) = MODIFIERS.iter().find(|(full, minimum)| matches_abbreviation(word, full, *minimum)) else {
            return Ok(count_start);
        };
        // A modifier must be followed by a command (or a bang for silent).
        let mut next = after;
        let bang = src.get(next) == Some(&b'!');
        if bang {
            next += 1;
        }
        let rest = skip_white(src, next);
        if full == "hide" || full == "tab" {
            // :hide and :tab alone are commands.
            if rest >= src.len() || matches!(src[rest], b'|' | b'"') {
                return Ok(count_start);
            }
        } else if count.is_some() && full != "verbose" && full != "tab" {
            return Ok(count_start);
        }
        match full {
            "silent" => {
                modifiers.silent = true;
                if bang {
                    modifiers.silent_errors = true;
                }
            }
            "unsilent" => modifiers.unsilent = true,
            "verbose" => modifiers.verbose = Some(count.unwrap_or(1)),
            "noautocmd" => modifiers.noautocmd = true,
            "sandbox" => modifiers.sandbox = true,
            "vertical" => modifiers.vertical = true,
            "horizontal" => modifiers.horizontal = true,
            "keepjumps" => modifiers.keepjumps = true,
            "keeppatterns" => modifiers.keeppatterns = true,
            "keepmarks" => modifiers.keepmarks = true,
            "keepalt" => modifiers.keepalt = true,
            "lockmarks" => modifiers.lockmarks = true,
            "confirm" => modifiers.confirm = true,
            "browse" => modifiers.browse = true,
            "legacy" => {}
            "vim9cmd" => return Err(VimError::plain("Viem does not support Vim9 script: vim9cmd")),
            "filter" => {
                // :filter[!] {pattern} {command}; the pattern is ignored.
                let mut at2 = rest;
                if let Some(&delimiter) = src.get(at2) {
                    if !delimiter.is_ascii_alphanumeric() && delimiter != b'"' && delimiter != b'|' {
                        at2 += 1;
                        while at2 < src.len() && src[at2] != delimiter {
                            if src[at2] == b'\\' {
                                at2 += 1;
                            }
                            at2 += 1;
                        }
                        at = at2 + 1;
                        continue;
                    }
                    while at2 < src.len() && !is_white(src[at2]) {
                        at2 += 1;
                    }
                }
                at = at2;
                continue;
            }
            "noswapfile" | "hide" => {}
            other => modifiers.placement.push(other.to_string()),
        }
        at = next;
        let _ = rest;
    }
}

/// Engine-implemented commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScriptCommand {
    Let,
    Const,
    Unlet,
    Lockvar,
    Unlockvar,
    If,
    Elseif,
    Else,
    Endif,
    While,
    Endwhile,
    For,
    Endfor,
    Break,
    Continue,
    Try,
    Catch,
    Finally,
    Endtry,
    Throw,
    Function,
    Endfunction,
    Delfunction,
    Return,
    Call,
    Execute,
    Echo,
    Echon,
    Echomsg,
    Echoerr,
    Echohl,
    Eval,
    Finish,
    Source,
    Runtime,
    Packadd,
    Vim9script,
    Scriptencoding,
    Augroup,
    Autocmd,
    Doautocmd,
    Command,
    Delcommand,
    Comclear,
    Messages,
    Redir,
    Redraw,
    Language,
    Behave,
}

fn script_command(full: &str) -> Option<ScriptCommand> {
    use ScriptCommand::*;
    Some(match full {
        "let" => Let,
        "const" => Const,
        "unlet" => Unlet,
        "lockvar" => Lockvar,
        "unlockvar" => Unlockvar,
        "if" => If,
        "elseif" => Elseif,
        "else" => Else,
        "endif" => Endif,
        "while" => While,
        "endwhile" => Endwhile,
        "for" => For,
        "endfor" => Endfor,
        "break" => Break,
        "continue" => Continue,
        "try" => Try,
        "catch" => Catch,
        "finally" => Finally,
        "endtry" => Endtry,
        "throw" => Throw,
        "function" => Function,
        "endfunction" => Endfunction,
        "delfunction" => Delfunction,
        "return" => Return,
        "call" => Call,
        "execute" => Execute,
        "echo" => Echo,
        "echon" => Echon,
        "echomsg" | "echoconsole" | "echowindow" => Echomsg,
        "echoerr" => Echoerr,
        "echohl" => Echohl,
        "eval" => Eval,
        "finish" => Finish,
        "source" => Source,
        "runtime" => Runtime,
        "packadd" | "packloadall" => Packadd,
        "vim9script" => Vim9script,
        "scriptencoding" => Scriptencoding,
        "augroup" => Augroup,
        "autocmd" => Autocmd,
        "doautocmd" | "doautoall" => Doautocmd,
        "command" => Command,
        "delcommand" => Delcommand,
        "comclear" => Comclear,
        "messages" => Messages,
        "redir" => Redir,
        "redraw" | "redrawstatus" | "redrawtabline" => Redraw,
        "language" => Language,
        "behave" => Behave,
        _ => return None,
    })
}

/// Whether the engine implements the full command name `full`.
pub(crate) fn engine_command(full: &str) -> bool {
    script_command(full).is_some()
}

fn structural(command: ScriptCommand) -> bool {
    use ScriptCommand::*;
    matches!(command, If | Elseif | Else | Endif | While | Endwhile | For | Endfor | Try | Catch | Finally | Endtry | Function)
}

/// Parsed command header.
struct Header {
    modifiers: Modifiers,
    /// Start of the command (after modifiers), used for messages.
    start: usize,
    range: Vec<u8>,
    name: Vec<u8>,
    bang: bool,
    /// End of the name and bang, before any white space.
    name_end: usize,
    /// Argument start, after white space.
    arg: usize,
}

fn parse_header(src: &[u8], col: usize) -> Result<Option<Header>, VimError> {
    let mut modifiers = Modifiers::default();
    let at = parse_modifiers(src, col, &mut modifiers)?;
    let mut at = skip_white(src, at);
    while src.get(at) == Some(&b':') {
        at = skip_white(src, at + 1);
    }
    let start = at;
    if at >= src.len() || src[at] == b'"' {
        return Ok(None);
    }
    let range_end = skip_range(src, at);
    let range = trim(&src[at..range_end]).to_vec();
    at = range_end;
    while src.get(at) == Some(&b':') {
        at += 1;
    }
    at = skip_white(src, at);
    let name_start = at;
    if at < src.len() && src[at].is_ascii_alphabetic() {
        let upper = src[at].is_ascii_uppercase();
        while at < src.len() && (src[at].is_ascii_alphabetic() || (upper && src[at].is_ascii_digit())) {
            at += 1;
        }
        // `:s` and friends directly followed by a pattern delimiter.
    } else if at < src.len() && matches!(src[at], b'&' | b'~' | b'<' | b'>' | b'!' | b'@' | b'*' | b'=' | b'#') {
        at += 1;
    }
    let name = src[name_start..at].to_vec();
    let mut bang = false;
    if !name.is_empty() && src.get(at) == Some(&b'!') && !matches!(name.as_slice(), b"!" | b"&" | b"~" | b"<" | b">") {
        bang = true;
        at += 1;
    }
    let arg = skip_white(src, at);
    Ok(Some(Header { modifiers, start, range, name, bang, name_end: at, arg }))
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !is_white(*b)).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !is_white(*b)).map_or(start, |end| end + 1);
    &bytes[start..end]
}

/// Execute (or, when `executing` is false, skip over) the command at `col`.
pub(crate) fn execute_command(rt: &mut Runtime, host: &mut dyn ScriptHost, line: &VStr, col: usize, executing: bool) -> CommandOutcome {
    let src = line.as_bytes();
    let header = match parse_header(src, col) {
        Ok(Some(header)) => header,
        Ok(None) => return outcome(Next::NextLine, Ok(())),
        Err(err) => return if executing { error(Next::NextLine, err) } else { outcome(Next::NextLine, Ok(())) },
    };
    // An empty command: ":|" or only a range.
    if header.name.is_empty() {
        let at = header.arg;
        let next = match src.get(at) {
            None => Next::NextLine,
            Some(b'|') => Next::Column(at + 1),
            Some(b'"') => Next::NextLine,
            Some(_) => {
                return if executing {
                    error(Next::NextLine, VimError::new(492, format!("Not an editor command: {}", String::from_utf8_lossy(&src[header.start..]))))
                } else {
                    outcome(Next::NextLine, Ok(()))
                }
            }
        };
        if !executing || header.range.is_empty() {
            return outcome(next, Ok(()));
        }
        let command = HostCommand {
            text: String::from_utf8_lossy(&header.range).into_owned(),
            name: None,
            bang: false,
            range: String::from_utf8_lossy(&header.range).into_owned(),
            argument: String::new(),
            modifiers: header.modifiers,
            sid: rt.current_sid(),
        };
        rt.current_command.clear();
        let result = host.execute(rt, &command);
        return outcome(next, result);
    }
    let name_text = String::from_utf8_lossy(&header.name).into_owned();
    // User commands take precedence for capitalized names.
    if header.name[0].is_ascii_uppercase() {
        match find_user_command(rt, &name_text) {
            Ok(Some(command)) => return run_user_command(rt, host, src, &header, command, executing),
            Ok(None) => {}
            Err(err) => return if executing { error(Next::NextLine, err) } else { outcome(skip_to_bar(src, header.arg), Ok(())) },
        }
    }
    let Some(full) = super::excmds::lookup(&name_text) else {
        if !executing {
            return outcome(Next::NextLine, Ok(()));
        }
        return error(Next::NextLine, VimError::new(492, format!("Not an editor command: {}", String::from_utf8_lossy(&src[header.start..]))));
    };
    if let Some(command) = script_command(full) {
        if !executing && !structural(command) {
            return outcome(skip_script_command(rt, host, src, &header, command), Ok(()));
        }
        rt.current_command = full.to_string();
        let silent_errors = header.modifiers.silent_errors;
        let silent = header.modifiers.silent && !header.modifiers.unsilent;
        enter_silence(rt, silent, silent_errors);
        let before = rt.frames.len();
        let noautocmd = header.modifiers.noautocmd;
        let previous = rt.frames.last().map(|frame| frame.noautocmd).unwrap_or(false);
        if noautocmd { if let Some(frame) = rt.frames.last_mut() { frame.noautocmd = true; } }
        let mut result = run_script_command(rt, host, src, &header, command, executing);
        if noautocmd {
            if let Some(frame) = rt.frames.get_mut(before.saturating_sub(1)) { frame.noautocmd = previous; }
            for frame in rt.frames.iter_mut().skip(before) { frame.noautocmd = true; }
            rt.queued_events.clear();
        }
        if silent_errors {
            result.result = silence_error(rt, result.result);
        }
        leave_silence(rt, before, silent, silent_errors);
        append_command(&mut result, src, &header);
        return result;
    }
    // Host commands: find where the argument ends.
    let (argument, next) = host_argument(src, &header, full);
    if !executing {
        return outcome(next, Ok(()));
    }
    let mut argument = argument;
    if super::excmds::is_map_like(full) {
        argument = expand_leaders(rt, host, &argument);
    }
    let command = host_command(src, &header, full, &argument, rt.current_sid());
    rt.current_command = full.to_string();
    let silent_errors = header.modifiers.silent_errors;
    let silent = header.modifiers.silent && !header.modifiers.unsilent;
    enter_silence(rt, silent, silent_errors);
    let before = rt.frames.len();
    let result = if rt.restricted && restricted_command(full) {
        Err(Fail::Error(VimError::plain(format!("startup cannot run :{full}; use it in a mapping or after startup"))))
    } else {
        host.execute(rt, &command)
    };
    let result = if silent_errors { silence_error(rt, result) } else { result };
    leave_silence(rt, before, silent, silent_errors);
    outcome(next, result)
}

/// `:silent!`: an error sets v:errmsg and is otherwise dropped while the
/// modifier is in effect; it neither reports, becomes an exception, nor
/// skips the rest of the line by itself.
fn silence_error(rt: &mut Runtime, result: EvalResult<()>) -> EvalResult<()> {
    match result {
        Err(Fail::Error(error)) => {
            rt.state.set_vvar("errmsg", error.text().into());
            Ok(())
        }
        other => other,
    }
}

/// The command given to the host: range, name and bang as written, the
/// original white space, then the argument without bars or comments.
fn host_command(src: &[u8], header: &Header, full: &'static str, argument: &[u8], sid: usize) -> HostCommand {
    // The host's Ex parser applies window orientation itself.
    let mut text = if header.modifiers.vertical {
        b"vertical ".to_vec()
    } else if header.modifiers.horizontal {
        b"horizontal ".to_vec()
    } else {
        Vec::new()
    };
    text.extend_from_slice(&header.range);
    text.extend_from_slice(&header.name);
    if header.bang {
        text.push(b'!');
    }
    if !argument.is_empty() {
        text.extend_from_slice(&src[header.name_end.min(header.arg)..header.arg.min(src.len())]);
        text.extend_from_slice(argument);
    }
    HostCommand {
        text: String::from_utf8_lossy(&text).into_owned(),
        name: Some(full),
        bang: header.bang,
        range: String::from_utf8_lossy(&header.range).into_owned(),
        argument: String::from_utf8_lossy(argument).into_owned(),
        modifiers: header.modifiers.clone(),
        sid,
    }
}

/// The single host command `line` consists of when running it needs no
/// script processing: no modifiers, script or user commands, commands after
/// `|`, or mapping-key expansion. Callers may execute such a line directly;
/// the result is exactly the command the engine would give the host.
pub fn single_host_command(state: &super::ScriptState, line: &str) -> Option<HostCommand> {
    let src = line.as_bytes();
    let header = parse_header(src, 0).ok()??;
    let orientation_only = Modifiers { vertical: false, horizontal: false, ..header.modifiers.clone() };
    if orientation_only != Modifiers::default() {
        return None;
    }
    if header.name.is_empty() {
        if header.range.is_empty() || header.arg < src.len() {
            return None;
        }
        let range = String::from_utf8_lossy(&header.range).into_owned();
        return Some(HostCommand { text: range.clone(), name: None, bang: false, range, argument: String::new(), modifiers: Modifiers::default(), sid: 0 });
    }
    let name = String::from_utf8_lossy(&header.name);
    if header.name[0].is_ascii_uppercase() && (state.user_command(&name).is_some() || !state.user_commands_with_prefix(&name).is_empty()) {
        return None;
    }
    let Some(full) = super::excmds::lookup(&name) else {
        // Not a Vim command: the host reports it with its own diagnostics.
        let (argument, next) = split_bar(src, header.arg, true, false);
        if !matches!(next, Next::NextLine) {
            return None;
        }
        let mut command = host_command(src, &header, "", &argument, 0);
        command.name = None;
        return Some(command);
    };
    if script_command(full).is_some() || super::excmds::is_map_like(full) || matches!(full, "colorscheme" | "setfiletype" | "set" | "setlocal" | "setglobal") {
        return None;
    }
    let (argument, next) = host_argument(src, &header, full);
    if !matches!(next, Next::NextLine) {
        return None;
    }
    // A file type change fires FileType autocommands through the engine.
    if matches!(full, "set" | "setlocal" | "setglobal")
        && String::from_utf8_lossy(&argument).split_whitespace().any(|part| part.starts_with("ft=") || part.starts_with("filetype="))
    {
        return None;
    }
    Some(host_command(src, &header, full, &argument, 0))
}

fn enter_silence(rt: &mut Runtime, silent: bool, silent_errors: bool) {
    if silent_errors {
        rt.emsg_silent += 1;
    }
    if silent {
        rt.msg_silent += 1;
    }
}

/// Keep `:silent` in effect for frames the command pushed (a called
/// function, an `:execute` string, ...) until they finish.
fn leave_silence(rt: &mut Runtime, before: usize, silent: bool, silent_errors: bool) {
    if !(silent || silent_errors) {
        return;
    }
    if rt.transfer_silence(before, silent, silent_errors) {
        return;
    }
    if silent_errors {
        rt.emsg_silent -= 1;
    }
    if silent {
        rt.msg_silent -= 1;
    }
}

/// Errors Vim reports through `eap->errmsg` end with ": {command}".
fn append_command(outcome: &mut CommandOutcome, src: &[u8], header: &Header) {
    const CODES: &[u32] = &[170, 171, 193, 477, 481, 580, 581, 582, 583, 584, 586, 587, 588, 600, 602, 603, 604, 606, 607, 732, 733];
    if let Err(Fail::Error(error)) = &mut outcome.result {
        if CODES.contains(&error.code) || (error.code == 488 && error.message.starts_with("Trailing characters") && header.name.first().is_some_and(u8::is_ascii_uppercase)) || (error.code == 471 && header.name.first().is_some_and(u8::is_ascii_uppercase)) {
            let command = split_bar(src, header.start, false, false).0;
            error.message.push_str(": ");
            error.message.push_str(&String::from_utf8_lossy(trim(&command)));
        }
    }
}

/// Commands startup configuration may not run: they edit text or act on
/// files and windows.
fn restricted_command(full: &str) -> bool {
    !matches!(
        full,
        "set" | "setlocal" | "setglobal" | "nohlsearch" | "colorscheme" | "highlight" | "syntax" | "filetype"
            | "map" | "nmap" | "vmap" | "xmap" | "smap" | "omap" | "imap" | "lmap" | "cmap" | "noremap"
            | "nnoremap" | "vnoremap" | "xnoremap" | "snoremap" | "onoremap" | "inoremap" | "lnoremap"
            | "cnoremap" | "unmap" | "nunmap" | "vunmap" | "xunmap" | "sunmap" | "ounmap" | "iunmap"
            | "lunmap" | "cunmap" | "mapclear" | "nmapclear" | "vmapclear" | "xmapclear" | "smapclear"
            | "omapclear" | "imapclear" | "lmapclear" | "cmapclear" | "abbreviate" | "noreabbrev"
            | "iabbrev" | "inoreabbrev" | "cabbrev" | "cnoreabbrev" | "unabbreviate" | "iunabbrev"
            | "cunabbrev" | "abclear" | "iabclear" | "cabclear" | "setfiletype" | "winpos" | "winsize"
            | "simalt" | "map!" | "noremap!" | "unmap!" | "mapclear!" | "digraphs" | "sign" | "match"
            | "2match" | "3match" | "compiler" | "helptags" | "menu" | "amenu" | "nmenu" | "vmenu"
            | "xmenu" | "smenu" | "omenu" | "imenu" | "cmenu" | "tmenu" | "noremenu" | "anoremenu"
            | "nnoremenu" | "vnoremenu" | "xnoremenu" | "snoremenu" | "onoremenu" | "inoremenu"
            | "cnoremenu" | "unmenu" | "aunmenu" | "nunmenu" | "vunmenu" | "xunmenu" | "sunmenu"
            | "ounmenu" | "iunmenu" | "cunmenu" | "tunmenu" | "macmenu" | "popup" | "tearoff"
    )
}

/// The argument of a host command and where the next command starts.
fn host_argument(src: &[u8], header: &Header, full: &str) -> (Vec<u8>, Next) {
    let at = header.arg;
    // Commands that see `|` as their argument.
    let sees_bar = super::excmds::sees_bar(full)
        || (full == "read" && src.get(at) == Some(&b'!'))
        || (full == "write" && src.get(at) == Some(&b'!'));
    if sees_bar {
        let mut argument = src[at.min(src.len())..].to_vec();
        if full != "normal" {
            trim_trailing(&mut argument);
        }
        return (argument, Next::NextLine);
    }
    match full {
        "substitute" | "&" | "~" | "smagic" | "snomagic" => substitute_argument(src, at),
        "sort" => sort_argument(src, at),
        _ => split_bar(src, at, super::excmds::allows_comment(full), super::excmds::is_map_like(full)),
    }
}

fn substitute_argument(src: &[u8], mut at: usize) -> (Vec<u8>, Next) {
    let start = at;
    if let Some(&delimiter) = src.get(at) {
        if !delimiter.is_ascii_alphanumeric() && !is_white(delimiter) && !matches!(delimiter, b'"' | b'|' | b'\\') {
            // pattern, replacement
            for _ in 0..2 {
                at += 1;
                while at < src.len() && src[at] != delimiter {
                    if src[at] == b'\\' && at + 1 < src.len() {
                        at += 1;
                    }
                    at += 1;
                }
                if at >= src.len() {
                    return (src[start..].to_vec(), Next::NextLine);
                }
            }
            at += 1;
        }
    }
    // Flags and count.
    while at < src.len() && b"&cegiInp#lr".contains(&src[at]) {
        at += 1;
    }
    at = skip_white(src, at);
    while at < src.len() && src[at].is_ascii_digit() {
        at += 1;
    }
    let end = skip_white(src, at);
    let mut argument = src[start..at].to_vec();
    trim_trailing(&mut argument);
    match src.get(end) {
        Some(b'|') => (argument, Next::Column(end + 1)),
        Some(b'"') | None => (argument, Next::NextLine),
        // Trailing text is passed on; the host reports it.
        Some(_) => (src[start..].to_vec(), Next::NextLine),
    }
}

fn sort_argument(src: &[u8], start: usize) -> (Vec<u8>, Next) {
    let mut at = start;
    while at < src.len() {
        let byte = src[at];
        if byte == b'|' {
            let mut argument = src[start..at].to_vec();
            trim_trailing(&mut argument);
            return (argument, Next::Column(at + 1));
        }
        if !byte.is_ascii_alphanumeric() && !is_white(byte) && byte != b'"' {
            // A pattern.
            at += 1;
            while at < src.len() && src[at] != byte {
                if src[at] == b'\\' {
                    at += 1;
                }
                at += 1;
            }
        }
        at += 1;
    }
    let mut argument = src[start.min(src.len())..].to_vec();
    trim_trailing(&mut argument);
    (argument, Next::NextLine)
}

/// `<Leader>`, `<LocalLeader>` and `<SID>` are expanded when a mapping is
/// defined, as in Vim.
fn expand_leaders(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8]) -> Vec<u8> {
    let leader = |rt: &mut Runtime, host: &mut dyn ScriptHost, name: &[u8]| -> Vec<u8> {
        match rt.get_variable(host, name) {
            Ok(Value::String(text)) if !text.is_empty() => text.as_bytes().to_vec(),
            Ok(value @ Value::Number(_)) => value.to_bytes().unwrap_or_default(),
            _ => b"\\".to_vec(),
        }
    };
    let mut out = Vec::with_capacity(argument.len());
    let mut at = 0;
    while at < argument.len() {
        let rest = &argument[at..];
        let starts = |tag: &[u8]| rest.len() >= tag.len() && rest[..tag.len()].eq_ignore_ascii_case(tag);
        if starts(b"<leader>") {
            out.extend(leader(rt, host, b"g:mapleader"));
            at += 8;
        } else if starts(b"<localleader>") {
            out.extend(leader(rt, host, b"g:maplocalleader"));
            at += 13;
        } else if starts(b"<sid>") {
            out.extend_from_slice(format!("<SNR>{}_", rt.current_sid()).as_bytes());
            at += 5;
        } else {
            out.push(argument[at]);
            at += 1;
        }
    }
    out
}

/// Find where a script command ends without executing it.
fn skip_script_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header, command: ScriptCommand) -> Next {
    use ScriptCommand::*;
    match command {
        Let | Const | Throw | Return | Eval | Execute | Echo | Echon | Echomsg | Echoerr | Call => {
            if command == Let || command == Const {
                return super::let_cmd::skip_let(rt, host, src, header.arg);
            }
            let comment = !matches!(command, Execute | Echo | Echon | Echomsg | Echoerr);
            let mut at = header.arg;
            loop {
                at = skip_white(src, at);
                if at >= src.len() || src[at] == b'|' || (comment && src[at] == b'"') {
                    break;
                }
                let mut eval = super::expr::Eval::new(rt, host, src, at, false);
                if eval.eval1().is_err() || eval.pos == at {
                    return skip_to_bar(src, header.arg);
                }
                at = eval.pos;
                if !matches!(command, Execute | Echo | Echon | Echomsg | Echoerr) {
                    break;
                }
            }
            after_expression(src, at, comment).unwrap_or_else(|_| skip_to_bar(src, at))
        }
        Autocmd | Command => Next::NextLine,
        _ => split_bar(src, header.arg, true, false).1,
    }
}

fn run_script_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header, command: ScriptCommand, executing: bool) -> CommandOutcome {
    use ScriptCommand::*;
    let at = header.arg;
    match command {
        Let | Const => super::let_cmd::let_command(rt, host, src, at, command == Const),
        Unlet | Lockvar | Unlockvar => {
            let (argument, next) = split_bar(src, at, true, false);
            let result = match command {
                Unlet => super::let_cmd::unlet_command(rt, host, &argument, header.bang),
                Lockvar => super::let_cmd::lock_command(rt, host, &argument, true, header.bang),
                _ => super::let_cmd::lock_command(rt, host, &argument, false, header.bang),
            };
            outcome(next, result)
        }
        If => cmd_if(rt, host, src, at, executing),
        Elseif => cmd_elseif(rt, host, src, at),
        Else => cmd_else(rt, src, at),
        Endif => cmd_endif(rt, src, at),
        While => cmd_while(rt, host, src, header, executing),
        For => cmd_for(rt, host, src, header, executing),
        Endwhile | Endfor => cmd_endloop(rt, src, at, command == Endfor),
        Break | Continue => {
            let next = split_bar(src, at, true, false).1;
            let action = if command == Break { Unwind::Break } else { Unwind::Continue };
            outcome(next, rt.unwind(Some(host), action))
        }
        Try => cmd_try(rt, src, at, executing),
        Catch => cmd_catch(rt, host, src, at),
        Finally => cmd_finally(rt, src, at),
        Endtry => cmd_endtry(rt, host, src, at),
        Throw => cmd_throw(rt, host, src, at),
        Function => super::function_cmd::function_command(rt, host, src, header.arg, header.bang, executing),
        Endfunction => error(Next::NextLine, VimError::new(193, ":endfunction not inside a function")),
        Delfunction => {
            let (argument, next) = split_bar(src, at, true, false);
            outcome(next, super::function_cmd::delfunction(rt, host, &argument, header.bang))
        }
        Return => cmd_return(rt, host, src, at),
        Call => cmd_call(rt, host, src, header),
        Execute => cmd_execute(rt, host, src, at),
        Echo | Echon | Echomsg | Echoerr => cmd_echo(rt, host, src, at, command),
        Echohl => outcome(split_bar(src, at, true, false).1, Ok(())),
        Eval => {
            let mut eval = super::expr::Eval::new(rt, host, src, at, true);
            let result = eval.eval1();
            let end = eval.pos;
            match result {
                Ok(_) => match after_expression(src, end, true) {
                    Ok(next) => outcome(next, Ok(())),
                    Err(err) => error(Next::NextLine, err),
                },
                Err(fail) => outcome(skip_to_bar(src, end), Err(fail)),
            }
        }
        Finish => {
            let next = split_bar(src, at, true, false).1;
            if !rt.frames.iter().any(|frame| matches!(frame.kind, FrameKind::Script { .. })) {
                return error(next, VimError::new(168, ":finish used outside of a sourced file"));
            }
            outcome(next, rt.unwind(Some(host), Unwind::Finish))
        }
        Source => {
            let (argument, next) = split_bar(src, at, true, false);
            if header.bang {
                return error(next, VimError::plain("Viem does not support :source! (Normal-mode command files)"));
            }
            outcome(next, source_file(rt, host, &argument))
        }
        Runtime | Packadd => outcome(split_bar(src, at, true, false).1, Ok(())),
        Vim9script => {
            let next = split_bar(src, at, true, false).1;
            if matches!(rt.frames.last().map(|frame| &frame.kind), Some(FrameKind::Script { .. })) {
                // Stop the file: Viem implements legacy script only.
                let result = Err(Fail::Error(VimError::plain("Viem does not support Vim9 script; the rest of this file was skipped")));
                if let Some(frame) = rt.current_frame_mut() {
                    frame.finished = true;
                    frame.line = frame.lines.len();
                    frame.cstack.clear();
                }
                return outcome(Next::Unchanged, result);
            }
            error(next, VimError::new(1038, "\"vim9script\" can only be used in a script"))
        }
        Scriptencoding => {
            let (argument, next) = split_bar(src, at, true, false);
            let encoding = String::from_utf8_lossy(&argument).to_ascii_lowercase();
            if encoding.is_empty() || matches!(encoding.as_str(), "utf-8" | "utf8" | "latin1" | "ascii" | "us-ascii") {
                outcome(next, Ok(()))
            } else {
                error(next, VimError::plain(format!("Viem reads scripts as UTF-8; scriptencoding {encoding} is not supported")))
            }
        }
        Augroup => {
            let (argument, next) = split_bar(src, at, true, false);
            let name = String::from_utf8_lossy(&argument).into_owned();
            if name.is_empty() {
                let table = rt.state.autocmds();
                let names: Vec<String> = (1..64).filter_map(|group| table.group_name(group).map(str::to_string)).collect();
                drop(table);
                rt.emit(host, Message::Output(names.join("  ")));
                return outcome(next, Ok(()));
            }
            let result = if header.bang {
                rt.state.autocmds().delete_group(&name).map(|_| ())
            } else {
                rt.state.autocmds().select_group(&name);
                Ok(())
            };
            outcome(next, result.map_err(Fail::Error))
        }
        Autocmd => cmd_autocmd(rt, host, src, header),
        Doautocmd => cmd_doautocmd(rt, host, src, at),
        Command => super::user_commands::define_command(rt, host, src, at, header.bang),
        Delcommand => {
            let (argument, next) = split_bar(src, at, true, false);
            let name = String::from_utf8_lossy(&argument).trim_start_matches("-buffer").trim().to_string();
            if rt.state.delete_user_command(&name) {
                outcome(next, Ok(()))
            } else {
                error(next, VimError::new(184, format!("No such user-defined command: {name}")))
            }
        }
        Comclear => {
            rt.state.clear_user_commands();
            outcome(split_bar(src, at, true, false).1, Ok(()))
        }
        Messages => {
            let (argument, next) = split_bar(src, at, true, false);
            let argument = String::from_utf8_lossy(&argument).into_owned();
            if argument.trim() == "clear" {
                let keep = header.range.is_empty().then_some(0).or_else(|| String::from_utf8_lossy(&header.range).parse().ok());
                rt.state.clear_messages(keep);
            } else {
                let messages = rt.state.messages();
                rt.emit(host, Message::Output(messages.join("\n")));
            }
            outcome(next, Ok(()))
        }
        Redir => {
            let (argument, next) = split_bar(src, at, true, false);
            outcome(next, redir(rt, host, &argument))
        }
        Redraw | Language => outcome(split_bar(src, at, true, false).1, Ok(())),
        Behave => {
            let (argument, next) = split_bar(src, at, true, false);
            let text = match String::from_utf8_lossy(&argument).trim() {
                "mswin" => "set selectmode=mouse,key mousemodel=popup keymodel=startsel,stopsel selection=exclusive",
                "xterm" => "set selectmode= mousemodel=extend keymodel= selection=inclusive",
                other => return error(next, VimError::new(475, format!("Invalid argument: {other}"))),
            };
            let command = HostCommand {
                text: text.to_string(),
                name: Some("set"),
                bang: false,
                range: String::new(),
                argument: text[4..].to_string(),
                modifiers: Modifiers::default(),
                sid: rt.current_sid(),
            };
            let result = host.execute(rt, &command);
            outcome(next, result)
        }
    }
}

// ----- conditionals ------------------------------------------------------

fn parent_active(rt: &Runtime) -> bool {
    rt.current_frame().is_none_or(Frame::executing)
}

fn eval_condition(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, evaluate: bool) -> (Result<bool, Fail>, Next) {
    let mut eval = Eval::new(rt, host, src, at, evaluate);
    eval.skip_white();
    if eval.at_end() || eval.peek() == b'|' {
        return (Err(Fail::Error(VimError::new(15, "Invalid expression: \"\""))), Next::NextLine);
    }
    let value = eval.eval1();
    let end = eval.pos;
    match value {
        Err(fail) if evaluate => return (Err(fail), skip_to_bar(src, end)),
        Err(_) => return (Ok(false), skip_to_bar(src, at)),
        Ok(_) => {}
    }
    let next = match after_expression(src, end, true) {
        Ok(next) => next,
        Err(err) => return (Err(Fail::Error(err)), skip_to_bar(src, end)),
    };
    match value {
        Ok(value) if evaluate => (value.to_bool().map_err(Fail::Error), next),
        _ => (Ok(false), next),
    }
}

fn push_cond(rt: &mut Runtime, cond: Cond) -> Result<(), VimError> {
    let frame = rt.current_frame_mut().expect("command runs in a frame");
    if frame.cstack.len() >= 50 {
        return Err(VimError::new(579, ":if nesting too deep"));
    }
    frame.cstack.push(cond);
    Ok(())
}

fn cmd_if(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, executing: bool) -> CommandOutcome {
    let active = executing && parent_active(rt);
    let (result, next) = eval_condition(rt, host, src, at, active);
    let (truth, failure) = match result {
        Ok(truth) => (truth, None),
        Err(fail) => (false, Some(fail)),
    };
    let mut cond = Cond::new(CondKind::If, active && truth && failure.is_none(), active);
    // An error or an inactive parent marks the conditional as decided so no
    // branch runs.
    cond.taken = !active || truth || failure.is_some();
    if let Err(err) = push_cond(rt, cond) {
        return error(next, err);
    }
    outcome(next, failure.map_or(Ok(()), Err))
}

fn cmd_elseif(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    let Some(cond) = rt.current_frame().and_then(|frame| frame.cstack.last()).cloned() else {
        return error(skip_to_bar(src, at), VimError::new(582, ":elseif without :if"));
    };
    if cond.kind != CondKind::If {
        return error(skip_to_bar(src, at), VimError::new(582, ":elseif without :if"));
    }
    if cond.had_else {
        return error(skip_to_bar(src, at), VimError::new(584, ":elseif after :else"));
    }
    let evaluate = cond.outer_active && !cond.taken;
    let (result, next) = eval_condition(rt, host, src, at, evaluate);
    let frame = rt.current_frame_mut().unwrap();
    let top = frame.cstack.last_mut().unwrap();
    match result {
        Ok(truth) if evaluate => {
            top.active = truth;
            top.taken = truth;
            outcome(next, Ok(()))
        }
        Ok(_) => {
            top.active = false;
            outcome(next, Ok(()))
        }
        Err(fail) => {
            top.active = false;
            top.taken = true;
            outcome(next, if evaluate { Err(fail) } else { Ok(()) })
        }
    }
}

fn cmd_else(rt: &mut Runtime, src: &[u8], at: usize) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let Some(frame) = rt.current_frame_mut() else { return outcome(next, Ok(())) };
    let Some(top) = frame.cstack.last_mut().filter(|cond| cond.kind == CondKind::If) else {
        return error(next, VimError::new(581, ":else without :if"));
    };
    if top.had_else {
        return error(next, VimError::new(583, "Multiple :else"));
    }
    top.had_else = true;
    top.active = top.outer_active && !top.taken;
    top.taken = true;
    outcome(next, Ok(()))
}

fn cmd_endif(rt: &mut Runtime, src: &[u8], at: usize) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let Some(frame) = rt.current_frame_mut() else { return outcome(next, Ok(())) };
    match frame.cstack.last() {
        Some(cond) if cond.kind == CondKind::If => {
            frame.cstack.pop();
            outcome(next, Ok(()))
        }
        _ => error(next, VimError::new(580, ":endif without :if")),
    }
}

fn command_position(rt: &Runtime, src_col: usize) -> (usize, usize) {
    let frame = rt.current_frame().unwrap();
    (frame.line, src_col)
}

fn cmd_while(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header, executing: bool) -> CommandOutcome {
    let start = command_position(rt, command_column(rt, header));
    let resuming = rt.current_frame().and_then(|frame| frame.cstack.last()).is_some_and(|cond| cond.looping && cond.start == start && cond.kind == CondKind::While);
    let active = if resuming {
        let frame = rt.current_frame().unwrap();
        frame.cstack.last().unwrap().outer_active
    } else {
        executing && parent_active(rt)
    };
    let (result, next) = eval_condition(rt, host, src, header.arg, active);
    let (truth, failure) = match result {
        Ok(truth) => (truth, None),
        Err(fail) => (false, Some(fail)),
    };
    if resuming {
        let top = rt.current_frame_mut().unwrap().cstack.last_mut().unwrap();
        top.looping = false;
        top.continued = false;
        top.active = truth && failure.is_none();
        top.broken = false;
    } else {
        let mut cond = Cond::new(CondKind::While, active && truth && failure.is_none(), active);
        cond.start = start;
        if let Err(err) = push_cond(rt, cond) {
            return error(next, err);
        }
    }
    outcome(next, failure.map_or(Ok(()), Err))
}

/// The column where the current command (including modifiers) started.
fn command_column(rt: &Runtime, _header: &Header) -> usize {
    rt.current_frame().map_or(0, |frame| frame.col)
}

fn parse_for_targets(src: &[u8], mut at: usize) -> Result<(Vec<VStr>, Option<VStr>, usize), VimError> {
    at = skip_white(src, at);
    let mut targets = Vec::new();
    let mut rest = None;
    let read_name = |at: &mut usize| -> Option<VStr> {
        let start = *at;
        if src.len() > *at + 1 && src[*at + 1] == b':' && matches!(src[*at], b'g' | b's' | b'l' | b'b' | b'w' | b't') {
            *at += 2;
        }
        while *at < src.len() && is_name_char(src[*at]) {
            *at += 1;
        }
        (*at > start).then(|| VStr::from(&src[start..*at]))
    };
    if src.get(at) == Some(&b'[') {
        at += 1;
        loop {
            at = skip_white(src, at);
            if src.get(at) == Some(&b';') {
                at = skip_white(src, at + 1);
                rest = Some(read_name(&mut at).ok_or_else(|| VimError::new(475, "Invalid argument"))?);
                at = skip_white(src, at);
            }
            if src.get(at) == Some(&b']') {
                at += 1;
                break;
            }
            let name = read_name(&mut at).ok_or_else(|| VimError::new(475, "Invalid argument"))?;
            targets.push(name);
            at = skip_white(src, at);
            if src.get(at) == Some(&b',') {
                at += 1;
            }
        }
    } else {
        targets.push(read_name(&mut at).ok_or_else(|| VimError::new(475, format!("Invalid argument: {}", String::from_utf8_lossy(&src[at..]))))?);
    }
    at = skip_white(src, at);
    if !(src[at.min(src.len())..].starts_with(b"in") && src.get(at + 2).is_none_or(|b| is_white(*b))) {
        return Err(VimError::new(690, "Missing \"in\" after :for"));
    }
    Ok((targets, rest, at + 2))
}

fn cmd_for(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header, executing: bool) -> CommandOutcome {
    let start = command_position(rt, command_column(rt, header));
    let resuming = rt.current_frame().and_then(|frame| frame.cstack.last()).is_some_and(|cond| cond.looping && cond.start == start && cond.kind == CondKind::For);
    if resuming {
        // Skip the expression; advance the iterator.
        let next = match parse_for_targets(src, header.arg) {
            Ok((_, _, at)) => {
                let mut eval = Eval::new(rt, host, src, at, false);
                let _ = eval.eval1();
                let end = eval.pos;
                after_expression(src, end, true).unwrap_or(Next::NextLine)
            }
            Err(_) => Next::NextLine,
        };
        let result = advance_for(rt, host);
        return outcome(next, result);
    }
    let active = executing && parent_active(rt);
    let (targets, rest, expr_at) = match parse_for_targets(src, header.arg) {
        Ok(parsed) => parsed,
        Err(err) => {
            let mut cond = Cond::new(CondKind::For, false, active);
            cond.start = start;
            let _ = push_cond(rt, cond);
            return if active { error(Next::NextLine, err) } else { outcome(Next::NextLine, Ok(())) };
        }
    };
    let mut eval = Eval::new(rt, host, src, expr_at, active);
    eval.skip_white();
    let value = eval.eval1();
    let end = eval.pos;
    let next = after_expression(src, end, true);
    let mut cond = Cond::new(CondKind::For, false, active);
    cond.start = start;
    let mut failure = None;
    match (value, next.as_ref()) {
        (Ok(value), Ok(_)) if active => {
            let items = match value {
                Value::List(list) => Some(ForItems::List(list)),
                Value::String(text) => Some(ForItems::Chars(split_chars(text.as_bytes()))),
                Value::Blob(blob) => Some(ForItems::Blob(blob.bytes())),
                _ => {
                    failure = Some(Fail::Error(VimError::new(1098, "String, List or Blob required")));
                    None
                }
            };
            if let Some(items) = items {
                cond.for_state = Some(ForState { items, index: 0, targets, rest });
            }
        }
        (Err(fail), _) if active => failure = Some(fail),
        (_, Err(err)) if active => failure = Some(Fail::Error(err.clone())),
        _ => {}
    }
    let next = next.unwrap_or(Next::NextLine);
    if let Err(err) = push_cond(rt, cond) {
        return error(next, err);
    }
    if let Some(fail) = failure {
        return outcome(next, Err(fail));
    }
    if !active {
        return outcome(next, Ok(()));
    }
    let result = advance_for(rt, host);
    outcome(next, result)
}

fn split_chars(bytes: &[u8]) -> Vec<VStr> {
    match std::str::from_utf8(bytes) {
        Ok(text) => unicode_segmentation::UnicodeSegmentation::graphemes(text, true)
            .flat_map(|grapheme| {
                // Vim iterates characters with their composing characters.
                let mut chars = grapheme.chars();
                let first = chars.next();
                first.map(|_| VStr::from(grapheme))
            })
            .collect(),
        Err(_) => bytes.iter().map(|b| VStr::from(vec![*b])).collect(),
    }
}

/// Assign the next item to the loop variables, or deactivate the loop.
fn advance_for(rt: &mut Runtime, host: &mut dyn ScriptHost) -> EvalResult<()> {
    let (value, targets, rest) = {
        let frame = rt.current_frame_mut().unwrap();
        let cond = frame.cstack.last_mut().unwrap();
        cond.looping = false;
        cond.broken = false;
        let Some(state) = cond.for_state.as_mut() else {
            cond.active = false;
            return Ok(());
        };
        let value = match &state.items {
            ForItems::List(list) => list.get(state.index),
            ForItems::Chars(chars) => chars.get(state.index).map(|c| Value::String(c.clone())),
            ForItems::Blob(bytes) => bytes.get(state.index).map(|b| Value::Number(*b as i64)),
        };
        state.index += 1;
        match value {
            Some(value) => {
                cond.active = true;
                (value, state.targets.clone(), state.rest.clone())
            }
            None => {
                cond.active = false;
                return Ok(());
            }
        }
    };
    if targets.len() == 1 && rest.is_none() {
        return rt.set_variable(host, targets[0].as_bytes(), value, false);
    }
    let Value::List(list) = value else {
        return Err(Fail::Error(VimError::new(714, "List required")));
    };
    let items = list.snapshot();
    if items.len() < targets.len() {
        return Err(Fail::Error(VimError::new(688, "More targets than List items")));
    }
    if items.len() > targets.len() && rest.is_none() {
        return Err(Fail::Error(VimError::new(687, "Less targets than List items")));
    }
    for (target, item) in targets.iter().zip(&items) {
        rt.set_variable(host, target.as_bytes(), item.clone(), false)?;
    }
    if let Some(rest) = rest {
        rt.set_variable(host, rest.as_bytes(), Value::list(items[targets.len()..].to_vec()), false)?;
    }
    Ok(())
}

fn cmd_endloop(rt: &mut Runtime, src: &[u8], at: usize, is_for: bool) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let Some(frame) = rt.current_frame_mut() else { return outcome(next, Ok(())) };
    let expected = if is_for { CondKind::For } else { CondKind::While };
    let without = || if is_for { VimError::new(588, ":endfor without :for") } else { VimError::new(588, ":endwhile without :while") };
    if !frame.cstack.iter().any(|cond| matches!(cond.kind, CondKind::While | CondKind::For)) {
        return error(next, without());
    }
    let Some(top) = frame.cstack.last_mut() else {
        return error(next, without());
    };
    if top.kind != expected {
        let err = match (top.kind, is_for) {
            (CondKind::While, true) => VimError::new(732, "Using :endfor with :while"),
            (CondKind::For, false) => VimError::new(733, "Using :endwhile with :for"),
            (CondKind::If, _) => VimError::new(171, "Missing :endif"),
            (CondKind::Try, _) => VimError::new(600, "Missing :endtry"),
            _ => VimError::new(588, ":endwhile without :while"),
        };
        return error(next, err);
    }
    // Loop again unless the condition failed, the loop was broken, or it
    // is inside a skipped block.
    if top.outer_active && !top.broken && (top.active || top.continued) {
        top.looping = true;
        top.continued = false;
        let (line, col) = top.start;
        return outcome(Next::Position(line, col), Ok(()));
    }
    frame.cstack.pop();
    outcome(next, Ok(()))
}

fn cmd_try(rt: &mut Runtime, src: &[u8], at: usize, executing: bool) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let active = executing && parent_active(rt);
    let cond = Cond::new(CondKind::Try, active, active);
    if let Err(err) = push_cond(rt, cond) {
        return error(next, err);
    }
    if active {
        rt.trylevel += 1;
    }
    outcome(next, Ok(()))
}

fn top_try(rt: &mut Runtime) -> Option<&mut Cond> {
    rt.current_frame_mut().and_then(|frame| frame.cstack.last_mut()).filter(|cond| cond.kind == CondKind::Try)
}

fn cmd_catch(rt: &mut Runtime, _host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    // Optional /pattern/.
    let mut pattern = None;
    let mut end = at;
    if let Some(&delimiter) = src.get(at) {
        if !delimiter.is_ascii_alphanumeric() && !matches!(delimiter, b'|' | b'"') && !is_white(delimiter) {
            let mut close = at + 1;
            while close < src.len() && src[close] != delimiter {
                if src[close] == b'\\' && close + 1 < src.len() {
                    close += 1;
                }
                close += 1;
            }
            pattern = Some(src[at + 1..close.min(src.len())].to_vec());
            end = (close + 1).min(src.len());
        }
    }
    let next = match after_expression(src, end, true) {
        Ok(next) => next,
        Err(err) => return error(Next::NextLine, err),
    };
    let Some(top) = rt.current_frame().and_then(|frame| frame.cstack.last()).cloned() else {
        return error(next, VimError::new(603, ":catch without :try"));
    };
    if top.kind != CondKind::Try {
        return error(next, VimError::new(603, ":catch without :try"));
    }
    if top.in_finally {
        return error(next, VimError::new(604, ":catch after :finally"));
    }
    if !top.outer_active {
        return outcome(next, Ok(()));
    }
    let pending_exception = match &top.pending {
        Some(Pending::Exception(exception)) if !top.caught => Some(exception.clone()),
        _ => None,
    };
    let matches = match (&pending_exception, &pattern) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(exception), Some(pattern)) => {
            let text = exception.text();
            let mut fuel = 1_000_000u64;
            match super::regex::string_matches(text.as_bytes(), pattern, false, &mut fuel) {
                Ok(found) => found,
                Err(err) => return error(next, err),
            }
        }
    };
    if top.in_catch {
        // Leaving the previous catch clause finishes its exception.
        rt.leave_catch();
    }
    let top = top_try(rt).unwrap();
    top.in_catch = false;
    if matches {
        let Some(Pending::Exception(exception)) = top.pending.take() else { unreachable!() };
        top.caught = true;
        top.in_catch = true;
        top.active = true;
        let exception = *exception;
        rt.state.set_vvar("exception", exception.value.clone());
        rt.state.set_vvar("throwpoint", exception.throwpoint.clone().into());
        rt.caught_exceptions.push(exception);
    } else {
        // The body finished normally, or the exception does not match.
        if top.pending.is_none() {
            top.caught = true;
        }
        top.active = false;
    }
    outcome(next, Ok(()))
}

fn cmd_finally(rt: &mut Runtime, src: &[u8], at: usize) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let Some(top) = rt.current_frame().and_then(|frame| frame.cstack.last()).cloned() else {
        return error(next, VimError::new(606, ":finally without :try"));
    };
    if top.kind != CondKind::Try {
        return error(next, VimError::new(606, ":finally without :try"));
    }
    if top.in_finally {
        return error(next, VimError::new(607, "Multiple :finally"));
    }
    if top.in_catch {
        rt.leave_catch();
    }
    let top = top_try(rt).unwrap();
    top.in_catch = false;
    top.in_finally = true;
    top.caught = true;
    top.active = top.outer_active;
    outcome(next, Ok(()))
}

fn cmd_endtry(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    let next = split_bar(src, at, true, false).1;
    let Some(top) = rt.current_frame().and_then(|frame| frame.cstack.last()).cloned() else {
        return error(next, VimError::new(602, ":endtry without :try"));
    };
    if top.kind != CondKind::Try {
        return error(next, VimError::new(602, ":endtry without :try"));
    }
    if top.in_catch {
        rt.leave_catch();
        if let Some(top) = top_try(rt) {
            top.in_catch = false;
        }
    }
    rt.pop_try();
    let action = match top.pending {
        Some(Pending::Exception(exception)) => Unwind::Exception(exception),
        Some(Pending::Return(value)) => Unwind::Return(value),
        Some(Pending::Break) => Unwind::Break,
        Some(Pending::Continue) => Unwind::Continue,
        Some(Pending::Finish) => Unwind::Finish,
        Some(Pending::Aborted) => Unwind::Aborted,
        None => return outcome(next, Ok(())),
    };
    outcome(next, rt.unwind(Some(host), action))
}

fn cmd_throw(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    let mut eval = Eval::new(rt, host, src, at, true);
    eval.skip_white();
    if eval.at_end() {
        return error(Next::NextLine, VimError::new(471, "Argument required"));
    }
    let value = eval.eval1();
    let end = eval.pos;
    let value = match value {
        Ok(value) => value,
        Err(fail) => return outcome(skip_to_bar(src, end), Err(fail)),
    };
    let next = match after_expression(src, end, true) {
        Ok(next) => next,
        Err(err) => return error(Next::NextLine, err),
    };
    // The value is converted to a String; a failed conversion is reported
    // directly, even inside :try, as in Vim.
    let value = match value.to_vstr() {
        Ok(text) => Value::String(text),
        Err(err) => {
            rt.report(host, err);
            return outcome(next, Ok(()));
        }
    };
    if value.is_empty_value() && matches!(value, Value::String(_)) {
        return error(next, VimError::new(608, "Cannot :throw exceptions with 'Vim' prefix"));
    }
    if let Value::String(text) = &value {
        if text.as_bytes().starts_with(b"Vim") {
            return error(next, VimError::new(608, "Cannot :throw exceptions with 'Vim' prefix"));
        }
    }
    let exception = Exception { value, throwpoint: rt.throwpoint(), from_error: false, interrupt: false };
    outcome(next, Err(Fail::Exception(Box::new(exception))))
}

fn cmd_return(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    if !rt.frames.iter().any(|frame| matches!(frame.kind, FrameKind::Function)) && rt.current_scope().is_none() {
        return error(Next::NextLine, VimError::new(133, ":return not inside a function"));
    }
    let at = skip_white(src, at);
    let (value, next) = if at >= src.len() || src[at] == b'|' || src[at] == b'"' {
        (Value::Number(0), after_expression(src, at, true).unwrap_or(Next::NextLine))
    } else {
        let mut eval = Eval::new(rt, host, src, at, true);
        let value = eval.eval1();
        let end = eval.pos;
        let value = match value {
            Ok(value) => value,
            Err(fail) => return outcome(skip_to_bar(src, end), Err(fail)),
        };
        let next = match after_expression(src, end, true) {
            Ok(next) => next,
            Err(err) => return error(Next::NextLine, err),
        };
        (value, next)
    };
    outcome(next, rt.unwind(Some(host), Unwind::Return(value)))
}

fn cmd_call(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header) -> CommandOutcome {
    let at = header.arg;
    if at >= src.len() {
        return error(Next::NextLine, VimError::new(471, "Argument required"));
    }
    let mut eval = Eval::new(rt, host, src, at, true);
    let callee = eval.parse_callee();
    let mut end = eval.pos;
    let callee = match callee {
        Ok(callee) => callee,
        // A failed call stops the line outside `:try` (Vim's ex_call()).
        Err(fail) => return outcome(if rt.trylevel > 0 { next_after_failure(src, end) } else { Next::NextLine }, Err(fail)),
    };
    let Some(callee) = callee else {
        return error(Next::NextLine, VimError::new(129, format!("Function name required: {}", String::from_utf8_lossy(&src[at..]))));
    };
    let range = if header.range.is_empty() {
        None
    } else {
        match host_range(rt, host, &header.range) {
            Ok(range) => Some(range),
            Err(fail) => return outcome(Next::NextLine, Err(fail)),
        }
    };
    // More subscripts after the call: evaluate synchronously.
    let rest = skip_white(src, end);
    let has_subscripts = src.get(rest) == Some(&b'[') || src[rest.min(src.len())..].starts_with(b"->") || (src.get(end) == Some(&b'.') && src.get(end + 1).is_some_and(|b| is_name_char(*b)));
    let function = rt.state.function(callee.funcref.name().as_bytes());
    if has_subscripts || function.is_none() || range.is_some() && !function.as_ref().is_some_and(|f| f.flags.range) {
        let result = (|| -> EvalResult<()> {
            let value = match (&range, &function) {
                (Some((first, last)), Some(function)) if !function.flags.range => {
                    let mut value = Value::Number(0);
                    for line in *first..=*last {
                        host_set_line(rt, host, line)?;
                        let mut args = callee.funcref.args().to_vec();
                        args.extend(callee.args.clone());
                        value = rt.call_user_function(host, function.clone(), args, callee.funcref.dict().cloned(), Some((line, line)))?;
                    }
                    value
                }
                _ => rt.call_funcref(host, &callee.funcref, callee.args.clone(), None)?,
            };
            if has_subscripts {
                let mut eval = Eval::new(rt, host, src, end, true);
                eval.subscripts(value, None)?;
                end = eval.pos;
            }
            Ok(())
        })();
        // A failed call skips the rest of the line outside `:try`, even
        // under `:silent!` (Vim's ex_call() sets no next command).
        let next = if result.is_err() && rt.trylevel == 0 {
            Next::NextLine
        } else {
            after_expression(src, end, true).unwrap_or_else(|_| skip_to_bar(src, end))
        };
        return outcome(next, result);
    }
    let next = match after_expression(src, end, true) {
        Ok(next) => next,
        Err(err) => return error(Next::NextLine, err),
    };
    let function = function.unwrap();
    let mut args = callee.funcref.args().to_vec();
    args.extend(callee.args);
    let dict = callee.funcref.dict().cloned();
    // The caller resumes at `next` when the function's frame finishes.
    let result = rt.push_function_call(host, function, args, dict, range);
    outcome(next, result)
}

fn host_range(rt: &mut Runtime, host: &mut dyn ScriptHost, range: &[u8]) -> EvalResult<(i64, i64)> {
    let text = String::from_utf8_lossy(range).into_owned();
    match host.call_function(rt, "__viem_resolve_range", &[Value::String(text.into())]) {
        Some(Ok(Value::List(list))) => {
            let items = list.snapshot();
            let first = items.first().map(Value::to_number).transpose()?.unwrap_or(1);
            let last = items.get(1).map(Value::to_number).transpose()?.unwrap_or(first);
            Ok((first, last))
        }
        Some(Err(fail)) => Err(fail),
        _ => Err(Fail::Error(VimError::new(481, "No range allowed"))),
    }
}

fn host_set_line(rt: &mut Runtime, host: &mut dyn ScriptHost, line: i64) -> EvalResult<()> {
    match host.call_function(rt, "cursor", &[line.into(), 1.into()]) {
        Some(result) => result.map(|_| ()),
        None => Ok(()),
    }
}

fn cmd_execute(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut position = at;
    loop {
        position = skip_white(src, position);
        if position >= src.len() || src[position] == b'|' {
            break;
        }
        let mut eval = Eval::new(rt, host, src, position, true);
        let value = eval.eval1();
        let end = eval.pos;
        let value = match value {
            Ok(value) => value,
            Err(fail) => return outcome(skip_to_bar(src, end), Err(fail)),
        };
        let bytes = match &value {
            Value::Float(_) => value.to_bytes(),
            Value::List(_) | Value::Dict(_) | Value::Funcref(_) | Value::Blob(_) => value.to_bytes(),
            _ => value.to_bytes(),
        };
        match bytes {
            Ok(bytes) => parts.push(bytes),
            Err(err) => return error(skip_to_bar(src, end), err),
        }
        position = end;
    }
    let next = if position < src.len() && src[position] == b'|' { Next::Column(position + 1) } else { Next::NextLine };
    if parts.is_empty() {
        return outcome(next, Ok(()));
    }
    let text = parts.join(&b' ');
    if rt.frames.len() >= super::exec::MAX_NESTING {
        return error(next, VimError::new(169, "Command too recursive"));
    }
    let lines = super::source::split_command_lines(&text, 1);
    let (sid, scope) = {
        let frame = rt.current_frame().unwrap();
        (frame.sid, frame.scope.clone())
    };
    let mut frame = Frame::new(FrameKind::Execute, lines.into(), sid);
    frame.scope = scope;
    rt.frames.push(frame);
    outcome(next, Ok(()))
}

fn cmd_echo(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, command: ScriptCommand) -> CommandOutcome {
    // :echo and :echon print each value as soon as it is evaluated, so
    // messages from a later argument appear between them, as in Vim.
    let incremental = matches!(command, ScriptCommand::Echo | ScriptCommand::Echon);
    let mut parts: Vec<String> = Vec::new();
    let mut position = at;
    let mut started = false;
    loop {
        position = skip_white(src, position);
        if position >= src.len() || src[position] == b'|' {
            break;
        }
        let mut eval = Eval::new(rt, host, src, position, true);
        let value = eval.eval1();
        let end = eval.pos;
        let value = match value {
            Ok(value) => value,
            Err(fail) => return outcome(next_after_failure(src, end), Err(fail)),
        };
        let text = match command {
            ScriptCommand::Echomsg | ScriptCommand::Echoerr => match &value {
                Value::String(text) => text.to_str_lossy().into_owned(),
                other => other.echo_string(),
            },
            _ => value.echo_string(),
        };
        if incremental {
            let message = match (command, started) {
                (ScriptCommand::Echo, false) => Message::Echo(text),
                (ScriptCommand::Echo, true) => Message::Echon(format!(" {text}")),
                _ => Message::Echon(text),
            };
            rt.emit(host, message);
            started = true;
        } else {
            parts.push(text);
        }
        position = end;
    }
    let next = if position < src.len() && src[position] == b'|' { Next::Column(position + 1) } else { Next::NextLine };
    match command {
        ScriptCommand::Echo if !started => rt.emit(host, Message::Echo(String::new())),
        ScriptCommand::Echo | ScriptCommand::Echon => {}
        ScriptCommand::Echomsg => {
            let text = parts.join(" ");
            rt.state.record_message(&text);
            rt.emit(host, Message::EchoMsg(text));
        }
        _ => {
            let text = parts.join(" ");
            if rt.emsg_silent > 0 {
                return outcome(next, Ok(()));
            }
            if rt.trylevel > 0 {
                let exception = Exception {
                    value: Value::String(format!("Vim(echoerr):{text}").into()),
                    throwpoint: rt.throwpoint(),
                    from_error: true,
                    interrupt: false,
                };
                return outcome(next, Err(Fail::Exception(Box::new(exception))));
            }
            return outcome(next, Err(Fail::Error(VimError::plain(text))));
        }
    }
    outcome(next, Ok(()))
}

// ----- :source and :redir ----------------------------------------------------

pub(crate) fn source_file(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8]) -> EvalResult<()> {
    let argument = String::from_utf8_lossy(argument).into_owned();
    if argument.trim().is_empty() {
        return Err(Fail::Error(VimError::new(471, "Argument required")));
    }
    let path = super::source::expand_path(rt, host, argument.trim())?;
    if rt.source_depth >= MAX_SOURCE_DEPTH {
        return Err(Fail::Error(VimError::plain("source nesting limit exceeded")));
    }
    let bytes = host.read_file(&path).map_err(Fail::Error)?;
    if rt.restricted {
        // Startup configuration may source other configuration files.
    }
    rt.queue_event_name("SourcePre", path.clone());
    rt.push_script(&path, &bytes).map_err(Fail::Error)?;
    rt.source_depth += 1;
    let index = rt.frames.len() - 1;
    rt.frames[index].source_post = Some(path.clone());
    // SourcePre runs before the file's first command.
    let events = std::mem::take(&mut rt.queued_events);
    let frames: Vec<Frame> = events.into_iter().flat_map(|(event, matched)| rt.autocmd_frames(event, &matched, None)).collect();
    for frame in frames.into_iter().rev() {
        rt.autocmd_depth += 1;
        rt.frames.push(frame);
    }
    Ok(())
}

fn redir(rt: &mut Runtime, host: &mut dyn ScriptHost, argument: &[u8]) -> EvalResult<()> {
    let text = String::from_utf8_lossy(argument).trim().to_string();
    if text.eq_ignore_ascii_case("end") || text.eq_ignore_ascii_case("END") {
        match rt.redirect.take() {
            Some(super::exec::Redirect::Variable { name, append, text }) => {
                let value = if append {
                    let mut existing = rt.get_variable(host, &name).ok().and_then(|v| v.to_bytes().ok()).unwrap_or_default();
                    existing.extend(text);
                    existing
                } else {
                    text
                };
                rt.set_variable(host, &name, Value::String(value.into()), false)?;
            }
            Some(super::exec::Redirect::Register { name, append, text }) => {
                host.set_register(name, &Value::String(text.into()), append).map_err(Fail::Error)?;
            }
            None => {}
        }
        return Ok(());
    }
    if let Some(rest) = text.strip_prefix("=>>") {
        rt.redirect = Some(super::exec::Redirect::Variable { name: rest.trim().as_bytes().to_vec(), append: true, text: Vec::new() });
        return Ok(());
    }
    if let Some(rest) = text.strip_prefix("=>") {
        let name = rest.trim().as_bytes().to_vec();
        rt.set_variable(host, &name, "".into(), false)?;
        rt.redirect = Some(super::exec::Redirect::Variable { name, append: false, text: Vec::new() });
        return Ok(());
    }
    if let Some(rest) = text.strip_prefix('@') {
        let mut chars = rest.chars();
        let name = chars.next().unwrap_or('"');
        let rest: String = chars.collect();
        let append = rest.trim_start().starts_with(">>");
        let register = if name.is_ascii_uppercase() { name.to_ascii_lowercase() } else { name };
        rt.redirect = Some(super::exec::Redirect::Register { name: register, append: append || name.is_ascii_uppercase(), text: Vec::new() });
        return Ok(());
    }
    Err(Fail::Error(VimError::plain("Viem scripts cannot redirect output to files")))
}

// ----- autocommands ---------------------------------------------------------

fn parse_group(rt: &Runtime, src: &[u8], at: usize) -> (Option<usize>, usize) {
    let at = skip_white(src, at);
    let end = src[at..].iter().position(|b| is_white(*b) || *b == b'|').map_or(src.len(), |n| at + n);
    if end == at {
        return (None, at);
    }
    let word = String::from_utf8_lossy(&src[at..end]).into_owned();
    // A group name that is not also an event name.
    if autocmd::event_index(&word).is_none() && word != "*" && !word.contains(',') {
        if let Some(group) = rt.state.autocmds().find_group(&word) {
            return (Some(group), end);
        }
    }
    (None, at)
}

fn cmd_autocmd(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header) -> CommandOutcome {
    let bang = header.bang;
    let (group, mut at) = parse_group(rt, src, header.arg);
    let group = group.unwrap_or_else(|| rt.state.autocmds().current_group());
    at = skip_white(src, at);
    // `:au! | ...` and `:au!` with nothing else.
    if at >= src.len() || src[at] == b'|' {
        let next = if at < src.len() { Next::Column(at + 1) } else { Next::NextLine };
        if bang {
            rt.state.autocmds().remove(group, None, None);
        } else {
            list_autocmds(rt, host, Some(group), None);
        }
        return outcome(next, Ok(()));
    }
    // Events.
    let events_end = src[at..].iter().position(|b| is_white(*b) || *b == b'|').map_or(src.len(), |n| at + n);
    let events_text = String::from_utf8_lossy(&src[at..events_end]).into_owned();
    let mut events = Vec::new();
    if events_text == "*" {
        events.extend(0..autocmd::EVENTS.len());
    } else {
        for name in events_text.split(',') {
            match autocmd::event_index(name) {
                Some(event) => events.push(event),
                None => return error(Next::NextLine, VimError::new(216, format!("No such group or event: {}", String::from_utf8_lossy(&src[at..])))),
            }
        }
    }
    at = skip_white(src, events_end);
    if at >= src.len() || src[at] == b'|' {
        let next = if at < src.len() { Next::Column(at + 1) } else { Next::NextLine };
        if bang {
            for event in events {
                rt.state.autocmds().remove(group, Some(event), None);
            }
        } else {
            list_autocmds(rt, host, Some(group), Some(&events));
        }
        return outcome(next, Ok(()));
    }
    // Pattern.
    let mut pattern_end = at;
    while pattern_end < src.len() && !is_white(src[pattern_end]) {
        if src[pattern_end] == b'\\' && pattern_end + 1 < src.len() {
            pattern_end += 1;
        }
        pattern_end += 1;
    }
    let pattern_text = String::from_utf8_lossy(&src[at..pattern_end]).into_owned();
    let pattern = if pattern_text.to_ascii_lowercase().starts_with("<buffer") {
        AutocmdPattern::Buffer
    } else {
        AutocmdPattern::Pattern(super::source::expand_pattern(rt, host, &pattern_text))
    };
    at = skip_white(src, pattern_end);
    let mut once = false;
    let mut nested = false;
    loop {
        if src[at.min(src.len())..].starts_with(b"++once") {
            once = true;
            at = skip_white(src, at + 6);
        } else if src[at.min(src.len())..].starts_with(b"++nested") {
            nested = true;
            at = skip_white(src, at + 8);
        } else if src[at.min(src.len())..].starts_with(b"nested") && src.get(at + 6).is_none_or(|b| is_white(*b)) {
            nested = true;
            at = skip_white(src, at + 6);
        } else {
            break;
        }
    }
    let command = &src[at.min(src.len())..];
    if bang {
        for &event in &events {
            rt.state.autocmds().remove(group, Some(event), Some(&pattern));
        }
    }
    if command.is_empty() {
        if !bang {
            list_autocmds(rt, host, Some(group), Some(&events));
        }
        return outcome(Next::NextLine, Ok(()));
    }
    let sid = rt.current_sid();
    for event in events {
        rt.state.autocmds().add(Autocmd {
            group,
            event,
            pattern: pattern.clone(),
            once,
            nested,
            command: VStr::from(command),
            sid,
            id: 0,
        });
    }
    outcome(Next::NextLine, Ok(()))
}

fn list_autocmds(rt: &mut Runtime, host: &mut dyn ScriptHost, group: Option<usize>, events: Option<&[usize]>) {
    let mut lines = vec!["--- Autocommands ---".to_string()];
    {
        let table = rt.state.autocmds();
        for entry in table.entries() {
            if group.is_some_and(|group| group != 0 && entry.group != group) {
                continue;
            }
            if events.is_some_and(|events| !events.contains(&entry.event)) {
                continue;
            }
            let group_name = table.group_name(entry.group).map(|name| format!("{name}  ")).unwrap_or_default();
            let pattern = match &entry.pattern {
                AutocmdPattern::Buffer => "<buffer>".to_string(),
                AutocmdPattern::Pattern(pattern) => pattern.clone(),
            };
            lines.push(format!("{group_name}{}\n    {pattern}\n        {}", autocmd::event_name(entry.event), entry.command.to_str_lossy()));
        }
    }
    rt.emit(host, Message::Output(lines.join("\n")));
}

fn cmd_doautocmd(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize) -> CommandOutcome {
    let (argument, next) = split_bar(src, at, true, false);
    let mut text = String::from_utf8_lossy(&argument).into_owned();
    if let Some(rest) = text.strip_prefix("<nomodeline>") {
        text = rest.trim_start().to_string();
    }
    let mut words = text.split_whitespace().peekable();
    let mut group = None;
    if let Some(first) = words.peek() {
        if autocmd::event_index(first).is_none() {
            group = rt.state.autocmds().find_group(first);
            if group.is_some() {
                words.next();
            }
        }
    }
    let Some(events) = words.next() else {
        return error(next, VimError::new(471, "Argument required"));
    };
    let matched = words.collect::<Vec<_>>().join(" ");
    let matched = if matched.is_empty() { host.buffer_name().unwrap_or_default() } else { matched };
    let mut frames = Vec::new();
    for name in events.split(',') {
        let Some(event) = autocmd::event_index(name) else {
            return error(next, VimError::new(216, format!("No such group or event: {name}")));
        };
        frames.extend(rt.autocmd_frames(event, &matched, group));
    }
    if frames.is_empty() {
        rt.emit(host, Message::Echo(format!("No matching autocommands: {}", text.trim())));
    }
    for frame in frames.into_iter().rev() {
        rt.autocmd_depth += 1;
        rt.frames.push(frame);
    }
    outcome(next, Ok(()))
}

// ----- user commands -----------------------------------------------------------

fn find_user_command(rt: &Runtime, name: &str) -> Result<Option<Arc<UserCommand>>, VimError> {
    if let Some(command) = rt.state.user_command(name) {
        return Ok(Some(command));
    }
    let candidates = rt.state.user_commands_with_prefix(name);
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(Some(candidates[0].clone())),
        _ => Err(VimError::new(464, format!("Ambiguous use of user-defined command: {name}"))),
    }
}

fn run_user_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], header: &Header, command: Arc<UserCommand>, executing: bool) -> CommandOutcome {
    let (argument, next) = if command.bar {
        split_bar(src, header.arg, true, false)
    } else {
        let mut argument = src[header.arg.min(src.len())..].to_vec();
        trim_trailing(&mut argument);
        (argument, Next::NextLine)
    };
    if !executing {
        return outcome(next, Ok(()));
    }
    rt.current_command = command.name.clone();
    match super::user_commands::expand(rt, host, &command, &header.range, header.bang, &argument, &header.modifiers) {
        Ok(text) => {
            if rt.frames.len() >= super::exec::MAX_NESTING {
                return error(next, VimError::new(169, "Command too recursive"));
            }
            let lines = super::source::split_command_lines(&text, 1);
            let (scope, sid) = {
                let frame = rt.current_frame().unwrap();
                (frame.scope.clone(), if command.keepscript { frame.sid } else { command.sid })
            };
            let mut frame = Frame::new(FrameKind::UserCommand { name: command.name.clone() }, lines.into(), sid);
            frame.scope = scope;
            rt.frames.push(frame);
            outcome(next, Ok(()))
        }
        Err(err) => {
            let mut result = error(next, err);
            append_command(&mut result, src, header);
            result
        }
    }
}

/// Heredoc and function-body collection need to know if a line starts with
/// a given command.
pub(crate) fn line_command_name(src: &[u8]) -> Option<(&'static str, usize, bool)> {
    let mut at = skip_white(src, 0);
    while src.get(at) == Some(&b':') {
        at = skip_white(src, at + 1);
    }
    let start = at;
    while at < src.len() && src[at].is_ascii_alphabetic() {
        at += 1;
    }
    if at == start {
        return None;
    }
    let name = std::str::from_utf8(&src[start..at]).ok()?;
    let full = super::excmds::lookup(name)?;
    let bang = src.get(at) == Some(&b'!');
    Some((full, if bang { at + 1 } else { at }, bang))
}

pub(crate) fn command_args_from(text: &str) -> Option<CommandArgs> {
    Some(match text {
        "0" => CommandArgs::Zero,
        "1" => CommandArgs::One,
        "*" => CommandArgs::Any,
        "?" => CommandArgs::ZeroOrOne,
        "+" => CommandArgs::OneOrMore,
        _ => return None,
    })
}
