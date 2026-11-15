//! Native Selection and Vim Select are independent interaction policies.
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, Mode, NavigationKey};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};

fn fixture() -> (Document, CommandInterpreter) {
    (
        Document::from_bytes(b"abc def".to_vec(), Encoding::Utf8, Format::PlainText).unwrap(),
        CommandInterpreter::new(),
    )
}
fn send(c: &mut CommandInterpreter, d: &mut Document, event: InputEvent) {
    let result = c.handle(d, event).unwrap();
    assert!(
        matches!(
            result.status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "{:?}",
        result.status
    );
}
fn key(c: &mut CommandInterpreter, d: &mut Document, key: Key) {
    send(c, d, InputEvent::Key(key));
}
fn keys(c: &mut CommandInterpreter, d: &mut Document, keys: &str) {
    for ch in keys.chars() { key(c, d, Key::Char(ch)); }
}
fn text(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
    send(c, d, InputEvent::text(text));
}
fn shifted(key: NavigationKey) -> Key {
    Key::ModifiedNavigation { key, modifiers: 1 }
}
fn set(c: &mut CommandInterpreter, d: &mut Document, value: &str) {
    text(c, d, &format!(":set {value}"));
    key(c, d, Key::Enter);
}

#[test]
fn defaults_keep_vim_options_and_start_visual_selection_from_normal() {
    let (mut d, mut c) = fixture();
    assert!(c.selection_options().autoselect);
    assert!(c.selection_options().shiftselect);
    assert!(c.selection_options().keymodel.is_empty());
    assert!(c.selection_options().selectmode.is_empty());
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert_eq!(c.mode(), Mode::VisualCharacter);
    assert!(!c.is_text_selection());
    text(&mut c, &mut d, "d");
    assert_eq!(d.text(), "c def");
    assert_eq!(c.mode(), Mode::Normal);
}

#[test]
fn shift_selection_option_is_independent_of_insert_selection_option() {
    for autoselect in [false, true] {
        for shiftselect in [false, true] {
            for (entry, mode) in [("", Mode::Normal), ("i", Mode::Insert), ("R", Mode::Replace)] {
                let (mut d, mut c) = fixture();
                set(&mut c, &mut d, &format!("{}autoselect {}shiftselect",
                    if autoselect { "" } else { "no" }, if shiftselect { "" } else { "no" }));
                text(&mut c, &mut d, entry);
                key(&mut c, &mut d, shifted(NavigationKey::Right));
                assert_eq!(c.mode(), if shiftselect { Mode::VisualCharacter } else { mode },
                    "autoselect={autoselect}, shiftselect={shiftselect}, mode={mode:?}");
                assert_eq!(c.is_native_selection(), shiftselect && mode == Mode::Insert);
                assert_eq!(c.cursor(), if shiftselect { 1 } else { 4 });
                if shiftselect && mode == Mode::Insert {
                    text(&mut c, &mut d, "X");
                    assert_eq!(d.text(), "Xbc def");
                    assert_eq!(c.mode(), Mode::Insert);
                }
            }
        }
    }
}

#[test]
fn mouse_selection_stays_native_only_when_enabled_in_insert() {
    for autoselect in [false, true] {
        for shiftselect in [false, true] {
            for (entry, mode) in [("", Mode::Normal), ("i", Mode::Insert), ("R", Mode::Replace)] {
                let (mut d, mut c) = fixture();
                set(&mut c, &mut d, &format!("{}autoselect {}shiftselect",
                    if autoselect { "" } else { "no" }, if shiftselect { "" } else { "no" }));
                text(&mut c, &mut d, entry);
                assert!(c.set_cursor_from_pointer(&d, 1, BoundaryAffinity::Downstream, true));
                assert_eq!(c.mode(), Mode::VisualCharacter);
                assert_eq!(c.is_native_selection(), autoselect && mode == Mode::Insert,
                    "autoselect={autoselect}, shiftselect={shiftselect}, mode={mode:?}");
                assert!(!c.is_select_mode());
            }
        }
    }
}

#[test]
fn visual_entry_and_shift_extension_stay_visual_and_accept_operators() {
    for entry in ["v", "V"] {
        let (mut d, mut c) = fixture();
        text(&mut c, &mut d, entry);
        key(&mut c, &mut d, shifted(NavigationKey::Right));
        key(&mut c, &mut d, Key::Right);
        assert!(!c.is_native_selection());
        assert!(!c.is_select_mode());
        assert_eq!(
            c.mode(),
            if entry == "v" {
                Mode::VisualCharacter
            } else {
                Mode::VisualLine
            }
        );
        text(&mut c, &mut d, "d");
        assert_eq!(d.text(), if entry == "v" { " def" } else { "" });
        assert_eq!(c.mode(), Mode::Normal);
    }
}

#[test]
fn noautoselect_delegates_each_keymodel_and_selectmode_combination_to_vim() {
    for km in ["", "startsel", "stopsel", "startsel,stopsel"] {
        for slm in [
            "",
            "mouse",
            "key",
            "cmd",
            "mouse,key",
            "mouse,cmd",
            "key,cmd",
            "mouse,key,cmd",
        ] {
            let (mut d, mut c) = fixture();
            set(&mut c, &mut d, &format!("noautoselect noshiftselect km={km} slm={slm}"));
            key(&mut c, &mut d, shifted(NavigationKey::Right));
            assert!(!c.is_native_selection());
            let started = km.contains("startsel");
            assert_eq!(
                c.mode(),
                if started {
                    Mode::VisualCharacter
                } else {
                    Mode::Normal
                }
            );
            assert_eq!(
                c.is_select_mode(),
                started && slm.split(',').any(|s| s == "key")
            );
            if started {
                key(&mut c, &mut d, Key::Right);
                assert_eq!(
                    c.mode(),
                    if km.contains("stopsel") {
                        Mode::Normal
                    } else {
                        Mode::VisualCharacter
                    }
                );
            }
            key(&mut c, &mut d, Key::Escape);
            text(&mut c, &mut d, "v");
            assert_eq!(c.is_select_mode(), slm.split(',').any(|s| s == "cmd"));
            key(&mut c, &mut d, Key::Escape);
            assert!(c.set_cursor_from_pointer(&d, 0, BoundaryAffinity::Downstream, false));
            assert!(c.set_cursor_from_pointer(&d, 1, BoundaryAffinity::Downstream, true));
            assert_eq!(c.is_select_mode(), slm.split(',').any(|s| s == "mouse"));
            assert!(!c.is_native_selection());
        }
    }
}

#[test]
fn native_policy_overrides_entry_without_mutating_vim_option_values() {
    let (mut d, mut c) = fixture();
    set(&mut c, &mut d, "km=stopsel slm=cmd");
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert!(c.is_native_selection());
    assert_eq!(c.selection_options().keymodel, "stopsel");
    assert_eq!(c.selection_options().selectmode, "cmd");
    key(&mut c, &mut d, Key::Escape);
    text(&mut c, &mut d, "v");
    assert!(c.is_select_mode());
    assert!(!c.is_native_selection());
}

#[test]
fn vim_shift_selection_is_inclusive_and_stopsel_moves_instead_of_collapsing() {
    let (mut d, mut c) = fixture();
    set(&mut c, &mut d, "noautoselect noshiftselect km=startsel,stopsel slm=key");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert!(c.is_select_mode());
    key(&mut c, &mut d, Key::Right);
    assert_eq!(c.mode(), Mode::Normal);
    assert_eq!(c.cursor(), 2);
    text(&mut c, &mut d, "0");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    text(&mut c, &mut d, "X");
    assert_eq!(d.text(), "Xc def");
}

#[test]
fn vim_select_delete_returns_normal_native_delete_continues_typing() {
    for deletion in [Key::Delete, Key::Backspace] {
        let (mut d, mut c) = fixture();
        keys(&mut c, &mut d, "gh");
        assert!(c.is_select_mode());
        key(&mut c, &mut d, deletion);
        assert_eq!(c.mode(), Mode::Normal);
        assert_eq!(d.text(), "bc def");
        let (mut d, mut c) = fixture();
        text(&mut c, &mut d, "i");
        key(&mut c, &mut d, shifted(NavigationKey::Right));
        key(&mut c, &mut d, deletion);
        assert_eq!(c.mode(), Mode::Insert);
        assert_eq!(d.text(), "bc def");
    }
}

#[test]
fn native_selection_does_not_run_vim_select_mappings_and_ctrl_g_is_explicit() {
    let (mut d, mut c) = fixture();
    set(&mut c, &mut d, "km= slm=");
    text(&mut c, &mut d, ":snoremap x Q");
    key(&mut c, &mut d, Key::Enter);
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Char('x'));
    assert_eq!(d.text(), "xbc def");
    key(&mut c, &mut d, Key::Escape);
    keys(&mut c, &mut d, "gh");
    key(&mut c, &mut d, Key::Char('x'));
    assert_eq!(d.text(), "Qbc def");
    key(&mut c, &mut d, Key::Escape);
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Ctrl('g'));
    assert_eq!(c.mode(), Mode::VisualCharacter);
    assert!(!c.is_text_selection());
    key(&mut c, &mut d, Key::Ctrl('g'));
    assert!(c.is_select_mode());
    assert!(!c.is_native_selection());
}

