use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BlockKind, ModelRequest, SemanticInlineStyle, StyleApplication, StyleId,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};
use viem_core::{Document, Encoding, Format};

fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn quote_containers_project_bodies_and_keep_original_bytes() {
    for (source, expected, styles) in [
        ("> first\n> second", "first second", vec!["Block quote"]),
        (
            "before\n> first\n> second\n\nafter",
            "before\nfirst second\nafter",
            vec!["Paragraph", "Block quote", "Paragraph"],
        ),
        (
            "> first\nlazy\n>\n> second",
            "first lazy\nsecond",
            vec!["Block quote", "Block quote"],
        ),
        (
            "> outer\n>> inner\n> outer again",
            "outer\ninner\nouter again",
            vec!["Block quote", "Block quote", "Block quote"],
        ),
        (
            "> quote\n# Heading",
            "quote\nHeading",
            vec!["Block quote", "Heading1"],
        ),
    ] {
        let doc = document(source, Format::Markdown);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(
            doc.projection()
                .blocks()
                .iter()
                .map(|block| block.style.0.as_str())
                .collect::<Vec<_>>(),
            styles,
            "{source:?}"
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn quote_preserves_nested_heading_list_and_fenced_code_syntax() {
    let source = "> # Heading\n>\n> - one\n>   continuation\n> - two\n>\n> ```\n> > literal\n> ```";
    let doc = document(source, Format::Markdown);
    assert_eq!(doc.text(), "Heading\none continuation\ntwo\n> literal");
    assert_eq!(doc.projection().blocks()[0].kind, BlockKind::Heading(1));
    assert!(matches!(
        doc.projection().blocks()[1].kind,
        BlockKind::ListItem { .. }
    ));
    assert!(doc
        .projection()
        .blocks()
        .iter()
        .all(|block| block.style.0 == "Block quote"));
    assert!(doc
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)));
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn source_quote_paragraphs_include_the_visible_markers() {
    let source = "> first\n> second\n\n> third";
    let doc = document(source, Format::MarkdownSource);
    assert_eq!(doc.text(), "> first\n> second\n> third");
    assert_eq!(doc.projection().blocks().len(), 2);
    assert_eq!(doc.projection().blocks()[0].range, 0..16);
    assert!(doc
        .projection()
        .blocks()
        .iter()
        .all(|block| block.style == StyleId::from("Block quote")));
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn quote_assignment_is_local_and_restores_exact_source_on_removal_and_history() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for ending in ["\n", "\r\n"] {
            let source =
                format!("Before{ending}{ending}__first__{ending}second{ending}{ending}After");
            let mut doc = document(&source, format);
            let at = doc.text().find("first").unwrap();
            doc.set_paragraph_style(at..at, "Block quote".into())
                .unwrap();
            let expected =
                format!("Before{ending}{ending}> __first__{ending}> second{ending}{ending}After");
            assert_eq!(doc.source_bytes(), expected.as_bytes(), "{format:?}");
            let reopened = document(&expected, format);
            assert_eq!(reopened.text(), doc.text());
            assert_eq!(
                reopened.projection().blocks()[1].style,
                "Block quote".into()
            );
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), expected.as_bytes());
            let at = doc.text().find("first").unwrap();
            doc.set_paragraph_style(at..at, "Paragraph".into()).unwrap();
            assert_eq!(doc.source_bytes(), source.as_bytes(), "{format:?}");
        }
    }
}

#[test]
fn quote_removal_preserves_body_structure_and_neighbor_boundaries() {
    for (source, expected) in [
        ("before\n> quote\n\nafter", "before\n\nquote\n\nafter"),
        ("> # heading\n>\n> - item", "heading\n\nitem"),
        ("> first\n>\n> second", "first\n\nsecond"),
        ("> ```\n> > literal\n> ```", "\\> literal"),
    ] {
        let mut doc = document(source, Format::Markdown);
        let before = doc.text().to_owned();
        let at = doc
            .projection()
            .blocks()
            .iter()
            .find(|block| block.style.0 == "Block quote")
            .unwrap()
            .range
            .start;
        doc.set_paragraph_style(at..doc.text().len(), "Paragraph".into())
            .unwrap();
        assert_eq!(doc.source_bytes(), expected.as_bytes(), "{source:?}");
        assert_eq!(doc.text(), before);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn enter_continues_a_quoted_prose_paragraph_with_exact_history() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "> first";
        let mut core = Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        for event in [
            InputEvent::Key(Key::Char('A')),
            InputEvent::Key(Key::Enter),
            InputEvent::text("second"),
            InputEvent::Key(Key::Escape),
        ] {
            core.handle(view, CoreEvent::Input(event)).unwrap();
        }
        assert_eq!(
            core.document().source_bytes(),
            b"> first\n>\n> second",
            "{format:?}"
        );
        assert!(core
            .document()
            .projection()
            .blocks()
            .iter()
            .all(|block| block.style.0 == "Block quote"));
        let saved = core.document().source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}

#[test]
fn quote_mapping_and_edits_preserve_utf16_bom_and_non_ascii_source() {
    for (encoding, little) in [(Encoding::Utf16Le, true), (Encoding::Utf16Be, false)] {
        let encode = |text: &str| {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in text.encode_utf16() {
                bytes.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            bytes
        };
        for format in [Format::Markdown, Format::MarkdownSource] {
            let original = encode("é العربية\r\n\r\nTail");
            let quoted = encode("> é العربية\r\n\r\nTail");
            let mut doc = Document::from_bytes(original.clone(), encoding, format).unwrap();
            doc.set_paragraph_style(0..0, "Block quote".into()).unwrap();
            assert_eq!(doc.source_bytes(), quoted);
            let reopened = Document::from_bytes(quoted.clone(), encoding, format).unwrap();
            assert_eq!(doc.text(), reopened.text());
            doc.set_paragraph_style(0..0, "Paragraph".into()).unwrap();
            assert_eq!(doc.source_bytes(), original);
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), quoted);
        }
    }
}

