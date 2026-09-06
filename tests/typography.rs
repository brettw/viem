use evim_core::document::*;
use evim_core::layout::DocumentLayoutStyles;

fn configured(source: &str, format: Format, weight: u16) -> Document {
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let request = StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Configuration(ConfigurationStyleIntent::SetDocumentDefaultCharacter(
            CharacterProperties {
                weight: Some(weight),
                ..Default::default()
            },
        )),
    );
    document.apply_style_request(request).unwrap();
    document
}

#[test]
fn semantic_bold_adds_three_hundred_to_base_weight_in_all_rich_adapters() {
    for (format, source) in [
        (Format::Markdown, "word"),
        (Format::Html, "<p>word</p>"),
        (Format::Rtf, r"{\rtf1 word}"),
    ] {
        let mut document = configured(source, format, 200);
        document
            .set_semantic_style(0..4, SemanticInlineStyle::Strong, true)
            .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
        assert_eq!(
            (style.base_weight, style.weight, style.bold),
            (200, 500, true),
            "{format:?}"
        );
        let after = document.source_bytes();
        document
            .set_semantic_style(0..4, SemanticInlineStyle::Strong, false)
            .unwrap();
        let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
        assert_eq!(
            (style.base_weight, style.weight, style.bold),
            (200, 200, false),
            "{format:?}"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), after);
    }
}

#[test]
fn relative_html_bold_preserves_base_weight_source_and_partial_toggle() {
    let source = "<p style='font-weight:200;font-family:AvenirNext-UltraLight'><b data-keep='yes'>word</b> tail</p><!--keep-->";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
    assert_eq!(
        (style.base_weight, style.weight, style.bold),
        (200, 500, true)
    );
    document
        .set_semantic_style(1..3, SemanticInlineStyle::Strong, false)
        .unwrap();
    let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
    assert_eq!(
        (style.base_weight, style.weight, style.bold),
        (200, 200, false)
    );
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains("font-weight: 200; --evim-base-weight: 200; --evim-bold: false"));
    assert!(saved.contains("data-keep='yes'"));
    assert!(saved.ends_with("<!--keep-->"));
    let reopened =
        Document::from_bytes(saved.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(reopened.projection(), 1, false).unwrap(),
        style
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn absolute_weight_is_distinct_from_bold_and_empty_paragraph_queries_keep_current_style() {
    let source = "<p style='font-weight:200'><b>one</b><span style='font-weight:700'>two</span></p><p style='font-size:36pt'></p>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let absolute = DocumentLayoutStyles::character_at(document.projection(), 3, false).unwrap();
    assert_eq!(
        (absolute.base_weight, absolute.weight, absolute.bold),
        (700, 700, false)
    );
    let empty =
        DocumentLayoutStyles::character_at(document.projection(), document.text().len(), true)
            .unwrap();
    assert_eq!(empty.size, 36.0);
}

#[test]
fn changing_font_face_preserves_independent_mixed_bold_and_is_one_source_transaction() {
    for (format, source) in [
        (Format::Html, "<p><b>A</b>B</p><!--keep-->"),
        (Format::Rtf, r"{\rtf1{\b A}B{\*\unknown keep}}"),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .apply_model_request(ModelRequest::SetDirectCharacterProperties {
                document: document.id(),
                revision: document.revision(),
                range: 0..2,
                values: vec![
                    (
                        StyleProperty::CharacterFontFamilies,
                        StylePropertyValue::FontFamilies(vec!["AvenirNext-UltraLight".into()]),
                    ),
                    (
                        StyleProperty::CharacterWeight,
                        StylePropertyValue::FontWeight(200),
                    ),
                    (
                        StyleProperty::CharacterSize,
                        StylePropertyValue::Float(24.0),
                    ),
                ],
            })
            .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        let a = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
        let b = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
        assert_eq!((a.base_weight, a.weight, a.bold), (200, 500, true));
        assert_eq!((b.base_weight, b.weight, b.bold), (200, 200, false));
        assert_eq!(a.size, 24.0);
        assert_eq!(b.size, 24.0);
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            DocumentLayoutStyles::character_at(reopened.projection(), 0, false).unwrap(),
            a
        );
        assert_eq!(
            DocumentLayoutStyles::character_at(reopened.projection(), 1, false).unwrap(),
            b
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn first_rtf_font_table_does_not_change_the_unspecified_font_of_unselected_text() {
    let source = r"{\rtf1 One tail{\*\unknown keep}}";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let before = DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap();
    document
        .apply_model_request(ModelRequest::SetDirectCharacterProperties {
            document: document.id(),
            revision: document.revision(),
            range: 0..3,
            values: vec![(
                StyleProperty::CharacterFontFamilies,
                StylePropertyValue::FontFamilies(vec!["Georgia".into()]),
            )],
        })
        .unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap(),
        before
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap()
            .font_families,
        ["Georgia"]
    );
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains(r"{\fonttbl{\f1\fnil Georgia;}}"));
    assert!(saved.ends_with(r"tail{\*\unknown keep}}"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn rtf_optional_features_are_scoped_lossless_clearable_and_reset_by_plain() {
    use std::collections::{BTreeMap, BTreeSet};
    let source = r"{\rtf1 word tail{\*\unknown \evimfeaturegmgjghgb0 opaque}}";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let features = BTreeMap::from([
        ("liga".into(), 0),
        ("ss01".into(), 1),
        ("cv01".into(), u32::MAX),
    ]);
    document
        .apply_model_request(ModelRequest::SetDirectCharacterProperties {
            document: document.id(),
            revision: document.revision(),
            range: 0..4,
            values: vec![(
                StyleProperty::CharacterOpenTypeFeatures,
                StylePropertyValue::OpenTypeFeatures(features.clone()),
            )],
        })
        .unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 1, false)
            .unwrap()
            .open_type_features,
        features
    );
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 6, false)
            .unwrap()
            .open_type_features
            .is_empty()
    );
    let saved = document.source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(reopened.projection(), 1, false)
            .unwrap()
            .open_type_features,
        features
    );
    let range = TextRange::new(
        document.text_point(1).unwrap(),
        document.text_point(3).unwrap(),
    )
    .unwrap();
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::ClearDirectCharacterProperties {
                range,
                properties: BTreeSet::from([StyleProperty::CharacterOpenTypeFeatures]),
            }),
        ))
        .unwrap();
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 1, false)
            .unwrap()
            .open_type_features
            .is_empty()
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap()
            .open_type_features,
        features
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), saved);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    let reset = Document::from_bytes(
        br"{\rtf1\evimfeaturegmgjghgb0 old\plain new}".to_vec(),
        Encoding::Utf8,
        Format::Rtf,
    )
    .unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(reset.projection(), 1, false)
            .unwrap()
            .open_type_features
            .get("liga"),
        Some(&0)
    );
    assert!(
        DocumentLayoutStyles::character_at(reset.projection(), 5, false)
            .unwrap()
            .open_type_features
            .is_empty()
    );
}

#[test]
fn clearing_typography_preserves_unrecognized_private_control_parameters() {
    let source = r"{\rtf1\evimfeatures7\evimweight9999\evimfeaturegmgjghgb Text}";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let revision = document.revision();
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::ClearDirectCharacterProperties {
                range,
                properties: std::collections::BTreeSet::from([
                    StyleProperty::CharacterOpenTypeFeatures,
                    StyleProperty::CharacterWeight,
                ]),
            }),
        ))
        .unwrap();
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert_eq!(document.revision(), revision);
}
