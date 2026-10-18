//! `:command` definitions and expansion of user commands.
use super::error::{EvalResult, Fail, VimError};
use super::exec::{CommandOutcome, Message, Modifiers, Next, Runtime, ScriptHost};
use super::expr::{is_white, skip_white};
use super::state::{CommandArgs, CommandRange, UserCommand};
use super::value::{VStr, Value};

const COMPLETIONS: &[&str] = &[
    "arglist", "augroup", "behave", "breakpoint", "buffer", "color", "command", "compiler", "cscope",
    "diff_buffer", "dir", "dir_in_path", "environment", "event", "expression", "file", "file_in_path",
    "filetype", "function", "help", "highlight", "history", "keymap", "locale", "mapclear", "mapping",
    "menu", "messages", "option", "packadd", "runtime", "scriptnames", "shellcmd", "shellcmdline",
    "sign", "syntax", "syntime", "tag", "tag_listfiles", "user", "var",
];

fn outcome(next: Next, result: EvalResult<()>) -> CommandOutcome {
    CommandOutcome { next, result }
}

pub(crate) fn define_command(rt: &mut Runtime, host: &mut dyn ScriptHost, src: &[u8], at: usize, bang: bool) -> CommandOutcome {
    let text = String::from_utf8_lossy(&src[at.min(src.len())..]).into_owned();
    let result = (|| -> EvalResult<()> {
        let mut rest = text.trim_start();
        let mut command = UserCommand {
            name: String::new(),
            nargs: CommandArgs::Zero,
            range: CommandRange::None,
            bang: false,
            bar: false,
            register: false,
            buffer: false,
            keepscript: false,
            complete: None,
            addr: None,
            replacement: VStr::empty(),
            sid: rt.current_sid(),
        };
        let mut count_seen = false;
        while let Some(attribute) = rest.strip_prefix('-') {
            let end = attribute.find(char::is_whitespace).unwrap_or(attribute.len());
            let word = &attribute[..end];
            rest = attribute[end..].trim_start();
            let (key, value) = match word.split_once('=') {
                Some((key, value)) => (key, Some(value)),
                None => (word, None),
            };
            match key {
                "nargs" => {
                    command.nargs = value.and_then(super::commands::command_args_from).ok_or_else(|| Fail::Error(VimError::new(176, "Invalid number of arguments")))?;
                }
                "range" => {
                    if count_seen {
                        return Err(Fail::Error(VimError::new(177, "Count cannot be specified twice")));
                    }
                    count_seen = true;
                    command.range = match value {
                        None => CommandRange::Line,
                        Some("%") => CommandRange::File,
                        Some(number) => CommandRange::Count(number.parse().map_err(|_| Fail::Error(VimError::new(178, "Invalid default value for count")))?),
                    };
                }
                "count" => {
                    if count_seen {
                        return Err(Fail::Error(VimError::new(177, "Count cannot be specified twice")));
                    }
                    count_seen = true;
                    command.range = CommandRange::CountArg(match value {
                        None => 0,
                        Some(number) => number.parse().map_err(|_| Fail::Error(VimError::new(178, "Invalid default value for count")))?,
                    });
                }
                "bang" => command.bang = true,
                "bar" => command.bar = true,
                "register" => command.register = true,
                "buffer" => command.buffer = true,
                "keepscript" => command.keepscript = true,
                "complete" => {
                    let value = value.ok_or_else(|| Fail::Error(VimError::new(179, "argument required for -complete")))?;
                    let kind = value.split(',').next().unwrap_or("");
                    if !COMPLETIONS.contains(&kind) && kind != "custom" && kind != "customlist" {
                        return Err(Fail::Error(VimError::new(180, format!("Invalid complete value: {value}"))));
                    }
                    command.complete = Some(value.to_string());
                }
                "addr" => {
                    command.addr = Some(value.ok_or_else(|| Fail::Error(VimError::new(179, "argument required for -addr")))?.to_string());
                }
                _ => return Err(Fail::Error(VimError::new(181, format!("Invalid attribute: -{word}")))),
            }
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let name = &rest[..end];
        let replacement = rest[end..].trim_start();
        if name.is_empty() {
            list_commands(rt, host, "");
            return Ok(());
        }
        if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return Err(Fail::Error(VimError::new(183, "User defined commands must start with an uppercase letter")));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Fail::Error(VimError::new(182, format!("Invalid command name: {name}"))));
        }
        if matches!(name, "X" | "Next" | "Print") {
            return Err(Fail::Error(VimError::new(841, "Reserved name, cannot be used for user defined command")));
        }
        if replacement.is_empty() {
            list_commands(rt, host, name);
            return Ok(());
        }
        if rt.state.user_command(name).is_some() && !bang {
            return Err(Fail::Error(VimError::new(174, format!("Command already exists: add ! to replace it: {name}"))));
        }
        command.name = name.to_string();
        command.replacement = VStr::from(replacement);
        rt.state.define_user_command(command);
        Ok(())
    })();
    outcome(Next::NextLine, result)
}

