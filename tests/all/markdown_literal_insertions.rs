use viem_core::document::{
    FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
};
use viem_core::{Document, Encoding, Format};

#[test]
fn inserting_delimiters_preserves_literal_block_prefixes_and_existing_spaces() {
    for source in ["<p>A</p><p></p>", "- ```\n  A\n  ```", "é\nA", "👩‍💻\nA"] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=original.text().len() {
            if original.text_point(at).is_err() {
                continue;
            }
            for input in ["\n", "`"] {
                for payload in [false, true] {
                    let mut document = Document::from_bytes(
                        source.as_bytes().to_vec(),
                        Encoding::Utf8,
                        Format::Markdown,
                    )
                    .unwrap();
                    let result = if payload {
                        let text = FormattedTextPayload::new(
                            &document.hard_line_snapshot(),
                            input,
                            if input == "\n" { vec![0] } else { vec![] },
                        )
                        .unwrap();
                        let request = FormattedPayloadEditRequest::new(
                            document.id(),
                            document.revision(),
                            vec![FormattedPayloadEdit::new(at..at, text)],
                        );
                        document
                            .apply_formatted_payload_request(request)
                            .map(|_| ())
                    } else {
                        document.insert(at, input).map_err(Into::into)
                    };
                    result.unwrap_or_else(|error| {
                        panic!("{source:?} at{at} input{input:?} payload{payload}: {error:?}")
                    });
                    let mut expected = original.text().to_owned();
                    expected.insert_str(at, input);
                    assert_eq!(document.text(), expected);
                    let reopened = Document::from_bytes(
                        document.source_bytes(),
                        Encoding::Utf8,
                        Format::Markdown,
                    )
                    .unwrap();
                    assert_eq!(reopened.text(), expected);
                    assert!(document.undo());
                    assert_eq!(document.source_bytes(), source.as_bytes());
                }
            }
        }
    }
}

#[test]
fn ordinary_punctuation_keeps_its_original_source_spelling() {
    for (source, at, input, expected_source) in [
        ("one,two", 3, "\n", "one\n\n,two"),
        ("one,two", 3, ",", "one,,two"),
        ("one.two", 4, ".", "one..two"),
        ("x1. Item", 1, "\n", "x\n\n1\\. Item"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        document.insert(at, input).unwrap();
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut expected = source.to_owned();
        expected.insert_str(at, input);
        assert_eq!(reopened.text(), expected);
    }
}
