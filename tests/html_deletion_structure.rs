use viem_core::document::{Document, Encoding, Format, TextEdit};
use viem_core::layout::DocumentLayoutStyles;

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        _ => unreachable!(),
    }
}

fn html(source: Vec<u8>, encoding: Encoding) -> Document {
    Document::from_bytes(source, encoding, Format::Html).unwrap()
}

#[test]
fn deleting_pre_prefix_preserves_newly_leading_lf_after_reopening() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (body, expected) in [
            ("ab\n c", "<br> c"),
            ("ab\r\n c", "<br> c"),
            ("ab&#10; c", "<br> c"),
            ("\nab\n c", "\n\n c"),
            ("ab<br class='keep'> c", "<br class='keep'> c"),
            ("<b>ab</b>\n c", "<br> c"),
            ("ab<!--keep-->\n c", "<!--keep-->\n c"),
        ] {
            for batch in [false, true] {
                let source = format!("<pre data-keep='yes'>{body}</pre><p>tail</p>");
                let bytes = encode(&source, encoding);
                let mut document = html(bytes.clone(), encoding);
                let before = document.text().to_owned();
                let edits = if batch {
                    vec![TextEdit::new(0..1, ""), TextEdit::new(1..2, "")]
                } else {
                    vec![TextEdit::new(0..2, "")]
                };
                document.apply_edits(edits).unwrap_or_else(|error| {
                    panic!("{source}, {encoding:?}, batch={batch}: {error:?}")
                });
                assert_eq!(document.text(), &before[2..]);
                assert_eq!(
                    document.source_bytes(),
                    encode(
                        &format!("<pre data-keep='yes'>{expected}</pre><p>tail</p>"),
                        encoding
                    )
                );
                let changed = document.source_bytes();
                assert_eq!(html(changed.clone(), encoding).text(), document.text());
                assert!(document.undo());
                assert_eq!(document.source_bytes(), bytes);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), changed);
            }
        }
    }
}

#[test]
fn deleting_parent_list_body_preserves_empty_paragraph_and_nested_ownership() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for body in ["ab", "ab <!--keep--> ", "<b>ab</b>", "<p>ab</p>"] {
            for batch in [false, true] {
                let source = format!("<ol start='4'><li style='color:#123456'>{body}<ul><li>child</li></ul></li><li>tail</li></ol>");
                let bytes = encode(&source, encoding);
                let mut document = html(bytes.clone(), encoding);
                let before = document.projection().blocks()[0].clone();
                let original_list = document.projection().list_structure();
                let mut original_character =
                    DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
                        .unwrap();
                // Empty character scopes are removed with their consumed text.
                // The retained list-item paragraph keeps its own declarations;
                // pending replacement typography belongs to the editing view.
                if body == "<b>ab</b>" {
                    original_character.bold = false;
                    original_character.weight = original_character.base_weight;
                }
                let edits = if batch {
                    vec![TextEdit::new(0..1, ""), TextEdit::new(1..2, "")]
                } else {
                    vec![TextEdit::new(0..2, "")]
                };
                document.apply_edits(edits).unwrap_or_else(|error| {
                    panic!("{source}, {encoding:?}, batch={batch}: {error:?}")
                });
                assert_eq!(document.text(), "\nchild\ntail");
                let changed = document.source_bytes();
                let reopened = html(changed.clone(), encoding);
                assert_eq!(reopened.text(), document.text(), "{source}");
                assert_eq!(reopened.projection().blocks()[0].style, before.style);
                assert_eq!(reopened.projection().blocks()[0].kind, before.kind);
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false)
                        .unwrap(),
                    original_character
                );
                let changed_list = reopened.projection().list_structure();
                assert_eq!(changed_list.lists.len(), original_list.lists.len());
                assert_eq!(
                    changed_list.lists[0].items.len(),
                    original_list.lists[0].items.len()
                );
                assert_eq!(changed_list.lists[0].items[0].ordinal, 4);
                assert_eq!(changed_list.lists[0].items[1].ordinal, 5);
                assert_eq!(changed_list.lists[0].items[0].child_lists.len(), 1);
                assert!(document.undo());
                assert_eq!(document.source_bytes(), bytes);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), changed);
            }
        }
    }
}

#[test]
fn emptied_final_paragraph_keeps_its_editable_source_boundary() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for body in ["B", "<b>B</b>", "<span style='color:red'>B</span>"] {
            let source = format!("<p>A</p><p>{body}</p><!--tail-->");
            let mut document = html(encode(&source, encoding), encoding);
            document.replace(2..3, "").unwrap();
            let emptied = document.source_bytes();
            let mut reopened = html(emptied.clone(), encoding);
            document.replace(2..2, "X").unwrap();
            reopened.replace(2..2, "X").unwrap();
            assert_eq!(document.text(), "A\nX");
            assert_eq!(document.source_bytes(), reopened.source_bytes());
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(document.projection(), 2, false)
                    .unwrap(),
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), 2, false)
                    .unwrap()
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), emptied);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), encode(&source, encoding));
        }
    }
}
