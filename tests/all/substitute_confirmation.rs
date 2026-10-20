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
    let source = "**one** *two*";
    let mut d =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
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


#[test]
fn confirmation_scrolls_without_changing_the_match_or_approvals() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let original = "x\n".repeat(60);
    let mut core = Core::new(Document::new(&original));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    for ch in ":%s/x/z/gc".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch)))).unwrap();
    }
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    let prompt = core.command_state(view).unwrap().substitute_confirmation_prompt();
    let cursor = core.command_state(view).unwrap().cursor();
    let before = core.layout(view).unwrap().viewport_top();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('e')))).unwrap();
    assert!(core.layout(view).unwrap().viewport_top() > before);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    assert_eq!(core.command_state(view).unwrap().substitute_confirmation_prompt(), prompt);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('y')))).unwrap();
    assert_eq!(core.layout(view).unwrap().viewport_top(), before);
    assert_eq!(core.document().text(), original);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('l')))).unwrap();
    assert!(core.document().text().starts_with("z\nx\n"));
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
    assert_eq!(core.document().text(), original);
}

#[test]
fn global_choices_are_per_invocation_and_publish_together() {
    for (choices, expected) in [
        ("yal", "long long\nkeep\nlong x"),
        ("qa", "x x\nkeep\nlong long"),
        ("la", "long x\nkeep\nlong long"),
        ("nnya", "x x\nkeep\nlong long"),
    ] {
        let original = "x x\nkeep\nx x";
        let mut d = Document::new(original);
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, ":g/x/s/x/long/gc");
        for (index, choice) in choices.chars().enumerate() {
            assert_eq!(d.text(), original, "{choices} before choice {index}");
            key(&mut c, &mut d, Key::Char(choice));
        }
        assert!(c.substitute_confirmation_prompt().is_none(), "{choices}");
        assert_eq!(d.text(), expected, "{choices}");
        assert!(d.undo());
        assert_eq!(d.text(), original);
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.text(), expected);
    }
}

#[test]
fn global_confirmation_handles_nested_inverse_errors_and_interrupts() {
    for command in [":g/x/v/keep/s/x/z/gc", ":v/keep/s/x/z/gc", ":g!/keep/s/x/z/gc"] {
        let mut d = Document::new("xkeep\nx x\nx x");
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, command);
        key(&mut c, &mut d, Key::Char('a'));
        assert_eq!(d.text(), "xkeep\nx x\nx x");
        key(&mut c, &mut d, Key::Ctrl('c'));
        assert!(c.substitute_confirmation_prompt().is_none());
        assert_eq!(d.text(), "xkeep\nz z\nx x");
        assert!(d.undo());
        assert!(!d.undo());
    }
    let mut d = Document::new("xkeep\nxdelete\nxagain");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/x/s/delete/gone/c");
    let status = key(&mut c, &mut d, Key::Char('y'));
    assert!(format!("{status:?}").contains("PatternNotFound"));
    assert_eq!(d.text(), "xkeep\nxgone\nxagain");
    assert!(d.undo());
    assert!(!d.undo());
}

#[test]
fn global_confirmation_preserves_encoded_source_captures_and_line_endings() {
    for (source, encoding, format, command, expected) in [
        (b"**one** *two*\r\n\r\n**one** *two*\r\n".to_vec(), Encoding::Utf8, Format::Markdown,
            r":g/one/s/(one) (two)/\2 \1/c", "two one\ntwo one"),
        (b"x \xe9\r\nkeep\nx \xe9".to_vec(), Encoding::Latin1, Format::PlainText,
            ":g/x/s/x/z/c", "z é\r\nkeep\nz é"),
    ] {
        let mut d = Document::from_bytes(source.clone(), encoding, format).unwrap();
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, command);
        key(&mut c, &mut d, Key::Char('a'));
        assert_eq!(d.source_bytes(), source);
        assert_eq!(key(&mut c, &mut d, Key::Char('a')), CommandStatus::Complete);
        assert_eq!(d.text(), expected);
        let saved = d.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), encoding, format).unwrap();
        assert_eq!(reopened.text(), expected);
        assert!(d.undo());
        assert_eq!(d.source_bytes(), source);
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.source_bytes(), saved);
    }
}

#[test]
fn global_confirmation_cancels_all_staged_choices_after_an_external_edit() {
    let mut d = Document::new("x\nx\nx");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/x/s/x/z/c");
    key(&mut c, &mut d, Key::Char('y'));
    d.apply_edits(vec![TextEdit::new(0..0, "other ")]).unwrap();
    let status = key(&mut c, &mut d, Key::Char('a'));
    assert!(format!("{status:?}").contains("document changed"));
    assert_eq!(d.text(), "other x\nx\nx");
    assert!(d.undo());
    assert!(!d.undo());
}

#[test]
fn global_confirmation_keeps_selected_source_lines_within_folded_markdown() {
    let source = "x first\nx second\n\nkeep";
    let mut d = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut c = CommandInterpreter::new();
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource).unwrap();
    begin(&mut c, &mut d, ":g/x/s/x/z/gc");
    key(&mut c, &mut d, Key::Char('a'));
    assert!(c.substitute_confirmation_prompt().is_some());
    assert_eq!(key(&mut c, &mut d, Key::Char('a')), CommandStatus::Complete);
    assert_eq!(d.source_bytes(), b"z first\nz second\n\nkeep");
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source.as_bytes());
    assert!(!d.undo());
}

#[test]
fn global_confirmation_honors_disjoint_counts_and_rejects_overlapping_questions() {
    let mut d = Document::new("x start\nx\nkeep\nx start\nx");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/start/s/x/z/gc 2");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(key(&mut c, &mut d, Key::Char('a')), CommandStatus::Complete);
    assert_eq!(d.text(), "z start\nz\nkeep\nz start\nz");
    assert!(d.undo());
    let original = d.source_bytes();
    let status = ex(&mut c, &mut d, ":g/x/%s/x/xx/gc");
    assert!(format!("{status:?}").contains("disjoint match ranges"));
    assert!(c.substitute_confirmation_prompt().is_none());
    assert_eq!(d.source_bytes(), original);
    assert!(!d.undo());
}

#[test]
fn global_confirmation_retains_valid_targets_when_another_cannot_encode_the_replacement() {
    let source = b"x\n\n```\nx\n```";
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Latin1, Format::Markdown).unwrap();
    let mut c = CommandInterpreter::new();
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource).unwrap();
    begin(&mut c, &mut d, ":g/x/s/x/好/c");
    key(&mut c, &mut d, Key::Char('a'));
    let status = key(&mut c, &mut d, Key::Char('a'));
    assert_ne!(status, CommandStatus::Complete);
    assert_eq!(d.text(), "好\nx");
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source);
    assert!(!d.undo());
}

#[test]
fn global_confirmation_coordinator_restores_views_cursor_and_jump_origin() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(Document::new("origin\nx\nx\ntail"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    let other = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    core.handle(other, CoreEvent::Input(InputEvent::Key(Key::Char('G')))).unwrap();
    for ch in ":g/x/s/x/long/c".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch)))).unwrap();
    }
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    for ch in "aa".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch)))).unwrap();
    }
    assert_eq!(core.document().text(), "origin\nlong\nlong\ntail");
    assert_eq!(core.command_state(other).unwrap().cursor(), 17);
    assert_eq!(core.command_state(view).unwrap().cursor(), 12);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o')))).unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
    assert_eq!(core.document().text(), "origin\nx\nx\ntail");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_eq!(core.command_state(other).unwrap().cursor(), 11);
}
