use evim_core::command::{CommandStatus, InputEvent, Key};
use evim_core::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, DocumentError, Encoding, Format,
    ModelRequest, ModelTransactionError, ProjectionWorkScope, TextEdit,
};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent};

#[test]
fn physical_source_changes_with_identical_projection_have_an_identity_text_map() {
    // Projection seed 4 changed a hidden HTML attribute without moving any
    // visible source ranges, producing an empty list of formatted splices.
    for (source, range, expected) in [
        ("<p x>x</p>", 3..4, "<p y>x</p>"),
        ("<!--x-->", 4..5, "<!--y-->"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let before_text = document.text().to_owned();
        let end = document.text_point(before_text.len()).unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ReplacePhysicalSource {
                document: document.id(),
                revision: document.revision(),
                range: range.clone(),
                replacement: "y".into(),
            })
            .unwrap();
        assert!(prepared.summary().formatted_splices().is_empty());
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(prepared.summary().source_patches()[0].range(), range);
        let mapped = prepared
            .text_position_map()
            .map_text_point(
                end,
                Association::AfterInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .unwrap()
            .value()
            .unwrap()
            .offset();
        assert_eq!(mapped, before_text.len());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(document.text(), before_text);
        assert_eq!(
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html)
                .unwrap()
                .text(),
            before_text
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn markdown_source_delimiters_reparse_blank_line_grammar_with_local_source_patches() {
    for source in [
        "```\nline one\n\nline two\n```\n\nTail.",
        "```\na\n\nb\n```\n\nTail",
    ] {
        let mut document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let before_text = document.text().to_owned();
        let old_b = document.text().find('b');
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(0..2, "")],
            })
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(prepared.summary().source_patches()[0].range(), 0..2);
        assert!(prepared.summary().source_patches()[0]
            .replacement()
            .is_empty());
        let mapped_b = old_b.map(|at| {
            prepared
                .text_position_map()
                .map_text_point(
                    document.text_point(at).unwrap(),
                    Association::AfterInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )
                .unwrap()
                .value()
                .unwrap()
                .offset()
        });
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes()[2..]);
        let fresh = Document::from_bytes(
            document.source_bytes(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(document.text(), fresh.text());
        assert_ne!(document.text(), &before_text[2..]);
        if let Some(mapped_b) = mapped_b {
            assert_eq!(&document.text()[mapped_b..mapped_b + 1], "b");
        }
        let changed_text = document.text().to_owned();
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.text(), before_text);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), source.as_bytes()[2..]);
        assert_eq!(document.text(), changed_text);
    }
}

#[test]
fn ordinary_markdown_source_typing_retains_bounded_regional_projection() {
    let source = "paragraph text\n\n".repeat(20_000);
    let mut document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(5..5, "new")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(prepared).unwrap();
    assert!(document.text().starts_with("paragnewraph text\n"));
}

#[test]
fn markdown_source_typing_before_an_empty_paragraph_keeps_separator_ownership() {
    for (source, expected_text) in [("a\n\n\n\nb", "ax\n\nb"), ("a\n\n\n\n\n\nb", "ax\n\n\nb")] {
        let mut document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(1..1, "x")],
            })
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(prepared.summary().source_patches()[0].range(), 1..1);
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), expected_text);
        let expected_source = format!("ax{}", &source[1..]);
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        let fresh = Document::from_bytes(
            document.source_bytes(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(document.text(), fresh.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
    }
}

#[test]
fn utf16_append_after_a_truncated_code_unit_is_an_atomic_opaque_conflict() {
    for (encoding, source) in [
        (Encoding::Utf16Be, vec![0, b'a', b' ']),
        (Encoding::Utf16Le, vec![b'a', 0, b' ']),
    ] {
        let mut document =
            Document::from_bytes(source.clone(), encoding, Format::PlainText).unwrap();
        assert_eq!(document.text(), "a\u{fffd}");
        let before = document.revision();
        let result = document.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: before,
            edits: vec![TextEdit::new(4..4, "word")],
        });
        assert!(matches!(
            result,
            Err(ModelTransactionError::Document(DocumentError::OpaqueDecodingConflict {
                source_range
            })) if source_range == (2..3)
        ));
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), before);
        document.insert(1, "word").unwrap();
        assert_eq!(document.text(), "aword\u{fffd}");
        assert_eq!(document.source_bytes().last(), Some(&b' '));
        assert!(document.undo());
        document.replace(1..4, "b").unwrap();
        assert_eq!(document.text(), "ab");
        assert!(document.decoding_diagnostics().is_empty());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn sorting_markdown_source_fences_reprojects_their_separator_context() {
    for (source, command, expected) in [
        ("b\n\n```", ":sort", "```\n\nb"),
        ("b\n\n```\n```\n\nTail", ":1,3sort", "```\n\n```\nb\n\nTail"),
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200., 200.);
        for key in command.chars().map(Key::Char).chain([Key::Enter]) {
            let outcome = core
                .handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
            assert!(matches!(
                outcome.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ));
        }
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        let fresh = Document::from_bytes(
            expected.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(core.document().text(), fresh.text());
        core.document()
            .text_point(core.command_state(view).unwrap().cursor())
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}
