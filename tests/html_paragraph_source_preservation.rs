use viem_core::{Document, Encoding, Format};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => [vec![0xff, 0xfe], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
        Encoding::Utf16Be => [vec![0xfe, 0xff], text.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
        _ => unreachable!(),
    }
}

#[test]
fn paragraph_merges_retain_original_container_and_metadata_bytes() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for ending in ["\r\n", "\r", "\n"] {
            let container = format!("<div data-keep='é'>{ending}<!--keep{ending}α-->{ending}</div>");
            let comment = format!("<!--keep{ending}é-->");
            for (source, expected) in [
                (format!("<p>a</p>{container}<p>b</p>"), format!("<p>ab</p>{container}")),
                (format!("<p>a</p><p>b</p>{container}<p>tail</p>"), format!("<p>ab</p>{container}<p>tail</p>")),
                (format!("<p>a</p>{comment}<p>b</p>"), format!("<p>a{comment}b</p>")),
            ] {
                let original = encoded(&source, encoding);
                let expected = encoded(&expected, encoding);
                let mut document = Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
                document.delete(1..2).unwrap();
                assert_eq!(document.source_bytes(), expected, "{encoding:?} {source:?}");
                let reopened = Document::from_bytes(document.source_bytes(), encoding, Format::Html).unwrap();
                assert_eq!(reopened.text(), document.text());
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), expected);
            }
        }
    }
}
