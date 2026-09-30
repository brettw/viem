use super::*;

type TestCore = Core<MockTextMeasurementProvider>;

fn setup(source: String, wrap: bool) -> (TestCore, ViewId) {
    let mut core = Core::new(Document::new(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 200., 120.);
    core.set_view_insets(view, crate::layout::EdgeInsets {
        left: 11., right: 13., ..Default::default()
    }).unwrap();
    core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
    (core, view)
}

fn key(core: &mut TestCore, view: ViewId, key: Key) {
    let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert!(matches!(outcome.command.unwrap().status,
        CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled), "{key:?}");
}

fn keys(core: &mut TestCore, view: ViewId, text: &str) {
    for character in text.chars() {
        key(core, view, Key::Char(character));
    }
}

fn command(core: &mut TestCore, view: ViewId, text: &str) {
    keys(core, view, text);
    let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete, "{text}");
}

fn endpoint_x(core: &TestCore, view: ViewId, offset: usize, affinity: BoundaryAffinity) -> f32 {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    snapshot.logical_endpoint_geometry(offset, affinity).or_else(|_| {
        snapshot.logical_endpoint_geometry(offset, match affinity {
            BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
        })
    }).unwrap().rect.x
}

fn assert_span_visible(core: &TestCore, view: ViewId, start: f32, end: f32) {
    let layout = core.layout(view).unwrap();
    let left = layout.viewport_left();
    let right = left + layout.width();
    assert!(start >= left - 0.05 && end <= right + 0.05,
        "horizontal span {start}..{end} must fit viewport {left}..{right}");
}

fn assert_normal_cursor_visible(core: &TestCore, view: ViewId) {
    let offset = core.command_state(view).unwrap().cursor();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let cluster = snapshot.rows.iter().flat_map(|row| &row.clusters)
        .find(|cluster| cluster.text_range.contains(&offset))
        .expect("the normal-mode cursor's complete glyph must be materialized");
    assert_span_visible(core, view, cluster.x, cluster.x + cluster.advance);
}

fn assert_match_visible(core: &TestCore, view: ViewId, start: usize, end: usize) {
    let start = endpoint_x(core, view, start, BoundaryAffinity::Downstream);
    let end = endpoint_x(core, view, end, BoundaryAffinity::Upstream);
    assert_span_visible(core, view, start, end);
}

fn assert_horizontal_origin(core: &TestCore, view: ViewId, expected: f32) {
    let actual = core.layout(view).unwrap().viewport_left();
    assert!((actual - expected).abs() < 0.05,
        "horizontal origin {actual} must equal {expected}");
}

fn assert_search_cursor_centered(core: &TestCore, view: ViewId, offset: usize) {
    let x = endpoint_x(core, view, offset, BoundaryAffinity::Downstream);
    let width = core.layout(view).unwrap().width();
    assert_horizontal_origin(core, view, x - width / 2.);
}

#[test]
fn horizontal_motion_reveals_the_entire_normal_cursor_with_and_without_wrapping() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(200), wrap);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 1, "an unbreakable word overflows even with wrapping");
        assert!(snapshot.rows[0].width > core.layout(view).unwrap().width());
        let initial_left = core.layout(view).unwrap().viewport_left();
        key(&mut core, view, Key::Char('l'));
        assert_eq!(core.layout(view).unwrap().viewport_left(), initial_left,
            "moving inside the viewport must leave its content stationary");
        keys(&mut core, view, "119l");
        assert_eq!(core.command_state(view).unwrap().cursor(), 120);
        assert_normal_cursor_visible(&core, view);
        let distant_left = core.layout(view).unwrap().viewport_left();
        assert!(distant_left > initial_left);
        for _ in 0..12 {
            key(&mut core, view, Key::Char('l'));
            assert_normal_cursor_visible(&core, view);
        }
        for _ in 0..40 {
            key(&mut core, view, Key::Char('h'));
            assert_normal_cursor_visible(&core, view);
        }
        key(&mut core, view, Key::Char('0'));
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_normal_cursor_visible(&core, view);
        assert!(core.layout(view).unwrap().viewport_left() < distant_left);
    }
}

