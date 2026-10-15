use viem_core::document::{Document, Encoding, FileFormat, Format};

#[test]
fn authored_marker_on_markdown_source_continuation_stays_literal() {
    for (encoding, endings, delimiter) in [
        (Encoding::Utf8, FileFormat::Unix, "\n"),
        (Encoding::Utf16Le, FileFormat::Mac, "\r"),
        (Encoding::Utf16Be, FileFormat::Dos, "\r\n"),
    ] {
        for replacement in ["- ", "+ ", "1. ", "2) "] {
            let source = format!("1. first{delimiter}   continuation{delimiter}2. second");
            let bytes = encode(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                bytes.clone(),
                encoding,
                Format::Markdown,
                endings,
            )
            .unwrap();
            document.replace(6..8, replacement).unwrap();
            let fresh = Document::from_bytes_with_file_format(
                document.source_bytes(),
                encoding,
                Format::Markdown,
                endings,
            )
            .unwrap();
            assert_eq!(
                document.text(),
                format!("first {replacement}ntinuation\nsecond")
            );
            assert_eq!(document.text(), fresh.text());
            assert_eq!(
                document.projection().provenance(),
                fresh.projection().provenance()
            );
            assert_eq!(document.projection().blocks().len(), 2);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn counted_open_lines_in_markdown_lists_keep_structural_ownership_and_repeat() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    for source in [
        "- first\n  continuation\n- second",
        "3. first\n   continuation\n1. second",
        "- \n- second",
    ] {
        for above in [false, true] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let original = document.source_bytes();
            let first = document.text().split('\n').next().unwrap().to_owned();
            let mut commands = CommandInterpreter::new();
            for key in [Key::Char('2'), Key::Char(if above { 'O' } else { 'o' })] {
                commands
                    .handle(&mut document, InputEvent::Key(key))
                    .unwrap_or_else(|error| panic!("{source:?} above={above}: {error:?}"));
            }
            commands
                .handle(&mut document, InputEvent::text("Next"))
                .unwrap();
            commands
                .handle(&mut document, InputEvent::Key(Key::Escape))
                .unwrap();
            let expected = if above {
                format!("Next\nNext\n{first}\nsecond")
            } else {
                format!("{first}\nNext\nNext\nsecond")
            };
            assert_eq!(document.text(), expected, "{source:?} above={above}");
            assert!(document.projection().blocks().iter().all(|block| matches!(
                block.kind,
                viem_core::document::BlockKind::ListItem { .. }
            )));
            let once = document.source_bytes();
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('.')))
                .unwrap();
            assert_eq!(document.text().matches("Next").count(), 4);
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('u')))
                .unwrap();
            assert_eq!(document.source_bytes(), once);
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('u')))
                .unwrap();
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn contextual_markdown_sort_owners_return_precise_atomic_policy() {
    use viem_core::document::{DocumentError, ModelRequest, ModelTransactionError};
    for source in [
        "\tcode\n\nprose",
        "```\ncode\n```\n\nprose",
        "- b\n- a\n\nprose",
        "body\\\nnext\n\nprose",
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.source_bytes();
        let count = document.projection().hard_line_count();
        let revision = document.revision();
        let error = document
            .prepare_model_request(ModelRequest::ReorderHardLines {
                document: document.id(),
                revision,
                source_lines: 0..count,
                order: (0..count).rev().collect(),
            })
            .unwrap_err();
        assert!(
            matches!(
                error,
                ModelTransactionError::Document(DocumentError::UnsupportedFormatting)
            ),
            "{source:?}: {error:?}; blocks={:?}; text={:?}",
            document.projection().blocks(),
            document.text()
        );
        assert_eq!(document.source_bytes(), before);
        assert_eq!(document.revision(), revision);
    }
}

#[test]
fn backspace_new_empty_markdown_item_becomes_an_ordinary_empty_paragraph() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    for source in [
        "- first\n- second",
        "5. first\n6. second",
        "- first\n- **second**",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let original = document.source_bytes();
        let before = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        for key in [Key::Char('A'), Key::Enter, Key::Backspace, Key::Escape] {
            commands
                .handle(&mut document, InputEvent::Key(key))
                .unwrap();
        }
        assert_eq!(document.text(), before.replacen('\n', "\n\n", 1));
        let blocks = document.projection().blocks();
        assert_eq!(blocks[1].style.0, "Paragraph");
        assert!(blocks[1].range.is_empty());
        let after = document.source_bytes();
        let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        assert_eq!(reopened.projection().blocks()[1].style.0, "Paragraph");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), after);
    }
}

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|ch| u8::try_from(ch as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

