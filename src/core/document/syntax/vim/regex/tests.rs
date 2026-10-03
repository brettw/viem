use super::*;

fn bundled_syntax_source(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/vim/runtime/syntax")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read bundled syntax {}: {error}", path.display()))
}

fn find(pattern: &str, text: &str, slice: usize) -> Option<std::ops::Range<usize>> {
    let pattern = VimPattern::compile(pattern, false, VimRegexLimits::default()).unwrap();
    find_pattern(&pattern, text, slice)
}

fn find_pattern(pattern: &VimPattern, text: &str, slice: usize) -> Option<std::ops::Range<usize>> {
    let tree = crate::document::formatted_text::FormattedTextTree::try_from_text(text).unwrap();
    for at in text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
    {
        let mut continuation = pattern.start(at);
        for _ in 0..1_000_000 {
            let mut fuel = slice;
            match pattern.resume_reader(
                &mut continuation,
                text.len(),
                |at| text.as_bytes().get(at).copied(),
                &|at| {
                    let row = tree.hard_line_at_byte(at).ok()?;
                    Some((row + 1, at - tree.hard_line_start(row).ok()? + 1))
                },
                &mut fuel,
                &mut || false,
            ) {
                VimRegexProgress::Pending => assert_eq!(fuel, 0),
                VimRegexProgress::Failed(error) => panic!("{error}"),
                VimRegexProgress::Complete(Some(found)) => return Some(found.start..found.end),
                VimRegexProgress::Complete(None) => break,
            }
        }
    }
    None
}

