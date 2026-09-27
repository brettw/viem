use viem_core::document::{
    Document, Encoding, Format, ModelRequest, ProjectionWorkScope,
};

fn encode_with_bom(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => {
            let mut bytes = vec![0xef, 0xbb, 0xbf];
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::Latin1 => text
            .chars()
            .map(|character| {
                u8::try_from(character as u32).expect("fixture is representable in Latin-1")
            })
            .collect(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut bytes = if encoding == Encoding::Utf16Le {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in text.encode_utf16() {
                let encoded = if encoding == Encoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                bytes.extend_from_slice(&encoded);
            }
            bytes
        }
    }
}

#[test]
fn opening_decodes_each_encoding_once_and_preserves_exact_source() {
    let source_text = "# Title **café**\r\nsecond line";

    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            let source = encode_with_bom(source_text, encoding);
            let document = Document::from_bytes(source.clone(), encoding, format).unwrap();
            let work = document.open_work_statistics();

            assert_eq!(work.source_decode_passes(), 1, "{encoding:?} {format:?}");
            assert_eq!(
                work.decoded_source_bytes(),
                source.len(),
                "{encoding:?} {format:?}"
            );
            assert_eq!(
                work.decoded_utf8_bytes(),
                source_text.len(),
                "{encoding:?} {format:?}"
            );
            assert_eq!(work.projected_formatted_bytes(), document.text().len());
            assert_eq!(
                work.projected_hard_lines(),
                document.projection().hard_line_count()
            );
            assert_eq!(document.source_bytes(), source);
        }
    }
}

#[test]
fn large_open_reports_one_linear_decode_pass() {
    let line = "0123456789abcdef **styled** payload\n\n";
    let source_text = line.repeat(32_768);
    let source = source_text.as_bytes().to_vec();
    let document = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    let work = document.open_work_statistics();

    assert_eq!(work.source_decode_passes(), 1);
    assert_eq!(work.decoded_source_bytes(), source.len());
    assert_eq!(work.decoded_utf8_bytes(), source_text.len());
    assert_eq!(work.projected_hard_lines(), 32_769);
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn format_reinterpretation_accounts_for_one_candidate_decode() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let source = encode_with_bom("# alpha\n\nbeta", encoding);
        let document = Document::from_bytes(source, encoding, Format::PlainText).unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::SetFormat {
                document: document.id(),
                revision: document.revision(),
                target: Format::Markdown,
                operation: viem_core::document::FormatOperation::Reinterpret,
            })
            .unwrap();
        let summary = prepared.summary();
        let work = summary.projection_work();
        let candidate_source_len = document
            .source_byte_len()
            .checked_add(
                summary
                    .source_patches()
                    .iter()
                    .map(|patch| patch.replacement().len())
                    .sum::<usize>(),
            )
            .unwrap()
            .checked_sub(
                summary
                    .source_patches()
                    .iter()
                    .map(|patch| patch.range().len())
                    .sum::<usize>(),
            )
            .unwrap();

        assert_eq!(work.scope(), ProjectionWorkScope::FullDocument);
        assert_eq!(work.source_decode_passes(), 1);
        assert_eq!(work.decoded_source_bytes(), candidate_source_len);
    }
}
