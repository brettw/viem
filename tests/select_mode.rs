use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, Mode, NavigationKey};
use viem_core::document::{Document, Encoding, Format};
fn doc(text: &str) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap()
}
fn key(c: &mut CommandInterpreter, d: &mut Document, k: Key) {
    let out = c.handle(d, InputEvent::Key(k)).unwrap();
    assert!(
        matches!(
            out.status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "{k:?}: {:?}",
        out.status
    );
}
fn text(c: &mut CommandInterpreter, d: &mut Document, text: &str) {
    let out = c.handle(d, InputEvent::Text(text.into())).unwrap();
    assert!(
        matches!(
            out.status,
            CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
        ),
        "{}: {:?}",
        text,
        out.status
    );
}
fn shifted(k: NavigationKey) -> Key {
    Key::ModifiedNavigation {
        key: k,
        modifiers: 1,
    }
}
fn ex(c: &mut CommandInterpreter, d: &mut Document, s: &str) {
    text(c, d, &format!(":{s}"));
    key(c, d, Key::Enter);
}
#[test]
fn shift_selection_replaces_exactly_one_grapheme_and_undoes_as_one_edit() {
    let mut d = doc("abc xyz");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert!(c.is_text_selection());
    assert!(!c.caret_target(&d).is_cell());
    text(&mut c, &mut d, "QQ");
    assert_eq!(d.text(), "QQbc xyz");
    assert_eq!(c.mode(), Mode::Insert);
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    assert_eq!(d.text(), "abc xyz");
}
#[test]
fn options_choose_visual_or_select_independently_for_keys_and_commands() {
    let mut d = doc("abcdef");
    let mut c = CommandInterpreter::new();
    ex(&mut c, &mut d, "set noautoselect selectmode= keymodel=startsel,stopsel");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert!(!c.is_text_selection());
    assert_eq!(c.mode(), Mode::VisualCharacter);
    key(&mut c, &mut d, Key::Escape);
    ex(&mut c, &mut d, "set selectmode=cmd");
    key(&mut c, &mut d, Key::Char('v'));
    assert!(c.is_text_selection());
    key(&mut c, &mut d, Key::Ctrl('g'));
    assert!(!c.is_text_selection());
    key(&mut c, &mut d, Key::Escape);
    ex(&mut c, &mut d, "set selectmode= keymodel=");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    assert_eq!(c.mode(), Mode::Normal);
    key(&mut c, &mut d, Key::Char('g'));
    key(&mut c, &mut d, Key::Char('h'));
    assert!(c.is_text_selection());
    text(&mut c, &mut d, "X");
    assert_eq!(d.text(), "abcdeX");
}
#[test]
fn select_register_blackhole_and_stopsel() {
    let mut d = doc("abcdef");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Ctrl('r'));
    key(&mut c, &mut d, Key::Char('_'));
    text(&mut c, &mut d, "Z");
    assert_eq!(d.text(), "Zbcdef");
    assert!(c.register('"').is_none_or(|v| v.text.is_empty()));
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Right);
    assert!(!c.is_text_selection());
    assert_eq!(c.mode(), Mode::Normal);
}
#[test]
fn select_replacement_dot_preserves_selection_size_and_count() {
    let mut d = doc("abcdef");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    text(&mut c, &mut d, "Z");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('l'));
    key(&mut c, &mut d, Key::Char('.'));
    assert_eq!(d.text(), "ZZcdef");
    key(&mut c, &mut d, Key::Char('u'));
    assert_eq!(d.text(), "Zbcdef");
}
#[test]
fn explicit_select_count_line_and_temporary_visual_command() {
    let mut d = doc("abcd\nefgh\nijkl");
    let mut c = CommandInterpreter::new();
    text(&mut c, &mut d, "3gh");
    assert!(c.is_text_selection());
    text(&mut c, &mut d, "Q");
    assert_eq!(d.text(), "Qd\nefgh\nijkl");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    text(&mut c, &mut d, "gH");
    assert!(c.is_text_selection());
    key(&mut c, &mut d, Key::Ctrl('o'));
    key(&mut c, &mut d, Key::Char('$'));
    assert!(c.is_text_selection());
    key(&mut c, &mut d, Key::Backspace);
    assert_eq!(d.text(), "efgh\nijkl");
}
#[test]
fn startsel_from_insert_and_backwards_unicode_boundaries() {
    let mut d = doc("á🙂z");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, Key::Char('A'));
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    assert!(c.is_text_selection());
    text(&mut c, &mut d, "Q");
    assert_eq!(d.text(), "á🙂Q");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    assert_eq!(d.text(), "á🙂z");
}
#[test]
fn core_layout_shift_selection_and_block_replacement() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(doc("abcd\nefgh"));
    let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    for event in [
        InputEvent::Key(shifted(NavigationKey::Right)),
        InputEvent::Text("Q".into()),
        InputEvent::Key(Key::Escape),
    ] {
        let out = core.handle(v, CoreEvent::Input(event)).unwrap();
        assert!(
            out.command.as_ref().is_none_or(|c| matches!(
                c.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            )),
            "{out:?}"
        );
    }
    assert_eq!(core.document().text(), "Qbcd\nefgh");
    for event in [
        InputEvent::Key(Key::Char('g')),
        InputEvent::Key(Key::Ctrl('h')),
        InputEvent::Key(shifted(NavigationKey::Down)),
        InputEvent::Text("X".into()),
        InputEvent::Key(Key::Escape),
    ] {
        let out = core.handle(v, CoreEvent::Input(event)).unwrap();
        assert!(
            out.command.as_ref().is_none_or(|c| matches!(
                c.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            )),
            "{out:?}"
        );
    }
    assert_eq!(core.document().text(), "Xbcd\nXfgh");
}
#[test]
fn select_composition_cancel_restores_selection_and_commit_replaces_atomically() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(doc("abc xyz"));
    let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(
        v,
        CoreEvent::Input(InputEvent::Key(shifted(NavigationKey::Right))),
    )
    .unwrap();
    let begin =
        CompositionEvent::Begin(CompositionTarget::at_offsets(core.document(), 0..1).unwrap());
    core.handle(v, CoreEvent::Composition(begin.clone()))
        .unwrap();
    core.handle(
        v,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("界", 3..3))),
    )
    .unwrap();
    assert_eq!(core.document().text(), "abc xyz");
    core.handle(v, CoreEvent::Composition(CompositionEvent::Cancel))
        .unwrap();
    assert!(core.command_state(v).unwrap().is_text_selection());
    assert_eq!(core.command_state(v).unwrap().cursor(), 1);
    core.handle(v, CoreEvent::Composition(begin)).unwrap();
    core.handle(
        v,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("界", 3..3))),
    )
    .unwrap();
    core.handle(v, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    assert_eq!(core.document().text(), "界bc xyz");
    assert_eq!(core.command_state(v).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(v).unwrap().cursor(), 3);
    assert_eq!(
        core.command_state(v).unwrap().register('"').unwrap().text,
        "a"
    );
    core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
        .unwrap();
    assert_eq!(core.document().text(), "abc xyz");
}
#[test]
fn native_stopsel_collapses_bounds_and_empty_selection_types_without_deletion() {
    let mut d = doc("abcd");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Left);
    assert_eq!(c.cursor(), 0);
    assert!(!c.is_text_selection());
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Right);
    assert_eq!(c.cursor(), 1);
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    text(&mut c, &mut d, "Q");
    assert_eq!(d.text(), "aQbcd");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    assert_eq!(d.text(), "abcd");
    key(&mut c, &mut d, Key::Char('A'));
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    key(&mut c, &mut d, Key::Right);
    assert_eq!(c.mode(), Mode::Insert);
    assert_eq!(c.cursor(), 4);
    text(&mut c, &mut d, "!");
    assert_eq!(d.text(), "abcd!");
}
#[test]
fn temporary_visual_yank_retains_reversed_native_selection_and_register() {
    let mut d = doc("abcd");
    let mut c = CommandInterpreter::new();
    key(&mut c, &mut d, Key::Char('A'));
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    key(&mut c, &mut d, shifted(NavigationKey::Left));
    let anchor = c.visual_anchor();
    let active = c.cursor();
    key(&mut c, &mut d, Key::Ctrl('o'));
    key(&mut c, &mut d, Key::Char('y'));
    assert!(c.is_text_selection());
    assert_eq!(c.visual_anchor(), anchor);
    assert_eq!(c.cursor(), active);
    assert_eq!(c.register('"').unwrap().text, "cd");
    text(&mut c, &mut d, "Q");
    assert_eq!(d.text(), "abQ");
}
#[test]
fn mappings_distinguish_visual_and_select_and_shift_notation() {
    let mut d = doc("abcd");
    let mut c = CommandInterpreter::new();
    ex(&mut c, &mut d, "xnoremap <F1> U");
    ex(&mut c, &mut d, "snoremap <F1> Q");
    text(&mut c, &mut d, "gh");
    key(
        &mut c,
        &mut d,
        Key::Function {
            number: 1,
            modifiers: 0,
        },
    );
    assert_eq!(d.text(), "Qbcd");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('l'));
    key(&mut c, &mut d, Key::Char('v'));
    key(
        &mut c,
        &mut d,
        Key::Function {
            number: 1,
            modifiers: 0,
        },
    );
    assert_eq!(d.text(), "QBcd");
    assert_eq!(
        viem_core::command::mappings::parse_key_notation("<S-Left><C-S-Right>").unwrap(),
        vec![
            shifted(NavigationKey::Left),
            Key::ModifiedNavigation {
                key: NavigationKey::WordRight,
                modifiers: 3
            }
        ]
    );
}
#[test]
fn host_selection_option_setter_preserves_all_view_pending_input() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(doc("one two three"));
    let a = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let b = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(a, CoreEvent::Input(InputEvent::Key(Key::Char('d'))))
        .unwrap();
    assert!(core.set_selection_option(false, "cmd"));
    assert_eq!(
        core.command_state(b)
            .unwrap()
            .selection_options()
            .selectmode,
        "cmd"
    );
    assert!(!core.set_selection_option(true, "startsel,bogus"));
    assert_eq!(core.selection_options().keymodel, "");
    core.handle(a, CoreEvent::Input(InputEvent::Key(Key::Char('w'))))
        .unwrap();
    assert_eq!(core.document().text(), "two three");
}
#[test]
fn select_backspace_and_delete_enter_insert_and_group_continued_typing() {
    for deletion in [Key::Backspace, Key::Delete] {
        let mut d = doc("abcd");
        let mut c = CommandInterpreter::new();
        key(&mut c, &mut d, shifted(NavigationKey::Right));
        key(&mut c, &mut d, deletion);
        assert_eq!(c.mode(), Mode::Insert);
        assert_eq!(d.text(), "bcd");
        assert_eq!(c.register('"').unwrap().text, "a");
        text(&mut c, &mut d, "XYZ");
        assert_eq!(d.text(), "XYZbcd");
        key(&mut c, &mut d, Key::Escape);
        key(&mut c, &mut d, Key::Char('u'));
        assert_eq!(d.text(), "abcd");
    }
}
#[test]
fn visual_mapping_in_select_runs_as_visual_and_first_replacement_uses_insert_mapping() {
    let mut d = doc("abcd");
    let mut c = CommandInterpreter::new();
    ex(&mut c, &mut d, "vnoremap <F2> U");
    ex(&mut c, &mut d, "inoremap x XYZ");
    text(&mut c, &mut d, "gh");
    key(
        &mut c,
        &mut d,
        Key::Function {
            number: 2,
            modifiers: 0,
        },
    );
    assert_eq!(d.text(), "Abcd");
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Char('x'));
    assert_eq!(d.text(), "XYZbcd");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    assert_eq!(d.text(), "Abcd");
}
#[test]
fn select_line_replacement_is_characterwise_and_native_selection_gv_preserves_extent() {
    let mut d = doc("abc\ndef");
    let mut c = CommandInterpreter::new();
    text(&mut c, &mut d, "gH");
    text(&mut c, &mut d, "X");
    assert_eq!(d.text(), "Xdef");
    key(&mut c, &mut d, Key::Escape);
    key(&mut c, &mut d, Key::Char('u'));
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    key(&mut c, &mut d, Key::Escape);
    text(&mut c, &mut d, "gv");
    key(&mut c, &mut d, Key::Char('y'));
    assert_eq!(c.register('"').unwrap().text, "a");
}
#[test]
fn selected_clean_character_style_survives_replacement_and_replacement_undo() {
    use viem_core::document::StyleNamespace;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for choice in ["Code", ""] {
        let source =
            "<p><span style='color:red;font-size:30pt'><b><sup>word</sup></b></span> tail</p>";
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap(),
        );
        let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.handle(
            v,
            CoreEvent::Input(InputEvent::Key(shifted(NavigationKey::Right))),
        )
        .unwrap();
        core.handle(
            v,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(v).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: choice.into(),
            },
        )
        .unwrap();
        let after_choice = core.document().source_bytes();
        let expected = core
            .document()
            .typing_named_style_at(
                0,
                viem_core::document::BoundaryAffinity::Downstream,
                &choice.into(),
            )
            .unwrap();
        assert!(core.command_state(v).unwrap().is_text_selection());
        core.handle(v, CoreEvent::Input(InputEvent::Text("XY".into())))
            .unwrap();
        assert_eq!(core.document().text(), "XYord tail");
        for at in 0..2 {
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false
                )
                .unwrap(),
                expected,
                "choice={choice} source={:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            );
        }
        core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), after_choice);
    }
}
#[test]
fn select_all_delete_clears_rich_structure_and_enters_insert() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (format, source) in [
        (Format::Markdown, "> quote\n> more"),
        (Format::Html, "<blockquote><p>quote</p></blockquote>"),
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
        );
        let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        core.handle(
            v,
            CoreEvent::SelectAll {
                document: core.document().id(),
                revision: core.document().revision(),
            },
        )
        .unwrap();
        core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(core.document().text(), "");
        assert_eq!(core.command_state(v).unwrap().mode(), Mode::Insert);
        core.handle(v, CoreEvent::Input(InputEvent::Text("plain".into())))
            .unwrap();
        let source = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(
            !source.contains("blockquote") && !source.contains("> plain"),
            "{source}"
        );
    }
}
#[test]
fn counted_shift_motion_and_unshifted_modified_navigation_dispatch_normally() {
    let mut d = doc("one two three");
    let mut c = CommandInterpreter::new();
    key(
        &mut c,
        &mut d,
        Key::ModifiedNavigation {
            key: NavigationKey::WordRight,
            modifiers: 4,
        },
    );
    assert_eq!(c.cursor(), 4);
    key(
        &mut c,
        &mut d,
        Key::ModifiedNavigation {
            key: NavigationKey::DocumentStart,
            modifiers: 2,
        },
    );
    assert_eq!(c.cursor(), 0);
    key(&mut c, &mut d, Key::Char('3'));
    key(&mut c, &mut d, shifted(NavigationKey::Right));
    text(&mut c, &mut d, "X");
    assert_eq!(d.text(), "X two three");
}
#[test]
fn option_lists_query_modify_reset_and_invalid_compound_assignment_is_atomic() {
    let mut d = doc("abc");
    let mut c = CommandInterpreter::new();
    for (command, keymodel, selectmode) in [
        ("set km= slm=", "", ""),
        ("set km+=startsel slm+=mouse", "startsel", "mouse"),
        ("set km^=stopsel slm^=key", "stopsel,startsel", "key,mouse"),
        ("set km-=startsel slm+=cmd", "stopsel", "key,mouse,cmd"),
        ("set km+=stopsel slm-=mouse", "stopsel", "key,cmd"),
        ("set km& slm&", "", ""),
    ] {
        ex(&mut c, &mut d, command);
        assert_eq!(c.selection_options().keymodel, keymodel);
        assert_eq!(c.selection_options().selectmode, selectmode);
    }
    text(&mut c, &mut d, ":set km= slm=invalid");
    let result = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
    assert!(matches!(result.status, CommandStatus::ExError(_)));
    assert_eq!(c.selection_options().keymodel, "");
    assert_eq!(c.selection_options().selectmode, "");
    text(&mut c, &mut d, ":set km? slm?");
    let result = c.handle(&mut d, InputEvent::Key(Key::Enter)).unwrap();
    let result = result.ex_outcome.unwrap();
    assert!(result.option_effects.is_empty());
    assert_eq!(result.frontend_requests.len(), 2);
}
#[test]
fn native_mouse_and_word_origin_use_exact_boundaries_and_collapse_without_extra_motion() {
    use viem_core::command::SelectionOrigin;
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(doc("word tail"));
    let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(
        v,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 4,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: true,
        },
    )
    .unwrap();
    assert!(core.command_state(v).unwrap().is_text_selection());
    assert_eq!(core.list_selection_identity(v).unwrap().range(), 0..4);
    core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Left)))
        .unwrap();
    assert_eq!(core.command_state(v).unwrap().cursor(), 0);
    for k in ['v', 'i', 'w'] {
        core.handle(v, CoreEvent::Input(InputEvent::key(k)))
            .unwrap();
    }
    core.set_selection_origin(v, SelectionOrigin::Mouse, Mode::Normal)
        .unwrap();
    assert_eq!(core.list_selection_identity(v).unwrap().range(), 0..4);
    core.handle(v, CoreEvent::Input(InputEvent::Key(Key::Right)))
        .unwrap();
    assert_eq!(core.command_state(v).unwrap().cursor(), 4);
}
#[test]
fn collapsed_native_selection_accepts_clean_pending_style_then_typing() {
    use viem_core::document::StyleNamespace;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(
        Document::from_bytes(b"<p><b>word</b></p>".to_vec(), Encoding::Utf8, Format::Html).unwrap(),
    );
    let v = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    for k in [NavigationKey::Right, NavigationKey::Left] {
        core.handle(v, CoreEvent::Input(InputEvent::Key(shifted(k))))
            .unwrap();
    }
    let before = core.document().source_bytes();
    core.handle(
        v,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(v).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: "Code".into(),
        },
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), before);
    core.handle(v, CoreEvent::Input(InputEvent::text("X")))
        .unwrap();
    assert_eq!(core.document().text(), "Xword");
    let style = DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false)
        .unwrap();
    assert!(!style.bold);
    assert_eq!(style.font_families, vec!["monospace"]);
}
#[test]
fn native_control_word_key_matches_the_same_mapping_as_control_notation() {
    let mut d = doc("abcd");
    let mut c = CommandInterpreter::new();
    ex(&mut c, &mut d, "nnoremap <C-Right> iQ<Esc>");
    key(
        &mut c,
        &mut d,
        Key::ModifiedNavigation {
            key: NavigationKey::WordRight,
            modifiers: 2,
        },
    );
    assert_eq!(d.text(), "Qabcd");
    let mut c = CommandInterpreter::new();
    let mut d = doc("abc");
    assert!(c
        .configure_startup(&mut d, "set keymodel=stopsel\nset selectmode=cmd")
        .is_empty());
    assert_eq!(c.selection_options().keymodel, "stopsel");
    assert_eq!(c.selection_options().selectmode, "cmd");
    assert_eq!(c.mode(), Mode::Normal);
}
#[test]
fn temporary_visual_copy_retains_forward_and_reversed_select_block_geometry() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for reversed in [false, true] {
        let mut core = Core::new(doc("abcd\nefgh"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let mut inputs = vec![InputEvent::key('l')];
        if reversed {
            inputs.push(InputEvent::key('j'));
        }
        inputs.extend([
            InputEvent::key('g'),
            InputEvent::Key(Key::Ctrl('h')),
            InputEvent::Key(shifted(if reversed {
                NavigationKey::Up
            } else {
                NavigationKey::Down
            })),
            InputEvent::Key(shifted(NavigationKey::Left)),
        ]);
        for input in inputs {
            core.handle(view, CoreEvent::Input(input)).unwrap();
        }
        let before = core
            .command_state(view)
            .unwrap()
            .visual_block()
            .unwrap()
            .clone();
        let before_segments = viem_core::command::visual_block::resolve_block_selection(
            &before,
            core.layout(view).unwrap().snapshot().unwrap(),
            core.document().text(),
        )
        .unwrap()
        .range_set
        .segments;
        let source = core.document().source_bytes();
        for input in [Key::Ctrl('o'), Key::Char('y')] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(input)))
                .unwrap();
        }
        let commands = core.command_state(view).unwrap();
        assert!(commands.is_text_selection());
        let after = commands
            .visual_block()
            .expect("copy must retain the selected rectangle");
        assert_eq!(after.anchor.text_offset, before.anchor.text_offset);
        assert_eq!(after.active.text_offset, before.active.text_offset);
        assert_eq!(after.left_x(), before.left_x());
        assert_eq!(after.right_x(), before.right_x());
        let after_segments = viem_core::command::visual_block::resolve_block_selection(
            after,
            core.layout(view).unwrap().snapshot().unwrap(),
            core.document().text(),
        )
        .unwrap()
        .range_set
        .segments;
        assert_eq!(after_segments, before_segments);
        assert_eq!(commands.register('"').unwrap().text, "ab\nef");
        assert_eq!(core.document().source_bytes(), source);
    }
}
#[test]
fn oversized_shift_motion_stops_at_the_document_boundary() {
    let mut document = doc("abc");
    let mut commands = CommandInterpreter::new();
    text(&mut commands, &mut document, "999999999999");
    key(&mut commands, &mut document, shifted(NavigationKey::Right));
    assert_eq!(commands.cursor(), 3);
    text(&mut commands, &mut document, "X");
    assert_eq!(document.text(), "X");
}

