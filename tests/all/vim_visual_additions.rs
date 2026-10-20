//! Visual commands and text objects added after the command audit: Visual
//! `Y`, Visual number changes and tag blocks. Each case enters through the
//! core coordinator with mock layout.

use viem_core::command::{CommandOutput, CommandStatus, InputEvent, Key, Mode, RegisterKind};
use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn editor(text: &str, format: Format) -> (TestCore, ViewId) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    (core, view)
}

fn text_editor(text: &str) -> (TestCore, ViewId) {
    editor(text, Format::PlainText)
}

/// Parse `<Esc>`, `<CR>`, `<BS>`, `<C-x>` and `<Text:...>` tokens; every
/// other character is one key.
fn events(mut notation: &str) -> Vec<InputEvent> {
    let mut events = Vec::new();
    while !notation.is_empty() {
        if let Some(rest) = notation.strip_prefix("<Text:") {
            let end = rest.find('>').expect("terminated text token");
            events.push(InputEvent::Text(rest[..end].to_owned()));
            notation = &rest[end + 1..];
            continue;
        }
        let special = [("<Esc>", Key::Escape), ("<CR>", Key::Enter), ("<BS>", Key::Backspace)]
            .into_iter()
            .find(|(spelling, _)| notation.starts_with(spelling));
        if let Some((spelling, key)) = special {
            events.push(InputEvent::Key(key));
            notation = &notation[spelling.len()..];
            continue;
        }
        if let Some(rest) = notation.strip_prefix("<C-") {
            let character = rest.chars().next().expect("control key");
            assert_eq!(rest[character.len_utf8()..].chars().next(), Some('>'));
            events.push(InputEvent::Key(Key::Ctrl(character)));
            notation = &rest[character.len_utf8() + 1..];
            continue;
        }
        let character = notation.chars().next().unwrap();
        events.push(InputEvent::Key(Key::Char(character)));
        notation = &notation[character.len_utf8()..];
    }
    events
}

/// Send every event and return the final command output, failing on any
/// unsupported or failed step.
fn run(core: &mut TestCore, view: ViewId, notation: &str) -> CommandOutput {
    let mut last = None;
    for event in events(notation) {
        let outcome = core.handle_with_layout(view, CoreEvent::Input(event.clone())).unwrap();
        let output = outcome.command.expect("input produces command output");
        assert!(
            matches!(output.status, CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled),
            "{notation:?} at {event:?}: {:?}",
            output.status
        );
        last = Some(output);
    }
    last.expect("notation has events")
}

/// Send every event and return the final status without asserting success.
fn attempt(core: &mut TestCore, view: ViewId, notation: &str) -> CommandStatus {
    let mut last = None;
    for event in events(notation) {
        let outcome = core.handle_with_layout(view, CoreEvent::Input(event)).unwrap();
        last = Some(outcome.command.expect("input produces command output").status);
    }
    last.expect("notation has events")
}

fn cursor(core: &TestCore, view: ViewId) -> usize {
    core.command_state(view).unwrap().cursor()
}

fn mode(core: &TestCore, view: ViewId) -> Mode {
    core.command_state(view).unwrap().mode()
}



fn register(core: &TestCore, view: ViewId, name: char) -> (String, RegisterKind) {
    let value = core.command_state(view).unwrap().register(name).expect("register is set");
    (value.text.clone(), value.kind)
}

#[test]
fn visual_capital_y_yanks_lines_and_visual_block_y_yanks_the_block() {
    let (mut core, view) = text_editor("one\ntwo\nthree");
    run(&mut core, view, "lvjY");
    assert_eq!(register(&core, view, '"'), ("one\ntwo\n".to_owned(), RegisterKind::Linewise));
    assert_eq!(mode(&core, view), Mode::Normal);
    assert_eq!(core.document().text(), "one\ntwo\nthree", "Y never edits");
    run(&mut core, view, "ggjV\"aY");
    assert_eq!(register(&core, view, 'a'), ("two\n".to_owned(), RegisterKind::Linewise));
    run(&mut core, view, "gg<C-v>jlY");
    assert_eq!(register(&core, view, '"'), ("on\ntw".to_owned(), RegisterKind::Blockwise));
    assert_eq!(mode(&core, view), Mode::Normal);
    assert_eq!(core.document().text(), "one\ntwo\nthree");
}

