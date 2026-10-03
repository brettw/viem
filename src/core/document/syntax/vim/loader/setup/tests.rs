use super::*;

fn buffer_context(text: &str, revision: u64) -> VimSetupContext {
    use crate::document::{
        syntax::{SyntaxInputIdentity, SyntaxInputSnapshot},
        FormattedTextTree,
    };
    VimSetupContext::from_input(&SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 903,
            revision,
            generation: 1,
        },
        FormattedTextTree::try_from_text(text).unwrap(),
    ))
}

#[test]
fn setup_buffer_line_count_excludes_terminal_ending_and_tracks_real_tail() {
    for (text, count, tail) in [
        ("", 1, ""),
        ("first", 1, "first"),
        ("first\n", 1, "first"),
        ("first\n\n", 2, ""),
        ("first\n; comment\n", 2, "; comment"),
    ] {
        let context = buffer_context(text, 1);
        let mut setup = Setup::new(&context, None);
        assert_eq!(setup.evaluate("line('$')").unwrap(), Value::Number(count));
        assert_eq!(
            setup.evaluate("getline(line('$'))").unwrap(),
            Value::Text(tail.into())
        );
    }
    let before = buffer_context(&format!("{}; previous", "body\n".repeat(40)), 1);
    let after = buffer_context(&format!("{}# current", "body\n".repeat(40)), 2);
    let mut setup = Setup::new(&before, None);
    setup.evaluate("getline(line('$'))").unwrap();
    assert!(!setup.reads.matches(&before, &after));
}

#[test]
fn setup_buffer_search_wraps_origin_and_honors_stop_lines() {
    let context = buffer_context("marker\nnext\nmarker\n", 1);
    for (expression, expected) in [
        ("search('marker', 'cnW')", 1),
        ("search('marker', 'nW')", 3),
        ("search('marker', 'nw')", 3),
        ("search('marker', 'nW', 2)", 0),
        ("search('marker', 'nw', 2)", 0),
        ("search('\\%^marker', 'nW')", 0),
        ("search('\\%^marker', 'nw')", 1),
    ] {
        let mut setup = Setup::new(&context, None);
        assert_eq!(
            setup.evaluate(expression).unwrap(),
            Value::Number(expected),
            "{expression}"
        );
    }
}

#[test]
fn setup_absolute_start_search_does_not_scan_or_depend_on_large_suffix() {
    let context = buffer_context(&format!("{}", "a".repeat(2 * 1024 * 1024)), 1);
    let mut setup = Setup::new(&context, None);
    assert_eq!(
        setup.evaluate("search('\\%^z', 'cnW')").unwrap(),
        Value::Number(0)
    );
    assert!(setup.reads.search_end.unwrap() < 8);
    let mut changed = context.clone();
    let input = changed.input.as_ref().unwrap();
    changed.input = Some(crate::document::syntax::SyntaxInputSnapshot::new(
        crate::document::syntax::SyntaxInputIdentity {
            revision: 2,
            ..input.identity()
        },
        input.text_tree().splice(1000..1001, "b").unwrap(),
    ));
    assert!(setup.reads.matches(&context, &changed));
}

#[test]
fn failed_buffer_queries_retain_dependencies_for_retry() {
    let context = buffer_context(&"a".repeat(2 * 1024 * 1024), 1);
    let mut setup = Setup::new(&context, None);
    assert!(setup.evaluate("search('z', 'cnW')").is_err());
    let next = buffer_context(
        &format!("{}z{}", "a".repeat(100), "a".repeat(2 * 1024 * 1024 - 101)),
        2,
    );
    assert!(!setup.reads.matches(&context, &next));
    let mut setup = Setup::new(&context, None);
    assert!(setup.evaluate("getline(1)").is_err());
    assert!(!setup.reads.matches(&context, &buffer_context("small", 2)));
}

