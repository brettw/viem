use viem_core::document::*;
use std::collections::BTreeSet;
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn apply(document: &mut Document, intent: PersistedStyleIntent) {
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(intent),
        ))
        .unwrap();
}
fn range(document: &Document, start: usize, end: usize) -> TextRange {
    TextRange::new(
        document.text_point(start).unwrap(),
        document.text_point(end).unwrap(),
    )
    .unwrap()
}
fn direct(document: &Document, at: usize) -> CharacterProperties {
    let mut result = CharacterProperties::default();
    for span in document.projection().style_spans() {
        if span.range.contains(&at) {
            if let StyleApplication::Direct(properties) = &span.application {
                result = properties.clone();
            }
        }
    }
    result
}
#[test]
fn html_clear_direct_property_splits_only_affected_inline_context() {
    for source in [
        "<p style='font-weight:700;unknown:keep'>abcdef</p><!--keep-->",
        "<p><b foo='bar'>abcdef</b></p><!--keep-->",
        "<p><b foo='bar'><i>abcdef</i></b></p><!--keep-->",
        "<p><b foo='bar'>abc</b><b>def</b></p><!--keep-->",
    ] {
        let mut document = open(source, Format::Html);
        let selected = range(&document, 2, 4);
        apply(
            &mut document,
            PersistedStyleIntent::ClearDirectCharacterProperties {
                range: selected,
                properties: BTreeSet::from([
                    StyleProperty::CharacterWeight,
                    StyleProperty::CharacterBold,
                ]),
            },
        );
        assert_eq!(document.text(), "abcdef");
        assert!(
            direct(&document, 0).weight == Some(700) || direct(&document, 0).bold == Some(true)
        );
        assert_eq!(direct(&document, 2).weight, None);
        assert!(
            direct(&document, 4).weight == Some(700) || direct(&document, 4).bold == Some(true)
        );
        let fresh = open(
            std::str::from_utf8(&document.source_bytes()).unwrap(),
            Format::Html,
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn html_paragraph_properties_apply_to_all_hard_lines_and_clear_to_inheritance() {
    let source="<p id='keep' style='text-align:center;unknown:opaque;font-weight:700'>one<br>two</p><p>tail</p>";
    let mut document = open(source, Format::Html);
    let selected = range(&document, 4, 7);
    apply(
        &mut document,
        PersistedStyleIntent::SetDirectBlockProperties {
            target: StyleBlockTarget::Paragraphs(selected),
            properties: BlockProperties {
                spacing_after: Some(18.0),
                first_line_indent: Some(12.0),
                ..Default::default()
            },
        },
    );
    assert_eq!(document.projection().blocks().len(), 2);
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .spacing_after,
        Some(18.0)
    );
    assert_eq!(
        document.projection().blocks()[1]
            .direct_paragraph
            .spacing_after,
        None
    );
    assert!(direct(&document, 0).weight == Some(700) || direct(&document, 0).bold == Some(true));
    let selected = range(&document, 0, 7);
    apply(
        &mut document,
        PersistedStyleIntent::ClearDirectBlockProperties {
            target: StyleBlockTarget::Paragraphs(selected),
            properties: BTreeSet::from([
                StyleProperty::ParagraphAlignment,
                StyleProperty::ParagraphSpacingAfter,
            ]),
        },
    );
    assert_eq!(
        document.projection().blocks()[0].direct_paragraph.alignment,
        None
    );
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .spacing_after,
        None
    );
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .first_line_indent,
        Some(12.0)
    );
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("unknown: opaque"));
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn rtf_paragraph_property_deltas_preserve_inline_groups_and_other_paragraphs() {
    for source in [
        r"{\rtf1\qc\sa120 one{\b bold}\line two\par tail}",
        r"{\rtf1\qc\sa120 one{\b bold}\line two}",
    ] {
        let mut document = open(source, Format::Rtf);
        let selected = range(&document, 0, 1);
        apply(
            &mut document,
            PersistedStyleIntent::SetDirectBlockProperties {
                target: StyleBlockTarget::Paragraphs(selected),
                properties: BlockProperties {
                    spacing_after: Some(18.0),
                    first_line_indent: Some(12.0),
                    ..Default::default()
                },
            },
        );
        assert_eq!(
            document.projection().blocks()[0]
                .direct_paragraph
                .spacing_after,
            Some(18.0)
        );
        let selected = range(&document, 0, 1);
        apply(
            &mut document,
            PersistedStyleIntent::ClearDirectBlockProperties {
                target: StyleBlockTarget::Paragraphs(selected),
                properties: BTreeSet::from([
                    StyleProperty::ParagraphAlignment,
                    StyleProperty::ParagraphSpacingAfter,
                ]),
            },
        );
        assert_eq!(
            document.projection().blocks()[0].direct_paragraph.alignment,
            None
        );
        assert_eq!(
            document.projection().blocks()[0]
                .direct_paragraph
                .spacing_after,
            None
        );
        assert_eq!(
            document.projection().blocks()[0]
                .direct_paragraph
                .first_line_indent,
            Some(12.0)
        );
        let fresh = open(
            std::str::from_utf8(&document.source_bytes()).unwrap(),
            Format::Rtf,
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn direct_character_formatting_crosses_existing_markup_without_rewriting_it() {
    for (format, source) in [
        (
            Format::Html,
            "<p><b foo='bar'>bold</b> and <i>italic</i></p>",
        ),
        (Format::Rtf, r"{\rtf1{\b bold} and {\i italic}}"),
    ] {
        let mut document = open(source, format);
        let selected = range(&document, 0, 15);
        apply(
            &mut document,
            PersistedStyleIntent::SetDirectCharacterProperties {
                range: selected,
                properties: CharacterProperties {
                    underline: Some(true),
                    ..Default::default()
                },
            },
        );
        assert_eq!(document.text(), "bold and italic");
        assert!(
            direct(&document, 0).weight == Some(700) || direct(&document, 0).bold == Some(true)
        );
        assert_eq!(direct(&document, 9).slant, Some(FontSlant::Italic));
        for at in 0..15 {
            assert_eq!(direct(&document, at).underline, Some(true));
        }
        let fresh = open(
            std::str::from_utf8(&document.source_bytes()).unwrap(),
            format,
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn rtf_clearing_direct_properties_splits_control_lifetimes_and_preserves_unknown_controls() {
    for source in [r"{\rtf1{\b\unknown42 abcdef}}", r"{\rtf1\b ab{\i\b0 cd}ef}"] {
        let mut document = open(source, Format::Rtf);
        let selected = range(&document, 2, 4);
        apply(
            &mut document,
            PersistedStyleIntent::ClearDirectCharacterProperties {
                range: selected,
                properties: BTreeSet::from([
                    StyleProperty::CharacterWeight,
                    StyleProperty::CharacterBold,
                ]),
            },
        );
        assert_eq!(document.text(), "abcdef");
        assert!(
            direct(&document, 0).weight == Some(700) || direct(&document, 0).bold == Some(true)
        );
        assert_eq!(direct(&document, 2).weight, None);
        assert!(
            direct(&document, 4).weight == Some(700) || direct(&document, 4).bold == Some(true)
        );
        if source.contains("unknown") {
            assert!(String::from_utf8(document.source_bytes())
                .unwrap()
                .contains(r"\unknown42"));
        }
        let fresh = open(
            std::str::from_utf8(&document.source_bytes()).unwrap(),
            Format::Rtf,
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn rich_paragraph_layout_indents_first_hard_line_and_invalidates_spacing_only() {
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    for (format, source) in [
        (
            Format::Html,
            "<p style='text-indent:24pt;margin-block-end:18pt'>one<br>two</p><p>tail</p>",
        ),
        (
            Format::Rtf,
            r"{\rtf1\fi480\sa360 one\line two\par\pard tail}",
        ),
    ] {
        let mut document = open(source, format);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(800.0, 300.0);
        engine.relayout(&document, &mut view).unwrap();
        let rows = &view.snapshot().unwrap().rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].paragraph_id, rows[1].paragraph_id);
        assert_ne!(rows[1].paragraph_id, rows[2].paragraph_id);
        assert_eq!(rows[0].carets[0].x - rows[1].carets[0].x, 24.0);
        assert_eq!(rows[1].y - rows[0].y, rows[0].line_advance);
        assert_eq!(
            rows[2].y - rows[1].y,
            rows[1].line_advance + 18.0 + if format == Format::Html { 7.0 } else { 0.0 }
        );
        let shaped = engine.provider().request_calls();
        let old_y = rows[2].y;
        let selected = range(&document, 0, 1);
        apply(
            &mut document,
            PersistedStyleIntent::SetDirectBlockProperties {
                target: StyleBlockTarget::Paragraphs(selected),
                properties: BlockProperties {
                    spacing_after: Some(30.0),
                    ..Default::default()
                },
            },
        );
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(view.snapshot().unwrap().rows[2].y, old_y + 12.0);
        assert_eq!(
            engine.provider().request_calls(),
            shaped,
            "paragraph spacing must reuse shaped text"
        );
    }
}
#[test]
fn clearing_final_named_rtf_paragraph_keeps_named_and_inline_character_styles() {
    let source = r"{\rtf1{\stylesheet{\s0 Normal;}{\s4\sbasedon0\fs40 Heading 1;}{\*\cs2\i Accent;}}\s4\qc\sa180 one{\cs2\b two}}";
    let mut document = open(source, Format::Rtf);
    let selected = range(&document, 0, 1);
    let spans = document.projection().style_spans().to_vec();
    apply(
        &mut document,
        PersistedStyleIntent::ClearDirectBlockProperties {
            target: StyleBlockTarget::Paragraphs(selected),
            properties: BTreeSet::from([StyleProperty::ParagraphAlignment]),
        },
    );
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("RtfP4")
    );
    assert_eq!(
        document.projection().blocks()[0].direct_paragraph.alignment,
        None
    );
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .spacing_after,
        Some(9.0)
    );
    assert_eq!(document.projection().style_spans(), spans);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn rtf_clears_one_implicit_plain_reset_property_without_changing_other_defaults() {
    for source in [
        r"{\rtf1{\fonttbl{\f0 Arial;}}\b\plain abcdef}",
        r"{\rtf1{\stylesheet{\s0 Normal;}{\*\cs2\i Accent;}}\cs2\plain abcdef}",
    ] {
        let mut document = open(source, Format::Rtf);
        let before = direct(&document, 2);
        let selected = range(&document, 2, 4);
        apply(
            &mut document,
            PersistedStyleIntent::ClearDirectCharacterProperties {
                range: selected,
                properties: BTreeSet::from([StyleProperty::CharacterWeight]),
            },
        );
        assert_eq!(direct(&document, 0).weight, Some(400));
        assert_eq!(direct(&document, 2).weight, None);
        assert_eq!(direct(&document, 4).weight, Some(400));
        let mut expected = before;
        expected.weight = None;
        assert_eq!(direct(&document, 2), expected);
        let fresh = open(
            std::str::from_utf8(&document.source_bytes()).unwrap(),
            Format::Rtf,
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn html_mixed_direct_properties_preserve_existing_other_decoration() {
    let mut document = open("<p><s>word</s></p>", Format::Html);
    let selected = range(&document, 0, 4);
    apply(
        &mut document,
        PersistedStyleIntent::SetDirectCharacterProperties {
            range: selected,
            properties: CharacterProperties {
                weight: Some(700),
                underline: Some(true),
                ..Default::default()
            },
        },
    );
    assert!(direct(&document, 0).weight == Some(700) || direct(&document, 0).bold == Some(true));
    assert_eq!(direct(&document, 0).underline, Some(true));
    assert_eq!(direct(&document, 0).strikethrough, Some(true));
}

#[test]
fn rtf_paragraph_direct_clear_retains_generated_list_and_inline_styles() {
    let source = r"{\rtf1 {\b One} body\par Tail}";
    let mut document = open(source, Format::Rtf);
    document
        .set_list_style(0..0, Some(ListStyle::Numbered))
        .unwrap();
    let list_source = document.source_bytes();
    let selected = range(&document, 0, 1);
    apply(
        &mut document,
        PersistedStyleIntent::SetDirectBlockProperties {
            target: StyleBlockTarget::Paragraphs(selected),
            properties: BlockProperties {
                alignment: Some(ParagraphAlignment::Center),
                ..Default::default()
            },
        },
    );
    let selected = range(&document, 0, 1);
    apply(
        &mut document,
        PersistedStyleIntent::ClearDirectBlockProperties {
            target: StyleBlockTarget::Paragraphs(selected),
            properties: BTreeSet::from([StyleProperty::ParagraphAlignment]),
        },
    );
    assert_eq!(document.text(), "One body\nTail");
    assert_eq!(direct(&document, 0).bold, Some(true));
    assert_eq!(
        document.projection().blocks()[0].direct_paragraph.alignment,
        None
    );
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), list_source);
}
