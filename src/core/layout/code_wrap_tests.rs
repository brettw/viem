use super::*;
use crate::document::{Document, Encoding, Format};
use crate::layout::*;

fn fixture(
    text: &str,
    format: Format,
    width: f32,
) -> (
    Document,
    LayoutEngine<MockTextMeasurementProvider>,
    ViewLayout,
) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(width, 160.0);
    view.set_wrap(true);
    view.set_whitespace_presentation(Default::default(), format, 4)
        .unwrap();
    (document, engine, view)
}

fn assert_continuation_margin(snapshot: &LayoutSnapshot, margin: f32) {
    assert!(snapshot.rows.len() >= 3);
    assert_eq!(snapshot.rows[0].paragraph_content_x, 0.0);
    for row in snapshot.rows.iter().skip(1) {
        assert!(row.wrapped_from_previous);
        assert_eq!(row.paragraph_content_x, margin);
        assert_eq!(row.clusters[0].x, margin);
        assert!(row.carets.iter().all(|caret| caret.x >= margin));
    }
}

#[test]
fn wrapped_indent_defaults_and_validation_are_backward_compatible_for_saved_settings() {
    let defaults = WhitespacePresentationOptions::default();
    assert_eq!(defaults.code_wrapped_line_indent, 4);
    assert_eq!(
        serde_json::to_value(&defaults).unwrap()["codeWrappedLineIndent"],
        4
    );
    let old: WhitespacePresentationOptions =
        serde_json::from_str(r#"{"codeWhitespace":"spaces"}"#).unwrap();
    assert_eq!(old.code_wrapped_line_indent, 4);
    for indent in [0, 4, 1024] {
        let options = WhitespacePresentationOptions {
            code_wrapped_line_indent: indent,
            ..Default::default()
        };
        assert!(options.validate().is_ok());
    }
    let options = WhitespacePresentationOptions {
        code_wrapped_line_indent: 1025,
        ..Default::default()
    };
    assert!(options.validate().is_err());
}

#[test]
fn paragraph_en_adds_the_original_mixed_indent_and_ignores_character_font_size() {
    let text = " \t  aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp";
    let (document, mut engine, mut view) = fixture(text, Format::Code, 230.0);
    view.set_style_runs(vec![ShapeStyleRun {
        text_range: 0..text.len(),
        style: ResolvedTextStyle {
            size: 40.0,
            letter_spacing: 2.0,
            ..Default::default()
        },
    }])
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    // Space + tab-to-column-four + two spaces = six paragraph ens. The
    // independent default adds four ens on every continuation, without nesting.
    assert_eq!(snapshot.whitespace_unit, 7.0);
    assert_eq!(snapshot.rows[0].clusters[4].x, 42.0);
    assert_continuation_margin(snapshot, 70.0);
    assert!(snapshot
        .rows
        .iter()
        .skip(1)
        .all(|row| row.paragraph_content_width == 160.0));
}

#[test]
fn spaces_basis_uses_the_leading_fonts_measured_spaces_and_tab_stops() {
    let text = " \t  aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt";
    let (document, mut engine, mut view) = fixture(text, Format::Code, 340.0);
    let options = WhitespacePresentationOptions {
        code_whitespace: WhitespaceBasis::Spaces,
        ..Default::default()
    };
    view.set_whitespace_presentation(options, Format::Code, 4)
        .unwrap();
    view.set_style_runs(vec![ShapeStyleRun {
        text_range: 0..4,
        style: ResolvedTextStyle {
            size: 40.0,
            letter_spacing: 2.0,
            font_families: vec!["Writer Serif".into()],
            ..Default::default()
        },
    }])
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    // The mock font measures each leading space at 22 units. The body font
    // remains 14 points; it must not replace the leading font for the margin.
    assert_eq!(snapshot.rows[0].clusters[0].advance, 22.0);
    assert_eq!(snapshot.rows[0].clusters[1].advance, 66.0);
    assert_eq!(snapshot.rows[0].clusters[4].x, 132.0);
    assert_continuation_margin(snapshot, 220.0);
}

#[test]
fn unindented_lines_measure_extra_spaces_in_the_hard_line_start_style() {
    let text = "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr";
    let (document, mut engine, mut view) = fixture(text, Format::Code, 170.0);
    view.set_whitespace_presentation(
        WhitespacePresentationOptions {
            code_whitespace: WhitespaceBasis::Spaces,
            ..Default::default()
        },
        Format::Code,
        4,
    )
    .unwrap();
    view.set_style_runs(vec![ShapeStyleRun {
        text_range: 0..2,
        style: ResolvedTextStyle {
            size: 40.0,
            letter_spacing: 2.0,
            ..Default::default()
        },
    }])
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_continuation_margin(view.snapshot().unwrap(), 88.0);
}

#[test]
fn zero_extra_indent_still_aligns_continuations_with_original_indentation() {
    let (document, mut engine, mut view) = fixture(
        " \t  aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp",
        Format::Code,
        130.0,
    );
    view.set_whitespace_presentation(
        WhitespacePresentationOptions {
            code_wrapped_line_indent: 0,
            ..Default::default()
        },
        Format::Code,
        4,
    )
    .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_continuation_margin(view.snapshot().unwrap(), 42.0);
}

#[test]
fn continuation_indent_is_recomputed_at_each_hard_line() {
    let body = "aa bb cc dd ee ff gg hh ii jj kk ll";
    let text = format!("  {body}\n\t  {body}\n{body}");
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 130.0);
    engine.relayout(&document, &mut view).unwrap();
    for (hard_line, expected_margin) in [42.0, 70.0, 28.0].into_iter().enumerate() {
        let rows: Vec<_> = view
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .filter(|row| row.hard_line_index == hard_line)
            .collect();
        assert!(rows.len() > 1);
        assert_eq!(rows[0].paragraph_content_x, 0.0);
        assert!(rows
            .iter()
            .skip(1)
            .all(|row| row.paragraph_content_x == expected_margin));
    }
}

