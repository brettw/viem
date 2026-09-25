use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key, Mode};
use viem_core::document::Document;

fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
    for ch in text.chars() {
        let output = c.handle(d, InputEvent::Key(Key::Char(ch))).unwrap();
        assert!(
            !matches!(output.status, CommandStatus::Error(_)),
            "{output:?}"
        );
    }
}
fn ex(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandOutput {
    keys(c, d, text);
    c.handle(d, InputEvent::Key(Key::Enter)).unwrap()
}
fn complete(output: &CommandOutput) {
    assert!(
        matches!(output.status, CommandStatus::Complete),
        "{output:?}"
    );
}

#[test]
fn ex_history_crosses_abandoned_branches_and_restores_command_state() {
    let mut d = Document::new("abcdef");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "x");
    keys(&mut c, &mut d, "x");
    let abandoned = d.history_status().current;
    keys(&mut c, &mut d, "u");
    keys(&mut c, &mut d, "lx");
    assert_eq!(d.text(), "bdef");
    let register = c.register('"').cloned();
    let nodes = d.history_status().node_count;
    let earlier = ex(&mut c, &mut d, ":earlier");
    complete(&earlier);
    assert!(earlier.history_navigation);
    assert_eq!(d.text(), "cdef");
    assert_eq!(d.history_status().current, abandoned);
    assert_eq!(c.mode(), Mode::Normal);
    assert_eq!(c.register('"'), register.as_ref());
    complete(&ex(&mut c, &mut d, ":lat"));
    assert_eq!(d.text(), "bdef");
    assert_eq!(d.history_status().node_count, nodes);
    // Ordinary undo follows the selected branch's parent rather than undoing
    // a newly-created navigation transaction.
    keys(&mut c, &mut d, "u");
    assert_eq!(d.text(), "bcdef");
}

#[test]
fn file_write_units_restore_saved_identity_and_do_not_replay_register_effects() {
    let mut d = Document::new("abcdef");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "x");
    d.mark_saved();
    keys(&mut c, &mut d, "x");
    d.mark_saved();
    keys(&mut c, &mut d, "x");
    complete(&ex(&mut c, &mut d, ":ea 1f"));
    assert_eq!(d.text(), "cdef");
    assert!(!d.is_dirty());
    complete(&ex(&mut c, &mut d, ":ea 1f"));
    assert_eq!(d.text(), "bcdef");
    assert!(d.is_dirty());
    complete(&ex(&mut c, &mut d, ":ea 1f"));
    assert_eq!(d.text(), "abcdef");
    complete(&ex(&mut c, &mut d, ":lat 2f"));
    assert_eq!(d.text(), "cdef");
    assert!(!d.is_dirty());
    complete(&ex(&mut c, &mut d, ":lat 1f"));
    assert_eq!(d.text(), "def");
}

#[test]
fn elapsed_time_units_and_counts_zero_or_beyond_boundary_are_nondestructive() {
    for unit in ["s", "m", "h", "d"] {
        let mut d = Document::new("abcdef");
        let mut c = CommandInterpreter::new();
        keys(&mut c, &mut d, "xx");
        let changed = d.history_status().current;
        complete(&ex(&mut c, &mut d, &format!(":earlier 99{unit}")));
        assert_eq!(d.text(), "abcdef");
        complete(&ex(&mut c, &mut d, &format!(":later 99{unit}")));
        assert_eq!(d.history_status().current, changed);
        complete(&ex(&mut c, &mut d, ":earlier 0"));
        assert_eq!(d.history_status().current, changed);
    }
}

#[test]
fn undolist_only_lists_leaves_and_has_no_document_or_cursor_effect() {
    let mut d = Document::new("abcdef");
    let mut c = CommandInterpreter::new();
    let empty = ex(&mut c, &mut d, ":undol");
    assert!(matches!(&empty.ex_outcome.unwrap().frontend_requests[..],
        [ExFrontendRequest::Info(ExInfoRequest::Message(message))] if message == "Nothing to undo"));
    keys(&mut c, &mut d, "xx");
    d.mark_saved();
    keys(&mut c, &mut d, "ulx");
    let before = (d.source_bytes(), d.history_status(), c.cursor());
    let output = ex(&mut c, &mut d, ":undolist");
    complete(&output);
    let outcome = output.ex_outcome.unwrap();
    let [ExFrontendRequest::Info(ExInfoRequest::Message(message))] =
        outcome.frontend_requests.as_slice()
    else {
        panic!("{outcome:?}");
    };
    let lines = message.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("number changes"));
    assert_eq!(lines[1].split_whitespace().next(), Some("2"));
    assert_eq!(lines[2].split_whitespace().next(), Some("3"));
    assert!(lines[1].ends_with('1'));
    assert_eq!((d.source_bytes(), d.history_status(), c.cursor()), before);
}

#[test]
fn invalid_history_commands_do_not_change_source_cursor_registers_or_undo() {
    let mut d = Document::new("abcdef");
    let mut c = CommandInterpreter::new();
    keys(&mut c, &mut d, "x");
    for input in [
        ":earlier nope",
        ":earlier 2x",
        ":later!",
        ":%earlier",
        ":undolist arg",
        ":later 18446744073709551615d",
    ] {
        let before = (
            d.source_bytes(),
            d.history_status(),
            c.cursor(),
            c.register('"').cloned(),
        );
        let output = ex(&mut c, &mut d, input);
        assert!(!matches!(output.status, CommandStatus::Complete), "{input}");
        assert_eq!(
            (
                d.source_bytes(),
                d.history_status(),
                c.cursor(),
                c.register('"').cloned()
            ),
            before
        );
    }
}
