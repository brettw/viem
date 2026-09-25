use viem_core::document::{Document, Encoding, FileFormat, Format, ModelRequest, TextEdit};
use viem_core::layout::DocumentLayoutStyles;

fn markdown(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn encoded(source: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Latin1 => source.bytes().collect(),
        Encoding::Utf16Le => [
            vec![0xff, 0xfe],
            source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        Encoding::Utf16Be => [
            vec![0xfe, 0xff],
            source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
    }
}

#[test]
fn split_preserves_untouched_bytes_and_character_styles_in_each_encoding() {
    for ending in ["\n", "\r\n", "\r"] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let source =
                "Before **untouched**.\n\n_abc_\n\nAfter __unchanged__.".replace('\n', ending);
            let original = encoded(&source, encoding);
            let file_format = match ending {
                "\r" => FileFormat::Mac,
                "\r\n" => FileFormat::Dos,
                _ => FileFormat::Unix,
            };
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            let at = document.text().find("abc").unwrap() + 1;
            let before_styles = (0..document.text().len())
                .filter(|&offset| document.text().is_char_boundary(offset))
                .map(|offset| {
                    (
                        offset,
                        DocumentLayoutStyles::semantic_character_at(
                            document.projection(),
                            offset,
                            false,
                        )
                        .unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at, "\n")],
                })
                .unwrap();
            let source_at = encoded(&source[..source.find("abc").unwrap() + 1], encoding).len();
            let patches = prepared.summary().source_patches();
            assert_eq!(patches.len(), 1);
            assert_eq!(patches[0].range(), source_at..source_at);
            let mut expected_bytes = original.clone();
            expected_bytes.splice(patches[0].range(), patches[0].replacement().iter().copied());
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.source_bytes(), expected_bytes);
            let reopened = Document::from_bytes_with_file_format(
                expected_bytes.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            assert_eq!(document.text(), reopened.text());
            for (offset, style) in before_styles {
                let mapped = offset + usize::from(offset >= at);
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        document.projection(),
                        mapped,
                        false
                    )
                    .unwrap(),
                    style
                );
            }
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected_bytes);
        }
    }
}

