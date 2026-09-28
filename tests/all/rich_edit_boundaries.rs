use viem_core::{Document, Encoding, Format};

#[test]
fn structural_batches_keep_empty_insertions_before_same_start_replacements() {
    use viem_core::document::TextEdit;
    for (format, source) in [
        (Format::Markdown, "A\n\nB\n\nC"),

    ] {
        for (edits, expected) in [
            (
                vec![
                    TextEdit::new(1..2, ""),
                    TextEdit::new(2..2, "X"),
                    TextEdit::new(2..3, "Y"),
                ],
                "AXY\nC",
            ),
            (
                vec![
                    TextEdit::new(0..0, "X"),
                    TextEdit::new(0..1, "Y"),
                    TextEdit::new(1..2, ""),
                ],
                "XYB\nC",
            ),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            document
                .apply_edits(edits)
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
            assert_eq!(document.text(), expected);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}