#[test]
fn syntax_magic_modes_numeric_atoms_and_collection_escapes_are_resumable() {
    let cases = [
        (r"\V${", "${target}", Some(0..2)),
        (r"\V\[-+/*=^&?|!><%~]", "x+y", Some(1..2)),
        (r#"\M**\|*'\|*""#, "a**b", Some(1..3)),
        (r"\M\[0-9]\+", "a123b", Some(1..4)),
        (r"\M\[0-9A-F\]\+", "[0-9A-F]", Some(0..8)),
        (r"[abc", "[abc", Some(0..4)),
        (r"[]", "[]", Some(0..2)),
        (r"^*", "*", Some(0..1)),
        (r"^*", "abc", None),
        (r"\M^a\.\*$", "abc", Some(0..3)),
        (r"\V\^a\.\*\$", "abc", Some(0..3)),
        (r"\V^a$", "a ^a$ b", Some(2..5)),
        (r"\Va.*\mb.*", "a.*bcd", Some(0..6)),
        (r"\V\(ab\)\+\1", "ababab", Some(0..6)),
        (r"\%d65\%o101\%x41\%u0041\%U00000041", "AAAAA", Some(0..5)),
        (r"\%u03a3\%U0001f600", "Σ😀", Some(0..6)),
        (r"[\x00-\x7f]\+", "abcé", Some(0..3)),
        (r"[\u2010-]", "‐", Some(0..3)),
        (r"[\d65\o101\x41]", "A", Some(0..1)),
        (r"[\s]", " s", Some(1..2)),
        (r"[\s]", "\\", Some(0..1)),
        (r"[\+]", "+", Some(0..1)),
        (r"[\+]", "\\", Some(0..1)),
        (r"[[:ident:]]\+", "café", Some(0..5)),
        (r"[[:ident:]]", "Σ", None),
        (r"[[:lower:]]\+", "café", Some(0..5)),
        (r"\F\f*", "12/usr/é.txt", Some(2..13)),
        (r"[[:fname:]]\+", "/a/b.txt ", Some(0..8)),
        (r"\%\(^\|\s\)#", "x #comment", Some(1..3)),
        (r"\<r\%[[eo]ad]\>", "road", Some(0..4)),
        (r"\<r\%[[eo]ad]\>", "rod", None),
        (r"index\%[[[]0[]]]", "index[0]", Some(0..8)),
        (r"x\%[\d\x]", "x2a", Some(0..3)),
        (r"[[.é.]]", "é", Some(0..2)),
        (r"[| \t([.,=\]]", "[", Some(0..1)),
        (r"[^[:space]]", "x]", Some(0..2)),
        (r"[[:unknown:]]", "u]", Some(0..2)),
        (r"\_\s\{-}>", "  >", Some(0..3)),
        (r"\p\+", "é 😀\u{200b}", Some(0..7)),
        (r"\P\+", "12aé", Some(2..5)),
        (r"[[:print:]]\+", "\t café\n", Some(1..7)),
        (
            r"\<\(linear-\|radial-\|conic-\)\=\gradient\s*(",
            "linear-gradient(",
            Some(0..16),
        ),
    ];
    for (pattern, text, expected) in cases {
        for fuel in [1, 8192] {
            assert_eq!(find(pattern, text, fuel), expected, "{pattern} in {text}");
        }
    }
    for pattern in [
        r"\%u",
        r"\%U00110000",
        r"\%uD800",
        r"\%V",
        r"\M\~",
        r"\V\~",
        r"\M\*",
        r"\V\*",
    ] {
        assert!(
            VimPattern::compile(pattern, false, VimRegexLimits::default()).is_err(),
            "{pattern}"
        );
    }
}

#[test]
fn keyword_options_snapshot_classes_boundaries_and_assertions() {
    let original = VimKeyword::default();
    let custom = VimKeyword::parse("@,48-57,_,-,^a-z").unwrap();
    for (source, expected_original, expected_custom) in [
        (r"\<one\>", Some(0..3), None),
        (r"\k\+", Some(0..3), Some(3..7)),
        (r"\K\+", Some(0..3), Some(3..7)),
        (r"[[:keyword:]]\+", Some(0..3), Some(3..7)),
        (r"\%(\<ONE\>\)\@=ONE", Some(4..7), None),
    ] {
        for (environment, expected) in [(&original, expected_original), (&custom, expected_custom)]
        {
            let pattern = VimPattern::compile_with_keyword(
                source,
                false,
                VimRegexLimits::default(),
                environment,
            )
            .unwrap();
            assert_eq!(
                find_pattern(&pattern, "one-ONE", 1),
                expected,
                "{source}: {environment:?}"
            );
        }
    }
    let cases = [
        ("48-57,,,_", ','),
        (" -~,^,,9", '\t'),
        ("@-@", '@'),
        ("^", '^'),
        ("45,92", '\\'),
    ];
    for (option, member) in cases {
        assert!(
            VimKeyword::parse(option).unwrap().contains(member),
            "{option}"
        );
    }
    assert!(!VimKeyword::parse(" -~,^,,9").unwrap().contains(','));
    for option in [
        "256",
        "z-a",
        "a-999",
        "a,",
        "abc",
        "999999999999999999999999999999999",
    ] {
        assert!(VimKeyword::parse(option).is_err(), "{option}");
    }
    let lower = VimKeyword::parse("a-z").unwrap();
    let pattern =
        VimPattern::compile_with_keyword(r"\c\k\+", false, VimRegexLimits::default(), &lower)
            .unwrap();
    assert_eq!(find_pattern(&pattern, "ABC abc", 1), Some(4..7));
}

#[test]
fn bounded_text_find_validates_start_and_preserves_capture_offsets() {
    let pattern = VimPattern::compile(r"\(é\)\zs\1", false, VimRegexLimits::default()).unwrap();
    let mut fuel = 10_000;
    let found = pattern
        .find_text_with_control("éé éé", 4, &mut fuel, &mut || false)
        .unwrap()
        .unwrap();
    assert_eq!(found.start..found.end, 7..9);
    assert_eq!(found.captures[1], Some(5..7));
    assert!(pattern
        .find_text_with_control("é", 1, &mut fuel, &mut || false)
        .is_err());
    assert!(pattern
        .find_text_with_control("é", 3, &mut fuel, &mut || false)
        .is_err());
    assert!(pattern
        .find_text_with_control("é", 0, &mut 0, &mut || false)
        .is_err());
    assert!(pattern
        .find_text_with_control("é", 0, &mut fuel, &mut || true)
        .is_err());
}

#[test]
fn final_keyword_environment_rebind_preserves_declaration_case() {
    let limits = VimRegexLimits::default();
    let pattern = VimPattern::compile(r"FOO\k*", true, limits).unwrap();
    assert_eq!(find_pattern(&pattern, "foo-bar", 1), Some(0..3));
    let keyword = VimKeyword::parse("@,48-57,_,-").unwrap();
    let rebound = pattern.rebind_keyword(&keyword, limits).unwrap();
    assert_eq!(find_pattern(&rebound, "foo-bar", 1), Some(0..7));
    // Rebinding produces an immutable program; the old pattern keeps its env.
    assert_eq!(find_pattern(&pattern, "foo-bar", 1), Some(0..3));
}

#[test]
fn alternate_external_capture_group_spelling_retains_delimiter() {
    let pattern =
        VimPattern::compile(r"{\z\([a-z_]*\)|", false, VimRegexLimits::default()).unwrap();
    assert_eq!(pattern.external_groups, [1]);
    let found = pattern
        .find_text_with_control("{delim_|", 0, &mut 10_000, &mut || false)
        .unwrap()
        .unwrap();
    assert_eq!(found.start..found.end, 0..8);
    assert_eq!(found.captures[1], Some(1..7));
}

#[test]
fn matcher_selection_prefixes_are_portable_and_position_failures_are_precise() {
    for selector in [0, 1, 2] {
        let source = format!(r"\%#={selector}\<\%(true\|false\)\>[?!]\@!");
        assert_eq!(find(&source, "true? false", 1), Some(6..11));
    }
    for (source, reason) in [
        (r"\%#=3", "engine selector"),
        (r"\%#", "editor cursor-position"),
        (r"\%V", "editor Visual-selection"),
        (r"\%'m", "editor mark-position"),
        (r"\%>.l", "editor current-position"),
    ] {
        let error = VimPattern::compile(source, false, VimRegexLimits::default()).unwrap_err();
        assert!(error.contains(reason), "{source}: {error}");
    }
}

#[test]
fn numeric_controls_distinguish_source_nul_hard_lines_and_string_lf() {
    for source in [
        r"\%d0",
        r"\%d10",
        r"\%o000",
        r"\%x0a",
        r"\%u0000",
        r"[\d0]",
        r"[\d10]",
        r"[\x00-\x09]",
        r"[\x01-\x0a]",
        r"[\x09-\x0b]",
    ] {
        let pattern = VimPattern::compile(source, false, VimRegexLimits::default()).unwrap();
        assert!(!pattern.multiline, "{source}");
        for slice in [1, 8192] {
            assert_eq!(
                find_pattern(&pattern, "a\0b\nc", slice),
                Some(1..2),
                "{source}"
            );
            assert_eq!(find_pattern(&pattern, "a\nb", slice), None, "{source}");
        }
        let mut fuel = 1000;
        assert_eq!(
            pattern
                .find_text_with_control("a\nb", 0, &mut fuel, &mut || false)
                .unwrap()
                .map(|m| m.start..m.end),
            Some(1..2),
            "{source}"
        );
        let mut fuel = 1000;
        assert!(pattern
            .find_text_with_control("a\nb", 0, &mut fuel, &mut || true)
            .is_err());
        assert_eq!(fuel, 1000);
    }
    for (source, text, expected) in [
        (r"a[^\x00]b", "a\0b", None),
        (r"a[^\x0a]b", "a\0b", None),
        (r"a[^\x0a]b", "axb", Some(0..3)),
        (r"[\x00-\x09]", "\t", Some(0..1)),
        (r"[\x00-\x09]", "\u{b}", None),
        (r"[\x01-\x0a]", "\u{1}", Some(0..1)),
        (r"[\x09-\x0b]", "\u{b}", Some(0..1)),
        (r"[\x09-\x0b]", "\u{8}", None),
        (r"\n", "a\0b\nc", Some(3..4)),
        (r"\0", "a\0b\n0", Some(4..5)),
        (r"a\%[\%d0b]", "a\0b", Some(0..3)),
        (r"[\d0\n]", "\n", Some(0..1)),
        (r"[\d0\n]", "\0", Some(0..1)),
        (r"\_[\d0]", "\n", Some(0..1)),
    ] {
        assert_eq!(find(source, text, 1), expected, "{source}");
    }
}

#[test]
fn absolute_source_line_and_byte_column_assertions_are_resumable() {
    for (source, text, expected) in [
        (r"\%2lfoo", "foo\nfoo\nfoo", Some(4..7)),
        (r"\%<3lfoo", "foo\nfoo\nfoo", Some(0..3)),
        (r"\%>2lfoo", "foo\nfoo\nfoo", Some(8..11)),
        (r"\%0l", "", None),
        (r"\%>0l", "", Some(0..0)),
        (r"\%3cx", "éx", Some(2..3)),
        (r"\%2cx", "éx", None),
        (r"\%1cx", "a\nx", Some(2..3)),
        (r"\%>3c.", "abcéx", Some(3..5)),
        (r"\%<3c.", "éx", Some(0..2)),
        (r"\%3l$", "a\nb\n", Some(4..4)),
        (r"\%2l\%3cx", "a\néx", Some(4..5)),
        (r"\v%2l%3cx", "a\néx", Some(4..5)),
        (r"\%(\%2l\)\@!foo", "foo\nfoo", Some(0..3)),
    ] {
        for fuel in [1, 8192] {
            assert_eq!(find(source, text, fuel), expected, "{source} in {text}");
        }
    }
    for (source, global) in [(r"\%2lfoo", true), (r"\%3cx", false)] {
        assert_eq!(
            VimPattern::compile(source, false, VimRegexLimits::default())
                .unwrap()
                .multiline,
            global
        );
    }
    // Vim string predicates have no source-line context, and their byte-column
    // values are relative to the string even when it contains a newline.
    for (source, text, expected) in [
        (r"\%1l", "foo", false),
        (r"\%<3l", "foo", false),
        (r"\%3cx", "a\nx", true),
        (r"\%1cx", "a\nx", false),
    ] {
        assert_eq!(
            VimPattern::compile(source, false, VimRegexLimits::default())
                .unwrap()
                .is_match_text(text, 1000)
                .unwrap(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn virtual_columns_use_portable_vim_cells_and_resume_with_single_instruction_slices() {
    for (source, text, expected) in [
        (r"\%1v.", "x", Some(0..1)),
        (r"\%0v.", "x", None),
        (r"\%2v\t", "a\tx", Some(1..2)),
        (r"\%3v.", "a\tx", None),
        (r"\%9vx", "a\tx", Some(2..3)),
        (r"\%>8vx", "\tx", Some(1..2)),
        (r"\%<9vx", "\tx", None),
        (r"\%2vx", "éx", Some(2..3)),
        (r"\%3vx", "界x", Some(3..4)),
        (r"\%3vx", "😀x", Some(4..5)),
        (r"\%2vx", "e\u{301}x", Some(3..4)),
        (r"\%2vx", "\u{301}x", Some(2..3)),
        (r"\%2vx", "♥\u{fe0f}x", Some(6..7)),
        (r"\%3vx", "\u{1}x", Some(1..2)),
        (r"\%5vx", "\u{85}x", Some(2..3)),
        (r"\%7vx", "\u{200b}x", Some(3..4)),
        (r"\%2vx", "\u{ad}x", Some(2..3)),
        (r"\%2vx", "\u{1160}x", Some(3..4)),
        (r"\%3vx", "\u{1f1e6}x", Some(4..5)),
        (r"\%2vx", "\u{e0020}x", Some(4..5)),
        (r"\%9vx", "prefix\n\tx", Some(8..9)),
        (r"\v%9vx", "\tx", Some(1..2)),
        (r"\%(\%<3v.\)\+", "é界x", Some(0..5)),
        (r"^.*\%3vx", "abx", Some(0..3)),
        (r"\%(\%9v\)\@!x", "\tx x", Some(3..4)),
        (r"\%4v$", "abc", Some(3..3)),
    ] {
        for slice in [1, 8192] {
            assert_eq!(find(source, text, slice), expected, "{source} in {text:?}");
        }
    }
    // Virtual columns are source-line-local; edits retain ordinary whole-line
    // repair instead of forcing global syntax invalidation.
    let pattern = VimPattern::compile(r"\%9vx", false, VimRegexLimits::default()).unwrap();
    assert!(!pattern.multiline);
    assert!(pattern.is_match_text("\tx", 100).unwrap());
    // Vim string inputs treat LF as a displayed control, not a line boundary.
    let string = VimPattern::compile(r"\%4vx", false, VimRegexLimits::default()).unwrap();
    assert!(string.is_match_text("a\nx", 100).unwrap());
}

#[test]
fn virtual_column_prefix_work_is_cached_metered_and_cancellable() {
    let text = "x".repeat(2_048);
    let input = SyntaxInputSnapshot::new(
        crate::document::syntax::SyntaxInputIdentity {
            document: 72,
            revision: 1,
            generation: 1,
        },
        crate::document::formatted_text::FormattedTextTree::try_from_text(text.as_str()).unwrap(),
    );
    let pattern = VimPattern::compile(r"\%<2049v.\+", false, VimRegexLimits::default()).unwrap();
    // An assertion late in a hostile line cannot hide a prefix scan in one
    // instruction, and cancellation retains its resumable scan position.
    let mut continuation = pattern.start(text.len() - 1);
    let mut fuel = 8;
    assert_eq!(
        pattern.resume(&mut continuation, &input, &mut fuel),
        VimRegexProgress::Pending
    );
    assert_eq!(fuel, 0);
    let mut fuel = 8;
    assert_eq!(
        pattern.resume_with_control(&mut continuation, &input, &mut fuel, &mut || true),
        VimRegexProgress::Pending
    );
    assert_eq!(fuel, 8);
    let mut fuel = 20_000;
    assert!(matches!(
        pattern.resume(&mut continuation, &input, &mut fuel),
        VimRegexProgress::Complete(Some(_))
    ));

    // Git's repeated column guard must share its prefix work, not rescan every
    // previous character at every next position (which would be quadratic).
    let repeated =
        VimPattern::compile(r"\%(\%<2049v.\)\+", false, VimRegexLimits::default()).unwrap();
    let mut continuation = repeated.start(0);
    let mut fuel = 80_000;
    let result = repeated.resume(&mut continuation, &input, &mut fuel);
    assert!(matches!(
        result,
        VimRegexProgress::Complete(Some(VimRegexMatch {
            start: 0,
            end: 2048,
            ..
        }))
    ));
    assert!(
        fuel > 40_000,
        "virtual columns rescanned unchanged prefixes"
    );

    let summary = VimPattern::compile(r"^.*\%<51v.", false, VimRegexLimits::default()).unwrap();
    let mut continuation = summary.start(0);
    let mut fuel = 25_000;
    let result = summary.resume(&mut continuation, &input, &mut fuel);
    assert!(matches!(
        result,
        VimRegexProgress::Complete(Some(VimRegexMatch {
            start: 0,
            end: 50,
            ..
        }))
    ));
    assert!(
        fuel > 10_000,
        "greedy summary backtracking rescanned line prefixes"
    );
}

#[test]
fn source_position_lookup_remains_bounded_at_a_million_lines() {
    let text = format!("{}éx", "a\n".repeat(1_000_000));
    let input = SyntaxInputSnapshot::new(
        crate::document::syntax::SyntaxInputIdentity {
            document: 71,
            revision: 1,
            generation: 1,
        },
        crate::document::formatted_text::FormattedTextTree::try_from_text(text.as_str()).unwrap(),
    );
    let pattern =
        VimPattern::compile(r"\%1000001l\%3cx", false, VimRegexLimits::default()).unwrap();
    let mut continuation = pattern.start(text.len() - 1);
    let mut fuel = 32;
    assert_eq!(
        pattern.resume_with_control(&mut continuation, &input, &mut fuel, &mut || true),
        VimRegexProgress::Pending
    );
    assert_eq!(fuel, 32);
    let found = pattern.resume(&mut continuation, &input, &mut fuel);
    assert!(
        matches!(found, VimRegexProgress::Complete(Some(VimRegexMatch { start, end, .. })) if start == text.len() - 1 && end == text.len())
    );
    assert!(
        fuel >= 24,
        "source lookup scanned instead of using tree aggregates"
    );
}

#[test]
fn source_position_atoms_match_installed_vim_buffer_searches() {
    use std::process::Command;
    let executable = std::env::var_os("VIEM_VIM_REGEX_ORACLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/MacOS/Vim".into()
        });
    if !executable.is_file() {
        return;
    }
    let cases = [
        (r"\%2lfoo", "foo\nfoo\nfoo"),
        (r"\%<3lfoo", "foo\nfoo\nfoo"),
        (r"\%>2lfoo", "foo\nfoo\nfoo"),
        (r"\%3cx", "éx"),
        (r"\%2cx", "éx"),
        (r"\%1cx", "a\nx"),
        (r"\%>3c.", "abcéx"),
        (r"\%<3c.", "éx"),
        (r"\%2l\%3cx", "a\néx"),
        (r"\v%2l%3cx", "a\néx"),
        (r"\%(\%2l\)\@!foo", "foo\nfoo"),
        (r"\%2v\t", "a\tx"),
        (r"\%3v.", "a\tx"),
        (r"\%9vx", "a\tx"),
        (r"\%>8vx", "\tx"),
        (r"\%<9vx", "\tx"),
        (r"\%2vx", "éx"),
        (r"\%3vx", "界x"),
        (r"\%3vx", "😀x"),
        (r"\%2vx", "e\u{301}x"),
        (r"\%2vx", "\u{301}x"),
        (r"\%2vx", "♥\u{fe0f}x"),
        (r"\%3vx", "\u{1}x"),
        (r"\%5vx", "\u{85}x"),
        (r"\%7vx", "\u{200b}x"),
        (r"\%2vx", "\u{ad}x"),
        (r"\%2vx", "\u{1160}x"),
        (r"\%3vx", "\u{1f1e6}x"),
        (r"\%2vx", "\u{e0020}x"),
        (r"\%9vx", "prefix\n\tx"),
        (r"\%(\%<3v.\)\+", "é界x"),
        (r"^.*\%3vx", "abx"),
        (r"\%(\%9v\)\@!x", "\tx x"),
        (r"\%d0", "a\0b\nc"),
        (r"\%d10", "a\0b\nc"),
        (r"[\x00-\x09]", "a\0b\nc"),
        (r"[\x01-\x0a]", "a\0b\nc"),
        (r"[\x09-\x0b]", "a\0b\nc"),
        (r"a[^\x00]b", "a\0b\nc"),
        (r"a[^\x0a]b", "a\0b\nc"),
        (r"\0", "a\0b0\nc"),
    ];
    let directory = std::env::temp_dir().join(format!(
        "viem-source-position-oracle-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("results.json");
    let script = directory.join("oracle.vim");
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let mut program = String::from("set encoding=utf-8\nlet results=[]\n");
    for (pattern, text) in cases {
        program.push_str(&format!("enew!\ncall setline(1, {})\ncall cursor(1,1)\nlet first=searchpos({}, 'cnW')\nlet last=searchpos({}, 'cenW')\nif first[0] == 0\ncall add(results, [-1,-1])\nelse\ncall add(results, [line2byte(first[0])+first[1]-2, line2byte(last[0])+last[1]-2+strlen(matchstr(strpart(getline(last[0]),last[1]-1), '^.'))])\nendif\n", serde_json::to_string(&text.split('\n').map(|line|line.replace('\0', "\n")).collect::<Vec<_>>()).unwrap(), quote(pattern), quote(pattern)));
    }
    program.push_str(&format!(
        "call writefile([json_encode(results)], {})\nqa!\n",
        quote(output.to_str().unwrap())
    ));
    std::fs::write(&script, program).unwrap();
    let result = Command::new(executable)
        .args([
            "-u", "NONE", "-U", "NONE", "-i", "NONE", "-n", "-N", "-es", "-S",
        ])
        .arg(script)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected: Vec<[isize; 2]> =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    for ((pattern, text), [start, end]) in cases.into_iter().zip(expected) {
        assert_eq!(
            find(pattern, text, 1),
            (start >= 0).then_some(start as usize..end as usize),
            "Vim buffer differential: {pattern} in {text}"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bundled_clojure_unicode_block_pattern_fits_bounded_source_profile() {
    let limits = VimRegexLimits::default();
    assert_eq!(limits.pattern_bytes, 16 * 1024);
    let error =
        VimPattern::compile(&"x".repeat(limits.pattern_bytes + 1), false, limits).unwrap_err();
    assert!(error.contains("pattern byte budget"));
    let source = bundled_syntax_source("clojure.vim");
    let source = source
        .lines()
        .filter_map(|line| {
            line.strip_prefix("syntax match clojureRegexpUnicodeCharClass \"")
                .and_then(|line| line.split('"').next())
        })
        .max_by_key(|source| source.len())
        .unwrap();
    assert!(source.len() > 8192);
    let pattern = VimPattern::compile(source, false, limits).unwrap();
    assert!(pattern.memory_usage() < limits.nfa_bytes);
    assert!(pattern
        .is_match_text(r"\p{InBasicLatin}", 1_000_000)
        .unwrap());
    assert!(!pattern
        .is_match_text(r"\p{InNotAUnicodeBlock}", 1_000_000)
        .unwrap());
}

#[test]
fn bundled_j_number_pattern_has_resumable_semantics() {
    let source = bundled_syntax_source("j.vim");
    let pattern = source
        .lines()
        .filter_map(|line| {
            line.split_once("jNumber /")
                .and_then(|(_, source)| source.strip_suffix('/'))
        })
        .max_by_key(|pattern| pattern.len())
        .unwrap();
    let pattern = VimPattern::compile(pattern, false, VimRegexLimits::default()).unwrap();
    for text in ["_3", "3r4", "2j3", "2ad90", "16bff", "1e_3", "_0.25"] {
        for fuel in [1, 8192] {
            assert_eq!(
                find_pattern(&pattern, text, fuel),
                Some(0..text.len()),
                "{text}"
            );
        }
    }
}

#[test]
fn installed_vim_syntax_regex_atoms_have_resumable_semantics() {
    let cases = [
        (r"\<se\%[t]\>", "set number", Some(0..3)),
        (r"\<se\%[t]\>", "se number", Some(0..2)),
        (r"\<se\%[t]\>", "setting", None),
        (r"\a\@1<=!", "set!", Some(3..4)),
        (r"\\\@1<!|", "x\\|y|", Some(4..5)),
        (r"\<new\>(\@!", "new() new", Some(6..9)),
        (r"\%(\<as\s\+\)\@<=\h\w*\>", "import as name", Some(10..14)),
        (r"\%(a*\)\@>a", "aaa", None),
        (r"\(a\|ab\)\1$", "abab", Some(0..4)),
        (r"\(foo\)\zs\1", "foofoo", Some(3..6)),
        (r"a\&ab", "ab", Some(0..2)),
        (r"[^][:space:]]\+", "a[b c", Some(0..3)),
        (r"[gjf]\{,3\}", "gjfgg", Some(0..3)),
        (r"\u\l\+", "Hello", Some(0..5)),
        (r"\c\l\+", "ABC", None),
        (r"\c\u\+", "abc", None),
        (r"\%(\(a*\)\)\@<=b\1", "aaabaaa", Some(3..7)),
        (r"\%#=1\<se\%[t]\>", "set", Some(0..3)),
    ];
    for (pattern, text, expected) in cases {
        assert_eq!(find(pattern, text, 1), expected, "{pattern} in {text}");
        assert_eq!(find(pattern, text, 8192), expected, "{pattern} in {text}");
    }
}

#[test]
fn vim_lookbehind_byte_limits_and_multiline_atoms() {
    assert_eq!(find(r"\%(ab\)\@1<=!", "ab!", 1), None);
    assert_eq!(find(r"\%(ab\)\@2<=!", "ab!", 1), Some(2..3));
    assert_eq!(find(r"\%(é\)\@1<=!", "é!", 1), Some(2..3));
    assert_eq!(find(r"\%(é\)\@2<=!", "é!", 1), Some(2..3));
    assert_eq!(find(r"\%({\n\=\)\@1<=x", "{\nx", 1), Some(2..3));
    assert_eq!(find(r"a\_s\+b", "a\n b", 1), Some(0..4));
    assert_eq!(find(r"a\_[^]]\+b", "a\n b", 1), Some(0..4));
    assert_eq!(find(r"\%(a\n.*\n\)\@<=x", "a\nb\nx", 1), None);
}

#[test]
fn same_line_assertions_preserve_local_repair_and_bounded_lookbehind() {
    let pattern = VimPattern::compile(r"\a\@<=!", false, VimRegexLimits::default()).unwrap();
    assert!(!pattern.multiline);
    assert_eq!(pattern.minimum_chars, 1);
    let text = format!("{}!", "a".repeat(200_000));
    let mut continuation = pattern.start(text.len() - 1);
    let mut fuel = 64;
    assert!(matches!(
        pattern.resume_reader(
            &mut continuation,
            text.len(),
            |at| text.as_bytes().get(at).copied(),
            &|_| None,
            &mut fuel,
            &mut || false
        ),
        VimRegexProgress::Complete(Some(_))
    ));
    for source in [r"[\n]", r"[^x\n]", r"a\_s\+b", r"\%(a\n\)\@<=b"] {
        assert!(
            VimPattern::compile(source, false, VimRegexLimits::default())
                .unwrap()
                .multiline,
            "{source}"
        );
    }
    for source in [r"[[:space:]]", r"a\@=a", r"\(a\)\1"] {
        assert!(
            !VimPattern::compile(source, false, VimRegexLimits::default())
                .unwrap()
                .multiline,
            "{source}"
        );
    }
    assert_eq!(find(r"[[:space:]]", "\n", 1), None);
    assert_eq!(find(r"[^x\n]", "\n", 1), Some(0..1));
}

#[test]
fn advanced_regex_cancellation_and_workspace_exhaustion_are_explicit() {
    let pattern = VimPattern::compile(r"\%(a*\)\@>b", false, VimRegexLimits::default()).unwrap();
    let mut continuation = pattern.start(0);
    let mut fuel = 100;
    assert_eq!(
        pattern.resume_reader(
            &mut continuation,
            3,
            |at| b"aab".get(at).copied(),
            &|_| None,
            &mut fuel,
            &mut || true
        ),
        VimRegexProgress::Pending
    );
    assert_eq!(fuel, 100);
    let mut fuel = 1000;
    assert!(matches!(
        pattern.resume_reader(
            &mut continuation,
            3,
            |at| b"aab".get(at).copied(),
            &|_| None,
            &mut fuel,
            &mut || false
        ),
        VimRegexProgress::Complete(Some(_))
    ));

    let text = "a".repeat(5000);
    let mut continuation = pattern.start(0);
    let mut fuel = 1_000_000;
    assert!(
        matches!(pattern.resume_reader(&mut continuation, text.len(), |at| text.as_bytes().get(at).copied(), &|_| None, &mut fuel, &mut || false), VimRegexProgress::Failed(error) if error.contains("workspace budget"))
    );
}

#[test]
fn vim_regex_matches_installed_vim_oracle() {
    use std::process::Command;
    let executable = std::env::var_os("VIEM_VIM_REGEX_ORACLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/MacOS/Vim".into()
        });
    if !executable.is_file() {
        return;
    }
    let mut cases = vec![
        (r"\<se\%[t]\>", "se number set"),
        (r"\a\@1<=!", "set!"),
        (r"\\\@1<!|", "x\\|y|"),
        (r"\<new\>(\@!", "new() new"),
        (r"\%(\<as\s\+\)\@<=\h\w*\>", "import as name"),
        (r"\%(a*\)\@>a", "aaa"),
        (r"\(a\|ab\)\1$", "abab"),
        (r"\(foo\)\zs\1", "foofoo"),
        (r"a\&ab", "ab"),
        (r"[^][:space:]]\+", "a[b c"),
        (r"[gjf]\{,3\}", "gjfgg"),
        (r"\u\l\+", "Hello"),
        (r"\c\l\+", "ABC"),
        (r"\c\u\+", "abc"),
        (r"\%(\(a*\)\)\@<=b\1", "aaabaaa"),
        (r"\(a\|ab\)\@=\1$", "ab"),
        (r"\(a\)\=\1b", "b"),
        (r"\c\(Σ\)\1", "Σς"),
        (r"\c\(K\)\1", "Kk"),
        (r"\%(é\)\@1<=!", "é!"),
        (r"\%(é\)\@2<=!", "é!"),
        (r"\%#=0\<\%(true\|false\)\>[?!]\@!", "true? false"),
        (r"\%#=1\<\%(true\|false\)\>[?!]\@!", "true? false"),
        (r"\%#=2\<\%(true\|false\)\>[?!]\@!", "true? false"),
        (r"\V${", "${target}"),
        (r"\V\[-+/*=^&?|!><%~]", "x+y"),
        (r#"\M**\|*'\|*""#, "a**b"),
        (r"\M\[0-9]\+", "a123b"),
        (r"\M\[0-9A-F\]\+", "[0-9A-F]"),
        (r"[abc", "[abc"),
        (r"[]", "[]"),
        (r"^*", "*"),
        (r"^*", "abc"),
        (r"\M^a\.\*$", "abc"),
        (r"\V\^a\.\*\$", "abc"),
        (r"\V^a$", "a ^a$ b"),
        (r"\Va.*\mb.*", "a.*bcd"),
        (r"\V\(ab\)\+\1", "ababab"),
        (r"\%d65\%o101\%x41\%u0041\%U00000041", "AAAAA"),
        (r"\%u03a3\%U0001f600", "Σ😀"),
        (r"[\x00-\x7f]\+", "abcé"),
        (r"\%d0", "a\nb"),
        (r"\%d10", "a\nb"),
        (r"\%x00", "a\nb"),
        (r"\%x0a", "a\nb"),
        (r"[\d0]", "a\nb"),
        (r"[\d10]", "a\nb"),
        (r"[\x00-\x09]", "a\nb"),
        (r"[\x01-\x0a]", "a\nb"),
        (r"[\x09-\x0b]", "a\nb"),
        (r"a[^\x00]b", "a\nb"),
        (r"a[^\x0a]b", "a\nb"),
        (r"a[^\x0a]b", "axb"),
        (r"a\%[\%d0b]", "a\nb"),
        (r"\n", "a\nb"),
        (r"\0", "a\nb0"),
        (r"[\u2010-]", "‐"),
        (r"[\d65\o101\x41]", "A"),
        (r"[\s]", " s"),
        (r"[\s]", "\\"),
        (r"[\+]", "+"),
        (r"[\+]", "\\"),
        (r"[[:ident:]]\+", "café"),
        (r"[[:ident:]]", "Σ"),
        (r"[[:lower:]]\+", "café"),
        (r"\F\f*", "12/usr/é.txt"),
        (r"[[:fname:]]\+", "/a/b.txt "),
        (r"\%\(^\|\s\)#", "x #comment"),
        (r"\<r\%[[eo]ad]\>", "road"),
        (r"\<r\%[[eo]ad]\>", "rod"),
        (r"index\%[[[]0[]]]", "index[0]"),
        (r"x\%[\d\x]", "x2a"),
        (r"[[.é.]]", "é"),
        (r"[| \t([.,=\]]", "["),
        (r"[^[:space]]", "x]"),
        (r"[[:unknown:]]", "u]"),
        (r"\_\s\{-}>", "  >"),
        (r"\p\+", "é 😀\u{200b}"),
        (r"\P\+", "12aé"),
        (r"[[:print:]]\+", " café"),
        (
            r"\<\(linear-\|radial-\|conic-\)\=\gradient\s*(",
            "linear-gradient(",
        ),
    ];
    let j_runtime = bundled_syntax_source("j.vim");
    if let Some(pattern) = j_runtime.lines().find_map(|line| {
        line.split_once("jNumber /")
            .and_then(|(_, source)| source.strip_suffix('/'))
    }) {
        for text in ["_3", "3r4", "2j3", "2ad90", "16bff", "1e_3", ".25"] {
            cases.push((pattern, text));
        }
    }
    let directory = std::env::temp_dir().join(format!(
        "viem-vim-regex-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("result.json");
    let script = directory.join("oracle.vim");
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let mut source = String::from("set encoding=utf-8\nlet results = []\n");
    for (pattern, text) in &cases {
        source.push_str(&format!(
            "call add(results, matchstrpos({}, {})[1:2])\n",
            serde_json::to_string(text).unwrap(),
            quote(pattern)
        ));
    }
    source.push_str(&format!(
        "call writefile([json_encode(results)], {})\nqa!\n",
        quote(output.to_str().unwrap())
    ));
    std::fs::write(&script, source).unwrap();
    let result = Command::new(executable)
        .args([
            "-u", "NONE", "-U", "NONE", "-i", "NONE", "-n", "-N", "-es", "-S",
        ])
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected: Vec<[isize; 2]> =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    for ((pattern, text), [start, end]) in cases.into_iter().zip(expected) {
        let expected = (start >= 0).then_some(start as usize..end as usize);
        let compiled = VimPattern::compile(pattern, false, VimRegexLimits::default()).unwrap();
        let mut fuel = 1_000_000;
        assert_eq!(
            compiled
                .find_text_with_control(text, 0, &mut fuel, &mut || false)
                .unwrap()
                .map(|m| m.start..m.end),
            expected,
            "Vim string differential: {pattern} in {text}"
        );
        if text.contains('\n') {
            // Source hard-line semantics are covered by the buffer oracle;
            // this oracle's string LF is Vim's embedded NUL representation.
            continue;
        }
        assert_eq!(
            find(pattern, text, 3),
            expected,
            "Vim differential: {pattern} in {text}"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn gitcommit_trailer_pattern_keeps_multiline_lookbehind() {
    let pattern = r"\n\@<=\n\%([[:alnum:]-]\+\s*:.*\|(cherry picked from commit .*\)\%(\n\s.*\|\n[[:alnum:]-]\+\s*:.*\|\n(cherry picked from commit .*\)*\%(\n\n*\%(#\)\|\n*\%$\)\@=";
    let text = "Summary\n\nBody\n\nSigned-off-by: Writer <writer@example.test>\n\n# Please enter a message.\n";
    let begin = text.find("\nSigned").unwrap();
    let end = text.find("\n\n#").unwrap();
    assert_eq!(find(pattern, text, 17), Some(begin..end));
}

#[test]
fn newline_star_repeats_the_atom_before_a_line_start_anchor() {
    for pattern in [r"a\n*b", r"\ma\n*b", r"\Ma\n\*b", r"\va\n*b"] {
        assert_eq!(find(pattern, "ab", 3), Some(0..2), "{pattern}");
        assert_eq!(find(pattern, "a\n\nb", 3), Some(0..4), "{pattern}");
    }
    assert_eq!(find(r"a\n*^b", "a\n\nb", 3), Some(0..4));
    assert_eq!(find(r"a\n*^b", "ab", 3), None);
}
