use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{Document, Encoding, FontSlant, Format, TextEdit};
use viem_core::layout::DocumentLayoutStyles;

fn key(c: &mut CommandInterpreter, d: &mut Document, key: Key) -> CommandStatus {
    c.handle(d, InputEvent::Key(key)).unwrap().status
}
fn ex(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandStatus {
    for ch in text.chars() {
        key(c, d, Key::Char(ch));
    }
    key(c, d, Key::Enter)
}
fn begin(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
    let status = ex(c, d, text);
    assert!(c.substitute_confirmation_prompt().is_some(), "{status:?}");
}
#[test]
fn choices_are_staged_and_commit_one_undo_unit_without_changing_registers() {
    let mut d = Document::new("cat cat\ncat cat");
    let mut c = CommandInterpreter::new();
    let register_before = c.register('"').cloned();
    begin(&mut c, &mut d, ":%s/cat/dog/gc");
    assert_eq!(d.text(), "cat cat\ncat cat");
    assert!(!d.undo());
    assert_eq!(
        c.search_presentation(&d)
            .incremental_match
            .unwrap()
            .matched_range,
        0..3
    );
    key(&mut c, &mut d, Key::Char('n'));
    assert_eq!(c.cursor(), 4);
    key(&mut c, &mut d, Key::Char('y'));
    assert_eq!(d.text(), "cat cat\ncat cat");
    assert!(c
        .substitute_confirmation_prompt()
        .unwrap()
        .contains("1 approved"));
    assert_eq!(key(&mut c, &mut d, Key::Char('a')), CommandStatus::Complete);
    assert_eq!(d.text(), "cat dog\ndog dog");
    assert!(c.substitute_confirmation_prompt().is_none());
    assert_eq!(c.register('"'), register_before.as_ref());
    assert!(d.undo());
    assert_eq!(d.text(), "cat cat\ncat cat");
    assert!(!d.undo());
    assert!(d.redo());
    assert_eq!(d.text(), "cat dog\ndog dog");
}
#[test]
fn quit_escape_and_ctrl_c_retain_only_previous_approvals() {
    for quit in [Key::Char('q'), Key::Escape, Key::Ctrl('c')] {
        let mut d = Document::new("x x x");
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, ":s/x/long/gc");
        key(&mut c, &mut d, Key::Char('y'));
        key(&mut c, &mut d, quit);
        assert_eq!(d.text(), "long x x");
        assert!(d.undo());
        assert!(!d.undo());
    }
    let mut d = Document::new("x x");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":s/x/z/gc");
    key(&mut c, &mut d, Key::Escape);
    assert_eq!(d.text(), "x x");
    assert!(!d.undo());
}
#[test]
fn last_accepts_one_match_and_unrecognized_or_mapped_keys_cannot_edit() {
    let mut d = Document::new("x x x");
    let mut c = CommandInterpreter::new();
    ex(&mut c, &mut d, ":map y dd");
    begin(&mut c, &mut d, ":s/x/z/gc");
    key(&mut c, &mut d, Key::Char('i'));
    assert!(c.substitute_confirmation_prompt().is_some());
    key(&mut c, &mut d, Key::Char('y'));
    key(&mut c, &mut d, Key::Char('l'));
    assert_eq!(d.text(), "z z x");
    assert!(d.undo());
    assert!(!d.undo());
}
#[test]
fn stale_approval_never_retargets_a_changed_snapshot() {
    let mut d = Document::new("x x");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":s/x/z/gc");
    key(&mut c, &mut d, Key::Char('y'));
    d.apply_edits(vec![TextEdit::new(0..0, "other ")]).unwrap();
    assert!(c.search_presentation(&d).incremental_match.is_none());
    let status = key(&mut c, &mut d, Key::Char('a'));
    assert!(format!("{status:?}").contains("document changed"));
    assert_eq!(d.text(), "other x x");
    assert!(c.substitute_confirmation_prompt().is_none());
    assert!(d.undo());
    assert!(!d.undo());
}
#[test]
fn ranges_counts_repeat_flags_and_zero_width_matches_keep_substitute_semantics() {
    let mut d = Document::new("x x\nx x\nx x");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":2s/x/z/c 2");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), "x x\nz x\nz x");
    begin(&mut c, &mut d, ":1s/x/z/c");
    key(&mut c, &mut d, Key::Char('q'));
    begin(&mut c, &mut d, ":1&&");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), "z x\nz x\nz x");
    begin(&mut c, &mut d, ":%s/^/>/gc");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), ">z x\n>z x\n>z x");
}
#[test]
fn confirmed_captures_preserve_rich_styles_and_breaks() {
    let source = "<p><b>one</b> <i>two</i></p><!--keep-->";
    let mut d =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, r":s/(one) (two)/\2 \1/c");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), "two one");
    assert_eq!(
        DocumentLayoutStyles::character_at(d.projection(), 0, false)
            .unwrap()
            .slant,
        FontSlant::Italic
    );
    assert!(
        DocumentLayoutStyles::character_at(d.projection(), 4, false)
            .unwrap()
            .bold
    );
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source.as_bytes());
    let mut d = Document::new("a\nb");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, r":%s/a\nb/a\rb/c");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), "a\nb");
}
#[test]
fn recorded_confirmation_quit_is_replayed_as_a_choice() {
    let mut d = Document::new("x x");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, Key::Char('q'));
    key(&mut c, &mut d, Key::Char('a'));
    begin(&mut c, &mut d, ":s/x/z/gc");
    key(&mut c, &mut d, Key::Char('y'));
    key(&mut c, &mut d, Key::Char('q'));
    key(&mut c, &mut d, Key::Char('q'));
    key(&mut c, &mut d, Key::Char('u'));
    key(&mut c, &mut d, Key::Char('@'));
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(d.text(), "z x");
    assert!(c.substitute_confirmation_prompt().is_none());
}
#[test]
fn coordinator_undo_restores_invocation_cursor_instead_of_final_preview() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(Document::new("prefix x\nx\nx"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400.0, 100.0);
    for ch in r":%s/\<x\>/z/gc".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
            .unwrap();
    }
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    for ch in "nyl".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
            .unwrap();
    }
    assert_eq!(core.document().text(), "prefix x\nz\nz");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert_eq!(core.document().text(), "prefix x\nx\nx");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), 11);
}
