use super::*;
use crate::document::{Document, Encoding};
use crate::layout::*;

fn fixture(
    text: &str,
    format: Format,
    tabstop: u32,
) -> (
    Document,
    LayoutEngine<MockTextMeasurementProvider>,
    ViewLayout,
) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(400.0, 160.0);
    view.set_wrap(false);
    view.set_whitespace_presentation(Default::default(), format, tabstop)
        .unwrap();
    (document, engine, view)
}
fn markers(document: &Document, view: &ViewLayout) -> Vec<WhitespaceMarker> {
    view.whitespace_markers(
        document.projection().text_tree(),
        view.snapshot().unwrap(),
        LayoutRect {
            x: 0.0,
            y: 0.0,
            width: view.width(),
            height: view.height(),
        },
    )
}

#[test]
fn listchars_validates_all_categories_escapes_width_and_duplicates() {
    let chars = ListChars::parse("eol:$,tab:<->,leadtab:>-,space:.,multispace:-+,lead:.,leadmultispace:xy,trail:*,extends:>,precedes:<,conceal:?,nbsp:%,eol:\\u21b5").unwrap();
    assert_eq!(chars.get("eol"), Some(['↵'].as_slice()));
    assert_eq!(
        ListChars::parse("space:\\x2c").unwrap().get("space"),
        Some([','].as_slice())
    );
    for valid in ["space:\\", "tab:\\-", "trail:*,", "space::"] {
        assert!(ListChars::parse(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "tab:x",
        "tab:abcd",
        "trail:ab",
        "trail:",
        "multispace:",
        "leadtab:a",
        "leadtab:>-",
        "space:界",
        "space:\\u0301",
        "space:\\x09",
        "space:\\uD800",
        "space:\\u12",
        "unknown:x",
        "trail:*,,",
    ] {
        assert!(ListChars::parse(invalid).is_err(), "{invalid}");
    }
    assert!(ListChars::parse("").is_ok());
    let value = serde_json::to_value(WhitespacePresentationOptions::default()).unwrap();
    assert_eq!(value["codeWhitespace"], "paragraphEn");
    assert_eq!(value["visibleWhitespace"]["enabled"], true);
    let mut options = WhitespacePresentationOptions::default();
    options.visible_whitespace.style.size = Some((-1.0).into());
    assert!(options.validate().is_err());
}

#[test]
fn leading_advances_ignore_character_styles_body_spaces_keep_their_font() {
    let (document, mut engine, mut view) = fixture(" \t  x x\t", Format::Code, 4);
    view.set_style_runs(vec![ShapeStyleRun {
        text_range: 0..document.text().len(),
        style: ResolvedTextStyle {
            size: 42.0,
            letter_spacing: 3.0,
            ..Default::default()
        },
    }])
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    let clusters = &snapshot.rows[0].clusters;
    assert_eq!(snapshot.whitespace_unit, 7.0);
    assert_eq!(
        clusters
            .iter()
            .take(4)
            .map(|c| c.advance)
            .collect::<Vec<_>>(),
        [7.0, 21.0, 7.0, 7.0]
    );
    assert_eq!(clusters[5].advance, 24.0);
    let before_tab: f32 = clusters.iter().take(7).map(|c| c.advance).sum();
    assert!(((before_tab + clusters[7].advance) % 28.0).abs() < 0.001);
    let stops: Vec<_> = snapshot.rows[0]
        .carets
        .iter()
        .map(|c| c.point.text_offset)
        .collect();
    assert!(stops.iter().all(|offset| *offset <= document.text().len()));
    assert_eq!(document.text(), " \t  x x\t");
}

#[test]
fn spaces_mode_preserves_current_font_spaces_and_measures_each_tabs_font() {
    let (document, mut engine, mut view) = fixture(" \tX \t", Format::PlainText, 4);
    view.set_style_runs(vec![
        ShapeStyleRun {
            text_range: 0..2,
            style: ResolvedTextStyle {
                size: 40.0,
                letter_spacing: 2.0,
                weight: 700.0,
                font_families: vec!["Writer Serif".into()],
                ..Default::default()
            },
        },
        ShapeStyleRun {
            text_range: 3..5,
            style: ResolvedTextStyle {
                size: 24.0,
                ..Default::default()
            },
        },
    ])
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let clusters = &view.snapshot().unwrap().rows[0].clusters;
    assert_eq!(clusters[0].advance, 22.0);
    assert_eq!(clusters[1].whitespace_unit, Some(22.0));
    assert_eq!(clusters[1].advance, 66.0);
    assert_eq!(clusters[3].advance, 12.0);
    assert_eq!(clusters[4].whitespace_unit, Some(12.0));
    let end: f32 = clusters.iter().map(|c| c.advance).sum();
    assert!((end % 48.0).abs() < 0.001);
    let before_calls = engine.provider().request_calls();
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(engine.provider().request_calls(), before_calls);
}

#[test]
fn marker_patterns_use_remaining_tab_width_and_do_not_change_layout() {
    let (document, mut engine, mut view) = fixture(" \tX  \n\tY\u{a0}", Format::Code, 2);
    let mut options = WhitespacePresentationOptions::default();
    options.visible_whitespace.listchars = "tab:<->,trail:*,lead:.,nbsp:%,eol:$".into();
    view.set_whitespace_presentation(options.clone(), Format::Code, 2)
        .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let before = view.snapshot().unwrap().clone();
    let markers = markers(&document, &view);
    let first_tab: Vec<_> = markers
        .iter()
        .filter(|m| m.kind == WhitespaceMarkerKind::Tab && m.row_index == 0)
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(first_tab, [">"]);
    let second_tab: Vec<_> = markers
        .iter()
        .filter(|m| m.kind == WhitespaceMarkerKind::Tab && m.row_index == 1)
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(second_tab, ["<", ">"]);
    assert_eq!(markers.iter().filter(|m| m.text == "*").count(), 2);
    assert_eq!(markers.iter().filter(|m| m.text == "%").count(), 1);
    options.visible_whitespace.enabled = false;
    view.set_whitespace_presentation(options, Format::Code, 2)
        .unwrap();
    assert!(super::tests::markers(&document, &view).is_empty());
    assert_eq!(view.snapshot().unwrap(), &before);
    assert_ne!(
        view.configuration_generation(),
        before.configuration_generation
    );
    let calls = engine.provider().request_calls();
    engine.relayout(&document, &mut view).unwrap();
    assert_ne!(
        view.snapshot().unwrap().configuration_generation,
        before.configuration_generation
    );
    assert_eq!(
        view.snapshot().unwrap().rows[0].clusters,
        before.rows[0].clusters
    );
    assert_eq!(engine.provider().request_calls(), calls);
}

#[test]
fn trailing_multispace_falls_back_without_using_leading_patterns_on_blank_lines() {
    let (document, mut engine, mut view) = fixture("  A  \n    \n A \n  A \nA  B  ", Format::Code, 2);
    engine.relayout(&document, &mut view).unwrap();
    let original_rows = view.snapshot().unwrap().rows.clone();
    let source = document.source_bytes();
    for (listchars, expected_rows) in [
        ("multispace:xy", ["xyxy", "xyxy", "", "xy", "xyxy"]),
        ("space:.,multispace:xy", ["xyxy", "xyxy", "..", "xy.", "xyxy"]),
        ("leadmultispace:ab,multispace:xy", ["abxy", "xyxy", "", "ab", "xyxy"]),
        ("lead:L,leadmultispace:ab,multispace:xy,space:.", ["abxy", "xyxy", "L.", "ab.", "xyxy"]),
        ("trail:*,lead:L,leadmultispace:ab,multispace:xy,space:.", ["ab**", "****", "L*", "ab*", "xy**"]),
    ] {
        let mut options = WhitespacePresentationOptions::default();
        options.visible_whitespace.listchars = listchars.into();
        view.set_whitespace_presentation(options, Format::Code, 2).unwrap();
        engine.relayout(&document, &mut view).unwrap();
        let planned = markers(&document, &view);
        let actual_rows: Vec<String> = (0..5).map(|row| planned.iter()
            .filter(|marker| marker.row_index == row && marker.kind == WhitespaceMarkerKind::Space)
            .map(|marker| marker.text.as_str()).collect()).collect();
        assert_eq!(actual_rows, expected_rows, "{listchars}");
        for (row, original) in view.snapshot().unwrap().rows.iter().zip(original_rows.iter()) {
            assert_eq!((row.y, row.baseline, row.width, row.line_advance),
                (original.y, original.baseline, original.width, original.line_advance));
            assert_eq!(row.clusters, original.clusters);
            assert_eq!(row.carets.iter().map(|caret| (caret.x, caret.row_index)).collect::<Vec<_>>(),
                original.carets.iter().map(|caret| (caret.x, caret.row_index)).collect::<Vec<_>>());
        }
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn omitted_tab_falls_back_and_wysiwyg_suppresses_markers() {
    let (document, mut engine, mut view) = fixture("\t ", Format::PlainText, 2);
    let mut options = WhitespacePresentationOptions::default();
    options.visible_whitespace.listchars = "trail:*".into();
    view.set_whitespace_presentation(options.clone(), Format::PlainText, 2)
        .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let planned = markers(&document, &view);
    assert!(planned.iter().any(|m| m.text == "^I"));
    assert!(planned.iter().any(|m| m.text == "*"));
    view.set_whitespace_presentation(options, Format::Html, 2)
        .unwrap();
    assert!(markers(&document, &view).is_empty());
}

#[test]
fn whitespace_geometry_changes_invalidate_layout_but_reuse_shaping() {
    let (document, mut engine, mut view) = fixture(" \tx", Format::Code, 2);
    engine.relayout(&document, &mut view).unwrap();
    let calls = engine.provider().request_calls();
    let generation = view.configuration_generation();
    let old_x = view.snapshot().unwrap().rows[0].clusters[2].x;
    view.set_whitespace_presentation(Default::default(), Format::Code, 4)
        .unwrap();
    assert_ne!(view.configuration_generation(), generation);
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(engine.provider().request_calls(), calls);
    assert_eq!(view.snapshot().unwrap().rows[0].clusters[2].x - old_x, 14.0);
}

#[test]
fn large_document_regional_whitespace_work_stays_local() {
    let text = " \tline\n".repeat(100_000);
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 4);
    let request = prepare_layout_job(
        &document,
        &mut view,
        inspect_layout_provider(&engine),
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(50_000..50_003).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    assert!(request.captured_text_len() < 100);
    let result =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    assert!(
        result
            .regional_snapshot()
            .work_statistics()
            .segmented_text_bytes()
            < 100
    );
    for line in result.regional_snapshot().lines() {
        assert_eq!(
            line.rows()[0].clusters[2].x - line.rows()[0].clusters[0].x,
            28.0
        );
    }
}

#[test]
fn giant_unwrapped_whitespace_geometry_matches_complete_layout_and_reuses_summary() {
    let text = format!(" \t{}\tZ", "ab cd ".repeat(12_000));
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 4);
    let mut reference = view.clone();
    let mut reference_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    reference_engine
        .relayout(&document, &mut reference)
        .unwrap();
    let expected = reference.snapshot().unwrap().rows[0].width;
    for id in 1..=2 {
        let requirements = inspect_layout_provider(&engine);
        let region = ViewportLayoutRegion::new(0..1, 0.0, view.height())
            .unwrap()
            .with_horizontal_focus(text.len() - 1, None);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(id),
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        if id == 2 {
            assert!(
                candidate
                    .regional_snapshot()
                    .work_statistics()
                    .segmented_text_bytes()
                    < 20_000
            );
        }
        assert_eq!(
            candidate.regional_snapshot().lines()[0].rows()[0].width,
            expected
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
    }
}

#[test]
fn giant_whitespace_bounds_are_scanned_once_per_snapshot_and_invalidate_on_edit() {
    let (mut document, mut engine, mut view) = fixture(&" ".repeat(100_000), Format::Code, 2);
    let requirements = inspect_layout_provider(&engine);
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 160.0).unwrap()),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
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
    let mut cache = WhitespaceBoundsCache::default();
    let config = WhitespaceConfiguration {
        format: Format::Code,
        ..Default::default()
    };
    let viewport = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 160.0,
    };
    for _ in 0..3 {
        let result = marker_plan(
            &config,
            &mut cache,
            document.projection().text_tree(),
            view.snapshot().unwrap(),
            viewport,
            false,
        );
        assert!(result.len() <= 60);
        assert!(result.iter().any(|m| m.text == "*"));
    }
    assert_eq!(cache.scans, 1);
    document.insert(0, "x").unwrap();
    assert_eq!(
        cache.bounds(document.projection().text_tree(), 0, 0..100_001),
        (0, 1)
    );
    assert_eq!(cache.scans, 2);
}

#[test]
fn deep_giant_multispace_patterns_reuse_the_cached_run_start() {
    let (document, mut engine, mut view) =
        fixture(&format!("{}X", " ".repeat(100_000)), Format::Code, 2);
    view.set_viewport_left(350_000.0).unwrap();
    let requirements = inspect_layout_provider(&engine);
    let request = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(
            ViewportLayoutRegion::new(0..1, 0.0, 160.0)
                .unwrap()
                .with_horizontal_focus(50_000, None),
        ),
        LayoutCancellationToken::new(),
    )
    .unwrap();
    let candidate =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
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
    let mut cache = WhitespaceBoundsCache::default();
    let mut config = WhitespaceConfiguration {
        format: Format::Code,
        ..Default::default()
    };
    config.options.visible_whitespace.listchars = "leadmultispace:-+,multispace:ab".into();
    let viewport = LayoutRect {
        x: 350_000.0,
        y: 0.0,
        width: 400.0,
        height: 160.0,
    };
    for _ in 0..3 {
        let result = marker_plan(
            &config,
            &mut cache,
            document.projection().text_tree(),
            view.snapshot().unwrap(),
            viewport,
            false,
        );
        assert!(!result.is_empty());
        assert!(result.len() <= 60);
        assert_eq!(result[0].text, "-");
        assert_eq!(result[1].text, "+");
    }
    assert_eq!(cache.scans, 1);
    assert_eq!(cache.run_scans, 1);
}
