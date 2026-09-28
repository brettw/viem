//! Explicit presentation recovery when a file is replaced, without treating
//! coordinates from the previous document as points in the new revision.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRestoration {
    pub cursor: (usize, usize),
    pub cursor_affinity: BoundaryAffinity,
    pub viewport: (usize, usize),
    pub row_fraction: f32,
    pub left: f32,
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn capture_view_restoration(&self, id: ViewId) -> Result<ViewRestoration, CoreError> {
        let view = self.views.get(&id).ok_or(CoreError::UnknownView(id))?;
        let lines = self.document.hard_line_snapshot();
        let cursor = lines.recovery_location(view.commands.cursor())
            .ok_or(LayoutError::InvalidTextOffset(view.commands.cursor()))?;
        // Capture what is still displayed when metrics/configuration have
        // invalidated layout. Its text positions remain valid in this source
        // revision; the replacement will materialize fresh geometry.
        let snapshot = view.layout.snapshot().filter(|snapshot|
            snapshot.document_id == self.document.id()
                && snapshot.document_revision == self.document.revision())
            .ok_or(LayoutError::NoRows)?;
        let top = view.layout.viewport_top();
        let row = snapshot.rows.iter().find(|row| row.y + row.height() > top)
            .or_else(|| snapshot.rows.last()).ok_or(LayoutError::NoRows)?;
        Ok(ViewRestoration {
            cursor,
            cursor_affinity: view.commands.boundary_affinity(),
            viewport: lines.recovery_location(row.text_range.start)
                .ok_or(LayoutError::InvalidTextOffset(row.text_range.start))?,
            row_fraction: (top - row.y) / row.height().max(f32::EPSILON),
            left: view.layout.viewport_left(),
        })
    }

    /// A replacement view starts in Normal mode. Resolve its cursor and scroll
    /// independently so scrolling away from the caret stays away after reload.
    /// Callers stage this view before publishing the replacement document.
    pub fn restore_view(&mut self, id: ViewId, document: DocumentId, revision: Revision,
        state: ViewRestoration) -> Result<(), CoreError> {
        if document != self.document.id() { return Err(DocumentError::WrongDocument.into()); }
        if revision != self.document.revision() {
            return Err(DocumentError::WrongSnapshot { expected: self.document.revision(), actual: revision }.into());
        }
        if !state.left.is_finite() || !state.row_fraction.is_finite() {
            return Err(LayoutError::InvalidGeometry.into());
        }
        let lines = self.document.hard_line_snapshot();
        let cursor = lines.recover_location(state.cursor.0, state.cursor.1).ok_or(LayoutError::NoRows)?;
        let top = lines.recover_location(state.viewport.0, state.viewport.1).ok_or(LayoutError::NoRows)?;
        let anchor = self.document.text_anchor(self.document.text_point(top)?,
            Association::AfterInsertion, BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding)?;
        let view = self.views.get_mut(&id).ok_or(CoreError::UnknownView(id))?;
        view.commands.set_cursor_from_pointer(&self.document, cursor, state.cursor_affinity, false);
        view.viewport_anchor = Some(ViewportTextAnchor {
            anchor, offset_from_reference: state.row_fraction,
            reference: ViewportAnchorReference::RowFraction,
        });
        self.materialize_immediate_viewport(id, ImmediateLayoutIntent::PreserveViewport)?;
        self.handle(id, CoreEvent::SetViewportOrigin { left: state.left.max(0.0), top: None })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;

    fn opened(text: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
        let mut core = Core::new(Document::new(text));
        let view = core.add_view(MockTextMeasurementProvider::new(), 180.0, 100.0);
        (core, view)
    }

    fn restore(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, state: ViewRestoration) {
        core.restore_view(view, core.document.id(), core.document.revision(), state).unwrap();
    }

    #[test]
    fn replacement_recovers_line_and_grapheme_column_not_old_byte_offsets() {
        let (mut old, view) = opened("prefix\naé👩‍💻tail\nlast");
        let at = "prefix\naé".len();
        assert!(old.views.get_mut(&view).unwrap().commands.set_cursor(&old.document, at));
        let saved = old.capture_view_restoration(view).unwrap();
        assert_eq!(saved.cursor, (1, 2));
        let (mut next, replacement) = opened("a much longer prefix\n界o\u{301}👍rest\nlast");
        restore(&mut next, replacement, saved);
        assert_eq!(next.command_state(replacement).unwrap().cursor(), "a much longer prefix\n界o\u{301}".len());
    }

    #[test]
    fn replacement_retains_independent_viewports_and_offscreen_cursors() {
        let source = "a long line with a little content\n".repeat(120);
        let (mut old, first) = opened(&source);
        let second = old.add_view(MockTextMeasurementProvider::new(), 330.0, 100.0);
        old.handle(first, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(303.0) }).unwrap();
        old.handle(second, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(677.0) }).unwrap();
        let first_state = old.capture_view_restoration(first).unwrap();
        let second_state = old.capture_view_restoration(second).unwrap();
        assert_ne!(first_state.viewport, second_state.viewport);
        assert_eq!(first_state.cursor, (0, 0));
        let (mut next, replacement) = opened(&source);
        let other = next.add_view(MockTextMeasurementProvider::new(), 330.0, 100.0);
        restore(&mut next, replacement, first_state);
        restore(&mut next, other, second_state);
        for (id, saved) in [(replacement, first_state), (other, second_state)] {
            let actual = next.capture_view_restoration(id).unwrap();
            assert_eq!(actual.cursor, saved.cursor);
            assert_eq!(actual.viewport, saved.viewport);
            assert!((actual.row_fraction - saved.row_fraction).abs() < 0.001);
        }
        assert!((next.viewport_state(replacement).unwrap().top() - old.viewport_state(first).unwrap().top()).abs() < 0.01);
    }

    #[test]
    fn replacement_clamps_shortened_unicode_lines_and_empty_files() {
        let (mut old, view) = opened(&"long content\n".repeat(60));
        let at = old.document.hard_line_snapshot().recover_location(50, 10).unwrap();
        assert!(old.views.get_mut(&view).unwrap().commands.set_cursor(&old.document, at));
        old.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(600.0) }).unwrap();
        let state = old.capture_view_restoration(view).unwrap();
        for (source, expected) in [("one\né👩‍💻", "one\né".len()), ("", 0)] {
            let (mut next, replacement) = opened(source);
            restore(&mut next, replacement, state);
            assert_eq!(next.command_state(replacement).unwrap().cursor(), expected);
            let viewport = next.viewport_state(replacement).unwrap();
            assert!(viewport.top() >= 0.0);
            assert!(viewport.top() <= viewport.maximum_top().unwrap());
        }
    }

    #[test]
    fn replacement_clamps_horizontal_scroll_to_visible_rows() {
        let (mut old, view) = opened(&"a".repeat(200));
        old.handle(view, CoreEvent::SetWrap(false)).unwrap();
        old.handle(view, CoreEvent::SetViewportOrigin { left: 300.0, top: None }).unwrap();
        let state = old.capture_view_restoration(view).unwrap();
        assert!(state.left > 0.0);
        let (mut next, replacement) = opened("short");
        next.handle(replacement, CoreEvent::SetWrap(false)).unwrap();
        restore(&mut next, replacement, state);
        assert_eq!(next.viewport_state(replacement).unwrap().left(), 0.0);
    }

    #[test]
    fn distant_replacement_recovery_keeps_million_line_documents_unflattened() {
        let source = "line\n".repeat(1_000_000);
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::PlainText).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let state = ViewRestoration {
            cursor: (900_000, 2), cursor_affinity: BoundaryAffinity::Downstream,
            viewport: (800_000, 0), row_fraction: 0.25, left: 0.0,
        };
        restore(&mut core, view, state);
        let captured = core.capture_view_restoration(view).unwrap();
        assert_eq!(captured.cursor, state.cursor);
        assert_eq!(captured.viewport, state.viewport);
        assert!(!core.document.projection().compatibility_text_is_materialized());
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 100);
    }

    #[test]
    fn capture_retains_displayed_position_during_pending_metric_changes() {
        let (mut core, view) = opened(&"line of content\n".repeat(120));
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0.0, top: Some(309.0) }).unwrap();
        let expected = core.capture_view_restoration(view).unwrap();
        core.views.get_mut(&view).unwrap().engine.provider_mut()
            .set_metrics_generation(MetricsGeneration(2));
        assert_eq!(core.capture_view_restoration(view).unwrap(), expected);
    }

    #[test]
    fn stale_replacement_identity_does_not_move_a_view() {
        let (mut core, view) = opened("text");
        let before = core.capture_view_restoration(view).unwrap();
        let mut state = before;
        state.cursor = (9, 9);
        assert!(core.restore_view(view, core.document.id(), Revision(core.document.revision().0 + 1), state).is_err());
        assert_eq!(core.capture_view_restoration(view).unwrap(), before);
    }
}
