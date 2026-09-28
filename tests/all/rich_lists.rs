use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::{ListStyle};
use viem_core::{Document, Encoding, Format};

fn ordinals(document: &Document) -> Vec<u64> {
    document
        .projection()
        .list_structure()
        .lists
        .into_iter()
        .flat_map(|list| list.items.into_iter().map(|item| item.ordinal))
        .collect()
}

#[test]
fn numbered_list_enter_replays_semantically_for_counted_dot() {
    for (source, format) in [
        ("First", Format::PlainText),
        ("First", Format::Markdown),
        ("First", Format::MarkdownSource),

    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_list_style(0..0, Some(ListStyle::Numbered))
            .unwrap();
        let mut commands = CommandInterpreter::new();
        for event in [
            InputEvent::key('A'),
            InputEvent::Key(Key::Enter),
            InputEvent::text("Next"),
            InputEvent::Key(Key::Escape),
            InputEvent::key('2'),
            InputEvent::key('.'),
        ] {
            commands
                .handle(&mut document, event)
                .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        }
        assert_eq!(
            document.text(),
            if matches!(format, Format::PlainText | Format::MarkdownSource) {
                "1. First\n2. Next\n3. Next\n4. Next"
            } else {
                "First\nNext\nNext\nNext"
            },
            "{format:?}"
        );
        if format != Format::PlainText {
            assert_eq!(ordinals(&document), vec![1, 2, 3, 4], "{format:?}");
        }
        commands
            .handle(&mut document, InputEvent::key('u'))
            .unwrap();
        assert_eq!(
            document.text(),
            if matches!(format, Format::PlainText | Format::MarkdownSource) {
                "1. First\n2. Next"
            } else {
                "First\nNext"
            },
            "{format:?}"
        );
    }
}

#[test]
fn continuation_paragraphs_align_with_list_body_and_preserve_local_layout_queries() {
    use viem_core::layout::DocumentLayoutStyles;
    let mut source = String::new();
    for _ in 0..2000 {
        source.push_str("- First\n\n  Continuation\n\n");
    }
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let continuation = document.projection().blocks()[1].range.clone();
    let styles =
        DocumentLayoutStyles::resolve_region(document.projection(), continuation.clone()).unwrap();
    let paragraph = styles
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.text_range == continuation)
        .unwrap();
    assert_eq!(
        paragraph.leading_indent
            + paragraph
                .containers
                .iter()
                .map(|c| c.style.left())
                .sum::<f32>(),
        32.0
    );
    assert_eq!(paragraph.first_line_indent, 0.0);
    let first_id = document.projection().blocks()[0].id;
    document
        .replace(continuation.start..continuation.start + 1, "K")
        .unwrap();
    let updated = document.projection().blocks()[1].range.clone();
    let styles =
        DocumentLayoutStyles::resolve_region(document.projection(), updated.clone()).unwrap();
    assert_eq!(
        styles
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.text_range == updated)
            .unwrap()
            .first_line_indent,
        0.0
    );
    assert_eq!(document.projection().blocks()[0].id, first_id);
}
