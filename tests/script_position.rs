use std::collections::BTreeSet;
use viem_core::command::{InputEvent, Key};
use viem_core::document::*;
use viem_core::layout::{
    DocumentLayoutStyles, LayoutEngine, MockTextMeasurementProvider, ViewLayout,
};
use viem_core::{Core, CoreEvent};

fn open(format: Format, source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn position(document: &Document, at: usize) -> ScriptPosition {
    DocumentLayoutStyles::semantic_character_at(document.projection(), at, false)
        .unwrap()
        .script_position
}
fn apply(
    document: &mut Document,
    range: std::ops::Range<usize>,
    position: Option<ScriptPosition>,
) -> CommittedModelTransaction {
    let range = TextRange::new(
        document.text_point(range.start).unwrap(),
        document.text_point(range.end).unwrap(),
    )
    .unwrap();
    let intent = if let Some(position) = position {
        PersistedStyleIntent::SetDirectCharacterProperties {
            range,
            properties: CharacterProperties {
                script_position: Some(position),
                ..Default::default()
            },
        }
    } else {
        PersistedStyleIntent::ClearDirectCharacterProperties {
            range,
            properties: BTreeSet::from([StyleProperty::CharacterScriptPosition]),
        }
    };
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(intent),
        ))
        .unwrap()
}

