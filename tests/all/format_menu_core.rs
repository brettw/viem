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
    for (format, source) in [
        (
            Format::Html,
            "<!--keep--><p data-x='keep'>ABC</p><p>DEF</p>",
        ),
        (Format::Rtf, "{\\rtf1 ABC\\par DEF}{\\*\\unknown keep}"),
    ] {
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
        let source = "<p>ABC</p>";
        let mut doc = document(Format::Html, source);
        let action = request(&doc, 0..2, values);
        assert!(doc.apply_model_request(action).is_err());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(!doc.history_status().can_undo);
    }
}
#[test]
fn clearing_direct_formatting_retains_inherited_heading_and_unselected_runs() {
    let source = "<h1 style='text-align:center'><sup>ABC</sup></h1>";
    let mut doc = document(Format::Html, source);
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
    let source =
        "<p style='text-align:center;margin-block-end:9pt'>A</p><p style='text-align:end'>B</p>";
    for format in [Format::Html, Format::HtmlSource] {
        let mut core = Core::new(document(format, source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        for (at, alignment, after) in [
            (
                if format == Format::Html {
                    0
                } else {
                    source.find('A').unwrap()
                },
                ParagraphAlignment::Center,
                9.,
            ),
            (
                if format == Format::Html {
                    2
                } else {
                    source.find('B').unwrap()
                },
                ParagraphAlignment::End,
                7.,
            ),
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
    let source = "<p>AB</p>";
    let mut core = Core::new(document(Format::Html, source));
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
        ("<p><sup><b>A</b>B</sup></p>", false),
        ("<p><sup>A</sup><sub>B</sub></p>", true),
    ] {
        let mut core = Core::new(document(Format::Html, source));
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
fn html_paragraph_direction_sets_and_clears_with_nested_direction_preserved() {
    let source = "<p data-x='keep'>A<span dir='rtl'>B</span>C</p><p>D</p>";
    let mut doc = document(Format::Html, source);
    for value in [
        Some(WritingDirection::LeftToRight),
        Some(WritingDirection::RightToLeft),
        Some(WritingDirection::Natural),
        None,
    ] {
        let action = request(
            &doc,
            0..3,
            vec![(
                StyleProperty::ParagraphBaseDirection,
                value.map(StylePropertyValue::WritingDirection),
            )],
        );
        doc.apply_model_request(action).unwrap();
        assert_eq!(
            doc.projection().blocks()[0].direct_paragraph.base_direction,
            value
        );
        assert_eq!(
            doc.projection().blocks()[1].direct_paragraph.base_direction,
            None
        );
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false)
                .unwrap()
                .direction,
            WritingDirection::RightToLeft
        );
        assert!(String::from_utf8(doc.source_bytes())
            .unwrap()
            .contains("<span dir='rtl'>B</span>"));
    }
    for _ in 0..4 {
        assert!(doc.undo());
    }
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn html_explicit_automatic_direction_overrides_inherited_rtl() {
    let mut doc = document(Format::Html, "<div dir='rtl'><p>AB</p></div>");
    let action = request(
        &doc,
        0..2,
        vec![
            (
                StyleProperty::ParagraphBaseDirection,
                Some(StylePropertyValue::WritingDirection(
                    WritingDirection::Natural,
                )),
            ),
            (
                StyleProperty::CharacterDirection,
                Some(StylePropertyValue::WritingDirection(
                    WritingDirection::Natural,
                )),
            ),
        ],
    );
    doc.apply_model_request(action).unwrap();
    assert_eq!(
        doc.projection().blocks()[0].direct_paragraph.base_direction,
        Some(WritingDirection::Natural)
    );
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(doc.projection(), 0, false)
            .unwrap()
            .direction,
        WritingDirection::Natural
    );
    assert!(String::from_utf8(doc.source_bytes())
        .unwrap()
        .contains("dir=\"auto\""));
}

#[test]
fn full_effective_html_format_copy_pastes_onto_an_inherited_rtl_target() {
    let mut source = Core::new(document(Format::Html, "<p style='text-align:center'><span style='font-family:Arial;font-size:22pt'><sup>A</sup></span></p>"));
    let source_view = source.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    let c = source.selected_typography(source_view).unwrap().0;
    let p = source.selected_paragraph_style(source_view).unwrap();
    let values = full_effective_values(&c, &p);
    let original = "<div dir='rtl' style='background-color:#ffff00'><p><b>B</b></p></div>";
    for format in [Format::Html, Format::HtmlSource] {
        let mut target = document(format, original);
        let at = if format == Format::HtmlSource {
            original.find('B').unwrap()
        } else {
            0
        };
        let action = request(&target, at..at + 1, values.clone());
        target.apply_model_request(action).unwrap();
        let visible = document(
            Format::Html,
            &String::from_utf8(target.source_bytes()).unwrap(),
        );
        let pasted =
            DocumentLayoutStyles::semantic_character_at(visible.projection(), 0, false).unwrap();
        assert_eq!(pasted.font_families, c.font_families);
        assert_eq!(pasted.size, c.size);
        assert_eq!(pasted.script_position, c.script_position);
        assert_eq!(pasted.direction, WritingDirection::Natural);
        assert_eq!(pasted.bold, c.bold);
        assert_eq!(pasted.background.unwrap().alpha, 0.);
        assert_eq!(
            visible.projection().blocks()[0]
                .direct_paragraph
                .base_direction,
            Some(WritingDirection::Natural)
        );
        assert!(target.undo());
        assert_eq!(target.source_bytes(), original.as_bytes());
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
fn clear_formatting_removes_shared_html_direction_declarations() {
    let mut doc = document(Format::Html, "<p dir='rtl'>A<span dir='ltr'>B</span>C</p>");
    let action = request(
        &doc,
        0..3,
        vec![
            (StyleProperty::CharacterDirection, None),
            (StyleProperty::ParagraphBaseDirection, None),
        ],
    );
    doc.apply_model_request(action).unwrap();
    assert_eq!(
        doc.projection().blocks()[0].direct_paragraph.base_direction,
        None
    );
    for at in 0..3 {
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), at, false)
                .unwrap()
                .direction,
            WritingDirection::Natural
        );
    }
}

#[test]
fn native_font_batch_retains_semantic_bold_relative_to_the_new_face_weight() {
    let mut doc = document(
        Format::Html,
        "<p><b style='font-family:Helvetica'>B</b></p>",
    );
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

fn full_effective_values(
    c: &ResolvedCharacterStyle,
    p: &ResolvedParagraphStyle,
) -> Vec<(StyleProperty, Option<StylePropertyValue>)> {
    use StyleProperty as P;
    use StylePropertyValue as V;
    vec![
        (P::BlockMarginTop, Some(V::Float(p.margin_top))),
        (P::BlockMarginBottom, Some(V::Float(p.margin_bottom))),
        (
            P::ParagraphLineSpacing,
            Some(V::LineSpacing(p.line_spacing)),
        ),
        (
            P::ParagraphFirstLineIndent,
            Some(V::Float(p.first_line_indent)),
        ),
        (P::ParagraphLeadingIndent, Some(V::Float(p.leading_indent))),
        (
            P::ParagraphTrailingIndent,
            Some(V::Float(p.trailing_indent)),
        ),
        (
            P::ParagraphAlignment,
            Some(V::ParagraphAlignment(p.alignment)),
        ),
        (
            P::ParagraphBaseDirection,
            Some(V::WritingDirection(p.base_direction)),
        ),
        (
            P::CharacterFontFamilies,
            Some(V::FontFamilies(c.font_families.clone())),
        ),
        (P::CharacterSize, Some(V::Float(c.size))),
        (P::CharacterWeight, Some(V::FontWeight(c.base_weight))),
        (P::CharacterBold, Some(V::Boolean(c.bold))),
        (P::CharacterSlant, Some(V::FontSlant(c.slant))),
        (P::CharacterForeground, Some(V::Color(c.foreground))),
        (
            P::CharacterBackground,
            Some(V::Color(c.background.unwrap_or(Color {
                red: 0.,
                green: 0.,
                blue: 0.,
                alpha: 0.,
            }))),
        ),
        (P::CharacterUnderline, Some(V::Boolean(c.underline))),
        (P::CharacterStrikethrough, Some(V::Boolean(c.strikethrough))),
        (P::CharacterLanguage, c.language.clone().map(V::Text)),
        (
            P::CharacterDirection,
            Some(V::WritingDirection(c.direction)),
        ),
        (
            P::CharacterOpenTypeFeatures,
            Some(V::OpenTypeFeatures(c.open_type_features.clone())),
        ),
        (P::CharacterLetterSpacing, Some(V::Float(c.letter_spacing))),
        (
            P::CharacterScriptPosition,
            Some(V::ScriptPosition(c.script_position)),
        ),
    ]
}

#[test]
fn full_html_copy_formats_rtf_and_full_clear_removes_the_html_paste() {
    let mut source = Core::new(document(Format::Html, "<p style='font-family:Georgia;font-size:19pt;color:#336699;text-align:center'><sup>source</sup></p>"));
    let view = source.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    let c = source.selected_typography(view).unwrap().0;
    let p = source.selected_paragraph_style(view).unwrap();
    let values = full_effective_values(&c, &p);
    for (format, text) in [
        (Format::Rtf, "{\\rtf1 target{\\*\\opaque keep}}"),
        (Format::Html, "<p>target</p><!--keep-->"),
    ] {
        let mut target = document(format, text);
        let payload = values
            .iter()
            .filter(|(_, value)| {
                format != Format::Rtf
                    || !matches!(
                        value,
                        Some(StylePropertyValue::WritingDirection(
                            WritingDirection::Natural
                        ))
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        let action = request(&target, 0..6, payload.clone());
        if let Err(error) = target.apply_model_request(action) {
            for value in payload {
                let mut single = document(format, text);
                let action = request(&single, 0..6, vec![value.clone()]);
                assert!(
                    single.apply_model_request(action).is_ok(),
                    "{format:?} rejected {value:?}; batch {error:?}"
                );
            }
            panic!("{format:?} batch failed {error:?}");
        }
        let pasted =
            DocumentLayoutStyles::semantic_character_at(target.projection(), 0, false).unwrap();
        assert_eq!(pasted.font_families, c.font_families);
        assert_eq!(pasted.script_position, ScriptPosition::Superscript);
        let pasted_source = target.source_bytes();
        let clear = values
            .iter()
            .map(|(property, _)| (*property, None))
            .collect();
        let action = request(&target, 0..6, clear);
        target.apply_model_request(action).unwrap_or_else(|error| {
            panic!(
                "{format:?} clear failed {error:?}; source {}",
                String::from_utf8_lossy(&pasted_source)
            )
        });
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(target.projection(), 0, false)
                .unwrap()
                .script_position,
            ScriptPosition::Normal
        );
        assert!(target.undo());
        assert_eq!(target.source_bytes(), pasted_source);
    }
}
