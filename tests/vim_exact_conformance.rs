//! Exact black-box fixtures for the required Vim command surface.
//!
//! `vim_command_matrix` establishes that the grammar accepts every required
//! command. These fixtures complement that smoke coverage by pinning the
//! observable result: document text, cursor byte boundary, mode, register
//! side effects, and undo grouping. They deliberately enter through `Core`
//! rather than constructing the command interpreter directly.

use viem_core::command::{CommandStatus, InputEvent, Key, Mode, RegisterKind};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn new_core(text: &str) -> (TestCore, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    (core, view)
}

fn input(core: &mut TestCore, view: ViewId, event: InputEvent) -> CommandStatus {
    let outcome = core
        .handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| panic!("dispatching {event:?} failed: {error:?}"));
    outcome
        .command
        .unwrap_or_else(|| panic!("input {event:?} produced no command outcome"))
        .status
}

fn press(core: &mut TestCore, view: ViewId, key: Key) -> CommandStatus {
    input(core, view, InputEvent::Key(key))
}

fn keys(core: &mut TestCore, view: ViewId, sequence: &str) -> CommandStatus {
    let mut final_status = None;
    for character in sequence.chars() {
        let status = press(core, view, Key::Char(character));
        assert_success_or_pending(character, &status);
        final_status = Some(status);
    }
    final_status.expect("key sequence must not be empty")
}

fn type_text(core: &mut TestCore, view: ViewId, text: &str) -> CommandStatus {
    let status = input(core, view, InputEvent::Text(text.to_owned()));
    assert_success_or_pending('∅', &status);
    status
}

