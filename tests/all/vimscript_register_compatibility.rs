//! Vim-compatible script registers, replay context, ranges and history.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
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
fn press(core: &mut Editor, view: ViewId, event: InputEvent) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(event)).unwrap()
}
fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for c in text.chars() {
        let result = press(core, view, InputEvent::key(c));
        assert!(matches!(status(&result), CommandStatus::Complete | CommandStatus::Pending), "key {c:?}: {:?}", status(&result));
    }
}
fn ex(core: &mut Editor, view: ViewId, text: &str) -> CoreOutcome {
    press(core, view, InputEvent::key(':'));
    for c in text.chars() { press(core, view, InputEvent::key(c)); }
    let result = press(core, view, InputEvent::Key(Key::Enter));
    assert_eq!(status(&result), CommandStatus::Complete, "{text}");
    result
}
fn status(outcome: &CoreOutcome) -> CommandStatus {
    outcome.command.as_ref().map_or(CommandStatus::Complete, |command| command.status.clone())
}
fn messages(outcome: &CoreOutcome) -> Vec<String> {
    outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref()).into_iter()
        .flat_map(|outcome| &outcome.frontend_requests).filter_map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()), _ => None,
        }).collect()
}
fn source(core: &mut Editor, view: ViewId, name: &str, text: &str) {
    let path = std::env::temp_dir().join(format!("viem-script-register-{name}-{}.vim", std::process::id()));
    std::fs::write(&path, text).unwrap();
    ex(core, view, &format!("source {}", path.display()));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn semicolon_offsets_resolve_from_the_preceding_address() {
    let (mut core, view) = editor("one\ntwo\nthree\nfour\nfive\n");
    ex(&mut core, view, "command! -range Drop <line1>,<line2>delete _");
    ex(&mut core, view, "3;+1Drop");
    assert_eq!(core.document().text(), "one\ntwo\nfive\n");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "one\ntwo\nthree\nfour\nfive\n");
    press(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().text(), "one\ntwo\nfive\n");
}

#[test]
fn text_register_macros_replay_escape_from_source_and_entered_commands() {
    for sourced in [false, true] {
        let (mut core, view) = editor("abc");
        let text = "call setreg('q', \"A!\\<Esc>\")\nnormal! gg0@q";
        if sourced { source(&mut core, view, "macro", text); }
        else { for line in text.lines() { ex(&mut core, view, line); } }
        assert_eq!(core.document().source_bytes(), b"abc!");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), b"abc");
        press(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
        assert_eq!(core.document().source_bytes(), b"abc!");
    }
}

#[test]
fn histdel_filters_only_the_requested_history_and_entries() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call histadd('/', 'keepsearch') | call histadd('/', 'dropsearch')");
    ex(&mut core, view, "call histdel('/', 'dropsearch')");
    assert_eq!(messages(&ex(&mut core, view, "echo histget('/', -1) histget('/', -2)")), ["keepsearch "]);
    ex(&mut core, view, "call histadd('/', 'last') | call histdel('/', -1)");
    assert_eq!(messages(&ex(&mut core, view, "echo histget('/', -1)")), ["keepsearch"]);
}

#[test]
fn expression_register_list_reads_keep_the_evaluated_string_whole() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call setreg('=', \"['one','two']\")");
    assert_eq!(messages(&ex(&mut core, view, "echo len(getreg('=', 0, 1)) str2list(getreg('=', 0, 1)[0])")), ["1 [111, 110, 101, 10, 116, 119, 111, 10]"]);
}

#[test]
fn script_registers_preserve_non_utf8_bytes_and_append_them() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call setreg('a', \"\\xff\") | call setreg('A', \"\\xfe\")");
    ex(&mut core, view, "call assert_equal(\"\\xff\\xfe\", getreg('a')) | call assert_equal([\"\\xff\\xfe\"], getreg('a', 0, 1))");
    assert_eq!(messages(&ex(&mut core, view, "echo v:errors getregtype('a')")), ["[] v"]);
    keys(&mut core, view, "\"a");
    let rejected = press(&mut core, view, InputEvent::key('p'));
    assert!(matches!(status(&rejected), CommandStatus::RegisterReadError(_)));
    assert_eq!(core.document().source_bytes(), b"text");
    ex(&mut core, view, "call assert_equal(\"\\xff\\xfe\", getreg('a'))");
    assert_eq!(messages(&ex(&mut core, view, "echo v:errors")), ["[]"]);
}

