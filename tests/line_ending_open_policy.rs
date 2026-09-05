use evim_core::document::{
    Document, Encoding, FileFormat, FileFormatOrigin, Format, LineEndingOpenPolicy,
    LineEndingPolicyError,
};

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|character| u8::try_from(u32::from(character)).expect("Latin-1 fixture"))
            .collect(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn default_policy_is_shared_by_plain_text_and_markdown_in_every_encoding() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            let source = encode("alpha\r\nbeta\r\n", encoding);
            let document = Document::from_bytes(source.clone(), encoding, format).unwrap();
            assert_eq!(document.file_format(), FileFormat::Dos);
            assert_eq!(document.file_format_origin(), FileFormatOrigin::Detected);
            assert_eq!(document.text(), "alpha\nbeta\n");
            assert_eq!(document.source_bytes(), source);
        }
    }
}

#[test]
fn empty_single_and_forced_policies_bypass_detection_without_rewriting() {
    let empty = LineEndingOpenPolicy::new(Vec::new(), FileFormat::Mac).unwrap();
    let source = b"one\rtwo\r".to_vec();
    let document = Document::from_bytes_with_line_ending_policy(
        source.clone(),
        Encoding::Utf8,
        Format::PlainText,
        empty,
    )
    .unwrap();
    assert_eq!(document.file_format(), FileFormat::Mac);
    assert_eq!(document.file_format_origin(), FileFormatOrigin::Defaulted);
    assert_eq!(document.text(), "one\ntwo\n");
    assert_eq!(document.source_bytes(), source);

    let single = LineEndingOpenPolicy::new(vec![FileFormat::Dos], FileFormat::Unix).unwrap();
    let source = b"one\r\ntwo".to_vec();
    let document = Document::from_bytes_with_line_ending_policy(
        source.clone(),
        Encoding::Utf8,
        Format::Markdown,
        single,
    )
    .unwrap();
    assert_eq!(document.file_format(), FileFormat::Dos);
    assert_eq!(document.file_format_origin(), FileFormatOrigin::Defaulted);
    assert_eq!(document.source_bytes(), source);

    let forced = LineEndingOpenPolicy::default().with_forced_file_format(FileFormat::Unix);
    let source = b"one\r\ntwo".to_vec();
    let document = Document::from_bytes_with_line_ending_policy(
        source.clone(),
        Encoding::Utf8,
        Format::PlainText,
        forced,
    )
    .unwrap();
    assert_eq!(document.file_format(), FileFormat::Unix);
    assert_eq!(document.file_format_origin(), FileFormatOrigin::Forced);
    assert_eq!(document.text(), "one\r\ntwo");
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn ordered_multi_policy_detects_content_and_uses_first_entry_as_fallback() {
    let policy = LineEndingOpenPolicy::new(
        vec![FileFormat::Mac, FileFormat::Dos, FileFormat::Unix],
        FileFormat::Unix,
    )
    .unwrap();

    let dos = Document::from_bytes_with_line_ending_policy(
        b"a\r\nb\r\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        policy.clone(),
    )
    .unwrap();
    assert_eq!(dos.file_format(), FileFormat::Dos);
    assert_eq!(dos.file_format_origin(), FileFormatOrigin::Detected);

    let unix = Document::from_bytes_with_line_ending_policy(
        b"a\r\nb\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        policy.clone(),
    )
    .unwrap();
    assert_eq!(unix.file_format(), FileFormat::Unix);

    let fallback = Document::from_bytes_with_line_ending_policy(
        b"no line endings".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        policy,
    )
    .unwrap();
    assert_eq!(fallback.file_format(), FileFormat::Mac);
    assert_eq!(fallback.file_format_origin(), FileFormatOrigin::Defaulted);
}

#[test]
fn legacy_mac_heuristic_is_explicitly_opted_in_and_bounded() {
    let policy =
        LineEndingOpenPolicy::new(vec![FileFormat::Unix, FileFormat::Mac], FileFormat::Unix)
            .unwrap();
    let mac = Document::from_bytes_with_line_ending_policy(
        b"a\rb\rc\nd".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        policy.clone(),
    )
    .unwrap();
    assert_eq!(mac.file_format(), FileFormat::Mac);

    let unix = Document::from_bytes_with_line_ending_policy(
        b"a\nb\rc\r".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        policy,
    )
    .unwrap();
    assert_eq!(unix.file_format(), FileFormat::Unix);

    let default =
        Document::from_bytes(b"a\rb\r".to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    assert_eq!(default.file_format(), FileFormat::Unix);
}

#[test]
fn policy_rejects_duplicates_before_a_document_is_opened() {
    assert_eq!(
        LineEndingOpenPolicy::new(
            vec![FileFormat::Unix, FileFormat::Dos, FileFormat::Unix],
            FileFormat::Unix,
        ),
        Err(LineEndingPolicyError::DuplicateFileFormat(FileFormat::Unix))
    );
}
