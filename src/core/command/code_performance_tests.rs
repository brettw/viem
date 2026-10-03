use super::*;
use crate::document::{Encoding, Format};

fn event(commands: &mut CommandInterpreter, document: &mut Document, event: InputEvent) {
    let previous = document.projection().clone();
    let output = commands.handle(document, event.clone()).unwrap();
    assert!(
        !matches!(output.status, CommandStatus::Error(_)),
        "{event:?}: {:?}",
        output.status
    );
    assert!(
        !previous.compatibility_text_is_materialized(),
        "{event:?} flattened its input snapshot"
    );
    assert!(
        !document.projection().compatibility_text_is_materialized(),
        "{event:?} flattened its result snapshot"
    );
}

#[test]
fn ordinary_code_edits_do_not_flatten_at_start_middle_or_end() {
    let bytes = b"first word\n  second word\nlast word\n".repeat(10_000);
    for at in [0, bytes.len() / 2, bytes.len() - 3] {
        let mut document =
            Document::from_bytes(bytes.clone(), Encoding::Utf8, Format::Code).unwrap();
        // A fresh revision has no compatibility flat string, even if opening
        // reused a contiguous decode buffer as an optimization.
        document.replace(0..0, " ").unwrap();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, at));
        for input in [
            InputEvent::key('i'),
            InputEvent::text("x"),
            InputEvent::Key(Key::Backspace),
            InputEvent::Key(Key::Delete),
            InputEvent::Key(Key::Enter),
            InputEvent::Key(Key::Escape),
            InputEvent::key('x'),
            InputEvent::key('J'),
            InputEvent::key('o'),
            InputEvent::text("new"),
            InputEvent::Key(Key::Escape),
            InputEvent::key('O'),
            InputEvent::Key(Key::Escape),
            InputEvent::key('d'),
            InputEvent::key('d'),
            InputEvent::key('G'),
            InputEvent::key('g'),
            InputEvent::key('g'),
        ] {
            event(&mut commands, &mut document, input);
        }
    }
}

#[test]
fn joining_a_giant_code_line_only_reads_its_edges() {
    let text = format!("{}\n  tail\nnext", "x".repeat(2 * 1024 * 1024));
    let mut document =
        Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
    document.replace(0..0, "a").unwrap();
    let mut commands = CommandInterpreter::new();
    event(&mut commands, &mut document, InputEvent::key('J'));
    assert_eq!(document.hard_line_snapshot().line_count(), 2);
    assert_eq!(
        document
            .hard_line_snapshot()
            .slice_utf8(2 * 1024 * 1024..2 * 1024 * 1024 + 6)
            .unwrap(),
        "x tail"
    );
}

#[test]
fn first_nonblank_keeps_the_whitespace_combining_grapheme_whole() {
    let document = Document::new(" \u{301} word");
    assert_eq!(
        first_nonblank_document(&document, &document.hard_line_snapshot(), 0),
        0
    );
}

#[test]
fn normal_cursor_floors_stale_byte_ordinals_without_flattening() {
    let mut text = "ae\u{301}🙂x\n".repeat(1024);
    let mut document =
        Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    document.replace(0..0, "q").unwrap();
    text.insert(0, 'q');
    let lines = document.hard_line_snapshot();
    for offset in (0..64).chain([text.len(), text.len() + 1, usize::MAX]) {
        assert_eq!(
            normalize_normal_cursor_document(&document, &lines, offset),
            normalize_normal_cursor(&text, &lines, offset),
            "cursor byte {offset}",
        );
    }
    assert!(!document.projection().compatibility_text_is_materialized());
}

#[test]
fn ex_join_and_delete_keep_code_snapshots_unflattened() {
    let mut document = Document::from_bytes(
        b"one\n  two\nthree\n".repeat(3000),
        Encoding::Utf8,
        Format::Code,
    )
    .unwrap();
    document.replace(0..0, "x").unwrap();
    for command in [":1join", ":2delete"] {
        let previous = document.projection().clone();
        ex_execute::execute_ex(
            &mut document,
            &mut ex_execute::ExExecutionState::default(),
            &ex_execute::ExExecutionContext::default(),
            &ex::parse_ex(command).unwrap(),
            &(),
        )
        .unwrap();
        assert!(
            !previous.compatibility_text_is_materialized(),
            "{command} flattened input"
        );
        assert!(
            !document.projection().compatibility_text_is_materialized(),
            "{command} flattened result"
        );
    }
}

#[test]
fn sentence_and_pair_commands_counts_registers_and_repeat_keep_snapshots_unflattened() {
    for command in [")", "2)", "(", "%", "y%", "d%", "d)", "2d)", "c%"] {
        let prefix = "unrelated line\n".repeat(10_000);
        let text = format!(
            "{prefix}(one [two]). Next. Last.\n{}",
            "unrelated line\n".repeat(10_000)
        );
        let mut document =
            Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        document.replace(0..0, "X").unwrap();
        let original = document.source_bytes();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, prefix.len() + 1));
        for character in command.chars() {
            event(&mut commands, &mut document, InputEvent::key(character));
        }
        if command == "c%" {
            event(
                &mut commands,
                &mut document,
                InputEvent::text("replacement"),
            );
            event(&mut commands, &mut document, InputEvent::Key(Key::Escape));
        }
        if document.source_bytes() != original {
            let changed = document.source_bytes();
            event(&mut commands, &mut document, InputEvent::key('u'));
            assert_eq!(document.source_bytes(), original);
            event(
                &mut commands,
                &mut document,
                InputEvent::Key(Key::Ctrl('r')),
            );
            assert_eq!(document.source_bytes(), changed);
            event(&mut commands, &mut document, InputEvent::key('.'));
        }
    }
}

#[test]
fn local_case_commands_keep_snapshots_unflattened_and_restore_exact_source() {
    for command in ["~", "3~", "gUl", "gU)", "guu", "g~~"] {
        let prefix = "unrelated line\n".repeat(10_000);
        let mut document = Document::from_bytes(
            format!(
                "{prefix}aßİe\u{301}. Next sentence.\n{}",
                "unrelated line\n".repeat(10_000)
            )
            .into_bytes(),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap();
        document.replace(0..0, "X").unwrap();
        let original = document.source_bytes();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, prefix.len() + 1));
        for character in command.chars() {
            event(&mut commands, &mut document, InputEvent::key(character));
        }
        assert_ne!(document.source_bytes(), original, "{command}");
        let changed = document.source_bytes();
        event(&mut commands, &mut document, InputEvent::key('u'));
        assert_eq!(document.source_bytes(), original);
        event(
            &mut commands,
            &mut document,
            InputEvent::Key(Key::Ctrl('r')),
        );
        assert_eq!(document.source_bytes(), changed);
    }
}
