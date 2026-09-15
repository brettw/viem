use super::*;

#[test]
fn vim_provider_cancelled_compile_remains_retryable() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("vim", "syn keyword Native set\n");
    let mut req = request("set number", "vim", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let mut provider = BackendProvider::default();
    provider.configure(&req);
    let cancelled = AtomicBool::new(true);
    // Inject the completed load outcome to exercise cancellation after loading
    // began, without racing a timer against filesystem/regex work.
    assert!(!provider.finish_fallback_load(
        Err(vec![VimDiagnostic {
            file: "vim.vim".into(),
            line: 1,
            message: "syntax compilation cancelled".into(),
        }]),
        &cancelled,
    ));
    assert!(!provider.fallback_attempted);
    assert!(provider.fallback_failure.is_empty());
    assert!(provider.fallback.is_none());
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.0, "Native");
    assert!(provider.primary.is_none());
}

const DIALECT_SYNTAX: &str = "\
if getline(1, 1)->join('') ==# 'vim9script'\n\
  syn keyword Modern token\n\
else\n\
  syn keyword Legacy token\n\
endif\n";

#[test]
fn vim_provider_prefix_changes_recompile_but_later_edits_reuse_setup() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("vim", DIALECT_SYNTAX);
    let text = format!("legacy\n{}token tail", "\n".repeat(31));
    let mut req = request(&text, "vim", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let mut provider = BackendProvider::default();
    assert_eq!(finish(&mut provider, &req).runs[0].name.0, "Legacy");

    let tree = req.input.text_tree().splice(0..6, "vim9script").unwrap();
    let mut identity = req.input.identity();
    identity.revision += 1;
    req.input = SyntaxInputSnapshot::new(identity, tree);
    req.range = 0..req.input.byte_len();
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.0, "Modern");
    let fresh = finish(&mut BackendProvider::default(), &req);
    assert_eq!(
        fresh.runs, output.runs,
        "worker eviction must not change dialect"
    );

    // A changed runtime file is observed only on configuration reload or a
    // changed setup prefix. Editing the body must retain the loaded program.
    std::fs::write(fixture.0.join("vim.vim"), "unsupported command\n").unwrap();
    let len = req.input.byte_len();
    let tree = req.input.text_tree().splice(len - 4..len, "body").unwrap();
    let mut identity = req.input.identity();
    identity.revision += 1;
    req.input = SyntaxInputSnapshot::new(identity, tree);
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.0, "Modern");
}

