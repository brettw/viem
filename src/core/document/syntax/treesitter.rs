//! Worker-owned Tree-sitter analysis and bounded, snapshot-bound queries.
//! Parsing covers a language region; querying a viewport never reparses.
//! Native cancellation is cooperative and cannot preempt external scanners.

use super::{Coverage, SyntaxInputIdentity, SyntaxInputSnapshot, SyntaxRun, SyntaxStyleName};
use regex_automata::dfa::{dense, Automaton};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tree_sitter::{
    InputEdit, Language, Node, ParseOptions, Parser, Point, Query, QueryCursor, QueryCursorOptions,
    QueryMatch, QueryPredicateArg, StreamingIterator, Tree,
};

mod native;
pub use native::{
    global_metrics as native_allocation_metrics, initialize as initialize_native_accounting,
    AllocationMetrics,
};

pub const BUNDLED_LANGUAGES: &[&str] = &[
    "c",
    "cpp",
    "rust",
    "swift",
    "objc",
    "c_sharp",
    "javascript",
    "typescript",
    "tsx",
    "python",
];

const MAX_REGISTERED_PACKAGES: usize = 64;
static PACKAGE_REGISTRY: OnceLock<RwLock<BTreeMap<String, Arc<TreeSitterPackage>>>> =
    OnceLock::new();
static PACKAGE_REGISTRY_GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
pub(crate) fn package_registry_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sampled by the service when requesting work, including without a text edit.
pub fn package_registry_generation() -> u64 {
    PACKAGE_REGISTRY_GENERATION.load(Ordering::Acquire)
}

/// A platform loader resolves native libraries and query composition first.
/// Only a fully validated, immutable package is installed here. Retired native
/// libraries stay alive until every outstanding parser and snapshot is dropped.
pub fn register_package(
    language: &str,
    package: Arc<TreeSitterPackage>,
) -> Result<u64, TreeSitterError> {
    if language.is_empty()
        || language.len() > 256
        || !language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
    {
        return Err(TreeSitterError::InvalidQuery(
            "invalid registry language".into(),
        ));
    }
    let mut registry = PACKAGE_REGISTRY
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|e| e.into_inner());
    if !registry.contains_key(language) && registry.len() >= MAX_REGISTERED_PACKAGES {
        return Err(TreeSitterError::Limit("registered packages"));
    }
    registry.insert(language.to_owned(), package);
    Ok(PACKAGE_REGISTRY_GENERATION.fetch_add(1, Ordering::AcqRel) + 1)
}

pub fn unregister_package(language: &str) -> bool {
    let mut registry = PACKAGE_REGISTRY
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|e| e.into_inner());
    if registry.remove(language).is_none() {
        return false;
    }
    PACKAGE_REGISTRY_GENERATION.fetch_add(1, Ordering::AcqRel);
    true
}

pub fn package_for_language(language: &str) -> Result<Arc<TreeSitterPackage>, TreeSitterError> {
    let registered = PACKAGE_REGISTRY
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(language)
        .cloned();
    registered.map_or_else(|| TreeSitterPackage::bundled(language), Ok)
}

/// Input/capture/output limits are hard; the native tree-node limit is a
/// structural soft limit checked before retaining a completed tree.
#[derive(Clone, Debug)]
pub struct TreeSitterBudget {
    pub max_input_bytes: usize,
    pub max_query_source_bytes: usize,
    pub max_progress_callbacks: usize,
    pub max_input_bytes_supplied: usize,
    pub max_matches: usize,
    pub max_captures: usize,
    pub max_predicate_bytes: usize,
    pub max_predicate_steps: usize,
    pub max_pending_matches: u32,
    pub max_injections: usize,
    pub max_tree_nodes: usize,
    pub max_native_bytes: usize,
    pub max_output_bytes: usize,
    pub max_edit_tree_nodes: usize,
    pub max_edit_root_children: usize,
    pub slice_duration: Duration,
}

