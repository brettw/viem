//! `CTRL-W` window commands: grammar, counts, cancellation, and the typed
//! requests the frontend receives. These commands never change the document.

use viem_core::command::ex_execute::{ExFileRequest, ExFrontendRequest};
use viem_core::command::window::WindowRequest;
use viem_core::command::{CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key, Mode};
use viem_core::document::Document;

fn setup() -> (CommandInterpreter, Document) {
    (
        CommandInterpreter::new(),
        Document::new("alpha beta\nsecond line\nthird"),
    )
}

fn send(c: &mut CommandInterpreter, d: &mut Document, key: Key) -> CommandOutput {
    c.handle(d, InputEvent::Key(key)).unwrap()
}

fn requests(output: &CommandOutput) -> Vec<ExFrontendRequest> {
    output
        .ex_outcome
        .as_ref()
        .map(|outcome| outcome.frontend_requests.clone())
        .unwrap_or_default()
}

/// Type an optional count, then `CTRL-W`, then one command key.
fn window(c: &mut CommandInterpreter, d: &mut Document, count: &str, key: Key) -> CommandOutput {
    for digit in count.chars() {
        send(c, d, Key::Char(digit));
    }
    let pending = send(c, d, Key::Ctrl('w'));
    assert!(
        matches!(pending.status, CommandStatus::Pending),
        "{:?}",
        pending.status
    );
    send(c, d, key)
}

fn single(
    c: &mut CommandInterpreter,
    d: &mut Document,
    count: &str,
    key: Key,
) -> ExFrontendRequest {
    let output = window(c, d, count, key);
    assert!(
        matches!(output.status, CommandStatus::Complete),
        "{key:?}: {:?}",
        output.status
    );
    let requests = requests(&output);
    assert_eq!(requests.len(), 1, "{key:?}");
    requests.into_iter().next().unwrap()
}

#[test]
fn focus_commands_publish_their_requests_with_vim_counts() {
    let (mut c, mut d) = setup();
    for key in [Key::Char('j'), Key::Down, Key::Ctrl('j')] {
        assert_eq!(
            single(&mut c, &mut d, "", key),
            ExFrontendRequest::Window(WindowRequest::FocusDown { count: 1 })
        );
        assert_eq!(
            single(&mut c, &mut d, "3", key),
            ExFrontendRequest::Window(WindowRequest::FocusDown { count: 3 })
        );
    }
    for key in [Key::Char('k'), Key::Up, Key::Ctrl('k')] {
        assert_eq!(
            single(&mut c, &mut d, "2", key),
            ExFrontendRequest::Window(WindowRequest::FocusUp { count: 2 })
        );
    }
    assert_eq!(
        single(&mut c, &mut d, "", Key::Char('w')),
        ExFrontendRequest::Window(WindowRequest::FocusNext { index: None })
    );
    assert_eq!(
        single(&mut c, &mut d, "2", Key::Ctrl('w')),
        ExFrontendRequest::Window(WindowRequest::FocusNext { index: Some(2) })
    );
    assert_eq!(
        single(&mut c, &mut d, "", Key::Char('W')),
        ExFrontendRequest::Window(WindowRequest::FocusPrevious { index: None })
    );
    for (key, expected) in [
        (Key::Char('t'), WindowRequest::FocusTop),
        (Key::Char('b'), WindowRequest::FocusBottom),
        (Key::Char('p'), WindowRequest::FocusLastAccessed),
    ] {
        assert_eq!(
            single(&mut c, &mut d, "", key),
            ExFrontendRequest::Window(expected)
        );
    }
}

#[test]
fn order_and_size_commands_publish_their_requests() {
    let (mut c, mut d) = setup();
    for (count, key, expected) in [
        ("", Key::Char('r'), WindowRequest::RotateDown { count: 1 }),
        ("2", Key::Char('r'), WindowRequest::RotateDown { count: 2 }),
        ("2", Key::Char('R'), WindowRequest::RotateUp { count: 2 }),
        ("", Key::Char('x'), WindowRequest::Exchange { index: None }),
        (
            "3",
            Key::Char('x'),
            WindowRequest::Exchange { index: Some(3) },
        ),
        ("", Key::Char('K'), WindowRequest::MoveToTop),
        ("", Key::Char('J'), WindowRequest::MoveToBottom),
        ("", Key::Char('o'), WindowRequest::CloseOthers),
        ("", Key::Char('+'), WindowRequest::Grow { rows: 1 }),
        ("4", Key::Char('+'), WindowRequest::Grow { rows: 4 }),
        ("4", Key::Char('-'), WindowRequest::Shrink { rows: 4 }),
        ("", Key::Char('_'), WindowRequest::SetHeight { rows: None }),
        (
            "9",
            Key::Char('_'),
            WindowRequest::SetHeight { rows: Some(9) },
        ),
        ("", Key::Char('='), WindowRequest::EqualizeHeights),
    ] {
        assert_eq!(
            single(&mut c, &mut d, count, key),
            ExFrontendRequest::Window(expected),
            "{count}<C-w>{key:?}"
        );
    }
}