#[test]
fn pointer_extension_preserves_an_existing_visual_selection_policy() {
    let (mut d, mut c) = fixture();
    text(&mut c, &mut d, "v");
    assert!(c.set_cursor_from_pointer(&d, 2, BoundaryAffinity::Downstream, true));
    assert!(!c.is_text_selection());
    text(&mut c, &mut d, "y");
    assert_eq!(c.register('"').unwrap().text, "abc");
}

#[test]
fn boolean_option_grammar_startup_and_compound_failure_retains_valid_prefix() {
    let (mut d, mut c) = fixture();
    assert!(c.configure_startup(&mut d, "set noautoselect").is_empty());
    for (command, enabled) in [
        ("autoselect", true),
        ("noautoselect", false),
        ("invautoselect", true),
        ("autoselect!", false),
        ("autoselect&", true),
    ] {
        set(&mut c, &mut d, command);
        assert_eq!(c.selection_options().autoselect, enabled);
    }
    text(&mut c, &mut d, ":set noautoselect slm=invalid");
    let result = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
    assert!(matches!(result.status, CommandStatus::ExError(_)));
    assert!(!c.selection_options().autoselect, "the valid prefix commits before the invalid selectmode");
    text(&mut c, &mut d, ":set autoselect?");
    let result = c
        .handle(&mut d, InputEvent::Key(Key::Enter))
        .unwrap()
        .ex_outcome
        .unwrap();
    assert!(result.option_effects.is_empty());
    assert_eq!(result.frontend_requests.len(), 1);
}

