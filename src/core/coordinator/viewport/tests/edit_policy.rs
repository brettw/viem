use super::*;

fn code_padding_defaults(document: &Document, top: u32) -> Vec<u8> {
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let code = defaults["block_styles"]
        .as_array_mut().unwrap().iter_mut()
        .find(|style| style["id"] == "Code Block").unwrap();
    code["block"]["padding_top"] = top.into();
    code["block"]["padding_bottom"] = 4.into();
    serde_json::to_vec(&defaults).unwrap()
}

fn block_fixture(middle: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let source = format!("{}Landmark\n\nSpacer\n\n{middle}\n\n{}",
        "Before\n\n".repeat(40), "After\n\n".repeat(40));
    let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let defaults = code_padding_defaults(&document, 36);
    document.initialize_style_defaults(&defaults).unwrap();
    let mut core = Core::new(document);
    core.set_syntax_provider_factory(Arc::new(|| Box::new(DeferredProvider)));
    let at = core.document.projection().blocks().iter()
        .find(|block| block.style.0 == "Code Block")
        .map(|block| block.range.start)
        .unwrap_or_else(|| core.document.text().find("Target").unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    let landmark = core.document.text().find("Landmark").unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let geometry = snapshot.logical_endpoint_geometry(landmark, BoundaryAffinity::Downstream).unwrap();
    let top = snapshot.rows[geometry.row_index].baseline - 40.;
    core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
    assert!(core.layout(view).unwrap().viewport_top() > 0., "fixture must be scrolled");
    assert_caret_visible(&core, view);
    (core, view)
}

fn landmark_baseline(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> f32 {
    let at = core.document.text().find("Landmark").unwrap();
    let layout = core.layout(view).unwrap();
    let snapshot = layout.snapshot().unwrap();
    let geometry = snapshot.logical_endpoint_geometry(at, BoundaryAffinity::Downstream).unwrap();
    snapshot.rows[geometry.row_index].baseline - layout.viewport_top()
}

fn assert_caret_visible(core: &Core<MockTextMeasurementProvider>, view: ViewId) {
    let row = caret_row(core, view);
    let layout = core.layout(view).unwrap();
    assert!(row.reveal_bounds().start >= layout.viewport_top() - 0.05);
    assert!(row.reveal_bounds().end <= layout.viewport_top() + layout.height() + 0.05);
}

fn assert_landmark_stationary(core: &Core<MockTextMeasurementProvider>, view: ViewId, expected: f32) {
    let actual = landmark_baseline(core, view);
    assert!((actual - expected).abs() < 0.05,
        "unchanged content above the edit moved from {expected} to {actual}");
    assert_caret_visible(core, view);
}

#[test]
fn menu_code_block_creation_and_removal_keep_surrounding_text_stationary() {
    let (mut core, view) = block_fixture("Target");
    let landmark = landmark_baseline(&core, view);
    let plain_baseline = baseline(&core, view);
    for style in ["Code Block", "Paragraph"] {
        let before = core.document.source_bytes();
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(view, CoreEvent::SetParagraphStyle { expected, style: style.into() }).unwrap();
        let after = core.document.source_bytes();
        assert_ne!(before, after);
        assert_landmark_stationary(&core, view, landmark);
        if style == "Code Block" {
            assert!(baseline(&core, view) > plain_baseline + 20.,
                "new block padding should move the caret naturally, without scrolling");
        } else {
            assert!((baseline(&core, view) - plain_baseline).abs() < 0.05);
        }
        for (request, bytes) in [
            (HistoryNavigationRequest::Undo, &before),
            (HistoryNavigationRequest::Redo, &after),
        ] {
            core.handle(view, CoreEvent::NavigateHistory(request)).unwrap();
            assert_eq!(core.document.source_bytes(), *bytes);
            assert_landmark_stationary(&core, view, landmark);
        }
    }
}

#[test]
fn backspace_in_an_empty_code_block_removes_padding_without_scrolling() {
    check_empty_code_backspace(false);
}

#[test]
fn an_ordinary_edit_retires_a_pending_syntax_baseline_anchor() {
    check_empty_code_backspace(true);
}

fn check_empty_code_backspace(pending_syntax: bool) {
    let (mut core, view) = block_fixture("```\n```");
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    let before = core.document.source_bytes();
    let landmark = landmark_baseline(&core, view);
    let code_baseline = baseline(&core, view);
    if pending_syntax {
        // A current syntax publication captured its baseline, but its layout
        // has not been installed before the next structural edit arrives.
        let anchor = capture_caret_baseline_anchor(&core.document, &core.views[&view]).unwrap();
        core.views.get_mut(&view).unwrap().viewport_anchor = Some(anchor);
    }
    let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Backspace))).unwrap();
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
    assert!(core.document.projection().blocks().iter().all(|block| block.style.0 != "Code Block"));
    assert_landmark_stationary(&core, view, landmark);
    assert!(baseline(&core, view) < code_baseline - 20.,
        "removing padding should move the caret up, without moving unchanged text");
    let after = core.document.source_bytes();
    for (request, bytes) in [
        (HistoryNavigationRequest::Undo, &before),
        (HistoryNavigationRequest::Redo, &after),
    ] {
        core.handle(view, CoreEvent::NavigateHistory(request)).unwrap();
        assert_eq!(core.document.source_bytes(), *bytes);
        assert_landmark_stationary(&core, view, landmark);
    }
}

