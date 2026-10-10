use super::*;

#[test]
fn bundled_zsh_highlights_shell_options_and_commands() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "autoload -Uz compinit\nsetopt auto_cd\nrepeat 3 do\n  print \"${(U)USER}\"\ndone\n";
    let mut req = request(text, "zsh", 1);
    req.configuration.vim_directory =
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
    req.configuration.filename = Some("/writing/.zshrc".into());
    let result = finish(&mut BackendProvider::default(), &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    for (token, style) in [("autoload", "Keyword"), ("auto_cd", "Constant"), ("repeat", "Repeat")] {
        let start = text.find(token).unwrap();
        assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == style),
            "{token}: {:?}; {:?}", result.runs, result.diagnostics);
    }
}

#[test]
fn detected_jsonc_uses_native_comment_syntax_without_json_comment_errors() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "// settings\n{\"enabled\": true, /* preference */ \"name\": \"writer\"}\n";
    let mut req = request(text, "jsonc", 1);
    req.configuration.vim_directory =
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
    req.configuration.filename = Some("/writing/settings.jsonc".into());
    req.configuration.language = crate::document::syntax::detection::detect(
        &req.input, "settings.jsonc",
        &crate::document::syntax::detection::LanguageSelection::Automatic, &[],
    ).language;
    assert_eq!(req.configuration.language.as_deref(), Some("jsonc"));
    let result = finish(&mut BackendProvider::default(), &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    for token in ["// settings", "/* preference */"] {
        let start = text.find(token).unwrap();
        assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == "Comment"),
            "{token}: {:?}; {:?}", result.runs, result.diagnostics);
        assert!(!result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == "Error"));
    }
}

#[test]
fn bundled_scala_highlights_keywords_numbers_and_comments() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "object Main {\n  val answer = 42\n  // answer\n}\n";
    let mut req = request(text, "scala", 1);
    req.configuration.vim_directory =
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
    req.configuration.filename = Some("/writing/Main.scala".into());
    let result = finish(&mut BackendProvider::default(), &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    for (token, style) in [("object", "Keyword"), ("val", "Keyword"), ("42", "Number"), ("// answer", "Comment")] {
        let start = text.find(token).unwrap();
        assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == style),
            "{token}: {:?}; {:?}", result.runs, result.diagnostics);
    }
}

#[test]
fn bundled_shell_dialects_and_typescript_jsx_keep_their_native_syntax() {
    let _registry = treesitter::package_registry_test_guard();
    for (language, text, expected) in [
        ("sh", "printf 'hello'\n", "Statement"),
        ("bash", "shopt -s nullglob\n", "Statement"),
        ("ksh", "autoload helper\n", "Statement"),
        ("dash", "printf 'hello'\n", "Statement"),
        ("mksh", "autoload helper\nbind '^L=clear-screen'\n", "Statement"),
        ("csh", "setenv EDITOR viem\n", "Statement"),
        ("tcsh", "bindkey -e\n", "Statement"),
        ("tsx", "const element: JSX.Element = <div title=\"yes\">hello</div>;\n", "htmlTagName"),
    ] {
        let mut req = request(text, language, 1);
        req.configuration.vim_directory =
            concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
        let mut provider = BackendProvider::default();
        provider.configure(&req);
        // Exercise the Vim path even for a dialect with a preferred grammar.
        provider.primary = None;
        provider.primary_failure = None;
        let result = finish(&mut provider, &req);
        assert_eq!(result.coverage, Coverage::Exact, "{language}: {:?}", result.diagnostics);
        assert!(result.runs.iter().any(|run| run.name.as_str() == expected),
            "{language}: {:?}; {:?}", result.runs, result.diagnostics);
        if language == "mksh" {
            for token in ["autoload", "bind"] {
                let start = text.find(token).unwrap();
                assert!(result.runs.iter().any(|run| run.range.contains(&start) && run.name.as_str() == "Statement"),
                    "{language}: {token}: {:?}; {:?}", result.runs, result.diagnostics);
            }
        }
    }
}

