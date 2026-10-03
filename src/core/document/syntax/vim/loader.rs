use super::regex::{VimKeyword, VimPattern, VimRegexLimits};
use super::{
    vim_literal, Offset, OffsetBase, PatternOffsets, PatternTemplate, Rule, RuleKind, RuleOptions,
    VimDiagnostic, VimLoadLimits, VimProgram, VimSetupContext, NATIVE_PROFILE_VERSION,
};

mod commands;
mod setup;
#[cfg(test)]
mod tests;
use setup::Setup;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct Loader<'a> {
    program: VimProgram,
    limits: VimLoadLimits,
    errors: Vec<VimDiagnostic>,
    root: Option<PathBuf>,
    active: BTreeSet<(PathBuf, u64)>,
    bytes: usize,
    files: usize,
    compiled_bytes: usize,
    case_ignore: bool,
    cancel: Option<&'a AtomicBool>,
    setup: Setup<'a>,
    command_depth: usize,
    expanded_bytes: usize,
    group_pattern_fuel: usize,
    statement_fuel: usize,
    statement_depth: usize,
    highlight_definitions: BTreeSet<String>,
    keyword: VimKeyword,
    keyword_option: String,
    syntax_keyword_option: Option<String>,
    current_source: Option<PathBuf>,
    loop_depth: usize,
    include_stack: Vec<Option<String>>,
}
pub(super) fn compile(
    name: &str,
    text: &str,
    limits: VimLoadLimits,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    let mut l = Loader::new(limits, None);
    l.source(name, text);
    l.finish()
}
pub(super) fn compile_cancellable(
    name: &str,
    text: &str,
    limits: VimLoadLimits,
    cancel: Arc<AtomicBool>,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    let mut l = Loader::new(limits, None);
    l.cancel = Some(cancel.as_ref());
    l.setup = Setup::new(&VimSetupContext::default(), Some(cancel.as_ref()));
    l.source(name, text);
    l.finish()
}
pub(super) fn directory(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    directory_with_context(root, language, limits, &VimSetupContext::default(), None)
}
pub(super) fn directory_cancellable(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
    cancel: Arc<AtomicBool>,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    directory_with_context(
        root,
        language,
        limits,
        &VimSetupContext::default(),
        Some(cancel.as_ref()),
    )
}
pub(super) fn directory_with_context(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
    context: &VimSetupContext,
    cancel: Option<&AtomicBool>,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    directory_with_reads(root, language, limits, context, cancel, &mut super::VimSetupReads::default())
}
pub(super) fn directory_with_reads(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
    context: &VimSetupContext,
    cancel: Option<&AtomicBool>,
    reads: &mut super::VimSetupReads,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    if language.is_empty()
        || !language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(vec![VimDiagnostic::new(
            language,
            0,
            "invalid language package name",
        )]);
    }
    let canonical = root.canonicalize().map_err(|e| {
        vec![VimDiagnostic::new(
            &root.display().to_string(),
            0,
            &e.to_string(),
        )]
    })?;
    let mut l = Loader::new(limits, Some(canonical));
    l.cancel = cancel;
    if context.prefix.len() > 64 * 1024 || context.prefix.lines().count() > 32 {
        return Err(vec![VimDiagnostic::new(
            language,
            0,
            "syntax setup prefix budget exceeded",
        )]);
    }
    if context
        .filename
        .as_ref()
        .is_some_and(|name| name.len() > 16 * 1024 || name.contains('\0'))
    {
        return Err(vec![VimDiagnostic::new(
            language,
            0,
            "invalid or oversized syntax setup filename",
        )]);
    }
    l.setup = Setup::new(context, l.cancel);
    l.setup.set_filetype(language);
    for b in b"\0filename\0"
        .iter()
        .copied()
        .chain([u8::from(context.filename.is_some())])
        .chain(context.filename.as_deref().unwrap_or("").bytes())
        .chain([0])
    {
        l.program.generation = (l.program.generation ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    for b in context.prefix.bytes() {
        l.program.generation = (l.program.generation ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    l.file(&format!("{language}.vim"));
    *reads = l.setup.reads.clone();
    l.finish()
}
impl<'a> Loader<'a> {
    fn new(limits: VimLoadLimits, root: Option<PathBuf>) -> Self {
        Self {
            program: VimProgram {
                retained_bytes: 0,
                rules: Vec::new(),
                links: BTreeMap::new(),
                clusters: BTreeMap::new(),
                group_names: BTreeMap::new(),
                minlines: 0,
                maxlines: 200,
                fromstart: false,
                multiline: false,
                generation: NATIVE_PROFILE_VERSION as u64,
                source_files: Vec::new(),
            },
            limits,
            errors: Vec::new(),
            root,
            active: BTreeSet::new(),
            bytes: 0,
            files: 0,
            compiled_bytes: 0,
            case_ignore: false,
            cancel: None,
            setup: Setup::new(&VimSetupContext::default(), None),
            command_depth: 0,
            expanded_bytes: 0,
            group_pattern_fuel: 1_000_000,
            statement_fuel: 100_000,
            statement_depth: 0,
            highlight_definitions: BTreeSet::new(),
            keyword: VimKeyword::default(),
            keyword_option: "@,48-57,_,192-255".into(),
            syntax_keyword_option: None,
            current_source: None,
            loop_depth: 0,
            include_stack: Vec::new(),
        }
    }
    fn err(&mut self, file: &str, line: usize, message: impl AsRef<str>) {
        if self.errors.len() < 256 {
            self.errors
                .push(VimDiagnostic::new(file, line, message.as_ref()));
        }
    }
    fn finish(mut self) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
        if self.errors.is_empty() {
            if let Err(error) = self.finalize_keyword_environment() {
                self.err("syntax keyword environment", 0, error);
            }
        }
        for key in self.program.links.keys() {
            let mut seen = BTreeSet::new();
            let mut next = key;
            while let Some(target) = self.program.links.get(next) {
                if !seen.insert(next) {
                    self.errors.push(VimDiagnostic::new(
                        "highlight links",
                        0,
                        &format!("cyclic highlight link at {key}"),
                    ));
                    break;
                }
                next = target;
            }
        }
        let mut checked = BTreeMap::new();
        for name in self.program.clusters.keys() {
            if cluster_cycle(
                name,
                &self.program.clusters,
                &mut BTreeSet::new(),
                &mut checked,
                0,
            ) {
                self.errors.push(VimDiagnostic::new(
                    "syntax clusters",
                    0,
                    &format!("cyclic or overdeep cluster at {name}"),
                ));
            }
        }
        if self.errors.is_empty() {
            // Queried source values participate in the same identity as the
            // prefix and runtime files. Unread body bytes do not.
            let reads = format!("{:?}", self.setup.reads);
            for byte in reads.bytes() {
                self.program.generation = (self.program.generation ^ u64::from(byte))
                    .wrapping_mul(1099511628211);
            }
            let strings = |values: &[String]| {
                values
                    .iter()
                    .map(|value| value.capacity() + 16)
                    .sum::<usize>()
                    + values.len() * std::mem::size_of::<String>()
            };
            self.program.retained_bytes = self.compiled_bytes
                + self.program.rules.capacity() * std::mem::size_of::<Rule>()
                + self
                    .program
                    .rules
                    .iter()
                    .map(|rule| {
                        rule.group.capacity()
                            + 16
                            + strings(&rule.options.contains)
                            + strings(&rule.options.containedin)
                            + strings(&rule.options.nextgroup)
                            + rule
                                .options
                                .matchgroup
                                .as_ref()
                                .map_or(0, |name| name.capacity() + 16)
                    })
                    .sum::<usize>()
                + self
                    .program
                    .links
                    .iter()
                    .map(|(name, target)| name.capacity() + target.capacity() + 96)
                    .sum::<usize>()
                + self
                    .program
                    .clusters
                    .iter()
                    .map(|(name, groups)| name.capacity() + strings(groups) + 96)
                    .sum::<usize>()
                + self
                    .program
                    .group_names
                    .iter()
                    .map(|(key, name)| key.capacity() + name.capacity() + 96)
                    .sum::<usize>()
                + strings(&self.program.source_files);
            if self.program.retained_bytes > self.limits.program_bytes {
                return Err(vec![VimDiagnostic::new(
                    "syntax program",
                    0,
                    "retained syntax program byte budget exceeded",
                )]);
            }
            Ok(Arc::new(self.program))
        } else {
            Err(self.errors)
        }
    }
    fn file(&mut self, name: &str) {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            self.err(name, 0, "syntax compilation cancelled");
            return;
        }
        let Some(root) = self.root.clone() else {
            self.err(name, 0, "includes require a syntax directory");
            return;
        };
        let relative;
        let name = if let Some(tail) = name
            .strip_prefix("<sfile>:p:h/")
            .or_else(|| name.strip_prefix("<sfile>:h/"))
        {
            relative = self
                .current_source
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or(&root)
                .join(tail)
                .to_string_lossy()
                .into_owned();
            relative.as_str()
        } else {
            name
        };
        let name = name
            .strip_prefix("$VIMRUNTIME/syntax/")
            .or_else(|| name.strip_prefix("syntax/"))
            .unwrap_or(name);
        let path = match root.join(name).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.err(name, 0, e.to_string());
                return;
            }
        };
        if !path.starts_with(&root) {
            self.err(name, 0, "include escapes configured syntax directory");
            return;
        }
        let environment = match self.setup.environment_fingerprint() {
            Ok(value) => value,
            Err(error) => {
                self.err(name, 0, error);
                return;
            }
        };
        let active_key = (path.clone(), environment);
        if self.active.contains(&active_key) {
            self.err(name, 0, "cyclic syntax include");
            return;
        }
        if self.files >= self.limits.files {
            self.err(name, 0, "syntax file budget exceeded");
            return;
        }
        let metadata = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                self.err(name, 0, e.to_string());
                return;
            }
        };
        if metadata.len() > self.limits.source_bytes.saturating_sub(self.bytes) as u64 {
            self.err(name, 0, "syntax source byte budget exceeded");
            return;
        }
        let remaining = self.limits.source_bytes.saturating_sub(self.bytes);
        let mut bytes = Vec::new();
        let read = std::fs::File::open(&path).and_then(|f| {
            f.take(remaining.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
        });
        if let Err(e) = read {
            self.err(name, 0, e.to_string());
            return;
        }
        if bytes.len() > remaining {
            self.err(name, 0, "syntax source byte budget exceeded");
            return;
        }
        let text = match commands::decode_source(&bytes) {
            Ok(text) => text,
            Err(error) => {
                self.err(name, 0, error);
                return;
            }
        };
        self.bytes = self
            .bytes
            .saturating_add(bytes.len().saturating_sub(text.len()));
        for b in bytes {
            self.program.generation =
                (self.program.generation ^ u64::from(b)).wrapping_mul(1099511628211);
        }
        self.active.insert(active_key.clone());
        self.source(&path.display().to_string(), &text);
        self.active.remove(&active_key);
    }
    fn source(&mut self, file: &str, text: &str) {
        let scope = self.setup.begin_script();
        let script_file = self.setup.set_source_file(file);
        let caller = self.current_source.replace(PathBuf::from(file));
        self.source_body(file, text);
        self.current_source = caller;
        self.setup.set_source_file(&script_file);
        self.setup.end_script(scope);
    }
    fn source_body(&mut self, file: &str, text: &str) {
        self.bytes = self.bytes.saturating_add(text.len());
        self.files += 1;
        if self.bytes > self.limits.source_bytes || self.files > self.limits.files {
            self.err(file, 0, "syntax load budget exceeded");
            return;
        }
        self.program.source_files.push(file.into());
        for (i, b) in file.bytes().chain(text.bytes()).enumerate() {
            if i & 4095 == 0
                && self
                    .cancel
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                self.err(file, 0, "syntax compilation cancelled");
                return;
            }
            self.program.generation =
                (self.program.generation ^ u64::from(b)).wrapping_mul(1099511628211);
        }
        match commands::logical_lines(text) {
            Ok(lines) => {
                self.statements(file, &lines);
            }
            Err((line, error)) => self.err(file, line, error),
        }
    }
    fn condition(&mut self, s: &str) -> Result<bool, String> {
        self.setup.evaluate(s)?.truth()
    }
    fn command(&mut self, text: &str) -> Result<(), String> {
        if self.command_depth >= 32 {
            return Err("syntax command expansion depth budget exceeded".into());
        }
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err("syntax compilation cancelled".into());
        }
        self.command_depth += 1;
        let result = self.command_inner(text);
        self.command_depth -= 1;
        result
    }
    fn command_inner(&mut self, text: &str) -> Result<(), String> {
        let (cmd, rest) = word(text)?;
        if let Some((expanded, scope)) = self.setup.expand_macro(cmd, rest)? {
            let caller = self.setup.replace_script(scope);
            let result = self.expanded_command(&expanded);
            self.setup.end_script(caller);
            return result;
        }
        let forced = cmd.ends_with('!');
        match commands::command_name(cmd) {
            "syn" | "sy" | "syntax" => self.syntax(rest),
            "highlight" => self.highlight(rest, forced),
            "let" => {
                if let Some((name, expression)) = rest.split_once('=') {
                    if matches!(
                        name.trim(),
                        "&isk" | "&iskeyword" | "&l:isk" | "&l:iskeyword"
                    ) {
                        let value = self.setup.evaluate(expression)?.text()?;
                        return self.set_buffer_keyword(&value);
                    }
                }
                self.setup.assign(rest)
            }
            "set" | "setlocal" => self.set_options(rest),
            "unlet" | "unlet!" => self.setup.unlet(rest, forced),
            "com" | "com!" | "command" | "command!" => self.setup.define_macro(rest),
            "delc" | "delcommand" => self.setup.delete_macro(rest),
            "exe" | "execute" => {
                let expanded = self.setup.evaluate_execute(rest)?;
                self.expanded_command(&expanded)
            }
            "runtime" | "runtime!" => {
                self.runtime(rest, forced)?;
                Ok(())
            }
            "scriptencoding" if matches!(rest, "utf-8" | "utf8" | "") => Ok(()),
            "call" => {
                for command in self.setup.call_statement(rest)? {
                    self.expanded_command(&command)?;
                }
                Ok(())
            }
            "delfunction" => self.setup.delete_function(rest, forced),
            "source" => {
                self.file(rest);
                Ok(())
            }
            _ => Err(format!("unsupported native setup command: {cmd}")),
        }
    }
    fn expanded_command(&mut self, text: &str) -> Result<(), String> {
        self.expanded_bytes = self.expanded_bytes.saturating_add(text.len());
        if self.expanded_bytes > self.limits.source_bytes {
            return Err("syntax command expansion byte budget exceeded".into());
        }
        if text.contains(['\n', '\r']) {
            return Err("multiline syntax command expansion is unsupported".into());
        }
        let lines = commands::logical_lines(text).map_err(|(_, message)| message)?;
        let before = self.errors.len();
        self.expanded_statements(&lines)?;
        if self.errors.len() != before {
            let errors = self.errors.split_off(before);
            return Err(errors[0].message.clone());
        }
        Ok(())
    }
    fn syntax(&mut self, rest: &str) -> Result<(), String> {
        let (kind, rest) = word(rest)?;
        if kind == "iskeyword" {
            return self.set_keyword(rest.trim());
        }
        if kind == "clear" {
            if !self.include_stack.is_empty() {
                return Ok(());
            }
            if rest.trim().is_empty() {
                self.program.rules.clear();
                self.program.clusters.clear();
                self.program.multiline = false;
                self.setup.unlet("b:current_syntax", true)?;
            } else {
                for group in rest.split_whitespace() {
                    if let Some(cluster) = group.strip_prefix('@') {
                        let cluster = self.program.intern_cluster_name(cluster);
                        self.program.clusters.remove(&cluster);
                    } else {
                        self.program
                            .rules
                            .retain(|r| !r.group.eq_ignore_ascii_case(group));
                    }
                }
            }
            return Ok(());
        }
        if kind == "case" {
            self.case_ignore = match rest.trim() {
                "ignore" => true,
                "match" => false,
                _ => return Err("invalid syntax case".into()),
            };
            return Ok(());
        }
        if kind == "spell" && matches!(rest, "toplevel" | "notoplevel" | "default") {
            return Ok(());
        }
        if kind == "conceal" && matches!(rest, "on" | "off") {
            return Ok(());
        }
        if kind == "sync" {
            return self.synchronization(rest);
        }
        if kind == "include" {
            let (first, tail) = word(rest)?;
            let (cluster, source) = if let Some(name) = first.strip_prefix('@') {
                validate_groups(&[name.to_owned()])?;
                (Some(self.program.intern_cluster_name(name)), tail)
            } else {
                (None, rest)
            };
            if source.is_empty() {
                return Err("syntax include requires a source file".into());
            }
            self.include_stack.push(cluster);
            let result = if let Some(patterns) = source.strip_prefix("runtime! ") {
                self.runtime(patterns, true)
            } else if let Some(patterns) = source.strip_prefix("runtime ") {
                self.runtime(patterns, false)
            } else {
                self.file(source);
                Ok(())
            };
            self.include_stack.pop();
            return result;
        }

        let (group, rest) = word(rest)?;
        if group.len() > 128
            || !group
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && kind != "include"
        {
            return Err("unsupported syntax group name".into());
        }
        let group = if kind == "cluster" {
            self.program.intern_cluster_name(group)
        } else {
            self.intern_group_name(group)
        };
        let group = group.as_str();
        if kind == "cluster" {
            self.program.clusters.entry(group.into()).or_default();
            let mut tail = rest;
            while !tail.is_empty() {
                let (token, next) = option_word(tail)?;
                let token = token.as_str();
                tail = next;
                if let Some(list) = token.strip_prefix("contains=") {
                    let groups = self.expand_groups(
                        list.split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )?;
                    self.program.clusters.insert(group.into(), groups);
                } else if let Some(list) = token.strip_prefix("add=") {
                    let groups = self.expand_groups(
                        list.split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )?;
                    self.program
                        .clusters
                        .entry(group.into())
                        .or_default()
                        .extend(groups);
                } else if let Some(list) = token.strip_prefix("remove=") {
                    let groups = self.expand_groups(
                        list.split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )?;
                    if let Some(items) = self.program.clusters.get_mut(group) {
                        items.retain(|s| !groups.contains(s));
                    }
                } else {
                    return Err(format!("unsupported cluster option: {token}"));
                }
            }
            return Ok(());
        }
        let mut options = RuleOptions::default();
        let mut patterns = Vec::new();
        let mut keywords = Vec::new();
        let mut remaining = rest;
        if kind == "match" {
            while remaining
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric())
            {
                let (token, tail) = option_word(remaining)?;
                if !parse_option(&token, &mut options)? {
                    return Err(format!("unsupported leading syntax option: {token}"));
                }
                remaining = tail;
            }
            let (pattern, tail) = delimited(remaining)?;
            let (offsets, tail) = attached_offsets(tail)?;
            if offsets.rs.is_some() || offsets.re.is_some() {
                return Err("region offsets cannot be attached to a match".into());
            }
            options.ms = offsets.ms;
            options.me = offsets.me;
            options.hs = offsets.hs;
            options.he = offsets.he;
            options.lc = offsets.lc;
            patterns.push(("match", pattern, PatternOffsets::default(), None, false));
            remaining = tail;
        }
        while !remaining.trim().is_empty() {
            remaining = remaining.trim_start();
            if remaining.starts_with('"') {
                break;
            }
            if kind == "region" {
                let mut found = false;
                for key in ["start", "skip", "end"] {
                    if let Some(tail) = remaining.strip_prefix(&format!("{key}=")) {
                        let (pattern, tail) = delimited(tail)?;
                        let (offsets, tail) = attached_offsets(tail)?;
                        patterns.push((
                            key,
                            pattern,
                            offsets,
                            options
                                .matchgroup
                                .as_deref()
                                .filter(|s| *s != "NONE")
                                .map(|name| Arc::<str>::from(self.intern_group_name(name))),
                            options.excludenl,
                        ));
                        remaining = tail;
                        found = true;
                        break;
                    }
                }
                if found {
                    continue;
                }
            }
            let (token, tail) = option_word(remaining)?;
            remaining = tail;
            if !parse_option(&token, &mut options)? {
                if kind == "keyword" {
                    keywords.extend(expand_keyword(&token)?);
                } else {
                    return Err(format!("unsupported syntax option: {token}"));
                }
            }
        }
        let regexlimits = VimRegexLimits {
            nfa_bytes: self.limits.pattern_nfa_bytes,
            ..Default::default()
        };
        for list in [
            &mut options.contains,
            &mut options.containedin,
            &mut options.nextgroup,
        ] {
            *list = self.expand_groups(std::mem::take(list))?;
        }
        if let Some(group) = &mut options.matchgroup {
            *group = self.intern_group_name(group);
        }
        let rule_kind = match kind {
            "keyword" => {
                if keywords.is_empty() {
                    return Ok(());
                }
                // Long runtime keyword inventories are several equivalent
                // rules with the same group/options. Keep each NFA bounded
                // without rejecting an otherwise ordinary keyword list.
                let mut alternatives = String::new();
                for word in keywords {
                    let literal = vim_literal(&word);
                    if !alternatives.is_empty() && alternatives.len() + literal.len() + 16 > 4096 {
                        let pattern = VimPattern::compile_with_keyword(
                            &format!("\\<\\%({alternatives}\\)\\>"),
                            self.case_ignore,
                            regexlimits,
                            &self.keyword,
                        )?;
                        self.add_rule(group, RuleKind::Keywords(pattern), options.clone())?;
                        alternatives.clear();
                    }
                    if !alternatives.is_empty() {
                        alternatives.push_str("\\|");
                    }
                    alternatives.push_str(&literal);
                }
                RuleKind::Keywords(VimPattern::compile_with_keyword(
                    &format!("\\<\\%({alternatives}\\)\\>"),
                    self.case_ignore,
                    regexlimits,
                    &self.keyword,
                )?)
            }
            "match" => {
                let p = VimPattern::compile_with_keyword(
                    &patterns[0].1,
                    self.case_ignore,
                    regexlimits,
                    &self.keyword,
                )?;
                validate_pattern_offsets(
                    "match",
                    PatternOffsets {
                        ms: options.ms,
                        ..Default::default()
                    },
                    p.minimum_chars,
                )?;
                RuleKind::Match(p)
            }
            "region" => {
                let mut starts = Vec::new();
                let mut ends = Vec::new();
                let mut skip = None;
                for (kind, p, offsets, matchgroup, excludenl) in patterns {
                    if kind == "end" || kind == "skip" {
                        if !external_references(&p)?.is_empty() {
                            // expanded only using captured literal text at runtime
                            validate_external_template(
                                &p,
                                self.case_ignore,
                                regexlimits,
                                &self.keyword,
                            )?;
                            // Capture lengths are known only after the region
                            // start matches. The scanner validates these offsets
                            // against the expanded template before publishing it.
                            let template = PatternTemplate {
                                source: p.into(),
                                compiled: None,
                                offsets,
                                matchgroup,
                                excludenl,
                            };
                            if kind == "end" {
                                ends.push(template);
                            } else {
                                if skip.is_some() {
                                    return Err("multiple skip patterns".into());
                                }
                                skip = Some(template);
                            }
                            continue;
                        }
                    }
                    let compiled = VimPattern::compile_with_keyword(
                        &p,
                        self.case_ignore,
                        regexlimits,
                        &self.keyword,
                    )?;
                    validate_pattern_offsets(kind, offsets, compiled.minimum_chars)?;
                    self.program.multiline |= compiled.multiline;
                    let template = PatternTemplate {
                        source: p.into(),
                        compiled: Some(compiled),
                        offsets,
                        matchgroup,
                        excludenl,
                    };
                    match kind {
                        "start" => starts.push(template),
                        "end" => ends.push(template),
                        "skip" => {
                            if skip.is_some() {
                                return Err("multiple skip patterns".into());
                            }
                            skip = Some(template);
                        }
                        _ => unreachable!(),
                    }
                }
                if starts.is_empty() || ends.is_empty() {
                    return Err("region requires start and end patterns".into());
                }
                for template in ends.iter().chain(skip.iter()) {
                    for number in external_references(&template.source)? {
                        if starts
                            .iter()
                            .any(|p| p.compiled.as_ref().unwrap().external_groups.len() < number)
                        {
                            return Err("external reference has no matching start capture".into());
                        }
                    }
                }
                RuleKind::Region {
                    starts,
                    ends,
                    skip,
                    ignore_case: self.case_ignore,
                    keyword: self.keyword.clone(),
                }
            }
            _ => return Err(format!("unsupported syntax declaration: {kind}")),
        };
        self.add_rule(group, rule_kind, options)
    }
    fn add_rule(
        &mut self,
        group: &str,
        rule_kind: RuleKind,
        mut options: RuleOptions,
    ) -> Result<(), String> {
        if let RuleKind::Match(p) = &rule_kind {
            self.program.multiline |= p.multiline;
        }
        if self.program.rules.len() >= self.limits.rules {
            return Err("syntax rule budget exceeded".into());
        }
        let bytes = match &rule_kind {
            RuleKind::Keywords(p) | RuleKind::Match(p) => p.memory_usage(),
            RuleKind::Region {
                starts, ends, skip, ..
            } => starts
                .iter()
                .chain(ends.iter())
                .chain(skip.iter())
                .map(|p| p.source.len() + p.compiled.as_ref().map_or(0, VimPattern::memory_usage))
                .sum::<usize>(),
        };
        self.compiled_bytes = self.compiled_bytes.saturating_add(bytes);
        if self.compiled_bytes > self.limits.program_bytes {
            return Err("compiled syntax program byte budget exceeded".into());
        }
        if let Some(cluster) = self.include_stack.last() {
            if !options.contained {
                if let Some(cluster) = cluster {
                    let groups = self.program.clusters.entry(cluster.clone()).or_default();
                    if !groups.iter().any(|name| name == group) {
                        groups.push(group.to_owned());
                    }
                }
            }
            options.contained = true;
        }
        self.program.rules.push(Rule {
            group: group.to_owned(),
            kind: rule_kind,
            options,
        });
        Ok(())
    }
    fn expand_groups(&mut self, groups: Vec<String>) -> Result<Vec<String>, String> {
        if groups.len() > 4096 {
            return Err("syntax group-list budget exceeded".into());
        }
        if validate_groups(&groups).is_ok() {
            return Ok(groups
                .into_iter()
                .map(|name| self.intern_reference(&name))
                .collect());
        }
        // Vim expands group-name patterns against names known before this
        // declaration. Later group declarations must not retroactively match.
        let mut known = BTreeSet::new();
        for rule in &self.program.rules {
            known.insert(rule.group.clone());
            for name in rule
                .options
                .contains
                .iter()
                .chain(&rule.options.containedin)
                .chain(&rule.options.nextgroup)
                .chain(rule.options.matchgroup.iter())
            {
                if !name.starts_with('@') {
                    known.insert(name.clone());
                }
            }
            if let RuleKind::Region {
                starts, ends, skip, ..
            } = &rule.kind
            {
                for pattern in starts.iter().chain(ends).chain(skip.iter()) {
                    if let Some(name) = &pattern.matchgroup {
                        known.insert(name.to_string());
                    }
                }
            }
        }
        known.extend(
            self.program
                .group_names
                .iter()
                .filter(|(key, _)| !key.starts_with('@'))
                .map(|(_, name)| name.clone()),
        );
        for name in self
            .program
            .links
            .keys()
            .chain(self.program.links.values())
            .chain(self.program.clusters.values().flatten())
        {
            if !name.starts_with('@') {
                known.insert(name.clone());
            }
        }
        known.retain(|name| {
            !matches!(
                name.as_str(),
                "ALL" | "ALLBUT" | "TOP" | "CONTAINED" | "NONE"
            )
        });
        let mut result = Vec::new();
        for group in groups {
            if group.is_empty()
                || group.starts_with('@') && validate_groups(std::slice::from_ref(&group)).is_err()
            {
                return Err("invalid syntax group or cluster name".into());
            }
            if validate_groups(std::slice::from_ref(&group)).is_ok() {
                result.push(self.intern_reference(&group));
                continue;
            }
            let pattern = VimPattern::compile(
                &format!("^\\%({group}\\)$"),
                true,
                VimRegexLimits {
                    nfa_bytes: self.limits.pattern_nfa_bytes,
                    ..Default::default()
                },
            )?;
            for name in &known {
                if pattern.matches_text_at_with_control(
                    name,
                    0,
                    &mut self.group_pattern_fuel,
                    &mut || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)),
                )? {
                    if result.len() >= 4096 {
                        return Err("syntax group-list expansion budget exceeded".into());
                    }
                    result.push(name.clone());
                }
            }
        }
        Ok(result)
    }
    fn intern_reference(&mut self, name: &str) -> String {
        if let Some(name) = name.strip_prefix('@') {
            return format!("@{}", self.program.intern_cluster_name(name));
        }
        let upper = name.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "ALL" | "ALLBUT" | "TOP" | "CONTAINED" | "NONE"
        ) {
            upper
        } else {
            self.intern_group_name(name)
        }
    }
    fn intern_group_name(&mut self, name: &str) -> String {
        if let Err(error) = self.setup.define_highlight(name) {
            self.err("syntax group environment", 0, error);
        }
        self.program.intern_group_name(name)
    }
    fn finalize_keyword_environment(&mut self) -> Result<(), String> {
        let limits = VimRegexLimits {
            nfa_bytes: self.limits.pattern_nfa_bytes,
            ..Default::default()
        };
        let mut bytes = 0usize;
        for rule in &mut self.program.rules {
            if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err("syntax compilation cancelled".into());
            }
            match &mut rule.kind {
                RuleKind::Keywords(pattern) | RuleKind::Match(pattern) => {
                    *pattern = pattern.rebind_keyword(&self.keyword, limits)?;
                    bytes = bytes.saturating_add(pattern.memory_usage());
                }
                RuleKind::Region {
                    starts,
                    ends,
                    skip,
                    keyword,
                    ..
                } => {
                    *keyword = self.keyword.clone();
                    for template in starts
                        .iter_mut()
                        .chain(ends.iter_mut())
                        .chain(skip.iter_mut())
                    {
                        if let Some(pattern) = &mut template.compiled {
                            *pattern = pattern.rebind_keyword(&self.keyword, limits)?;
                            bytes = bytes.saturating_add(pattern.memory_usage());
                        }
                        bytes = bytes.saturating_add(template.source.len());
                    }
                }
            }
            if bytes > self.limits.program_bytes {
                return Err("compiled syntax program byte budget exceeded".into());
            }
        }
        self.compiled_bytes = bytes;
        Ok(())
    }
}
fn cluster_cycle(
    name: &str,
    clusters: &BTreeMap<String, Vec<String>>,
    active: &mut BTreeSet<String>,
    checked: &mut BTreeMap<String, usize>,
    depth: usize,
) -> bool {
    if let Some(height) = checked.get(name) {
        return depth + height > 33;
    }
    if depth > 32 || !active.insert(name.into()) {
        return true;
    }
    let result = clusters.get(name).is_some_and(|v| {
        v.iter()
            .filter_map(|s| s.strip_prefix('@'))
            .any(|n| cluster_cycle(n, clusters, active, checked, depth + 1))
    });
    active.remove(name);
    if !result {
        let height = 1 + clusters
            .get(name)
            .map(|v| {
                v.iter()
                    .filter_map(|s| s.strip_prefix('@'))
                    .filter_map(|n| checked.get(n))
                    .copied()
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        checked.insert(name.into(), height);
    }
    result
}
fn validate_groups(groups: &[String]) -> Result<(), String> {
    if groups.iter().any(|g| {
        g.is_empty()
            || !g
                .strip_prefix('@')
                .unwrap_or(g)
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    }) {
        return Err("invalid literal syntax group name".into());
    }
    Ok(())
}
fn external_references(p: &str) -> Result<Vec<usize>, String> {
    let mut chars = p.chars();
    let mut refs = Vec::new();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next() == Some('z') {
            if let Some(n) = chars.next().and_then(|c| c.to_digit(10)).filter(|n| *n > 0) {
                refs.push(n as usize);
            }
        }
    }
    Ok(refs)
}
pub(super) fn validate_pattern_offsets(
    kind: &str,
    offsets: PatternOffsets,
    minimum: usize,
) -> Result<(), String> {
    if let Some(offset) = offsets.ms.filter(|_| matches!(kind, "match" | "start")) {
        let earliest = match offset.base {
            OffsetBase::Start => offset.delta,
            OffsetBase::End => minimum as isize - 1 + offset.delta,
        };
        if earliest < 0 {
            return Err(
                "retroactive match-start offset is outside the native syntax profile".into(),
            );
        }
    }
    if kind == "end" || kind == "skip" {
        let ends = [
            (offsets.me, true),
            (offsets.he.filter(|_| kind == "end"), true),
            (offsets.re.filter(|_| kind == "end"), false),
        ];
        for (offset, inclusive) in ends {
            let Some(offset) = offset else { continue };
            let earliest = match offset.base {
                OffsetBase::Start => offset.delta + isize::from(inclusive),
                OffsetBase::End => minimum as isize + offset.delta,
            };
            if earliest < 0 || kind == "skip" && earliest == 0 {
                return Err(
                    "retroactive/nonadvancing region offset is outside the native syntax profile"
                        .into(),
                );
            }
        }
    }
    Ok(())
}
fn validate_external_template(
    p: &str,
    case: bool,
    limits: VimRegexLimits,
    keyword: &VimKeyword,
) -> Result<(), String> {
    let mut p = p.to_owned();
    for n in 1..=9 {
        p = p.replace(&format!("\\z{n}"), "X");
    }
    VimPattern::compile_with_keyword(&p, case, limits, keyword).map(|_| ())
}
fn word(s: &str) -> Result<(&str, &str), String> {
    let s = s.trim_start();
    if s.is_empty() {
        return Err("missing command argument".into());
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Ok((&s[..end], s[end..].trim_start()))
}
fn delimited(s: &str) -> Result<(String, &str), String> {
    let s = s.trim_start();
    let delim = s.chars().next().ok_or("missing pattern")?;
    if delim.is_alphanumeric() || delim.is_whitespace() || delim == '\\' {
        return Err("invalid pattern delimiter".into());
    }
    let mut escaped = false;
    let mut magic_brackets = true;
    let mut multiline_class = false;
    let mut indices = s.char_indices().skip(1).peekable();
    while let Some((i, c)) = indices.next() {
        if c == delim && !escaped {
            return Ok((s[delim.len_utf8()..i].to_owned(), &s[i + c.len_utf8()..]));
        }
        if c == '[' && (escaped != magic_brackets || multiline_class) {
            if let Some(end) = bracket_end(s, i) {
                while indices.peek().is_some_and(|(i, _)| *i <= end) {
                    indices.next();
                }
                escaped = false;
                multiline_class = false;
                continue;
            }
        }
        if escaped {
            match c {
                'm' | 'v' => magic_brackets = true,
                'M' | 'V' => magic_brackets = false,
                _ => {}
            }
        }
        multiline_class = escaped && c == '_';
        if c == '\\' {
            escaped = !escaped;
        } else {
            escaped = false;
        }
    }
    Err("unterminated syntax pattern".into())
}
fn bracket_end(source: &str, start: usize) -> Option<usize> {
    let mut chars = source[start + 1..].char_indices().peekable();
    if chars.peek().is_some_and(|(_, c)| *c == '^') {
        chars.next();
    }
    if chars.peek().is_some_and(|(_, c)| *c == ']') {
        chars.next();
    }
    while let Some((at, ch)) = chars.next() {
        if ch == '\\' {
            chars.next();
            continue;
        }
        if ch == ']' {
            return Some(start + 1 + at);
        }
        if ch == '['
            && chars
                .peek()
                .is_some_and(|(_, c)| matches!(c, ':' | '.' | '='))
        {
            let (_, marker) = chars.next()?;
            while let Some((_, ch)) = chars.next() {
                if ch == marker && chars.peek().is_some_and(|(_, c)| *c == ']') {
                    chars.next();
                    break;
                }
            }
        }
    }
    None
}
fn attached_offsets(s: &str) -> Result<(PatternOffsets, &str), String> {
    if s.is_empty()
        || s.starts_with(char::is_whitespace)
        || !["ms=", "me=", "hs=", "he=", "rs=", "re=", "lc="]
            .iter()
            .any(|prefix| s.starts_with(prefix))
    {
        return Ok((PatternOffsets::default(), s));
    }
    let (token, tail) = word(s)?;
    let mut offsets = PatternOffsets::default();
    for part in token.strip_suffix(',').unwrap_or(token).split(',') {
        let (key, v) = part.split_once('=').ok_or("invalid pattern offset")?;
        if key == "lc" {
            offsets.lc = v.parse().map_err(|_| "invalid leading context offset")?;
            if offsets.lc > 1024 {
                return Err("offset work budget exceeded".into());
            }
            continue;
        }
        let base = match v.chars().next() {
            Some('s') => OffsetBase::Start,
            Some('e') => OffsetBase::End,
            _ => return Err("invalid offset base".into()),
        };
        let delta = if v.len() == 1 {
            0
        } else {
            v[1..].parse::<isize>().map_err(|_| "invalid offset")?
        };
        if delta.unsigned_abs() > 1024 {
            return Err("offset work budget exceeded".into());
        }
        let slot = match key {
            "ms" => &mut offsets.ms,
            "me" => &mut offsets.me,
            "hs" => &mut offsets.hs,
            "he" => &mut offsets.he,
            "rs" => &mut offsets.rs,
            "re" => &mut offsets.re,
            _ => return Err("unsupported pattern offset".into()),
        };
        *slot = Some(Offset { base, delta });
    }
    if offsets.lc != 0 && offsets.ms.is_none() {
        offsets.ms = Some(Offset {
            base: OffsetBase::Start,
            delta: offsets.lc as isize,
        });
    }
    Ok((offsets, tail))
}
fn expand_keyword(s: &str) -> Result<Vec<String>, String> {
    if let Some(start) = s.find('[') {
        if !s.ends_with(']') {
            return Err("invalid optional keyword suffix".into());
        }
        let prefix = &s[..start];
        let suffix = &s[start + 1..s.len() - 1];
        Ok(std::iter::once(prefix.to_owned())
            .chain(
                suffix
                    .char_indices()
                    .map(|(i, c)| format!("{prefix}{}", &suffix[..i + c.len_utf8()])),
            )
            .collect())
    } else {
        Ok(vec![s.into()])
    }
}
fn parse_option(s: &str, o: &mut RuleOptions) -> Result<bool, String> {
    // Vim's option names are case-insensitive; group names and values retain
    // their spelling (the distributed make.vim uses `nextGroup`).
    let (name, value) = s.split_once('=').unwrap_or((s, ""));
    let normalized;
    let s = if name.bytes().any(|b| b.is_ascii_uppercase()) {
        normalized = if s.contains('=') {
            format!("{}={value}", name.to_ascii_lowercase())
        } else {
            name.to_ascii_lowercase()
        };
        normalized.as_str()
    } else {
        s
    };
    match s {
        "contained" => o.contained = true,
        "transparent" => o.transparent = true,
        "oneline" => o.oneline = true,
        "keepend" => o.keepend = true,
        "extend" => o.extend = true,
        "excludenl" => o.excludenl = true,
        "fold" => {}
        "display" => o.display = true,
        "skipwhite" => o.skipwhite = true,
        "skipnl" => o.skipnl = true,
        "skipempty" => o.skipempty = true,
        "conceal" | "concealends" => {}
        _ => {
            if let Some(v) = s.strip_prefix("cchar=") {
                if v.chars().count() != 1 || v.chars().any(char::is_control) {
                    return Err("invalid syntax conceal character".into());
                }
            } else if let Some(v) = s.strip_prefix("contains=") {
                o.contains = v
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                o.contains_set = true;
            } else if let Some(v) = s.strip_prefix("containedin=") {
                o.containedin = v
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            } else if let Some(v) = s.strip_prefix("nextgroup=") {
                o.nextgroup = v
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            } else if let Some(v) = s.strip_prefix("matchgroup=") {
                let v = if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
                    &v[1..v.len() - 1]
                } else {
                    v
                };
                validate_groups(&[v.to_owned()])?;
                o.matchgroup = Some(v.into());
            } else if s.starts_with("ms=")
                || s.starts_with("me=")
                || s.starts_with("hs=")
                || s.starts_with("he=")
                || s.starts_with("lc=")
            {
                let (offsets, _) = attached_offsets(s)?;
                if offsets.rs.is_some() || offsets.re.is_some() {
                    return Err("unsupported match offset".into());
                }
                if offsets.ms.is_some() {
                    o.ms = offsets.ms;
                }
                if offsets.me.is_some() {
                    o.me = offsets.me;
                }
                if offsets.hs.is_some() {
                    o.hs = offsets.hs;
                }
                if offsets.he.is_some() {
                    o.he = offsets.he;
                }
                if s.split(',').any(|s| s.starts_with("lc=")) {
                    o.lc = offsets.lc;
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn option_word(source: &str) -> Result<(String, &str), String> {
    let (token, mut rest) = word(source)?;
    let mut token = token.to_owned();
    let name = token.split('=').next().unwrap_or("").to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "contains" | "containedin" | "nextgroup" | "add" | "remove"
    ) {
        if !token.contains('=') && rest.starts_with('=') {
            let (next, tail) = word(rest)?;
            token.push_str(next);
            rest = tail;
        }
        while (token.ends_with(',') || token.ends_with('=') || rest.starts_with(','))
            && !rest.is_empty()
        {
            let (next, tail) = word(rest)?;
            if next.contains('=')
                || next.starts_with('"')
                || matches!(
                    next,
                    "contained"
                        | "transparent"
                        | "keepend"
                        | "oneline"
                        | "skipwhite"
                        | "skipnl"
                        | "skipempty"
                        | "display"
                        | "fold"
                        | "extend"
                        | "excludenl"
                )
            {
                break;
            }
            token.push_str(next);
            rest = tail;
        }
    }
    Ok((token, rest))
}
