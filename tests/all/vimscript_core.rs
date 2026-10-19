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
fn typed_lines_evaluate_expressions_and_echo() {
    let (mut core, view) = editor("one\ntwo\n");
    let outcome = ex(&mut core, view, "let g:x = 3 | echo g:x * 2 'lines:' line('$')");
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(messages(&outcome), vec!["6 lines: 2".to_string()]);
    let outcome = ex(&mut core, view, "echo exists('g:x') getline(2)");
    assert_eq!(messages(&outcome), vec!["1 two".to_string()]);
}

#[test]
fn script_errors_report_vim_messages_and_continue_with_the_next_command() {
    let (mut core, view) = editor("text\n");
    let outcome = ex(&mut core, view, "echo nosuch");
    assert_eq!(error_text(&outcome), "E121: Undefined variable: nosuch");
    let outcome = ex(&mut core, view, "try | echo nosuch | catch | echo 'caught' v:exception | endtry");
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(messages(&outcome), vec!["caught Vim(echo):E121: Undefined variable: nosuch".to_string()]);
}

#[test]
fn host_commands_run_inside_scripts_with_one_undo_unit() {
    let (mut core, view) = editor("alpha\nbeta\n");
    let outcome = ex(&mut core, view, "s/alpha/ALPHA/ | 2s/beta/BETA/ | echo getline(1) getline(2)");
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(messages(&outcome), vec!["ALPHA BETA".to_string()]);
    assert_eq!(core.document().text(), "ALPHA\nBETA\n");
    press(&mut core, view, InputEvent::Key(Key::Char('u')));
    assert_eq!(core.document().text(), "alpha\nbeta\n");
}

#[test]
fn editor_builtins_change_lines_and_cursor() {
    let (mut core, view) = editor("one\ntwo\nthree\n");
    let outcome = ex(&mut core, view, "call setline(2, 'TWO') | call append(3, ['four', 'five']) | call cursor(4, 2)");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "one\nTWO\nthree\nfour\nfive\n");
    let outcome = ex(&mut core, view, "echo line('.') col('.')");
    assert_eq!(messages(&outcome), vec!["4 2".to_string()]);
    press(&mut core, view, InputEvent::Key(Key::Char('u')));
    assert_eq!(core.document().text(), "one\ntwo\nthree\n");
}

#[test]
fn execute_normal_runs_as_a_nested_replay_and_the_script_resumes() {
    let (mut core, view) = editor("abc\n");
    let outcome = ex(&mut core, view, "exe 'normal! x' | let g:after = getline(1) | echo g:after");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "bc\n");
    assert_eq!(messages(&outcome), vec!["bc".to_string()]);
}

#[test]
fn options_read_and_write_through_let_and_set() {
    let (mut core, view) = editor("x\n");
    let outcome = ex(&mut core, view, "set ts=4 | let &sw = 3 | echo &ts &sw &cursorline exists('+cursorline')");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(messages(&outcome), vec!["4 3 0 0".to_string()]);
    let outcome = ex(&mut core, view, "set nosuchoption");
    assert!(matches!(status(&outcome), CommandStatus::ExError(_)), "{outcome:?}");
    let outcome = ex(&mut core, view, "silent! set nosuchoption | echo 'next'");
    assert_eq!(messages(&outcome), vec!["next".to_string()]);
}

#[test]
fn source_runs_files_through_the_engine() {
    let directory = std::env::temp_dir().join(format!("viem-vimscript-core-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("script.vim");
    std::fs::write(
        &path,
        "\" comment\nlet s:count = 0\nfunction! Bump(n) abort\n  let s:count += a:n\n  return s:count\nendfunction\n\
         call Bump(2)\nlet g:result = Bump(3)\ncall setline(1, 'result ' . g:result)\n",
    )
    .unwrap();
    let (mut core, view) = editor("line\n");
    let outcome = ex(&mut core, view, &format!("source {}", path.display()));
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "result 5\n");
    // Functions defined by the file remain available to later lines.
    let outcome = ex(&mut core, view, "echo Bump(1)");
    assert_eq!(messages(&outcome), vec!["6".to_string()]);
    std::fs::remove_dir_all(&directory).ok();
}

fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for character in text.chars() {
        press(core, view, InputEvent::Key(Key::Char(character)));
    }
}

