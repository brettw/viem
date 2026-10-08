use super::*;
use crate::document::FormattedTextTree;

fn input(text: &str, revision: u64) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 1,
            revision,
            generation: 1,
        },
        FormattedTextTree::try_from_text(text).unwrap(),
    )
}
fn generous() -> TreeSitterBudget {
    TreeSitterBudget {
        max_progress_callbacks: 2_000_000,
        max_input_bytes_supplied: 512 * 1024 * 1024,
        max_edit_tree_nodes: usize::MAX,
        max_edit_root_children: usize::MAX,
        slice_duration: Duration::from_secs(30),
        ..TreeSitterBudget::default()
    }
}
fn parsed(
    session: &mut TreeSitterSession,
    input: SyntaxInputSnapshot,
    edits: &[SyntaxInputEdit],
) -> (ParseSnapshot, TreeSitterWork) {
    match session
        .parse(input, edits, &[], &generous(), &AtomicBool::new(false))
        .unwrap()
    {
        ParseOutcome::Complete { snapshot, work } => (snapshot, work),
        ParseOutcome::Yielded { work } => panic!("unexpected yield: {work:?}"),
    }
}
fn custom(source: &str, profile: QueryProfile) -> Arc<TreeSitterPackage> {
    TreeSitterPackage::compile(
        "test",
        7,
        tree_sitter_c::LANGUAGE.into(),
        source,
        None,
        profile,
        &generous(),
        None,
    )
    .unwrap()
}