#[test]
fn html_and_rtf_import_semantic_scripts_and_preserve_unsupported_offsets() {
    for (format, source, expected) in [
        (
            Format::Html,
            "<p>A<sup>B<sub>C</sub>D</sup>E</p>",
            vec![
                ScriptPosition::Normal,
                ScriptPosition::Superscript,
                ScriptPosition::Subscript,
                ScriptPosition::Superscript,
                ScriptPosition::Normal,
            ],
        ),
        (
            Format::Rtf,
            "{\\rtf1 A{\\super B{\\sub C}D}E}",
            vec![
                ScriptPosition::Normal,
                ScriptPosition::Superscript,
                ScriptPosition::Subscript,
                ScriptPosition::Superscript,
                ScriptPosition::Normal,
            ],
        ),
        (
            Format::Html,
            "<p><span style='vertical-align:9pt'>A</span></p>",
            vec![ScriptPosition::Normal],
        ),
        (
            Format::Rtf,
            "{\\rtf1{\\up12 A}{\\dn8 B}}",
            vec![ScriptPosition::Normal, ScriptPosition::Normal],
        ),
    ] {
        let document = open(format, source);
        for (at, expected) in expected.into_iter().enumerate() {
            assert_eq!(position(&document, at), expected, "{format:?} at {at}");
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn direct_script_edits_switch_reset_clear_reopen_and_undo_without_touching_neighbors() {
    for (format, source) in [
        (
            Format::Html,
            "<!--keep--><p data-x='untouched'>ABC</p><!--tail-->",
        ),
        (Format::Rtf, "{\\rtf1 ABC}{\\*\\unknown keep}"),
    ] {
        let mut document = open(format, source);
        for desired in [
            ScriptPosition::Superscript,
            ScriptPosition::Subscript,
            ScriptPosition::Normal,
        ] {
            apply(&mut document, 1..2, Some(desired));
            assert_eq!(position(&document, 0), ScriptPosition::Normal);
            assert_eq!(position(&document, 1), desired);
            assert_eq!(position(&document, 2), ScriptPosition::Normal);
            let bytes = document.source_bytes();
            let saved = String::from_utf8(bytes.clone()).unwrap();
            assert!(saved.contains(if format == Format::Html {
                "<!--keep--><p data-x='untouched'>"
            } else {
                "{\\*\\unknown keep}"
            }));
            assert_eq!(position(&open(format, &saved), 1), desired);
        }
        apply(&mut document, 1..2, None);
        assert_eq!(position(&document, 1), ScriptPosition::Normal);
        for _ in 0..4 {
            assert!(document.undo());
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
        for _ in 0..4 {
            assert!(document.redo());
        }
        assert_eq!(position(&document, 1), ScriptPosition::Normal);
    }
    let mut tagged = open(Format::Html, "<p><sup>ABC</sup></p>");
    apply(&mut tagged, 1..2, None);
    assert_eq!(
        (
            position(&tagged, 0),
            position(&tagged, 1),
            position(&tagged, 2)
        ),
        (
            ScriptPosition::Superscript,
            ScriptPosition::Normal,
            ScriptPosition::Superscript
        )
    );
}

#[test]
fn script_typing_switches_are_exclusive_and_one_undo_group() {
    for (format, source) in [
        (Format::Html, "<p>x</p>"),
        (Format::Rtf, "{\\rtf1 x}"),
        (Format::HtmlSource, "<p>x</p>"),
    ] {
        let mut core = Core::new(open(format, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        let at = if format == Format::HtmlSource { 3 } else { 0 };
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
        for (script, text) in [
            (ScriptPosition::Superscript, "1"),
            (ScriptPosition::Subscript, "2"),
            (ScriptPosition::Normal, "3"),
        ] {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetDirectCharacterProperties {
                    expected,
                    values: vec![(
                        StyleProperty::CharacterScriptPosition,
                        StylePropertyValue::ScriptPosition(script),
                    )],
                },
            )
            .unwrap();
            assert_eq!(
                core.selected_typography(view).unwrap().0.script_position,
                script
            );
            core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                .unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        let visible = open(
            if format == Format::HtmlSource {
                Format::Html
            } else {
                format
            },
            &saved,
        );
        assert_eq!(visible.text(), "123x");
        for (at, expected) in [
            ScriptPosition::Superscript,
            ScriptPosition::Subscript,
            ScriptPosition::Normal,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(position(&visible, at), expected, "{format:?}: {saved}");
        }
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn portable_clipboard_roundtrip_carries_script_position() {
    for (format, source) in [
        (Format::Html, "<p>A<sup>2</sup><sub>3</sub></p>"),
        (Format::Rtf, "{\\rtf1 A{\\super 2}{\\sub 3}}"),
    ] {
        let document = open(format, source);
        let fragment = document.clipboard_fragment(0..3).unwrap();
        assert!(fragment
            .json()
            .contains("\"script_position\":\"Superscript\""));
        assert!(fragment
            .json()
            .contains("\"script_position\":\"Subscript\""));
        assert_eq!(
            ClipboardFragment::from_json(fragment.json(), "A23").unwrap(),
            fragment
        );
    }
}

#[test]
fn script_edit_invalidates_only_affected_shaping_in_large_document() {
    let source = (0..2_000)
        .map(|i| format!("<p>line {i:04} content</p>"))
        .collect::<String>();
    let mut document = open(Format::Html, &source);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(2_100);
    engine.set_shape_cache_byte_budgets(64 * 1024 * 1024, 1024 * 1024);
    let mut view = ViewLayout::new(300., 100.);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    let before = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    apply(&mut document, 0..1, Some(ScriptPosition::Superscript));
    let after = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    assert_eq!(before.size, after.size);
    assert_ne!(before.script_position, after.script_position);
    assert_eq!(
        StyleProperty::CharacterScriptPosition.invalidation_effect(),
        StyleInvalidationEffect::Shaping
    );
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() > shaped);
    assert!(
        engine.provider().request_calls() - shaped <= 2,
        "requests {}",
        engine.provider().request_calls() - shaped
    );
    let shaped = engine.provider().request_calls();
    view.resize(350., 100.);
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(engine.provider().request_calls(), shaped);
    assert!(document.undo());
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(position(&document, 0), ScriptPosition::Normal);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn disabling_script_at_a_caret_splits_its_inline_scope_and_keeps_surrounding_script() {
    for (format, source, at) in [
        (Format::Html, "<p><sup>AB</sup></p><!--keep-->", 1),
        (Format::HtmlSource, "<p><sup>AB</sup></p><!--keep-->", 9),
        (Format::Rtf, "{\\rtf1 {\\super AB}}", 1),
    ] {
        let mut core = Core::new(open(format, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
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
            CoreEvent::SetDirectCharacterProperties {
                expected,
                values: vec![(
                    StyleProperty::CharacterScriptPosition,
                    StylePropertyValue::ScriptPosition(ScriptPosition::Normal),
                )],
            },
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::text("x")))
            .unwrap();
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        let visible = open(
            if format == Format::HtmlSource {
                Format::Html
            } else {
                format
            },
            &saved,
        );
        assert_eq!(visible.text(), "AxB", "{format:?}: {saved}");
        assert_eq!(
            (
                position(&visible, 0),
                position(&visible, 1),
                position(&visible, 2)
            ),
            (
                ScriptPosition::Superscript,
                ScriptPosition::Normal,
                ScriptPosition::Superscript
            ),
            "{format:?}: {saved}"
        );
    }
}
