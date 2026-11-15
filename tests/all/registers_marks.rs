use viem_core::command::clipboard::ClipboardCommandContext;
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{
    ArtifactBinding, ArtifactIdentity, ArtifactPath, Encoding, Format, LineEndingOpenPolicy,
    LoadedArtifact,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};
type Editor = Core<MockTextMeasurementProvider>;
fn editor(text: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    (core, view)
}
fn key(core: &mut Editor, view: ViewId, key: Key) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap()
}
fn keys(core: &mut Editor, view: ViewId, text: &str) -> CoreOutcome {
    text.chars()
        .map(|ch| key(core, view, Key::Char(ch)))
        .last()
        .unwrap()
}
fn ex(core: &mut Editor, view: ViewId, command: &str) -> CoreOutcome {
    core.stage_command_line(view, command).unwrap();
    key(core, view, Key::Enter)
}
fn cursor(core: &Editor, view: ViewId) -> usize {
    core.command_state(view).unwrap().cursor()
}
#[test]
fn search_command_and_filename_registers_share_all_read_paths() {
    let (mut core, view) = editor("one two");
    ex(&mut core, view, "let @/ = 'two'");
    keys(&mut core, view, "\"/2p");
    assert_eq!(core.document().text(), "otwotwone two");
    keys(&mut core, view, "u");
    ex(&mut core, view, "put /");
    assert_eq!(core.document().text(), "one two\ntwo");
    keys(&mut core, view, "u");
    ex(&mut core, view, "echo 'hello'");
    keys(&mut core, view, "\":p");
    assert!(core.document().text().contains("echo 'hello'"));
    let document = Document::from_loaded_artifact(
        LoadedArtifact::new(
            ArtifactBinding::new(
                ArtifactPath::from_utf8("/tmp/notes.txt"),
                ArtifactIdentity::new(b"notes".to_vec()),
            ),
            b"text".to_vec(),
        ),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    let output = ex(&mut core, view, "echo @% getreg('%')");
    assert!(
        format!("{output:?}").contains("/tmp/notes.txt /tmp/notes.txt"),
        "{output:?}"
    );
    core.set_alternate_artifact(view, Some(ArtifactPath::from_utf8("/tmp/previous.txt")))
        .unwrap();
    assert!(format!("{:?}", ex(&mut core, view, "echo @#")).contains("/tmp/previous.txt"));
    keys(&mut core, view, "\"#p");
    assert!(core.document().text().contains("/tmp/previous.txt"));
}
#[test]
fn expression_register_prompts_put_insert_and_nest_without_losing_outer_prompt() {
    let (mut core, view) = editor("ab");
    keys(&mut core, view, "2\"=1 + 2");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "p");
    assert_eq!(core.document().text(), "a33b");
    keys(&mut core, view, "u");
    keys(&mut core, view, "i");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "='é'");
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "éab");
    ex(&mut core, view, "let g:answer = 6");
    keys(&mut core, view, ":echo ");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "=g:answer * 7");
    key(&mut core, view, Key::Enter);
    assert_eq!(
        core.command_state(view).unwrap().command_line(),
        Some("echo 42")
    );
    assert!(format!("{:?}", key(&mut core, view, Key::Enter)).contains("42"));
    keys(&mut core, view, ":echo 'keep'");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "=bad(");
    key(&mut core, view, Key::Escape);
    assert_eq!(
        core.command_state(view).unwrap().command_line(),
        Some("echo 'keep'")
    );
    key(&mut core, view, Key::Escape);
    let before = core.document().source_bytes();
    keys(&mut core, view, "\"=1 /");
    let failed = key(&mut core, view, Key::Enter);
    assert!(matches!(
        failed.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    assert_eq!(core.document().source_bytes(), before);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
}
#[test]
fn affected_marks_last_insert_and_previous_position_are_usable_and_rebased() {
    let (mut core, view) = editor("one\n  two\nthree");
    keys(&mut core, view, "j0y$");
    keys(&mut core, view, "gg`]");
    assert_eq!(cursor(&core, view), 8);
    keys(&mut core, view, "`[");
    assert_eq!(cursor(&core, view), 4);
    keys(&mut core, view, "ggG``");
    assert_eq!(cursor(&core, view), 0);
    keys(&mut core, view, "``");
    assert_eq!(cursor(&core, view), 10);
    keys(&mut core, view, "ggiXY");
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "G`^");
    assert_eq!(cursor(&core, view), 2);
    keys(&mut core, view, "`[");
    assert_eq!(cursor(&core, view), 0);
    keys(&mut core, view, "`]");
    assert_eq!(cursor(&core, view), 1);
    keys(&mut core, view, "`.");
    assert_eq!(cursor(&core, view), 1);
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    keys(&mut core, other, "`]");
    assert_eq!(cursor(&core, other), 1);
    keys(&mut core, view, "ggd`]");
    assert_eq!(core.document().text(), "Yone\n  two\nthree");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "XYone\n  two\nthree");
}
#[test]
fn read_only_register_destinations_fail_without_changing_source_or_unnamed() {
    for name in ['/', ':', '#', '~', '%'] {
        let (mut core, view) = editor("one\ntwo");
        keys(&mut core, view, "yy");
        let revision = core.document().revision();
        let before = core.command_state(view).unwrap().register('"').cloned();
        let result = keys(&mut core, view, &format!("\"{name}dd"));
        assert!(
            matches!(
                result.command.unwrap().status,
                CommandStatus::RegisterWriteError(_)
            ),
            "{name}"
        );
        assert_eq!(core.document().revision(), revision);
        assert_eq!(
            core.command_state(view).unwrap().register('"'),
            before.as_ref()
        );
    }
}
#[test]
fn drop_and_alternate_registers_do_not_share_view_context() {
    let (mut core, a) = editor("ab");
    let b = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    core.set_alternate_artifact(a, Some(ArtifactPath::from_utf8("first")))
        .unwrap();
    core.set_alternate_artifact(b, Some(ArtifactPath::from_utf8("second")))
        .unwrap();
    core.set_drop_register(
        a,
        viem_core::command::RegisterValue::characterwise("dropped"),
    )
    .unwrap();
    let read = |view, name| {
        core.command_state(view)
            .unwrap()
            .register_with_context(core.document(), &ClipboardCommandContext::new(), name)
            .unwrap()
            .unwrap()
            .text
    };
    assert_eq!(read(a, '#'), "first");
    assert_eq!(read(b, '#'), "second");
    assert_eq!(read(b, '~'), "dropped");
}
#[test]
fn expression_lists_ex_put_repeat_history_and_failed_textlock() {
    let (mut core, view) = editor("one");
    let output = ex(&mut core, view, "put =['two', 'three']");
    assert!(matches!(
        output.command.unwrap().status,
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "one\ntwo\nthree");
    keys(&mut core, view, "u");
    ex(&mut core, view, "let @= = '21 * 2'");
    assert!(format!(
        "{:?}",
        ex(&mut core, view, "echo getreg('=', 1) getreg('=')")
    )
    .contains("21 * 2 42"));
    keys(&mut core, view, "\"=");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "p.");
    assert_eq!(core.document().text(), "o4242ne");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "o42ne");
    let before = core.document().source_bytes();
    keys(&mut core, view, "\"=setline(1, 'bad')");
    let outcome = key(&mut core, view, Key::Enter);
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    assert_eq!(core.document().source_bytes(), before);
}
#[test]
fn expression_errors_clear_prefix_and_recursive_reads_are_bounded() {
    let (mut core, view) = editor("abcdef");
    keys(&mut core, view, "3\"=unknown(");
    assert!(matches!(
        key(&mut core, view, Key::Enter).command.unwrap().status,
        CommandStatus::Error(_)
    ));
    keys(&mut core, view, "x");
    assert_eq!(
        core.document().text(),
        "bcdef",
        "failed expression must not leave count three pending"
    );
    ex(&mut core, view, "let @= = \"getregtype('==')\"");
    keys(&mut core, view, "\"=");
    let outcome = key(&mut core, view, Key::Enter);
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    keys(&mut core, view, "2\"=1");
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "x");
    assert_eq!(core.document().text(), "cdef");
}
#[test]
fn expression_register_macro_and_normal_execution_keep_one_undo_unit() {
    let (mut core, view) = editor("abc");
    keys(&mut core, view, "qq\"=7");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "pq@q");
    assert_eq!(core.document().text(), "a77bc");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a7bc");
    ex(&mut core, view, "normal! \"/p"); // Empty search register is a no-op.
    ex(&mut core, view, "let @/ = 'search'");
    // Like Vim, a range-less :normal puts after the cursor, which undo left
    // on the 7.
    ex(&mut core, view, "normal! \"/p");
    assert_eq!(core.document().text(), "a7searchbc");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a7bc");
}
#[test]
fn visual_mark_setting_and_script_positions_use_current_locations() {
    let (mut core, view) = editor("one\n  two");
    keys(&mut core, view, "jllvmA");
    key(&mut core, view, Key::Escape);
    let output = ex(&mut core, view, "echo getpos(\"'A\") col(\"'A\")");
    assert!(
        format!("{output:?}").contains("[0, 2, 3, 0] 3"),
        "{output:?}"
    );
    ex(&mut core, view, "delmarks A");
    let output = ex(&mut core, view, "echo getpos(\"'A\")");
    assert!(format!("{output:?}").contains("[0, 0, 0, 0]"), "{output:?}");
}
#[test]
fn expression_put_inside_global_uses_the_same_evaluator_and_undo_unit() {
    let (mut core, view) = editor("one\ntwo");
    let outcome = ex(&mut core, view, "g/o/put =6 * 7");
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "one\n42\ntwo\n42");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "one\ntwo");
}
#[test]
fn global_deletion_rebases_the_new_previous_jump_mark() {
    for format in [Format::PlainText, Format::Code, Format::Markdown] {
        let document = Document::from_bytes(
            b"First paragraph.\nSecond line.\n\nLast line.".to_vec(),
            Encoding::Utf8,
            format,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        keys(&mut core, view, "G");
        let before = core.document().source_bytes();
        let result = ex(&mut core, view, "g/a/d");
        assert!(matches!(
            result.command.unwrap().status,
            CommandStatus::Complete
        ));
        keys(&mut core, view, "``");
        assert!(cursor(&core, view) <= core.document().text().len());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), before);
    }
    let mut document = Document::new("First paragraph.\nSecond line.\n\nLast line.");
    let mut commands = viem_core::command::CommandInterpreter::new();
    for ch in "G:g/a/d".chars() {
        commands.handle(&mut document, InputEvent::key(ch)).unwrap();
    }
    let output = commands
        .handle(&mut document, InputEvent::Key(Key::Enter))
        .unwrap();
    assert!(matches!(output.status, CommandStatus::Complete));
    for ch in "``".chars() {
        commands.handle(&mut document, InputEvent::key(ch)).unwrap();
    }
    assert!(commands.cursor() <= document.text().len());
}

