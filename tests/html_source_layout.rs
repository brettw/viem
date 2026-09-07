use evim_core::document::{Encoding, Format};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent, Document};

#[test]
fn consecutive_source_breaks_never_create_empty_semantic_style_spans() {
    for source in [
        "<pre>first\n\nsecond</pre>",
        "<pre>first\r\n\r\n\r\nsecond</pre>",
        "<p title='first\n\nsecond'>café &amp; text</p>\n\n<!--one\n\ntwo-->",
        "<pre>first<br><span style='white-space: pre-wrap'>&#32;</span>added\n\nlast</pre>",
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap();
        assert!(
            document
                .projection()
                .style_spans()
                .iter()
                .all(|span| !span.range.is_empty()),
            "{source:?}"
        );
        DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn large_source_code_with_blank_lines_keeps_resize_and_zoom_layout_bounded() {
    let source = "<pre>first\n\nlast</pre>\n".repeat(10_000);
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 640., 150.);
    let original_configuration = core.layout(view).unwrap().configuration_generation();
    core.handle(view, CoreEvent::SetScale(1.75)).unwrap();
    for width in [200., 800.] {
        core.handle(
            view,
            CoreEvent::Resize {
                width,
                height: 150.,
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap();
        assert!(layout.configuration_generation() > original_configuration);
        let snapshot = layout.snapshot().unwrap();
        assert!(!snapshot.rows.is_empty());
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.coverage.hard_lines().len() < 100);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision().0, 0);
}
