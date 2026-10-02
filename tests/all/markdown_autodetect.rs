use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{FontSlant, SemanticInlineStyle, StyleApplication};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn fixture(source: &str) -> (Document, CommandInterpreter) {
    (
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
        CommandInterpreter::new(),
    )
}
fn key(d: &mut Document, c: &mut CommandInterpreter, key: Key) {
    let output = c.handle(d, InputEvent::Key(key)).unwrap_or_else(|e| {
        panic!(
            "{key:?} on {:?}: {e:?}",
            String::from_utf8_lossy(&d.source_bytes())
        )
    });
    assert!(
        matches!(
            output.status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "{key:?}: {:?}",
        output.status
    );
}
fn type_keys(d: &mut Document, c: &mut CommandInterpreter, text: &str) {
    for ch in text.chars() {
        key(d, c, Key::Char(ch));
    }
}
fn reopened(d: &Document) -> Document {
    Document::from_bytes(d.source_bytes(), d.encoding(), Format::Markdown).unwrap()
}

#[test]
fn inline_completion_waits_then_formats_and_exits() {
    for (markers, bold, italic, code, strike) in [
        ("*", false, true, false, false),
        ("**", true, false, false, false),
        ("***", true, true, false, false),
        ("_", false, true, false, false),
        ("__", true, false, false, false),
        ("___", true, true, false, false),
        ("`", false, false, true, false),
        ("~~", false, false, false, true),
    ] {
        let (mut d, mut c) = fixture("");
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, &format!("{markers}word"));
        assert_eq!(d.text(), format!("{markers}word"));
        type_keys(&mut d, &mut c, markers);
        assert_eq!(
            d.text(),
            "word",
            "{markers}: {:?}",
            String::from_utf8_lossy(&d.source_bytes())
        );
        assert_eq!(c.cursor(), 4);
        let style = DocumentLayoutStyles::semantic_character_at(d.projection(), 1, false).unwrap();
        assert_eq!(style.bold, bold);
        assert_eq!(style.slant == FontSlant::Italic, italic);
        assert_eq!(
            d.projection()
                .style_spans()
                .iter()
                .any(|s| s.application == StyleApplication::Semantic(SemanticInlineStyle::Code)),
            code
        );
        assert_eq!(
            d.projection()
                .style_spans()
                .iter()
                .any(|s| s.application == StyleApplication::Automatic("Strikethrough".into())),
            strike
        );
        assert_eq!(reopened(&d).text(), "word");
        type_keys(&mut d, &mut c, " tail");
        assert_eq!(d.text(), "word tail");
        let tail = DocumentLayoutStyles::semantic_character_at(d.projection(), 6, false).unwrap();
        assert!(
            !tail.bold && tail.slant != FontSlant::Italic,
            "{markers}: tail inherited formatting"
        );
        key(&mut d, &mut c, Key::Escape);
        let final_bytes = d.source_bytes();
        key(&mut d, &mut c, Key::Char('u'));
        assert_eq!(d.source_bytes(), b"");
        key(&mut d, &mut c, Key::Ctrl('r'));
        assert_eq!(d.source_bytes(), final_bytes);
    }
}

#[test]
fn block_prefixes_wait_for_space_and_fences_trigger_immediately() {
    for (prefix, style) in [
        ("# ", "Heading1"),
        ("###### ", "Heading6"),
        ("- ", "BulletedList1"),
        ("* ", "BulletedList1"),
        ("+ ", "BulletedList1"),
        ("12. ", "NumberedList1"),
        ("3) ", "NumberedList1"),
        ("> ", "Paragraph"),
        ("```", "Code Block"),
        ("~~~", "Code Block"),
    ] {
        let (mut d, mut c) = fixture("");
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, &prefix[..prefix.len() - 1]);
        assert_eq!(d.text(), &prefix[..prefix.len() - 1]);
        type_keys(&mut d, &mut c, &prefix[prefix.len() - 1..]);
        assert_eq!(
            d.text(),
            "",
            "{prefix:?}: {:?}",
            String::from_utf8_lossy(&d.source_bytes())
        );
        assert_eq!(c.cursor(), 0);
        assert_eq!(d.projection().blocks()[0].style.0, style, "{prefix}");
        type_keys(&mut d, &mut c, "body");
        assert_eq!(reopened(&d).text(), "body");
        key(&mut d, &mut c, Key::Escape);
        key(&mut d, &mut c, Key::Char('u'));
        assert_eq!(d.source_bytes(), b"");
    }
}