fn messages(outcome: &CoreOutcome) -> Vec<String> {
    use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
    outcome.command.as_ref().and_then(|command| command.ex_outcome.as_ref())
        .map(|ex| ex.frontend_requests.iter().filter_map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()),
            _ => None,
        }).collect())
        .unwrap_or_default()
}

fn register(core: &Editor, view: ViewId, name: char) -> (String, viem_core::command::RegisterKind, Vec<usize>) {
    let value = core.command_state(view).unwrap().register(name).unwrap();
    (value.text.clone(), value.kind, value.hard_break_offsets().to_vec())
}

#[test]
fn setreg_types_and_inference_match_vim() {
    use viem_core::command::RegisterKind::{Blockwise, Characterwise, Linewise};
    let (mut core, view) = editor("one");
    for (call, text, kind, breaks) in [
        ("setreg('a', 'qq')", "qq", Characterwise, vec![]),
        ("setreg('a', \"qq\\n\")", "qq\n", Linewise, vec![2]),
        ("setreg('a', \"x\\ny\")", "x\ny", Characterwise, vec![1]),
        ("setreg('a', 'qq', 'l')", "qq\n", Linewise, vec![2]),
        ("setreg('a', 'qq', 'V')", "qq\n", Linewise, vec![2]),
        ("setreg('a', \"qq\\n\", 'c')", "qq\n", Characterwise, vec![2]),
        ("setreg('a', \"qq\\n\", 'v')", "qq\n", Characterwise, vec![2]),
        ("setreg('a', \"x\\nyy\", 'b')", "x\nyy", Blockwise, vec![]),
        ("setreg('a', \"x\\nyy\", nr2char(22) .. '4')", "x\nyy", Blockwise, vec![]),
        ("setreg('a', ['x', 'y'])", "x\ny\n", Linewise, vec![1, 3]),
        ("setreg('a', ['x', 'y'], 'c')", "x\ny", Characterwise, vec![1]),
        ("setreg('a', ['x', 'yy'], 'b')", "x\nyy", Blockwise, vec![]),
        // A line feed inside a list item is that line's literal content.
        ("setreg('a', [\"x\\ny\"])", "x\ny\n", Linewise, vec![3]),
    ] {
        ex(&mut core, view, &format!("call {call}"));
        assert_eq!(register(&core, view, 'a'), (text.to_owned(), kind, breaks), "{call}");
    }
    ex(&mut core, view, "call setreg('a', 'qq', 'l')");
    ex(&mut core, view, "call setreg('a', 'rr', 'a')");
    assert_eq!(register(&core, view, 'a'), ("qq\nrr".to_owned(), Characterwise, vec![2]));
    for (call, expected) in [
        ("setreg('a', 'qq')", "v"),
        ("setreg('a', 'qq', 'l')", "V"),
        ("setreg('a', \"x\\nyyy\", 'b')", "\u{16}3"),
    ] {
        ex(&mut core, view, &format!("call {call}"));
        assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a')")), [expected], "{call}");
    }
    ex(&mut core, view, "call setreg('a', [\"x\\ny\", 'z'])");
    assert_eq!(messages(&ex(&mut core, view, "echo len(getreg('a', 1, 1)) getreg('a', 1, 1)[0] ==# \"x\\ny\"")), ["2 1"]);
}

