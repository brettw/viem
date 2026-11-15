//! Recorded Vim 9.2 behavior for ranged calls, mapping metadata, redirection
//! and explicit versus automatic autocommands.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let document = Document::from_bytes_with_file_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    (core, view)
}

fn press(core: &mut Editor, view: ViewId, key: Key) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap()
}

fn ex(core: &mut Editor, view: ViewId, line: &str) -> CoreOutcome {
    press(core, view, Key::Char(':'));
    for c in line.chars() { press(core, view, Key::Char(c)); }
    let outcome = press(core, view, Key::Enter);
    assert_eq!(outcome.command.as_ref().map_or(CommandStatus::Complete, |command| command.status.clone()), CommandStatus::Complete, "{line}: {outcome:?}");
    outcome
}

fn messages(outcome: &CoreOutcome) -> Vec<String> {
    outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).map(|ex| ex.frontend_requests.iter().filter_map(|request| match request {
        ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()),
        _ => None,
    }).collect()).unwrap_or_default()
}

fn source(core: &mut Editor, view: ViewId, name: &str, script: &str) {
    let path = std::env::temp_dir().join(format!("viem-event-compat-{}-{name}.vim", std::process::id()));
    std::fs::write(&path, script).unwrap();
    ex(core, view, &format!("source {}", path.display()));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn ranged_user_call_evaluates_arguments_at_each_addressed_line() {
    let (mut core, view) = editor("one\ntwo\nthree");
    source(&mut core, view, "ranged-user", "let g:stamps = []\nfunction! Stamp(n) abort\n call add(g:stamps, [a:n, line('.')])\nendfunction\n");
    ex(&mut core, view, "call cursor(2, 1)");
    ex(&mut core, view, "1,3call Stamp(line('.'))");
    assert_eq!(messages(&ex(&mut core, view, "echo g:stamps")), vec!["[[1, 1], [2, 2], [3, 3]]"]);
}

#[test]
fn ranged_builtin_call_edits_each_line_in_one_undo_unit() {
    let (mut core, view) = editor("one\ntwo\nthree");
    ex(&mut core, view, "call cursor(2, 1)");
    ex(&mut core, view, "1,3call setline('.', 'X')");
    assert_eq!(core.document().source_bytes(), b"X\nX\nX");
    press(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), b"one\ntwo\nthree");
    press(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), b"X\nX\nX");
}

#[test]
fn range_function_receives_the_range_once_at_its_first_line() {
    let (mut core, view) = editor("one\ntwo\nthree");
    source(&mut core, view, "ranged-function", "let g:stamps = []\nfunction! Stamp(n) range abort\n call add(g:stamps, [a:n, a:firstline, a:lastline, line('.')])\nendfunction\n");
    ex(&mut core, view, "call cursor(2, 1)");
    ex(&mut core, view, "1,3call Stamp(line('.'))");
    assert_eq!(messages(&ex(&mut core, view, "echo g:stamps")), vec!["[[1, 1, 3, 1]]"]);
}

#[test]
fn filetype_handlers_observe_each_assignment_before_the_next() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:types = []");
    ex(&mut core, view, "autocmd FileType * call add(g:types, [&ft, expand('<amatch>')])");
    ex(&mut core, view, "set ft=rust ft=python");
    assert_eq!(messages(&ex(&mut core, view, "echo g:types")), vec!["[['rust', 'rust'], ['python', 'python']]"]);
    ex(&mut core, view, "let &comments = 'b:ft='");
    assert_eq!(messages(&ex(&mut core, view, "echo len(g:types)")), vec!["2"]);
    ex(&mut core, view, "let &ft = 'java'");
    assert_eq!(messages(&ex(&mut core, view, "echo g:types[-1]")), vec!["['java', 'java']"]);
}

#[test]
fn once_autocommands_are_consumed_independently_per_pattern() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:seen = []");
    ex(&mut core, view, "autocmd User Red,Blue ++once call add(g:seen, expand('<amatch>'))");
    for pattern in ["Red", "Blue", "Red", "Blue"] { ex(&mut core, view, &format!("doautocmd User {pattern}")); }
    assert_eq!(messages(&ex(&mut core, view, "echo g:seen")), vec!["['Red', 'Blue']"]);
}

