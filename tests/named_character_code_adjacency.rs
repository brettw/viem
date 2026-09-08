use viem_core::document::*;

#[test]
fn assigning_code_joins_adjacent_spans_without_rewriting_their_bodies() {
    for (source, start, end, expected) in [
        ("`a`bc", 1, 2, "`ab`c"),
        ("ab`c`", 1, 2, "a`bc`"),
        ("`a`b`c`", 1, 2, "`abc`"),
        ("a`b`c`d`e", 0, 5, "`abcde`"),
        ("``a ` b``c", 5, 6, "``a ` bc``"),
        ("`a`\\`", 1, 2, "`` a` ``"),
        ("\\``b`", 0, 1, "`` `b ``"),
        ("` a `b", 1, 2, "`ab`"),
        ("` a` ", 2, 3, "`  a  `"),
        ("`a`**b**", 1, 2, "`a`**`b`**"),
        ("**a**`b`", 0, 1, "**`a`**`b`"),
        ("`é`👩‍💻x", 2, 13, "`é👩‍💻`x"),
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let encode = |text: &str| match encoding {
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                _ => text.as_bytes().to_vec(),
            };
            let before = encode(source);
            let mut document =
                Document::from_bytes(before.clone(), encoding, Format::Markdown).unwrap();
            let text = document.text().to_owned();
            let strong = document
                .projection()
                .style_spans()
                .iter()
                .filter(|span| {
                    span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)
                })
                .cloned()
                .collect::<Vec<_>>();
            let changed = document
                .apply_model_request(ModelRequest::AssignNamedStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: start..end,
                    namespace: StyleNamespace::Character,
                    style: "Code".into(),
                })
                .unwrap_or_else(|error| {
                    panic!("{source:?} {start}..{end} {encoding:?}: {error:?}")
                });
            let after = document.source_bytes();
            assert_eq!(after, encode(expected), "{source:?} {encoding:?}");
            assert_eq!(document.text(), text);
            assert_eq!(
                document
                    .projection()
                    .style_spans()
                    .iter()
                    .filter(|span| span.application
                        == StyleApplication::Semantic(SemanticInlineStyle::Strong))
                    .cloned()
                    .collect::<Vec<_>>(),
                strong
            );
            // Every original byte outside the explicit syntax patches survives.
            let (mut old, mut new) = (0, 0);
            for patch in changed.summary().source_patches() {
                let length = patch.range().start - old;
                assert_eq!(&before[old..old + length], &after[new..new + length]);
                old = patch.range().end;
                new += length + patch.replacement().len();
            }
            assert_eq!(&before[old..], &after[new..]);
            let reopened = Document::from_bytes(after.clone(), encoding, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), text);
            assert_eq!(reopened.line_count(), document.line_count());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}
