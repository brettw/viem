use evim_core::{
    command::regex_v1::*,
    document::{Document, Encoding, FileFormat, Format},
};
fn doc(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap()
}
fn matches(document: &Document, pattern: &str) -> Vec<RegexMatch> {
    let snapshot = document.hard_line_snapshot();
    CompiledRegex::compile(pattern, false, RegexLimits::default())
        .unwrap()
        .find_all(
            &RegexInput::new(&snapshot),
            0..document.text().len(),
            &mut RegexWork::new(RegexLimits::default()),
        )
        .unwrap()
}
#[test]
fn thompson_vm_matches_unicode_reference_priority_and_captures() {
    let texts = [
        "",
        "ababa",
        "aaabbb\naba",
        "Café κόσμος 漢字",
        "a\u{301} b",
        "foo123bar",
        "ababac",
    ];
    let patterns = [
        "",
        "a",
        "a*",
        "a+?",
        "a|ab",
        "ab|a",
        "(a*)(b*)",
        "(?:ab)*",
        "(a(b)?)+",
        ".*",
        ".*?",
        r"\b\w+\b",
        r"[\p{L}&&[^a]]+",
        "(?i:CAFÉ)",
        "(?s:.)*",
        "(?x:a # hi\n b)",
        "a{1,3}",
        "(?P<word>[^0-9]+)([0-9]*)",
    ];
    for text in texts {
        for pattern in patterns {
            let document = doc(text);
            let ours = matches(&document, pattern);
            let reference = regex::RegexBuilder::new(pattern)
                .multi_line(true)
                .build()
                .unwrap();
            let reference: Vec<_> = reference.captures_iter(text).collect();
            // Our mandated empty iteration skips unavailable grapheme interiors.
            let reference: Vec<_> = reference
                .into_iter()
                .filter(|m| {
                    !m.get(0).unwrap().is_empty()
                        || document
                            .hard_line_snapshot()
                            .is_grapheme_boundary(m.get(0).unwrap().start())
                })
                .collect();
            assert_eq!(ours.len(), reference.len(), "{pattern:?} in {text:?}");
            for (ours, reference) in ours.iter().zip(reference) {
                for index in 0..reference.len() {
                    assert_eq!(
                        ours.capture(index),
                        reference.get(index).map(|m| m.range()),
                        "{pattern:?} in {text:?} capture {index}"
                    );
                }
            }
        }
    }
}
#[test]
fn semantic_anchors_are_distinct_from_literal_lf_and_document_edges() {
    let document = Document::from_bytes_with_file_format(
        b"one\ntwo\rthree\nfour".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    assert_eq!(matches(&document, r"^.*$"), Vec::new());
    assert_eq!(matches(&document, r"(?s:^.*$)")[0].range(), 0..18);
    assert_eq!(matches(&document, r"two$\n^three")[0].range(), 4..13);
    assert!(matches(&document, r"^two").is_empty());
    assert!(matches(&document, r"one$").is_empty());
    assert_eq!(matches(&document, r"\Aone")[0].range(), 0..3);
    assert_eq!(matches(&document, r"four\z")[0].range(), 14..18);
}
#[test]
fn validator_distinguishes_escaped_literals_flags_classes_and_vim_families() {
    for pattern in [
        r"\vfoo",
        r"a\@=",
        r"\_s",
        r"\%23l",
        r"\1",
        r"\Z",
        r"(?m)a",
        r"(?-u:a)",
        r"(?=a)",
        r"\h",
        r"\b{start}",
        r"[[:alpha:]]",
        r"a\+",
    ] {
        assert!(
            matches!(
                validate_pattern(pattern),
                Err(RegexError::UnsupportedRegexAtom(_))
            ),
            "{pattern}"
        );
    }
    for pattern in [
        r"\\vfoo",
        r"[a-z&&[^x]]",
        r"(?x:a # \v ignored
    )",
        r"(?i)a(?-i:b)",
        r"(?P<Title>\p{L}+)",
        r"\x41\u{41}",
        r"[A-Z~~[X]]",
    ] {
        assert!(
            CompiledRegex::compile(pattern, false, RegexLimits::default()).is_ok(),
            "{pattern}"
        );
    }
    for literal in ["a+b?", "(x)|{y}", r"\v", "[a]#.$"] {
        assert_eq!(
            matches(&doc(literal), &escape_literal(literal))[0].range(),
            0..literal.len()
        );
    }
    assert!(!has_smartcase_uppercase(r"(?P<NAME>[A-Z]\p{Lu}\x41)").unwrap());
    assert!(has_smartcase_uppercase("éÉ").unwrap());
}
#[test]
fn budgets_cancellation_staleness_and_grapheme_errors_are_structured() {
    let document = doc("a\u{301}");
    let snapshot = document.hard_line_snapshot();
    let matched = &matches(&document, "a")[0];
    assert!(matches!(
        matched.validate_edit(&snapshot),
        Err(RegexError::RegexMatchSplitsGraphemeCluster(_))
    ));
    assert!(matches!(
        matched.validate(&doc("a").hard_line_snapshot()),
        Err(RegexError::StaleProjection)
    ));
    assert!(matches!(
        CompiledRegex::compile(
            "abc",
            false,
            RegexLimits {
                pattern_bytes: 2,
                ..Default::default()
            }
        ),
        Err(RegexError::RegexResourceLimit(_))
    ));
    let regex = CompiledRegex::compile("a", false, RegexLimits::default()).unwrap();
    let mut work = RegexWork::new(RegexLimits {
        work: 0,
        ..Default::default()
    });
    assert!(matches!(
        regex.find(&RegexInput::new(&snapshot), 0, 3, &mut work),
        Err(RegexError::RegexResourceLimit(_))
    ));
    let mut work = RegexWork::new(RegexLimits::default());
    work.cancellation_handle()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        regex.find(&RegexInput::new(&snapshot), 0, 3, &mut work),
        Err(RegexError::Cancelled)
    ));
}
#[test]
fn replacement_lexer_rejects_trailing_backslash_and_retains_literal_atoms() {
    let regex = CompiledRegex::compile("(?P<x>a)(b)?", false, RegexLimits::default()).unwrap();
    assert!(matches!(
        ReplacementTemplate::compile("\\", &regex),
        Err(RegexError::UnsupportedReplacementAtom(_))
    ));
    let d = doc("a");
    let snapshot = d.hard_line_snapshot();
    let input = RegexInput::new(&snapshot);
    let matched = regex
        .find(&input, 0, 1, &mut RegexWork::new(RegexLimits::default()))
        .unwrap()
        .unwrap();
    let parts = ReplacementTemplate::compile(r"\g{x}\2\t\n\r\\\&", &regex)
        .unwrap()
        .expand(&matched, &input)
        .unwrap();
    assert!(matches!(&parts[0], ExpandedFragment::Capture(range) if *range == (0..1)));
    assert!(parts.iter().any(|part| matches!(part, ExpandedFragment::Literal{text,break_offsets} if text == "\n" && break_offsets == &[0])));
}
#[test]
fn every_regex_construct_and_vim_rejection_family_has_a_fixture() {
    for (pattern, text, expected) in [
        (r"\d+", "a٣٤b", "٣٤"),
        (r"\D+", "12漢字3", "漢字"),
        (r"\s+", "a\u{2003} b", "\u{2003} "),
        (r"\S+", " 漢字 ", "漢字"),
        (r"\w+", "é٣漢!", "é٣漢"),
        (r"\W+", "a!?b", "!?"),
        (r"\p{Greek}+", "aκόσμος", "κόσμος"),
        (r"\P{Greek}+", "αwordβ", "word"),
        (r"[a-z--[aeiou]]+", "aeibcdfou", "bcdf"),
        (r"[a-c~~[b-d]]+", "bbacdd", "a"),
        (r"a\Bb", "ab", "ab"),
        (r"\t\r\x41\u{1F49A}", "\t\rA💚", "\t\rA💚"),
        (r"a{2,3}?", "aaaa", "aa"),
        (r"(?:ab)?c", "abc", "abc"),
        (r"~", "~", "~"),
        (r"(?i:σ)", "ς", "ς"),
        (r"(?i)a(?-i:B)", "AB", "AB"),
    ] {
        let d = doc(text);
        let found = matches(&d, pattern);
        assert_eq!(&d.text()[found[0].range()], expected, "{pattern}");
    }
    for pattern in [
        r"\m", r"\M", r"\V", r"\c", r"\C", r"\(", r"\)", r"\=", r"\?", r"\|", r"\{2}", r"\}",
        r"\zs", r"\ze", r"\z1", r"\z(", r"\<", r"\>", r"\@123<=", r"\_d", r"\%[abc]", r"\%C",
        r"\%#=1", r"\i", r"\I", r"\k", r"\K", r"\f", r"\F", r"\p", r"\P", r"\x", r"\X", r"\o",
        r"\O", r"\h", r"\H", r"\a", r"\l", r"\L", r"\u", r"\U", r"[[=a=]]", r"[[.a.]]", r"(?<=a)",
        r"(?!a)",
    ] {
        assert!(
            matches!(
                validate_pattern(pattern),
                Err(RegexError::UnsupportedRegexAtom(_))
            ),
            "{pattern}"
        );
    }
}
#[test]
fn verbose_class_comments_and_nonascii_flags_are_lexed_without_panics() {
    assert!(has_smartcase_uppercase("[^^]A").unwrap());
    assert!(!has_smartcase_uppercase("[^^A]").unwrap());
    for pattern in ["(?x)[a# \\v ignored\nb]", "[]A]", "[^]A]"] {
        assert!(
            CompiledRegex::compile(pattern, false, RegexLimits::default()).is_ok(),
            "{pattern}"
        );
        assert!(!has_smartcase_uppercase(pattern).unwrap());
    }
    assert!(matches!(
        CompiledRegex::compile("(?é)a", false, RegexLimits::default()),
        Err(RegexError::UnsupportedRegexAtom(_))
    ));
}