#[test]
fn select_register_operand_accepts_native_text_and_batched_replacement() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (register, combined) in [('_', false), ('_', true), ('a', false), ('a', true)] {
        let mut core = Core::new(doc("abc"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let mut events = vec![
            InputEvent::Key(shifted(NavigationKey::Right)),
            InputEvent::Key(Key::Ctrl('r')),
            InputEvent::Text(String::new()),
        ];
        if combined {
            events.push(InputEvent::text(format!("{register}XY")));
        } else {
            events.extend([
                InputEvent::text(register.to_string()),
                InputEvent::text("XY"),
            ]);
        }
        for event in events {
            let outcome = core.handle(view, CoreEvent::Input(event)).unwrap();
            assert!(
                outcome.command.as_ref().is_none_or(|command| matches!(
                    command.status,
                    CommandStatus::Complete | CommandStatus::Pending
                )),
                "{outcome:?}"
            );
        }
        assert_eq!(core.document().text(), "XYbc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        if register == '_' {
            assert!(core
                .command_state(view)
                .unwrap()
                .register('"')
                .is_none_or(|value| value.text.is_empty()));
        } else {
            assert_eq!(
                core.command_state(view)
                    .unwrap()
                    .register('a')
                    .unwrap()
                    .text,
                "a"
            );
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().text(), "abc");
    }
}

#[test]
fn select_register_text_operand_is_not_remapped_and_invalid_operands_do_not_replace() {
    let mut document = doc("abc");
    let mut commands = CommandInterpreter::new();
    ex(&mut commands, &mut document, "snoremap _ X");
    key(&mut commands, &mut document, shifted(NavigationKey::Right));
    key(&mut commands, &mut document, Key::Ctrl('r'));
    text(&mut commands, &mut document, "_Q");
    assert_eq!(document.text(), "Qbc");
    assert!(commands
        .register('"')
        .is_none_or(|value| value.text.is_empty()));

    let mut document = doc("abc");
    let mut commands = CommandInterpreter::new();
    key(&mut commands, &mut document, shifted(NavigationKey::Right));
    key(&mut commands, &mut document, Key::Ctrl('r'));
    let output = commands
        .handle(&mut document, InputEvent::text("界Q"))
        .unwrap();
    assert!(matches!(output.status, CommandStatus::Unsupported(_)));
    assert_eq!(document.text(), "abc");
    assert!(commands.is_text_selection());
}