#[test]
fn lifecycle_commands_reuse_the_existing_ex_requests() {
    let (mut c, mut d) = setup();
    for key in [
        Key::Char('s'),
        Key::Char('S'),
        Key::Ctrl('s'),
        Key::Char('v'),
        Key::Ctrl('v'),
    ] {
        assert_eq!(
            single(&mut c, &mut d, "", key),
            ExFrontendRequest::File(ExFileRequest::Split { path: None }),
            "{key:?}"
        );
    }
    assert_eq!(
        single(&mut c, &mut d, "", Key::Char('n')),
        ExFrontendRequest::File(ExFileRequest::New { force: false })
    );
    for key in [Key::Char('q'), Key::Char('c'), Key::Ctrl('q')] {
        assert_eq!(
            single(&mut c, &mut d, "", key),
            ExFrontendRequest::File(ExFileRequest::Quit { force: false }),
            "{key:?}"
        );
    }
}

#[test]
fn horizontal_focus_is_accepted_and_unsupported_layouts_are_reported() {
    let (mut c, mut d) = setup();
    for key in [Key::Char('h'), Key::Char('l'), Key::Left, Key::Right] {
        let output = window(&mut c, &mut d, "", key);
        assert!(matches!(output.status, CommandStatus::Complete), "{key:?}");
        assert!(requests(&output).is_empty(), "{key:?}");
    }
    for key in [
        Key::Char('H'),
        Key::Char('L'),
        Key::Char('<'),
        Key::Char('>'),
        Key::Char('|'),
        Key::Char('T'),
        Key::Char('5'),
        Key::Enter,
    ] {
        let output = window(&mut c, &mut d, "", key);
        assert!(
            matches!(output.status, CommandStatus::Unsupported(_)),
            "{key:?}: {:?}",
            output.status
        );
        assert!(requests(&output).is_empty(), "{key:?}");
    }
}

#[test]
fn the_prefix_cancels_and_leaves_no_trace() {
    let (mut c, mut d) = setup();
    let source = d.source_bytes();
    let cursor = c.cursor();
    let pending = send(&mut c, &mut d, Key::Ctrl('w'));
    assert!(matches!(pending.status, CommandStatus::Pending));
    let cancelled = send(&mut c, &mut d, Key::Escape);
    assert!(matches!(
        cancelled.status,
        CommandStatus::Cancelled | CommandStatus::Complete
    ));
    assert_eq!(c.mode(), Mode::Normal);
    assert_eq!(c.cursor(), cursor);
    assert_eq!(d.source_bytes(), source);
    // The next key is an ordinary command again.
    send(&mut c, &mut d, Key::Char('j'));
    assert_ne!(c.cursor(), cursor);
}

#[test]
fn window_commands_change_no_text_registers_or_repeat_state() {
    let (mut c, mut d) = setup();
    send(&mut c, &mut d, Key::Char('y'));
    send(&mut c, &mut d, Key::Char('y'));
    let yanked = c.register('"').cloned();
    let revision = d.revision();
    send(&mut c, &mut d, Key::Char('x'));
    let after_delete = d.text().to_owned();
    let deleted = c.register('"').cloned();
    assert_ne!(deleted, yanked);

    for key in [
        Key::Char('j'),
        Key::Char('r'),
        Key::Char('='),
        Key::Char('K'),
    ] {
        let output = window(&mut c, &mut d, "", key);
        assert!(!output.document_changed, "{key:?}");
        assert_eq!(d.text(), after_delete, "{key:?}");
        assert_eq!(c.register('"').cloned(), deleted, "{key:?}");
    }
    assert_ne!(d.revision(), revision);
    // Dot still repeats the last real change, not a window command.
    send(&mut c, &mut d, Key::Char('.'));
    assert_eq!(d.text(), "pha beta\nsecond line\nthird");
}

#[test]
fn visual_mode_takes_window_commands_and_keeps_its_selection() {
    let (mut c, mut d) = setup();
    send(&mut c, &mut d, Key::Char('v'));
    send(&mut c, &mut d, Key::Char('l'));
    assert_eq!(c.mode(), Mode::VisualCharacter);
    let anchor = c.visual_anchor();
    assert_eq!(
        single(&mut c, &mut d, "", Key::Char('j')),
        ExFrontendRequest::Window(WindowRequest::FocusDown { count: 1 })
    );
    assert_eq!(
        c.mode(),
        Mode::VisualCharacter,
        "a window command does not leave Visual"
    );
    assert_eq!(c.visual_anchor(), anchor);
}

#[test]
fn insert_mode_ctrl_w_still_deletes_the_previous_word() {
    let mut d = Document::new("base");
    let mut c = CommandInterpreter::new();
    for key in "A one two".chars() {
        send(&mut c, &mut d, Key::Char(key));
    }
    assert_eq!(d.text(), "base one two");
    let output = send(&mut c, &mut d, Key::Ctrl('w'));
    assert!(
        matches!(output.status, CommandStatus::Complete),
        "{:?}",
        output.status
    );
    assert_eq!(d.text(), "base one ");
    assert!(requests(&output).is_empty());
}