fn gitcommit_request(text: &str, revision: u64) -> SyntaxRequest {
    let mut req = request(text, "gitcommit", revision);
    req.configuration.vim_directory =
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax").into();
    req.configuration.filename = Some("/writing/.git/COMMIT_EDITMSG".into());
    req
}

fn assert_gitcommit_group(result: &SyntaxResult, text: &str, token: &str, expected: &str) {
    let start = text.find(token).unwrap();
    assert!(
        result
            .runs
            .iter()
            .any(|run| run.range.contains(&start) && run.name.as_str() == expected),
        "{token:?} expected {expected}: {:?}; {:?}",
        result.runs,
        result.diagnostics
    );
}

#[test]
fn bundled_gitcommit_highlights_summary_comments_trailers_and_verbose_diff() {
    let _registry = treesitter::package_registry_test_guard();
    let text = "Support Git commit highlighting\n\nKeep comments and the verbose patch visible.\n\nSigned-off-by: Writer <writer@example.test>\n\n# Please enter the commit message.\n# On branch topic\n# Changes to be committed:\n#\tmodified:   src/main.rs\n#\n# ------------------------ >8 ------------------------\n# Do not modify or remove the line above.\ndiff --git a/src/main.rs b/src/main.rs\nindex 1234567..abcdef0 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let result = finish(&mut BackendProvider::default(), &gitcommit_request(text, 1));
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert_gitcommit_group(&result, text, "Support Git", "Keyword");
    assert_gitcommit_group(&result, text, "Signed-off-by:", "Label");
    assert_gitcommit_group(&result, text, "# Please enter", "Comment");
    assert_gitcommit_group(&result, text, "topic", "Special");
    assert_gitcommit_group(&result, text, "src/main.rs", "Constant");
    assert_gitcommit_group(&result, text, "+new", "Added");
    assert_gitcommit_group(&result, text, "-old", "Removed");
}

#[test]
fn bundled_gitcommit_uses_actual_tail_for_custom_comment_character() {
    let _registry = treesitter::package_registry_test_guard();
    for ending in ["", "\n"] {
        let text = format!(
            "A commit summary\n\n{}; Please enter a message.{ending}",
            "body\n".repeat(40)
        );
        let result = finish(
            &mut BackendProvider::default(),
            &gitcommit_request(&text, 1),
        );
        assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
        assert_gitcommit_group(&result, &text, "; Please enter", "Comment");
    }
}

#[test]
fn bundled_gitcommit_setup_reuses_large_verbose_patch_after_scissors() {
    let _registry = treesitter::package_registry_test_guard();
    let prefix = "Summary\n\nBody ב\n; ------------------------ >8 ------------------------\n; Everything below is ignored.\ndiff --git a/file b/file\n@@ -1 +1 @@\n";
    let text = format!("{prefix}{}", "unchanged patch body\n".repeat(70_000));
    let fixture = Fixture::new(
        "gitcommit",
        include_str!("../../../../../../../assets/vim/runtime/syntax/gitcommit.vim"),
    );
    std::fs::write(
        fixture.0.join("diff.vim"),
        include_str!("../../../../../../../assets/vim/runtime/syntax/diff.vim"),
    )
    .unwrap();
    let mut req = gitcommit_request(&text, 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().into();
    req.range = 0..prefix.len();
    let mut provider = BackendProvider::default();
    let result = finish(&mut provider, &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert_gitcommit_group(&result, &text, "; Everything below", "Comment");
    let setup = provider.fallback_context.clone();
    std::fs::write(fixture.0.join("gitcommit.vim"), "unsupported command\n").unwrap();
    let at = text.len() - 100;
    req.input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..req.input.identity()
        },
        req.input.text_tree().splice(at..at + 1, "X").unwrap(),
    );
    let result = finish(&mut provider, &req);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    assert_eq!(
        provider.fallback_context.as_ref().unwrap().prefix,
        setup.as_ref().unwrap().prefix
    );
    assert_eq!(
        provider
            .fallback_context
            .as_ref()
            .unwrap()
            .input
            .as_ref()
            .unwrap()
            .identity(),
        req.input.identity()
    );
}

