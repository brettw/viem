//! Completion preview policy for layouts whose exact geometry is expensive.

use super::*;

/// A completion word's logical leading edge in one exact presentation layout.
/// The frontend only converts its coordinates and accounts for native padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompletionPopupAnchor {
    pub layout_revision: crate::layout::LayoutRevision,
    pub rect: crate::layout::LayoutRect,
    pub right_to_left: bool,
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn completion_popup_anchor(
        &self,
        view_id: ViewId,
    ) -> Result<Option<CompletionPopupAnchor>, CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        let Some(session) = view
            .completion
            .as_ref()
            .filter(|session| session.is_current(&self.document, &view.commands))
        else {
            return Ok(None);
        };
        let Some(mut start) = session.prefix_start() else {
            return Ok(None);
        };
        let layout = view.composition_layout.as_ref().unwrap_or(&view.layout);
        let Some(snapshot) = current_snapshot_for_layout(
            &self.document,
            layout,
            inspect_layout_provider(&view.engine),
        ) else {
            return Ok(None);
        };
        if view.composition_layout.is_some() {
            let Some(overlay) = self.composition_overlay(view_id)? else {
                return Ok(None);
            };
            let Some(mapped) =
                overlay.overlay_offset_for_base_boundary(start, Association::BeforeInsertion)
            else {
                return Ok(None);
            };
            start = mapped;
        }
        // Downstream chooses the word's side of a wrap/bidi boundary, rather
        // than the previous word's end. Never substitute the moving end caret.
        let Ok(geometry) = snapshot
            .logical_endpoint_geometry(start, BoundaryAffinity::Downstream)
            .or_else(|_| snapshot.logical_endpoint_geometry(start, BoundaryAffinity::Upstream))
        else {
            return Ok(None);
        };
        let row = &snapshot.rows[geometry.row_index];
        let following = row
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.contains(&start));
        let preceding = row
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.end == start);
        let has_word = start < view.commands.cursor() || session.has_inline_preview();
        let cluster =
            if has_word || view.commands.boundary_affinity() == BoundaryAffinity::Downstream {
                following.or(preceding)
            } else {
                preceding.or(following)
            };
        let right_to_left = cluster.is_some_and(|cluster| cluster.bidi_level % 2 == 1);
        let mut rect = geometry.rect;
        if geometry.is_cluster_fallback && right_to_left {
            rect.x += rect.width;
        }
        rect.width = 0.0;
        Ok(Some(CompletionPopupAnchor {
            layout_revision: snapshot.revision,
            rect,
            right_to_left,
        }))
    }

    /// The unwrapped layout engine currently builds a whole-line width/bidi
    /// summary for each distinct text tree. A disposable candidate changes that
    /// identity, so attempting inline previews of huge unwrapped lines would
    /// repeat a document-sized synchronous operation on every selection. Keep
    /// their backend-selected candidates in the popup until acceptance. Wrapped
    /// lines use the existing bounded composition slices and checkpoints.
    pub(super) fn refresh_completion_preview_policy(
        &mut self,
        view_id: ViewId,
    ) -> Result<(), CoreError> {
        let view = self
            .views
            .get(&view_id)
            .ok_or(CoreError::UnknownView(view_id))?;
        if view.completion.is_none() {
            return Ok(());
        }
        let inline_preview = if view.layout.wrap() {
            true
        } else {
            let flow = view.layout.paragraph_flow();
            let caret = view.commands.cursor();
            let line = self
                .document
                .projection()
                .presentation_line_at_offset(caret, flow)
                .and_then(|index| {
                    self.document
                        .projection()
                        .presentation_line_range(index, flow)
                })
                .ok_or(LayoutError::InvalidTextOffset(caret))?;
            line.len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES
        };
        let view = self
            .views
            .get_mut(&view_id)
            .expect("completion view remains attached");
        if view
            .completion
            .as_mut()
            .unwrap()
            .set_inline_preview(inline_preview)
        {
            // Clear incompatible composed geometry immediately. Ordinary base
            // layout and its caches remain reusable and authoritative.
            view.composition_layout = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;

    fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
        core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap();
    }

    fn begin_large_completion() -> (Core<MockTextMeasurementProvider>, ViewId, String) {
        let text = "c camel cameo ".to_owned() + &"word ".repeat(80_000);
        let mut core = Core::new(Document::new(&text));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 120.0);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        key(&mut core, view, Key::Char('a'));
        (core, view, text)
    }

    #[test]
    fn large_unwrapped_completion_search_and_cycle_reuse_base_geometry() {
        let (mut core, view, text) = begin_large_completion();
        let calls = core.views[&view].engine.provider().request_calls();
        let base_revision = core
            .presentation_layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .revision;
        key(&mut core, view, Key::Ctrl('n'));
        let presentation = core.completion_presentation(view).unwrap().unwrap();
        assert_eq!(&presentation.items[..2], &["camel", "cameo"]);
        assert_eq!(presentation.selected_index, Some(0));
        assert!(
            presentation.searching,
            "the initial turn must leave background work pending"
        );
        assert!(core.composition_overlay(view).unwrap().is_none());
        for _ in 0..3 {
            core.poll_completion(view).unwrap();
        }
        key(&mut core, view, Key::Ctrl('n'));
        assert_eq!(
            core.completion_presentation(view)
                .unwrap()
                .unwrap()
                .selected_index,
            Some(1)
        );
        assert_eq!(
            core.views[&view].engine.provider().request_calls(),
            calls,
            "background candidate search and cycling must not reshape a huge unwrapped line"
        );
        assert_eq!(
            core.presentation_layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            base_revision
        );
        assert_eq!(core.document().source_bytes(), text.as_bytes());
        assert!(!core.document().is_dirty());
        assert!(!core
            .document()
            .projection()
            .compatibility_text_is_materialized());

        core.accept_completion(view).unwrap();
        assert!(core.completion_presentation(view).unwrap().is_none());
        assert_eq!(
            core.document().source_bytes(),
            ("cameo".to_owned() + &text[1..]).as_bytes()
        );
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    }

    #[test]
    fn wrap_changes_invalidate_completion_preview_policy_and_geometry() {
        let (mut core, view, text) = begin_large_completion();
        key(&mut core, view, Key::Ctrl('n'));
        assert!(core.composition_overlay(view).unwrap().is_none());
        assert!(core.views[&view].composition_layout.is_none());

        core.handle(view, CoreEvent::SetWrap(true)).unwrap();
        assert!(core.composition_overlay(view).unwrap().is_some());
        let preview = core
            .presentation_layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .revision;
        assert!(core.views[&view].composition_layout.is_some());
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        assert!(core.composition_overlay(view).unwrap().is_none());
        assert!(core.views[&view].composition_layout.is_none());
        assert_ne!(
            core.presentation_layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            preview
        );

        core.handle(view, CoreEvent::SetWrap(true)).unwrap();
        assert!(core.composition_overlay(view).unwrap().is_some());
        assert!(core.views[&view].composition_layout.is_some());
        assert_ne!(
            core.presentation_layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            preview
        );
        assert_eq!(core.document().source_bytes(), text.as_bytes());
    }
}
