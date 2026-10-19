//! Script builtins: `exists()`, `has()`, `function()`, `call()`,
//! `execute()`, `eval()`, input functions and the `assert_*()` family.
use super::{arity, text, vstr};
use crate::vimscript::error::{EvalResult, Fail, VimError};
use crate::vimscript::exec::{Capture, Runtime, ScriptHost};
use crate::vimscript::value::{values_equal, Funcref, Value};

pub(super) const NAMES: &[&str] = &[
    "exists", "has", "function", "funcref", "call", "execute", "eval", "confirm", "input",
    "inputlist", "inputsecret", "inputdialog", "inputsave", "inputrestore", "getchar", "getcharstr",
    "getcharmod", "assert_equal", "assert_notequal", "assert_true", "assert_false", "assert_match",
    "assert_notmatch", "assert_report", "assert_inrange", "assert_exception", "assert_fails",
    "getscriptinfo", "exists_compiled", "test_null_list", "test_null_dict", "test_null_string",
];

pub(super) fn call(rt: &mut Runtime, host: &mut dyn ScriptHost, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
    Some(match name {
        "exists" | "exists_compiled" => arity(name, args, 1, 1).and_then(|()| exists(rt, host, &text(args, 0)?).map(Value::Number)),
        "has" => arity(name, args, 1, 2).and_then(|()| {
            let feature = text(args, 0)?;
            Ok(Value::Number(has(host, &feature) as i64))
        }),
        "function" | "funcref" => arity(name, args, 1, 3).and_then(|()| function(rt, host, args)),
        "call" => arity(name, args, 2, 3).and_then(|()| {
            let Value::List(list) = &args[1] else {
                return Err(Fail::Error(VimError::new(1211, "List required for argument 2")));
            };
            let dict = match args.get(2) {
                Some(crate::vimscript::value::Value::Dict(dict)) => Some(dict.clone()),
                Some(_) => return Err(Fail::Error(VimError::new(715, "Dictionary required"))),
                None => None,
            };
            let arguments = list.snapshot();
            match &args[0] {
                Value::Funcref(funcref) => rt.call_funcref(host, funcref, arguments, dict),
                Value::String(name) => {
                    let funcref = rt.resolve_function_name(host, name.as_bytes())?;
                    rt.call_funcref(host, &funcref, arguments, dict)
                }
                _ => Err(Fail::Error(VimError::new(921, "Invalid callback argument"))),
            }
        }),
        "execute" => arity(name, args, 1, 2).and_then(|()| execute(rt, host, args)),
        "eval" => arity(name, args, 1, 1).and_then(|()| {
            let source = vstr(args, 0)?;
            let sid = rt.current_sid();
            rt.eval_expression(host, source.as_bytes(), sid)
        }),
        "confirm" => arity(name, args, 1, 4).and_then(|()| {
            // No modal prompts during scripts: answer the default choice.
            let default = super::opt_number(args, 2, 1)?;
            Ok(Value::Number(default))
        }),
        "input" | "inputdialog" | "inputsecret" => arity(name, args, 1, 3).and_then(|()| Ok(args.get(1).cloned().unwrap_or_else(|| Value::str("")))),
        "inputlist" => arity(name, args, 1, 1).map(|()| Value::Number(0)),
        "inputsave" | "inputrestore" => arity(name, args, 0, 0).map(|()| Value::Number(0)),
        "getchar" | "getcharmod" => arity(name, args, 0, 2).map(|()| Value::Number(0)),
        "getcharstr" => arity(name, args, 0, 2).map(|()| Value::str("")),
        "assert_equal" | "assert_notequal" => arity(name, args, 2, 3).and_then(|()| {
            let equal = values_equal(&args[0], &args[1], false, 0)? && args[0].type_number() == args[1].type_number();
            let failed = if name == "assert_equal" { !equal } else { equal };
            if failed {
                let message = if name == "assert_equal" {
                    format!("Expected {} but got {}", args[0].string_repr(), args[1].string_repr())
                } else {
                    format!("Expected not equal to {}", args[1].string_repr())
                };
                assert_failure(rt, args.get(2), message)
            } else {
                Ok(Value::Number(0))
            }
        }),
        "assert_true" | "assert_false" => arity(name, args, 1, 2).and_then(|()| {
            let truth = match &args[0] {
                Value::Bool(value) => Some(*value),
                Value::Number(number) => Some(*number != 0),
                _ => None,
            };
            let expected = name == "assert_true";
            if truth == Some(expected) {
                Ok(Value::Number(0))
            } else {
                let message = format!("Expected {} but got {}", if expected { "True" } else { "False" }, args[0].string_repr());
                assert_failure(rt, args.get(1), message)
            }
        }),
        "assert_match" | "assert_notmatch" => arity(name, args, 2, 3).and_then(|()| {
            let (pattern, actual) = (vstr(args, 0)?, vstr(args, 1)?);
            let matched = crate::vimscript::regex::string_matches(actual.as_bytes(), pattern.as_bytes(), false, &mut rt.fuel)?;
            if matched == (name == "assert_match") {
                Ok(Value::Number(0))
            } else {
                let message = format!(
                    "Pattern {} {}match {}",
                    args[0].string_repr(),
                    if name == "assert_match" { "does not " } else { "does " },
                    args[1].string_repr()
                );
                assert_failure(rt, args.get(2), message)
            }
        }),
        "assert_report" => arity(name, args, 1, 1).and_then(|()| {
            let message = text(args, 0)?;
            assert_failure(rt, None, message)
        }),
        "assert_inrange" => arity(name, args, 3, 4).and_then(|()| {
            let (low, high, actual) = (args[0].to_float()?, args[1].to_float()?, args[2].to_float()?);
            if (low..=high).contains(&actual) {
                Ok(Value::Number(0))
            } else {
                let message = format!("Expected range {} - {}, but got {}", args[0].echo_string(), args[1].echo_string(), args[2].echo_string());
                assert_failure(rt, args.get(3), message)
            }
        }),
        "assert_exception" => arity(name, args, 1, 2).and_then(|()| {
            let expected = text(args, 0)?;
            let actual = rt.state.vvar("exception").to_text()?;
            if actual.is_empty() {
                assert_failure(rt, args.get(1), "v:exception is not set".into())
            } else if !actual.contains(&expected) {
                assert_failure(rt, args.get(1), format!("Expected '{expected}' but got '{actual}'"))
            } else {
                Ok(Value::Number(0))
            }
        }),
        "assert_fails" => arity(name, args, 1, 5).and_then(|()| {
            let command = text(args, 0)?;
            let before = rt.errors.len();
            let sid = rt.current_sid();
            rt.captures.push(Capture { text: Vec::new(), silent: true });
            rt.emsg_silent += 0;
            let saved_trylevel = rt.trylevel;
            rt.trylevel = 0;
            let result = rt.execute_now(host, &command, sid);
            rt.trylevel = saved_trylevel;
            rt.captures.pop();
            let errors: Vec<String> = rt.errors.drain(before..).map(|error| error.error.text()).collect();
            let mut texts = errors;
            if let Err(Fail::Exception(exception)) = &result {
                texts.push(exception.text());
            }
            if let Err(Fail::Error(error)) = &result {
                texts.push(error.text());
            }
            if texts.is_empty() {
                return assert_failure(rt, args.get(2), format!("command did not fail: {command}"));
            }
            if let Some(expected) = args.get(1) {
                let wanted: Vec<String> = match expected {
                    Value::List(list) => list.snapshot().iter().map(|item| item.to_text()).collect::<Result<_, _>>()?,
                    other => vec![other.to_text()?],
                };
                if let Some(first) = wanted.first() {
                    if !first.is_empty() && !texts.iter().any(|text| text.contains(first.as_str())) {
                        return assert_failure(rt, args.get(2), format!("Expected {} but got {}", expected.string_repr(), Value::str(&texts[0]).string_repr()));
                    }
                }
            }
            Ok(Value::Number(0))
        }),
        "getscriptinfo" => arity(name, args, 0, 1).map(|()| {
            Value::list(
                rt.state
                    .scripts()
                    .into_iter()
                    .enumerate()
                    .filter(|(_, script)| !script.name.starts_with('<'))
                    .map(|(index, script)| {
                        Value::Dict(crate::vimscript::value::Dict::from_pairs([
                            ("sid".into(), ((index + 1) as i64).into()),
                            ("name".into(), Value::str(&script.name)),
                            ("autoload".into(), Value::Bool(false)),
                            ("version".into(), 1.into()),
                        ]))
                    })
                    .collect(),
            )
        }),
        "test_null_list" => arity(name, args, 0, 0).map(|()| Value::list(Vec::new())),
        "test_null_dict" => arity(name, args, 0, 0).map(|()| Value::Dict(Default::default())),
        "test_null_string" => arity(name, args, 0, 0).map(|()| Value::str("")),
        _ => return None,
    })
}

