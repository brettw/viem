use serde_json::json;
use std::ops::Range;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use viem_core::document::{
    Color, Encoding, Format, StyleDefinitionFieldEdit, StyleNamespace, StyleProperty,
    StylePropertyValue,
};
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, DocumentLayoutStyles,
    HardLineLayoutRegion, LayoutCancellationToken, LayoutEngine, LayoutExecutionContext,
    LayoutJobId, LayoutJobPriority, LayoutJobRegion, LayoutSnapshot, MeasurementEnvironmentId,
    MeasurementError, MetricsGeneration, MockTextMeasurementProvider, PaintStyleRun,
    RenderRunPolicy, ResolvedTextPaint, ShapeRequest, ShapedFragment, TextMeasurementProvider,
    ViewLayout, ViewportLayoutRegion,
};
use viem_core::{Core, CoreEvent, Document, ViewId};

const SOURCE: &str = "| Header | Other |\n| :--- | ---: |\n| body | value |";
const RED: Color = Color {
    red: 0.8,
    green: 0.1,
    blue: 0.2,
    alpha: 1.,
};
const BLUE: Color = Color {
    red: 0.1,
    green: 0.2,
    blue: 0.9,
    alpha: 1.,
};
const EDGES: [StyleProperty; 4] = [
    StyleProperty::BlockBorderTopColor,
    StyleProperty::BlockBorderRightColor,
    StyleProperty::BlockBorderBottomColor,
    StyleProperty::BlockBorderLeftColor,
];

struct CountingProvider {
    inner: MockTextMeasurementProvider,
    calls: Arc<AtomicUsize>,
}

impl TextMeasurementProvider for CountingProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.inner.measurement_environment_id()
    }
    fn metrics_generation(&self) -> MetricsGeneration {
        self.inner.metrics_generation()
    }
    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.inner.render_run_policy()
    }
    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.calls.fetch_add(requests.len(), Ordering::Relaxed);
        self.inner.shape_batch(requests)
    }
}

fn document(source: &str, color: Option<Color>) -> Document {
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let diagnostics = document.replace_style_defaults(&serde_json::to_vec(&json!({"version":1,"block_styles":[
        {"id":"Table cell","name":"Table cell","role":"Paragraph","based_on":"Paragraph","block":{
            "border_top_width":1,"border_right_width":1,"border_bottom_width":1,"border_left_width":1,
            "border_top_color":color,"border_right_color":color,"border_bottom_color":color,"border_left_color":color
        }},
        {"id":"Table header","name":"Table header","role":"Paragraph","based_on":"Table cell","block":{}}
    ]})).unwrap()).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    document
}

fn edit<P: TextMeasurementProvider>(
    core: &mut Core<P>,
    view: ViewId,
    style: &str,
    property: StyleProperty,
    color: Option<Color>,
) {
    let document = core.document();
    core.handle(
        view,
        CoreEvent::EditGeneratedStyle {
            document: document.id(),
            revision: document.revision(),
            style_sheet_revision: document.projection().style_sheet().revision,
            namespace: StyleNamespace::Block,
            style: style.into(),
            edit: color.map_or(
                StyleDefinitionFieldEdit::ClearDeclaration(property),
                |color| StyleDefinitionFieldEdit::SetDeclaration {
                    property,
                    value: StylePropertyValue::Color(color),
                },
            ),
        },
    )
    .unwrap();
}

fn paint<'a>(
    runs: &'a [PaintStyleRun],
    default: &'a ResolvedTextPaint,
    at: usize,
) -> &'a ResolvedTextPaint {
    runs.iter()
        .find(|run| run.text_range.contains(&at))
        .map_or(default, |run| &run.paint)
}

