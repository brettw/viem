use super::*;

fn mermaid_fixture(format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let source = format!("{}\n```mermaid\ngraph LR\n    Writing --> Editing\n    Editing --> Saving\n```\n\n{}",
        "Before\n\n".repeat(100), "After\n\n".repeat(100));
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 240.);
    let at = core.document.text().find("Writing").unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    let top = caret_row(&core, view).baseline - 120.;
    core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
    (core, view)
}

#[test]
fn history_keeps_a_visible_code_block_stationary() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for native in [true, false] {
            let (mut core, view) = mermaid_fixture(format);
            let expected = core.list_selection_identity(view).unwrap();
            // Reproduce history created before the toolbar disallowed this
            // operation; semantic containers still support imported quotes.
            core.handle(view, CoreEvent::SetBlockQuote { expected, enabled: true }).unwrap();
            let before = baseline(&core, view);
            for redo in [false, true] {
                let event = if native {
                    CoreEvent::NavigateHistory(if redo { HistoryNavigationRequest::Redo } else { HistoryNavigationRequest::Undo })
                } else {
                    CoreEvent::Input(InputEvent::Key(if redo { Key::Ctrl('r') } else { Key::Char('u') }))
                };
                core.handle(view, event).unwrap();
                assert!((baseline(&core, view) - before).abs() < 0.01,
                    "{format:?}, native={native}, redo={redo}: before={before}, after={}", baseline(&core, view));
            }
        }
    }
}

#[test]
fn history_reveals_the_whole_fitting_change_with_minimal_scrolling() {
    for native in [true, false] {
        let (mut core, view) = mermaid_fixture(Format::Markdown);
        let start = core.document.text().find("graph LR").unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: start,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(view, CoreEvent::SetBlockQuote { expected, enabled: true }).unwrap();
        let top = caret_row(&core, view).baseline - 220.;
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
        let before = baseline(&core, view);
        let event = if native { CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo) }
            else { CoreEvent::Input(InputEvent::Key(Key::Char('u'))) };
        core.handle(view, event).unwrap();
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        let last = core.document.text().find("Editing --> Saving").unwrap();
        let row = snapshot.rows.iter().find(|row| row.text_range.contains(&last)).unwrap();
        assert!((row.reveal_bounds().end - layout.viewport_top() - layout.height()).abs() < 0.01,
            "the final changed row should just fit: native={native}, bottom={}, top={}, height={}", row.reveal_bounds().end, layout.viewport_top(), layout.height());
        assert!(baseline(&core, view) < before);
        assert!(caret_row(&core, view).reveal_bounds().start >= layout.viewport_top());
    }
}

#[test]
fn distant_history_keeps_work_bounded_and_prioritizes_the_caret_for_a_tall_change() {
    let source = format!("{}\n```\n{}\n```\n\n{}", "Before\n\n".repeat(10_000),
        "long code block line\n".repeat(500), "After\n\n".repeat(10_000));
    let mut core = Core::new(Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 240.);
    let at = core.document.text().find("long code block line").unwrap() + "long code block line\n".len() * 250;
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::SetBlockQuote { expected, enabled: true }).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: 0,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    let calls = core.views[&view].engine.provider().request_calls();
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
    let layout = core.layout(view).unwrap();
    let row = caret_row(&core, view);
    assert!(row.reveal_bounds().start >= layout.viewport_top());
    assert!(row.reveal_bounds().end <= layout.viewport_top() + layout.height());
    assert!(core.views[&view].engine.provider().request_calls() - calls < 1000,
        "history reveal must only shape viewport-sized regions");
}

#[test]
fn undo_of_visible_inline_formatting_does_not_reveal_unrelated_paragraph_rows() {
    let source = format!("{}first line\\\nsecond **word** tail\\\nthird line\n\n{}",
        "Before\n\n".repeat(100), "After\n\n".repeat(100));
    let mut core = Core::new(Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 240.);
    let at = core.document.text().find("word").unwrap();
    for (offset, extend) in [(at, false), (at + 4, true)] {
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: offset,
            affinity: BoundaryAffinity::Downstream, extend_selection: extend,
        }).unwrap();
    }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::SetSelectionSemanticStyle {
        expected, style: SemanticInlineStyle::Strong, enabled: false,
    }).unwrap();
    let row = caret_row(&core, view);
    let top = row.reveal_bounds().end - 240.;
    core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
    let before = baseline(&core, view);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert!((baseline(&core, view) - before).abs() < 0.01);
}