impl Default for TreeSitterBudget {
    fn default() -> Self {
        Self {
            max_input_bytes: 512 * 1024 * 1024,
            max_query_source_bytes: 2 * 1024 * 1024,
            max_progress_callbacks: 16_384,
            max_input_bytes_supplied: 32 * 1024 * 1024,
            max_matches: 65_536,
            max_captures: 131_072,
            max_predicate_bytes: 1024 * 1024,
            max_predicate_steps: 2_000_000,
            max_pending_matches: 4096,
            max_injections: 128,
            max_tree_nodes: 16_000_000,
            max_native_bytes: 256 * 1024 * 1024,
            max_output_bytes: 4 * 1024 * 1024,
            max_edit_tree_nodes: 500_000,
            max_edit_root_children: 16_384,
            slice_duration: Duration::from_millis(4),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TreeSitterError {
    UnsupportedLanguage(String),
    UnsupportedQuery(String),
    InvalidQuery(String),
    IncompatibleGrammar,
    InvalidInput,
    InvalidEdit,
    StaleInput,
    Cancelled,
    Limit(&'static str),
}

impl std::fmt::Display for TreeSitterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tree-sitter: {self:?}")
    }
}
impl std::error::Error for TreeSitterError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryProfile {
    Upstream,
    NeovimV1,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TreeSitterWork {
    pub input_callbacks: usize,
    pub input_bytes_supplied: usize,
    pub maximum_input_chunk: usize,
    pub progress_callbacks: usize,
    pub query_matches: usize,
    pub query_captures: usize,
    pub predicate_bytes_copied: usize,
    pub predicate_steps: usize,
    pub incremental: bool,
    pub tree_nodes: usize,
    pub native_retained_bytes: usize,
    pub native_peak_bytes: usize,
    pub output_bytes: usize,
}

/// A checked single splice between adjacent normalized input snapshots.
/// A batch is an ordered chain, not several offsets in one old revision.
#[derive(Clone, Debug)]
pub struct SyntaxInputEdit {
    before: SyntaxInputIdentity,
    after: SyntaxInputIdentity,
    edit: InputEdit,
}

impl SyntaxInputEdit {
    pub fn new(
        before: &SyntaxInputSnapshot,
        after: &SyntaxInputSnapshot,
        old: Range<usize>,
        new: Range<usize>,
    ) -> Result<Self, TreeSitterError> {
        if before.identity().document != after.identity().document
            || before.identity().generation != after.identity().generation
            || before.identity() == after.identity()
            || old.start != new.start
            || old.start > old.end
            || new.start > new.end
            || before
                .byte_len()
                .checked_sub(old.len())
                .and_then(|n| n.checked_add(new.len()))
                != Some(after.byte_len())
        {
            return Err(TreeSitterError::InvalidEdit);
        }
        Ok(Self {
            before: before.identity(),
            after: after.identity(),
            edit: InputEdit {
                start_byte: old.start,
                old_end_byte: old.end,
                new_end_byte: new.end,
                start_position: point(before, old.start)?,
                old_end_position: point(before, old.end)?,
                new_end_position: point(after, new.end)?,
            },
        })
    }
    pub fn old_range(&self) -> Range<usize> {
        self.edit.start_byte..self.edit.old_end_byte
    }
    pub fn new_range(&self) -> Range<usize> {
        self.edit.start_byte..self.edit.new_end_byte
    }
}

fn point(input: &SyntaxInputSnapshot, offset: usize) -> Result<Point, TreeSitterError> {
    check_coordinate_size(input.byte_len())?;
    if offset > input.byte_len()
        || !input
            .text_tree()
            .is_char_boundary(offset)
            .map_err(|_| TreeSitterError::InvalidInput)?
    {
        return Err(TreeSitterError::InvalidInput);
    }
    let row = input
        .text_tree()
        .hard_line_at_byte(offset)
        .map_err(|_| TreeSitterError::InvalidInput)?;
    let start = input
        .text_tree()
        .hard_line_start(row)
        .map_err(|_| TreeSitterError::InvalidInput)?;
    if row > u32::MAX as usize || offset - start > u32::MAX as usize {
        return Err(TreeSitterError::Limit("32-bit coordinates"));
    }
    Ok(Point::new(row, offset - start))
}

pub fn check_coordinate_size(bytes: usize) -> Result<(), TreeSitterError> {
    if bytes > u32::MAX as usize {
        Err(TreeSitterError::Limit("32-bit coordinates"))
    } else {
        Ok(())
    }
}

#[derive(Debug)]
enum Predicate {
    Ancestor {
        capture: u32,
        kinds: Vec<String>,
        positive: bool,
        direct: bool,
    },
    Equal {
        left: u32,
        right: Argument,
        positive: bool,
        any: bool,
    },
    Membership {
        capture: u32,
        values: Vec<String>,
        positive: bool,
    },
    Match {
        capture: u32,
        matcher: Matcher,
        positive: bool,
        any: bool,
    },
}
#[derive(Debug)]
enum Matcher {
    Regular(dense::DFA<Vec<u32>>),
    Vim(super::vim::VimPattern),
}
#[derive(Clone, Debug)]
enum Argument {
    Capture(u32),
    String(String),
}
#[derive(Clone, Debug)]
struct Offset {
    capture: u32,
    start_row: i32,
    start_column: i32,
    end_row: i32,
    end_column: i32,
}
#[derive(Default, Debug)]
struct Pattern {
    predicates: Vec<Predicate>,
    offsets: Vec<Offset>,
    priority: i32,
    injection_language: Option<String>,
    combined: bool,
    include_children: bool,
}
struct CompiledQuery {
    query: Query,
    patterns: Vec<Pattern>,
}

/// Dynamic loaders retain their library owner for every parser/tree/query use.
pub struct TreeSitterPackage {
    id: String,
    generation: u64,
    language: Language,
    highlights: CompiledQuery,
    injections: Option<CompiledQuery>,
    native: Arc<native::Account>,
    _resource_owner: Option<Arc<dyn Send + Sync>>,
}
impl std::fmt::Debug for TreeSitterPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSitterPackage")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .finish()
    }
}

impl TreeSitterPackage {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Names are returned without the style namespace's leading @. This is
    /// introspection of a package already compiled by its worker/loader.
    pub fn highlight_capture_names(&self) -> &[&str] {
        self.highlights.query.capture_names()
    }
    pub fn native_allocation_metrics(&self) -> AllocationMetrics {
        self.native.metrics()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn compile(
        id: impl Into<String>,
        generation: u64,
        language: Language,
        highlight_source: &str,
        injection_source: Option<&str>,
        profile: QueryProfile,
        budget: &TreeSitterBudget,
        resource_owner: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Arc<Self>, TreeSitterError> {
        let id = id.into();
        if id.is_empty() || id.len() > 256 {
            return Err(TreeSitterError::InvalidQuery(
                "invalid package identity".into(),
            ));
        }
        let native = Arc::new(native::Account::default());
        let _scope = native::Scope::new(&native);
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|_| TreeSitterError::IncompatibleGrammar)?;
        let package = Arc::new(Self {
            id,
            generation,
            highlights: compile_query(&language, highlight_source, profile, false, budget)?,
            injections: injection_source
                .filter(|s| !s.trim().is_empty())
                .map(|s| compile_query(&language, s, profile, true, budget))
                .transpose()?,
            language,
            native: native.clone(),
            _resource_owner: resource_owner,
        });
        if native.exceeded(budget.max_native_bytes) {
            return Err(TreeSitterError::Limit("native allocation soft limit"));
        }
        Ok(package)
    }

    pub fn bundled(id: &str) -> Result<Arc<Self>, TreeSitterError> {
        let javascript = include_str!("treesitter/javascript.scm");
        let (language, highlights, injections) = match id {
            "c" => (
                tree_sitter_c::LANGUAGE.into(),
                tree_sitter_c::HIGHLIGHT_QUERY.to_owned(),
                None,
            ),
            "cpp" => (
                tree_sitter_cpp::LANGUAGE.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    tree_sitter_cpp::HIGHLIGHT_QUERY
                ),
                None,
            ),
            "rust" => (
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
                Some(tree_sitter_rust::INJECTIONS_QUERY),
            ),
            "swift" => (
                tree_sitter_swift::LANGUAGE.into(),
                tree_sitter_swift::HIGHLIGHTS_QUERY.to_owned(),
                Some(tree_sitter_swift::INJECTIONS_QUERY),
            ),
            "objc" => (
                tree_sitter_objc::LANGUAGE.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    tree_sitter_objc::HIGHLIGHTS_QUERY.replace("; inherits: c", "")
                ),
                None,
            ),
            "c_sharp" | "cs" => (
                tree_sitter_c_sharp::LANGUAGE.into(),
                include_str!("treesitter/c_sharp.scm").to_owned(),
                None,
            ),
            "javascript" | "javascriptreact" => (
                tree_sitter_javascript::LANGUAGE.into(),
                format!(
                    "{javascript}\n{}",
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
                Some(include_str!("treesitter/javascript_injections.scm")),
            ),
            "typescript" => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                format!("{javascript}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY),
                None,
            ),
            "tsx" | "typescriptreact" => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                format!(
                    "{javascript}\n{}\n{}",
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY
                ),
                Some(include_str!("treesitter/javascript_injections.scm")),
            ),
            "python" => (
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
                None,
            ),
            _ => return Err(TreeSitterError::UnsupportedLanguage(id.to_owned())),
        };
        Self::compile(
            id,
            1,
            language,
            &highlights,
            injections,
            QueryProfile::Upstream,
            &TreeSitterBudget::default(),
            None,
        )
    }
}