#[test]
fn deferred_declaration_queries_are_rejected_across_nested_calls() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup
        .define_function("QueryGroup()", &["return hlexists('Normal')".into()])
        .unwrap();
    setup
        .define_function(
            "EmitGroup()",
            &["execute 'syn keyword Generated generated'".into()],
        )
        .unwrap();
    setup
        .define_function(
            "EmptyEmit()",
            &["execute ''".into(), "return QueryGroup()".into()],
        )
        .unwrap();
    assert_eq!(setup.evaluate("QueryGroup()").unwrap(), Value::Number(1));
    assert!(setup.call_statement("QueryGroup()").unwrap().is_empty());
    assert_eq!(setup.call_statement("EmptyEmit()").unwrap(), [""]);
    for (name, body) in [
        (
            "DirectQuery()",
            vec!["call EmitGroup()", "return hlexists('Generated')"],
        ),
        (
            "NestedQuery()",
            vec!["call EmitGroup()", "return QueryGroup()"],
        ),
    ] {
        setup
            .define_function(
                name,
                &body.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
            .unwrap();
        assert!(setup
            .call_statement(name)
            .unwrap_err()
            .contains("pending generated commands"));
        // A failed call cannot leak its pending output into the next call.
        assert!(setup.call_statement("QueryGroup()").unwrap().is_empty());
    }
}

#[test]
fn function_output_cannot_defer_setup_mutations_or_hide_them_in_separators() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    for command in [
        "let g:value = 1",
        "set iskeyword=_",
        "source syntax/other.vim",
        "runtime syntax/other.vim",
        "syn include @Child syntax/other.vim",
        "syn clear",
        "if 1 | syn keyword A a | endif",
        "syn keyword A a | let g:value = 1",
        "syn keyword A a\nlet g:value = 1",
        "execute 'let g:value = 1'",
        "AliasToMutation",
        "silent! let g:value = 1",
        "vim9script\nsyn keyword A a",
    ] {
        let literal = command.replace('\'', "''");
        setup
            .define_function("EmitUnsafe()", &[format!("execute '{literal}'")])
            .unwrap();
        assert!(
            setup
                .call_statement("EmitUnsafe()")
                .unwrap_err()
                .contains("function output"),
            "{command}"
        );
    }
    setup
        .define_function(
            "EmitSafe()",
            &[
                "execute 'syn match Safe /a|b/ | hi def link Safe String'".into(),
                "execute 'syn case ignore'".into(),
                "execute 'syn iskeyword @,48-57,_'".into(),
            ],
        )
        .unwrap();
    assert_eq!(setup.call_statement("EmitSafe()").unwrap().len(), 3);
}

#[test]
fn native_filename_formatting_and_highlight_queries_are_bounded() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.assign("g:ada#Keywords = ['begin', 'end']").unwrap();
    for (expression, expected) in [
        ("g:ada#Keywords[0]", Value::Text("begin".into())),
        ("'prefix'.string(42)", Value::Text("prefix42".into())),
        (
            "fnamemodify('/code/module-info.java', ':t:r')",
            Value::Text("module-info".into()),
        ),
        (
            "fnamemodify('archive.tar.gz', ':e:e')",
            Value::Text("tar.gz".into()),
        ),
        ("fnamemodify('.vimrc', ':e')", Value::Text(String::new())),
        (
            "fnamemodify('/tmp/.vimrc', ':r')",
            Value::Text("/tmp/.vimrc".into()),
        ),
        (
            "printf('%s:%03d:%02x:%%', 'name', -2, 10)",
            Value::Text("name:-02:0a:%".into()),
        ),
        (
            "printf('%-4s:%+.3d', 'x', 2)",
            Value::Text("x   :+002".into()),
        ),
        ("printf('%.0d/%.2s', 0, 'abc')", Value::Text("/ab".into())),
        ("17->printf('%d')", Value::Text("17".into())),
        ("&encoding ==# 'utf-8' && !&compatible", Value::Number(1)),
        ("hlexists('Normal')", Value::Number(1)),
        ("hlexists('String')", Value::Number(0)),
        ("hlexists('NoSuchGroup')", Value::Number(0)),
    ] {
        assert_eq!(
            setup.evaluate(expression).unwrap(),
            expected,
            "{expression}"
        );
    }
    let before = setup.environment_fingerprint().unwrap();
    setup.define_highlight("CustomGroup").unwrap();
    assert_eq!(
        setup.evaluate("hlexists('CUSTOMGROUP')").unwrap(),
        Value::Number(1)
    );
    assert_ne!(setup.environment_fingerprint().unwrap(), before);
    for source in [
        "printf('%9999999999d', 1)",
        "printf('%99999999999999999999999s', 'a')",
        "printf('%s')",
        "printf('%d', 1, 2)",
        "printf('%f', 1)",
        "printf('%s%s', repeat('x', 200000), repeat('x', 200000))",
        "printf('%.1s', 'é')",
        "fnamemodify('relative', ':p')",
    ] {
        assert!(setup.evaluate(source).is_err(), "{source}");
    }
}