fn assert_failure(rt: &mut Runtime, message: Option<&Value>, default: String) -> EvalResult<Value> {
    let text = match message {
        Some(message) => message.to_text()?,
        None => default,
    };
    let point = rt.throwpoint();
    let entry = if point.is_empty() { text } else { format!("{point}: {text}") };
    if let Value::List(errors) = rt.state.vvar("errors") {
        errors.push(Value::String(entry.into()));
    }
    Ok(Value::Number(1))
}

fn execute(rt: &mut Runtime, host: &mut dyn ScriptHost, args: &[Value]) -> EvalResult<Value> {
    let commands: Vec<String> = match &args[0] {
        Value::List(list) => list.snapshot().iter().map(|item| item.to_text()).collect::<Result<_, _>>()?,
        other => other.to_text()?.split('\n').map(str::to_string).collect(),
    };
    let mode = match args.get(1) {
        Some(value) => value.to_text()?,
        None => "silent".into(),
    };
    let silent_errors = mode == "silent!";
    rt.captures.push(Capture { text: Vec::new(), silent: !mode.is_empty() });
    if silent_errors {
        rt.emsg_silent += 1;
    }
    let sid = rt.current_sid();
    let scope = rt.current_scope();
    let mut result = Ok(());
    for command in commands {
        if command.trim().is_empty() {
            continue;
        }
        result = rt.with_context(scope.clone(), sid, |rt| rt.execute_now(host, &command, sid));
        if result.is_err() {
            break;
        }
    }
    if silent_errors {
        rt.emsg_silent -= 1;
    }
    let capture = rt.captures.pop().unwrap_or_default();
    result?;
    Ok(Value::String(capture.text.into()))
}

