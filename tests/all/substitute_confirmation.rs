use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactWriteIntent,
    Document, Encoding, FontSlant, Format, InMemoryArtifactStorage, TextEdit,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

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
fn global_invocations_publish_before_the_next_target_in_one_undo_unit() {
    // Each invocation stages its own answers, then publishes them when it ends.
    for (choices, first_answers, after_first, expected) in [
        ("yal", 2, "long long\nkeep\nx x", "long long\nkeep\nlong x"),
        ("qa", 1, "x x\nkeep\nx x", "x x\nkeep\nlong long"),
        ("la", 1, "long x\nkeep\nx x", "long x\nkeep\nlong long"),
        ("nnya", 2, "x x\nkeep\nx x", "x x\nkeep\nlong long"),
    ] {
        let original = "x x\nkeep\nx x";
        let mut d = Document::new(original);
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, ":g/x/s/x/long/gc");
        for (index, choice) in choices.chars().enumerate() {
            let staged = if index < first_answers { original } else { after_first };
            assert_eq!(d.text(), staged, "{choices} before choice {index}");
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
fn global_invocations_see_earlier_replacements_like_vim() {
    // Vim 9.1 oracle for lines `x`, `x` and `:g/x/%s/x/xx/gc`. Each invocation
    // scans the text published by earlier ones, without recursively matching
    // its own replacements; a/q/l/Escape end one invocation, Ctrl-C all.
    let char = Key::Char;
    for (responses, expected) in [
        (vec![char('a'), char('a')], "xxxx\nxxxx"),
        (vec![char('q'), char('a')], "xx\nxx"),
        (vec![Key::Escape, char('a')], "xx\nxx"),
        (vec![char('l'), char('a')], "xxxx\nxx"),
        (vec![char('y'), char('q'), char('a')], "xxxx\nxx"),
        (vec![char('y'), Key::Ctrl('c')], "xx\nx"),
    ] {
        let mut d = Document::new("x\nx");
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, ":g/x/%s/x/xx/gc");
        let mut status = CommandStatus::Pending;
        for response in &responses {
            assert!(c.substitute_confirmation_prompt().is_some(), "{responses:?}");
            status = key(&mut c, &mut d, *response);
        }
        assert_eq!(status, CommandStatus::Complete, "{responses:?}");
        assert!(c.substitute_confirmation_prompt().is_none(), "{responses:?}");
        assert_eq!(d.text(), expected, "{responses:?}");
        assert!(d.undo());
        assert_eq!(d.text(), "x\nx");
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.text(), expected);
    }
}

#[test]
fn global_confirmation_skips_targets_deleted_by_earlier_invocations() {
    // Joining the first line removes the second target; it must not collapse
    // onto the joined line and ask about that line's break again.
    let original = "x1\nx2\nx3\nend";
    let mut d = Document::new(original);
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/x/.,+1s/\\n/ /c");
    key(&mut c, &mut d, Key::Char('y'));
    assert_eq!(d.text(), "x1 x2\nx3\nend");
    // The next question is the break after `x3`, not the joined line's break.
    assert!(c.substitute_confirmation_prompt().is_some());
    assert_eq!(c.cursor(), 7);
    assert_eq!(key(&mut c, &mut d, Key::Char('n')), CommandStatus::Complete);
    assert!(c.substitute_confirmation_prompt().is_none());
    assert_eq!(d.text(), "x1 x2\nx3\nend");
    assert!(d.undo());
    assert_eq!(d.text(), original);
    assert!(!d.undo());
}

#[test]
fn global_confirmation_bounds_geometric_growth_across_invocations() {
    let original = "x\n".repeat(200);
    let mut d = Document::new(original.as_str());
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/x/%s/x/xx/gc");
    let mut status = CommandStatus::Pending;
    let mut answers = 0;
    while status == CommandStatus::Pending {
        assert!(answers < 20, "the cumulative match budget never stopped the command");
        status = key(&mut c, &mut d, Key::Char('a'));
        answers += 1;
    }
    // Matches double per invocation, so the match or work budget ends it.
    let reason = format!("{status:?}");
    assert!(reason.contains("global confirmation matches") || reason.contains("global confirmation work"), "{reason}");
    assert!(c.substitute_confirmation_prompt().is_none());
    // Completed invocations stay published and undo together.
    let expected = format!("{}\n", "x".repeat(1 << answers)).repeat(200);
    assert_eq!(d.text(), expected);
    assert!(d.undo());
    assert_eq!(d.text(), original);
    assert!(!d.undo());
}