#[test]
fn mapping_commands_accept_abbreviated_names_attributes_and_leader() {
    let (mut core, view) = editor("one two\n");
    ex(&mut core, view, "let mapleader = ','");
    let outcome = ex(&mut core, view, "nno <silent> <Leader>d dw");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    keys(&mut core, view, ",d");
    assert_eq!(core.document().text(), "two\n");
    let outcome = ex(&mut core, view, "nmap ,");
    assert_eq!(messages(&outcome), vec!["n  ,d          * dw".to_string()]);
    let outcome = ex(&mut core, view, "echo maparg(',d', 'n') maparg(',d', 'n', 0, 1).silent hasmapto('dw', 'n')");
    assert_eq!(messages(&outcome), vec!["dw 1 1".to_string()]);
    let outcome = ex(&mut core, view, "nunmap ,d | nunmap ,d");
    assert_eq!(error_text(&outcome), "E31: No such mapping");
    let outcome = ex(&mut core, view, "nmap <unique> Z x | nmap <unique> Z y");
    assert_eq!(error_text(&outcome), "E227: Mapping already exists for Z");
}

#[test]
fn expr_mappings_evaluate_when_typed_and_keep_the_count() {
    let (mut core, view) = editor("abcdef\n");
    ex(&mut core, view, "nnoremap <expr> X v:count ? 'x' : \"A!\\<Esc>\"");
    keys(&mut core, view, "X");
    assert_eq!(core.document().text(), "abcdef!\n");
    keys(&mut core, view, "0");
    keys(&mut core, view, "2X");
    assert_eq!(core.document().text(), "cdef!\n");
}

#[test]
fn cmd_mappings_run_without_leaving_insert_mode() {
    let (mut core, view) = editor("text\n");
    ex(&mut core, view, "inoremap <F2> <Cmd>let g:seen = mode()<CR>");
    keys(&mut core, view, "i");
    press(&mut core, view, InputEvent::Key(Key::Function { number: 2, modifiers: 0 }));
    keys(&mut core, view, "x");
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().text(), "xtext\n");
    let outcome = ex(&mut core, view, "echo g:seen");
    assert_eq!(messages(&outcome), vec!["i".to_string()]);
}

#[test]
fn nowait_mappings_do_not_wait_for_longer_left_sides() {
    let (mut core, view) = editor("abc\n");
    ex(&mut core, view, "nnoremap <nowait> Q x");
    ex(&mut core, view, "nnoremap Qa dd");
    keys(&mut core, view, "Q");
    assert_eq!(core.document().text(), "bc\n");
}

#[test]
fn insert_abbreviations_expand_after_typed_words_only() {
    let (mut core, view) = editor("\n");
    // Like maps, abbreviations keep white space before `|`, so use two lines.
    ex(&mut core, view, "iab teh the");
    let outcome = ex(&mut core, view, "inorea <expr> ddate 'today'");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    keys(&mut core, view, "isay teh steh ddate.");
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().text(), "say the steh today.\n");
    // One undo removes the whole insertion, expansions included.
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "\n");
    let outcome = ex(&mut core, view, "iab");
    assert_eq!(messages(&outcome), vec!["i  teh           the\ni  ddate       * 'today'".to_string()]);
}

#[test]
fn command_line_abbreviations_expand_before_execution() {
    let (mut core, view) = editor("one\n");
    ex(&mut core, view, "cnoreabbrev W s/one/two/");
    let outcome = ex(&mut core, view, "W");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "two\n");
}

