use evim_core::command::{InputEvent, Key};
use evim_core::document::{BoundaryAffinity, Encoding, Format};
use evim_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, HardLineLayoutRegion,
    LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutJobId, LayoutJobPriority,
    LayoutJobRegion, MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use evim_core::{Core, CoreEvent, Document};

fn source_document(source: &str) -> Document {
    Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap()
}

fn spaced_document(source: &str) -> Document {
    let mut document = source_document(source);
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["block"]["spacing_before"] = 7.into();
            style["block"]["spacing_after"] = 11.into();
            style["block"]["first_line_indent"] = 10.into();
        }
    }
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    document
}

#[test]
fn source_group_spacing_and_indent_apply_once_with_flow_off_or_on() {
    let source = "First **strong** line\nsecond line.\n\nThird paragraph\nfourth line.";
    let document = spaced_document(source);
    assert_eq!(
        document.text(),
        "First **strong** line\nsecond line.\nThird paragraph\nfourth line."
    );
    assert_eq!(document.projection().blocks().len(), 2);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(800., 300.);
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    assert_eq!(snapshot.rows.len(), 4);
    let rows = &snapshot.rows;
    assert_eq!(rows[0].paragraph_id, rows[1].paragraph_id);
    assert_eq!(rows[2].paragraph_id, rows[3].paragraph_id);
    assert_ne!(rows[0].paragraph_id, rows[2].paragraph_id);
    assert!((rows[1].y - rows[0].y - rows[0].height()).abs() < 0.001);
    assert!((rows[2].y - rows[1].y - rows[1].height() - 18.).abs() < 0.001);
    assert!((rows[3].y - rows[2].y - rows[2].height()).abs() < 0.001);
    assert!((rows[0].paragraph_content_x - rows[1].paragraph_content_x - 10.).abs() < 0.001);
    assert!((rows[2].paragraph_content_x - rows[3].paragraph_content_x - 10.).abs() < 0.001);
    let physical = snapshot.clone();
    view.set_paragraph_flow(true);
    engine.relayout(&document, &mut view).unwrap();
    let rows = &view.snapshot().unwrap().rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].text_range, document.projection().blocks()[0].range);
    assert_eq!(rows[1].text_range, document.projection().blocks()[1].range);
    assert!((rows[1].y - rows[0].y - rows[0].height() - 18.).abs() < 0.001);
    view.set_paragraph_flow(false);
    engine.relayout(&document, &mut view).unwrap();
    let restored = view.snapshot().unwrap();
    assert_eq!(
        restored
            .rows
            .iter()
            .map(|row| (row.y, row.paragraph_id, row.paragraph_content_x))
            .collect::<Vec<_>>(),
        physical
            .rows
            .iter()
            .map(|row| (row.y, row.paragraph_id, row.paragraph_content_x))
            .collect::<Vec<_>>()
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn source_explicit_breaks_keep_one_paragraph_geometry_even_when_flow_is_on() {
    let document = spaced_document("First hard  \nsecond hard\\\nlast line.\n\nNext paragraph.");
    assert_eq!(document.projection().blocks().len(), 2);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(800., 300.);
    for flow in [false, true] {
        view.set_paragraph_flow(flow);
        engine.relayout(&document, &mut view).unwrap();
        let rows = &view.snapshot().unwrap().rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].paragraph_id, rows[2].paragraph_id);
        assert!(
            (rows[1].y - rows[0].y - rows[0].height()).abs() < 0.001,
            "flow={flow}"
        );
        assert!(
            (rows[2].y - rows[1].y - rows[1].height()).abs() < 0.001,
            "flow={flow}"
        );
        assert!(
            (rows[3].y - rows[2].y - rows[2].height() - 18.).abs() < 0.001,
            "flow={flow}"
        );
        assert!(
            (rows[0].paragraph_content_x - rows[1].paragraph_content_x - 10.).abs() < 0.001,
            "flow={flow}"
        );
    }
}

