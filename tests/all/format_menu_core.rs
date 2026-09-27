use viem_core::command::{InputEvent, Key};
use viem_core::document::*;
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

fn document(format: Format, source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn request(
    document: &Document,
    range: std::ops::Range<usize>,
    values: Vec<(StyleProperty, Option<StylePropertyValue>)>,
) -> ModelRequest {
    ModelRequest::EditDirectProperties {
        document: document.id(),
        revision: document.revision(),
        range,
        values,
    }
}
#[test]
fn character_and_paragraph_batch_applies_and_undoes_atomically() {
    for (format, source) in [(Format::Rtf, "{\\rtf1 ABC\\par DEF}{\\*\\unknown keep}")] {
        let mut doc = document(format, source);
        let action = request(
            &doc,
            1..2,
            vec![
                (
                    StyleProperty::CharacterScriptPosition,
                    Some(StylePropertyValue::ScriptPosition(
                        ScriptPosition::Superscript,
                    )),
                ),
                (
                    StyleProperty::CharacterUnderline,
                    Some(StylePropertyValue::Boolean(true)),
                ),
                (
                    StyleProperty::ParagraphAlignment,
                    Some(StylePropertyValue::ParagraphAlignment(
                        ParagraphAlignment::Center,
                    )),
                ),
                (
                    StyleProperty::BlockMarginBottom,
                    Some(StylePropertyValue::Float(9.)),
                ),
            ],
        );
        let changed = doc.apply_model_request(action).unwrap();
        assert_eq!(doc.text(), "ABC\nDEF");
        assert!(!changed.summary().source_patches().is_empty());
        let c = DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false).unwrap();
        assert_eq!(c.script_position, ScriptPosition::Superscript);
        assert!(c.underline);
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center)
        );
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.margin_bottom,
            Some(9.)
        );
        assert_eq!(
            doc.projection().blocks()[1].direct_paragraph.alignment,
            None
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(!doc.undo());
        assert!(doc.redo());
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false)
                .unwrap()
                .script_position,
            ScriptPosition::Superscript
        );
    }
}
#[test]
fn failed_or_duplicate_batches_publish_no_partial_character_or_paragraph_changes() {
    for values in [
        vec![
            (
                StyleProperty::CharacterUnderline,
                Some(StylePropertyValue::Boolean(true)),
            ),
            (
                StyleProperty::BlockMarginBottom,
                Some(StylePropertyValue::Float(f32::NAN)),
            ),
        ],
        vec![
            (
                StyleProperty::CharacterUnderline,
                Some(StylePropertyValue::Boolean(true)),
            ),
            (StyleProperty::CharacterUnderline, None),
        ],
    ] {
        let source = r"{\rtf1 ABC}";
        let mut doc = document(Format::Rtf, source);
        let action = request(&doc, 0..2, values);
        assert!(doc.apply_model_request(action).is_err());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(!doc.history_status().can_undo);
    }
}
#[test]
fn clearing_direct_formatting_retains_inherited_heading_and_unselected_runs() {
    let source = r"{\rtf1{\stylesheet{\s1\b Heading 1;}}\s1\qc\super ABC}";
    let mut doc = document(Format::Rtf, source);
    let action = request(
        &doc,
        1..2,
        vec![
            (StyleProperty::CharacterScriptPosition, None),
            (StyleProperty::CharacterBold, None),
            (StyleProperty::ParagraphAlignment, None),
        ],
    );
    doc.apply_model_request(action).unwrap();
    let selected = DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false).unwrap();
    assert_eq!(selected.script_position, ScriptPosition::Normal);
    assert_eq!(selected.weight, 700);
    assert_eq!(
        doc.projection().blocks()[0].direct_paragraph.alignment,
        None
    );
    for at in [0, 2] {
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), at, false)
                .unwrap()
                .script_position,
            ScriptPosition::Superscript
        );
    }
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
}
#[test]
fn paragraph_readback_uses_semantic_source_blocks_and_caret_location() {
    let source = r"{\rtf1\qc\sa180 A\par \qr\sa140 B}";
    for format in [Format::Rtf] {
        let mut core = Core::new(document(format, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        for (at, alignment, after) in [
            (0, ParagraphAlignment::Center, 9.),
            (2, ParagraphAlignment::End, 7.),
        ] {
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
            let paragraph = core.selected_paragraph_style(view).unwrap();
            assert_eq!(paragraph.alignment, alignment, "{format:?}");
            assert_eq!(paragraph.margin_bottom, after, "{format:?}");
        }
    }
}
#[test]
fn pending_batch_preserves_validation_atomicity_and_source_cleanliness() {
    let source = r"{\rtf1 AB}";
    let mut core = Core::new(document(Format::Rtf, source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    assert!(core
        .handle(
            view,
            CoreEvent::EditDirectProperties {
                expected: expected.clone(),
                values: vec![
                    (
                        StyleProperty::CharacterScriptPosition,
                        Some(StylePropertyValue::ScriptPosition(
                            ScriptPosition::Superscript
                        ))
                    ),
                    (
                        StyleProperty::BlockMarginBottom,
                        Some(StylePropertyValue::Float(f32::NAN))
                    ),
                ]
            }
        )
        .is_err());
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(
        core.selected_typography(view).unwrap().0.script_position,
        ScriptPosition::Normal
    );
    core.handle(
        view,
        CoreEvent::EditDirectProperties {
            expected,
            values: vec![
                (
                    StyleProperty::CharacterScriptPosition,
                    Some(StylePropertyValue::ScriptPosition(
                        ScriptPosition::Subscript,
                    )),
                ),
                (
                    StyleProperty::ParagraphAlignment,
                    Some(StylePropertyValue::ParagraphAlignment(
                        ParagraphAlignment::Center,
                    )),
                ),
            ],
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "AB");
    assert_eq!(
        core.selected_typography(view).unwrap().0.script_position,
        ScriptPosition::Subscript
    );
    core.handle(view, CoreEvent::Input(InputEvent::text("x")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
            .unwrap()
            .script_position,
        ScriptPosition::Subscript
    );
}
#[test]
fn script_mixed_state_ignores_unrelated_font_and_color_differences() {
    for (source, expected_mixed) in [
        (r"{\rtf1\super {\b A}B}", false),
        (r"{\rtf1{\super A}{\sub B}}", true),
    ] {
        let mut core = Core::new(document(Format::Rtf, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
        for c in ['v', 'l'] {
            core.handle(view, CoreEvent::Input(InputEvent::key(c)))
                .unwrap();
        }
        let (_, mixed, script_mixed) = core.selected_typography_details(view).unwrap();
        assert!(mixed);
        assert_eq!(script_mixed, expected_mixed);
    }
}

#[test]
fn rtf_explicit_transparent_background_overrides_existing_highlight() {
    let source = "{\\rtf1{\\colortbl;\\red255\\green255\\blue0;}\\highlight1 AB}";
    let mut doc = document(Format::Rtf, source);
    let action = request(
        &doc,
        0..1,
        vec![(
            StyleProperty::CharacterBackground,
            Some(StylePropertyValue::Color(Color {
                red: 0.,
                green: 0.,
                blue: 0.,
                alpha: 0.,
            })),
        )],
    );
    doc.apply_model_request(action).unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(doc.projection(), 0, false)
            .unwrap()
            .background
            .unwrap()
            .alpha,
        0.
    );
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false)
            .unwrap()
            .background
            .unwrap()
            .alpha,
        1.
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn native_font_batch_retains_semantic_bold_relative_to_the_new_face_weight() {
    let mut doc = document(Format::Rtf, r"{\rtf1{\fonttbl{\f0 Helvetica;}}\f0\b B}");
    let action = ModelRequest::SetDirectCharacterProperties {
        document: doc.id(),
        revision: doc.revision(),
        range: 0..1,
        values: vec![
            (
                StyleProperty::CharacterFontFamilies,
                StylePropertyValue::FontFamilies(vec!["Arial".into()]),
            ),
            (
                StyleProperty::CharacterWeight,
                StylePropertyValue::FontWeight(400),
            ),
            (StyleProperty::CharacterSize, StylePropertyValue::Float(30.)),
            (
                StyleProperty::CharacterSlant,
                StylePropertyValue::FontSlant(FontSlant::Upright),
            ),
        ],
    };
    doc.apply_model_request(action).unwrap();
    let style = DocumentLayoutStyles::semantic_character_at(doc.projection(), 0, false).unwrap();
    assert_eq!(style.font_families, ["Arial"]);
    assert_eq!(style.size, 30.);
    assert_eq!(style.base_weight, 400);
    assert_eq!(style.weight, 700);
    assert!(style.bold);
    assert_eq!(style.slant, FontSlant::Upright);
}
