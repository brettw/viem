use viem_core::document::{Document, Encoding, FileFormat, Format, ModelRequest, TextEdit};
use viem_core::layout::DocumentLayoutStyles;

fn ending(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Unix => "\n",
        FileFormat::Dos => "\r\n",
        FileFormat::Mac => "\r",
    }
}
fn bytes(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}
fn reopened(document: &Document) {
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        document.format(),
        document.file_format(),
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    for (at, _) in document.text().char_indices() {
        assert_eq!(
            DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap(),
            DocumentLayoutStyles::character_at(fresh.projection(), at, false).unwrap(),
            "at {at}"
        );
    }
    let blocks = |doc: &Document| {
        doc.projection()
            .blocks()
            .iter()
            .map(|block| {
                (
                    block.range.clone(),
                    block.kind.clone(),
                    block.style.clone(),
                    block.quote_depth,
                    block.markdown_html,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(document), blocks(&fresh));
}
#[test]
fn reference_boundary_and_definition_edits_preserve_visible_content_and_reopen() {
    let cases = [
        ("[foo]\n\n[foo]: /bar\n", "[foo]", 0),
        ("[foo]\n\n[foo]: /bar\n", "[foo]", 5),
        (
            "[link *foo **bar** `#`*][ref]\n\n[ref]: /uri\n",
            "[ref]:",
            0,
        ),
        ("![foo][bar]\n\n[bar]: /uri 'a title'\n", "[bar]:", 0),
        ("[foo]:\n/url\n'title'\n", "/url", 0),
        ("[Foo\n  bar]: /url\n", "/url", 0),
    ];
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            for (source, needle, delta) in cases {
                let original = bytes(&source.replace('\n', ending(format)), encoding);
                let mut doc = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::Markdown,
                    format,
                )
                .unwrap();
                let at = doc.text().find(needle).unwrap() + delta;
                let mut expected = doc.text().to_owned();
                expected.insert(at, 'x');
                doc.apply_model_request(ModelRequest::ApplyTextEdits {
                    document: doc.id(),
                    revision: doc.revision(),
                    edits: vec![TextEdit::new(at..at, "x")],
                })
                .unwrap_or_else(|e| panic!("{source:?} {encoding:?} {format:?}: {e:?}"));
                assert_eq!(doc.text(), expected);
                reopened(&doc);
                if source.starts_with("![") {
                    let images = doc
                        .projection()
                        .inline_images_for_region(&(0..doc.text().len()));
                    assert_eq!(images.len(), 1);
                    assert_eq!(images[0].destination, "/uri");
                    assert_eq!(images[0].text, "foo");
                }
                let saved = doc.source_bytes();
                assert!(doc.undo());
                assert_eq!(doc.source_bytes(), original);
                assert!(doc.redo());
                assert_eq!(doc.source_bytes(), saved);
                reopened(&doc);
            }
        }
    }
}
#[test]
fn source_enter_reprojects_separator_whitespace_without_losing_source() {
    for (source, needle, delta, expected) in [
        (
            "  \n\naaa\n  \n\n# aaa\n\n  \n",
            "aaa",
            3,
            "  \n\naaa\n\n  \n\n# aaa\n\n  \n",
        ),
        ("- foo\n\n\n  bar\n", "  bar", 1, "- foo\n\n\n \n bar\n"),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let original = bytes(&source.replace('\n', ending(format)), encoding);
                let mut doc = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::MarkdownSource,
                    format,
                )
                .unwrap();
                let at = doc.text().find(needle).unwrap() + delta;
                doc.apply_model_request(ModelRequest::ApplyTextEdits {
                    document: doc.id(),
                    revision: doc.revision(),
                    edits: vec![TextEdit::new(at..at, "\n")],
                })
                .unwrap();
                assert_eq!(
                    doc.source_bytes(),
                    bytes(&expected.replace('\n', ending(format)), encoding)
                );
                reopened(&doc);
                let saved = doc.source_bytes();
                assert!(doc.undo());
                assert_eq!(doc.source_bytes(), original);
                assert!(doc.redo());
                assert_eq!(doc.source_bytes(), saved);
                reopened(&doc);
            }
        }
    }
}

#[test]
fn every_multiline_definition_insertion_keeps_its_visible_rows() {
    for source in ["[foo]:\n/url\n'title'\n", "[Foo\n  bar]: /url\n"] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=original.text().len() {
            let mut doc =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let mut expected = doc.text().to_owned();
            expected.insert(at, 'x');
            doc.apply_model_request(ModelRequest::ApplyTextEdits {
                document: doc.id(),
                revision: doc.revision(),
                edits: vec![TextEdit::new(at..at, "x")],
            })
            .unwrap_or_else(|e| panic!("{source:?} at {at}: {e:?}"));
            assert_eq!(doc.text(), expected);
            reopened(&doc);
            let saved = doc.source_bytes();
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), saved);
            reopened(&doc);
        }
    }
}

#[test]
fn invalidating_a_definition_preserves_unselected_reference_label_bytes() {
    for source in [
        b"[a \x80 **b**][ref]\n\n[ref]: /uri\n".as_slice(),
        b"![a \x80 **b**][ref]\n\n[ref]: /uri 'title'\n".as_slice(),
    ] {
        let mut doc =
            Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = doc.text().find("[ref]:").unwrap();
        let mut expected = doc.text().to_owned();
        expected.insert(at, 'x');
        let committed = doc
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: doc.id(),
                revision: doc.revision(),
                edits: vec![TextEdit::new(at..at, "x")],
            })
            .unwrap();
        // The only consumed original bytes may be an image's reference suffix;
        // all literal label protection is insertion beside existing bytes.
        for patch in committed.summary().source_patches() {
            assert!(patch.range().is_empty() || &source[patch.range()] == b"[ref]");
        }
        assert_eq!(doc.text(), expected);
        reopened(&doc);
        assert_eq!(
            doc.source_bytes()
                .iter()
                .filter(|&&byte| byte == 0x80)
                .count(),
            1
        );
        let saved = doc.source_bytes();
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source);
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), saved);
        reopened(&doc);
    }
}
