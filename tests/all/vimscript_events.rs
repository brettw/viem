//! Vim script through the core coordinator: typed command lines, `:source`,
//! editor builtins, nested `:normal`, messages and undo grouping.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let document =
        Document::from_bytes_with_file_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix)
            .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    (core, view)
}

fn press(core: &mut Editor, view: ViewId, event: InputEvent) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(event)).unwrap()
}

/// Type `:{line}<CR>` and return the outcome of Enter.
fn ex(core: &mut Editor, view: ViewId, line: &str) -> CoreOutcome {
    press(core, view, InputEvent::Key(Key::Char(':')));
    for character in line.chars() {
        press(core, view, InputEvent::Key(Key::Char(character)));
    }
    press(core, view, InputEvent::Key(Key::Enter))
}

fn status(outcome: &CoreOutcome) -> CommandStatus {
    outcome.command.as_ref().map_or(CommandStatus::Complete, |command| command.status.clone())
}

fn messages(outcome: &CoreOutcome) -> Vec<String> {
    outcome
        .command
        .as_ref()
        .and_then(|command| command.ex_outcome.as_ref())
        .map(|ex| {
            ex.frontend_requests
                .iter()
                .filter_map(|request| match request {
                    ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn error_text(outcome: &CoreOutcome) -> String {
    match status(outcome) {
        CommandStatus::Error(text) => text,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn automatic_insert_events_run_in_order_and_keep_the_mode() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:events = []");
    ex(&mut core, view, "autocmd InsertEnter * call add(g:events, 'enter:' . mode())");
    ex(&mut core, view, "autocmd InsertLeavePre * call add(g:events, 'pre')");
    ex(&mut core, view, "autocmd InsertLeave * call add(g:events, 'leave:' . mode())");
    press(&mut core, view, InputEvent::key('i'));
    assert_eq!(core.command_state(view).unwrap().mode(), viem_core::command::Mode::Insert);
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(messages(&ex(&mut core, view, "echo join(g:events, ',')")), vec!["enter:i,pre,leave:n"]);
}

#[test]
fn yank_event_context_is_available_only_during_the_event() {
    let (mut core, view) = editor("alpha\nbeta");
    ex(&mut core, view, "autocmd TextYankPost * let g:yank = [v:event.regcontents, v:event.regtype, v:event.regname, v:event.operator]");
    for c in "\"ayy".chars() { press(&mut core, view, InputEvent::key(c)); }
    assert_eq!(messages(&ex(&mut core, view, "echo g:yank empty(v:event)")), vec!["[['alpha'], 'V', 'a', 'y'] 1"]);
}

#[test]
fn filetype_events_handle_nested_once_and_noautocmd() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:types = []");
    ex(&mut core, view, "autocmd FileType * call add(g:types, expand('<amatch>'))");
    ex(&mut core, view, "autocmd FileType rust set ft=python");
    ex(&mut core, view, "set ft=rust");
    assert_eq!(messages(&ex(&mut core, view, "echo g:types")), vec!["['rust']"]);
    ex(&mut core, view, "autocmd! FileType rust");
    ex(&mut core, view, "autocmd FileType rust ++nested set ft=python");
    ex(&mut core, view, "set ft=rust");
    assert_eq!(messages(&ex(&mut core, view, "echo g:types")), vec!["['rust', 'rust', 'python']"]);
    ex(&mut core, view, "noautocmd set ft=java");
    assert_eq!(messages(&ex(&mut core, view, "echo len(g:types)")), vec!["3"]);
    ex(&mut core, view, "autocmd User Once ++once let g:once = get(g:, 'once', 0) + 1");
    ex(&mut core, view, "doautocmd User Once");
    ex(&mut core, view, "doautocmd User Once");
    assert_eq!(messages(&ex(&mut core, view, "echo g:once")), vec!["1"]);
}

#[test]
fn native_lifecycle_events_match_names_and_keep_successful_prefix_on_error() {
    let (mut core, view) = editor("old");
    ex(&mut core, view, "autocmd BufWritePre *.txt call setline(1, 'saved')");
    let outcome = core.fire_autocmd(view, "BufWritePre", Some("/tmp/file.txt")).unwrap();
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(core.document().text(), "saved");
    ex(&mut core, view, "autocmd BufWritePost *.txt let g:saved = expand('<afile>')");
    core.fire_autocmd(view, "BufWritePost", Some("/tmp/file.txt")).unwrap();
    assert_eq!(messages(&ex(&mut core, view, "echo g:saved")), vec!["/tmp/file.txt"]);
    ex(&mut core, view, "autocmd FocusGained * let g:focused = 1 | echo missing_variable");
    let outcome = core.fire_autocmd(view, "FocusGained", None).unwrap();
    assert!(error_text(&outcome).contains("E121"));
    assert_eq!(messages(&ex(&mut core, view, "echo g:focused")), vec!["1"]);
}

#[test]
fn mappings_fire_each_insert_transition_and_yank_without_duplicate_outer_events() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:events = []");
    ex(&mut core, view, "autocmd InsertEnter * call add(g:events, 'i')");
    ex(&mut core, view, "autocmd InsertLeave * call add(g:events, 'n')");
    ex(&mut core, view, "autocmd TextYankPost * call add(g:events, v:event.regcontents[0])");
    ex(&mut core, view, "nnoremap Q ix<Esc>yyA");
    press(&mut core, view, InputEvent::key('Q'));
    assert_eq!(core.command_state(view).unwrap().mode(), viem_core::command::Mode::Insert);
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(messages(&ex(&mut core, view, "echo join(g:events, ',')")), vec!["i,n,xtext,i,n"]);
}

#[test]
fn noautocmd_normal_suppresses_input_events_and_script_yanks_fire_before_continuing() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:events = []");
    ex(&mut core, view, "autocmd InsertEnter * call add(g:events, 'i')");
    ex(&mut core, view, "autocmd InsertLeave * call add(g:events, 'n')");
    ex(&mut core, view, "noautocmd normal! ix");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), vec!["[]"]);
    ex(&mut core, view, "autocmd TextYankPost * let g:yanked = v:event.regcontents[0]");
    assert_eq!(messages(&ex(&mut core, view, "yank | echo g:yanked")), vec!["xtext"]);
}

#[test]
fn nested_autocmds_have_a_finite_depth() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:depth = 0");
    ex(&mut core, view, "autocmd User Recur ++nested let g:depth += 1 | doautocmd User Recur");
    ex(&mut core, view, "doautocmd User Recur");
    assert_eq!(messages(&ex(&mut core, view, "echo g:depth")), vec!["10"]);
}

