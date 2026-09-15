use super::*;
use crate::document::formatted_text::FormattedTextTree;

fn input(text: &str, revision: u64) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 71,
            revision,
            generation: 1,
        },
        FormattedTextTree::try_from_text(text).unwrap(),
    )
}

fn program(source: &str) -> Arc<VimProgram> {
    VimProgram::compile("fixture.vim", source, VimLoadLimits::default()).unwrap()
}

const MAKE_SYNTAX: &str = include_str!("fixtures/make.vim");

#[test]
fn pinned_make_runtime_loads_and_highlights_target_recipe_and_variable() {
    let p = program(MAKE_SYNTAX);
    assert!(p.rule_count() >= 40);
    let text = "CC := clang\nall: main.o\n\t@echo $(CC)\n";
    let input = input(text, 1);
    let result = finish(&mut VimSession::new(p), &input, 0..text.len(), 10000);
    let groups = names(&result, text.len());
    for (needle, expected) in [
        ("CC", "Identifier"),
        ("all:", "Function"),
        ("@", "Special"),
        ("echo", "Number"),
        ("$(CC)", "Identifier"),
    ] {
        let start = text.find(needle).unwrap();
        assert!(
            groups[start..start + needle.len()]
                .iter()
                .all(|group| group == expected),
            "{needle}: {:?}",
            &groups[start..start + needle.len()]
        );
    }
}

#[test]
fn setup_prefix_is_bounded_and_does_not_copy_a_giant_first_line() {
    let text = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
    let context = VimSetupContext::from_input(&input(&text, 1));
    assert_eq!(context.prefix.lines().count(), 32);
    assert!(context.prefix.ends_with("line 31"));
    let giant = input(&format!("{}\nvim9script\n", "x".repeat(100_000)), 1);
    assert!(VimSetupContext::from_input(&giant).prefix.is_empty());
}

#[test]
fn leading_context_reuses_previously_matched_unicode_content() {
    let p = program("syn match Previous /[_é]\\+/\nsyn match Follow /[^\\\\]w/lc=1\n");
    let text = "___wwww éww\n";
    let input = input(text, 1);
    let result = finish(&mut VimSession::new(p), &input, 0..text.len(), 100);
    assert_eq!(result.coverage, Coverage::Exact, "{:?}", result.diagnostics);
    let groups = names(&result, text.len());
    for (at, byte) in text.bytes().enumerate() {
        if byte == b'w' {
            assert_eq!(groups[at], "Follow", "byte {at}");
        }
    }
}

fn finish(
    session: &mut VimSession,
    input: &SyntaxInputSnapshot,
    range: Range<usize>,
    instructions: usize,
) -> VimResult {
    for _ in 0..100_000 {
        let result = session.highlight(
            input,
            range.clone(),
            VimBudget {
                instructions,
                allow_provisional: false,
                ..Default::default()
            },
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.stats.instructions <= instructions);
        if !result.stats.yielded {
            return result;
        }
    }
    panic!("Vim job did not make bounded progress");
}

fn names(result: &VimResult, len: usize) -> Vec<String> {
    let mut names = vec![String::new(); len];
    for run in &result.runs {
        for slot in &mut names[run.range.clone()] {
            *slot = run.name.0.clone();
        }
    }
    names
}

#[test]
fn strict_loader_rejects_partial_programs() {
    let errors = VimProgram::compile(
        "bad.vim",
        "syn keyword Good good\nsyn match Bad /\\%#bad/\n",
        VimLoadLimits::default(),
    )
    .unwrap_err();
    assert_eq!(errors[0].line, 2);
    assert!(errors[0].message.contains("unsupported"));
}

#[test]
fn regular_vm_preserves_priority_and_resumes_inside_matching() {
    let cases = [
        ("a.*b", "axbyb", 0..5),
        (r"a.\{-}b", "axbyb", 0..3),
        (r"\(ab\|a\)b", "ab", 0..2),
        (r"\<foo\>", "foo!", 0..3),
        ("a^b", "a^b", 0..3),
        ("a$b", "a$b", 0..3),
    ];
    for (pattern, text, expected) in cases {
        let p = VimPattern::compile(pattern, false, VimRegexLimits::default()).unwrap();
        let input = input(text, 1);
        let mut continuation = p.start(0);
        let mut calls = 0;
        loop {
            calls += 1;
            let mut fuel = 3;
            match p.resume(&mut continuation, &input, &mut fuel) {
                VimRegexProgress::Failed(message) => panic!("{message}"),
                VimRegexProgress::Pending => {
                    assert_eq!(fuel, 0);
                    assert!(calls < 10000);
                }
                VimRegexProgress::Complete(found) => {
                    let found = found.unwrap();
                    assert_eq!(found.start..found.end, expected, "{pattern}");
                    break;
                }
            }
        }
    }
}

