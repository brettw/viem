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

#[test]
fn getline_keeps_carriage_returns_that_the_file_format_reads_as_content() {
    let (mut core, view) = editor("x\r\nlast");
    let outcome = ex(&mut core, view, "echo strlen(getline(1)) char2nr(getline(1)[1]) getline(2)");
    assert_eq!(messages(&outcome), vec!["2 13 last".to_string()]);
    // Round-tripping the value leaves the authored CR in place.
    ex(&mut core, view, "call setline(1, getline(1))");
    assert_eq!(core.document().text(), "x\r\nlast");

    let document =
        Document::from_bytes_with_file_format(b"x\r\ny\n".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Dos).unwrap();
    let mut core: Editor = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    let outcome = ex(&mut core, view, "echo strlen(getline(1)) strlen(getline(2))");
    assert_eq!(messages(&outcome), vec!["1 1".to_string()]);
}

#[test]
fn line_values_spell_nul_as_lf_without_splitting_lines() {
    let (mut core, view) = editor("one\ntwo\nthree\n");
    let outcome = ex(&mut core, view, "call setline(1, \"a\\nb\") | echo line('$') strlen(getline(1)) char2nr(getline(1)[1])");
    assert_eq!(messages(&outcome), vec!["3 3 10".to_string()]);
    assert_eq!(core.document().text(), "a\0b\ntwo\nthree\n");
    let outcome = ex(&mut core, view, "call append(1, [\"c\\nd\"]) | echo line('$') string(getline(2))");
    assert_eq!(messages(&outcome), vec!["4 'c\nd'".to_string()]);
    assert_eq!(core.document().text(), "a\0b\nc\0d\ntwo\nthree\n");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a\0b\ntwo\nthree\n");
}

