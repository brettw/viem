use super::*;
use crate::document::{Encoding, Format};
use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

fn fixture(format: Format, source: &str) -> (Document, CommandInterpreter) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut commands = CommandInterpreter::new();
    commands.set_smart_quotes(true);
    (document, commands)
}

fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) {
    let output = commands.handle(document, InputEvent::Key(key)).unwrap();
    assert!(
        matches!(
            output.status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "{key:?}: {:?}",
        output.status
    );
}

fn keys(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
    for ch in text.chars() {
        key(commands, document, Key::Char(ch));
    }
}

fn layout_keys(commands: &mut CommandInterpreter, document: &mut Document, keys: &[Key]) {
    for key in keys {
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(500.0, 500.0);
        engine.relayout(document, &mut view).unwrap();
        let mut context = LayoutCommandContext::new(
            view.snapshot().unwrap(),
            true,
            Viewport::new(0.0, 500.0).unwrap(),
        );
        let output = commands
            .handle_with_layout(document, InputEvent::Key(*key), &mut context)
            .unwrap();
        assert!(
            matches!(
                output.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            ),
            "{key:?}: {:?}",
            output.status
        );
    }
}

#[test]
fn puts_assist_at_resolved_destination_without_rewriting_registers() {
    for (put, expected) in [('P', "“q”ab"), ('p', "a”q”b")] {
        let (mut document, mut commands) = fixture(Format::PlainText, "ab");
        let value = RegisterValue::characterwise("\"q\"");
        commands.registers.yank(Some('a'), value.clone());
        keys(&mut commands, &mut document, &format!("\"a{put}"));
        assert_eq!(document.text(), expected);
        assert_eq!(commands.register('a'), Some(&value));
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.text(), "ab");
    }
    let (mut document, mut commands) = fixture(Format::Markdown, "ab\n\n```\ncd\n```");
    commands
        .registers
        .yank(Some('a'), RegisterValue::characterwise("\"q\""));
    assert!(commands.set_cursor(&document, document.text().find("cd").unwrap()));
    keys(&mut commands, &mut document, "\"a2P");
    assert_eq!(document.text(), "ab\n\"q\"\"q\"cd");
}

#[test]
fn ctrl_r_preflights_assisted_text_and_records_raw_repeat_with_actual_insert() {
    for mode in ['i', 'R'] {
        let (mut document, mut commands) = fixture(Format::PlainText, "xxxxxx");
        let raw = RegisterValue::characterwise("\"q\"");
        commands.registers.yank(Some('a'), raw.clone());
        key(&mut commands, &mut document, Key::Char(mode));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        key(&mut commands, &mut document, Key::Char('a'));
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(
            document.text(),
            if mode == 'i' {
                "“q”xxxxxx"
            } else {
                "“q”xxx"
            }
        );
        assert_eq!(commands.register('.').unwrap().text, "“q”");
        assert_eq!(commands.register('a'), Some(&raw));
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.text(), "xxxxxx");
    }
}

#[test]
fn insert_and_replace_dot_reinterpret_raw_quotes_in_code() {
    for mode in ['i', 'r', 'R'] {
        let (mut document, mut commands) = fixture(Format::Markdown, "abcd\n\n```\nefgh\n```");
        key(&mut commands, &mut document, Key::Char(mode));
        key(&mut commands, &mut document, Key::Char('"'));
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.register('.').unwrap().text, "“", "{mode}");
        assert!(commands.set_cursor(&document, document.text().find("efgh").unwrap()));
        key(&mut commands, &mut document, Key::Char('.'));
        key(&mut commands, &mut document, Key::Escape);
        assert!(
            document
                .text()
                .contains(if mode == 'i' { "\n\"efgh" } else { "\n\"fgh" }),
            "{mode}: {}",
            document.text()
        );
    }
}

