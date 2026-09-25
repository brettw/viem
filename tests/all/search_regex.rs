use viem_core::{
    command::search_regex::*,
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
        r"\b{end}",
        r"\b{start-half}",
        r"\b{end-half}",
        r"\B{start}",
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
        r"\<",
        r"\>",
        r"\<\w+\>",
        r"(?i:\<foo\>)",
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
        r"\zs", r"\ze", r"\z1", r"\z(", r"\@123<=", r"\_d", r"\%[abc]", r"\%C", r"\%#=1", r"\i",
        r"\I", r"\k", r"\K", r"\f", r"\F", r"\p", r"\P", r"\x", r"\X", r"\o", r"\O", r"\h", r"\H",
        r"\a", r"\l", r"\L", r"\u", r"\U", r"[[=a=]]", r"[[.a.]]", r"(?<=a)", r"(?!a)",
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

#[test]
fn directional_word_assertions_distinguish_starts_and_ends_from_both_sides() {
    let document = doc("foo,bar _baz!");
    for (pattern, expected) in [
        (r"\<", vec![0..0, 4..4, 8..8]),
        (r"\>", vec![3..3, 7..7, 12..12]),
        (r"\b", vec![0..0, 3..3, 4..4, 7..7, 8..8, 12..12]),
        (r"\<\w+\>", vec![0..3, 4..7, 8..12]),
        (r"\>,\<", vec![3..4]),
    ] {
        assert_eq!(
            matches(&document, pattern)
                .iter()
                .map(RegexMatch::range)
                .collect::<Vec<_>>(),
            expected,
            "{pattern}"
        );
    }
    for pattern in [r"foo\<", r"\>foo", r"\<oo", r"ba\>"] {
        assert!(matches(&document, pattern).is_empty(), "{pattern}");
    }
    assert_eq!(matches(&document, r"foo\b")[0].range(), 0..3);
    assert_eq!(matches(&document, r"\bfoo")[0].range(), 0..3);
    assert_eq!(matches(&doc("__one_two__"), r"\<\w+\>")[0].range(), 0..11);
    assert_eq!(matches(&doc("foo-bar"), r"\<foo-bar\>")[0].range(), 0..7);
    assert_eq!(matches(&doc("foo-bar"), r"\<foo\>")[0].range(), 0..3);
    assert!(matches(&doc("foo中文bar"), r"\<foo\>").is_empty());
    assert!(matches(&doc("foo中文bar"), r"\<中文\>").is_empty());
    for text in ["", "!", " - . ", "²½"] {
        for pattern in [r"\<", r"\>", r"\b"] {
            assert!(
                matches(&doc(text), pattern).is_empty(),
                "{pattern} in {text:?}"
            );
        }
    }
}

#[test]
fn directional_word_assertions_use_the_complete_unicode_word_definition() {
    // Alphabetic is broader than Letter, while Numeric is broader than Nd.
    // Marks, connector punctuation and join controls are independent members.
    for (text, word) in [
        ("A", true),
        ("é", true),
        ("λ", true),
        ("Ж", true),
        ("漢", true),
        ("अ", true),
        ("ا", true),
        ("א", true),
        ("ก", true),
        ("Ⅷ", true),
        ("²", false),
        ("½", false),
        ("٣", true),
        ("９", true),
        ("\u{301}", true),
        ("\u{93f}", true),
        ("\u{20dd}", true),
        ("_", true),
        ("\u{203f}", true),
        ("\u{200c}", true),
        ("\u{200d}", true),
        ("\u{200b}", false),
        ("!", false),
        ("😀", false),
    ] {
        let document = doc(text);
        for (pattern, expected) in [
            (r"\<", 0..0),
            (r"\>", text.len()..text.len()),
            (r"\<\w+\>", 0..text.len()),
        ] {
            let found = matches(&document, pattern);
            let expected = if word { vec![expected] } else { Vec::new() };
            assert_eq!(
                found.iter().map(RegexMatch::range).collect::<Vec<_>>(),
                expected,
                "{pattern} in {text:?}"
            );
        }
    }
    let document = doc("e\u{301}!");
    assert_eq!(matches(&document, r"\<\w+\>")[0].range(), 0..3);
    assert!(matches(&document, r"e\>").is_empty());
    assert!(matches(&document, "\\<\u{301}").is_empty());
}

#[test]
fn directional_assertion_escapes_remain_distinct_from_literal_angle_spellings() {
    for (pattern, text, expected) in [
        (r"\\<", r"\<", 0..2),
        (r"\\>", r"\>", 0..2),
        (r"\\\<word\>", r"\word", 0..5),
        (r"<word>", "<word>", 0..6),
        (r"[<>]+", "<>", 0..2),
        (r"\x3Cword\x3E", "<word>", 0..6),
    ] {
        assert_eq!(
            matches(&doc(text), pattern)[0].range(),
            expected,
            "{pattern}"
        );
    }
    for literal in [r"\<", r"\>", r"\<word\>", "<word>"] {
        assert_eq!(
            matches(&doc(literal), &escape_literal(literal))[0].range(),
            0..literal.len(),
            "{literal}"
        );
    }
    for pattern in [r"[\<]", r"[\>]", r"[^\<]", r"[a\>z]", r"[a&&[\>]]"] {
        assert!(
            matches!(
                CompiledRegex::compile(pattern, false, RegexLimits::default()),
                Err(RegexError::InvalidRegex(_))
            ),
            "{pattern}"
        );
    }
}

#[test]
fn directional_assertions_preserve_smartcase_and_scoped_case_behavior() {
    let options = SearchOptions {
        ignorecase: true,
        smartcase: true,
        ..Default::default()
    };
    for (pattern, uppercase) in [
        (r"\<foo\>", false),
        (r"\<Foo\>", true),
        (r"\<É\>", true),
        (r"\<[A-Z]+\>", false),
        (r"\<\p{Uppercase}+\>", false),
        (r"\<\x41\>", false),
        (r"\<\u{41}\>", false),
        (r"\<(?P<NAME>foo)\>", false),
        ("(?x:\\< # COMMENT\n foo \\>)", false),
    ] {
        assert_eq!(
            has_smartcase_uppercase(pattern).unwrap(),
            uppercase,
            "{pattern}"
        );
        assert_eq!(
            options.case_insensitive(pattern).unwrap(),
            !uppercase,
            "{pattern}"
        );
    }
    let document = doc("FOO Foo foo");
    let snapshot = document.hard_line_snapshot();
    for (pattern, expected) in [
        (r"\<foo\>", vec![0..3, 4..7, 8..11]),
        (r"\<Foo\>", vec![4..7]),
        (r"(?i:\<Foo\>)", vec![0..3, 4..7, 8..11]),
        (r"(?-i:\<foo\>)", vec![8..11]),
    ] {
        let compiled = CompiledRegex::compile(
            pattern,
            options.case_insensitive(pattern).unwrap(),
            RegexLimits::default(),
        )
        .unwrap();
        let found = compiled
            .find_all(
                &RegexInput::new(&snapshot),
                0..11,
                &mut RegexWork::new(RegexLimits::default()),
            )
            .unwrap();
        assert_eq!(
            found.iter().map(RegexMatch::range).collect::<Vec<_>>(),
            expected,
            "{pattern}"
        );
    }
}

#[test]
fn scalar_word_assertions_do_not_make_interior_grapheme_boundaries_editable() {
    for (text, pattern, at, editable) in [
        ("!\u{301}", r"\<", 1, false),
        ("!\u{301}", r"\>", 3, true),
        ("👩\u{200d}💻", r"\<", 4, false),
        ("👩\u{200d}💻", r"\>", 7, false),
        ("a\u{301}", r"\<", 0, true),
        ("a\u{301}", r"\>", 3, true),
    ] {
        let document = doc(text);
        let snapshot = document.hard_line_snapshot();
        let compiled = CompiledRegex::compile(pattern, false, RegexLimits::default()).unwrap();
        let matched = compiled
            .find(
                &RegexInput::new(&snapshot),
                0,
                text.len(),
                &mut RegexWork::new(RegexLimits::default()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(matched.range(), at..at, "{pattern} in {text:?}");
        assert!(matched.validate(&snapshot).is_ok());
        assert_eq!(
            matched.validate_edit(&snapshot),
            if editable {
                Ok(())
            } else {
                Err(RegexError::RegexMatchSplitsGraphemeCluster(at..at))
            },
            "{pattern} in {text:?}"
        );
    }
}

#[test]
fn compiled_search_patterns_record_the_current_language_version() {
    assert_eq!(DIALECT_VERSION, 2);
    let compiled = CompiledRegex::compile(r"\<word\>", false, RegexLimits::default()).unwrap();
    assert_eq!(compiled.dialect_version(), DIALECT_VERSION);
}