/// Operator tokens only, not strings/comments. All predicates run through
/// budgeted host handlers instead of the binding's unbounded text collection.
fn rewrite_predicates(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let (mut quoted, mut escaped, mut comment) = (false, false, false);
    for ch in source.chars() {
        if comment {
            result.push(ch);
            if ch == '\n' {
                comment = false;
            }
        } else if quoted {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
        } else {
            if ch == ';' {
                comment = true;
            }
            if ch == '"' {
                quoted = true;
            }
            result.push(ch);
            if ch == '#' {
                result.push_str("viem-");
            }
        }
    }
    result
}

fn compile_query(
    language: &Language,
    source: &str,
    profile: QueryProfile,
    injection: bool,
    budget: &TreeSitterBudget,
) -> Result<CompiledQuery, TreeSitterError> {
    if source.len() > budget.max_query_source_bytes {
        return Err(TreeSitterError::Limit("query source bytes"));
    }
    for line in source.lines() {
        let trimmed = line.trim_start_matches(|c: char| c == ';' || c.is_whitespace());
        if trimmed.starts_with("inherits:") || trimmed == "extends" {
            return Err(TreeSitterError::UnsupportedQuery(
                "unresolved query inheritance/extension".into(),
            ));
        }
    }
    let rewritten = rewrite_predicates(source);
    let query = Query::new(language, &rewritten)
        .map_err(|e| TreeSitterError::InvalidQuery(e.to_string()))?;
    let mut patterns = Vec::with_capacity(query.pattern_count());
    for index in 0..query.pattern_count() {
        let mut pattern = Pattern {
            priority: 100,
            ..Pattern::default()
        };
        for predicate in query.general_predicates(index) {
            let op = predicate
                .operator
                .strip_prefix("viem-")
                .unwrap_or(&predicate.operator);
            let args = &predicate.args;
            if profile == QueryProfile::NeovimV1 {
                if op.starts_with("any-not-") {
                    return Err(TreeSitterError::UnsupportedQuery(format!(
                        "Neovim predicate {op}"
                    )));
                }
                for arg in args {
                    if let QueryPredicateArg::Capture(id) = arg {
                        if query.capture_quantifiers(index)[*id as usize]
                            != tree_sitter::CaptureQuantifier::One
                        {
                            return Err(TreeSitterError::UnsupportedQuery(
                                "Neovim quantified predicate/directive capture".into(),
                            ));
                        }
                    }
                }
            }
            let capture = |arg: Option<&QueryPredicateArg>| match arg {
                Some(QueryPredicateArg::Capture(value)) => Ok(*value),
                _ => Err(TreeSitterError::InvalidQuery(format!(
                    "{op} requires a capture"
                ))),
            };
            let string = |arg: Option<&QueryPredicateArg>| match arg {
                Some(QueryPredicateArg::String(value)) => Ok(value.to_string()),
                _ => Err(TreeSitterError::InvalidQuery(format!(
                    "{op} requires a string"
                ))),
            };
            match op {
                "has-ancestor?" | "not-has-ancestor?" | "has-parent?" | "not-has-parent?"
                    if args.len() >= 2 =>
                {
                    pattern.predicates.push(Predicate::Ancestor {
                        capture: capture(args.first())?,
                        kinds: args[1..]
                            .iter()
                            .map(|arg| string(Some(arg)))
                            .collect::<Result<_, _>>()?,
                        positive: !op.starts_with("not-"),
                        direct: op.ends_with("parent?"),
                    });
                }
                "eq?" | "not-eq?" | "any-eq?" | "any-not-eq?" if args.len() == 2 => {
                    let right = match &args[1] {
                        QueryPredicateArg::Capture(value) => Argument::Capture(*value),
                        QueryPredicateArg::String(value) => Argument::String(value.to_string()),
                    };
                    pattern.predicates.push(Predicate::Equal {
                        left: capture(args.first())?,
                        right,
                        positive: !op.contains("not-"),
                        any: op.starts_with("any-"),
                    });
                }
                "any-of?" | "not-any-of?" if args.len() >= 2 => {
                    pattern.predicates.push(Predicate::Membership {
                        capture: capture(args.first())?,
                        values: args[1..]
                            .iter()
                            .map(|arg| string(Some(arg)))
                            .collect::<Result<_, _>>()?,
                        positive: op == "any-of?",
                    });
                }
                "match?" | "not-match?" | "any-match?" | "any-not-match?" if args.len() == 2 => {
                    let source = string(args.get(1))?;
                    let matcher = match profile {
                        QueryProfile::Upstream => Matcher::Regular(
                            dense::Builder::new()
                                .configure(
                                    dense::Config::new()
                                        .dfa_size_limit(Some(budget.max_query_source_bytes))
                                        .determinize_size_limit(Some(
                                            budget.max_query_source_bytes,
                                        )),
                                )
                                .thompson(
                                    regex_automata::nfa::thompson::Config::new()
                                        .nfa_size_limit(Some(budget.max_query_source_bytes)),
                                )
                                .build(&source)
                                .map_err(|e| {
                                    TreeSitterError::UnsupportedQuery(format!(
                                        "bounded upstream regex: {e}"
                                    ))
                                })?,
                        ),
                        QueryProfile::NeovimV1 => Matcher::Vim(
                            super::vim::VimPattern::compile_neovim_query(
                                &source,
                                super::vim::VimRegexLimits::default(),
                            )
                            .map_err(TreeSitterError::UnsupportedQuery)?,
                        ),
                    };
                    pattern.predicates.push(Predicate::Match {
                        capture: capture(args.first())?,
                        matcher,
                        positive: !op.contains("not-"),
                        any: op.starts_with("any-"),
                    });
                }
                "offset!" if args.len() == 5 => {
                    let number = |at| {
                        string(args.get(at))?.parse::<i32>().map_err(|_| {
                            TreeSitterError::InvalidQuery("offset! requires signed integers".into())
                        })
                    };
                    pattern.offsets.push(Offset {
                        capture: capture(args.first())?,
                        start_row: number(1)?,
                        start_column: number(2)?,
                        end_row: number(3)?,
                        end_column: number(4)?,
                    });
                }
                "set!" => {
                    let key = string(args.first())?;
                    let value = args.get(1).map(|v| string(Some(v))).transpose()?;
                    if args.len() > 2 {
                        return Err(TreeSitterError::UnsupportedQuery(
                            "capture-specific property metadata".into(),
                        ));
                    }
                    match key.as_str() {
                        "priority" => {
                            pattern.priority = value
                                .ok_or_else(|| {
                                    TreeSitterError::InvalidQuery("missing priority".into())
                                })?
                                .parse()
                                .map_err(|_| {
                                    TreeSitterError::InvalidQuery("invalid priority".into())
                                })?
                        }
                        "injection.language" if injection => pattern.injection_language = value,
                        "injection.combined" if injection => pattern.combined = true,
                        "injection.include-children" if injection => {
                            pattern.include_children = true
                        }
                        _ => {
                            return Err(TreeSitterError::UnsupportedQuery(format!(
                                "property {key}"
                            )))
                        }
                    }
                }
                _ => {
                    return Err(TreeSitterError::UnsupportedQuery(format!(
                        "predicate/directive {op}"
                    )))
                }
            }
        }
        patterns.push(pattern);
    }
    Ok(CompiledQuery { query, patterns })
}

