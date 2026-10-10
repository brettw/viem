//! Logical syntax-package statements. Expansion stays inside the declaration
//! loader, with shared instruction/storage limits and no editor side effects.
use super::setup::Value;
use super::*;

#[cfg(test)]
#[path = "review_tests.rs"]
mod review_tests;

#[derive(Clone)]
pub(super) struct Statement {
    line: usize,
    text: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Next,
    Break,
    Continue,
    Finish,
    Error,
}

pub(super) fn validate_function_output(commands: &[String]) -> Result<(), String> {
    // Helpers run to completion before their output reaches the loader. Only
    // declarations whose effects cannot change later setup evaluation can be
    // deferred. Parse every command separator and reject expansion wrappers.
    for source in commands {
        if source.lines().any(|line| {
            line.trim_start()
                .trim_start_matches(':')
                .trim_start()
                .split_whitespace()
                .next()
                == Some("vim9script")
        }) {
            return Err("setup function output cannot change script mode".into());
        }
        for statement in logical_lines(source).map_err(|(_, error)| error)? {
            let (command, rest) = word(&statement.text)?;
            let allowed = match command_name(command) {
                "highlight" => true,
                "syntax" => word(rest).is_ok_and(|(kind, _)| {
                    matches!(
                        kind,
                        "keyword"
                            | "match"
                            | "region"
                            | "cluster"
                            | "case"
                            | "sync"
                            | "conceal"
                            | "spell"
                            | "iskeyword"
                    )
                }),
                _ => false,
            };
            if !allowed {
                return Err("setup function output supports only syntax and highlight declarations without setup side effects".into());
            }
        }
    }
    Ok(())
}

pub(super) fn command_name(name: &str) -> &str {
    let bare = name.trim_end_matches('!');
    for (full, minimum) in [
        ("syntax", 2),
        ("highlight", 2),
        ("execute", 3),
        ("function", 2),
        ("endfunction", 4),
        ("endif", 2),
        ("elseif", 5),
        ("else", 2),
        ("endfor", 4),
        ("endwhile", 4),
        ("finish", 4),
        ("runtime", 2),
        ("setlocal", 4),
        ("set", 2),
        ("source", 2),
        ("scriptencoding", 6),
        ("command", 3),
        ("delcommand", 4),
        ("unlet", 3),
        ("call", 3),
        ("delfunction", 4),
    ] {
        if bare.len() >= minimum && full.starts_with(bare) {
            return full;
        }
    }
    bare
}

