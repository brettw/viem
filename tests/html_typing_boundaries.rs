use evim_core::command::{InputEvent, Key};
use evim_core::document::{
    BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest, SemanticInlineStyle,
};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent};

fn fixture(source: &str, format: Format) -> (Core<MockTextMeasurementProvider>, evim_core::ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    (core, view)
}
fn input(core: &mut Core<MockTextMeasurementProvider>, view: evim_core::ViewId, event: InputEvent) {
    let description = format!(
        "{event:?} at {} in {}",
        core.command_state(view).unwrap().cursor(),
        String::from_utf8_lossy(&core.document().source_bytes())
    );
    core.handle(view, CoreEvent::Input(event))
        .unwrap_or_else(|error| panic!("{description}: {error:?}"));
}
#[test]
fn trailing_blank_lines_accept_return_and_open_below() {
    for source in [
        "word",
        "<p>word</p>",
        "<p><b>word</b></p><!--keep-->",
        "<div>word<br></div>",
        "<p>word<br></p>",
        "",
        "<p></p>",
    ] {
        for open in [false, true] {
            let (mut core, view) = fixture(source, Format::Html);
            let original = core.document().text().to_owned();
            input(&mut core, view, InputEvent::key('G'));
            input(
                &mut core,
                view,
                InputEvent::key(if open { 'o' } else { 'A' }),
            );
            if !open {
                input(&mut core, view, InputEvent::Key(Key::Enter));
            }
            input(&mut core, view, InputEvent::Key(Key::Enter));
            assert_eq!(
                core.document().text(),
                format!("{original}\n\n"),
                "{source} open {open}"
            );
            input(&mut core, view, InputEvent::text("tail"));
            assert_eq!(core.document().text(), format!("{original}\n\ntail"));
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
            let reopened =
                Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html)
                    .unwrap();
            assert_eq!(reopened.text(), format!("{original}\n\ntail"));
        }
    }
}

#[test]
fn toggle_off_crosses_closing_tags_and_preserves_other_inline_properties() {
    for (source, at, style, bold, italic, expected) in [
        (
            "<p><b>word</b></p><!--keep-->",
            4,
            SemanticInlineStyle::Strong,
            false,
            false,
            "<p><b>word</b>X</p><!--keep-->",
        ),
        (
            "<p><i>word</i></p>",
            4,
            SemanticInlineStyle::Emphasis,
            false,
            false,
            "<p><i>word</i>X</p>",
        ),
        (
            "<p><b><b>word</b></b></p>",
            4,
            SemanticInlineStyle::Strong,
            false,
            false,
            "<p><b><b>word</b></b>X</p>",
        ),
        (
            "<p><b><i>word</i></b></p>",
            4,
            SemanticInlineStyle::Strong,
            false,
            true,
            "<p><b><i>word</i></b><i>X</i></p>",
        ),
        (
            "<p><i><b>word</b></i></p>",
            4,
            SemanticInlineStyle::Strong,
            false,
            true,
            "<p><i><b>word</b>X</i></p>",
        ),
        (
            "<p><b><i>word</i></b></p>",
            4,
            SemanticInlineStyle::Emphasis,
            true,
            false,
            "<p><b><i>word</i>X</b></p>",
        ),
        (
            "<p><b></b></p>",
            0,
            SemanticInlineStyle::Strong,
            false,
            false,
            "<p><b></b>X</p>",
        ),
    ] {
        for format in [Format::Html, Format::HtmlSource] {
            let (mut core, view) = fixture(source, format);
            input(&mut core, view, InputEvent::key('i'));
            let offset = if format == Format::HtmlSource {
                source.find("</").unwrap()
            } else {
                at
            };
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: offset,
                    affinity: BoundaryAffinity::Upstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let expected_selection = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected: expected_selection,
                    style,
                    enabled: false,
                },
            )
            .unwrap();
            assert_eq!(
                core.document().source_bytes(),
                source.as_bytes(),
                "toggle itself must not dirty source"
            );
            if format == Format::HtmlSource {
                let close = if style == SemanticInlineStyle::Strong {
                    "</b>"
                } else {
                    "</i>"
                };
                let last = source.rfind(close).unwrap() + close.len();
                assert_eq!(core.command_state(view).unwrap().cursor(), last, "{source}");
            } else {
                assert_eq!(core.command_state(view).unwrap().cursor(), offset);
            }
            input(&mut core, view, InputEvent::text("X"));
            assert_eq!(
                String::from_utf8(core.document().source_bytes()).unwrap(),
                expected,
                "{format:?}"
            );
            let caret = core.command_state(view).unwrap().cursor();
            let style = DocumentLayoutStyles::semantic_character_at(
                core.document().projection(),
                caret,
                true,
            )
            .unwrap();
            assert_eq!(style.bold, bold);
            assert_eq!(
                style.slant != evim_core::document::FontSlant::Upright,
                italic
            );
            input(&mut core, view, InputEvent::text("Y"));
            input(&mut core, view, InputEvent::Key(Key::Escape));
            let changed = core.document().source_bytes();
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
            assert_eq!(core.document().source_bytes(), changed);
        }
    }
}

