use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
    LayoutJobId, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider,
    RegionalLayoutCacheLimits, ViewLayout, ViewportLayoutRegion,
};
use viem_core::Document;

fn candidate(
    document: &Document, view: &mut ViewLayout,
    engine: &mut LayoutEngine<MockTextMeasurementProvider>, job: u64,
    lines: std::ops::Range<usize>,
) -> (viem_core::layout::LayoutJobCaptureStatistics, viem_core::layout::LayoutJobCandidate) {
    let requirements = inspect_layout_provider(engine);
    let height = view.height();
    let request = prepare_layout_job(document, view, requirements, LayoutJobId(job),
        LayoutJobPriority::NewlyExposedRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(lines, 0.0, height).unwrap()),
        LayoutCancellationToken::new()).unwrap();
    let statistics = request.capture_statistics();
    let result = compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    (statistics, result)
}

#[test]
fn large_markdown_pages_reuse_only_requested_wrapped_lines() {
    let source = (0..2_000).map(|line| format!(
        "Paragraph {line}: **office** and *words* in a long paragraph that wraps over several visual rows.\n\n"
    )).collect::<String>();
    let document = Document::from_bytes(source.into_bytes(), viem_core::Encoding::Utf8,
        viem_core::Format::Markdown).unwrap();
    let mut view = ViewLayout::new(180.0, 64.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    install_viewport(&document, &mut view, &mut engine, 1, 0..8);
    let calls = engine.provider().request_calls();
    // Prove that reuse is of positioned rows, independent of the shaping cache.
    engine.clear_caches();
    let (capture, reused) = candidate(&document, &mut view, &mut engine, 2, 2..4);
    assert_eq!(capture.retained_regional_cache_lines(), 2);
    assert!(capture.retained_positioned_rows() > 2);
    assert_eq!(capture.retained_height_index_nodes(), 0);
    assert_eq!(reused.regional_snapshot().work_statistics().wrapped_cluster_count(), 0);
    assert_eq!(reused.regional_snapshot().work_statistics().positioned_cluster_count(), 0);
    assert_eq!(engine.provider().request_calls(), calls);

    let mut fresh_view = ViewLayout::new(180.0, 64.0);
    let mut fresh_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let (_, fresh) = candidate(&document, &mut fresh_view, &mut fresh_engine, 1, 2..4);
    for (cached, recomputed) in reused.regional_snapshot().lines().iter().zip(fresh.regional_snapshot().lines()) {
        assert_eq!(cached.height(), recomputed.height());
        let mut expected = recomputed.rows().to_vec();
        for row in &mut expected {
            for caret in &mut row.carets { caret.point.layout_revision = reused.regional_snapshot().revision(); }
        }
        assert_eq!(cached.rows(), expected);
    }
    let (overlap, mixed) = candidate(&document, &mut view, &mut engine, 3, 4..10);
    assert_eq!(overlap.retained_regional_cache_lines(), 4);
    assert!(mixed.regional_snapshot().work_statistics().positioned_cluster_count() > 0);
    let (_, fully_recomputed) = candidate(&document, &mut fresh_view, &mut fresh_engine, 2, 4..10);
    assert!(mixed.regional_snapshot().work_statistics().positioned_cluster_count()
        < fully_recomputed.regional_snapshot().work_statistics().positioned_cluster_count());
}

#[test]
fn paragraph_geometry_cache_rejects_changed_dependencies() {
    use viem_core::layout::{MetricsGeneration, MeasurementEnvironmentId, RenderRunPolicy, RenderRunOwner, RenderRunThreading};
    let mut document = Document::new("office and wrapped words\nsecond paragraph with words\nthird");
    let mut view = ViewLayout::new(150.0, 64.0);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    for change in 0..6 {
        let job = change * 3 + 1;
        install_viewport(&document, &mut view, &mut engine, job, 0..2);
        let (capture, _) = candidate(&document, &mut view, &mut engine, job + 1, 0..2);
        assert_eq!(capture.retained_regional_cache_lines(), 2);
        match change {
            0 => { view.resize(80.0, 64.0); }
            1 => { view.set_scale(1.5).unwrap(); }
            2 => engine.provider_mut().set_metrics_generation(MetricsGeneration(2)),
            3 => engine.provider_mut().set_measurement_environment_id(MeasurementEnvironmentId(2)),
            4 => { document.insert(0, "new ").unwrap(); }
            5 => engine.provider_mut().set_render_run_policy(RenderRunPolicy {
                owner: RenderRunOwner(9), threading: RenderRunThreading::AnyThread,
            }),
            _ => unreachable!(),
        }
        let (capture, recomputed) = candidate(&document, &mut view, &mut engine, job + 2, 0..2);
        if change != 5 { assert_eq!(capture.retained_regional_cache_lines(), 0); }
        assert!(recomputed.regional_snapshot().work_statistics().positioned_cluster_count() > 0);
    }
}

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
