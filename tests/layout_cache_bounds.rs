use evim_core::layout::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
    LayoutJobId, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider,
    RegionalLayoutCacheLimits, ViewLayout, ViewportLayoutRegion,
};
use evim_core::Document;

fn install_viewport(
    document: &Document,
    view: &mut ViewLayout,
    engine: &mut LayoutEngine<MockTextMeasurementProvider>,
    job: u64,
    hard_lines: std::ops::Range<usize>,
) {
    let requirements = inspect_layout_provider(engine);
    let top = hard_lines.start as f32 * 16.0;
    let viewport_height = view.height();
    let request = prepare_layout_job(
        document,
        view,
        requirements,
        LayoutJobId(job),
        LayoutJobPriority::NewlyExposedRows,
        LayoutJobRegion::Viewport(
            ViewportLayoutRegion::new(hard_lines, top, viewport_height).unwrap(),
        ),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let candidate =
        compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    install_layout_job(
        view,
        LayoutInstallTarget {
            document_id: document.id(),
            document_revision: document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        },
        candidate,
    )
    .unwrap();
}

#[test]
fn repeated_scrolling_keeps_regional_layout_cache_within_all_budgets() {
    let text = (0..128)
        .map(|line| format!("line {line:03}"))
        .collect::<Vec<_>>()
        .join("\n");
    let document = Document::new(text);
    let mut view = ViewLayout::new(1_000.0, 32.0);
    let limits = RegionalLayoutCacheLimits {
        max_hard_lines: 5,
        max_visual_rows: 5,
        max_estimated_bytes: usize::MAX,
    };
    view.set_regional_cache_limits(limits);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());

    for first in 0..100 {
        install_viewport(
            &document,
            &mut view,
            &mut engine,
            first as u64 + 1,
            first..first + 2,
        );
        let statistics = view.regional_cache_statistics();
        assert!(statistics.hard_line_count() <= limits.max_hard_lines);
        assert!(statistics.visual_row_count() <= limits.max_visual_rows);
        assert!(statistics.estimated_bytes() <= limits.max_estimated_bytes);

        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.coverage.hard_lines(), first..first + 2);
        assert_eq!(snapshot.rows.first().unwrap().hard_line_index, first);
        assert_eq!(snapshot.rows.last().unwrap().hard_line_index, first + 1);
    }

    assert!(view.regional_cache_statistics().eviction_count() > 0);
    assert_eq!(view.regional_cached_ranges(), vec![96..101]);
}