fn function(rt: &mut Runtime, host: &mut dyn ScriptHost, args: &[Value]) -> EvalResult<Value> {
    let (mut bound_args, mut bound_dict) = (Vec::new(), None);
    for extra in &args[1..] {
        match extra {
            Value::List(list) => bound_args = list.snapshot(),
            Value::Dict(dict) => bound_dict = Some(dict.clone()),
            _ => return Err(Fail::Error(VimError::new(923, "Second argument of function() must be a list or a dict"))),
        }
    }
    let base = match &args[0] {
        Value::Funcref(funcref) => funcref.clone(),
        Value::String(name) => {
            if name.is_empty() {
                return Err(Fail::Error(VimError::new(129, "Function name required")));
            }
            let canonical = rt.canonical_function_name(name.as_bytes());
            let exists = rt.state.function(canonical.as_bytes()).is_some()
                || (crate::vimscript::exec::is_builtin_name(name.as_bytes()) && (super::is_implemented(&name.to_str_lossy()) || host.has_function(&name.to_str_lossy()) || super::is_vim_builtin(&name.to_str_lossy())))
                || name.as_bytes().contains(&b'#');
            if !exists {
                return Err(Fail::Error(VimError::new(700, format!("Unknown function: {}", name.to_str_lossy()))));
            }
            Funcref::named(canonical)
        }
        _ => return Err(Fail::Error(VimError::new(129, "Function name required"))),
    };
    if bound_args.is_empty() && bound_dict.is_none() {
        return Ok(Value::Funcref(base));
    }
    let mut all = base.args().to_vec();
    all.extend(bound_args);
    let dict = bound_dict.or_else(|| base.dict().cloned());
    Ok(Value::Funcref(base.bind(all, dict, false)))
}

