use super::*;

fn compile_fixture(source: &str) -> Arc<VimProgram> {
    compile("setup.vim", source, VimLoadLimits::default()).unwrap()
}

#[test]
fn native_setup_conditions_macros_functions_and_execute() {
    let program = compile_fixture(
        r#"
let s:mode = 0
let lua_version = 5
let lua_subversion = 3
function s:has(feature)
  return has(a:feature) || index(get(g:, "vim_features", []), a:feature) != -1
endfunction
com! -nargs=* Vim9 execute <q-args> s:mode ? "" : "contained"
com! -nargs=* VimL execute <q-args> s:mode ? "contained" : ""
com! -nargs=* VimFold <args> fold
if exists("g:missing") && g:missing =~# 'x'
  unsupported command
elseif lua_version > 5 || (get(g:, "lua_version", 0) == 5 && lua_subversion >= 3)
  VimFold VimL syn keyword Legacy legacy
  Vim9 syn keyword Modern modern
endif
if s:has("nvim") || !get(g:, "enabled", 1)
  unsupported command
endif
exe "syn sync minlines=" .. get(g:, "minlines", 100)
exe "syn sync maxlines=" .. get(g:, "maxlines", 200)
delc Vim9
delc VimL
delc VimFold
unlet s:mode
"#,
    );
    assert_eq!(program.rules.len(), 2);
    assert!(!program.rules[0].options.contained);
    assert!(program.rules[1].options.contained);
    assert_eq!(program.minlines, 100);
    assert_eq!(program.maxlines, 200);
}

#[test]
fn native_setup_reads_only_supplied_prefix_for_vim9_mode() {
    let source = r#"
let s:vim9script = get(b:, "vimsyn_force_vim9", v:false) || "\n" .. getline(1, 32)->join("\n") =~# '\n\s*vim9\%[script]\>'
if s:vim9script
  syn keyword Modern def
else
  syn keyword Legacy function
endif
"#;
    for (prefix, expected) in [
        ("\" comment\n  vim9script\ndef Foo()\n", "Modern"),
        ("let g:x = 'vim9script'", "Legacy"),
    ] {
        let mut loader = Loader::new(VimLoadLimits::default(), None);
        loader.setup = Setup::new(
            &VimSetupContext {
                prefix: prefix.into(),
            },
            None,
        );
        loader.source("setup.vim", source);
        assert_eq!(loader.finish().unwrap().rules[0].group, expected);
    }
}

#[test]
fn native_setup_rejects_unsupported_commands_and_unbounded_recursion_atomically() {
    for source in [
        "syn keyword Good good\nlet s:x = system('date')",
        "syn keyword Good good\nset isk=a-z",
        "function s:Bad()\nlet x = 1\nreturn x\nendfunction",
        "syn keyword Good good\nfunction s:Bad()\nreturn system('date')\nendfunction",
        "syn keyword Good good\nfunction s:Bad()\nreturn readfile('/tmp/private')\nendfunction",
        "function s:Loop()\nreturn s:Loop()\nendfunction\nlet s:x = s:Loop()",
        "com! -nargs=* Loop execute <q-args> ' Loop'\nLoop Loop",
        "exe 'write /tmp/forbidden'",
        "com! -nargs=* Bad execute <q-args> execute('quit')",
    ] {
        assert!(
            compile("bad.vim", source, VimLoadLimits::default()).is_err(),
            "{source}"
        );
    }
    let cancelled = AtomicBool::new(true);
    let mut setup = Setup::new(&VimSetupContext::default(), Some(&cancelled));
    assert!(setup.evaluate("1").unwrap_err().contains("cancelled"));
    let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    assert!(Setup::new(&VimSetupContext::default(), None)
        .evaluate(&deep)
        .unwrap_err()
        .contains("depth"));
}

