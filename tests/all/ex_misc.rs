use viem_core::command::ex::{parse_ex, ExAction};
use viem_core::command::ex_execute::{
    execute_ex, ExExecutionContext, ExExecutionState, ExFrontendRequest, ExInfoRequest, ExOutcome,
    HardLineRange,
};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, FileFormat, Format, ParagraphAlignment};

fn run(doc: &mut Document, context: &ExExecutionContext, input: &str) -> ExOutcome {
    execute_ex(
        doc,
        &mut ExExecutionState::default(),
        context,
        &parse_ex(input).unwrap(),
        &(),
    )
    .unwrap()
}
fn ex(
    cmd: &mut CommandInterpreter,
    doc: &mut Document,
    input: &str,
) -> viem_core::command::CommandOutput {
    cmd.handle(doc, InputEvent::Key(Key::Char(':'))).unwrap();
    for ch in input.chars() {
        cmd.handle(doc, InputEvent::Key(Key::Char(ch))).unwrap();
    }
    cmd.handle(doc, InputEvent::Key(Key::Enter)).unwrap()
}
#[test]
fn print_aliases_counts_and_flags_do_not_edit_or_touch_registers() {
    let mut doc = Document::new("one\n\ttwo\nthree\nfour");
    let revision = doc.revision();
    for (input, range, number, list) in [
        (":p", HardLineRange { start: 0, end: 0 }, false, false),
        (
            ":2,3print",
            HardLineRange { start: 1, end: 2 },
            false,
            false,
        ),
        (":1,2nu 2", HardLineRange { start: 1, end: 2 }, true, false),
        (":%l", HardLineRange { start: 0, end: 3 }, false, true),
        (":# 2", HardLineRange { start: 0, end: 1 }, true, false),
    ] {
        let outcome = run(&mut doc, &ExExecutionContext::default(), input);
        assert_eq!(
            outcome.frontend_requests,
            [ExFrontendRequest::Info(ExInfoRequest::PrintLines {
                range,
                number,
                list
            })]
        );
        assert!(outcome.register_effects.is_empty());
        assert_eq!(doc.revision(), revision);
    }
    assert!(!doc.undo());
}
#[test]
fn ex_shifts_obey_repetition_line_counts_and_one_undo_unit() {
    let mut doc = Document::new("a\n  b\n\nc\n  d");
    let source = doc.source_bytes();
    let outcome = run(&mut doc, &ExExecutionContext::default(), ":1,2>> 3");
    assert_eq!(doc.text(), "a\n      b\n\n    c\n  d");
    assert!(outcome.register_effects.is_empty());
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source);
    assert!(!doc.undo());
    run(&mut doc, &ExExecutionContext::default(), ":%<");
    assert_eq!(doc.text(), "a\nb\n\nc\nd");
}
#[test]
fn retab_preserves_columns_with_old_tabstops_and_changes_only_requested_runs() {
    let mut doc = Document::new("\tword\tX\n  word  X\n\tother");
    let mut context = ExExecutionContext::default();
    context.indentation.local.tabstop = Some(4);
    context.indentation.local.expandtab = Some(false);
    let outcome = run(&mut doc, &context, ":1,2retab 8");
    assert_eq!(doc.text(), "    word    X\n  word  X\n\tother");
    assert_eq!(outcome.option_effects.len(), 1);
    assert!(doc.undo());
    assert!(!doc.undo());
    let mut cmd = CommandInterpreter::new();
    assert_eq!(
        ex(&mut cmd, &mut doc, "set ts=4 et").status,
        CommandStatus::Complete
    );
    assert_eq!(
        ex(&mut cmd, &mut doc, "retab -indentonly 8").status,
        CommandStatus::Complete
    );
    assert_eq!(doc.text(), "    word\tX\n  word  X\n    other");
    assert_eq!(cmd.indentation_options().tabstop, 8);
}
#[test]
fn retab_bang_compresses_spaces_and_invalid_width_is_atomic() {
    let mut doc = Document::new("a   b\n    c");
    let mut context = ExExecutionContext::default();
    context.indentation.local.tabstop = Some(4);
    context.indentation.local.expandtab = Some(false);
    run(&mut doc, &context, ":retab");
    assert_eq!(doc.text(), "a   b\n    c");
    assert!(!doc.undo());
    run(&mut doc, &context, ":retab!");
    assert_eq!(doc.text(), "a\tb\n\tc");
    let before = doc.source_bytes();
    assert!(execute_ex(
        &mut doc,
        &mut ExExecutionState::default(),
        &context,
        &parse_ex(":retab 9999").unwrap(),
        &()
    )
    .is_err());
    assert_eq!(doc.source_bytes(), before);
}
#[test]
fn alignment_uses_logical_columns_and_preserves_terminators_and_undo() {
    let mut doc = Document::from_bytes_with_file_format(
        b"  one\r\n \xe7\x95\x8c\r\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let original = doc.source_bytes();
    run(&mut doc, &ExExecutionContext::default(), ":%center 9");
    assert_eq!(doc.source_bytes(), "   one\r\n   界\r\n".as_bytes());
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), original);
    assert!(!doc.undo());
    run(&mut doc, &ExExecutionContext::default(), ":%right 9");
    assert_eq!(doc.text(), "      one\n       界\n");
    run(&mut doc, &ExExecutionContext::default(), ":%left 1");
    assert_eq!(doc.text(), " one\n 界\n");
}
#[test]
fn rich_alignment_changes_paragraph_properties_without_padding_or_style_loss() {
    for (format, text) in [
        (Format::Html, "<p><b>one</b></p><p>two</p>"),
        (Format::Rtf, "{\\rtf1 one\\par two}"),
    ] {
        let mut doc =
            Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let source = doc.source_bytes();
        let logical = doc.text().to_owned();
        execute_ex(
            &mut doc,
            &mut ExExecutionState::default(),
            &ExExecutionContext::default(),
            &parse_ex(":%center").unwrap(),
            &(),
        )
        .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        assert_eq!(doc.text(), logical);
        assert!(doc
            .projection()
            .blocks()
            .iter()
            .any(|block| block.direct_paragraph.alignment == Some(ParagraphAlignment::Center)));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source);
        assert!(!doc.undo());
    }
    let mut markdown =
        Document::from_bytes(b"one\n\ntwo".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let before = markdown.source_bytes();
    assert!(execute_ex(
        &mut markdown,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &parse_ex(":center").unwrap(),
        &()
    )
    .is_err());
    assert_eq!(markdown.source_bytes(), before);
}
#[test]
fn delmarks_removes_only_named_marks_and_only_preserves_force_in_host_request() {
    let mut doc = Document::new("one\ntwo");
    let mut cmd = CommandInterpreter::new();
    for ch in "mambmc".chars() {
        cmd.handle(&mut doc, InputEvent::Key(Key::Char(ch)))
            .unwrap();
    }
    assert_eq!(
        ex(&mut cmd, &mut doc, "delmarks a-b").status,
        CommandStatus::Complete
    );
    assert!(matches!(
        ex(&mut cmd, &mut doc, "'ap").status,
        CommandStatus::ExError(_)
    ));
    assert_eq!(
        ex(&mut cmd, &mut doc, "'cp").status,
        CommandStatus::Complete
    );
    assert_eq!(
        ex(&mut cmd, &mut doc, "delmarks!").status,
        CommandStatus::Complete
    );
    assert!(matches!(
        ex(&mut cmd, &mut doc, "'cp").status,
        CommandStatus::ExError(_)
    ));
    let outcome = ex(&mut cmd, &mut doc, "only").ex_outcome.unwrap();
    assert!(matches!(
        outcome.frontend_requests.as_slice(),
        [ExFrontendRequest::File(
            viem_core::command::ex_execute::ExFileRequest::Only { force: false }
        )]
    ));
    let outcome = ex(&mut cmd, &mut doc, "only!").ex_outcome.unwrap();
    assert!(matches!(
        outcome.frontend_requests.as_slice(),
        [ExFrontendRequest::File(
            viem_core::command::ex_execute::ExFileRequest::Only { force: true }
        )]
    ));
    assert!(!doc.undo());
}
#[test]
fn grammar_rejects_invalid_arguments_and_keeps_existing_abbreviations() {
    for command in [
        ":print!",
        ":retab ++bad",
        ":delmarks z-a",
        ":delmarks A",
        ":1only",
        ":left -2",
        ":read !echo hi",
        ":source! file",
    ] {
        assert!(parse_ex(command).is_err(), "{command}");
    }
    for command in [
        ":p",
        ":nu",
        ":l",
        ":>>> 2",
        ":<<",
        ":ret! -indentonly 0",
        ":le 0",
        ":ri",
        ":ce",
        ":delm a-z<>",
        ":on",
        ":0r file",
        ":so file",
        ":f file",
    ] {
        assert!(
            parse_ex(command).is_ok(),
            "{command}: {:?}",
            parse_ex(command)
        );
    }
    assert!(matches!(
        parse_ex(":s/a/b/").unwrap().action,
        ExAction::Substitute(_)
    ));
    assert!(matches!(
        parse_ex(":se ts=8").unwrap().action,
        ExAction::Set(_)
    ));
}
#[test]
fn print_and_shift_tail_flags_accept_count_then_flags_and_reject_bad_tails() {
    let mut doc = Document::new("  one\n  two\n  three");
    for command in [":p 2#l", ":print 2 #lp", ":nu 2 l", ":list 2 #"] {
        let outcome = run(&mut doc, &ExExecutionContext::default(), command);
        assert_eq!(
            outcome.frontend_requests,
            [ExFrontendRequest::Info(ExInfoRequest::PrintLines {
                range: HardLineRange { start: 0, end: 1 },
                number: true,
                list: true,
            })]
        );
        assert_eq!(
            outcome.navigation,
            Some(viem_core::command::ex_execute::ExNavigation::TextOffset(8))
        );
    }
    for command in [":p # 2", ":p 2x", ":> 2 x", ":< p2"] {
        assert!(parse_ex(command).is_err(), "{command}");
    }
    let outcome = run(&mut doc, &ExExecutionContext::default(), ":>> 2#l");
    assert_eq!(
        outcome.frontend_requests,
        [ExFrontendRequest::Info(ExInfoRequest::PrintLines {
            range: HardLineRange { start: 1, end: 1 },
            number: true,
            list: true,
        })]
    );
    assert_eq!(doc.text(), "      one\n      two\n  three");
    assert!(doc.undo());
    assert!(!doc.undo());
}
#[test]
fn alignment_preserves_current_line_uses_default_for_zero_and_clears_blank_left_indent() {
    let mut doc = Document::new("  one\nx\ty\n   ");
    let mut context = ExExecutionContext::default();
    context.indentation.local.tabstop = Some(8);
    let outcome = run(&mut doc, &context, ":2right 12");
    assert_eq!(doc.text(), "  one\n      x\ty\n   ");
    assert_eq!(
        outcome.navigation,
        Some(viem_core::command::ex_execute::ExNavigation::TextOffset(2))
    );
    run(&mut doc, &ExExecutionContext::default(), ":%left 0");
    assert_eq!(doc.text(), "one\nx\ty\n");
    run(&mut doc, &ExExecutionContext::default(), ":1center 0");
    assert_eq!(
        doc.text().split('\n').next(),
        Some(format!("{}one", " ".repeat(38)).as_str())
    );
}
#[test]
fn rich_alignment_handles_empty_paragraphs_and_retains_character_scopes() {
    for (format, source) in [
        (Format::Html, "<p></p>"),
        (Format::Html, "<p><b>one</b></p><p><i>two</i></p>"),
        (Format::Rtf, r"{\rtf1 }"),
        (Format::Rtf, r"{\rtf1 {\b one}\par {\i two}}"),
    ] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = doc.text().to_owned();
        let styles = doc.projection().style_spans().to_vec();
        execute_ex(
            &mut doc,
            &mut ExExecutionState::default(),
            &ExExecutionContext::default(),
            &parse_ex(":%center").unwrap(),
            &(),
        )
        .unwrap_or_else(|error| panic!("{format:?} {source:?}: {error:?}"));
        assert_eq!(doc.text(), text);
        assert_eq!(doc.projection().style_spans(), styles);
        assert!(doc
            .projection()
            .blocks()
            .iter()
            .all(|block| block.direct_paragraph.alignment == Some(ParagraphAlignment::Center)));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_rich_alignment_survives_typing_and_does_not_change_other_paragraphs() {
    for (format, source) in [
        (Format::Html, "<p></p><p>other</p>"),
        (Format::Html, "<p><b></b></p><p>other</p>"),
        (Format::Rtf, r"{\rtf1 \par other}"),
        (Format::Rtf, r"{\rtf1 {\b }\par other}"),
    ] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let before_other = doc.projection().blocks()[1].direct_paragraph.clone();
        let before_character = doc.projection().blocks()[0]
            .direct_default_character
            .clone();
        run(&mut doc, &ExExecutionContext::default(), ":1center");
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center),
            "{source}"
        );
        assert_eq!(
            doc.projection().blocks()[0].direct_default_character,
            before_character,
            "{source}"
        );
        assert_eq!(
            doc.projection().blocks()[1].direct_paragraph,
            before_other,
            "{source}"
        );
        doc.insert(0, "typed").unwrap();
        assert_eq!(doc.text(), "typed\nother", "{source}");
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center),
            "{source}"
        );
        assert_eq!(
            doc.projection().blocks()[1].direct_paragraph,
            before_other,
            "{source}"
        );
        assert!(doc.undo());
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    for (format, source) in [(Format::Html, "<p></p>"), (Format::Rtf, r"{\rtf1 }")] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        run(&mut doc, &ExExecutionContext::default(), ":center");
        doc.insert(0, "typed").unwrap();
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center)
        );
        assert_eq!(doc.text(), "typed");
    }
}
