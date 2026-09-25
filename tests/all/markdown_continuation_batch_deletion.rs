use viem_core::document::{Document, Encoding, FileFormat, Format, TextEdit};

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn adjacent_deletions_empty_a_continuation_like_one_edit() {
    for source in [
        "- a\n\n  bc\n- d",
        "3. a\n\n   **bc**\n4. d",
        "> - a\n> \n>   bc\n> - d",
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let ending = match file_format {
                    FileFormat::Unix => "\n",
                    FileFormat::Dos => "\r\n",
                    FileFormat::Mac => "\r",
                };
                let source = source.replace('\n', ending);
                let original = encode(&source, encoding);
                let open = || {
                    Document::from_bytes_with_file_format(
                        original.clone(),
                        encoding,
                        Format::Markdown,
                        file_format,
                    )
                    .unwrap()
                };
                let mut single = open();
                let at = single.text().find("bc").unwrap();
                single.delete(at..at + 2).unwrap();
                for reversed in [false, true] {
                    let mut batch = open();
                    let mut edits = vec![
                        TextEdit::new(at..at + 1, ""),
                        TextEdit::new(at + 1..at + 2, ""),
                    ];
                    if reversed {
                        edits.reverse();
                    }
                    batch.apply_edits(edits).unwrap_or_else(|error| {
                        panic!("{source:?} {encoding:?} {file_format:?}: {error:?}")
                    });
                    assert_eq!(batch.text(), "a\n\nd");
                    assert_eq!(batch.source_bytes(), single.source_bytes());
                    let reopened = Document::from_bytes_with_file_format(
                        batch.source_bytes(),
                        encoding,
                        Format::Markdown,
                        file_format,
                    )
                    .unwrap();
                    assert_eq!(reopened.text(), batch.text());
                    assert!(batch.undo());
                    assert_eq!(batch.source_bytes(), original);
                }
            }
        }
    }
}
