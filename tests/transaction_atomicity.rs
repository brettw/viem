//! Black-box regressions for the coordinator/transaction boundary.
//!
//! These tests pin the all-or-nothing behavior required at the current
//! coordinator boundary while the controller is incrementally moving toward
//! fully prepared, revision-bound command plans.

use viem_core::command::{CommandInterpreter, InputEvent, Key, Mode};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, DocumentError, Encoding, Format, Revision};

fn command_key(character: char) -> InputEvent {
    InputEvent::Key(Key::Char(character))
}

fn core_key(character: char) -> CoreEvent {
    CoreEvent::Input(command_key(character))
}

fn core_text(text: &str) -> CoreEvent {
    CoreEvent::Input(InputEvent::Text(text.to_owned()))
}

#[test]
fn failed_latin1_markdown_code_replacement_rolls_back_register_document_cursor_and_history() {
    let source = b"`a` *b*".to_vec();
    let mut document =
        Document::from_bytes(source.clone(), Encoding::Latin1, Format::Markdown).unwrap();
    let mut commands = CommandInterpreter::new();
    let before_register = commands.register('"').cloned();

    commands.handle(&mut document, command_key('r')).unwrap();
    let error = commands
        .handle(&mut document, command_key('😀'))
        .expect_err("Latin-1 code cannot use a prose character reference");

    assert_eq!(
        error,
        DocumentError::UnrepresentableCharacter {
            encoding: Encoding::Latin1,
            character: '😀',
        }
    );
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.revision(), Revision(0));
    assert_eq!(commands.cursor(), 0);
    assert_eq!(commands.register('"'), before_register.as_ref());
    assert!(
        !document.undo(),
        "a rejected edit must not create or alter an undo node"
    );
}

#[test]
fn latin1_markdown_change_across_a_style_boundary_is_one_undo_unit() {
    let source = b"*a* *b*".to_vec();
    let mut document =
        Document::from_bytes(source.clone(), Encoding::Latin1, Format::Markdown).unwrap();
    let mut commands = CommandInterpreter::new();

    // `cw` is the Vim-special `ce` range and is locally representable here.
    // `c2l` also consumes the following formatted space across the closing
    // emphasis delimiter. Relational reverse projection keeps that delimiter
    // untouched while the change and following Insert session remain grouped.
    commands.handle(&mut document, command_key('c')).unwrap();
    commands.handle(&mut document, command_key('2')).unwrap();
    commands.handle(&mut document, command_key('l')).unwrap();
    assert_eq!(commands.mode(), Mode::Insert);
    commands
        .handle(&mut document, InputEvent::Text("z".to_owned()))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::Key(Key::Escape))
        .unwrap();
    assert_eq!(document.text(), "zb");
    assert_eq!(commands.cursor(), 0);
    assert_eq!(commands.mode(), Mode::Normal);

    assert!(document.undo(), "the complete change must be one undo unit");
    assert_eq!(document.source_bytes(), source);
    assert!(!document.undo());
}

#[test]
fn failed_replace_text_event_is_atomic_across_all_graphemes() {
    let original = b"ab".to_vec();
    let mut document =
        Document::from_bytes(original.clone(), Encoding::Latin1, Format::PlainText).unwrap();
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut document, command_key('R')).unwrap();
    assert_eq!(commands.mode(), Mode::Replace);

    let error = commands
        .handle(&mut document, InputEvent::Text("x😀".to_owned()))
        .expect_err("the emoji is not representable in Latin-1");

    assert!(matches!(
        error,
        DocumentError::UnrepresentableCharacter {
            encoding: Encoding::Latin1,
            character: '😀'
        }
    ));
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.text(), "ab");
    assert_eq!(document.revision(), Revision(0));
    assert_eq!(commands.cursor(), 0);
    assert_eq!(commands.mode(), Mode::Replace);
    assert!(
        !document.undo(),
        "no prefix of a rejected text event may enter history"
    );
}

#[test]
fn insert_undo_groups_are_isolated_between_views() {
    let mut core = Core::new(Document::new("ab"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

    // Leave the first view's Insert session open, then make an unrelated Insert
    // change in the second view. The coordinator must finalize the first undo
    // unit before accepting the second view's change.
    core.handle(first, core_key('i')).unwrap();
    core.handle(first, core_text("X")).unwrap();
    core.handle(second, core_key('A')).unwrap();
    core.handle(second, core_text("Y")).unwrap();
    core.handle(second, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(first, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(core.document().text(), "XabY");

    core.handle(second, core_key('u')).unwrap();
    assert_eq!(
        core.document().text(),
        "Xab",
        "one undo must remove only the second view's Insert unit"
    );
    core.handle(second, core_key('u')).unwrap();
    assert_eq!(core.document().text(), "ab");
}

#[test]
fn inactive_view_cursor_rebases_before_its_next_command() {
    let mut core = Core::new(Document::new("abc"));
    let writer = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    let reader = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);

    core.handle(reader, core_key('l')).unwrap();
    assert_eq!(core.command_state(reader).unwrap().cursor(), 1);

    core.handle(writer, core_key('i')).unwrap();
    core.handle(writer, core_text("X")).unwrap();
    core.handle(writer, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();

    assert_eq!(
        core.command_state(reader).unwrap().cursor(),
        2,
        "the inactive cursor must remain attached to the original `b`"
    );

    core.handle(reader, core_key('x')).unwrap();
    assert_eq!(
        core.document().text(),
        "Xac",
        "the reader must delete its anchored `b`, not the shifted `a`"
    );
}