pub struct ParseSnapshot {
    input: SyntaxInputSnapshot,
    tree: Tree,
    included_ranges: Vec<Range<usize>>,
    changed_ranges: Vec<Range<usize>>,
    native: Arc<native::Account>,
    _single_reader: std::marker::PhantomData<std::cell::Cell<()>>,
    // Rust drops fields in declaration order: keep the native library owner
    // until every native tree handle has been destroyed.
    package: Arc<TreeSitterPackage>,
}
impl Clone for ParseSnapshot {
    fn clone(&self) -> Self {
        let _scope = native::Scope::new(&self.native);
        Self {
            input: self.input.clone(),
            package: self.package.clone(),
            tree: self.tree.clone(),
            included_ranges: self.included_ranges.clone(),
            changed_ranges: self.changed_ranges.clone(),
            native: self.native.clone(),
            _single_reader: std::marker::PhantomData,
        }
    }
}
impl std::fmt::Debug for ParseSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseSnapshot")
            .field("input", &self.input.identity())
            .field("package", &self.package.id)
            .finish()
    }
}
impl ParseSnapshot {
    pub fn input_identity(&self) -> SyntaxInputIdentity {
        self.input.identity()
    }
    pub fn changed_ranges(&self) -> &[Range<usize>] {
        &self.changed_ranges
    }
    pub fn has_errors(&self) -> bool {
        self.tree.root_node().has_error()
    }
    pub fn node_count(&self) -> usize {
        self.tree.root_node().descendant_count()
    }
    pub fn input(&self) -> &SyntaxInputSnapshot {
        &self.input
    }
    pub fn native_allocation_metrics(&self) -> AllocationMetrics {
        self.native.metrics()
    }
}

struct PendingParse {
    input: SyntaxInputSnapshot,
    old_tree: Option<Tree>,
    included_ranges: Vec<Range<usize>>,
    changes: Vec<Range<usize>>,
}
pub struct TreeSitterSession {
    parser: Parser,
    complete: Option<ParseSnapshot>,
    pending: Option<PendingParse>,
    native: Arc<native::Account>,
    // In particular, an interrupted cold parse has no completed snapshot to
    // retain this owner while the parser calls its scanner destructor.
    package: Arc<TreeSitterPackage>,
}
#[derive(Debug)]
pub enum ParseOutcome {
    Complete {
        snapshot: ParseSnapshot,
        work: TreeSitterWork,
    },
    Yielded {
        work: TreeSitterWork,
    },
}

impl TreeSitterSession {
    pub fn new(package: Arc<TreeSitterPackage>) -> Result<Self, TreeSitterError> {
        let native = Arc::new(native::Account::default());
        let _scope = native::Scope::new(&native);
        let mut parser = Parser::new();
        parser
            .set_language(&package.language)
            .map_err(|_| TreeSitterError::IncompatibleGrammar)?;
        Ok(Self {
            package,
            parser,
            complete: None,
            pending: None,
            native,
        })
    }
    pub fn completed(&self) -> Option<&ParseSnapshot> {
        self.complete.as_ref()
    }
    /// Query/package changes preserve parser state when the native grammar
    /// handle is identical. Published snapshots keep their original package.
    pub fn replace_package(
        &mut self,
        package: Arc<TreeSitterPackage>,
    ) -> Result<bool, TreeSitterError> {
        if self.package.language != package.language {
            *self = Self::new(package)?;
            return Ok(false);
        }
        self.package = package.clone();
        if let Some(snapshot) = &mut self.complete {
            snapshot.package = package;
        }
        Ok(true)
    }
    pub fn native_allocation_metrics(&self) -> AllocationMetrics {
        self.native.metrics()
    }
    /// Reset abandoned native continuations before changing their input.
    pub fn abandon(&mut self) {
        self.parser.reset();
        self.pending = None;
    }
    pub fn discard(&mut self) {
        let _scope = native::Scope::new(&self.native);
        self.pending = None;
        self.complete = None;
        self.parser = Parser::new();
        self.parser
            .set_language(&self.package.language)
            .expect("validated package");
    }