fn assert_success_or_pending(input: char, status: &CommandStatus) {
    assert!(
        matches!(
            status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "input {input:?} failed as {status:?}"
    );
}

fn escape(core: &mut TestCore, view: ViewId) -> CommandStatus {
    let status = press(core, view, Key::Escape);
    assert_success_or_pending('⎋', &status);
    status
}

fn assert_register(core: &TestCore, view: ViewId, name: char, kind: RegisterKind, text: &str) {
    let value = core
        .command_state(view)
        .and_then(|commands| commands.register(name))
        .unwrap_or_else(|| panic!("register {name:?} was not populated"));
    assert_eq!(value.kind, kind, "register {name:?} kind");
    assert_eq!(value.text, text, "register {name:?} text");
}

fn assert_register_absent(core: &TestCore, view: ViewId, name: char) {
    assert!(
        core.command_state(view)
            .and_then(|commands| commands.register(name))
            .is_none(),
        "register {name:?} should be empty"
    );
}

fn assert_normal_state(core: &TestCore, view: ViewId, text: &str, cursor: usize) {
    assert_eq!(core.document().text(), text);
    let commands = core.command_state(view).expect("view command state");
    assert_eq!(commands.mode(), Mode::Normal);
    assert_eq!(commands.cursor(), cursor);
}

fn assert_undo_redo(
    core: &mut TestCore,
    view: ViewId,
    before: &str,
    after: &str,
    after_cursor: usize,
) {
    assert_eq!(keys(core, view, "u"), CommandStatus::Complete);
    assert_eq!(core.document().text(), before, "undo image");
    assert_eq!(
        press(core, view, Key::Ctrl('r')),
        CommandStatus::Complete,
        "redo status"
    );
    assert_normal_state(core, view, after, after_cursor);
}

#[derive(Clone, Copy)]
struct OperatorFixture {
    name: &'static str,
    initial: &'static str,
    command: &'static str,
    expected: &'static str,
    cursor: usize,
    deleted: &'static str,
    kind: RegisterKind,
}

#[test]
fn operator_counts_and_endpoint_rules_have_exact_results() {
    let fixtures = [
        OperatorFixture {
            name: "operator and motion counts multiply",
            initial: "one two three four five six seven",
            command: "2d3w",
            expected: "seven",
            cursor: 0,
            deleted: "one two three four five six ",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "doubled operator counts hard lines",
            initial: "a\nb\nc\nd\ne",
            command: "2d2d",
            expected: "e",
            cursor: 0,
            deleted: "a\nb\nc\nd\n",
            kind: RegisterKind::Linewise,
        },
        OperatorFixture {
            name: "inner word excludes surrounding space",
            initial: "one two",
            command: "diw",
            expected: " two",
            cursor: 0,
            deleted: "one",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "around word consumes following space",
            initial: "one two",
            command: "daw",
            expected: "two",
            cursor: 0,
            deleted: "one ",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "find motion has an inclusive endpoint",
            initial: "aβcβd",
            command: "dfβ",
            expected: "cβd",
            cursor: 0,
            deleted: "aβ",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "till motion excludes its target grapheme",
            initial: "aβcβd",
            command: "dtβ",
            expected: "βcβd",
            cursor: 0,
            deleted: "a",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "pair match deletion includes both delimiters",
            initial: "(á😀) tail",
            command: "d%",
            expected: " tail",
            cursor: 0,
            deleted: "(á😀)",
            kind: RegisterKind::Characterwise,
        },
        OperatorFixture {
            name: "paragraph motion stops at and preserves the blank line",
            initial: "One. Two.\n\nNext paragraph.",
            command: "d}",
            expected: "\nNext paragraph.",
            cursor: 0,
            deleted: "One. Two.\n",
            kind: RegisterKind::Linewise,
        },
    ];

    for fixture in fixtures {
        let (mut core, view) = new_core(fixture.initial);
        assert_eq!(
            keys(&mut core, view, fixture.command),
            CommandStatus::Complete,
            "{} status",
            fixture.name
        );
        assert_normal_state(&core, view, fixture.expected, fixture.cursor);
        assert_register(&core, view, '"', fixture.kind, fixture.deleted);
        assert_undo_redo(
            &mut core,
            view,
            fixture.initial,
            fixture.expected,
            fixture.cursor,
        );
    }
}

#[test]
fn named_numbered_small_and_black_hole_register_effects_are_exact() {
    let (mut core, view) = new_core("one\ntwo\nthree");
    keys(&mut core, view, "dd\"add");
    assert_normal_state(&core, view, "three", 0);
    assert_register(&core, view, 'a', RegisterKind::Linewise, "two\n");
    assert_register(&core, view, '"', RegisterKind::Linewise, "two\n");
    assert_register(&core, view, '1', RegisterKind::Linewise, "two\n");
    assert_register(&core, view, '2', RegisterKind::Linewise, "one\n");
    assert_register_absent(&core, view, '-');
    keys(&mut core, view, "u");
    assert_normal_state(&core, view, "two\nthree", 0);

    let (mut core, view) = new_core("á😀bc");
    keys(&mut core, view, "x");
    assert_normal_state(&core, view, "😀bc", 0);
    assert_register(&core, view, '-', RegisterKind::Characterwise, "á");
    assert_register(&core, view, '"', RegisterKind::Characterwise, "á");
    assert_register_absent(&core, view, '1');

    keys(&mut core, view, "\"ax");
    assert_normal_state(&core, view, "bc", 0);
    assert_register(&core, view, 'a', RegisterKind::Characterwise, "😀");
    assert_register(&core, view, '"', RegisterKind::Characterwise, "😀");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "😀");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "á");

    keys(&mut core, view, "\"\"x");
    assert_normal_state(&core, view, "c", 0);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "b");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "b");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "á");

    keys(&mut core, view, "\"_x");
    assert_normal_state(&core, view, "", 0);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "b");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "b");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "á");
}

#[test]
fn yank_zero_and_uppercase_append_follow_exact_register_policy() {
    let (mut core, view) = new_core("one two\nthree");
    keys(&mut core, view, "yiw");
    assert_register(&core, view, '0', RegisterKind::Characterwise, "one");
    assert_register(&core, view, '"', RegisterKind::Characterwise, "one");

    keys(&mut core, view, "w\"ayiw");
    assert_register(&core, view, 'a', RegisterKind::Characterwise, "two");
    assert_register(&core, view, '0', RegisterKind::Characterwise, "one");
    assert_register(&core, view, '"', RegisterKind::Characterwise, "two");

    keys(&mut core, view, "\"AyiwG\"Ayy");
    assert_normal_state(&core, view, "one two\nthree", 8);
    assert_register(&core, view, 'a', RegisterKind::Linewise, "twotwo\nthree\n");
    assert_register(&core, view, '"', RegisterKind::Linewise, "twotwo\nthree\n");
    assert_register(&core, view, '0', RegisterKind::Characterwise, "one");
    assert_eq!(core.document().revision().0, 0, "yanks create no history");
}

