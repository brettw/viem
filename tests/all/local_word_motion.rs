//! Read-only word navigation must not leave a full text cache in a snapshot.
use viem_core::command::{CommandInterpreter, InputEvent, Key, RegisterValue};
use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
use viem_core::document::{measure_document_work, Document, Encoding, Format};

#[test]
fn local_word_motions_do_not_materialize_unrelated_text() {
    for format in [Format::PlainText, Format::Markdown] {
        for (keys, start, expected) in [("w", 0, 4), ("W", 0, 4), ("e", 0, 2), ("E", 0, 2),
            ("b", 8, 4), ("B", 8, 4), ("ge", 8, 6), ("gE", 8, 6), ("3w", 0, 11)] {
            let source = "one two ++ three\n\nfour five\n\n".repeat(4096);
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let mut command = CommandInterpreter::new();
            assert!(command.set_cursor(&document, start));
            let (_, work) = measure_document_work(|| {
                for key in keys.chars() { command.handle(&mut document, InputEvent::key(key)).unwrap(); }
            });
            assert_eq!(command.cursor(), expected, "{format:?} {keys}");
            assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{format:?} {keys}: {work:?}");
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(!document.history_status().can_undo);
        }
    }
}

#[test]
fn word_operators_preserve_source_and_undo() {
    for (keys, expected) in [("dw", "two ++ three"), ("d2w", "++ three"), ("de", " two ++ three"),
        ("cw", " two ++ three")] {
        let mut document = Document::from_bytes(b"one two ++ three".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        let mut command = CommandInterpreter::new();
        for key in keys.chars() { command.handle(&mut document, InputEvent::key(key)).unwrap(); }
        command.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes(), "{keys}");
        command.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), b"one two ++ three");
    }
}

#[test]
fn rich_paste_does_not_materialize_destination_text() {
    let copied = Document::from_bytes(b"**bold**".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let register = RegisterValue::from_clipboard_fragment(copied.clipboard_fragment(0..4).unwrap()).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_register(register)));
    let source = "plain paragraph\n\n".repeat(1024);
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut command = CommandInterpreter::new();
    let (result, work) = measure_document_work(|| command.handle_with_clipboard_context(
        &mut document, InputEvent::Key(Key::PasteClipboard), &context));
    result.unwrap();
    assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{work:?}");
    let saved = document.source_bytes();
    assert!(saved.ends_with(source.as_bytes()));
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), reopened.text());
    command.handle(&mut document, InputEvent::key('u')).unwrap();
    assert_eq!(document.source_bytes(), source.as_bytes());
    command.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
    assert_eq!(document.source_bytes(), saved);
}

#[test]
fn word_motions_preserve_unicode_clusters_empty_lines_and_atomic_images() {
    use unicode_segmentation::UnicodeSegmentation;
    for (format, source) in [
        (Format::PlainText, "a\u{301}b café 漢字 👩🏽‍💻!! end\n\nnext"),
        (Format::Markdown, "a\u{301}b café 漢字 ![](missing.png)!! end\n\nnext"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = document.text().to_owned();
        let words = [text.find("a\u{301}b").unwrap(), text.find("café").unwrap(), text.find("漢字").unwrap(),
            text.find(if format == Format::Markdown { "\u{fffc}" } else { "👩" }).unwrap(), text.find("end").unwrap()];
        for pair in words.windows(2) {
            let previous_end = text[..pair[1]].trim_end().grapheme_indices(true).last().unwrap().0;
            for (keys, at, expected) in [("w", pair[0], pair[1]), ("W", pair[0], pair[1]),
                ("b", pair[1], pair[0]), ("B", pair[1], pair[0]),
                ("ge", pair[1], previous_end), ("gE", pair[1], previous_end)] {
                let mut command = CommandInterpreter::new();
                assert!(command.set_cursor(&document, at));
                for key in keys.chars() { command.handle(&mut document, InputEvent::key(key)).unwrap(); }
                assert_eq!(command.cursor(), expected, "{format:?} {keys} at {at}");
                assert!(document.hard_line_snapshot().is_grapheme_boundary(command.cursor()));
            }
        }
        if format == Format::PlainText {
            let next = text.find("next").unwrap();
            let blank = next - 1;
            for (keys, expected) in [("b", blank), ("2b", *words.last().unwrap()), ("ge", text.find("end").unwrap() + 2)] {
                let mut command = CommandInterpreter::new();
                assert!(command.set_cursor(&document, next));
                for key in keys.chars() { command.handle(&mut document, InputEvent::key(key)).unwrap(); }
                assert_eq!(command.cursor(), expected, "{keys}");
            }
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.history_status().can_undo);
    }
}

#[test]
fn word_cursors_resolve_clusters_across_leaf_edges() {
    // Long regional-indicator and combining sequences force the Unicode
    // cursor to request context from earlier leaves. A forward string oracle
    // identifies the last complete grapheme without assuming a leaf boundary.
    use unicode_segmentation::UnicodeSegmentation;
    for word in [
        format!("{}a{}", "é".repeat(3000), "\u{301}".repeat(5000)),
        "🇺".repeat(5001),
        "👩🏽‍💻".repeat(1200),
        "漢字".repeat(4000),
    ] {
        let source = format!("start {word} tail");
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        let end = 6 + word.len();
        let last = 6 + word.grapheme_indices(true).last().unwrap().0;
        for (keys, at, expected) in [("b", last, 6), ("ge", last, 4), ("ge", end + 1, last),
            ("2b", end + 1, 0), ("2ge", end + 1, 4), ("w", 6, end + 1), ("e", 6, last),
            ("2w", 0, end + 1), ("2e", 0, last)] {
            let mut command = CommandInterpreter::new();
            assert!(command.set_cursor(&document, at));
            let (_, work) = measure_document_work(|| {
                for key in keys.chars() { command.handle(&mut document, InputEvent::key(key)).unwrap(); }
            });
            assert_eq!(command.cursor(), expected, "{keys} at {at}; {} bytes", word.len());
            assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{work:?}");
        }
    }
}

#[test]
fn word_cursors_keep_semantic_image_and_hard_break_edges() {
    use viem_core::document::FileFormat;
    for (source, target, expected) in [
        ("left ![](missing.png)\u{301} right", "\u{fffc}", "\u{301}"),
        ("left \u{600}![](missing.png) right", "\u{600}", "\u{fffc}"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find(target).unwrap();
        let expected = document.text().find(expected).unwrap();
        let mut command = CommandInterpreter::new();
        assert!(command.set_cursor(&document, at));
        command.handle(&mut document, InputEvent::key('e')).unwrap();
        assert_eq!(command.cursor(), expected, "{source:?}");
        assert!(document.hard_line_snapshot().is_grapheme_boundary(command.cursor()));
    }
    let mut document = Document::from_bytes_with_file_format(b"left\r\n\nright".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix).unwrap();
    let mut command = CommandInterpreter::new();
    command.handle(&mut document, InputEvent::key('w')).unwrap();
    assert_eq!(command.cursor(), 6, "the second semantic LF is an empty line");
    command.handle(&mut document, InputEvent::key('w')).unwrap();
    assert_eq!(command.cursor(), 7);
    command.handle(&mut document, InputEvent::key('b')).unwrap();
    assert_eq!(command.cursor(), 6);
}
