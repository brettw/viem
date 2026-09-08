use evim_core::document::{Document, Encoding, Format, ModelRequest, TextEdit};

#[test]
fn replacing_collapsed_space_patches_each_contributor_and_preserves_opaque_gaps() {
    for (gap, retained) in [
        ("\n\0\n", "\0"),
        ("\n\0\t\0\n", "\0\0"),
        ("\r\n\0\r\n", "\0"),
        ("\n<!--keep-->\n", "<!--keep-->"),
        ("\n<em></em>\n", "<em></em>"),
        ("&#32;\0&#10;", "\0"),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let encode = |text: &str| match encoding {
                Encoding::Utf8 => text.as_bytes().to_vec(),
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                _ => unreachable!(),
            };
            let decode = |bytes: &[u8]| match encoding {
                Encoding::Utf8 => String::from_utf8(bytes.to_vec()).unwrap(),
                Encoding::Utf16Le => String::from_utf16(
                    &bytes
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
                _ => unreachable!(),
            };
            for replacement in ["x", ""] {
                let original = encode(&format!("<p>a{gap}b</p>"));
                let expected = encode(&format!("<p>a{replacement}{retained}b</p>"));
                let mut document =
                    Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
                assert_eq!(document.text(), "a b");
                let prepared = document
                    .prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(1..2, replacement)],
                    })
                    .unwrap_or_else(|error| panic!("{gap:?} {encoding:?}: {error:?}"));
                // Every patch replaces a whitespace contributor; neither the
                // NULs nor markup separating those contributors is rewritten.
                for patch in prepared.summary().source_patches() {
                    let old = decode(&original[patch.range()]);
                    assert!(old.chars().all(char::is_whitespace) || old.starts_with("&#"));
                }
                document.commit_model_transaction(prepared).unwrap();
                assert_eq!(document.source_bytes(), expected, "{gap:?} {encoding:?}");
                assert_eq!(document.text(), format!("a{replacement}b"));
                let reopened =
                    Document::from_bytes(expected.clone(), encoding, Format::Html).unwrap();
                assert_eq!(reopened.text(), document.text());
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), expected);
            }
        }
    }
}

#[test]
fn collapsed_tail_merges_with_selected_following_content_without_consuming_nul() {
    let original = b"<p>a\n\0\nb c</p>";
    let mut document =
        Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "a b c");
    document.replace(1..3, "x").unwrap();
    assert_eq!(document.text(), "ax c");
    assert_eq!(document.source_bytes(), b"<p>ax\0 c</p>");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}