#[test]
fn empty_quote_paragraph_has_a_valid_caret_and_can_return_to_plain_prose() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document("", format);
        doc.set_paragraph_style(0..0, "Block quote".into()).unwrap();
        assert_eq!(doc.source_bytes(), b"> ");
        assert_eq!(doc.projection().blocks()[0].style, "Block quote".into());
        doc.text_point(doc.text().len()).unwrap();
        doc.set_paragraph_style(0..0, "Paragraph".into()).unwrap();
        assert_eq!(doc.source_bytes(), b"");
        assert_eq!(doc.projection().blocks()[0].style, "Paragraph".into());
    }
}

#[test]
fn late_quote_assignment_declares_only_local_source_prefix_patches() {
    let before = "earlier untouched paragraph\n\n".repeat(2_000);
    let source = format!("{before}__target__\ncontinuation\n\nunchanged tail");
    let mut doc = document(&source, Format::Markdown);
    let at = doc.text().find("target").unwrap();
    let revision = doc.revision();
    let history = doc.history_status();
    let prepared = doc
        .prepare_model_request(ModelRequest::SetParagraphStyle {
            document: doc.id(),
            revision,
            range: at..at,
            style: "Block quote".into(),
        })
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 2);
    for patch in prepared.summary().source_patches() {
        assert!(patch.range().is_empty());
        assert!(patch.range().start >= before.len());
        assert!(patch.range().start < before.len() + "__target__\ncontinuation".len());
        assert_eq!(patch.replacement(), b"> ");
    }
    assert_eq!(doc.source_bytes(), source.as_bytes());
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_status().current, history.current);
    doc.commit_model_transaction(prepared).unwrap();
    assert_eq!(
        doc.source_bytes(),
        format!("{before}> __target__\n> continuation\n\nunchanged tail").as_bytes()
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn quoted_list_and_code_enter_keep_their_native_body_structure() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (source, expected, code) in [
            ("> - first", "> - first\n> - second", false),
            ("> 3. first", "> 3. first\n> 4. second", false),
            (
                "> ```\n> code\n> ```",
                "> ```\n> code\n> second\n> ```",
                true,
            ),
        ] {
            let mut core = Core::new(document(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 500., 150.);
            if code && format == Format::MarkdownSource {
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('j'))))
                    .unwrap();
            }
            for event in [
                InputEvent::Key(Key::Char('A')),
                InputEvent::Key(Key::Enter),
                InputEvent::text("second"),
                InputEvent::Key(Key::Escape),
            ] {
                let description = format!("{event:?}");
                core.handle(view, CoreEvent::Input(event))
                    .unwrap_or_else(|error| {
                        panic!(
                            "{error:?}: {format:?}, {source:?}, {description}, bytes={:?}",
                            core.document().source_bytes()
                        )
                    });
            }
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?}"
            );
            assert_eq!(document(expected, format).text(), core.document().text());
        }
    }
}

#[test]
fn literal_quote_punctuation_typed_in_wysiwyg_stays_text() {
    let mut core = Core::new(document("", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text("> literal")))
        .unwrap();
    assert_eq!(core.document().text(), "> literal");
    assert_eq!(core.document().source_bytes(), b"\\> literal");
    assert_eq!(
        core.document().projection().blocks()[0].style,
        "Paragraph".into()
    );
}

#[test]
fn quoted_prose_after_a_closed_fence_still_enters_a_paragraph() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "> ```\n> code\n> ```\n> prose";
        let mut core = Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 250.);
        let at = core.document().text().find("prose").unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: viem_core::document::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        for event in [
            InputEvent::Key(Key::Char('A')),
            InputEvent::Key(Key::Enter),
            InputEvent::text("second"),
        ] {
            core.handle(view, CoreEvent::Input(event)).unwrap();
        }
        assert_eq!(
            core.document().source_bytes(),
            b"> ```\n> code\n> ```\n> prose\n>\n> second"
        );
    }
}

#[test]
fn enter_on_an_empty_quoted_list_item_exits_its_visible_paragraph_style() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in ["> - ", "> 1. ", "> - first\n> - "] {
            let mut core = Core::new(document(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 500., 250.);
            for event in [
                InputEvent::Key(Key::Char('i')),
                InputEvent::Key(Key::DocumentEnd),
                InputEvent::Key(Key::Enter),
                InputEvent::text("prose"),
                InputEvent::Key(Key::Escape),
            ] {
                let description = format!("{event:?}");
                core.handle(view, CoreEvent::Input(event))
                    .unwrap_or_else(|error| {
                        panic!("{error:?}: {format:?}, {source:?}, {description}, bytes={:?}, blocks={:?}", core.document().source_bytes(), core.document().projection().blocks())
                    });
            }
            let last = core.document().projection().blocks().last().unwrap();
            assert_eq!(last.kind, BlockKind::Paragraph);
            assert_eq!(last.style.0, if format == Format::Markdown { "Paragraph" } else { "Block quote" });
            let saved = core.document().source_bytes();
            let visible =
                Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(
                visible.projection().blocks().last().unwrap().kind,
                BlockKind::Paragraph
            );
            assert!(visible.text().ends_with("prose"));
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
                .unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                .unwrap();
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}
