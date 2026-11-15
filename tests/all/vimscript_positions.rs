//! Editor-facing Vim script compatibility, checked against Vim 9.1.1752.
use viem_core::command::{InputEvent, Key, CommandStatus};
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};
type Editor = Core<MockTextMeasurementProvider>;
fn editor(text: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    (core, view)
}
fn ex(core: &mut Editor, view: ViewId, text: &str) -> CoreOutcome {
    core.stage_command_line(view, text).unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap()
}
fn echo(core: &mut Editor, view: ViewId, text: &str) -> Vec<String> {
    let output = ex(core, view, text);
    assert!(matches!(output.command.as_ref().unwrap().status, CommandStatus::Complete), "{text}: {output:?}");
    output.command.unwrap().ex_outcome.unwrap().frontend_requests.into_iter().filter_map(|request| match request {
        ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text), _ => None,
    }).collect()
}
#[test]
fn position_lists_and_numeric_argument_errors_match_vim() {
    let (mut core, view) = editor("one\n\tété words\nthree\nfour");
    ex(&mut core, view, "set tabstop=8");
    for (expression, expected) in [
        ("line([2,1])", "2"), ("line(2)", "0"), ("line('2')", "0"),
        ("col([2,2])", "2"), ("col([2,'$'])", "13"), ("col([99,2])", "0"),
        ("col([2,99])", "0"), ("virtcol([2,2])", "9"), ("charcol([2,4])", "4"),
        ("setpos('.', [1,1])", "-1"),
    ] { assert_eq!(echo(&mut core, view, &format!("echo {expression}")), [expected], "{expression}"); }
    for (expression, code) in [("col(1)", "E1222"), ("cursor(-1,-1)", "E475")] {
        assert!(format!("{:?}", ex(&mut core, view, &format!("echo {expression}"))).contains(code));
    }
}
#[test]
fn cursor_desired_column_and_character_positions_round_trip() {
    let (mut core, view) = editor("one\n\tété words\nthree\nfour");
    assert_eq!(echo(&mut core, view, "call cursor([2,4,0,20]) | echo getcurpos()"), ["[0, 2, 4, 0, 20]"]);
    assert_eq!(echo(&mut core, view, "call setpos('.', [0,2,2,0,17]) | echo getcurpos() getcursorcharpos()"), ["[0, 2, 2, 0, 17] [0, 2, 2, 0, 17]"]);
}
#[test]
fn absent_buffer_and_window_queries_do_not_answer_for_current_objects() {
    let (mut core, view) = editor("one\ntwo\nthree");
    assert_eq!(echo(&mut core, view, "echo getbufline(99,1) string(getbufoneline(99,1)) bufexists(99) bufnr('nosuch') getbufvar(99,'x',17) getcurpos(99) winwidth(99)"),
        ["[] '' 0 -1 17 [0, 0, 0, 0, 0] -1"]);
}
#[test]
fn buffer_line_mutations_use_physical_lines_and_undo() {
    let (mut core, view) = editor("one\ntwo\nthree");
    assert_eq!(echo(&mut core, view, "echo setline(1,{}) getline(1)"), ["0 {}"]);
    ex(&mut core, view, "undo");
    assert_eq!(echo(&mut core, view, "echo setbufline(1,2,'TWO') appendbufline(1,2,['a','b']) deletebufline(1,1)"), ["0 0 0"]);
    assert_eq!(core.document().source_bytes(), b"TWO\na\nb\nthree");
    ex(&mut core, view, "undo");
    assert_eq!(core.document().source_bytes(), b"one\ntwo\nthree");
    assert_eq!(echo(&mut core, view, "echo setbufline(99,1,'x') appendbufline(99,0,'x') deletebufline(99,1)"), ["1 1 1"]);
}
#[test]
fn window_variables_and_indirect_option_assignments_are_visible_immediately() {
    let (mut core, view) = editor("one\ntwo");
    assert_eq!(echo(&mut core, view, "call setwinvar(0,'wv',7) | call setbufvar(1,'&tabstop',4) | call setwinvar(0,'&number',1) | echo w:wv &tabstop &number"), ["7 4 1"]);
    let other = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    assert_eq!(echo(&mut core, other, "echo exists('w:wv')"), ["0"]);
}
#[test]
fn history_queries_select_the_requested_history_and_reject_unknown_types() {
    let (mut core, view) = editor("one");
    assert_eq!(echo(&mut core, view, "echo histnr('nosuch') histadd('nosuch','x') histadd('/','xxx') histnr('/') histget('/',-1)"), ["-1 0 1 1 xxx"]);
}
#[test]
fn search_flags_captures_smartcase_and_multiline_matches_follow_vim() {
    let (mut core, view) = editor("one\ntwo WORDS\nthree\nfour");
    ex(&mut core, view, "set ignorecase smartcase");
    assert_eq!(echo(&mut core, view, "echo search('Words','nW') search('words','nW') search('\\(two\\)','npW') searchpos('\\(two\\)','npW')"), ["0 2 2 [2, 1, 2]"]);
    assert_eq!(echo(&mut core, view, "echo searchpos('WORDS\\nthree','nW')"), ["[2, 5]"]);
    assert!(format!("{:?}", ex(&mut core, view, "echo search('x','bad')")).contains("E475"));
    let (mut core, view) = editor("one");
    assert!(format!("{:?}", ex(&mut core, view, "echo search('')")).contains("E35"));
}
#[test]
fn cursor_actions_within_echo_preserve_a_single_message() {
    let (mut core, view) = editor("one\ntwo");
    assert_eq!(echo(&mut core, view, "echo 'a' search('two') 'b'"), ["a 2 b"]);
}
#[test]
fn feedkeys_execute_mode_applies_keys_before_returning() {
    let (mut core, view) = editor("one");
    assert_eq!(echo(&mut core, view, "call feedkeys('iXY', 'nx') | echo getline(1) mode()"), ["XYone n"]);
}
#[test]
fn normal_applies_a_mapping_whose_second_key_is_q() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "nnoremap zq ix<Esc>");
    ex(&mut core, view, "normal zq");
    assert_eq!(core.document().text(), "xone");
}
#[test]
fn wordcount_reports_cursor_progress_and_byte2line_rejects_past_eof() {
    let (mut core, view) = editor("one\n\tété words\nthree\nfour");
    ex(&mut core, view, "call cursor(2,2)");
    assert_eq!(echo(&mut core, view, "echo wordcount().cursor_words wordcount().cursor_chars wordcount().cursor_bytes wordcount().bytes byte2line(9999)"), ["2 6 7 28 -1"]);
}
#[test]
fn expand_preserves_an_unmatched_glob() {
    let (mut core, view) = editor("one");
    assert_eq!(echo(&mut core, view, "echo expand('nosuch*glob')"), ["nosuch*glob"]);
}

