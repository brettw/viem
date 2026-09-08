use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{
    CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key, RegisterKind,
    RegisterReadError, RegisterWriteError,
};
use viem_core::document::{
    ArtifactBinding, ArtifactIdentity, ArtifactPath, Document, Encoding, FileFormat, Format,
    LineEndingOpenPolicy, LoadedArtifact,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn key(character: char) -> InputEvent {
    InputEvent::Key(Key::Char(character))
}

fn keys(commands: &mut CommandInterpreter, document: &mut Document, input: &str) -> CommandOutput {
    let mut output = commands
        .handle(document, key(input.chars().next().unwrap()))
        .unwrap();
    for character in input.chars().skip(1) {
        output = commands.handle(document, key(character)).unwrap();
    }
    output
}

fn snapshot(target: ClipboardTarget, text: &str) -> ClipboardSnapshot {
    ClipboardSnapshot::new(
        target,
        ClipboardGeneration(1),
        ClipboardContent::from_plain_text(text),
    )
}

#[test]
fn plus_and_star_read_distinct_per_turn_clipboard_snapshots() {
    for (target, register, pasted) in [
        (ClipboardTarget::Clipboard, '+', "plus"),
        (ClipboardTarget::Primary, '*', "primary"),
    ] {
        let mut document = Document::new("ab");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, &format!("\"{register}"));
        let context = ClipboardCommandContext::new().with_read(snapshot(target, pasted));
        let output = commands
            .handle_with_clipboard_context(&mut document, key('p'), &context)
            .unwrap();

        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), format!("a{pasted}b"));
        assert!(commands.register(register).is_none());
    }
}

#[test]
fn clipboard_yank_emits_owned_portable_write_after_commit() {
    let mut document = Document::new("one\ntwo");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "\"+y");
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    let output = commands
        .handle_with_clipboard_context(&mut document, key('y'), &context)
        .unwrap();

    assert_eq!(output.status, CommandStatus::Complete);
    assert_eq!(output.clipboard_writes.len(), 1);
    let request = &output.clipboard_writes[0];
    assert_eq!(request.target(), ClipboardTarget::Clipboard);
    assert_eq!(request.content().plain_text(), "one\n");
    let portable = request.content().portable_register().unwrap();
    assert_eq!(portable.kind, RegisterKind::Linewise);
    assert_eq!(portable.hard_break_offsets(), &[3]);
    assert_eq!(commands.register('"').unwrap(), portable);
    assert!(commands.register('0').is_none());
    assert!(commands.register('+').is_none());
}

#[test]
fn unavailable_and_read_only_register_events_are_atomic_and_clear_pending_state() {
    let mut document = Document::new("one\ntwo");
    let mut commands = CommandInterpreter::new();
    let before_revision = document.revision();
    keys(&mut commands, &mut document, "\"+d");
    let unavailable = commands.handle(&mut document, key('d')).unwrap();
    assert_eq!(
        unavailable.status,
        CommandStatus::RegisterWriteError(RegisterWriteError::ClipboardUnavailable(
            ClipboardTarget::Clipboard
        ))
    );
    assert_eq!(document.text(), "one\ntwo");
    assert_eq!(document.revision(), before_revision);
    assert!(unavailable.clipboard_writes.is_empty());

    // The failed doubled operator and its register selection do not leak into
    // the next event.
    keys(&mut commands, &mut document, "dd");
    assert_eq!(document.text(), "two");

    for register in ['.', '%'] {
        let mut document = Document::new("one\ntwo");
        let mut commands = CommandInterpreter::new();
        let revision = document.revision();
        keys(&mut commands, &mut document, &format!("\"{register}d"));
        let output = commands.handle(&mut document, key('d')).unwrap();
        assert_eq!(
            output.status,
            CommandStatus::RegisterWriteError(RegisterWriteError::ReadOnly(register))
        );
        assert_eq!(document.text(), "one\ntwo");
        assert_eq!(document.revision(), revision);
    }

    let mut document = Document::new("ab");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "yw\"+");
    let output = commands.handle(&mut document, key('p')).unwrap();
    assert_eq!(
        output.status,
        CommandStatus::RegisterReadError(RegisterReadError::ClipboardUnavailable(
            ClipboardTarget::Clipboard
        ))
    );
    assert_eq!(document.text(), "ab");
    keys(&mut commands, &mut document, "p");
    assert_eq!(document.text(), "aabb");
}