fn same_geometry(before: &LayoutSnapshot, after: &LayoutSnapshot) {
    assert_eq!(before.rows.len(), after.rows.len());
    assert_eq!(before.content_width, after.content_width);
    for (a, b) in before.rows.iter().zip(after.rows.iter()) {
        assert_eq!(
            (
                &a.text_range,
                a.y,
                a.baseline,
                a.width,
                a.height(),
                a.paragraph_content_x
            ),
            (
                &b.text_range,
                b.y,
                b.baseline,
                b.width,
                b.height(),
                b.paragraph_content_x
            )
        );
        assert_eq!(
            a.clusters
                .iter()
                .map(|cluster| (
                    &cluster.text_range,
                    cluster.x,
                    cluster.advance,
                    cluster.ink_bounds
                ))
                .collect::<Vec<_>>(),
            b.clusters
                .iter()
                .map(|cluster| (
                    &cluster.text_range,
                    cluster.x,
                    cluster.advance,
                    cluster.ink_bounds
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            a.carets
                .iter()
                .map(|caret| (caret.point.text_offset, caret.point.affinity, caret.x))
                .collect::<Vec<_>>(),
            b.carets
                .iter()
                .map(|caret| (caret.point.text_offset, caret.point.affinity, caret.x))
                .collect::<Vec<_>>()
        );
    }
}

fn uncolored_work(
    source: &str,
    width: f32,
    height: f32,
    region: LayoutJobRegion,
) -> (usize, usize, usize) {
    let document = document(source, None);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(width, height);
    let request = prepare_layout_job(
        &document,
        &mut view,
        inspect_layout_provider(&engine),
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        region,
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let work = candidate.regional_snapshot().work_statistics();
    (
        work.table_measured_cells(),
        work.table_measured_text_bytes(),
        engine.provider().request_calls(),
    )
}

#[test]
fn source_border_colors_refresh_warm_views_inherit_and_clear_without_geometry_changes() {
    let mut core = Core::new(document(SOURCE, None));
    let calls = Arc::new(AtomicUsize::new(0));
    let view = core.add_view(
        CountingProvider {
            inner: MockTextMeasurementProvider::new(),
            calls: calls.clone(),
        },
        800.,
        300.,
    );
    let baseline_calls = calls.load(Ordering::Relaxed);
    assert!(baseline_calls > 0);
    let baseline = core.layout(view).unwrap().snapshot().unwrap().clone();
    let table = &core.document().projection().tables()[0];
    let header_pipe = table.source_rows[0].pipes[1];
    let body_pipe = table.source_rows[2].pipes[1];
    let delimiter = table.source_rows[1].cells[0].start + 1;
    let body_text = table.source_rows[2].cells[0].start + 1;
    let baseline_text = paint(&baseline.paint_runs, &baseline.default_paint, body_text).clone();
    for color in [
        RED,
        Color { alpha: 0.4, ..BLUE },
        Color { alpha: 0., ..RED },
    ] {
        for property in EDGES {
            edit(&mut core, view, "Table cell", property, Some(color));
        }
        let current = core.layout(view).unwrap().snapshot().unwrap();
        same_geometry(&baseline, current);
        for at in [header_pipe, body_pipe, delimiter] {
            let actual = paint(&current.paint_runs, &current.default_paint, at);
            assert_eq!(actual.foreground, color, "offset {at}");
            assert!(
                !actual.foreground_is_default,
                "transparent is an explicit color"
            );
        }
        assert_eq!(
            paint(&current.paint_runs, &current.default_paint, body_text),
            &baseline_text
        );
        assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
        assert_eq!(
            calls.load(Ordering::Relaxed),
            baseline_calls,
            "paint-only changes must reuse shaping"
        );
    }
    edit(
        &mut core,
        view,
        "Table header",
        StyleProperty::BlockBorderRightColor,
        Some(BLUE),
    );
    let current = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(
        paint(&current.paint_runs, &current.default_paint, header_pipe).foreground,
        BLUE
    );
    assert_eq!(
        paint(&current.paint_runs, &current.default_paint, body_pipe)
            .foreground
            .alpha,
        0.
    );
    edit(
        &mut core,
        view,
        "Table header",
        StyleProperty::BlockBorderRightColor,
        None,
    );
    let current = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(
        paint(&current.paint_runs, &current.default_paint, header_pipe)
            .foreground
            .alpha,
        0.
    );
    for property in EDGES {
        edit(&mut core, view, "Table cell", property, None);
    }
    let current = core.layout(view).unwrap().snapshot().unwrap();
    same_geometry(&baseline, current);
    for at in [header_pipe, body_pipe, delimiter, body_text] {
        assert_eq!(
            paint(&current.paint_runs, &current.default_paint, at),
            paint(&baseline.paint_runs, &baseline.default_paint, at)
        );
    }
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
    assert_eq!(
        calls.load(Ordering::Relaxed),
        baseline_calls,
        "inheritance/clear must reuse shaping"
    );
}

#[test]
fn late_source_table_paint_capture_stays_regional_in_a_large_document() {
    let source = "| H | V |\n| --- | --- |\n".to_owned()
        + &(0..50_000)
            .map(|row| format!("| row{row} | value |\n"))
            .collect::<String>();
    let document = document(&source, Some(RED));
    let table = &document.projection().tables()[0];
    let selected = 49_990..49_993;
    let range = table.source_rows[selected.start].range.start
        ..table.source_rows[selected.end - 1].range.end;
    let styles =
        DocumentLayoutStyles::resolve_region(document.projection(), range.clone()).unwrap();
    assert!(
        styles.paint_runs.len() <= 16,
        "regional paint expanded across the table"
    );
    assert!(styles
        .paint_runs
        .iter()
        .all(|run| range.start <= run.text_range.start && run.text_range.end <= range.end));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(500., 150.);
    let request = prepare_layout_job(
        &document,
        &mut view,
        inspect_layout_provider(&engine),
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(selected.clone()).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(request.captured_text_len() < 256);
    assert!(request.capture_statistics().document_paint_style_runs() <= 16);
    assert!(request.capture_statistics().document_paragraph_styles() < 8);
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let snapshot = candidate.regional_snapshot();
    let baseline = uncolored_work(
        &source,
        500.,
        150.,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(selected.clone()).unwrap()),
    );
    assert_eq!(
        (
            snapshot.work_statistics().table_measured_cells(),
            snapshot.work_statistics().table_measured_text_bytes(),
            engine.provider().request_calls()
        ),
        baseline,
        "border paint must not add table discovery or shaping"
    );
    // One 128-cell discovery slice plus two passes over three physical rows.
    // Each Source row shapes two cells, two gutter samples, and its literal row.
    assert!(
        snapshot.work_statistics().table_measured_cells() <= 128 + selected.len() * 2 * 5,
        "cells: {}",
        snapshot.work_statistics().table_measured_cells()
    );
    assert!(
        snapshot.work_statistics().table_measured_text_bytes() < 16_384,
        "bytes: {}",
        snapshot.work_statistics().table_measured_text_bytes()
    );
    assert!(snapshot.paint_runs().len() <= 16);
    for row in table
        .source_rows
        .iter()
        .skip(selected.start)
        .take(selected.len())
    {
        for at in &row.pipes {
            assert!(snapshot
                .paint_runs()
                .iter()
                .any(|run| run.text_range.contains(at) && run.paint.foreground == RED));
        }
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn giant_source_cell_viewport_keeps_border_paint_and_capture_bounded() {
    let source = format!(
        "| H | V |\n| --- | --- |\n| {} | next |",
        "x".repeat(2 * 1024 * 1024)
    );
    let document = document(&source, Some(BLUE));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(400., 120.);
    let request = prepare_layout_job(
        &document,
        &mut view,
        inspect_layout_provider(&engine),
        LayoutJobId(2),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..3, 0., 120.).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert_eq!(request.captured_text_len(), 0);
    assert!(request.capture_statistics().document_paint_style_runs() < 24);
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let snapshot = candidate.regional_snapshot();
    let baseline = uncolored_work(
        &source,
        400.,
        120.,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..3, 0., 120.).unwrap()),
    );
    assert_eq!(
        (
            snapshot.work_statistics().table_measured_cells(),
            snapshot.work_statistics().table_measured_text_bytes(),
            engine.provider().request_calls()
        ),
        baseline,
        "border paint must not add giant-cell discovery or shaping"
    );
    assert!(snapshot.paint_runs().len() < 24);
    // Matches the existing progressive table budget: a 64KiB discovery slice
    // plus bounded visible fragments and their shaping context in both passes.
    assert!(
        snapshot.work_statistics().table_measured_text_bytes() < 140_000,
        "bytes: {}",
        snapshot.work_statistics().table_measured_text_bytes()
    );
    assert!(
        snapshot
            .lines()
            .iter()
            .flat_map(|line| line.rows())
            .map(|row| row.clusters.len())
            .sum::<usize>()
            < 10_000
    );
    let pipe = document.projection().tables()[0].source_rows[2].pipes[0];
    assert!(snapshot
        .paint_runs()
        .iter()
        .any(|run| run.text_range.contains(&pipe) && run.paint.foreground == BLUE));
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn giant_delimiter_paint_uses_compact_markers_and_preserves_them_after_rebasing() {
    let source = format!(
        "before\n\n| H | V |\n| :{}: | --- |\n| body | next |",
        "-".repeat(200_000)
    );
    let mut document = document(&source, Some(RED));
    let markers: Vec<Range<usize>> = document.projection().tables()[0].source_rows[1]
        .delimiter_markers
        .clone();
    assert_eq!(markers.len(), 2);
    assert_eq!(markers[0].len(), 200_002);
    for range in [
        markers[0].start..markers[0].start + 40,
        markers[0].end - 40..markers[0].end,
    ] {
        let styles =
            DocumentLayoutStyles::resolve_region(document.projection(), range.clone()).unwrap();
        assert!(styles.paint_runs.len() <= 2);
        assert_eq!(
            paint(&styles.paint_runs, &styles.default_paint, range.start).foreground,
            RED
        );
        assert!(styles
            .paint_runs
            .iter()
            .all(|run| range.start <= run.text_range.start && run.text_range.end <= range.end));
    }
    document.insert(0, "prefix ").unwrap();
    let shifted = &document.projection().tables()[0].source_rows[1].delimiter_markers;
    assert_eq!(
        shifted,
        &markers
            .iter()
            .map(|range| range.start + 7..range.end + 7)
            .collect::<Vec<_>>()
    );
    let reopened = Document::from_bytes(
        document.source_bytes(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_eq!(
        shifted,
        &reopened.projection().tables()[0].source_rows[1].delimiter_markers
    );
    assert_eq!(
        document.source_bytes(),
        format!("prefix {source}").as_bytes()
    );
}