#[test]
fn global_confirmation_handles_nested_inverse_errors_and_interrupts() {
    for command in [":g/x/v/keep/s/x/z/gc", ":v/keep/s/x/z/gc", ":g!/keep/s/x/z/gc"] {
        let mut d = Document::new("xkeep\nx x\nx x");
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, command);
        key(&mut c, &mut d, Key::Char('a'));
        assert_eq!(d.text(), "xkeep\nz z\nx x");
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
    for (source, encoding, format, command, first, expected) in [
        (b"**one** *two*\r\n\r\n**one** *two*\r\n".to_vec(), Encoding::Utf8, Format::Markdown,
            r":g/one/s/(one) (two)/\2 \1/c", "two one\none two", "two one\ntwo one"),
        (b"x \xe9\r\nkeep\nx \xe9".to_vec(), Encoding::Latin1, Format::PlainText,
            ":g/x/s/x/z/c", "z é\r\nkeep\nx é", "z é\r\nkeep\nz é"),
    ] {
        let mut d = Document::from_bytes(source.clone(), encoding, format).unwrap();
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, command);
        key(&mut c, &mut d, Key::Char('a'));
        assert_eq!(d.text(), first);
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

fn core_key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) -> CommandStatus {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap().command.unwrap().status
}

fn core_begin(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, command: &str) {
    for ch in command.chars() {
        core_key(core, view, Key::Char(ch));
    }
    core_key(core, view, Key::Enter);
    assert!(core.command_state(view).unwrap().substitute_confirmation_prompt().is_some());
}

#[test]
fn global_confirmation_external_edit_cancels_remaining_targets_but_keeps_completed_invocations() {
    let mut core = Core::new(Document::new("x\nx\nx"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    let other = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    core_begin(&mut core, view, ":g/x/s/x/z/c");
    core_key(&mut core, view, Key::Char('y'));
    assert_eq!(core.document().text(), "z\nx\nx");
    // Another writer closes the confirming view's undo group first, so its
    // edit never joins the :global unit.
    core_key(&mut core, other, Key::Char('A'));
    core.handle(other, CoreEvent::Input(InputEvent::Text(" other".into()))).unwrap();
    core_key(&mut core, other, Key::Escape);
    assert_eq!(core.document().text(), "z other\nx\nx");
    let status = core_key(&mut core, view, Key::Char('a'));
    assert!(format!("{status:?}").contains("document changed"), "{status:?}");
    assert!(core.command_state(view).unwrap().substitute_confirmation_prompt().is_none());
    assert_eq!(core.document().text(), "z other\nx\nx");
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "z\nx\nx");
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "x\nx\nx");
}