#[test]
fn explicit_style_defaults_change_preserves_viewport_instead_of_caret_baseline() {
    let (mut core, view) = block_fixture("```\nTarget\n```");
    let source = core.document.source_bytes();
    let landmark = landmark_baseline(&core, view);
    let before = baseline(&core, view);
    let defaults = code_padding_defaults(&core.document, 72);
    core.replace_style_defaults(&defaults).unwrap();
    core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport).unwrap();
    assert_eq!(core.document.source_bytes(), source);
    assert_landmark_stationary(&core, view, landmark);
    assert!((baseline(&core, view) - before - 36.).abs() < 0.05,
        "the explicit padding change should move the caret by the padding difference");
}

#[test]
fn explicit_code_sheet_change_supersedes_pending_syntax_baseline_even_with_completed_analysis() {
    use crate::document::{FontSize, StyleDefinitionEdit, StyleSheetRevision};
    use crate::document::syntax::service::PublicationDelta;

    for completed_analysis in [false, true] {
        let source = format!("{}Landmark\nSpacer\nEditing\n{}",
            "Before\n".repeat(40), "After\n".repeat(40));
        let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Code).unwrap();
        let mut core = Core::new(document);
        core.set_syntax_provider_factory(Arc::new(|| Box::new(DeferredProvider)));
        core.set_code_language(crate::document::syntax::detection::LanguageSelection::None);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        let at = core.document.text().find("Editing").unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document.revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let landmark = core.document.text().find("Landmark").unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let geometry = snapshot.logical_endpoint_geometry(landmark, BoundaryAffinity::Downstream).unwrap();
        let top = snapshot.rows[geometry.row_index].y;
        core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap();
        let ordinary_anchor = core.views[&view].viewport_anchor.unwrap();
        assert_eq!(ordinary_anchor.reference, ViewportAnchorReference::RowTop);
        assert_eq!(ordinary_anchor.anchor.offset(), landmark);
        let pending = capture_caret_baseline_anchor(&core.document, &core.views[&view]).unwrap();
        assert_ne!(pending.anchor.offset(), landmark);
        core.views.get_mut(&view).unwrap().viewport_anchor = Some(pending);

        let mut sheet = core.document.projection().style_sheet().clone();
        let mut base = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        base.character.size = Some(FontSize::Points(32.));
        assert!(sheet.apply_configuration_edit(
            &StyleDefinitionEdit::UpdateBlock(base),
            StyleSheetRevision(sheet.revision.0 + 1), false,
        ).unwrap());
        let source = core.document.source_bytes();
        let history = core.document.history_status();
        core.publish_code_presentation(Arc::new(sheet), &PublicationDelta {
            unbounded: true, ..Default::default()
        }, completed_analysis);

        // Inspect the real publication boundary without polling again: the
        // live global sheet would overwrite this isolated configuration edit.
        assert_eq!(core.views[&view].viewport_anchor, Some(ordinary_anchor),
            "an explicit style choice must retire pending syntax compensation; completed={completed_analysis}");
        assert_eq!(core.layout(view).unwrap().viewport_top(), top);
        assert_eq!(core.document.source_bytes(), source);
        assert_eq!(core.document.history_status(), history);
    }
}

#[test]
fn editing_above_another_view_preserves_its_text_position_not_numeric_offset() {
    let (mut core, view) = block_fixture("Target");
    let landmark = landmark_baseline(&core, view);
    let top = core.layout(view).unwrap().viewport_top();
    let other = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let outcome = core.handle_with_layout(other, CoreEvent::Input(InputEvent::key('O'))).unwrap();
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
    core.handle_with_layout(other, CoreEvent::Input(InputEvent::Text("Inserted paragraph".into()))).unwrap();
    core.handle_with_layout(other, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    assert_landmark_stationary(&core, view, landmark);
    assert!(core.layout(view).unwrap().viewport_top() > top,
        "the offset must follow inserted content above the preserved viewport anchor");
}
