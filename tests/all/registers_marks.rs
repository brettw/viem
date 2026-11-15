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
    assert_eq!(cursor(&core, view), 2);
    keys(&mut core, view, "`.");
    assert_eq!(cursor(&core, view), 1);
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    keys(&mut core, other, "`]");
    assert_eq!(cursor(&core, other), 2);
    keys(&mut core, view, "ggd`]");
    assert_eq!(core.document().text(), "one\n  two\nthree");
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
        // A file mark names its buffer, as in Vim.
        format!("{output:?}").contains("[1, 2, 3, 0] 3"),
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
fn script_register_writes_reject_lists_and_invalid_names() {
    let (mut core, view) = editor("one");
    let error = |core: &mut Editor, line: &str| match ex(core, view, line).command.map(|command| command.status) {
        Some(CommandStatus::Error(text)) => text,
        other => panic!("{line}: expected an error, got {other:?}"),
    };
    assert!(error(&mut core, "let @a = ['x']").contains("E730"));
    for name in ["!", "?", "$", ":", ".", "%", "~"] {
        assert!(error(&mut core, &format!("call setreg('{name}', 'x')")).contains("E354"), "{name}");
    }
    assert!(error(&mut core, "let @: = 'x'").contains("E354"));
    // Unknown setreg() options are ignored.
    assert_eq!(messages(&ex(&mut core, view, "echo setreg('a', 'x', 'q') getreg('a') getregtype('a')")), ["0 x v"]);
}

#[test]
fn setpos_sets_marks_and_mark_commands_use_the_first_nonblank() {
    let (mut core, view) = editor("one\n  two words\nthree");
    let echo = |core: &mut Editor, line: &str| messages(&ex(core, view, line));
    assert_eq!(echo(&mut core, "echo setpos(\"'a\", [0, 2, 3, 0]) getpos(\"'a\")"), ["0 [0, 2, 3, 0]"]);
    // A file mark names its buffer.
    assert_eq!(echo(&mut core, "echo setpos(\"'A\", [0, 3, 2, 0]) getpos(\"'A\")"), ["0 [1, 3, 2, 0]"]);
    assert_eq!(echo(&mut core, "echo setcharpos(\"'b\", [0, 2, 4, 0]) getpos(\"'b\")"), ["0 [0, 2, 4, 0]"]);
    assert_eq!(echo(&mut core, "echo setpos(\"'<\", [0, 1, 2, 0]) setpos(\"'>\", [0, 2, 4, 0]) getpos(\"'<\") getpos(\"'>\")"),
        ["0 0 [0, 1, 2, 0] [0, 2, 4, 0]"]);
    // Marks follow the text, so they cannot be placed past its end.
    assert_eq!(echo(&mut core, "echo setpos(\"'z\", [0, 9, 1, 0]) setpos(\"'y\", [0, 1, 99, 0]) getpos(\"'y\")"), ["-1 0 [0, 1, 4, 0]"]);
    keys(&mut core, view, "G");
    ex(&mut core, view, "mark c | 2mark d | 1k e | 2kf");
    assert_eq!(echo(&mut core, "echo getpos(\"'c\") getpos(\"'d\") getpos(\"'e\") getpos(\"'f\")"),
        ["[0, 3, 1, 0] [0, 2, 3, 0] [0, 1, 1, 0] [0, 2, 3, 0]"]);
    assert_eq!(echo(&mut core, "echo getpos('x') getpos('nosuch') getpos(\"'Q\")"), ["[0, 0, 0, 0] [0, 0, 0, 0] [0, 0, 0, 0]"]);
}

