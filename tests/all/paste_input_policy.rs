use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
use viem_core::document::{Document, Encoding, FileFormat, Format};

fn context(content: ClipboardContent) -> ClipboardCommandContext {
    ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        content,
    ))
}

fn key(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    clipboard: &ClipboardCommandContext,
    key: Key,
) {
    let output = commands
        .handle_with_clipboard_context(document, InputEvent::Key(key), clipboard)
        .unwrap();
    assert!(
        matches!(
            output.status,
            CommandStatus::Complete | CommandStatus::Pending
        ),
        "{key:?}: {:?}",
        output.status
    );
}

fn paste(document: &mut Document, clipboard: &ClipboardCommandContext) -> CommandInterpreter {
    let mut commands = CommandInterpreter::new();
    for event in [Key::Char('a'), Key::Ctrl('r'), Key::Char('+'), Key::Escape] {
        key(&mut commands, document, clipboard, event);
    }
    commands
}

#[test]
fn plain_clipboard_line_endings_become_semantic_breaks_in_each_format() {
    for (format, source) in [
        (Format::PlainText, "AB"),
        (Format::Code, "AB"),
        (Format::MarkdownSource, "AB"),
        (Format::Markdown, "AB"),
        (Format::Rtf, r"{\rtf1 AB}"),
    ] {
        for ending in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let mut document = Document::from_bytes_with_file_format(
                source.as_bytes().to_vec(),
                Encoding::Utf8,
                format,
                ending,
            )
            .unwrap();
            let before = document.source_bytes();
            let clipboard = context(ClipboardContent::from_plain_text("x\r\ny\rz\nq"));
            let mut commands = paste(&mut document, &clipboard);
            assert_eq!(document.text(), "Ax\ny\nz\nqB", "{format:?}/{ending:?}");
            let reopened = Document::from_bytes_with_file_format(
                document.source_bytes(),
                Encoding::Utf8,
                format,
                ending,
            )
            .unwrap();
            assert_eq!(reopened.text(), document.text());
            assert_eq!(
                clipboard
                    .read(ClipboardTarget::Clipboard)
                    .unwrap()
                    .content()
                    .plain_text(),
                "x\r\ny\rz\nq"
            );
            key(&mut commands, &mut document, &clipboard, Key::Char('u'));
            assert_eq!(document.source_bytes(), before);
        }
    }
}

#[test]
fn paste_does_not_change_representable_null_or_private_literal_carriage_return() {
    for content in [
        ClipboardContent::from_plain_text("x\0y"),
        ClipboardContent::from_register(RegisterValue::characterwise("x\ry\0z")),
    ] {
        let inserted = content.plain_text().to_owned();
        let clipboard = context(content);
        let mut document =
            Document::from_bytes(b"AB".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        paste(&mut document, &clipboard);
        assert_eq!(document.text(), format!("A{inserted}B"));
    }
}

#[test]
fn command_prompt_register_insertion_keeps_exact_clipboard_text() {
    let mut document =
        Document::from_bytes(b"A".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut commands = CommandInterpreter::new();
    let clipboard = context(ClipboardContent::from_plain_text("x\0y"));
    for event in [Key::Char(':'), Key::Ctrl('r'), Key::Char('+')] {
        key(&mut commands, &mut document, &clipboard, event);
    }
    assert_eq!(commands.command_line(), Some("x\0y"));
    assert_eq!(document.source_bytes(), b"A");
}
