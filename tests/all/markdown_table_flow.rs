use viem_core::document::{Encoding, Format};
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, LayoutCancellationToken,
    LayoutEngine, LayoutExecutionContext, LayoutJobId, LayoutJobPriority, LayoutJobRegion,
    LayoutSnapshot, MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
};
use viem_core::{Core, Document};

fn styled_document(source: &str) -> Document {
    styled_document_in_format(source, Format::Markdown)
}

fn styled_document_in_format(source: &str, format: Format) -> Document {
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let diagnostics = document.replace_style_defaults(br#"{
        "version":1,
        "block_styles":[
            {"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","block":{
                "margin_top":11,"margin_bottom":13,"padding_top":7,"padding_bottom":5,
                "border_top_width":3,"border_bottom_width":2}},
            {"id":"Table","name":"Table","based_on":"Paragraph","role":"Table","block":{
                "margin_top":31,"margin_bottom":37,"padding_top":17,"padding_bottom":19,
                "border_top_width":23,"border_bottom_width":29}},
            {"id":"Table cell","name":"Table cell","based_on":"Paragraph","role":"Paragraph","block":{
                "margin_top":301,"margin_bottom":307,"padding_top":4,"padding_bottom":6,
                "border_top_width":1,"border_bottom_width":1}},
            {"id":"Table header","name":"Table header","based_on":"Table cell","role":"Paragraph","block":{},"character":{"bold":true}},
            {"id":"Heading2","name":"Heading 2","based_on":"Paragraph","role":"Paragraph","block":{
                "margin_top":41,"margin_bottom":43,"padding_top":11,"padding_bottom":13,
                "border_top_width":5,"border_bottom_width":7}}
        ]
    }"#).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    document
}

fn row_y(snapshot: &LayoutSnapshot, offset: usize) -> f32 {
    snapshot
        .rows
        .iter()
        .find(|row| row.text_range.start == offset)
        .unwrap()
        .y
}

#[test]
fn table_container_and_following_block_share_normal_flow_in_full_and_regional_layout() {
    for suffix in ["\n\nAfter", "\n## After", "\n\n## After"] {
        let source = format!("Before\n\n| A | B |\n| - | - |\n| x | y |{suffix}");
        let document = styled_document(&source);
        let after = document.text().find("After").unwrap();
        let heading = suffix.contains("##");
        let mut full_view = ViewLayout::new(1200., 1000.);
        LayoutEngine::new(MockTextMeasurementProvider::new())
            .relayout(&document, &mut full_view)
            .unwrap();
        let full = full_view.snapshot().unwrap();
        let table = &full.tables()[0];
        let before = &full.rows[0];
        let expected_top = before.y + before.height() + 5. + 2. + 31.;
        assert!(
            (table.rect.y - expected_top).abs() < 0.01,
            "{suffix:?}: table top={} expected={expected_top}",
            table.rect.y
        );
        let gap = if heading {
            41. + 11. + 5.
        } else {
            37. + 7. + 3.
        };
        let expected_after = table.rect.y + table.rect.height + gap;
        assert!(
            (row_y(full, after) - expected_after).abs() < 0.01,
            "{suffix:?}: after y={} expected={expected_after}, table={table:?}",
            row_y(full, after)
        );
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 1200., 1000.);
        let regional = core.layout(view).unwrap().snapshot().unwrap();
        assert!((regional.tables()[0].rect.y - table.rect.y).abs() < 0.01);
        assert!((regional.tables()[0].rect.height - table.rect.height).abs() < 0.01);
        assert!(
            (row_y(regional, after) - row_y(full, after)).abs() < 0.01,
            "{suffix:?}: regional={} full={}",
            row_y(regional, after),
            row_y(full, after)
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn huge_cell_tree_layout_preserves_the_gap_after_its_table() {
    let source = format!(
        "| H | V |\n| - | - |\n| {} | value |\n## After",
        "wide ".repeat(2500)
    );
    let document = styled_document(&source);
    let after = document.text().find("After").unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 1200., 1000.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let table = &snapshot.tables()[0];
    let expected = table.rect.y + table.rect.height + 41. + 11. + 5.;
    assert!(
        (row_y(snapshot, after) - expected).abs() < 0.01,
        "after={} expected={expected}, table={table:?}",
        row_y(snapshot, after)
    );
}

#[test]
fn giant_prose_before_table_preserves_following_container_spacing() {
    let source = format!("{}\n\n| A | B |\n| - | - |\n| x | y |", "x".repeat(70_000));
    for format in [Format::Markdown, Format::MarkdownSource] {
        for wrap in [false, true] {
            let document = styled_document_in_format(&source, format);
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            let mut view = ViewLayout::new(1000., 1000.);
            view.set_wrap(wrap);
            engine.relayout(&document, &mut view).unwrap();
            let full = view.snapshot().unwrap();
            let table_top = if format == Format::Markdown {
                full.tables()[0].rect.y
            } else {
                full.rows[1].y
            };
            // Capture only the giant prose line. Its trailing flow owns the
            // table's margin even when no table row is materialized yet.
            let request = prepare_layout_job(
                &document,
                &mut view,
                inspect_layout_provider(&engine),
                LayoutJobId(1),
                LayoutJobPriority::ChangedVisibleRows,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0., 1000.).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
            let result =
                compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            let line = &result.regional_snapshot().lines()[0];
            assert!(line.height_is_exact());
            assert!(
                (line.height() as f32 - table_top).abs() < 0.01,
                "{format:?} wrap={wrap}: preceding height={} full table top={table_top}",
                line.height()
            );
        }
    }
}
