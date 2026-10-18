//! Confined host for the shared Vim script interpreter. Script control flow,
//! values, functions and command expansion have exactly one implementation.
use super::*;
use crate::vimscript::{
    EvalResult, Fail, HostCommand, Message, OptionScope, Runtime, ScriptHost, ScriptState, Value,
    VimError,
};

fn fatal(error: impl Into<String>) -> Fail {
    Fail::Interrupt(error.into())
}
fn source_argument(source: &str) -> String {
    let mut result = String::new();
    let mut chars = source.trim_end().chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && chars.peek().is_some_and(|c| {
                matches!(
                    c,
                    ' ' | '\t'
                        | '\\'
                        | '%'
                        | '#'
                        | '|'
                        | '"'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '*'
                        | '?'
                        | '!'
                        | '<'
                        | '>'
                )
            })
        {
            result.push(chars.next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}
fn query_argument(value: &Value) -> Result<setup::QueryValue, VimError> {
    Ok(match value {
        Value::Number(n) => setup::QueryValue::Number(*n),
        Value::String(s) => setup::QueryValue::Text(s.to_str_lossy().into_owned()),
        _ => return Err(VimError::new(745, "Expected a scalar query argument")),
    })
}
fn script_value(old: setup::QueryValue) -> Value {
    match old {
        setup::QueryValue::Number(n) => Value::Number(n),
        setup::QueryValue::Text(s) => Value::from(s),
        setup::QueryValue::List(items) => {
            Value::list(items.into_iter().map(script_value).collect())
        }
    }
}

impl Loader<'_> {
    fn highlight_id(&mut self, name: &str) -> usize {
        if let Some(index) = self
            .highlight_ids
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
        {
            return index + 1;
        }
        self.highlight_ids.push(name.into());
        self.highlight_ids.len()
    }
    fn highlights(&self) -> &super::super::colorscheme::VimHighlights {
        self.colors
            .as_ref()
            .map_or(&self.script_highlights, |session| &session.highlights)
    }

    pub(super) fn new_script_runtime(&mut self) -> Runtime {
        self.script_runtime.take().unwrap_or_else(|| {
            let state = ScriptState::new();
            state.set_vvar("version", Value::Number(902));
            state.set_vvar("lang", Value::from("C"));
            if self.colors.is_some() {
                state.globals().insert("syntax_on", Value::Number(1));
            }
            for (name, n) in &self.setup.flags {
                let dict = if name.starts_with("g:") {
                    state.globals()
                } else {
                    state.buffer_vars()
                };
                dict.insert(&name[2..], Value::Number(*n));
            }
            let mut rt = Runtime::new(state);
            rt.set_fuel(8_000_000);
            rt
        })
    }
    pub(super) fn script_source(&mut self, file: &str, text: &str) {
        let mut rt = self.new_script_runtime();
        let result = self.run_source(&mut rt, file, text);
        self.finish_script_call(&mut rt, file, result);
        self.script_runtime = Some(rt);
    }
    pub(super) fn finish_script_call(
        &mut self,
        rt: &mut Runtime,
        file: &str,
        result: EvalResult<()>,
    ) {
        if let Err(fail) = result {
            let context = rt
                .interrupt_context
                .take()
                .unwrap_or_else(|| rt.error_context());
            let message = match fail {
                Fail::Error(e) => e.text(),
                Fail::Interrupt(_) if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) => {
                    "syntax compilation cancelled".into()
                }
                Fail::Interrupt(e) => e,
                Fail::Exception(e) => format!("uncaught exception: {}", e.text()),
                Fail::Aborted => "syntax setup aborted".into(),
            };
            self.err(
                context.file.as_deref().unwrap_or(file),
                context.lnum.unwrap_or(0),
                message,
            );
        }
        for error in rt.errors.drain(..) {
            self.err(
                error.file.as_deref().unwrap_or(file),
                error.lnum.unwrap_or(0),
                error.error.text(),
            );
        }
    }
    fn run_source(&mut self, rt: &mut Runtime, file: &str, text: &str) -> EvalResult<()> {
        self.bytes = self.bytes.saturating_add(text.len());
        self.files += 1;
        if self.bytes > self.limits.source_bytes || self.files > self.limits.files {
            return Err(fatal("syntax load budget exceeded"));
        }
        self.program.source_files.push(file.into());
        for byte in file.bytes().chain(text.bytes()) {
            self.program.generation =
                (self.program.generation ^ u64::from(byte)).wrapping_mul(1099511628211);
        }
        let (lines, script_local) = crate::vimscript::source::syntax_script_lines(text.as_bytes())
            .map_err(|(line, error)| {
                self.err(file, line, &error);
                fatal(error)
            })?;
        let caller = self.current_source.replace(PathBuf::from(file));
        let previous = self.setup.set_source_file(file);
        let result = rt.source_lines_now(self, file, lines, script_local);
        self.setup.set_source_file(&previous);
        self.current_source = caller;
        result
    }
    fn script_path(&self, name: &str) -> Result<PathBuf, VimError> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| VimError::new(484, "Includes require a syntax directory"))?;
        let expanded;
        let name = if let Some(tail) = name
            .strip_prefix("<sfile>:p:h/")
            .or_else(|| name.strip_prefix("<sfile>:h/"))
        {
            expanded = self
                .current_source
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or(root)
                .join(tail)
                .to_string_lossy()
                .into_owned();
            &expanded
        } else {
            name
        };
        let name = name
            .strip_prefix("$VIMRUNTIME/syntax/")
            .or_else(|| name.strip_prefix("syntax/"))
            .unwrap_or(name);
        let base = if name.starts_with("autoload/") {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        let path = base
            .join(name)
            .canonicalize()
            .map_err(|e| VimError::new(484, e.to_string()))?;
        let autoload = root.parent().unwrap_or(root).join("autoload");
        if !path.starts_with(root) && !path.starts_with(&autoload) {
            return Err(VimError::plain(
                "include escapes configured syntax directory",
            ));
        }
        Ok(path)
    }
    pub(super) fn script_file(&mut self, rt: &mut Runtime, name: &str) -> EvalResult<()> {
        let path = self.script_path(name).map_err(|error| {
            if error.code == 0 {
                fatal(error.message)
            } else {
                Fail::Error(error)
            }
        })?;
        let remaining = self.limits.source_bytes.saturating_sub(self.bytes);
        if self.files >= self.limits.files
            || std::fs::metadata(&path)
                .map_err(|e| VimError::new(484, e.to_string()))?
                .len()
                > remaining as u64
        {
            return Err(fatal("syntax load budget exceeded"));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(remaining as u64 + 1).read_to_end(&mut bytes))
            .map_err(|e| VimError::new(484, e.to_string()))?;
        if bytes.len() > remaining {
            return Err(fatal("syntax source byte budget exceeded"));
        }
        let text = commands::decode_source(&bytes).map_err(fatal)?;
        self.bytes = self
            .bytes
            .saturating_add(bytes.len().saturating_sub(text.len()));
        for byte in bytes {
            self.program.generation =
                (self.program.generation ^ u64::from(byte)).wrapping_mul(1099511628211);
        }
        self.run_source(rt, &path.to_string_lossy(), &text)
    }
    fn script_glob(&mut self, pattern: &str) -> EvalResult<Vec<PathBuf>> {
        let root = self
            .root
            .clone()
            .ok_or_else(|| fatal("includes require a syntax directory"))?;
        self.script_glob_at(root, pattern.strip_prefix("syntax/").unwrap_or(pattern))
    }
    fn script_glob_at(&mut self, root: PathBuf, pattern: &str) -> EvalResult<Vec<PathBuf>> {
        if Path::new(pattern)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(fatal("include escapes configured syntax directory"));
        }
        let components = pattern.split('/').collect::<Vec<_>>();
        if components.len() > 128 {
            return Err(fatal("runtime glob depth budget exceeded"));
        }
        let mut pending = vec![(root.clone(), 0)];
        let mut results = Vec::new();
        while let Some((parent, index)) = pending.pop() {
            self.charge_statement_work().map_err(fatal)?;
            if index == components.len() {
                results.push(parent);
                continue;
            }
            let component = components[index];
            if component == "**" {
                pending.push((parent.clone(), index + 1));
            }
            if component.contains(['*', '?', '[', '{']) {
                if !parent.is_dir() {
                    continue;
                }
                for entry in std::fs::read_dir(&parent).map_err(|e| fatal(e.to_string()))? {
                    self.charge_statement_work().map_err(fatal)?;
                    let entry = entry.map_err(|e| fatal(e.to_string()))?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') && !component.starts_with('.') {
                        continue;
                    }
                    if crate::vimscript::autocmd::glob_match_bounded(
                        component,
                        &name,
                        false,
                        &mut self.statement_fuel,
                    )
                    .map_err(fatal)?
                    {
                        let path = entry
                            .path()
                            .canonicalize()
                            .map_err(|e| fatal(e.to_string()))?;
                        if !path.starts_with(&root) {
                            return Err(fatal("include escapes configured syntax directory"));
                        }
                        if component == "**" {
                            // No symlink recursion, including cycles inside the
                            // runtime. Bundled resources contain no symlinks.
                            if entry
                                .file_type()
                                .map_err(|e| fatal(e.to_string()))?
                                .is_dir()
                            {
                                pending.push((path, index));
                            }
                        } else {
                            pending.push((path, index + 1));
                        }
                    }
                }
            } else if let Ok(path) = parent.join(component).canonicalize() {
                if !path.starts_with(&root) {
                    return Err(fatal("include escapes configured syntax directory"));
                }
                pending.push((path, index + 1));
            }
        }
        results.sort();
        results.dedup();
        Ok(results)
    }
    pub(super) fn script_import(
        &mut self,
        rt: &mut Runtime,
        patterns: &str,
        all: bool,
    ) -> EvalResult<()> {
        for pattern in patterns.split_whitespace() {
            for path in self.script_glob(pattern)? {
                if path.is_file() {
                    self.script_file(rt, &path.to_string_lossy())?;
                    if !all {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    fn query(&mut self, name: &str, args: &[Value]) -> EvalResult<Value> {
        if name == "hlexists" && self.colors.is_some() {
            let name = args
                .first()
                .ok_or_else(|| VimError::argument(119, name))?
                .to_text()?;
            return Ok(Value::Number(self.highlights().contains(&name) as i64));
        }
        let mut args = args
            .iter()
            .map(query_argument)
            .collect::<Result<Vec<_>, _>>()?;
        if name == "getline" {
            for arg in &mut args {
                if let setup::QueryValue::Text(s) = arg {
                    *arg = if s == "$" {
                        self.setup
                            .context_function("line", vec![setup::QueryValue::Text("$".into())])
                            .map_err(fatal)?
                    } else {
                        setup::QueryValue::Number(s.parse().unwrap_or(0))
                    };
                }
            }
        }
        self.setup
            .context_function(name, args)
            .map(script_value)
            .map_err(|e| {
                if let Some((code, message)) = e.strip_prefix('E').and_then(|s| s.split_once(": "))
                {
                    if let Ok(code) = code.parse() {
                        return Fail::Error(VimError::new(code, message));
                    }
                }
                fatal(e)
            })
    }
    fn option_name(name: &str) -> &str {
        match name {
            "cpo" => "cpoptions",
            "isk" => "iskeyword",
            "ft" => "filetype",
            "rtp" => "runtimepath",
            "bg" => "background",
            "ts" => "tabstop",
            "sw" => "shiftwidth",
            "et" => "expandtab",
            "fdm" => "foldmethod",
            "fde" => "foldexpr",
            "fdt" => "foldtext",
            "fdi" => "foldignore",
            "fen" => "foldenable",
            "cole" => "conceallevel",
            "cin" => "cindent",
            "com" => "comments",
            "isi" => "isident",
            "smc" => "synmaxcol",
            "syn" => "syntax",
            "ic" => "ignorecase",
            "cpt" => "complete",
            "fo" => "formatoptions",
            "sts" => "softtabstop",
            "fenc" => "fileencoding",
            "enc" => "encoding",
            "cocu" => "concealcursor",
            _ => name,
        }
    }
    fn option_default(&self, name: &str) -> Option<Value> {
        Some(match Self::option_name(name) {
            "cpoptions" => Value::from("aABceFs"),
            "iskeyword" => Value::from("@,48-57,_,192-255"),
            "filetype" => script_value(self.setup.context_option("&filetype").ok()?),
            "runtimepath" => Value::from(
                self.root
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map_or(String::new(), |p| p.to_string_lossy().into_owned()),
            ),
            "background" => Value::from("light"),
            "encoding" => Value::from("utf-8"),
            "compatible" | "ignorecase" | "expandtab" | "cindent" | "lisp" | "conceallevel"
            | "spell" => Value::Number(0),
            "foldenable" | "wrap" | "termguicolors" => Value::Number(1),
            "tabstop" | "shiftwidth" => Value::Number(8),
            "synmaxcol" => Value::Number(3000),
            "t_Co" => Value::Number(256),
            "foldmethod" => Value::from("manual"),
            "foldexpr" => Value::from("0"),
            "foldtext" => Value::from("foldtext()"),
            "foldignore" => Value::from("#"),
            "isident" => Value::from("@,48-57,_,192-255"),
            "comments" => Value::from("s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-"),
            "syntax" | "buftype" | "fileencoding" | "concealcursor" => Value::from(""),
            "helplang" => Value::from("en"),
            "formatoptions" => Value::from("tcq"),
            "softtabstop" => Value::Number(0),
            "pyxversion" => Value::Number(3),
            _ => return None,
        })
    }
    pub(super) fn script_set(&mut self, rt: &mut Runtime, text: &str) -> EvalResult<()> {
        let mut tokens = Vec::new();
        let mut token = String::new();
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                token.push(
                    chars
                        .next()
                        .ok_or_else(|| VimError::new(474, "Trailing option escape"))?,
                );
            } else if ch.is_whitespace() {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            } else {
                token.push(ch);
            }
        }
        if !token.is_empty() {
            tokens.push(token);
        }
        for token in &tokens {
            let token = token.as_str();
            if token.starts_with('"') {
                break;
            }
            let (raw, tail) = token
                .find(['=', '&', '+', '-', '^', '?', '<'])
                .map_or((token, ""), |i| (&token[..i], &token[i..]));
            let (raw, negative) = if self.option_default(raw).is_none() {
                raw.strip_prefix("no").map_or((raw, false), |s| (s, true))
            } else {
                (raw, false)
            };
            let name = Self::option_name(raw);
            let default = self
                .option_default(name)
                .ok_or_else(|| VimError::new(518, format!("Unknown option: {raw}")))?;
            let current = self.get_option(name, OptionScope::Local)?;
            let new = if tail.starts_with('&') || tail == "<" {
                default
            } else if tail == "?" {
                continue;
            } else if let Some(s) = tail.strip_prefix('=') {
                if matches!(default, Value::Number(_)) {
                    Value::Number(
                        s.parse()
                            .map_err(|_| VimError::new(521, "Number required"))?,
                    )
                } else {
                    Value::from(s)
                }
            } else if let Some(s) = tail.strip_prefix("+=") {
                Value::from(format!(
                    "{}{}{s}",
                    current.to_text()?,
                    if name == "iskeyword" { "," } else { "" }
                ))
            } else if let Some(s) = tail.strip_prefix("-=") {
                Value::from(current.to_text()?.replace(s, ""))
            } else if tail.is_empty() {
                Value::Number(i64::from(!negative))
            } else {
                return Err(VimError::new(474, format!("Invalid option: {token}")).into());
            };
            self.set_option(rt, name, OptionScope::Local, &new)?;
        }
        Ok(())
    }
}
impl ScriptHost for Loader<'_> {
    fn autoload(&mut self, rt: &mut Runtime, name: &[u8]) -> EvalResult<bool> {
        let name = String::from_utf8_lossy(name);
        let Some((namespace, _)) = name.rsplit_once('#') else {
            return Ok(false);
        };
        if !namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'#'))
        {
            return Err(fatal("invalid syntax autoload namespace"));
        }
        let path = format!("autoload/{}.vim", namespace.replace('#', "/"));
        if self.loaded_autoload.contains(&path) {
            return Ok(false);
        }
        self.loaded_autoload.insert(path.clone());
        self.script_file(rt, &path)?;
        Ok(true)
    }
    fn runtime_event(&mut self, rt: &mut Runtime, event: &str, matched: &str) -> EvalResult<()> {
        if event.eq_ignore_ascii_case("syntax") && self.colors.is_none() {
            if !matched
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Err(fatal("invalid syntax event language"));
            }
            rt.state.buffer_vars().remove(b"current_syntax");
            for language in matched.split('.') {
                self.script_import(rt, &format!("syntax/{language}.vim"), true)?;
            }
        }
        Ok(())
    }
    fn handles_runtime_commands(&self) -> bool {
        true
    }
    fn execute(&mut self, rt: &mut Runtime, command: &HostCommand) -> EvalResult<()> {
        let rest = if command.name == Some("syntax") {
            command.argument.trim_start()
        } else {
            command.argument.trim()
        };
        if self.colors.is_some() {
            return self.color_command(rt, command);
        }
        match command.name {
            Some("syntax") => {
                let (kind, tail) = word(rest).map_err(fatal)?;
                if kind == "include" {
                    let rest = tail;
                    let (first, tail) = word(rest).map_err(fatal)?;
                    let (cluster, source) = if let Some(name) = first.strip_prefix('@') {
                        (Some(self.program.intern_cluster_name(name)), tail)
                    } else {
                        (None, rest)
                    };
                    if let Some(name) = &cluster {
                        let groups = self.program.clusters.entry(name.clone()).or_default();
                        self.setup
                            .update_syntax_cluster(name, groups)
                            .map_err(fatal)?;
                    }
                    self.include_stack.push(cluster);
                    let result = if let Some(pattern) = source.strip_prefix("runtime! ") {
                        self.script_import(rt, pattern, true)
                    } else if let Some(pattern) = source.strip_prefix("runtime ") {
                        self.script_import(rt, pattern, false)
                    } else {
                        self.script_file(rt, &source_argument(source))
                    };
                    self.include_stack.pop();
                    result
                } else if kind == "list" {
                    let output = self
                        .query("execute", &[Value::from(format!("syntax {rest}"))])?
                        .to_text()?;
                    rt.emit(self, Message::Output(output));
                    Ok(())
                } else {
                    self.syntax(rest).map_err(|error| {
                        fatal(format!(
                            "{error} (:syntax {})",
                            rest.chars().take(300).collect::<String>()
                        ))
                    })?;
                    if rest.trim() == "clear" && self.include_stack.is_empty() {
                        rt.state.buffer_vars().remove(b"current_syntax");
                    }
                    Ok(())
                }
            }
            Some("highlight") => {
                self.highlight(rest, command.bang).map_err(fatal)?;
                self.script_highlights
                    .command(rest, command.bang)
                    .map_err(fatal)?;
                Ok(())
            }
            Some("set" | "setlocal" | "setglobal") => self.script_set(rt, rest),
            // Matchit installs editor motions; its metadata may be retained
            // in this private script state without loading an editor plugin.
            Some("packadd") if command.bang && rest == "matchit" => Ok(()),
            Some("foldopen" | "foldclose") => Ok(()),
            Some("source") => self.script_file(rt, &source_argument(rest)),
            Some("runtime") => self.script_import(rt, rest, command.bang),
            _ => Err(fatal(format!(
                "unsupported syntax setup command: {}",
                command.text
            ))),
        }
    }
    fn get_option(&mut self, name: &str, _scope: OptionScope) -> Result<Value, VimError> {
        let name = Self::option_name(name);
        self.script_options
            .get(name)
            .cloned()
            .or_else(|| self.option_default(name))
            .ok_or_else(|| VimError::new(113, format!("Unknown option: {name}")))
    }
    fn set_option(
        &mut self,
        rt: &mut Runtime,
        name: &str,
        _scope: OptionScope,
        value: &Value,
    ) -> EvalResult<()> {
        let name = Self::option_name(name);
        if self.option_default(name).is_none() {
            return Err(VimError::new(113, format!("Unknown option: {name}")).into());
        }
        if self.colors.is_some() && name == "background" {
            self.color_background(rt, &value.to_text()?)
                .map_err(Fail::Error)?;
        }
        if name == "iskeyword" {
            self.set_buffer_keyword(&value.to_text()?).map_err(fatal)?;
        }
        if name == "ignorecase" {
            self.setup.ignore_case = value.to_number()? != 0;
        }
        self.script_options.insert(name.into(), value.clone());
        Ok(())
    }
    fn option_exists(&self, name: &str, _working: bool) -> bool {
        self.option_default(name).is_some()
    }
    fn get_register(&mut self, _name: char) -> Result<Value, VimError> {
        Err(VimError::plain("syntax setup cannot inspect registers"))
    }
    fn set_register(&mut self, _name: char, _value: &Value, _append: bool) -> Result<(), VimError> {
        Err(VimError::plain("syntax setup cannot change registers"))
    }
    fn call_function(
        &mut self,
        _rt: &mut Runtime,
        name: &str,
        args: &[Value],
    ) -> Option<EvalResult<Value>> {
        Some(match name {
            "hlID" => (|| {
                let name = args
                    .first()
                    .ok_or_else(|| VimError::argument(119, name))?
                    .to_text()?;
                if self
                    .query("hlexists", &[Value::from(name.clone())])?
                    .to_number()?
                    == 0
                {
                    return Ok(Value::Number(0));
                }
                Ok(Value::Number(self.highlight_id(&name) as i64))
            })(),
            "synIDtrans" => (|| {
                let id = args
                    .first()
                    .ok_or_else(|| VimError::argument(119, name))?
                    .to_number()?;
                let Some(name) = id
                    .checked_sub(1)
                    .and_then(|n| self.highlight_ids.get(n as usize))
                    .cloned()
                else {
                    return Ok(Value::Number(0));
                };
                let name = if self.colors.is_some() {
                    self.highlights()
                        .resolve(&name)
                        .map_or(name, |group| group.group)
                } else {
                    self.program.effective_group(&name).to_owned()
                };
                Ok(Value::Number(self.highlight_id(&name) as i64))
            })(),
            "synIDattr" => (|| {
                if args.len() < 2 {
                    return Err(VimError::argument(119, name).into());
                }
                let id = args[0].to_number()?;
                let what = args[1].to_text()?;
                let mode = args
                    .get(2)
                    .map(Value::to_text)
                    .transpose()?
                    .unwrap_or_else(|| "cterm".into());
                let name = id
                    .checked_sub(1)
                    .and_then(|n| self.highlight_ids.get(n as usize));
                Ok(Value::from(name.map_or_else(String::new, |name| {
                    if what == "name" {
                        name.clone()
                    } else {
                        self.highlights().attribute(name, &what, &mode)
                    }
                })))
            })(),
            "getcurpos" | "getpos" => (|| {
                if name == "getpos" && (args.len() != 1 || args[0].to_text()? != ".") {
                    return Err(fatal(
                        "syntax setup position queries are confined to its private cursor",
                    ));
                }
                if name == "getcurpos" && !args.is_empty() {
                    return Err(VimError::argument(118, name).into());
                }
                let mut position = vec![
                    Value::Number(0),
                    Value::Number(self.setup.cursor.0 as i64),
                    Value::Number(self.setup.cursor.1 as i64),
                    Value::Number(0),
                ];
                if name == "getcurpos" {
                    position.push(Value::Number(self.setup.cursor.1 as i64));
                }
                Ok(Value::list(position))
            })(),
            "cursor" => (|| {
                let values = if let [Value::List(list)] = args {
                    list.snapshot()
                } else {
                    args.to_vec()
                };
                if values.len() < 2 {
                    return Err(VimError::argument(119, name).into());
                }
                let result = self
                    .setup
                    .set_cursor(values[0].to_number()?, values[1].to_number()?)
                    .map_err(fatal)?;
                Ok(Value::Number(result))
            })(),
            "setpos" => (|| {
                let [position, Value::List(list)] = args else {
                    return Err(VimError::new(474, "Invalid setpos arguments").into());
                };
                if position.to_text()? != "." {
                    return Err(fatal("syntax setup cannot change marks"));
                }
                let values = list.snapshot();
                if values.len() < 4 {
                    return Err(VimError::new(474, "Invalid position").into());
                }
                if values[0].to_number()? != 0 {
                    return Err(fatal("syntax setup cannot access other buffers"));
                }
                let result = self
                    .setup
                    .set_cursor(values[1].to_number()?, values[2].to_number()?)
                    .map_err(fatal)?;
                Ok(Value::Number(result))
            })(),
            _ => self.query(name, args),
        })
    }
    fn has_function(&self, name: &str) -> bool {
        matches!(
            name,
            "getline"
                | "line"
                | "search"
                | "hlexists"
                | "hlID"
                | "synIDtrans"
                | "synIDattr"
                | "bufname"
                | "col"
                | "cursor"
                | "getcurpos"
                | "getpos"
                | "setpos"
        )
    }
    fn feature(&self, name: &str) -> Option<bool> {
        if self.colors.is_some() && matches!(name, "gui" | "gui_running" | "termguicolors") {
            return Some(true);
        }
        // Version queries share the evaluator's declared Vim version. Ambient
        // clocks and editor presentation features are not capabilities here.
        if name.starts_with("patch") {
            return None;
        }
        Some(matches!(
            name,
            "syntax"
                | "eval"
                | "multi_byte"
                | "multi_byte_encoding"
                | "float"
                | "lambda"
                | "num64"
                | "autocmd"
                | "user_commands"
        ))
    }
    fn command_exists(&self, name: &str) -> bool {
        matches!(
            name,
            "syntax" | "highlight" | "set" | "setlocal" | "runtime" | "source"
        )
    }
    fn message(&mut self, _message: Message) {}
    fn ignorecase(&self) -> bool {
        self.script_options
            .get("ignorecase")
            .is_some_and(|value| value.to_number().is_ok_and(|n| n != 0))
    }
    fn interrupted(&mut self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }
    fn getenv(&self, name: &str) -> Option<String> {
        if name == "VIMRUNTIME" {
            self.root
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        }
    }
    fn read_file(&mut self, _path: &str) -> Result<Vec<u8>, VimError> {
        Err(VimError::plain(
            "syntax reads require a confined runtime import",
        ))
    }
    fn override_builtin(
        &mut self,
        _rt: &mut Runtime,
        name: &str,
        args: &[Value],
    ) -> Option<EvalResult<Value>> {
        match name {
            "expand" | "fnamemodify" => Some(self.query(name, args)),
            "filereadable" => Some((|| {
                let path = args
                    .first()
                    .ok_or_else(|| VimError::argument(119, name))?
                    .to_text()?;
                Ok(Value::Number(
                    self.script_path(&path).is_ok_and(|p| p.is_file()) as i64,
                ))
            })()),
            "globpath" => Some((|| {
                if !(2..=5).contains(&args.len()) {
                    return Err(
                        VimError::argument(if args.len() < 2 { 119 } else { 118 }, name).into(),
                    );
                }
                let directories = args[0].to_text()?;
                let pattern = args[1].to_text()?;
                let syntax = self
                    .root
                    .clone()
                    .ok_or_else(|| fatal("includes require a syntax directory"))?;
                let runtime = syntax.parent().unwrap_or(&syntax).to_owned();
                let mut paths = Vec::new();
                for directory in directories.split(',').filter(|s| !s.is_empty()) {
                    let directory = PathBuf::from(directory)
                        .canonicalize()
                        .map_err(|e| fatal(e.to_string()))?;
                    if directory != runtime && !directory.starts_with(&syntax) {
                        return Err(fatal("globpath escapes configured runtime directory"));
                    }
                    paths.extend(self.script_glob_at(directory, &pattern)?);
                }
                let paths = paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                Ok(
                    if args.get(3).is_some_and(|v| v.to_number().unwrap_or(0) != 0) {
                        Value::list(paths.into_iter().map(Value::from).collect())
                    } else {
                        Value::from(paths.join("\n"))
                    },
                )
            })()),
            "executable" => Some(Ok(Value::Number(0))),
            "exepath" => Some(Ok(Value::from(""))),
            "environ" | "setenv" | "expandcmd" | "filewritable" | "isdirectory" | "getftype"
            | "getfsize" | "getftime" | "getfperm" | "glob" | "resolve" | "readfile"
            | "readdir" | "readblob" | "getcwd" | "haslocaldir" | "hostname" | "tempname"
            | "mkdir" | "delete" | "rename" | "writefile" | "localtime" | "strftime"
            | "reltime" | "reltimestr" | "reltimefloat" | "getpid" | "system" | "systemlist"
            | "job_start" | "libcall" | "libcallnr" | "timer_start" | "chdir" => Some(Err(fatal(
                format!("syntax setup cannot call ambient {name}()"),
            ))),
            _ => None,
        }
    }
}