#[test]
fn literal_next_survives_more_typing_reopen_and_repeat() {
    for (quoted, rest) in [
        ('*', "word*"),
        ('`', "word`"),
        ('#', " title"),
        ('-', " item"),
        ('1', ". item"),
        ('>', " quote"),
    ] {
        let (mut d, mut c) = fixture("");
        key(&mut d, &mut c, Key::Char('i'));
        key(&mut d, &mut c, Key::Ctrl('q'));
        if quoted == '1' {
            type_keys(&mut d, &mut c, "u0031");
        } else {
            key(&mut d, &mut c, Key::Char(quoted));
        }
        type_keys(&mut d, &mut c, rest);
        assert_eq!(d.text(), format!("{quoted}{rest}"));
        assert_eq!(reopened(&d).text(), d.text());
        assert!(String::from_utf8_lossy(&d.source_bytes()).contains("&#x0000"));
        key(&mut d, &mut c, Key::Escape);
        let bytes = d.source_bytes();
        key(&mut d, &mut c, Key::Char('u'));
        key(&mut d, &mut c, Key::Char('.'));
        assert_eq!(d.source_bytes(), bytes);
    }
}

#[test]
fn preference_off_and_other_formats_keep_literal_input() {
    for format in [
        Format::Markdown,
        Format::MarkdownSource,
        Format::PlainText,
        Format::Code,
    ] {
        let mut d = Document::from_bytes(vec![], Encoding::Utf8, format).unwrap();
        let mut c = CommandInterpreter::new();
        if format == Format::Markdown {
            c.set_markdown_autodetect(false);
        }
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, "**word**");
        assert_eq!(d.text(), "**word**");
    }
}

#[test]
fn text_batches_links_and_core_layout_entry_share_detection() {
    for input in ["**word**", "[word](https://example.com)"] {
        let (d, _) = fixture("");
        let mut core = Core::new(d);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
            .unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Text(input.into())))
            .unwrap();
        assert_eq!(core.document().text(), "word", "{input}");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    }
}

#[test]
fn code_delimiters_take_precedence_and_preserve_straight_quotes() {
    for batched in [false, true] {
        let (mut d, mut c) = fixture("");
        c.set_smart_quotes(true);
        key(&mut d, &mut c, Key::Char('i'));
        let input = "`**word** \"quoted\"`";
        if batched {
            c.handle(&mut d, InputEvent::text(input)).unwrap();
        } else {
            type_keys(&mut d, &mut c, input);
        }
        assert_eq!(d.text(), "**word** \"quoted\"");
        assert_eq!(reopened(&d).text(), d.text());
        assert!(d
            .projection()
            .style_spans()
            .iter()
            .any(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)));
    }
}

#[test]
fn inline_detection_in_list_continuations_preserves_ownership() {
    for source in [
        "- first\n\n  continued\n- tail",
        "> - first\n>\n>   continued\n> - tail",
    ] {
        let (mut d, mut c) = fixture(source);
        key(&mut d, &mut c, Key::Char('i'));
        let at = d.text().find("continued").unwrap();
        assert!(c.set_cursor(&d, at));
        type_keys(&mut d, &mut c, "*word*");
        assert!(
            d.text().contains("wordcontinued"),
            "text: {:?}, source: {:?}",
            d.text(),
            String::from_utf8_lossy(&d.source_bytes())
        );
        assert_eq!(reopened(&d).text(), d.text());
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(d.projection(), at + 1, false)
                .unwrap()
                .slant,
            FontSlant::Italic
        );
    }
}

#[test]
fn replace_backspace_restores_completion_and_exact_source() {
    let (mut d, mut c) = fixture("abcdefghij");
    key(&mut d, &mut c, Key::Char('R'));
    type_keys(&mut d, &mut c, "*word");
    let before = d.source_bytes();
    key(&mut d, &mut c, Key::Char('*'));
    assert_eq!(d.text(), "wordghij");
    key(&mut d, &mut c, Key::Backspace);
    assert_eq!(d.source_bytes(), before);
    assert_eq!(c.cursor(), 5);
    key(&mut d, &mut c, Key::Escape);
    key(&mut d, &mut c, Key::Char('u'));
    assert_eq!(d.source_bytes(), b"abcdefghij");
}

