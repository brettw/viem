//! Source-line gutter geometry derived from an immutable local layout.
use super::*;
use crate::layout::LayoutRect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineNumberRegion {
    /// One-based physical source ordinal, independent of command line mode.
    pub source_line: usize,
    pub top: f32,
    pub bottom: f32,
    /// A huge partially materialized wrapped line uses its indexed estimate.
    pub estimated: bool,
    /// This band owns the installed presentation row containing the caret.
    /// The displayed ordinal can differ from the caret's physical source line.
    pub current: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;

    #[test]
    fn finite_long_line_scroll_reuses_checkpoints_and_keeps_failed_work_atomic() {
        let source = "word ".repeat(30_000) + "\n" + &"word ".repeat(30_000);
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(f32::MAX),
            },
        )
        .unwrap();
        let top = core
            .layout(id)
            .unwrap()
            .hard_line_range_height(0..1)
            .unwrap()
            .height() as f32
            * 0.4;
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(0.),
            },
        )
        .unwrap();
        let anchor = core.views[&id].viewport_anchor;
        core.views
            .get_mut(&id)
            .unwrap()
            .layout
            .set_regional_cache_limits(crate::layout::RegionalLayoutCacheLimits {
                max_hard_lines: 0,
                ..Default::default()
            });
        let before = core.layout(id).unwrap().clone();
        core.views.get_mut(&id).unwrap().engine.clear_caches();
        core.views
            .get_mut(&id)
            .unwrap()
            .engine
            .provider_mut()
            .fail_next_batches(2, "finite scroll failed");
        let request = CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(top),
        };
        assert!(core.handle(id, request.clone()).is_err());
        assert_eq!(core.layout(id).unwrap(), &before);
        assert_eq!(core.views[&id].viewport_anchor, anchor);
        assert_eq!(core.command_state(id).unwrap().cursor(), 0);
        assert_eq!(core.document.source_bytes(), source.as_bytes());
        core.handle(id, request.clone()).unwrap();
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        assert!(
            snapshot.rows.first().unwrap().text_range.start
                >= MAX_LONG_LINE_LAYOUT_SLICE_BYTES - 128
        );
        let coverage = snapshot.coverage.vertical_range().unwrap();
        assert!(coverage.start <= top && top + 100. <= coverage.end);
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(0.),
            },
        )
        .unwrap();
        let jobs = core.next_layout_job.unwrap();
        core.handle(id, request).unwrap();
        assert!(
            core.next_layout_job.unwrap() - jobs <= 2,
            "a warm finite scroll resumes the nearest validated checkpoint"
        );
    }

    fn install_number_chunk(
        core: &mut Core<MockTextMeasurementProvider>,
        id: ViewId,
        checkpoint: Option<crate::layout::LongLineLayoutCheckpoint>,
    ) -> crate::layout::LongLineLayoutCheckpoint {
        use crate::layout::{
            install_layout_job, LayoutInstallTarget, ViewportLayoutRegion,
            MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
        };
        let job = core.allocate_layout_job_id().unwrap();
        let view = core.views.get_mut(&id).unwrap();
        let requirements = inspect_layout_provider(&view.engine);
        let top = checkpoint
            .as_ref()
            .map_or(0., |point| point.completed_height());
        let region = checkpoint.map_or_else(
            || ViewportLayoutRegion::new(0..1, top, 100.).unwrap(),
            |point| ViewportLayoutRegion::resume_long_line(point, top, 100.).unwrap(),
        );
        let request = prepare_layout_job(
            &core.document,
            &mut view.layout,
            requirements,
            job,
            LayoutJobPriority::NewlyExposedRows,
            LayoutJobRegion::Viewport(region),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 4096);
        let candidate =
            compute_layout_job(&mut view.engine, &request, view.immediate_layout_context).unwrap();
        let next = candidate.next_long_line_checkpoint().unwrap().clone();
        install_layout_job(
            &mut view.layout,
            LayoutInstallTarget {
                document_id: core.document.id(),
                document_revision: core.document.revision(),
                measurement_environment_id: requirements.measurement_environment_id,
                metrics_generation: requirements.metrics_generation,
            },
            candidate,
        )
        .unwrap();
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(top),
            },
        )
        .unwrap();
        next
    }

    #[test]
    fn folded_wraps_give_each_shared_row_to_its_first_physical_line() {
        let source = "first ".repeat(11) + "\n" + &"second ".repeat(12) + "\nthird";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut defaults: serde_json::Value =
            serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        let paragraph = defaults["block_styles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|style| style["id"] == "Paragraph")
            .unwrap();
        paragraph["character"]["size"] = 12.into();
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 160., 700.);
        core.set_number(id, true).unwrap();
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        let transition = snapshot
            .rows
            .iter()
            .find(|row| {
                let contributors = core
                    .document
                    .physical_line_indexes_in_text(row.text_range.clone())
                    .unwrap();
                contributors.contains(&0) && contributors.contains(&1)
            })
            .expect("the two wrapped physical lines must share a transition row");
        let bands = core.line_number_regions(id).unwrap();
        let first = bands.iter().find(|band| band.source_line == 1).unwrap();
        let second = bands.iter().find(|band| band.source_line == 2).unwrap();
        assert_eq!(first.top, snapshot.rows[0].y);
        assert_eq!(first.bottom, transition.y + transition.height());
        assert_eq!(second.top, first.bottom);
        assert_eq!(
            second.bottom,
            snapshot.rows.last().unwrap().y + snapshot.rows.last().unwrap().height()
        );
        assert!(bands.iter().all(|band| band.source_line <= 2));
        assert!(bands.iter().all(|band| !band.estimated));
        let transition_top = transition.y;
        let transition_height = transition.height();
        let next_row_offset = snapshot
            .rows
            .iter()
            .find(|row| row.y >= transition.y + transition.height())
            .unwrap()
            .text_range
            .start;
        let source_start = source.find('\n').unwrap() + 1;
        let transition_cursor = core
            .document
            .visible_point_for_source(source_start, true)
            .unwrap();
        for (cursor, representative) in [(transition_cursor, 1), (next_row_offset, 2)] {
            core.handle(
                id,
                CoreEvent::PlaceCursor {
                    document_revision: core.document.revision(),
                    text_offset: cursor,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            assert_eq!(core.current_physical_source_line(id).unwrap(), 2);
            assert_eq!(
                core.line_number_regions(id)
                    .unwrap()
                    .iter()
                    .filter(|band| band.current)
                    .map(|band| band.source_line)
                    .collect::<Vec<_>>(),
                vec![representative],
                "current styling follows the cursor row's displayed representative"
            );
        }
        let before = core.line_number_regions(id).unwrap();
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(transition_top + transition_height * 0.25),
            },
        )
        .unwrap();
        assert_eq!(core.line_number_regions(id).unwrap(), before);
        assert_eq!(core.document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn partial_folded_end_row_has_one_full_number_band_and_no_speculative_current() {
        let source = "abcdef ".repeat(30_000) + "\nb\nc";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        core.set_number(id, true).unwrap();
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(f32::MAX),
            },
        )
        .unwrap();
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.rows[0].text_range.start > 0);
        let row = snapshot.rows.last().unwrap();
        assert_eq!(
            core.document
                .physical_line_indexes_in_text(row.text_range.clone())
                .unwrap(),
            vec![0, 1, 2],
            "the final partial row contains all three physical source lines"
        );
        let calls = core.views[&id].engine.provider().request_calls();
        let bands = core.line_number_regions(id).unwrap();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].source_line, 1);
        assert_eq!(bands[0].bottom, row.y + row.height());
        assert!(bands[0].estimated);
        assert!(!bands[0].current, "the caret's offscreen row is not installed");
        assert_eq!(core.views[&id].engine.provider().request_calls(), calls);
        assert_eq!(core.document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn horizontally_sliced_folded_line_retains_its_first_ordinal_and_current_row() {
        let source = "abcdef ".repeat(12_000) + "\n" + &"second ".repeat(12_000);
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        core.set_number(id, true).unwrap();
        core.handle(id, CoreEvent::SetWrap(false)).unwrap();
        let cursor = core
            .document
            .visible_point_for_source(source.find('\n').unwrap() + 20_000, true)
            .unwrap();
        core.handle(
            id,
            CoreEvent::PlaceCursor {
                document_revision: core.document.revision(),
                text_offset: cursor,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        assert!(snapshot.has_horizontal_materialization());
        assert!(snapshot.rows.iter().all(|row| row.clusters.len() < 6_000));
        assert_eq!(core.current_physical_source_line(id).unwrap(), 2);
        let calls = core.views[&id].engine.provider().request_calls();
        let text_was_materialized = core
            .document
            .projection()
            .compatibility_text_is_materialized();
        let bands = core.line_number_regions(id).unwrap();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].source_line, 1);
        assert_eq!(bands[0].top, snapshot.rows[0].y);
        assert_eq!(bands[0].bottom, snapshot.rows[0].y + snapshot.rows[0].height());
        assert!(bands[0].current);
        assert!(!bands[0].estimated);
        assert_eq!(core.views[&id].engine.provider().request_calls(), calls);
        assert_eq!(
            core.document.projection().compatibility_text_is_materialized(),
            text_was_materialized,
            "a gutter query preserves the materialization state of the preceding cursor path"
        );
        assert_eq!(core.document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn folded_physical_line_keeps_its_whole_estimated_band_across_disjoint_chunks() {
        let source = "abcdef ".repeat(30_000) + "\ncontinuation words\nlast words";
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400., 100.);
        core.set_number(id, true).unwrap();
        let checkpoint = install_number_chunk(&mut core, id, None);
        let first_end = core
            .layout(id)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .last()
            .unwrap()
            .text_range
            .end;
        let initial = core.line_number_regions(id).unwrap()[0];
        let local_end = core
            .layout(id)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .last()
            .unwrap()
            .y
            + core
                .layout(id)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows
                .last()
                .unwrap()
                .height();
        assert!(initial.estimated && initial.bottom > local_end * 2.);
        let _ = install_number_chunk(&mut core, id, Some(checkpoint));
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        assert!(snapshot.rows.first().unwrap().text_range.start >= first_end);
        let calls = core.views[&id].engine.provider().request_calls();
        let text_was_materialized = core
            .document
            .projection()
            .compatibility_text_is_materialized();
        let later = core.line_number_regions(id).unwrap()[0];
        let advance = snapshot.rows[0].height();
        assert_eq!(later.source_line, 1);
        assert_eq!(later.top, initial.top);
        assert!(
            (later.bottom - initial.bottom).abs() <= advance * 2.,
            "{initial:?} -> {later:?}"
        );
        assert!(later.estimated);
        assert_eq!(core.views[&id].engine.provider().request_calls(), calls);
        assert_eq!(
            core.document
                .projection()
                .compatibility_text_is_materialized(),
            text_was_materialized
        );
        assert_eq!(core.document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn partial_number_bands_exclude_paragraph_and_canvas_flow_spacing() {
        let mut document = Document::new("abcdef ".repeat(30_000));
        let mut defaults: serde_json::Value =
            serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        let paragraph = defaults["block_styles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|style| style["id"] == "Paragraph")
            .unwrap();
        paragraph["block"]["margin_top"] = 31.into();
        paragraph["block"]["padding_top"] = 17.into();
        paragraph["block"]["border_top_width"] = 5.into();
        paragraph["block"]["margin_bottom"] = 43.into();
        paragraph["block"]["padding_bottom"] = 19.into();
        paragraph["block"]["border_bottom_width"] = 7.into();
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        core.set_view_insets(
            id,
            crate::layout::EdgeInsets {
                top: 29.,
                bottom: 37.,
                ..Default::default()
            },
        )
        .unwrap();
        core.set_number(id, true).unwrap();
        let checkpoint = install_number_chunk(&mut core, id, None);
        let first = core.line_number_regions(id).unwrap()[0];
        let first_row = &core.layout(id).unwrap().snapshot().unwrap().rows[0];
        assert!(first_row.y >= 82.);
        assert_eq!(first.top, first_row.y);
        let _ = install_number_chunk(&mut core, id, Some(checkpoint));
        let second = core.line_number_regions(id).unwrap()[0];
        assert_eq!(
            second.top, first.top,
            "the indexed band origin must exclude flow space"
        );
        assert!(
            (second.bottom - first.bottom).abs()
                <= core.layout(id).unwrap().snapshot().unwrap().rows[0].height() * 2.
        );
        assert!(first.estimated && second.estimated);
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(f32::MAX),
            },
        )
        .unwrap();
        let end_band = core.line_number_regions(id).unwrap()[0];
        let last = core
            .layout(id)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .last()
            .unwrap();
        assert_eq!(
            end_band.bottom,
            last.y + last.height(),
            "trailing border, padding, margin and canvas padding are outside the text band"
        );
        core.handle(
            id,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(0.),
            },
        )
        .unwrap();
        let start_band = core.line_number_regions(id).unwrap()[0];
        assert_eq!(start_band.top, end_band.top);
        assert!(
            (start_band.bottom - end_band.bottom).abs() < 0.02,
            "a measured trailing flow inset survives a later middle-only snapshot"
        );
    }

    #[test]
    fn million_line_gutter_queries_reuse_local_layout_without_decoding_or_shaping() {
        let mut core = Core::new(Document::new("small row\n".repeat(1_000_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 180., 100.);
        core.set_number(view, true).unwrap();
        let requests = core.views[&view].engine.provider().request_calls();
        let revision = core.views[&view].layout.snapshot().unwrap().revision;
        for _ in 0..10 {
            assert!(core.line_number_regions(view).unwrap().len() < 32);
            assert_eq!(core.current_physical_source_line(view).unwrap(), 1);
        }
        assert_eq!(
            core.views[&view].engine.provider().request_calls(),
            requests
        );
        assert_eq!(
            core.views[&view].layout.snapshot().unwrap().revision,
            revision
        );
        assert!(!core
            .document
            .projection()
            .compatibility_text_is_materialized());
        core.views
            .get_mut(&view)
            .unwrap()
            .engine
            .provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        assert!(
            core.line_number_regions(view).is_err(),
            "stale metrics cannot become gutter geometry"
        );
        core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport)
            .unwrap();
        assert_ne!(
            core.views[&view].layout.snapshot().unwrap().revision,
            revision
        );
        assert!(core.line_number_regions(view).unwrap().len() < 32);

        let other = core.add_view(MockTextMeasurementProvider::new(), 180., 100.);
        core.set_number(other, true).unwrap();
        let other_requests = core.views[&other].engine.provider().request_calls();
        let other_revision = core.views[&other].layout.snapshot().unwrap().revision;
        core.remove_view(view).unwrap();
        assert!(matches!(core.number(view), Err(CoreError::UnknownView(id)) if id == view));
        assert!(
            matches!(core.line_number_regions(view), Err(CoreError::UnknownView(id)) if id == view)
        );
        assert!(core.line_number_regions(other).unwrap().len() < 32);
        assert_eq!(
            core.views[&other].engine.provider().request_calls(),
            other_requests
        );
        assert_eq!(
            core.views[&other].layout.snapshot().unwrap().revision,
            other_revision
        );
        let replacement = core.add_view(MockTextMeasurementProvider::new(), 180., 100.);
        assert_ne!(
            replacement, view,
            "disposed view identities cannot be reused"
        );
        assert!(!core.number(replacement).unwrap());
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn number(&self, id: ViewId) -> Result<bool, CoreError> {
        Ok(self
            .views
            .get(&id)
            .ok_or(CoreError::UnknownView(id))?
            .commands
            .number())
    }
    /// A presentation preference; native gutter sizing owns any width change.
    pub fn set_number(&mut self, id: ViewId, enabled: bool) -> Result<(), CoreError> {
        self.views
            .get_mut(&id)
            .ok_or(CoreError::UnknownView(id))?
            .commands
            .set_number(enabled);
        Ok(())
    }
    pub fn current_physical_source_line(&self, id: ViewId) -> Result<usize, CoreError> {
        let view = self.views.get(&id).ok_or(CoreError::UnknownView(id))?;
        let index = if view.composition_layout.is_some() {
            let overlay = self.composition_overlay(id)?.ok_or(LayoutError::NoRows)?;
            self.document.physical_line_indexes_in_text(
                overlay.replacement_range().start..overlay.replacement_range().start,
            )?[0]
        } else {
            view.commands.physical_display_source_line_index(&self.document)?
        };
        Ok((index + 1).min(self.document.physical_line_count()?))
    }
    /// No text decoding or layout is performed. Only installed regional rows
    /// and logarithmic provenance/source-line indexes are consulted.
    pub fn line_number_regions(&self, id: ViewId) -> Result<Vec<LineNumberRegion>, CoreError> {
        let view = self.views.get(&id).ok_or(CoreError::UnknownView(id))?;
        if !view.commands.number() {
            return Ok(Vec::new());
        }
        let layout = view.composition_layout.as_ref().unwrap_or(&view.layout);
        let overlay = if view.composition_layout.is_some() {
            self.composition_overlay(id)?
        } else {
            None
        };
        let snapshot = current_snapshot_for_layout(
            &self.document,
            layout,
            inspect_layout_provider(&view.engine),
        )
        .ok_or(LayoutError::NoRows)?;
        let (caret_offset, caret_affinity) = overlay.as_ref().map_or_else(
            || {
                let position = view.commands.visual_position().unwrap_or_else(|| {
                    crate::command::layout_motion::VisualPosition {
                        text_offset: view.commands.cursor(),
                        affinity: view.commands.boundary_affinity(),
                    }
                });
                (position.text_offset, position.affinity)
            },
            |overlay| (
                overlay.selected_range_in_overlay().end,
                BoundaryAffinity::Downstream,
            ),
        );
        let current_row = snapshot
            .logical_endpoint_geometry(caret_offset, caret_affinity)
            .or_else(|error| match error {
                LayoutError::NotACaretStop { .. } => snapshot.logical_endpoint_geometry(
                    caret_offset,
                    match caret_affinity {
                        BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
                        BoundaryAffinity::Downstream => BoundaryAffinity::Upstream,
                    },
                ),
                error => Err(error),
            })
            .ok()
            .map(|geometry| geometry.row_index);
        let mut bands = BTreeMap::<usize, LineNumberRegion>::new();
        let source_line_count = self.document.physical_line_count()?;
        let has_horizontal_materialization = snapshot.has_horizontal_materialization();
        let mut hard_lines = BTreeMap::<usize, PartialHardLineNumbers>::new();
        for (row_index, row) in snapshot.rows.iter().enumerate() {
            if row.table_cell.is_none() {
                let line = hard_lines.entry(row.hard_line_index).or_insert_with(|| {
                    PartialHardLineNumbers {
                        range: row.hard_line_range.clone(),
                        has_first: false,
                        has_last: false,
                        first_row: row_index,
                        last_row: row_index,
                        source_lines: Default::default(),
                    }
                });
                line.has_first |=
                    row.text_range.start == row.hard_line_range.start && !row.wrapped_from_previous;
                line.has_last |=
                    row.text_range.end == row.hard_line_range.end && !row.wraps_to_next;
                line.last_row = row_index;
            }
            // A horizontally sliced unwrapped row can name an enormous hard
            // line while retaining only local clusters. Query those contributors,
            // never the unseen logical hull between distant cached fragments.
            let mut indexes = std::collections::BTreeSet::new();
            if row.clusters.is_empty() {
                indexes.extend(physical_indexes_for_presentation_range(
                    &self.document,
                    row.text_range.start..row.text_range.start,
                    overlay.as_ref(),
                )?);
            } else {
                for cluster in &row.clusters {
                    indexes.extend(physical_indexes_for_presentation_range(
                        &self.document,
                        cluster.text_range.clone(),
                        overlay.as_ref(),
                    )?);
                }
                if row.table_cell.is_none()
                    && !row.wrapped_from_previous
                    && !row.wraps_to_next
                    && has_horizontal_materialization
                {
                    // An unwrapped row keeps its complete logical range even
                    // when horizontal refinement only installs later glyphs.
                    // Its leading physical ordinal is still the number shown.
                    indexes.extend(physical_indexes_for_presentation_range(
                        &self.document,
                        row.text_range.start..row.text_range.start,
                        overlay.as_ref(),
                    )?);
                }
            }
            let Some(index) = indexes
                .iter()
                .copied()
                .find(|index| *index < source_line_count)
            else {
                continue;
            };
            if let Some(line) = hard_lines
                .get_mut(&row.hard_line_index)
                .filter(|_| row.table_cell.is_none())
            {
                line.source_lines.insert(index);
            }
            let bounds = row
                .table_cell
                .as_ref()
                .map_or(row.y..row.y + row.height(), |cell| {
                    cell.rect.y..cell.rect.y + cell.rect.height
                });
            // Folded prose can put several physical source lines on one
            // visual row. Its earliest contributor owns the whole row;
            // subsequent contributors must not add extra, smaller numbers.
            let entry = bands.entry(index).or_insert(LineNumberRegion {
                source_line: index + 1,
                top: bounds.start,
                bottom: bounds.end,
                estimated: false,
                current: false,
            });
            entry.top = entry.top.min(bounds.start);
            entry.bottom = entry.bottom.max(bounds.end);
            entry.current |= current_row == Some(row_index);
        }
        // A regional paragraph can contain several physical source lines and
        // only a slice of their wraps. Recover each contributing line's full
        // projected extent through indexes, then extrapolate offscreen row
        // placement from the installed checkpoint geometry. Never shape or
        // walk the unseen paragraph to obtain a gutter band.
        for (index, line) in hard_lines {
            if line.has_first && line.has_last {
                continue;
            }
            let first_row = &snapshot.rows[line.first_row];
            let last_row = &snapshot.rows[line.last_row];
            let prefix = layout
                .hard_line_prefix_height(index)
                .map_err(LayoutError::HeightIndex)?;
            let height = layout
                .hard_line_range_height(index..index + 1)
                .map_err(LayoutError::HeightIndex)?;
            let (before, after, after_exact) = snapshot
                .hard_line_text_band_insets(index)
                .unwrap_or((0., 0., false));
            let text_top = if line.has_first {
                first_row.y
            } else {
                prefix.height() as f32 + before
            };
            let known_bottom = last_row.y + last_row.height();
            let text_bottom = if line.has_last {
                known_bottom
            } else if height.is_exact() && after_exact {
                (prefix.height() + height.height()) as f32 - after
            } else {
                // Height indexes begin with a generic per-hard-line estimate.
                // A checkpoint already knows the advance through its consumed
                // prefix, so project that density over the whole hard line.
                let consumed = last_row
                    .text_range
                    .end
                    .saturating_sub(line.range.start)
                    .max(1);
                text_top + (known_bottom - text_top) * line.range.len() as f32 / consumed as f32
            }
            .max(known_bottom);
            for physical in line.source_lines {
                let Some(extent) = self.document.physical_line_projected_extent(physical) else {
                    continue;
                };
                let extent = presentation_range_for_base_extent(extent, overlay.as_ref());
                let start = extent.start.max(line.range.start).min(line.range.end);
                let end = extent.end.max(start).min(line.range.end);
                let Some(band) = bands.get_mut(&physical) else {
                    continue;
                };
                let rows = &snapshot.rows[line.first_row..=line.last_row];
                let covered = |offset| {
                    let at = rows.partition_point(|row| row.text_range.end < offset);
                    rows.get(at).is_some_and(|row| {
                        row.text_range.start <= offset && offset <= row.text_range.end
                    })
                };
                let has_start = covered(start);
                let has_end = covered(end);
                let at = |offset: usize| {
                    text_top
                        + (text_bottom - text_top) * offset.saturating_sub(line.range.start) as f32
                            / line.range.len().max(1) as f32
                };
                if !has_start {
                    band.top = band.top.min(at(start));
                }
                if !has_end {
                    band.bottom = band.bottom.max(at(end));
                }
                band.estimated |= !has_start || !has_end;
            }
        }
        let visible = LayoutRect {
            x: 0.0,
            y: layout.viewport_top(),
            width: layout.width(),
            height: layout.height(),
        };
        Ok(bands
            .into_values()
            .filter(|band| band.top < visible.y + visible.height && band.bottom > visible.y)
            .collect())
    }
}

struct PartialHardLineNumbers {
    range: std::ops::Range<usize>,
    has_first: bool,
    has_last: bool,
    first_row: usize,
    last_row: usize,
    source_lines: std::collections::BTreeSet<usize>,
}

fn presentation_range_for_base_extent(
    range: std::ops::Range<usize>,
    overlay: Option<&CompositionOverlay>,
) -> std::ops::Range<usize> {
    let Some(overlay) = overlay else {
        return range;
    };
    let marked = overlay.marked_range();
    let start = overlay
        .overlay_offset_for_base_boundary(range.start, Association::BeforeInsertion)
        .unwrap_or(marked.start);
    let end = overlay
        .overlay_offset_for_base_boundary(range.end, Association::AfterInsertion)
        .unwrap_or(marked.end);
    start.min(end)..start.max(end)
}

/// Preview payloads are disposable. Their glyphs stay owned by the physical
/// source line at the replacement start, while unchanged suffixes retain exact
/// source provenance. At most three local ranges are returned for one cluster.
fn base_ranges_for_presentation(
    range: std::ops::Range<usize>,
    overlay: Option<&CompositionOverlay>,
) -> Vec<std::ops::Range<usize>> {
    let Some(overlay) = overlay else {
        return vec![range];
    };
    let marked = overlay.marked_range();
    let replaced = overlay.replacement_range();
    if range.is_empty() {
        let at = if range.start < marked.start {
            range.start
        } else if range.start > marked.end {
            replaced.end + range.start - marked.end
        } else {
            replaced.start
        };
        return vec![at..at];
    }
    let mut result = Vec::with_capacity(3);
    if range.start < marked.start {
        result.push(range.start..range.end.min(marked.start));
    }
    if range.start < marked.end && marked.start < range.end {
        result.push(replaced.start..replaced.start);
    }
    if range.end > marked.end {
        result.push(
            replaced.end + range.start.max(marked.end) - marked.end
                ..replaced.end + range.end - marked.end,
        );
    }
    result
}
fn physical_indexes_for_presentation_range(
    document: &Document,
    range: std::ops::Range<usize>,
    overlay: Option<&CompositionOverlay>,
) -> Result<Vec<usize>, DocumentError> {
    let mut indexes = std::collections::BTreeSet::new();
    for range in base_ranges_for_presentation(range, overlay) {
        indexes.extend(document.physical_line_indexes_in_text(range)?);
    }
    Ok(indexes.into_iter().collect())
}
