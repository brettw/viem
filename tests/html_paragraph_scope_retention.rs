use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::DocumentLayoutStyles;

#[test]
fn materialization_keeps_cross_paragraph_inline_ancestors_around_untouched_content() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |source: &str| -> Vec<u8> {
            match encoding {
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => source.as_bytes().to_vec(),
            }
        };
        for (source, expected) in [
            ("<span style='color:red'>a<div>b</div>c</span>", [
                "<span style='color:red'><p>aX</p><div>b</div>c</span>",
                "<span style='color:red'>a<div><p>bX</p></div>c</span>",
                "<span style='color:red'>a<div>b</div><p>cX</p></span>",
            ]),
            ("<span style='color:red'><i>a</i><div>b</div><b>c</b></span>", [
                "<span style='color:red'><p><i>aX</i></p><div>b</div><b>c</b></span>",
                "<span style='color:red'><i>a</i><div><p>bX</p></div><b>c</b></span>",
                "<span style='color:red'><i>a</i><div>b</div><p><b>cX</b></p></span>",
            ]),
        ] {
            for (paragraph, expected) in expected.into_iter().enumerate() {
                let original = encode(source);
                let mut document = Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
                let before = [0, 2, 4].map(|at| DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap());
                let at = paragraph * 2 + 1;
                document.insert(at, "X").unwrap();
                assert_eq!(document.source_bytes(), encode(expected));
                let after = document.source_bytes();
                let reopened = Document::from_bytes(after.clone(), encoding, Format::Html).unwrap();
                assert_eq!(reopened.text(), document.text());
                for (index, old) in before.into_iter().enumerate() {
                    let current = index * 2 + usize::from(index * 2 >= at);
                    assert_eq!(DocumentLayoutStyles::semantic_character_at(document.projection(), current, false).unwrap(), old);
                    assert_eq!(DocumentLayoutStyles::semantic_character_at(reopened.projection(), current, false).unwrap(), old);
                }
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), after);
            }
        }
    }
}