#[test]
fn syntax_order_keywords_containment_links_and_offsets() {
    let p = program(
        r##"
syn keyword Todo contained TODO
syn match Comment "#.*" contains=Todo
syn match Earlier "foo"
syn match Later "foo"
syn keyword Keyword foo
syn match Spaced "\s#.*"ms=s+1 contains=Todo
hi link Comment CommentBase
hi def link Comment Wrong
hi def link CommentBase Comment
"##
        .replace(
            "hi def link CommentBase Comment",
            "hi def link CommentBase Final",
        )
        .as_str(),
    );
    let input = input("foo # TODO", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 4000);
    assert_eq!(
        names(&result, input.byte_len()),
        ["Keyword", "Keyword", "Keyword", "", "Spaced", "Spaced", "Todo", "Todo", "Todo", "Todo"]
    );
    assert_eq!(result.coverage, Coverage::Exact);
}

#[test]
fn syntax_group_and_cluster_identities_ignore_case_and_keep_first_spelling() {
    let source = r#"syn cluster Children contains=bar
syn region Foo start=/\[/ end=/\]/ contains=@children nextgroup=bAR
syn match Bar /x/ contained
hi link bAR Comment
"#;
    let p = program(source);
    assert_eq!(p.effective_group("BAR"), "Comment");
    let input = input("[x]x", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(
        names(&result, input.byte_len()),
        ["Foo", "Comment", "Foo", "Comment"]
    );
    assert_eq!(
        result
            .runs
            .iter()
            .filter(|run| run.name.0 == "Comment")
            .map(|run| run.origin.as_str())
            .collect::<Vec<_>>(),
        ["bar", "bar"]
    );

    let source = "hi link First CustomStyle\nsyn match first /x/\n";
    let input = self::input("x", 1);
    let result = finish(
        &mut VimSession::new(program(source)),
        &input,
        0..input.byte_len(),
        10000,
    );
    assert_eq!(
        (
            result.runs[0].origin.as_str(),
            result.runs[0].name.0.as_str()
        ),
        ("First", "CustomStyle")
    );
    for source in [
        "hi link A b\nhi link B a\n",
        "syn cluster A contains=@b\nsyn cluster B contains=@a\n",
    ] {
        let errors =
            VimProgram::compile("case-cycle.vim", source, VimLoadLimits::default()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains("cyclic")),
            "{errors:?}"
        );
    }
}

#[test]
fn syntax_include_ignores_all_clear_forms_and_keeps_nested_cluster_ownership() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("viem-vim-include-clears-{}", std::process::id())),
    );
    std::fs::create_dir_all(&directory.0).unwrap();
    let parent = "syn match Parent /P/\nsyn match Before /B/\nsyn include @Inc syntax/child.vim\nsyn region Box start=/\\[/ end=/\\]/ contains=@inc\n";
    std::fs::write(directory.0.join("parent.vim"), parent).unwrap();
    for (child, old_survives) in [
        (
            "syn match ChildOld /y/\nsyn clear\nsyn match Child /x/\n",
            true,
        ),
        ("syn clear Parent\nsyn match Child /x/\n", false),
        (
            "syn match ChildOld /y/\nsyn clear ChildOld\nsyn match Child /x/\n",
            true,
        ),
    ] {
        std::fs::write(directory.0.join("child.vim"), child).unwrap();
        let p =
            VimProgram::load_directory(&directory.0, "parent", VimLoadLimits::default()).unwrap();
        let input = input("PBxy [PBxy] {y}", 1);
        let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
        assert_eq!(
            names(&result, input.byte_len()),
            [
                "Parent",
                "Before",
                "",
                "",
                "",
                "Box",
                "Box",
                "Box",
                "Child",
                if old_survives { "ChildOld" } else { "Box" },
                "Box",
                "",
                "",
                "",
                ""
            ],
            "{child}"
        );
    }
    std::fs::write(directory.0.join("child.vim"), "syn match Outer /x/\nsyn include @Nested syntax/grand.vim\nsyn region NestedBox start=/{/ end=/}/ contains=@nested\n").unwrap();
    std::fs::write(
        directory.0.join("grand.vim"),
        "syn clear\nsyn match Inner /y/\n",
    )
    .unwrap();
    let p = VimProgram::load_directory(&directory.0, "parent", VimLoadLimits::default()).unwrap();
    let input = input("xy [x{y}y] {y}", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(
        names(&result, input.byte_len()),
        [
            "",
            "",
            "",
            "Box",
            "Outer",
            "NestedBox",
            "Inner",
            "NestedBox",
            "Box",
            "Box",
            "",
            "",
            "",
            ""
        ]
    );

    // Vim includes synchronization group names in the cluster, although their
    // patterns are recovery hints rather than ordinary highlighting rules.
    std::fs::write(directory.0.join("parent.vim"), "syn match Hidden /S/ contained\nsyn include @Inc syntax/child.vim\nsyn region Box start=/\\[/ end=/\\]/ contains=@Inc\n").unwrap();
    std::fs::write(
        directory.0.join("child.vim"),
        "syn match Child /x/\nsyn sync match Hidden /S/\n",
    )
    .unwrap();
    let p = VimProgram::load_directory(&directory.0, "parent", VimLoadLimits::default()).unwrap();
    let input = self::input("S [Sx]", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(
        names(&result, input.byte_len()),
        ["", "", "Box", "Hidden", "Child", "Box"]
    );
}

#[test]
fn inactive_throw_messages_preserve_following_inline_endif() {
    let source = "if version < 704 | throw \"old | version\" | endif\nsyn keyword Ready token\n";
    let input = input("token", 1);
    let result = finish(
        &mut VimSession::new(program(source)),
        &input,
        0..input.byte_len(),
        10000,
    );
    assert_eq!(
        names(&result, input.byte_len()),
        ["Ready", "Ready", "Ready", "Ready", "Ready"]
    );

    let errors = VimProgram::compile(
        "active-throw.vim",
        &source.replace("version < 704", "1"),
        VimLoadLimits::default(),
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("unsupported native setup command: throw")),
        "{errors:?}"
    );
    assert!(
        !errors
            .iter()
            .any(|error| error.message.contains("unterminated")),
        "{errors:?}"
    );
}

