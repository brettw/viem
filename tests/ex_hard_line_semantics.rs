use std::collections::BTreeMap;

use evim_core::command::ex::parse_ex;
use evim_core::command::ex_execute::{
    execute_ex, ExExecutionContext, ExExecutionState, ExOutcome, ExRegisterKind, ExRegisterReader,
    ExRegisterValue,
};
use evim_core::command::{CommandInterpreter, InputEvent, Key};
use evim_core::document::{Document, Encoding, FileFormat, Format};

#[derive(Default)]
struct Registers(BTreeMap<char, ExRegisterValue>);

impl ExRegisterReader for Registers {
    fn read(&self, requested: Option<char>) -> Option<ExRegisterValue> {
        self.0.get(&requested.unwrap_or('"')).cloned()
    }
}

fn execute(
    document: &mut Document,
    state: &mut ExExecutionState,
    current_line: usize,
    command: &str,
    registers: &impl ExRegisterReader,
) -> ExOutcome {
    let context = ExExecutionContext {
        current_line,
        ..ExExecutionContext::default()
    };
    execute_ex(
        document,
        state,
        &context,
        &parse_ex(command).unwrap(),
        registers,
    )
    .unwrap()
}

fn command_keys(commands: &mut CommandInterpreter, document: &mut Document, input: &str) {
    for character in input.chars() {
        commands
            .handle(document, InputEvent::Key(Key::Char(character)))
            .unwrap();
    }
    commands
        .handle(document, InputEvent::Key(Key::Enter))
        .unwrap();
}

fn mac_document() -> Document {
    Document::from_bytes_with_file_format(
        b"a\rb\nc".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap()
}

#[test]
fn ex_line_actions_ignore_literal_lf_content() {
    let mut document = mac_document();
    let mut state = ExExecutionState::default();
    assert_eq!(document.line_count(), 2);

    let context = ExExecutionContext {
        current_line: 1,
        ..ExExecutionContext::default()
    };
    let yank = execute_ex(
        &mut document,
        &mut state,
        &context,
        &parse_ex(":yank").unwrap(),
        &(),
    )
    .unwrap();
    assert_eq!(yank.register_effects[0].value.text, "b\nc\n");
    assert_eq!(yank.register_effects[0].value.hard_break_offsets(), &[3]);
    assert_eq!(
        yank.register_effects[0].value.kind,
        ExRegisterKind::Linewise
    );

    execute(&mut document, &mut state, 1, ":s/c/C/", &());
    assert_eq!(document.text(), "a\nb\nC");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.source_bytes(), b"a\rb\nC");

    execute(&mut document, &mut state, 0, ":1,2join!", &());
    assert_eq!(document.text(), "ab\nC");
    assert_eq!(document.line_count(), 1);
    assert_eq!(document.source_bytes(), b"ab\nC");
}

#[test]
fn substitute_preserves_captured_literal_lf_and_marks_replacement_breaks() {
    let mut captured = mac_document();
    execute(
        &mut captured,
        &mut ExExecutionState::default(),
        1,
        ":s/b\\nc/[&]/",
        &(),
    );
    assert_eq!(captured.text(), "a\n[b\nc]");
    assert_eq!(captured.line_count(), 2);
    assert_eq!(captured.source_bytes(), b"a\r[b\nc]");

    let mut inserted = mac_document();
    execute(
        &mut inserted,
        &mut ExExecutionState::default(),
        1,
        ":s/b/B\\rZ/",
        &(),
    );
    assert_eq!(inserted.text(), "a\nB\nZ\nc");
    assert_eq!(inserted.line_count(), 3);
    assert_eq!(inserted.source_bytes(), b"a\rB\rZ\nc");
}

