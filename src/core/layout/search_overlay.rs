//! Extend sparse search paint to indivisible shaping clusters. Logical search
//! ranges remain independent of the platform's available caret geometry.

use super::{PaintStyleRun, ResolvedTextPaint};
use crate::document::{CharacterProperties, Color};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SearchPaintOverlay {
    ranges: Vec<Range<usize>>,
    foreground: Option<Color>,
    background: Option<Color>,
    underline: Option<bool>,
    strikethrough: Option<bool>,
}

impl SearchPaintOverlay {
    /// Rebase presentation ranges into a disposable composition snapshot using
    /// the same mapping as shaping and paint runs.
    pub(crate) fn remap_ranges(
        &mut self,
        mut map: impl FnMut(Range<usize>) -> Option<Range<usize>>,
    ) {
        self.ranges = self
            .ranges
            .drain(..)
            .filter_map(&mut map)
            .filter(|range| !range.is_empty())
            .collect();
        coalesce_ranges(&mut self.ranges);
    }

    pub(super) fn new(
        matches: &[Range<usize>],
        region: Range<usize>,
        properties: &CharacterProperties,
    ) -> Option<Self> {
        if properties.foreground.is_none()
            && properties.background.is_none()
            && properties.underline.is_none()
            && properties.strikethrough.is_none()
        {
            return None;
        }
        let mut ranges: Vec<_> = matches
            .iter()
            .map(|range| range.start.max(region.start)..range.end.min(region.end))
            .filter(|range| !range.is_empty())
            .collect();
        coalesce_ranges(&mut ranges);
        (!ranges.is_empty()).then_some(Self {
            ranges,
            foreground: properties.foreground,
            background: properties.background,
            underline: properties.underline,
            strikethrough: properties.strikethrough,
        })
    }

    fn apply(&self, paint: &mut ResolvedTextPaint) {
        if let Some(foreground) = self.foreground {
            paint.foreground = foreground;
            paint.foreground_is_default = false;
        }
        if let Some(background) = self.background {
            paint.background = Some(background);
        }
        if let Some(underline) = self.underline {
            paint.underline = underline;
        }
        if let Some(strikethrough) = self.strikethrough {
            paint.strikethrough = strikethrough;
        }
    }
}

/// A native renderer samples paint at a cluster's start. Apply only the search
/// style's declared paint properties across each intersecting cluster, keeping
/// every unrelated authored/syntax paint value at its existing text range.
pub(super) fn normalize_search_paint<'a>(
    paint_runs: &mut Vec<PaintStyleRun>,
    default_paint: &ResolvedTextPaint,
    clusters: impl Iterator<Item = &'a Range<usize>>,
    overlay: Option<&SearchPaintOverlay>,
) {
    let Some(overlay) = overlay else { return };
    let mut coverage = Vec::new();
    for cluster in clusters {
        let index = overlay
            .ranges
            .partition_point(|range| range.end <= cluster.start);
        if !cluster.is_empty()
            && overlay
                .ranges
                .get(index)
                .is_some_and(|range| range.start < cluster.end)
        {
            coverage.push(cluster.clone());
        }
    }
    if coverage.is_empty() {
        return;
    }
    // Cluster order is visual, so bidi text may contribute ranges in reverse
    // logical order or interleave them with another run on the same row.
    coalesce_ranges(&mut coverage);
    let mut boundaries = Vec::with_capacity(2 * (paint_runs.len() + coverage.len()));
    for range in paint_runs
        .iter()
        .map(|run| &run.text_range)
        .chain(&coverage)
    {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut output: Vec<PaintStyleRun> = Vec::new();
    let mut paint_index = 0;
    let mut coverage_index = 0;
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        while paint_index < paint_runs.len()
            && paint_runs[paint_index].text_range.end <= range.start
        {
            paint_index += 1;
        }
        while coverage_index < coverage.len() && coverage[coverage_index].end <= range.start {
            coverage_index += 1;
        }
        let mut paint = paint_runs
            .get(paint_index)
            .filter(|run| run.text_range.start <= range.start)
            .map_or(default_paint, |run| &run.paint)
            .clone();
        if coverage
            .get(coverage_index)
            .is_some_and(|covered| covered.start <= range.start)
        {
            overlay.apply(&mut paint);
        }
        if paint == *default_paint {
            continue;
        }
        if let Some(previous) = output
            .last_mut()
            .filter(|previous| previous.text_range.end == range.start && previous.paint == paint)
        {
            previous.text_range.end = range.end;
        } else {
            output.push(PaintStyleRun {
                text_range: range,
                paint,
            });
        }
    }
    *paint_runs = output;
}