pub(super) fn logical_lines(text: &str) -> Result<Vec<Statement>, (usize, String)> {
    let mut result = Vec::new();
    let mut physical = text.lines().enumerate().peekable();
    let mut vim9 = false;
    while let Some((index, raw)) = physical.next() {
        let mut logical = raw
            .trim_start()
            .trim_start_matches(':')
            .trim_start()
            .to_owned();
        if logical == "vim9script" || logical.starts_with("vim9script ") {
            vim9 = true;
            continue;
        }
        if vim9 && logical.starts_with('#') {
            continue;
        }
        if logical.starts_with('\\') {
            return Err((index + 1, "orphan continuation".into()));
        }
        while physical.peek().is_some_and(|(_, s)| {
            s.trim_start().starts_with('\\') || s.trim_start().starts_with("\"\\")
        }) {
            let (_, continuation) = physical.next().unwrap();
            if continuation.trim_start().starts_with("\"\\") {
                continue;
            }
            logical.push_str(continuation.trim_start().strip_prefix('\\').unwrap());
        }
        // Literal heredocs are setup lists, not executable source statements.
        if logical.starts_with("let ") {
            if let Some((binding, tail)) = logical.split_once("=<<") {
                let mut words = tail.split_whitespace();
                let mut trim = false;
                let first = words
                    .next()
                    .ok_or((index + 1, "missing heredoc terminator".into()))?;
                let end = if first == "trim" {
                    trim = true;
                    words.next().unwrap_or("")
                } else {
                    first
                };
                if end.is_empty()
                    || words.next().is_some_and(|word| !word.starts_with('"'))
                    || end == "eval"
                {
                    return Err((index + 1, "unsupported setup heredoc options".into()));
                }
                let mut values = Vec::new();
                let mut closed = false;
                let mut indentation = None;
                for (_, line) in physical.by_ref() {
                    if (if trim { line.trim() } else { line }) == end {
                        closed = true;
                        break;
                    }
                    let line = if trim {
                        let indent = *indentation
                            .get_or_insert_with(|| line.len() - line.trim_start().len());
                        let strip = (line.len() - line.trim_start().len()).min(indent);
                        &line[strip..]
                    } else {
                        line
                    };
                    values.push(format!("'{}'", line.replace('\'', "''")));
                }
                if !closed {
                    return Err((index + 1, "unterminated setup heredoc".into()));
                }
                logical = format!("{binding}= [{}]", values.join(","));
            }
        }
        if vim9 {
            if let Some(rest) = logical
                .strip_prefix("var ")
                .or_else(|| logical.strip_prefix("const "))
                .or_else(|| logical.strip_prefix("final "))
            {
                logical = format!("let {rest}");
            } else if let Some((name, rest)) = logical.split_once(char::is_whitespace) {
                if rest.trim_start().starts_with('=')
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':'))
                {
                    logical = format!("let {logical}");
                }
            }
        }
        for text in split_commands(&logical, vim9) {
            if !text.is_empty() {
                result.push(Statement {
                    line: index + 1,
                    text,
                });
            }
        }
    }
    Ok(result)
}

/// Locate separators without confusing expression strings or syntax patterns
/// (whose delimiter may itself be a quote or bar) with Ex comments/separators.
fn split_commands(source: &str, vim9: bool) -> Vec<String> {
    let mut result = Vec::new();
    let mut source = source.trim();
    while !source.is_empty() && !source.starts_with('"') {
        let (raw, rest) = source
            .split_once(char::is_whitespace)
            .unwrap_or((source, ""));
        let name = command_name(raw);
        if raw.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            result.push(source.to_owned());
            break;
        }
        // Bars in user-command definitions belong to the command body.
        if name == "command" {
            result.push(source.to_owned());
            break;
        }
        let expression = matches!(
            name,
            "if" | "elseif" | "let" | "execute" | "call" | "return" | "for" | "while" | "throw"
        );
        let syntax = name == "syntax";
        let kind = rest.split_whitespace().next().unwrap_or("");
        let mut i = raw.len();
        let mut match_pattern = syntax && (kind == "match" || kind == "sync");
        let mut words = 0;
        let bytes = source.as_bytes();
        let mut split = None;
        let mut comment = None;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            let token_start = i;
            if syntax
                && (source[i..].starts_with("start=")
                    || source[i..].starts_with("skip=")
                    || source[i..].starts_with("end="))
            {
                i += source[i..].find('=').unwrap() + 1;
                if let Ok((_, tail)) = super::delimited(&source[i..]) {
                    i = source.len() - tail.len();
                    continue;
                }
            }
            if match_pattern && words >= 2 && !c.is_ascii_alphanumeric() && c != b'_' {
                if let Ok((_, tail)) = super::delimited(&source[i..]) {
                    i = source.len() - tail.len();
                    match_pattern = false;
                    continue;
                }
            }
            if c == b'|'
                && !(expression
                    && (bytes.get(i + 1) == Some(&b'|') || (i > 0 && bytes[i - 1] == b'|')))
            {
                split = Some(i);
                break;
            }
            if c == b'"' && !expression {
                comment = Some(i);
                break;
            }
            if vim9 && c == b'#' {
                comment = Some(i);
                break;
            }
            if expression && (c == b'\'' || c == b'"') {
                let quote = c;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && quote == b'"' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[i] == quote {
                        if quote == b'\'' && bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == b'|' || expression && matches!(bytes[i], b'\'' | b'"') {
                    break;
                } else {
                    i += 1;
                }
            }
            if i == token_start {
                i += 1;
            }
            words += 1;
        }
        if let Some(i) = split {
            result.push(source[..i].trim().to_owned());
            source = source[i + 1..].trim();
        } else {
            result.push(source[..comment.unwrap_or(source.len())].trim().to_owned());
            break;
        }
    }
    result
}

