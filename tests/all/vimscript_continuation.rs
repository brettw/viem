//! Native commands must complete before later script commands run.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreError, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
fn editor() -> (Editor, ViewId) {
    let document = Document::from_bytes_with_file_format(b"one\ntwo".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    (core, view)
}
fn ex(core: &mut Editor, view: ViewId, line: &str) -> CoreOutcome {
    core.stage_command_line(view, line).unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap()
}
fn token(out: &CoreOutcome) -> u64 {
    let requests = &out.command.as_ref().unwrap().ex_outcome.as_ref().unwrap().frontend_requests;
    match requests.last().unwrap() { ExFrontendRequest::ScriptContinuation(token) => *token, other => panic!("no continuation: {other:?}") }
}
fn messages(out: &CoreOutcome) -> Vec<String> {
    out.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).map(|ex| ex.frontend_requests.iter().filter_map(|request| {
        if let ExFrontendRequest::Info(ExInfoRequest::Message(text)) = request { Some(text.clone()) } else { None }
    }).collect()).unwrap_or_default()
}

#[test]
fn host_commands_suspend_in_order_and_tokens_are_single_use() {
    let (mut core, view) = editor();
    let first = ex(&mut core, view, "let g:step = 1 | pwd | let g:step = 2 | pwd | call setline(1, 'done') | echo g:step");
    assert_eq!(core.document().text(), "one\ntwo");
    let first = token(&first);
    let second = core.resume_script(view, first, true, None).unwrap();
    let second = token(&second);
    assert_ne!(first, second);
    assert_eq!(core.document().text(), "one\ntwo");
    assert_eq!(core.resume_script(view, first, false, None), Err(CoreError::StaleScriptContinuation));
    let done = core.resume_script(view, second, true, None).unwrap();
    assert_eq!(core.document().text(), "done\ntwo");
    assert_eq!(messages(&done), ["2"]);
    assert_eq!(core.resume_script(view, second, true, None), Err(CoreError::StaleScriptContinuation));
}

#[test]
fn cancellation_keeps_completed_prefix_and_abandons_later_edits() {
    let (mut core, view) = editor();
    let out = ex(&mut core, view, "call setline(1, 'prefix') | cd / | call setline(2, 'never')");
    assert_eq!(core.document().text(), "prefix\ntwo");
    core.resume_script(view, token(&out), false, None).unwrap();
    assert_eq!(core.document().text(), "prefix\ntwo");
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
    assert_eq!(core.document().text(), "one\ntwo");
    assert_eq!(messages(&ex(&mut core, view, "echo 42")), ["42"]);
}

#[test]
fn new_input_and_view_closure_cancel_pending_continuations() {
    let (mut core, view) = editor();
    let out = ex(&mut core, view, "pwd | call setline(1, 'never')");
    let old = token(&out);
    ex(&mut core, view, "let g:new = 1");
    assert_eq!(core.resume_script(view, old, true, None), Err(CoreError::StaleScriptContinuation));
    let out = ex(&mut core, view, "pwd | call setline(1, 'never')");
    core.remove_view(view).unwrap();
    assert_eq!(core.resume_script(view, token(&out), true, None), Err(CoreError::UnknownView(view)));
    assert_eq!(core.document().text(), "one\ntwo");
}

#[test]
fn native_action_inside_expression_is_rejected_before_effects_escape() {
    let (mut core, view) = editor();
    let out = ex(&mut core, view, "echo execute('pwd')");
    // execute() captures ordinary error output, as it does in Vim.
    assert!(messages(&out).iter().any(|message| message.contains("E565: Native host commands cannot run inside an expression")), "{out:?}");
    assert!(!out.command.as_ref().unwrap().ex_outcome.as_ref().unwrap().frontend_requests.iter().any(|r| matches!(r, ExFrontendRequest::File(_))));
}

#[test]
fn messages_precede_the_native_request_that_follows_them() {
    let (mut core, view) = editor();
    let out = ex(&mut core, view, "echo 'before' | pwd | echo 'after'");
    let requests = &out.command.as_ref().unwrap().ex_outcome.as_ref().unwrap().frontend_requests;
    assert!(matches!(&requests[0], ExFrontendRequest::Info(ExInfoRequest::Message(message)) if message == "before"));
    assert!(matches!(&requests[1], ExFrontendRequest::File(_)));
    let done = core.resume_script(view, token(&out), true, None).unwrap();
    assert_eq!(messages(&done), ["after"]);
}

