use viem_core::document::{BlockKind, StyleId};
use viem_core::{Document, Encoding, Format};

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
        assert_eq!(document.source_bytes(), b"__First__\r\n\r\nSecond");
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
    use viem_core::document::ListStyle;
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
                    "Before\nFirst soft continuation\nTail"
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
    use viem_core::document::ListStyle;
    for (source, expected) in [
        ("Before\n# Heading\nTail", "Before\n\nHeading\n\nTail"),
        ("# First\n## Second\nTail", "First\n\nSecond\n\nTail"),
        (
            "Before\n- First\n- Second\n\nTail",
            "Before\n\nFirst\n\nSecond\n\nTail",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.text().to_owned();
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

#[test]
fn source_list_removal_keeps_the_middle_item_a_separate_paragraph() {
    for marker in ["- ", "3. "] {
        let source = format!("{marker}Before\n{marker}Middle\n{marker}After");
        let mut document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let at = document.text().find("Middle").unwrap();
        document.set_list_style(at..at, None).unwrap();
        let tail = if marker == "3. " { "1. " } else { marker };
        let expected = format!("{marker}Before\n\nMiddle\n\n{tail}After");
        assert_eq!(document.source_bytes(), expected.as_bytes());
        for format in [Format::MarkdownSource, Format::Markdown] {
            let reopened =
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
            assert_eq!(reopened.projection().blocks().len(), 3);
            assert_eq!(reopened.projection().blocks()[1].kind, BlockKind::Paragraph);
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn code_paragraph_assignment_wraps_and_unwraps_the_whole_paragraph() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for newline in ["\n", "\r\n"] {
            let source = format!("Before{newline}{newline}Words{newline}{newline}After");
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let at = document.text().find("Words").unwrap() + 2;
            document
                .set_paragraph_style(at..at, "Code Block".into())
                .unwrap();
            let fenced = format!(
                "Before{newline}{newline}```{newline}Words{newline}```{newline}{newline}After"
            );
            assert_eq!(document.source_bytes(), fenced.as_bytes());
            assert_eq!(document.projection().blocks()[1].style.0, "Code Block");
            let at = document.text().find("Words").unwrap() + 2;
            document
                .set_paragraph_style(at..at, "Paragraph".into())
                .unwrap();
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert_eq!(document.projection().blocks()[1].style.0, "Paragraph");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), fenced.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn clearing_code_fences_preserves_boundaries_without_existing_blank_lines() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "Before\n```rust\nWords\n```\nAfter";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let at = document.text().find("Words").unwrap();
        document
            .set_paragraph_style(at..at, "Paragraph".into())
            .unwrap();
        assert_eq!(document.source_bytes(), b"Before\n\nWords\n\nAfter");
        assert_eq!(document.projection().blocks().len(), 3);
        assert_eq!(document.projection().blocks()[1].style.0, "Paragraph");
    }
}

#[test]
fn code_toggle_handles_empty_multiline_and_literal_delimiters() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in ["", "Words", "one\ntwo", "# Heading", "- Item"] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            document
                .set_paragraph_style(0..0, "Code Block".into())
                .unwrap_or_else(|error| panic!("enable {format:?} {source:?}: {error:?}"));
            assert_eq!(document.projection().blocks()[0].style.0, "Code Block");
            let saved = document.source_bytes();
            let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(reopened.text(), document.text());
            document
                .set_paragraph_style(0..0, "Paragraph".into())
                .unwrap_or_else(|error| panic!("disable {format:?} {source:?}: {error:?}"));
            assert_eq!(document.projection().blocks()[0].style.0, "Paragraph");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), saved);
        }
        for source in [
            "```",
            "```\n```",
            "~~~rust\none\ntwo\n~~~",
            "```\none\n\n  two  \n```",
            "```\n# Heading\n```",
            "```\n1. Item\n```",
            "```\n- Item\n```",
            "```\n> Quote\n```",
            "```\n~~~\n```",
            "```\nHello, world! x = (a + b);\n```",
            "```\nWords",
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let visible =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap()
                    .text()
                    .to_owned();
            document
                .set_paragraph_style(0..0, "Paragraph".into())
                .unwrap_or_else(|error| panic!("clear {format:?} {source:?}: {error:?}"));
            let reopened =
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(reopened.text(), visible, "{format:?} {source:?}");
            assert_eq!(reopened.projection().blocks().len(), 1);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn source_structural_removal_preserves_paragraphs_and_existing_separators() {
    for (source, needle, expected) in [
        (
            "Before\n# Heading\nAfter",
            "Heading",
            "Before\n\nHeading\n\nAfter",
        ),
        (
            "Before\n> Quote\n\nAfter",
            "Quote",
            "Before\n\nQuote\n\nAfter",
        ),
        (
            "- Before\n- Middle\n  continuation\n- After",
            "Middle",
            "- Before\n\nMiddle\ncontinuation\n\n- After",
        ),
        (
            "- Before\n \t\n- Middle\n\n- After",
            "Middle",
            "- Before\n \t\nMiddle\n\n- After",
        ),
    ] {
        let mut document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let at = document.text().find(needle).unwrap();
        document
            .set_paragraph_style(at..at, "Paragraph".into())
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(document.projection().blocks().len(), 3);
        let saved = document.source_bytes();
        let reopened =
            Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.projection().blocks().len(), 3);
        assert_eq!(reopened.projection().blocks()[1].kind, BlockKind::Paragraph);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn code_toolbar_command_keeps_the_caret_in_the_body_and_groups_undo() {
    use viem_core::command::InputEvent;
    use viem_core::document::{BoundaryAffinity, StyleNamespace};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "Before\n\nWords\n\nAfter";
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 400.);
        let at = core.document().text().find("Words").unwrap() + 2;
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        for style in ["Code Block", "Paragraph"] {
            core.handle(
                view,
                CoreEvent::AssignNamedStyle {
                    expected: core.list_selection_identity(view).unwrap(),
                    style_sheet_revision: core.document().projection().style_sheet().revision,
                    namespace: StyleNamespace::Block,
                    style: style.into(),
                },
            )
            .unwrap();
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                core.document().text().find("Words").unwrap() + 2
            );
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(
            core.document().projection().blocks()[1].style.0,
            "Code Block"
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn code_fences_preserve_encodings_and_selected_paragraph_boundaries() {
    let encode = |text: &str, encoding| match encoding {
        Encoding::Utf16Le => [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        Encoding::Utf16Be => [
            vec![0xfe, 0xff],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
        Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
        Encoding::Utf8 => text.as_bytes().to_vec(),
    };
    for format in [Format::Markdown, Format::MarkdownSource] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for ending in ["\n", "\r\n", "\r"] {
                let source = format!("Hé{ending}{ending}Tail");
                let mut document = Document::from_bytes_with_file_format(
                    encode(&source, encoding),
                    encoding,
                    format,
                    match ending {
                        "\r" => viem_core::document::FileFormat::Mac,
                        "\r\n" => viem_core::document::FileFormat::Dos,
                        _ => viem_core::document::FileFormat::Unix,
                    },
                )
                .unwrap();
                document
                    .set_paragraph_style(0..0, "Code Block".into())
                    .unwrap();
                let expected = format!("```{ending}Hé{ending}```{ending}{ending}Tail");
                assert_eq!(document.source_bytes(), encode(&expected, encoding));
                document
                    .set_paragraph_style(0..0, "Paragraph".into())
                    .unwrap();
                assert_eq!(document.source_bytes(), encode(&source, encoding));
            }
        }
        for source in ["```\nOne\n```\n```\nTwo\n```", "Before\n```\n```\nAfter"] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            document
                .set_paragraph_style(0..document.text().len(), "Paragraph".into())
                .unwrap();
            let reopened =
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(
                reopened.text(),
                if source.starts_with("Before") {
                    "Before\n\nAfter"
                } else {
                    "One\nTwo"
                }
            );
        }
    }
    let mut document = Document::from_bytes(
        b"Text ````literal````".to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    document
        .set_paragraph_style(0..0, "Code Block".into())
        .unwrap();
    assert_eq!(
        document.source_bytes(),
        b"`````\nText ````literal````\n`````"
    );
}
