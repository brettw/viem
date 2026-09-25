use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, FontSlant, HistoryNavigationRequest, SemanticInlineStyle,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, event: InputEvent) {
    core.handle(view, CoreEvent::Input(event)).unwrap();
}

#[test]
fn markdown_off_exits_exact_closing_markers_and_preserves_nested_emphasis() {
    for (source, disabled, expected_exit, bold, italic) in [
        ("**word**", SemanticInlineStyle::Strong, 8, false, false),
        ("__word__", SemanticInlineStyle::Strong, 8, false, false),
        ("*word*", SemanticInlineStyle::Emphasis, 6, false, false),
        ("_word_", SemanticInlineStyle::Emphasis, 6, false, false),
        ("**_word_**", SemanticInlineStyle::Strong, 10, false, true),
        ("_**word**_", SemanticInlineStyle::Strong, 9, false, true),
        ("**_word_**", SemanticInlineStyle::Emphasis, 8, true, false),
        ("_**word**_", SemanticInlineStyle::Emphasis, 10, true, false),
        ("***word***", SemanticInlineStyle::Strong, 10, false, true),
        ("___word___", SemanticInlineStyle::Emphasis, 10, true, false),
        (
            "**__word__**",
            SemanticInlineStyle::Strong,
            12,
            false,
            false,
        ),
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut core = Core::new(
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
            );
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
            input(&mut core, view, InputEvent::key('i'));
            let at = if format == Format::MarkdownSource {
                source.find("word").unwrap() + 4
            } else {
                4
            };
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: BoundaryAffinity::Upstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let revision = core.document().revision();
            let history = core.document().history_status().current;
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style: disabled,
                    enabled: false,
                },
            )
            .unwrap_or_else(|error| panic!("toggle {format:?} {source}: {error:?}"));
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                if format == Format::MarkdownSource {
                    expected_exit
                } else {
                    at
                },
                "{format:?} {source}"
            );
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status().current, history);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            for value in ["X", "Y"] {
                core.handle(view, CoreEvent::Input(InputEvent::text(value)))
                    .unwrap_or_else(|error| panic!("{format:?} {source} {disabled:?}: {error:?}"));
            }
            let saved = core.document().source_bytes();
            let reopened =
                Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(
                reopened.text(),
                "wordXY",
                "{format:?} {source} {:?}",
                String::from_utf8_lossy(&saved)
            );
            let styled =
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), 4, false)
                    .unwrap();
            assert_eq!(
                styled.bold,
                bold,
                "{format:?} {source} {:?}",
                String::from_utf8_lossy(&saved)
            );
            assert_eq!(
                styled.slant != FontSlant::Upright,
                italic,
                "{format:?} {source} {:?}",
                String::from_utf8_lossy(&saved)
            );
            input(&mut core, view, InputEvent::Key(Key::Escape));
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}

#[test]
fn markdown_off_in_middle_does_not_skip_text_or_remove_suffix_emphasis() {
    for source in [
        "**word**",
        "__word__",
        "**_word_**",
        "_**word**_",
        "**__word__**",
        "***word***",
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut core = Core::new(
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
            );
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
            input(&mut core, view, InputEvent::key('i'));
            let at = if format == Format::MarkdownSource {
                source.find("word").unwrap() + 2
            } else {
                2
            };
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: BoundaryAffinity::Upstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style: SemanticInlineStyle::Strong,
                    enabled: false,
                },
            )
            .unwrap();
            assert_eq!(core.command_state(view).unwrap().cursor(), at);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::text("X")))
                .unwrap_or_else(|error| panic!("{format:?} {source}: {error:?}"));
            let reopened = Document::from_bytes(
                core.document().source_bytes(),
                Encoding::Utf8,
                Format::Markdown,
            )
            .unwrap();
            assert_eq!(reopened.text(), "woXrd");
            for (offset, expected) in [(0, true), (2, false), (3, true)] {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        reopened.projection(),
                        offset,
                        false
                    )
                    .unwrap()
                    .bold,
                    expected,
                    "{format:?} {source}"
                );
            }
            if source.contains('_') && !source.starts_with("__") && source != "**__word__**"
                || source.starts_with("***")
            {
                for offset in [0, 2, 3] {
                    assert_eq!(
                        DocumentLayoutStyles::semantic_character_at(
                            reopened.projection(),
                            offset,
                            false
                        )
                        .unwrap()
                        .slant,
                        FontSlant::Italic,
                        "{format:?} {source}"
                    );
                }
            }
            let saved = core.document().source_bytes();
            input(&mut core, view, InputEvent::Key(Key::Escape));
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), saved);
        }
    }
}

#[test]
fn source_exit_uses_marker_boundaries_next_to_unicode_and_ignores_literal_marker_text() {
    for (source, at, expected) in [
        ("*👩🏽‍💻*", "*👩🏽‍💻".len(), "*👩🏽‍💻*".len()),
        ("_é_", "_é".len(), "_é_".len()),
        ("`*literal*`", 9, 9),
        ("\\*literal\\*", 9, 9),
        ("*unclosed", 9, 9),
        ("", 0, 0),
    ] {
        let mut core = Core::new(
            Document::from_bytes(
                source.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::MarkdownSource,
            )
            .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        input(&mut core, view, InputEvent::key('i'));
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let selection = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected: selection,
                style: SemanticInlineStyle::Emphasis,
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            expected,
            "{source}"
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let source = "*word*\u{301}";
    let mut core = Core::new(
        Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    input(&mut core, view, InputEvent::key('i'));
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 5,
            affinity: BoundaryAffinity::Upstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    assert!(core
        .handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style: SemanticInlineStyle::Emphasis,
                enabled: false
            }
        )
        .is_err());
    assert_eq!(core.command_state(view).unwrap().cursor(), 5);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn source_exit_is_recorded_by_counted_insert_and_dot_as_one_undo_unit() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core =
            Core::new(Document::from_bytes(b"**word**".to_vec(), Encoding::Utf8, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let at = if format == Format::MarkdownSource {
            5
        } else {
            3
        };
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
        input(&mut core, view, InputEvent::key('2'));
        input(&mut core, view, InputEvent::key('a'));
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style: SemanticInlineStyle::Strong,
                enabled: false,
            },
        )
        .unwrap();
        input(&mut core, view, InputEvent::text("X"));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_eq!(core.document().source_bytes(), b"**word**XX", "{format:?}");
        input(&mut core, view, InputEvent::key('.'));
        assert_eq!(
            core.document().source_bytes(),
            b"**word**XXXX",
            "{format:?}"
        );
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), b"**word**XX");
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), b"**word**");
    }
}

#[test]
fn nested_markdown_split_keeps_large_document_projection_and_layout_local() {
    use viem_core::document::{
        FormattedPayloadEdit, FormattedTextPayload, StyleProperty, StylePropertyValue,
    };
    use viem_core::layout::{LayoutEngine, ViewLayout};
    let source = "Unchanged paragraph\n\n".repeat(10_000) + "**_word_**";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let untouched = document.projection().blocks()[5000].id;
    let at = document.projection().text_tree().byte_len() - 2;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(10_010);
    let mut view = ViewLayout::new(300.0, 100.0);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "X", vec![]).unwrap();
    let (prepared, _) = document
        .prepare_insertion_with_typing_properties(
            FormattedPayloadEdit::new(at..at, payload)
                .with_boundary_affinity(BoundaryAffinity::Upstream),
            &[(
                StyleProperty::CharacterBold,
                StylePropertyValue::Boolean(false),
            )],
        )
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[5000].id, untouched);
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(
        document.projection().style_spans(),
        reopened.projection().style_spans()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
