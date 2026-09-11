use super::regex::{VimPattern, VimRegexLimits};
use super::{
    vim_literal, Offset, OffsetBase, PatternOffsets, PatternTemplate, Rule, RuleKind, RuleOptions,
    VimDiagnostic, VimLoadLimits, VimProgram,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct Loader {
    program: VimProgram,
    limits: VimLoadLimits,
    errors: Vec<VimDiagnostic>,
    root: Option<PathBuf>,
    active: BTreeSet<PathBuf>,
    bytes: usize,
    files: usize,
    compiled_bytes: usize,
    current_syntax: bool,
    case_ignore: bool,
    cancel: Option<Arc<AtomicBool>>,
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
    l.cancel = Some(cancel);
    l.source(name, text);
    l.finish()
}
pub(super) fn directory(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    directory_impl(root, language, limits, None)
}
pub(super) fn directory_cancellable(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
    cancel: Arc<AtomicBool>,
) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
    directory_impl(root, language, limits, Some(cancel))
}
fn directory_impl(
    root: &Path,
    language: &str,
    limits: VimLoadLimits,
    cancel: Option<Arc<AtomicBool>>,
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
    l.file(&format!("{language}.vim"));
    l.finish()
}
impl Loader {
    fn new(limits: VimLoadLimits, root: Option<PathBuf>) -> Self {
        Self {
            program: VimProgram {
                rules: Vec::new(),
                links: BTreeMap::new(),
                clusters: BTreeMap::new(),
                minlines: 0,
                maxlines: 200,
                fromstart: false,
                multiline: false,
                generation: 0,
                source_files: Vec::new(),
            },
            limits,
            errors: Vec::new(),
            root,
            active: BTreeSet::new(),
            bytes: 0,
            files: 0,
            compiled_bytes: 0,
            current_syntax: false,
            case_ignore: false,
            cancel: None,
        }
    }
    fn err(&mut self, file: &str, line: usize, message: impl AsRef<str>) {
        if self.errors.len() < 256 {
            self.errors
                .push(VimDiagnostic::new(file, line, message.as_ref()));
        }
    }
    fn finish(mut self) -> Result<Arc<VimProgram>, Vec<VimDiagnostic>> {
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
        let name = name.strip_prefix("syntax/").unwrap_or(name);
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
        if self.active.contains(&path) {
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
        let mut text = String::new();
        let read = std::fs::File::open(&path).and_then(|f| {
            f.take(remaining.saturating_add(1) as u64)
                .read_to_string(&mut text)
        });
        if let Err(e) = read {
            self.err(name, 0, e.to_string());
            return;
        }
        if text.len() > remaining {
            self.err(name, 0, "syntax source byte budget exceeded");
            return;
        }
        self.active.insert(path.clone());
        self.source(&path.display().to_string(), &text);
        self.active.remove(&path);
    }
    fn source(&mut self, file: &str, text: &str) {
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
        // Each entry stores parent enabled, condition selected, current branch.
        let mut branches = Vec::<(bool, bool, bool)>::new();
        let mut enabled = true;
        let mut physical = text.lines().enumerate().peekable();
        while let Some((index, line)) = physical.next() {
            if self
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                self.err(file, index + 1, "syntax compilation cancelled");
                return;
            }
            let mut logical = line.trim().to_owned();
            let line = index + 1;
            if logical.starts_with('\\') {
                self.err(file, line, "orphan continuation");
                continue;
            }
            while physical
                .peek()
                .is_some_and(|(_, s)| s.trim_start().starts_with('\\'))
            {
                let (_, continuation) = physical.next().unwrap();
                logical.push(' ');
                logical.push_str(continuation.trim_start().strip_prefix('\\').unwrap());
            }
            let text = logical.trim();
            if text.is_empty() || text.starts_with('"') {
                continue;
            }
            if let Some(condition) = text.strip_prefix("if ") {
                if branches.len() >= 64 {
                    self.err(file, line, "setup conditional depth budget exceeded");
                    return;
                }
                let value = if enabled {
                    match self.condition(condition) {
                        Ok(v) => v,
                        Err(e) => {
                            self.err(file, line, e);
                            false
                        }
                    }
                } else {
                    false
                };
                branches.push((enabled, value, enabled && value));
                enabled &= value;
                continue;
            }
            if text == "else" {
                if let Some((parent, selected, active)) = branches.last_mut() {
                    *active = *parent && !*selected;
                    *selected = true;
                    enabled = *active;
                } else {
                    self.err(file, line, "unmatched else");
                }
                continue;
            }
            if let Some(condition) = text.strip_prefix("elseif ") {
                if let Some((parent, selected, _)) = branches.last().copied() {
                    let value = if parent && !selected {
                        match self.condition(condition) {
                            Ok(v) => v,
                            Err(e) => {
                                self.err(file, line, e);
                                false
                            }
                        }
                    } else {
                        false
                    };
                    *branches.last_mut().unwrap() = (parent, selected || value, parent && value);
                    enabled = parent && value;
                } else {
                    self.err(file, line, "unmatched elseif");
                }
                continue;
            }
            if text == "endif" {
                if let Some((parent, _, _)) = branches.pop() {
                    enabled = parent;
                } else {
                    self.err(file, line, "unmatched endif");
                }
                continue;
            }
            if !enabled {
                continue;
            }
            if text == "finish" {
                branches.clear();
                break;
            }
            if let Err(e) = self.command(text) {
                self.err(file, line, e);
            }
        }
        if !branches.is_empty() {
            self.err(file, text.lines().count(), "unterminated conditional");
        }
    }
    fn condition(&self, s: &str) -> Result<bool, String> {
        let s = s.trim();
        if let Some(s) = s.strip_prefix('!') {
            return self.condition(s).map(|v| !v);
        }
        if s == "0" {
            return Ok(false);
        }
        if s == "1" {
            return Ok(true);
        }
        if let Some(s) = s.strip_prefix("exists(").and_then(|s| s.strip_suffix(')')) {
            let name = s.trim_matches(['\'', '"']);
            if name == "b:current_syntax" {
                return Ok(self.current_syntax);
            }
            // Unknown setup variables are explicitly absent in the v1 profile.
            if name.starts_with("g:") || !name.contains(':') {
                return Ok(false);
            }
        }
        Err(format!("unsupported setup condition: {s}"))
    }
    fn command(&mut self, text: &str) -> Result<(), String> {
        let (cmd, rest) = word(text)?;
        match cmd {
            "syn" | "sy" | "syntax" => self.syntax(rest),
            "hi" | "highlight" | "hi!" | "highlight!" => {
                let mut w = rest.split_whitespace().collect::<Vec<_>>();
                let default = matches!(w.first(), Some(&"def") | Some(&"default"));
                if default {
                    w.remove(0);
                }
                if w.len() == 3 && w[0] == "link" {
                    if default {
                        self.program
                            .links
                            .entry(w[1].into())
                            .or_insert_with(|| w[2].into());
                    } else {
                        self.program.links.insert(w[1].into(), w[2].into());
                    }
                    Ok(())
                } else {
                    Err("only highlight link declarations are supported (colors belong to Code styles)".into())
                }
            }
            "let" if rest.starts_with("b:current_syntax") => {
                self.current_syntax = true;
                Ok(())
            }
            "let" if rest.ends_with("&cpo") || rest.ends_with("&cpoptions") => Ok(()),
            "let" if rest.starts_with("&cpo = s:") || rest.starts_with("&cpoptions = s:") => Ok(()),
            "set" if matches!(rest, "cpo&vim" | "cpoptions&vim") => Ok(()),
            "unlet" | "unlet!" if rest == "b:current_syntax" => {
                self.current_syntax = false;
                Ok(())
            }
            "unlet" | "unlet!" if rest.starts_with("s:") && !rest.contains(' ') => Ok(()),
            "runtime" | "runtime!" => {
                self.file(rest);
                Ok(())
            }
            _ => Err(format!("unsupported native setup command: {cmd}")),
        }
    }
    fn syntax(&mut self, rest: &str) -> Result<(), String> {
        let (kind, rest) = word(rest)?;
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
        if kind == "sync" {
            for token in rest.split_whitespace() {
                if token == "fromstart" {
                    self.program.fromstart = true;
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
            return Ok(());
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
        if kind == "include" {
            if !group.starts_with('@') {
                return Err("include requires a cluster".into());
            }
            let previous = self.current_syntax;
            self.current_syntax = false;
            let start = self.program.rules.len();
            self.file(rest);
            self.current_syntax = previous;
            let groups = self.program.rules[start..]
                .iter_mut()
                .map(|r| {
                    r.options.contained = true;
                    r.group.clone()
                })
                .collect::<Vec<_>>();
            self.program
                .clusters
                .entry(group[1..].into())
                .or_default()
                .extend(groups);
            return Ok(());
        }
        if kind == "cluster" {
            for token in rest.split_whitespace() {
                if let Some(list) = token.strip_prefix("contains=") {
                    self.program
                        .clusters
                        .insert(group.into(), list.split(',').map(str::to_owned).collect());
                } else if let Some(list) = token.strip_prefix("add=") {
                    self.program
                        .clusters
                        .entry(group.into())
                        .or_default()
                        .extend(list.split(',').map(str::to_owned));
                } else if let Some(list) = token.strip_prefix("remove=") {
                    if let Some(items) = self.program.clusters.get_mut(group) {
                        items.retain(|s| !list.split(',').any(|v| v == s));
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
                let (token, tail) = word(remaining)?;
                if !parse_option(token, &mut options)? {
                    return Err(format!("unsupported leading syntax option: {token}"));
                }
                remaining = tail;
            }
            let (pattern, tail) = delimited(remaining)?;
            let (offsets, tail) = attached_offsets(tail)?;
            options.ms = offsets.ms;
            options.me = offsets.me;
            options.hs = offsets.hs;
            options.he = offsets.he;
            patterns.push(("match", pattern, PatternOffsets::default(), None, false));
            remaining = tail;
        }
        while !remaining.trim().is_empty() {
            remaining = remaining.trim_start();
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
                                .map(Arc::<str>::from),
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
            let (token, tail) = word(remaining)?;
            remaining = tail;
            if !parse_option(token, &mut options)? {
                if kind == "keyword" {
                    keywords.extend(expand_keyword(token)?);
                } else {
                    return Err(format!("unsupported syntax option: {token}"));
                }
            }
        }
        let regexlimits = VimRegexLimits {
            nfa_bytes: self.limits.pattern_nfa_bytes,
            ..Default::default()
        };
        for list in [&options.contains, &options.containedin, &options.nextgroup] {
            validate_groups(list)?;
        }
        let rule_kind = match kind {
            "keyword" => {
                if keywords.is_empty() {
                    return Err("empty keyword declaration".into());
                }
                let pattern = format!(
                    "\\<\\%({}\\)\\>",
                    keywords
                        .iter()
                        .map(|s| vim_literal(s))
                        .collect::<Vec<_>>()
                        .join("\\|")
                );
                RuleKind::Keywords(VimPattern::compile(
                    &pattern,
                    self.case_ignore,
                    regexlimits,
                )?)
            }
            "match" => {
                let p = VimPattern::compile(&patterns[0].1, self.case_ignore, regexlimits)?;
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
                            validate_external_template(&p, self.case_ignore, regexlimits)?;
                            validate_pattern_offsets(kind, offsets, 0)?;
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
                    let compiled = VimPattern::compile(&p, self.case_ignore, regexlimits)?;
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
                if options.oneline
                    && starts
                        .iter()
                        .chain(ends.iter())
                        .chain(skip.iter())
                        .filter_map(|e| e.compiled.as_ref())
                        .any(|p| p.multiline)
                {
                    return Err(
                        "multiline atoms in oneline regions are outside native profile v1".into(),
                    );
                }
                RuleKind::Region {
                    starts,
                    ends,
                    skip,
                    ignore_case: self.case_ignore,
                }
            }
            _ => return Err(format!("unsupported syntax declaration: {kind}")),
        };
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
        self.program.rules.push(Rule {
            group: group.into(),
            kind: rule_kind,
            options,
        });
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
        return Err("group-name regular patterns are outside native profile v1".into());
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
fn validate_pattern_offsets(
    kind: &str,
    offsets: PatternOffsets,
    minimum: usize,
) -> Result<(), String> {
    if let Some(offset) = offsets.ms {
        let earliest = match offset.base {
            OffsetBase::Start => offset.delta,
            OffsetBase::End => minimum as isize - 1 + offset.delta,
        };
        if earliest < 0 {
            return Err("retroactive match-start offset is outside native profile v1".into());
        }
    }
    if kind == "end" || kind == "skip" {
        for offset in [offsets.me, offsets.he, offsets.re].into_iter().flatten() {
            let earliest = match offset.base {
                OffsetBase::Start => offset.delta + 1,
                OffsetBase::End => minimum as isize + offset.delta,
            };
            if earliest < 0 || kind == "skip" && earliest == 0 {
                return Err(
                    "retroactive/nonadvancing region offset is outside native profile v1".into(),
                );
            }
        }
    }
    Ok(())
}
fn validate_external_template(p: &str, case: bool, limits: VimRegexLimits) -> Result<(), String> {
    let mut p = p.to_owned();
    for n in 1..=9 {
        p = p.replace(&format!("\\z{n}"), "X");
    }
    VimPattern::compile(&p, case, limits).map(|_| ())
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
    for (i, c) in s.char_indices().skip(1) {
        if c == delim && !escaped {
            return Ok((s[delim.len_utf8()..i].to_owned(), &s[i + c.len_utf8()..]));
        }
        if c == '\\' {
            escaped = !escaped;
        } else {
            escaped = false;
        }
    }
    Err("unterminated syntax pattern".into())
}
fn attached_offsets(s: &str) -> Result<(PatternOffsets, &str), String> {
    if s.is_empty() || s.starts_with(char::is_whitespace) {
        return Ok((PatternOffsets::default(), s));
    }
    let (token, tail) = word(s)?;
    let mut offsets = PatternOffsets::default();
    for part in token.split(',') {
        let (key, v) = part.split_once('=').ok_or("invalid pattern offset")?;
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
        "conceal" | "concealends" => return Err(format!("{s} is not supported in literal Code")),
        _ => {
            if let Some(v) = s.strip_prefix("contains=") {
                o.contains = v.split(',').map(str::to_owned).collect();
                o.contains_set = true;
            } else if let Some(v) = s.strip_prefix("containedin=") {
                o.containedin = v.split(',').map(str::to_owned).collect();
            } else if let Some(v) = s.strip_prefix("nextgroup=") {
                o.nextgroup = v.split(',').map(str::to_owned).collect();
            } else if let Some(v) = s.strip_prefix("matchgroup=") {
                o.matchgroup = Some(v.into());
            } else if s.starts_with("ms=")
                || s.starts_with("me=")
                || s.starts_with("hs=")
                || s.starts_with("he=")
            {
                for part in s.split(',') {
                    let (key, v) = part.split_once('=').ok_or("invalid offset")?;
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
                    let off = Offset { base, delta };
                    if delta.unsigned_abs() > 1024 {
                        return Err("offset work budget exceeded".into());
                    }
                    match key {
                        "ms" => o.ms = Some(off),
                        "me" => o.me = Some(off),
                        "hs" => o.hs = Some(off),
                        "he" => o.he = Some(off),
                        _ => return Err("unsupported offset".into()),
                    }
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
