use evim_core::command::ex::{parse_ex, ExAction};
use evim_core::command::ex_execute::{
    commit_ex, execute_ex, prepare_ex, ExExecutionContext, ExExecutionState, ExOutcome,
};
use evim_core::command::{CommandInterpreter, InputEvent, Key};
use evim_core::document::{Document, Encoding, FileFormat, Format, ModelRequest};

fn run(document: &mut Document, command: &str) -> ExOutcome {
    execute_ex(
        document,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &parse_ex(command).unwrap(),
        &(),
    )
    .unwrap()
}
fn source(text: &str, format: Format) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn restored(mut document: Document, command: &str, expected: &str) {
    let original = document.source_bytes();
    let original_revision = document.revision();
    let outcome = run(&mut document, command);
    assert_eq!(document.text(), expected);
    assert!(outcome.register_effects.is_empty());
    assert_ne!(document.revision(), original_revision);
    let sorted = document.source_bytes();
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(!document.undo(), "sort is one undo unit");
    assert!(document.redo());
    assert_eq!(document.source_bytes(), sorted);
}
#[test]
fn grammar_accepts_abbreviations_and_rejects_invalid_arguments_atomically() {
    for command in [
        ":sort",
        ":sor! iu",
        ":2,4sort n",
        ":%sort /[^,]*,/ ri",
        ":sort #a\\#b# x",
    ] {
        assert!(
            matches!(parse_ex(command).unwrap().action, ExAction::Sort(_)),
            "{command}"
        );
    }
    for command in [
        ":so",
        ":sort f",
        ":sort l",
        ":sort nx",
        ":sort nn",
        ":sort z",
        ":sort /open",
        ":sort /a/ /b/",
        ":sort 2",
        ":sort a",
        ":sort | quit",
        ":sort \"a",
        ":sort! !",
    ] {
        assert!(parse_ex(command).is_err(), "{command}");
    }
}
#[test]
fn default_all_lines_range_reverse_and_history() {
    restored(
        Document::new("pear\napple\nbanana"),
        ":sort",
        "apple\nbanana\npear",
    );
    restored(
        Document::new("outside\nc\na\nb\ntail"),
        ":2,4sor!",
        "outside\nc\nb\na\ntail",
    );
    restored(Document::new("B\n a\n  z"), ":sort", "  z\n a\nB");
}
#[test]
fn final_terminator_and_empty_lines_are_preserved() {
    for (before, after) in [
        ("b\na\n", "a\nb\n"),
        ("b\n\na\n", "\na\nb\n"),
        ("b\na", "a\nb"),
    ] {
        let mut doc = Document::new(before);
        run(&mut doc, ":sort");
        assert_eq!(doc.source_bytes(), after.as_bytes());
    }
    for text in ["", "\n", "a", "a\n", "a\nb"] {
        let mut doc = Document::new(text);
        let revision = doc.revision();
        run(&mut doc, ":sort");
        assert_eq!(doc.revision(), revision);
        assert!(!doc.undo());
    }
}
#[test]
fn unicode_case_insensitive_and_unique_use_full_lines() {
    restored(
        Document::new("Zèbre\nélan\nÉlan\naardvark"),
        ":sort iu",
        "aardvark\nZèbre\nélan",
    );
    restored(Document::new("x2\ny1\nx2\nx1"), ":sort nu", "y1\nx1\nx2");
    restored(Document::new("A\na\nB"), ":sort! iu", "B\na");
    restored(
        Document::new("z👩🏽‍💻\na🇳🇿\ne\u{301}"),
        ":sort",
        "a🇳🇿\ne\u{301}\nz👩🏽‍💻",
    );
}
#[test]
fn numeric_bases_signed_values_missing_numbers_and_stable_ties() {
    for (flag, before, after) in [
        (
            "n",
            "x10\nmissing\nz-2\nb2\na2",
            "missing\nz-2\nb2\na2\nx10",
        ),
        ("x", "0x10\n0x2\n-ff\nmissing", "missing\n-ff\n0x2\n0x10"),
        ("o", "10\n7\n9\n-2", "-2\n9\n7\n10"),
        ("b", "0b10\n1\n0b11\n-10", "-10\n1\n0b10\n0b11"),
    ] {
        restored(Document::new(before), &format!(":sort {flag}"), after);
    }
    restored(Document::new("a1\nb1\nc2"), ":sort! n", "c2\nb1\na1");
}
#[test]
fn pattern_keys_skip_or_match_and_missing_patterns_stay_separate() {
    restored(
        Document::new("z,apple\na,pear\nmissing\nnone\nb,banana"),
        ":sort /[^,]*,/",
        "missing\nnone\nz,apple\nb,banana\na,pear",
    );
    restored(Document::new("z9\na2\nb3"), ":sort r /[0-9]/", "a2\nb3\nz9");
    restored(
        Document::new("z,apple\na,pear\nmissing\nnone"),
        ":sort! /[^,]*,/",
        "a,pear\nz,apple\nnone\nmissing",
    );
}
#[test]
fn pattern_uses_ignorecase_without_smartcase_or_search_history_mutation() {
    let mut document = Document::new("X2\nx1\nx3");
    let mut context = ExExecutionContext::default();
    context.search_options.ignorecase = true;
    context.search_options.smartcase = true;
    context.last_search_pattern = Some("X".into());
    let mut state = ExExecutionState::default();
    execute_ex(
        &mut document,
        &mut state,
        &context,
        &parse_ex(":sort // n").unwrap(),
        &(),
    )
    .unwrap();
    assert_eq!(document.text(), "x1\nX2\nx3");
    assert_eq!(context.last_search_pattern.as_deref(), Some("X"));
    assert!(!state.has_previous_substitute());
}
#[test]
fn invalid_pattern_range_and_stale_preparation_preserve_everything() {
    let mut document = Document::new("z\na");
    let original = document.source_bytes();
    for command in [
        ":sort /[/",
        ":sort /\\(a\\)/",
        ":sort //",
        ":0sort",
        ":3sort",
        ":2,1sort",
    ] {
        assert!(
            execute_ex(
                &mut document,
                &mut ExExecutionState::default(),
                &ExExecutionContext::default(),
                &parse_ex(command).unwrap(),
                &()
            )
            .is_err(),
            "{command}"
        );
        assert_eq!(document.source_bytes(), original);
        assert!(!document.undo());
    }
    let mut state = ExExecutionState::default();
    let plan = prepare_ex(
        &document,
        &state,
        &ExExecutionContext::default(),
        &parse_ex(":sort").unwrap(),
        &(),
    )
    .unwrap();
    document.replace(0..0, "other ").unwrap();
    let changed = document.source_bytes();
    assert!(commit_ex(&mut document, &mut state, plan).is_err());
    assert_eq!(document.source_bytes(), changed);
}
#[test]
fn source_delimiters_encoding_and_literal_lf_are_preserved() {
    let mut doc = Document::from_bytes_with_file_format(
        b"z\r\na\nb\r\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    run(&mut doc, ":sort");
    assert_eq!(doc.source_bytes(), b"a\r\nb\nz\r\n");
    let mut doc = Document::from_bytes_with_file_format(
        b"z\ra\nb\rc".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    run(&mut doc, ":sort");
    assert_eq!(doc.source_bytes(), b"a\nb\rc\rz");
    assert_eq!(doc.line_count(), 3);
    let mut bytes = vec![0xff, 0xfe];
    for unit in "z\na".encode_utf16() {
        bytes.extend(unit.to_le_bytes());
    }
    let mut doc =
        Document::from_bytes(bytes.clone(), Encoding::Utf16Le, Format::PlainText).unwrap();
    run(&mut doc, ":sort");
    assert_eq!(doc.text(), "a\nz");
    assert_eq!(&doc.source_bytes()[..2], &[0xff, 0xfe]);
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), bytes);
}
#[test]
fn markdown_source_reorders_literal_syntax() {
    let mut doc = source("__z__\n**a**\nplain", Format::MarkdownSource);
    run(&mut doc, ":sort");
    assert_eq!(doc.source_bytes(), b"**a**\n__z__\nplain");
}
#[test]
fn markdown_rich_paragraphs_keep_delimiters_and_styles() {
    let mut doc = source("__z__\n\n**a**\n\nplain", Format::Markdown);
    run(&mut doc, ":sort");
    assert_eq!(doc.text(), "a\nplain\nz");
    assert_eq!(doc.source_bytes(), b"**a**\n\nplain\n\n__z__");
}
#[test]
fn html_rich_paragraphs_keep_exact_inline_source_and_outer_trivia() {
    let original="<!doctype html><!--lead--><div><p lang='en'>Z &amp; last</p><!--gap--><h2 style='color:red'><b>A</b></h2></div><!--tail-->";
    let mut doc = source(original, Format::Html);
    let ids = doc
        .projection()
        .blocks()
        .iter()
        .map(|b| b.id)
        .collect::<Vec<_>>();
    let outcome = run(&mut doc, ":sort");
    assert_eq!(doc.text(), "A\nZ & last");
    assert_eq!(doc.source_bytes(), b"<!doctype html><!--lead--><div><h2 style='color:red'><b>A</b></h2><!--gap--><p lang='en'>Z &amp; last</p></div><!--tail-->");
    assert_eq!(
        doc.projection()
            .blocks()
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    assert_eq!(
        outcome
            .model_transaction()
            .unwrap()
            .summary()
            .source_patches()
            .len(),
        1
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), original.as_bytes());
    assert!(doc.redo());
}
#[test]
fn rich_ambiguous_context_rejects_without_flattening() {
    for (format, text) in [
        (Format::Html, "<div><p>Z</p></div><div><p>A</p></div>"),
        (Format::Html, "<p>Z<br>B</p><p>A</p>"),
        (Format::Rtf, r"{\rtf1\b Z\par\i A}"),
        (
            Format::Rtf,
            r"{\rtf1{\fonttbl{\f0 Times;}}{\pard\b Z\par}{\pard\i A\par}}",
        ),
    ] {
        let mut doc = source(text, format);
        let bytes = doc.source_bytes();
        assert!(execute_ex(
            &mut doc,
            &mut ExExecutionState::default(),
            &ExExecutionContext::default(),
            &parse_ex(":sort").unwrap(),
            &()
        )
        .is_err());
        assert_eq!(doc.source_bytes(), bytes);
        assert!(!doc.undo());
    }
}
#[test]
fn document_reorder_rejects_duplicate_or_outside_origins() {
    let doc = Document::new("z\na");
    for order in [vec![0, 0], vec![2], vec![]] {
        assert!(doc
            .prepare_model_request(ModelRequest::ReorderHardLines {
                document: doc.id(),
                revision: doc.revision(),
                source_lines: 0..2,
                order
            })
            .is_err());
    }
}
#[test]
fn command_prompt_sort_preserves_registers_and_undo_grouping() {
    let mut doc = Document::new("z\na");
    let mut commands = CommandInterpreter::default();
    for key in [Key::Char('y'), Key::Char('y')] {
        commands.handle(&mut doc, InputEvent::Key(key)).unwrap();
    }
    for key in ":sort".chars().map(Key::Char).chain([Key::Enter]) {
        commands.handle(&mut doc, InputEvent::Key(key)).unwrap();
    }
    assert_eq!(doc.text(), "a\nz");
    commands
        .handle(&mut doc, InputEvent::Key(Key::Char('u')))
        .unwrap();
    assert_eq!(doc.text(), "z\na");
    commands
        .handle(&mut doc, InputEvent::Key(Key::Char('p')))
        .unwrap();
    assert_eq!(doc.text(), "z\nz\na");
}
#[test]
fn source_view_sorts_raw_lines_even_when_html_interpretation_changes() {
    let original = "<p>\nzebra\n</p>\n<p>apple</p>\n";
    let mut doc = source(original, Format::HtmlSource);
    run(&mut doc, ":sort");
    assert_eq!(doc.source_bytes(), b"</p>\n<p>\n<p>apple</p>\nzebra\n");
    assert_eq!(doc.text(), "</p>\n<p>\n<p>apple</p>\nzebra\n");
    let reopened =
        Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::HtmlSource).unwrap();
    assert_eq!(
        doc.projection().style_spans(),
        reopened.projection().style_spans()
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), original.as_bytes());
}
#[test]
fn unique_keeps_terminal_newline_and_one_source_row() {
    for (before, after) in [("b\na\nb\n", "a\nb\n"), ("x\nx\n", "x\n"), ("x\nx", "x")] {
        restored(Document::new(before), ":sort u", after);
    }
    let mut doc = Document::from_bytes_with_file_format(
        b"x\r\nx\r\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    run(&mut doc, ":sort u");
    assert_eq!(doc.source_bytes(), b"x\r\n");
}
#[test]
fn sorted_line_identity_keeps_anchor_and_large_document_history() {
    use evim_core::document::{Association, BoundaryAffinity, DeletionRecovery, MappingOutcome};
    let before = (0..10_000)
        .rev()
        .map(|n| format!("row{n:05}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut doc = Document::new(&before);
    let anchor = doc
        .text_anchor(
            doc.text_point(2).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .unwrap();
    run(&mut doc, ":sort");
    assert!(doc.text().starts_with("row00000\nrow00001\n"));
    assert!(
        matches!(doc.resolve_text_anchor(anchor).unwrap(), MappingOutcome::Moved(point) if point.offset()==9_999*9+2)
    );
    assert!(doc.undo());
    assert_eq!(doc.text(), before);
}
#[test]
fn html_subrange_can_sort_one_container_without_touching_other_containers() {
    let mut doc = source(
        "<div><p>Z</p><p>A</p></div><section><p>Other</p></section>",
        Format::Html,
    );
    run(&mut doc, ":1,2sort");
    assert_eq!(
        doc.source_bytes(),
        b"<div><p>A</p><p>Z</p></div><section><p>Other</p></section>"
    );
}
#[test]
fn rich_equal_text_reverse_and_unique_preserve_selected_run_identity() {
    let original = "<p><b>same</b></p><!--between--><p><i>same</i></p>";
    let mut doc = source(original, Format::Html);
    run(&mut doc, ":sort!");
    assert_eq!(
        doc.source_bytes(),
        b"<p><i>same</i></p><!--between--><p><b>same</b></p>"
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), original.as_bytes());
    run(&mut doc, ":sort u");
    assert_eq!(doc.source_bytes(), b"<p><b>same</b></p><!--between-->");
    assert_eq!(doc.text(), "same");
}
#[test]
fn numeric_overflow_saturates_and_pattern_resource_failure_is_atomic() {
    restored(
        Document::new("999999999999999999999999\n-999999999999999999999999\n0"),
        ":sort n",
        "-999999999999999999999999\n0\n999999999999999999999999",
    );
    let mut doc = Document::new("z\na");
    let original = doc.source_bytes();
    let command = parse_ex(&format!(":sort /{}/", "a".repeat(17_000))).unwrap();
    assert!(execute_ex(
        &mut doc,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &command,
        &()
    )
    .is_err());
    assert_eq!(doc.source_bytes(), original);
    assert!(!doc.undo());
    let invalid = evim_core::command::ex::ExCommand {
        range: None,
        bang: false,
        action: ExAction::Sort(evim_core::command::ex::SortOptions {
            radix: Some(1),
            ..Default::default()
        }),
    };
    assert!(execute_ex(
        &mut doc,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &invalid,
        &()
    )
    .is_err());
}
#[test]
fn markdown_multiline_soft_paragraph_reorders_as_one_styled_line() {
    let mut doc = source("__zebra\nlast__\n\n**apple**", Format::Markdown);
    run(&mut doc, ":sort");
    assert_eq!(doc.text(), "apple\nzebra last");
    assert_eq!(doc.source_bytes(), b"**apple**\n\n__zebra\nlast__");
}
#[test]
fn coordinator_obeys_sort_cursor_even_when_new_ordinal_equals_old_zero() {
    use evim_core::layout::MockTextMeasurementProvider;
    use evim_core::{Core, CoreEvent};
    for (format, before, command, expected_cursor) in [
        (
            Format::PlainText,
            "pear\nApple\nbanana\napple\n10\n2\n",
            ":sort iu",
            0,
        ),
        (
            Format::MarkdownSource,
            "outside\n  zebra\n apple\ntail",
            ":2,3sort / */",
            9,
        ),
    ] {
        let mut core = Core::new(source(before, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
        for key in command.chars().map(Key::Char).chain([Key::Enter]) {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            expected_cursor,
            "{format:?}"
        );
    }
}
#[test]
fn visual_line_sort_uses_selected_hard_line_range_and_one_undo() {
    let mut document = Document::new("z\na\noutside");
    let mut commands = CommandInterpreter::new();
    for key in "Vj:sort".chars().map(Key::Char).chain([Key::Enter]) {
        commands
            .handle(&mut document, InputEvent::Key(key))
            .unwrap();
    }
    assert_eq!(document.text(), "a\nz\noutside");
    assert_eq!(commands.cursor(), 0);
    assert!(document.undo());
    assert_eq!(document.text(), "z\na\noutside");
    assert!(!document.undo());
}
#[test]
fn visual_character_line_and_block_enter_checked_hard_line_ex_ranges() {
    use evim_core::command::Mode;
    use evim_core::layout::MockTextMeasurementProvider;
    use evim_core::{Core, CoreEvent};
    for (prefix, block, expected_range, expected) in [
        ("v2j", false, "1,3", "a\nb\nz\noutside"),
        ("Vj", false, "1,2", "a\nz\nb\noutside"),
        ("j", true, "1,2", "a\nz\nb\noutside"),
    ] {
        let mut core = Core::new(Document::new("z\na\nb\noutside"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
        if block {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('q'))))
                .unwrap();
        }
        for key in prefix.chars().chain([':']).map(Key::Char) {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert_eq!(
            core.command_state(view).unwrap().command_line(),
            Some(expected_range)
        );
        assert_eq!(core.document().text(), "z\na\nb\noutside");
        for key in "sort".chars().map(Key::Char).chain([Key::Enter]) {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().text(), "z\na\nb\noutside");
    }
}
#[test]
fn visual_ex_escape_remembers_selection_and_foreign_edit_makes_prefill_stale() {
    use evim_core::command::{CommandStatus, Mode};
    use evim_core::layout::MockTextMeasurementProvider;
    use evim_core::{Core, CoreEvent};
    let mut core = Core::new(Document::new("z\na\noutside"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    let second = core.add_view(MockTextMeasurementProvider::new(), 250., 200.);
    for key in "Vj:".chars().map(Key::Char).chain([Key::Escape]) {
        core.handle(first, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    assert_eq!(core.document().text(), "z\na\noutside");
    assert_eq!(core.command_state(first).unwrap().mode(), Mode::Normal);
    for key in "gv:".chars().map(Key::Char) {
        core.handle(first, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    assert_eq!(
        core.command_state(first).unwrap().command_line(),
        Some("1,2")
    );
    for key in "iX".chars().map(Key::Char).chain([Key::Escape]) {
        core.handle(second, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    for key in "sort".chars().map(Key::Char) {
        core.handle(first, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }
    let output = core
        .handle(first, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    assert!(
        matches!(output.command.unwrap().status, CommandStatus::Error(message) if message.contains("stale"))
    );
    assert_eq!(core.document().text(), "Xz\na\noutside");
    assert_eq!(core.command_state(first).unwrap().mode(), Mode::Normal);
}