fn list_commands(rt: &mut Runtime, host: &mut dyn ScriptHost, prefix: &str) {
    let mut lines = vec!["    Name              Args Address Complete    Definition".to_string()];
    for command in rt.state.user_commands() {
        if !command.name.starts_with(prefix) {
            continue;
        }
        let nargs = match command.nargs {
            CommandArgs::Zero => "0",
            CommandArgs::One => "1",
            CommandArgs::Any => "*",
            CommandArgs::ZeroOrOne => "?",
            CommandArgs::OneOrMore => "+",
        };
        let range = match command.range {
            CommandRange::None => String::new(),
            CommandRange::Line => ".".into(),
            CommandRange::File => "%".into(),
            CommandRange::Count(n) => format!("{n}c"),
            CommandRange::CountArg(n) => format!("{n}c"),
        };
        lines.push(format!(
            "{}{}  {:<17} {:<4} {:<7} {:<11} {}",
            if command.bang { '!' } else { ' ' },
            if command.bar { '|' } else { ' ' },
            command.name,
            nargs,
            range,
            command.complete.as_deref().unwrap_or(""),
            command.replacement.to_str_lossy()
        ));
    }
    rt.emit(host, Message::Output(lines.join("\n")));
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Split `<f-args>`: white space separates arguments; a backslash escapes
/// white space and backslashes.
fn split_f_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_arg = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(' ' | '\t' | '\\') => {
                    current.push(chars.next().unwrap());
                    in_arg = true;
                    continue;
                }
                _ => {
                    current.push(c);
                    in_arg = true;
                    continue;
                }
            }
        }
        if c == ' ' || c == '\t' {
            if in_arg {
                args.push(std::mem::take(&mut current));
                in_arg = false;
            }
            continue;
        }
        current.push(c);
        in_arg = true;
    }
    if in_arg {
        args.push(current);
    }
    args
}

