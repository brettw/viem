//! Gutter hit testing supplies the line domain used by Visual Line selection.
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
        let mut range = if view.commands.line_mode() == crate::command::LineMode::PhysicalSource {
            let source = self.document.projection().source_insertion_point(row.text_range.start, true)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let index = self.document.physical_line_index_at_source(source)?;
            self.document.physical_line_projected_extent(index).ok_or(DocumentError::AmbiguousProjection)?
        } else if let Some(cell) = &row.table_cell {
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
        if view.commands.line_mode() != crate::command::LineMode::PhysicalSource && range.end < text.byte_len()
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
        core.command_state(view).unwrap().line_selection_range(core.document(),
            core.layout(view).unwrap().snapshot()).unwrap()
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
            assert_eq!(core.command_state(view).unwrap().cursor(), 11);
            row(&mut core, view, 1, true);
            assert_eq!(selected(&core, view), 6..13);
            assert_eq!(core.command_state(view).unwrap().cursor(), 11);
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::VisualLine);
            assert!(!core.command_state(view).unwrap().is_native_selection());
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
        assert!(core.table_selection(view).unwrap().is_some());
        assert_eq!(core.command_state(view).unwrap().cursor(), expected.end - 1);
    }

    #[test]
    fn gutter_table_rows_keep_the_final_empty_cell_caret_and_reversal() {
        let document = Document::from_bytes(b"| a | b |\n| - | - |\n| one |  |\n| two |  |".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let indices = [1, 2, 0].map(|target| snapshot.rows.iter().position(|row|
            row.table_cell.as_ref().is_some_and(|cell| cell.row == target)).unwrap());
        let expected = core.document().projection().tables()[0].rows[1].cells[1].range.start;
        row(&mut core, view, indices[0], false);
        assert_eq!(core.command_state(view).unwrap().cursor(), expected);
        row(&mut core, view, indices[1], true);
        row(&mut core, view, indices[2], true);
        assert_eq!(core.table_selection(view).unwrap().unwrap().rows(), 0..2);
        assert_eq!(core.command_state(view).unwrap().cursor(), expected);
    }

    #[test]
    fn gutter_matches_v_line_domains_and_command_selection_policy() {
        use crate::command::LineMode;
        for format in [Format::PlainText, Format::Code, Format::Markdown] {
            for line_mode in [LineMode::Visual, LineMode::PhysicalSource] {
                for selectmode in ["", "mouse", "cmd"] {
                    let document = Document::from_bytes(b"first words wrap across many rows\n\nlast".to_vec(), Encoding::Utf8, format).unwrap();
                    let mut core = Core::new(document);
                    let view = core.add_view(MockTextMeasurementProvider::new(), 130., 600.);
                    core.handle(view, CoreEvent::SetLineMode(line_mode)).unwrap();
                    assert!(core.set_selection_option(false, selectmode));
                    let at = core.layout(view).unwrap().snapshot().unwrap().rows[1].text_range.start;
                    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                        text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('V'))).unwrap();
                    let expected = selected(&core, view);
                    let select = core.command_state(view).unwrap().is_select_mode();
                    row(&mut core, view, 1, false);
                    assert_eq!(selected(&core, view), expected, "{format:?} {line_mode:?} {selectmode}");
                    assert_eq!(core.command_state(view).unwrap().mode(), Mode::VisualLine);
                    assert_eq!(core.command_state(view).unwrap().is_select_mode(), select);
                    assert!(!core.command_state(view).unwrap().is_native_selection());
                }
            }
        }
    }

    #[test]
    fn gutter_final_caret_uses_graphemes_and_empty_row_boundaries() {
        let mut core = Core::new(Document::new("a e\u{301} 👩‍💻\n\nlast"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        row(&mut core, view, 0, false);
        assert_eq!(core.command_state(view).unwrap().cursor(), "a e\u{301} ".len());
        row(&mut core, view, 1, true);
        let empty = "a e\u{301} 👩‍💻\n".len();
        assert_eq!(core.command_state(view).unwrap().cursor(), empty);
        assert_eq!(selected(&core, view), 0..empty + 1);
    }

    #[test]
    fn row_selection_margins_include_list_markers_and_reach_tables() {
        for source in ["- first item wraps over many many many many words\n  - nested item", "| head | other |\n| --- | --- |\n| body | last |"] {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 160., 300.);
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let requests = core.views[&view].engine.provider().request_calls();
            for row in snapshot.rows.iter() {
                let y = row.y + row.height() / 2.;
                let edge = row.table_cell.as_ref().map_or(row.paragraph_content_x - 2., |cell| cell.table_x + 1.);
                assert!(snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: edge - 0.1, y }).unwrap());
                assert!(!snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: edge, y }).unwrap());
                assert!(!snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: row.paragraph_content_x - 1., y }).unwrap());
            }
            assert_eq!(core.views[&view].engine.provider().request_calls(), requests);
        }
    }

    #[test]
    fn right_to_left_list_labels_are_row_selection_targets_without_claiming_right_margin() {
        let document = Document::from_bytes("9. שלום עולם".as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 300.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let visual = &snapshot.rows[0];
        let label = visual.decorations.iter().find(|item| item.kind == crate::layout::DecorationKind::Text).unwrap();
        let bounds = label.typographic_bounds;
        assert!(bounds.x >= visual.paragraph_content_x + visual.paragraph_content_width);
        let point = crate::layout::LayoutPoint { x: bounds.x + bounds.width / 2., y: bounds.y + bounds.height / 2. };
        assert!(snapshot.row_selection_hit_test(point).unwrap());
        let after_label = visual.decorations.iter().filter(|item| item.kind == crate::layout::DecorationKind::Text)
            .map(|item| item.typographic_bounds.x + item.typographic_bounds.width).fold(0_f32, f32::max) + 1.;
        assert!(!snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: after_label, y: point.y }).unwrap());
        let expected = visual.text_range.clone();
        row(&mut core, view, 0, false);
        assert_eq!(selected(&core, view), expected);
    }

    #[test]
    fn gutter_enters_visual_line_from_visual_and_select_mouse_policies() {
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
            assert!(!core.command_state(view).unwrap().is_select_mode());
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::VisualLine);
            row(&mut core, view, 0, true);
            assert_eq!(core.list_selection_identity(view).unwrap().range(), 0..8);
            assert_eq!(core.command_state(view).unwrap().cursor(), 6);
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
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let visual = &snapshot.rows[0];
        let y = visual.y + visual.height() / 2.;
        for _ in 0..1000 {
            assert!(snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: visual.paragraph_content_x - 3., y }).unwrap());
            assert!(!snapshot.row_selection_hit_test(crate::layout::LayoutPoint { x: 200_000., y }).unwrap());
        }
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
        assert!(matches!(core.layout(view).unwrap().snapshot().unwrap()
            .row_selection_hit_test(crate::layout::LayoutPoint { x: -100., y: 1_000_000. }),
            Err(LayoutError::OutsideMaterializedCoverage)));
        for index in 0..100 {
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let visual = &snapshot.rows[index % 5];
            assert!(snapshot.row_selection_hit_test(crate::layout::LayoutPoint {
                x: visual.paragraph_content_x - 3., y: visual.y + visual.height() / 2.,
            }).unwrap());
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
