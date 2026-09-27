use viem_core::document::{BoundaryAffinity, Encoding, Format, ModelRequest};
use viem_core::Document;

#[test]
fn native_inline_breaks_preserve_paragraph_ownership_and_literal_contexts() {
    for (source, text, style) in [
        ("# a<br>b", "a\nb", "Heading1"),
        ("a<BR/>b", "a\nb", "Paragraph"),
        ("> a<br />b", "a\nb", "Paragraph"),
        ("- a<br\t/>b", "a\nb", "BulletedList1"),
        ("**a<br>b**", "a\nb", "Paragraph"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(document.text(), text, "{source}");
        assert_eq!(document.projection().blocks().len(), 1);
        assert_eq!(document.projection().blocks()[0].style.0, style);
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.source_bytes(), source.as_bytes());
        let source_view = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(source_view.text(), source);
        assert_eq!(source_view.line_count(), 1);
    }
    let source = "\\<br> `<br>` <bracket> <br data-x='keep'>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), "<br> <br> <bracket> \n");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn heading_break_is_a_local_encoded_insertion_and_deleting_it_restores_exact_source() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let source = "# αβ\r\n\r\nUntouched **tail**\r\n";
        let mut bytes = match encoding {
            Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => vec![0xff, 0xfe],
            Encoding::Utf16Be => vec![0xfe, 0xff],
            _ => unreachable!(),
        };
        bytes.extend(match encoding {
            Encoding::Utf8 => source.as_bytes().to_vec(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        });
        let mut document = Document::from_bytes(bytes.clone(), encoding, Format::Markdown).unwrap();
        let before = document.text().to_owned();
        let at = "α".len();
        let prepared = document
            .prepare_model_request(ModelRequest::InsertHardBreak {
                document: document.id(),
                revision: document.revision(),
                at,
                affinity: BoundaryAffinity::Downstream,
            })
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        let patch = &prepared.summary().source_patches()[0];
        assert!(patch.range().is_empty());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document.text(),
            format!("{}\n{}", &before[..at], &before[at..])
        );
        assert_eq!(document.projection().blocks()[0].style.0, "Heading1");
        let reopened =
            Document::from_bytes(document.source_bytes(), encoding, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        document.delete(at..at + 1).unwrap();
        assert_eq!(document.source_bytes(), bytes);
    }
}