#[test]
fn number_formats_control_normal_and_visual_number_changes() {
    for (source, formats, expected) in [
        ("007", None, "010"), ("077", None, "0100"), ("007", Some("bin,hex"), "008"),
        ("-001", Some("bin,hex"), "000"),
        ("0x0f", Some(""), "1x0f"), ("a", Some("alpha"), "b"),
        ("-007", Some("unsigned"), "-008"),
    ] {
        let (mut core, view) = editor(source);
        if let Some(formats) = formats { ex(&mut core, view, &format!("set nrformats={formats}")); }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('a')))).unwrap();
        assert_eq!(core.document().text(), expected, "{source} {formats:?}");
        ex(&mut core, view, "undo");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let (mut core, view) = editor("007\n077");
    ex(&mut core, view, "execute \"normal! Vj\\<C-A>\"");
    assert_eq!(core.document().text(), "010\n0100");
}

#[test]
fn paired_search_tracks_nesting_and_can_preserve_the_cursor() {
    let (mut core, view) = editor("{\n {\n  x\n }\n}");
    assert_eq!(echo(&mut core, view, "echo searchpair('{','','}','nW') getpos('.')[1:2]"), ["5 [1, 1]"]);
    assert_eq!(echo(&mut core, view, "echo searchpairpos('{','','}','W') getpos('.')[1:2]"), ["[5, 1] [5, 1]"]);
    assert_eq!(echo(&mut core, view, "echo searchpairpos('{','','}','bW')"), ["[1, 1]"]);
}

