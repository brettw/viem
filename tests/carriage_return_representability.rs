use viem_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, FormattedPayloadEdit,
    FormattedPayloadEditRequest, FormattedTextPayload, ModelTransactionError, TextEdit,
};

fn bytes(text: &str, encoding: Encoding) -> Vec<u8> {
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

const ENCODINGS: [Encoding; 4] = [
    Encoding::Utf8,
    Encoding::Latin1,
    Encoding::Utf16Le,
    Encoding::Utf16Be,
];

fn open(source: &str, encoding: Encoding, format: Format, endings: FileFormat) -> Document {
    Document::from_bytes_with_file_format(bytes(source, encoding), encoding, format, endings)
        .unwrap()
}

#[test]
fn mac_literal_cr_is_a_precise_atomic_policy_in_literal_source_adapters() {
    for encoding in ENCODINGS {
        for format in [
            Format::PlainText,
            Format::Markdown,
            Format::MarkdownSource,
            Format::HtmlSource,
        ] {
            for replacement in ["\r", "x\ry", "\r\n"] {
                let mut document = open("AB", encoding, format, FileFormat::Mac);
                let source = document.source_bytes();
                let revision = document.revision();
                let history = document.history_status();
                assert_eq!(
                    document.replace(1..1, replacement),
                    Err(DocumentError::UnrepresentableFormattedCharacter {
                        format,
                        character: '\r',
                    }),
                    "{format:?}/{encoding:?}: {replacement:?}"
                );
                assert_eq!(document.source_bytes(), source);
                assert_eq!(document.revision(), revision);
                assert_eq!(document.history_status(), history);
                assert_eq!(document.text(), "AB");
                document.insert(1, "x").unwrap();
                assert_eq!(document.text(), "AxB");
            }
        }
    }
}

#[test]
fn mac_literal_cr_rejects_an_entire_batch_or_structured_payload_before_commit() {
    for format in [
        Format::PlainText,
        Format::Markdown,
        Format::MarkdownSource,
        Format::HtmlSource,
    ] {
        let mut document = open("ABCD", Encoding::Utf8, format, FileFormat::Mac);
        let expected = DocumentError::UnrepresentableFormattedCharacter {
            format,
            character: '\r',
        };
        assert_eq!(
            document.apply_edits(vec![TextEdit::new(0..1, "x"), TextEdit::new(3..4, "\r")]),
            Err(expected.clone())
        );
        assert_eq!(document.text(), "ABCD");
        let payload =
            FormattedTextPayload::new(&document.hard_line_snapshot(), "\r", Vec::new()).unwrap();
        let request = FormattedPayloadEditRequest::new(
            document.id(),
            document.revision(),
            vec![FormattedPayloadEdit::new(1..2, payload)],
        );
        assert_eq!(
            document
                .prepare_formatted_payload_request(request)
                .unwrap_err(),
            ModelTransactionError::Document(expected)
        );
        assert_eq!(document.source_bytes(), b"ABCD");
        assert!(!document.history_status().can_undo);
    }
}

#[test]
fn rtf_literal_cr_survives_reopen_in_every_line_ending_mode_and_encoding() {
    for encoding in ENCODINGS {
        for endings in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let format = Format::Rtf;
            // RTF owns its byte grammar and code-page/Unicode decoding;
            // the generic encoding hint does not encode its syntax.
            let original = bytes("{\\rtf1 AB}", Encoding::Latin1);
            let mut document =
                Document::from_bytes_with_file_format(original.clone(), encoding, format, endings)
                    .unwrap();
            assert_eq!(document.text(), "AB");
            document.insert(1, "\r").unwrap();
            assert_eq!(
                document.text(),
                "A\rB",
                "{format:?}/{encoding:?}/{endings:?}"
            );
            let reopened = Document::from_bytes_with_file_format(
                document.source_bytes(),
                encoding,
                format,
                endings,
            )
            .unwrap();
            assert_eq!(reopened.text(), "A\rB");
            assert_eq!(reopened.line_count(), 1);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn html_exact_literal_cr_is_rejected_atomically_in_every_mode_and_encoding() {
    // HTML preprocessing turns raw CR into LF; CSS treats escaped CR as a
    // space, even under pre. Neither spelling represents exact U+000D text.
    for encoding in ENCODINGS {
        for endings in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            for source in ["<p>AB</p>", "<pre>AB</pre>"] {
                let mut document = open(source, encoding, Format::Html, endings);
                let original = document.source_bytes();
                let revision = document.revision();
                let history = document.history_status();
                assert_eq!(
                    document.insert(1, "\r"),
                    Err(DocumentError::UnrepresentableFormattedCharacter { format: Format::Html, character: '\r' })
                );
                assert_eq!(document.source_bytes(), original);
                assert_eq!(document.text(), "AB");
                assert_eq!(document.revision(), revision);
                assert_eq!(document.history_status(), history);
                document.insert(1, "x").unwrap();
                assert_eq!(document.text(), "AxB");
            }
        }
    }
}

#[test]
fn unix_and_dos_keep_representable_literal_cr_in_literal_source_adapters() {
    for encoding in ENCODINGS {
        for endings in [FileFormat::Unix, FileFormat::Dos] {
            for format in [
                Format::PlainText,
                Format::Markdown,
                Format::MarkdownSource,
                Format::HtmlSource,
            ] {
                let mut document = open("AB", encoding, format, endings);
                document.insert(1, "\r").unwrap();
                assert_eq!(
                    document.text(),
                    "A\rB",
                    "{format:?}/{encoding:?}/{endings:?}"
                );
                assert_eq!(document.source_bytes(), bytes("A\rB", encoding));
                let reopened = Document::from_bytes_with_file_format(
                    document.source_bytes(),
                    encoding,
                    format,
                    endings,
                )
                .unwrap();
                assert_eq!(reopened.text(), "A\rB");
            }
        }
    }
}

#[test]
fn mac_markdown_code_contexts_do_not_offer_a_literal_cr_escape() {
    for source in ["`AB`", "```\rAB\r```", "    AB"] {
        let mut document = open(source, Encoding::Utf8, Format::Markdown, FileFormat::Mac);
        let at = document.text().find('B').unwrap();
        assert_eq!(
            document.insert(at, "\r"),
            Err(DocumentError::UnrepresentableFormattedCharacter {
                format: Format::Markdown,
                character: '\r',
            })
        );
        assert_eq!(document.source_bytes(), bytes(source, Encoding::Utf8));
    }
}