#[test]
fn unused_end_start_offsets_and_trailing_offset_commas_follow_vim() {
    let errors = VimProgram::compile(
        "retroactive-region-end.vim",
        "syn region Body start=/a/ end=/X/re=s-1",
        VimLoadLimits::default(),
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("retroactive")),
        "{errors:?}"
    );
    for (source, text, expected) in [
        ("syn region Body start=/a/ end=/X/re=s", "aXrest", vec!["Body", "Body", "", "", "", ""]),
        (r"syn region Body start=/a/me=s-99,he=s-99,re=s-99 skip=/\\./ms=s-99,hs=s-99,he=s-99,rs=s-99,re=s-99 end=/X/ms=s-99,hs=s-99,rs=s-99", "a\\XbXrest", vec!["Body", "Body", "Body", "Body", "Body", "", "", "", ""]),
        ("syn region Body start=/a/ end=/X/ms=s-1,me=s-1\nsyn match Tail /X/", "aXrest", vec!["Body", "Tail", "", "", "", ""]),
        ("syn region Body start=/a/ end=/in/me=e-2, nextgroup=Tail\nsyn match Tail /in/ contained", "aXinrest", vec!["Body", "Body", "Tail", "Tail", "", "", "", ""]),
    ] {
        let input = input(text, 1);
        let result = finish(&mut VimSession::new(program(source)), &input, 0..input.byte_len(), 10000);
        assert_eq!(names(&result, input.byte_len()), expected, "{source}");
    }
}