#[test]
fn case_indent_and_join_commands_pin_text_and_cursor_images() {
    for (_name, initial, command, expected, cursor) in [
        (
            "counted word uppercase",
            "one two three",
            "2gUw",
            "ONE TWO three",
            0,
        ),
        (
            "counted doubled uppercase",
            "ab c\nde f\ngh i",
            "2gUU",
            "AB C\nDE F\ngh i",
            0,
        ),
        (
            "short doubled toggle-case",
            "Ab C\nDe F",
            "g~~",
            "aB c\nDe F",
            0,
        ),
        ("join inserts Vim spacing", "a\nb\nc", "3J", "a b c", 3),
        (
            "g-join removes only source boundaries and indentation",
            "a\n b\n c",
            "3gJ",
            "a b c",
            3,
        ),
        (
            "toggle case count advances over unchanged space",
            "ab cd",
            "3~",
            "AB cd",
            3,
        ),
    ] {
        let (mut core, view) = new_core(initial);
        keys(&mut core, view, command);
        assert_normal_state(&core, view, expected, cursor);
        assert_undo_redo(&mut core, view, initial, expected, cursor);
    }

    let (mut core, view) = new_core("a\nb\nc");
    keys(&mut core, view, "2>>");
    assert_eq!(core.document().text(), "    a\n    b\nc");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a\nb\nc");

    let (mut core, view) = new_core("  a\n    b");
    keys(&mut core, view, "==");
    assert_normal_state(&core, view, "a\n    b", 0);
    assert_undo_redo(&mut core, view, "  a\n    b", "a\n    b", 0);
}

#[test]
fn shorthand_changes_pin_their_ranges_registers_and_cursors() {
    let original = "one two\nnext";

    let (mut core, view) = new_core(original);
    keys(&mut core, view, "wY");
    assert_normal_state(&core, view, original, 4);
    assert_register(&core, view, '0', RegisterKind::Linewise, "one two\n");
    assert_register(&core, view, '"', RegisterKind::Linewise, "one two\n");
    assert_eq!(core.document().revision().0, 0, "Y is a yank, not an edit");

    let (mut core, view) = new_core(original);
    keys(&mut core, view, "wD");
    assert_normal_state(&core, view, "one \nnext", 3);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "two");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "two");
    assert_undo_redo(&mut core, view, original, "one \nnext", 3);

    let (mut core, view) = new_core(original);
    keys(&mut core, view, "wC");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    type_text(&mut core, view, "X");
    escape(&mut core, view);
    assert_normal_state(&core, view, "one X\nnext", 4);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "two");
    assert_undo_redo(&mut core, view, original, "one X\nnext", 4);

    let (mut core, view) = new_core(original);
    keys(&mut core, view, "S");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    type_text(&mut core, view, "X");
    escape(&mut core, view);
    assert_normal_state(&core, view, "X\nnext", 0);
    assert_register(&core, view, '"', RegisterKind::Linewise, "one two\n");
    assert_undo_redo(&mut core, view, original, "X\nnext", 0);

    let (mut core, view) = new_core(original);
    keys(&mut core, view, "2s");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    type_text(&mut core, view, "X");
    escape(&mut core, view);
    assert_normal_state(&core, view, "Xe two\nnext", 0);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "on");
    assert_undo_redo(&mut core, view, original, "Xe two\nnext", 0);

    let (mut core, view) = new_core("aé日");
    keys(&mut core, view, "2rX");
    assert_normal_state(&core, view, "XX日", 1);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "");
    assert_register(&core, view, '.', RegisterKind::Characterwise, "X");
    assert_undo_redo(&mut core, view, "aé日", "XX日", 1);

    let (mut core, view) = new_core("abcdef");
    keys(&mut core, view, "3l2X");
    assert_normal_state(&core, view, "adef", 1);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "bc");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "bc");
    assert_undo_redo(&mut core, view, "abcdef", "adef", 1);
}

