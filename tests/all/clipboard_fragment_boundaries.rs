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
        // Text in an empty document gets an explicit `p` owner; the copied
        // scope keeps its exact unclosed spelling inside it.
        let expected = [b"<p>".as_slice(), source, b"</p>"].concat();
        assert_eq!(target.source_bytes(), expected);
        assert_undo_redo(&mut target, &mut commands, b"", &expected);
    }
}

#[test]
fn rich_clipboard_replacement_preserves_hidden_syntax_between_selected_runs() {
    let content = copy_word(b"<b>word</b>", true, FontSlant::Upright);
    let source = b"<!--outside--><p>A<b>bc</b><!--keep--><i>de</i>Z</p><!--tail-->";
    let mut document = html(source);
    let mut commands = CommandInterpreter::new();
    let before_a = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    let before_z = DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        content,
    ));
    for key in "lv3l\"+p".chars() {
        event(&mut commands, &mut document, &context, InputEvent::key(key));
    }
    assert_eq!(document.text(), "AwordZ");
    let changed = document.source_bytes();
    let serialized = std::str::from_utf8(&changed).unwrap();
    assert!(serialized.starts_with("<!--outside--><p>A<b>"));
    // The replacement consumes `<i>de</i>`, so its emptied delimiters are
    // removed; the comment between the selected runs keeps its exact bytes.
    assert!(serialized.contains("</b><!--keep-->Z</p><!--tail-->"), "{serialized}");
    for snapshot in [&document, &html(&changed)] {
        for at in 1..5 {
            let style =
                DocumentLayoutStyles::character_at(snapshot.projection(), at, false).unwrap();
            assert!(style.bold);
            assert_eq!(style.slant, FontSlant::Upright);
        }
        assert_eq!(
            DocumentLayoutStyles::character_at(snapshot.projection(), 0, false).unwrap(),
            before_a
        );
        assert_eq!(
            DocumentLayoutStyles::character_at(snapshot.projection(), 5, false).unwrap(),
            before_z
        );
    }
    assert_undo_redo(&mut document, &mut commands, source, &changed);
}

#[test]
fn rich_clipboard_at_entity_interior_preserves_unselected_entity_text() {
    let content = copy_word(b"<b>word</b>", true, FontSlant::Upright);
    let source = b"<p><b>&fjlig;</b><!--keep-->Z</p>";
    let mut document = html(source);
    let mut commands = paste_word(&mut document, content, 'a');
    assert_eq!(document.text(), "fwordjZ");
    let changed = document.source_bytes();
    assert!(std::str::from_utf8(&changed)
        .unwrap()
        .ends_with("</b><!--keep-->Z</p>"));
    assert_undo_redo(&mut document, &mut commands, source, &changed);
}

#[test]
fn entity_suffix_keeps_its_original_style_after_cross_run_text_replacement() {
    let source = b"<p><i>A</i><b>&fjlig;</b><!--keep-->Z</p>";
    let mut document = html(source);
    let original_j = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    document.replace(0..2, "X").unwrap();
    assert_eq!(document.text(), "XjZ");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap(),
        original_j
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn entity_suffix_keeps_its_original_style_after_cross_run_rich_paste() {
    let content = copy_word(b"<i>word</i>", false, FontSlant::Italic);
    let source = b"<p><i>A</i><b>&fjlig;</b><!--keep-->Z</p>";
    let mut document = html(source);
    let original_j = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    let mut commands = CommandInterpreter::new();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        content,
    ));
    for key in "vl\"+p".chars() {
        event(&mut commands, &mut document, &context, InputEvent::key(key));
    }
    assert_eq!(document.text(), "wordjZ");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 4, false).unwrap(),
        original_j
    );
    let changed = document.source_bytes();
    assert!(std::str::from_utf8(&changed)
        .unwrap()
        .ends_with("</b><!--keep-->Z</p>"));
    assert_undo_redo(&mut document, &mut commands, source, &changed);
}

