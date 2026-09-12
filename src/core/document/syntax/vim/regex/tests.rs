use super::*;

fn find(pattern: &str, text: &str, slice: usize) -> Option<std::ops::Range<usize>> {
    let pattern = VimPattern::compile(pattern, false, VimRegexLimits::default()).unwrap();
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
            &mut fuel,
            &mut || false
        ),
        VimRegexProgress::Complete(Some(_))
    ));

    let text = "a".repeat(5000);
    let mut continuation = pattern.start(0);
    let mut fuel = 1_000_000;
    assert!(
        matches!(pattern.resume_reader(&mut continuation, text.len(), |at| text.as_bytes().get(at).copied(), &mut fuel, &mut || false), VimRegexProgress::Failed(error) if error.contains("workspace budget"))
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
    let cases = [
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
    ];
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
    for (pattern, text) in cases {
        source.push_str(&format!(
            "call add(results, matchstrpos({}, {})[1:2])\n",
            quote(text),
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
        assert_eq!(
            find(pattern, text, 3),
            expected,
            "Vim differential: {pattern} in {text}"
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