#[test]
fn visual_ctrl_a_and_ctrl_x_change_each_selected_line() {
    let (mut core, view) = text_editor("1\n1\n1");
    run(&mut core, view, "Vj<C-a>");
    assert_eq!(core.document().text(), "2\n2\n1");
    assert_eq!((mode(&core, view), cursor(&core, view)), (Mode::Normal, 0));
    run(&mut core, view, "u");
    assert_eq!(core.document().text(), "1\n1\n1", "one Visual change is one undo unit");
    run(&mut core, view, "ggVjg<C-a>");
    assert_eq!(core.document().text(), "2\n3\n1", "g CTRL-A is progressive");
    run(&mut core, view, "u");
    run(&mut core, view, "ggVG3g<C-x>");
    assert_eq!(core.document().text(), "-2\n-5\n-8");

    let (mut core, view) = text_editor("1\n1\n1\n1");
    run(&mut core, view, "Vj<C-a>jj.");
    assert_eq!(core.document().text(), "2\n2\n2\n2", "dot repeats over the same number of lines");

    let (mut core, view) = text_editor("a1 b2\nc3 d4");
    run(&mut core, view, "fbvj<C-a>");
    assert_eq!(core.document().text(), "a1 b3\nc4 d4", "characterwise regions start at the selection");
    let (mut core, view) = text_editor("x-5 123");
    run(&mut core, view, "f5v<C-a>");
    assert_eq!(core.document().text(), "x-6 123", "an unselected minus sign is not part of the number");
    run(&mut core, view, "f1v<C-a>");
    assert_eq!(core.document().text(), "x-6 223", "digits after the selection are not read");
    let (mut core, view) = text_editor("none\n7");
    run(&mut core, view, "Vj<C-x>");
    assert_eq!(core.document().text(), "none\n6", "lines without a number are skipped");
}

#[test]
fn visual_block_number_changes_use_the_block_columns() {
    let (mut core, view) = text_editor("1 1\n1 1\n1 1");
    run(&mut core, view, "l<C-v>jjl<C-a>");
    assert_eq!(core.document().text(), "1 2\n1 2\n1 2");
    assert_eq!(mode(&core, view), Mode::Normal);
    run(&mut core, view, "u");
    run(&mut core, view, "gg0l<C-v>jjlg<C-a>");
    assert_eq!(core.document().text(), "1 2\n1 3\n1 4");
    run(&mut core, view, "u");
    run(&mut core, view, "gg0l<C-v>jl2<C-x>");
    assert_eq!(core.document().text(), "1 -1\n1 -1\n1 1");
    run(&mut core, view, "j.");
    assert_eq!(core.document().text(), "1 -1\n1 -1\n1 -1",
        "dot repeats the two-column block, which no longer reaches the second row's digit");
}

#[test]
fn tag_block_text_objects_select_inner_and_whole_blocks() {
    let text = "<div>\n  <p>Hello <b>bold</b> text</p>\n</div>";
    let (mut core, view) = editor(text, Format::Code);
    run(&mut core, view, "/bold<CR>yit");
    assert_eq!(register(&core, view, '"').0, "bold");
    run(&mut core, view, "yat");
    assert_eq!(register(&core, view, '"').0, "<b>bold</b>");
    run(&mut core, view, "y2it");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text");
    run(&mut core, view, "/Hello<CR>dit");
    assert_eq!(core.document().text(), "<div>\n  <p></p>\n</div>");
    run(&mut core, view, "u/Hello<CR>dat");
    assert_eq!(core.document().text(), "<div>\n  \n</div>");
    run(&mut core, view, "u/Hello<CR>d2at");
    assert_eq!(core.document().text(), "");
    run(&mut core, view, "u");

    run(&mut core, view, "gg/<p<CR>lyit");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text", "a start tag selects its block");
    run(&mut core, view, "/<.p<CR>lyit");
    assert_eq!(register(&core, view, '"').0, "Hello <b>bold</b> text", "an end tag selects its block");
    run(&mut core, view, "gg0jyat");
    assert_eq!(register(&core, view, '"').0, "<p>Hello <b>bold</b> text</p>", "indentation selects the next block");

    run(&mut core, view, "/bold<CR>vit");
    assert_eq!(mode(&core, view), Mode::VisualCharacter);
    run(&mut core, view, "ity");
    assert_eq!(register(&core, view, '"').0, "<b>bold</b>", "a repeated Visual it includes the tags");
    run(&mut core, view, "/bold<CR>vatat");
    run(&mut core, view, "y");
    assert_eq!(register(&core, view, '"').0, "<p>Hello <b>bold</b> text</p>", "a repeated Visual at selects the parent");

    for (source, expected) in [
        ("<p>a<br>b</p>", "a<br>b"),
        ("<p>a<br/>b</p>", "a<br/>b"),
        ("<P class=\"x\">a b</p>", "a b"),
    ] {
        let (mut core, view) = editor(source, Format::Code);
        run(&mut core, view, "$hyit");
        assert_eq!(register(&core, view, '"').0, expected, "{source}");
    }
    let (mut core, view) = text_editor("no tags");
    let revision = core.document().revision();
    assert!(!matches!(attempt(&mut core, view, "dit"), CommandStatus::Complete));
    assert_eq!(core.document().revision(), revision);
}