#[test]
fn global_setup_helpers_retain_their_defining_script_scope() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.assign("s:prefix = 'first'").unwrap();
    setup
        .define_function(
            "NativeHelper(suffix)",
            &["return s:prefix . a:suffix".into()],
        )
        .unwrap();
    let script = setup.begin_script();
    setup.assign("s:prefix = 'second'").unwrap();
    assert_eq!(
        setup.evaluate("NativeHelper('!')").unwrap(),
        Value::Text("first!".into())
    );
    assert_eq!(
        setup.evaluate("s:prefix").unwrap(),
        Value::Text("second".into())
    );
    setup
        .define_function("UnsafeHelper()", &["call system('anything')".into()])
        .unwrap();
    assert!(setup.evaluate("UnsafeHelper()").is_err());
    assert_eq!(
        setup.evaluate("s:prefix").unwrap(),
        Value::Text("second".into())
    );
    setup.end_script(script);
    setup.delete_function("NativeHelper", false).unwrap();
    assert!(setup.evaluate("NativeHelper('!')").is_err());
}

#[test]
fn make_flavor_uses_buffer_scope_then_script_default() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.assign("s:make_flavor = 'gnu'").unwrap();
    assert_eq!(
        setup
            .evaluate("get(b:, 'make_flavor', s:make_flavor) == 'gnu'")
            .unwrap(),
        Value::Number(1)
    );
    setup.assign("b:make_flavor = 'bsd'").unwrap();
    assert_eq!(
        setup
            .evaluate("get(b:, 'make_flavor', s:make_flavor)")
            .unwrap(),
        Value::Text("bsd".into())
    );
}

#[test]
fn runtime_collection_string_and_arithmetic_expressions() {
    let mut setup = Setup::new(
        &VimSetupContext {
            prefix: "#!/bin/bash\nother".into(),
            ..Default::default()
        },
        None,
    );
    setup.set_filetype("cpp");
    for (expression, expected) in [
        ("getline(1)", Value::Text("#!/bin/bash".into())),
        ("matchstr(&ft, '^[^.]*')", Value::Text("cpp".into())),
        ("get({'key': [3, 5]}, 'key')[1]", Value::Number(5)),
        ("get({}, 'missing')", Value::Number(0)),
        ("empty({}) && len([1,2]) == 2", Value::Number(1)),
        ("range(2, 8, 3)[-1]", Value::Number(8)),
        ("1+2*3-4", Value::Number(3)),
        ("'CPP' ==? 'cpp'", Value::Number(1)),
        ("'CPP' =~? '^cpp$'", Value::Number(1)),
        ("split('a  b')[1]", Value::Text("b".into())),
        ("split('a,b', ',')->join(':')", Value::Text("a:b".into())),
    ] {
        assert_eq!(
            setup.evaluate(expression).unwrap(),
            expected,
            "{expression}"
        );
    }
    setup.assign("s:n = 3").unwrap();
    setup.assign("s:n += 4").unwrap();
    assert_eq!(setup.evaluate("s:n").unwrap(), Value::Number(7));
}

#[test]
fn go_runtime_variadic_guarded_return_helpers() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.define_function("s:FoldEnable(...) abort", &[
        "if a:0 > 0", "return index(s:FoldEnable(), a:1) > -1", "endif",
        "return get(g:, 'go_fold_enable', ['block', 'import', 'varconst', 'package_comment'])",
    ].map(str::to_owned)).unwrap();
    assert_eq!(
        setup.evaluate("s:FoldEnable('import')").unwrap(),
        Value::Number(1)
    );
    assert_eq!(
        setup.evaluate("s:FoldEnable('unknown')").unwrap(),
        Value::Number(0)
    );
    setup
        .define_function(
            "s:Local(flag) abort",
            &[
                "let local = a:flag + 2",
                "if local == 3",
                "return local",
                "else",
                "return 9",
                "endif",
            ]
            .map(str::to_owned),
        )
        .unwrap();
    assert_eq!(setup.evaluate("s:Local(1)").unwrap(), Value::Number(3));
    assert!(setup.evaluate("local").is_err());
}