#[derive(Clone, Copy)]
struct InsertFixture {
    name: &'static str,
    initial: &'static str,
    entry: &'static str,
    payload: &'static str,
    expected: &'static str,
    cursor: usize,
}

#[test]
fn counted_insert_replace_and_open_line_sessions_are_single_undo_units() {
    let fixtures = [
        InsertFixture {
            name: "counted insert",
            initial: "ab",
            entry: "3i",
            payload: "X",
            expected: "XXXab",
            cursor: 2,
        },
        InsertFixture {
            name: "counted append",
            initial: "ab",
            entry: "3a",
            payload: "X",
            expected: "aXXXb",
            cursor: 3,
        },
        InsertFixture {
            name: "counted first-nonblank insert",
            initial: "  ab",
            entry: "3I",
            payload: "X",
            expected: "  XXXab",
            cursor: 4,
        },
        InsertFixture {
            name: "counted end-of-line append",
            initial: "ab",
            entry: "3A",
            payload: "X",
            expected: "abXXX",
            cursor: 4,
        },
        InsertFixture {
            name: "counted replace replays the typed payload",
            initial: "abcdefghij",
            entry: "3R",
            payload: "XY",
            expected: "XYXYXYghij",
            cursor: 5,
        },
        InsertFixture {
            name: "counted open below repeats whole inserted lines",
            initial: "one\ntwo",
            entry: "3o",
            payload: "X",
            expected: "one\nX\nX\nX\ntwo",
            cursor: 8,
        },
        InsertFixture {
            name: "counted open above repeats whole inserted lines",
            initial: "one\ntwo",
            entry: "3O",
            payload: "X",
            expected: "X\nX\nX\none\ntwo",
            cursor: 4,
        },
    ];

    for fixture in fixtures {
        let (mut core, view) = new_core(fixture.initial);
        assert_eq!(
            keys(&mut core, view, fixture.entry),
            CommandStatus::Complete,
            "{} entry",
            fixture.name
        );
        assert!(matches!(
            core.command_state(view).unwrap().mode(),
            Mode::Insert | Mode::Replace
        ));
        type_text(&mut core, view, fixture.payload);
        escape(&mut core, view);
        assert_normal_state(&core, view, fixture.expected, fixture.cursor);
        assert_register(
            &core,
            view,
            '.',
            RegisterKind::Characterwise,
            fixture.payload,
        );
        assert_undo_redo(
            &mut core,
            view,
            fixture.initial,
            fixture.expected,
            fixture.cursor,
        );
    }
}

#[test]
fn insert_controls_and_change_insert_grouping_are_exact() {
    let (mut core, view) = new_core("base");
    keys(&mut core, view, "A");
    type_text(&mut core, view, " one ábc");
    press(&mut core, view, Key::Ctrl('w'));
    assert_eq!(core.document().text(), "base one ");
    press(&mut core, view, Key::Ctrl('u'));
    assert_eq!(core.document().text(), "base");
    type_text(&mut core, view, "done");
    escape(&mut core, view);
    assert_normal_state(&core, view, "basedone", 7);
    assert_undo_redo(&mut core, view, "base", "basedone", 7);

    let (mut core, view) = new_core("one,two three four");
    keys(&mut core, view, "2cW");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    type_text(&mut core, view, "X");
    escape(&mut core, view);
    assert_normal_state(&core, view, "X four", 0);
    assert_register(
        &core,
        view,
        '"',
        RegisterKind::Characterwise,
        "one,two three",
    );
    assert_undo_redo(&mut core, view, "one,two three four", "X four", 0);

    let (mut core, view) = new_core("one two");
    keys(&mut core, view, "i");
    assert_eq!(
        press(&mut core, view, Key::Ctrl('o')),
        CommandStatus::Complete
    );
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    keys(&mut core, view, "dw");
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    type_text(&mut core, view, "new ");
    escape(&mut core, view);
    assert_normal_state(&core, view, "new two", 3);
    keys(&mut core, view, "u");
    assert_normal_state(&core, view, "two", 0);
    keys(&mut core, view, "u");
    assert_normal_state(&core, view, "one two", 0);
}

