//! Native link edits share verified document transactions and controller history.
use super::*;
use crate::document::ImageEditIntent;
impl<P: TextMeasurementProvider> Core<P> {
    pub fn edit_image(
        &mut self,
        view: ViewId,
        expected: LogicalSelectionIdentity,
        intent: ImageEditIntent,
    ) -> Result<CoreOutcome, CoreError> {
        if self.list_selection_identity(view)? != expected {
            return Err(CoreError::StaleLogicalSelection);
        }
        let (preflight, caret) = self
            .document
            .prepare_image_edit(expected.document(), expected.revision(), intent)
            .map_err(command_model_transaction_error)?;
        if preflight.is_no_op() {
            return Ok(CoreOutcome {
                command: None,
                document_changed: false,
                position_map: None,
                layout_changed: false,
                composition_changes: Vec::new(),
            });
        }
        let map = preflight.text_position_map().clone();
        let before = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        let mapped =
            self.prepare_mapped_commands(&map, CommandInterpreter::capture_position_anchors)?;
        self.finalize_open_edit_group(view)?;
        let prepared = self
            .document
            .rebind_prepared_after_group_close(preflight)
            .map_err(command_model_transaction_error)?;
        self.document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        for (id, commands) in mapped {
            self.views.get_mut(&id).expect("prepared view").commands = commands;
        }
        let resumes_typing = self
            .views
            .get_mut(&view)
            .expect("prepared view")
            .commands
            .finish_native_link_edit(&mut self.document, caret);
        let after = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before, after.clone()),
        )?;
        if resumes_typing {
            self.edit_group_owner = Some(view);
            self.edit_group_restoration = Some(OpenGroupRestoration {
                generation: self.document.edit_group_generation(),
                parent: self.document.history_status().current,
                before: after,
            });
        }
        let composition_changes = self.refresh_views_after_native_change(view, &map)?;
        if let Err(error) =
            self.materialize_immediate_viewport(view, ImmediateLayoutIntent::RevealCaret)
        {
            self.record_presentation_error(view, error);
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: true,
            position_map: Some(map),
            layout_changed: true,
            composition_changes,
        })
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Select an atomic image without replaying pointer or Vim input. The
    /// projection identity is checked before closing a typing undo group.
    pub fn select_image(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        offset: usize,
    ) -> Result<CoreOutcome, CoreError> {
        if document != self.document.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if revision != self.document.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }
            .into());
        }
        self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        if !self.document.format().is_wysiwyg() {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let image = self
            .document
            .image_snapshot_at(self.document.text_point(offset)?)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        self.document.text_point(image.range.start)?;
        self.document.text_point(image.range.end)?;
        let composition_changes = if self.views[&view].composition.is_some() {
            self.handle_composition_event(view, CompositionEvent::Cancel)?
                .composition_changes
        } else {
            Vec::new()
        };
        self.finalize_open_edit_group(view)?;
        self.views
            .get_mut(&view)
            .expect("validated view")
            .commands
            .select_inline_image_from_pointer(&self.document, image.range)?;
        if let Err(error) =
            self.materialize_immediate_viewport(view, ImmediateLayoutIntent::RevealCaret)
        {
            self.record_presentation_error(view, error);
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: false,
            position_map: None,
            layout_changed: true,
            composition_changes,
        })
    }
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Publish a native image resource generation before refreshing the view.
    /// A caller may preserve exact heights only for a single, verified provider
    /// generation containing bitmap/status changes with unchanged dimensions.
    pub fn image_resources_changed(
        &mut self,
        view_id: ViewId,
        previous_generation: crate::layout::MetricsGeneration,
        geometry_changed: bool,
    ) -> Result<(), CoreError> {
        let view = self.views.get_mut(&view_id).ok_or(CoreError::UnknownView(view_id))?;
        let requirements = inspect_layout_provider(&view.engine);
        let verified = view.observed_metrics_generation == previous_generation
            && previous_generation.0.checked_add(1) == Some(requirements.metrics_generation.0);
        cancel_active_layout_work(view);
        if verified {
            view.layout.invalidate_image_metrics(requirements.measurement_environment_id,
                requirements.metrics_generation, geometry_changed);
        } else {
            // An intervening font or environment change is not an image-only
            // refresh. Keep learned estimates but retain the generic refresh
            // policy until current geometry is installed.
            view.layout.invalidate_text_metrics();
        }
        view.observed_metrics_generation = requirements.metrics_generation;
        Ok(())
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    use crate::document::{Encoding, Format};
    use crate::layout::*;

    struct ImageProvider {
        text: MockTextMeasurementProvider,
        changed_height: f32,
    }
    impl TextMeasurementProvider for ImageProvider {
        fn measurement_environment_id(&self) -> MeasurementEnvironmentId { self.text.measurement_environment_id() }
        fn metrics_generation(&self) -> MetricsGeneration { self.text.metrics_generation() }
        fn render_run_policy(&self) -> Option<RenderRunPolicy> { self.text.render_run_policy() }
        fn shape_batch(&mut self, requests: &[ShapeRequest<'_>]) -> Result<Vec<ShapedFragment>, MeasurementError> {
            let mut result = self.text.shape_batch(requests)?;
            for (request, shaped) in requests.iter().zip(&mut result) {
                for image in request.inline_images {
                    let height = if image.destination == "100.png" || image.destination == "Word style.png" { self.changed_height } else { 160. };
                    let cluster = shaped.clusters.iter_mut().find(|cluster| cluster.text_range == image.text_range).unwrap();
                    cluster.advance = 300.;
                    cluster.metrics = TextMetrics { ascent: height, descent: 0., leading: 0. };
                    cluster.typographic_bounds = ShapedBounds { x: 0., y: -height, width: 300., height };
                    cluster.ink_bounds = cluster.typographic_bounds;
                    for stop in &mut cluster.caret_stops {
                        stop.inline_offset = if stop.text_offset == image.text_range.start { 0. } else { 300. };
                    }
                }
            }
            Ok(result)
        }
    }

    #[test]
    fn image_publications_keep_large_document_scroll_extents_and_wheel_origin_stable() {
        let source = (0..10_000).map(|i| format!("![image]({i}.png)\n\n")).collect::<String>();
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view_id = core.add_view(ImageProvider { text: MockTextMeasurementProvider::new(), changed_height: 160. }, 400., 300.);
        {
            let view = core.views.get_mut(&view_id).unwrap();
            view.engine.relayout(&core.document, &mut view.layout).unwrap();
        }
        let original_extent = core.layout(view_id).unwrap().content_height();
        assert!(original_extent.is_exact());
        let start = core.layout(view_id).unwrap().hard_line_prefix_height(100).unwrap().height() as f32;
        core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(start) }).unwrap();
        let requests = core.views[&view_id].engine.provider().text.request_calls();
        // Bitmap-only publication retires handles without changing geometry certainty.
        core.views.get_mut(&view_id).unwrap().engine.provider_mut().text.set_metrics_generation(MetricsGeneration(2));
        core.image_resources_changed(view_id, MetricsGeneration(1), false).unwrap();
        assert_eq!(core.layout(view_id).unwrap().content_height(), original_extent);
        for amount in [8., 16., 24., 32.] {
            let requested = start + amount;
            core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(requested) }).unwrap();
            assert!((core.layout(view_id).unwrap().viewport_top() - requested).abs() < 0.01);
            assert_eq!(core.layout(view_id).unwrap().content_height(), original_extent);
        }
        let before_top = core.layout(view_id).unwrap().viewport_top();
        {
            let provider = core.views.get_mut(&view_id).unwrap().engine.provider_mut();
            provider.changed_height = 260.;
            provider.text.set_metrics_generation(MetricsGeneration(3));
        }
        core.image_resources_changed(view_id, MetricsGeneration(2), true).unwrap();
        assert_eq!(core.layout(view_id).unwrap().content_height().height(), original_extent.height());
        assert!(!core.layout(view_id).unwrap().content_height().is_exact());
        core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(before_top) }).unwrap();
        assert!((core.layout(view_id).unwrap().viewport_top() - before_top).abs() < 0.01);
        assert!((core.layout(view_id).unwrap().content_height().height() - original_extent.height() - 100.).abs() < 0.01);
        for amount in [8., 16., 24.] {
            core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(before_top + amount) }).unwrap();
            assert!((core.layout(view_id).unwrap().viewport_top() - before_top - amount).abs() < 0.01);
        }
        assert!(core.views[&view_id].engine.provider().text.request_calls() - requests < 100,
            "resource publication must shape only the requested viewport");
    }
    #[test]
    fn cold_demo_image_growth_and_bitmap_refresh_preserve_the_within_image_scroll_offset() {
        let document = Document::from_bytes(include_bytes!("../../../docs/markdown_demo.md").to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let image_at = document.projection().inline_images_for_region(&(0..document.projection().text_tree().byte_len()))
            .into_iter().find(|image| image.destination == "Word style.png").unwrap().range.start;
        let mut core = Core::new(document);
        let view_id = core.add_view(ImageProvider { text: MockTextMeasurementProvider::new(), changed_height: 64. }, 400., 300.);
        assert!(core.layout(view_id).unwrap().snapshot().unwrap().coverage.hard_lines().end < core.document.line_count());
        let mut reached = false;
        for _ in 0..2_000 {
            let layout = core.layout(view_id).unwrap();
            let top = layout.viewport_top();
            let image = layout.snapshot().unwrap().rows.iter().flat_map(|row| &row.clusters)
                .find(|cluster| cluster.text_range.start == image_at);
            if image.is_some_and(|image| top >= image.typographic_bounds.y + 8.
                && top < image.typographic_bounds.y + image.typographic_bounds.height) {
                reached = true; break;
            }
            core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(top + 13.) }).unwrap();
        }
        assert!(reached, "wheel traversal must reach the initially short image");
        assert_eq!(core.command_state(view_id).unwrap().cursor(), 0, "caret stays offscreen throughout scrolling");
        let before_top = core.layout(view_id).unwrap().viewport_top();
        {
            let provider = core.views.get_mut(&view_id).unwrap().engine.provider_mut();
            provider.changed_height = 4_000.;
            provider.text.set_metrics_generation(MetricsGeneration(2));
        }
        core.image_resources_changed(view_id, MetricsGeneration(1), true).unwrap();
        // This is the native refreshLayoutIfNeeded route, not an absolute-scroll
        // event and not a fully pre-laid-out document.
        core.handle(view_id, CoreEvent::Resize { width: 400., height: 300. }).unwrap();
        assert!((core.layout(view_id).unwrap().viewport_top() - before_top).abs() < 0.01,
            "decoding must retain the pixel offset from the image top");
        let image = core.layout(view_id).unwrap().snapshot().unwrap().rows.iter().flat_map(|row| &row.clusters)
            .find(|cluster| cluster.text_range.start == image_at).unwrap();
        assert_eq!(image.typographic_bounds.height, 1024.);
        for tick in 0..80 {
            let requested = core.layout(view_id).unwrap().viewport_top() + 13.;
            core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(requested) }).unwrap();
            assert!((core.layout(view_id).unwrap().viewport_top() - requested).abs() < 0.01,
                "wheel tick {tick} must not jump around a tall image");
            if tick == 30 {
                core.views.get_mut(&view_id).unwrap().engine.provider_mut().text.set_metrics_generation(MetricsGeneration(3));
                core.image_resources_changed(view_id, MetricsGeneration(2), false).unwrap();
                core.handle(view_id, CoreEvent::Resize { width: 400., height: 300. }).unwrap();
                assert!((core.layout(view_id).unwrap().viewport_top() - requested).abs() < 0.01,
                    "bitmap-only refresh inside the image keeps the wheel origin");
            }
        }
        assert_eq!(core.command_state(view_id).unwrap().cursor(), 0);
    }

    #[test]
    fn same_size_resource_refresh_at_document_end_does_not_pin_to_a_growing_image_bottom() {
        for image_notification in [true, false] {
            let source = "Before image.\n\n".repeat(40) + "![last](100.png)";
            let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            let mut core = Core::new(document);
            let view_id = core.add_view(ImageProvider { text: MockTextMeasurementProvider::new(), changed_height: 64. }, 400., 300.);
            core.handle(view_id, CoreEvent::SetViewportOrigin { left: 0., top: Some(f32::MAX) }).unwrap();
            let before_anchor = core.views[&view_id].viewport_anchor.unwrap();
            assert!(core.layout(view_id).unwrap().viewport_top() > 0.);
            assert_eq!(core.command_state(view_id).unwrap().cursor(), 0);
            {
                let provider = core.views.get_mut(&view_id).unwrap().engine.provider_mut();
                provider.changed_height = 4_000.;
                provider.text.set_metrics_generation(MetricsGeneration(2));
            }
            if image_notification {
                core.image_resources_changed(view_id, MetricsGeneration(1), true).unwrap();
            }
            core.handle(view_id, CoreEvent::Resize { width: 400., height: 300. }).unwrap();
            let after_anchor = core.views[&view_id].viewport_anchor.unwrap();
            if image_notification {
                // Jumping directly to EOF leaves an estimated offscreen prefix.
                // Refining it may change document y; the retained logical row's
                // position on screen is the observable scrolling invariant.
                assert_eq!(after_anchor.anchor.offset(), before_anchor.anchor.offset());
                assert!((after_anchor.offset_from_reference - before_anchor.offset_from_reference).abs() < 0.01,
                    "a resource refresh must retain the visible content instead of jumping to the new bottom");
                assert_eq!(after_anchor.reference, ViewportAnchorReference::RowTop);
            } else {
                assert_ne!(after_anchor.anchor.offset(), before_anchor.anchor.offset(),
                    "generic metrics changes retain the existing EOF pinning policy");
                let snapshot = core.layout(view_id).unwrap().snapshot().unwrap();
                assert!((core.layout(view_id).unwrap().viewport_top() - (snapshot.total_height - 300.)).abs() < 0.01);
            }
        }
    }

}