#[test]
fn oneline_requires_an_actual_end_and_skip_hides_escaped_quotes() {
    let p = program(r#"syn region String start=+"+ skip=+\\\\\|\\"+ end=+"+ oneline"#);
    let text = "\"yes \\\" still\"\n\"unterminated\nplain";
    let input = input(text, 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 31);
    assert_eq!(result.runs.len(), 1);
    assert_eq!(result.runs[0].range, 0..14);
}

#[test]
fn nextgroup_whitespace_and_failed_transition_retry_same_character() {
    let p=program("syn match Colon /:/ nextgroup=Value skipwhite\nsyn keyword Value contained foo\nsyn keyword Other bar\n");
    let input = input(":  foo : bar", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    let names = names(&result, input.byte_len());
    assert_eq!(&names[3..6], ["Value", "Value", "Value"]);
    assert_eq!(&names[9..12], ["Other", "Other", "Other"]);
}

#[test]
fn nextgroup_skipempty_implies_one_newline_and_then_skips_empty_lines() {
    for (options, text, expected) in [
        ("skipempty", "begin\nvalue", true),
        ("skipempty", "begin\n\n\nvalue", true),
        ("skipnl", "begin\nvalue", true),
        ("skipnl", "begin\n\nvalue", false),
        ("", "begin\nvalue", false),
    ] {
        let p = program(&format!(
            "syn match Begin /begin$/ nextgroup=Value {options}\nsyn keyword Value contained value\n"
        ));
        let input = input(text, 1);
        let result = finish(&mut VimSession::new(p), &input, 0..text.len(), 100);
        assert_eq!(
            result.runs.iter().any(|run| run.name.0 == "Value"),
            expected,
            "{options}: {text:?}"
        );
    }
}

#[test]
fn external_delimiters_survive_exact_checkpoint_restarts() {
    let p = program(r#"syn region Here start=/<<\z(\h\w*\)/ end=/^\z1$/"#);
    let text = "<<END\nbody\nEND\nafter\n";
    let input = input(text, 1);
    let mut session = VimSession::new(p);
    let first = finish(&mut session, &input, 0..11, 10000);
    assert_eq!(first.coverage, Coverage::Exact);
    let second = finish(&mut session, &input, 6..input.byte_len(), 10000);
    assert_eq!(second.runs[0].range, 6..14);
    assert_eq!(second.runs.len(), 1);
}

#[test]
fn external_delimiters_escape_vim_pattern_metacharacters() {
    let p = program(r#"syn region Quoted start=/\z([~.*[^$]\)/ end=/\z1/"#);
    for delimiter in ['~', '.', '*', '[', '^', '$'] {
        let text = format!("{delimiter}inside{delimiter} outside");
        let input = input(&text, 1);
        let result = finish(
            &mut VimSession::new(p.clone()),
            &input,
            0..text.len(),
            10000,
        );
        assert_eq!(result.runs.len(), 1, "{delimiter}");
        assert_eq!(result.runs[0].range, 0..8, "{delimiter}");
    }
}

#[test]
fn external_delimiters_use_the_final_keyword_environment() {
    for (initial, final_value, expected_end) in [
        ("@,48-57,_", "@,48-57,_,45", 17),
        ("@,48-57,_,45", "@,48-57,_", 22),
    ] {
        let syntax = format!("syn iskeyword {initial}\nsyn region Comment start=/<\\z(\\w\\+\\)>/ end=/\\z1\\k/\nsyn iskeyword {final_value}\n");
        let input = input("<END>payload END-\nnext", 1);
        let result = finish(
            &mut VimSession::new(program(&syntax)),
            &input,
            0..input.byte_len(),
            10000,
        );
        assert_eq!(
            result
                .runs
                .iter()
                .map(|run| (run.range.clone(), run.name.0.as_str()))
                .collect::<Vec<_>>(),
            [(0..expected_end, "Comment")]
        );
    }
}

#[test]
fn zero_width_end_finishes_before_the_real_newline() {
    let p = program("syn region Line start=/@/ end=/$/\n");
    let input = input("@one\ntwo", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(result.runs[0].range, 0..4);
    assert_eq!(result.runs.len(), 1);
}

#[test]
fn actual_installed_macvim_conf_file_loads_without_partial_translation() {
    let path = Path::new(
        "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax",
    );
    if !path.exists() {
        return;
    }
    let p = VimProgram::load_directory(path, "conf", VimLoadLimits::default()).unwrap();
    assert_eq!(p.rule_count(), 5);
    let input = input("# TODO\nx # FIXME\n\"literal\"\n", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 1000);
    let names = names(&result, input.byte_len());
    assert_eq!(names[0], "Comment");
    assert_eq!(names[2], "Todo");
    assert_eq!(names[9], "Comment");
    assert_eq!(names[17], "String");
}

#[test]
fn actual_installed_macvim_dosini_file_preserves_declarations() {
    let path = Path::new(
        "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax",
    );
    if !path.exists() {
        return;
    }
    let p = VimProgram::load_directory(path, "dosini", VimLoadLimits::default()).unwrap();
    let input = input("[Main]\nport=123\nname=hello\n; remark\n", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 5000);
    let names = names(&result, input.byte_len());
    assert_eq!(names[1], "Special");
    assert_eq!(names[7], "Type");
    assert_eq!(names[12], "Number");
    assert_eq!(names[21], "String");
    assert_eq!(names[29], "Comment");
}

#[test]
fn region_offsets_are_per_pattern_and_character_based() {
    let p = program(r#"syn region Comment start="/\*"hs=e+1 end="\*/"he=s-1"#);
    let input = input("/* hé */!", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(result.runs[0].range, 2..7);
    assert_eq!(result.runs.len(), 1);
    let p = program(r#"syn region Body matchgroup=Open start=/«/ matchgroup=Close end=/»/"#);
    let input = self::input("«x»", 1);
    let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
    assert_eq!(
        result
            .runs
            .iter()
            .map(|r| (r.range.clone(), r.name.0.as_str()))
            .collect::<Vec<_>>(),
        [(0..2, "Open"), (2..3, "Body"), (3..5, "Close")]
    );
}

#[test]
fn region_body_offsets_use_pattern_boundaries_for_both_delimiters() {
    // Expected groups are independently checked against Vim's synID values.
    for (start, end, expected) in [
        (
            "rs=e",
            "",
            vec![(3..6, "Open"), (6..12, "Body"), (12..15, "Close")],
        ),
        (
            "rs=e-1",
            "",
            vec![(3..5, "Open"), (5..12, "Body"), (12..15, "Close")],
        ),
        (
            "rs=s+1",
            "",
            vec![(3..4, "Open"), (4..12, "Body"), (12..15, "Close")],
        ),
        (
            "rs=e+2",
            "re=s+1",
            vec![(3..8, "Open"), (8..13, "Body"), (13..15, "Close")],
        ),
    ] {
        let syntax = format!(
            "syn region Body matchgroup=Open start=/foo/{start} matchgroup=Close end=/bar/{end}"
        );
        let input = input("abcfoostringbarabc", 1);
        let result = finish(
            &mut VimSession::new(program(&syntax)),
            &input,
            0..input.byte_len(),
            10000,
        );
        assert_eq!(
            result
                .runs
                .iter()
                .map(|run| (run.range.clone(), run.name.0.as_str()))
                .collect::<Vec<_>>(),
            expected,
            "{syntax}"
        );
    }
    let input = input("«éx»", 1);
    let syntax = "syn region Body matchgroup=Open start=/«é/rs=e-1 matchgroup=Close end=/»/";
    let result = finish(
        &mut VimSession::new(program(syntax)),
        &input,
        0..input.byte_len(),
        10000,
    );
    assert_eq!(
        result
            .runs
            .iter()
            .map(|run| (run.range.clone(), run.name.0.as_str()))
            .collect::<Vec<_>>(),
        [(0..2, "Open"), (2..5, "Body"), (5..7, "Close")]
    );
}

#[test]
fn transparent_make_target_keeps_colon_in_start_matchgroup() {
    let source = r#"syn region Target transparent matchgroup=Function start="^[a-z]\+: "rs=e-1 end="$" keepend"#;
    let input = input("all: app\nnext: main.o\n", 1);
    let result = finish(
        &mut VimSession::new(program(source)),
        &input,
        0..input.byte_len(),
        10000,
    );
    assert_eq!(
        result
            .runs
            .iter()
            .map(|run| (run.range.clone(), run.name.0.as_str()))
            .collect::<Vec<_>>(),
        [(0..4, "Function"), (9..14, "Function")]
    );
}

#[test]
fn transparent_region_end_uses_body_transparency_for_its_own_group() {
    for (group, expected) in [
        ("Target", vec![(0..4, "Target")]),
        ("Delimiter", vec![(0..4, "Delimiter"), (7..8, "Delimiter")]),
    ] {
        let source = format!(
            r#"syn region Target transparent matchgroup={group} start="^all: "rs=e-1 end="[^\\]$" keepend"#
        );
        let input = input("all: app\n", 1);
        let result = finish(
            &mut VimSession::new(program(&source)),
            &input,
            0..input.byte_len(),
            10000,
        );
        assert_eq!(
            result
                .runs
                .iter()
                .map(|run| (run.range.clone(), run.name.0.as_str()))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn keepend_preserves_contained_color_through_an_unstyled_end_pattern() {
    for (matchgroup, expected) in [
        ("", vec![(0..1, "Outer"), (1..5, "Inner")]),
        (
            "matchgroup=Delimiter",
            vec![(0..1, "Delimiter"), (1..4, "Inner"), (4..5, "Delimiter")],
        ),
    ] {
        let syntax = format!("syn region Outer {matchgroup} start=/{{/ end=/}}/ keepend contains=Inner\nsyn region Inner start=/a/ end=/$/ contained");
        let input = input("{abc} rest\n", 1);
        let result = finish(
            &mut VimSession::new(program(&syntax)),
            &input,
            0..input.byte_len(),
            10000,
        );
        assert_eq!(
            result
                .runs
                .iter()
                .map(|run| (run.range.clone(), run.name.0.as_str()))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn shifted_region_end_allows_containment_until_its_delimiter_boundary() {
    let syntax = "syn region Outer matchgroup=Delimiter start=/{/ end=/}x/re=s+1 keepend contains=Inner\nsyn match Inner /}/ contained";
    let input = input("{abc}x rest\n", 1);
    let result = finish(
        &mut VimSession::new(program(syntax)),
        &input,
        0..input.byte_len(),
        10000,
    );
    assert_eq!(
        result
            .runs
            .iter()
            .map(|run| (run.range.clone(), run.name.0.as_str()))
            .collect::<Vec<_>>(),
        [
            (0..1, "Delimiter"),
            (1..4, "Outer"),
            (4..5, "Inner"),
            (5..6, "Delimiter")
        ]
    );
}

#[test]
fn excludenl_and_keepend_control_contained_eol_extension() {
    for (flag, keepend, expected) in [
        ("", "", 0..9),
        ("excludenl ", "", 0..4),
        ("", " keepend", 0..4),
    ] {
        let p=program(&format!("syn match Continue {flag}/\\\\$/ contained\nsyn region Parent start=/@/ end=/$/ contains=Continue{keepend}\n"));
        let input = input("@hi\\\nnext\nplain", 1);
        let result = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000);
        let names = names(&result, input.byte_len());
        assert_eq!(names[expected.end], "");
        assert_eq!(
            names[expected.end - 1],
            if expected.end == 4 {
                "Continue"
            } else {
                "Parent"
            },
            "{flag}/{keepend}"
        );
        assert_eq!(
            names[5],
            if expected.end == 9 { "Parent" } else { "" },
            "{flag}/{keepend}"
        );
    }
}

#[test]
fn million_line_cold_jump_is_provisional_and_warm_repaint_is_free() {
    let text = "plain\n".repeat(1_000_000);
    let input = input(&text, 1);
    let mut session = VimSession::new(program("syn keyword Keyword return\n"));
    let range = text.len() - 120..text.len();
    let result = session.highlight(&input, range.clone(), VimBudget::default());
    assert_eq!(result.coverage, Coverage::Provisional);
    assert!(!result.stats.yielded);
    assert!(result.stats.evaluated_lines <= 60);
    assert!(result.stats.input_bytes < 4096);
    let cached = session.highlight(&input, range, VimBudget::default());
    assert_eq!(cached.stats.instructions, 0);
    assert_eq!(cached.stats.cache_hits, 1);
}

#[test]
fn pinned_make_large_document_edits_have_bounded_repair_and_free_repaint() {
    fn bounded(
        session: &mut VimSession,
        input: &SyntaxInputSnapshot,
        range: Range<usize>,
    ) -> VimResult {
        let mut instructions = 0;
        let mut bytes = 0;
        let mut lines = 0;
        for _ in 0..32 {
            let result = session.highlight(input, range.clone(), VimBudget::default());
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            instructions += result.stats.instructions;
            bytes += result.stats.input_bytes;
            lines += result.stats.evaluated_lines;
            assert!(instructions <= 4_000_000, "{instructions} instructions");
            assert!(bytes <= 128 * 1024, "{bytes} input bytes");
            assert!(lines <= 96, "{lines} evaluated lines");
            if !result.stats.yielded {
                assert_eq!(result.covered, range);
                return result;
            }
        }
        panic!("Makefile highlighting exceeded its finite slice budget");
    }
    let normalized = |result: &VimResult, start: usize| {
        result
            .runs
            .iter()
            .map(|run| {
                (
                    run.range.start - start..run.range.end - start,
                    run.name.0.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let p = program(MAKE_SYNTAX);
    let unit = "all: main.o\n\t@echo $(CC)\n";
    let local = unit.repeat(2);
    let mut edited_local = local.clone();
    edited_local.remove(3);
    let oracle_input = input(&edited_local, 1);
    let oracle = finish(
        &mut VimSession::new(p.clone()),
        &oracle_input,
        0..oracle_input.byte_len(),
        10000,
    );
    for line_count in [10_000, 1_000_000] {
        let text = unit.repeat(line_count / 2);
        let before = input(&text, 1);
        for start in [0, text.len() - local.len()] {
            let range = start..start + local.len();
            let mut session = VimSession::new(p.clone());
            let initial = bounded(&mut session, &before, range.clone());
            assert_eq!(
                initial.coverage,
                if start == 0 {
                    Coverage::Exact
                } else {
                    Coverage::Provisional
                }
            );
            let cached = session.highlight(&before, range, VimBudget::default());
            assert_eq!((cached.stats.instructions, cached.stats.cache_hits), (0, 1));

            // Deleting the target colon removes the recipe's containing context.
            // Use the persistent tree splice so unchanged suffix identities survive.
            let edit = start + 3..start + 4;
            let after = SyntaxInputSnapshot::new(
                SyntaxInputIdentity {
                    revision: 2,
                    ..before.identity()
                },
                before.text_tree().splice(edit.clone(), "").unwrap(),
            );
            session
                .apply_edit(
                    before.identity(),
                    after.identity(),
                    edit.clone(),
                    edit.start,
                )
                .unwrap();
            let changed_range = start..start + edited_local.len();
            let repaired = bounded(&mut session, &after, changed_range.clone());
            assert_eq!(
                normalized(&repaired, start),
                normalized(&oracle, 0),
                "{line_count} lines at {start}"
            );
            let cached = session.highlight(&after, changed_range, VimBudget::default());
            assert_eq!((cached.stats.instructions, cached.stats.cache_hits), (0, 1));
        }
    }
}

#[test]
fn short_line_cold_jump_obeys_sync_line_limit_below_the_byte_limit() {
    let input = input(&"x\n".repeat(10_000), 1);
    let mut session = VimSession::new(program("syn keyword Keyword x\n"));
    let range = 10_000..10_080;
    let output = session.highlight(&input, range.clone(), VimBudget::default());
    assert_eq!(output.coverage, Coverage::Provisional);
    assert_eq!(output.covered, range);
    assert!(output.stats.evaluated_lines <= 80);
    assert!(output.stats.input_bytes < 4096);
}

#[test]
fn giant_line_failed_match_yields_without_artificial_eol() {
    let text = format!("{}!", "a".repeat(4 * 1024 * 1024));
    let input = input(&text, 1);
    let mut session = VimSession::new(program(r#"syn match Expensive /a\+$/"#));
    let result = session.highlight(
        &input,
        0..80,
        VimBudget {
            instructions: 1000,
            ..Default::default()
        },
    );
    assert!(result.stats.yielded);
    assert_eq!(result.coverage, Coverage::Missing);
    assert!(result.covered.is_empty());
    assert!(result.runs.is_empty());
    assert!(result.stats.input_bytes < 1000);
    let next = session.highlight(
        &input,
        0..80,
        VimBudget {
            instructions: 1000,
            ..Default::default()
        },
    );
    assert!(next.stats.yielded);
    assert_eq!(next.stats.instructions, 1000);
}

#[test]
fn delimiter_edit_and_pending_suffix_repair_match_fresh_execution() {
    let p = program(r#"syn region String start=/"/ end=/"/"#);
    let old = input("first\n\"open\nbody\nclose\"\nafter\n", 1);
    let mut session = VimSession::new(p.clone());
    finish(&mut session, &old, 0..old.byte_len(), 10000);
    let changed = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..old.identity()
        },
        old.text_tree().splice(17..22, "").unwrap(),
    );
    session
        .apply_edit(old.identity(), changed.identity(), 17..22, 17)
        .unwrap();
    let incremental = finish(&mut session, &changed, 6..changed.byte_len(), 200);
    let fresh = finish(
        &mut VimSession::new(p),
        &changed,
        6..changed.byte_len(),
        200,
    );
    assert_eq!(incremental.runs, fresh.runs);
    assert_eq!(incremental.coverage, Coverage::Exact);
}

#[test]
fn capped_pattern_is_memoized_across_repaint_and_unrelated_edit() {
    let input = input(&format!("{}!\nunrelated\n", "a".repeat(10000)), 1);
    let mut session = VimSession::new(program(r#"syn match Expensive /a\+$/"#));
    let budget = VimBudget {
        instructions: 2000,
        match_instructions: 200,
        ..Default::default()
    };
    let capped = session.highlight(&input, 0..80, budget);
    assert!(!capped.diagnostics.is_empty());
    assert!(!capped.stats.yielded);
    let repeat = session.highlight(&input, 0..80, budget);
    assert_eq!(repeat.stats.instructions, 0);
    assert_eq!(repeat.stats.cache_hits, 1);
    let at = input.byte_len() - 2;
    let changed = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..input.identity()
        },
        input.text_tree().splice(at..at, "x").unwrap(),
    );
    session
        .apply_edit(input.identity(), changed.identity(), at..at, at + 1)
        .unwrap();
    let repeat = session.highlight(&changed, 0..80, budget);
    assert_eq!(repeat.stats.instructions, 0);
    assert_eq!(repeat.stats.cache_hits, 1);
}

#[test]
fn checkpoint_suffix_shift_does_not_visit_all_retained_states() {
    let mut checkpoints = Checkpoints::default();
    for at in (0..40000).step_by(10) {
        checkpoints.insert(
            at,
            Restart {
                anchor: None,
                regions: Vec::new(),
                next: None,
            },
        );
    }
    checkpoints.visits = 0;
    checkpoints.edit(100, 101, 109);
    assert!(checkpoints.visits < 150);
    assert_eq!(checkpoints.before(39999).unwrap().0, 39998);
    assert_eq!(checkpoints.len(), 3999);
}

#[test]
fn one_instruction_slices_make_progress_and_equal_unsliced_output() {
    let p = program("syn keyword Keyword foo\n");
    let input = input("foo bar", 1);
    let expected = finish(
        &mut VimSession::new(p.clone()),
        &input,
        0..input.byte_len(),
        10000,
    );
    let sliced = finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 1);
    assert_eq!(expected.runs, sliced.runs);
}

#[test]
fn cancelled_and_oversized_compilers_never_publish_partial_rules() {
    use std::sync::atomic::AtomicBool;
    let cancel = Arc::new(AtomicBool::new(true));
    let result = VimProgram::compile_cancellable(
        "cancelled.vim",
        "syn keyword Good yes\n",
        VimLoadLimits::default(),
        cancel,
    );
    assert!(result.unwrap_err()[0].message.contains("cancelled"));
    let result = VimProgram::compile(
        "small.vim",
        "syn keyword Good yes\n",
        VimLoadLimits {
            program_bytes: 1,
            ..Default::default()
        },
    );
    assert!(result.unwrap_err()[0].message.contains("byte budget"));
}

#[test]
fn same_shape_and_multiline_failed_dependencies_invalidate_earlier_runs() {
    let p = program("syn match Future /a\\nb/\n");
    let old = input("a\nc\n", 1);
    let mut session = VimSession::new(p.clone());
    assert!(finish(&mut session, &old, 0..old.byte_len(), 10000)
        .runs
        .is_empty());
    let changed = SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            revision: 2,
            ..old.identity()
        },
        old.text_tree().splice(2..3, "b").unwrap(),
    );
    session
        .apply_edit(old.identity(), changed.identity(), 2..3, 3)
        .unwrap();
    let repaired = finish(&mut session, &changed, 0..changed.byte_len(), 10000);
    assert_eq!(
        repaired.runs,
        finish(
            &mut VimSession::new(p),
            &changed,
            0..changed.byte_len(),
            10000
        )
        .runs
    );
    assert_eq!(repaired.runs[0].range, 0..3);
}

#[test]
fn neovim_query_regex_uses_its_declared_magic_prefix_policy() {
    for (pattern, yes, no) in [
        ("^[A-Z][A-Z_]+$", "ABC_X", "AbC"),
        (r"\m^a\+$", "aaa", "a+"),
        (r"\v^(foo|bar)$", "bar", "baz"),
        (r"\v^a\+$", "a+", "aaa"),
    ] {
        let pattern = VimPattern::compile_neovim_query(pattern, VimRegexLimits::default()).unwrap();
        assert!(pattern.is_match_text(yes, 20000).unwrap());
        assert!(!pattern.is_match_text(no, 20000).unwrap());
    }
    assert!(
        VimPattern::compile_neovim_query("a@=", VimRegexLimits::default())
            .unwrap()
            .is_match_text("a", 20000)
            .unwrap()
    );
    assert!(VimPattern::compile_neovim_query(r"\Mfoo", VimRegexLimits::default()).is_ok());
    let pattern = VimPattern::compile(r"abc\c", false, VimRegexLimits::default()).unwrap();
    assert!(pattern.is_match_text("ABC", 20000).unwrap());
}

#[test]
fn cooperative_control_interrupts_inside_the_regex_and_resumes_frozen_input() {
    let pattern = VimPattern::compile(r"a\+", false, VimRegexLimits::default()).unwrap();
    let input = input(&"a".repeat(10000), 1);
    let mut continuation = pattern.start(0);
    let mut fuel = 1000000;
    let mut polls = 0;
    let result = pattern.resume_with_control(&mut continuation, &input, &mut fuel, &mut || {
        polls += 1;
        polls > 4
    });
    assert_eq!(result, VimRegexProgress::Pending);
    assert!(fuel > 990000);
    assert!(continuation.instructions < 1000);
    let result = pattern.resume(&mut continuation, &input, &mut fuel);
    assert_eq!(
        match result {
            VimRegexProgress::Complete(Some(found)) => found.end,
            _ => panic!("continuation did not complete"),
        },
        10000
    );
}

/// The optional installed-runtime test compares observable effective Vim groups
/// at every source byte; fixture availability is separate from portable tests.
#[test]
fn installed_macvim_runtime_matches_reference_colors_byte_for_byte() {
    let runtime = Path::new(
        "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax",
    );
    let macvim = Path::new("/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/MacOS/Vim");
    let executable = if macvim.exists() {
        macvim
    } else {
        Path::new("/usr/bin/vim")
    };
    if !executable.exists() {
        return;
    }
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("viem-vim-reference-{}", std::process::id())));
    std::fs::create_dir_all(&directory.0).unwrap();
    for (language, text) in [
        (
            "make",
            "# TODO: build\nCC := clang\nSOURCES = $(wildcard *.c)\n.PHONY: all clean\nall: app\n\t@echo \"building $(SOURCES)\"\napp: main.o\n\t$(CC) -o $@ $^\nclean:\n\trm -f app\ninclude config.mk\nifeq ($(DEBUG),1)\nCFLAGS += -g\nendif\n",
        ),
        (
            "make",
            "define compile\n$(CC) -o $$@ $$<\nendef\n\nall: main.o \\\n helper.o # dependencies\n\t@echo '$(CC)' `date`\n\tprintf \"done\"\ninline: ; @echo ok\nempty:\n malformed recipe\n",
        ),
        (
            "conf",
            "# TODO\nx # FIXME\n\"quoted \\\" value\"\n'unclosed\n",
        ),
        (
            "dosini",
            "[Main]\nport=123\nname=hello\n; remark\n[Second]\nx=4.2\n",
        ),
        (
            "vim",
            "\" editor settings\nset number\nset tabstop=4\nlet g:example = 'value'\nnnoremap <leader>w :write<CR>\nif has('gui_running')\n  colorscheme desert\nendif\n",
        ),
        (
            "vim",
            "vim9script\n# editor settings\nset number\nvar enabled = true\ndef Configure(): bool\n  return enabled\nenddef\n",
        ),
    ] {
        if language != "make" && !runtime.exists() { continue; }
        let syntax_path = if language == "make" {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/document/syntax/vim/fixtures/make.vim")
        } else { runtime.join(format!("{language}.vim")) };
        let input_path = directory.0.join("input.txt");
        let output_path = directory.0.join("groups.txt");
        let script_path = directory.0.join("reference.vim");
        std::fs::write(&input_path, text).unwrap();
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let script=format!("set nomore\nset encoding=utf-8\nexecute 'edit ' . fnameescape({})\nsyntax clear\nunlet! b:current_syntax\nexecute 'source ' . fnameescape({})\nsyntax sync fromstart\nlet result = []\nfor lnum in range(1, line('$'))\n  for col in range(1, strlen(getline(lnum)))\n    call add(result, synIDattr(synIDtrans(synID(lnum,col,1)), 'name'))\n  endfor\nendfor\ncall writefile(result,{})\nqa!\n",quote(&input_path),quote(&syntax_path),quote(&output_path));
        std::fs::write(&script_path, script).unwrap();
        let mut child = std::process::Command::new(executable)
            .args(["-Nu", "NONE", "-n", "-es", "-i", "NONE", "-S"])
            .arg(&script_path)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "reference Vim failed");
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("reference Vim exceeded fixture deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let reference = std::fs::read_to_string(&output_path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let input = input(text, 1);
        let p = if language == "make" { program(MAKE_SYNTAX) } else { VimProgram::load_directory_with_context(
            runtime, language, VimLoadLimits::default(),
            &VimSetupContext::from_input(&input),
            &std::sync::atomic::AtomicBool::new(false),
        ).unwrap() };
        let actual = names(
            &finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000),
            text.len(),
        )
        .into_iter()
        .zip(text.bytes())
        .filter_map(|(name, byte)| (byte != b'\n').then_some(name))
        .collect::<Vec<_>>();
        assert_eq!(actual.len(), reference.len(), "installed {language}.vim byte count");
        let mismatches = text.bytes().enumerate().filter(|(_, byte)| *byte != b'\n')
            .zip(actual.iter().zip(&reference))
            .filter_map(|((offset, byte), (actual, expected))| (actual != expected).then_some((offset, char::from(byte), actual, expected)))
            .take(16).collect::<Vec<_>>();
        assert!(mismatches.is_empty(), "installed {language}.vim effective groups differ (byte, character, actual, expected): {mismatches:?}");
    }
}
