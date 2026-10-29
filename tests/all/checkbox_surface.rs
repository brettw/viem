use viem_core::document::{Encoding, Format, HistoryNavigationRequest, ListStyle};
use viem_core::layout::{BoundaryAffinity, DecorationKind, LayoutEngine, MockTextMeasurementProvider, ViewLayout};
use viem_core::{Core, CoreEvent, Document};

fn markdown(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

#[test]
fn checkbox_geometry_is_square_between_baseline_and_native_cap_height() {
    let document = markdown("- [ ] ordinary **bold** text that wraps across several rows\n- [X] done\n- [ ] ");
    for scale in [0.75, 1.0, 2.0] {
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(150., 500.);
        view.set_scale(scale).unwrap();
        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap();
        let mut checked_states = Vec::new();
        for row in snapshot.rows.iter() {
            for decoration in &row.decorations {
                let DecorationKind::Checkbox { checked } = decoration.kind else { continue; };
                checked_states.push(checked);
                let rect = decoration.typographic_bounds;
                assert_eq!(rect.width, rect.height);
                assert!((rect.y + rect.height - row.baseline).abs() < 0.0001);
                assert!((rect.height - decoration.font_size * 0.7).abs() < 0.0001);
                assert!(rect.x + rect.width < row.paragraph_content_x);
                assert_eq!(row.fragment_index, 0);
                assert!(decoration.text.is_empty());
            }
        }
        assert_eq!(checked_states, [false, true, false]);
    }
}

#[test]
fn native_checkbox_actions_preserve_caret_and_exact_history() {
    let mut core = Core::new(markdown("- **task**\n- next"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    assert!(core.can_set_task_list(view).unwrap());
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::SetListStyle { expected, style: Some(ListStyle::Task) }).unwrap();
    assert_eq!(core.document().source_bytes(), b"- [ ] **task**\n- next");
    assert!(core.can_set_task_list(view).unwrap());
    let selected = core.selected_named_styles(view).unwrap();
    assert!(selected.has_tasks && !selected.has_bullets && !selected.has_numbering);
    let (block_id, checked) = core.document().checkbox_at(0, BoundaryAffinity::Downstream).unwrap();
    assert!(!checked);
    core.handle(view, CoreEvent::ToggleCheckbox { document: core.document().id(),
        revision: core.document().revision(), at: 0, block_id }).unwrap();
    assert_eq!(core.document().source_bytes(), b"- [x] **task**\n- next");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), b"- [ ] **task**\n- next");
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), b"- **task**\n- next");
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), b"- [ ] **task**\n- next");
}