#[test]
fn typing_reveals_the_advancing_insertion_caret_on_overflowing_rows() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(40), wrap);
        key(&mut core, view, Key::Char('A'));
        let original_left = core.layout(view).unwrap().viewport_left();
        for _ in 0..40 {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Text("x".into()))).unwrap();
            let commands = core.command_state(view).unwrap();
            let x = endpoint_x(&core, view, commands.cursor(), commands.boundary_affinity());
            assert_span_visible(&core, view, x, x + 1.);
        }
        assert!(core.layout(view).unwrap().viewport_left() > original_left);
        assert_eq!(core.document.source_bytes(), "x".repeat(80).as_bytes());
    }
}

#[test]
fn explicit_horizontal_scroll_keeps_the_cursor_in_place_until_a_cursor_motion() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(200), wrap);
        core.handle(view, CoreEvent::SetViewportOrigin { left: 500., top: None }).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.layout(view).unwrap().viewport_left(), 500.);
        core.poll_search(view).unwrap();
        assert_eq!(core.layout(view).unwrap().viewport_left(), 500.,
            "presentation refresh must preserve an explicit scroll away from the cursor");
        key(&mut core, view, Key::Char('l'));
        assert_normal_cursor_visible(&core, view);
        assert!(core.layout(view).unwrap().viewport_left() < 500.);
    }
}

#[test]
fn accepted_find_centers_the_cursor_and_repeat_searches_in_both_directions() {
    for wrap in [false, true] {
        let source = format!("{}needle{}needle{}", "x".repeat(100), "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        assert_match_visible(&core, view, 100, 106);
        assert_search_cursor_centered(&core, view, 100);
        key(&mut core, view, Key::Char('n'));
        assert_eq!(core.command_state(view).unwrap().cursor(), 206);
        assert_match_visible(&core, view, 206, 212);
        assert_search_cursor_centered(&core, view, 206);
        let second_left = core.layout(view).unwrap().viewport_left();
        key(&mut core, view, Key::Char('N'));
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        assert_match_visible(&core, view, 100, 106);
        assert_search_cursor_centered(&core, view, 100);
        assert!(core.layout(view).unwrap().viewport_left() < second_left);
    }
}

#[test]
fn a_fitting_find_shifts_only_as_far_from_center_as_its_right_edge_requires() {
    for wrap in [false, true] {
        let matched = "needle".repeat(3);
        let source = format!("{}{matched}{}", "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, &format!("/{matched}"));
        let start = endpoint_x(&core, view, 100, BoundaryAffinity::Downstream);
        let end = endpoint_x(&core, view, 100 + matched.len(), BoundaryAffinity::Upstream);
        let width = core.layout(view).unwrap().width();
        assert!(end - start > width / 2. && end - start < width,
            "the fixture must fit completely after shifting away from centered placement");
        assert_horizontal_origin(&core, view, end - width);
        assert_match_visible(&core, view, 100, 100 + matched.len());
        assert_normal_cursor_visible(&core, view);
    }
}

#[test]
fn find_centering_is_clamped_to_the_horizontal_document_edges() {
    for wrap in [false, true] {
        let (mut core, view) = setup(format!("needle{}", "x".repeat(100)), wrap);
        command(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_horizontal_origin(&core, view, 0.);
        assert_match_visible(&core, view, 0, 6);

        let (mut core, view) = setup(format!("{}needle", "x".repeat(100)), wrap);
        command(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        let layout = core.layout(view).unwrap();
        let maximum = layout.maximum_viewport_left().unwrap();
        let centered = endpoint_x(&core, view, 100, BoundaryAffinity::Downstream)
            - layout.width() / 2.;
        assert!(centered > maximum,
            "centering the final match would scroll past the document edge");
        assert_horizontal_origin(&core, view, maximum);
        assert_match_visible(&core, view, 100, 106);
    }
}

#[test]
fn accepted_overwide_find_prioritizes_the_beginning_of_the_match() {
    for wrap in [false, true] {
        let matched = "needle".repeat(40);
        let source = format!("{}{matched}{}", "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, &format!("/{matched}"));
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        let start = endpoint_x(&core, view, 100, BoundaryAffinity::Downstream);
        let end = endpoint_x(&core, view, 100 + matched.len(), BoundaryAffinity::Upstream);
        let layout = core.layout(view).unwrap();
        assert!(end - start > layout.width());
        assert_horizontal_origin(&core, view, start);
        assert_match_visible(&core, view, 100, 106);
    }
}

#[test]
fn incremental_find_reveals_the_match_and_cancel_restores_horizontal_scroll() {
    for wrap in [false, true] {
        let source = format!("{}needle{}", "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, ":set incsearch");
        core.handle(view, CoreEvent::SetViewportOrigin { left: 200., top: None }).unwrap();
        let before_left = core.layout(view).unwrap().viewport_left();
        let before_top = core.layout(view).unwrap().viewport_top();
        let history = core.document.history_status();
        keys(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0,
            "incremental preview must not commit its destination");
        assert_match_visible(&core, view, 100, 106);
        assert_search_cursor_centered(&core, view, 100);
        assert!(core.layout(view).unwrap().viewport_left() > before_left);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.layout(view).unwrap().viewport_left(), before_left);
        assert_eq!(core.layout(view).unwrap().viewport_top(), before_top);
        assert_eq!(core.document.history_status(), history);
    }
}

#[test]
fn distant_cursor_reveal_materializes_the_visible_band_with_bounded_geometry() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(200_000), wrap);
        let calls = core.views[&view].engine.provider().request_calls();
        keys(&mut core, view, "100000l");
        assert_eq!(core.command_state(view).unwrap().cursor(), 100_000);
        assert_normal_cursor_visible(&core, view);
        let layout = core.layout(view).unwrap();
        assert!(layout.viewport_left() > 100_000.);
        let snapshot = layout.snapshot().unwrap();
        assert_eq!(snapshot.rows.len(), 1, "the giant unbreakable word must overflow in either wrap mode");
        assert!(snapshot.has_horizontal_materialization());
        assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 6_000),
            "wrap={wrap}: revealing a distant cursor must retain sparse horizontal geometry");
        for fraction in [0.25, 0.5, 0.75] {
            let x = layout.viewport_left() + layout.width() * fraction;
            assert!(snapshot.horizontal_geometry_is_materialized(0, x),
                "wrap={wrap}: revealing the cursor must also materialize visible x={x}");
        }
        assert!(core.views[&view].engine.provider().request_calls() - calls < 128,
            "wrap={wrap}: revealing distant geometry must reuse the long-line summary");
    }
}