#[test]
fn source_policy_options_use_verified_undoable_document_transactions() {
    let (mut core, view) = editor();
    let before = core.document().source_bytes();
    let out = ex(&mut core, view, "set fenc=utf-16le bomb | echo &fenc &bomb");
    assert_eq!(out.command.as_ref().unwrap().status, CommandStatus::Complete, "{out:?}");
    assert_eq!(messages(&out), ["utf-16le 1"]);
    assert_eq!(core.document().encoding(), Encoding::Utf16Le);
    assert!(core.document().has_bom());
    assert_eq!(core.document().text(), "one\ntwo");
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
    assert_eq!(core.document().source_bytes(), before);
    assert_eq!(core.document().encoding(), Encoding::Utf8);
    ex(&mut core, view, "set ro");
    assert!(core.document().is_read_only());
    ex(&mut core, view, "set noro");
    assert!(!core.document().is_read_only());
    let out = ex(&mut core, view, "set fenc=unknown");
    assert!(matches!(&out.command.as_ref().unwrap().status, CommandStatus::ExError(_) | CommandStatus::Error(_)));
    assert_eq!(core.document().source_bytes(), before);
}

#[test]
fn startup_assignments_cannot_change_source_policy_and_effects_drain_once() {
    let doc = Document::new("text");
    let before = doc.source_bytes();
    let mut core = Core::new(doc);
    let diagnostics = core.initialize_startup("let &fenc = 'utf-16le'\nlet &bomb = 1\nset timeoutlen=42\nlet g:ready = 1\n");
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_eq!(core.document().source_bytes(), before);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    let out = core.take_startup_effects(view).unwrap();
    assert!(out.command.unwrap().ex_outcome.unwrap().frontend_requests.iter().any(|r| matches!(r, ExFrontendRequest::StartupApplication(_))));
    assert!(core.take_startup_effects(view).unwrap().command.unwrap().ex_outcome.is_none());
    assert_eq!(messages(&ex(&mut core, view, "echo g:ready")), ["1"]);
}

#[test]
fn invalid_native_option_values_do_not_publish_shadow_values_or_effects() {
    let (mut core, view) = editor();
    for command in ["set guifont=Mono:h1001", "set guifont=Mono:x", "set breakindentopt=shift:1025"] {
        let out = ex(&mut core, view, command);
        assert!(matches!(&out.command.as_ref().unwrap().status, CommandStatus::Error(_) | CommandStatus::ExError(_)), "{out:?}");
        assert!(!out.command.as_ref().unwrap().ex_outcome.as_ref().is_some_and(|ex| ex.frontend_requests.iter().any(|request| matches!(request, ExFrontendRequest::Application(_) | ExFrontendRequest::ScriptContinuation(_)))));
    }
    assert_eq!(messages(&ex(&mut core, view, "echo string(&guifont) string(&breakindentopt)")), ["'' ''"]);
}

fn cancelled_ctrl_o_returns_to_typing(new_input: bool) {
    use viem_core::command::Mode;
    for (start, mode, before, after) in [
        ('i', Mode::Insert, "Aone", "ABone"),
        ('R', Mode::Replace, "Ane", "ABe"),
    ] {
        let (mut core, view) = editor();
        let key = |core: &mut Editor, key| core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        key(&mut core, Key::Char(start));
        key(&mut core, Key::Char('A'));
        key(&mut core, Key::Ctrl('o'));
        let out = ex(&mut core, view, "call setline(2, 'prefix') | pwd | call setline(1, 'never')");
        let pending = token(&out);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.document().text(), format!("{before}\nprefix"));
        if new_input {
            let out = key(&mut core, Key::Char('B'));
            assert!(out.command.unwrap().mode_changed);
        } else {
            let out = core.resume_script(view, pending, false, None).unwrap();
            assert!(out.command.unwrap().mode_changed);
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            key(&mut core, Key::Char('B'));
        }
        assert_eq!(core.command_state(view).unwrap().mode(), mode);
        assert_eq!(core.document().text(), format!("{after}\nprefix"));
        assert_eq!(core.resume_script(view, pending, true, None), Err(CoreError::StaleScriptContinuation));
        key(&mut core, Key::Escape);
        for expected in [format!("{before}\nprefix"), format!("{before}\ntwo"), "one\ntwo".into()] {
            key(&mut core, Key::Char('u'));
            assert_eq!(core.document().text(), expected);
        }
        for expected in [format!("{before}\ntwo"), format!("{before}\nprefix"), format!("{after}\nprefix")] {
            key(&mut core, Key::Ctrl('r'));
            assert_eq!(core.document().text(), expected);
        }
    }
}

#[test]
fn native_cancellation_finishes_ctrl_o_and_restores_insert_replace_undo_groups() {
    cancelled_ctrl_o_returns_to_typing(false);
}

#[test]
fn new_input_cancels_native_replay_before_dispatching_in_restored_typing_mode() {
    cancelled_ctrl_o_returns_to_typing(true);
}