#[test]
fn regional_source_group_bands_match_complete_layout_at_internal_and_final_lines() {
    let document = spaced_document("one  \ntwo\n\nthree\nfour\n\nfive");
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600., 100.);
    for flow in [false, true] {
        view.set_paragraph_flow(flow);
        engine.relayout(&document, &mut view).unwrap();
        let full = view.snapshot().unwrap().clone();
        for line in 0..document.projection().presentation_line_count(flow) {
            let request = prepare_layout_job(
                &document,
                &mut view,
                inspect_layout_provider(&engine),
                LayoutJobId(line as u64 + 1),
                LayoutJobPriority::Background,
                LayoutJobRegion::HardLines(HardLineLayoutRegion::new(line..line + 1).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
            let result =
                compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            let regional = &result.regional_snapshot().lines()[0];
            let row = &regional.rows()[0];
            assert_eq!(row.paragraph_id, full.rows[line].paragraph_id);
            assert_eq!(row.paragraph_content_x, full.rows[line].paragraph_content_x);
            let band_start = if line == 0 { 0. } else { full.rows[line].y };
            let band_end = full
                .rows
                .get(line + 1)
                .map_or(full.total_height, |next| next.y);
            assert!(
                (regional.height() - f64::from(band_end - band_start)).abs() < 0.001,
                "flow={flow} line={line}"
            );
        }
    }
}

#[test]
fn fenced_source_blank_lines_keep_code_paragraph_geometry_when_flowed() {
    let source = "```text\ncode\n\nlast\n```\n\nAfter";
    let mut document = spaced_document(source);
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Code Block" {
            style["block"]["spacing_before"] = 9.into();
            style["block"]["spacing_after"] = 13.into();
            style["block"]["first_line_indent"] = 6.into();
        }
    }
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600., 300.);
    for flow in [false, true] {
        view.set_paragraph_flow(flow);
        engine.relayout(&document, &mut view).unwrap();
        let rows = &view.snapshot().unwrap().rows;
        assert_eq!(rows.len(), 6);
        assert!(rows[2].text_range.is_empty());
        for i in 1..5 {
            assert_eq!(rows[i].paragraph_id, rows[0].paragraph_id);
            assert!(
                (rows[i].y - rows[i - 1].y - rows[i - 1].height()).abs() < 0.001,
                "flow={flow} line={i}"
            );
        }
        assert!((rows[0].paragraph_content_x - rows[1].paragraph_content_x - 6.).abs() < 0.001);
        assert!((rows[5].y - rows[4].y - rows[4].height() - 20.).abs() < 0.001);
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn ten_thousand_source_groups_keep_capture_edits_and_flow_invalidation_bounded() {
    let source = format!(
        "{}Last paragraph",
        "First *styled* physical line.\nSecond physical line.\n\n".repeat(10_000)
    );
    let mut core = Core::new(source_document(&source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 420., 100.);
    assert_eq!(core.document().projection().blocks().len(), 10_001);
    for flow in [false, true, false] {
        core.handle(view, CoreEvent::SetParagraphFlow(flow))
            .unwrap();
        let line = if flow { 5_000 } else { 10_000 };
        let before = core
            .prepare_view_layout_job(
                view,
                LayoutJobPriority::Background,
                LayoutJobRegion::HardLines(HardLineLayoutRegion::new(line..line + 1).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
        assert!(before.captured_text_len() < 128);
        assert!(before.capture_statistics().document_paragraph_styles() <= 2);
        assert!(before.capture_statistics().document_shaping_style_runs() < 8);
        let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
        let stale =
            compute_layout_job(&mut worker, &before, LayoutExecutionContext::WorkerPool).unwrap();
        let at = core.document().projection().blocks()[5_000].range.start + 1;
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
        for key in [Key::Char('i'), Key::Char('X'), Key::Escape] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert!(core.install_view_layout_job(view, stale).is_err());
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.rows.len() < 150);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn one_long_source_group_uses_bounded_flow_capture_and_single_paragraph_metadata() {
    let source = "Ordinary source words for a long paragraph.\n".repeat(10_000) + "Final words.";
    let document = source_document(&source);
    assert_eq!(document.projection().blocks().len(), 1);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(240., 100.);
    view.set_paragraph_flow(true);
    let request = prepare_layout_job(
        &document,
        &mut view,
        inspect_layout_provider(&engine),
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0., 100.).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 256);
    assert_eq!(request.capture_statistics().document_paragraph_styles(), 1);
    let result =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    assert!(
        result
            .regional_snapshot()
            .work_statistics()
            .maximum_shaping_fragment_bytes()
            <= 4096
    );
    let id = document.projection().blocks()[0].id;
    assert!(result.regional_snapshot().lines()[0]
        .rows()
        .iter()
        .all(|row| row.paragraph_id == Some(id)));
    assert!(result.next_long_line_checkpoint().is_some());
}