#[test]
fn script_register_appends_take_the_appended_type() {
    use viem_core::command::RegisterKind::{Characterwise, Linewise};
    let (mut core, view) = editor("one");
    for (name, first, append, text, kind, breaks) in [
        ('a', "setreg('a', 'old', 'l')", "setreg('a', 'new', 'a')", "old\nnew", Characterwise, vec![3]),
        ('a', "setreg('a', 'old', 'l')", "setreg('A', 'new')", "old\nnew", Characterwise, vec![3]),
        ('a', "setreg('a', 'old', 'l')", "let @A = 'new'", "old\nnew", Characterwise, vec![3]),
        // A String continues a characterwise register's last line.
        ('a', "setreg('a', 'old', 'c')", "let @A = \"new\\n\"", "oldnew\n", Linewise, vec![6]),
        ('a', "setreg('a', 'old', 'c')", "setreg('a', \"new\\n\", 'ac')", "oldnew\n", Characterwise, vec![6]),
        // List items are always new lines.
        ('a', "setreg('a', 'old', 'c')", "setreg('a', ['n1', 'n2'], 'a')", "old\nn1\nn2\n", Linewise, vec![3, 6, 9]),
        ('a', "setreg('a', 'old', 'c')", "setreg('a', [], 'a')", "old\n", Linewise, vec![3]),
        ('a', "setreg('a', 'old', 'l')", "setreg('a', 'new', 'al')", "old\nnew\n", Linewise, vec![3, 7]),
        // The `a` option appends to registers without an uppercase name.
        ('1', "setreg('1', 'old', 'c')", "setreg('1', 'new', 'ac')", "oldnew", Characterwise, vec![]),
    ] {
        ex(&mut core, view, &format!("call {first}"));
        let call = if append.starts_with("let") { append.to_owned() } else { format!("call {append}") };
        ex(&mut core, view, &call);
        assert_eq!(register(&core, view, name), (text.to_owned(), kind, breaks), "{first} then {append}");
    }
}