#[test]
fn stale_task_list_action_does_not_change_source_or_cursor() {
    let mut core = Core::new(markdown("first\n\nsecond"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: 2, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    assert!(core.handle(view, CoreEvent::SetListStyle { expected, style: Some(ListStyle::Task) }).is_err());
    assert_eq!(core.document().source_bytes(), b"first\n\nsecond");
    assert_eq!(core.command_state(view).unwrap().cursor(), 2);
}

#[test]
fn failed_checkbox_action_preserves_active_style_gesture() {
    let mut core = Core::new(markdown("ordinary"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let gesture = core.begin_style_edit_group(view, core.document().id(), core.document().revision(),
        core.document().projection().style_sheet().revision).unwrap();
    assert!(core.handle(view, CoreEvent::ToggleCheckbox { document: core.document().id(),
        revision: core.document().revision(), at: 0, block_id: u64::MAX }).is_err());
    core.end_style_edit_group(view, gesture).unwrap();
    assert_eq!(core.document().source_bytes(), b"ordinary");
}

#[test]
fn failed_checkbox_action_preserves_completion_preview() {
    use viem_core::command::{InputEvent, Key};
    let mut core = Core::new(markdown("alphabet\n\nal"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
    let end = core.document().text().len();
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: end, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('p')))).unwrap();
    for _ in 0..20_000 {
        if !core.completion_presentation(view).unwrap().is_some_and(|menu| menu.searching) { break; }
        core.poll_completion(view).unwrap();
    }
    assert!(core.completion_presentation(view).unwrap().is_some());
    assert!(core.handle(view, CoreEvent::ToggleCheckbox { document: core.document().id(),
        revision: core.document().revision(), at: 0, block_id: u64::MAX }).is_err());
    assert!(core.completion_presentation(view).unwrap().is_some());
    assert_eq!(core.document().source_bytes(), b"alphabet\n\nal");
}

#[test]
fn toggle_refreshes_cached_checkbox_decoration_without_changing_geometry() {
    let mut core = Core::new(markdown("- [ ] task"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let bounds = core.layout(view).unwrap().snapshot().unwrap().rows[0].decorations.iter()
        .find(|item| matches!(item.kind, DecorationKind::Checkbox { .. })).unwrap().typographic_bounds;
    let (block_id, _) = core.document().checkbox_at(0, BoundaryAffinity::Downstream).unwrap();
    core.handle(view, CoreEvent::ToggleCheckbox { document: core.document().id(),
        revision: core.document().revision(), at: 0, block_id }).unwrap();
    let updated = core.layout(view).unwrap().snapshot().unwrap().rows[0].decorations.iter()
        .find(|item| matches!(item.kind, DecorationKind::Checkbox { .. })).unwrap();
    assert_eq!(updated.kind, DecorationKind::Checkbox { checked: true });
    assert_eq!(updated.typographic_bounds, bounds);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut fresh = ViewLayout::new(600., 400.);
    engine.relayout(core.document(), &mut fresh).unwrap();
    let fresh_box = fresh.snapshot().unwrap().rows[0].decorations.iter()
        .find(|item| matches!(item.kind, DecorationKind::Checkbox { .. })).unwrap();
    assert_eq!(updated, fresh_box);
}

#[test]
fn checkbox_ffi_rejects_stale_geometry_and_invalid_storage() {
    use viem_core::ffi::*;
    use std::ptr;
    let fixture = super::ffi_core_surface::table_ffi_fixture(b"task", VIEM_FORMAT_MARKDOWN);
    let (core, view) = (fixture.handle, fixture.view);
    let mut selection = ViemLogicalSelectionIdentityV1::default();
    let mut outcome = ViemCoreOutcomeV1::default();
    unsafe {
        assert_eq!(viem_core_view_list_selection(core, view, ptr::null_mut()), ViemStatus::NullPointer);
        assert_eq!(viem_core_view_list_selection(core, view, &mut selection), ViemStatus::Ok);
        let request = ViemSetListStyleV1 { struct_size: VIEM_SET_LIST_STYLE_V1_SIZE,
            style: VIEM_LIST_STYLE_TASK, expected_selection: selection };
        assert_eq!(viem_core_view_set_list_style(core, view, &request, &mut outcome), ViemStatus::Ok);
        assert_ne!(viem_core_view_set_list_style(core, view, &request, &mut outcome), ViemStatus::Ok);
        let mut snapshot = ViemLayoutSnapshotInfoV1::default();
        assert_eq!(viem_core_view_layout_snapshot_info(core, view, &mut snapshot), ViemStatus::Ok);
        let mut info = ViemLayoutDecorationsInfoV1::default();
        assert_eq!(viem_core_view_copy_layout_decorations(core, view, &snapshot.identity,
            ptr::null_mut(), 0, ptr::null_mut(), 0, &mut info), ViemStatus::BufferTooSmall);
        let mut decorations = vec![ViemLayoutDecorationV1::default(); info.decoration_count as usize];
        let mut labels = vec![0u8; info.label_bytes as usize];
        assert_eq!(viem_core_view_copy_layout_decorations(core, view, &snapshot.identity,
            decorations.as_mut_ptr(), decorations.len() as u64, labels.as_mut_ptr(), labels.len() as u64,
            &mut info), ViemStatus::Ok);
        let checkbox = decorations.iter().find(|item| item.flags & VIEM_LAYOUT_DECORATION_CHECKBOX != 0).unwrap();
        assert_eq!(viem_core_view_toggle_checkbox(core, view, &snapshot.identity, checkbox.paragraph_id, &mut outcome), ViemStatus::Ok);
        assert_ne!(viem_core_view_toggle_checkbox(core, view, &snapshot.identity, checkbox.paragraph_id, &mut outcome), ViemStatus::Ok);
    }
}

#[test]
fn task_list_availability_obeys_editability_and_ignores_save_only_readonly() {
    use viem_core::command::{InputEvent, Key};
    let mut core = Core::new(markdown("ordinary"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    for (command, available) in [("set readonly", true), ("set nomodifiable", false), ("set modifiable", true)] {
        core.stage_command_line(view, command).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert_eq!(core.can_set_task_list(view).unwrap(), available, "{command}");
        if !available {
            let expected = core.list_selection_identity(view).unwrap();
            let outcome = core.handle(view, CoreEvent::SetListStyle { expected, style: Some(ListStyle::Task) }).unwrap();
            let message = format!("{outcome:?}");
            assert!(message.contains("E21") || message.contains("NotModifiable"));
            assert_eq!(core.document().source_bytes(), b"ordinary");
        }
    }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::SetListStyle { expected, style: Some(ListStyle::Task) }).unwrap();
    assert_eq!(core.document().source_bytes(), b"- [ ] ordinary");
}

#[test]
fn selected_style_ffi_reports_exclusive_task_state_and_local_eligibility() {
    use viem_core::ffi::*;
    for (source, task, available) in [
        ("ordinary", false, true), ("3. [X] task", true, true),
        ("- [ ] task", true, true), ("# heading", false, false),
        ("```\ncode\n```", false, false),
    ] {
        let fixture = super::ffi_core_surface::table_ffi_fixture(source.as_bytes(), VIEM_FORMAT_MARKDOWN);
        let mut selection = ViemLogicalSelectionIdentityV1::default();
        let mut info = ViemSelectedStylesInfoV1::default();
        unsafe {
            assert_eq!(viem_core_view_list_selection(fixture.handle, fixture.view, &mut selection), ViemStatus::Ok);
            let status = viem_core_view_selected_styles_export(fixture.handle, fixture.view,
                selection.document_revision, &mut info, std::ptr::null_mut(), 0);
            assert!(matches!(status, ViemStatus::Ok | ViemStatus::BufferTooSmall));
        }
        assert_eq!(info.flags & VIEM_SELECTED_STYLE_HAS_TASKS != 0, task, "{source}");
        assert_eq!(info.flags & VIEM_SELECTED_STYLE_CAN_SET_TASK_LIST != 0, available, "{source}");
        if task { assert_eq!(info.flags & ((1 << 2) | (1 << 3) | (1 << 4)), 0, "{source}"); }
    }
}

#[test]
fn rejected_task_list_action_preserves_active_style_gesture() {
    let mut core = Core::new(markdown("# heading"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let gesture = core.begin_style_edit_group(view, core.document().id(), core.document().revision(),
        core.document().projection().style_sheet().revision).unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    assert!(core.handle(view, CoreEvent::SetListStyle { expected, style: Some(ListStyle::Task) }).is_err());
    core.end_style_edit_group(view, gesture).unwrap();
    assert_eq!(core.document().source_bytes(), b"# heading");
}
