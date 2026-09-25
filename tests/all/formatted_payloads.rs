use viem_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, FormattedPayloadEdit,
    FormattedPayloadEditRequest, FormattedTextPayload, ModelTransactionError, Revision,
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
fn mac_capture_and_paste_preserve_literal_lf_in_all_text_encodings() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            let source = match format {
                Format::PlainText => "a\rb\nc",
                Format::Markdown => "# a\r# b\nc",
                _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
            };
            let original = encoded(source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                format,
                FileFormat::Mac,
            )
            .unwrap();
            let payload = document.capture_formatted_payload(0..5).unwrap();
            assert_eq!(payload.text(), "a\nb\nc");
            assert_eq!(payload.break_offsets(), &[1]);
            let at = document.text().len();
            let prepared = document
                .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                    document.id(),
                    document.revision(),
                    vec![FormattedPayloadEdit::new(at..at, payload)],
                ))
                .unwrap();
            assert_eq!(prepared.before_revision(), Revision(0));
            assert_eq!(prepared.after_revision(), Revision(1));
            assert_eq!(prepared.summary().formatted_splices().len(), 1);
            assert_eq!(prepared.summary().source_patches().len(), 1);
            let committed = document.commit_model_transaction(prepared).unwrap();

            assert_eq!(document.text(), "a\nb\nca\nb\nc");
            assert_eq!(
                document
                    .hard_line_snapshot()
                    .capture(0..document.text().len())
                    .unwrap()
                    .break_offsets(),
                &[1, 6]
            );
            assert_eq!(
                document.source_bytes(),
                encoded(
                    &format!(
                        "{source}a{}b\nc",
                        if format == Format::Markdown {
                            "\r\r"
                        } else {
                            "\r"
                        }
                    ),
                    encoding
                )
            );
            assert_eq!(committed.text_position_map().source_revision(), Revision(0));
            assert_eq!(committed.text_position_map().target_revision(), Revision(1));

            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn payload_metadata_can_change_with_identical_flat_text_and_is_checked() {
    let mut mac = Document::from_bytes_with_file_format(
        b"a\nb".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    let marked = FormattedTextPayload::new(&mac.hard_line_snapshot(), "\n", vec![0]).unwrap();
    mac.replace_with_formatted_payload(1..2, marked).unwrap();
    assert_eq!(mac.text(), "a\nb");
    assert_eq!(mac.line_count(), 2);
    assert_eq!(mac.source_bytes(), b"a\rb");

    let mut unix = Document::new("x");
    let literal = FormattedTextPayload::new(&unix.hard_line_snapshot(), "\n", Vec::new()).unwrap();
    assert_eq!(
        unix.insert_formatted_payload(1, literal),
        Err(DocumentError::FormattedPayloadCannotReproject)
    );
    assert_eq!(unix.text(), "x");
    assert_eq!(unix.revision(), Revision(0));
}

#[test]
fn payload_preparation_reports_requested_and_current_revisions_in_order() {
    let mut document = Document::new("x");
    let stale_payload =
        FormattedTextPayload::new(&document.hard_line_snapshot(), "y", Vec::new()).unwrap();
    document.insert(0, "!").unwrap();
    let current_revision = document.revision();
    let unchanged_source = document.source_bytes();

    let stale_request = FormattedPayloadEditRequest::new(
        document.id(),
        Revision(0),
        vec![FormattedPayloadEdit::new(0..0, stale_payload.clone())],
    );
    let error = document
        .prepare_formatted_payload_request(stale_request)
        .unwrap_err();
    assert_eq!(
        error,
        ModelTransactionError::StaleRevision {
            expected: Revision(0),
            actual: current_revision,
        }
    );
    assert_eq!(
        error.to_string(),
        "prepared transaction expects revision 0, current revision is 1"
    );

    let stale_payload_request = FormattedPayloadEditRequest::new(
        document.id(),
        current_revision,
        vec![FormattedPayloadEdit::new(0..0, stale_payload)],
    );
    assert_eq!(
        document
            .prepare_formatted_payload_request(stale_payload_request)
            .unwrap_err(),
        ModelTransactionError::StaleRevision {
            expected: Revision(0),
            actual: current_revision,
        }
    );
    assert_eq!(document.source_bytes(), unchanged_source);
    assert_eq!(document.revision(), current_revision);
}