#[test]
fn linewise_visual_and_yank_marks_report_the_maximum_column() {
    let (mut core, view) = editor("one\ntwo words\nthree");
    let echo = |core: &mut Editor, line: &str| messages(&ex(core, view, line));
    keys(&mut core, view, "Vjy");
    assert_eq!(echo(&mut core, "echo getpos(\"'>\") getpos(\"']\") col(\"'>\") getcharpos(\"'>\") getpos(\"'<\")"),
        ["[0, 2, 2147483647, 0] [0, 2, 2147483647, 0] 10 [0, 2, 9, 0] [0, 1, 1, 0]"]);
    keys(&mut core, view, "ggyiw");
    assert_eq!(echo(&mut core, "echo getpos(\"']\") getpos(\"'>\")"), ["[0, 1, 3, 0] [0, 2, 2147483647, 0]"]);
    ex(&mut core, view, "2yank");
    assert_eq!(echo(&mut core, "echo getpos(\"']\")"), ["[0, 2, 2147483647, 0]"]);
    // A column given to setpos() replaces the maximum column.
    ex(&mut core, view, "call setpos(\"'>\", [0, 2, 4, 0])");
    assert_eq!(echo(&mut core, "echo getpos(\"'>\")"), ["[0, 2, 4, 0]"]);
    keys(&mut core, view, "jVd");
    assert_eq!(echo(&mut core, "echo getpos(\"']\")"), ["[0, 2, 1, 0]"]);
}

#[test]
fn change_end_mark_never_rests_on_a_line_break() {
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    let marks = |core: &mut Editor| messages(&ex(core, view, "echo getpos(\"'[\")[1:2] getpos(\"']\")[1:2]"));
    // A linewise put ends on the last character of its last line.
    ex(&mut core, view, "2yank");
    keys(&mut core, view, "ggp");
    assert_eq!(marks(&mut core), ["[2, 1] [2, 9]"]);
    keys(&mut core, view, "u");
    ex(&mut core, view, "2,3yank");
    keys(&mut core, view, "ggP");
    assert_eq!(marks(&mut core), ["[1, 1] [2, 5]"]);
    keys(&mut core, view, "u");
    // An empty last line ends at its start.
    ex(&mut core, view, "call setreg('a', ['', 'x', ''], 'l')");
    keys(&mut core, view, "gg\"ap");
    assert_eq!(marks(&mut core), ["[2, 1] [4, 1]"]);
    keys(&mut core, view, "u");
    // So does a characterwise yank that includes the line break.
    keys(&mut core, view, "ggjv$y");
    assert_eq!(marks(&mut core), ["[2, 1] [2, 9]"]);
    // A characterwise change keeps its last character.
    keys(&mut core, view, "0yiwP");
    assert_eq!(marks(&mut core), ["[2, 1] [2, 3]"]);
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

// Vim 9.1.1752 oracle: each operation starts at line 2, column 1 with
// shiftwidth=2 expandtab. Endpoints are getpos()'s byte columns.
const CHANGE_MARK_CASES: &[(&str, &str, &str, &str)] = &[
    (">>", "[2, 1] [2, 11]", "one\n  two words\nthree\nfour", ""),
    ("3>>", "[2, 1] [4, 6]", "one\n  two words\n  three\n  four", ""),
    ("J", "[2, 10] [2, 16]", "one\ntwo words three\nfour", ""),
    ("3J", "[2, 10] [2, 21]", "one\ntwo words three four", ""),
    ("itext", "[2, 1] [2, 5]", "one\ntexttwo words\nthree\nfour", "insert"),
    ("A tail", "[2, 10] [2, 15]", "one\ntwo words tail\nthree\nfour", "insert"),
    ("oadded", "[3, 1] [3, 6]", "one\ntwo words\nadded\nthree\nfour", "insert"),
    ("Oabove", "[2, 1] [2, 6]", "one\nabove\ntwo words\nthree\nfour", "insert"),
    ("ccnew", "[2, 1] [2, 4]", "one\nnew\nthree\nfour", "insert"),
    (":2,3s/o/0/g", "[2, 1] [3, 1]", "one\ntw0 w0rds\nthree\nfour", "ex"),
];
const CHANGE_MARK_QUERY: &str = "echo getpos(\"'[\")[1:2] getpos(\"']\")[1:2]";

#[test]
fn change_marks_match_vim_through_keys_normal_and_scripts() {
    let original = "one\ntwo words\nthree\nfour";
    for route in ["keys", "normal", "script"] {
        for &(input, expected, source, kind) in CHANGE_MARK_CASES {
            let (mut core, view) = editor(original);
            ex(&mut core, view, "set shiftwidth=2 expandtab");
            keys(&mut core, view, "j0");
            let output = if kind == "ex" {
                ex(&mut core, view, &input[1..])
            } else if route == "keys" {
                let output = keys(&mut core, view, input);
                if kind == "insert" { key(&mut core, view, Key::Escape) } else { output }
            } else if route == "normal" {
                // :normal ends Insert mode even without an explicit Escape.
                ex(&mut core, view, &format!("normal! {input}"))
            } else {
                let escape = if kind == "insert" { "\\<Esc>" } else { "" };
                ex(&mut core, view, &format!("execute \"normal! {input}{escape}\""))
            };
            assert!(matches!(output.command.as_ref().unwrap().status, CommandStatus::Complete), "{route} {input}: {output:?}");
            assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), [expected], "{route} {input}");
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{route} {input}");
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), original.as_bytes(), "{route} {input}: undo");
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{route} {input}: redo");
        }
    }
}