#[test]
fn joining_markdown_list_items_consumes_the_following_label() {
    for original in ["1. A\n2. B", "1. A\n   continued\n2. B"] {
        let mut document = markdown(original);
        let boundary = document.text().find('\n').unwrap();
        let mut expected = document.text().to_owned();
        expected.replace_range(boundary..boundary + 1, " ");
        document
            .replace(boundary..boundary + 1, " ")
            .unwrap_or_else(|error| panic!("{original:?}: {error:?}"));
        assert_eq!(document.text(), expected);
        let changed = document.source_bytes();
        let reopened = markdown(&String::from_utf8(changed.clone()).unwrap());
        assert_eq!(reopened.text(), expected);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn joining_prose_to_an_indented_following_line_preserves_the_rest_of_its_block() {
    for original in ["A\n\n\tB\nC", "A\n\n    B\nC", "A\n\n\tB", "A\n\n\tB\n"] {
        let mut document = markdown(original);
        let boundary = document.text().find('\n').unwrap();
        let indent = document.text()[boundary + 1..]
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .map(char::len_utf8)
            .sum::<usize>();
        let mut expected = document.text().to_owned();
        expected.replace_range(boundary..boundary + 1 + indent, " ");
        document
            .replace(boundary..boundary + 1 + indent, " ")
            .unwrap_or_else(|error| panic!("{original:?}, text={:?}: {error:?}", document.text()));
        assert_eq!(document.text(), expected);
        let changed = document.source_bytes();
        assert_eq!(
            markdown(&String::from_utf8(changed.clone()).unwrap()).text(),
            expected
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn multiline_markdown_case_replacement_retains_existing_break_syntax() {
    for original in [
        "\ta\nb",
        "\t prose \n👩‍💻é\n1. a\n1. \n2. العربية",
        "a  \nb",
        "- **a**\n- b",
    ] {
        let mut document = markdown(original);
        let expected = document.text().to_uppercase();
        document
            .replace(0..document.text().len(), &expected)
            .unwrap_or_else(|error| panic!("{original:?}: {error:?}"));
        assert_eq!(document.text(), expected);
        let changed = document.source_bytes();
        assert_eq!(
            markdown(&String::from_utf8(changed.clone()).unwrap()).text(),
            expected
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn splitting_markdown_emphasis_keeps_delimiters_structural() {
    for original in [
        "_abc_",
        "**abc**",
        "***abc***",
        "**a_b_c**",
        "**_abc_**",
        "Malformed *one **two_",
    ] {
        let original_text = markdown(original).text().to_owned();
        for at in 0..=original_text.len() {
            if !original_text.is_char_boundary(at) {
                continue;
            }
            for replacement in ["\n", "\n\n", "x\ny", "\nx\n"] {
                let mut document = markdown(original);
                let mut expected = original_text.clone();
                expected.insert_str(at, replacement);
                document
                    .replace(at..at, replacement)
                    .unwrap_or_else(|error| {
                        panic!("{original:?} at {at} with {replacement:?}: {error:?}")
                    });
                assert_eq!(document.text(), expected);
                let changed = document.source_bytes();
                assert_eq!(
                    markdown(&String::from_utf8(changed.clone()).unwrap()).text(),
                    expected
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), changed);
            }
        }
    }
}

#[test]
fn local_nested_list_edit_keeps_enclosing_list_context() {
    for original in [
        "1. parent\n   - nested\n\nTail",
        "1. parent\n   continuation\n   - nested\n\nTail",
    ] {
        let mut document = markdown(original);
        let at = document.text().find("nested").unwrap() + 3;
        document.replace(at..at + 1, "**").unwrap();
        let reopened = markdown(&String::from_utf8(document.source_bytes()).unwrap());
        let kinds = |doc: &Document| {
            doc.projection()
                .blocks()
                .iter()
                .map(|block| (block.range.clone(), block.kind.clone(), block.style.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(&document), kinds(&reopened));
        assert_eq!(document.text(), reopened.text());
    }
}

#[test]
fn multiline_replacement_across_inline_scope_edges_preserves_visible_text() {
    for source in ["**ab**_cd_", "**a_b_c**", "***abc***"] {
        let original_text = markdown(source).text().to_owned();
        for start in 0..original_text.len() {
            for end in start + 1..=original_text.len() {
                for replacement in ["\n", "x\ny", "\n\n"] {
                    let mut document = markdown(source);
                    let mut expected = original_text.clone();
                    expected.replace_range(start..end, replacement);
                    document
                        .replace(start..end, replacement)
                        .unwrap_or_else(|error| {
                            panic!("{source:?} {start}..{end} {replacement:?}: {error:?}")
                        });
                    assert_eq!(document.text(), expected);
                    assert_eq!(
                        markdown(&String::from_utf8(document.source_bytes()).unwrap()).text(),
                        expected
                    );
                }
            }
        }
    }
}

#[test]
fn joining_after_a_fenced_code_block_moves_its_closing_fence() {
    for original in [
        "```rust\nA\n```\n\nB",
        "```rust\nA\n```\n\n**B**\n\nUntouched _tail_.",
        "~~~lang\nA\n~~~\n\nB  \nC\n\nTail.",
        "```rust\nA\n\n```\n\nB\n\nTail.",
        "```rust\nA\n```\n\n- B\n\nTail.",
        "```rust\nA\n```\n\n\\`\\`\\`\n\nTail.",
        "```rust\nA\n\n```\n\n\\`\\`\\`\n\nTail.",
        "```rust\nA\n\n`````\n\n\\`\\`\\`\n\nTail.",
    ] {
        for replacement in ["", " ", "e\u{301}"] {
            let mut document = markdown(original);
            let mut expected = document.text().to_owned();
            let at = document.projection().blocks()[0].range.end;
            expected.replace_range(at..at + 1, replacement);
            document
                .replace(at..at + 1, replacement)
                .unwrap_or_else(|error| panic!("{original:?} with {replacement:?}: {error:?}"));
            assert_eq!(document.text(), expected);
            let changed = document.source_bytes();
            assert_eq!(
                markdown(&String::from_utf8(changed.clone()).unwrap()).text(),
                expected
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}
