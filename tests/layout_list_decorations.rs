use viem_core::document::{Encoding, Format, TextRange, WritingDirection};
use viem_core::layout::{
    BoundaryAffinity, DocumentLayoutStyles, LayoutEngine, LayoutPoint, MetricsGeneration,
    MockTextMeasurementProvider, ViewLayout,
};
use viem_core::{Core, CoreEvent, Document};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}
fn label(row: &viem_core::layout::VisualRow) -> String {
    row.decorations
        .iter()
        .map(|item| item.text.as_str())
        .collect()
}

#[test]
fn labels_are_furniture_outside_body_selection_and_empty_item_carets() {
    let source = "<ol start='9'><li>Alpha words wrap into several rows here</li><li></li><li><p>Third</p><p>continuation</p></li></ol>";
    let document = html(source);
    assert_eq!(
        document.text(),
        "Alpha words wrap into several rows here\n\nThird\ncontinuation"
    );
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(160., 500.);
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    let labels: Vec<_> = snapshot
        .rows
        .iter()
        .map(label)
        .filter(|label| !label.is_empty())
        .collect();
    assert_eq!(labels, ["9.", "10.", "11."]);
    for row in &snapshot.rows {
        for marker in &row.decorations {
            assert!(marker.x + marker.advance < row.paragraph_content_x);
        }
        if row.fragment_index > 0 {
            assert!(row.decorations.is_empty());
        }
    }
    let empty = snapshot
        .rows
        .iter()
        .find(|row| row.text_range.is_empty())
        .unwrap();
    assert_eq!(label(empty), "10.");
    assert!(!empty.carets.is_empty());
    assert!(empty.clusters.is_empty());
    assert!(empty.ink_bounds().is_some());
    let marker = &snapshot.rows[0].decorations[0];
    assert_eq!(
        snapshot
            .hit_test(LayoutPoint {
                x: marker.x,
                y: snapshot.rows[0].y + 1.
            })
            .unwrap()
            .text_offset,
        0
    );
    let selection = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    let rectangles = snapshot
        .selection_rectangles(selection, BoundaryAffinity::Downstream)
        .unwrap();
    assert!(rectangles
        .iter()
        .all(|rect| rect.rect.x >= snapshot.rows[0].paragraph_content_x));
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn marker_shape_cache_tracks_style_spelling_zoom_metrics_and_paint() {
    let document = html("<ol><li>Body</li></ol>");
    let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(200., 100.);
    let run =
        |engine: &mut LayoutEngine<_>, view: &mut ViewLayout, styles: DocumentLayoutStyles| {
            engine
                .relayout_styled_text(
                    document.id(),
                    document.revision(),
                    document.text(),
                    styles,
                    view,
                )
                .unwrap();
        };
    run(&mut engine, &mut view, styles.clone());
    let count = engine.provider().request_calls();
    let first = view.snapshot().unwrap().rows[0].decorations[0].clone();
    view.resize(260., 100.);
    run(&mut engine, &mut view, styles.clone());
    assert_eq!(engine.provider().request_calls(), count);
    styles.paragraphs[0].marker_paint.underline = true;
    run(&mut engine, &mut view, styles.clone());
    assert_eq!(engine.provider().request_calls(), count);
    assert!(
        view.snapshot().unwrap().rows[0].decorations[0]
            .paint
            .underline
    );
    styles.paragraphs[0].list_marker_decoration = Some("200.".into());
    run(&mut engine, &mut view, styles.clone());
    assert_eq!(engine.provider().request_calls(), count + 1);
    assert_eq!(label(&view.snapshot().unwrap().rows[0]), "200.");
    styles.paragraphs[0].default_shaping_style.size = 28.;
    run(&mut engine, &mut view, styles.clone());
    assert!(view.snapshot().unwrap().rows[0].decorations[0].advance > first.advance);
    view.set_scale(2.).unwrap();
    run(&mut engine, &mut view, styles.clone());
    assert_eq!(
        view.snapshot().unwrap().rows[0].decorations[0].font_size,
        56.
    );
    let count = engine.provider().request_calls();
    engine
        .provider_mut()
        .set_metrics_generation(MetricsGeneration(2));
    run(&mut engine, &mut view, styles);
    assert!(engine.provider().request_calls() > count);
}

#[test]
fn rtl_markers_use_right_gutter_and_source_modes_keep_literal_text() {
    let document = html("<ol start='12'><li dir='rtl'>2026 שלום עולם</li></ol>");
    let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    styles.paragraphs[0].base_direction = WritingDirection::RightToLeft;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(240., 100.);
    engine
        .relayout_styled_text(
            document.id(),
            document.revision(),
            document.text(),
            styles,
            &mut view,
        )
        .unwrap();
    let row = &view.snapshot().unwrap().rows[0];
    assert_eq!(label(row), "12.");
    assert!(row.decorations[0].x > row.paragraph_content_x + row.paragraph_content_width);
    for (format, source) in [
        (Format::MarkdownSource, "1. source"),
        (Format::HtmlSource, "<ol><li>source</li></ol>"),
        (Format::PlainText, "1. plain"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(document.text(), source);
        assert!(view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .all(|row| row.decorations.is_empty()));
    }
}

#[test]
fn large_list_regional_marker_layout_is_bounded_and_rebased_with_height_changes() {
    let source = format!(
        "<ol>{}</ol>",
        "<li>Body text with enough words for wrapping and more content</li>".repeat(10_000)
    );
    let mut core = Core::new(html(&source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240., 100.);
    for (scale, width) in [(1., 240.), (2., 160.), (0.5, 320.)] {
        core.handle(view, CoreEvent::SetScale(scale)).unwrap();
        core.handle(
            view,
            CoreEvent::Resize {
                width,
                height: 100.,
            },
        )
        .unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.rows.len() < 500, "rows {}", snapshot.rows.len());
        assert!(
            snapshot
                .rows
                .iter()
                .map(|row| row.decorations.len())
                .sum::<usize>()
                < 100
        );
        assert!(snapshot.rows.iter().any(|row| !row.decorations.is_empty()));
        for row in &snapshot.rows {
            for item in &row.decorations {
                assert!(item.typographic_bounds.y >= row.y - 1.);
                assert!(item.typographic_bounds.y <= row.baseline);
            }
        }
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision().0, 0);
}
