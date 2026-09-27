use viem_core::document::{Document, Encoding, Format};

#[test]
fn literal_formats_preserve_nul_bytes() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource] {
        let mut document = Document::from_bytes(b"a".to_vec(), Encoding::Utf8, format).unwrap();
        document.insert(1, "\0").unwrap();
        assert_eq!(document.source_bytes(), b"a\0");
        assert_eq!(document.text(), "a\0");
    }
}
