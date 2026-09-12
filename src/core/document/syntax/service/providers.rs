//! Worker adapter: loading, parsing and querying never run in a UI turn.
use super::{SyntaxConfiguration, SyntaxProvider, SyntaxRequest, SyntaxResult};
use crate::document::syntax::{
    treesitter::{
        self, InjectionRegion, ParseOutcome, SyntaxInputEdit, TreeSitterBudget, TreeSitterSession,
    },
    vim::{VimBudget, VimDiagnostic, VimLoadLimits, VimProgram, VimSession, VimSetupContext},
    Coverage, SyntaxInputSnapshot, SyntaxRun,
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_PARSE_SLICES: usize = 4096;
const MAX_CHILDREN: usize = 16;
const MAX_CHILD_DEPTH: usize = 3;
const MAX_TOTAL_PARSE_PROGRESS: usize = 2_000_000;
const MAX_TOTAL_REPAIR_PROGRESS: usize = 4096;
const MAX_TOTAL_VIM_INSTRUCTIONS: usize = 64_000_000;
const MAX_TOTAL_CHILD_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHILD_RESULT_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub struct BackendProvider {
    configuration: Option<SyntaxConfiguration>,
    registry_generation: u64,
    primary: Option<TreeSitterSession>,
    primary_failure: Option<String>,
    fallback: Option<VimSession>,
    fallback_attempted: bool,
    fallback_failure: Vec<String>,
    fallback_input: Option<SyntaxInputSnapshot>,
    fallback_context: Option<VimSetupContext>,
    input: Option<SyntaxInputSnapshot>,
    parse_slices: usize,
    parse_progress: usize,
    primary_capped: bool,
    fallback_work: usize,
    capped_query: Option<CappedQuery>,
    capped_parse: Option<CappedQuery>,
    children: Vec<Child>,
}
struct CappedQuery {
    input: SyntaxInputSnapshot,
    range: Range<usize>,
    dependencies: Range<usize>,
    diagnostic: String,
}
impl CappedQuery {
    fn rebase(&mut self, next: &SyntaxInputSnapshot) -> bool {
        if self.input.identity() == next.identity() {
            return true;
        }
        if self.input.identity().document != next.identity().document
            || self.input.identity().generation != next.identity().generation
        {
            return false;
        }
        if let Some((old, new)) = self.input.text_tree().changed_extent(next.text_tree()) {
            // Boundary insertions can change the interpretation of a token.
            if old.start <= self.dependencies.end && old.end >= self.dependencies.start {
                return false;
            }
            if old.end < self.dependencies.start {
                let shift = |at: usize| {
                    if new.len() >= old.len() {
                        at.checked_add(new.len() - old.len())
                    } else {
                        at.checked_sub(old.len() - new.len())
                    }
                };
                let (Some(start), Some(end), Some(dependency_start), Some(dependency_end)) = (
                    shift(self.range.start),
                    shift(self.range.end),
                    shift(self.dependencies.start),
                    shift(self.dependencies.end),
                ) else {
                    return false;
                };
                self.range = start..end;
                self.dependencies = dependency_start..dependency_end;
            }
        }
        self.input = next.clone();
        true
    }
}
struct Child {
    language: String,
    ranges: Vec<Range<usize>>,
    session: Option<TreeSitterSession>,
    fallback: Option<VimSession>,
    fallback_attempted: bool,
    slices: usize,
    progress: usize,
    touched: bool,
    input: Option<SyntaxInputSnapshot>,
    failed: Option<String>,
    fallback_input: Option<SyntaxInputSnapshot>,
    fallback_context: Option<VimSetupContext>,
    fallback_work: usize,
    cache: Option<ChildOutput>,
}
#[derive(Clone)]
struct ChildOutput {
    range: Range<usize>,
    runs: Vec<SyntaxRun>,
    coverage: Coverage,
    diagnostics: Vec<String>,
    injections: Vec<InjectionRegion>,
    continuation: bool,
}
impl ChildOutput {
    fn missing(range: Range<usize>, message: impl Into<String>) -> Self {
        Self {
            range,
            runs: Vec::new(),
            coverage: Coverage::Missing,
            diagnostics: vec![message.into()],
            injections: Vec::new(),
            continuation: false,
        }
    }
}

impl BackendProvider {
    fn configure(&mut self, request: &SyntaxRequest) {
        let registry_generation = treesitter::package_registry_generation();
        if self.configuration.as_ref() == Some(&request.configuration)
            && self.registry_generation == registry_generation
        {
            return;
        }
        let same_language = self
            .configuration
            .as_ref()
            .is_some_and(|old| old.language == request.configuration.language);
        let only_vim_directory = same_language
            && self.registry_generation == registry_generation
            && self
                .configuration
                .as_ref()
                .is_some_and(|old| old.vim_directory != request.configuration.vim_directory);
        if same_language {
            self.fallback = None;
            self.fallback_attempted = false;
            self.fallback_failure.clear();
            self.fallback_input = None;
            self.fallback_work = 0;
            if !only_vim_directory {
                self.capped_query = None;
                for child in &mut self.children {
                    child.cache = None;
                    child.failed = None;
                    if child.language == "vim" {
                        child.session = None;
                        continue;
                    }
                    if let Ok(package) = treesitter::package_for_language(&child.language) {
                        if let Some(session) = &mut child.session {
                            let _ = session.replace_package(package);
                        } else {
                            child.session = TreeSitterSession::new(package).ok();
                        }
                    } else {
                        child.session = None;
                    }
                }
            }
            for child in &mut self.children {
                child.fallback = None;
                child.fallback_attempted = false;
                child.fallback_input = None;
                child.fallback_work = 0;
                if only_vim_directory {
                    child.cache = None;
                }
            }
        } else {
            *self = Self::default();
        }
        self.registry_generation = registry_generation;
        self.configuration = Some(request.configuration.clone());
        if only_vim_directory {
            return;
        }
        if let Some(language) = &request.configuration.language {
            // Vim-script highlighting is supplied by the configured Vim
            // runtime, including its syntax groups and highlight links.
            if language == "vim" {
                self.primary = None;
                self.primary_failure = None;
                return;
            }
            match treesitter::package_for_language(language) {
                Ok(package) => {
                    if let Some(session) = &mut self.primary {
                        match session.replace_package(package) {
                            Ok(true) => {}
                            Ok(false) => {
                                self.capped_parse = None;
                                self.primary_capped = false;
                                self.parse_progress = 0;
                                self.parse_slices = 0;
                            }
                            Err(error) => self.primary_failure = Some(error.to_string()),
                        }
                    } else {
                        self.primary = TreeSitterSession::new(package).ok();
                    }
                }
                Err(error) => {
                    self.primary = None;
                    self.primary_failure = Some(error.to_string());
                }
            }
        }
    }

    fn fallback(&mut self, request: &SyntaxRequest, cancelled: &AtomicBool) -> SyntaxResult {
        let mut result =
            SyntaxResult::missing(request, self.primary_failure.clone().unwrap_or_default());
        if cancelled.load(Ordering::Relaxed) {
            return result;
        }
        let Some(language) = &request.configuration.language else {
            result.diagnostics.clear();
            return result;
        };
        if !self.fallback_attempted {
            if !request.configuration.vim_directory.is_empty() {
                let loaded = VimProgram::load_directory_with_context(
                    &request.configuration.vim_directory,
                    vim_language(language),
                    VimLoadLimits::default(),
                    self.fallback_context.as_ref().expect("input setup context"),
                    cancelled,
                );
                if !self.finish_fallback_load(loaded, cancelled) {
                    return SyntaxResult::missing(request, "Syntax work cancelled");
                }
            } else {
                self.fallback_attempted = true;
            }
        }
        result.diagnostics.extend(self.fallback_failure.clone());
        if self.fallback_work >= MAX_TOTAL_VIM_INSTRUCTIONS {
            result
                .diagnostics
                .push("Vim repair exhausted the buffer work budget".into());
            return result;
        }
        if let Some(session) = &mut self.fallback {
            if let Some(old) = &self.fallback_input {
                if old.identity() != request.input.identity() {
                    let (old_range, new_range) = old
                        .text_tree()
                        .changed_extent(request.input.text_tree())
                        .unwrap_or((0..0, 0..0));
                    let _ = session.apply_edit(
                        old.identity(),
                        request.input.identity(),
                        old_range,
                        new_range.end,
                    );
                }
            }
            self.fallback_input = Some(request.input.clone());
            let fallback = session.highlight_with_control(
                &request.input,
                request.range.clone(),
                VimBudget::default(),
                &mut || cancelled.load(Ordering::Relaxed),
            );
            self.fallback_work = self
                .fallback_work
                .saturating_add(fallback.stats.instructions);
            result.diagnostics.extend(
                fallback
                    .diagnostics
                    .into_iter()
                    .map(|d| format!("{}:{}: {}", d.file, d.line, d.message)),
            );
            result.continuation =
                fallback.stats.yielded && self.fallback_work < MAX_TOTAL_VIM_INSTRUCTIONS;
            if fallback.covered.start <= request.range.start
                && fallback.covered.end >= request.range.end
            {
                result.coverage = fallback.coverage;
                result.runs = fallback.runs;
                self.fallback_work = 0;
            }
        }
        result.diagnostics.retain(|d| !d.is_empty());
        result
    }

    fn finish_fallback_load(
        &mut self,
        loaded: Result<std::sync::Arc<VimProgram>, Vec<VimDiagnostic>>,
        cancelled: &AtomicBool,
    ) -> bool {
        // Workers retain providers after request cancellation. A cancelled
        // compile is transient and must never suppress the next load attempt.
        if cancelled.load(Ordering::Relaxed) {
            self.fallback_attempted = false;
            self.fallback_failure.clear();
            return false;
        }
        self.fallback_attempted = true;
        self.fallback_failure.clear();
        match loaded {
            Ok(program) => self.fallback = Some(VimSession::new(program)),
            Err(errors) => {
                self.fallback_failure = errors
                    .into_iter()
                    .map(|d| format!("{}:{}: {}", d.file, d.line, d.message))
                    .collect();
            }
        }
        true
    }

    fn children(
        &mut self,
        request: &SyntaxRequest,
        regions: Vec<InjectionRegion>,
        result: &mut SyntaxResult,
        cancelled: &AtomicBool,
    ) {
        for child in &mut self.children {
            child.touched = false;
        }
        let mut queue = regions
            .into_iter()
            .map(|region| (region, 0))
            .collect::<Vec<_>>();
        let mut computed = false;
        let mut included_bytes = 0usize;
        let mut visited = Vec::new();
        while let Some((region, depth)) = queue.pop() {
            if cancelled.load(Ordering::Relaxed) {
                result.coverage = Coverage::Provisional;
                break;
            }
            if region.parent_input != request.input.identity()
                || !region
                    .ranges
                    .iter()
                    .any(|range| overlaps(range, &request.range))
            {
                continue;
            }
            if visited.len() >= TreeSitterBudget::default().max_injections {
                result.runs.clear();
                result
                    .diagnostics
                    .push("Aggregate injection discovery budget exceeded".into());
                result.coverage = Coverage::Provisional;
                break;
            }
            let key = (region.language.clone(), region.ranges.clone());
            if visited.contains(&key) {
                continue;
            }
            visited.push(key);
            // A declared child owns its region even while its providers are
            // unavailable. Missing child coverage therefore paints the default.
            for range in &region.ranges {
                let intersection =
                    range.start.max(request.range.start)..range.end.min(request.range.end);
                if !intersection.is_empty() {
                    replace_runs(&mut result.runs, intersection, Vec::new());
                }
            }
            included_bytes =
                included_bytes.saturating_add(region.ranges.iter().map(Range::len).sum::<usize>());
            if depth >= MAX_CHILD_DEPTH || included_bytes > MAX_TOTAL_CHILD_INPUT_BYTES {
                result
                    .diagnostics
                    .push("Embedded language input/nesting budget exceeded".into());
                result.coverage = Coverage::Provisional;
                continue;
            }
            let index = match self.children.iter().position(|child| {
                child.language == region.language && child.ranges == region.ranges
            }) {
                Some(index) => index,
                None => {
                    if self.children.len() >= MAX_CHILDREN {
                        self.children.retain(|child| child.touched);
                    }
                    if self.children.len() >= MAX_CHILDREN {
                        result
                            .diagnostics
                            .push("Embedded language session budget exceeded".into());
                        result.coverage = Coverage::Provisional;
                        continue;
                    }
                    let session = (region.language != "vim")
                        .then(|| {
                            treesitter::package_for_language(&region.language)
                                .and_then(TreeSitterSession::new)
                                .ok()
                        })
                        .flatten();
                    self.children.push(Child {
                        language: region.language.clone(),
                        ranges: region.ranges.clone(),
                        session,
                        fallback: None,
                        fallback_attempted: false,
                        slices: 0,
                        progress: 0,
                        touched: true,
                        input: None,
                        failed: None,
                        fallback_input: None,
                        fallback_context: None,
                        fallback_work: 0,
                        cache: None,
                    });
                    self.children.len() - 1
                }
            };
            let total_native = self
                .primary
                .as_ref()
                .map_or(0, |session| {
                    session.native_allocation_metrics().retained_bytes
                })
                .saturating_add(
                    self.children
                        .iter()
                        .filter_map(|child| child.session.as_ref())
                        .map(|session| session.native_allocation_metrics().retained_bytes)
                        .sum::<usize>(),
                );
            let own_native = self.children[index].session.as_ref().map_or(0, |session| {
                session.native_allocation_metrics().retained_bytes
            });
            let child = &mut self.children[index];
            child.touched = true;
            if child
                .input
                .as_ref()
                .is_none_or(|old| old.identity() != request.input.identity())
            {
                child.slices = 0;
                child.progress = 0;
                child.failed = None;
                child.fallback_work = 0;
                child.input = Some(request.input.clone());
                child.cache = None;
            }
            let cached = child
                .cache
                .as_ref()
                .filter(|cache| cache.range == request.range)
                .cloned();
            let output = if let Some(cached) = cached {
                cached
            } else {
                if computed {
                    result.continuation = true;
                    result.coverage = Coverage::Provisional;
                    continue;
                }
                computed = true;
                let budget = TreeSitterBudget {
                    max_native_bytes: TreeSitterBudget::default()
                        .max_native_bytes
                        .saturating_sub(total_native.saturating_sub(own_native)),
                    max_output_bytes: MAX_CHILD_RESULT_BYTES,
                    ..TreeSitterBudget::default()
                };
                let output = analyze_child(child, request, &budget, cancelled);
                if !output.continuation {
                    child.cache = Some(output.clone());
                }
                output
            };
            for range in &child.ranges {
                let intersection =
                    range.start.max(request.range.start)..range.end.min(request.range.end);
                if intersection.is_empty() {
                    continue;
                }
                let runs = clip_runs(&output.runs, &intersection);
                replace_runs(&mut result.runs, intersection, runs);
            }
            if output.coverage != Coverage::Exact {
                result.coverage = Coverage::Provisional;
            }
            result.continuation |= output.continuation;
            result.diagnostics.extend(output.diagnostics);
            queue.extend(
                output
                    .injections
                    .into_iter()
                    .map(|region| (region, depth + 1)),
            );
        }
        self.children.retain(|child| child.touched);
    }
}

impl SyntaxProvider for BackendProvider {
    fn analyze(&mut self, request: &SyntaxRequest, cancelled: &AtomicBool) -> SyntaxResult {
        if request.configuration.registry_generation != treesitter::package_registry_generation() {
            return SyntaxResult::missing(
                request,
                "Syntax package registry changed before execution",
            );
        }
        self.configure(request);
        if cancelled.load(Ordering::Relaxed) {
            return SyntaxResult::missing(request, "Syntax work cancelled");
        }
        if self
            .input
            .as_ref()
            .is_none_or(|old| old.identity() != request.input.identity())
        {
            let context = VimSetupContext::from_input(&request.input);
            if self.fallback_context.as_ref() != Some(&context) {
                self.fallback = None;
                self.fallback_attempted = false;
                self.fallback_failure.clear();
                self.fallback_input = None;
                self.fallback_work = 0;
                self.fallback_context = Some(context);
            }
            let preserve_cap = self
                .capped_parse
                .as_mut()
                .is_some_and(|capped| capped.rebase(&request.input));
            if !preserve_cap {
                self.parse_slices = 0;
                self.parse_progress = 0;
                self.capped_parse = None;
            }
            self.primary_capped = preserve_cap;
            self.fallback_work = 0;
            self.input = Some(request.input.clone());
            if let Some(capped) = &mut self.capped_query {
                if !capped.rebase(&request.input) {
                    self.capped_query = None;
                }
            }
        }
        let Some(primary) = &mut self.primary else {
            return self.fallback(request, cancelled);
        };
        if self.primary_capped {
            return self.fallback(request, cancelled);
        }
        if let Some(capped) = &self.capped_query {
            if capped.range == request.range {
                self.primary_failure = Some(capped.diagnostic.clone());
                return self.fallback(request, cancelled);
            }
        }
        let total_progress_limit = if primary.completed().is_some() {
            MAX_TOTAL_REPAIR_PROGRESS
        } else {
            MAX_TOTAL_PARSE_PROGRESS
        };
        let budget = TreeSitterBudget {
            max_progress_callbacks: TreeSitterBudget::default()
                .max_progress_callbacks
                .min(total_progress_limit.saturating_sub(self.parse_progress)),
            ..TreeSitterBudget::default()
        };
        let edits = edits_for(primary, &request.input);
        let repair_dependencies = edits
            .as_ref()
            .ok()
            .and_then(|edits| edits.first())
            .map_or(0..request.input.byte_len(), |edit| edit.new_range());
        let outcome = match edits {
            Ok(edits) => primary.parse(request.input.clone(), &edits, &[], &budget, cancelled),
            Err(error) => Err(error),
        };
        match outcome {
            Ok(ParseOutcome::Yielded { work }) => {
                self.parse_slices += 1;
                self.parse_progress = self.parse_progress.saturating_add(work.progress_callbacks);
                let exhausted = self.parse_slices >= MAX_PARSE_SLICES
                    || self.parse_progress >= total_progress_limit;
                if exhausted {
                    self.primary_capped = true;
                    if let Some(primary) = &mut self.primary {
                        primary.abandon();
                    }
                    self.primary_failure =
                        Some("Tree-sitter repair exhausted the buffer work budget".into());
                    self.capped_parse = Some(CappedQuery {
                        input: request.input.clone(),
                        range: request.range.clone(),
                        dependencies: repair_dependencies,
                        diagnostic: self.primary_failure.clone().unwrap(),
                    });
                }
                let mut fallback = self.fallback(request, cancelled);
                fallback.continuation |= !exhausted;
                fallback
            }
            Ok(ParseOutcome::Complete { snapshot, work }) => {
                self.parse_progress = self.parse_progress.saturating_add(work.progress_callbacks);
                let output = treesitter::highlight(
                    &snapshot,
                    request.range.clone(),
                    &TreeSitterBudget::default(),
                    cancelled,
                );
                if output.coverage != Coverage::Exact {
                    self.primary_failure = output.diagnostic.map(|d| d.to_string());
                    self.capped_query = Some(CappedQuery {
                        input: request.input.clone(),
                        range: request.range.clone(),
                        dependencies: output.failure_dependencies,
                        diagnostic: self
                            .primary_failure
                            .clone()
                            .unwrap_or_else(|| "Tree-sitter coverage unavailable".into()),
                    });
                    return self.fallback(request, cancelled);
                }
                let mut result = SyntaxResult {
                    input: request.input.identity(),
                    configuration: request.configuration.clone(),
                    range: output.range,
                    runs: output.runs,
                    coverage: output.coverage,
                    diagnostics: Vec::new(),
                    continuation: false,
                };
                self.children(request, output.injections, &mut result, cancelled);
                result
            }
            Err(error) => {
                self.primary_failure = Some(error.to_string());
                self.primary_capped = true;
                self.capped_parse = Some(CappedQuery {
                    input: request.input.clone(),
                    range: request.range.clone(),
                    dependencies: repair_dependencies,
                    diagnostic: self.primary_failure.clone().unwrap(),
                });
                self.fallback(request, cancelled)
            }
        }
    }
}

fn analyze_child(
    child: &mut Child,
    request: &SyntaxRequest,
    budget: &TreeSitterBudget,
    cancelled: &AtomicBool,
) -> ChildOutput {
    let mut continuation = false;
    if child.failed.is_none() {
        if let Some(session) = &mut child.session {
            let limit = if session.completed().is_some() {
                MAX_TOTAL_REPAIR_PROGRESS
            } else {
                MAX_TOTAL_PARSE_PROGRESS
            };
            let budget = TreeSitterBudget {
                max_progress_callbacks: budget
                    .max_progress_callbacks
                    .min(limit.saturating_sub(child.progress)),
                ..budget.clone()
            };
            let result = edits_for(session, &request.input).and_then(|edits| {
                session.parse(
                    request.input.clone(),
                    &edits,
                    &child.ranges,
                    &budget,
                    cancelled,
                )
            });
            child.slices += 1;
            match result {
                Ok(ParseOutcome::Complete { snapshot, work }) => {
                    child.progress = child.progress.saturating_add(work.progress_callbacks);
                    let output =
                        treesitter::highlight(&snapshot, request.range.clone(), &budget, cancelled);
                    if output.coverage == Coverage::Exact {
                        return ChildOutput {
                            range: request.range.clone(),
                            runs: output.runs,
                            coverage: Coverage::Exact,
                            injections: output.injections,
                            diagnostics: Vec::new(),
                            continuation: false,
                        };
                    }
                    child.failed = Some(output.diagnostic.map_or_else(
                        || "Embedded query unavailable".into(),
                        |error| error.to_string(),
                    ));
                }
                Ok(ParseOutcome::Yielded { work }) => {
                    child.progress = child.progress.saturating_add(work.progress_callbacks);
                    continuation = child.progress < limit && child.slices < MAX_PARSE_SLICES;
                    if !continuation {
                        child.failed = Some("Embedded parse repair budget exceeded".into());
                        session.abandon();
                    }
                }
                Err(error) => {
                    child.failed = Some(error.to_string());
                }
            }
        }
    }
    let mut result = ChildOutput::missing(
        request.range.clone(),
        child
            .failed
            .clone()
            .unwrap_or_else(|| format!("Embedded {} coverage unavailable", child.language)),
    );
    result.continuation = continuation;
    if cancelled.load(Ordering::Relaxed) {
        result.continuation = false;
        return result;
    }
    let bytes = child.ranges.iter().map(Range::len).sum::<usize>();
    if bytes > super::MAX_REGION_BYTES || child.fallback_work >= MAX_TOTAL_VIM_INSTRUCTIONS {
        result
            .diagnostics
            .push("Embedded Vim input/work budget exceeded".into());
        return result;
    }
    if child
        .fallback_input
        .as_ref()
        .is_none_or(|old| old.identity() != request.input.identity())
    {
        let mut text = String::with_capacity(bytes);
        for range in &child.ranges {
            text.push_str(
                &request
                    .input
                    .slice(range.clone())
                    .expect("validated injection"),
            );
        }
        let tree = crate::document::FormattedTextTree::try_from_text(text).expect("valid UTF-8");
        let input = SyntaxInputSnapshot::new(request.input.identity(), tree);
        let context = VimSetupContext::from_input(&input);
        if child.fallback_context.as_ref() != Some(&context) {
            child.fallback = None;
            child.fallback_attempted = false;
            child.fallback_work = 0;
            child.fallback_context = Some(context);
        } else if let (Some(old), Some(fallback)) = (&child.fallback_input, &mut child.fallback) {
            let (changed, replacement) = old
                .text_tree()
                .changed_extent(input.text_tree())
                .unwrap_or((0..0, 0..0));
            let _ = fallback.apply_edit(old.identity(), input.identity(), changed, replacement.end);
        }
        child.fallback_input = Some(input);
    }
    if !child.fallback_attempted {
        if !request.configuration.vim_directory.is_empty() {
            let loaded = VimProgram::load_directory_with_context(
                &request.configuration.vim_directory,
                vim_language(&child.language),
                VimLoadLimits::default(),
                child
                    .fallback_context
                    .as_ref()
                    .expect("embedded setup context"),
                cancelled,
            );
            if cancelled.load(Ordering::Relaxed) {
                result.continuation = false;
                return result;
            }
            match loaded {
                Ok(program) => child.fallback = Some(VimSession::new(program)),
                Err(errors) => result
                    .diagnostics
                    .extend(errors.into_iter().map(|diagnostic| diagnostic.message)),
            }
        }
        child.fallback_attempted = true;
    }
    let Some(fallback) = &mut child.fallback else {
        return result;
    };
    let input = child.fallback_input.as_ref().unwrap();
    let output =
        fallback.highlight_with_control(input, 0..bytes, VimBudget::default(), &mut || {
            cancelled.load(Ordering::Relaxed)
        });
    child.fallback_work = child
        .fallback_work
        .saturating_add(output.stats.instructions);
    result.continuation |= output.stats.yielded && child.fallback_work < MAX_TOTAL_VIM_INSTRUCTIONS;
    result.diagnostics.extend(
        output
            .diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message),
    );
    if output.covered.start != 0
        || output.covered.end != bytes
        || output.coverage == Coverage::Missing
    {
        return result;
    }
    let mut base = 0;
    for range in &child.ranges {
        let start = range.start.max(request.range.start);
        let end = range.end.min(request.range.end);
        if start < end {
            for run in &output.runs {
                let lower = run.range.start.max(base + start - range.start);
                let upper = run.range.end.min(base + end - range.start);
                if lower < upper {
                    let mut run = run.clone();
                    run.range = lower - base + range.start..upper - base + range.start;
                    result.runs.push(run);
                }
            }
        }
        base += range.len();
    }
    let bytes = result
        .runs
        .iter()
        .map(|run| std::mem::size_of::<SyntaxRun>() + run.name.0.len() + run.origin.len())
        .sum::<usize>();
    if bytes > MAX_CHILD_RESULT_BYTES {
        result.runs.clear();
        result
            .diagnostics
            .push("Embedded output budget exceeded".into());
    } else {
        result.coverage = output.coverage;
    }
    child.fallback_work = 0;
    result
}