#[test]
fn giant_overflow_preview_materializes_the_match_far_from_the_authoritative_cursor() {
    for wrap in [false, true] {
        let source = format!("{}needle{}", "x".repeat(100_000), "x".repeat(100_000));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, ":set incsearch");
        let before_left = core.layout(view).unwrap().viewport_left();
        let revision = core.document.revision();
        let calls = core.views[&view].engine.provider().request_calls();
        keys(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_match_visible(&core, view, 100_000, 100_006);
        let layout = core.layout(view).unwrap();
        assert!(layout.viewport_left() > 100_000.);
        let snapshot = layout.snapshot().unwrap();
        assert!(snapshot.has_horizontal_materialization());
        assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 6_000),
            "wrap={wrap}: a distant preview must preserve sparse geometry");
        for fraction in [0.25, 0.5, 0.75] {
            let x = layout.viewport_left() + layout.width() * fraction;
            assert!(snapshot.horizontal_geometry_is_materialized(0, x),
                "wrap={wrap}: preview must materialize its visible band, not the old cursor's band");
        }
        key(&mut core, view, Key::Escape);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.layout(view).unwrap().viewport_left(), before_left);
        assert_normal_cursor_visible(&core, view);
        assert_eq!(core.document.revision(), revision);
        assert!(core.views[&view].engine.provider().request_calls() - calls < 256,
            "wrap={wrap}: preview and cancellation must reuse existing long-line summaries");
    }
}