#[test]
fn continuation_margin_has_no_text_whitespace_markers_or_extra_caret_stops() {
    let text = "  aa bb cc dd ee ff gg hh ii jj kk ll mm nn";
    let (document, mut engine, mut view) = fixture(text, Format::Code, 120.0);
    let source = document.source_bytes();
    let mut options = WhitespacePresentationOptions::default();
    options.visible_whitespace.listchars = "space:.,lead:.,tab:>-,eol:$".into();
    view.set_whitespace_presentation(options, Format::Code, 4)
        .unwrap();
    engine.relayout(&document, &mut view).unwrap();
    let snapshot = view.snapshot().unwrap();
    assert_continuation_margin(snapshot, 42.0);
    let clusters: Vec<_> = snapshot.rows.iter().flat_map(|row| &row.clusters).collect();
    assert_eq!(clusters.len(), text.len());
    for (offset, cluster) in clusters.iter().enumerate() {
        assert_eq!(cluster.text_range, offset..offset + 1);
    }
    let markers = view.whitespace_markers(
        document.projection().text_tree(),
        snapshot,
        LayoutRect {
            x: 0.0,
            y: 0.0,
            width: view.width(),
            height: 10_000.0,
        },
    );
    assert!(!markers.is_empty());
    for (index, row) in snapshot.rows.iter().enumerate().skip(1) {
        for x in [0.0, 10.0, 41.0] {
            let hit = snapshot
                .hit_test(LayoutPoint {
                    x,
                    y: row.y + row.height() / 2.0,
                })
                .unwrap();
            assert_eq!(hit.text_offset, row.text_range.start);
            assert_eq!(snapshot.caret_geometry(hit).unwrap().rect.x, 42.0);
        }
        assert!(markers
            .iter()
            .filter(|marker| marker.row_index == index)
            .all(|marker| marker.rect.x >= 42.0));
        assert!(row
            .carets
            .iter()
            .all(|caret| row.text_range.contains(&caret.point.text_offset)
                || caret.point.text_offset == row.text_range.end));
    }
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.text(), text);
}

