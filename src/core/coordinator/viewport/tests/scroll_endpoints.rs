use super::*;

fn completed_key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete,
        "{key:?} must complete after the host services layout demand");
}

fn scroll_to_edge(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key, down: bool) -> f32 {
    let mut settled = 0;
    for _ in 0..300 {
        let before = core.layout(view).unwrap().viewport_top();
        let old_cursor = core.command_state(view).unwrap().cursor();
        completed_key(core, view, key);
        let after = core.layout(view).unwrap().viewport_top();
        assert!(
            if down { after + 0.05 >= before } else { after <= before + 0.05 },
            "{key:?} reversed scrolling from {before} to {after}"
        );
        if (after - before).abs() < 0.05 && core.command_state(view).unwrap().cursor() == old_cursor {
            settled += 1;
            if settled == 4 {
                return after;
            }
        } else {
            settled = 0;
        }
    }
    panic!("{key:?} did not reach a stable endpoint");
}

fn assert_scrollbar_edge(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, down: bool) {
    let key = if down { Key::PageDown } else { Key::PageUp };
    assert_key_scrollbar_edge(core, view, key, down);
}

fn assert_key_scrollbar_edge(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key, down: bool) {
    let paged_top = scroll_to_edge(core, view, key, down);
    core.handle(view, CoreEvent::SetViewportOrigin {
        left: 0., top: Some(if down { 1_000_000_000. } else { 0. }),
    }).unwrap();
    let absolute_top = core.layout(view).unwrap().viewport_top();
    assert!((paged_top - absolute_top).abs() < 0.05,
        "{key:?} endpoint {paged_top} must match scrollbar endpoint {absolute_top}");
    // The cursor already occupies its endpoint, so the next page cannot rely
    // on cursor movement to repair a different scroll limit.
    assert!((scroll_to_edge(core, view, key, down) - absolute_top).abs() < 0.05);
}

#[test]
fn half_page_and_row_scrolling_share_margin_inclusive_scrollbar_endpoints() {
    for (down, up) in [(Key::Ctrl('d'), Key::Ctrl('u')), (Key::Ctrl('e'), Key::Ctrl('y'))] {
        let source = "# Timer\n\n".to_owned()
            + &"A paragraph with enough words to wrap on the narrow view.\n\n".repeat(12)
            + "## Final heading";
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 240., 140.);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top: 17., bottom: 29., ..Default::default()
        }).unwrap();
        assert_key_scrollbar_edge(&mut core, view, down, true);
        assert_key_scrollbar_edge(&mut core, view, up, false);
    }
}

#[test]
fn paging_after_a_scrollbar_move_does_not_reverse_to_reveal_the_old_caret() {
    for (key, down) in [(Key::PageDown, true), (Key::PageUp, false)] {
        let mut core = Core::new(Document::new("line\n".repeat(80)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240., 140.);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top: 17., bottom: 29., ..Default::default()
        }).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(),
            text_offset: if down { 0 } else { core.document.text().len() },
            affinity: if down { BoundaryAffinity::Downstream } else { BoundaryAffinity::Upstream },
            extend_selection: false,
        }).unwrap();
        let old_cursor = core.command_state(view).unwrap().cursor();
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(450.) }).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), old_cursor,
            "a scrollbar move must leave the old caret outside the new viewport");
        let before = core.layout(view).unwrap().viewport_top();
        completed_key(&mut core, view, key);
        let after = core.layout(view).unwrap().viewport_top();
        assert!(if down { after > before + 0.05 } else { after + 0.05 < before },
            "{key:?} must advance from scrolled viewport {before}, got {after}");
        let row = caret_row(&core, view);
        let layout = core.layout(view).unwrap();
        assert!(row.reveal_bounds().start >= layout.viewport_top() - 0.05);
        assert!(row.reveal_bounds().end <= layout.viewport_top() + layout.height() + 0.05,
            "the page command must choose a caret in its new viewport");
    }
}

#[test]
fn counted_viewport_alignment_keeps_its_requested_position_with_margins() {
    let mut core = Core::new(Document::new("line\n".repeat(40)));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 96.);
    core.set_view_insets(view, crate::layout::EdgeInsets {
        top: 17., bottom: 29., ..Default::default()
    }).unwrap();
    for alignment in ['t', 'z', 'b'] {
        let mut aligned_top = None;
        for _ in 0..3 {
            for (index, key) in ['1', '2', 'z', alignment].into_iter().enumerate() {
                let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char(key)))).unwrap();
                assert_eq!(outcome.command.unwrap().status,
                    if index == 3 { CommandStatus::Complete } else { CommandStatus::Pending },
                    "counted alignment must complete after its final key");
            }
            let row = caret_row(&core, view);
            let layout = core.layout(view).unwrap();
            assert_eq!(row.hard_line_index, 11, "the count selects hard line 12");
            let visible = layout.snapshot().unwrap().reveal_vertical_range(&row, layout.height());
            assert_eq!(visible, 17.0..67.0, "both configured margins remain reserved");
            let bounds = row.reveal_bounds();
            let screen_start = bounds.start - layout.viewport_top();
            let screen_end = bounds.end - layout.viewport_top();
            match alignment {
                't' => assert!((screen_start - visible.start).abs() < 0.05,
                    "zt must align the full row below the top margin"),
                'z' => assert!((screen_start + screen_end - visible.start - visible.end).abs() < 0.05,
                    "zz must center the full row between the margins"),
                'b' => assert!((screen_end - visible.end).abs() < 0.05,
                    "zb must align the full row above the bottom margin"),
                _ => unreachable!(),
            }
            if let Some(previous) = aligned_top {
                assert_eq!(layout.viewport_top(), previous,
                    "repeating a counted alignment must not depend on whether its cursor moves");
            }
            aligned_top = Some(layout.viewport_top());
        }
    }
}

