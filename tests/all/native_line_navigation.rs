use viem_core::command::{InputEvent, Key, Mode};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

fn go_to_line(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, line: u64) {
    let outcome = core
        .handle(
            view,
            CoreEvent::GoToLine {
                document: core.document().id(),
                revision: core.document().revision(),
                line,
            },
        )
        .unwrap();
    assert!(!outcome.document_changed);
    assert!(outcome.position_map.is_none());
}

#[test]
fn native_line_navigation_cancels_pending_modes_without_executing_them() {
    use Key::{Char as C, Ctrl};
    let cases = [
        vec![C('3'), C('i'), C('X')],
        vec![C('3'), C('R'), C('X')],
        vec![C('i'), C('X'), Ctrl('v')],
        vec![C('i'), C('X'), Ctrl('g')],
        vec![C('i'), C('X'), Ctrl('o'), C('d')],
        vec![C('d')],
        vec![C('f')],
        vec![C('1'), C('2')],
        vec![C('"'), C('a')],
        vec![C('v'), C('l')],
        vec![C('V')],
        vec![Ctrl('v'), C('j')],
        vec![Ctrl('v'), C('j'), C('I'), C('X')],
        vec![C(':'), C('d'), Ctrl('v')],
    ];
    for keys in cases {
        let mut core = Core::new(Document::new("first\n  second\nthird"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for key in keys.iter().cloned() {
            input(&mut core, view, key);
        }
        let before = core.document().text().to_owned();
        let revision = core.document().revision();
        let history = core.document().history_status().current;
        let target = before.find("second").unwrap();

        go_to_line(&mut core, view, 2);

        let state = core.command_state(view).unwrap();
        assert_eq!(state.mode(), Mode::Normal, "{keys:?}");
        assert_eq!(state.cursor(), target, "{keys:?}");
        assert!(state.visual_anchor().is_none(), "{keys:?}");
        assert!(state.command_line().is_none(), "{keys:?}");
        assert!(state.visual_block_insert_kind().is_none(), "{keys:?}");
        assert_eq!(core.document().text(), before, "{keys:?}");
        assert_eq!(core.document().revision(), revision, "{keys:?}");
        assert_eq!(
            core.document().history_status().current,
            history,
            "{keys:?}"
        );

        // A new command must execute normally, without an old count, literal
        // request, operator, register prefix, or deferred insertion plan.
        input(&mut core, view, C('x'));
        let mut expected = before;
        expected.remove(target);
        assert_eq!(core.document().text(), expected, "{keys:?}");
    }
}

#[test]
fn native_line_navigation_preserves_committed_insert_and_its_undo_unit() {
    let original = "first\nsecond\nthird";
    let mut core = Core::new(Document::new(original));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    for key in ['3', 'i', 'X', 'Y'] {
        input(&mut core, view, Key::Char(key));
    }
    go_to_line(&mut core, view, 2);
    assert_eq!(core.document().text(), "XYfirst\nsecond\nthird");
    input(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), original);
    input(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().text(), "XYfirst\nsecond\nthird");
}

#[test]
fn native_line_navigation_clamps_and_uses_hard_lines_and_graphemes() {
    let text = "  e\u{301} first line wraps across many rows\n\t👩‍💻 last";
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 45.0, 48.0);
    for line in [2, 100, u64::MAX] {
        go_to_line(&mut core, view, line);
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            text.find('👩').unwrap()
        );
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot
            .coverage
            .contains_text_offset(text.find('👩').unwrap()));
    }
    for line in [0, 1] {
        go_to_line(&mut core, view, line);
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);
    }
    for text in ["", "first\n"] {
        let mut core = Core::new(Document::new(text));
        let view = core.add_view(MockTextMeasurementProvider::new(), 100.0, 48.0);
        go_to_line(&mut core, view, u64::MAX);
        assert_eq!(core.command_state(view).unwrap().cursor(), text.len());
    }
}

#[test]
fn native_line_navigation_is_view_local_and_is_not_macro_input() {
    let mut core = Core::new(Document::new("first\nsecond\nthird"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 200.0, 80.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    for key in ['q', 'a', 'l'] {
        input(&mut core, first, Key::Char(key));
    }
    go_to_line(&mut core, first, 2);
    assert!(core.command_state(first).unwrap().is_recording_macro());
    assert_eq!(core.command_state(second).unwrap().cursor(), 0);
    input(&mut core, first, Key::Char('q'));
    assert_eq!(
        core.command_state(first)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        "l"
    );
}