#[test]
fn accepted_find_refills_matches_that_cross_the_old_sparse_band_edge() {
    for wrap in [false, true] {
        for (repetitions, scroll_distance, centered_band_present) in [(1, 1.9, false), (3, 1.4, true)] {
            let prefix_len = 100_000;
            let matched = "needle".repeat(repetitions);
            let source = format!("{}{matched}{}", "x".repeat(prefix_len), "x".repeat(100_000));
            let (mut core, view) = setup(source, wrap);
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            assert!(snapshot.has_horizontal_materialization());
            let first = snapshot.rows[0].clusters.iter()
                .find(|cluster| cluster.text_range.start == 0).unwrap();
            let match_x = first.x + first.advance * prefix_len as f32;
            let width = layout.width();
            let old_left = match_x - width * scroll_distance;
            core.handle(view, CoreEvent::SetViewportOrigin { left: old_left, top: None }).unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert_eq!(snapshot.covers_horizontal_viewport(match_x - width / 2., width),
                centered_band_present,
                "the longer-match fixture must already cover the centered viewport");
            assert!(snapshot.rows[0].clusters.iter()
                .any(|cluster| cluster.text_range.start == prefix_len),
                "the first match glyph must already be present at the old materialization edge");
            assert!(!snapshot.rows[0].clusters.iter()
                .any(|cluster| cluster.text_range.start == prefix_len + matched.len() - 1),
                "the last match glyph must require additional geometry");
            let calls = core.views[&view].engine.provider().request_calls();
            command(&mut core, view, &format!("/{matched}"));
            assert_eq!(core.command_state(view).unwrap().cursor(), prefix_len);
            if repetitions == 1 {
                assert_search_cursor_centered(&core, view, prefix_len);
            } else {
                let end = endpoint_x(&core, view, prefix_len + matched.len(), BoundaryAffinity::Upstream);
                assert!(end - match_x > width / 2. && end - match_x < width);
                assert_horizontal_origin(&core, view, end - width);
            }
            assert_match_visible(&core, view, prefix_len, prefix_len + matched.len());
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            for offset in prefix_len..prefix_len + matched.len() {
                assert!(snapshot.rows[0].clusters.iter()
                    .any(|cluster| cluster.text_range.contains(&offset)),
                    "the centered match must have complete glyph coverage at {offset}");
            }
            assert!(snapshot.rows[0].clusters.len() < 6_000);
            assert!(core.views[&view].engine.provider().request_calls() - calls < 128,
                "crossing old coverage must reuse the long-line summary");
        }
    }
}

#[test]
fn incremental_find_reveals_a_growing_match_even_when_its_start_does_not_change() {
    for wrap in [false, true] {
        let source = format!("{}needletailmore{}", "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, ":set incsearch");
        keys(&mut core, view, "/needle");
        assert_match_visible(&core, view, 100, 106);
        assert_search_cursor_centered(&core, view, 100);
        let short_left = core.layout(view).unwrap().viewport_left();
        keys(&mut core, view, "tail");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_match_visible(&core, view, 100, 110);
        assert_horizontal_origin(&core, view, short_left);
        keys(&mut core, view, "more");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        let start = endpoint_x(&core, view, 100, BoundaryAffinity::Downstream);
        let end = endpoint_x(&core, view, 114, BoundaryAffinity::Upstream);
        let width = core.layout(view).unwrap().width();
        assert!(end - start > width / 2. && end - start < width);
        assert_horizontal_origin(&core, view, end - width);
        assert!(core.layout(view).unwrap().viewport_left() > short_left,
            "same-start preview growth beyond the centered right edge must reveal its new end");
        assert_match_visible(&core, view, 100, 114);
        key(&mut core, view, Key::Enter);
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        assert_horizontal_origin(&core, view, end - width);
        assert_match_visible(&core, view, 100, 114);
    }
}

#[test]
fn rtl_find_centers_short_matches_and_minimally_shifts_to_fit_longer_matches() {
    for wrap in [false, true] {
        for repetitions in [1, 3] {
            let matched = "אבגדה".repeat(repetitions);
            let source = format!("{}{matched}{}", "x".repeat(30), "x".repeat(30));
            let (mut core, view) = setup(source, wrap);
            command(&mut core, view, &format!("/{matched}"));
            assert_eq!(core.command_state(view).unwrap().cursor(), 30);
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let first = snapshot.rows.iter().flat_map(|row| &row.clusters)
                .find(|cluster| cluster.text_range.start == 30).unwrap();
            let last = snapshot.rows.iter().flat_map(|row| &row.clusters)
                .find(|cluster| cluster.text_range.end == 30 + matched.len()).unwrap();
            assert_eq!(first.bidi_level % 2, 1);
            let start_x = first.x + first.advance;
            let end_x = last.x;
            let width = layout.width();
            if repetitions == 1 {
                assert!(start_x - end_x < width / 2.);
                assert_horizontal_origin(&core, view, start_x - width / 2.);
            } else {
                assert!(start_x - end_x > width / 2. && start_x - end_x < width);
                assert_horizontal_origin(&core, view, end_x);
            }
            assert_span_visible(&core, view, end_x, start_x);
            assert_normal_cursor_visible(&core, view);
        }
    }
}

#[test]
fn an_overwide_rtl_match_reveals_its_logical_beginning_at_the_visual_right() {
    for wrap in [false, true] {
        let matched = "אבגדה".repeat(30);
        let source = format!("{}{matched}{}", "x".repeat(30), "x".repeat(30));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, &format!("/{matched}"));
        assert_eq!(core.command_state(view).unwrap().cursor(), 30);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let logical_start = snapshot.rows.iter().flat_map(|row| &row.clusters)
            .find(|cluster| cluster.text_range.start == 30).unwrap();
        let logical_end = snapshot.rows.iter().flat_map(|row| &row.clusters)
            .find(|cluster| cluster.text_range.end == 30 + matched.len()).unwrap();
        assert_eq!(logical_start.bidi_level % 2, 1);
        assert!(logical_start.x - logical_end.x > core.layout(view).unwrap().width(),
            "the fixture's logical beginning must be at the distant visual right");
        assert_horizontal_origin(&core, view,
            logical_start.x + logical_start.advance + 2. - core.layout(view).unwrap().width());
        assert_span_visible(&core, view, logical_start.x, logical_start.x + logical_start.advance);
        for cluster in snapshot.rows.iter().flat_map(|row| &row.clusters)
            .filter(|cluster| (30..30 + "אבג".len()).contains(&cluster.text_range.start))
        {
            assert_span_visible(&core, view, cluster.x, cluster.x + cluster.advance);
        }
    }
}

