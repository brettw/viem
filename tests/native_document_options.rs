use evim_core::command::{InputEvent, Key};
use evim_core::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, Encoding, FileFormat, Format,
    HistoryNavigationRequest, ListStyle, MappingOutcome, ModelRequest, ModelTransactionError,
};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreError, CoreEvent};

#[test]
fn switching_into_html_installs_source_styles_and_switching_back_uses_plain_styles() {
    let mut document = Document::new("<p style='font-size: 30pt'>Large</p>");
    document.set_format(Format::Html).unwrap();
    assert_eq!(document.text(), "Large");
    assert!(document.projection().style_spans().iter().any(|span|matches!(&span.application,evim_core::document::StyleApplication::Direct(properties)if properties.size==Some(30.0))));
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .block_style_metadata(&"Document".into())
            .unwrap()
            .origin,
        evim_core::document::StyleDefinitionOrigin::SourceBacked
    );
    document.set_format(Format::PlainText).unwrap();
    assert_eq!(document.text(), "<p style='font-size: 30pt'>Large</p>");
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .block_style_metadata(&"Document".into())
            .unwrap()
            .origin,
        evim_core::document::StyleDefinitionOrigin::GeneratedConfiguration
    );
    assert!(document.undo());
    assert_eq!(document.format(), Format::Html);
}

#[test]
fn format_switch_with_identical_visible_text_including_empty_is_undoable() {
    for text in ["", "plain text", "__visible__"] {
        let mut document = Document::new(text);
        document.set_format(Format::MarkdownSource).unwrap();
        assert_eq!(document.text(), text);
        assert_eq!(document.source_bytes(), text.as_bytes());
        assert_eq!(document.format(), Format::MarkdownSource);
        assert!(document.undo());
        assert_eq!(document.format(), Format::PlainText);
        if !text.contains('_') {
            document.set_format(Format::Markdown).unwrap();
            assert_eq!(document.text(), text);
            assert!(document.undo());
        }
    }
}

#[test]
fn format_switch_preserves_source_and_anchors_between_distant_markers() {
    let original = b"# Heading\r\n__bold__ middle *tail*\r\n";
    let mut document = Document::from_bytes_with_file_format(
        original.to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Dos,
    )
    .unwrap();
    let offset = document.text().find("middle").unwrap();
    let anchor = document
        .text_anchor(
            document.text_point(offset).unwrap(),
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::SetFormat {
            document: document.id(),
            revision: document.revision(),
            target: Format::Markdown,
        })
        .unwrap();
    assert!(prepared.summary().source_patches().is_empty());
    assert!(prepared.summary().formatted_splices().len() >= 3);
    let mapped = prepared
        .text_position_map()
        .map_text_anchor(anchor)
        .unwrap();
    let mapped = match mapped {
        MappingOutcome::Exact(anchor) | MappingOutcome::Moved(anchor) => anchor,
        other => panic!("unchanged source content lost identity: {other:?}"),
    };
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.text(), "Heading\nbold middle tail");
    assert_eq!(mapped.offset(), document.text().find("middle").unwrap());
    assert!(document.undo());
    assert_eq!(document.format(), Format::PlainText);
    assert_eq!(document.source_bytes(), original);
    assert!(document.redo());
    document.set_format(Format::MarkdownSource).unwrap();
    assert_eq!(document.text(), "# Heading\n__bold__ middle *tail*\n");
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn encoding_switch_preserves_unicode_syntax_line_endings_and_undo() {
    let original = b"\xef\xbb\xbf# caf\xc3\xa9\r\n__text__";
    let mut document = Document::from_bytes_with_file_format(
        original.to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    let initial = document.text().to_owned();
    let block_ids = document
        .projection()
        .blocks()
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        document.set_encoding(encoding).unwrap();
        assert_eq!(document.encoding(), encoding);
        assert_eq!(document.text(), initial);
        assert_eq!(document.file_format(), FileFormat::Dos);
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>(),
            block_ids
        );
        let reopened = Document::from_bytes_with_file_format(
            document.source_bytes(),
            encoding,
            Format::Markdown,
            FileFormat::Dos,
        )
        .unwrap();
        assert_eq!(reopened.text(), initial);
        assert!(document.undo());
        assert_eq!(document.encoding(), Encoding::Utf8);
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn invalid_or_stale_options_leave_source_history_and_identity_unchanged() {
    for bytes in [vec![b'a', 0xff]] {
        let mut document = Document::from_bytes(bytes, Encoding::Utf8, Format::PlainText).unwrap();
        let source = document.source_bytes();
        let history = document.history_status();
        assert!(document.set_encoding(Encoding::Latin1).is_err());
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.history_status(), history);
    }
    let mut document = Document::new("text");
    let revision = document.revision();
    document.insert(0, "new ").unwrap();
    let history = document.history_status();
    assert!(matches!(
        document.prepare_model_request(ModelRequest::SetFormat {
            document: document.id(),
            revision,
            target: Format::Markdown,
        }),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(document.history_status(), history);
}

#[test]
fn native_options_relayout_all_views_and_keep_undo_units_separate() {
    let mut core = Core::new(Document::new("__alpha__ beta"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let observer = core.add_view(MockTextMeasurementProvider::new(), 120.0, 100.0);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Text("X".into())))
        .unwrap();
    let document = core.document().id();
    let revision = core.document().revision();
    core.handle(
        view,
        CoreEvent::SetFormat {
            document,
            revision,
            target: Format::Markdown,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "Xalpha beta");
    for attached in [view, observer] {
        assert_eq!(
            core.layout(attached)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document().revision()
        );
    }
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "X__alpha__ beta");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "__alpha__ beta");
}

#[test]
fn native_list_action_uses_exact_current_paragraph_or_visual_range() {
    let mut core = Core::new(Document::new("one\ntwo\nthree"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let stale = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('j'))))
        .unwrap();
    assert_eq!(
        core.handle(
            view,
            CoreEvent::SetListStyle {
                expected: stale,
                style: Some(ListStyle::Bullet),
            }
        ),
        Err(CoreError::StaleLogicalSelection)
    );
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetListStyle {
            expected,
            style: Some(ListStyle::Bullet),
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "one\n- two\nthree");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "one\ntwo\nthree");
    for key in ['g', 'g', 'V', 'j'] {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
    }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetListStyle {
            expected,
            style: Some(ListStyle::Numbered),
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "1. one\n2. two\nthree");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().text(), "one\ntwo\nthree");
}

#[test]
fn rejected_native_format_change_keeps_pending_insert_undo_group_open() {
    let mut core = Core::new(
        Document::from_bytes(b"__word__".to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let original_revision = core.document().revision();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text("X")))
        .unwrap();
    let history = core.document().history_status();
    assert!(core
        .handle(
            view,
            CoreEvent::SetFormat {
                document: core.document().id(),
                revision: original_revision,
                target: Format::Markdown
            }
        )
        .is_err());
    assert_eq!(core.document().history_status(), history);
    core.handle(view, CoreEvent::Input(InputEvent::text("Y")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"XY__word__");
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"__word__");
}