#[test]
fn wrapped_indent_does_not_change_other_formats_or_unwrapped_code_geometry() {
    let text = "  aa bb cc dd ee ff gg hh ii jj kk ll mm nn";
    for (format, wrapped) in [
        (Format::PlainText, true),
        (Format::MarkdownSource, true),
        (Format::HtmlSource, true),
        (Format::Code, false),
    ] {
        let (document, mut engine, mut view) = fixture(text, format, 120.0);
        view.set_wrap(wrapped);
        engine.relayout(&document, &mut view).unwrap();
        let before = view.snapshot().unwrap().rows.clone();
        view.set_whitespace_presentation(
            WhitespacePresentationOptions {
                code_wrapped_line_indent: 20,
                ..Default::default()
            },
            format,
            4,
        )
        .unwrap();
        engine.relayout(&document, &mut view).unwrap();
        let after = &view.snapshot().unwrap().rows;
        assert_eq!(after.len(), before.len());
        for (after, before) in after.iter().zip(before.iter()) {
            assert_eq!(after.text_range, before.text_range);
            assert_eq!(after.paragraph_content_x, before.paragraph_content_x);
            assert_eq!(
                after.paragraph_content_width,
                before.paragraph_content_width
            );
            assert_eq!(after.clusters, before.clusters);
        }
    }
}

#[test]
fn changing_wrapped_indent_invalidates_wrapping_and_height_but_reuses_shaping() {
    let text = format!("  {}", "aa bb cc dd ".repeat(15));
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 140.0);
    engine.relayout(&document, &mut view).unwrap();
    let before = view.snapshot().unwrap().clone();
    let calls = engine.provider().request_calls();
    view.set_whitespace_presentation(
        WhitespacePresentationOptions {
            code_wrapped_line_indent: 8,
            ..Default::default()
        },
        Format::Code,
        4,
    )
    .unwrap();
    assert_ne!(
        view.configuration_generation(),
        before.configuration_generation
    );
    engine.relayout(&document, &mut view).unwrap();
    let after = view.snapshot().unwrap();
    assert_continuation_margin(after, 70.0);
    assert_eq!(after.rows[0].text_range, before.rows[0].text_range);
    assert!(after.rows.len() > before.rows.len());
    assert!(after.rows.last().unwrap().y > before.rows.last().unwrap().y);
    assert_eq!(engine.provider().request_calls(), calls);
}

#[test]
fn large_document_regional_wrapping_only_measures_requested_hard_lines() {
    let text = " \talpha beta gamma delta epsilon\n".repeat(100_000);
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 120.0);
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
    assert!(request.captured_text_len() < 200);
    let result =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    assert!(
        result
            .regional_snapshot()
            .work_statistics()
            .segmented_text_bytes()
            < 200
    );
    assert_eq!(result.regional_snapshot().lines().len(), 3);
    for line in result.regional_snapshot().lines() {
        assert!(line.rows().len() >= 3);
        assert_eq!(line.rows()[0].paragraph_content_x, 0.0);
        assert!(line
            .rows()
            .iter()
            .skip(1)
            .all(|row| row.paragraph_content_x == 56.0));
    }
}

#[test]
fn resumed_long_code_line_preserves_original_indent_and_matches_complete_geometry() {
    let text = format!(" \t{}", "ab cd\t".repeat(24_000));
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 240.0);
    let requirements = inspect_layout_provider(&engine);
    let mut checkpoint = None;
    let mut rows = Vec::new();
    let mut jobs = 0;
    for job in 1..=10 {
        let region = checkpoint.take().map_or_else(
            || ViewportLayoutRegion::new(0..1, 0.0, 160.0).unwrap(),
            |checkpoint| ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 160.0).unwrap(),
        );
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(job),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let result =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let line = &result.regional_snapshot().lines()[0];
        rows.extend(line.rows().iter().cloned());
        checkpoint = line.next_checkpoint().cloned();
        jobs = job;
        if checkpoint.is_none() {
            assert!(line.height_is_exact());
            break;
        }
    }
    assert!(checkpoint.is_none());
    assert!(jobs > 1);
    let mut complete_view = view.clone();
    let mut complete_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    complete_engine
        .relayout(&document, &mut complete_view)
        .unwrap();
    let complete = complete_view.snapshot().unwrap();
    assert_continuation_margin(complete, 56.0);
    assert_eq!(rows.len(), complete.rows.len());
    for (resumed, full) in rows.iter().zip(complete.rows.iter()) {
        assert_eq!(resumed.text_range, full.text_range);
        assert_eq!(resumed.fragment_index, full.fragment_index);
        assert_eq!(resumed.y, full.y);
        assert_eq!(resumed.baseline, full.baseline);
        assert_eq!(resumed.width, full.width);
        assert_eq!(resumed.paragraph_content_x, full.paragraph_content_x);
        assert_eq!(
            resumed.paragraph_content_width,
            full.paragraph_content_width
        );
        assert_eq!(resumed.clusters, full.clusters);
    }
}

