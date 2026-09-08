use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, DocumentLayoutStyles,
    LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutJobId, LayoutJobPriority,
    LayoutJobRegion, MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};

fn html(body: &str) -> Document {
    let source = format!(
        "<ol><li style='margin-inline-start:40pt;margin-inline-end:8pt;text-indent:0pt'>{body}</li></ol>"
    );
    Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap()
}

fn leading_x(document: &Document, rtl: bool) -> f32 {
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    styles.document_insets.left
        + if rtl {
            styles.paragraphs[0].trailing_indent
        } else {
            styles.paragraphs[0].leading_indent
        }
}

#[test]
fn numeric_and_isolated_prefixes_choose_the_correct_list_gutter() {
    for (prefix, rtl) in [
        ("2026 שלום", true),
        ("١٢٣ مرحبا", true),
        ("\u{2066}English\u{2069} 2026 שלום", true),
        ("\u{2067}שלום\u{2069} 2026 English", false),
    ] {
        let document = html(&format!("{prefix} {}", "word שלום ".repeat(12)));
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(200., 200.);
        engine.relayout(&document, &mut view).unwrap();
        let rows = &view.snapshot().unwrap().rows;
        assert!(rows.len() > 1);
        assert_eq!(
            rows[1].paragraph_content_x,
            leading_x(&document, rtl),
            "{prefix}"
        );
        let shaped = engine.provider().request_calls();
        view.resize(260., 200.);
        engine.relayout(&document, &mut view).unwrap();
        assert_eq!(engine.provider().request_calls(), shaped);
        assert_eq!(
            view.snapshot().unwrap().rows[1].paragraph_content_x,
            leading_x(&document, rtl),
            "{prefix} after resize"
        );
    }
}

#[test]
fn first_strong_edit_changes_the_gutter_and_reuses_unchanged_line_shaping() {
    let source = "<ol>".to_owned()
        + &"<li style='margin-inline-start:40pt;margin-inline-end:8pt;text-indent:0pt'>2026 שלום עולם word word word word</li>".repeat(1_000)
        + "</ol>";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(1_100);
    let mut view = ViewLayout::new(200., 200.);
    engine.relayout(&document, &mut view).unwrap();
    let original_x = view.snapshot().unwrap().rows[1].paragraph_content_x;
    assert_eq!(original_x, leading_x(&document, true));
    let shaped = engine.provider().request_calls();
    let at = document.text().find("שלום").unwrap();
    document.replace(at..at + "שלום".len(), "hello").unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(
        view.snapshot().unwrap().rows[1].paragraph_content_x,
        leading_x(&document, false)
    );
    assert!(
        engine.provider().request_calls() - shaped <= 2,
        "requests {}",
        engine.provider().request_calls() - shaped
    );
    assert!(document.undo());
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(
        view.snapshot().unwrap().rows[1].paragraph_content_x,
        original_x
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn long_line_checkpoints_keep_the_initial_rtl_direction_without_recapturing_the_prefix() {
    let document = html(&format!("2026 שלום {}", "English words ".repeat(20_000)));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(200., 100.);
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0., 100.).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 256);
    assert!(request.captured_text_len() < document.text().len() / 4);
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let line = &candidate.regional_snapshot().lines()[0];
    assert_eq!(
        line.rows()[1].paragraph_content_x,
        leading_x(&document, true)
    );
    assert!(!line.rows()[0].decorations.is_empty());
    assert!(line.rows()[1..]
        .iter()
        .all(|row| row.decorations.is_empty()));
    let checkpoint = candidate.next_long_line_checkpoint().unwrap().clone();
    assert!(checkpoint.next_text_offset() > "2026 שלום".len() + 32);
    let region = ViewportLayoutRegion::resume_long_line(checkpoint, 0., 100.).unwrap();
    let resumed = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(2),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(region),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(resumed.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 256);
    let next =
        compute_layout_job(&mut engine, &resumed, LayoutExecutionContext::WorkerPool).unwrap();
    assert_eq!(
        next.regional_snapshot().lines()[0].rows()[0].paragraph_content_x,
        leading_x(&document, true)
    );
    assert!(next.regional_snapshot().lines()[0]
        .rows()
        .iter()
        .all(|row| row.decorations.is_empty()));
    assert!(
        next.regional_snapshot()
            .work_statistics()
            .maximum_shaping_fragment_bytes()
            <= 4096
    );
}
