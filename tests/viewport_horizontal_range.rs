use evim_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
use evim_core::{Core, CoreEvent, Document};

#[test]
fn horizontal_range_tracks_intersecting_rows_and_vertical_scroll_clamps_left() {
    let document = Document::new(format!("short\n{}\nshort\nshort", "W".repeat(40)));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(100., 8.);
    view.set_wrap(false);
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    let revision = snapshot.revision;
    let wide_top = snapshot.rows[1].y;
    let wide_bottom = wide_top + snapshot.rows[1].height();
    let requests = engine.provider().request_calls();
    assert!(snapshot.content_width > 100.);
    assert_eq!(view.maximum_viewport_left(), Some(0.));
    view.set_viewport_top(wide_top - 4.).unwrap();
    let maximum = view.maximum_viewport_left().unwrap();
    assert!(maximum > 100.);
    view.set_viewport_left(maximum).unwrap();
    view.set_viewport_top(wide_bottom).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(0.));
    assert_eq!(view.viewport_left(), 0.);
    assert_eq!(view.snapshot().unwrap().revision, revision);
    assert_eq!(engine.provider().request_calls(), requests);
    view.set_viewport_top(wide_top).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(maximum));
    assert_eq!(view.viewport_left(), 0.);
}

#[test]
fn changing_visible_width_invalidates_range_and_preserves_unaffected_shaping() {
    let original = "W".repeat(40) + "\nshort\nshort";
    let mut document = Document::new(&original);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(100., 8.);
    view.set_wrap(false);
    engine.relayout(&document, &mut view).unwrap();
    let requests = engine.provider().request_calls();
    view.set_viewport_left(100.).unwrap();
    document.replace(0..40, "fits").unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(0.));
    assert_eq!(view.viewport_left(), 0.);
    assert!(engine.provider().request_calls() - requests <= 1);
    assert!(document.undo());
    engine.relayout(&document, &mut view).unwrap();
    assert!(view.maximum_viewport_left().unwrap() > 100.);
    assert!(engine.provider().request_calls() - requests <= 1);
    view.resize(1_000., 8.);
    assert_eq!(view.maximum_viewport_left(), None);
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(0.));
    assert!(engine.provider().request_calls() - requests <= 1);
}

#[test]
fn oversized_indivisible_wrapped_rows_still_have_a_horizontal_range() {
    let document = Document::new("W");
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(4., 40.);
    assert!(view.wrap());
    engine.relayout(&document, &mut view).unwrap();
    let maximum = view.maximum_viewport_left().unwrap();
    assert!(maximum > 0.);
    view.set_viewport_left(maximum).unwrap();
    assert_eq!(view.viewport_left(), maximum);
    view.resize(100., 40.);
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(0.));
    assert_eq!(view.viewport_left(), 0.);
}

#[test]
fn large_partial_viewport_has_exact_visible_width_without_global_height_or_layout() {
    let mut core = Core::new(Document::new(
        "W".repeat(40) + "\n" + &"short\n".repeat(10_000),
    ));
    let view = core.add_view(MockTextMeasurementProvider::new(), 100., 32.);
    core.handle(view, CoreEvent::SetWrap(false)).unwrap();
    let initial = core.viewport_state(view).unwrap();
    assert!(initial.maximum_left().unwrap() > 100.);
    assert!(
        !core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .total_height_is_exact
    );
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 100.,
            top: None,
        },
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 100.,
            top: Some(2_000.),
        },
    )
    .unwrap();
    let state = core.viewport_state(view).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(state.maximum_left(), Some(0.));
    assert_eq!(state.left(), 0.);
    assert_eq!(state.estimated_maximum_left(), 0.);
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    assert!(!snapshot.total_height_is_exact);
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(0.),
        },
    )
    .unwrap();
    assert_eq!(
        core.viewport_state(view).unwrap().maximum_left(),
        initial.maximum_left()
    );
}