/// `exists()`.
pub(crate) fn exists(rt: &mut Runtime, host: &mut dyn ScriptHost, query: &str) -> EvalResult<i64> {
    let query = query.trim();
    if let Some(name) = query.strip_prefix('&') {
        let name = name.trim_start_matches("l:").trim_start_matches("g:");
        return Ok(host.option_exists(name, false) as i64);
    }
    if let Some(name) = query.strip_prefix('+') {
        let name = name.trim_start_matches("l:").trim_start_matches("g:");
        return Ok(host.option_exists(name, true) as i64);
    }
    if let Some(name) = query.strip_prefix('$') {
        return Ok(host.getenv(name).is_some() as i64);
    }
    if let Some(name) = query.strip_prefix('*') {
        if name.is_empty() {
            return Ok(0);
        }
        if let Ok(Value::Funcref(_)) = rt.lookup_variable(host, name.as_bytes(), false) {
            return Ok(1);
        }
        return Ok(rt.function_exists(host, name.as_bytes()) as i64);
    }
    if let Some(name) = query.strip_prefix('?') {
        return Ok(rt.function_exists(host, name.as_bytes()) as i64);
    }
    if let Some(name) = query.strip_prefix(':') {
        let name = name.trim_end_matches('!');
        if name.is_empty() {
            return Ok(0);
        }
        if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            if rt.state.user_command(name).is_some() {
                return Ok(2);
            }
            let candidates = rt.state.user_commands_with_prefix(name);
            if candidates.len() > 1 {
                return Ok(3);
            }
            if candidates.len() == 1 {
                return Ok(1);
            }
        }
        let Some(full) = crate::vimscript::excmds::lookup(name) else { return Ok(0) };
        let implemented = crate::vimscript::commands::engine_command(full) || host.command_exists(full);
        return Ok(if !implemented { 0 } else if full == name { 2 } else { 1 });
    }
    if let Some(rest) = query.strip_prefix('#') {
        return Ok(autocmd_exists(rt, rest) as i64);
    }
    // A variable, optionally with subscripts.
    let bytes = query.as_bytes();
    let sid = rt.current_sid();
    let scope = rt.current_scope();
    let found = rt.with_context(scope, sid, |rt| {
        let saved = rt.emsg_silent;
        rt.emsg_silent += 1;
        let mut eval = crate::vimscript::expr::Eval::new(rt, host, bytes, 0, true);
        let result: EvalResult<bool> = (|| {
            let Some(name) = eval.parse_name()? else { return Ok(false) };
            if name.is_empty() {
                return Ok(false);
            }
            let value = match eval.rt.lookup_variable(eval.host, &name, false) {
                Ok(value) => value,
                Err(_) => return Ok(false),
            };
            let value = eval.subscripts(value, None)?;
            let _ = value;
            Ok(eval.at_end())
        })();
        eval.rt.emsg_silent = saved;
        result.or(Ok(false))
    });
    Ok(found.unwrap_or(false) as i64)
}

fn autocmd_exists(rt: &mut Runtime, query: &str) -> bool {
    // "#event", "#group", "#group#event", "#group#event#pattern", "##event"
    if let Some(event) = query.strip_prefix('#') {
        return crate::vimscript::autocmd::event_index(event).is_some_and(crate::vimscript::autocmd::is_fired);
    }
    let parts: Vec<&str> = query.split('#').collect();
    let table = rt.state.autocmds();
    let (group, rest) = match table.find_group(parts[0]) {
        Some(group) => (Some(group), &parts[1..]),
        None => (None, &parts[..]),
    };
    if rest.is_empty() {
        return group.is_some();
    }
    let Some(event) = crate::vimscript::autocmd::event_index(rest[0]) else { return false };
    table.entries().iter().any(|entry| {
        entry.event == event
            && group.is_none_or(|group| entry.group == group)
            && rest.get(1).is_none_or(|pattern| match &entry.pattern {
                crate::vimscript::autocmd::AutocmdPattern::Pattern(own) => crate::vimscript::autocmd::pattern_matches(own, pattern, true),
                crate::vimscript::autocmd::AutocmdPattern::Buffer => *pattern == "<buffer>",
            })
    })
}