#[test]
fn percent_is_exact_bound_artifact_path_and_rejects_non_utf8() {
    let commands = CommandInterpreter::new();
    let clipboard = ClipboardCommandContext::new();
    let binding = ArtifactBinding::new(
        ArtifactPath::from_utf8("/tmp/notes.md"),
        ArtifactIdentity::new(b"notes".to_vec()),
    );
    let document = Document::from_loaded_artifact(
        LoadedArtifact::new(binding, b"text".to_vec()),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        commands
            .register_with_context(&document, &clipboard, '%')
            .unwrap()
            .unwrap()
            .text,
        "/tmp/notes.md"
    );

    let binding = ArtifactBinding::new(
        ArtifactPath::new(vec![b'/', 0xff]),
        ArtifactIdentity::new(b"opaque".to_vec()),
    );
    let non_utf8 = Document::from_loaded_artifact(
        LoadedArtifact::new(binding, b"text".to_vec()),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        commands.register_with_context(&non_utf8, &clipboard, '%'),
        Err(RegisterReadError::ArtifactPathIsNotUtf8)
    );
    assert_eq!(
        commands.register_with_context(&Document::new(""), &clipboard, '%'),
        Err(RegisterReadError::NoCurrentArtifact)
    );
}

#[test]
fn dot_distinguishes_semantic_enter_from_literal_lf_in_mac_source() {
    let mut semantic = Document::from_bytes_with_file_format(
        Vec::new(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut semantic, key('i')).unwrap();
    commands
        .handle(&mut semantic, InputEvent::Key(Key::Enter))
        .unwrap();
    commands
        .handle(&mut semantic, InputEvent::Key(Key::Escape))
        .unwrap();
    let inserted = commands.register('.').unwrap();
    assert_eq!(inserted.text, "\n");
    assert_eq!(inserted.hard_break_offsets(), &[0]);
    assert_eq!(semantic.source_bytes(), b"\r");

    let mut literal = Document::from_bytes_with_file_format(
        Vec::new(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut literal, key('i')).unwrap();
    commands
        .handle(&mut literal, InputEvent::Text("\n".to_owned()))
        .unwrap();
    commands
        .handle(&mut literal, InputEvent::Key(Key::Escape))
        .unwrap();
    let inserted = commands.register('.').unwrap();
    assert_eq!(inserted.text, "\n");
    assert!(inserted.hard_break_offsets().is_empty());
    assert_eq!(literal.source_bytes(), b"\n");
}

#[test]
fn core_accepts_turn_scoped_clipboard_context_and_returns_write_requests() {
    let mut core = Core::new(Document::new("ab"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
    core.handle(view, CoreEvent::Input(key('"'))).unwrap();
    core.handle(view, CoreEvent::Input(key('+'))).unwrap();
    let read = ClipboardCommandContext::new().with_read(snapshot(ClipboardTarget::Clipboard, "X"));
    core.handle(
        view,
        CoreEvent::InputWithClipboard {
            input: key('p'),
            clipboard: read,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "aXb");

    let mut core = Core::new(Document::new("one\ntwo"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
    for character in ['"', '+', 'y'] {
        core.handle(view, CoreEvent::Input(key(character))).unwrap();
    }
    let outcome = core
        .handle(
            view,
            CoreEvent::InputWithClipboard {
                input: key('y'),
                clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
            },
        )
        .unwrap();
    let command = outcome.command.unwrap();
    assert_eq!(command.clipboard_writes.len(), 1);
    assert_eq!(command.clipboard_writes[0].content().plain_text(), "one\n");
}
