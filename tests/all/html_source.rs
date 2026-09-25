use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;
fn source(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::HtmlSource).unwrap()
}
#[test]
fn source_is_exact_normalized_decoding_with_lossless_bytes_and_semantic_context() {
    let bytes=b"<!DOCTYPE html>\r\n<p title='keep'><b>Bold &amp; caf\xe9</b></p>\r\n<script>x<y</script><!-- opaque -->";
    let document =
        Document::from_bytes(bytes.to_vec(), Encoding::Latin1, Format::HtmlSource).unwrap();
    assert_eq!(document.source_bytes(), bytes);
    assert_eq!(document.text(),"<!DOCTYPE html>\n<p title='keep'><b>Bold &amp; café</b></p>\n<script>x<y</script><!-- opaque -->");
    let at = document.text().find("Bold").unwrap();
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), at, false)
            .unwrap()
            .bold
    );
    let bracket = document.text().find("<b>").unwrap();
    let authored =
        DocumentLayoutStyles::semantic_character_at(document.projection(), bracket, false).unwrap();
    let colored =
        DocumentLayoutStyles::character_at(document.projection(), bracket, false).unwrap();
    assert!(authored.bold && colored.bold);
    assert_ne!(authored.foreground, colored.foreground);
    // HTML syntax styles have their own contract alongside shared internal overlays.
    let mut html_style_ids = document
        .projection()
        .style_sheet()
        .character_styles()
        .filter(|style| style.id.0.starts_with("* HTML "))
        .map(|style| {
            assert!(style.id.is_internal(), "HTML syntax style {:?}", style.id);
            style.id.0.as_str()
        })
        .collect::<Vec<_>>();
    html_style_ids.sort_unstable();
    assert_eq!(
        html_style_ids,
        [
            "* HTML Attribute key",
            "* HTML Attribute value",
            "* HTML Brackets",
            "* HTML Entity",
            "* HTML Equals",
            "* HTML Tag name",
            "* HTML Uninterpreted",
        ]
    );
}
#[test]
fn source_tag_and_style_edits_synchronize_and_undo_exactly() {
    let original = "<p data-keep='yes'><b>Words</b> plain</p><!--keep-->";
    let mut document = source(original);
    document.replace(21..22, "i").unwrap();
    assert_eq!(
        document.text(),
        String::from_utf8(document.source_bytes()).unwrap()
    );
    document.undo();
    assert_eq!(document.source_bytes(), original.as_bytes());
    let start = document.text().find("plain").unwrap();
    document
        .set_semantic_style(start..start + 5, SemanticInlineStyle::Emphasis, true)
        .unwrap();
    assert!(document.text().contains("<i>plain</i>"));
    assert!(document.text().contains("data-keep='yes'"));
    assert!(document.text().ends_with("<!--keep-->"));
    let at = document.text().find("plain").unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), at, false)
            .unwrap()
            .slant,
        FontSlant::Italic
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
}
#[test]
fn prose_boundary_query_skips_raw_text_tags_entities_and_comments() {
    for text in [
        "<script>here</script>",
        "<style>here</style>",
        "<!--here-->",
        "<p title='here'>Text</p>",
        "<p>&amp;</p>",
    ] {
        let doc = source(text);
        let at = text
            .find("here")
            .or_else(|| text.find('&').map(|at| at + 1))
            .unwrap();
        assert!(
            !doc.html_source_prose_at(at, BoundaryAffinity::Downstream)
                .unwrap(),
            "{text}"
        );
    }
    let doc = source("<b>here</b>");
    assert!(doc
        .html_source_prose_at(3, BoundaryAffinity::Downstream)
        .unwrap());
    assert!(source("")
        .html_source_prose_at(0, BoundaryAffinity::Downstream)
        .unwrap());
}
#[test]
fn isolated_source_paragraph_edits_reparse_locally_in_large_document() {
    let mut doc = source(&"<p><b>Words</b> plain</p>\n".repeat(10_000));
    let last = doc.projection().blocks()[9_999].id;
    let line = doc.projection().hard_line_range(5_000).unwrap();
    let at = line.start + 7;
    let prepared = doc
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(),
            revision: doc.revision(),
            edits: vec![TextEdit::new(at..at + 1, "X")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(
        work.scope(),
        ProjectionWorkScope::RegionalHardLines,
        "{work:?}"
    );
    assert!(work.decoded_source_bytes() < 256);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    doc.commit_model_transaction(prepared).unwrap();
    assert_eq!(doc.projection().blocks()[9_999].id, last);
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
    assert_eq!(
        doc.projection().provenance(),
        fresh.projection().provenance()
    );
}
#[test]
fn internal_palette_edits_are_sparse_undoable_configuration_and_identity_is_protected() {
    let mut doc = source("<p><b>Bold</b></p>");
    let original = doc.source_bytes();
    let id: StyleId = "* HTML Tag name".into();
    let mut style = doc
        .projection()
        .style_sheet()
        .character_style(&id)
        .unwrap()
        .clone();
    let before = style.clone();
    style.properties.foreground = Some(Color {
        red: 0.2,
        green: 0.4,
        blue: 0.8,
        alpha: 1.0,
    });
    let apply = |doc: &mut Document, edit| {
        let request = StyleModelRequest::new(
            doc.id(),
            doc.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit)),
        );
        doc.apply_style_request(request)
    };
    apply(
        &mut doc,
        StyleDefinitionEdit::UpdateCharacter(style.clone()),
    )
    .unwrap();
    assert_eq!(doc.source_bytes(), original);
    assert_eq!(
        doc.projection().style_sheet().character_style(&id),
        Some(&style)
    );
    assert!(
        DocumentLayoutStyles::character_at(doc.projection(), 4, false)
            .unwrap()
            .bold
    );
    doc.insert(8, "X").unwrap();
    assert_eq!(
        doc.projection().style_sheet().character_style(&id),
        Some(&style)
    );
    assert!(doc.undo());
    assert!(doc.undo());
    assert_eq!(
        doc.projection().style_sheet().character_style(&id),
        Some(&before)
    );
    let mut parent = before.clone();
    parent.based_on = Some("Code".into());
    assert!(apply(&mut doc, StyleDefinitionEdit::UpdateCharacter(parent)).is_err());
    assert!(apply(&mut doc, StyleDefinitionEdit::DeleteCharacter(id.clone())).is_err());
    assert!(apply(
        &mut doc,
        StyleDefinitionEdit::UpdateMetadata {
            namespace: StyleNamespace::Character,
            id: id.clone(),
            metadata: StyleDefinitionMetadata {
                display_name: "Renamed".into(),
                origin: StyleDefinitionOrigin::GeneratedConfiguration
            }
        }
    )
    .is_err());
    let range = TextRange::new(doc.text_point(6).unwrap(), doc.text_point(10).unwrap()).unwrap();
    let request = StyleModelRequest::new(
        doc.id(),
        doc.revision(),
        StyleModelIntent::Persisted(PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: id,
        }),
    );
    assert!(doc.apply_style_request(request).is_err());
    assert_eq!(doc.source_bytes(), original);
}
#[test]
fn source_internal_palette_invalidation_separates_paint_and_metrics_in_large_document() {
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let mut doc = source(&"<p><b>Words</b> plain</p>\n".repeat(200));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600., 100.);
    engine.set_cache_capacity(210);
    engine.relayout(&doc, &mut view).unwrap();
    let original = doc.source_bytes();
    let calls = engine.provider().request_calls();
    let edit = |doc: &mut Document, property, value| {
        let definition = doc
            .projection()
            .style_sheet()
            .prepare_generated_field_edit(
                StyleNamespace::Character,
                &"* HTML Tag name".into(),
                &StyleDefinitionFieldEdit::SetDeclaration { property, value },
            )
            .unwrap();
        doc.apply_style_request(StyleModelRequest::new(
            doc.id(),
            doc.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(definition)),
        ))
        .unwrap();
    };
    edit(
        &mut doc,
        StyleProperty::CharacterForeground,
        StylePropertyValue::Color(Color {
            red: 0.1,
            green: 0.2,
            blue: 0.3,
            alpha: 1.,
        }),
    );
    engine.relayout(&doc, &mut view).unwrap();
    assert_eq!(
        engine.provider().request_calls(),
        calls,
        "paint should reuse shaping"
    );
    edit(
        &mut doc,
        StyleProperty::CharacterSize,
        StylePropertyValue::Float(28.),
    );
    engine.relayout(&doc, &mut view).unwrap();
    assert!(engine.provider().request_calls() > calls);
    assert_eq!(doc.source_bytes(), original);
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false)
            .unwrap()
            .size,
        14.0
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(doc.projection(), 1, false)
            .unwrap()
            .size,
        28.0
    );
    let calls = engine.provider().request_calls();
    let at = doc.projection().hard_line_range(100).unwrap().start + 8;
    doc.insert(at, "X").unwrap();
    engine.relayout(&doc, &mut view).unwrap();
    assert!(
        engine.provider().request_calls() - calls <= 1,
        "one source line changed"
    );
}
#[test]
fn raw_tag_edits_recompute_cross_line_context_and_do_not_reuse_unsafe_checkpoints() {
    let mut doc = source("<b>\n<p>Words</p>\n<p>Tail</p>\n</b>");
    let at = doc.text().find("Words").unwrap();
    assert!(
        DocumentLayoutStyles::character_at(doc.projection(), at, false)
            .unwrap()
            .bold
    );
    doc.replace(1..2, "i").unwrap();
    let at = doc.text().find("Words").unwrap();
    let style = DocumentLayoutStyles::character_at(doc.projection(), at, false).unwrap();
    assert!(!style.bold);
    assert_eq!(style.slant, FontSlant::Italic);
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
    let at = doc.text().find("Words").unwrap() + 2;
    doc.insert(at, "X").unwrap();
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
}
#[test]
fn heading_and_list_actions_modify_visible_source_and_preserve_unknown_bytes() {
    let original = "<p data-x='keep'>Words</p>\n<p>Tail</p><!--opaque-->";
    let mut doc = source(original);
    doc.set_paragraph_style(16..21, "Heading2".into()).unwrap();
    assert!(doc.text().starts_with("<h2 data-x='keep'>Words</h2>"));
    doc.undo();
    doc.set_list_style(16..21, Some(ListStyle::Bullet)).unwrap();
    assert!(doc.text().contains("<ul>"));
    assert!(doc.text().ends_with("<!--opaque-->"));
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
    doc.undo();
    assert_eq!(doc.source_bytes(), original.as_bytes());
}
#[test]
fn inherited_source_prose_typing_is_regional_and_matches_fresh_context() {
    let original = format!(
        "<div style='font-size:20pt'><b>\n{}\n</b></div>",
        "words and words\n".repeat(10_000)
    );
    let mut doc = source(&original);
    let last = doc.projection().blocks()[9_999].id;
    for (offset, length, value) in [(0, 0, "X"), (4, 1, "Y"), (15, 0, "X"), (3, 3, "")] {
        let line = doc.projection().hard_line_range(5_000).unwrap();
        let at = line.start + offset;
        let prepared = doc
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: doc.id(),
                revision: doc.revision(),
                edits: vec![TextEdit::new(at..at + length, value)],
            })
            .unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::RegionalHardLines
        );
        assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
        doc.commit_model_transaction(prepared).unwrap();
        let style =
            DocumentLayoutStyles::character_at(doc.projection(), line.start, false).unwrap();
        assert!(style.bold);
        assert_eq!(style.size, 20.);
    }
    assert_eq!(doc.projection().blocks()[9_999].id, last);
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
    assert_eq!(
        doc.projection().provenance(),
        fresh.projection().provenance()
    );
}
#[test]
fn source_attribute_style_edit_restarts_only_its_balanced_paragraph() {
    let mut doc = source(&"<p><span style='font-size:14pt'>Words</span></p>\n".repeat(10_000));
    let line = doc.projection().hard_line_range(5_000).unwrap();
    let at = line.start + 26;
    assert_eq!(&doc.text()[at..at + 2], "14");
    let prepared = doc
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(),
            revision: doc.revision(),
            edits: vec![TextEdit::new(at..at + 2, "28")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    doc.commit_model_transaction(prepared).unwrap();
    let at = line.start + 32;
    assert_eq!(
        DocumentLayoutStyles::character_at(doc.projection(), at, false)
            .unwrap()
            .size,
        28.
    );
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
}
#[test]
fn html_whitespace_owns_its_style_before_an_inline_wrapper() {
    let mut doc = Document::from_bytes(
        b"<p><b>Words</b> plain</p>".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    doc.set_semantic_style(6..11, SemanticInlineStyle::Emphasis, true)
        .unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(doc.projection(), 5, false)
            .unwrap()
            .slant,
        FontSlant::Upright
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(doc.projection(), 6, false)
            .unwrap()
            .slant,
        FontSlant::Italic
    );
}
#[test]
fn wysiwyg_tail_heading_assignment_preserves_intervening_raw_elements() {
    let source="<h1>Heading</h1>\n<p title=\"example\" data-key=\"kept\">Body <b>bold</b> &amp; entities.</p>\n<script>const quote = \"raw\";</script>\n<style>.lead { color: red; }</style>\n<p>Tail paragraph.</p>\n";
    let mut doc =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    doc.set_format(
        Format::HtmlSource,
        viem_core::document::FormatOperation::Reinterpret,
    )
    .unwrap();
    doc.set_format(
        Format::Html,
        viem_core::document::FormatOperation::Reinterpret,
    )
    .unwrap();
    let at = doc.text().find("Tail").unwrap();
    doc.set_paragraph_style(at..at, "Heading1".into()).unwrap();
    assert!(String::from_utf8(doc.source_bytes())
        .unwrap()
        .ends_with("<h1>Tail paragraph.</h1>\n"));
    doc.undo();
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

fn assert_spans_equal(left: &Document, right: &Document) {
    let a = left.projection().style_spans();
    let b = right.projection().style_spans();
    assert_eq!(a.len(), b.len(), "style span count");
    if let Some((index, (a, b))) = a.iter().zip(b).enumerate().find(|(_, (a, b))| a != b) {
        panic!("first differing span {index}: {a:?} != {b:?}");
    }
}
#[test]
fn raw_cross_line_style_edit_invalidates_already_visible_following_rows() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(source(
        "<span style='font-size:14pt'>\nWords\nTail\n</span>",
    ));
    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 300.);
    let width = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.hard_line_index == 1)
        .unwrap()
        .width;
    let at = core.document().text().find("14pt").unwrap();
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
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('r'))))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('2'))))
        .unwrap();
    assert!(core.document().text().contains("24pt"));
    let after = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.hard_line_index == 1)
        .unwrap()
        .width;
    assert!(after>width,"style change in a preceding source line must invalidate following shaped rows: {width} -> {after}");
}