#[test]
fn script_register_writes_leave_the_unnamed_register_alone() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "call setreg('\"', 'keep')");
    for write in ["call setreg('a', 'scratch')", "let @b = 'temp'", "call setreg('c', 'cc', 'a')", "call setreg('d', 'dee', 'u')"] {
        ex(&mut core, view, write);
        assert_eq!(messages(&ex(&mut core, view, "echo getreg('\"')")), ["keep"], "{write}");
    }
    keys(&mut core, view, "p");
    assert_eq!(core.document().text(), "okeepne");
    ex(&mut core, view, "let @\" = 'direct'");
    assert_eq!(messages(&ex(&mut core, view, "echo getreg('\"') getreg('0') getreg('d')")), ["direct direct dee"]);
}

#[test]
fn setreg_with_an_empty_list_empties_the_register() {
    let (mut core, view) = editor("one");
    for options in ["''", "'l'", "'c'", "'b'"] {
        ex(&mut core, view, "call setreg('a', 'x')");
        ex(&mut core, view, &format!("call setreg('a', [], {options})"));
        assert_eq!(
            messages(&ex(&mut core, view, "echo string(getreg('a')) string(getreg('a', 1, 1)) string(getregtype('a'))")),
            ["'' [] ''"],
            "{options}"
        );
    }
    // Appending an empty List keeps the contents but takes its linewise type.
    ex(&mut core, view, "call setreg('a', 'x')");
    ex(&mut core, view, "call setreg('a', [], 'a')");
    assert_eq!(messages(&ex(&mut core, view, "echo string(getreg('a', 1, 1)) getregtype('a')")), ["['x'] V"]);
}