#[test]
fn replace_backspace_restores_overwritten_graphemes_in_lifo_order() {
    let original = "á👩‍💻z";
    let (mut core, view) = new_core(original);
    keys(&mut core, view, "R");
    type_text(&mut core, view, "é日");
    assert_eq!(core.document().text(), "é日z");

    press(&mut core, view, Key::Backspace);
    assert_eq!(core.document().text(), "é👩‍💻z");
    assert_eq!(core.command_state(view).unwrap().cursor(), "é".len());
    press(&mut core, view, Key::Backspace);
    assert_eq!(core.document().text(), original);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    escape(&mut core, view);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
}

#[derive(Clone, Copy)]
struct PutFixture {
    command: &'static str,
    expected: &'static str,
    cursor: usize,
}

#[test]
fn characterwise_and_linewise_put_variants_have_exact_cursor_semantics() {
    for fixture in [
        PutFixture {
            command: "\"ap",
            expected: "XY aXYb",
            cursor: 5,
        },
        PutFixture {
            command: "\"aP",
            expected: "XY XYab",
            cursor: 4,
        },
        PutFixture {
            command: "\"agp",
            expected: "XY aXYb",
            cursor: 6,
        },
        PutFixture {
            command: "\"agP",
            expected: "XY XYab",
            cursor: 5,
        },
        PutFixture {
            command: "\"a2p",
            expected: "XY aXYXYb",
            cursor: 7,
        },
    ] {
        let (mut core, view) = new_core("XY ab");
        keys(&mut core, view, "\"ayiww");
        keys(&mut core, view, fixture.command);
        assert_normal_state(&core, view, fixture.expected, fixture.cursor);
        assert_register(&core, view, 'a', RegisterKind::Characterwise, "XY");
        assert_undo_redo(&mut core, view, "XY ab", fixture.expected, fixture.cursor);
    }

    for (command, expected_cursor) in [("\"ap", 10), ("\"agp", 12)] {
        let (mut core, view) = new_core("  X\none\ntwo");
        keys(&mut core, view, "\"ayyj");
        keys(&mut core, view, command);
        assert_normal_state(&core, view, "  X\none\n  X\ntwo", expected_cursor);
        assert_register(&core, view, 'a', RegisterKind::Linewise, "  X\n");
        assert_undo_redo(
            &mut core,
            view,
            "  X\none\ntwo",
            "  X\none\n  X\ntwo",
            expected_cursor,
        );
    }
}

#[test]
fn normal_motion_counts_land_on_exact_grapheme_and_line_boundaries() {
    for (name, initial, setup, command, expected_cursor) in [
        ("grapheme-right", "áé日z", "", "2l", "áé".len()),
        ("word-forward", "one two three", "", "2w", 8),
        ("word-end", "one two three", "", "2e", 6),
        ("word-backward", "one two three", "G", "2b", 0),
        ("counted-find", "abacadaba", "", "2fa", 4),
        ("counted-till", "abacadaba", "", "2ta", 3),
        ("absolute-line", "one\ntwo\nthree", "", "2G", 4),
        ("counted-gg", "one\ntwo\nthree", "G", "2gg", 4),
        ("percentage", "one\ntwo\nthree\nfour", "", "50%", 4),
        (
            "paragraph-forward-stops-on-blank-line",
            "One.\n\nNext.",
            "",
            "}",
            5,
        ),
        (
            "paragraph-backward-stops-on-blank-line",
            "One.\n\nNext.",
            "G",
            "{",
            5,
        ),
    ] {
        let (mut core, view) = new_core(initial);
        if !setup.is_empty() {
            keys(&mut core, view, setup);
        }
        keys(&mut core, view, command);
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            expected_cursor,
            "{name}"
        );
        assert_eq!(core.document().text(), initial, "{name} must not edit");
    }

    let (mut core, view) = new_core("abcd\nxy\nmnop");
    keys(&mut core, view, "2ljj");
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        10,
        "vertical movement retains desired x through a shorter row"
    );
}

