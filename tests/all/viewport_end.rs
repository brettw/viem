use viem_core::layout::{
    EdgeInsets, MockTextMeasurementProvider, MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use viem_core::{Core, CoreEvent, Document, ViewId};

fn request_end(core: &mut Core<MockTextMeasurementProvider>, view: ViewId) {
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(f32::MAX),
        },
    )
    .unwrap();
}
fn assert_end_fills_viewport(core: &Core<MockTextMeasurementProvider>, view: ViewId) {
    let layout = core.layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    assert_eq!(
        snapshot.rows.last().unwrap().text_range.end,
        core.document().text().len()
    );
    let end = snapshot.coverage.vertical_range().unwrap().end;
    assert!(
        (layout.viewport_top() + layout.height() - end).abs() < 0.1,
        "top={} height={} end={end}",
        layout.viewport_top(),
        layout.height()
    );
    assert!(snapshot.rows.len() < 100, "{} rows", snapshot.rows.len());
}

#[test]
fn estimated_document_end_extends_short_final_band_backwards_to_fill_viewport() {
    let source = "short\n".repeat(20_000);
    let mut core = Core::new(Document::new(&source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    core.set_view_insets(
        view,
        EdgeInsets {
            top: 18.,
            bottom: 28.,
            left: 10.,
            right: 10.,
        },
    )
    .unwrap();
    request_end(&mut core, view);
    assert_end_fills_viewport(&core, view);
    assert!(
        !core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .total_height_is_exact
    );
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn wrapped_document_end_resumes_chunks_and_publishes_only_the_terminal_viewport() {
    for length in [200_000, MAX_LONG_LINE_LAYOUT_SLICE_BYTES * 3 + 1] {
        let source = "word ".repeat(length / 5) + &"x".repeat(length % 5);
        let mut core = Core::new(Document::new(&source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        core.set_view_insets(
            view,
            EdgeInsets {
                top: 18.,
                bottom: 28.,
                left: 10.,
                right: 10.,
            },
        )
        .unwrap();
        request_end(&mut core, view);
        assert_end_fills_viewport(&core, view);
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows[0].fragment_index > 0);
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(0.),
            },
        )
        .unwrap();
        request_end(&mut core, view);
        assert_end_fills_viewport(&core, view);
        core.handle(
            view,
            CoreEvent::Resize {
                width: 350.,
                height: 300.,
            },
        )
        .unwrap();
        request_end(&mut core, view);
        assert_end_fills_viewport(&core, view);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn trailing_empty_and_short_lines_share_only_the_visible_long_paragraph_tail() {
    let long = "word ".repeat(40_000);
    for (prefix, suffix) in [
        (String::new(), "\n"),
        (String::new(), "\nshort\nlast\n"),
        ("short\n".repeat(20_000), "\nshort\n\nlast"),
    ] {
        let source = prefix + &long + suffix;
        let mut core = Core::new(Document::new(&source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        request_end(&mut core, view);
        assert_end_fills_viewport(&core, view);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(
            snapshot.rows.first().unwrap().fragment_index > 0,
            "the long paragraph contributes only its final visual rows"
        );
        assert!(snapshot.rows.first().unwrap().text_range.start > source.len() - 5_000);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let source = format!("<p><b>{long}</b></p><h2>Tail</h2><p></p>");
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        viem_core::document::Encoding::Utf8,
        viem_core::document::Format::Html,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    request_end(&mut core, view);
    assert_end_fills_viewport(&core, view);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
