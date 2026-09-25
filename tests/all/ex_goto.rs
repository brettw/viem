use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::SourceToTextError;
use viem_core::{Document, Encoding, Format};

fn enter_ex(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    command: &str,
) -> viem_core::command::CommandOutput {
    commands
        .handle(document, InputEvent::Key(Key::Char(':')))
        .unwrap();
    for character in command.chars() {
        commands
            .handle(document, InputEvent::Key(Key::Char(character)))
            .unwrap();
    }
    commands
        .handle(document, InputEvent::Key(Key::Enter))
        .unwrap()
}

#[test]
fn goto_reaches_the_controller_cursor_without_mutating_the_document() {
    let mut document = Document::new("abcdef");
    let revision = document.revision();
    let source = document.source_bytes();
    let mut commands = CommandInterpreter::new();

    let output = enter_ex(&mut commands, &mut document, "goto 5");
    assert_eq!(output.status, CommandStatus::Complete);
    assert_eq!(commands.cursor(), 4);
    assert_eq!(document.revision(), revision);
    assert_eq!(document.source_bytes(), source);

    let output = enter_ex(&mut commands, &mut document, "goto 0");
    assert_eq!(output.status, CommandStatus::Complete);
    // Ex resolves EOF exactly; Normal mode then associates the block cursor
    // with the final grapheme rather than retaining an insertion boundary.
    assert_eq!(commands.cursor(), 5);
    assert_eq!(document.revision(), revision);
}

#[test]
fn goto_surfaces_hidden_source_interiors_as_typed_command_errors() {
    let mut document =
        Document::from_bytes(b"**x**".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let revision = document.revision();
    let source = document.source_bytes();
    let mut commands = CommandInterpreter::new();

    let output = enter_ex(&mut commands, &mut document, "goto 2");
    assert!(matches!(
        output.status,
        CommandStatus::ExError(viem_core::command::ExCommandError::Execute(
            viem_core::command::ex_execute::ExExecuteError::SourceMapping(
                SourceToTextError::InteriorHiddenSyntax { .. }
            )
        ))
    ));
    assert_eq!(commands.cursor(), 0);
    assert_eq!(document.revision(), revision);
    assert_eq!(document.source_bytes(), source);
}