#[test]
fn visual_counts_edits_and_pending_cancellation_preserve_exact_modes() {
    let (mut core, view) = new_core("abcdef");
    keys(&mut core, view, "3vd");
    assert_normal_state(&core, view, "def", 0);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "abc");
    assert_register(&core, view, '-', RegisterKind::Characterwise, "abc");
    assert_register_absent(&core, view, '1');
    assert_undo_redo(&mut core, view, "abcdef", "def", 0);

    let (mut core, view) = new_core("one\ntwo\nthree");
    keys(&mut core, view, "2V\"ad");
    assert_normal_state(&core, view, "three", 0);
    assert_register(&core, view, 'a', RegisterKind::Linewise, "one\ntwo\n");
    assert_register(&core, view, '1', RegisterKind::Linewise, "one\ntwo\n");
    assert_undo_redo(&mut core, view, "one\ntwo\nthree", "three", 0);

    let (mut core, view) = new_core("abcdef");
    keys(&mut core, view, "vl");
    assert_eq!(
        core.command_state(view).unwrap().mode(),
        Mode::VisualCharacter
    );
    assert_eq!(keys(&mut core, view, "f"), CommandStatus::Pending);
    assert_eq!(escape(&mut core, view), CommandStatus::Cancelled);
    assert_eq!(
        core.command_state(view).unwrap().mode(),
        Mode::VisualCharacter
    );
    keys(&mut core, view, "d");
    assert_normal_state(&core, view, "cdef", 0);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "ab");
}

#[test]
fn counted_macro_replay_is_one_history_segment_separate_from_recording() {
    let (mut core, view) = new_core("one two three four");
    keys(&mut core, view, "qadwq2@a");
    assert_normal_state(&core, view, "four", 0);

    keys(&mut core, view, "u");
    assert_normal_state(&core, view, "two three four", 0);
    keys(&mut core, view, "u");
    assert_normal_state(&core, view, "one two three four", 0);

    press(&mut core, view, Key::Ctrl('r'));
    assert_normal_state(&core, view, "two three four", 0);
    press(&mut core, view, Key::Ctrl('r'));
    assert_normal_state(&core, view, "four", 0);
}

#[test]
fn unsupported_normal_u_preserves_history_registers_counts_and_dot() {
    let (mut core, view) = new_core("abcde");
    let initial_revision = core.document().revision();
    assert!(matches!(
        press(&mut core, view, Key::Char('U')),
        CommandStatus::Unsupported(_)
    ));
    assert_eq!(core.document().revision(), initial_revision);

    keys(&mut core, view, "A");
    type_text(&mut core, view, "!");
    escape(&mut core, view);
    keys(&mut core, view, "0x");
    let edited_revision = core.document().revision();
    keys(&mut core, view, "\"b3");
    assert!(matches!(
        press(&mut core, view, Key::Char('U')),
        CommandStatus::Unsupported(_)
    ));
    assert_normal_state(&core, view, "bcde!", 0);
    assert_eq!(core.document().revision(), edited_revision);
    assert_register(&core, view, '"', RegisterKind::Characterwise, "a");
    assert_register_absent(&core, view, 'b');

    // The rejected command consumes its own count/register, preserving the
    // previous successful change as dot's recipe and adding no undo unit.
    keys(&mut core, view, ".");
    assert_normal_state(&core, view, "cde!", 0);
    assert_register_absent(&core, view, 'b');
    keys(&mut core, view, "3u");
    assert_eq!(core.document().text(), "abcde");
    keys(&mut core, view, "3");
    press(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().text(), "cde!");
}

#[test]
fn visual_u_and_g_uppercase_remain_undoable() {
    let (mut core, view) = new_core("one two");
    keys(&mut core, view, "vllU");
    assert_eq!(core.document().text(), "ONE two");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "one two");
    keys(&mut core, view, "0gU2w");
    assert_eq!(core.document().text(), "ONE TWO");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "one two");
    press(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().text(), "ONE TWO");
}
