use super::*;
use crate::layout::{
    compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
    LayoutCancellationToken, LayoutExecutionContext, LayoutInstallTarget, LayoutJobCandidate,
    LayoutJobInstallRejection, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider,
    ViewportLayoutRegion,
};

fn joined_candidate(
    document: &Document,
    engine: &mut LayoutEngine<MockTextMeasurementProvider>,
    view: &mut ViewLayout,
    first_job: u64,
    prepend: bool,
) -> LayoutJobCandidate {
    let mut preceding_or_following = None;
    for (index, line) in if prepend { [0, 1] } else { [1, 0] }
        .into_iter()
        .enumerate()
    {
        let request = prepare_layout_job(
            document,
            view,
            inspect_layout_provider(engine),
            LayoutJobId(first_job + index as u64),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(
                ViewportLayoutRegion::new(line..line + 1, 0.0, view.height()).unwrap(),
            ),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let mut candidate =
            compute_layout_job(engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        if let Some(other) = preceding_or_following {
            if prepend {
                candidate.prepend_adjacent_region(&other);
            } else {
                candidate.append_adjacent_region(&other);
            }
            return candidate;
        }
        preceding_or_following = Some(candidate.regional_snapshot().clone());
    }
    unreachable!()
}

fn target(
    document: &Document,
    engine: &LayoutEngine<MockTextMeasurementProvider>,
) -> LayoutInstallTarget {
    let requirements = inspect_layout_provider(engine);
    LayoutInstallTarget {
        document_id: document.id(),
        document_revision: document.revision(),
        measurement_environment_id: requirements.measurement_environment_id,
        metrics_generation: requirements.metrics_generation,
    }
}

#[test]
fn adjacent_regions_preserve_sparse_hit_testing_and_logical_selection_boundaries() {
    let giant = "a".repeat(128 * 1024);
    for (first, second) in [
        (giant.as_str(), "tail"),
        ("head", giant.as_str()),
        (giant.as_str(), giant.as_str()),
    ] {
        let document = Document::new(format!("{first}\n{second}"));
        for wrap in [false, true] {
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            let mut reference_view = ViewLayout::new(320.0, 100.0);
            reference_view.set_wrap(wrap);
            engine.relayout(&document, &mut reference_view).unwrap();
            let reference = reference_view.snapshot().unwrap();

            for prepend in [true, false] {
                let mut view = ViewLayout::new(320.0, 100.0);
                view.set_wrap(wrap);
                let candidate = joined_candidate(&document, &mut engine, &mut view, 1, prepend);
                assert_eq!(candidate.regional_snapshot().hard_lines(), 0..2);
                install_layout_job(&mut view, target(&document, &engine), candidate).unwrap();
                let snapshot = view.snapshot().unwrap();
                assert!(snapshot.has_horizontal_materialization());
                assert_eq!(snapshot.rows.len(), 2);
                assert!(view.regional_cached_ranges().is_empty());

                for (row_index, row) in snapshot.rows.iter().enumerate() {
                    for x in [0.0, 160.0, 319.0] {
                        let point = LayoutPoint { x, y: row.y + 1.0 };
                        let actual = snapshot.hit_test(point).unwrap();
                        let expected = reference.hit_test(point).unwrap();
                        assert_eq!(actual.layout_revision, snapshot.revision);
                        assert_eq!(
                            (actual.text_offset, actual.affinity),
                            (expected.text_offset, expected.affinity)
                        );
                        assert_eq!(
                            snapshot.caret_geometry(actual).unwrap().row_index,
                            row_index
                        );
                    }
                    if row.text_range.len() < giant.len() {
                        continue;
                    }
                    assert!(row.clusters.len() < giant.len() / 4);
                    let offset = row.text_range.start + giant.len() / 2;
                    let middle = LayoutPoint {
                        x: row.width / 2.0,
                        y: row.y + 1.0,
                    };
                    assert!(!snapshot.horizontal_geometry_is_materialized(row_index, middle.x));
                    assert_eq!(
                        snapshot.hit_test(middle),
                        Err(LayoutError::OutsideMaterializedCoverage)
                    );
                    assert_eq!(
                        snapshot.caret_point(offset, BoundaryAffinity::Downstream),
                        Err(LayoutError::OutsideMaterializedCoverage)
                    );
                    // Logical selections can cross geometry gaps: the shared text
                    // tree still validates the exact grapheme endpoints.
                    let selection = TextRange::new(
                        document.text_point(offset).unwrap(),
                        document.text_point(offset + 1).unwrap(),
                    )
                    .unwrap();
                    snapshot
                        .selection_rectangles(selection, BoundaryAffinity::Downstream)
                        .unwrap();
                }
            }
        }
    }
}

#[test]
fn adjacent_sparse_regions_keep_scroll_resize_and_edit_invalidation() {
    // The sparse band is contributed by the region that used to lose its
    // horizontal metadata when joined to the ordinary following paragraph.
    let mut document = Document::new(format!("{}\ntail", "a".repeat(128 * 1024)));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(320.0, 100.0);
    view.set_wrap(false);
    let candidate = joined_candidate(&document, &mut engine, &mut view, 1, true);
    install_layout_job(&mut view, target(&document, &engine), candidate).unwrap();
    let initial_revision = view.snapshot().unwrap().revision;
    let initial_width = view.snapshot().unwrap().rows[0].width;

    let candidate = joined_candidate(&document, &mut engine, &mut view, 3, true);
    view.set_viewport_left(initial_width / 2.0).unwrap();
    assert_eq!(
        install_layout_job(&mut view, target(&document, &engine), candidate),
        Err(LayoutJobInstallRejection::OutsideHorizontalCoverage)
    );
    assert_eq!(view.snapshot().unwrap().revision, initial_revision);

    view.set_viewport_left(0.0).unwrap();
    let candidate = joined_candidate(&document, &mut engine, &mut view, 5, true);
    view.resize(640.0, 100.0);
    assert!(matches!(
        install_layout_job(&mut view, target(&document, &engine), candidate),
        Err(LayoutJobInstallRejection::StaleConfiguration { .. })
    ));
    assert_eq!(view.snapshot().unwrap().revision, initial_revision);

    document.insert(64 * 1024, "W").unwrap();
    engine
        .provider_mut()
        .set_metrics_generation(MetricsGeneration(2));
    let candidate = joined_candidate(&document, &mut engine, &mut view, 7, false);
    install_layout_job(&mut view, target(&document, &engine), candidate).unwrap();
    let snapshot = view.snapshot().unwrap();
    assert_eq!(snapshot.document_revision, document.revision());
    assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
    assert_eq!(snapshot.viewport_width, 640.0);
    assert!(snapshot.rows[0].width > initial_width);
    assert!(snapshot.has_horizontal_materialization());
    assert!(snapshot.hit_test(LayoutPoint { x: 639.0, y: 1.0 }).is_ok());
    assert!(view.regional_cached_ranges().is_empty());
}