#[test]
fn setup_collections_and_functions_retain_work_and_type_boundaries() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    for expression in [
        "range(1000000000)",
        "range(1,3,0)",
        "1/0",
        "9223372036854775807+1",
        "[1][2]",
        "'é'[0]",
        "system('echo unsafe')",
        "readfile('/etc/passwd')",
    ] {
        assert!(setup.evaluate(expression).is_err(), "{expression}");
    }
    setup
        .define_function(
            "s:SideEffect() abort",
            &["let g:global = 1".into(), "return 0".into()],
        )
        .unwrap();
    assert!(setup.evaluate("s:SideEffect()").is_err());
    setup
        .define_function("s:SideEffect() abort", &["return system('date')".into()])
        .unwrap();
    assert!(setup.evaluate("s:SideEffect()").is_err());
}

#[test]
fn expression_comments_do_not_consume_quoted_operands_or_execute_arguments() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    assert_eq!(
        setup.evaluate("!exists(\"c_no_c99\") \" ISO C99").unwrap(),
        Value::Number(1)
    );
    assert_eq!(
        setup
            .evaluate("get(g:, \"name\", \"text\") == \"text\" \" comment")
            .unwrap(),
        Value::Number(1)
    );
    assert_eq!(
        setup
            .evaluate_execute("\"syntax keyword Foo\" \"hello\"")
            .unwrap(),
        "syntax keyword Foo hello"
    );
    assert_eq!(
        setup.evaluate(r#""\x41\u00e9\101""#).unwrap(),
        Value::Text("AéA".into())
    );
    assert_eq!(
        setup.evaluate("0x20 + 0b11 + 010").unwrap(),
        Value::Number(43)
    );
    assert!(setup.evaluate("1.5").is_err());
}

#[test]
fn bounded_function_loops_early_returns_and_generated_commands() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup
        .define_function(
            "s:Contains(...) abort",
            &[
                "for l:item in a:000",
                "if l:item == 'present'",
                "return 1",
                "endif",
                "endfor",
                "return 0",
            ]
            .map(str::to_owned),
        )
        .unwrap();
    assert_eq!(
        setup.evaluate("s:Contains('absent', 'present')").unwrap(),
        Value::Number(1)
    );
    assert_eq!(
        setup.evaluate("s:Contains('absent')").unwrap(),
        Value::Number(0)
    );
    setup
        .define_function(
            "s:Generate(dict) abort",
            &[
                "let group = a:dict.itemGroup",
                "for item in ['one', 'two']",
                "exec 'syn keyword ' . group . ' ' . item",
                "endfor",
                "return 0",
            ]
            .map(str::to_owned),
        )
        .unwrap();
    assert!(setup
        .evaluate("s:Generate({'itemGroup': 'Generated'})")
        .is_err());
    assert_eq!(
        setup
            .call_statement("s:Generate({'itemGroup': 'Generated'})")
            .unwrap(),
        ["syn keyword Generated one", "syn keyword Generated two"]
    );
    setup
        .define_function(
            "s:Dead()",
            &["return 3".into(), "call system('date')".into()],
        )
        .unwrap();
    assert_eq!(setup.evaluate("s:Dead()").unwrap(), Value::Number(3));
    setup
        .define_function(
            "s:Loop()",
            &["while 1".into(), "let x = 1".into(), "endwhile".into()],
        )
        .unwrap();
    assert!(setup.evaluate("s:Loop()").unwrap_err().contains("budget"));
}

#[test]
fn macros_preserve_arguments_quote_text_and_use_strict_loader_commands() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup
        .define_macro("-nargs=+ HiLink hi def link <args>")
        .unwrap();
    assert_eq!(
        setup.expand_macro("HiLink", "From To").unwrap().unwrap().0,
        "hi def link From To"
    );
    assert!(setup.expand_macro("HiLink", "").is_err());
    setup
        .define_macro("-nargs=* Wrap execute <q-args> 'contained'")
        .unwrap();
    let expanded = setup
        .expand_macro("Wrap", "syn match Group 'pattern'")
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(
        setup
            .evaluate_execute(expanded.strip_prefix("execute ").unwrap())
            .unwrap(),
        "syn match Group 'pattern' contained"
    );
}