#[test]
fn counted_insert_dot_macros_and_local_work_preserve_semantics() {
    for marker in ["*", "**", "`", "~~"] {
        let (mut d, mut c) = fixture("");
        type_keys(&mut d, &mut c, &format!("2i{marker}word{marker}"));
        key(&mut d, &mut c, Key::Escape);
        assert_eq!(d.text(), "wordword", "{marker}");
        let bytes = d.source_bytes();
        key(&mut d, &mut c, Key::Char('u'));
        key(&mut d, &mut c, Key::Char('.'));
        assert_eq!(d.source_bytes(), bytes, "{marker}");
    }
    let (mut d, mut c) = fixture("");
    type_keys(&mut d, &mut c, "qai**word**");
    key(&mut d, &mut c, Key::Escape);
    type_keys(&mut d, &mut c, "qu@a");
    assert_eq!(d.text(), "word");

    for count in [100, 10_000] {
        let source = format!("front\n\n{}", "unrelated paragraph\n\n".repeat(count));
        let (mut d, mut c) = fixture(&source);
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, "*word");
        let (_, work) =
            viem_core::document::measure_document_work(|| key(&mut d, &mut c, Key::Char('*')));
        assert_eq!(work.source_full_materializations, 0, "{count}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{count}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{count}: {work:?}");
        assert!(d.source_bytes().ends_with(&source.as_bytes()[5..]));

        let source = format!("\n\n{}", "unrelated paragraph\n\n".repeat(count));
        let (mut d, mut c) = fixture(&source);
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, "``");
        let (_, work) =
            viem_core::document::measure_document_work(|| key(&mut d, &mut c, Key::Char('`')));
        assert_eq!(
            work.source_full_materializations, 0,
            "fence {count}: {work:?}"
        );
        assert_eq!(
            work.full_projection_candidates, 0,
            "fence {count}: {work:?}"
        );
        assert!(work.source_decoded_bytes < 4096, "fence {count}: {work:?}");
        let (_, work) =
            viem_core::document::measure_document_work(|| key(&mut d, &mut c, Key::Char('x')));
        assert_eq!(
            work.source_full_materializations,
            0,
            "body {count}: {work:?}, source {:?}, provenance {:?}",
            &d.source_bytes()[..12],
            d.projection()
                .provenance()
                .iter()
                .take(4)
                .collect::<Vec<_>>()
        );
        assert_eq!(work.full_projection_candidates, 0, "body {count}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "body {count}: {work:?}");
        assert!(d.source_bytes().ends_with(source.as_bytes()));
    }
}

#[test]
fn encoding_line_endings_quoted_closers_and_invalid_patterns_remain_exact() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Latin1] {
        let source = match encoding {
            Encoding::Utf16Le => "before\r\n\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
            _ => b"before\r\n\r\n".to_vec(),
        };
        let mut d = Document::from_bytes(source.clone(), encoding, Format::Markdown).unwrap();
        let mut c = CommandInterpreter::new();
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, "**café**");
        assert_eq!(d.projection().text_tree().slice(0..5).unwrap(), "café");
        assert!(d.source_bytes().ends_with(&source));
        key(&mut d, &mut c, Key::Escape);
        key(&mut d, &mut c, Key::Char('u'));
        assert_eq!(d.source_bytes(), source);
    }
    for input in [
        "snake_case_",
        "####### title",
        "**word*",
        "`unfinished",
        "a* word*",
    ] {
        let (mut d, mut c) = fixture("");
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, input);
        assert_eq!(d.text(), input);
    }
    let (mut d, mut c) = fixture("");
    type_keys(&mut d, &mut c, "i*word");
    key(&mut d, &mut c, Key::Ctrl('q'));
    key(&mut d, &mut c, Key::Char('*'));
    assert_eq!(d.text(), "*word*");
    assert_eq!(reopened(&d).text(), "*word*");
}

#[test]
fn fenced_blocks_and_rules_preserve_paragraph_owners_and_nearby_source() {
    for source in [
        "",
        "> ",
        "- ",
        "-\t",
        "12.\t",
        "> - ",
        "> -\t",
        "# heading",
        "ordinary body",
    ] {
        let (mut d, mut c) = fixture(source);
        let prior = d.source_bytes();
        key(&mut d, &mut c, Key::Char('A'));
        type_keys(&mut d, &mut c, "```");
        assert_eq!(
            c.cursor(),
            0,
            "{source}: {:?}",
            String::from_utf8_lossy(&d.source_bytes())
        );
        assert_eq!(d.projection().blocks()[0].style.0, "Code Block", "{source}");
        assert_eq!(reopened(&d).text(), d.text());
        type_keys(&mut d, &mut c, "new ");
        assert!(d.text().starts_with("new "), "{source}");
        key(&mut d, &mut c, Key::Escape);
        key(&mut d, &mut c, Key::Char('u'));
        assert_eq!(d.source_bytes(), prior);
    }
    for marker in ["---", "___ ", "*** "] {
        let source = "\n\nuntouched\n\n";
        let (mut d, mut c) = fixture(source);
        key(&mut d, &mut c, Key::Char('i'));
        type_keys(&mut d, &mut c, marker);
        assert_eq!(c.cursor(), 0);
        assert!(d.projection().blocks()[0].thematic_break, "{marker}");
        assert!(d.source_bytes().ends_with(source.as_bytes()));
        assert_eq!(reopened(&d).text(), d.text());
    }
}

#[test]
fn marked_input_waits_for_commit_and_cancellation_is_source_preserving() {
    let (d, _) = fixture("");
    let mut core = Core::new(d);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target.clone())),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
            "**word**",
            8..8,
        ))),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"");
    core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"");
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
            "**word**",
            8..8,
        ))),
    )
    .unwrap();
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    assert_eq!(core.document().text(), "word");
    core.handle(view, CoreEvent::Input(InputEvent::text(" tail")))
        .unwrap();
    assert_eq!(core.document().text(), "word tail");
    assert!(
        !DocumentLayoutStyles::semantic_character_at(core.document().projection(), 6, false)
            .unwrap()
            .bold
    );
}
