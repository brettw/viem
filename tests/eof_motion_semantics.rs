use evim_core::command::{CommandStatus, InputEvent, Key};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

fn editor(text: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 300.0);
    (core, view)
}

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for key in input.chars() {
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .expect("command dispatch succeeds");
        let status = outcome.command.expect("input produces a command").status;
        assert!(
            matches!(status, CommandStatus::Complete | CommandStatus::Pending),
            "{key:?} returned {status:?}"
        );
    }
}

#[test]
fn terminal_word_motion_clamps_cursor_but_delete_uses_exclusive_eof() {
    let text = "a\u{301}bc";
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, "w");
    assert_eq!(normal.command_state(normal_view).unwrap().cursor(), 4);

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "dw");
    assert_eq!(operator.document().text(), "");
}

#[test]
fn terminal_sentence_motion_clamps_to_punctuation_but_operator_consumes_eof() {
    let text = "One sentence!";
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, ")");
    assert_eq!(
        normal.command_state(normal_view).unwrap().cursor(),
        text.find('!').unwrap()
    );

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "d)");
    assert_eq!(operator.document().text(), "");
}

#[test]
fn terminal_paragraph_motion_clamps_to_final_grapheme_but_operator_consumes_eof() {
    let text = "aaa\nbbb\n終\u{301}";
    let final_grapheme = text.find('終').unwrap();
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, "}");
    assert_eq!(
        normal.command_state(normal_view).unwrap().cursor(),
        final_grapheme
    );

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "d}");
    assert_eq!(operator.document().text(), "");
}