#[test]
fn multiline_tokens_do_not_invent_prose_boundaries() {
    use viem_core::command::{CommandInterpreter, InputEvent};
    for text in [
        "<!--\nhere-->",
        "<p title=\n\"here\">text</p>",
        "<p\n title='here'>text</p>",
        "<script\n type='text/javascript'>here</script>",
    ] {
        let at = text.find('\n').unwrap() + 1;
        let doc = source(text);
        assert!(
            !doc.html_source_prose_at(at, BoundaryAffinity::Downstream)
                .unwrap(),
            "{text:?}"
        );
        for authored in ["\"", "<"] {
            let mut doc = source(text);
            let mut commands = CommandInterpreter::new();
            commands.set_smart_quotes(true);
            commands.set_cursor(&doc, at);
            commands.handle(&mut doc, InputEvent::key('i')).unwrap();
            commands
                .handle(&mut doc, InputEvent::text(authored))
                .unwrap();
            let mut expected = text.to_owned();
            expected.insert_str(at, authored);
            assert_eq!(doc.text(), expected);
        }
    }
    let mut doc = source("<!--\ncomment-->Here prose");
    let at = doc.text().find("prose").unwrap() + 2;
    doc.insert(at, "X").unwrap();
    let fresh = source(doc.text());
    assert_spans_equal(&doc, &fresh);
}