#[test]
fn toggle_off_in_middle_splits_only_inline_scope_and_keeps_suffix_styled() {
    for format in [Format::Html, Format::HtmlSource] {
        let source = "<p><b data-keep='yes'><i>word</i></b></p><!--keep-->";
        let (mut core, view) = fixture(source, format);
        input(&mut core, view, InputEvent::key('i'));
        let at = if format == Format::HtmlSource {
            source.find("word").unwrap() + 2
        } else {
            2
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
        input(&mut core, view, InputEvent::text("X"));
        assert_eq!(String::from_utf8(core.document().source_bytes()).unwrap(), "<p><b data-keep='yes'><i>wo</i></b><i>X</i><b data-keep='yes'><i>rd</i></b></p><!--keep-->");
        let reopened =
            Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html)
                .unwrap();
        assert_eq!(reopened.text(), "woXrd");
        for (at, bold) in [(0, true), (2, false), (3, true)] {
            let style =
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false)
                    .unwrap();
            assert_eq!(style.bold, bold);
            assert_ne!(style.slant, evim_core::document::FontSlant::Upright);
        }
    }
}

#[test]
fn direct_decoration_exit_and_unclosed_emphasis_keep_other_properties() {
    use evim_core::document::{StyleProperty as P, StylePropertyValue as V};
    for (source, property) in [
        ("<p><u><s>word</s></u></p>", P::CharacterUnderline),
        ("<p><s><u>word</u></s></p>", P::CharacterStrikethrough),
        (
            "<p><strong style='color:red' lang='fr'>word</strong></p>",
            P::CharacterBold,
        ),
        ("<b>word", P::CharacterBold),
    ] {
        for format in [Format::Html, Format::HtmlSource] {
            let (mut core, view) = fixture(source, format);
            input(&mut core, view, InputEvent::key('i'));
            let offset = if format == Format::HtmlSource {
                source.find("word").unwrap() + 4
            } else {
                4
            };
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: offset,
                    affinity: BoundaryAffinity::Upstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::EditDirectProperty {
                    expected,
                    property,
                    value: Some(V::Boolean(false)),
                },
            )
            .unwrap();
            input(&mut core, view, InputEvent::text("XY"));
            let reopened =
                Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html)
                    .unwrap();
            assert_eq!(reopened.text(), "wordXY");
            let style =
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), 4, false)
                    .unwrap();
            assert!(!style.bold);
            assert_eq!(style.underline, property == P::CharacterStrikethrough);
            assert_eq!(style.strikethrough, property == P::CharacterUnderline);
            if source.contains("color:red") {
                assert_eq!(style.foreground.red, 1.);
                assert_eq!(style.language.as_deref(), Some("fr"));
            }
            assert!(
                !String::from_utf8_lossy(&core.document().source_bytes()).contains("--evim-bold")
            );
        }
    }
}

#[test]
fn local_toggle_split_keeps_distant_block_identity_and_cached_layout() {
    use evim_core::document::{
        FormattedPayloadEdit, FormattedTextPayload, StyleProperty as P, StylePropertyValue as V,
    };
    use evim_core::layout::{LayoutEngine, ViewLayout};
    let source = "<p>Unchanged paragraph</p>".repeat(10_000) + "<p><b>word</b></p><!--tail-->";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let unaffected = document.projection().blocks()[5000].id;
    let at = document.text().len();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(10_010);
    let mut view = ViewLayout::new(300., 100.);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "X", vec![]).unwrap();
    let (prepared, caret) = document
        .prepare_insertion_with_typing_properties(
            FormattedPayloadEdit::new(at..at, payload)
                .with_boundary_affinity(BoundaryAffinity::Upstream),
            &[(P::CharacterBold, V::Boolean(false))],
        )
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(prepared.summary().source_patches()[0].replacement(), b"X");
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(caret, at + 1);
    assert_eq!(document.projection().blocks()[5000].id, unaffected);
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
}

#[test]
fn append_menu_toggle_uses_current_style_and_exits_before_whitespace() {
    let source = "<html><body><p><i><b>Bold</b></i></p></body></html>";
    let (mut core, view) = fixture(source, Format::Html);
    input(&mut core, view, InputEvent::key('A'));
    let presentation = core
        .selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
        .unwrap();
    assert_eq!(presentation.state(), evim_core::SemanticStyleState::On);
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected: presentation.selection().unwrap().clone(),
            style: SemanticInlineStyle::Strong,
            enabled: presentation.state() != evim_core::SemanticStyleState::On,
        },
    )
    .unwrap();
    input(&mut core, view, InputEvent::text(" plain"));
    let bytes = core.document().source_bytes();
    let source = String::from_utf8(bytes.clone()).unwrap();
    assert!(source.contains("<b>Bold</b>"), "{source}");
    let reopened = Document::from_bytes(bytes, Encoding::Utf8, Format::Html).unwrap();
    let style =
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), 5, false).unwrap();
    assert!(!style.bold);
    assert_eq!(style.slant, evim_core::document::FontSlant::Italic);
}
