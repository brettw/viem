use super::*;
use crate::command::ex_execute::ExFrontendRequest;
use crate::ffi::{
    viem_core_view_execute_command_line, viem_core_view_fire_autocmd,
    viem_core_view_resume_script, viem_core_view_take_startup_effects,
    viem_effect_batch_release, ViemClipboardTurnEntryV2, ViemCommandTurnContextV2,
    VIEM_CLIPBOARD_TARGET_CLIPBOARD, VIEM_CLIPBOARD_TURN_HAS_READ,
    VIEM_COMMAND_STATUS_COMPLETE,
};

#[test]
fn script_continuation_ffi_validates_arguments_without_consuming_pending_work() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("before"));
    let view = core.add_view(paint_test_provider(&mut storage, 601, 701), 240.0, 80.0);
    core.stage_command_line(view, "pwd | call setline(1, 'after')").unwrap();
    let pending = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    let token = pending.command.unwrap().ex_outcome.unwrap().frontend_requests.into_iter().find_map(|request| {
        match request { ExFrontendRequest::ScriptContinuation(token) => Some(token), _ => None }
    }).unwrap();
    let handle = register_core(core).unwrap();
    let context = ViemCommandTurnContextV2::default();
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    unsafe {
        for (requested_token, success, expected) in [
            (0, 1, ViemStatus::InvalidArgument),
            (token, 2, ViemStatus::InvalidArgument),
            (token + 1, 0, ViemStatus::StaleRevision),
        ] {
            assert_eq!(viem_core_view_resume_script(handle, view.0, requested_token, success,
                &context, &mut outcome, &mut effects), expected);
        }
        let aliased = (&mut outcome as *mut ViemCoreOutcomeV1).cast::<u64>();
        assert_eq!(viem_core_view_resume_script(handle, view.0, token, 1, &context,
            &mut outcome, aliased), ViemStatus::InvalidArgument);
        assert_eq!(viem_core_view_resume_script(handle, view.0, token, 1, ptr::null(),
            &mut outcome, &mut effects), ViemStatus::NullPointer);
        assert_eq!(viem_core_view_take_startup_effects(handle, view.0, &mut outcome, aliased),
            ViemStatus::InvalidArgument);
        assert_eq!(checkout_core(handle).unwrap().core().document().text(), "before");
        assert_eq!(viem_core_view_resume_script(handle, view.0, token, 1, &context,
            &mut outcome, &mut effects), ViemStatus::Ok);
        assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_COMPLETE);
        if effects != 0 { assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok); }
        assert_eq!(checkout_core(handle).unwrap().core().document().text(), "after");
        assert_eq!(viem_core_view_resume_script(handle, view.0, token, 1, &context,
            &mut outcome, &mut effects), ViemStatus::StaleRevision);
    }
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn autocmd_ffi_rejects_invalid_inputs_and_aliased_regions_before_core_access() {
    let context = ViemCommandTurnContextV2::default();
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    unsafe {
        for (event, expected) in [
            (b"NoSuchEvent".as_slice(), ViemStatus::InvalidArgument),
            (b"CursorHold".as_slice(), ViemStatus::InvalidArgument),
            (&[0xff], ViemStatus::InvalidUtf8),
        ] {
            assert_eq!(viem_core_view_fire_autocmd(0, 0, event.as_ptr(), event.len() as u64,
                ptr::null(), 0, &context, &mut outcome, &mut effects), expected);
        }
        let event = b"FocusGained";
        assert_eq!(viem_core_view_fire_autocmd(0, 0, ptr::null(), 1, ptr::null(), 0,
            &context, &mut outcome, &mut effects), ViemStatus::NullPointer);
        assert_eq!(viem_core_view_fire_autocmd(0, 0, event.as_ptr(), event.len() as u64,
            [0xff].as_ptr(), 1, &context, &mut outcome, &mut effects), ViemStatus::InvalidUtf8);
        let aliased = (&mut outcome as *mut ViemCoreOutcomeV1).cast::<u64>();
        assert_eq!(viem_core_view_fire_autocmd(0, 0, event.as_ptr(), event.len() as u64,
            ptr::null(), 0, &context, &mut outcome, aliased), ViemStatus::InvalidArgument);
        let context_alias = (&outcome as *const ViemCoreOutcomeV1).cast::<ViemCommandTurnContextV2>();
        assert_eq!(viem_core_view_fire_autocmd(0, 0, event.as_ptr(), event.len() as u64,
            ptr::null(), 0, context_alias, &mut outcome, &mut effects), ViemStatus::InvalidArgument);
    }
}

#[test]
fn native_autocmd_receives_the_command_turn_clipboard_snapshot() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("seed"));
    let view = core.add_view(paint_test_provider(&mut storage, 602, 702), 240.0, 80.0);
    let handle = register_core(core).unwrap();
    let clipboard = b"clip";
    let entry = ViemClipboardTurnEntryV2 {
        target: VIEM_CLIPBOARD_TARGET_CLIPBOARD,
        flags: VIEM_CLIPBOARD_TURN_HAS_READ,
        generation: 1,
        plain_text: ViemUtf8Slice { data: clipboard.as_ptr(), length: clipboard.len() as u64 },
        ..ViemClipboardTurnEntryV2::default()
    };
    let context = ViemCommandTurnContextV2 { clipboards: &entry, clipboard_count: 1,
        ..ViemCommandTurnContextV2::default() };
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut effects = 0;
    unsafe {
        let command = b"autocmd BufWritePre * normal! \"+P";
        assert_eq!(viem_core_view_execute_command_line(handle, view.0, command.as_ptr(), command.len() as u64,
            &context, &mut outcome, &mut effects), ViemStatus::Ok);
        if effects != 0 { assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok); }
        let event = b"BufWritePre";
        assert_eq!(viem_core_view_fire_autocmd(handle, view.0, event.as_ptr(), event.len() as u64,
            ptr::null(), 0, &context, &mut outcome, &mut effects), ViemStatus::Ok);
        assert_eq!(outcome.command_status, VIEM_COMMAND_STATUS_COMPLETE);
        if effects != 0 { assert_eq!(viem_effect_batch_release(effects), ViemStatus::Ok); }
    }
    assert_eq!(checkout_core(handle).unwrap().core().document().text(), "clipseed");
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}