#[test]
fn search_counts_and_declaration_queries_use_current_source() {
    let (mut core, view) = editor("one one\none");
    ex(&mut core, view, "call cursor(2,1)");
    assert_eq!(echo(&mut core, view, "let d = searchcount({'pattern':'one','timeout':0}) | echo d.current d.total d.exact_match d.incomplete"), ["3 3 1 0"]);
    assert_eq!(echo(&mut core, view, "echo searchcount({'pattern':'one','maxcount':1,'timeout':0}).incomplete"), ["2"]);
    let (mut core, view) = editor("int value;\nvoid f()\n{\n int value;\n use(value);\n}");
    assert_eq!(echo(&mut core, view, "call cursor(5,6) | echo searchdecl('value') getpos('.')[1:2]"), ["0 [4, 6]"]);
    assert_eq!(echo(&mut core, view, "call cursor(5,6) | echo searchdecl('value',1) getpos('.')[1:2]"), ["0 [1, 5]"]);
}

#[test]
fn introspection_returns_current_objects_and_rejects_missing_ones() {
    let (mut core, view) = editor("one\ntwo");
    assert_eq!(echo(&mut core, view, "echo getbufinfo()[0].bufnr getwininfo()[0].winid gettabinfo()[0].tabnr win_id2win(1000) win_id2win(9999) len(getbufinfo(99)) len(getwininfo(99)) len(gettabinfo(99))"), ["1 1000 1 1 0 0 0 0"]);
    assert_eq!(echo(&mut core, view, "call setpos(\"'a\",[0,2,2,0]) | echo getmarklist(1)[0].mark getmarklist(1)[0].pos"), ["'a [1, 2, 2, 0]"]);
    assert_eq!(echo(&mut core, view, "echo type(getjumplist()[0]) getjumplist(99)"), ["3 [[], 0]"]);
}

#[test]
fn undo_tree_reports_branches_and_current_sequence() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "call setline(1,'first')");
    ex(&mut core, view, "undo");
    ex(&mut core, view, "call setline(1,'second')");
    assert_eq!(echo(&mut core, view, "let d = undotree() | echo d.seq_last d.seq_cur len(d.entries) d.entries[0].seq d.entries[0].alt[0].seq"), ["2 2 1 2 1"]);
    ex(&mut core, view, "undo");
    assert_eq!(echo(&mut core, view, "echo undotree().seq_cur"), ["0"]);
}

#[test]
fn undojoin_amends_the_previous_change_and_rejects_history_navigation() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "call setline(1,'first')");
    let joined = ex(&mut core, view, "undojoin");
    assert!(matches!(joined.command.unwrap().status, CommandStatus::Complete));
    ex(&mut core, view, "call setline(1,'second')");
    ex(&mut core, view, "undo");
    assert_eq!(core.document().source_bytes(), b"one");
    assert!(format!("{:?}", ex(&mut core, view, "undojoin")).contains("E790"));
    ex(&mut core, view, "redo");
    assert_eq!(core.document().source_bytes(), b"second");
    assert!(format!("{:?}", ex(&mut core, view, "undojoin")).contains("E790"));
}

#[test]
fn ex_register_commands_execute_script_lines_and_visual_range_aliases() {
    let (mut core, view) = editor("one\ntwo\nthree");
    assert_eq!(echo(&mut core, view, "let @q = 'let g:ran = get(g:, \"ran\", 0) + 1' | @q | echo g:ran"), ["1"]);
    ex(&mut core, view, "call setpos(\"'<\",[0,1,1,0]) | call setpos(\"'>\",[0,2,1,0])");
    ex(&mut core, view, "*@q");
    assert_eq!(echo(&mut core, view, "echo g:ran line('.')"), ["2 2"]);
}

#[test]
fn expand_reports_the_source_line_of_a_sourced_script() {
    let path = std::env::temp_dir().join(format!("viem-script-line-{}.vim", std::process::id()));
    std::fs::write(&path, "let g:line = 0\nlet g:line = expand('<slnum>')\n").unwrap();
    let (mut core, view) = editor("one");
    let result = ex(&mut core, view, &format!("source {}", path.display()));
    std::fs::remove_file(path).unwrap();
    assert!(matches!(result.command.unwrap().status, CommandStatus::Complete));
    assert_eq!(echo(&mut core, view, "echo g:line"), ["2"]);
}