#[test]
fn visual_replacement_and_put_use_selection_start_and_keep_code_literal() {
    let (mut document, mut commands) = fixture(Format::Markdown, "ab`cd`ef");
    keys(&mut commands, &mut document, "v5lr\"");
    assert_eq!(
        document.text().matches('"').count(),
        2,
        "{}",
        String::from_utf8_lossy(&document.source_bytes())
    );
    assert_eq!(document.text().matches('"').count(), 2);
    assert_eq!(commands.register('.').unwrap().text, "“");

    let (mut document, mut commands) = fixture(Format::PlainText, "xxxx");
    commands
        .registers
        .yank(Some('a'), RegisterValue::characterwise("\"q\""));
    keys(&mut commands, &mut document, "vl\"aP");
    assert_eq!(document.text(), "“q”xx");
    assert!(document.undo());
    assert_eq!(document.text(), "xxxx");
}

#[test]
fn block_insert_replace_and_put_assist_each_destination_row() {
    for action in ['I', 'r', 'P'] {
        let (mut document, mut commands) = fixture(Format::Markdown, "abcd\n\n```\nefgh\n```");
        commands
            .registers
            .yank(Some('a'), RegisterValue::characterwise("\""));
        let mut input = vec![Key::Ctrl('v'), Key::Char('j')];
        if action == 'P' {
            input.extend([Key::Char('"'), Key::Char('a')]);
        }
        input.push(Key::Char(action));
        if action != 'P' {
            input.push(Key::Char('"'));
        }
        if action == 'I' {
            input.push(Key::Escape);
        }
        layout_keys(&mut commands, &mut document, &input);
        assert!(
            document.text().starts_with('“'),
            "{action}: {}",
            document.text()
        );
        assert!(
            document.text().contains("\n\""),
            "{action}: {}",
            document.text()
        );
        assert!(document.undo());
        assert_eq!(document.text(), "abcd\nefgh");
    }
}

#[test]
fn normal_block_put_and_physical_source_put_preserve_code_context() {
    let (mut document, mut commands) = fixture(Format::Markdown, "abcd\n\n```\nefgh\n```");
    commands
        .registers
        .yank(Some('a'), RegisterValue::blockwise("\"q\"\n\"r\""));
    layout_keys(
        &mut commands,
        &mut document,
        &[Key::Char('"'), Key::Char('a'), Key::Char('P')],
    );
    assert_eq!(document.text(), "“q”abcd\n\"r\"efgh");

    let (mut document, mut commands) =
        fixture(Format::MarkdownSource, "x\n\n```\ncode\nline\n```");
    commands
        .set_line_mode(&document, LineMode::PhysicalSource)
        .unwrap();
    commands.registers.yank(
        Some('a'),
        RegisterValue::linewise("\"q\"\n"),
    );
    keys(&mut commands, &mut document, "\"aP");
    assert!(document.text().starts_with("“q”\n"));
    assert!(commands.set_cursor(&document, document.text().find("line\n```").unwrap()));
    keys(&mut commands, &mut document, "\"aP");
    assert!(document
        .text()
        .contains("```\ncode\n\"q\"\nline\n```"));
}

#[test]
fn rich_register_paste_preserves_bold_and_code_while_transforming_prose() {
    let source = Document::from_bytes(
        b"**\"bold\"** `\"code\"`".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let fragment = source.clipboard_fragment(0..source.text().len()).unwrap();
    let raw = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    for mode in ['P', 'i', 'v'] {
        let (mut document, mut commands) = fixture(Format::Markdown, "x");
        commands.registers.yank(Some('a'), raw.clone());
        match mode {
            'P' => keys(&mut commands, &mut document, "\"aP"),
            'i' => {
                key(&mut commands, &mut document, Key::Char('i'));
                key(&mut commands, &mut document, Key::Ctrl('r'));
                key(&mut commands, &mut document, Key::Char('a'));
            }
            _ => keys(&mut commands, &mut document, "v\"aP"),
        }
        assert!(
            document.text().starts_with("“bold” \"code\""),
            "{mode}: {}",
            document.text()
        );
        let serialized = String::from_utf8(document.source_bytes()).unwrap();
        assert!(serialized.contains("**“bold”**"), "{mode}: {serialized}");
        assert!(serialized.contains("`\"code\"`"), "{mode}: {serialized}");
        assert_eq!(commands.register('a'), Some(&raw));
        key(&mut commands, &mut document, Key::Escape);
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.source_bytes(), b"x");
    }
}

