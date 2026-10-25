use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::*;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn apply(document: &mut Document, range: std::ops::Range<usize>) {
    let request = ModelRequest::AssignNamedStyle {
        document: document.id(),
        revision: document.revision(),
        range,
        namespace: StyleNamespace::Character,
        style: "Comment".into(),
    };
    let prepared = document.prepare_model_request(request).unwrap();
    document.commit_model_transaction(prepared).unwrap();
}

#[test]
fn comment_choice_wraps_visible_selection_and_preserves_exact_history() {
    for (source, range, expected) in [
        ("word", 0..4, "<!-- word -->"),
        ("before word after", 7..11, "before <!-- word --> after"),
        ("word **after**", 0..4, "<!-- word --> after"),
        (
            "before\nword **after**",
            7..11,
            "before <!-- word --> after",
        ),
        ("# word", 0..4, "<!-- word -->"),
        ("before **word** after", 7..11, "before <!-- word --> after"),
        ("before `word` after", 7..11, "before <!-- word --> after"),
        ("- word", 0..4, "<!-- word -->"),
        ("> word", 0..4, "<!-- word -->"),
        ("one\n\ntwo", 0..7, "<!-- one\ntwo -->"),
        ("one\n\ntwo\n\nthree", 1..6, "o<!-- ne\ntw -->o\nthree"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        apply(&mut document, range);
        assert_eq!(document.text(), expected, "{source}");
        let changed = document.source_bytes();
        let reopened =
            Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), expected);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn caret_comment_inserts_immediately_then_types_inside_one_span() {
    for source in [
        "",
        "tail",
        "before tail",
        "**tail**",
        "`tail`",
        "- tail",
        "> tail",
        "# tail",
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: "Comment".into(),
            },
        )
        .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(core.document().text().starts_with("<!--  -->"));
        assert_eq!(core.command_state(view).unwrap().cursor(), 5);
        let empty_comment = core.document().source_bytes();
        assert_eq!(
            core.selected_named_styles(view).unwrap().character,
            Some("Comment".into())
        );
        for text in ["he", "llo"] {
            core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                .unwrap_or_else(|error| panic!("{source:?} {text:?}: {error:?}"));
        }
        assert!(
            core.document().text().starts_with("<!-- hello -->"),
            "{source:?}: {:?}",
            core.document().text()
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), 10);
        let changed = core.document().source_bytes();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), empty_comment);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), changed);
    }
}

#[test]
fn comment_choice_rejects_unsafe_or_literal_owners_atomically() {
    for (source, format) in [
        ("a --> b", Format::Markdown),
        ("```\nword\n```", Format::Markdown),
        ("word", Format::PlainText),
        ("word", Format::Code),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let request = ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 0..document.text().len(),
            namespace: StyleNamespace::Character,
            style: "Comment".into(),
        };
        assert!(document.prepare_model_request(request).is_err(), "{source}");
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.history_status().can_undo);
    }
}

#[test]
fn comment_assignment_preserves_encoding_bom_and_unselected_line_endings() {
    fn bytes(text: &str, encoding: Encoding) -> Vec<u8> {
        match encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
            Encoding::Utf16Le => [
                vec![0xff, 0xfe],
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat(),
            _ => unreachable!(),
        }
    }
    let source = "before café after\r\n\r\ntail\n";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Latin1] {
        let original = bytes(source, encoding);
        let mut document =
            Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
        apply(&mut document, 7..12);
        assert_eq!(
            document.source_bytes(),
            bytes(&source.replace("café", "<!-- café -->"), encoding)
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn comment_assignment_is_local_and_reapplying_it_is_a_no_op() {
    for paragraphs in [10, 2000] {
        let source = format!(
            "word tail\n\n{}",
            "unrelated paragraph\n\n".repeat(paragraphs)
        );
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::AssignNamedStyle {
                document: document.id(),
                revision: document.revision(),
                range: 0..4,
                namespace: StyleNamespace::Character,
                style: "Comment".into(),
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert!(work.decoded_source_bytes() < 150, "{work:?}");
        assert_eq!(work.full_text_bytes_materialized(), 0);
        document.commit_model_transaction(prepared).unwrap();
        let revision = document.revision();
        apply(&mut document, 5..9);
        assert_eq!(document.revision(), revision);
    }
}

#[test]
fn created_comment_preserves_literal_punctuation() {
    let mut core =
        Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    let choose = |core: &mut Core<MockTextMeasurementProvider>, name: &str| {
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: name.into(),
            },
        )
        .unwrap();
    };
    choose(&mut core, "Comment");
    core.handle(
        view,
        CoreEvent::Input(InputEvent::text("**literal** &amp; `code`")),
    )
    .unwrap();
    assert_eq!(core.document().text(), "<!-- **literal** &amp; `code` -->");
    assert_eq!(
        core.document().source_bytes(),
        b"<!-- **literal** &amp; `code` -->"
    );
}

#[test]
fn comment_choice_formats_selected_table_cells() {
    use viem_core::command::TableSelectionExtent;
    let source = "| one | two |\n| --- | --- |\n| red | blue |";
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    let table = core.document().projection().tables()[0].id;
    core.select_table_cells(
        view,
        core.document().id(),
        core.document().revision(),
        Some(TableSelectionExtent {
            table,
            anchor_row: 0,
            active_row: 1,
            anchor_column: 0,
            active_column: 0,
        }),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: "Comment".into(),
        },
    )
    .unwrap();
    assert_eq!(
        core.document().text(),
        "<!-- one -->\ntwo\n<!-- red -->\nblue"
    );
    assert!(core.table_selection(view).unwrap().is_some());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn caret_comment_leaves_replace_mode_and_preserves_following_text() {
    for (source, suffix) in [
        ("abcdef", "abcdef"),
        ("a", "a"),
        ("", ""),
        ("**abc**def", "abcdef"),
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core.handle(view, CoreEvent::Input(InputEvent::text("R")))
            .unwrap();
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: "Comment".into(),
            },
        )
        .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.command_state(view).unwrap().cursor(), 5);
        let empty_comment = core.document().source_bytes();
        for text in ["xy", "z"] {
            core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                .unwrap_or_else(|error| panic!("{source:?} {text:?}: {error:?}"));
        }
        assert_eq!(core.document().text(), format!("<!-- xyz -->{suffix}"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(core.document().text(), format!("<!-- xy -->{suffix}"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), empty_comment);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_comment_can_be_undone_before_typing() {
    let mut core =
        Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: "Comment".into(),
        },
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"<!--  -->");
    assert_eq!(core.command_state(view).unwrap().cursor(), 5);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert!(core.document().source_bytes().is_empty());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"<!--  -->");
}

