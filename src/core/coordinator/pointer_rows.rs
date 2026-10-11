//! Whole displayed-row pointer selection uses core-owned logical ranges.
use super::*;

impl<P: TextMeasurementProvider> Core<P> {
    pub(super) fn pointer_row_range(
        &self,
        id: ViewId,
        revision: Revision,
        layout_revision: LayoutRevision,
        y: f32,
    ) -> Result<std::ops::Range<usize>, CoreError> {
        if revision != self.document.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.document.revision(),
                actual: revision,
            }
            .into());
        }
        let view = self.views.get(&id).ok_or(CoreError::UnknownView(id))?;
        let snapshot = current_snapshot_for_layout(
            &self.document,
            &view.layout,
            inspect_layout_provider(&view.engine),
        )
        .ok_or(LayoutError::NoRows)?;
        if snapshot.revision != layout_revision {
            return Err(LayoutError::StaleLayout {
                expected: snapshot.revision,
                actual: layout_revision,
            }
            .into());
        }
        let row = snapshot.gutter_row(y)?;
        let mut range = if let Some(cell) = &row.table_cell {
            // A table has several text rows at the same vertical coordinate.
            // Its gutter addresses the complete semantic row, including cells
            // whose glyphs are outside horizontal materialization.
            self.document
                .projection()
                .table_at(cell.text_range.start)
                .filter(|table| table.id == cell.table_id)
                .and_then(|table| table.rows.get(cell.row))
                .ok_or(DocumentError::AmbiguousProjection)?
                .range
                .clone()
        } else {
            row.text_range.clone()
        };
        let text = self.document.projection().text_tree();
        if range.end < text.byte_len()
            && text.slice(range.end..range.end + 1).as_deref() == Ok("\n")
        {
            range.end += 1;
        }
        self.document.text_point(range.start)?;
        self.document.text_point(range.end)?;
        Ok(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;

    fn row(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, index: usize, extend: bool) {
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let visual = &snapshot.rows[index];
        let event = CoreEvent::SelectPointerRow {
            document_revision: core.document.revision(),
            layout_revision: snapshot.revision,
            y: visual.y + visual.height() / 2.,
            extend_selection: extend,
        };
        core.handle(view, event).unwrap();
    }
    fn selected(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> std::ops::Range<usize> {
        let state = core.command_state(view).unwrap();
        let anchor = state.visual_anchor().unwrap();
        anchor.min(state.cursor())..anchor.max(state.cursor())
    }

    #[test]
    fn gutter_drag_preserves_whole_origin_and_reverses_direction() {
        for insertion in [false, true] {
            let mut core = Core::new(Document::new("first\nsecond\nthird\nlast"));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            if insertion {
                core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                    .unwrap();
            }
            row(&mut core, view, 1, false);
            assert_eq!(selected(&core, view), 6..13);
            row(&mut core, view, 2, true);
            assert_eq!(selected(&core, view), 6..19);
            row(&mut core, view, 0, true);
            assert_eq!(selected(&core, view), 0..13);
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
            row(&mut core, view, 1, true);
            assert_eq!(selected(&core, view), 6..13);
            assert_eq!(core.command_state(view).unwrap().cursor(), 13);
            assert!(core.command_state(view).unwrap().is_native_selection());
            assert_eq!(core.document.source_bytes(), b"first\nsecond\nthird\nlast");
        }
    }

    #[test]
    fn gutter_selects_visible_wraps_and_complete_semantic_table_rows() {
        let mut core = Core::new(Document::new("word ".repeat(60)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 120., 300.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let expected = snapshot.rows[1].text_range.clone();
        assert!(snapshot.rows[1].wraps_to_next);
        row(&mut core, view, 1, false);
        assert_eq!(selected(&core, view), expected);

        let document = Document::from_bytes(
            b"| head | other |\n| --- | --- |\n| long body | last |".to_vec(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 160., 300.);
        let table = &core.document.projection().tables()[0];
        let expected = table.rows[1].range.clone();
        let index = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .position(|row| row.table_cell.as_ref().is_some_and(|cell| cell.row == 1))
            .unwrap();
        row(&mut core, view, index, false);
        assert_eq!(selected(&core, view), expected);
    }

    #[test]
    fn gutter_uses_whole_rows_with_visual_and_select_mouse_policies() {
        for select in [false, true] {
            let mut core = Core::new(Document::new("one\ntwo\nthree"));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
            core.handle(view, CoreEvent::Input(InputEvent::key('V')))
                .unwrap();
            if select {
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('g'))))
                    .unwrap();
            }
            row(&mut core, view, 1, false);
            assert_eq!(core.list_selection_identity(view).unwrap().range(), 4..8);
            assert_eq!(core.command_state(view).unwrap().is_select_mode(), select);
            row(&mut core, view, 0, true);
            assert_eq!(core.list_selection_identity(view).unwrap().range(), 0..8);
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        }
    }

    #[test]
    fn gutter_origin_rebases_across_another_views_edit() {
        let mut core = Core::new(Document::new("first\nsecond\nthird"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        let other = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        row(&mut core, view, 1, false);
        core.handle(other, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        core.handle(other, CoreEvent::Input(InputEvent::Text("new\n".into())))
            .unwrap();
        core.handle(
            view,
            CoreEvent::Resize {
                width: 400.,
                height: 300.,
            },
        )
        .unwrap();
        row(&mut core, view, 0, true);
        assert_eq!(selected(&core, view), 0..17);
    }

    #[test]
    fn gutter_long_unwrapped_row_does_not_shape_or_reveal_its_far_end() {
        let mut core = Core::new(Document::new("abc ".repeat(100_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240., 200.);
        core.handle(view, CoreEvent::SetWrap(false)).unwrap();
        let calls = core.views[&view].engine.provider().request_calls();
        let revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        row(&mut core, view, 0, false);
        assert_eq!(selected(&core, view), 0..400_000);
        assert_eq!(core.layout(view).unwrap().viewport_left(), 0.);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            revision
        );
        assert_eq!(core.views[&view].engine.provider().request_calls(), calls);
    }

    #[test]
    fn gutter_large_document_reuses_layout_and_rejects_old_geometry() {
        let mut core = Core::new(Document::new("row of text\n".repeat(25_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240., 300.);
        let shaped = core.views[&view].engine.provider().request_calls();
        let layout_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
        for index in 0..100 {
            row(&mut core, view, index % 5, index != 0);
        }
        assert_eq!(core.views[&view].engine.provider().request_calls(), shaped);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            layout_revision
        );
        core.handle(
            view,
            CoreEvent::Resize {
                width: 200.,
                height: 300.,
            },
        )
        .unwrap();
        let before = selected(&core, view);
        assert!(core
            .handle(
                view,
                CoreEvent::SelectPointerRow {
                    document_revision: core.document.revision(),
                    layout_revision,
                    y: 0.,
                    extend_selection: true
                }
            )
            .is_err());
        assert_eq!(selected(&core, view), before);
        row(&mut core, view, 3, true);
        assert_eq!(core.views[&view].engine.provider().request_calls(), shaped);
    }
}