impl Loader<'_> {
    pub(super) fn statements(&mut self, file: &str, lines: &[Statement]) {
        self.run_statements(file, lines);
    }
    pub(super) fn expanded_statements(&mut self, lines: &[Statement]) -> Result<(), String> {
        match self.run_statements("expanded syntax command", lines) {
            Flow::Finish | Flow::Break | Flow::Continue => {
                Err("nonlocal control flow in expanded syntax commands is unsupported".into())
            }
            _ => Ok(()),
        }
    }
    fn run_statements(&mut self, file: &str, lines: &[Statement]) -> Flow {
        if self.statement_depth >= 64 {
            self.err(file, 0, "setup statement depth budget exceeded");
            return Flow::Error;
        }
        self.statement_depth += 1;
        let result = self.run_statements_inner(file, lines);
        self.statement_depth -= 1;
        result
    }
    fn run_statements_inner(&mut self, file: &str, lines: &[Statement]) -> Flow {
        let errors_before = self.errors.len();
        let mut branches = Vec::<(bool, bool)>::new();
        let mut enabled = true;
        let mut i = 0;
        while i < lines.len() {
            if self.errors.len() != errors_before {
                return Flow::Error;
            }
            let statement = &lines[i];
            i += 1;
            if self.statement_fuel == 0 || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                self.err(
                    file,
                    statement.line,
                    "syntax setup cancelled or statement budget exceeded",
                );
                return Flow::Error;
            }
            self.statement_fuel -= 1;
            let (raw, rest) = match word(&statement.text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name = command_name(raw);
            if matches!(name, "function" | "for" | "while" | "try") {
                let end = match name {
                    "function" => "endfunction",
                    "for" => "endfor",
                    "try" => "endtry",
                    _ => "endwhile",
                };
                let body_start = i;
                let mut depth = 1;
                while i < lines.len() {
                    if let Err(error) = self.charge_statement_work() {
                        self.err(file, statement.line, error);
                        return Flow::Error;
                    }
                    let nested = lines[i]
                        .text
                        .split_whitespace()
                        .next()
                        .map(command_name)
                        .unwrap_or("");
                    if nested == name {
                        depth += 1;
                    }
                    if nested == end {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                if i == lines.len() {
                    self.err(file, statement.line, format!("unterminated setup {name}"));
                    return Flow::Error;
                }
                let body = &lines[body_start..i];
                i += 1;
                if !enabled {
                    continue;
                }
                if name == "try" {
                    let flow = self.try_statements(file, body);
                    if flow != Flow::Next {
                        return flow;
                    }
                    continue;
                }
                if name == "function" {
                    let body = body.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
                    if let Err(e) = self.setup.define_function(rest, &body) {
                        self.err(file, statement.line, e);
                    }
                    continue;
                }
                self.loop_depth += 1;
                if name == "for" {
                    let Some((binding, expression)) = rest.split_once(" in ") else {
                        self.err(file, statement.line, "invalid setup for loop");
                        self.loop_depth -= 1;
                        continue;
                    };
                    match self.setup.evaluate(expression) {
                        Ok(Value::List(values)) => {
                            for value in values {
                                if let Err(e) = self.bind_loop(binding.trim(), value) {
                                    self.err(file, statement.line, e);
                                    break;
                                }
                                match self.run_statements(file, body) {
                                    Flow::Break => break,
                                    flow @ (Flow::Finish | Flow::Error) => {
                                        self.loop_depth -= 1;
                                        return flow;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(_) => self.err(file, statement.line, "setup for requires a list"),
                        Err(e) => self.err(file, statement.line, e),
                    }
                } else {
                    loop {
                        if self.statement_fuel == 0 {
                            self.err(file, statement.line, "setup loop budget exceeded");
                            self.loop_depth -= 1;
                            return Flow::Error;
                        }
                        self.statement_fuel -= 1;
                        match self.condition(rest) {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(e) => {
                                self.err(file, statement.line, e);
                                break;
                            }
                        }
                        match self.run_statements(file, body) {
                            Flow::Break => break,
                            flow @ (Flow::Finish | Flow::Error) => {
                                self.loop_depth -= 1;
                                return flow;
                            }
                            _ => {}
                        }
                    }
                }
                self.loop_depth -= 1;
                continue;
            }
            match name {
                "if" => {
                    if branches.len() >= 64 {
                        self.err(
                            file,
                            statement.line,
                            "setup conditional depth budget exceeded",
                        );
                        return Flow::Error;
                    }
                    let selected = if enabled {
                        match self.condition(rest) {
                            Ok(v) => v,
                            Err(e) => {
                                self.err(file, statement.line, e);
                                false
                            }
                        }
                    } else {
                        false
                    };
                    branches.push((enabled, selected));
                    enabled &= selected;
                }
                "elseif" => {
                    if let Some((parent, selected)) = branches.last().copied() {
                        let value = if parent && !selected {
                            match self.condition(rest) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.err(file, statement.line, e);
                                    false
                                }
                            }
                        } else {
                            false
                        };
                        *branches.last_mut().unwrap() = (parent, selected || value);
                        enabled = parent && value;
                    } else {
                        self.err(file, statement.line, "unmatched elseif");
                    }
                }
                "else" => {
                    if let Some((parent, selected)) = branches.last_mut() {
                        enabled = *parent && !*selected;
                        *selected = true;
                    } else {
                        self.err(file, statement.line, "unmatched else");
                    }
                }
                "endif" => {
                    if let Some((parent, _)) = branches.pop() {
                        enabled = parent;
                    } else {
                        self.err(file, statement.line, "unmatched endif");
                    }
                }
                _ if !enabled => {}
                "finish" => return Flow::Finish,
                "break" | "continue" if self.loop_depth == 0 => {
                    self.err(file, statement.line, "setup loop control outside a loop")
                }
                "break" => return Flow::Break,
                "continue" => return Flow::Continue,
                _ => {
                    if let Err(e) = self.command(&statement.text) {
                        self.err(file, statement.line, e);
                    }
                }
            }
        }
        if !branches.is_empty() {
            self.err(
                file,
                lines.last().map_or(0, |s| s.line),
                "unterminated conditional",
            );
        }
        if self.errors.len() != errors_before {
            Flow::Error
        } else {
            Flow::Next
        }
    }
    fn bind_loop(&mut self, binding: &str, value: Value) -> Result<(), String> {
        if let Some(names) = binding.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let Value::List(values) = value else {
                return Err("setup loop destructuring requires a list".into());
            };
            let names = names.split(',').map(str::trim).collect::<Vec<_>>();
            if names.len() != values.len() {
                return Err("setup loop destructuring length mismatch".into());
            }
            for (name, value) in names.into_iter().zip(values) {
                self.setup.assign_value(name, value)?;
            }
            Ok(())
        } else {
            self.setup.assign_value(binding, value)
        }
    }
}

impl Loader<'_> {
    pub(super) fn set_keyword(&mut self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        if value == "clear" {
            self.keyword = VimKeyword::parse(&self.keyword_option)?;
            self.syntax_keyword_option = None;
        } else {
            self.keyword = VimKeyword::parse(value)?;
            self.syntax_keyword_option = Some(value.to_owned());
        }
        Ok(())
    }
    pub(super) fn set_buffer_keyword(&mut self, value: &str) -> Result<(), String> {
        let keyword = VimKeyword::parse(value)?;
        if self.syntax_keyword_option.is_none() {
            self.keyword = keyword;
        }
        self.keyword_option = value.to_owned();
        self.setup.set_keyword_option(value);
        Ok(())
    }
    pub(super) fn set_options(&mut self, source: &str) -> Result<(), String> {
        for raw in source
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
        {
            let mut chars = raw.chars();
            let mut option = String::new();
            while let Some(c) = chars.next() {
                option.push(if c == '\\' {
                    chars.next().ok_or("trailing setup option escape")?
                } else {
                    c
                });
            }
            let option = option.as_str();
            if matches!(option, "cpo&vim" | "cpoptions&vim" | "cpo&" | "cpoptions&") {
                continue;
            }
            if ["cpo+=", "cpo-=", "cpoptions+=", "cpoptions-="]
                .iter()
                .any(|s| option.starts_with(s))
            {
                continue;
            }
            let key = option
                .strip_prefix("iskeyword")
                .or_else(|| option.strip_prefix("isk"));
            if let Some(value) = key {
                if let Some(value) = value.strip_prefix("+=") {
                    self.set_buffer_keyword(&format!("{},{value}", self.keyword_option))?;
                } else if let Some(value) = value.strip_prefix("-=") {
                    let items = self
                        .keyword_option
                        .split(',')
                        .filter(|s| !value.split(',').any(|v| v == *s))
                        .collect::<Vec<_>>()
                        .join(",");
                    self.set_buffer_keyword(&items)?;
                } else if let Some(value) = value.strip_prefix('=') {
                    self.set_buffer_keyword(value)?;
                } else if matches!(value, "&" | "&vim" | "<") {
                    self.set_buffer_keyword("@,48-57,_,192-255")?;
                } else {
                    return Err(format!("unsupported syntax keyword option: {option}"));
                }
                continue;
            }
            // These are editor presentation hints. Code keeps literal text and
            // owns folding/spelling independently of syntax-package metadata.
            if matches!(
                option,
                "foldmethod=syntax"
                    | "fdm=syntax"
                    | "foldenable"
                    | "nofoldenable"
                    | "spell"
                    | "nospell"
            ) {
                continue;
            }
            return Err(format!("unsupported syntax setup option: {option}"));
        }
        Ok(())
    }
    fn charge_statement_work(&mut self) -> Result<(), String> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err("syntax setup cancelled".into());
        }
        self.statement_fuel = self
            .statement_fuel
            .checked_sub(1)
            .ok_or("syntax setup traversal budget exceeded")?;
        Ok(())
    }
    pub(super) fn runtime(&mut self, source: &str, all: bool) -> Result<(), String> {
        let root = self
            .root
            .clone()
            .ok_or("includes require a syntax directory")?;
        for pattern in source
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
        {
            self.charge_statement_work()?;
            let relative = pattern.strip_prefix("syntax/").unwrap_or(pattern);
            if Path::new(relative).is_absolute()
                || Path::new(relative)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("include escapes configured syntax directory".into());
            }
            if relative.contains(['[', ']', '{', '}']) {
                return Err(
                    "syntax runtime bracket and brace glob patterns are unsupported".into(),
                );
            }
            let mut candidates = vec![root.clone()];
            for component in relative.split('/') {
                self.charge_statement_work()?;
                let mut next = Vec::new();
                if component.contains(['*', '?']) {
                    let mut expression = String::from("^");
                    for c in component.chars() {
                        match c {
                            '*' => expression.push_str(".*"),
                            '?' => expression.push('.'),
                            _ => expression.push_str(&::regex::escape(&c.to_string())),
                        }
                    }
                    expression.push('$');
                    let matcher = ::regex::Regex::new(&expression).map_err(|e| e.to_string())?;
                    for directory in candidates {
                        self.charge_statement_work()?;
                        let Some(directory) = confined_candidate(&root, &directory)? else {
                            continue;
                        };
                        if !directory.is_dir() {
                            continue;
                        }
                        let entries =
                            std::fs::read_dir(directory).map_err(|error| error.to_string())?;
                        for entry in entries {
                            self.charge_statement_work()?;
                            let entry = entry.map_err(|e| e.to_string())?;
                            if matcher.is_match(&entry.file_name().to_string_lossy()) {
                                if let Some(path) = confined_candidate(&root, &entry.path())? {
                                    next.push(path);
                                }
                            }
                        }
                    }
                } else {
                    for candidate in candidates {
                        self.charge_statement_work()?;
                        if let Some(path) = confined_candidate(&root, &candidate.join(component))? {
                            next.push(path);
                        }
                    }
                }
                candidates = next;
            }
            candidates.sort();
            for path in candidates {
                self.charge_statement_work()?;
                if path.is_file() {
                    self.file(
                        &path
                            .strip_prefix(&root)
                            .map_err(|_| "include escapes configured syntax directory")?
                            .to_string_lossy(),
                    );
                    if !all {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}

impl Loader<'_> {
    pub(super) fn highlight(&mut self, rest: &str, forced: bool) -> Result<(), String> {
        let mut words = rest
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
            .collect::<Vec<_>>();
        let default = words
            .first()
            .is_some_and(|s| matches!(*s, "def" | "default"));
        if default {
            words.remove(0);
        }
        let Some(&first) = words.first() else {
            return Ok(());
        };
        if first == "clear" {
            if words.len() == 1 {
                self.highlight_definitions.clear();
                self.program.links.clear();
            } else {
                for name in &words[1..] {
                    let name = self.intern_group_name(name);
                    self.highlight_definitions.remove(&name);
                    self.program.links.remove(&name);
                }
            }
            return Ok(());
        }
        if first == "link" {
            if words.len() != 3 {
                return Err("invalid highlight link declaration".into());
            }
            validate_groups(&[words[1].to_owned(), words[2].to_owned()])?;
            let group = self.intern_group_name(words[1]);
            let target = if words[2].eq_ignore_ascii_case("NONE") {
                "NONE".to_owned()
            } else {
                self.intern_group_name(words[2])
            };
            if default
                && (self.program.links.contains_key(&group)
                    || self.highlight_definitions.contains(&group))
            {
                return Ok(());
            }
            if !forced && self.highlight_definitions.contains(&group) {
                return Ok(());
            }
            if target.eq_ignore_ascii_case("NONE") || group == target {
                self.program.links.remove(&group);
            } else {
                self.program.links.insert(group, target);
            }
            return Ok(());
        }
        validate_groups(&[first.to_owned()])?;
        if words.len() == 1 {
            return Ok(());
        } // read-only query does not create a group
        let first = self.intern_group_name(first);
        let first = first.as_str();
        for attribute in &words[1..] {
            if attribute.eq_ignore_ascii_case("NONE") {
                continue;
            }
            let (key, value) = attribute
                .split_once('=')
                .ok_or("invalid highlight attribute")?;
            if !matches!(
                key.to_ascii_lowercase().as_str(),
                "term"
                    | "cterm"
                    | "gui"
                    | "ctermfg"
                    | "ctermbg"
                    | "ctermul"
                    | "guifg"
                    | "guibg"
                    | "guisp"
                    | "font"
                    | "start"
                    | "stop"
                    | "blend"
            ) || value.is_empty()
            {
                return Err(format!("unsupported highlight attribute: {attribute}"));
            }
        }
        if default
            && (self.program.links.contains_key(first)
                || self.highlight_definitions.contains(first))
        {
            return Ok(());
        }
        // Syntax-owned presentation declarations establish the named group and
        // link precedence. Its visual properties come from the Code stylesheet.
        self.program.links.remove(first);
        self.highlight_definitions.insert(first.to_owned());
        Ok(())
    }
    pub(super) fn synchronization(&mut self, rest: &str) -> Result<(), String> {
        let (kind, arguments) = word(rest)?;
        if kind == "clear" {
            return Ok(());
        }
        if matches!(kind, "match" | "region") {
            let (name, mut tail) = word(arguments)?;
            let mut declaration = format!("{kind} {name}");
            while !tail.is_empty() && !tail.starts_with('"') {
                let (token, next) = word(tail)?;
                if matches!(token, "grouphere" | "groupthere") {
                    let (group, next) = word(next)?;
                    validate_groups(&[group.to_owned()])?;
                    if group != "NONE"
                        && !self.program.rules.iter().any(|r| {
                            r.group.eq_ignore_ascii_case(group)
                                && matches!(r.kind, RuleKind::Region { .. })
                        })
                    {
                        return Err(format!(
                            "syntax sync target is not a defined region: {group}"
                        ));
                    }
                    tail = next;
                } else {
                    // The rest may begin with a quoted pattern. Copy delimited
                    // patterns intact; synchronization directives outside them
                    // are metadata and never become ordinary content rules.
                    let prefix = ["start=", "skip=", "end="]
                        .into_iter()
                        .find(|p| tail.starts_with(p))
                        .unwrap_or("");
                    let pattern = &tail[prefix.len()..];
                    if !prefix.is_empty()
                        || pattern
                            .chars()
                            .next()
                            .is_some_and(|c| !c.is_alphanumeric() && c != '_')
                    {
                        let (_, after) = delimited(pattern)?;
                        let length = tail.len() - after.len();
                        declaration.push(' ');
                        declaration.push_str(&tail[..length]);
                        tail = after.trim_start();
                    } else {
                        declaration.push(' ');
                        declaration.push_str(token);
                        tail = next;
                    }
                }
            }
            // A double quote at the first pattern position is a delimiter.
            if !tail.is_empty() {
                declaration.push(' ');
                declaration.push_str(tail);
            }
            let before = self.program.rules.len();
            let result = self.syntax(&declaration);
            self.program.rules.truncate(before);
            return result;
        }
        if kind == "linecont" {
            let (pattern, rest) = delimited(arguments)?;
            VimPattern::compile_with_keyword(
                &pattern,
                self.case_ignore,
                VimRegexLimits::default(),
                &self.keyword,
            )?;
            if !rest.trim().is_empty() && !rest.trim_start().starts_with('"') {
                return self.synchronization(rest.trim());
            }
            return Ok(());
        }
        let mut tokens = rest
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
            .peekable();
        while let Some(token) = tokens.next() {
            if token == "fromstart" {
                self.program.fromstart = true;
                continue;
            }
            if token == "ccomment" {
                if tokens
                    .peek()
                    .is_some_and(|s| !s.contains('=') && *s != "fromstart")
                {
                    validate_groups(&[tokens.next().unwrap().to_owned()])?;
                }
                continue;
            }
            if let Some(v) = token
                .strip_prefix("minlines=")
                .or_else(|| token.strip_prefix("lines="))
            {
                self.program.minlines = v.parse().map_err(|_| "invalid minlines")?;
                continue;
            }
            if let Some(v) = token.strip_prefix("maxlines=") {
                self.program.maxlines = v.parse().map_err(|_| "invalid maxlines")?;
                continue;
            }
            if let Some(v) = token.strip_prefix("linebreaks=") {
                let _: usize = v.parse().map_err(|_| "invalid linebreaks")?;
                continue;
            }
            return Err(format!("unsupported synchronization declaration: {token}"));
        }
        // Recovery hints cannot establish Exact coverage. The scanner continues
        // to recover from validated checkpoints or clearly provisional context.
        Ok(())
    }
}

impl Loader<'_> {
    fn try_statements(&mut self, file: &str, body: &[Statement]) -> Flow {
        let mut branches = Vec::new();
        let mut depth = 0;
        let mut saw_finally = false;
        for (i, statement) in body.iter().enumerate() {
            if let Err(error) = self.charge_statement_work() {
                self.err(file, statement.line, error);
                return Flow::Error;
            }
            let name = statement.text.split_whitespace().next().unwrap_or("");
            if name == "try" {
                depth += 1;
            }
            if name == "endtry" {
                depth -= 1;
            }
            if depth == 0 && matches!(name, "catch" | "finally") {
                if saw_finally {
                    self.err(file, statement.line, "catch or finally after finally");
                    return Flow::Error;
                }
                saw_finally = name == "finally";
                branches.push(i);
            }
        }
        let before = self.errors.len();
        let mut flow = self.run_statements(
            file,
            &body[..branches.first().copied().unwrap_or(body.len())],
        );
        let thrown = self.errors.get(before).and_then(|e| super::setup::catchable_error(&e.message));
        let mut caught = false;
        let mut exception_before = None;
        for (n, &at) in branches.iter().enumerate() {
            let statement = &body[at];
            let (kind, rest) = word(&statement.text).unwrap();
            let end = branches.get(n + 1).copied().unwrap_or(body.len());
            if kind == "finally" {
                let final_flow = self.run_statements(file, &body[at + 1..end]);
                if final_flow != Flow::Next {
                    flow = final_flow;
                }
            } else if !caught {
                if let Some(error) = &thrown {
                    let matches = if rest.is_empty() {
                        Ok(true)
                    } else {
                        delimited(rest)
                            .and_then(|(pattern, tail)| {
                                if !tail.trim().is_empty() && !tail.trim_start().starts_with('"') {
                                    return Err("invalid setup catch pattern suffix".into());
                                }
                                VimPattern::compile(&pattern, false, VimRegexLimits::default())
                            })
                            .and_then(|pattern| {
                                pattern.is_match_text_with_control(
                                    error,
                                    &mut self.group_pattern_fuel,
                                    &mut || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)),
                                )
                            })
                    };
                    match matches {
                        Ok(true) if self.errors.len() == before + 1 => {
                            self.errors.truncate(before);
                            match self.setup.set_exception(Some(error.clone())) {
                                Ok(previous) => {
                                    exception_before = Some(previous);
                                    flow = self.run_statements(file, &body[at + 1..end]);
                                }
                                Err(error) => {
                                    self.err(file, statement.line, error);
                                    flow = Flow::Error;
                                }
                            }
                            caught = true;
                        }
                        Ok(_) => {}
                        Err(e) => self.err(file, statement.line, e),
                    }
                }
            }
        }
        if let Some(previous) = exception_before {
            if let Err(error) = self.setup.set_exception(previous) {
                self.err(file, 0, error);
                flow = Flow::Error;
            }
        }
        flow
    }
}

pub(super) fn decode_source(bytes: &[u8]) -> Result<String, String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    // Legacy runtime attribution comments can contain opaque non-UTF8 bytes.
    // They are not executable declarations; only their physical line and input
    // hash matter. Never substitute invalid bytes in an active command.
    let mut source = String::new();
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        match std::str::from_utf8(line) {
            Ok(text) => source.push_str(text),
            Err(_) => {
                if line.iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b'"') {
                    return Err("invalid UTF-8 in syntax declaration".into());
                }
                source.push('"');
                if line.ends_with(b"\n") {
                    source.push('\n');
                }
            }
        }
    }
    Ok(source)
}

fn confined_candidate(root: &Path, path: &Path) -> Result<Option<PathBuf>, String> {
    match path.canonicalize() {
        Ok(path) if path.starts_with(root) => Ok(Some(path)),
        Ok(_) => Err("include escapes configured syntax directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}