#[test]
fn changing_global_option_preserves_current_selection_and_pending_commands() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let (d, _) = fixture();
    let mut core = Core::new(d);
    let a = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let b = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(a, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(
        a,
        CoreEvent::Input(InputEvent::Key(shifted(NavigationKey::Right))),
    )
    .unwrap();
    core.handle(b, CoreEvent::Input(InputEvent::key('d')))
        .unwrap();
    core.set_autoselect(false);
    assert!(core.command_state(a).unwrap().is_native_selection());
    assert!(
        !core
            .command_state(a)
            .unwrap()
            .selection_options()
            .autoselect
    );
    core.handle(b, CoreEvent::Input(InputEvent::key('w')))
        .unwrap();
    assert_eq!(core.document().text(), "def");
    assert!(
        !core
            .command_state(b)
            .unwrap()
            .selection_options()
            .autoselect
    );
}

#[test]
fn native_copy_preserves_its_own_policy() {
    let (mut d, mut c) = fixture();
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Ctrl('o'));
    text(&mut c, &mut d, "y");
    assert!(c.is_native_selection());
    assert!(!c.is_select_mode());
    assert_eq!(c.register('"').unwrap().text, "a");
    text(&mut c, &mut d, "X");
    assert_eq!(d.text(), "Xbc def");
}