#[test]
fn vim9_comments_leave_substitute_and_normal_payloads_intact() {
    let directory = std::env::temp_dir().join(format!("viem-vim9-payloads-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("payloads.vim");
    std::fs::write(
        &path,
        "vim9script\nsetline(1, 'old')\ns/old/ #tag/\nvar first = getline(1)\nsetline(1, 'old')\nnormal! A #tag\n\
         var second = getline(1)\ndef F()\n  setline(1, 'x')\n  normal! A #in def\nenddef\nF()\n\
         echo string(first) string(second) string(getline(1)) # a real comment\n",
    )
    .unwrap();
    let (mut core, view) = editor("before\n");
    let outcome = ex(&mut core, view, &format!("source {}", path.display()));
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(messages(&outcome), vec!["' #tag' 'old #tag' 'x #in def'".to_string()]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn column_functions_count_utf8_bytes_characters_and_screen_cells() {
    let (mut core, view) = editor("x\n");
    let echo = |core: &mut Editor, line: &str| messages(&ex(core, view, line));
    ex(&mut core, view, "set tabstop=8 | call setline(1, \"\\u00e9\\tx\") | call cursor(1, 4)");
    assert_eq!(echo(&mut core, "echo col('.') charcol('.') virtcol('.') getcharpos('.')[2] getpos('.')[2] getcurpos()[4]"), ["4 3 9 3 4 9"]);
    assert_eq!(echo(&mut core, "echo col('$') charcol('$') virtcol('$')"), ["5 4 10"]);
    ex(&mut core, view, "call setcursorcharpos(1, 3)");
    assert_eq!(echo(&mut core, "echo col('.')"), ["4"]);
    ex(&mut core, view, "call setcharpos('.', [0, 1, 2, 0])");
    assert_eq!(echo(&mut core, "echo col('.') charcol('.') virtcol('.')"), ["3 2 8"]);
    // A composing character belongs to its base; a wide character spans two cells.
    ex(&mut core, view, "call setline(1, \"e\\u0301\\u65e5x\") | call cursor(1, 4)");
    assert_eq!(echo(&mut core, "echo col('.') charcol('.') virtcol('.') getcurpos()[4]"), ["4 2 3 2"]);
    ex(&mut core, view, "call setcursorcharpos(1, 3)");
    assert_eq!(echo(&mut core, "echo col('.') charcol('.') virtcol('.')"), ["7 3 4"]);
    ex(&mut core, view, "normal! 0ma");
    assert_eq!(echo(&mut core, "echo col(\"'a\") charcol(\"'a\") getcharpos(\"'a\")[2]"), ["1 1 1"]);
}

#[test]
fn column_functions_use_utf8_columns_in_latin1_files() {
    let document =
        Document::from_bytes_with_file_format(b"\xe9xy\n".to_vec(), Encoding::Latin1, Format::PlainText, FileFormat::Unix).unwrap();
    let mut core: Editor = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    ex(&mut core, view, "call cursor(1, 3)");
    assert_eq!(messages(&ex(&mut core, view, "echo col('.') charcol('.') getline('.')[col('.') - 1]")), ["3 2 x"]);
    ex(&mut core, view, "call setcursorcharpos(1, 3)");
    assert_eq!(messages(&ex(&mut core, view, "echo col('.') charcol('.')")), ["4 3"]);
    assert_eq!(core.document().source_bytes(), b"\xe9xy\n");
}

#[test]
fn append_above_the_cursor_keeps_it_on_its_content() {
    let (mut core, view) = editor("one\ntwo\nthree\n");
    ex(&mut core, view, "call cursor(2, 2)");
    for append in ["call append(0, ['added'])", "call append(1, ['b1', 'b2'])", "call append(line('.'), 'after')", "call append(line('$'), [])"] {
        ex(&mut core, view, append);
        let expected = if append.contains("(0,") { "3 2 two" } else { "5 2 two" };
        assert_eq!(messages(&ex(&mut core, view, "echo line('.') col('.') getline('.')")), [expected], "{append}");
    }
}

#[test]
fn search_honors_wrapscan_and_the_skip_expression() {
    let (mut core, view) = editor("one\ntwo\nthree\n");
    let echo = |core: &mut Editor, line: &str| messages(&ex(core, view, line));
    ex(&mut core, view, "set nowrapscan | call cursor(3, 1)");
    assert_eq!(echo(&mut core, "echo search('one') line('.')"), ["0 3"]);
    assert_eq!(echo(&mut core, "echo search('one', 'w') line('.')"), ["1 1"]);
    ex(&mut core, view, "set wrapscan | call setline(1, ['one', 'two', 'one']) | call cursor(1, 1)");
    // Every match is skipped, so the cursor stays put.
    assert_eq!(echo(&mut core, "echo search('one', 'W', 0, 0, '1') line('.')"), ["0 1"]);
    assert_eq!(echo(&mut core, "echo search('one', 'W', 0, 0, 'line(\".\") == 1') line('.')"), ["3 3"]);
    ex(&mut core, view, "call cursor(1, 1)");
    assert_eq!(echo(&mut core, "echo search('one', 'W', 0, 0, {-> line('.') == 1}) line('.')"), ["3 3"]);
    ex(&mut core, view, "call cursor(1, 1)");
    assert_eq!(echo(&mut core, "echo search('one', 'nW', 0, 0, '0') line('.')"), ["3 1"]);
    // An error in the skip expression propagates and leaves the cursor.
    ex(&mut core, view, "call cursor(2, 2)");
    let outcome = ex(&mut core, view, "echo search('one', 'W', 0, 0, 'nosuchvar')");
    assert!(error_text(&outcome).contains("E121"), "{outcome:?}");
    assert_eq!(echo(&mut core, "echo line('.') col('.')"), ["2 2"]);
}

#[test]
fn vim9_functions_compile_ex_commands_by_their_own_grammar() {
    let directory = std::env::temp_dir().join(format!("viem-vim9-ex-compile-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("compile.vim");
    std::fs::write(&path, "vim9script\ndef Replace()\n  s/old/[new]/\n  normal! A(z)\nenddef\nsetline(1, 'old')\nReplace()\n").unwrap();
    let (mut core, view) = editor("before\n");
    let outcome = ex(&mut core, view, &format!("source {}", path.display()));
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "[new](z)\n");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn line_values_swap_carriage_returns_and_line_feeds_in_mac_files() {
    let document =
        Document::from_bytes_with_file_format(b"a\nb\rnext\r".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Mac).unwrap();
    let mut core: Editor = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    // A literal LF inside a line reads as CR, because CR ends lines here.
    let outcome = ex(&mut core, view, "echo line('$') strlen(getline(1)) char2nr(getline(1)[1])");
    assert_eq!(messages(&outcome), vec!["2 3 13".to_string()]);
    ex(&mut core, view, "call setline(1, getline(1))");
    assert_eq!(core.document().source_bytes(), b"a\nb\rnext\r");
    // A CR in a value is stored as a literal LF and keeps one line.
    let outcome = ex(&mut core, view, "call setline(2, \"x\\r\") | echo line('$') strlen(getline(2)) char2nr(getline(2)[1])");
    assert_eq!(messages(&outcome), vec!["2 2 13".to_string()]);
    assert_eq!(core.document().source_bytes(), b"a\nb\rx\n\r");
    // LF in a value is still NUL.
    ex(&mut core, view, "call setline(1, \"c\\nd\")");
    assert_eq!(core.document().source_bytes(), b"c\0d\rx\n\r");
}

#[test]
fn appended_script_lines_use_one_dos_line_ending() {
    let document =
        Document::from_bytes_with_file_format(b"a\r\nb\r\n".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Dos).unwrap();
    let mut core: Editor = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    ex(&mut core, view, "call append(1, ['x', 'y'])");
    assert_eq!(core.document().source_bytes(), b"a\r\nx\r\ny\r\nb\r\n");
    ex(&mut core, view, "call setline(5, 'z') | call append(0, 'top')");
    assert_eq!(core.document().source_bytes(), b"top\r\na\r\nx\r\ny\r\nb\r\nz\r\n");
    // Each command line is one undo step.
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), b"a\r\nx\r\ny\r\nb\r\n");
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), b"a\r\nb\r\n");
}

/// Source `script` into a fresh editor holding `text` and return the editor
/// with the outcome.
fn source_script(name: &str, text: &str, script: &str) -> (Editor, ViewId, CoreOutcome) {
    let directory = std::env::temp_dir().join(format!("viem-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("script.vim");
    std::fs::write(&path, script).unwrap();
    let (mut core, view) = editor(text);
    let outcome = ex(&mut core, view, &format!("source {}", path.display()));
    std::fs::remove_dir_all(directory).unwrap();
    (core, view, outcome)
}

#[test]
fn setreg_writes_are_visible_to_later_commands_in_the_same_script() {
    let (core, _, outcome) = source_script(
        "setreg-visible",
        "one\n",
        "let @a = 'L1' | call setreg('a', 'word') | put a\ncall setreg('b', 'B1')\ncall setreg('b', 'B2')\nnormal! gg\"bp\n",
    );
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(core.document().text(), "oB2ne\nword\n");
}

#[test]
fn normal_without_a_range_runs_at_the_cursor() {
    let (mut core, view, outcome) = source_script(
        "normal-cursor",
        "one two three four\nnext line\n",
        "call cursor(1, 5)\nnormal! w\nlet g:first = col('.')\nnormal! w\nlet g:second = col('.')\ncall cursor(1, 5)\nnormal! iX\n",
    );
    assert_eq!(status(&outcome), CommandStatus::Complete, "{outcome:?}");
    assert_eq!(messages(&ex(&mut core, view, "echo g:first g:second")), ["9 15"]);
    assert_eq!(core.document().text(), "one Xtwo three four\nnext line\n");
    // A range still starts each line at its beginning.
    ex(&mut core, view, "call cursor(2, 4) | 2normal! iY");
    assert_eq!(core.document().text(), "one Xtwo three four\nYnext line\n");
}

#[test]
fn failures_inside_normal_reach_try_and_let_the_script_continue() {
    let (core, _, outcome) = source_script(
        "normal-failure",
        "abc\n",
        "try\n  normal! qaq\ncatch\n  let g:caught = 1\nendtry\nnormal! qaq\ncall setline(1, 'after ' .. g:caught)\n",
    );
    // The uncaught failure is reported, and the rest of the script runs.
    assert!(matches!(status(&outcome), CommandStatus::Error(text) if text.contains("macro recording")), "{outcome:?}");
    assert_eq!(core.document().text(), "after 1\n");
}

#[test]
fn setline_and_append_keep_empty_lines_at_an_unterminated_end() {
    let (mut core, view) = editor("");
    ex(&mut core, view, "call setline(1, ['a', '', 'b'])");
    assert_eq!(core.document().source_bytes(), b"a\n\nb");
    ex(&mut core, view, "%d _ | call setline(1, 'z') | call append(1, ['', 'b', '', ''])");
    assert_eq!(messages(&ex(&mut core, view, "echo string(getline(1, '$'))")), ["['z', '', 'b', '', '']"]);
    ex(&mut core, view, "%d _ | call append(0, ['p', ''])");
    assert_eq!(messages(&ex(&mut core, view, "echo string(getline(1, '$'))")), ["['p', '', '']"]);
    ex(&mut core, view, "%d _ | call setline(1, 'z') | call append(1, ['', 'b'])");
    assert_eq!(core.document().source_bytes(), b"z\n\nb");
}

#[test]
fn command_exists_uses_editor_host_capabilities() {
    let (mut core, view) = editor("abc");
    let outcome = ex(&mut core, view, "echo exists(':2match') exists(':3match')");
    assert_eq!(status(&outcome), CommandStatus::Complete);
    assert_eq!(messages(&outcome), ["2 2"]);
}
