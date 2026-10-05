//! Cross-path checks for the document extent consumed by scrolling.

use super::*;
use crate::layout::{MockTextMeasurementProvider, MAX_LONG_LINE_LAYOUT_SLICE_BYTES};

#[derive(Default)]
struct OverhangingInkProvider(MockTextMeasurementProvider);

impl TextMeasurementProvider for OverhangingInkProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.0.measurement_environment_id()
    }

    fn metrics_generation(&self) -> MetricsGeneration {
        self.0.metrics_generation()
    }

    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.0.render_run_policy()
    }

    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        let mut fragments = self.0.shape_batch(requests)?;
        for fragment in &mut fragments {
            for cluster in &mut fragment.clusters {
                cluster.ink_bounds.height += 13.0;
            }
        }
        Ok(fragments)
    }
}

#[test]
fn terminal_document_extent_matches_full_regional_and_streamed_long_rows() {
    let document = Document::new(format!(
        "first\n{}", "x".repeat(MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 1),
    ));
    let text = document.text();
    let ranges = hard_line_ranges(text, 0);
    let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    styles.document_insets = EdgeInsets { top: 7.0, bottom: 11.0, ..Default::default() };
    styles.paragraphs[0].margin_top = 3.0;
    styles.paragraphs[0].margin_bottom = 5.0;
    styles.paragraphs[1].margin_top = 2.0;
    styles.paragraphs[1].margin_bottom = 17.0;

    // A short exact advance must retain overflowing ink; a long one must
    // retain its blank leading. Both must add trailing spacing and padding once.
    for advance in [8.0, 48.0] {
        styles.paragraphs[1].line_spacing = LineSpacing::Exact(advance);
        let mut engine = LayoutEngine::new(OverhangingInkProvider::default());
        let mut view = ViewLayout::new(400.0, 80.0);
        view.set_wrap(false);
        for margin in [28.0, 72.0] {
            let previous_configuration = view.configuration_generation();
            view.set_insets(EdgeInsets { top: 13.0, bottom: margin, ..Default::default() });
            assert_ne!(previous_configuration, view.configuration_generation());
            engine.relayout_styled_text(
                document.id(), document.revision(), text, styles.clone(), &mut view,
            ).unwrap();
            let full = view.snapshot().unwrap();
            let last = full.rows.last().unwrap();
            let ink_bottom = last.ink_bounds().unwrap().y + last.ink_bounds().unwrap().height;
            assert!(ink_bottom > last.y + last.natural_height());
            assert_eq!(last.line_advance, advance);
            let trailing_space = full.total_height - (last.y + advance).max(ink_bottom);
            assert!((trailing_space - (17.0 + 11.0 + margin)).abs() < 0.001);
            let expected = full.total_height;
            let captured = view.capture_for_regional_layout_job(0..text.len());
            let regional = engine.layout_hard_line_region_cancellable(
                document.id(), document.revision(), text, 0, &ranges, 0,
                ranges.len(), text.len(), None, &styles, &captured, &NeverCancelled,
            ).unwrap();
            let streamed = engine.layout_unwrapped_viewport_cancellable(
                document.id(), document.revision(), document.projection().text_tree(),
                &ranges, 0, ranges.len(), None, &styles, &captured, &NeverCancelled,
            ).unwrap();
            assert!(streamed.horizontal_materialization.is_some(),
                "the giant final row must exercise sparse unwrapped layout");
            assert!(streamed.lines.last().unwrap().rows[0].clusters.len() < 1_000,
                "matching the scroll extent must not retain the entire giant row");
            for (path, region) in [("regional", regional), ("streamed", streamed)] {
                assert!(region.lines.iter().all(|line| line.height_is_exact));
                let height = region.lines.iter().map(|line| line.height).sum::<f64>() as f32;
                assert!((height - expected).abs() < 0.001,
                    "{path} extent {height} differs from full extent {expected}; advance={advance}, margin={margin}");
            }
        }
    }
}

#[test]
fn padded_block_decorations_only_scroll_when_the_box_overflows() {
    let document = Document::new("short");
    let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    styles.paragraphs[0].block_box = block_box::BlockBoxStyle {
        padding: EdgeInsets { left: 12.0, right: 8.0, ..Default::default() },
        border: EdgeInsets { right: 2.0, ..Default::default() },
        background: Some(Color { red: 0.2, green: 0.3, blue: 0.4, alpha: 1.0 }),
        ..Default::default()
    };
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(400.0, 100.0);
    view.set_insets(EdgeInsets { left: 24.0, right: 24.0, ..Default::default() });
    for width in [400.0, 320.0] {
        view.resize(width, 100.0);
        engine.relayout_styled_text(
            document.id(), document.revision(), document.text(), styles.clone(), &mut view,
        ).unwrap();
        assert_eq!(view.maximum_viewport_left(), Some(0.0),
            "fitting backgrounds and borders already include the block's padding");
    }

    // Keep genuine decoration overflow reachable even when the text fits.
    styles.paragraphs[0].block_box.margin.right = -100.0;
    engine.relayout_styled_text(
        document.id(), document.revision(), document.text(), styles, &mut view,
    ).unwrap();
    assert_eq!(view.maximum_viewport_left(), Some(76.0));
    view.set_viewport_left(f32::MAX).unwrap();
    assert_eq!(view.viewport_left(), 76.0);
}