#[test]
fn explicit_paging_preserves_scrollbar_extent_when_trailing_space_exceeds_the_viewport() {
    for (format, source, bottom) in [
        (Format::PlainText, "line\n".repeat(20) + "last", 200.),
        (Format::Markdown, "line\n\n".repeat(20) + "# last", 29.),
    ] {
        let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        if format == Format::Markdown {
            use crate::document::{ConfigurationStyleIntent, StyleDefinitionEdit, StyleModelIntent, StyleModelRequest};
            let mut heading = document.projection().style_sheet().block_style(&"Heading1".into()).unwrap().clone();
            heading.block.margin_bottom = Some(200.);
            document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(heading))))).unwrap();
        }
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 80.);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top: 17., bottom, ..Default::default()
        }).unwrap();
        assert_scrollbar_edge(&mut core, view, true);
        let layout = core.layout(view).unwrap();
        let last = layout.snapshot().unwrap().rows.last().unwrap();
        assert!(last.reveal_bounds().end < layout.viewport_top(),
            "fixture must scroll past the final row to display its trailing space");
        assert_scrollbar_edge(&mut core, view, false);
    }
}

#[test]
fn paging_and_scrollbar_share_stable_endpoints_with_document_margins() {
    for (format, wrap, mode, top, bottom, ending) in [
        (Format::Markdown, true, None, 17., 29., "A final paragraph with wrapped words at the document end."),
        (Format::Markdown, false, Some('i'), 24., 24., "## Final heading"),
        (Format::PlainText, true, Some('R'), 0., 28., "last line\n"),
    ] {
        let source = format!("# Timer\n\n{}{}",
            "## A section\n\nAn ordinary paragraph with several words that wrap across the narrow text view.\n\n".repeat(6),
            ending);
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 280., 140.);
        core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top, bottom, ..Default::default()
        }).unwrap();
        if let Some(mode) = mode {
            completed_key(&mut core, view, Key::Char(mode));
        }
        assert_scrollbar_edge(&mut core, view, true);
        let layout = core.layout(view).unwrap();
        let last = layout.snapshot().unwrap().rows.last().unwrap();
        assert!(last.reveal_bounds().end <= layout.viewport_top() + layout.height() - bottom + 0.05,
            "the final row must remain above the configured bottom margin");
        assert_scrollbar_edge(&mut core, view, false);
        assert_eq!(core.layout(view).unwrap().viewport_top(), 0.);
    }
}

#[test]
fn paging_a_document_shorter_than_the_viewport_never_scrolls_its_margins_away() {
    for text in ["", "# Timer\n\nA short paragraph."] {
        let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        core.set_view_insets(view, crate::layout::EdgeInsets {
            top: 17., bottom: 29., ..Default::default()
        }).unwrap();
        for down in [true, false, true] {
            assert_scrollbar_edge(&mut core, view, down);
            assert_eq!(core.layout(view).unwrap().viewport_top(), 0.);
        }
    }
}

#[test]
fn regional_paging_endpoints_follow_width_metrics_and_margin_invalidation() {
    let paragraph = "Ordinary words wrap across the narrow text view and keep the final paragraph taller than one row.\n\n";
    let document = Document::from_bytes(
        ("# Timer\n\n".to_owned() + &paragraph.repeat(20_000)).into_bytes(),
        Encoding::Utf8, Format::Markdown,
    ).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 240.);
    core.set_view_insets(view, crate::layout::EdgeInsets {
        top: 17., bottom: 29., ..Default::default()
    }).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(),
        text_offset: core.document.text().len() - paragraph.len(),
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    for invalidation in 0..4 {
        let calls = core.views[&view].engine.provider().request_calls();
        match invalidation {
            1 => { core.handle(view, CoreEvent::Resize { width: 260., height: 180. }).unwrap(); }
            2 => {
                core.views.get_mut(&view).unwrap().engine.provider_mut()
                    .set_metrics_generation(MetricsGeneration(2));
            }
            3 => {
                core.set_view_insets(view, crate::layout::EdgeInsets {
                    top: 31., bottom: 43., ..Default::default()
                }).unwrap();
            }
            _ => {}
        }
        // Prefix height estimates may legitimately change when width or the
        // metrics generation changes. Finish that geometry replacement before
        // testing the direction of subsequent explicit scroll commands.
        core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport).unwrap();
        assert_scrollbar_edge(&mut core, view, true);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        assert!(snapshot.coverage.hard_lines().len() < 128,
            "reaching an endpoint must retain regional layout");
        assert!(core.views[&view].engine.provider().request_calls() - calls < 256,
            "invalidating endpoint geometry must not measure the whole document");
        if invalidation >= 2 {
            assert_eq!(snapshot.metrics_generation, MetricsGeneration(2));
        }
    }
}