#[test]
fn native_language_selection_fires_filetype() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "autocmd FileType rust let g:language = expand('<amatch>')");
    let event = CoreEvent::SetDocumentMode { document: core.document().id(), revision: core.document().revision(),
        mode: viem_core::DocumentMode::Code("rust".into()) };
    core.handle_with_layout(view, event).unwrap();
    assert_eq!(messages(&ex(&mut core, view, "echo g:language")), vec!["rust"]);
}

#[test]
fn yank_handlers_cannot_edit_text_and_black_hole_yanks_do_not_fire() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "autocmd TextYankPost * call setline(1, 'bad')");
    for c in "\"_yy".chars() { press(&mut core, view, InputEvent::key(c)); }
    assert_eq!(core.document().text(), "text");
    press(&mut core, view, InputEvent::key('y'));
    let outcome = press(&mut core, view, InputEvent::key('y'));
    assert!(error_text(&outcome).contains("E565"));
    assert_eq!(core.document().text(), "text");
}

#[test]
fn nested_autocommands_can_fire_input_events_from_normal() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:events = []");
    ex(&mut core, view, "autocmd InsertEnter * call add(g:events, 'i')");
    ex(&mut core, view, "autocmd InsertLeave * call add(g:events, 'n')");
    ex(&mut core, view, "autocmd User Edit normal! ix");
    ex(&mut core, view, "doautocmd User Edit");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), vec!["[]"]);
    ex(&mut core, view, "autocmd! User Edit");
    ex(&mut core, view, "autocmd User Edit ++nested normal! iy");
    ex(&mut core, view, "doautocmd User Edit");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), vec!["['i', 'n']"]);
}

#[test]
fn native_save_pre_rejects_host_effects_before_publishing_options_or_theme() {
    for event in ["BufWritePre", "BufWritePost"] {
    for command in [
        "set timeoutlen=23",
        "let &timeoutlen = 23",
        "colorscheme Default",
        "pwd",
        "write",
        "normal! ZZ",
        r#"execute "normal! \<C-W>s""#,
        r#"execute "normal! :set timeoutlen=23\<CR>""#,
        "normal Q",
        "normal R",
    ] {
        let (mut core, view) = editor("old");
        ex(&mut core, view, "let g:colors_name = 'before'");
        ex(&mut core, view, "nnoremap Q <C-W>s");
        ex(&mut core, view, "nnoremap R :set timeoutlen=23<CR>");
        ex(&mut core, view, &format!("autocmd {event} * {command}"));
        let outcome = core.fire_autocmd(view, event, None).unwrap();
        assert!(matches!(status(&outcome), CommandStatus::Error(ref text) if text.contains("E565")),
            "{command}: {:?}; messages {:?}", status(&outcome), messages(&outcome));
        assert!(outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).is_none_or(|ex|
            !ex.frontend_requests.iter().any(|request| matches!(request,
                ExFrontendRequest::File(_) | ExFrontendRequest::Window(_) | ExFrontendRequest::Application(_) | ExFrontendRequest::ScriptContinuation(_)
            ))), "{command}");
        assert_eq!(messages(&ex(&mut core, view, "echo &timeoutlen g:colors_name")), vec!["1000 before"], "{command}");
        assert_eq!(core.document().text(), "old");
        // The synchronous guard is retired even after an error.
        let ordinary = ex(&mut core, view, "set timeoutlen=42");
        assert!(ordinary.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).is_some_and(|ex|
            ex.frontend_requests.iter().any(|request| matches!(request, ExFrontendRequest::Application(_)))));
    }
}
}

#[test]
fn native_save_pre_keeps_core_edits_synchronous_and_callback_rejection_keeps_pending_script() {
    let (mut core, view) = editor("old");
    ex(&mut core, view, "autocmd BufWritePre * set ignorecase");
    ex(&mut core, view, "autocmd BufWritePre * normal! x");
    ex(&mut core, view, "autocmd BufWritePre * call setline(1, 'saved')");
    let outcome = core.fire_autocmd(view, "BufWritePre", None).unwrap();
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(core.document().text(), "saved");
    assert_eq!(messages(&ex(&mut core, view, "echo &ignorecase")), vec!["1"]);

    ex(&mut core, view, "autocmd FocusGained * let &timeoutlen = 2");
    let pending = ex(&mut core, view, "pwd | let g:after = 1");
    let token = pending.command.as_ref().unwrap().ex_outcome.as_ref().unwrap().frontend_requests.iter().find_map(|request|
        if let ExFrontendRequest::ScriptContinuation(token) = request { Some(*token) } else { None }).unwrap();
    let callback = core.fire_autocmd(view, "FocusGained", None).unwrap();
    assert!(error_text(&callback).contains("E565"));
    core.resume_script(view, token, true, None).unwrap();
    assert_eq!(messages(&ex(&mut core, view, "echo &timeoutlen g:after")), vec!["1000 1"]);
}