#[test]
fn search_and_expression_registers_keep_the_register_api() {
    let (mut core, view) = editor("one");
    ex(&mut core, view, "call setreg('/', ['needle'])");
    assert_eq!(messages(&ex(&mut core, view, "echo getreg('/') getregtype('/')")), ["needle v"]);
    let outcome = ex(&mut core, view, "call setreg('/', ['a', 'b'])");
    assert!(matches!(outcome.command.as_ref().map(|command| &command.status), Some(CommandStatus::Error(text)) if text.contains("E883")), "{outcome:?}");
    assert_eq!(messages(&ex(&mut core, view, "echo getreg('/')")), ["needle"]);
    ex(&mut core, view, "call setreg('/', [])");
    assert_eq!(messages(&ex(&mut core, view, "echo string(getreg('/'))")), ["''"]);

    ex(&mut core, view, "let @= = '1 + 2'");
    assert_eq!(
        messages(&ex(&mut core, view, "echo getreg('=') string(getreg('=', 1, 1)) string(getreg('=', 0, 1)) @= getregtype('=')")),
        ["3 ['1 + 2'] ['3'] 1 + 2 v"]
    );
    ex(&mut core, view, "call setreg('=', ['3 + 4'])");
    assert_eq!(messages(&ex(&mut core, view, "echo getreg('=', 1) getreg('=')")), ["3 + 4 7"]);
    assert_eq!(messages(&ex(&mut core, view, "echo getregtype(':') getregtype('.') getregtype('%') string(getregtype('-'))")), ["v v v ''"]);
}

#[test]
fn block_register_widths_match_vim_and_round_trip() {
    let (mut core, view) = editor("one");
    for (setup, width) in [
        ("call setreg('a', ['x', 'yy'], 'b5')", "5"),
        ("call setreg('a', ['x', 'yy'], 'b')", "2"),
        ("call setreg('a', ['x', 'yy'], 'b1')", "1"),
        // Wide characters take two cells; ASCII, a tab included, one.
        ("call setreg('a', ['\u{65e5}\u{672c}'], 'b')", "4"),
        ("call setreg('a', [\"a\\tb\"], 'b')", "3"),
        ("call setreg('a', [\"e\\u0301\"], 'b')", "1"),
        // An append takes the width of the appended rows.
        ("call setreg('a', ['o1', 'o22'], 'b') | call setreg('a', ['n1', 'n2'], 'ab')", "2"),
        ("call setreg('a', 'x', 'b5') | call setreg('a', 'y', 'ab')", "1"),
    ] {
        ex(&mut core, view, setup);
        assert_eq!(messages(&ex(&mut core, view, "echo getregtype('a')")), [format!("\u{16}{width}")], "{setup}");
    }
    ex(&mut core, view, "call setreg('a', ['x', 'yy'], 'b5') | call setreg('b', getreg('a', 1, 1), getregtype('a'))");
    assert_eq!(messages(&ex(&mut core, view, "echo getregtype('b') getreg('b', 1, 1)")), ["\u{16}5 ['x', 'yy']"]);
}

