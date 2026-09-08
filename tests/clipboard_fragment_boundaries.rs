use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, FontSlant, Format};
use viem_core::layout::DocumentLayoutStyles;

fn html(source: &[u8]) -> Document {
    Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn event(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    context: &ClipboardCommandContext,
    input: InputEvent,
) -> CommandOutput {
    let output = commands
        .handle_with_clipboard_context(document, input, context)
        .unwrap();
    assert!(matches!(
        output.status,
        CommandStatus::Complete | CommandStatus::Pending
    ));
    document.text_point(commands.cursor()).unwrap();
    output
}

fn copy_word(source: &[u8], bold: bool, slant: FontSlant) -> ClipboardContent {
    let mut document = html(source);
    let revision = document.revision();
    assert_eq!(document.text(), "word");
    let style = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    assert_eq!((style.bold, style.slant), (bold, slant));
    let mut commands = CommandInterpreter::new();
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    let mut content = None;
    for character in "v$\"+y".chars() {
        let output = event(
            &mut commands,
            &mut document,
            &context,
            InputEvent::key(character),
        );
        if let Some(write) = output.clipboard_writes.first() {
            assert!(content.is_none());
            content = Some(write.content().clone());
        }
    }
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.revision(), revision);
    let content = content.unwrap();
    assert_eq!(content.plain_text(), "word");
    assert!(content
        .portable_register()
        .unwrap()
        .clipboard_fragment()
        .is_some());
    content
}

fn paste_word(
    document: &mut Document,
    content: ClipboardContent,
    enter: char,
) -> CommandInterpreter {
    let mut commands = CommandInterpreter::new();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        content,
    ));
    for input in [
        InputEvent::key(enter),
        InputEvent::Key(Key::Ctrl('r')),
        InputEvent::key('+'),
        InputEvent::Key(Key::Escape),
    ] {
        event(&mut commands, document, &context, input);
    }
    commands
}

fn assert_undo_redo(
    document: &mut Document,
    commands: &mut CommandInterpreter,
    before: &[u8],
    after: &[u8],
) {
    let context = ClipboardCommandContext::new();
    event(commands, document, &context, InputEvent::key('u'));
    assert_eq!(document.source_bytes(), before);
    event(
        commands,
        document,
        &context,
        InputEvent::Key(Key::Ctrl('r')),
    );
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn unclosed_html_clipboard_scope_cannot_restyle_untouched_suffix() {
    for (source, bold, slant) in [
        (b"<b>word".as_slice(), true, FontSlant::Upright),
        (b"<i>word".as_slice(), false, FontSlant::Italic),
        (b"<b>word</b>".as_slice(), true, FontSlant::Upright),
    ] {
        let content = copy_word(source, bold, slant);
        let original = b"<!--keep--><p data-local='yes'>AB</p><!--tail-->";
        let mut target = html(original);
        let original_a = DocumentLayoutStyles::character_at(target.projection(), 0, false).unwrap();
        let original_b = DocumentLayoutStyles::character_at(target.projection(), 1, false).unwrap();
        // Normal A, then `a`, places the Insert caret at A|B.
        let mut commands = paste_word(&mut target, content, 'a');
        assert_eq!(target.text(), "AwordB");
        let after = target.source_bytes();
        assert!(after.starts_with(b"<!--keep--><p data-local='yes'>A"));
        assert!(after.ends_with(b"B</p><!--tail-->"));
        assert_undo_redo(&mut target, &mut commands, original, &after);

        // Check both the live projection after redo and a fresh reopen.
        let reopened = html(&after);
        for document in [&target, &reopened] {
            assert_eq!(document.text(), "AwordB");
            assert_eq!(
                DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap(),
                original_a
            );
            assert_eq!(
                DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap(),
                original_b,
                "copied {source:?} must not change B's style"
            );
            for offset in 1..5 {
                let style =
                    DocumentLayoutStyles::character_at(document.projection(), offset, false)
                        .unwrap();
                assert_eq!((style.bold, style.slant), (bold, slant));
            }
        }
    }
}

#[test]
fn whole_html_clipboard_reconstruction_retains_unclosed_source_exactly() {
    for (source, bold, slant) in [
        (b"<b>word".as_slice(), true, FontSlant::Upright),
        (b"<i>word".as_slice(), false, FontSlant::Italic),
    ] {
        let content = copy_word(source, bold, slant);
        let mut target = html(b"");
        let mut commands = paste_word(&mut target, content, 'i');
        assert_eq!(target.text(), "word");
        assert_eq!(target.source_bytes(), source);
        assert_undo_redo(&mut target, &mut commands, b"", source);
    }
}