fn coalesce_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut count = 0;
    for index in 0..ranges.len() {
        if count > 0 && ranges[index].start <= ranges[count - 1].end {
            ranges[count - 1].end = ranges[count - 1].end.max(ranges[index].end);
        } else {
            ranges.swap(count, index);
            count += 1;
        }
    }
    ranges.truncate(count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, StyleId};
    use crate::layout::{
        compute_layout_job, inspect_layout_provider, install_layout_job, prepare_layout_job,
        LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutInstallTarget,
        LayoutJobId, LayoutJobInstallRejection, LayoutJobPriority, LayoutJobRegion,
        MockTextMeasurementProvider, ViewLayout, ViewportLayoutRegion,
    };

    const YELLOW: Color = Color {
        red: 1.0,
        green: 1.0,
        blue: 0.0,
        alpha: 0.4,
    };
    const RED: Color = Color {
        red: 0.9,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    };

    #[test]
    fn match_inside_ligature_extends_only_declared_paint() {
        let default = ResolvedTextPaint::default();
        let authored = ResolvedTextPaint {
            foreground: RED,
            foreground_is_default: false,
            underline: true,
            ..default.clone()
        };
        let mut runs = vec![
            PaintStyleRun {
                text_range: 0..2,
                paint: authored.clone(),
            },
            PaintStyleRun {
                text_range: 2..3,
                paint: ResolvedTextPaint {
                    background: Some(YELLOW),
                    ..authored.clone()
                },
            },
        ];
        let matches = vec![2..3];
        let overlay = SearchPaintOverlay::new(
            &matches,
            0..6,
            &CharacterProperties {
                background: Some(YELLOW),
                ..Default::default()
            },
        );
        normalize_search_paint(
            &mut runs,
            &default,
            [0..3, 3..4, 4..5, 5..6].iter(),
            overlay.as_ref(),
        );
        assert_eq!(
            runs,
            vec![PaintStyleRun {
                text_range: 0..3,
                paint: ResolvedTextPaint {
                    background: Some(YELLOW),
                    ..authored
                }
            }]
        );
        assert_eq!(matches, vec![2..3]);
    }

    #[test]
    fn sparse_overlay_preserves_differing_base_paints_and_handles_visual_order() {
        let default = ResolvedTextPaint::default();
        let authored = ResolvedTextPaint {
            foreground: RED,
            foreground_is_default: false,
            underline: true,
            ..default.clone()
        };
        let mut runs = vec![PaintStyleRun {
            text_range: 1..2,
            paint: authored.clone(),
        }];
        let overlay = SearchPaintOverlay::new(
            &[1..2, 5..6],
            0..8,
            &CharacterProperties {
                background: Some(YELLOW),
                underline: Some(false),
                ..Default::default()
            },
        );
        normalize_search_paint(
            &mut runs,
            &default,
            [4..7, 2..4, 0..2].iter(),
            overlay.as_ref(),
        );
        let paint_at = |offset| {
            runs.iter()
                .find(|run| run.text_range.contains(&offset))
                .map_or(&default, |run| &run.paint)
        };
        assert_eq!(paint_at(0).background, Some(YELLOW));
        assert_eq!(paint_at(1).foreground, RED);
        assert!(!paint_at(1).underline);
        assert_eq!(paint_at(2), &default);
        assert_eq!(paint_at(4).background, Some(YELLOW));
        assert_eq!(paint_at(7), &default);
    }

    #[test]
    fn geometry_only_or_empty_search_adds_no_paint_metadata() {
        assert!(SearchPaintOverlay::new(
            &[1..2],
            0..3,
            &CharacterProperties {
                size: Some(20.0),
                ..Default::default()
            }
        )
        .is_none());
        assert!(SearchPaintOverlay::new(
            &[1..1, 8..9],
            0..3,
            &CharacterProperties {
                background: Some(YELLOW),
                ..Default::default()
            }
        )
        .is_none());
    }

    #[test]
    fn composition_remapping_retains_sparse_paint_at_the_new_cluster() {
        let default = ResolvedTextPaint::default();
        let mut overlay = SearchPaintOverlay::new(
            &[1..2, 5..6],
            0..8,
            &CharacterProperties {
                background: Some(YELLOW),
                ..Default::default()
            },
        )
        .unwrap();
        overlay.remap_ranges(|range| (range.start >= 5).then(|| range.start - 3..range.end - 3));
        let mut runs = Vec::new();
        normalize_search_paint(&mut runs, &default, [0..1, 1..4].iter(), Some(&overlay));
        assert_eq!(
            runs,
            vec![PaintStyleRun {
                text_range: 1..4,
                paint: ResolvedTextPaint {
                    background: Some(YELLOW),
                    ..default
                }
            }]
        );
    }

    #[test]
    fn full_and_regional_layout_paint_the_ligature_containing_a_partial_match() {
        let document = Document::new("office other");
        let expected = document
            .projection()
            .style_sheet()
            .character_style(&StyleId::incremental_match())
            .unwrap()
            .properties
            .background;
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(300.0, 100.0);
        view.set_search_matches(document.id(), document.revision(), vec![2..3], false);
        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        let cluster = snapshot.rows[0]
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.contains(&2))
            .unwrap();
        assert_eq!(cluster.text_range, 1..4);
        assert_eq!(
            snapshot
                .paint_runs
                .iter()
                .find(|run| run.text_range.contains(&1))
                .unwrap()
                .paint
                .background,
            expected
        );
        for (job, wrap) in [(1, true), (2, false)] {
            view.set_wrap(wrap);
            let requirements = inspect_layout_provider(&engine);
            let request = prepare_layout_job(
                &document,
                &mut view,
                requirements,
                LayoutJobId(job),
                LayoutJobPriority::ChangedVisibleRows,
                LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 100.0).unwrap()),
                LayoutCancellationToken::new(),
            )
            .unwrap();
            let candidate =
                compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool)
                    .unwrap();
            let snapshot = candidate.regional_snapshot();
            let cluster = snapshot.lines()[0].rows()[0]
                .clusters
                .iter()
                .find(|cluster| cluster.text_range.contains(&2))
                .unwrap();
            assert_eq!(cluster.text_range, 1..4);
            assert_eq!(
                snapshot
                    .paint_runs()
                    .iter()
                    .find(|run| run.text_range.contains(&1))
                    .unwrap()
                    .paint
                    .background,
                expected
            );
        }
        assert_eq!(
            view.search_matches(document.id(), document.revision()),
            &[2..3]
        );
    }

    #[test]
    fn changing_search_paint_reuses_shaping_and_removes_old_coverage() {
        let document = Document::new("office other");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(300.0, 100.0);
        engine.relayout(&document, &mut view).unwrap();
        let baseline = view.snapshot().unwrap().rows.clone();
        let calls = engine.provider().request_calls();
        for ranges in [vec![2..3], vec![8..9], vec![]] {
            let previous_generation = view.configuration_generation();
            assert!(view.set_search_matches(
                document.id(),
                document.revision(),
                ranges.clone(),
                false
            ));
            assert_ne!(view.configuration_generation(), previous_generation);
            assert!(!view.set_search_matches(document.id(), document.revision(), ranges, false));
            engine.relayout(&document, &mut view).unwrap();
            assert_eq!(engine.provider().request_calls(), calls);
            assert_eq!(
                view.snapshot().unwrap().rows[0].clusters,
                baseline[0].clusters
            );
        }
        assert!(view.snapshot().unwrap().paint_runs.is_empty());
    }

    #[test]
    fn dense_single_line_matches_resolve_without_changing_shaping() {
        const MATCHES: usize = 16_384;
        let text = format!("before\n{}\nafter", "a ".repeat(MATCHES));
        let document = Document::new(&text);
        let start = "before\n".len();
        let end = start + MATCHES * 2;
        let mut ranges = vec![0..6];
        ranges.extend((0..MATCHES).map(|index| start + index * 2..start + index * 2 + 1));
        ranges.push(end + 1..text.len());
        let styles = crate::layout::DocumentLayoutStyles::resolve_region_with_search(
            document.projection(),
            start..end,
            false,
            &ranges,
        )
        .unwrap();
        assert_eq!(styles.paint_runs.len(), MATCHES);
        assert!(styles.shaping_runs.is_empty());
        for (index, run) in styles.paint_runs.iter().enumerate() {
            assert_eq!(run.text_range, start + index * 2..start + index * 2 + 1);
            assert!(run.paint.background.is_some());
        }
        let cleared = crate::layout::DocumentLayoutStyles::resolve_region_with_search(
            document.projection(),
            start..end,
            false,
            &[],
        )
        .unwrap();
        assert!(cleared.paint_runs.is_empty());
        assert_eq!(styles.shaping_runs, cleared.shaping_runs);
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn search_style_change_retires_jobs_even_when_coverage_is_unchanged() {
        let document = Document::new("office other");
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(300.0, 100.0);
        view.set_search_matches(document.id(), document.revision(), vec![2..3], false);
        engine.relayout(&document, &mut view).unwrap();
        assert!(view.content_height().is_exact());
        let requirements = inspect_layout_provider(&engine);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0.0, 100.0).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        view.invalidate_search_style(false);
        assert!(view.content_height().is_exact());
        assert_eq!(
            view.search_matches(document.id(), document.revision()),
            &[2..3]
        );
        assert!(matches!(
            install_layout_job(
                &mut view,
                LayoutInstallTarget {
                    document_id: document.id(),
                    document_revision: document.revision(),
                    measurement_environment_id: requirements.measurement_environment_id,
                    metrics_generation: requirements.metrics_generation,
                },
                candidate
            ),
            Err(LayoutJobInstallRejection::StaleConfiguration { .. })
        ));
        view.invalidate_search_style(true);
        assert!(!view.content_height().is_exact());
    }

    #[test]
    fn large_document_overlay_stays_regional_and_rejects_stale_search_jobs() {
        let line = "office other\n";
        let document = Document::new(line.repeat(50_000));
        let origin = line.len() * 25_000;
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(300.0, 100.0);
        view.set_search_matches(
            document.id(),
            document.revision(),
            vec![2..3, origin + 2..origin + 3],
            false,
        );
        let requirements = inspect_layout_provider(&engine);
        let request = prepare_layout_job(
            &document,
            &mut view,
            requirements,
            LayoutJobId(1),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(
                ViewportLayoutRegion::new(25_000..25_003, 0.0, 100.0).unwrap(),
            ),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert!(request.captured_text_len() <= line.len() * 4);
        assert!(request.capture_statistics().document_paint_style_runs() <= 2);
        assert!(!document.projection().compatibility_text_is_materialized());
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let region = candidate.regional_snapshot();
        assert!(region.work_statistics().positioned_cluster_count() < 50);
        assert_eq!(region.paint_runs().len(), 1);
        assert_eq!(region.paint_runs()[0].text_range, origin + 1..origin + 4);
        view.set_search_matches(document.id(), document.revision(), vec![], false);
        assert!(matches!(
            install_layout_job(
                &mut view,
                LayoutInstallTarget {
                    document_id: document.id(),
                    document_revision: document.revision(),
                    measurement_environment_id: requirements.measurement_environment_id,
                    metrics_generation: requirements.metrics_generation,
                },
                candidate
            ),
            Err(LayoutJobInstallRejection::StaleConfiguration { .. })
        ));
        assert!(!document.projection().compatibility_text_is_materialized());
    }
}