#[test]
fn buffer_open_autocommands_run_when_the_first_view_attaches() {
    let document =
        Document::from_bytes_with_file_format(b"one\ntwo\nthree\n".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix)
            .unwrap();
    let mut core: Editor = Core::new(document);
    let diagnostics = core.initialize_startup(
        "autocmd BufReadPost *.txt let g:opened = expand('<afile>:t')\nautocmd BufReadPost * normal! G\nautocmd VimEnter * let g:entered = 1",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let path = std::env::temp_dir().join(format!("viem-events-{}.txt", std::process::id()));
    std::fs::write(&path, b"one\ntwo\nthree\n").unwrap();
    core.initialize_code_detection(path.to_str().unwrap(), false).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    core.fire_autocmd(view, "VimEnter", None).unwrap();
    let outcome = ex(&mut core, view, "echo g:opened g:entered line('.')");
    assert_eq!(messages(&outcome), vec![format!("{} 1 3", path.file_name().unwrap().to_str().unwrap())]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn insert_and_file_type_autocommands_fire() {
    let (mut core, view) = editor("x\n");
    ex(&mut core, view, "autocmd InsertEnter * let g:enter = get(g:, 'enter', 0) + 1");
    ex(&mut core, view, "autocmd InsertLeave * let g:leave = get(g:, 'leave', 0) + 1");
    ex(&mut core, view, "autocmd FileType python let g:ft = expand('<amatch>')");
    keys(&mut core, view, "ia");
    press(&mut core, view, InputEvent::Key(Key::Escape));
    let outcome = ex(&mut core, view, "echo g:enter g:leave");
    assert_eq!(messages(&outcome), vec!["1 1".to_string()]);
    let outcome = ex(&mut core, view, "set ft=python");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    let outcome = ex(&mut core, view, "echo g:ft &filetype");
    assert_eq!(messages(&outcome), vec!["python python".to_string()]);
}

#[test]
fn colorscheme_selects_a_published_theme_and_fires_colorscheme() {
    let names = b"Default\0Paper\0Midnight Mono";
    assert_eq!(unsafe { viem_core::ffi::viem_set_theme_names(names.as_ptr(), names.len() as u64) }, viem_core::ffi::ViemStatus::Ok);
    let (mut core, view) = editor("x\n");
    ex(&mut core, view, "autocmd ColorScheme * let g:scheme = expand('<amatch>')");
    let outcome = ex(&mut core, view, "colorscheme paper");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    let requests = &outcome.command.as_ref().unwrap().ex_outcome.as_ref().unwrap().frontend_requests;
    assert!(requests.contains(&ExFrontendRequest::Application(viem_core::command::ex_execute::ApplicationRequest::Theme("Paper".into()))), "{requests:?}");
    let token = requests.iter().find_map(|request| match request {
        ExFrontendRequest::ScriptContinuation(token) => Some(*token), _ => None,
    }).expect("theme selection waits for the native host");
    core.resume_script(view, token, true, None).unwrap();
    let outcome = ex(&mut core, view, "echo g:scheme g:colors_name");
    assert_eq!(messages(&outcome), vec!["paper Paper".to_string()]);
    let outcome = ex(&mut core, view, "colorscheme nosuch");
    assert_eq!(error_text(&outcome), "E185: Cannot find color scheme 'nosuch'");
}

#[test]
fn vim9_source_and_legacy_calls_use_the_same_editor_transactions() {
    let directory = std::env::temp_dir().join(format!("viem-vim9-core-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("main.vim");
    std::fs::write(&path, "vim9script\nvar count = 0\ndef g:Vim9Update(prefix: string): number\n  count += 1\n  setline(1, prefix .. count)\n  return count\nenddef\nVim9Update('first ')\n").unwrap();
    let (mut core, view) = editor("before\n");
    let outcome = ex(&mut core, view, &format!("source {}", path.display()));
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "first 1\n");
    let outcome = ex(&mut core, view, "echo Vim9Update('second ')");
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "second 2\n");
    press(&mut core, view, InputEvent::Key(Key::Char('u')));
    assert_eq!(core.document().text(), "first 1\n");
    std::fs::remove_dir_all(directory).unwrap();
}
