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
        "syn keyword Good good\nsyn match Bad /\\%23lbad/\n",
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
    assert!(VimPattern::compile_neovim_query(r"\Mfoo", VimRegexLimits::default()).is_err());
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
    let executable =
        Path::new("/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/MacOS/Vim");
    if !runtime.exists() || !executable.exists() {
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
        let input_path = directory.0.join("input.txt");
        let output_path = directory.0.join("groups.txt");
        let script_path = directory.0.join("reference.vim");
        std::fs::write(&input_path, text).unwrap();
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let script=format!("set nomore\nset encoding=utf-8\nexecute 'edit ' . fnameescape({})\nsyntax clear\nunlet! b:current_syntax\nexecute 'source ' . fnameescape({})\nsyntax sync fromstart\nlet result = []\nfor lnum in range(1, line('$'))\n  for col in range(1, strlen(getline(lnum)))\n    call add(result, synIDattr(synIDtrans(synID(lnum,col,1)), 'name'))\n  endfor\nendfor\ncall writefile(result,{})\nqa!\n",quote(&input_path),quote(&runtime.join(format!("{language}.vim"))),quote(&output_path));
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
        let p = VimProgram::load_directory_with_context(
            runtime, language, VimLoadLimits::default(),
            &VimSetupContext::from_input(&input),
            &std::sync::atomic::AtomicBool::new(false),
        ).unwrap();
        let actual = names(
            &finish(&mut VimSession::new(p), &input, 0..input.byte_len(), 10000),
            text.len(),
        )
        .into_iter()
        .zip(text.bytes())
        .filter_map(|(name, byte)| (byte != b'\n').then_some(name))
        .collect::<Vec<_>>();
        assert_eq!(
            actual, reference,
            "installed {language}.vim effective groups"
        );
    }
}