#[test]
fn cache_eviction_never_removes_the_installed_exact_viewport_snapshot() {
    let document = Document::new("zero\none\ntwo\nthree");
    let mut view = ViewLayout::new(300.0, 32.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    install_viewport(&document, &mut view, &mut engine, 1, 1..3);
    let exact_viewport = view.snapshot().unwrap().clone();
    assert_eq!(view.regional_cache_statistics().hard_line_count(), 2);

    view.set_regional_cache_limits(RegionalLayoutCacheLimits {
        max_hard_lines: usize::MAX,
        max_visual_rows: usize::MAX,
        max_estimated_bytes: 0,
    });

    assert_eq!(view.regional_cache_statistics().hard_line_count(), 0);
    assert_eq!(view.regional_cache_statistics().visual_row_count(), 0);
    assert_eq!(view.regional_cache_statistics().estimated_bytes(), 0);
    assert_eq!(view.snapshot(), Some(&exact_viewport));
    assert!(view.hard_line_range_height(1..3).unwrap().is_exact());
}

#[test]
fn visual_row_budget_evicts_older_wrapped_lines_independently() {
    let long_line = "wrapped words ".repeat(24);
    let document = Document::new(format!("{long_line}\n{long_line}"));
    let mut view = ViewLayout::new(90.0, 64.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    install_viewport(&document, &mut view, &mut engine, 1, 0..1);
    let one_line_rows = view.regional_cache_statistics().visual_row_count();
    assert!(one_line_rows > 1);
    view.set_regional_cache_limits(RegionalLayoutCacheLimits {
        max_hard_lines: usize::MAX,
        max_visual_rows: one_line_rows,
        max_estimated_bytes: usize::MAX,
    });

    install_viewport(&document, &mut view, &mut engine, 2, 1..2);

    let statistics = view.regional_cache_statistics();
    assert_eq!(statistics.hard_line_count(), 1);
    assert!(statistics.visual_row_count() <= one_line_rows);
    assert_eq!(view.regional_cached_ranges(), vec![1..2]);
    assert_eq!(view.snapshot().unwrap().coverage.hard_lines(), 1..2);
}

#[test]
fn background_request_capture_excludes_viewport_cache_and_height_state() {
    let text = (0..2_000)
        .map(|line| format!("line {line:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    let document = Document::new(text);
    let mut view = ViewLayout::new(500.0, 64.0);
    let mut full_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    full_engine.relayout(&document, &mut view).unwrap();
    assert_eq!(view.snapshot().unwrap().rows.len(), 2_000);
    assert_eq!(view.height_index_statistics().hard_line_count(), 2_000);

    let worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&worker);
    let line = 1_337;
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::Background,
        LayoutJobRegion::Viewport(
            ViewportLayoutRegion::new(line..line + 1, line as f32 * 16.0, 64.0).unwrap(),
        ),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let statistics = request.capture_statistics();
    let expected_text_bytes = document.line_end(line).unwrap() - document.line_start(line).unwrap();

    assert_eq!(statistics.regional_text_bytes(), expected_text_bytes);
    assert_eq!(statistics.retained_positioned_rows(), 0);
    assert_eq!(statistics.retained_height_index_nodes(), 0);
    assert_eq!(statistics.retained_regional_cache_lines(), 0);
    assert!(statistics.document_paragraph_styles() <= 2);
    assert!(statistics.document_shaping_style_runs() <= 1);
    assert!(statistics.document_paint_style_runs() <= 1);
}

#[test]
fn oversized_words_in_large_documents_preserve_regional_work_and_resize_invalidation() {
    let line = "prefix extraordinarilylongunbreakableword suffix";
    let document = Document::new(std::iter::repeat(line).take(20_000).collect::<Vec<_>>().join("\n"));
    let source = document.source_bytes();
    let revision = document.revision();
    let mut view = ViewLayout::new(90.0, 64.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let limits = RegionalLayoutCacheLimits {
        max_hard_lines: 12,
        max_visual_rows: 36,
        max_estimated_bytes: usize::MAX,
    };
    view.set_regional_cache_limits(limits);
    for (job, first) in (0..20_000).step_by(211).enumerate() {
        install_viewport(&document, &mut view, &mut engine, job as u64 + 1, first..first + 2);
        let snapshot = view.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 6);
        for rows in snapshot.rows.chunks_exact(3) {
            assert_eq!(rows[1].text_range.len(), "extraordinarilylongunbreakableword ".len());
            assert!(rows[1].width > snapshot.usable_width);
        }
        assert!(view.regional_cache_statistics().hard_line_count() <= limits.max_hard_lines);
        assert!(view.regional_cache_statistics().visual_row_count() <= limits.max_visual_rows);
    }
    let coverage = view.snapshot().unwrap().coverage.hard_lines();
    let calls = engine.provider().request_calls();
    let generation = view.configuration_generation();
    view.resize(1_000.0, 64.0);
    assert_ne!(view.configuration_generation(), generation);
    assert_eq!(view.regional_cache_statistics().hard_line_count(), 0);
    install_viewport(&document, &mut view, &mut engine, 500, coverage.clone());
    assert_eq!(view.snapshot().unwrap().rows.len(), 2);
    assert_eq!(engine.provider().request_calls(), calls, "resize reuses shaping");
    view.resize(90.0, 64.0);
    install_viewport(&document, &mut view, &mut engine, 501, coverage);
    assert_eq!(view.snapshot().unwrap().rows.len(), 6);
    assert_eq!(engine.provider().request_calls(), calls);
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.revision(), revision);
}