#[test]
fn changing_wrapped_indent_rejects_previous_long_line_checkpoints() {
    let text = format!("  {}", "aa bb ".repeat(15_000));
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 140.0);
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
    let result =
        compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let checkpoint = result.next_long_line_checkpoint().unwrap().clone();
    view.set_whitespace_presentation(
        WhitespacePresentationOptions {
            code_wrapped_line_indent: 8,
            ..Default::default()
        },
        Format::Code,
        4,
    )
    .unwrap();
    let result = prepare_layout_job(
        &document,
        &mut view,
        requirements,
        LayoutJobId(2),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::Viewport(
            ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 160.0).unwrap(),
        ),
        LayoutCancellationToken::new(),
    );
    assert!(matches!(
        result,
        Err(LayoutJobError::InvalidLongLineCheckpoint(_))
    ));
}

#[test]
fn leading_tabs_beyond_first_long_line_slice_keep_the_complete_original_indent() {
    let tab_count = MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 14;
    // The trailing space belongs to an extended grapheme with its combining
    // mark, so it must not be split off and counted as another indent unit.
    let text = format!("{} \u{301}x aa bb cc dd ee ff", "\t".repeat(tab_count));
    let (document, mut engine, mut view) = fixture(&text, Format::Code, 240.0);
    let margin = (tab_count * 4 + 4) as f32 * 7.0;
    let mut complete_view = view.clone();
    let mut complete_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    complete_engine
        .relayout(&document, &mut complete_view)
        .unwrap();
    let complete = complete_view.snapshot().unwrap();
    assert_eq!(complete.rows[0].paragraph_content_x, 0.0);
    assert!(complete
        .rows
        .iter()
        .skip(1)
        .all(|row| row.paragraph_content_x == margin));

    let requirements = inspect_layout_provider(&engine);
    let mut checkpoint = None;
    let mut completed_rows = 0;
    let mut jobs = 0;
    for job in 1..=4 {
        let region = checkpoint.take().map_or_else(
            || ViewportLayoutRegion::new(0..1, 0.0, 160.0).unwrap(),
            |checkpoint| ViewportLayoutRegion::resume_long_line(checkpoint, 0.0, 160.0).unwrap(),
        );
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(job),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        // Discovering an unusually deep leading prefix must not copy the
        // entire hard line into each ordinary 64KiB shaping slice.
        assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 8192);
        let result =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let work = result.regional_snapshot().work_statistics();
        // Only the first slice measures the complete original indentation;
        // checkpoints retain that result for every subsequent slice.
        let maximum_segmented =
            MAX_LONG_LINE_LAYOUT_SLICE_BYTES + if job == 1 { tab_count } else { 0 };
        assert!(work.segmented_text_bytes() <= maximum_segmented);
        assert!(work.maximum_shaping_fragment_bytes() <= 4096);
        let line = &result.regional_snapshot().lines()[0];
        assert!(!line.rows().is_empty());
        if job == 1 {
            // Consecutive tabs are an unbreakable Unicode run. Its first row
            // overflows, but only viewport-relevant clusters are retained.
            assert!(line.rows()[0].clusters.len() < 4096);
        }
        for row in line.rows() {
            let expected = &complete.rows[row.fragment_index];
            assert_eq!(row.text_range, expected.text_range);
            assert_eq!(row.paragraph_content_x, expected.paragraph_content_x);
            assert_eq!(
                row.paragraph_content_width,
                expected.paragraph_content_width
            );
            assert_eq!(row.y, expected.y);
            for cluster in &row.clusters {
                let index = expected.clusters.partition_point(|candidate| {
                    candidate.text_range.start < cluster.text_range.start
                });
                let matching = expected
                    .clusters
                    .get(index)
                    .expect("each materialized cluster belongs to the complete row");
                assert_eq!(cluster, matching);
            }
        }
        completed_rows += line.rows().len();
        checkpoint = line.next_checkpoint().cloned();
        jobs = job;
        if checkpoint.is_none() {
            assert!(line.height_is_exact());
            break;
        }
    }
    assert!(checkpoint.is_none());
    assert!(jobs > 1);
    assert_eq!(completed_rows, complete.rows.len());
}