/// Expand a user command's replacement text.
pub(crate) fn expand(
    rt: &mut Runtime,
    host: &mut dyn ScriptHost,
    command: &UserCommand,
    range: &[u8],
    bang: bool,
    argument: &[u8],
    modifiers: &Modifiers,
) -> Result<Vec<u8>, VimError> {
    if bang && !command.bang {
        return Err(VimError::new(477, "No ! allowed"));
    }
    let mut argument = String::from_utf8_lossy(argument).into_owned();
    let mut register = String::new();
    if command.register {
        if let Some(first) = argument.chars().next() {
            if first.is_ascii_alphanumeric() || "\"-+*:.%#/=_".contains(first) {
                let rest = argument[first.len_utf8()..].to_string();
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    register = first.to_string();
                    argument = rest.trim_start().to_string();
                }
            }
        }
    }
    let range_text = String::from_utf8_lossy(range).trim().to_string();
    if !range_text.is_empty() && command.range == CommandRange::None {
        return Err(VimError::new(481, "No range allowed"));
    }
    let mut count_from_arg = None;
    if let CommandRange::CountArg(_) = command.range {
        let digits: String = argument.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && argument[digits.len()..].chars().next().is_none_or(char::is_whitespace) {
            count_from_arg = digits.parse::<i64>().ok();
            argument = argument[digits.len()..].trim_start().to_string();
        }
    }
    match command.nargs {
        CommandArgs::Zero if !argument.is_empty() => return Err(VimError::trailing(argument.as_bytes())),
        CommandArgs::One | CommandArgs::OneOrMore if argument.is_empty() => return Err(VimError::new(471, "Argument required")),
        _ => {}
    }
    // Resolve the range through the host, which owns line addresses.
    let (mut line1, mut line2, items) = match command.range {
        CommandRange::None => (0, 0, 0),
        _ => {
            if range_text.is_empty() {
                match command.range {
                    CommandRange::File => {
                        let last = match host.call_function(rt, "line", &[Value::str("$")]) {
                            Some(Ok(value)) => value.to_number().unwrap_or(1),
                            _ => 1,
                        };
                        (1, last, 0)
                    }
                    CommandRange::Count(n) | CommandRange::CountArg(n) => (n, n, 0),
                    _ => {
                        let current = match host.call_function(rt, "line", &[Value::str(".")]) {
                            Some(Ok(value)) => value.to_number().unwrap_or(1),
                            _ => 1,
                        };
                        (current, current, 0)
                    }
                }
            } else if matches!(command.range, CommandRange::Count(_) | CommandRange::CountArg(_)) && range_text.chars().all(|c| c.is_ascii_digit()) {
                let n = range_text.parse().unwrap_or(0);
                (n, n, 1)
            } else {
                match host.call_function(rt, "__viem_resolve_range", &[Value::String(range_text.clone().into())]) {
                    Some(Ok(Value::List(list))) => {
                        let values = list.snapshot();
                        let first = values.first().and_then(|v| v.to_number().ok()).unwrap_or(1);
                        let last = values.get(1).and_then(|v| v.to_number().ok()).unwrap_or(first);
                        (first, last, if range_text.contains([',', ';']) || range_text == "%" { 2 } else { 1 })
                    }
                    Some(Err(Fail::Error(error))) => return Err(error),
                    _ => (1, 1, 1),
                }
            }
        }
    };
    if let Some(count) = count_from_arg {
        line1 = count;
        line2 = count;
    }
    let count = match command.range {
        CommandRange::Count(n) | CommandRange::CountArg(n) => {
            if items == 0 && count_from_arg.is_none() {
                n
            } else {
                line2
            }
        }
        _ => line2,
    };
    let mut mods = Vec::new();
    for (enabled, word) in [
        (modifiers.silent && modifiers.silent_errors, "silent!"),
        (modifiers.silent && !modifiers.silent_errors, "silent"),
        (modifiers.vertical, "vertical"),
        (modifiers.horizontal, "horizontal"),
        (modifiers.keepjumps, "keepjumps"),
        (modifiers.keeppatterns, "keeppatterns"),
        (modifiers.keepmarks, "keepmarks"),
        (modifiers.keepalt, "keepalt"),
        (modifiers.noautocmd, "noautocmd"),
        (modifiers.confirm, "confirm"),
        (modifiers.browse, "browse"),
        (modifiers.sandbox, "sandbox"),
        (modifiers.unsilent, "unsilent"),
    ] {
        if enabled {
            mods.push(word.to_string());
        }
    }
    mods.extend(modifiers.placement.iter().cloned());
    if let Some(level) = modifiers.verbose {
        mods.push(format!("{level}verbose"));
    }
    let mods = mods.join(" ");
    let replacement = command.replacement.to_str_lossy().into_owned();
    let mut out = String::new();
    let mut at = 0;
    while at < replacement.len() {
        let rest = &replacement[at..];
        if rest.starts_with('<') {
            if let Some(close) = rest.find('>') {
                let tag = rest[1..close].to_ascii_lowercase();
                let expansion = match tag.as_str() {
                    "line1" => Some(line1.to_string()),
                    "line2" => Some(line2.to_string()),
                    "range" => Some(items.to_string()),
                    "count" => Some(count.to_string()),
                    "bang" => Some(if bang { "!".into() } else { String::new() }),
                    "reg" | "register" => Some(register.clone()),
                    "args" => Some(argument.clone()),
                    "q-args" => Some(quote(&argument)),
                    "f-args" => Some(match command.nargs {
                        CommandArgs::One | CommandArgs::ZeroOrOne => {
                            if argument.is_empty() {
                                String::new()
                            } else {
                                quote(&argument)
                            }
                        }
                        _ => split_f_args(&argument).iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(","),
                    }),
                    "lt" => Some("<".into()),
                    "mods" => Some(mods.clone()),
                    "q-mods" => Some(quote(&mods)),
                    "q-bang" | "q-reg" | "q-count" | "q-line1" | "q-line2" => {
                        let inner = &tag[2..];
                        Some(quote(&match inner {
                            "bang" => if bang { "!".into() } else { String::new() },
                            "reg" => register.clone(),
                            "count" => count.to_string(),
                            "line1" => line1.to_string(),
                            _ => line2.to_string(),
                        }))
                    }
                    _ => None,
                };
                if let Some(expansion) = expansion {
                    out.push_str(&expansion);
                    at += close + 1;
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        at += c.len_utf8();
    }
    let _ = skip_white(b"", 0);
    let _ = is_white(b' ');
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_args_split_on_unescaped_white_space() {
        assert_eq!(split_f_args("a b\\ c d\\\\"), vec!["a", "b c", "d\\"]);
        assert_eq!(split_f_args("  "), Vec::<String>::new());
        assert_eq!(quote("a\"b\\"), "\"a\\\"b\\\\\"");
    }
}