#[test]
fn rich_paste_uses_shared_paragraph_merge_and_overlapping_object_source_plans() {
    let copied_style =
        DocumentLayoutStyles::character_at(html(b"<b>word</b>").projection(), 0, false).unwrap();
    for (source, selection, expected) in [
        ("<h2>A</h2><!--keep--><p>B</p><p>C</p>", "vjl", "word\nC"),
        (
            "<table><b>foster</b><tr><td>keep</td></tr></table><!--tail-->",
            "6lv",
            "fosterword",
        ),
    ] {
        let content = copy_word(b"<b>word</b>", true, FontSlant::Upright);
        let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            ClipboardTarget::Clipboard,
            ClipboardGeneration(1),
            content,
        ));
        let mut document = html(source.as_bytes());
        let mut commands = CommandInterpreter::new();
        for key in format!("{selection}\"+p").chars() {
            event(&mut commands, &mut document, &context, InputEvent::key(key));
        }
        assert_eq!(document.text(), expected, "{source}");
        let after = document.source_bytes();
        let reopened = html(&after);
        for snapshot in [&document, &reopened] {
            assert_eq!(snapshot.text(), expected);
            let start = expected.find("word").unwrap();
            for at in start..start + 4 {
                assert_eq!(
                    DocumentLayoutStyles::character_at(snapshot.projection(), at, false).unwrap(),
                    copied_style
                );
            }
        }
        let serialized = std::str::from_utf8(&after).unwrap();
        if source.starts_with("<h2>") {
            assert!(serialized.starts_with("<h2><b>"));
            assert!(serialized.ends_with("<!--keep--></h2><p>C</p>"));
            assert_eq!(document.projection().blocks()[0].style.0, "Heading2");
        } else {
            assert!(serialized.starts_with("<b>foster"));
            assert!(serialized.ends_with("</b><!--tail-->"));
        }
        assert_undo_redo(&mut document, &mut commands, source.as_bytes(), &after);
    }
}

#[test]
fn direct_character_formatting_at_a_paragraph_end_keeps_following_unstyled_text() {
    use viem_core::document::{ModelRequest, StyleProperty, StylePropertyValue};
    let source = b"<p>A</p><!--keep--><p>B</p>";
    let mut document = html(source);
    let original_b = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    document
        .apply_model_request(ModelRequest::SetDirectCharacterProperties {
            document: document.id(),
            revision: document.revision(),
            range: 0..1,
            values: vec![(
                StyleProperty::CharacterSize,
                StylePropertyValue::Float(20.0),
            )],
        })
        .unwrap();
    let after = document.source_bytes();
    for snapshot in [&document, &html(&after)] {
        assert_eq!(snapshot.text(), "A\nB");
        assert_eq!(
            DocumentLayoutStyles::character_at(snapshot.projection(), 0, false)
                .unwrap()
                .size,
            20.0
        );
        assert_eq!(
            DocumentLayoutStyles::character_at(snapshot.projection(), 2, false).unwrap(),
            original_b
        );
    }
    assert!(std::str::from_utf8(&after)
        .unwrap()
        .ends_with("</p><!--keep--><p>B</p>"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn rich_paste_replaces_an_entire_recovered_anonymous_paragraph() {
    let copied_style =
        DocumentLayoutStyles::character_at(html(b"<i>word</i>").projection(), 0, false).unwrap();
    for prefix in ["", "<!doctype html>"] {
        let source = format!("{prefix}<p>A</p><!--before--><table><b>B</b><tr><td>keep</td></tr></table><!--after--><p>C</p>");
        let mut document = html(source.as_bytes());
        assert_eq!(document.text(), "A\nB\u{fffc}\nC");
        let before_a = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
        let before_c = DocumentLayoutStyles::character_at(document.projection(), 7, false).unwrap();
        let content = copy_word(b"<i>word</i>", false, FontSlant::Italic);
        let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            ClipboardTarget::Clipboard,
            ClipboardGeneration(1),
            content,
        ));
        let mut commands = CommandInterpreter::new();
        for key in "jv$\"+p".chars() {
            event(&mut commands, &mut document, &context, InputEvent::key(key));
        }
        let after = document.source_bytes();
        for snapshot in [&document, &html(&after)] {
            assert_eq!(snapshot.text(), "A\nword\nC");
            assert_eq!(
                DocumentLayoutStyles::character_at(snapshot.projection(), 0, false).unwrap(),
                before_a
            );
            assert_eq!(
                DocumentLayoutStyles::character_at(snapshot.projection(), 7, false).unwrap(),
                before_c
            );
            for at in 2..6 {
                assert_eq!(
                    DocumentLayoutStyles::character_at(snapshot.projection(), at, false).unwrap(),
                    copied_style
                );
            }
        }
        let serialized = std::str::from_utf8(&after).unwrap();
        assert!(serialized.starts_with(&format!("{prefix}<p>A</p><!--before-->")));
        assert!(serialized.ends_with("<!--after--><p>C</p>"));
        assert_undo_redo(&mut document, &mut commands, source.as_bytes(), &after);
    }
}
