use super::*;
use crate::document::{
    syntax::{
        treesitter::{QueryProfile, TreeSitterPackage},
        SyntaxInputIdentity,
    },
    FormattedTextTree,
};
use std::sync::{atomic::AtomicUsize, Arc};

mod vim;
mod markdown;

fn request(text: &str, language: &str, revision: u64) -> SyntaxRequest {
    SyntaxRequest {
        input: SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 81,
                revision,
                generation: 1,
            },
            FormattedTextTree::try_from_text(text).unwrap(),
        ),
        configuration: SyntaxConfiguration {
            generation: 1,
            registry_generation: treesitter::package_registry_generation(),
            language: Some(language.into()),
            vim_directory: String::new(),
            filename: None,
        },
        range: 0..text.len(),
    }
}
fn finish(provider: &mut BackendProvider, request: &SyntaxRequest) -> SyntaxResult {
    for _ in 0..MAX_PARSE_SLICES + 2 {
        let result = provider.analyze(request, &AtomicBool::new(false));
        if !result.continuation {
            return result;
        }
    }
    panic!("provider exceeded continuation budget")
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(language: &str, program: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let path = std::env::temp_dir().join(format!(
            "viem-provider-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join(format!("{language}.vim")), program).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scroll_reuses_completed_tree_and_edits_are_incremental() {
    let _registry = treesitter::package_registry_test_guard();
    let mut provider = BackendProvider::default();
    let mut first = request("int first; int second;\n", "c", 1);
    first.range = 0..10;
    let result = finish(&mut provider, &first);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    let retained = provider
        .primary
        .as_ref()
        .unwrap()
        .completed()
        .unwrap()
        .clone();
    first.range = 11..22;
    assert_eq!(finish(&mut provider, &first).coverage, Coverage::Exact);
    assert_eq!(
        provider
            .primary
            .as_ref()
            .unwrap()
            .completed()
            .unwrap()
            .input_identity(),
        retained.input_identity()
    );
    assert_eq!(provider.parse_slices, 0);
    let second = request("int changed; int second;\n", "c", 2);
    let output = finish(&mut provider, &second);
    assert_eq!(output.coverage, Coverage::Exact);
    let fresh = finish(&mut BackendProvider::default(), &second);
    assert_eq!(output.runs, fresh.runs);
    assert_eq!(retained.input().slice(0..10).unwrap(), "int first;");
}

#[test]
fn absent_primary_uses_vim_and_unavailable_vim_stays_missing() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("fixture", "syn keyword Important hello\n");
    let mut req = request("hello world", "fixture", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let result = finish(&mut BackendProvider::default(), &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert_eq!(result.runs[0].name.as_str(), "Important");
    req.configuration.vim_directory = fixture.0.join("absent").to_str().unwrap().to_owned();
    let missing = finish(&mut BackendProvider::default(), &req);
    assert_eq!(missing.coverage, Coverage::Missing);
    assert!(!missing.continuation);
    assert!(!missing.diagnostics.is_empty());
}

#[test]
fn registry_reload_without_edit_replaces_fallback_with_empty_primary() {
    let _registry = treesitter::package_registry_test_guard();
    const ID: &str = "provider-registry-test";
    let fixture = Fixture::new(ID, "syn keyword Fallback hello\n");
    let mut req = request("hello", ID, 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let mut provider = BackendProvider::default();
    assert_eq!(finish(&mut provider, &req).runs[0].name.as_str(), "Fallback");
    struct Owner(Arc<AtomicBool>);
    impl Drop for Owner {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let package = TreeSitterPackage::compile(
        ID,
        1,
        tree_sitter_c::LANGUAGE.into(),
        "",
        None,
        QueryProfile::Upstream,
        &TreeSitterBudget::default(),
        Some(Arc::new(Owner(dropped.clone()))),
    )
    .unwrap();
    let old_generation = treesitter::package_registry_generation();
    treesitter::register_package(ID, package.clone()).unwrap();
    assert!(treesitter::package_registry_generation() > old_generation);
    req.configuration.registry_generation = treesitter::package_registry_generation();
    let result = finish(&mut provider, &req);
    assert_eq!(result.coverage, Coverage::Exact);
    assert!(result.runs.is_empty());
    assert!(treesitter::unregister_package(ID));
    drop(package);
    assert!(!dropped.load(Ordering::Relaxed));
    drop(provider);
    assert!(dropped.load(Ordering::Relaxed));
    assert!(!treesitter::unregister_package(ID));
}

#[test]
fn embedded_provider_replaces_host_coverage_and_matches_fresh_after_language_change() {
    let _registry = treesitter::package_registry_test_guard();
    let mut provider = BackendProvider::default();
    let first = request("const q = python\u{0060}print(1)\u{0060};", "javascript", 1);
    let result = finish(&mut provider, &first);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert!(result
        .runs
        .iter()
        .any(|r| r.origin.starts_with("treesitter:python:")));
    let second = request("const q = rust\u{0060}let x = 1;\u{0060};", "javascript", 2);
    let result = finish(&mut provider, &second);
    assert_eq!(
        result.runs,
        finish(&mut BackendProvider::default(), &second).runs
    );
    assert!(!result
        .runs
        .iter()
        .any(|r| r.origin.starts_with("treesitter:python:")));
    assert!(provider.children.len() <= MAX_CHILDREN);
}

#[test]
fn nvim_json_injections_use_the_bundled_parser_in_javascript_and_rust() {
    let _registry = treesitter::package_registry_test_guard();
    for (language, text) in [
        (
            "javascript",
            "const value = json`{\"enabled\": true, \"count\": 42}`;\n",
        ),
        (
            "rust",
            "fn main() { let value = json!({\"enabled\": true, \"count\": 42}); }\n",
        ),
    ] {
        let result = finish(&mut BackendProvider::default(), &request(text, language, 1));
        assert_eq!(
            result.coverage,
            Coverage::Exact,
            "{language}: {:?}",
            result.diagnostics
        );
        assert_eq!(
            name_at(&result, text, "enabled").as_deref(),
            Some("Property")
        );
        assert_eq!(name_at(&result, text, "true").as_deref(), Some("Boolean"));
        assert!(
            result
                .runs
                .iter()
                .any(|run| run.origin.starts_with("treesitter:json:")),
            "{language}"
        );
    }
}

#[test]
fn cancelled_provider_does_not_load_fallback_or_parse() {
    let _registry = treesitter::package_registry_test_guard();
    let req = request("int x;", "c", 1);
    let mut provider = BackendProvider::default();
    let result = provider.analyze(&req, &AtomicBool::new(true));
    assert_eq!(result.coverage, Coverage::Missing);
    assert!(!result.continuation);
    assert!(provider.primary.as_ref().unwrap().completed().is_none());
    assert!(!provider.fallback_attempted);
}

#[test]
fn capped_query_memo_rebases_without_retrying_unrelated_text() {
    let _registry = treesitter::package_registry_test_guard();
    let old = request("int x;\nint y;\n", "c", 1).input;
    let mut capped = CappedQuery {
        input: old.clone(),
        range: 0..6,
        dependencies: 0..6,
        diagnostic: "cap".into(),
    };
    let unrelated = SyntaxInputSnapshot::new(
        request("", "c", 2).input.identity(),
        old.text_tree().splice(11..12, "replacement").unwrap(),
    );
    assert!(capped.rebase(&unrelated));
    assert_eq!(capped.range, 0..6);
    let relevant = SyntaxInputSnapshot::new(
        request("", "c", 3).input.identity(),
        unrelated.text_tree().splice(4..5, "changed").unwrap(),
    );
    assert!(!capped.rebase(&relevant));
}

#[test]
fn vim_directory_change_preserves_the_completed_primary_parser() {
    let _registry = treesitter::package_registry_test_guard();
    let mut req = request("int value;", "c", 1);
    let mut provider = BackendProvider::default();
    assert_eq!(finish(&mut provider, &req).coverage, Coverage::Exact);
    let progress = provider.parse_progress;
    req.configuration.vim_directory = "/absent/new-directory".into();
    req.configuration.generation += 1;
    provider.configure(&req);
    assert_eq!(
        provider
            .primary
            .as_ref()
            .unwrap()
            .completed()
            .unwrap()
            .input_identity(),
        req.input.identity()
    );
    assert_eq!(finish(&mut provider, &req).coverage, Coverage::Exact);
    assert_eq!(provider.parse_progress, progress);
}

#[test]
fn failed_child_tree_sitter_uses_vim_and_unknown_children_keep_host_colors() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("fixture", "syn keyword ChildKeyword hello\n");
    let mut req = request("hello", "fixture", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().into();
    let mut child = Child {
        language: "fixture".into(),
        ranges: vec![0..5],
        session: Some(
            TreeSitterSession::new(TreeSitterPackage::bundled("python").unwrap()).unwrap(),
        ),
        fallback: None,
        fallback_attempted: false,
        slices: 0,
        progress: 0,
        deadline_retries: 0,
        touched: true,
        input: Some(req.input.clone()),
        failed: None,
        fallback_input: None,
        fallback_context: None,
        fallback_reads: VimSetupReads::default(),
        fallback_work: 0,
        cache: None,
    };
    let output = analyze_child(
        &mut child,
        &req,
        &TreeSitterBudget {
            max_native_bytes: 0,
            ..TreeSitterBudget::default()
        },
        &AtomicBool::new(false),
        &mut |_| true,
    );
    assert!(child.failed.is_some());
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.as_str(), "ChildKeyword");
    let req = request(
        "const q = nonexistent\u{0060}hello\u{0060};",
        "javascript",
        1,
    );
    let output = finish(&mut BackendProvider::default(), &req);
    let start = req
        .input
        .slice(0..req.input.byte_len())
        .unwrap()
        .find("hello")
        .unwrap();
    // Like Neovim, a language without a provider is not injected at all.
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert!(
        output
            .runs
            .iter()
            .any(|run| run.range.start <= start && run.range.end >= start + 5),
        "unavailable child hid host colors: {:?}",
        output.runs
    );
}

#[test]
fn total_child_input_limit_keeps_host_runs_without_creating_parser() {
    let _registry = treesitter::package_registry_test_guard();
    let mut req = request(&"x".repeat(MAX_TOTAL_CHILD_INPUT_BYTES + 1), "c", 1);
    req.range = 0..10;
    let mut provider = BackendProvider::default();
    let mut output = SyntaxResult::missing(&req, "");
    output.coverage = Coverage::Exact;
    output.runs.push(SyntaxRun {
        range: 0..10,
        name: crate::document::syntax::SyntaxStyleName("String".into()),
        origin: "host".into(),
        priority: 100,
    });
    provider.children(
        &req,
        vec![InjectionRegion {
            language: "python".into(),
            ranges: vec![0..req.input.byte_len()],
            combined: false,
            parent_input: req.input.identity(),
            package_generation: 1,
        }],
        &mut output,
        &AtomicBool::new(false),
    );
    assert!(provider.children.is_empty());
    assert_eq!(output.runs.len(), 1, "host runs remain until a child has coverage");
    assert_eq!(output.coverage, Coverage::Provisional);
    assert!(!output.continuation);
}

#[test]
fn native_wide_root_repair_exhaustion_uses_fallback_without_repaint_retry() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "int x = 1;\n".repeat(200_000);
    let mut original = request(&text, "c", 1);
    original.range = 0..20;
    let mut provider = BackendProvider::default();
    let cold_started = std::time::Instant::now();
    assert_eq!(finish(&mut provider, &original).coverage, Coverage::Exact);
    let cold_ms = cold_started.elapsed().as_secs_f64() * 1000.;
    let tree = original.input.text_tree().splice(8..9, "2").unwrap();
    let mut changed = original.clone();
    let mut identity = changed.input.identity();
    identity.revision += 1;
    changed.input = SyntaxInputSnapshot::new(identity, tree);
    let repair_started = std::time::Instant::now();
    let output = finish(&mut provider, &changed);
    let repair_ms = repair_started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(output.coverage, Coverage::Missing);
    assert!(provider.primary_capped);
    assert!(provider.primary.as_ref().unwrap().completed().is_none(),
        "a capped repair must release its unusable whole-file tree");
    assert!(provider.parse_progress <= MAX_TOTAL_REPAIR_PROGRESS + 1);
    let slices = provider.parse_slices;
    assert!(output.diagnostics.iter().any(|d| d.contains("budget")));
    for _ in 0..3 {
        assert!(
            !provider
                .analyze(&changed, &AtomicBool::new(false))
                .continuation
        );
        assert_eq!(provider.parse_slices, slices);
    }
    let tree = changed
        .input
        .text_tree()
        .splice(
            changed.input.byte_len() - 3..changed.input.byte_len() - 2,
            "3",
        )
        .unwrap();
    let mut identity = changed.input.identity();
    identity.revision += 1;
    changed.input = SyntaxInputSnapshot::new(identity, tree);
    assert_eq!(finish(&mut provider, &changed).coverage, Coverage::Missing);
    assert_eq!(
        provider.parse_slices, slices,
        "an unrelated edit retried identical capped repair"
    );
    if let Ok(path) = std::env::var("VIEM_SYNTAX_POLICY_REPORT") {
        let budget = TreeSitterBudget::default();
        let report = serde_json::json!({
            "fixture_revision": 1,
            "gate": "production bounded fallback policy",
            "build": if cfg!(debug_assertions) { "debug" } else { "release" },
            "engine": "tree-sitter 0.25.10",
            "fixture": "200000 independent C declarations; one-byte edit near BOF",
            "input_bytes": text.len(),
            "gate_passed": true,
            "initial_coverage": "Exact",
            "edited_coverage": format!("{:?}", output.coverage),
            "cold_provider_ms": cold_ms,
            "fallback_repair_ms": repair_ms,
            "timing_samples": 1,
            "timing_note": "Single policy regression observation, not a latency percentile or hard native deadline claim",
            "warm_progress_callbacks": provider.parse_progress,
            "warm_worker_slices": slices,
            "warm_callback_limit": MAX_TOTAL_REPAIR_PROGRESS,
            "max_edit_tree_nodes": budget.max_edit_tree_nodes,
            "max_edit_root_children": budget.max_edit_root_children,
            "repaint_retries": 0,
            "unrelated_edit_retries": 0,
            "diagnostics": output.diagnostics,
            "reference_hardware": {"cpu": "Apple M1 Ultra", "model": "Mac13,2", "memory_bytes": 137438953472u64},
            "separate_exact_native_diagnostic": "performance-baseline.json records known failing native-work ceilings"
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

fn bundled_vim(text: &str, language: &str) -> SyntaxRequest {
    let mut request = request(text, language, 1);
    request.configuration.vim_directory =
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
    request
}
fn name_at(result: &SyntaxResult, text: &str, needle: &str) -> Option<String> {
    let start = text.find(needle).unwrap();
    result
        .runs
        .iter()
        .find(|run| run.range.start <= start && run.range.end >= start + needle.len())
        .map(|run| run.name.to_string())
}

#[test]
fn nvim_cpp_queries_highlight_preprocessor_comments_and_injections() {
    let _registry = treesitter::package_registry_test_guard();
    let text = concat!(
        "#include <vector>\n",
        "#define MAX_SIZE (1 << 10)\n",
        "/// Brief doc comment.\n",
        "// Plain comment.\n",
        "class Widget { int m_count; };\n",
        "int main() {\n",
        "  auto q = R\"sql(select 1)sql\";\n",
        "  printf(\"%d\\n\", MAX_SIZE);\n",
        "  return nullptr != nullptr;\n",
        "}\n",
    );
    let request = bundled_vim(text, "cpp");
    let result = finish(&mut BackendProvider::default(), &request);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    let name = |needle| name_at(&result, text, needle);
    assert_eq!(name("#include").as_deref(), Some("Keyword.import"));
    assert_eq!(name("<vector>").as_deref(), Some("String"));
    assert_eq!(name("#define").as_deref(), Some("Keyword.directive.define"));
    // lua-match? constants and the self-injected macro body.
    assert_eq!(name("MAX_SIZE").as_deref(), Some("Constant.macro"));
    assert_eq!(name("<<").as_deref(), Some("Operator"));
    assert_eq!(name("m_count").as_deref(), Some("Variable.member"));
    assert_eq!(name("nullptr").as_deref(), Some("Constant.builtin"));
    // The comment, doxygen and printf injections have no available provider,
    // so the host colors remain.
    assert_eq!(name("/// Brief doc comment.").as_deref(), Some("Comment"));
    assert_eq!(name("// Plain comment.").as_deref(), Some("Comment"));
    assert_eq!(name("\"%d").as_deref(), Some("String"));
    // A raw-string delimiter selects Vim sql; its gaps keep the host color.
    assert_eq!(name("select").as_deref(), Some("Statement"));
    let space = text.find("select").unwrap() + "select".len();
    assert!(result
        .runs
        .iter()
        .any(|run| run.range.contains(&space) && run.name.as_str() == "String"));
}

#[test]
fn many_injected_macro_bodies_are_all_highlighted() {
    let _registry = treesitter::package_registry_test_guard();
    let text = (0..100)
        .map(|n| format!("#define VALUE_{n} ({n} + 1)\n"))
        .collect::<String>();
    let request = bundled_vim(&text, "c");
    let result = finish(&mut BackendProvider::default(), &request);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    let plus = text.match_indices(" + ").map(|(at, _)| at + 1).collect::<Vec<_>>();
    assert_eq!(plus.len(), 100);
    for at in plus {
        assert!(
            result
                .runs
                .iter()
                .any(|run| run.range.contains(&at) && run.name.as_str() == "Operator"),
            "macro body at {at} was not highlighted"
        );
    }
}

#[test]
fn injection_overflow_keeps_host_highlighting() {
    let _registry = treesitter::package_registry_test_guard();
    let count = TreeSitterBudget::default().max_injections + 10;
    let text = (0..count)
        .map(|n| format!("#define V{n} {n}\n"))
        .collect::<String>();
    let request = bundled_vim(&text, "c");
    let result = finish(&mut BackendProvider::default(), &request);
    assert_eq!(result.coverage, Coverage::Provisional);
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.contains("discovery budget")));
    let last = text.rfind("#define").unwrap();
    assert!(result
        .runs
        .iter()
        .any(|run| run.range.start == last && run.name.as_str() == "Keyword.directive.define"));
}
