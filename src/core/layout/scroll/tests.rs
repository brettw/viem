use super::*;
use crate::document::Document;
use crate::layout::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
    LayoutJobId, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider, ViewLayout,
    ViewportLayoutRegion,
};

#[test]
fn a_slice_of_the_final_hard_line_is_not_the_document_end() {
    let document = Document::new("word ".repeat(100_000));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(200., 80.);
    view.set_insets(EdgeInsets { top: 17., bottom: 29., ..Default::default() });
    let requirements = inspect_layout_provider(&engine);
    let request = prepare_layout_job(&document, &mut view, requirements, LayoutJobId(1),
        LayoutJobPriority::NewlyExposedRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0., 80.).unwrap()),
        LayoutCancellationToken::new()).unwrap();
    let candidate = compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    assert!(candidate.next_long_line_checkpoint().is_some());
    install_layout_job(&mut view, LayoutInstallTarget {
        document_id: document.id(), document_revision: document.revision(),
        measurement_environment_id: requirements.measurement_environment_id,
        metrics_generation: requirements.metrics_generation,
    }, candidate).unwrap();
    let snapshot = view.snapshot().unwrap();
    assert!(snapshot.contains_document_start());
    assert!(!snapshot.contains_document_end());
    assert_eq!(view.maximum_viewport_top(), None);
    let end = snapshot.coverage.vertical_range().unwrap().end;
    assert_eq!(snapshot.missing_viewport_edges(end, view.height()), (false, true));
    assert_eq!(snapshot.clamp_viewport_top(end, view.height()), end,
        "a cache edge must not truncate the scroll range");
    assert!(engine.provider().request_calls() < 100);
}

#[test]
fn changed_margins_retire_exported_scroll_bounds_until_layout_is_current() {
    let document = Document::new("line\n".repeat(30));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(200., 80.);
    engine.relayout(&document, &mut view).unwrap();
    let old = view.maximum_viewport_top().unwrap();
    view.set_insets(EdgeInsets { top: 17., bottom: 29., ..Default::default() });
    assert_eq!(view.maximum_viewport_top(), None);
    engine.relayout(&document, &mut view).unwrap();
    assert!((view.maximum_viewport_top().unwrap() - old - 46.).abs() < 0.01);
}
