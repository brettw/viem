use viem_core::document::{BlockKind, Document, Encoding, Format, StyleId};

#[test]
fn typing_on_blank_indented_code_lines_reopens_with_exact_history() {
    use viem_core::command::{CommandStatus, InputEvent, Key};
    use viem_core::document::{BoundaryAffinity, HistoryNavigationRequest};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (source, positions) in [
        ("    a\n\n\n    b\n", vec![2, 3]),
        ("    chunk1\n\n    chunk2\n  \n \n \n    chunk3\n", vec![7, 15, 16, 17]),
    ] {
        for at in positions {
            let mut core = Core::new(open(source, Format::Markdown));
            let mut expected = core.document().text().to_owned();
            expected.insert(at, 'x');
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            for key in [Key::Char('i'), Key::Char('x'), Key::Escape] {
                let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                if let Some(command) = outcome.command {
                    assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending), "{at}: {:?}", command.status);
                }
            }
            assert_eq!(core.document().text(), expected);
            let saved = core.document().source_bytes();
            let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected, "saved {:?}", String::from_utf8_lossy(&saved));
            assert_eq!(core.document().projection().style_spans(), reopened.projection().style_spans());
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn gfm_indented_code_preserves_literal_body_and_source() {
    for (source, expected, code_count) in [
        ("    code\n\nTail", "code\nTail", 1),
        (
            "    *literal* &amp; <b>x</b>\n    next",
            "*literal* &amp; <b>x</b>\nnext",
            1,
        ),
        ("    a\n\n      b\n\nTail", "a\n\n  b\nTail", 1),
        ("\tfoo\n\t\tbar", "foo\n\tbar", 1),
        ("    foo\nbar", "foo\nbar", 1),
        (
            "paragraph\n    continuation",
            "paragraph continuation",
            0,
        ),
        ("paragraph\n\n    code", "paragraph\ncode", 1),
        (">     code\n>     next\n>\n> tail", "code\nnext\ntail", 1),
        (
            "- item\n\n      code\n      next\n- tail",
            "item\ncode\nnext\ntail",
            1,
        ),
        ("-     code", "code", 1),
        ("    ```\n    *literal*", "```\n*literal*", 1),
        ("    a\n      \n    b", "a\n  \nb", 1),
        ("- x\n\n\t\tcode", "x\n  code", 1),
        ("- \t\tcode", "  code", 1),
        ("- item\n\n    continuation", "item\ncontinuation", 0),
    ] {
        let doc = open(source, Format::Markdown);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(
            doc.projection()
                .blocks()
                .iter()
                .filter(|b| b.style.0 == "Code Block")
                .count(),
            code_count,
            "{source:?}"
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
        let source_doc = open(source, Format::MarkdownSource);
        assert_eq!(
            source_doc
                .projection()
                .blocks()
                .iter()
                .filter(|b| b.style.0 == "Code Block")
                .count(),
            code_count,
            "source: {source:?}"
        );
        assert_eq!(source_doc.source_bytes(), source.as_bytes());
        if source.starts_with("- ") && code_count > 0 {
            assert!(doc
                .projection()
                .blocks()
                .iter()
                .filter(|b| b.style.0 == "Code Block")
                .all(|b| matches!(b.kind, BlockKind::ListItem { .. })));
        }
    }
}

#[test]
fn indented_code_editing_preserves_encodings_endings_and_unrelated_source() {
    use viem_core::document::FileFormat;
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for ending in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let source = "Before\n\n    café\n    next\n\nAfter".replace(
                '\n',
                match ending {
                    FileFormat::Unix => "\n",
                    FileFormat::Dos => "\r\n",
                    FileFormat::Mac => "\r",
                },
            );
            let bytes = match encoding {
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                Encoding::Latin1 => source.chars().map(|ch| ch as u8).collect(),
                _ => source.as_bytes().to_vec(),
            };
            let mut doc = Document::from_bytes_with_file_format(
                bytes.clone(),
                encoding,
                Format::Markdown,
                ending,
            )
            .unwrap();
            let at = doc.text().find("café").unwrap();
            doc.replace(at..at, "*").unwrap();
            let mut expected = "Before\ncafé\nnext\nAfter".to_owned();
            expected.insert(at, '*');
            assert_eq!(doc.text(), expected);
            let saved = doc.source_bytes();
            let reopened = Document::from_bytes_with_file_format(
                saved.clone(),
                encoding,
                Format::Markdown,
                ending,
            )
            .unwrap();
            assert_eq!(reopened.text(), expected);
            assert_eq!(reopened.projection().blocks()[1].style.0, "Code Block");
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), bytes);
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), saved);
        }
    }
}

#[test]
fn indented_code_typing_breaks_deletion_and_toolbar_reopen_exactly() {
    for source in ["    code\n\nTail", "    first\n    second", "\tcode"] {
        let original = open(source, Format::Markdown);
        let body = original.projection().blocks()[0].range.clone();
        for at in body.clone().chain(std::iter::once(body.end)) {
            for inserted in ["x", "*", "`", "\n", "x\ny"] {
                let mut doc = open(source, Format::Markdown);
                let mut expected = doc.text().to_owned();
                expected.insert_str(at, inserted);
                doc.replace(at..at, inserted)
                    .unwrap_or_else(|e| panic!("{source:?} at {at} {inserted:?}: {e:?}"));
                assert_eq!(doc.text(), expected);
                let reopened =
                    Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Markdown)
                        .unwrap();
                assert_eq!(reopened.text(), expected);
                assert_eq!(reopened.projection().blocks()[0].style.0, "Code Block");
                assert!(doc.undo());
                assert_eq!(doc.source_bytes(), source.as_bytes());
                assert!(doc.redo());
                assert_eq!(doc.text(), expected);
            }
        }
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut doc = open(source, format);
            doc.set_paragraph_style(0..0, StyleId::from("Paragraph"))
                .unwrap();
            assert_eq!(doc.projection().blocks()[0].style.0, "Paragraph");
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn clearing_indented_code_in_containers_removes_the_code_treatment() {
    for source in [">     code\n>     next", "-     code"] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut doc = open(source, format);
            doc.set_paragraph_style(0..0, StyleId::from("Paragraph"))
                .unwrap_or_else(|error| panic!("{source:?}, {format:?}: {error:?}"));
            assert_eq!(doc.projection().blocks()[0].style.0, "Paragraph");
            assert!(doc.projection().style_spans().iter().all(|span| {
                span.application != viem_core::document::StyleApplication::Semantic(
                    viem_core::document::SemanticInlineStyle::Code,
                )
            }));
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}