#[test]
fn change_marks_match_vim_without_a_coordinator() {
    use viem_core::command::CommandInterpreter;
    use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
    for &(input, expected, source, kind) in CHANGE_MARK_CASES {
        let mut document = Document::new("one\ntwo words\nthree\nfour");
        let mut commands = CommandInterpreter::new();
        let send = |commands: &mut CommandInterpreter, document: &mut Document, input: &str| {
            for ch in input.chars() {
                commands.handle(document, InputEvent::key(ch)).unwrap();
            }
        };
        send(&mut commands, &mut document, ":set sw=2 et");
        commands.handle(&mut document, InputEvent::Key(Key::Enter)).unwrap();
        send(&mut commands, &mut document, "j0");
        send(&mut commands, &mut document, input);
        if kind == "insert" {
            commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        } else if kind == "ex" {
            commands.handle(&mut document, InputEvent::Key(Key::Enter)).unwrap();
        }
        send(&mut commands, &mut document, &format!(":{CHANGE_MARK_QUERY}"));
        let output = commands.handle(&mut document, InputEvent::Key(Key::Enter)).unwrap();
        let messages: Vec<_> = output.ex_outcome.unwrap().frontend_requests.into_iter().filter_map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text),
            _ => None,
        }).collect();
        assert_eq!(messages, [expected], "{input}");
        assert_eq!(document.source_bytes(), source.as_bytes(), "{input}");
    }
}

#[test]
fn substitute_marks_cover_unmatched_lines_and_inserted_line_breaks() {
    for (command, expected) in [
        ("1,4s/words/X/", "[1, 1] [4, 1]"),
        ("1,4s/words/X\\rY/", "[1, 1] [5, 1]"),
        ("2,2s/words/X\\rY/", "[2, 1] [3, 1]"),
    ] {
        let (mut core, view) = editor("one\ntwo words\nthree\nfour");
        ex(&mut core, view, command);
        assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), [expected], "{command}");
    }
}

#[test]
fn change_marks_are_shared_with_other_views() {
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    keys(&mut core, view, "jA tail");
    key(&mut core, view, Key::Escape);
    assert_eq!(messages(&ex(&mut core, other, CHANGE_MARK_QUERY)), ["[2, 10] [2, 15]"]);
}

#[test]
fn ex_join_and_shift_publish_command_mark_ranges() {
    for (command, expected) in [
        ("2,4>", "[2, 1] [4, 6]"),
        ("2,3join", "[2, 10] [2, 16]"),
        ("2,4join!", "[2, 10] [2, 19]"),
    ] {
        let (mut core, view) = editor("one\ntwo words\nthree\nfour");
        ex(&mut core, view, "set sw=2 et");
        ex(&mut core, view, command);
        assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), [expected], "{command}");
    }
}