#[test]
fn vim_shift_arrows_without_startsel_are_word_motions() {
    let (mut d, mut c) = fixture();
    set(&mut c, &mut d, "noshiftselect");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert_eq!(c.mode(), Mode::Normal);
    assert_eq!(c.cursor(), 4);
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    assert_eq!(c.cursor(), 0);
}

#[test]
fn native_tab_replaces_selection_instead_of_running_vim_jump_forward() {
    let (mut d, mut c) = fixture();
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Tab);
    assert_eq!(c.mode(), Mode::Insert);
    // Native replacement still respects the configured two-space smart Tab.
    assert_eq!(d.text(), "  bc def");
}

#[test]
fn native_line_selection_collapses_to_its_edges_and_resumes_the_originating_mode() {
    use viem_core::command::SelectionOrigin;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for resume in [Mode::Insert] {
        for (direction, boundary) in [(Key::Left, 0), (Key::Right, 7)] {
            let (document, _) = fixture();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
            core.handle(view, CoreEvent::Input(InputEvent::key('V')))
                .unwrap();
            core.set_selection_origin(view, SelectionOrigin::Key, resume)
                .unwrap();
            assert!(core.command_state(view).unwrap().is_native_selection());
            core.handle(view, CoreEvent::Input(InputEvent::Key(direction)))
                .unwrap();
            let commands = core.command_state(view).unwrap();
            assert_eq!(commands.mode(), resume);
            assert_eq!(commands.cursor(), boundary);
            assert!(!commands.is_text_selection());
            assert_eq!(core.document().text(), "abc def");
        }
    }
}

#[test]
fn explicit_shift_mappings_override_both_settings_from_startup_and_commands() {
    for startup in [false, true] {
        for enabled in [false, true] {
            for (mapping, entry, mode) in [("nnoremap", "", Mode::Normal), ("inoremap", "i", Mode::Insert)] {
                let (mut d, mut c) = fixture();
                let options = format!("set {}autoselect {}shiftselect",
                    if enabled { "" } else { "no" }, if enabled { "" } else { "no" });
                let mapping = format!("{mapping} <S-Right> <Right>");
                if startup {
                    assert!(c.configure_startup(&mut d, &format!("{options}\n{mapping}")).is_empty());
                } else {
                    text(&mut c, &mut d, &format!(":{options}"));
                    key(&mut c, &mut d, Key::Enter);
                    text(&mut c, &mut d, &format!(":{mapping}"));
                    key(&mut c, &mut d, Key::Enter);
                }
                text(&mut c, &mut d, entry);
                key(&mut c, &mut d, shifted(NavigationKey::Right));
                assert_eq!(c.mode(), mode);
                assert_eq!(c.cursor(), 1);
                assert!(!c.is_text_selection());
                assert_eq!(d.text(), "abc def");
            }
        }
    }
}

#[test]
fn insert_navigation_mapping_overrides_an_active_insert_selection() {
    for navigation in [Key::Left, shifted(NavigationKey::Left)] {
        let (mut d, mut c) = fixture();
        let lhs = if navigation == Key::Left { "<Left>" } else { "<S-Left>" };
        assert!(c.configure_startup(&mut d, &format!("inoremap {lhs} <Esc>")).is_empty());
        text(&mut c, &mut d, "i");
        key(&mut c, &mut d, shifted(NavigationKey::Right));
        assert!(c.is_native_selection());
        key(&mut c, &mut d, navigation);
        assert_eq!(c.mode(), Mode::Normal);
        assert!(!c.is_text_selection());
        assert_eq!(d.text(), "abc def");
    }
}