#[test]
fn global_confirmation_save_splits_the_undo_unit_without_changing_the_saved_state() {
    let mut core = Core::new(Document::new("x\nx\nx"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    core_begin(&mut core, view, ":g/x/s/x/z/c");
    core_key(&mut core, view, Key::Char('y'));
    let prepared = core.prepare_artifact_write(ArtifactWriteIntent::SaveAs {
        destination: ArtifactPath::from("confirmation.txt"),
        overwrite: ArtifactOverwrite::ReplaceExisting,
    }).unwrap();
    let mut storage = InMemoryArtifactStorage::new();
    let receipt = execute_prepared_artifact_write(&mut storage, &prepared).unwrap();
    core.complete_artifact_write(prepared.succeeded(receipt)).unwrap();
    assert!(!core.document().is_dirty());
    assert!(core.command_state(view).unwrap().substitute_confirmation_prompt().is_some());
    core_key(&mut core, view, Key::Char('y'));
    assert_eq!(core_key(&mut core, view, Key::Char('y')), CommandStatus::Complete);
    assert_eq!(core.document().text(), "z\nz\nz");
    assert!(core.document().is_dirty());
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "z\nx\nx");
    assert!(!core.document().is_dirty());
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "x\nx\nx");
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
fn global_confirmation_honors_counted_ranges_including_overlaps() {
    let mut d = Document::new("x start\nx\nkeep\nx start\nx");
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/start/s/x/z/gc 2");
    key(&mut c, &mut d, Key::Char('a'));
    assert_eq!(key(&mut c, &mut d, Key::Char('a')), CommandStatus::Complete);
    assert_eq!(d.text(), "z start\nz\nkeep\nz start\nz");
    assert!(d.undo());
    // The second target's range covers lines the first invocation changed.
    let mut d = Document::new("x\nx\nx");
    begin(&mut c, &mut d, ":g/x/s/x/xx/gc 2");
    for _ in 0..3 {
        key(&mut c, &mut d, Key::Char('a'));
    }
    assert!(c.substitute_confirmation_prompt().is_none());
    assert_eq!(d.text(), "xx\nxxxx\nxxxx");
    assert!(d.undo());
    assert_eq!(d.text(), "x\nx\nx");
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
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a')))).unwrap();
    // Every view shows a finished invocation while the next question is open.
    assert_eq!(core.document().text(), "origin\nlong\nx\ntail");
    assert_eq!(core.command_state(other).unwrap().cursor(), 14);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a')))).unwrap();
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

fn wrapped_row_starts(text: &str, width: f32) -> Vec<usize> {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::default(), width, 200.);
    core.layout(view).unwrap().snapshot().unwrap().rows.iter().map(|row| row.text_range.start).collect()
}

#[test]
fn global_confirmation_prepares_later_invocations_on_their_displayed_rows() {
    let states = ["x aa x bb x cc", "z aa x bb x cc", "z aa z bb x cc", "z aa z bb z cc"];
    // Wrap the hard line into one row per `x`, before and after each edit.
    let width = (40..600).map(|width| width as f32)
        .find(|width| states.iter().all(|text| wrapped_row_starts(text, *width) == [0, 5, 10]))
        .expect("a width wraps every state at each match");
    let mut core = Core::new(Document::new(states[0]));
    let view = core.add_view(MockTextMeasurementProvider::default(), width, 200.);
    core_begin(&mut core, view, ":.,$g/x/s/x/z/gc");
    for (index, expected) in states.iter().enumerate().skip(1) {
        // Each invocation's `.` is its own displayed row, with one match.
        let prompt = core.command_state(view).unwrap().substitute_confirmation_prompt().unwrap();
        assert!(prompt.ends_with("[1/1; 0 approved]"), "{prompt}");
        let status = core_key(&mut core, view, Key::Char('a'));
        assert_eq!(core.document().text(), *expected);
        if index + 1 < states.len() { assert_eq!(status, CommandStatus::Pending); }
        else { assert_eq!(status, CommandStatus::Complete); }
    }
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), states[0]);
}

#[test]
fn global_confirmation_skips_markdown_targets_merged_by_earlier_invocations() {
    let source = b"x1\n\nx2\n\nx3\n\nend";
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, ":g/x/%s/\\n/ /gc");
    key(&mut c, &mut d, Key::Char('l'));
    assert_eq!(d.text(), "x1 x2\nx3\nend");
    // The `x2` paragraph no longer exists, so only the `x3` target remains.
    assert!(c.substitute_confirmation_prompt().is_some());
    assert_eq!(key(&mut c, &mut d, Key::Char('q')), CommandStatus::Complete);
    assert!(c.substitute_confirmation_prompt().is_none());
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source);
    assert!(!d.undo());
}