#[test]
fn replace_backspace_restores_text_and_style_after_quote_width_changes() {
    for (format, source) in [
        (Format::PlainText, "éabcd"),
        (Format::Rtf, r"{\rtf1{\b \u233?}abcd}"),
    ] {
        let (mut document, mut commands) = fixture(format, source);
        let original_styles = [0, 2].map(|at| crate::layout::DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap());
        keys(&mut commands, &mut document, "R\"");
        assert!(document.text().starts_with('“'));
        key(&mut commands, &mut document, Key::Backspace);
        assert_eq!(document.text(), "éabcd");
        assert_eq!([0, 2].map(|at| crate::layout::DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap()), original_styles);
        if !format.is_rich_text() { assert_eq!(document.source_bytes(), source.as_bytes()); }
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut document, Key::Char('\''));
        key(&mut commands, &mut document, Key::Escape);
        assert_eq!(commands.register('.').unwrap().text, "‘");
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn markdown_source_fence_batches_share_text_and_put_projection_and_caret_rules() {
    let input = "\"prose\" `\"code\"`\n\n```\n'code'\n```\n\n'prose'";
    let assisted = "“prose” `\"code\"`\n\n```\n'code'\n```\n\n‘prose’";
    for smart in [false, true] {
        for paste in [false, true] {
            let (mut document, mut commands) = fixture(Format::MarkdownSource, "tail");
            commands.set_smart_quotes(smart);
            if paste {
                commands
                    .registers
                    .yank(Some('a'), RegisterValue::characterwise(input));
                keys(&mut commands, &mut document, "\"aP");
            } else {
                key(&mut commands, &mut document, Key::Char('i'));
                let output = commands
                    .handle(&mut document, InputEvent::text(input))
                    .unwrap();
                assert_eq!(output.status, CommandStatus::Complete);
            }
            let authored = if smart { assisted } else { input };
            let expected_source = format!("{authored}tail");
            assert_eq!(document.source_bytes(), expected_source.as_bytes());
            let expected = Document::from_bytes(
                expected_source.into_bytes(),
                Encoding::Utf8,
                Format::MarkdownSource,
            )
            .unwrap();
            assert_eq!(document.text(), expected.text());
            let insertion_end = document.text().len() - 4;
            let expected_caret = if paste {
                document
                    .hard_line_snapshot()
                    .previous_grapheme_boundary(insertion_end)
                    .unwrap()
            } else {
                insertion_end
            };
            assert_eq!(
                commands.cursor(),
                expected_caret,
                "smart={smart}, paste={paste}"
            );
            key(&mut commands, &mut document, Key::Escape);
            key(&mut commands, &mut document, Key::Char('u'));
            assert_eq!(document.source_bytes(), b"tail");
        }
    }
}

#[test]
fn markdown_source_typing_caret_does_not_cross_unchanged_grapheme_suffixes() {
    for (before, input) in [("\u{301}next", "a"), ("🇧🇨🇩next", "🇦")] {
        let (mut document, mut commands) = fixture(Format::MarkdownSource, before);
        key(&mut commands, &mut document, Key::Char('i'));
        commands
            .handle(&mut document, InputEvent::text(input))
            .unwrap();
        let first_cluster_end = document.text().graphemes(true).next().unwrap().len();
        assert_eq!(commands.cursor(), first_cluster_end, "{before:?}");
        assert!(document.text()[commands.cursor()..].ends_with("next"));
    }
}
