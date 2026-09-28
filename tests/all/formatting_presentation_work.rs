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

        ] {
            let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
            core.handle(view, CoreEvent::Input(InputEvent::key('v'))).unwrap();
            let (presentation, work) = measure_document_work(|| core.selection_semantic_style_presentation(view, SemanticInlineStyle::Strong).unwrap());
            assert!(presentation.can_set() && !presentation.can_clear());
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