#[test]
fn vim_provider_retains_failed_setup_until_a_queried_tail_changes() {
    let _registry = treesitter::package_registry_test_guard();
    let source = "if getline(line('$')) ==# 'bad'\n unsupported command\nelse\n syn keyword Valid token\nendif\n";
    let fixture = Fixture::new("fixture", source);
    let text = format!("token\n{}bad", "body\n".repeat(40));
    let mut req = request(&text, "fixture", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().into();
    let mut provider = BackendProvider::default();
    let failed = finish(&mut provider, &req);
    assert_eq!(failed.coverage, Coverage::Missing);
    // A file reload would produce a different error. Editing unqueried body
    // content must preserve the cached failure instead.
    std::fs::write(fixture.0.join("fixture.vim"), "totally different failure\n").unwrap();
    let at = text.len() - 8;
    req.input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..req.input.identity()
        },
        req.input.text_tree().splice(at..at + 1, "B").unwrap(),
    );
    assert_eq!(finish(&mut provider, &req).diagnostics, failed.diagnostics);
    assert_eq!(
        provider
            .fallback_context
            .as_ref()
            .unwrap()
            .input
            .as_ref()
            .unwrap()
            .identity(),
        req.input.identity()
    );
    std::fs::write(fixture.0.join("fixture.vim"), source).unwrap();
    let end = req.input.byte_len();
    req.input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 3,
            ..req.input.identity()
        },
        req.input.text_tree().splice(end - 3..end, "good").unwrap(),
    );
    req.range = 0..req.input.byte_len();
    let recovered = finish(&mut provider, &req);
    assert_eq!(
        recovered.coverage,
        Coverage::Exact,
        "{:?}",
        recovered.diagnostics
    );
    assert_eq!(recovered.runs[0].name.as_str(), "Valid");
}

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
    assert_eq!(output.runs[0].name.as_str(), "Native");
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
    assert_eq!(finish(&mut provider, &req).runs[0].name.as_str(), "Legacy");

    let tree = req.input.text_tree().splice(0..6, "vim9script").unwrap();
    let mut identity = req.input.identity();
    identity.revision += 1;
    req.input = SyntaxInputSnapshot::new(identity, tree);
    req.range = 0..req.input.byte_len();
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.as_str(), "Modern");
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
    assert_eq!(output.runs[0].name.as_str(), "Modern");
}

#[test]
fn vim_provider_filename_changes_recompile_setup_for_unchanged_input() {
    let _registry = treesitter::package_registry_test_guard();
    let fixture = Fixture::new("vim", "if expand('%:e') ==# 'h'\n  syn keyword Header token\nelse\n  syn keyword Source token\nendif\n");
    let mut req = request("token", "vim", 1);
    req.configuration.vim_directory = fixture.0.to_str().unwrap().to_owned();
    req.configuration.filename = Some("/writing/first.h".into());
    let mut provider = BackendProvider::default();
    assert_eq!(finish(&mut provider, &req).runs[0].name.as_str(), "Header");
    let old_input = req.input.identity();
    req.configuration.filename = Some("/writing/first.rs".into());
    req.configuration.generation += 1;
    let output = finish(&mut provider, &req);
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostics);
    assert_eq!(output.runs[0].name.as_str(), "Source");
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
                input: None,
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
                    assert_eq!(run.name.as_str(), "Positioned");
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
        assert_eq!(
            provider.fallback_context.as_ref().unwrap().prefix,
            setup.as_ref().unwrap().prefix
        );
        assert_eq!(
            provider
                .fallback_context
                .as_ref()
                .unwrap()
                .input
                .as_ref()
                .unwrap()
                .identity(),
            req.input.identity()
        );
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
    assert_eq!(output.runs[0].name.as_str(), "Modern");
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
        output.runs[0].name.as_str(),
        "Modern",
        "setup must see the embedded prefix"
    );
}