/// The engine's answer to `has()`; the host may override any feature.
pub(crate) fn has(host: &dyn ScriptHost, feature: &str) -> bool {
    if let Some(answer) = host.feature(feature) {
        return answer;
    }
    let lower = feature.to_ascii_lowercase();
    if let Some(patch) = lower.strip_prefix("patch-") {
        let mut parts = patch.split('.').map(|part| part.parse::<i64>().unwrap_or(i64::MAX));
        let (major, minor, number) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        let version = major * 100 + minor;
        return version < crate::vimscript::vvars::VERSION || (version == crate::vimscript::vvars::VERSION && number == 0);
    }
    if let Some(number) = lower.strip_prefix("patch") {
        return number.parse::<i64>().is_ok_and(|number| number == 0);
    }
    let macos = cfg!(target_os = "macos");
    let windows = cfg!(windows);
    match lower.as_str() {
        "vim9script" | "eval" | "autocmd" | "syntax" | "user_commands" | "lambda" | "float" | "num64" | "multi_byte"
        | "multi_byte_encoding" | "insert_expand" | "listcmds" | "localmap" | "modify_fname" | "mouse"
        | "path_extra" | "reltime" | "vertsplit" | "windows" | "cmdline_compl" | "cmdline_hist"
        | "comments" | "extra_search" | "file_in_path" | "find_in_path" | "jumplist" | "linebreak"
        | "clipboard" | "clipboard_working" | "gui" | "gui_running" | "iconv" | "wildignore"
        | "byte_offset" | "termguicolors" | "smartindent" | "textobjects" | "visual" | "visualextra"
        | "virtualedit" | "wildmenu" | "keymap" | "title" | "statusline" | "user-commands" | "vreplace"
        | "unicode" | "emacs_tags" | "dialog_con" | "builtin_terms" | "gettext" | "modify-fname"
        | "fork" | "startuptime" | "viem" | "digraphs" | "scrollbind" | "cursorbind" | "autochdir"
        | "smarttab" | "mksession" | "quickfix" | "postscript" => !matches!(lower.as_str(), "quickfix" | "postscript" | "mksession"),
        "mac" | "macunix" | "osx" | "osxdarwin" | "bsd" => macos,
        "unix" => !windows,
        "win32" | "win64" | "win95" | "win16" | "win32unix" | "gui_win32" | "gui_win64" => windows && lower != "win32unix",
        "fname_case" => !macos && !windows,
        "gui_macvim" | "gui_gtk" | "gui_gtk2" | "gui_gtk3" | "gui_gnome" | "gui_kde" | "gui_motif" | "gui_athena" | "gui_photon"
        | "gui_haiku" | "nvim" | "terminal" | "job" | "channel" | "timers" | "python"
        | "python3" | "pythonx" | "python_compiled" | "python3_compiled" | "perl" | "lua" | "ruby"
        | "tcl" | "mzscheme" | "spell" | "conceal" | "signs" | "folding" | "diff" | "cscope"
        | "netbeans_intg" | "netbeans_enabled" | "balloon_eval" | "balloon_eval_term" | "balloon_multiline"
        | "dialog_gui" | "menu" | "toolbar" | "xterm_clipboard" | "clientserver" | "sound"
        | "popupwin" | "textprop" | "langmap" | "persistent_undo" | "viminfo" | "writebackup"
        | "arabic" | "rightleft" | "farsi" | "crypt" | "sodium" | "acl" | "beval" | "browse"
        | "cindent" | "lispindent" | "libcall" | "printer" | "profile" | "vertsplit_bar" | "wayland"
        | "x11" | "xim" | "xfontset" | "xpm" | "xsmp" | "tabpanel" | "tag_binary" | "tag_old_static"
        | "cmdwin" | "ole" | "gui_tabline" | "showcmd" | "ruler" | "spell_suggest" | "vim_starting"
        | "hangul_input" | "mouse_dec" | "mouse_gpm" | "mouse_jsbterm" | "mouse_netterm"
        | "mouse_pterm" | "mouse_sgr" | "mouse_sysmouse" | "mouse_urxvt" | "mouse_xterm" | "mouseshape"
        | "multi_lang" | "ttyin" | "ttyout" | "xattr" | "vtp" | "filterpipe" | "dnd" | "eval_quoted" => false,
        _ => false,
    }
}
