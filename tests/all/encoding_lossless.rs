use viem_core::document::{
    DecodingDiagnosticKind, Document, DocumentError, Encoding, FileFormat, Format,
};

const REPLACEMENT: &str = "\u{fffd}";

#[test]
fn malformed_utf8_with_bom_is_visible_diagnosed_and_byte_exact() {
    let bytes = vec![0xef, 0xbb, 0xbf, b'a', 0xf0, 0x9f, b'b', 0x80, b'\n'];
    let document = Document::from_bytes_with_file_format(
        bytes.clone(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();

    assert_eq!(document.text(), format!("a{REPLACEMENT}b{REPLACEMENT}\n"));
    assert_eq!(document.source_bytes(), bytes);
    assert!(document.has_bom());
    assert_eq!(document.decoding_diagnostics().len(), 2);
    assert_eq!(
        document.decoding_diagnostics()[0].revision,
        document.revision()
    );
    assert_eq!(
        document.decoding_diagnostics()[0].kind,
        DecodingDiagnosticKind::InvalidUtf8Sequence
    );
    assert_eq!(document.decoding_diagnostics()[0].source_range, 4..6);
    assert_eq!(document.decoding_diagnostics()[0].formatted_range, 1..4);
    assert_eq!(document.decoding_diagnostics()[1].source_range, 7..8);
    assert_eq!(document.decoding_diagnostics()[1].formatted_range, 5..8);
    assert!(document
        .projection()
        .provenance()
        .iter()
        .any(|span| span.formatted == (1..4) && span.source == (4..6)));
    assert_eq!(
        document.projection().decoding_diagnostics(),
        document.decoding_diagnostics()
    );
}

#[test]
fn malformed_utf16_in_both_byte_orders_is_visible_and_byte_exact() {
    let cases = [
        (
            Encoding::Utf16Le,
            vec![
                0xff, 0xfe, // BOM
                0x41, 0x00, // A
                0x00, 0xd8, // unpaired high surrogate
                0x42, 0x00, // B
                0x00, 0xdc, // unpaired low surrogate
                0xab, // truncated code unit
            ],
        ),
        (
            Encoding::Utf16Be,
            vec![
                0xfe, 0xff, // BOM
                0x00, 0x41, // A
                0xd8, 0x00, // unpaired high surrogate
                0x00, 0x42, // B
                0xdc, 0x00, // unpaired low surrogate
                0xab, // truncated code unit
            ],
        ),
    ];

    for (encoding, bytes) in cases {
        let document = Document::from_bytes_with_file_format(
            bytes.clone(),
            encoding,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap();
        assert_eq!(
            document.text(),
            format!("A{REPLACEMENT}B{REPLACEMENT}{REPLACEMENT}")
        );
        assert_eq!(document.source_bytes(), bytes);
        assert_eq!(
            document
                .decoding_diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.kind, diagnostic.source_range.clone()))
                .collect::<Vec<_>>(),
            [
                (DecodingDiagnosticKind::UnpairedUtf16HighSurrogate, 4..6,),
                (DecodingDiagnosticKind::UnpairedUtf16LowSurrogate, 8..10,),
                (DecodingDiagnosticKind::TruncatedUtf16CodeUnit, 10..11),
            ]
        );
    }
}

#[test]
fn plain_text_edits_copy_opaque_bytes_until_the_item_is_explicitly_replaced() {
    let original = vec![b'A', 0xff, b'B'];
    let mut document = Document::from_bytes_with_file_format(
        original.clone(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();

    document.insert(0, "<").unwrap();
    document.replace(5..6, "C").unwrap();
    assert_eq!(document.text(), format!("<A{REPLACEMENT}C"));
    assert_eq!(document.source_bytes(), vec![b'<', b'A', 0xff, b'C']);
    assert_eq!(document.decoding_diagnostics()[0].source_range, 2..3);
    assert_eq!(document.decoding_diagnostics()[0].formatted_range, 2..5);

    document.replace(2..5, "x").unwrap();
    assert_eq!(document.text(), "<AxC");
    assert_eq!(document.source_bytes(), b"<AxC");
    assert!(document.decoding_diagnostics().is_empty());

    assert!(document.undo());
    assert_eq!(document.source_bytes(), vec![b'<', b'A', 0xff, b'C']);
    document.delete(2..5).unwrap();
    assert_eq!(document.source_bytes(), b"<AC");
    assert!(document.decoding_diagnostics().is_empty());
}

#[test]
fn replacing_opaque_item_with_literal_replacement_character_is_not_a_no_op() {
    let mut document = Document::from_bytes_with_file_format(
        vec![b'a', 0xff, b'b'],
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();
    let before = document.revision();

    document.replace(1..4, REPLACEMENT).unwrap();

    assert_eq!(document.text(), format!("a{REPLACEMENT}b"));
    assert_eq!(document.source_bytes(), "a\u{fffd}b".as_bytes());
    assert!(document.decoding_diagnostics().is_empty());
    assert_ne!(document.revision(), before);
}

#[test]
fn utf16_edits_around_and_over_opaque_items_are_local() {
    let original = vec![
        0xff, 0xfe, // BOM
        0x41, 0x00, // A
        0x00, 0xdc, // unpaired low surrogate
        0x42, 0x00, // B
    ];
    let mut document = Document::from_bytes_with_file_format(
        original,
        Encoding::Utf16Le,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();

    document.insert(0, "X").unwrap();
    document.replace(5..6, "C").unwrap();
    assert_eq!(
        document.source_bytes(),
        [0xff, 0xfe, 0x58, 0x00, 0x41, 0x00, 0x00, 0xdc, 0x43, 0x00]
    );
    assert_eq!(document.decoding_diagnostics()[0].source_range, 6..8);

    document.replace(2..5, "Y").unwrap();
    assert_eq!(document.text(), "XAYC");
    assert_eq!(
        document.source_bytes(),
        [0xff, 0xfe, 0x58, 0x00, 0x41, 0x00, 0x59, 0x00, 0x43, 0x00]
    );
    assert!(document.decoding_diagnostics().is_empty());
}

#[test]
fn markdown_preserves_opaque_bytes_and_maps_diagnostic_to_visible_content() {
    let source = vec![
        b'#', b' ', b'A', b' ', b'*', b'*', 0xff, b' ', b'B', b'*', b'*', b'\n',
    ];
    let mut document = Document::from_bytes_with_file_format(
        source.clone(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Unix,
    )
    .unwrap();

    assert_eq!(document.text(), format!("A {REPLACEMENT} B"));
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.decoding_diagnostics()[0].source_range, 6..7);
    assert_eq!(document.decoding_diagnostics()[0].formatted_range, 2..5);

    document.replace(6..7, "C").unwrap();
    assert_eq!(
        document.source_bytes(),
        vec![b'#', b' ', b'A', b' ', b'*', b'*', 0xff, b' ', b'C', b'*', b'*', b'\n',]
    );
    assert_eq!(document.decoding_diagnostics()[0].source_range, 6..7);

    document.replace(2..5, "x").unwrap();
    assert_eq!(document.text(), "A x C");
    assert_eq!(document.source_bytes(), b"# A **x C**\n");
    assert!(document.decoding_diagnostics().is_empty());
}

#[test]
fn markdown_with_malformed_utf16_keeps_syntax_bom_and_history_losslessly() {
    let cases = [
        (
            Encoding::Utf16Le,
            vec![
                0xff, 0xfe, // BOM
                0x23, 0x00, // #
                0x20, 0x00, // space
                0x2a, 0x00, 0x2a, 0x00, // **
                0x00, 0xdc, // unpaired low surrogate
                0x2a, 0x00, 0x2a, 0x00, // **
                0x0a, 0x00, // newline
            ],
            vec![
                0xff, 0xfe, 0x23, 0x00, 0x20, 0x00, 0x2a, 0x00, 0x2a, 0x00, 0x78, 0x00, 0x2a, 0x00,
                0x2a, 0x00, 0x0a, 0x00,
            ],
        ),
        (
            Encoding::Utf16Be,
            vec![
                0xfe, 0xff, // BOM
                0x00, 0x23, // #
                0x00, 0x20, // space
                0x00, 0x2a, 0x00, 0x2a, // **
                0xdc, 0x00, // unpaired low surrogate
                0x00, 0x2a, 0x00, 0x2a, // **
                0x00, 0x0a, // newline
            ],
            vec![
                0xfe, 0xff, 0x00, 0x23, 0x00, 0x20, 0x00, 0x2a, 0x00, 0x2a, 0x00, 0x78, 0x00, 0x2a,
                0x00, 0x2a, 0x00, 0x0a,
            ],
        ),
    ];

    for (encoding, source, edited) in cases {
        let mut document = Document::from_bytes_with_file_format(
            source.clone(),
            encoding,
            Format::Markdown,
            FileFormat::Unix,
        )
        .unwrap();

        assert_eq!(document.text(), format!("{REPLACEMENT}"));
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.decoding_diagnostics()[0].source_range, 10..12);
        assert_eq!(document.decoding_diagnostics()[0].formatted_range, 0..3);

        document.replace(0..3, "x").unwrap();
        assert_eq!(document.text(), "x");
        assert_eq!(document.source_bytes(), edited);
        assert!(document.decoding_diagnostics().is_empty());

        assert!(document.undo());
        assert_eq!(document.text(), format!("{REPLACEMENT}"));
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.decoding_diagnostics()[0].source_range, 10..12);

        assert!(document.redo());
        assert_eq!(document.text(), "x");
        assert_eq!(document.source_bytes(), edited);
        assert!(document.decoding_diagnostics().is_empty());
    }
}

#[test]
fn attempts_to_split_an_opaque_item_are_structured_and_non_destructive() {
    let mut document = Document::from_bytes_with_file_format(
        vec![b'a', 0xff, b'b'],
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Unix,
    )
    .unwrap();
    let bytes = document.source_bytes();
    let revision = document.revision();

    assert_eq!(
        document.replace(2..4, "x"),
        Err(DocumentError::NotGraphemeBoundary(2))
    );
    assert_eq!(document.source_bytes(), bytes);
    assert_eq!(document.revision(), revision);
    assert_eq!(document.decoding_diagnostics().len(), 1);
}