#[test]
fn find_inside_an_indivisible_rtl_ligature_reveals_its_complete_leading_cluster() {
    use crate::layout::{
        MeasurementEnvironmentId, MeasurementError, MetricsGeneration, RenderRunPolicy,
        ShapeRequest, ShapedFragment, TextMeasurementProvider,
    };

    #[derive(Clone)]
    struct RtlLigatureProvider(MockTextMeasurementProvider);

    impl TextMeasurementProvider for RtlLigatureProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
            self.0.measurement_environment_id()
        }

        fn metrics_generation(&self) -> MetricsGeneration {
            self.0.metrics_generation()
        }

        fn render_run_policy(&self) -> Option<RenderRunPolicy> {
            self.0.render_run_policy()
        }

        fn shape_batch(&mut self, requests: &[ShapeRequest<'_>])
            -> Result<Vec<ShapedFragment>, MeasurementError>
        {
            let mut fragments = self.0.shape_batch(requests)?;
            for fragment in &mut fragments {
                for cluster in &mut fragment.clusters {
                    cluster.bidi_level = 1;
                    for caret in &mut cluster.caret_stops {
                        caret.inline_offset = cluster.advance - caret.inline_offset;
                    }
                }
                fragment.visual_order.reverse();
            }
            Ok(fragments)
        }
    }

    for wrap in [false, true] {
        for oversized in [false, true] {
            let matched = format!("fi{}", "needle".repeat(if oversized { 40 } else { 0 }));
            let source = format!("{}f{matched}{}", "x".repeat(30), "x".repeat(30));
            let mut core = Core::new(Document::new(source));
            let view = core.add_view(RtlLigatureProvider(MockTextMeasurementProvider::new()), 200., 120.);
            core.set_view_insets(view, crate::layout::EdgeInsets {
                left: 11., right: 13., ..Default::default()
            }).unwrap();
            core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
            for key in format!("/{matched}").chars().map(Key::Char).chain(std::iter::once(Key::Enter)) {
                let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                assert!(matches!(outcome.command.unwrap().status,
                    CommandStatus::Complete | CommandStatus::Pending), "{key:?}");
            }
            assert_eq!(core.command_state(view).unwrap().cursor(), 31,
                "the logical match must start inside the ffi shaping cluster");
            let layout = core.layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let geometry = snapshot.logical_endpoint_geometry(31, BoundaryAffinity::Downstream).unwrap();
            assert!(geometry.is_cluster_fallback);
            let first = snapshot.rows[geometry.row_index].clusters.iter()
                .find(|cluster| cluster.text_range.contains(&31)).unwrap();
            assert_eq!(first.text_range, 30..33);
            assert_eq!(first.bidi_level % 2, 1);
            let leading_edge = first.x + first.advance;
            let expected = if oversized {
                leading_edge + 2. - layout.width()
            } else {
                leading_edge - layout.width() / 2.
            };
            assert!((layout.viewport_left() - expected).abs() < 0.05,
                "wrap={wrap}, oversized={oversized}: the logical RTL beginning must anchor horizontal reveal");
            assert!(first.x >= layout.viewport_left() - 0.05,
                "the whole indivisible cursor cluster must remain visible");
            assert!(leading_edge + 2. <= layout.viewport_left() + layout.width() + 0.05,
                "the cluster's logical leading edge must leave room for the caret");
            let match_end = snapshot.logical_endpoint_geometry(31 + matched.len(), BoundaryAffinity::Upstream).unwrap();
            assert_eq!(leading_edge - match_end.rect.x > layout.width(), oversized,
                "the fixture must exercise both centered and oversized matches");
        }
    }
}

