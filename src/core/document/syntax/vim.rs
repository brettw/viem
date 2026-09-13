//! Native Vim syntax profile v2: strict declaration loading and resumable,
//! fuel-metered regular matching. See `vim/PROFILE.md` for compatibility limits.
mod checkpoints;
mod loader;
mod regex;
use super::super::formatted_text::{FormattedLeafLocation, LeafBoundarySide};
use super::{Coverage, SyntaxInputIdentity, SyntaxInputSnapshot, SyntaxRun, SyntaxStyleName};
use checkpoints::Checkpoints;
pub use regex::{
    VimPattern, VimRegexContinuation, VimRegexLimits, VimRegexMatch, VimRegexProgress,
};
use std::{collections::BTreeMap, ops::Range, path::Path, sync::Arc};

pub const NATIVE_PROFILE_VERSION: u32 = 2;

/// Bounded, immutable input available while compiling a syntax program. This
/// supplies runtime dialect detection without exposing editor commands or
/// rescanning the document during highlighting.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VimSetupContext {
    pub prefix: String,
}
impl VimSetupContext {
    pub fn from_input(input: &SyntaxInputSnapshot) -> Self {
        let text = input.text_tree();
        let mut end = 0;
        for line in 0..text.hard_line_count().min(32) {
            let Ok(line_end) = text.hard_line_end(line) else {
                break;
            };
            if line_end > 64 * 1024 {
                break;
            }
            end = line_end;
        }
        Self {
            prefix: input.slice(0..end).unwrap_or_default(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimDiagnostic {
    pub file: String,
    pub line: usize,
    pub message: String,
}
impl VimDiagnostic {
    fn new(file: &str, line: usize, message: &str) -> Self {
        Self {
            file: file.into(),
            line,
            message: message.into(),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct VimLoadLimits {
    pub source_bytes: usize,
    pub files: usize,
    pub rules: usize,
    pub pattern_nfa_bytes: usize,
    pub program_bytes: usize,
}
impl Default for VimLoadLimits {
    fn default() -> Self {
        Self {
            source_bytes: 4 * 1024 * 1024,
            files: 64,
            rules: 4096,
            pattern_nfa_bytes: 1024 * 1024,
            program_bytes: 32 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub struct VimProgram {
    retained_bytes: usize,
    rules: Vec<Rule>,
    links: BTreeMap<String, String>,
    clusters: BTreeMap<String, Vec<String>>,
    minlines: usize,
    maxlines: usize,
    fromstart: bool,
    multiline: bool,
    pub generation: u64,
    pub source_files: Vec<String>,
}
impl VimProgram {
    pub fn compile(
        name: &str,
        text: &str,
        limits: VimLoadLimits,
    ) -> Result<Arc<Self>, Vec<VimDiagnostic>> {
        loader::compile(name, text, limits)
    }
    pub fn compile_cancellable(
        name: &str,
        text: &str,
        limits: VimLoadLimits,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Arc<Self>, Vec<VimDiagnostic>> {
        loader::compile_cancellable(name, text, limits, cancel)
    }
    pub fn load_directory(
        root: impl AsRef<Path>,
        language: &str,
        limits: VimLoadLimits,
    ) -> Result<Arc<Self>, Vec<VimDiagnostic>> {
        loader::directory(root.as_ref(), language, limits)
    }
    pub fn load_directory_cancellable(
        root: impl AsRef<Path>,
        language: &str,
        limits: VimLoadLimits,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Arc<Self>, Vec<VimDiagnostic>> {
        loader::directory_cancellable(root.as_ref(), language, limits, cancel)
    }
    pub fn load_directory_with_context(
        root: impl AsRef<Path>,
        language: &str,
        limits: VimLoadLimits,
        context: &VimSetupContext,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Arc<Self>, Vec<VimDiagnostic>> {
        loader::directory_with_context(root.as_ref(), language, limits, context, Some(cancelled))
    }
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
    pub fn effective_group<'a>(&'a self, group: &'a str) -> &'a str {
        let mut group = group;
        for _ in 0..=self.links.len() {
            match self.links.get(group) {
                Some(next) => group = next,
                None => break,
            }
        }
        group
    }
    fn group_in(&self, group: &str, contained: bool, list: &[String], depth: usize) -> bool {
        self.group_in_cached(group, contained, list, depth, &mut BTreeMap::new())
    }
    fn group_in_cached(
        &self,
        group: &str,
        contained: bool,
        list: &[String],
        depth: usize,
        memo: &mut BTreeMap<String, bool>,
    ) -> bool {
        if depth > 32 {
            return false;
        }
        if list.first().is_some_and(|s| s == "TOP") && contained
            || list.first().is_some_and(|s| s == "CONTAINED") && !contained
        {
            return false;
        }
        let excluded = list
            .first()
            .is_some_and(|s| matches!(s.as_str(), "ALLBUT" | "TOP" | "CONTAINED"));
        let matches = list.iter().skip(usize::from(excluded)).any(|name| {
            name == group
                || name.strip_prefix('@').is_some_and(|name| {
                    if let Some(found) = memo.get(name) {
                        return *found;
                    }
                    let found = self.clusters.get(name).is_some_and(|items| {
                        self.group_in_cached(group, contained, items, depth + 1, memo)
                    });
                    memo.insert(name.into(), found);
                    found
                })
        });
        if excluded {
            !matches
        } else {
            list.iter().any(|s| s == "ALL") || matches
        }
    }
}
#[derive(Clone, Debug)]
struct Rule {
    group: String,
    kind: RuleKind,
    options: RuleOptions,
}
#[derive(Clone, Debug)]
enum RuleKind {
    Keywords(VimPattern),
    Match(VimPattern),
    Region {
        starts: Vec<PatternTemplate>,
        ends: Vec<PatternTemplate>,
        skip: Option<PatternTemplate>,
        ignore_case: bool,
    },
}
#[derive(Clone, Debug)]
struct PatternTemplate {
    source: Arc<str>,
    compiled: Option<VimPattern>,
    offsets: PatternOffsets,
    matchgroup: Option<Arc<str>>,
    excludenl: bool,
}
#[derive(Clone, Copy, Debug, Default)]
struct PatternOffsets {
    lc: usize,
    ms: Option<Offset>,
    me: Option<Offset>,
    hs: Option<Offset>,
    he: Option<Offset>,
    rs: Option<Offset>,
    re: Option<Offset>,
}
#[derive(Clone, Debug, Default)]
struct RuleOptions {
    lc: usize,
    contained: bool,
    transparent: bool,
    oneline: bool,
    keepend: bool,
    extend: bool,
    display: bool,
    excludenl: bool,
    skipwhite: bool,
    skipnl: bool,
    skipempty: bool,
    contains: Vec<String>,
    contains_set: bool,
    containedin: Vec<String>,
    nextgroup: Vec<String>,
    matchgroup: Option<String>,
    ms: Option<Offset>,
    me: Option<Offset>,
    hs: Option<Offset>,
    he: Option<Offset>,
}
#[derive(Clone, Copy, Debug)]
enum OffsetBase {
    Start,
    End,
}
#[derive(Clone, Copy, Debug)]
struct Offset {
    base: OffsetBase,
    delta: isize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VimBudget {
    pub instructions: usize,
    pub match_instructions: usize,
    pub spans: usize,
    pub checkpoints: usize,
    pub stack_depth: usize,
    pub continuation_bytes: usize,
    pub captured_bytes: usize,
    pub retained_bytes: usize,
    pub allow_provisional: bool,
}
impl Default for VimBudget {
    fn default() -> Self {
        Self {
            instructions: 200_000,
            match_instructions: 8_000_000,
            spans: 8192,
            checkpoints: 256,
            stack_depth: 64,
            continuation_bytes: 16 * 1024 * 1024,
            captured_bytes: 4096,
            retained_bytes: 64 * 1024 * 1024,
            allow_provisional: true,
        }
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VimStats {
    pub instructions: usize,
    pub evaluated_lines: usize,
    pub input_bytes: usize,
    pub checkpoint_visits: usize,
    pub checkpoints: usize,
    pub cache_hits: usize,
    pub retained_bytes: usize,
    pub yielded: bool,
}
#[derive(Clone, Debug)]
pub struct VimResult {
    pub identity: SyntaxInputIdentity,
    pub requested: Range<usize>,
    pub covered: Range<usize>,
    pub runs: Vec<SyntaxRun>,
    pub coverage: Coverage,
    pub stats: VimStats,
    pub diagnostics: Vec<VimDiagnostic>,
}

#[derive(Clone, Debug)]
struct RegionState {
    rule: usize,
    captures: Vec<String>,
    ends: Vec<PatternTemplate>,
    skip: Option<PatternTemplate>,
}
impl PartialEq for RegionState {
    fn eq(&self, other: &Self) -> bool {
        self.rule == other.rule && self.captures == other.captures
    }
}
impl Eq for RegionState {}
impl RegionState {
    fn memory_usage(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.captures.iter().map(String::len).sum::<usize>()
            + self
                .ends
                .iter()
                .chain(self.skip.iter())
                .map(|p| {
                    std::mem::size_of::<PatternTemplate>()
                        + p.source.len()
                        + p.compiled.as_ref().map_or(0, VimPattern::memory_usage)
                })
                .sum::<usize>()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Transition {
    groups: Vec<String>,
    skipwhite: bool,
    skipnl: bool,
    skipempty: bool,
}
#[derive(Clone, Debug)]
struct Restart {
    anchor: Option<FormattedLeafLocation>,
    regions: Vec<Arc<RegionState>>,
    next: Option<Transition>,
}
impl Restart {
    fn memory_usage(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.regions.iter().map(|r| r.memory_usage()).sum::<usize>()
            + self
                .next
                .as_ref()
                .map_or(0, |n| n.groups.iter().map(String::len).sum::<usize>())
    }
    fn same_state(&self, other: &Self) -> bool {
        self.regions == other.regions && self.next == other.next
    }
    fn valid_at(&self, input: &SyntaxInputSnapshot, at: usize) -> bool {
        let current = input
            .text_tree()
            .locate_byte(at, LeafBoundarySide::Following)
            .ok()
            .flatten();
        match (&self.anchor, current) {
            (None, None) => true,
            (Some(a), Some(b)) => a.buffer_id == b.buffer_id && a.buffer_byte == b.buffer_byte,
            _ => false,
        }
    }
}
#[derive(Clone, Debug)]
struct Frame {
    rule: usize,
    region: Option<Arc<RegionState>>,
    start_end: usize,
    match_end: Option<usize>,
    paint: Range<usize>,
    closing: Option<Range<usize>>,
    skip_until: usize,
    search_start: usize,
    start_group: Option<Arc<str>>,
    end_group: Option<Arc<str>>,
    eol_extension: bool,
}
#[derive(Clone, Debug)]
enum ProbeKind {
    Start { rule: usize, region: bool },
    End { frame: usize },
    Skip { frame: usize },
}
#[derive(Clone, Debug)]
struct Probe {
    kind: ProbeKind,
    pattern: VimPattern,
    offsets: PatternOffsets,
    matchgroup: Option<Arc<str>>,
    excludenl: bool,
}
impl Probe {
    fn template(kind: ProbeKind, p: &PatternTemplate) -> Self {
        Self {
            kind,
            pattern: p.compiled.as_ref().unwrap().clone(),
            offsets: p.offsets,
            matchgroup: p.matchgroup.clone(),
            excludenl: p.excludenl,
        }
    }
}
#[derive(Clone, Debug)]
struct Candidate {
    kind: ProbeKind,
    found: VimRegexMatch,
    pattern: VimPattern,
    region: Option<Arc<RegionState>>,
    offsets: PatternOffsets,
    matchgroup: Option<Arc<str>>,
    excludenl: bool,
}
#[derive(Clone, Debug)]
struct OnelineCheck {
    candidate: Candidate,
    position: usize,
    line_end: usize,
    index: usize,
    active: Option<VimRegexContinuation>,
    context_fuel: Option<usize>,
}
#[derive(Clone, Debug)]
struct Probes {
    items: Vec<Probe>,
    index: usize,
    active: Option<VimRegexContinuation>,
    best: Option<Candidate>,
    oneline: Option<OnelineCheck>,
    dispatch: usize,
}
#[derive(Clone, Debug)]
struct Job {
    input: SyntaxInputSnapshot,
    request: Range<usize>,
    position: usize,
    exact: bool,
    frames: Vec<Frame>,
    next: Option<Transition>,
    probes: Option<Probes>,
    runs: Vec<SyntaxRun>,
    guard: Vec<usize>,
    last_checkpoint: usize,
    run_bytes: usize,
}
#[derive(Clone, Debug)]
struct Cached {
    identity: SyntaxInputIdentity,
    range: Range<usize>,
    runs: Vec<SyntaxRun>,
    coverage: Coverage,
    bytes: usize,
}
#[derive(Clone, Debug)]
struct Failed {
    range: Range<usize>,
    dependency_end: usize,
    budget: VimBudget,
    diagnostics: Vec<VimDiagnostic>,
}

#[derive(Debug)]
pub struct VimSession {
    program: Arc<VimProgram>,
    identity: Option<SyntaxInputIdentity>,
    checkpoints: Checkpoints,
    dirty: Option<Range<usize>>,
    job: Option<Job>,
    cache: Vec<Cached>,
    failures: Vec<Failed>,
}
impl VimSession {
    pub fn new(program: Arc<VimProgram>) -> Self {
        Self {
            program,
            identity: None,
            checkpoints: Checkpoints::default(),
            dirty: None,
            job: None,
            cache: Vec::new(),
            failures: Vec::new(),
        }
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        let cached = self.cache.iter().map(|cache| cache.bytes
            + cache.runs.capacity().saturating_sub(cache.runs.len()) * std::mem::size_of::<SyntaxRun>()).sum::<usize>();
        let pending = self.job.as_ref().map_or(0, |job| {
            job.run_bytes
                + job.runs.capacity().saturating_sub(job.runs.len()) * std::mem::size_of::<SyntaxRun>()
                + job.frames.capacity() * std::mem::size_of::<Frame>()
                + job.frames.iter().filter_map(|frame| frame.region.as_ref()).map(|region| region.memory_usage()).sum::<usize>()
                + job.guard.capacity() * std::mem::size_of::<usize>()
                + job.probes.as_ref().map_or(0, |probes| {
                    probes.items.capacity() * std::mem::size_of::<Probe>()
                        + probes.active.as_ref().map_or(0, VimRegexContinuation::retained_bytes)
                        + probes.oneline.as_ref().and_then(|check| check.active.as_ref()).map_or(0, VimRegexContinuation::retained_bytes)
                })
        });
        self.program.retained_bytes.saturating_add(self.checkpoints.memory_usage())
            .saturating_add(cached).saturating_add(pending)
    }

    /// Abandon private in-flight work after a scheduler's aggregate repair
    /// budget expires. Valid published checkpoints remain available to a
    /// subsequent viewport request with a different recovery policy.
    pub fn discard_pending(&mut self) {
        self.job = None;
    }
    /// Apply a syntax-coordinate splice, never a physical-source byte patch.
    /// Tree shifts and suffix loss of exact authority are logarithmic/lazy.
    pub fn apply_edit(
        &mut self,
        old: SyntaxInputIdentity,
        new: SyntaxInputIdentity,
        old_range: Range<usize>,
        new_end: usize,
    ) -> Result<(), String> {
        if self.identity != Some(old)
            || old.document != new.document
            || old_range.start > old_range.end
            || new_end < old_range.start
        {
            return Err("wrong snapshot or invalid syntax edit".into());
        }
        self.job = None;
        self.cache.clear();
        self.checkpoints.visits = 0;
        // The conservative dependency includes the complete entry-state prefix
        // and inspected line. Later independent changes preserve capped work.
        self.failures
            .retain(|f| old_range.start > f.dependency_end && old_range.start >= f.range.end);
        self.checkpoints
            .edit(old_range.start, old_range.end, new_end);
        let delta = new_end as isize - old_range.end as isize;
        let prior = self.dirty.take().map(|r| {
            let map = |p: usize| {
                if p <= old_range.start {
                    p
                } else if p >= old_range.end {
                    p.checked_add_signed(delta).unwrap_or(new_end)
                } else {
                    new_end
                }
            };
            map(r.start)..map(r.end)
        });
        let start = if self.program.multiline {
            0
        } else {
            old_range.start
        };
        self.dirty = Some(prior.map_or(start..new_end, |r| r.start.min(start)..r.end.max(new_end)));
        self.identity = Some(new);
        Ok(())
    }
    pub fn highlight(
        &mut self,
        input: &SyntaxInputSnapshot,
        requested: Range<usize>,
        budget: VimBudget,
    ) -> VimResult {
        self.highlight_with_control(input, requested, budget, &mut || false)
    }
    pub fn highlight_with_control(
        &mut self,
        input: &SyntaxInputSnapshot,
        requested: Range<usize>,
        budget: VimBudget,
        cancel: &mut dyn FnMut() -> bool,
    ) -> VimResult {
        let mut result = VimResult {
            identity: input.identity(),
            requested: requested.clone(),
            covered: requested.start..requested.start,
            runs: Vec::new(),
            coverage: Coverage::Missing,
            stats: VimStats::default(),
            diagnostics: Vec::new(),
        };
        if requested.start > requested.end
            || requested.end > input.byte_len()
            || !input
                .text_tree()
                .is_char_boundary(requested.start)
                .unwrap_or(false)
            || !input
                .text_tree()
                .is_char_boundary(requested.end)
                .unwrap_or(false)
        {
            result.diagnostics.push(VimDiagnostic::new(
                "syntax input",
                0,
                "invalid UTF-8 requested range",
            ));
            return result;
        }
        if self.identity != Some(input.identity()) {
            self.identity = Some(input.identity());
            self.checkpoints = Checkpoints::default();
            self.dirty = None;
            self.job = None;
            self.cache.clear();
            self.failures.clear();
        }
        self.trim_caches(budget);
        if let Some(cache) = self.cache.iter().find(|c| {
            c.identity == input.identity()
                && c.range == requested
                && (budget.allow_provisional || c.coverage == Coverage::Exact)
        }) {
            result.runs = cache.runs.clone();
            result.covered = requested;
            result.coverage = cache.coverage;
            result.stats.cache_hits = 1;
            return result;
        }
        if let Some(failed) = self
            .failures
            .iter()
            .find(|f| f.range == requested && f.budget == budget)
        {
            result.diagnostics = failed.diagnostics.clone();
            result.stats.cache_hits = 1;
            return result;
        }
        if requested.is_empty() {
            result.coverage = Coverage::Exact;
            return result;
        }
        if self.job.as_ref().is_none_or(|j| {
            j.input.identity() != input.identity()
                || j.request != requested
                || !budget.allow_provisional && !j.exact
        }) {
            self.job = Some(self.begin(input, requested.clone(), budget));
        }
        let mut job = self.job.take().unwrap();
        let mut fuel = budget.instructions;
        let visits = self.checkpoints.visits;
        while job.position < job.request.end && fuel > 0 {
            if cancel() {
                break;
            }
            if job.frames.len() > budget.stack_depth || job.run_bytes > budget.retained_bytes {
                result.diagnostics.push(VimDiagnostic::new(
                    "syntax execution",
                    0,
                    "retained syntax byte/stack budget exceeded",
                ));
                break;
            }
            if job.runs.len() >= budget.spans {
                result.diagnostics.push(VimDiagnostic::new(
                    "syntax execution",
                    0,
                    "syntax span budget exceeded",
                ));
                break;
            }
            if job.probes.is_none() {
                fuel -= 1;
                self.finish_frames(&mut job);
                if job.position == 0 || byte(&job.input, job.position - 1) == Some(b'\n') {
                    if job.guard.is_empty() {
                        result.stats.evaluated_lines += 1;
                        self.checkpoint(&mut job, budget);
                    }
                }
                match self.probes(&job, budget) {
                    Ok(p) => job.probes = Some(p),
                    Err(e) => {
                        result
                            .diagnostics
                            .push(VimDiagnostic::new("syntax execution", 0, &e));
                        break;
                    }
                }
            }
            let probes = job.probes.as_mut().unwrap();
            let dispatch = probes.dispatch.min(fuel);
            fuel -= dispatch;
            probes.dispatch -= dispatch;
            if probes.dispatch > 0 {
                break;
            }
            if let Some(check) = probes.oneline.as_mut() {
                match self.check_oneline(
                    check,
                    &job.input,
                    &mut fuel,
                    budget,
                    &mut result.stats,
                    cancel,
                ) {
                    Ok(Some(valid)) => {
                        let check = probes.oneline.take().unwrap();
                        if valid {
                            probes.best = Some(check.candidate);
                            probes.index = probes.items.len();
                        }
                        continue;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        result
                            .diagnostics
                            .push(VimDiagnostic::new("syntax execution", 0, &e));
                        break;
                    }
                }
            }
            if probes.index < probes.items.len() {
                let probe = &probes.items[probes.index];
                if probes.active.is_none() {
                    if probe.pattern.continuation_bytes() > budget.continuation_bytes {
                        result.diagnostics.push(VimDiagnostic::new(
                            "syntax execution",
                            0,
                            "regex continuation byte budget exceeded",
                        ));
                        break;
                    }
                    probes.active = Some(probe.pattern.start(leading_context_start(
                        &job.input,
                        job.position,
                        probe.offsets.lc,
                    )));
                }
                let c = probes.active.as_mut().unwrap();
                let inspected = c.inspected_end;
                let mut allowed =
                    fuel.min(budget.match_instructions.saturating_sub(c.instructions));
                let before = allowed;
                let progress =
                    probe
                        .pattern
                        .resume_with_control(c, &job.input, &mut allowed, cancel);
                fuel -= before - allowed;
                result.stats.input_bytes += c.inspected_end.saturating_sub(inspected);
                if matches!(progress, VimRegexProgress::Pending)
                    && c.instructions >= budget.match_instructions
                {
                    result.diagnostics.push(VimDiagnostic::new(
                        "syntax execution",
                        0,
                        "pattern total instruction budget exceeded",
                    ));
                    break;
                }
                match progress {
                    VimRegexProgress::Pending => break,
                    VimRegexProgress::Failed(message) => {
                        result.diagnostics.push(VimDiagnostic::new(
                            "syntax execution",
                            0,
                            &message,
                        ));
                        break;
                    }
                    VimRegexProgress::Complete(found) => {
                        if let Some(found) = found.filter(|m| m.end >= job.position) {
                            if found.end >= found.start
                                && (!matches!(probe.kind, ProbeKind::Skip { .. })
                                    || found.end > found.start)
                            {
                                if probe.offsets.lc > 0
                                    && matches!(probe.kind, ProbeKind::Start { .. })
                                {
                                    match offset(&job.input, &found, probe.offsets.ms, false) {
                                        Ok(start) if start != job.position => {
                                            // At line start less than lc characters may
                                            // exist. Do not let a future match suppress
                                            // intervening rules while waiting for its ms.
                                            probes.active = None;
                                            probes.index += 1;
                                            continue;
                                        }
                                        Ok(_) => {}
                                        Err(message) => {
                                            result.diagnostics.push(VimDiagnostic::new(
                                                "syntax execution",
                                                0,
                                                &message,
                                            ));
                                            break;
                                        }
                                    }
                                }
                                let mut candidate = Candidate {
                                    kind: probe.kind.clone(),
                                    found,
                                    pattern: probe.pattern.clone(),
                                    region: None,
                                    offsets: probe.offsets,
                                    matchgroup: probe.matchgroup.clone(),
                                    excludenl: probe.excludenl,
                                };
                                if let ProbeKind::Start { rule, region: true } = candidate.kind {
                                    match self.region_state(&job.input, &candidate, rule, budget) {
                                        Ok(state) => candidate.region = Some(state),
                                        Err(e) => {
                                            result.diagnostics.push(VimDiagnostic::new(
                                                "syntax execution",
                                                0,
                                                &e,
                                            ));
                                            break;
                                        }
                                    }
                                    if self.program.rules[rule].options.oneline {
                                        let line = job
                                            .input
                                            .text_tree()
                                            .hard_line_at_byte(candidate.found.end)
                                            .unwrap_or(0);
                                        let line_end = job
                                            .input
                                            .text_tree()
                                            .hard_line_end(line)
                                            .unwrap_or(job.input.byte_len());
                                        probes.oneline = Some(OnelineCheck {
                                            position: candidate.found.end,
                                            line_end,
                                            candidate,
                                            index: 0,
                                            active: None,
                                            context_fuel: None,
                                        });
                                    } else {
                                        probes.best = Some(candidate);
                                        probes.index = probes.items.len();
                                    }
                                } else {
                                    probes.best = Some(candidate);
                                    probes.index = probes.items.len();
                                }
                            }
                        }
                        probes.active = None;
                        probes.index += 1;
                        continue;
                    }
                }
            }
            let probes = job.probes.take().unwrap();
            if let Some(candidate) = probes.best {
                if let Err(e) = self.accept(&mut job, candidate, budget) {
                    result
                        .diagnostics
                        .push(VimDiagnostic::new("syntax execution", 0, &e));
                    break;
                }
                // A new frame changes containment at this same boundary. The
                // guard prevents recursive zero-progress starts of the same rule.
                continue;
            }
            // nextgroup's whitespace gates are state, not geometry.
            if let Some(next) = &job.next {
                let b = byte(&job.input, job.position);
                let empty = job.position == 0 || byte(&job.input, job.position - 1) == Some(b'\n');
                if !((next.skipwhite && matches!(b, Some(b' ' | b'\t')))
                    || (next.skipnl && b == Some(b'\n'))
                    || (next.skipempty && empty && b == Some(b'\n')))
                {
                    job.next = None;
                    continue;
                }
            }
            let end = next_char(&job.input, job.position).min(job.request.end);
            let at = job.position;
            self.emit(&mut job, at..end);
            if byte(&job.input, at) == Some(b'\n') {
                if let Some(next) = &mut job.next {
                    // skipnl crosses the first line boundary; only skipempty
                    // allows subsequent empty lines in this transition.
                    next.skipnl = false;
                }
            }
            job.position = end;
            job.guard.clear();
        }
        result.stats.instructions = budget.instructions - fuel;
        result.stats.checkpoint_visits = self.checkpoints.visits - visits;
        result.stats.checkpoints = self.checkpoints.len();
        result.stats.retained_bytes = self.checkpoints.memory_usage()
            + self.cache.iter().map(|c| c.bytes).sum::<usize>()
            + job.run_bytes;
        let end = job.position.min(requested.end).max(requested.start);
        result.covered = requested.start..end;
        result.runs = job.runs.clone();
        if end > requested.start {
            result.coverage = if job.exact {
                Coverage::Exact
            } else {
                Coverage::Provisional
            };
        }
        if !result.diagnostics.is_empty() {
            let line = input
                .text_tree()
                .hard_line_at_byte(job.position)
                .unwrap_or(0);
            let dependency_end = if self.program.multiline {
                input.byte_len()
            } else {
                input
                    .text_tree()
                    .hard_line_end(line)
                    .unwrap_or(input.byte_len())
            };
            if self.failures.len() == 8 {
                self.failures.remove(0);
            }
            self.failures.push(Failed {
                range: requested,
                dependency_end,
                budget,
                diagnostics: result.diagnostics.clone(),
            });
        } else if job.position >= requested.end {
            result.coverage = if job.exact {
                Coverage::Exact
            } else {
                Coverage::Provisional
            };
            if self.cache.len() == 8 {
                self.cache.remove(0);
            }
            self.cache.push(Cached {
                identity: input.identity(),
                range: requested,
                runs: result.runs.clone(),
                coverage: result.coverage,
                bytes: job.run_bytes,
            });
            self.trim_caches(budget);
        } else {
            result.stats.yielded = true;
            self.job = Some(job);
        }
        result
    }
    fn trim_caches(&mut self, budget: VimBudget) {
        self.cache.retain(|c| c.runs.len() <= budget.spans);
        while self.checkpoints.len() > budget.checkpoints {
            self.checkpoints.evict_first();
        }
        loop {
            let bytes =
                self.cache.iter().map(|c| c.bytes).sum::<usize>() + self.checkpoints.memory_usage();
            if bytes <= budget.retained_bytes {
                break;
            }
            if !self.cache.is_empty() {
                self.cache.remove(0);
            } else if self.checkpoints.len() > 0 {
                self.checkpoints.evict_first();
            } else {
                break;
            }
        }
    }
    fn begin(
        &mut self,
        input: &SyntaxInputSnapshot,
        request: Range<usize>,
        budget: VimBudget,
    ) -> Job {
        let line = input
            .text_tree()
            .hard_line_at_byte(request.start)
            .unwrap_or(0);
        let line_start = input.text_tree().hard_line_start(line).unwrap_or(0);
        let exact_limit = self
            .dirty
            .as_ref()
            .map_or(line_start, |r| line_start.min(r.start.saturating_sub(1)));
        let saved = self
            .checkpoints
            .before(exact_limit)
            .filter(|(at, c)| c.valid_at(input, *at));
        let (mut position, mut exact, mut restart) = (0, true, None);
        if let Some((at, c)) = saved {
            position = at;
            restart = Some(c);
        }
        if budget.allow_provisional
            && !self.program.fromstart
            && (line_start.saturating_sub(position) > 64 * 1024
                || line.saturating_sub(input.text_tree().hard_line_at_byte(position).unwrap_or(0))
                    > self.program.maxlines.max(32))
        {
            let back = self
                .program
                .minlines
                .max(32)
                .min(self.program.maxlines.max(32));
            position = input
                .text_tree()
                .hard_line_start(line.saturating_sub(back))
                .unwrap_or(0);
            exact = position == 0;
            restart = None;
        }
        let frames = restart
            .as_ref()
            .map(|r| {
                r.regions
                    .iter()
                    .map(|region| Frame {
                        rule: region.rule,
                        region: Some(region.clone()),
                        start_end: position,
                        match_end: None,
                        paint: position..usize::MAX,
                        closing: None,
                        skip_until: position,
                        search_start: position,
                        start_group: None,
                        end_group: None,
                        eol_extension: false,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Job {
            input: input.clone(),
            request,
            position,
            exact,
            frames,
            next: restart.and_then(|r| r.next.clone()),
            probes: None,
            runs: Vec::new(),
            guard: Vec::new(),
            last_checkpoint: position,
            run_bytes: 0,
        }
    }
    fn finish_frames(&self, j: &mut Job) {
        // A keepend boundary closes nested items; extend explicitly suspends it.
        let cutoff = j.frames.iter().enumerate().find_map(|(i, f)| {
            let opts = &self.program.rules[f.rule].options;
            let ends = f.match_end.or_else(|| f.closing.as_ref().map(|r| r.end));
            (opts.keepend
                && ends.is_some_and(|end| end <= j.position)
                && !j.frames[i + 1..]
                    .iter()
                    .any(|f| self.program.rules[f.rule].options.extend))
            .then_some(i)
        });
        if let Some(i) = cutoff {
            j.frames.truncate(i + 1);
        }
        while j.frames.last().is_some_and(|f| {
            f.match_end.is_some_and(|end| end <= j.position)
                || f.closing.as_ref().is_some_and(|r| r.end <= j.position)
                || self.program.rules[f.rule].options.oneline
                    && j.position >= f.search_start
                    && f.closing.is_none()
                    && f.skip_until <= j.position
                    && byte(&j.input, j.position) == Some(b'\n')
        }) {
            let frame = j.frames.pop().unwrap();
            let o = &self.program.rules[frame.rule].options;
            if frame.eol_extension && byte(&j.input, j.position) == Some(b'\n') {
                if let Some(parent) = j.frames.last_mut() {
                    if !self.program.rules[parent.rule].options.keepend || o.extend {
                        parent.skip_until = next_char(&j.input, j.position);
                    }
                }
            }
            if !o.nextgroup.is_empty() {
                j.next = Some(Transition {
                    groups: o.nextgroup.clone(),
                    skipwhite: o.skipwhite,
                    skipnl: o.skipnl || o.skipempty,
                    skipempty: o.skipempty,
                });
            }
        }
    }
    fn checkpoint(&mut self, j: &mut Job, budget: VimBudget) {
        let limit = budget.checkpoints;
        if !j.exact
            || limit == 0
            || j.frames.iter().any(|f| {
                f.region.is_none()
                    || f.start_end > j.position
                    || f.search_start > j.position
                    || f.paint.start > j.position
                    || f.closing.is_some()
                    || f.skip_until > j.position
            })
        {
            return;
        }
        let restart = Restart {
            anchor: j
                .input
                .text_tree()
                .locate_byte(j.position, LeafBoundarySide::Following)
                .ok()
                .flatten(),
            regions: j.frames.iter().filter_map(|f| f.region.clone()).collect(),
            next: j.next.clone(),
        };
        if self.dirty.as_ref().is_some_and(|r| j.position > r.end) {
            if self
                .checkpoints
                .exact(j.position)
                .is_some_and(|old| old.valid_at(&j.input, j.position) && old.same_state(&restart))
            {
                self.dirty = None;
                if j.position < j.request.start {
                    let input = j.input.clone();
                    let next = self.begin(
                        &input,
                        j.request.clone(),
                        VimBudget {
                            allow_provisional: false,
                            ..budget
                        },
                    );
                    if next.position > j.position {
                        *j = next;
                        return;
                    }
                }
            }
        }
        if j.position >= j.request.start.saturating_sub(8192)
            || j.position.saturating_sub(j.last_checkpoint) >= 4096
        {
            self.checkpoints.insert(j.position, restart);
            j.last_checkpoint = j.position;
            while self.checkpoints.len() > limit
                || self.checkpoints.memory_usage() > budget.retained_bytes
            {
                self.checkpoints.evict_first();
            }
        }
    }
    fn probes(&self, j: &Job, _budget: VimBudget) -> Result<Probes, String> {
        let mut items = Vec::new();
        for (index, f) in j.frames.iter().enumerate().rev() {
            let o = &self.program.rules[f.rule].options;
            if index + 1 != j.frames.len()
                && (!o.keepend
                    || j.frames[index + 1..]
                        .iter()
                        .any(|f| self.program.rules[f.rule].options.extend))
            {
                continue;
            }
            if j.position < f.search_start || j.position < f.skip_until || f.closing.is_some() {
                continue;
            }
            if let Some(region) = &f.region {
                if let Some(p) = &region.skip {
                    items.push(Probe::template(ProbeKind::Skip { frame: index }, p));
                }
                for p in region.ends.iter().rev() {
                    items.push(Probe::template(ProbeKind::End { frame: index }, p));
                }
            }
        }
        let blocked = j.frames.last().is_some_and(|f| {
            f.start_group.is_some() && j.position < f.start_end
                || f.end_group.is_some() && f.closing.is_some()
        });
        if !blocked {
            let mut keywords = Vec::new();
            for (i, rule) in self.program.rules.iter().enumerate().rev() {
                if j.guard.contains(&i) {
                    continue;
                }
                let allowed = if let Some(next) = &j.next {
                    self.program
                        .group_in(&rule.group, rule.options.contained, &next.groups, 0)
                } else if let Some(parent) = j.frames.last() {
                    let parentrule = &self.program.rules[parent.rule];
                    let containing = j.frames.iter().rev().find(|f| {
                        let o = &self.program.rules[f.rule].options;
                        !o.transparent || o.contains_set
                    });
                    containing.map_or(!rule.options.contained, |f| {
                        self.program.group_in(
                            &rule.group,
                            rule.options.contained,
                            &self.program.rules[f.rule].options.contains,
                            0,
                        )
                    }) || self.program.group_in(
                        &parentrule.group,
                        parentrule.options.contained,
                        &rule.options.containedin,
                        0,
                    )
                } else {
                    !rule.options.contained
                };
                if !allowed {
                    continue;
                }
                match &rule.kind {
                    RuleKind::Keywords(pattern) => keywords.push(Probe {
                        kind: ProbeKind::Start {
                            rule: i,
                            region: false,
                        },
                        pattern: pattern.clone(),
                        offsets: PatternOffsets::default(),
                        matchgroup: None,
                        excludenl: false,
                    }),
                    RuleKind::Match(p) => items.push(Probe {
                        kind: ProbeKind::Start {
                            rule: i,
                            region: false,
                        },
                        pattern: p.clone(),
                        offsets: PatternOffsets {
                            lc: rule.options.lc,
                            ms: rule.options.ms,
                            me: rule.options.me,
                            hs: rule.options.hs,
                            he: rule.options.he,
                            ..Default::default()
                        },
                        matchgroup: None,
                        excludenl: rule.options.excludenl,
                    }),
                    RuleKind::Region { starts, .. } => {
                        for p in starts.iter().rev() {
                            items.push(Probe::template(
                                ProbeKind::Start {
                                    rule: i,
                                    region: true,
                                },
                                p,
                            ));
                        }
                    }
                }
            }
            let first_start = items
                .iter()
                .position(|p| matches!(p.kind, ProbeKind::Start { .. }))
                .unwrap_or(items.len());
            items.splice(first_start..first_start, keywords);
        }
        let dispatch = items.len()
            + self.program.rules.len()
            + items.iter().map(|p| p.offsets.lc).sum::<usize>();
        Ok(Probes {
            items,
            index: 0,
            active: None,
            best: None,
            oneline: None,
            dispatch,
        })
    }
    fn region_state(
        &self,
        input: &SyntaxInputSnapshot,
        c: &Candidate,
        rule: usize,
        budget: VimBudget,
    ) -> Result<Arc<RegionState>, String> {
        let RuleKind::Region {
            ends,
            skip,
            ignore_case,
            ..
        } = &self.program.rules[rule].kind
        else {
            unreachable!()
        };
        let mut captured = 0usize;
        let mut captures = Vec::new();
        for &group in &c.pattern.external_groups {
            let range = c
                .found
                .captures
                .get(group)
                .and_then(|r| r.clone())
                .unwrap_or(c.found.end..c.found.end);
            captured = captured.saturating_add(range.len());
            if captured > budget.captured_bytes {
                return Err("external delimiter capture byte budget exceeded".into());
            }
            captures.push(
                input
                    .slice(range)
                    .map_err(|e| format!("capture boundary: {e:?}"))?,
            );
        }
        let compile = |p: &PatternTemplate, kind: &str| {
            let mut p = p.clone();
            if p.compiled.is_none() {
                let compiled = VimPattern::compile(
                    &expand_external(&p.source, &captures)?,
                    *ignore_case,
                    VimRegexLimits::default(),
                )?;
                // Captured delimiters can be empty or multibyte. Validate
                // offsets using the expanded pattern's real minimum extent.
                loader::validate_pattern_offsets(kind, p.offsets, compiled.minimum_chars)?;
                p.compiled = Some(compiled);
            }
            Ok::<_, String>(p)
        };
        let mut compiled_ends = Vec::new();
        let mut bytes = captured;
        for p in ends {
            let p = compile(p, "end")?;
            bytes =
                bytes.saturating_add(p.compiled.as_ref().unwrap().memory_usage() + p.source.len());
            if bytes > budget.retained_bytes {
                return Err("region state byte budget exceeded".into());
            }
            compiled_ends.push(p);
        }
        let skip = skip.as_ref().map(|p| compile(p, "skip")).transpose()?;
        let state = RegionState {
            rule,
            captures,
            ends: compiled_ends,
            skip,
        };
        if state.memory_usage() > budget.retained_bytes {
            return Err("region state byte budget exceeded".into());
        }
        Ok(Arc::new(state))
    }
    /// Vim only admits an oneline region when its end can be found on this
    /// actual hard line. The lookahead is private resumable work, not a token
    /// boundary, and its full line dependency participates in edit invalidation.
    fn check_oneline(
        &self,
        check: &mut OnelineCheck,
        input: &SyntaxInputSnapshot,
        fuel: &mut usize,
        budget: VimBudget,
        stats: &mut VimStats,
        cancel: &mut dyn FnMut() -> bool,
    ) -> Result<Option<bool>, String> {
        let state = check.candidate.region.as_ref().unwrap();
        let skip_count = usize::from(state.skip.is_some());
        loop {
            if cancel() {
                return Ok(None);
            }
            if check.position > check.line_end {
                return Ok(Some(false));
            }
            if *fuel == 0 {
                return Ok(None);
            }
            let template = if check.index < skip_count {
                state.skip.as_ref().unwrap()
            } else {
                let index = check.index - skip_count;
                if index >= state.ends.len() {
                    if check.position == check.line_end {
                        return Ok(Some(false));
                    }
                    *fuel -= 1;
                    check.position = next_char(input, check.position);
                    check.index = 0;
                    continue;
                }
                &state.ends[state.ends.len() - 1 - index]
            };
            let pattern = template.compiled.as_ref().unwrap();
            if check.active.is_none() {
                // Prepay the bounded backward scan across resumptions so a
                // one-instruction slice still advances through leading context.
                let pending = check.context_fuel.get_or_insert(template.offsets.lc);
                let charged = (*pending).min(*fuel);
                *pending -= charged;
                *fuel -= charged;
                if *pending > 0 {
                    return Ok(None);
                }
                if pattern.continuation_bytes() > budget.continuation_bytes {
                    return Err("regex continuation byte budget exceeded".into());
                }
                check.active = Some(pattern.start(leading_context_start(
                    input,
                    check.position,
                    template.offsets.lc,
                )));
                check.context_fuel = None;
            }
            let c = check.active.as_mut().unwrap();
            let before = c.inspected_end;
            let mut allowed = (*fuel).min(budget.match_instructions.saturating_sub(c.instructions));
            let initial = allowed;
            let progress = pattern.resume_with_control(c, input, &mut allowed, cancel);
            *fuel -= initial - allowed;
            stats.input_bytes += c.inspected_end.saturating_sub(before);
            if matches!(progress, VimRegexProgress::Pending)
                && c.instructions >= budget.match_instructions
            {
                return Err("pattern total instruction budget exceeded".into());
            }
            match progress {
                VimRegexProgress::Pending => return Ok(None),
                VimRegexProgress::Failed(message) => return Err(message),
                VimRegexProgress::Complete(found) => {
                    check.active = None;
                    if let Some(found) =
                        found.filter(|m| m.end >= check.position && m.start <= check.line_end)
                    {
                        if check.index >= skip_count {
                            return Ok(Some(true));
                        }
                        if found.end > check.position {
                            let next = offset(input, &found, template.offsets.me, true)?;
                            if next <= check.position {
                                return Err("nonadvancing skip offset".into());
                            }
                            check.position = next;
                            check.index = 0;
                            continue;
                        }
                    }
                    check.index += 1;
                }
            }
        }
    }
    fn accept(&self, j: &mut Job, c: Candidate, budget: VimBudget) -> Result<(), String> {
        match c.kind {
            ProbeKind::End { frame } => {
                let end = offset(&j.input, &c.found, c.offsets.me, true)?;
                let paintend =
                    offset(&j.input, &c.found, c.offsets.he.or(c.offsets.me), true)?.min(end);
                let region_end = c
                    .offsets
                    .re
                    .map_or(Ok(c.found.start), |o| {
                        offset(&j.input, &c.found, Some(o), true)
                    })?
                    .min(end);
                j.frames.truncate(frame + 1);
                let f = &mut j.frames[frame];
                f.closing = Some(region_end..end);
                f.end_group = c.matchgroup;
                f.paint.end = paintend;
                f.eol_extension =
                    c.pattern.eol && !c.excludenl && byte(&j.input, c.found.end) == Some(b'\n');
            }
            ProbeKind::Skip { frame } => {
                j.frames[frame].skip_until = offset(&j.input, &c.found, c.offsets.me, true)?;
            }
            ProbeKind::Start { rule, region } => {
                j.guard.push(rule);
                if j.frames.len() >= budget.stack_depth {
                    return Err("syntax region stack budget exceeded".into());
                }
                let o = c.offsets;
                let start = offset(&j.input, &c.found, o.ms, false)?;
                let end = if region {
                    c.found.end
                } else {
                    offset(&j.input, &c.found, o.me, true)?
                };
                let paintstart = offset(&j.input, &c.found, o.hs.or(o.ms), false)?.max(start);
                let paintend = offset(&j.input, &c.found, o.he.or(o.me), true)?.min(end);
                if end < start {
                    return Err("reversed offset match is outside native profile v1".into());
                }
                let region = if region {
                    Some(c.region.ok_or("missing prepared region state")?)
                } else {
                    None
                };
                let bytes = j
                    .frames
                    .iter()
                    .filter_map(|f| f.region.as_ref())
                    .map(|r| r.memory_usage())
                    .sum::<usize>()
                    + region.as_ref().map_or(0, |r| r.memory_usage());
                if bytes > budget.retained_bytes {
                    return Err("region stack byte budget exceeded".into());
                }
                let region_start = if region.is_some() {
                    o.rs.map_or(Ok(c.found.end), |o| {
                        offset(&j.input, &c.found, Some(o), false)
                    })?
                } else {
                    end
                };
                j.frames.push(Frame {
                    rule,
                    region: region.clone(),
                    start_end: region_start,
                    match_end: region.is_none().then_some(end),
                    paint: paintstart..if region.is_some() {
                        usize::MAX
                    } else {
                        paintend
                    },
                    closing: None,
                    skip_until: j.position,
                    search_start: end,
                    start_group: c.matchgroup,
                    end_group: None,
                    eol_extension: region.is_none()
                        && c.pattern.eol
                        && !c.excludenl
                        && byte(&j.input, c.found.end) == Some(b'\n'),
                });
                j.next = None;
            }
        }
        Ok(())
    }
    fn emit(&self, j: &mut Job, range: Range<usize>) {
        let range = range.start.max(j.request.start)..range.end.min(j.request.end);
        if range.is_empty() {
            return;
        }
        let Some((group, origin)) = j.frames.iter().rev().find_map(|f| {
            let r = &self.program.rules[f.rule];
            if range.start < f.paint.start || range.start >= f.paint.end {
                return None;
            }
            let delimiter = if range.start < f.start_end {
                f.start_group.as_deref()
            } else if f.closing.as_ref().is_some_and(|c| range.start >= c.start) {
                f.end_group.as_deref()
            } else {
                None
            };
            if r.options.transparent && delimiter.is_none() {
                return None;
            }
            Some((delimiter.unwrap_or(&r.group), &r.group))
        }) else {
            return;
        };
        let name = self.program.effective_group(group);
        if name == "NONE" {
            return;
        }
        if let Some(last) = j.runs.last_mut() {
            if last.range.end == range.start && last.name.0 == name && last.origin == *origin {
                last.range.end = range.end;
                return;
            }
        }
        j.run_bytes += std::mem::size_of::<SyntaxRun>() + name.len() + origin.len();
        j.runs.push(SyntaxRun {
            range,
            name: SyntaxStyleName(name.into()),
            origin: origin.clone(),
            priority: 0,
        });
    }
}
fn byte(input: &SyntaxInputSnapshot, at: usize) -> Option<u8> {
    input.chunk_at(at).first().copied()
}
fn next_char(input: &SyntaxInputSnapshot, at: usize) -> usize {
    let Some(b) = byte(input, at) else {
        return at;
    };
    at + if b < 128 {
        1
    } else if b < 224 {
        2
    } else if b < 240 {
        3
    } else {
        4
    }
}
fn leading_context_start(input: &SyntaxInputSnapshot, mut at: usize, count: usize) -> usize {
    // Legacy lc counts bytes when rewinding, then rounds back to a scalar
    // boundary. Its implicit ms=s+lc still uses character-based offsets.
    for _ in 0..count {
        if at == 0 || byte(input, at - 1) == Some(b'\n') {
            break;
        }
        at -= 1;
    }
    while at > 0 && byte(input, at).is_some_and(|b| b & 0xc0 == 0x80) {
        at -= 1;
    }
    at
}

fn offset(
    input: &SyntaxInputSnapshot,
    m: &VimRegexMatch,
    o: Option<Offset>,
    end: bool,
) -> Result<usize, String> {
    let Some(o) = o else {
        return Ok(if end { m.end } else { m.start });
    };
    let mut at = match o.base {
        OffsetBase::Start => m.start,
        OffsetBase::End => m.end,
    };
    let delta = o.delta
        + match (o.base, end) {
            (OffsetBase::Start, true) => 1,
            (OffsetBase::End, false) => -1,
            _ => 0,
        };
    if delta.unsigned_abs() > 1025 {
        return Err("offset work budget exceeded".into());
    }
    if delta >= 0 {
        for _ in 0..delta as usize {
            let next = next_char(input, at);
            if next == at {
                return Err("offset outside input".into());
            }
            at = next;
        }
    } else {
        for _ in 0..delta.unsigned_abs() {
            if at == 0 {
                return Err("offset before input".into());
            }
            at -= 1;
            while at > 0 && byte(input, at).is_some_and(|b| b & 0xc0 == 0x80) {
                at -= 1;
            }
        }
    }
    Ok(at)
}
fn vim_literal(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '.' | '*' | '[' | '\\' | '^' | '$' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn expand_external(pattern: &str, captures: &[String]) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'z') {
            chars.next();
            let next = chars.next().ok_or("unsupported external reference")?;
            if let Some(n) = next.to_digit(10).filter(|n| *n > 0) {
                out.push_str(&vim_literal(
                    captures
                        .get(n as usize - 1)
                        .ok_or("external reference has no matching start capture")?,
                ));
            } else {
                out.push_str("\\z");
                out.push(next);
            }
        } else {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod oneline_tests;
#[cfg(test)]
mod performance;
#[cfg(test)]
mod tests;
