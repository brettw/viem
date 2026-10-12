//! Open untrusted bytes in every format/encoding/line-ending policy, then run
//! the read-only consumers that act on a freshly opened document.
#![no_main]
use libfuzzer_sys::fuzz_target;
use viem_core::document::{Document, Encoding, FileFormat, Format};

fn open(selector: u8, bytes: &[u8]) -> Option<(Document, Format)> {
    let format = match selector & 3 {
        0 => Format::Markdown,
        1 => Format::MarkdownSource,
        2 => Format::Code,
        _ => Format::PlainText,
    };
    let encoding = match (selector >> 2) & 7 {
        4 => Some(Encoding::Utf8),
        5 => Some(Encoding::Latin1),
        6 => Some(Encoding::Utf16Le),
        7 => Some(Encoding::Utf16Be),
        _ => None,
    };
    let file_format = match (selector >> 5) & 3 {
        1 => Some(FileFormat::Unix),
        2 => Some(FileFormat::Dos),
        3 => Some(FileFormat::Mac),
        _ => None,
    };
    let bytes = bytes.to_vec();
    let document = match (encoding, file_format) {
        (Some(e), Some(f)) => Document::from_bytes_with_file_format(bytes, e, format, f),
        (Some(e), None) => Document::from_bytes(bytes, e, format),
        (None, Some(f)) => Document::from_bytes_detect_encoding_with_file_format(bytes, format, f),
        (None, None) => Document::from_bytes_detect_encoding(bytes, format),
    };
    document.ok().map(|document| (document, format))
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, bytes)) = data.split_first() else {
        return;
    };
    let Some((mut document, format)) = open(selector, bytes) else {
        return;
    };
    // Identity: opening without an edit reproduces the exact artifact.
    assert_eq!(document.source_bytes(), bytes, "open must preserve source bytes");
    let text_len = document.text().len();
    let _ = document.decoding_diagnostics();
    let _ = document.prepare_html_export().render();
    if matches!(format, Format::Markdown | Format::MarkdownSource) {
        let source = format == Format::MarkdownSource;
        if document.set_markdown_source(!source).is_ok() {
            assert_eq!(document.source_bytes(), bytes, "view switch must preserve bytes");
            let _ = document.prepare_html_export().render();
            let _ = document.set_markdown_source(source);
            assert_eq!(document.source_bytes(), bytes, "view switch must preserve bytes");
        }
    }
    let _ = text_len;
});
