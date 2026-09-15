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
                filename: None,
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
        "syn keyword Good good\nset unknownoption=value",
        "function s:Bad()\nlet g:x = 1\nreturn x\nendfunction\ncall s:Bad()",
        "syn keyword Good good\nfunction s:Bad()\nreturn system('date')\nendfunction\ncall s:Bad()",
        "syn keyword Good good\nfunction s:Bad()\nreturn readfile('/tmp/private')\nendfunction\ncall s:Bad()",
        "function s:Loop()\nreturn s:Loop()\nendfunction\nlet s:x = s:Loop()",
        "com! -nargs=* Loop execute <q-args> ' Loop'\nLoop Loop",
        "exe 'write /tmp/forbidden'",
        "com! -nargs=* Bad execute <q-args> execute('quit')\nBad syn keyword Bad bad",
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
        (r"'\V[' end='\V]'", r"\V[", " end='\\V]'"),
        (r#"'\M\[']' contained"#, r#"\M\[']"#, " contained"),
        (r#"'\V\_[']' contained"#, r#"\V\_[']"#, " contained"),
    ] {
        let (pattern, rest) = delimited(source).unwrap();
        assert_eq!(pattern, expected);
        assert_eq!(rest, tail);
    }
    let program = compile(
        "literal-brackets.vim",
        "syn region Box start='\\V[' end='\\V]'",
        VimLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(program.rules.len(), 1);
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

#[test]
fn runtime_statements_keep_patterns_macros_and_comments_separate() {
    let program = compile_fixture(
        r#"
if !exists('b:current_syntax') | syn keyword A alpha | endif
com! -nargs=* Wrapped <args> contained
Wrapped syn match B "[|\"]\+" " preserve the pattern
syn match C |pipe| | hi def link C Statement " comment
syn region R start=+"+ end=+"+ contains=A, B,
  \ C
hi def link A Identifier " trailing comment
hi def link B String
"#,
    );
    assert_eq!(program.rule_count(), 4);
    assert_eq!(program.effective_group("A"), "Identifier");
    assert_eq!(program.effective_group("C"), "Statement");
    assert_eq!(program.rules[3].options.contains, ["A", "B", "C"]);
}

#[test]
fn runtime_setup_loops_heredocs_and_try_are_bounded_declarations() {
    let program = compile_fixture(
        r#"
let s:names =<< trim END
  Alpha
  Beta
END
for name in s:names
  execute 'syn keyword ' . name . ' ' . name
endfor
let s:n = 0
while s:n < 2
  let s:n += 1
  if s:n == 1 | continue | endif
  syn keyword Last last
endwhile
try
  syn keyword Try tried
catch /E121:/
  unsupported skipped
finally
  hi def link Try Statement
endtry
"#,
    );
    assert_eq!(program.rule_count(), 4);
    assert_eq!(program.effective_group("Try"), "Statement");
    assert!(
        compile("loop", "while 1\nendwhile", VimLoadLimits::default())
            .unwrap_err()
            .iter()
            .any(|e| e.message.contains("budget"))
    );
    // Compatibility errors cannot be hidden by catch-all runtime handlers.
    assert!(compile(
        "unsafe",
        "try\ncall system('date')\ncatch\nendtry",
        VimLoadLimits::default()
    )
    .is_err());
}

#[test]
fn runtime_declarations_support_keyword_settings_sync_and_named_styles() {
    let program = compile_fixture(
        r#"
setlocal iskeyword+=-
syn keyword Word alpha-beta
syn region Body start=+<+ end=+>+ nextGroup=Word
syn sync match Sync groupthere Body +<+
syn sync ccomment Body minlines=20 maxlines=200
syn sync clear
syn cluster Words contains=Word,
  \ Body
hi def Word gui=bold guifg=#abcdef
hi def link Word Statement
hi! link Word Identifier
syn clear Body
"#,
    );
    assert_eq!(program.rule_count(), 1);
    assert_eq!(program.effective_group("Word"), "Identifier");
    assert_eq!(program.minlines, 20);
    let RuleKind::Keywords(ref pattern) = program.rules[0].kind else {
        panic!()
    };
    assert!(pattern.is_match_text("alpha-beta", 100_000).unwrap());
}

#[test]
fn runtime_keyword_inventories_split_without_weakening_pattern_limits() {
    let words = (0..2500)
        .map(|i| format!("keyword{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let program = compile_fixture(&format!("syn keyword Inventory {words}"));
    assert!(program.rule_count() > 1);
    for word in ["keyword0", "keyword1250", "keyword2499"] {
        assert!(program.rules.iter().any(|rule| match &rule.kind {
            RuleKind::Keywords(pattern) => pattern.is_match_text(word, 1_000_000).unwrap(),
            _ => false,
        }));
    }
    assert!(VimPattern::compile(&"x".repeat(17000), false, VimRegexLimits::default()).is_err());
}

#[test]
fn recursive_includes_use_guarded_setup_state_and_stay_confined() {
    let directory =
        std::env::temp_dir().join(format!("viem-syntax-recursive-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("child")).unwrap();
    std::fs::write(directory.join("self.vim"), "if !exists('b:embedded')\nlet b:embedded = 1\nsyn include @Embedded <sfile>:h/self.vim\nunlet b:embedded\nelse\nsyn keyword Inner inner\nendif\nsyn keyword Outer outer\n").unwrap();
    let program = directory_with_context(
        &directory,
        "self",
        VimLoadLimits::default(),
        &VimSetupContext::default(),
        None,
    )
    .unwrap();
    assert_eq!(program.source_files.len(), 2);
    assert!(program
        .rules
        .iter()
        .any(|r| r.group == "Inner" && r.options.contained));
    std::fs::write(directory.join("cycle.vim"), "runtime! syntax/cycle.vim\n").unwrap();
    assert!(
        super::directory(&directory, "cycle", VimLoadLimits::default())
            .unwrap_err()
            .iter()
            .any(|e| e.message.contains("cyclic"))
    );
    std::fs::write(
        directory.join("child/base.vim"),
        "source <sfile>:h/leaf.vim\n",
    )
    .unwrap();
    std::fs::write(directory.join("child/leaf.vim"), "syn keyword Leaf leaf\n").unwrap();
    std::fs::write(
        directory.join("parent.vim"),
        "source <sfile>:h/child/base.vim\n",
    )
    .unwrap();
    assert_eq!(
        super::directory(&directory, "parent", VimLoadLimits::default())
            .unwrap()
            .rule_count(),
        1
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn syntax_source_preserves_opaque_comments_but_rejects_invalid_commands() {
    assert_eq!(
        commands::decode_source(b"\" Ola S\xf6der\nsyn keyword K key\n").unwrap(),
        "\"\nsyn keyword K key\n"
    );
    assert!(commands::decode_source(b"syn keyword K \xf6\n").is_err());
}