#[test]
fn find_ending_inside_a_combining_cluster_reveals_the_complete_displayed_cluster() {
    for wrap in [false, true] {
        let source = format!("{}needle\u{0301}{}", "x".repeat(100), "x".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, "/needle");
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let final_cluster = snapshot.rows.iter().flat_map(|row| &row.clusters)
            .find(|cluster| cluster.text_range.start == 105).unwrap();
        assert_eq!(final_cluster.text_range, 105..108,
            "the regex ends before the accent in the final grapheme");
        assert_span_visible(&core, view,
            endpoint_x(&core, view, 100, BoundaryAffinity::Downstream),
            final_cluster.x + final_cluster.advance);
    }
}

#[test]
fn multiline_find_reveals_its_first_row_without_chasing_the_distant_last_endpoint() {
    for wrap in [false, true] {
        let source = format!("{}start\n{}end{}", "x".repeat(100), "y".repeat(300), "y".repeat(100));
        let (mut core, view) = setup(source, wrap);
        command(&mut core, view, "/start\\n.*end");
        assert_eq!(core.command_state(view).unwrap().cursor(), 100);
        assert_match_visible(&core, view, 100, 105);
        assert_search_cursor_centered(&core, view, 100);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        let first = snapshot.logical_endpoint_geometry(100, BoundaryAffinity::Downstream).unwrap();
        let first_row = &snapshot.rows[first.row_index];
        assert!(first_row.y >= layout.viewport_top() - 0.05);
        assert!(first_row.y + first_row.height() <= layout.viewport_top() + layout.height() + 0.05);
        let last_x = endpoint_x(&core, view, 409, BoundaryAffinity::Upstream);
        assert!(last_x > layout.viewport_left() + layout.width(),
            "the distant endpoint may stay offscreen while the beginning is readable");
    }
}

#[test]
fn a_soft_wrapped_find_minimally_shifts_to_fit_only_its_first_visual_row() {
    let first_part = "needle".repeat(3);
    let source = format!("{}{first_part} {}", "x".repeat(100), "y".repeat(100));
    let (mut core, view) = setup(source, true);
    command(&mut core, view, &format!("/{first_part} yyy"));
    assert_eq!(core.command_state(view).unwrap().cursor(), 100);
    let layout = core.layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    let first = snapshot.logical_endpoint_geometry(100, BoundaryAffinity::Downstream).unwrap();
    let last = snapshot.logical_endpoint_geometry(100 + first_part.len() + 4,
        BoundaryAffinity::Upstream).unwrap();
    assert!(last.row_index > first.row_index,
        "the search must span a soft wrap within one logical source line");
    let first_row = &snapshot.rows[first.row_index];
    let match_start = endpoint_x(&core, view, 100, BoundaryAffinity::Downstream);
    let match_right = first_row.clusters.iter()
        .filter(|cluster| cluster.text_range.end > 100)
        .map(|cluster| cluster.x + cluster.advance)
        .fold(match_start, f32::max);
    let width = layout.width();
    assert!(match_right - match_start > width / 2. && match_right - match_start < width);
    assert_horizontal_origin(&core, view, match_right - width);
    assert_span_visible(&core, view, match_start, match_right);
    assert_normal_cursor_visible(&core, view);
}

