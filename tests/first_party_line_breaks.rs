//! Layout-level checks for line breaking across bounded capture boundaries.
//! Expected geometry comes from complete layout of the same immutable text,
//! not from a second line-breaking implementation or a third-party crate.

use std::ops::Range;
use viem_core::document::{Encoding, Format};
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, LayoutCancellationToken,
    LayoutEngine, LayoutExecutionContext, LayoutJobId, LayoutJobPriority, LayoutJobRegion,
    MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use viem_core::Document;

fn continued_rows(
    document: &Document,
    engine: &mut LayoutEngine<MockTextMeasurementProvider>,
    view: &mut ViewLayout,
    next_job: &mut u64,
) -> Vec<Range<usize>> {
    let requirements = inspect_layout_provider(engine);
    let mut checkpoint = None;
    let mut rows = Vec::new();
    for _ in 0..20 {
        let region = checkpoint.take().map_or_else(
            || ViewportLayoutRegion::new(0..1, 0.0, 80.0).unwrap(),
            |checkpoint| ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 80.0).unwrap(),
        );
        let request = prepare_layout_job(
            document,
            view,
            requirements,
            LayoutJobId(*next_job),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        *next_job += 1;
        assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 4096);
        let candidate =
            compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let line = &candidate.regional_snapshot().lines()[0];
        rows.extend(line.rows().iter().map(|row| row.text_range.clone()));
        checkpoint = candidate.next_long_line_checkpoint().cloned();
        if checkpoint.is_none() {
            assert!(line.height_is_exact());
            return rows;
        }
        assert_eq!(
            checkpoint.as_ref().unwrap().next_text_offset(),
            rows.last().unwrap().end
        );
    }
    panic!("bounded Unicode layout did not complete in twenty captures");
}

fn complete_rows(document: &Document, width: f32, flow: bool) -> Vec<Range<usize>> {
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(width, 80.0);
    view.set_paragraph_flow(flow);
    engine.relayout(document, &mut view).unwrap();
    view.snapshot()
        .unwrap()
        .rows
        .iter()
        .map(|row| row.text_range.clone())
        .collect()
}

fn assert_same_rows(actual: &[Range<usize>], expected: &[Range<usize>]) {
    assert_eq!(actual.len(), expected.len(), "visual row count");
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(actual, expected, "visual row {index}");
    }
}

#[test]
fn unicode_continuations_and_resize_match_complete_layout() {
    let mixed = "alpha a\u{301}\u{200d}b 👩🏽‍🚀 🇺🇸🇨🇦🇯🇵🇺 中文（中文） a\u{00a0}b a\u{2060}b ab\u{200b}cd 12,345.67% ";
    let samples = [
        mixed.repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES / mixed.len() + 120),
        format!("{} tail", "🇺".repeat(17_003)),
    ];
    for text in samples {
        assert!(text.len() > MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
        let document = Document::new(text);
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(93.0, 80.0);
        let mut next_job = 1;
        for width in [93.0, 213.0] {
            view.resize(width, 80.0);
            let actual = continued_rows(&document, &mut engine, &mut view, &mut next_job);
            assert_same_rows(&actual, &complete_rows(&document, width, false));
        }
    }
}

#[test]
fn opening_punctuation_with_giant_space_context_remains_one_sparse_overflow_row() {
    let spaces = MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 1000;
    let text = format!("({}x tail", " ".repeat(spaces));
    let document = Document::new(text);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(120.0, 80.0);
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 80.0).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert_eq!(request.captured_text_len(), 0);
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let line = &candidate.regional_snapshot().lines()[0];
    assert_eq!(line.rows().len(), 1);
    assert_eq!(line.rows()[0].text_range, 0..spaces + 3);
    assert!(line.rows()[0].width > 120.0);
    assert!(line.rows()[0].clusters.len() < 5000);
    assert_eq!(
        candidate
            .next_long_line_checkpoint()
            .unwrap()
            .next_text_offset(),
        spaces + 3
    );
}

#[test]
fn flowing_source_newlines_have_space_break_semantics_across_captures() {
    let pattern = "alpha\n中文（中文） a\u{00a0}b a\u{2060}b 🇺🇸🇨🇦 tail ";
    let source = pattern.repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES / pattern.len() + 50);
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(171.0, 80.0);
    view.set_paragraph_flow(true);
    let actual = continued_rows(&document, &mut engine, &mut view, &mut 1);
    assert_same_rows(&actual, &complete_rows(&document, 171.0, true));
    let spaces = Document::from_bytes(
        source.replace('\n', " ").into_bytes(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_same_rows(&actual, &complete_rows(&spaces, 171.0, true));
    assert_eq!(document.source_bytes(), source.as_bytes());
}
