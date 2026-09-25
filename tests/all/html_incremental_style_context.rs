use viem_core::document::{
    Document, Encoding, FontSlant, Format, ModelRequest, ProjectionWorkScope, StyleApplication,
    TextEdit,
};
use viem_core::layout::DocumentLayoutStyles;

#[test]
fn empty_html_soft_break_line_retains_enclosing_character_style() {
    for (source, expected_slant, expected_bold) in [
        ("<p><i>a<br><br>b</i></p>", FontSlant::Italic, false),
        (
            "<p><b><i>a<br><!-- keep --><BR/>b</i></b></p>",
            FontSlant::Italic,
            true,
        ),
        ("<p><i>a</i><br><br>b</p>", FontSlant::Upright, false),
        ("<p>a<br><i><br>b</i></p>", FontSlant::Italic, false),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let encode = |text: &str| match encoding {
                Encoding::Utf8 => text.as_bytes().to_vec(),
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => unreachable!(),
            };
            let original = encode(source);
            let mut doc = Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
            assert_eq!(doc.text(), "a\n\nb");
            doc.insert(2, "é**").unwrap();
            assert_eq!(doc.text(), "a\né**\nb");
            // The source insertion belongs before the second break, after
            // intervening comments/tags; every original byte is preserved.
            let source_at = source.to_ascii_lowercase().rfind("<br").unwrap();
            let mut expected_source = source.to_owned();
            expected_source.insert_str(source_at, "é**");
            assert_eq!(doc.source_bytes(), encode(&expected_source), "{source}");
            let fresh = Document::from_bytes(doc.source_bytes(), encoding, Format::Html).unwrap();
            for offset in [2, 4, 5] {
                let actual =
                    DocumentLayoutStyles::semantic_character_at(doc.projection(), offset, false)
                        .unwrap();
                let expected =
                    DocumentLayoutStyles::semantic_character_at(fresh.projection(), offset, false)
                        .unwrap();
                assert_eq!(expected.slant, expected_slant, "{source}");
                assert_eq!(expected.bold, expected_bold, "{source}");
                assert_eq!(actual, expected, "{source} {encoding:?}");
            }
            assert_eq!(
                doc.projection().provenance(),
                fresh.projection().provenance()
            );
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), original);
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), encode(&expected_source));
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(doc.projection(), 2, false)
                    .unwrap()
                    .slant,
                expected_slant
            );
        }
    }
}

#[test]
fn styled_empty_line_edit_in_large_html_document_stays_regional() {
    let source = "<p><i>a<br><br>b</i></p>\n".repeat(20_000);
    let mut doc = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
    let unaffected_block = doc.projection().blocks()[15_000].id;
    let at = doc.projection().hard_line_range(30_001).unwrap().start;
    for text in ["x", "y"] {
        let prepared = doc
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: doc.id(),
                revision: doc.revision(),
                edits: vec![TextEdit::new(at..at, text)],
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert!(work.decoded_source_bytes() < 128, "{work:?}");
        assert_eq!(work.full_text_bytes_materialized(), 0);
        doc.commit_model_transaction(prepared).unwrap();
        assert_eq!(doc.projection().blocks()[15_000].id, unaffected_block);
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), at, false)
                .unwrap()
                .slant,
            FontSlant::Italic
        );
        assert!(doc.undo());
    }
}

#[test]
fn malformed_outer_close_does_not_extend_pre_wrap_into_inserted_text() {
    let source = r#"<b>x<span style="white-space: pre-wrap">a<i>c</b> }"#;
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |text: &str| match encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        };
        let original = encode(source);
        let mut incremental =
            Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
        assert_eq!(incremental.text(), "xac }");

        incremental.replace(3..4, "\\").unwrap();
        assert_eq!(incremental.text(), "xac\\}");
        let fresh =
            Document::from_bytes(incremental.source_bytes(), encoding, Format::Html).unwrap();
        let preserved_ranges = |document: &Document| {
            document
                .projection()
                .style_spans()
                .iter()
                .filter(|span| span.application == StyleApplication::SourcePreservedWhitespace)
                .map(|span| span.range.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(preserved_ranges(&incremental), preserved_ranges(&fresh));
        assert!(!preserved_ranges(&incremental)
            .iter()
            .any(|range| range.contains(&3)));
        assert_eq!(
            incremental.projection().provenance(),
            fresh.projection().provenance()
        );
        assert!(incremental.undo());
        assert_eq!(incremental.source_bytes(), original);
    }
}