#[test]
fn vim_provider_filename_changes_recompile_setup_for_unchanged_input() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("vim", "if expand('%:e') ==# 'h'\n  syn keyword Header token\nelse\n  syn keyword Source token\nendif\n");
    let mut req = request("token", "vim", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    req.configuration.filename = Some("/writing/first.h".into());
    let mut provider = BackendProvider::default();
    assert_eq!(finish(&mut provider, &req).runs[0].name.0, "Header");
    let old_input = req.input.identity();
    req.configuration.filename = Some("/writing/first.rs".into());
    req.configuration.generation += 1;
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.0, "Source");
    assert_eq!(req.input.identity(), old_input);
    assert_eq!(
        output.runs,
        finish(&mut BackendProvider::default(), &req).runs
    );

    let load = |filename: &str| {
        VimProgram::load_directory_with_context(
            &fixture.0,
            "vim",
            VimLoadLimits::default(),
            &VimSetupContext {
                prefix: "token".into(),
                filename: Some(filename.into()),
            },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    assert_ne!(
        load("/writing/first.h").generation,
        load("/writing/first.rs").generation
    );
    assert_eq!(
        load("/writing/first.h").generation,
        load("/writing/first.h").generation
    );
}

#[test]
fn vim_provider_position_assertions_repair_after_body_edits() {
    let _registry = treesitter::package_registry_test_guard();
    let prefix = "prefix\n".repeat(32);
    for (source, body, insertion, before, after) in [
        (
            "syn match Positioned /^\\%34lfoo/\n",
            "foo\nfoo\n",
            "foo\n",
            4..7,
            4..7,
        ),
        ("syn match Positioned /\\%4cx/\n", "aéxx\n", "b", 3..4, 0..0),
        ("syn match Positioned /\\%3cx/\n", "axxx\n", "b", 2..3, 2..3),
    ] {
        let fixture = Fixture::new("vim", source);
        let mut req = request(&format!("{prefix}{body}"), "vim", 1);
        req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
        let mut provider = BackendProvider::default();
        let ranges = |output: &SyntaxResult| {
            output
                .runs
                .iter()
                .map(|run| {
                    assert_eq!(run.name.0, "Positioned");
                    run.range.start - prefix.len()..run.range.end - prefix.len()
                })
                .collect::<Vec<_>>()
        };
        let expected = |range: Range<usize>| {
            if range.is_empty() {
                vec![]
            } else {
                vec![range]
            }
        };
        let initial = finish(&mut provider, &req);
        assert_eq!(
            initial.coverage,
            Coverage::Exact,
            "{:?}",
            initial.diagnostics
        );
        assert_eq!(ranges(&initial), expected(before), "{source}: {body}");
        let setup = provider.fallback_context.clone();

        // Edits occur after the setup prefix, so a retained program and its
        // scanner caches must respond to the new physical line/byte column.
        let tree = req
            .input
            .text_tree()
            .splice(prefix.len()..prefix.len(), insertion)
            .unwrap();
        let mut identity = req.input.identity();
        identity.revision += 1;
        req.input = SyntaxInputSnapshot::new(identity, tree);
        req.range = 0..req.input.byte_len();
        let repaired = finish(&mut provider, &req);
        assert_eq!(
            repaired.coverage,
            Coverage::Exact,
            "{:?}",
            repaired.diagnostics
        );
        assert_eq!(provider.fallback_context, setup);
        assert_eq!(
            ranges(&repaired),
            expected(after),
            "{source}: {insertion}{body}"
        );
        assert_eq!(
            repaired.runs,
            finish(&mut BackendProvider::default(), &req).runs
        );
        assert_eq!(repaired.runs, finish(&mut provider, &req).runs);
    }
}

#[test]
fn vim_provider_and_embedded_vim_ignore_registered_tree_sitter_packages() {
    let _registry = treesitter::package_registry_test_guard();
    let previous = treesitter::package_for_language("vim").ok();
    struct Restore(Option<Arc<TreeSitterPackage>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                treesitter::register_package("vim", previous).unwrap();
            } else {
                treesitter::unregister_package("vim");
            }
        }
    }
    let _restore = Restore(previous);
    let package = TreeSitterPackage::compile(
        "vim",
        1,
        tree_sitter_c::LANGUAGE.into(),
        "",
        None,
        QueryProfile::Upstream,
        &TreeSitterBudget::default(),
        None,
    )
    .unwrap();
    treesitter::register_package("vim", package).unwrap();
    let fixture = Fixture::new("vim", DIALECT_SYNTAX);
    let mut req = request("vim9script\ntoken", "vim", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let mut provider = BackendProvider::default();
    let output = finish(&mut provider, &req);
    assert_eq!(output.runs[0].name.0, "Modern");
    assert!(provider.primary.is_none());

    let mut req = request("host\nvim9script\ntoken", "c", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    let mut output = SyntaxResult::missing(&req, "");
    output.coverage = Coverage::Exact;
    let mut provider = BackendProvider::default();
    provider.children(
        &req,
        vec![InjectionRegion {
            language: "vim".into(),
            ranges: vec![5..req.input.byte_len()],
            combined: false,
            parent_input: req.input.identity(),
            package_generation: 1,
        }],
        &mut output,
        &AtomicBool::new(false),
    );
    assert_eq!(provider.children.len(), 1);
    assert!(provider.children[0].session.is_none());
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(
        output.runs[0].name.0, "Modern",
        "setup must see the embedded prefix"
    );
}
