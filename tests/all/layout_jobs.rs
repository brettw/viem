use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    HardLineLayoutRegion, LayoutCancellationToken, LayoutExecutionContext, LayoutInstallTarget,
    LayoutJobId, LayoutJobPriority, LayoutJobRegion, LayoutJobRequest, LayoutProviderRequirements,
    MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use viem_core::{Core, CoreError, Document, ViewRemovalOutcome};

#[test]
fn public_background_layout_api_round_trips_a_revision_bound_request() {
    let document = Document::new("public API");
    let mut engine = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements: LayoutProviderRequirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(200.0, 100.0);
    let request: LayoutJobRequest = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..1).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();

    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    assert_eq!(
        request.measurement_environment_id(),
        requirements.measurement_environment_id
    );
    assert_eq!(
        candidate.measurement_environment_id(),
        requirements.measurement_environment_id
    );
    let installed = install_layout_job(
        &mut view,
        LayoutInstallTarget {
            document_id: document.id(),
            document_revision: document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        },
        candidate,
    )
    .unwrap();

    assert_eq!(installed.job_id, LayoutJobId(1));
    assert!(view.snapshot().is_none());
    assert_eq!(view.regional_cache_statistics().hard_line_count(), 1);
    let regional = view.regional_hard_line_layout(0).unwrap();
    assert_eq!(regional.hard_line_range(), 0..document.text().len());
    assert_eq!(regional.rows()[0].hard_line_index, 0);
    let content_height = view.content_height();
    assert!(content_height.is_exact());
    assert!(content_height.height() > 0.0);
    assert_eq!(view.hard_line_prefix_height(1).unwrap(), content_height);
    assert_eq!(view.hard_line_range_height(0..1).unwrap(), content_height);
    let hit = view.hard_line_at_y(0.0).unwrap().unwrap();
    assert_eq!(hit.hard_line(), 0);
    assert!(hit.prefix_is_exact());
    assert!(hit.line_is_exact());
}

#[test]
fn public_core_facade_captures_off_core_and_installs_atomically() {
    let mut core = Core::new(Document::new("zero\none\ntwo"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 32.0);
    let requirements = core.layout_provider_requirements(view).unwrap();
    let request = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::ViewportOverscan,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(1..3, 16.0, 32.0).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
    assert_eq!(
        request.metrics_generation(),
        requirements.metrics_generation
    );

    let mut worker = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
    let candidate =
        compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let installed = core.install_view_layout_job(view, candidate).unwrap();

    assert_eq!(installed.job_id, request.job_id());
    assert_eq!(
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .coverage
            .hard_lines(),
        1..3
    );
}

#[test]
fn public_view_lifecycle_is_fallible_and_cancels_owned_work() {
    let mut core = Core::new(Document::new("zero\none\ntwo"));
    let view = core
        .try_add_view(MockTextMeasurementProvider::new(), 200.0, 32.0)
        .unwrap();
    let cancellation = LayoutCancellationToken::new();
    core.prepare_view_layout_job(
        view,
        LayoutJobPriority::Background,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..2, 0.0, 32.0).unwrap()),
        cancellation.clone(),
    )
    .unwrap();

    assert_eq!(
        core.remove_view(view).unwrap(),
        ViewRemovalOutcome {
            edit_group_closed: false,
            composition_discarded: false,
            layout_work_cancelled: true,
        }
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(core.remove_view(view), Err(CoreError::UnknownView(view)));
}

#[test]
fn public_viewport_api_bounds_a_multi_megabyte_wrapped_hard_line() {
    const LONG_LINE_BYTES: usize = 2 * 1024 * 1024;
    let document = Document::new("word ".repeat(LONG_LINE_BYTES / 5 + 1));
    let mut engine = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(96.0, 80.0);
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
    assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 256);
    assert!(request.captured_text_len() < LONG_LINE_BYTES / 16);

    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let checkpoint = candidate.next_long_line_checkpoint().unwrap();
    assert!(checkpoint.next_text_offset() < LONG_LINE_BYTES);
    assert!(
        candidate
            .regional_snapshot()
            .work_statistics()
            .maximum_shaping_fragment_bytes()
            <= 4096
    );

    install_layout_job(
        &mut view,
        LayoutInstallTarget {
            document_id: document.id(),
            document_revision: document.revision(),
            measurement_environment_id: requirements.measurement_environment_id,
            metrics_generation: requirements.metrics_generation,
        },
        candidate,
    )
    .unwrap();
    assert!(!view.snapshot().unwrap().total_height_is_exact);
    assert!(view.regional_cached_ranges().is_empty());
}

#[test]
fn oversized_indivisible_words_extend_capture_without_creating_an_emergency_wrap() {
    let token = "x".repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 100);
    for tail in [String::new(), format!(" {}", "tail ".repeat(20_000))] {
        let document = Document::new(format!("{token}{tail}"));
        let mut engine = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
        let requirements = inspect_layout_provider(&engine);
        let mut view = ViewLayout::new(96.0, 80.0);
        let request = prepare_layout_job(
            &document, &mut view, requirements, LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 80.0).unwrap()),
            LayoutCancellationToken::new(),
        ).unwrap();
        assert!(request.captured_text_len() < token.len() + 300);
        let candidate = compute_layout_job(
            &mut engine, &request, LayoutExecutionContext::WorkerPool,
        ).unwrap();
        let line = &candidate.regional_snapshot().lines()[0];
        let rows = line.rows();
        let row_end = token.len() + usize::from(!tail.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text_range, 0..row_end);
        assert!(rows[0].width > 96.0);
        if tail.is_empty() {
            assert!(candidate.next_long_line_checkpoint().is_none());
            assert!(line.height_is_exact());
        } else {
            assert_eq!(candidate.next_long_line_checkpoint().unwrap().next_text_offset(), row_end);
        }
    }
}