#[test]
fn byte_register_rows_preserve_literal_lf_through_getreginfo_round_trips() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call setreg('a', [\"\\xff\\nx\", \"\\xfe\"], 'v')");
    ex(&mut core, view, "let g:saved = getreginfo('a') | call setreg('b', g:saved)");
    ex(&mut core, view, "call assert_equal([\"\\xff\\nx\", \"\\xfe\"], getreg('a', 0, 1)) | call assert_equal(getreg('a', 0, 1), getreg('b', 0, 1))");
    assert_eq!(messages(&ex(&mut core, view, "echo v:errors getregtype('b')")), ["[] v"]);
}

#[test]
fn raw_byte_block_widths_preserve_string_list_and_append_semantics() {
    let (mut core, view) = editor("text");
    for (value, width) in [
        ("\"\\xffx\"", 5), ("[\"\\xffx\"]", 5),
        ("\"\\xe1\\x80\"", 5), ("[\"\\xe1\\x80\"]", 8),
        ("\"\\xc3\"", 1), ("[\"\\xc3\"]", 4),
    ] {
        ex(&mut core, view, &format!("call setreg('a', {value}, 'b')"));
        assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a')")), [format!("^V{width}")], "{value}");
    }
    ex(&mut core, view, "call setreg('a', 'old', 'b2') | call setreg('a', \"\\xffx\", 'ab')");
    assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a')")), ["^V5"]);
}

#[test]
fn trailing_carriage_return_infers_a_linewise_register() {
    let (mut core, view) = editor("text");
    ex(&mut core, view, "call setreg('a', \"x\\r\")");
    assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a') str2list(getreg('a'))")), ["V [120, 13, 10]"]);
}

#[test]
fn mappings_keep_the_selected_register_in_script_calls() {
    let (mut core, view) = editor("text");
    source(&mut core, view, "mapping", "function! Capture()\nlet g:captured = [v:register, getreg(), getregtype()]\nendfunction\nnnoremap Q :call Capture()<CR>");
    ex(&mut core, view, "call setreg('a', 'chosen', 'v') | call setreg('b', 'unnamed', 'V')");
    keys(&mut core, view, "\"aQ");
    assert_eq!(messages(&ex(&mut core, view, "echo g:captured v:register")), ["['a', 'chosen', 'v'] \""]);
}

#[test]
fn yank_events_report_appended_contents_and_boolean_motion_flags() {
    let (mut core, view) = editor("one two");
    ex(&mut core, view, "let g:events = [] | autocmd TextYankPost * call add(g:events, [v:event.regcontents, v:event.inclusive, v:event.visual])");
    keys(&mut core, view, "\"Ayw\"Ayw");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), ["[[['one '], v:false, v:false], [['one one '], v:false, v:false]]"]);
}

#[test]
fn yank_events_keep_inclusive_motion_and_visual_line_flags() {
    let (mut core, view) = editor("one two");
    ex(&mut core, view, "let g:events = [] | autocmd TextYankPost * call add(g:events, [v:event.inclusive, v:event.visual])");
    keys(&mut core, view, "y$Vy");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), ["[[v:true, v:false], [v:true, v:true]]"]);
    ex(&mut core, view, "set selection=exclusive | let g:events = []");
    keys(&mut core, view, "Vy");
    assert_eq!(messages(&ex(&mut core, view, "echo g:events")), ["[[v:true, v:true]]"]);
}

#[test]
fn yank_handlers_can_query_the_executing_macro() {
    let (mut core, view) = editor("one\ntwo");
    ex(&mut core, view, "autocmd TextYankPost * let g:macro = reg_executing()");
    ex(&mut core, view, "call setreg('q', 'yy') | normal! @q");
    assert_eq!(messages(&ex(&mut core, view, "echo g:macro empty(reg_executing())")), ["q 1"]);
}

#[test]
fn counted_block_puts_pad_declared_width_before_surviving_text() {
    let (mut core, view) = editor("ab\ncd");
    ex(&mut core, view, "call setreg('a', ['x', 'y'], 'b4')");
    assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a')")), ["^V4"]);
    keys(&mut core, view, "\"a2p");
    assert_eq!(core.document().text(), "ax   x   b\ncy   y   d");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "ab\ncd");
    press(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().text(), "ax   x   b\ncy   y   d");
}

#[test]
fn counted_block_puts_omit_only_the_last_copys_padding_at_line_end() {
    let (mut core, view) = editor("ab\ncd");
    ex(&mut core, view, "call setreg('a', ['x', 'y'], 'b4')");
    keys(&mut core, view, "$\"a2p");
    assert_eq!(core.document().text(), "abx   x\ncdy   y");
}