#[test]
fn position_lists_validate_lines_and_character_columns_without_losing_byte_columns() {
    let (mut core, view) = editor("one\n\tété words");
    ex(&mut core, view, "set tabstop=8");
    assert_eq!(echo(&mut core, view, "echo line([99,1]) line([-1,1]) col([2,3]) charcol([2,'$']) charcol([2,12])"), ["0 0 3 11 0"]);
    assert_eq!(echo(&mut core, view, "echo setpos('.',[0,2,2]) getcurpos()"), ["0 [0, 2, 2, 0, 9]"]);
    assert_eq!(echo(&mut core, view, "echo setpos('.',[0,2,2,0,-1]) getcurpos()"), ["0 [0, 2, 2, 0, 9]"]);
}

#[test]
fn deleting_buffer_lines_keeps_the_cursor_with_surviving_text() {
    let (mut core, view) = editor("one\ntwo\nthree\nfour");
    assert_eq!(echo(&mut core, view, "call cursor(4,3) | call deletebufline(1,1,2) | echo getpos('.')[1:2]"), ["[2, 3]"]);
    ex(&mut core, view, "undo");
    assert_eq!(core.document().source_bytes(), b"one\ntwo\nthree\nfour");
    assert_eq!(echo(&mut core, view, "call deletebufline(1,1,'$') | echo getline(1,'$')"), ["['']"]);
}

#[test]
fn paired_search_skip_errors_restore_the_original_cursor() {
    let (mut core, view) = editor("{\n {\n  x\n }\n}");
    assert_eq!(echo(&mut core, view, "echo searchpair('{','','}','W','line(\".\") == 2') getpos('.')[1:2]"), ["4 [4, 2]"]);
    ex(&mut core, view, "call cursor(1,1)");
    assert!(format!("{:?}", ex(&mut core, view, "echo searchpair('{','','}','W','NoSuchFunction()')")).contains("E117"));
    assert_eq!(echo(&mut core, view, "echo getpos('.')[1:2]"), ["[1, 1]"]);
}

#[test]
fn failed_edits_preserve_a_pending_undo_join() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "call setline(1,'first')");
    ex(&mut core, view, "undojoin");
    ex(&mut core, view, "set nomodifiable");
    ex(&mut core, view, "call setline(1,'rejected')");
    assert_eq!(core.document().text(), "first");
    ex(&mut core, view, "set modifiable");
    ex(&mut core, view, "call setline(1,'second')");
    ex(&mut core, view, "undo");
    assert_eq!(core.document().source_bytes(), b"one");
}

#[test]
fn search_saves_the_previous_position_only_when_requested() {
    let (mut core, view) = editor("one\ntwo\nthree");
    assert_eq!(echo(&mut core, view, "call cursor(1,2) | echo search('two','sW') getpos(\"''\")[1:2]"), ["2 [1, 2]"]);
    let (mut core, view) = editor("été\nlast");
    assert_eq!(echo(&mut core, view, "echo searchpos('été','ceW')"), ["[1, 4]"]);
}

#[test]
fn empty_undo_tree_reports_a_synced_initial_state() {
    let (mut core, view) = editor("one");
    assert_eq!(echo(&mut core, view, "let d = undotree() | echo d.seq_cur d.seq_last d.time_cur d.synced d.entries"), ["0 0 0 1 []"]);
}

#[test]
fn window_ids_and_buffer_existence_use_vims_selector_rules() {
    let (mut core, view) = editor("one");
    assert_eq!(echo(&mut core, view, "echo bufexists('') bufnr('') bufexists('%') win_getid(1000) win_id2win(0) win_id2win(1) getwininfo(1) gettabinfo(0)"), ["0 1 0 0 0 0 [] []"]);
    assert_eq!(echo(&mut core, view, "call setwinvar(0,'wv',7) | echo getwininfo()[0].variables.wv"), ["7"]);
}
