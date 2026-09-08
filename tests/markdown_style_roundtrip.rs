use viem_core::document::{
    Document, DocumentError, Encoding, Format, ModelChangeKind, ModelRequest, SemanticInlineStyle,
    StyleApplication,
};

fn encode_source(encoding: Encoding, text: &str) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|ch| u8::try_from(u32::from(ch)).expect("fixture is Latin-1 representable"))
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xff, 0xfe];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = vec![0xfe, 0xff];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            bytes
        }
    }
}

fn encode_fragment(encoding: Encoding, text: &str) -> Vec<u8> {
    let mut bytes = encode_source(encoding, text);
    if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
        bytes.drain(..2);
    }
    bytes
}

#[test]
fn clearing_markdown_styles_uses_exact_source_delimiters_in_every_encoding() {
    let cases = [
        ("**", SemanticInlineStyle::Strong),
        ("__", SemanticInlineStyle::Strong),
        ("*", SemanticInlineStyle::Emphasis),
        ("_", SemanticInlineStyle::Emphasis),
        ("`", SemanticInlineStyle::Code),
    ];
    let encodings = [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ];

    for encoding in encodings {
        for (marker, style) in cases {
            let source_text = format!("prefix {marker}Hé{marker} suffix");
            let cleared_text = "prefix Hé suffix";
            let original = encode_source(encoding, &source_text);
            let expected = encode_source(encoding, cleared_text);
            let mut document =
                Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
            let formatted_before = document.text().to_owned();
            let range_start = document.text().find("Hé").unwrap();
            let range = range_start..range_start + "Hé".len();
            assert!(document.projection().style_spans().iter().any(|span| {
                span.range == range && span.application == StyleApplication::Semantic(style)
            }));

            let prepared = document
                .prepare_model_request(ModelRequest::SetSemanticStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: range.clone(),
                    style,
                    enabled: false,
                })
                .unwrap();

            assert_eq!(document.source_bytes(), original);
            assert_eq!(prepared.summary().kind(), ModelChangeKind::SemanticStyle);
            assert_eq!(prepared.summary().source_patches().len(), 2);
            let encoded_marker = encode_fragment(encoding, marker);
            for patch in prepared.summary().source_patches() {
                assert!(patch.replacement().is_empty());
                assert_eq!(&original[patch.range()], encoded_marker);
            }

            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), formatted_before);
            assert_eq!(document.source_bytes(), expected);
            assert!(!document.projection().style_spans().iter().any(|span| {
                span.range == range && span.application == StyleApplication::Semantic(style)
            }));

            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert_eq!(document.text(), formatted_before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected);
        }
    }
}

#[test]
fn malformed_styles_reject_and_combined_styles_clear_one_role_atomically() {
    for source in [
        "prefix __Hé** suffix",
        "prefix **Hé__ suffix",
        "prefix `Hé suffix",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before_source = document.source_bytes();
        let before_text = document.text().to_owned();
        let before_revision = document.revision();
        let before_history = document.history_status();
        let start = document.text().find("Hé").unwrap();
        let error = document
            .set_semantic_style(
                start..start + "Hé".len(),
                SemanticInlineStyle::Strong,
                false,
            )
            .unwrap_err();
        assert_eq!(error, DocumentError::UnsupportedFormatting);
        assert_eq!(document.source_bytes(), before_source);
        assert_eq!(document.text(), before_text);
        assert_eq!(document.revision(), before_revision);
        assert_eq!(document.history_status(), before_history);
    }

    let mut document = Document::from_bytes(
        "___Hé___".as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let projected = document
        .projection()
        .style_spans()
        .iter()
        .find(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong))
        .unwrap()
        .range
        .clone();
    let before_source = document.source_bytes();
    let before_text = document.text().to_owned();
    let before_revision = document.revision();
    let before_history = document.history_status();
    let prepared = document
        .prepare_model_request(ModelRequest::SetSemanticStyle {
            document: document.id(),
            revision: document.revision(),
            range: projected,
            style: SemanticInlineStyle::Strong,
            enabled: false,
        })
        .unwrap();
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.text(), before_text);
    assert_eq!(document.revision(), before_revision);
    assert_eq!(document.history_status(), before_history);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), "_Hé_".as_bytes());
    assert_eq!(document.text(), "Hé");
    assert!(document
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Emphasis)));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.text(), before_text);
}

#[test]
fn clearing_only_part_of_a_projected_style_is_rejected_without_mutation() {
    let mut document =
        Document::from_bytes(b"__bold__".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let before_source = document.source_bytes();
    let before_revision = document.revision();
    let before_history = document.history_status();
    let error = document
        .set_semantic_style(0..1, SemanticInlineStyle::Strong, false)
        .unwrap_err();
    assert_eq!(error, DocumentError::UnsupportedFormatting);
    assert_eq!(document.source_bytes(), before_source);
    assert_eq!(document.revision(), before_revision);
    assert_eq!(document.history_status(), before_history);
}
