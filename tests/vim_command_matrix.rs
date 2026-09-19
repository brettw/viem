//! Black-box regression coverage for repeat, undo grouping, and Visual Block joins.
//!
//! Commands enter through `Core` with mock-shaped layout. The retained fixtures
//! assert document text, register effects, and undo/repeat behavior.

use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

const PROSE: &str =
    "alpha beta (gamma). Next sentence!\n\nparagraph two delta epsilon\nthird target alpha beta\nfourth line";

fn parse_events(mut notation: &str) -> Vec<InputEvent> {
    const SPECIALS: &[(&str, Key)] = &[
        ("<Esc>", Key::Escape),
        ("<C-V>", Key::Ctrl('v')),
        ("<C-R>", Key::Ctrl('r')),
    ];

    let mut events = Vec::new();
    while !notation.is_empty() {
        if let Some(payload) = notation.strip_prefix("<Text:") {
            let end = payload
                .find('>')
                .unwrap_or_else(|| panic!("unterminated text token in {notation:?}"));
            events.push(InputEvent::Text(payload[..end].to_owned()));
            notation = &payload[end + 1..];
            continue;
        }
        if let Some((spelling, key)) = SPECIALS
            .iter()
            .find(|(spelling, _)| notation.starts_with(*spelling))
        {
            events.push(InputEvent::Key(*key));
            notation = &notation[spelling.len()..];
            continue;
        }
        let character = notation.chars().next().expect("notation is nonempty");
        events.push(InputEvent::Key(Key::Char(character)));
        notation = &notation[character.len_utf8()..];
    }
    events
}

fn new_core(text: &str, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 2_000.0);
    (core, view)
}

fn run_notation(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    notation: &str,
) -> Result<CommandStatus, String> {
    let mut final_status = None;
    for event in parse_events(notation) {
        let outcome = core
            .handle(view, CoreEvent::Input(event.clone()))
            .map_err(|error| format!("core error for {event:?}: {error:?}"))?;
        let status = outcome
            .command
            .ok_or_else(|| format!("input {event:?} returned no command output"))?
            .status;
        if let CommandStatus::Unsupported(reason) = &status {
            return Err(format!("{event:?} returned Unsupported({reason:?})"));
        }
        if matches!(
            status,
            CommandStatus::Error(_)
                | CommandStatus::CountError(_)
                | CommandStatus::RegisterReadError(_)
                | CommandStatus::RegisterWriteError(_)
                | CommandStatus::ExError(_)
                | CommandStatus::VisualBlockError(_)
                | CommandStatus::NeedsMoreLayout(_)
                | CommandStatus::SearchNotFound
        ) {
            return Err(format!("{event:?} returned {status:?}"));
        }
        final_status = Some(status);
    }
    final_status.ok_or_else(|| "case contains no input events".to_owned())
}

#[test]
fn repeat_macro_and_undo_grouping_conformance() {
    let (mut core, view) = new_core("one two three four", 500.0);
    run_notation(&mut core, view, "dw.").expect("delete and dot repeat are supported");
    assert_eq!(core.document().text(), "three four");
    run_notation(&mut core, view, "u").expect("dot repeat is independently undoable");
    assert_eq!(
        core.document().text(),
        "two three four",
        "one dot repeat must be one undo unit"
    );
    run_notation(&mut core, view, "<C-R>").expect("redo is supported");
    assert_eq!(core.document().text(), "three four");

    let (mut core, view) = new_core("one two three", 500.0);
    run_notation(&mut core, view, "cw<Text:\u{00e9}><Esc>")
        .expect("change plus Unicode Insert payload is supported");
    assert_ne!(core.document().text(), "one two three");
    run_notation(&mut core, view, "u").expect("change undo is supported");
    assert_eq!(
        core.document().text(),
        "one two three",
        "operator deletion and following Insert session must share one undo unit"
    );

    let (mut core, view) = new_core("one two three four", 500.0);
    run_notation(&mut core, view, "qadwq@a").expect("macro record and replay are supported");
    assert_eq!(core.document().text(), "three four");
    run_notation(&mut core, view, "u").expect("macro replay is independently undoable");
    assert_eq!(
        core.document().text(),
        "two three four",
        "one macro replay must be one undo unit"
    );
    run_notation(&mut core, view, "@@").expect("repeat-last-macro is supported");

    let (mut core, view) = new_core("one two\nthree", 500.0);
    run_notation(&mut core, view, "\"a2dd").expect("named-register counted delete is supported");
    let register = core
        .command_state(view)
        .and_then(|state| state.register('a'))
        .expect("successful named-register delete populates that register");
    assert_eq!(register.text, "one two\nthree\n");

    let (mut core, view) = new_core(PROSE, 500.0);
    let before = core.document().source_bytes();
    run_notation(&mut core, view, "\"ad<Esc>").expect("pending command cancellation is supported");
    assert_eq!(
        core.document().source_bytes(),
        before,
        "cancelling a prefixed operator must not mutate the source"
    );
    assert!(
        core.command_state(view).unwrap().register('a').is_none(),
        "cancelling before the operator commits must not update its register"
    );
}

#[test]
fn visual_block_join_is_line_mode_independent_and_dot_replays_its_block_extent() {
    use viem_core::command::LineMode;
    let original = "abcdefghij\nx\nthird";
    for mode in [LineMode::Visual, LineMode::PhysicalSource] {
        for (join, expected) in [("J", "abcdefghij x third"), ("gJ", "abcdefghijxthird")] {
            let (mut core, view) = new_core(original, 24.0);
            core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
            run_notation(&mut core, view, &format!("<C-V>G{join}")).unwrap();
            assert_eq!(core.document().text(), expected, "{mode:?} {join}");
            run_notation(&mut core, view, "ugg0.").unwrap();
            assert_eq!(core.document().text(), expected, "dot: {mode:?} {join}");
            run_notation(&mut core, view, "u").unwrap();
            assert_eq!(core.document().text(), original);
        }
    }
}