#[test]
fn getreginfo_and_setreg_dictionaries_round_trip_registers() {
    let (mut core, view) = editor("one two");
    let echo = |core: &mut Editor, line: &str| messages(&ex(core, view, line));
    assert_eq!(echo(&mut core, "echo exists('*getreginfo') getreginfo('z')"), ["1 {}"]);
    ex(&mut core, view, "call setreg('a', ['x', 'yy'], 'b5')");
    assert_eq!(echo(&mut core, "echo string(getreginfo('a'))"), ["{'isunnamed': v:false, 'regcontents': ['x', 'yy'], 'regtype': '\u{16}5'}"]);
    // Saving and restoring keeps the contents, type and width exactly.
    ex(&mut core, view, "let g:saved = getreginfo('a') | call setreg('a', 'junk') | call setreg('a', g:saved)");
    assert_eq!(echo(&mut core, "echo getreginfo('a') == g:saved"), ["1"]);
    // The unnamed register points to the register a change last wrote.
    ex(&mut core, view, "call setreg('\"', 'unnamed')");
    assert_eq!(echo(&mut core, "echo getreginfo('\"').points_to getreginfo('0').isunnamed"), ["0 v:true"]);
    keys(&mut core, view, "\"byiw");
    assert_eq!(echo(&mut core, "echo getreginfo().points_to getreginfo('b').isunnamed getreginfo('0').isunnamed"), ["b v:true v:false"]);
    keys(&mut core, view, "x");
    assert_eq!(echo(&mut core, "echo getreginfo().points_to"), ["-"]);
    // Without regcontents only the unnamed pointer can change.
    ex(&mut core, view, "call setreg('c', {'regcontents': ['p', 'q'], 'regtype': 'V'}) | call setreg('c', {'regtype': 'v'})");
    assert_eq!(echo(&mut core, "echo getreg('c', 1, 1) getregtype('c')"), ["['p', 'q'] V"]);
    ex(&mut core, view, "call setreg('c', {'isunnamed': v:true})");
    assert_eq!(echo(&mut core, "echo getreg('\"', 1, 1) getreginfo().points_to"), ["['p', 'q'] c"]);
    ex(&mut core, view, "call setreg('\"', {'points_to': 'b'})");
    assert_eq!(echo(&mut core, "echo getreg('\"') getreginfo().points_to"), ["one b"]);
    ex(&mut core, view, "call setreg('e', {})");
    assert_eq!(echo(&mut core, "echo getreginfo('e')"), ["{}"]);
    let outcome = ex(&mut core, view, "call setreg('x', {'regcontents': ['a'], 'regtype': 'q'})");
    assert!(matches!(outcome.command.as_ref().map(|command| &command.status), Some(CommandStatus::Error(text)) if text.contains("E475")), "{outcome:?}");
    assert_eq!(echo(&mut core, "echo getreginfo('x') string(getreginfo('='))"), ["{} {'isunnamed': v:false, 'regcontents': [''], 'regtype': 'v'}"]);
}

#[test]
fn script_linewise_register_puts_whole_lines() {
    for (format, source, keys_text, expected, expected_source) in [
        (Format::Markdown, "p\n\nz", "\"ap", "p\nqq\nz", "p\n\nqq\n\nz"),
        (Format::Markdown, "p\n\nz", "\"aP", "qq\np\nz", "qq\n\np\n\nz"),
        (Format::PlainText, "p\nz", "\"ap", "p\nqq\nz", "p\nqq\nz"),
        (Format::PlainText, "p\nz", "\"aP", "qq\np\nz", "qq\np\nz"),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        ex(&mut core, view, "let @a = \"qq\\n\"");
        keys(&mut core, view, keys_text);
        assert_eq!(core.document().text(), expected, "{format:?} {keys_text}");
        assert_eq!(core.document().source_bytes(), expected_source.as_bytes(), "{format:?} {keys_text}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{format:?} {keys_text}: undo");
    }
}
