use evim_core::ffi::{evim_core_adjacent_zoom_scale, EvimStatus};
use evim_core::layout::{
    adjacent_zoom_scale, LayoutError, MockTextMeasurementProvider, ZOOM_STOPS,
};
use evim_core::{Core, CoreEvent, Document};

#[test]
fn exact_zoom_stops_saturate_and_accept_intermediate_checked_scales() {
    let expected = [
        25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500,
    ];
    assert_eq!(
        ZOOM_STOPS.map(|value| (value * 100.).round() as u16),
        expected
    );
    for (index, &stop) in ZOOM_STOPS.iter().enumerate() {
        assert_eq!(
            adjacent_zoom_scale(stop, true).unwrap(),
            ZOOM_STOPS[(index + 1).min(16)]
        );
        assert_eq!(
            adjacent_zoom_scale(stop, false).unwrap(),
            ZOOM_STOPS[index.saturating_sub(1)]
        );
    }
    assert_eq!(adjacent_zoom_scale(1.03, true).unwrap(), 1.10);
    assert_eq!(adjacent_zoom_scale(1.03, false).unwrap(), 1.00);
    for invalid in [0.0, -1.0, 0.249, 5.001, f32::NAN, f32::INFINITY] {
        assert_eq!(
            adjacent_zoom_scale(invalid, true),
            Err(LayoutError::InvalidScale)
        );
        let mut output = 99.;
        assert_eq!(
            unsafe { evim_core_adjacent_zoom_scale(invalid, 1, &mut output) },
            EvimStatus::InvalidArgument
        );
        assert_eq!(output, 0.);
    }
    let mut output = 99.;
    assert_eq!(
        unsafe { evim_core_adjacent_zoom_scale(1., 2, &mut output) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(output, 0.);
    assert_eq!(
        unsafe { evim_core_adjacent_zoom_scale(1., 1, std::ptr::null_mut()) },
        EvimStatus::InvalidArgument
    );
    assert_eq!(
        unsafe { evim_core_adjacent_zoom_scale(4., 1, &mut output) },
        EvimStatus::Ok
    );
    assert_eq!(output, 5.);
}

#[test]
fn zoom_reflows_only_its_view_in_a_large_document_and_never_changes_history() {
    let source = "A proportional paragraph with words that can wrap.\n".repeat(10_000);
    let mut core = Core::new(Document::new(&source));
    let first = core.add_view(MockTextMeasurementProvider::new(), 240., 120.);
    let second = core.add_view(MockTextMeasurementProvider::new(), 240., 120.);
    let history = core.document().history_status();
    let revision = core.document().revision();
    let other = core.layout(second).unwrap().configuration_generation();
    for scale in [0.25, 0.33, 1.03, 4., 5., 1.] {
        let previous = core.layout(first).unwrap().configuration_generation();
        let outcome = core.handle(first, CoreEvent::SetScale(scale)).unwrap();
        assert!(!outcome.document_changed);
        assert!(outcome.layout_changed);
        assert!(core.layout(first).unwrap().configuration_generation() > previous);
        assert_eq!(
            core.layout(second).unwrap().configuration_generation(),
            other
        );
        let snapshot = core.layout(first).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.coverage.hard_lines().len() < 100);
    }
    let configuration = core.layout(first).unwrap().configuration_generation();
    assert!(
        !core
            .handle(first, CoreEvent::SetScale(1.))
            .unwrap()
            .layout_changed
    );
    for invalid in [0.1, 6., f32::NAN] {
        assert!(core.handle(first, CoreEvent::SetScale(invalid)).is_err());
    }
    assert_eq!(
        core.layout(first).unwrap().configuration_generation(),
        configuration
    );
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