#[test]
fn all_bundled_nvim_queries_compile() {
    let failures: Vec<_> = BUNDLED_LANGUAGES
        .iter()
        .filter_map(|id| {
            TreeSitterPackage::bundled(id)
                .err()
                .map(|error| format!("{id}: {error}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_bundled_grammar_compiles_queries_parses_and_highlights() {
    let fixtures = [
        ("c", "int main(void) { return 42; }\n"),
        (
            "cpp",
            "class Example { public: int value() { return 42; } };\n",
        ),
        ("rust", "fn main() { let answer = 42; }\n"),
        ("swift", "func answer() -> Int { return 42 }\n"),
        (
            "objc",
            "@interface Example : NSObject\n- (int)answer;\n@end\n",
        ),
        (
            "c_sharp",
            "class Example { public int Answer() { return 42; } }\n",
        ),
        (
            "javascript",
            "const element = <div title=\"yes\">hello</div>;\n",
        ),
        (
            "typescript",
            "function answer(value: number): number { return value; }\n",
        ),
        ("tsx", "const element: JSX.Element = <div>hello</div>;\n"),
        ("python", "def answer():\n    return 42\n"),
        ("json", "{\"answer\": 42, \"enabled\": true, \"items\": [null, \"text\"]}\n"),
        ("markdown", "# Heading\n\n- [x] item\n"),
        ("markdown_inline", "**bold** and *italic* with [link](https://example.com)"),
    ];
    for (language, text) in fixtures {
        let package =
            TreeSitterPackage::bundled(language).unwrap_or_else(|e| panic!("{language}: {e}"));
        let mut session = TreeSitterSession::new(package).unwrap();
        let (tree, _) = parsed(&mut session, input(text, 0), &[]);
        assert!(
            !tree.has_errors(),
            "{language}: {}",
            tree.tree.root_node().to_sexp()
        );
        let result = highlight(&tree, 0..text.len(), &generous(), &AtomicBool::new(false));
        assert_eq!(
            result.coverage,
            Coverage::Exact,
            "{language}: {:?}",
            result.diagnostic
        );
        assert!(!result.runs.is_empty(), "{language}");
        assert!(result
            .runs
            .windows(2)
            .all(|p| p[0].range.end <= p[1].range.start));
    }
}

#[test]
fn nvim_language_captures_include_directives_and_inherited_queries() {
    let fixtures = [
        ("c_sharp", "#if DEBUG\nclass Example {}\n#elif TRACE\nclass Other {}\n#else\nclass Release {}\n#endif\n",
         vec![("#if", "Keyword.directive"), ("DEBUG", "Constant"), ("#elif", "Keyword.directive"),
              ("#else", "Keyword.directive"), ("#endif", "Keyword.directive"), ("Example", "Type")]),
        ("rust", "fn main() { debug_assert!(true); let x = Some(1); }\n",
         vec![("debug_assert", "Keyword.exception"), ("Some", "Constant.builtin")]),
        ("swift", "#if DEBUG\nlet answer = 42\n#endif\n",
         vec![("#if DEBUG", "Keyword.directive"), ("42", "Number")]),
        ("objc", "#import <Foundation/Foundation.h>\n@interface Example : NSObject\n@end\n",
         vec![("#import", "Keyword.import"), ("Example", "Type")]),
        ("javascript", "const value = <div title=\"hello\">text</div>;\n",
         vec![("const", "Keyword"), ("div", "Tag.builtin"), ("title", "Tag.attribute")]),
        ("typescript", "const count: number = 42;\n",
         vec![("const", "Keyword"), ("number", "Type.builtin"), ("42", "Number")]),
        ("tsx", "const value: number = 42; const element = <div title=\"hello\" />;\n",
         vec![("const", "Keyword"), ("number", "Type.builtin"), ("div", "Tag.builtin"), ("title", "Tag.attribute")]),
        ("python", "@staticmethod\ndef answer():\n    return True\n",
         vec![("staticmethod", "Attribute.builtin"), ("answer", "Function"), ("True", "Boolean")]),
    ];
    for (language, text, expected) in fixtures {
        let mut session =
            TreeSitterSession::new(TreeSitterPackage::bundled(language).unwrap()).unwrap();
        let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
        assert!(
            !snapshot.has_errors(),
            "{language}: {}",
            snapshot.tree.root_node().to_sexp()
        );
        let result = highlight(
            &snapshot,
            0..text.len(),
            &generous(),
            &AtomicBool::new(false),
        );
        assert_eq!(
            result.coverage,
            Coverage::Exact,
            "{language}: {:?}",
            result.diagnostic
        );
        for (needle, expected) in expected {
            let offset = text.find(needle).unwrap();
            let name = result
                .runs
                .iter()
                .find(|run| run.range.contains(&offset))
                .map(|run| run.name.as_str());
            assert_eq!(name, Some(expected), "{language}: {needle}");
        }
    }
}

#[test]
fn json_styles_preserve_quotes_escapes_and_comments() {
    let text = "{\"key\": \"say \\\"hi\\\"\", \"n\": -1.5e2, \"b\": true, \"a\": [false, null]} // comment\n";
    let mut session = TreeSitterSession::new(TreeSitterPackage::bundled("json").unwrap()).unwrap();
    let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
    assert!(!snapshot.has_errors());
    let result = highlight(
        &snapshot,
        0..text.len(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostic);
    for (needle, expected) in [
        ("\"key\"", "Property"),
        ("\"say", "String"),
        ("\\\"hi", "String.escape"),
        ("-1.5e2", "Number"),
        ("true", "Boolean"),
        ("false", "Boolean"),
        ("null", "Constant.builtin"),
        ("{", "Punctuation.bracket"),
        (":", "Punctuation.delimiter"),
        ("// comment", "Comment"),
    ] {
        let offset = text.find(needle).unwrap();
        assert_eq!(
            result
                .runs
                .iter()
                .find(|r| r.range.contains(&offset))
                .map(|r| r.name.as_str()),
            Some(expected),
            "{needle}"
        );
    }
    assert!(!result.runs.iter().any(|run| run.name.as_str() == "Conceal"));
    assert_eq!(snapshot.input().slice(0..text.len()).unwrap(), text);
}

#[test]
fn all_nvim_packages_match_fresh_highlighting_after_incremental_edits() {
    for (language, text, old_token, new_token) in [
        ("c", "int value = 42;\n", "42", "1.5"),
        ("cpp", "auto value = nullptr;\n", "nullptr", "false"),
        (
            "rust",
            "fn main() { debug_assert!(true); }\n",
            "debug_assert",
            "println",
        ),
        ("swift", "let value = true\n", "true", "nil"),
        (
            "objc",
            "@interface Example : NSObject\n@end\n",
            "Example",
            "Changed",
        ),
        (
            "c_sharp",
            "#if DEBUG\nclass Example {}\n#endif\n",
            "DEBUG",
            "true",
        ),
        (
            "javascript",
            "const value = <div title=\"hi\" />;\n",
            "title",
            "href",
        ),
        (
            "typescript",
            "const value: number = 42;\n",
            "number",
            "string",
        ),
        (
            "tsx",
            "const value = <div title=\"hi\" />;\n",
            "title",
            "href",
        ),
        ("python", "value = True\n", "True", "None"),
        ("json", "{\"value\": true}\n", "true", "null"),
        ("markdown", "# Heading\n\n- [ ] item\n", "#", "##"),
        ("markdown_inline", "**strong** and *italic*", "**strong**", "`strong`"),
    ] {
        let package = TreeSitterPackage::bundled(language).unwrap();
        let mut session = TreeSitterSession::new(package.clone()).unwrap();
        let old = input(text, 0);
        let (original, _) = parsed(&mut session, old.clone(), &[]);
        let original_runs = highlight(
            &original,
            0..text.len(),
            &generous(),
            &AtomicBool::new(false),
        )
        .runs;
        let at = text.find(old_token).unwrap();
        let changed = text.replacen(old_token, new_token, 1);
        let new = input(&changed, 1);
        let edit = SyntaxInputEdit::new(
            &old,
            &new,
            at..at + old_token.len(),
            at..at + new_token.len(),
        )
        .unwrap();
        let (incremental, _) = parsed(&mut session, new.clone(), &[edit]);
        let (fresh, _) = parsed(&mut TreeSitterSession::new(package).unwrap(), new, &[]);
        assert!(!incremental.has_errors(), "{language}");
        let result = highlight(
            &incremental,
            0..changed.len(),
            &generous(),
            &AtomicBool::new(false),
        );
        assert_eq!(
            result.coverage,
            Coverage::Exact,
            "{language}: {:?}",
            result.diagnostic
        );
        assert_eq!(
            result.runs,
            highlight(
                &fresh,
                0..changed.len(),
                &generous(),
                &AtomicBool::new(false)
            )
            .runs,
            "{language}"
        );
        assert_eq!(
            original_runs,
            highlight(
                &original,
                0..text.len(),
                &generous(),
                &AtomicBool::new(false)
            )
            .runs,
            "retained {language}"
        );
    }
}

#[test]
fn json_large_document_edits_and_regional_queries_stay_local() {
    let text = format!("[\n{}null\n]\n", "{\"key\": true},\n".repeat(10_000));
    let package = TreeSitterPackage::bundled("json").unwrap();
    let mut session = TreeSitterSession::new(package.clone()).unwrap();
    let old = input(&text, 0);
    let (original, _) = parsed(&mut session, old.clone(), &[]);
    assert!(!original.has_errors());
    let at = text.find("true").unwrap();
    let changed = text.replacen("true", "null", 1);
    let new = input(&changed, 1);
    let edit = SyntaxInputEdit::new(&old, &new, at..at + 4, at..at + 4).unwrap();
    let (snapshot, work) = parsed(&mut session, new, &[edit]);
    assert!(work.input_bytes_supplied < 32 * 1024, "{work:?}");
    let result = highlight(&snapshot, 0..32, &generous(), &AtomicBool::new(false));
    assert_eq!(result.coverage, Coverage::Exact);
    assert!(result.work.query_matches < 128, "{:?}", result.work);
    assert_eq!(
        result
            .runs
            .iter()
            .find(|r| r.range.contains(&at))
            .unwrap()
            .name
            .as_str(),
        "Constant.builtin"
    );
}

#[test]
fn neovim_contains_matches_literal_text_with_all_any_and_negation() {
    for (predicate, expected) in [
        ("contains? @name \"assert\" \"debug\"", vec!["debug_assert"]),
        (
            "any-contains? @name \"assert\" \"print\"",
            vec!["debug_assert", "assert_eq", "print"],
        ),
        (
            "not-contains? @name \"assert\" \"debug\"",
            vec!["assert_eq", "print", "other"],
        ),
        (
            "not-any-contains? @name \"assert\" \"print\"",
            vec!["other"],
        ),
        ("contains? @name \".*\"", vec![]),
        (
            "contains? @name \"\"",
            vec!["debug_assert", "assert_eq", "print", "other"],
        ),
    ] {
        let package = custom(
            &format!("((identifier) @name (#{predicate}))"),
            QueryProfile::NeovimV1,
        );
        let mut session = TreeSitterSession::new(package).unwrap();
        let text = "int debug_assert, assert_eq, print, other;";
        let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
        let result = highlight(
            &snapshot,
            0..text.len(),
            &generous(),
            &AtomicBool::new(false),
        );
        assert_eq!(result.coverage, Coverage::Exact);
        assert_eq!(
            result
                .runs
                .iter()
                .map(|r| &text[r.range.clone()])
                .collect::<Vec<_>>(),
            expected,
            "{predicate}"
        );
        let limited = TreeSitterBudget {
            max_predicate_steps: 1,
            ..generous()
        };
        let result = highlight(&snapshot, 0..text.len(), &limited, &AtomicBool::new(false));
        assert_ne!(
            result.coverage,
            Coverage::Exact,
            "predicate exhaustion cannot claim a complete result"
        );
    }
}

#[test]
fn neovim_ui_metadata_never_hides_source_or_accepts_unknown_properties() {
    let package = custom(
        concat!(
            "(identifier) @variable\n",
            "((identifier) @conceal (#set! conceal \"\"))\n",
            "((identifier) @_node (#set! conceal_lines \"\"))\n",
            "((identifier) @_node (#set! @_node bo.commentstring \"// %s\"))\n",
            "((identifier) @_node (#set! @_node url @_node))\n",
        ),
        QueryProfile::NeovimV1,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let (snapshot, _) = parsed(&mut session, input("int value;", 0), &[]);
    let result = highlight(&snapshot, 0..10, &generous(), &AtomicBool::new(false));
    assert_eq!(result.coverage, Coverage::Exact);
    assert_eq!(result.runs.len(), 1);
    assert_eq!(result.runs[0].name.as_str(), "Variable");
    assert_eq!(snapshot.input().slice(0..10).unwrap(), "int value;");
    for query in [
        "((identifier) @name (#set! @name arbitrary \"x\"))",
        "((identifier) @name (#set! conceal))",
        "((identifier) @name (#set! conceal_lines))",
        "((identifier) @name (#set! @name url \"arbitrary\"))",
        "((identifier) @name (#set! @name url))",
        "((identifier) @name (#set! bo.commentstring \"x\"))",
    ] {
        assert!(
            TreeSitterPackage::compile(
                "bad",
                1,
                tree_sitter_c::LANGUAGE.into(),
                query,
                None,
                QueryProfile::NeovimV1,
                &generous(),
                None
            )
            .is_err(),
            "{query}"
        );
    }
}

#[test]
fn same_shape_text_edits_recompute_predicates_and_leave_published_tree_unchanged() {
    let package = custom(
        r#"((identifier) @constant (#eq? @constant "RIGHT"))"#,
        QueryProfile::Upstream,
    );
    let mut session = TreeSitterSession::new(package.clone()).unwrap();
    let old = input("int WRONG;\n", 0);
    let new = input("int RIGHT;\n", 1);
    let (before, _) = parsed(&mut session, old.clone(), &[]);
    assert!(highlight(
        &before,
        0..old.byte_len(),
        &generous(),
        &AtomicBool::new(false)
    )
    .runs
    .is_empty());
    let edit = SyntaxInputEdit::new(&old, &new, 4..9, 4..9).unwrap();
    let (after, work) = parsed(&mut session, new.clone(), &[edit]);
    assert!(work.incremental);
    assert!(!after.changed_ranges().is_empty());
    let result = highlight(
        &after,
        0..new.byte_len(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert_eq!(result.runs[0].range, 4..9);
    let mut fresh = TreeSitterSession::new(package).unwrap();
    let (fresh, _) = parsed(&mut fresh, new, &[]);
    assert_eq!(
        result.runs,
        highlight(&fresh, 0..11, &generous(), &AtomicBool::new(false)).runs
    );
    assert!(highlight(
        &before,
        0..old.byte_len(),
        &generous(),
        &AtomicBool::new(false)
    )
    .runs
    .is_empty());
}

#[test]
fn queries_validate_host_capabilities_before_activation() {
    for query in [
        r#"((identifier) @name (#lua-match? @name "x"))"#,
        r#"((identifier) @name (#is-not? local))"#,
        r#"((identifier) @name (#unknown! @name))"#,
        "; inherits: missing\n(identifier) @name",
        r#"((identifier) @name (#set! conceal "x"))"#,
        r#"((identifier) @name (#set! conceal_lines ""))"#,
    ] {
        assert!(
            matches!(
                TreeSitterPackage::compile(
                    "bad",
                    1,
                    tree_sitter_c::LANGUAGE.into(),
                    query,
                    None,
                    QueryProfile::Upstream,
                    &generous(),
                    None
                ),
                Err(TreeSitterError::UnsupportedQuery(_))
            ),
            "{query}"
        );
    }
    assert!(TreeSitterPackage::compile(
        "bad",
        1,
        tree_sitter_c::LANGUAGE.into(),
        r#"((identifier) @name (#match? @name "\\m\\%Vfoo"))"#,
        None,
        QueryProfile::NeovimV1,
        &generous(),
        None
    )
    .is_err());
}

#[test]
fn neovim_query_uses_very_magic_and_rejects_quantified_handler_ambiguity() {
    let package = custom(
        r#"((identifier) @name (#match? @name "^[A-Z]+$"))"#,
        QueryProfile::NeovimV1,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let (snapshot, _) = parsed(&mut session, input("int ABC; int Abc;", 0), &[]);
    let output = highlight(&snapshot, 0..16, &generous(), &AtomicBool::new(false));
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
    assert_eq!(output.runs.len(), 1);
    assert_eq!(output.runs[0].range, 4..7);
    let quantified = r#"((argument_list (identifier)+ @name) (#eq? @name "x"))"#;
    assert!(matches!(
        TreeSitterPackage::compile(
            "quantified",
            1,
            tree_sitter_c::LANGUAGE.into(),
            quantified,
            None,
            QueryProfile::NeovimV1,
            &generous(),
            None
        ),
        Err(TreeSitterError::UnsupportedQuery(_))
    ));
}

#[test]
fn neovim_lua_match_uses_lua_patterns_and_rejects_malformed_ones() {
    let package = custom(
        concat!(
            r#"((identifier) @constant (#lua-match? @constant "^[A-Z][A-Z0-9_]+$"))"#,
            "\n",
            r#"((identifier) @balanced (#lua-match? @balanced "^%f[%a]x%d$"))"#,
            "\n",
            r#"((identifier) @other (#not-lua-match? @other "^[A-Zx]"))"#,
        ),
        QueryProfile::NeovimV1,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let text = "int MAX_SIZE; int x1; int lower;";
    let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
    let output = highlight(&snapshot, 0..text.len(), &generous(), &AtomicBool::new(false));
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
    let names = output
        .runs
        .iter()
        .map(|run| (&text[run.range.clone()], run.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [("MAX_SIZE", "Constant"), ("x1", "Balanced"), ("lower", "Other")]
    );
    assert!(matches!(
        TreeSitterPackage::compile(
            "malformed",
            1,
            tree_sitter_c::LANGUAGE.into(),
            r#"((identifier) @name (#lua-match? @name "[a"))"#,
            None,
            QueryProfile::NeovimV1,
            &generous(),
            None
        ),
        Err(TreeSitterError::InvalidQuery(_))
    ));
}

#[test]
fn injection_self_and_captured_languages() {
    let package = TreeSitterPackage::compile(
        "cpp",
        1,
        tree_sitter_cpp::LANGUAGE.into(),
        "(identifier) @variable",
        Some(concat!(
            "((preproc_arg) @injection.content (#set! injection.self))\n",
            "(raw_string_literal delimiter: (raw_string_delimiter) @injection.language",
            " (raw_string_content) @injection.content)",
        )),
        QueryProfile::NeovimV1,
        &generous(),
        None,
    )
    .unwrap();
    let mut session = TreeSitterSession::new(package).unwrap();
    let text = "#define A (b)\nauto s = R\"sql(x)sql\"; auto t = R\"a.b(y)a.b\";\n";
    let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
    let output = highlight(&snapshot, 0..text.len(), &generous(), &AtomicBool::new(false));
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
    let regions = output
        .injections
        .iter()
        .map(|region| (region.language.as_str(), &text[region.ranges[0].clone()]))
        .collect::<Vec<_>>();
    // A delimiter that cannot name a language injects nothing.
    assert_eq!(regions, [("cpp", "(b)"), ("sql", "x")]);
}

#[test]
fn query_reload_preserves_native_tree_and_published_package() {
    let mut session =
        TreeSitterSession::new(custom("(identifier) @old", QueryProfile::Upstream)).unwrap();
    let input = input("int value;", 1);
    let (old, _) = parsed(&mut session, input.clone(), &[]);
    assert!(session
        .replace_package(custom("(identifier) @new", QueryProfile::Upstream))
        .unwrap());
    let (new, work) = parsed(&mut session, input.clone(), &[]);
    assert_eq!(work.input_callbacks, 0);
    assert_eq!(work.progress_callbacks, 0);
    assert_eq!(
        highlight(
            &old,
            0..input.byte_len(),
            &generous(),
            &AtomicBool::new(false)
        )
        .runs[0]
            .name
            .as_str(),
        "Old"
    );
    assert_eq!(
        highlight(
            &new,
            0..input.byte_len(),
            &generous(),
            &AtomicBool::new(false)
        )
        .runs[0]
            .name
            .as_str(),
        "New"
    );
    assert!(!session
        .replace_package(TreeSitterPackage::bundled("python").unwrap())
        .unwrap());
    assert!(session.completed().is_none());
}

#[test]
fn native_edit_preflight_keeps_large_completed_tree_queryable() {
    let mut session =
        TreeSitterSession::new(custom("(identifier) @name", QueryProfile::Upstream)).unwrap();
    let old = input("int a; int b; int c;", 1);
    let (published, _) = parsed(&mut session, old.clone(), &[]);
    let next = input("int z; int b; int c;", 2);
    let edit = SyntaxInputEdit::new(&old, &next, 4..5, 4..5).unwrap();
    let budget = TreeSitterBudget {
        max_edit_root_children: 1,
        ..generous()
    };
    assert_eq!(
        session
            .parse(next, &[edit], &[], &budget, &AtomicBool::new(false))
            .unwrap_err(),
        TreeSitterError::Limit("native edit preflight budget")
    );
    assert_eq!(
        session.completed().unwrap().input_identity(),
        old.identity()
    );
    assert_eq!(
        highlight(
            &published,
            0..old.byte_len(),
            &generous(),
            &AtomicBool::new(false)
        )
        .coverage,
        Coverage::Exact
    );
}

#[test]
fn priorities_resolve_before_names_and_empty_coverage_is_exact() {
    let package = custom(
        r#"
      ((identifier) @defined (#set! priority "10"))
      ((identifier) @missing.name (#set! priority "20"))
    "#,
        QueryProfile::Upstream,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let (tree, _) = parsed(&mut session, input("int value;", 0), &[]);
    let output = highlight(&tree, 0..10, &generous(), &AtomicBool::new(false));
    assert_eq!(output.runs.len(), 1);
    assert_eq!(output.runs[0].name.as_str(), "Missing.name");
    assert!(output.runs[0].origin.ends_with(":@missing.name"));
    let empty = highlight(&tree, 0..3, &generous(), &AtomicBool::new(false));
    assert_eq!(empty.coverage, Coverage::Exact);
    assert!(empty.runs.is_empty());
}

#[test]
fn capture_names_are_canonical_but_diagnostics_keep_the_query_spelling() {
    let package = custom("(comment) @comment.documentation\n(identifier) @custom.HTTPHeader", QueryProfile::Upstream);
    let mut session = TreeSitterSession::new(package).unwrap();
    let text = "/* doc */ int value;";
    let (snapshot, _) = parsed(&mut session, input(text, 1), &[]);
    let output = highlight(&snapshot, 0..text.len(), &generous(), &AtomicBool::new(false));
    assert!(output.runs.iter().any(|run| run.name.as_str() == "Comment.documentation"
        && run.origin.ends_with(":@comment.documentation")));
    assert!(output.runs.iter().any(|run| run.name.as_str() == "Custom.HTTPHeader"
        && run.origin.ends_with(":@custom.HTTPHeader")));
    assert!(output.runs.iter().all(|run| !run.name.0.starts_with('@')));
}

#[test]
fn nested_captures_keep_innermost_styles_and_ignore_spell_controls() {
    let package = custom(
        "(comment) @comment @spell\n(number_literal) @number\n(translation_unit) @outer",
        QueryProfile::Upstream,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let text = "/* note */ int x = 1;";
    let (snapshot, _) = parsed(&mut session, input(text, 1), &[]);
    let output = highlight(
        &snapshot,
        0..text.len(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert!(output
        .runs
        .iter()
        .any(|run| run.name.as_str() == "Comment" && run.range == (0..10)));
    assert!(output.runs.iter().any(|run| run.name.as_str() == "Number"));
    assert!(!output.runs.iter().any(|run| run.name.as_str() == "Spell"));
    let clipped = highlight(&snapshot, 19..20, &generous(), &AtomicBool::new(false));
    assert_eq!(clipped.runs[0].name.as_str(), "Number");
}

#[test]
#[ignore = "diagnostic regional-query timings; run serially in release mode"]
fn regional_query_scroll_timings() {
    let fixtures = [
        ("rust", "fn example(value: usize) -> usize {\n    let next = value + 42; // comment\n    next\n}\n"),
        ("typescript", "function example(value: number): number {\n    const next = value + 42; // comment\n    return next;\n}\n"),
        ("python", "def example(value):\n    next_value = value + 42 # comment\n    return next_value\n"),
    ];
    for (language, fixture) in fixtures {
        let text = fixture.repeat(4_000);
        let mut session =
            TreeSitterSession::new(TreeSitterPackage::bundled(language).unwrap()).unwrap();
        let (snapshot, _) = parsed(&mut session, input(&text, 1), &[]);
        let mut samples = Vec::new();
        let mut captures = 0;
        for _ in 0..7 {
            let started = Instant::now();
            for page in 0..40 {
                let start = (2_000 + page * 20) * fixture.len();
                let output = highlight(
                    &snapshot,
                    start..start + fixture.len() * 20,
                    &generous(),
                    &AtomicBool::new(false),
                );
                assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
                captures += output.work.query_captures;
                std::hint::black_box(output);
            }
            samples.push(started.elapsed().as_secs_f64() * 1_000.0 / 40.0);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!("regional query {language}: median {:.3} ms/page, captures {captures}, samples {samples:?}", samples[3]);
    }
}

#[test]
fn capture_sweep_matches_priority_extent_pattern_and_serial_precedence() {
    let names = ["comment", "Comment", "type.name"];
    let mut seed = 7u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) as usize
    };
    let mut runs = Vec::new();
    for serial in 0..512 {
        let start = next() % 128;
        let length = next() % 16 + 1;
        runs.push(RankedRun {
            range: start..start + length,
            name: names[next() % names.len()],
            priority: (next() % 3) as i32 - 1,
            pattern: next() % 4,
            serial,
            // Original captures can be wider than the requested viewport.
            extent: length + next() % 3,
        });
    }
    let expected: Vec<_> = (0..144)
        .map(|at| {
            runs.iter()
                .filter(|run| run.range.contains(&at))
                .max_by_key(|run| {
                    (
                        run.priority,
                        std::cmp::Reverse(run.extent),
                        run.pattern,
                        run.serial,
                    )
                })
                .map(|run| (run.name, run.priority))
        })
        .collect();
    let budget = generous();
    let cancelled = AtomicBool::new(false);
    let native = native::Account::default();
    let control = Control::new(&budget, &cancelled, &native);
    let actual = coalesce(runs, "test", &control).unwrap();
    for (at, expected) in expected.into_iter().enumerate() {
        let actual = actual.iter().find(|run| run.range.contains(&at));
        assert_eq!(
            actual.map(|run| (&*run.origin, run.priority)),
            expected
                .map(|(name, priority)| (format!("treesitter:test:@{name}"), priority))
                .as_ref()
                .map(|(origin, priority)| (origin.as_str(), *priority)),
            "byte {at}",
        );
    }
    assert!(actual.windows(2).all(|pair| {
        pair[0].range.end <= pair[1].range.start
            && (pair[0].range.end != pair[1].range.start
                || pair[0].origin != pair[1].origin
                || pair[0].priority != pair[1].priority)
    }));
}

#[test]
fn capture_sweep_preserves_clipped_extents_merges_winners_and_honors_cancellation() {
    let captures = || {
        vec![
            RankedRun {
                range: 0..2,
                name: "narrow",
                priority: 1,
                pattern: 0,
                serial: 0,
                extent: 2,
            },
            RankedRun {
                range: 0..2,
                name: "wide",
                priority: 1,
                pattern: 9,
                serial: 1,
                extent: 20,
            },
            RankedRun {
                range: 2..4,
                name: "narrow",
                priority: 1,
                pattern: 0,
                serial: 2,
                extent: 2,
            },
        ]
    };
    let budget = generous();
    let cancelled = AtomicBool::new(false);
    let native = native::Account::default();
    let control = Control::new(&budget, &cancelled, &native);
    let output = coalesce(captures(), "test", &control).unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].range, 0..4);
    assert_eq!(output[0].name.as_str(), "Narrow");
    cancelled.store(true, Ordering::Relaxed);
    assert_eq!(
        coalesce(captures(), "test", &control),
        Err(TreeSitterError::Cancelled)
    );
}

#[test]
fn parse_yields_resumes_and_supersedes_only_frozen_inputs() {
    let mut session = TreeSitterSession::new(TreeSitterPackage::bundled("c").unwrap()).unwrap();
    let first = input(&"int x = 1;\n".repeat(20_000), 0);
    let bounded = TreeSitterBudget {
        max_progress_callbacks: 0,
        ..generous()
    };
    assert!(matches!(
        session
            .parse(first.clone(), &[], &[], &bounded, &AtomicBool::new(false))
            .unwrap(),
        ParseOutcome::Yielded { .. }
    ));
    assert!(session.completed().is_none());
    let (complete, _) = parsed(&mut session, first.clone(), &[]);
    assert_eq!(complete.input_identity(), first.identity());
    let second = input("int replacement;\n", 1);
    let edit =
        SyntaxInputEdit::new(&first, &second, 0..first.byte_len(), 0..second.byte_len()).unwrap();
    let (complete, _) = parsed(&mut session, second.clone(), &[edit]);
    assert_eq!(complete.input_identity(), second.identity());
    assert_eq!(complete.input().byte_len(), second.byte_len());
}

#[test]
fn cancelled_and_limited_queries_never_publish_partial_exact_colors() {
    let mut session =
        TreeSitterSession::new(custom("(identifier) @name", QueryProfile::Upstream)).unwrap();
    let text = "int first; int second; int third;";
    let (tree, _) = parsed(&mut session, input(text, 0), &[]);
    for (budget, cancel) in [
        (generous(), AtomicBool::new(true)),
        (
            TreeSitterBudget {
                max_matches: 1,
                ..generous()
            },
            AtomicBool::new(false),
        ),
        (
            TreeSitterBudget {
                max_captures: 0,
                ..generous()
            },
            AtomicBool::new(false),
        ),
        (
            TreeSitterBudget {
                max_output_bytes: 1,
                ..generous()
            },
            AtomicBool::new(false),
        ),
    ] {
        let result = highlight(&tree, 0..text.len(), &budget, &cancel);
        assert_eq!(result.coverage, Coverage::Missing);
        assert!(result.runs.is_empty());
        assert!(result.diagnostic.is_some());
    }
}

#[test]
fn huge_capture_predicates_do_not_flatten_an_unbounded_line() {
    let package = custom(
        r#"((comment) @comment (#match? @comment "needle"))"#,
        QueryProfile::Upstream,
    );
    let mut session = TreeSitterSession::new(package).unwrap();
    let text = format!("/* {} */", "x".repeat(2 * 1024 * 1024));
    let (tree, work) = parsed(&mut session, input(&text, 0), &[]);
    assert!(work.maximum_input_chunk <= 4096);
    let budget = TreeSitterBudget {
        max_predicate_bytes: 4096,
        ..generous()
    };
    let result = highlight(&tree, 0..100, &budget, &AtomicBool::new(false));
    assert_eq!(result.coverage, Coverage::Missing);
    assert_eq!(result.work.predicate_bytes_copied, 0);
    assert_eq!(
        result.diagnostic,
        Some(TreeSitterError::Limit("predicate bytes"))
    );
}

#[test]
fn byte_coordinates_are_checked_and_multibyte_edit_points_are_exact() {
    assert_eq!(
        check_coordinate_size(u32::MAX as usize + 1),
        Err(TreeSitterError::Limit("32-bit coordinates"))
    );
    let old = input("// é\nint x;\n", 0);
    let new = input("// é\nint x;\nint y;\n", 1);
    let edit = SyntaxInputEdit::new(
        &old,
        &new,
        old.byte_len()..old.byte_len(),
        old.byte_len()..new.byte_len(),
    )
    .unwrap();
    assert_eq!(edit.edit.start_position, Point::new(2, 0));
    assert_eq!(edit.edit.new_end_position, Point::new(3, 0));
    assert!(point(&old, 4).is_err());
    let mut foreign = new.identity();
    foreign.document = 9;
    let foreign = SyntaxInputSnapshot::new(foreign, new.text_tree().clone());
    assert!(SyntaxInputEdit::new(&old, &foreign, 0..0, 0..0).is_err());
}

#[test]
fn included_language_ranges_preserve_parent_coordinates_and_injections_change() {
    let mut session =
        TreeSitterSession::new(TreeSitterPackage::bundled("javascript").unwrap()).unwrap();
    let old = input("const q = python\u{0060}print(1)\u{0060};", 0);
    let (old_tree, _) = parsed(&mut session, old.clone(), &[]);
    let result = highlight(
        &old_tree,
        0..old.byte_len(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert_eq!(result.injections.len(), 1);
    assert_eq!(result.injections[0].language, "python");
    let region = &result.injections[0];
    let mut child = TreeSitterSession::new(TreeSitterPackage::bundled("python").unwrap()).unwrap();
    let child = child
        .parse(
            old.clone(),
            &[],
            &region.ranges,
            &generous(),
            &AtomicBool::new(false),
        )
        .unwrap();
    let ParseOutcome::Complete {
        snapshot: child, ..
    } = child
    else {
        panic!()
    };
    assert!(!child.has_errors());
    let highlights = highlight(
        &child,
        region.ranges[0].clone(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert!(highlights
        .runs
        .iter()
        .all(|r| region.ranges[0].start <= r.range.start));
    let new = input("const q = rust\u{0060}print(1)\u{0060};", 1);
    let edit = SyntaxInputEdit::new(&old, &new, 10..16, 10..14).unwrap();
    let (new_tree, _) = parsed(&mut session, new.clone(), &[edit]);
    let result = highlight(
        &new_tree,
        0..new.byte_len(),
        &generous(),
        &AtomicBool::new(false),
    );
    assert_eq!(result.injections[0].language, "rust");
    assert_ne!(result.injections[0].parent_input, region.parent_input);
}

#[test]
fn regional_query_and_local_edit_work_do_not_grow_with_unrelated_suffix() {
    let package = TreeSitterPackage::bundled("c").unwrap();
    let mut costs = Vec::new();
    for lines in [10_000, 100_000] {
        let text = "int value(void) { return 42; }\n".repeat(lines);
        let old = input(&text, 0);
        let mut session = TreeSitterSession::new(package.clone()).unwrap();
        let (_, first_work) = parsed(&mut session, old.clone(), &[]);
        assert!(first_work.maximum_input_chunk <= 4096);
        let mut changed = text;
        changed.replace_range(25..27, "43");
        let new = input(&changed, 1);
        let edit = SyntaxInputEdit::new(&old, &new, 25..27, 25..27).unwrap();
        let (tree, work) = parsed(&mut session, new, &[edit]);
        let result = highlight(&tree, 0..29, &generous(), &AtomicBool::new(false));
        assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostic);
        assert!(work.input_bytes_supplied < 32 * 1024, "{work:?}");
        assert!(result.work.query_matches < 128, "{:?}", result.work);
        assert_eq!(result.work.input_callbacks, 0);
        costs.push((work.input_bytes_supplied, result.work.query_matches));
    }
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn native_bytes_are_measured_retained_and_soft_limits_cancel() {
    let package = TreeSitterPackage::bundled("c").unwrap();
    assert!(package.native_allocation_metrics().retained_bytes > 0);
    let mut session = TreeSitterSession::new(package).unwrap();
    let snapshot = input(&"int x = 1;\n".repeat(1000), 0);
    let budget = TreeSitterBudget {
        max_native_bytes: 64 * 1024,
        ..generous()
    };
    assert_eq!(
        session
            .parse(snapshot.clone(), &[], &[], &budget, &AtomicBool::new(false))
            .unwrap_err(),
        TreeSitterError::Limit("native allocation soft limit")
    );
    assert!(session.completed().is_none());
    let (tree, work) = parsed(&mut session, snapshot, &[]);
    assert!(work.native_retained_bytes > 64 * 1024);
    assert!(work.native_peak_bytes >= work.native_retained_bytes);
    drop(session);
    assert!(tree.native_allocation_metrics().retained_bytes > 0);
    let held = tree.clone();
    std::thread::spawn(move || drop(tree)).join().unwrap();
    assert!(held.native_allocation_metrics().retained_bytes > 0);
}

#[test]
fn native_library_owner_drops_after_unpublished_parser_handles() {
    use std::sync::{Mutex, Weak, atomic::AtomicUsize};
    struct Owner {
        account: Arc<Mutex<Option<Weak<native::Account>>>>,
        observed: Arc<AtomicUsize>,
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            let remaining = self.account.lock().unwrap().as_ref().and_then(Weak::upgrade)
                .map_or(0, |account| account.metrics().retained_bytes);
            self.observed.store(remaining, Ordering::Relaxed);
        }
    }
    let account = Arc::new(Mutex::new(None));
    let observed = Arc::new(AtomicUsize::new(usize::MAX));
    let package = TreeSitterPackage::compile("owner", 1, tree_sitter_c::LANGUAGE.into(), "", None,
        QueryProfile::Upstream, &generous(), Some(Arc::new(Owner { account:account.clone(),observed:observed.clone() }))).unwrap();
    let session = TreeSitterSession::new(package).unwrap();
    *account.lock().unwrap() = Some(Arc::downgrade(&session.native));
    assert!(session.native.metrics().retained_bytes > 0);
    drop(session);
    assert_eq!(observed.load(Ordering::Relaxed), 0, "native library released before native parser");
}

#[test]
fn combined_injections_discover_all_members_before_viewport_publication() {
    let package = TreeSitterPackage::compile(
        "combined",
        1,
        tree_sitter_c::LANGUAGE.into(),
        "(comment) @comment",
        Some(
            r#"((comment) @injection.content (#set! injection.language "python")
          (#set! injection.combined) (#set! injection.include-children)
          (#offset! @injection.content 0 2 0 0))"#,
        ),
        QueryProfile::Upstream,
        &generous(),
        None,
    )
    .unwrap();
    let text = "//x = 1\nint x;\n//print(x)\n";
    let mut session = TreeSitterSession::new(package).unwrap();
    let (snapshot, _) = parsed(&mut session, input(text, 0), &[]);
    let output = highlight(&snapshot, 0..7, &generous(), &AtomicBool::new(false));
    assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
    assert_eq!(output.injections.len(), 1);
    assert!(output.injections[0].combined);
    assert_eq!(output.injections[0].ranges, vec![2..7, 17..25]);
    let budget = TreeSitterBudget {
        max_injections: 1,
        ..generous()
    };
    assert_eq!(
        highlight(&snapshot, 0..7, &budget, &AtomicBool::new(false)).coverage,
        Coverage::Missing
    );
}

#[test]
fn combined_and_regional_injections_keep_separate_scopes_and_share_limits() {
    let package = TreeSitterPackage::compile(
        "mixed-injections", 1, tree_sitter_c::LANGUAGE.into(), "(comment) @comment",
        Some(concat!(
            "((comment) @injection.content (#set! injection.language \"python\")",
            " (#set! injection.combined) (#set! injection.include-children))\n",
            "((identifier) @injection.content (#set! injection.language \"rust\"))\n",
        )),
        QueryProfile::Upstream, &generous(), None,
    ).unwrap();
    let mut costs = Vec::new();
    for lines in [200, 2_000] {
        let text = format!("// head\n{}// tail\n", "int value;\n".repeat(lines));
        let mut session = TreeSitterSession::new(package.clone()).unwrap();
        let (snapshot, _) = parsed(&mut session, input(&text, 0), &[]);
        let output = highlight(&snapshot, 8..18, &generous(), &AtomicBool::new(false));
        assert_eq!(output.coverage, Coverage::Exact, "{:?}", output.diagnostic);
        assert_eq!(output.injections.len(), 2);
        assert_eq!(output.injections[0].ranges, vec![12..17]);
        assert!(!output.injections[0].combined);
        assert_eq!(output.injections[1].ranges, vec![0..7, text.len() - 8..text.len() - 1]);
        assert!(output.injections[1].combined);
        costs.push(output.work.query_matches);
        let limited = highlight(&snapshot, 8..18, &TreeSitterBudget {
            max_injections: 2, ..generous()
        }, &AtomicBool::new(false));
        assert_eq!(limited.coverage, Coverage::Exact, "{:?}", limited.diagnostic);
        assert!(limited.injections_truncated);
        assert_eq!(limited.injections.len(), 1);
        assert!(limited.injections[0].combined);
    }
    assert_eq!(costs[0], costs[1], "offscreen regional matches must not consume query work");
}