fn clip_runs(runs: &[SyntaxRun], range: &Range<usize>) -> Vec<SyntaxRun> {
    runs.iter()
        .filter_map(|run| {
            let start = range.start.max(run.range.start);
            let end = range.end.min(run.range.end);
            if start >= end {
                return None;
            }
            let mut run = run.clone();
            run.range = start..end;
            Some(run)
        })
        .collect()
}

fn edits_for(
    session: &TreeSitterSession,
    input: &SyntaxInputSnapshot,
) -> Result<Vec<SyntaxInputEdit>, treesitter::TreeSitterError> {
    let Some(previous) = session.completed() else {
        return Ok(Vec::new());
    };
    if previous.input_identity() == input.identity() {
        return Ok(Vec::new());
    }
    let (old, new) = previous
        .input()
        .text_tree()
        .changed_extent(input.text_tree())
        .unwrap_or((0..0, 0..0));
    SyntaxInputEdit::new(previous.input(), input, old, new).map(|edit| vec![edit])
}
fn vim_language(language: &str) -> &str {
    match language {
        "c_sharp" => "cs",
        "tsx" | "typescriptreact" => "typescript",
        "javascriptreact" => "javascript",
        other => other,
    }
}
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
fn replace_runs(runs: &mut Vec<SyntaxRun>, coverage: Range<usize>, replacements: Vec<SyntaxRun>) {
    let mut retained = Vec::with_capacity(runs.len() + replacements.len());
    for run in runs.drain(..) {
        if !overlaps(&run.range, &coverage) {
            retained.push(run);
            continue;
        }
        if run.range.start < coverage.start {
            let mut left = run.clone();
            left.range.end = coverage.start;
            retained.push(left);
        }
        if run.range.end > coverage.end {
            let mut right = run;
            right.range.start = coverage.end;
            retained.push(right);
        }
    }
    retained.extend(replacements);
    retained.sort_by_key(|r| r.range.start);
    *runs = retained;
}

#[cfg(test)]
mod tests;