#[test]
fn native_setup_bounds_expression_values_and_source_before_publication() {
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    let huge_expression = "1 ".repeat(40_000);
    assert!(setup
        .evaluate(&huge_expression)
        .unwrap_err()
        .contains("byte budget"));
    setup
        .assign(&format!("s:chunk = '{}'", "x".repeat(8192)))
        .unwrap();
    let list = format!(
        "[{}]",
        std::iter::repeat_n("s:chunk", 33)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(setup
        .evaluate(&list)
        .unwrap_err()
        .contains("value byte budget"));
    let source = "syn keyword Good good\nsyn keyword Other other\n";
    let limits = VimLoadLimits {
        source_bytes: 32,
        ..Default::default()
    };
    let errors = compile("oversized.vim", source, limits).unwrap_err();
    assert!(errors.iter().any(|d| d.message.contains("budget")));
    // Replacing one small binding repeatedly still observes the shared setup
    // declaration budget; entering another script scope cannot reset it.
    let mut setup = Setup::new(&VimSetupContext::default(), None);
    let declaration = format!("s:value = '{}'", "x".repeat(8192));
    let mut exhausted = false;
    for _ in 0..300 {
        let outer = setup.begin_script();
        let result = setup.assign(&declaration);
        setup.end_script(outer);
        if let Err(error) = result {
            assert!(error.contains("aggregate storage"), "{error}");
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
}

#[test]
fn native_setup_include_preserves_script_scope_and_guard_state() {
    let root = std::env::temp_dir().join(format!("viem-loader-scope-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("inner.vim"),
        "let s:mode = 0\nsyn keyword Inner inner\nlet b:current_syntax = 'inner'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("outer.vim"),
        r#"
let s:mode = 1
let b:current_syntax = 'outer'
syn include @Inner inner.vim
if !exists('b:current_syntax') || b:current_syntax != 'inner'
  unsupported guard state
endif
unlet! b:current_syntax
if s:mode && !exists('b:current_syntax')
  syn keyword Outer outer
endif
"#,
    )
    .unwrap();
    let program = directory(&root, "outer", VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(), 2);
    assert!(program.rules[0].options.contained);
    assert_eq!(program.rules[1].group, "Outer");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_setup_macros_keep_the_defining_script_for_expressions_and_arguments() {
    let root = std::env::temp_dir().join(format!("viem-loader-macro-scope-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("inner.vim"),
        r#"
let s:mode = 0
Mode syn keyword Inner inner
Forward let s:mode = 2
if s:mode != 0
  unsupported caller mutation
endif
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("outer.vim"),
        r#"
let s:mode = 1
com! -nargs=* Mode execute <q-args> s:mode ? '' : 'contained'
com! -nargs=* Forward <args>
runtime inner.vim
if s:mode == 2
  syn keyword Outer outer
endif
"#,
    )
    .unwrap();
    let program = directory(&root, "outer", VimLoadLimits::default()).unwrap();
    assert_eq!(program.rules.len(), 2);
    assert!(!program.rules[0].options.contained);
    assert_eq!(program.rules[1].group, "Outer");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_syntax_delimiters_ignore_delimiters_inside_bracket_classes() {
    for (source, expected, tail) in [
        (
            r##""["|[:space:]]\+" contained"##,
            r#"["|[:space:]]\+"#,
            " contained",
        ),
        (r##""[^\\]['"]"he=e-1"##, r#"[^\\]['"]"#, "he=e-1"),
        (r##""[]"]""##, r#"[]"]"#, ""),
        (r##""\[" nextgroup=Group"##, r#"\["#, " nextgroup=Group"),
    ] {
        let (pattern, rest) = delimited(source).unwrap();
        assert_eq!(pattern, expected);
        assert_eq!(rest, tail);
    }
}

#[test]
fn native_leading_context_offsets_and_sync_hints_are_parsed_strictly() {
    let (offsets, tail) = attached_offsets("lc=1,he=e-1 contains=Value").unwrap();
    assert_eq!(offsets.lc, 1);
    assert_eq!(offsets.ms.unwrap().delta, 1);
    assert_eq!(tail, "contains=Value");
    assert!(attached_offsets("lc=1025").is_err());
    let program = compile_fixture("syn keyword Word word\nsyn sync match State grouphere NONE /word/\nsyn sync linecont\t/\\\\$/\n");
    assert_eq!(program.rules.len(), 1);
    assert!(compile(
        "bad.vim",
        "syn sync match State grouphere Unknown /word/",
        VimLoadLimits::default()
    )
    .is_err());
}

#[test]
fn native_group_patterns_expand_at_declaration_time() {
    let program = compile_fixture(
        r#"
syn keyword vimKeymapLineComment contained old
syn cluster Comments contains=vim9\=KeymapLineComment
syn keyword vim9KeymapLineComment contained new
syn region Body start=/{/ end=/}/ contains=vim9\=KeymapLineComment
syn keyword vim99KeymapLineComment contained later
"#,
    );
    assert_eq!(program.clusters["Comments"], ["vimKeymapLineComment"]);
    assert_eq!(
        program.rules[2].options.contains,
        ["vim9KeymapLineComment", "vimKeymapLineComment"]
    );
}

#[test]
fn native_commented_continuations_preserve_following_region_options() {
    let program = compile_fixture(
        r#"
syn keyword Option contained number
syn region Args contained
      \ start=/[a-z]/
      \ end=/|/
      "\ comments within a continued command do not end it
      \ end=/$/
      \ contains=Option
      \ keepend
"#,
    );
    let rule = &program.rules[1];
    assert!(rule.options.keepend);
    assert_eq!(rule.options.contains, ["Option"]);
    let RuleKind::Region { ends, .. } = &rule.kind else {
        panic!("region expected")
    };
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[1].source.as_ref(), "$");
}