#[test]
fn explicit_doautocmd_runs_inside_a_non_nested_handler() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "let g:seen = []");
    ex(&mut core, view, "autocmd User Outer doautocmd User Inner");
    ex(&mut core, view, "autocmd User Inner call add(g:seen, 'inner')");
    ex(&mut core, view, "doautocmd User Outer");
    assert_eq!(messages(&ex(&mut core, view, "echo g:seen")), vec!["['inner']"]);
    ex(&mut core, view, "noautocmd doautocmd User Outer");
    assert_eq!(messages(&ex(&mut core, view, "echo len(g:seen)")), vec!["1"]);
}

#[test]
fn user_command_register_consumes_one_character_without_a_separator() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "command! -register -nargs=* Take echo [<q-reg>, <q-args>]");
    assert_eq!(messages(&ex(&mut core, view, "Take abc")), vec!["['a', 'bc']"]);
    assert_eq!(messages(&ex(&mut core, view, "Take a bc")), vec!["['a', 'bc']"]);
}

#[test]
fn sourced_mapping_metadata_keeps_raw_key_bytes_and_source_line() {
    let (mut core, view) = editor("text");
    source(&mut core, view, "mapping", "nnoremap <F2> x\nnnoremap <C-W> y\n");
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('<F2>', 'n', 0, 1).lnum strlen(maparg('<F2>', 'n', 0, 1).lhsraw) char2nr(maparg('<F2>', 'n', 0, 1).lhsraw)")), vec!["1 3 128"]);
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('<F2>', 'n', 0, 1).lhsraw[1:] maparg('<C-W>', 'n', 0, 1).lnum char2nr(maparg('<C-W>', 'n', 0, 1).lhsraw)")), vec!["k2 2 23"]);
}

#[test]
fn register_redirection_is_live_across_separate_commands() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call setreg('q', 'old')");
    ex(&mut core, view, "redir @q");
    ex(&mut core, view, "let g:cleared = empty(getreg('q'))");
    ex(&mut core, view, "echo 'hello'");
    ex(&mut core, view, r#"let g:live = getreg('q') == "\nhello""#);
    ex(&mut core, view, "redir END");
    assert_eq!(messages(&ex(&mut core, view, r#"echo [g:cleared, g:live, getreg('q') == "\nhello"]"#)), vec!["[1, 1, 1]"]);
}

#[test]
fn sourced_register_redirection_is_live_and_can_append() {
    let (mut core, view) = editor("text");
    source(&mut core, view, "redir", "call setreg('q', 'old')\nredir @q\nlet g:cleared = empty(getreg('q'))\necho 'hello'\nlet g:live = getreg('q') == \"\\nhello\"\nredir END\nredir @Q\nechon '!'\nredir END\n");
    assert_eq!(messages(&ex(&mut core, view, r#"echo [g:cleared, g:live, getreg('q') == "\nhello!"]"#)), vec!["[1, 1, 1]"]);
}

#[test]
fn mapping_register_prefix_updates_later_script_queries_in_the_same_rhs() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "let g:seen = []");
    ex(&mut core, view, "call setreg('a', 'chosen', 'v') | call setreg('b', 'other', 'V')");
    ex(&mut core, view, "nnoremap Q :call add(g:seen, [v:register, getreg() == getreg(v:register), getregtype() == getregtype(v:register)])<CR>\"byy:call add(g:seen, [v:register, getreg() == getreg(v:register), getregtype() == getregtype(v:register)])<CR>");
    for c in "\"aQ".chars() { press(&mut core, view, Key::Char(c)); }
    assert_eq!(messages(&ex(&mut core, view, "echo g:seen")), vec!["[['a', 1, 1], ['b', 1, 1]]"]);
}

#[test]
fn nested_mapping_register_selection_remains_current_when_the_caller_resumes() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "let g:seen = []");
    ex(&mut core, view, "nnoremap R \"cyy:call add(g:seen, v:register)<CR>");
    ex(&mut core, view, "nmap Q :call add(g:seen, v:register)<CR>R:call add(g:seen, v:register)<CR>");
    for c in "\"aQ".chars() { press(&mut core, view, Key::Char(c)); }
    assert_eq!(messages(&ex(&mut core, view, "echo g:seen")), vec!["['a', 'c', 'c']"]);
}