    pub fn parse(
        &mut self,
        input: SyntaxInputSnapshot,
        edits: &[SyntaxInputEdit],
        included_ranges: &[Range<usize>],
        budget: &TreeSitterBudget,
        cancelled: &AtomicBool,
    ) -> Result<ParseOutcome, TreeSitterError> {
        let _scope = native::Scope::new(&self.native);
        check_coordinate_size(input.byte_len())?;
        if input.byte_len() > budget.max_input_bytes {
            return Err(TreeSitterError::Limit("input bytes"));
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(TreeSitterError::Cancelled);
        }
        if self.native.exceeded(budget.max_native_bytes) {
            self.discard();
            return Err(TreeSitterError::Limit("native allocation soft limit"));
        }
        if let Some(previous) = self.complete.as_ref().filter(|p| {
            p.input.identity() == input.identity() && p.included_ranges == included_ranges
        }) {
            if !previous
                .input
                .text_tree()
                .shares_root_with(input.text_tree())
            {
                return Err(TreeSitterError::StaleInput);
            }
            return Ok(ParseOutcome::Complete {
                snapshot: previous.clone(),
                work: TreeSitterWork::default(),
            });
        }
        let resume = self.pending.as_ref().is_some_and(|p| {
            p.input.identity() == input.identity()
                && p.included_ranges == included_ranges
                && p.input.text_tree().shares_root_with(input.text_tree())
        });
        if !resume {
            if !edits.is_empty()
                && self.complete.as_ref().is_some_and(|previous| {
                    let root = previous.tree.root_node();
                    root.descendant_count() > budget.max_edit_tree_nodes
                        || root.child_count() > budget.max_edit_root_children
                })
            {
                self.abandon();
                return Err(TreeSitterError::Limit("native edit preflight budget"));
            }
            self.abandon();
            let mut ranges = Vec::with_capacity(included_ranges.len());
            let mut previous_end = 0;
            for range in included_ranges {
                if range.start >= range.end || range.start < previous_end {
                    return Err(TreeSitterError::InvalidInput);
                }
                ranges.push(tree_sitter::Range {
                    start_byte: range.start,
                    end_byte: range.end,
                    start_point: point(&input, range.start)?,
                    end_point: point(&input, range.end)?,
                });
                previous_end = range.end;
            }
            self.parser
                .set_included_ranges(&ranges)
                .map_err(|_| TreeSitterError::InvalidInput)?;
            let mut old_tree = None;
            let mut changes = Vec::new();
            if let Some(previous) = &self.complete {
                if !edits.is_empty() {
                    let mut identity = previous.input.identity();
                    let mut tree = previous.tree.clone();
                    for edit in edits {
                        if edit.before != identity {
                            return Err(TreeSitterError::InvalidEdit);
                        }
                        tree.edit(&edit.edit);
                        identity = edit.after;
                    }
                    if identity != input.identity() {
                        return Err(TreeSitterError::InvalidEdit);
                    }
                    // Intermediate offsets are not retained as current ranges.
                    // A conservative dirty suffix avoids shifting every range.
                    let start = edits.iter().map(|e| e.edit.start_byte).min().unwrap();
                    changes.push(start..input.byte_len());
                    old_tree = Some(tree);
                }
            }
            self.pending = Some(PendingParse {
                input,
                old_tree,
                included_ranges: included_ranges.to_vec(),
                changes,
            });
        }
        let pending = self.pending.as_ref().expect("captured parse");
        let control = Control::new(budget, cancelled, &self.native);
        let mut progress = |_: &tree_sitter::ParseState| control.progress();
        let mut read = |byte: usize, _: Point| {
            let bytes = pending.input.chunk_at(byte);
            if control.read(bytes.len()) {
                bytes
            } else {
                &[]
            }
        };
        let parsed = self.parser.parse_with_options(
            &mut read,
            pending.old_tree.as_ref(),
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        if self.native.exceeded(budget.max_native_bytes) {
            control.fail(TreeSitterError::Limit("native allocation soft limit"));
        }
        let mut work = control.work();
        work.incremental = pending.old_tree.is_some();
        if let Some(error) = control.failure() {
            if error == TreeSitterError::Cancelled {
                self.abandon();
                return Err(error);
            }
            if error == TreeSitterError::Limit("native allocation soft limit") {
                drop(parsed);
                self.discard();
                return Err(error);
            }
            if error == TreeSitterError::Limit("input bytes supplied") {
                self.abandon();
                return Err(error);
            }
            if parsed.is_none() {
                return Ok(ParseOutcome::Yielded { work });
            }
            self.abandon();
            return Err(error);
        }
        let Some(tree) = parsed else {
            return Ok(ParseOutcome::Yielded { work });
        };
        work.tree_nodes = tree.root_node().descendant_count();
        if work.tree_nodes > budget.max_tree_nodes {
            self.abandon();
            return Err(TreeSitterError::Limit("native tree node soft limit"));
        }
        let pending = self.pending.take().expect("captured parse");
        let mut changes = pending.changes;
        if let Some(old) = &pending.old_tree {
            if let Some(first) = old.changed_ranges(&tree).next() {
                let start = changes
                    .first()
                    .map_or(first.start_byte, |r| r.start.min(first.start_byte));
                changes = vec![start..pending.input.byte_len()];
            }
        } else {
            changes = vec![0..pending.input.byte_len()];
        }
        let snapshot = ParseSnapshot {
            input: pending.input,
            package: self.package.clone(),
            tree,
            included_ranges: pending.included_ranges,
            changed_ranges: changes,
            native: self.native.clone(),
            _single_reader: std::marker::PhantomData,
        };
        self.complete = Some(snapshot.clone());
        Ok(ParseOutcome::Complete { snapshot, work })
    }
}

struct Control<'a> {
    budget: &'a TreeSitterBudget,
    cancelled: &'a AtomicBool,
    started: Instant,
    statistics: RefCell<TreeSitterWork>,
    failure: RefCell<Option<TreeSitterError>>,
    native: &'a native::Account,
    dependencies: RefCell<Option<Range<usize>>>,
}
impl<'a> Control<'a> {
    fn new(
        budget: &'a TreeSitterBudget,
        cancelled: &'a AtomicBool,
        native: &'a native::Account,
    ) -> Self {
        Self {
            budget,
            cancelled,
            started: Instant::now(),
            statistics: RefCell::default(),
            failure: RefCell::default(),
            native,
            dependencies: RefCell::default(),
        }
    }
    fn fail(&self, error: TreeSitterError) -> bool {
        if self.failure.borrow().is_none() {
            *self.failure.borrow_mut() = Some(error);
        }
        true
    }
    fn check(&self) -> bool {
        if self.failure.borrow().is_some() {
            true
        } else if self.cancelled.load(Ordering::Relaxed) {
            self.fail(TreeSitterError::Cancelled)
        } else if self.native.exceeded(self.budget.max_native_bytes) {
            self.fail(TreeSitterError::Limit("native allocation soft limit"))
        } else if self.started.elapsed() >= self.budget.slice_duration {
            self.fail(TreeSitterError::Limit("slice deadline"))
        } else {
            false
        }
    }
    fn progress(&self) -> bool {
        self.statistics.borrow_mut().progress_callbacks += 1;
        if self.statistics.borrow().progress_callbacks > self.budget.max_progress_callbacks {
            self.fail(TreeSitterError::Limit("native progress callbacks"))
        } else {
            self.check()
        }
    }
    fn read(&self, length: usize) -> bool {
        let mut stats = self.statistics.borrow_mut();
        stats.input_callbacks += 1;
        let total = stats.input_bytes_supplied.saturating_add(length);
        if total > self.budget.max_input_bytes_supplied {
            drop(stats);
            self.fail(TreeSitterError::Limit("input bytes supplied"));
            false
        } else {
            stats.input_bytes_supplied = total;
            stats.maximum_input_chunk = stats.maximum_input_chunk.max(length);
            true
        }
    }
    fn work(&self) -> TreeSitterWork {
        let mut work = self.statistics.borrow().clone();
        let native = self.native.metrics();
        work.native_retained_bytes = native.retained_bytes;
        work.native_peak_bytes = native.peak_bytes;
        work
    }
    fn failure(&self) -> Option<TreeSitterError> {
        self.failure.borrow().clone()
    }
    fn dependency(&self, range: Range<usize>) {
        let mut dependency = self.dependencies.borrow_mut();
        if let Some(old) = dependency.as_mut() {
            old.start = old.start.min(range.start);
            old.end = old.end.max(range.end);
        } else {
            *dependency = Some(range);
        }
    }
    fn text(
        &self,
        input: &SyntaxInputSnapshot,
        range: Range<usize>,
    ) -> Result<Vec<u8>, TreeSitterError> {
        self.dependency(range.clone());
        if self.check() {
            return Err(self.failure().unwrap());
        }
        let mut stats = self.statistics.borrow_mut();
        let bytes = stats
            .predicate_bytes_copied
            .checked_add(range.len())
            .ok_or(TreeSitterError::Limit("predicate bytes"))?;
        if bytes > self.budget.max_predicate_bytes {
            return Err(TreeSitterError::Limit("predicate bytes"));
        }
        stats.predicate_bytes_copied = bytes;
        drop(stats);
        input
            .slice(range)
            .map(String::into_bytes)
            .map_err(|_| TreeSitterError::InvalidInput)
    }
    fn charge_predicate(&self, steps: usize) -> Result<(), TreeSitterError> {
        let mut stats = self.statistics.borrow_mut();
        stats.predicate_steps = stats.predicate_steps.saturating_add(steps);
        if stats.predicate_steps > self.budget.max_predicate_steps {
            return Err(TreeSitterError::Limit("predicate steps"));
        }
        drop(stats);
        if self.check() {
            Err(self.failure().unwrap())
        } else {
            Ok(())
        }
    }
    fn charge_output(&self, bytes: usize) -> Result<(), TreeSitterError> {
        let mut work = self.statistics.borrow_mut();
        work.output_bytes = work.output_bytes.saturating_add(bytes);
        if work.output_bytes > self.budget.max_output_bytes {
            return Err(TreeSitterError::Limit("query output bytes"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InjectionRegion {
    pub language: String,
    pub ranges: Vec<Range<usize>>,
    pub combined: bool,
    pub parent_input: SyntaxInputIdentity,
    pub package_generation: u64,
}
#[derive(Clone, Debug)]
pub struct HighlightResult {
    pub input: SyntaxInputIdentity,
    pub package_generation: u64,
    pub range: Range<usize>,
    pub coverage: Coverage,
    pub runs: Vec<SyntaxRun>,
    pub injections: Vec<InjectionRegion>,
    pub work: TreeSitterWork,
    pub diagnostic: Option<TreeSitterError>,
    /// Capture/text/ancestor reads, including attempted capped text reads. This
    /// is for conservative failure memoization, not exact-coverage reuse proof.
    pub failure_dependencies: Range<usize>,
}

/// Empty successful query output is authoritative. Interrupted or capped
/// evaluation publishes no arbitrary subset as Exact.
pub fn highlight(
    snapshot: &ParseSnapshot,
    range: Range<usize>,
    budget: &TreeSitterBudget,
    cancelled: &AtomicBool,
) -> HighlightResult {
    let native = Arc::new(native::Account::default());
    let _scope = native::Scope::new(&native);
    let control = Control::new(budget, cancelled, &native);
    let mut output = HighlightResult {
        input: snapshot.input.identity(),
        package_generation: snapshot.package.generation,
        range: range.clone(),
        coverage: Coverage::Missing,
        runs: Vec::new(),
        injections: Vec::new(),
        work: TreeSitterWork::default(),
        diagnostic: None,
        failure_dependencies: range.clone(),
    };
    let result = (|| {
        if range.start > range.end {
            return Err(TreeSitterError::InvalidInput);
        }
        point(&snapshot.input, range.start)?;
        point(&snapshot.input, range.end)?;
        let captures = execute_query(
            snapshot,
            &snapshot.package.highlights,
            &range,
            &control,
            false,
        )?;
        let runs = coalesce(captures.0);
        let output_bytes = runs
            .iter()
            .map(|run| std::mem::size_of::<SyntaxRun>() + run.name.0.len() + run.origin.len())
            .sum::<usize>();
        if output_bytes > budget.max_output_bytes {
            return Err(TreeSitterError::Limit("query output bytes"));
        }
        let injections = match &snapshot.package.injections {
            Some(query) => {
                // Combined groups require complete discovery, including members
                // outside the viewport. Exhaustion makes coverage Missing.
                let discovery = if query.patterns.iter().any(|p| p.combined) {
                    0..snapshot.input.byte_len()
                } else {
                    range.clone()
                };
                execute_query(snapshot, query, &discovery, &control, true)?.1
            }
            None => Vec::new(),
        };
        if control.check() {
            return Err(control.failure().unwrap());
        }
        Ok((runs, injections))
    })();
    match result {
        Ok((runs, injections)) => {
            output.coverage = Coverage::Exact;
            output.runs = runs;
            output.injections = injections;
        }
        Err(error) => output.diagnostic = Some(error),
    }
    output.work = control.work();
    if let Some(dependency) = control.dependencies.borrow().as_ref() {
        output.failure_dependencies.start = output.failure_dependencies.start.min(dependency.start);
        output.failure_dependencies.end = output.failure_dependencies.end.max(dependency.end);
    }
    output
}

fn execute_query(
    snapshot: &ParseSnapshot,
    compiled: &CompiledQuery,
    range: &Range<usize>,
    control: &Control<'_>,
    injection: bool,
) -> Result<(Vec<RankedRun>, Vec<InjectionRegion>), TreeSitterError> {
    if range.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    if injection
        && compiled.patterns.iter().any(|p| p.combined)
        && (range.start != 0 || range.end != snapshot.input.byte_len())
    {
        return Err(TreeSitterError::Limit(
            "combined injection discovery coverage",
        ));
    }
    if control.check() {
        return Err(control.failure().unwrap());
    }
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(control.budget.max_pending_matches.clamp(1, 65_536));
    cursor.set_byte_range(range.clone());
    let mut progress = |_: &tree_sitter::QueryCursorState| control.progress();
    let no_text = |_node: Node<'_>| std::iter::empty::<&'static [u8]>();
    let mut iterator = cursor.matches_with_options(
        &compiled.query,
        snapshot.tree.root_node(),
        no_text,
        QueryCursorOptions::new().progress_callback(&mut progress),
    );
    let mut runs = Vec::new();
    let mut injections = Vec::new();
    let mut combined = BTreeMap::<(usize, String), InjectionRegion>::new();
    let mut injection_ranges = 0usize;
    let mut serial = 0;
    while let Some(found) = iterator.next() {
        if control.check() {
            return Err(control.failure().unwrap());
        }
        {
            let mut work = control.statistics.borrow_mut();
            work.query_matches += 1;
            work.query_captures = work.query_captures.saturating_add(found.captures.len());
            if work.query_matches > control.budget.max_matches {
                return Err(TreeSitterError::Limit("query matches"));
            }
            if work.query_captures > control.budget.max_captures {
                return Err(TreeSitterError::Limit("query captures"));
            }
        }
        let pattern = &compiled.patterns[found.pattern_index];
        for capture in found.captures {
            control.dependency(capture.node.byte_range());
        }
        if !predicates_match(pattern, found, &snapshot.input, control)? {
            continue;
        }
        if injection {
            if let Some(region) = injection_region(snapshot, compiled, pattern, found, control)? {
                injection_ranges = injection_ranges.saturating_add(region.ranges.len());
                if injection_ranges > control.budget.max_injections {
                    return Err(TreeSitterError::Limit("injections"));
                }
                if region.combined {
                    let key = (found.pattern_index, region.language.clone());
                    match combined.entry(key) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(region);
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            entry.get_mut().ranges.extend(region.ranges);
                        }
                    }
                } else {
                    injections.push(region);
                }
            }
            continue;
        }
        for capture in found.captures {
            let name = compiled.query.capture_names()[capture.index as usize];
            if name.starts_with('_')
                || name.starts_with("injection.")
                || matches!(name, "spell" | "nospell")
            {
                continue;
            }
            let node_range = offset_range(capture.node, pattern, capture.index, &snapshot.input)?;
            let start = node_range.start.max(range.start);
            let end = node_range.end.min(range.end);
            if start >= end {
                continue;
            }
            control.charge_output(
                std::mem::size_of::<RankedRun>()
                    .saturating_add(name.len().saturating_mul(2))
                    .saturating_add(snapshot.package.id.len())
                    .saturating_add(16),
            )?;
            runs.push(RankedRun {
                run: SyntaxRun {
                    range: start..end,
                    name: SyntaxStyleName(format!("@{name}")),
                    origin: format!("treesitter:{}:@{name}", snapshot.package.id),
                    priority: pattern.priority,
                },
                pattern: found.pattern_index,
                serial,
                extent: node_range.len(),
            });
            serial += 1;
        }
    }
    drop(iterator);
    if cursor.did_exceed_match_limit() {
        return Err(TreeSitterError::Limit("pending query matches"));
    }
    if control.check() {
        return Err(control.failure().unwrap());
    }
    for mut region in combined.into_values() {
        region.ranges.sort_by_key(|r| r.start);
        if region.ranges.windows(2).any(|p| p[0].end > p[1].start) {
            return Err(TreeSitterError::InvalidQuery(
                "overlapping combined injection ranges".into(),
            ));
        }
        injections.push(region);
    }
    Ok((runs, injections))
}

fn capture_text(
    found: &QueryMatch<'_, '_>,
    capture: u32,
    input: &SyntaxInputSnapshot,
    control: &Control<'_>,
) -> Result<Vec<Vec<u8>>, TreeSitterError> {
    found
        .captures
        .iter()
        .filter(|c| c.index == capture)
        .map(|c| control.text(input, c.node.byte_range()))
        .collect()
}

fn predicates_match(
    pattern: &Pattern,
    found: &QueryMatch<'_, '_>,
    input: &SyntaxInputSnapshot,
    control: &Control<'_>,
) -> Result<bool, TreeSitterError> {
    for predicate in &pattern.predicates {
        let accepted = match predicate {
            Predicate::Ancestor {
                capture,
                kinds,
                positive,
                direct,
            } => {
                let mut accepted = true;
                for capture in found.captures.iter().filter(|c| c.index == *capture) {
                    let mut ancestor = capture.node.parent();
                    let mut found = false;
                    while let Some(node) = ancestor {
                        control.dependency(node.byte_range());
                        control.charge_predicate(kinds.len().max(1))?;
                        if kinds.iter().any(|kind| kind == node.kind()) {
                            found = true;
                            break;
                        }
                        if *direct {
                            break;
                        }
                        ancestor = node.parent();
                    }
                    accepted &= found == *positive;
                }
                accepted
            }
            Predicate::Equal {
                left,
                right,
                positive,
                any,
            } => {
                let left = capture_text(found, *left, input, control)?;
                let mut accepted = !*any;
                match right {
                    Argument::Capture(id) => {
                        let right = capture_text(found, *id, input, control)?;
                        if left.len() != right.len() {
                            false
                        } else {
                            for (a, b) in left.iter().zip(&right) {
                                control.charge_predicate(a.len().max(b.len()).max(1))?;
                                let matches = (a == b) == *positive;
                                if *any {
                                    accepted |= matches;
                                } else {
                                    accepted &= matches;
                                }
                                if accepted == *any {
                                    break;
                                }
                            }
                            accepted
                        }
                    }
                    Argument::String(value) => {
                        for a in &left {
                            control.charge_predicate(a.len().max(value.len()).max(1))?;
                            let matches = (a.as_slice() == value.as_bytes()) == *positive;
                            if *any {
                                accepted |= matches;
                            } else {
                                accepted &= matches;
                            }
                            if accepted == *any {
                                break;
                            }
                        }
                        accepted
                    }
                }
            }
            Predicate::Membership {
                capture,
                values,
                positive,
            } => {
                let mut yes = true;
                for text in capture_text(found, *capture, input, control)? {
                    control.charge_predicate(text.len().saturating_mul(values.len()).max(1))?;
                    yes &= values.iter().any(|v| v.as_bytes() == text) == *positive;
                }
                yes
            }
            Predicate::Match {
                capture,
                matcher,
                positive,
                any,
            } => {
                let mut outcomes = Vec::new();
                for text in capture_text(found, *capture, input, control)? {
                    control.charge_predicate(text.len().max(1))?;
                    let matched = match matcher {
                        Matcher::Regular(regex) => regular_match(regex, &text, control)?,
                        Matcher::Vim(pattern) => {
                            let text = std::str::from_utf8(&text)
                                .map_err(|_| TreeSitterError::InvalidInput)?;
                            let available = control
                                .budget
                                .max_predicate_steps
                                .saturating_sub(control.statistics.borrow().predicate_steps);
                            let mut remaining = available;
                            let matched = pattern
                                .is_match_text_with_control(text, &mut remaining, &mut || {
                                    control.check()
                                })
                                .map_err(|_| TreeSitterError::Limit("Vim predicate steps"))?;
                            control.charge_predicate(available - remaining)?;
                            matched
                        }
                    };
                    outcomes.push(matched == *positive);
                }
                if *any {
                    outcomes.into_iter().any(|b| b)
                } else {
                    outcomes.into_iter().all(|b| b)
                }
            }
        };
        if !accepted {
            return Ok(false);
        }
    }
    Ok(true)
}

fn regular_match(
    regex: &dense::DFA<Vec<u32>>,
    text: &[u8],
    control: &Control<'_>,
) -> Result<bool, TreeSitterError> {
    let mut state = regex
        .start_state_forward(&regex_automata::Input::new(text))
        .map_err(|e| TreeSitterError::UnsupportedQuery(format!("upstream regex start: {e}")))?;
    for &byte in text {
        control.charge_predicate(1)?;
        state = regex.next_state(state, byte);
        if regex.is_match_state(state) {
            return Ok(true);
        }
        if regex.is_dead_state(state) {
            return Ok(false);
        }
        if regex.is_quit_state(state) {
            return Err(TreeSitterError::UnsupportedQuery(
                "upstream regex quit state".into(),
            ));
        }
    }
    control.charge_predicate(1)?;
    state = regex.next_eoi_state(state);
    if regex.is_quit_state(state) {
        return Err(TreeSitterError::UnsupportedQuery(
            "upstream regex quit state".into(),
        ));
    }
    Ok(regex.is_match_state(state))
}

fn offset_range(
    node: Node<'_>,
    pattern: &Pattern,
    capture: u32,
    input: &SyntaxInputSnapshot,
) -> Result<Range<usize>, TreeSitterError> {
    let Some(offset) = pattern.offsets.iter().find(|o| o.capture == capture) else {
        return Ok(node.byte_range());
    };
    let move_point = |point: Point, row: i32, column: i32| {
        let row = point
            .row
            .checked_add_signed(row as isize)
            .ok_or(TreeSitterError::InvalidInput)?;
        let column = point
            .column
            .checked_add_signed(column as isize)
            .ok_or(TreeSitterError::InvalidInput)?;
        let start = input
            .text_tree()
            .hard_line_start(row)
            .map_err(|_| TreeSitterError::InvalidInput)?;
        let end = input
            .text_tree()
            .hard_line_end(row)
            .map_err(|_| TreeSitterError::InvalidInput)?;
        let byte = start
            .checked_add(column)
            .ok_or(TreeSitterError::InvalidInput)?;
        if byte > end {
            return Err(TreeSitterError::InvalidInput);
        }
        self::point(input, byte)?;
        Ok(byte)
    };
    let start = move_point(node.start_position(), offset.start_row, offset.start_column)?;
    let end = move_point(node.end_position(), offset.end_row, offset.end_column)?;
    if start > end {
        return Err(TreeSitterError::InvalidInput);
    }
    Ok(start..end)
}

fn injection_region(
    snapshot: &ParseSnapshot,
    compiled: &CompiledQuery,
    pattern: &Pattern,
    found: &QueryMatch<'_, '_>,
    control: &Control<'_>,
) -> Result<Option<InjectionRegion>, TreeSitterError> {
    let mut language = pattern.injection_language.clone();
    let mut ranges = Vec::new();
    for capture in found.captures {
        match compiled.query.capture_names()[capture.index as usize] {
            "injection.language" => {
                language = Some(
                    String::from_utf8(control.text(&snapshot.input, capture.node.byte_range())?)
                        .map_err(|_| TreeSitterError::InvalidInput)?,
                );
            }
            "injection.content" => {
                let range = offset_range(capture.node, pattern, capture.index, &snapshot.input)?;
                if pattern.include_children {
                    ranges.push(range);
                } else {
                    let mut at = range.start;
                    let mut walk = capture.node.walk();
                    for child in capture.node.children(&mut walk) {
                        control.charge_predicate(1)?;
                        if at < child.start_byte().min(range.end) {
                            ranges.push(at..child.start_byte().min(range.end));
                        }
                        at = at.max(child.end_byte()).min(range.end);
                    }
                    if at < range.end {
                        ranges.push(at..range.end);
                    }
                }
            }
            _ => {}
        }
    }
    let Some(language) = language.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if language.len() > 256
        || !language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
    {
        return Err(TreeSitterError::InvalidQuery(
            "invalid injection language".into(),
        ));
    }
    ranges.retain(|r| !r.is_empty());
    ranges.sort_by_key(|r| r.start);
    if ranges.is_empty() {
        return Ok(None);
    }
    if ranges.windows(2).any(|p| p[0].end > p[1].start) {
        return Err(TreeSitterError::InvalidQuery(
            "overlapping injection ranges".into(),
        ));
    }
    Ok(Some(InjectionRegion {
        language,
        ranges,
        combined: pattern.combined,
        parent_input: snapshot.input.identity(),
        package_generation: snapshot.package.generation,
    }))
}

struct RankedRun {
    run: SyntaxRun,
    pattern: usize,
    serial: usize,
    extent: usize,
}

/// Priority and tie ordering precede name lookup. Sweep events avoid quadratic
/// behavior when many nested nodes produce overlapping captures.
fn coalesce(runs: Vec<RankedRun>) -> Vec<SyntaxRun> {
    let mut events: BTreeMap<usize, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (id, run) in runs.iter().enumerate() {
        events.entry(run.run.range.start).or_default().0.push(id);
        events.entry(run.run.range.end).or_default().1.push(id);
    }
    let mut active = BTreeSet::new();
    let mut output: Vec<SyntaxRun> = Vec::new();
    let mut previous = 0;
    for (at, (starts, ends)) in events {
        if previous < at {
            if let Some((_, _, _, _, id)) = active.iter().next_back() {
                let chosen: &RankedRun = &runs[*id];
                if let Some(last) = output.last_mut().filter(|r| {
                    r.range.end == previous
                        && r.name == chosen.run.name
                        && r.origin == chosen.run.origin
                        && r.priority == chosen.run.priority
                }) {
                    last.range.end = at;
                } else {
                    let mut next = chosen.run.clone();
                    next.range = previous..at;
                    output.push(next);
                }
            }
        }
        for id in ends {
            let r = &runs[id];
            active.remove(&(
                r.run.priority,
                std::cmp::Reverse(r.extent),
                r.pattern,
                r.serial,
                id,
            ));
        }
        for id in starts {
            let r = &runs[id];
            active.insert((
                r.run.priority,
                std::cmp::Reverse(r.extent),
                r.pattern,
                r.serial,
                id,
            ));
        }
        previous = at;
    }
    output
}

#[cfg(test)]
mod performance;
#[cfg(test)]
mod tests;