#[test]
fn global_confirmation_undo_unit_survives_movement_in_another_view() {
    let mut core = Core::new(Document::new("x\nx\nx"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    let other = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    core_begin(&mut core, view, ":g/x/s/x/z/c");
    core_key(&mut core, view, Key::Char('y'));
    core_key(&mut core, other, Key::Char('j'));
    assert_eq!(core.command_state(other).unwrap().cursor(), 2);
    core_key(&mut core, view, Key::Char('y'));
    assert_eq!(core_key(&mut core, view, Key::Char('y')), CommandStatus::Complete);
    assert_eq!(core.document().text(), "z\nz\nz");
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "x\nx\nx");
}

#[test]
fn global_keeps_physical_markdown_lines_across_paragraph_splits() {
    // Splitting the first line's paragraph moves its folded space to the start
    // of the `x second` source line, which still exists and stays a target.
    let source = b"x first\nx second\n\nkeep";
    let expected = b"x z\n\n&#32;x z\n\n\n\nkeep";
    let physical = |d: &Document| {
        let mut c = CommandInterpreter::new();
        c.set_line_mode(d, viem_core::command::LineMode::PhysicalSource).unwrap();
        c
    };
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut c = physical(&d);
    begin(&mut c, &mut d, r":g/x/s/(first|second)/z\r/c");
    key(&mut c, &mut d, Key::Char('y'));
    assert_eq!(d.source_bytes(), b"x z\n\n&#32;x second\n\nkeep");
    assert!(c.substitute_confirmation_prompt().is_some());
    assert_eq!(key(&mut c, &mut d, Key::Char('y')), CommandStatus::Complete);
    assert_eq!(d.source_bytes(), expected);
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source);
    assert!(!d.undo());
    // Unconfirmed :global rebases its targets the same way.
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut c = physical(&d);
    assert_eq!(ex(&mut c, &mut d, r":g/x/s/(first|second)/z\r/"), CommandStatus::Complete);
    assert_eq!(d.source_bytes(), expected);
    assert!(d.undo());
    assert_eq!(d.source_bytes(), source);
}

#[test]
fn global_confirmation_skips_targets_joined_by_any_replacement() {
    // As in Vim, a match that consumes the break before `x2` joins it into the
    // line above, whether the replacement is text, empty or a new break. Only
    // the `x3` target remains, so quitting its question ends the command.
    for (replacement, joined) in [("NEW ", "NEW x2\nx3\nend"), ("", "x2\nx3\nend"), (r"NEW\r", "NEW\nx2\nx3\nend")] {
        let mut d = Document::new("x1\nx2\nx3\nend");
        let mut c = CommandInterpreter::new();
        begin(&mut c, &mut d, &format!(r":g/x/%s/(x1\n|x3)/{replacement}/gc"));
        key(&mut c, &mut d, Key::Char('l'));
        assert_eq!(d.text(), joined, "{replacement}");
        assert_eq!(key(&mut c, &mut d, Key::Char('q')), CommandStatus::Complete, "{replacement}");
        assert!(c.substitute_confirmation_prompt().is_none());
        assert!(d.undo());
        assert_eq!(d.text(), "x1\nx2\nx3\nend");
        assert!(!d.undo());
    }
}

#[test]
fn global_confirmation_keeps_join_targets_when_the_invocation_fails() {
    // Latin-1 cannot encode the replacement, so nothing is joined and `x2`
    // still gets its own invocation before `x3`.
    let source = b"x1\nx2\nx3\nend";
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
    let mut c = CommandInterpreter::new();
    begin(&mut c, &mut d, r":g/x/%s/(x1\n|x3)/好/gc");
    key(&mut c, &mut d, Key::Char('l'));
    assert_eq!(d.source_bytes(), source);
    assert!(c.substitute_confirmation_prompt().is_some(), "the `x2` invocation asks");
    key(&mut c, &mut d, Key::Char('q'));
    assert!(c.substitute_confirmation_prompt().is_some(), "the `x3` invocation asks");
    let status = key(&mut c, &mut d, Key::Char('q'));
    assert!(format!("{status:?}").contains("UnrepresentableCharacter"), "{status:?}");
    assert!(c.substitute_confirmation_prompt().is_none());
    assert!(!d.undo());
}

#[test]
fn global_confirmation_matching_a_folded_markdown_space_joins_nothing() {
    // The space between folded source lines is not a hard break, and this
    // replacement leaves the source line break in place.
    let source = b"x first\nx second\n\nkeep";
    let mut d = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut c = CommandInterpreter::new();
    c.set_line_mode(&d, viem_core::command::LineMode::PhysicalSource).unwrap();
    begin(&mut c, &mut d, ":g/x/%s/first /first /c");
    key(&mut c, &mut d, Key::Char('l'));
    assert_eq!(d.source_bytes(), source);
    assert!(c.substitute_confirmation_prompt().is_some(), "the `x second` target remains");
    assert_eq!(key(&mut c, &mut d, Key::Char('q')), CommandStatus::Complete);
    assert_eq!(d.source_bytes(), source);
}
