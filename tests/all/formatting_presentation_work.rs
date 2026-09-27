use viem_core::command::InputEvent;
use viem_core::document::*;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

#[test]
fn selected_inline_style_queries_only_read_local_contributors() {
    for count in [32, 4_000] {
        for (format, source) in [
            (Format::Markdown, format!("Words\n\n{}", "A **bold** paragraph.\n\n".repeat(count))),
            (Format::MarkdownSource, format!("Words\n\n{}", "A **bold** paragraph.\n\n".repeat(count))),
            (Format::Rtf, format!("{{\\rtf1 Words\\par {}}}", "A {\\b bold} paragraph.\\par ".repeat(count))),
        ] {
            let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
            core.handle(view, CoreEvent::Input(InputEvent::key('v'))).unwrap();
            let (presentation, work) = measure_document_work(|| core.selection_semantic_style_presentation(view, SemanticInlineStyle::Strong).unwrap());
            assert!(presentation.can_set() && presentation.can_clear() == format.has_rich_source());
            assert_eq!(work.source_full_materializations, 0, "{format:?}: {work:?}");
            assert_eq!(work.source_decoded_bytes, 0, "{format:?}: {work:?}");
            assert_eq!(work.full_projection_candidates, 0, "{format:?}: {work:?}");
            assert_eq!(work.scratch_documents, 0, "{format:?}: {work:?}");
        }
    }
}

#[test]
fn markdown_inline_toggles_reparse_only_the_selected_paragraph() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = format!("- First\n- Target words\n\n{}", "Unrelated **bold** paragraph.\n\n".repeat(4_000));
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let at = document.text().find("Target").unwrap();
        let (_, work) = measure_document_work(|| document.set_semantic_style(at..at + 6, SemanticInlineStyle::Strong, true).unwrap());
        assert_eq!(work.full_projection_candidates, 0, "{format:?}: {work:?}");
        assert_eq!(work.source_full_materializations, 0, "{format:?}: {work:?}");
        assert!(work.source_decoded_bytes < 2048, "{format:?}: {work:?}");
        let styled = document.source_bytes();
        let reopened = Document::from_bytes(styled.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), reopened.text());
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), styled);
    }
}

#[test]
fn rtf_inline_toggles_parse_only_the_generated_character_group() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = format!("{{\\rtf1{{\\fonttbl{{\\f0 Helvetica;}}}}{{\\colortbl;\\red255\\green0\\blue0;}}First\\par {{\\i\\cf1 Target words}}\\par {}}}",
        "Unrelated {\\b bold} paragraph.\\par ".repeat(4_000));
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let at = document.text().find("Target").unwrap();
    for properties in [
        CharacterProperties { bold: Some(true), ..Default::default() },
        CharacterProperties { bold: Some(false), ..Default::default() },
        CharacterProperties { underline: Some(true), ..Default::default() },
        CharacterProperties { script_position: Some(ScriptPosition::Superscript), ..Default::default() },
    ] {
        let range = TextRange::new(document.text_point(at).unwrap(), document.text_point(at + 6).unwrap()).unwrap();
        let request = StyleModelRequest::new(document.id(), document.revision(), StyleModelIntent::Persisted(
            PersistedStyleIntent::SetDirectCharacterProperties { range, properties }));
        let (_, work) = measure_document_work(|| document.apply_style_request(request).unwrap());
        assert_eq!(work.full_projection_candidates, 0, "{work:?}");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert!(work.source_decoded_bytes < 2048, "{work:?}");
        let saved = document.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Rtf).unwrap();
        assert_eq!(document.text(), reopened.text());
        for offset in [0, at, at + 6, document.text().len() - 2] {
            assert_eq!(DocumentLayoutStyles::character_at(document.projection(), offset, false).unwrap(),
                DocumentLayoutStyles::character_at(reopened.projection(), offset, false).unwrap());
        }
        assert!(document.undo());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn source_selection_style_command_retains_bounded_document_work() {
    let source = format!("Target\n\n{}",
        "A paragraph with **bold** and `code` text.\n\n".repeat(4_000));
    let at = source.find("Target").unwrap();
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 900., 350.);
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    for key in "v5l".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
    }
    let expected = core.selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
        .unwrap().selection().unwrap().clone();
    assert_eq!(expected.range(), at..at + 6);
    let (_, work) = measure_document_work(|| core.handle(view, CoreEvent::SetSelectionSemanticStyle {
        expected, style: SemanticInlineStyle::Strong, enabled: true,
    }).unwrap());
    assert!(work.source_decoded_bytes < 16384, "{work:?}");
    assert!(work.source_full_materialized_bytes < 16384, "{work:?}");
}

#[test]
fn character_formatting_invalidates_cached_layout_and_undo_restores_it() {
    let source = format!(r"{{\rtf1 Target\par {}}}", r"Unrelated text\par ".repeat(4_000));
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 900., 350.);
    let widths = |core: &Core<MockTextMeasurementProvider>| core.layout(view).unwrap().snapshot().unwrap()
        .rows.iter().take(2).map(|row| row.width).collect::<Vec<_>>();
    let before = widths(&core);
    for key in "v5l".chars() { core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap(); }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::EditDirectProperty { expected, property: StyleProperty::CharacterSize,
        value: Some(StylePropertyValue::Float(28.0)) }).unwrap();
    let after = widths(&core);
    assert!(after[0] > before[0]);
    assert_eq!(after[1], before[1]);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(widths(&core), before);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(widths(&core), after);
}
