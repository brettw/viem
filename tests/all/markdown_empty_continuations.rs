use viem_core::document::{Document, Encoding, FileFormat, Format, ModelRequest};

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn deleting_continuation_body_keeps_both_paragraph_boundaries_after_reopening() {
    for source in [
        "- a\n\n  b\n- c",
        "- a\n\n  **b**\n- c",
        "- a\n\n  b\n\n  c\n- d",
        "> - a\n> \n>   b\n> - c",
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for (file_format, ending) in [
                (FileFormat::Unix, "\n"),
                (FileFormat::Dos, "\r\n"),
                (FileFormat::Mac, "\r"),
            ] {
                let original = encode(&source.replace('\n', ending), encoding);
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                let mut expected = document.text().to_owned();
                let at = expected.find('b').unwrap();
                expected.remove(at);
                document.replace(at..at + 1, "").unwrap_or_else(|error| {
                    panic!("{source:?} {encoding:?} {file_format:?}: {error:?}")
                });
                assert_eq!(document.text(), expected);
                let after = document.source_bytes();
                let reopened = Document::from_bytes_with_file_format(
                    after.clone(),
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                assert_eq!(
                    reopened.text(),
                    expected,
                    "{source:?} {encoding:?} {file_format:?}"
                );
                assert_eq!(
                    reopened
                        .projection()
                        .blocks()
                        .iter()
                        .map(|b| (&b.range, &b.kind, &b.style))
                        .collect::<Vec<_>>(),
                    document
                        .projection()
                        .blocks()
                        .iter()
                        .map(|b| (&b.range, &b.kind, &b.style))
                        .collect::<Vec<_>>()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), after);
            }
        }
    }
}

#[test]
fn deleting_first_paragraph_promotes_surviving_continuation_into_the_same_list_item() {
    for (source, expected_source) in [
        ("- a\n\n  b\n- c", "- b\n- c"),
        ("4. a\n\n   b\n5. c", "4. b\n5. c"),
        ("- **a**\n\n  *b*\n- c", "- *b*\n- c"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        document
            .apply_model_request(ModelRequest::DeleteLines {
                document: document.id(),
                revision: document.revision(),
                range: 0..2,
            })
            .unwrap();
        assert_eq!(document.text(), "b\nc");
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(reopened.text(), document.text());
        assert_eq!(
            reopened.projection().list_structure().lists[0].items.len(),
            2
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