#[test]
fn ex_copy_and_move_preserve_literal_lf_structure() {
    let mut state = ExExecutionState::default();
    let mut copied = mac_document();
    let copied_outcome = execute(&mut copied, &mut state, 1, ":2copy 1", &());
    assert_eq!(copied.text(), "a\nb\nc\nb\nc");
    assert_eq!(copied.line_count(), 3);
    assert_eq!(copied.source_bytes(), b"a\rb\nc\rb\nc");
    assert_eq!(
        copied_outcome
            .model_transaction()
            .unwrap()
            .summary()
            .source_patches()
            .len(),
        1
    );
    assert!(copied.undo());
    assert_eq!(copied.source_bytes(), b"a\rb\nc");

    let mut moved = mac_document();
    let moved_outcome = execute(&mut moved, &mut state, 1, ":2move 0", &());
    assert_eq!(moved.text(), "b\nc\na");
    assert_eq!(moved.line_count(), 2);
    assert_eq!(moved.source_bytes(), b"b\nc\ra");
    assert_eq!(
        moved_outcome
            .model_transaction()
            .unwrap()
            .summary()
            .source_patches()
            .len(),
        2,
        "move is one atomic transaction with local delete/insert patches"
    );
    assert!(moved.undo());
    assert_eq!(moved.source_bytes(), b"a\rb\nc");
}

#[test]
fn ex_move_uses_local_delete_and_insert_patches_in_both_directions() {
    for (command, expected) in [
        (":2move 4", "a\nc\nd\nb"),
        (":2move 3", "a\nc\nb\nd"),
        (":4move 1", "a\nd\nb\nc"),
        (":1move 3", "b\nc\na\nd"),
    ] {
        let mut document = Document::new("a\nb\nc\nd");
        let outcome = execute(
            &mut document,
            &mut ExExecutionState::default(),
            0,
            command,
            &(),
        );
        assert_eq!(document.text(), expected, "{command}");
        assert_eq!(
            outcome
                .model_transaction()
                .unwrap()
                .summary()
                .source_patches()
                .len(),
            2,
            "{command}"
        );
        assert!(document.undo(), "{command}");
        assert_eq!(document.text(), "a\nb\nc\nd", "{command}");
    }
}

#[test]
fn command_line_markdown_transfer_targets_last_copied_line_first_nonblank() {
    let mut document = Document::from_bytes(
        b"# H\n  **one**\n_two_\ntail".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();

    command_keys(&mut commands, &mut document, ":1,2copy 4");

    assert_eq!(
        document.source_bytes(),
        b"# H\n  **one**\n_two_\ntail\n# H\n  **one**"
    );
    assert_eq!(commands.cursor(), "H\n  one\ntwo\ntail\nH\n  ".len());
}

#[test]
fn ex_put_uses_the_target_line_ending_spelling() {
    let mut registers = Registers::default();
    registers
        .0
        .insert('"', ExRegisterValue::linewise("middle\n"));
    let mut document = Document::from_bytes_with_file_format(
        b"first\rlast".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    execute(
        &mut document,
        &mut ExExecutionState::default(),
        0,
        ":1put",
        &registers,
    );
    assert_eq!(document.text(), "first\nmiddle\nlast");
    assert_eq!(document.line_count(), 3);
    assert_eq!(document.source_bytes(), b"first\rmiddle\rlast");
}

#[test]
fn normal_join_removes_only_the_forced_mac_separator() {
    for format in [Format::PlainText, Format::Markdown] {
        for encoding in [Encoding::Utf8, Encoding::Latin1] {
            let mut document = Document::from_bytes_with_file_format(
                b"a\rb\nc".to_vec(),
                encoding,
                format,
                FileFormat::Mac,
            )
            .unwrap();
            let mut commands = CommandInterpreter::new();
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('J')))
                .unwrap();
            assert_eq!(document.text(), "a b\nc", "{format:?} {encoding:?}");
            assert_eq!(document.line_count(), 1, "{format:?} {encoding:?}");
            assert_eq!(
                document.source_bytes(),
                b"a b\nc",
                "{format:?} {encoding:?}"
            );
        }
    }
}