#[test]
fn insertion_mark_units_follow_counts_movement_and_multibyte_text() {
    for (input, expected) in [
        ("3iXY", "[2, 1] [2, 7]"),
        ("ié日", "[2, 1] [2, 6]"),
        ("Rxy", "[2, 1] [2, 3]"),
    ] {
        let (mut core, view) = editor("one\ntwo words\nthree\nfour");
        keys(&mut core, view, "j0");
        keys(&mut core, view, input);
        key(&mut core, view, Key::Escape);
        assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), [expected], "{input}");
    }
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    keys(&mut core, view, "jiXY");
    key(&mut core, view, Key::Left);
    keys(&mut core, view, "Z");
    key(&mut core, view, Key::Escape);
    assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), ["[2, 2] [2, 3]"]);
}

#[test]
fn semantic_change_marks_are_readable_in_the_same_script() {
    use viem_core::command::CommandInterpreter;
    use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
    for &(input, expected, _, kind) in CHANGE_MARK_CASES {
        let mut document = Document::new("one\ntwo words\nthree\nfour");
        let mut commands = CommandInterpreter::new();
        let operation = if kind == "ex" { input[1..].to_owned() }
            else { format!("execute 'normal! {input}'") };
        assert!(commands.set_cursor(&document, 4));
        let script = format!(":set sw=2 et | {operation} | {CHANGE_MARK_QUERY}");
        for ch in script.chars() {
            commands.handle(&mut document, InputEvent::key(ch)).unwrap();
        }
        let output = commands.handle(&mut document, InputEvent::Key(Key::Enter)).unwrap();
        assert!(matches!(output.status, CommandStatus::Complete), "{input}: {output:?}");
        let messages: Vec<_> = output.ex_outcome.unwrap().frontend_requests.into_iter().filter_map(|request| match request {
            ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text),
            _ => None,
        }).collect();
        assert_eq!(messages, [expected], "{input}");
    }
}

#[test]
fn failed_changes_preserve_affected_marks() {
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    keys(&mut core, view, "jiXY");
    key(&mut core, view, Key::Escape);
    let before = messages(&ex(&mut core, view, CHANGE_MARK_QUERY));
    let source = core.document().source_bytes();
    ex(&mut core, view, "set nomodifiable");
    keys(&mut core, view, ">>");
    assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), before);
    assert_eq!(core.document().source_bytes(), source);
    ex(&mut core, view, "set modifiable");
    ex(&mut core, view, "1,4s/missing/never/");
    assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), before);
    assert_eq!(core.document().source_bytes(), source);
}

#[test]
fn substitute_confirmation_marks_cover_the_addressed_range() {
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    ex(&mut core, view, "2,4s/o/X/gc");
    keys(&mut core, view, "yq");
    assert_eq!(core.document().text(), "one\ntwX words\nthree\nfour");
    assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), ["[2, 1] [4, 1]"]);
}

#[test]
fn substitute_without_matches_preserves_marks_when_errors_are_suppressed() {
    let (mut core, view) = editor("one\ntwo words\nthree\nfour");
    ex(&mut core, view, "call setpos(\"'[\", [0, 2, 3, 0]) | call setpos(\"']\", [0, 3, 2, 0])");
    ex(&mut core, view, "1,4s/missing/X/e");
    assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), ["[2, 3] [3, 2]"]);
}

#[test]
fn opened_line_marks_include_generated_indentation() {
    for (input, expected) in [("oadded", "[3, 1] [3, 8]"), ("Oabove", "[2, 1] [2, 8]")] {
        let (mut core, view) = editor("one\n  two words\nthree\nfour");
        ex(&mut core, view, "set autoindent");
        keys(&mut core, view, "j0");
        keys(&mut core, view, input);
        key(&mut core, view, Key::Escape);
        assert_eq!(messages(&ex(&mut core, view, CHANGE_MARK_QUERY)), [expected], "{input}");
    }
}