#[test]
fn marked_text_selection_and_commit_keep_the_horizontal_caret_visible() {
    use crate::command::composition::{CompositionTarget, CompositionUpdate};
    for wrap in [false, true] {
        let original = "x".repeat(40);
        let (mut core, view) = setup(original.clone(), wrap);
        key(&mut core, view, Key::Char('A'));
        let offset = core.command_state(view).unwrap().cursor();
        let target = CompositionTarget::at_offsets(&core.document, offset..offset).unwrap();
        core.handle(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
        let marked = "m".repeat(40);
        for selected in [marked.len(), 0, 20, marked.len()] {
            core.handle(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate {
                marked_text: marked.clone(), selected_range: selected..selected,
            }))).unwrap();
            let layout = core.presentation_layout(view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let geometry = snapshot.logical_endpoint_geometry(offset + selected, BoundaryAffinity::Downstream)
                .or_else(|_| snapshot.logical_endpoint_geometry(offset + selected, BoundaryAffinity::Upstream)).unwrap();
            assert!(geometry.rect.x >= layout.viewport_left() - 0.05);
            assert!(geometry.rect.x + 1. <= layout.viewport_left() + layout.width() + 0.05,
                "the selected caret inside marked text must remain visible");
            assert_eq!(core.document.source_bytes(), original.as_bytes());
        }
        let composed_left = core.presentation_layout(view).unwrap().viewport_left();
        core.handle(view, CoreEvent::Composition(CompositionEvent::Commit)).unwrap();
        let commands = core.command_state(view).unwrap();
        let x = endpoint_x(&core, view, commands.cursor(), commands.boundary_affinity());
        assert_span_visible(&core, view, x, x + 1.);
        assert!((core.layout(view).unwrap().viewport_left() - composed_left).abs() < 0.05,
            "committing the same displayed text must preserve its horizontal position");
        assert_eq!(core.document.source_bytes(), format!("{original}{marked}").as_bytes());
    }
}

#[test]
fn ordinary_horizontal_cursor_reveal_reuses_shaping_and_snapshot_geometry() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(200), wrap);
        let calls = core.views[&view].engine.provider().request_calls();
        let revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        for motion in ["120l", "10h", "$", "0"] {
            keys(&mut core, view, motion);
            assert_normal_cursor_visible(&core, view);
            assert_eq!(core.views[&view].engine.provider().request_calls(), calls,
                "{motion}: moving through materialized text must reuse shaping");
            assert_eq!(core.layout(view).unwrap().snapshot().unwrap().revision, revision,
                "{motion}: scrolling alone must not rebuild the immutable snapshot");
        }
    }
}

#[test]
fn oversized_horizontal_margins_yield_room_for_the_cursor_without_reveal_oscillation() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(200), wrap);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            left: 500., right: 600., ..Default::default()
        }).unwrap();
        keys(&mut core, view, "120l");
        assert_normal_cursor_visible(&core, view);
        let left = core.layout(view).unwrap().viewport_left();
        for _ in 0..12 {
            reveal_caret_row(&core.document, core.views.get_mut(&view).unwrap()).unwrap();
            assert_normal_cursor_visible(&core, view);
            assert!((core.layout(view).unwrap().viewport_left() - left).abs() < 0.05,
                "repeated reveal must remain stationary when margins exceed the viewport width");
        }
    }
}


#[test]
fn insertion_at_the_overflowing_line_end_leaves_room_for_the_caret_without_margins() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(40), wrap);
        core.set_view_insets(view, crate::layout::EdgeInsets::default()).unwrap();
        key(&mut core, view, Key::Char('A'));
        for _ in 0..8 {
            let commands = core.command_state(view).unwrap();
            let x = endpoint_x(&core, view, commands.cursor(), commands.boundary_affinity());
            assert_span_visible(&core, view, x, x + 2.);
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Text("x".into()))).unwrap();
        }
    }
}

#[test]
fn existing_right_margin_accommodates_the_caret_without_spurious_horizontal_overflow() {
    for wrap in [false, true] {
        let (mut core, view) = setup("x".repeat(20), wrap);
        let end = endpoint_x(&core, view, 20, BoundaryAffinity::Upstream);
        core.handle(view, CoreEvent::Resize { width: end + 13., height: 120. }).unwrap();
        assert_eq!(core.layout(view).unwrap().maximum_viewport_left(), Some(0.));
        key(&mut core, view, Key::Char('A'));
        assert_span_visible(&core, view, end, end + 2.);
        assert_eq!(core.layout(view).unwrap().viewport_left(), 0.);
    }
}
