use viem_core::document::{
    Document, Encoding, Format, FormattedPayloadEdit, FormattedPayloadEditRequest,
    FormattedTextPayload, ModelRequest, TextEdit,
};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|character| u8::try_from(character as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xff, 0xfe];
            bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = vec![0xfe, 0xff];
            bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            bytes
        }
    }
}

#[test]
fn adjacent_style_replacement_is_local_and_lossless_in_every_encoding() {
    const SOURCE: &str = "prefix **ab**_cd_ suffix";
    const EXPECTED: &str = "prefix **aX**_Yd_ suffix";
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let original = encoded(SOURCE, encoding);
        let mut document =
            Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
        let start = document.text().find("bc").unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(start..start + 2, "XY")],
            })
            .unwrap();

        let patches = prepared.summary().source_patches();
        assert_eq!(patches.len(), 2);
        let bytes_per_ascii = if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
            2
        } else {
            1
        };
        let bom = usize::from(matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be)) * 2;
        let b = bom + ("prefix **a".len() * bytes_per_ascii);
        let c = bom + ("prefix **ab**_".len() * bytes_per_ascii);
        assert_eq!(patches[0].range(), b..b + bytes_per_ascii);
        assert_eq!(patches[1].range(), c..c + bytes_per_ascii);
        assert_eq!(patches[0].replacement(), encoded_fragment("X", encoding));
        assert_eq!(patches[1].replacement(), encoded_fragment("Y", encoding));

        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), "prefix aXYd suffix");
        assert_eq!(document.source_bytes(), encoded(EXPECTED, encoding));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn expansion_contraction_and_deletion_preserve_adjacent_delimiters() {
    for (replacement, expected_text, expected_source) in [
        ("XYZ", "aXYZd tail", "**aX**_YZd_ tail"),
        ("Q", "aQd tail", "**aQ**_d_ tail"),
        ("", "ad tail", "**a**_d_ tail"),
    ] {
        let mut document = Document::from_bytes(
            b"**ab**_cd_ tail".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        document.replace(1..3, replacement).unwrap();
        assert_eq!(document.text(), expected_text);
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"**ab**_cd_ tail");
    }
}

#[test]
fn replacement_distribution_never_splits_extended_graphemes() {
    let mut document = Document::from_bytes(
        "**aé**_😀d_ tail".as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let start = document.text().find('é').unwrap();
    let end = start + "é😀".len();
    document.replace(start..end, "X👩‍💻Z").unwrap();
    assert_eq!(document.text(), "aX👩‍💻Zd tail");
    assert_eq!(document.source_bytes(), "**aX**_👩‍💻Zd_ tail".as_bytes());
}

#[test]
fn insertion_at_an_adjacent_style_boundary_uses_the_downstream_style() {
    let mut document =
        Document::from_bytes(b"**ab**_cd_".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    document.insert(2, "Z").unwrap();
    assert_eq!(document.text(), "abZcd");
    assert_eq!(document.source_bytes(), b"**ab**_Zcd_");
}

#[test]
fn structured_payload_replacement_uses_the_same_relational_reverse_rule() {
    let mut document = Document::from_bytes(
        b"**ab**_cd_ tail".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let payload =
        FormattedTextPayload::new(&document.hard_line_snapshot(), "XYZ", Vec::new()).unwrap();
    let prepared = document
        .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
            document.id(),
            document.revision(),
            vec![FormattedPayloadEdit::new(1..3, payload)],
        ))
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 2);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "aXYZd tail");
    assert_eq!(document.source_bytes(), b"**aX**_YZd_ tail");
}

fn encoded_fragment(text: &str, encoding: Encoding) -> Vec<u8> {
    let mut bytes = encoded(text, encoding);
    if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
        bytes.drain(..2);
    }
    bytes
}