#[test]
fn insert_selection_navigation_mapping_prefix_retains_the_selection_until_resolved() {
    let (mut d, mut c) = fixture();
    assert!(c.configure_startup(&mut d, "inoremap <S-Left>x <Esc>").is_empty());
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    let anchor = c.visual_anchor();
    let cursor = c.cursor();
    let output = c.handle(&mut d, InputEvent::Key(shifted(NavigationKey::Left))).unwrap();
    assert_eq!(output.status, CommandStatus::Pending);
    assert!(c.is_native_selection());
    assert_eq!(c.visual_anchor(), anchor);
    assert_eq!(c.cursor(), cursor);
    key(&mut c, &mut d, Key::Char('x'));
    assert_eq!(c.mode(), Mode::Normal);
    assert!(!c.is_text_selection());
    assert_eq!(d.text(), "abc def");
}

#[test]
fn insert_selection_replacement_still_uses_insert_typing_mappings() {
    let (mut d, mut c) = fixture();
    assert!(c.configure_startup(&mut d, "inoremap x Q\nsnoremap x Z\nxnoremap x d").is_empty());
    text(&mut c, &mut d, "i");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Char('x'));
    assert_eq!(c.mode(), Mode::Insert);
    assert_eq!(d.text(), "Qbc def");
}

#[test]
fn disabled_shift_selection_keeps_explicit_vim_startsel_and_insert_policy() {
    for autoselect in [false, true] {
        let (mut d, mut c) = fixture();
        set(&mut c, &mut d, &format!("{}autoselect noshiftselect keymodel=startsel selectmode=key",
            if autoselect { "" } else { "no" }));
        text(&mut c, &mut d, "i");
        key(&mut c, &mut d, shifted(NavigationKey::Right));
        assert_eq!(c.mode(), Mode::VisualCharacter);
        assert_eq!(c.is_native_selection(), autoselect);
        assert_eq!(c.is_select_mode(), !autoselect);
    }
}

#[test]
fn shiftselect_boolean_option_grammar_and_startup_override() {
    let (mut d, mut c) = fixture();
    assert!(c.configure_startup(&mut d, "set noshiftselect").is_empty());
    assert!(!c.selection_options().shiftselect);
    for (command, enabled) in [
        ("shiftselect", true), ("noshiftselect", false), ("invshiftselect", true),
        ("shiftselect!", false), ("shiftselect&", true),
    ] {
        set(&mut c, &mut d, command);
        assert_eq!(c.selection_options().shiftselect, enabled);
    }
    text(&mut c, &mut d, ":set shiftselect?");
    let output = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap().ex_outcome.unwrap();
    assert!(output.option_effects.is_empty());
    assert_eq!(output.frontend_requests.len(), 1);
}

#[test]
fn clicking_an_insert_selection_resumes_typing_with_a_fresh_undo_and_repeat_unit() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let (document, _) = fixture();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    for input in [Key::Char('i'), Key::Char('A'), shifted(NavigationKey::Right)] {
        core.handle(view, CoreEvent::Input(InputEvent::Key(input))).unwrap();
    }
    assert!(core.command_state(view).unwrap().is_native_selection());
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: 4,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    for input in [Key::Char('X'), Key::Char('Y'), Key::Escape] {
        core.handle(view, CoreEvent::Input(InputEvent::Key(input))).unwrap();
    }
    assert_eq!(core.document().text(), "AabcXY def");
    core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
    assert_eq!(core.document().text(), "Aabc def");
    core.handle(view, CoreEvent::Input(InputEvent::key('.'))).unwrap();
    assert_eq!(core.document().text(), "AabcXY def");
}

#[test]
fn repeated_select_all_preserves_insert_selection_origin() {
    let (mut d, mut c) = fixture();
    text(&mut c, &mut d, "i");
    for _ in 0..2 {
        key(&mut c, &mut d, Key::SelectAll);
        assert!(c.is_native_selection());
    }
    key(&mut c, &mut d, Key::Right);
    assert_eq!(c.mode(), Mode::Insert);
    assert_eq!(c.cursor(), 7);
}