#[test]
fn bounded_html_prose_context_decodes_entities_and_skips_complete_opaque_tokens() {
    for (text, expected) in [
        ("<p>&nbsp;", '\u{a0}'),
        ("<p>&#45;", '-'),
        ("<p>&lpar;", '('),
        ("Words<b></b>", 's'),
        ("Words<!-- > hidden -->", 's'),
        ("Words<script>hidden > text</script>", 's'),
        ("Words<style>.x { color: red; }</style>", 's'),
        ("Words</p><h1>", '\n'),
    ] {
        let doc = source(text);
        let context = doc
            .html_source_prose_prefix(text.len(), 4096)
            .unwrap()
            .unwrap();
        assert_eq!(context.chars().last(), Some(expected), "{text}");
    }
    let text = "<script>x</script><span>";
    assert_eq!(
        source(text)
            .html_source_prose_prefix(text.len(), 4096)
            .unwrap(),
        Some(String::new())
    );
    let text = format!("<span title='{}'>", "x".repeat(10_000));
    assert_eq!(
        source(&text)
            .html_source_prose_prefix(text.len(), 4096)
            .unwrap(),
        None
    );
    let text = format!("word<!-- {} -->", "x".repeat(10_000));
    assert_eq!(
        source(&text)
            .html_source_prose_prefix(text.len(), 4096)
            .unwrap(),
        None
    );
    let text = format!("word<p title='{}'>", "x".repeat(10_000));
    assert_eq!(
        source(&text)
            .html_source_prose_prefix(text.len(), 4096)
            .unwrap(),
        Some("\n".into())
    );
}