#[test]
fn rejected_caret_comment_keeps_source_mode_and_history() {
    for source in ["```\nword\n```", "---\nkey: value\n---"] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        let revision = core.document().revision();
        let result = core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: "Comment".into(),
            },
        );
        assert!(result.is_err(), "{source}");
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert!(!core.document().history_status().can_undo);
    }
}

#[test]
fn unfinished_comment_assignment_reports_source_repair_without_changing_history() {
    let original = b"<!-- hello -->";
    let mut document =
        Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    document.delete(13..14).unwrap();
    let revision = document.revision();
    let error = document
        .prepare_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision,
            range: 5..10,
            namespace: StyleNamespace::Character,
            style: "Comment".into(),
        })
        .err()
        .unwrap();
    let ModelTransactionError::Document(error) = error else {
        panic!("{error:?}")
    };
    assert!(matches!(error, DocumentError::UnfinishedComment));
    assert_eq!(
        error.to_string(),
        "Can’t create a comment inside an unfinished comment, please fix the source."
    );
    assert_eq!(document.revision(), revision);
    assert_eq!(document.source_bytes(), b"<!-- hello --");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn complete_comment_with_trailing_text_does_not_report_an_unfinished_comment() {
    let document = Document::from_bytes(
        b"<!-- hello --> tail".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let error = document
        .prepare_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 5..10,
            namespace: StyleNamespace::Character,
            style: "Comment".into(),
        })
        .err()
        .unwrap();
    assert!(matches!(
        error,
        ModelTransactionError::Document(DocumentError::UnsupportedFormatting)
    ));
}

#[test]
fn unfinished_inline_comment_error_survives_lost_styling_and_literal_closer_input() {
    for original in [
        "<!-- hello -->",
        "before <!-- hello --> after",
        "<span><!-- hello --></span>tail",
        "<!-- previous -->\n\n<!-- hello -->",
    ] {
        for restore in [false, true] {
            let mut document = Document::from_bytes(
                original.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::Markdown,
            )
            .unwrap();
            let hello = document.text().find("hello").unwrap();
            let end = hello + "hello --".len();
            document.delete(end..end + 1).unwrap();
            if restore {
                document.insert(end, ">").unwrap();
            }
            let at = document.text().find("hello").unwrap();
            let before = document.source_bytes();
            let revision = document.revision();
            let result = document.prepare_model_request(ModelRequest::AssignNamedStyle {
                document: document.id(),
                revision,
                range: at..at + 5,
                namespace: StyleNamespace::Character,
                style: "Comment".into(),
            });
            // A standalone HTML comment accepts its closer literally and is complete again.
            if restore && original == "<!-- hello -->" {
                assert!(result.unwrap().is_no_op());
            } else if restore && original.starts_with("<!-- previous") {
                assert!(result.unwrap().is_no_op());
            } else {
                assert!(
                    matches!(
                        result,
                        Err(ModelTransactionError::Document(
                            DocumentError::UnfinishedComment
                        ))
                    ),
                    "{original:?} restore={restore}: {:?}",
                    result.err()
                );
            }
            assert_eq!(document.source_bytes(), before);
            assert_eq!(document.revision(), revision);
        }
    }
}
