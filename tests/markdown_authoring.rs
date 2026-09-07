use evim_core::document::{BlockKind, StyleId};
use evim_core::{Document, Encoding, Format};

#[test]
fn paragraph_menu_assignments_update_source_markers_and_undo() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = if format == Format::Markdown {
            b"__First__\r\n\r\nSecond".as_slice()
        } else {
            b"__First__\r\nSecond".as_slice()
        };
        let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_paragraph_style(0..document.text().len(), StyleId::from("Heading2"))
            .unwrap();
        assert_eq!(
            document.source_bytes(),
            if format == Format::Markdown {
                b"## __First__\r\n\r\n## Second".as_slice()
            } else {
                b"## __First__\r\n## Second".as_slice()
            }
        );
        assert!(document
            .projection()
            .blocks()
            .iter()
            .all(|block| block.kind == BlockKind::Heading(2)));
        assert_eq!(
            document.text(),
            if format == Format::Markdown {
                "First\nSecond"
            } else {
                "## __First__\n## Second"
            }
        );
        document
            .set_paragraph_style(0..0, StyleId::from("Heading1"))
            .unwrap();
        assert_eq!(
            document.source_bytes(),
            if format == Format::Markdown {
                b"# __First__\r\n\r\n## Second".as_slice()
            } else {
                b"# __First__\r\n## Second".as_slice()
            }
        );
        document
            .set_paragraph_style(0..document.text().len(), StyleId::from("Paragraph"))
            .unwrap();
        assert_eq!(document.source_bytes(), source);
        for _ in 0..3 {
            assert!(document.undo());
        }
        assert_eq!(document.source_bytes(), source);
        assert!(document.redo());
        assert_eq!(
            document.source_bytes(),
            if format == Format::Markdown {
                b"## __First__\r\n\r\n## Second".as_slice()
            } else {
                b"## __First__\r\n## Second".as_slice()
            }
        );
    }
}

#[test]
fn flowed_paragraph_styles_flatten_only_soft_source_breaks() {
    use evim_core::document::ListStyle;
    for newline in ["\n", "\r\n"] {
        let original = format!(
            "Before{newline}{newline}__First__{newline}soft continuation{newline}{newline}Tail"
        );
        for list in [false, true] {
            let mut document = Document::from_bytes(
                original.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::Markdown,
            )
            .unwrap();
            let range = 7..30;
            if list {
                document
                    .set_list_style(range, Some(ListStyle::Bullet))
                    .unwrap();
            } else {
                document
                    .set_paragraph_style(range, StyleId::from("Heading2"))
                    .unwrap();
            }
            let marker = if list { "- " } else { "## " };
            assert_eq!(document.source_bytes(), format!("Before{newline}{newline}{marker}__First__ soft continuation{newline}{newline}Tail").as_bytes());
            assert_eq!(
                document.text(),
                if list {
                    "Before\n- First soft continuation\nTail"
                } else {
                    "Before\nFirst soft continuation\nTail"
                }
            );
            let saved = document.source_bytes();
            let reopened =
                Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), document.text());
            assert_eq!(
                reopened.projection().blocks()[1].kind,
                document.projection().blocks()[1].kind
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}

#[test]
fn removing_structural_markers_keeps_neighboring_paragraphs_separate() {
    use evim_core::document::ListStyle;
    for (source, expected) in [
        ("Before\n# Heading\nTail", "Before\n\nHeading\n\nTail"),
        ("# First\n## Second\nTail", "First\n\nSecond\n\nTail"),
        (
            "Before\n- First\n- Second\nTail",
            "Before\n\nFirst\n\nSecond\n\nTail",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.text().replace("- ", "");
        if source.contains("- ") {
            document
                .set_list_style(0..document.text().len(), None::<ListStyle>)
                .unwrap();
        } else {
            document
                .set_paragraph_style(0..document.text().len(), StyleId::from("Paragraph"))
                .unwrap();
        }
        assert_eq!(document.text(), before);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert!(document
            .projection()
            .blocks()
            .iter()
            .all(|block| block.kind == BlockKind::Paragraph));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}
