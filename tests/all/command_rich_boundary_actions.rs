use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::{Document, Encoding, Format};

fn open(format: Format, source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn keys(commands: &mut CommandInterpreter, document: &mut Document, keys: &str) {
    for key in keys.chars() {
        commands.handle(document, InputEvent::key(key)).unwrap();
    }
}

fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) {
    commands
        .handle(document, InputEvent::Key(key))
        .unwrap_or_else(|error| {
            panic!(
                "{key:?}: {error:?}, {:?}",
                String::from_utf8_lossy(&document.source_bytes())
            )
        });
}

fn assert_reopen_and_undo(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    source: &str,
) {
    let reopened = open(
        document.format(),
        &String::from_utf8(document.source_bytes()).unwrap(),
    );
    assert_eq!(document.text(), reopened.text());
    key(commands, document, Key::Escape);
    keys(commands, document, "u");
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn backspace_recognizes_nested_list_and_code_bodies_after_arrow_navigation() {
    let mut failures = Vec::new();
    for (format, source, line) in [
        (Format::Markdown, "A\n\n- parent\n  - BC\n- D", 3),
        (Format::Markdown, "A\n\n> ```\n> BC\n> D\n> ```\n\nE", 2),
    ] {
        let mut doc = open(format, source);
        let text = doc.text().to_owned();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut doc, &format!("{line}G0i"));
        let at = commands.cursor();
        key(&mut commands, &mut doc, Key::Right);
        key(&mut commands, &mut doc, Key::Left);
        assert_eq!(commands.cursor(), at);
        // A nested item loses one list level per Backspace before its list
        // treatment is removed; no press deletes text.
        let style = |doc: &Document| {
            doc.projection()
                .blocks()
                .iter()
                .find(|block| block.range.start <= at && at <= block.range.end)
                .unwrap()
                .style
                .0
                .clone()
        };
        for _ in 0..4 {
            key(&mut commands, &mut doc, Key::Backspace);
            assert_eq!(doc.text(), text, "{source}");
            assert_eq!(commands.cursor(), at, "{source}");
            if style(&doc) == "Paragraph" {
                break;
            }
        }
        if style(&doc) != "Paragraph" {
            failures.push(format!(
                "{source}: style={} source={:?}",
                style(&doc),
                String::from_utf8_lossy(&doc.source_bytes())
            ));
        }
        assert_reopen_and_undo(&mut commands, &mut doc, source);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn backspace_at_inline_scope_start_deletes_the_preceding_grapheme() {
    for (format, source, expected) in [

        (Format::Markdown, "A**B**C", "BC"),
    ] {
        let mut doc = open(format, source);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut doc, "0li");
        key(&mut commands, &mut doc, Key::Backspace);
        assert_eq!(doc.text(), expected, "{source}");
        assert_eq!(commands.cursor(), 0);
        assert_reopen_and_undo(&mut commands, &mut doc, source);
    }
}

#[test]
fn forward_delete_merges_nested_blocks_using_the_preceding_paragraph() {
    for (format, source) in [
        (Format::Markdown, "A\n\n- BC\n  - D\n- E"),
        (Format::Markdown, "A\n\n> ```\n> BC\n> D\n> ```\n\nE"),
    ] {
        let mut doc = open(format, source);
        let mut expected = doc.text().to_owned();
        expected.remove(1);
        let preceding_style = doc.projection().blocks()[0].style.clone();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut doc, "1G$a");
        assert_eq!(commands.cursor(), 1);
        key(&mut commands, &mut doc, Key::Delete);
        assert_eq!(doc.text(), expected, "{source}");
        assert_eq!(
            doc.projection().blocks()[0].style,
            preceding_style,
            "{source}"
        );
        assert_reopen_and_undo(&mut commands, &mut doc, source);
    }
}

#[test]
fn deletion_at_document_outer_boundaries_is_a_byte_exact_noop() {
    for (format, source) in [
        (Format::Markdown, "**A**\n\n"),

    ] {
        let mut doc = open(format, source);
        let revision = doc.revision();
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut doc, "ggi");
        key(&mut commands, &mut doc, Key::Backspace);
        assert_eq!(commands.cursor(), 0);
        key(&mut commands, &mut doc, Key::Escape);
        keys(&mut commands, &mut doc, "GA");
        assert_eq!(commands.cursor(), doc.text().len());
        key(&mut commands, &mut doc, Key::Delete);
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert_eq!(doc.revision(), revision);
    }
}