#[test]
fn collection_helpers_update_owned_values_and_reject_shared_mutation() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.assign("s:aria = ['label']").unwrap();
    setup.assign("s:deprecated = ['dropeffect']").unwrap();
    setup
        .call_statement("extend(s:aria, s:deprecated)")
        .unwrap();
    assert_eq!(
        setup.evaluate("join(s:aria, ',')").unwrap(),
        Value::Text("label,dropeffect".into())
    );
    setup
        .assign("s:mapped = copy(s:aria)->map('toupper(v:val)')")
        .unwrap();
    assert_eq!(
        setup.evaluate("s:mapped[0]").unwrap(),
        Value::Text("LABEL".into())
    );
    setup.assign("s:alias = s:aria").unwrap();
    assert!(setup
        .call_statement("add(s:alias, 'new')")
        .unwrap_err()
        .contains("aliased"));
    assert!(setup
        .assign("s:aria[0] = 'changed'")
        .unwrap_err()
        .contains("aliased"));
    setup.assign("s:independent = copy(s:aria)").unwrap();
    setup.call_statement("add(s:independent, 'new')").unwrap();
    assert_eq!(setup.evaluate("len(s:aria)").unwrap(), Value::Number(2));
}

#[test]
fn setup_environment_identity_and_nested_value_budgets() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    let initial = setup.environment_fingerprint().unwrap();
    setup.assign("s:private = 1").unwrap();
    assert_eq!(setup.environment_fingerprint().unwrap(), initial);
    setup.assign("b:current_syntax_embed = 1").unwrap();
    assert_ne!(setup.environment_fingerprint().unwrap(), initial);
    setup.assign("s:nested = 1").unwrap();
    let mut exhausted = false;
    for _ in 0..100 {
        if let Err(error) = setup.assign("s:nested = [s:nested]") {
            assert!(error.contains("depth"), "{error}");
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
}

#[test]
fn filename_and_prefix_queries_use_only_explicit_context() {
    let mut setup = Setup::new(
        &VimSetupContext {
            prefix: "# header\n# marker\nbody".into(),
            filename: Some("/project/article.tex".into()),
            input: None,
        },
        None,
    );
    assert_eq!(
        setup.evaluate("expand('%:t:r')").unwrap(),
        Value::Text("article".into())
    );
    assert_eq!(
        setup.evaluate("expand('%:e')").unwrap(),
        Value::Text("tex".into())
    );
    assert_eq!(
        setup.evaluate("bufname('')").unwrap(),
        Value::Text("/project/article.tex".into())
    );
    assert_eq!(
        setup
            .evaluate("search('^# marker$', 'cnW', '', 100)")
            .unwrap(),
        Value::Number(2)
    );
    assert_eq!(
        setup.evaluate("search('missing', 'cnW')").unwrap(),
        Value::Number(0)
    );
    assert!(setup.evaluate("search('body', 'cW')").is_err());
    assert!(setup.evaluate("search('body', 'cnW', 33)").is_err());
    setup.set_source_file("/runtime/syntax/tex.vim");
    assert_eq!(
        setup.evaluate("expand('<sfile>:p:h')").unwrap(),
        Value::Text("/runtime/syntax".into())
    );
    assert!(setup.evaluate("expand('$HOME')").is_err());
    assert_eq!(
        setup.evaluate("executable('/bin/sh')").unwrap(),
        Value::Number(0)
    );
}

#[test]
fn yaml_expression_substitutions_and_slices_stay_bounded() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    setup.assign("s:indicator = '[xyz]'").unwrap();
    assert_eq!(
        setup.evaluate("s:indicator[1:-2]").unwrap(),
        Value::Text("xyz".into())
    );
    assert_eq!(
        setup
            .evaluate(r#"substitute('a[b]', '\[\zs', '\=s:indicator[1:-2]', '')"#)
            .unwrap(),
        Value::Text("a[xyzb]".into())
    );
    assert_eq!(
        setup
            .evaluate(r#"substitute('red blue', '\w\+', '\=toupper(submatch(0))', 'g')"#)
            .unwrap(),
        Value::Text("RED BLUE".into())
    );
    assert_eq!(
        setup.evaluate("[1, 2, 3][1:]").unwrap(),
        Value::List(vec![Value::Number(2), Value::Number(3)])
    );
    assert!(setup
        .evaluate(r#"substitute('x', 'x', '\=system("date")', '')"#)
        .is_err());
    assert!(setup.evaluate("submatch(0)").is_err());
}
