//! Shared menu, key, prompt and status contracts exercised through coordinator input.
use viem_core::command::{
    CommandStatus, InputEvent, Key, Mode, NativeAction, NativeActionKind, NavigationKey,
    SelectionOrigin,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};
fn fixture(text: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 300.);
    (core, view)
}
fn send(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) -> CommandStatus {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap()
        .command
        .unwrap()
        .status
}
fn text(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for c in text.chars() {
        let status = send(core, view, Key::Char(c));
        assert!(
            matches!(status, CommandStatus::Complete | CommandStatus::Pending),
            "{c}: {status:?}"
        );
    }
}
fn action(kind: NativeActionKind) -> Key {
    Key::NativeAction(NativeAction {
        kind,
        origin: SelectionOrigin::Key,
        return_mode: None,
    })
}
#[test]
fn native_selection_ignores_mappings_and_returns_to_insert_with_separate_undo() {
    let (mut core, view) = fixture("one two");
    text(&mut core, view, ":nnoremap v x");
    send(&mut core, view, Key::Enter);
    text(&mut core, view, "iA");
    assert_eq!(
        send(&mut core, view, action(NativeActionKind::SelectWord)),
        CommandStatus::Complete
    );
    assert!(core.command_state(view).unwrap().is_native_selection());
    assert_eq!(core.document().text(), "Aone two");
    send(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    text(&mut core, view, "B");
    send(&mut core, view, Key::Escape);
    text(&mut core, view, "u");
    assert_eq!(core.document().text(), "Aone two");
    text(&mut core, view, "u");
    assert_eq!(core.document().text(), "one two");
}
#[test]
fn menu_actions_respect_pending_input_and_find_uses_shared_prompt() {
    let (mut core, view) = fixture("one two one");
    text(&mut core, view, "2d");
    let before = core.command_state(view).unwrap().pending_command_text();
    for kind in [
        NativeActionKind::SelectWord,
        NativeActionKind::Find,
        NativeActionKind::FindNext,
    ] {
        assert!(!core
            .command_state(view)
            .unwrap()
            .native_action_available(kind));
        assert!(matches!(
            send(&mut core, view, action(kind)),
            CommandStatus::Unsupported(_)
        ));
        assert_eq!(
            core.command_state(view).unwrap().pending_command_text(),
            before
        );
    }
    send(&mut core, view, Key::Escape);
    assert_eq!(
        send(&mut core, view, action(NativeActionKind::Find)),
        CommandStatus::Complete
    );
    text(&mut core, view, "one");
    send(&mut core, view, Key::Enter);
    assert_eq!(core.command_state(view).unwrap().cursor(), 8);
    send(&mut core, view, action(NativeActionKind::FindNext));
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    send(&mut core, view, action(NativeActionKind::FindReplace));
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        "%s/"
    );
    assert_eq!(core.document().text(), "one two one");
}
#[test]
fn each_selection_intention_has_a_native_selection() {
    for kind in [
        NativeActionKind::SelectWord,
        NativeActionKind::SelectSentence,
        NativeActionKind::SelectParagraph,
        NativeActionKind::SelectLine,
        NativeActionKind::SelectVisualRow,
        NativeActionKind::SelectBlock,
    ] {
        let (mut core, view) = fixture("First sentence. Second sentence.\n\nNext paragraph.");
        assert_eq!(
            send(&mut core, view, action(kind)),
            CommandStatus::Complete,
            "{kind:?}"
        );
        assert!(
            core.command_state(view).unwrap().is_native_selection(),
            "{kind:?}"
        );
        assert!(!core.document().is_dirty());
    }
}
#[test]
fn shifted_prompt_navigation_keeps_direction_and_graphemes() {
    for prompt in [':', '/', '?'] {
        let (mut core, view) = fixture("untouched");
        send(&mut core, view, Key::Char(prompt));
        text(&mut core, view, "aé👩‍💻z");
        let end = "aé👩‍💻z".len();
        let shift = |key| Key::ModifiedNavigation { key, modifiers: 1 };
        send(&mut core, view, shift(NavigationKey::Left));
        let selected = core
            .command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap();
        assert_eq!((selected.anchor, selected.active), (end, end - 1));
        send(&mut core, view, shift(NavigationKey::Left));
        let selected = core
            .command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap();
        assert_eq!((selected.anchor, selected.active), (end, 3));
        send(&mut core, view, Key::Char('X'));
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .command_line_snapshot()
                .unwrap()
                .text,
            "aéX"
        );
        send(&mut core, view, shift(NavigationKey::Home));
        send(&mut core, view, shift(NavigationKey::End));
        let selected = core
            .command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap();
        assert_eq!((selected.anchor, selected.active), (4, 4));
        send(&mut core, view, shift(NavigationKey::Home));
        send(&mut core, view, Key::Delete);
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .command_line_snapshot()
                .unwrap()
                .text,
            ""
        );
        assert_eq!(core.document().text(), "untouched");
    }
}
#[test]
fn pending_prefix_counts_and_recording_are_view_local() {
    let (mut core, view) = fixture("one two three four five six seven");
    let other = core.add_view(MockTextMeasurementProvider::new(), 180., 300.);
    for (c, expected, count) in [
        ('"', "\"", None),
        ('a', "\"a", None),
        ('2', "\"a2", Some(2)),
        ('d', "\"a2d", Some(2)),
        ('3', "\"a2d3", Some(6)),
    ] {
        send(&mut core, view, Key::Char(c));
        let state = core.command_state(view).unwrap();
        assert_eq!(state.pending_command_text(), expected);
        assert_eq!(state.active_count(), count);
        assert_eq!(
            core.command_state(other).unwrap().pending_command_text(),
            ""
        );
    }
    text(&mut core, view, "w");
    assert_eq!(core.command_state(view).unwrap().pending_command_text(), "");
    text(&mut core, view, "qq");
    assert_eq!(
        core.command_state(view).unwrap().recording_register(),
        Some('q')
    );
    text(&mut core, view, "iA");
    assert_eq!(
        core.command_state(view).unwrap().recording_register(),
        Some('q')
    );
    send(&mut core, view, Key::Escape);
    text(&mut core, view, "q");
    assert_eq!(core.command_state(view).unwrap().recording_register(), None);
}
#[test]
fn keypad_mappings_are_distinct_and_unmapped_keys_fall_back() {
    let (mut core, view) = fixture("");
    for command in [
        ":inoremap <kEnter> KEYPAD",
        ":inoremap <CR> RETURN",
        ":inoremap <k1> ONE",
        ":inoremap <C-kPlus> PLUS",
    ] {
        text(&mut core, view, command);
        send(&mut core, view, Key::Enter);
    }
    text(&mut core, view, "i");
    for key in [
        Key::Keypad {
            code: 30,
            modifiers: 0,
        },
        Key::Enter,
        Key::Keypad {
            code: 41,
            modifiers: 0,
        },
        Key::Char('1'),
        Key::Keypad {
            code: 26,
            modifiers: 2,
        },
        Key::Keypad {
            code: 42,
            modifiers: 0,
        },
    ] {
        assert_eq!(send(&mut core, view, key), CommandStatus::Complete);
    }
    assert_eq!(core.document().text(), "KEYPADRETURNONE1PLUS2");
}
#[test]
fn recorded_keypad_input_replays_its_original_key_identity() {
    let (mut core, view) = fixture("");
    text(&mut core, view, "qqi");
    send(
        &mut core,
        view,
        Key::Keypad {
            code: 41,
            modifiers: 0,
        },
    );
    send(&mut core, view, Key::Escape);
    text(&mut core, view, "q");
    text(&mut core, view, ":inoremap <k1> M");
    send(&mut core, view, Key::Enter);
    text(&mut core, view, "@q");
    assert_eq!(core.document().text(), "M1");
}
#[test]
fn application_config_preserves_unknowns_and_sparse_styles_with_shared_defaults() {
    use viem_core::application_config::normalize;
    let minimal = normalize(br#"{"version":1}"#).unwrap();
    assert_eq!(minimal["editing"]["textWidth"], 80);
    assert_eq!(minimal["editing"]["smartHome"], true);
    assert_eq!(
        normalize(&serde_json::to_vec(&minimal).unwrap()).unwrap(),
        minimal,
        "normalizing defaults must be idempotent before any settings writes"
    );
    let integral = normalize(br#"{"version":1,"editing":{"textWidth":72.0}}"#).unwrap();
    assert_eq!(integral["editing"]["textWidth"].as_u64(), Some(72));
    let value=normalize(br#"{"version":1,"future":{"x":4},"editing":{"indentation":{"softtabstop":-1,"future":8},"whitespacePresentation":{"visibleWhitespace":{"style":{}}}},"formattingToolbar":{"future":"opaque"}}"#).unwrap();
    assert_eq!(value["editing"]["textWidth"], 80);
    assert_eq!(value["editing"]["indentation"]["softtabstop"], -1);
    assert_eq!(value["editing"]["indentation"]["future"], 8);
    assert_eq!(value["future"]["x"], 4);
    assert!(
        value["editing"]["whitespacePresentation"]["visibleWhitespace"]["style"]
            .get("foreground")
            .is_none_or(|v| v.is_null())
    );
    assert_eq!(value["formattingToolbar"]["plainText"], true);
    for invalid in [
        r#"{"version":true}"#,
        r#"{"version":1,"editing":{"textWidth":false}}"#,
        r#"{"version":1,"editing":{"textWidth":0}}"#,
        r#"{"version":1,"windows":{"showMenu":null}}"#,
        r#"{"version":1,"editing":{"smartHome":1}}"#,
        r#"{"version":1,"recentDocuments":["relative"]}"#,
        r#"{"version":1,"editing":{"indentation":{"tabstop":0}}}"#,
        r#"{"version":1,"code":{"filenameAssociations":[{"pattern":"*","language":"bad/"}]}}"#,
    ] {
        assert!(normalize(invalid.as_bytes()).is_err(), "{invalid}");
    }
    let recent = serde_json::json!({"version":1,"recentDocuments":vec!["/a";11]});
    assert!(normalize(&serde_json::to_vec(&recent).unwrap()).is_err());
    assert!(normalize(br#"{"version":1,"recentDocuments":["C:\\Users\\a.txt","/tmp/a"]}"#).is_ok());
}
#[test]
fn application_config_zoom_defaults_and_explicit_scales_follow_layout_limits() {
    use viem_core::application_config::normalize;
    assert_eq!(normalize(br#"{"version":1}"#).unwrap()["view"]["zoomScale"], 1.0);
    for scale in [0.25, 1.0, 1.03, 5.0] {
        let input = serde_json::json!({"version":1,"view":{"zoomScale":scale,"future":42}});
        let value = normalize(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert_eq!(value["view"]["zoomScale"], scale);
        assert_eq!(value["view"]["future"], 42);
        assert!(viem_core::layout::valid_zoom_scale(scale as f32));
    }
    for scale in ["0", "-1", "0.249999999", "5.000000001", "null", "true", "\"1.25\"", "[]"] {
        let input = format!(r#"{{"version":1,"view":{{"zoomScale":{scale}}}}}"#);
        assert!(normalize(input.as_bytes()).is_err(), "{input}");
    }
}
#[test]
fn parent_candidates_exclude_descendants_and_incompatible_roles() {
    use viem_core::document::{Encoding, Format, StyleId, StyleNamespace};
    let mut document =
        Document::from_bytes(b"> text".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let block = |id: &str, role: &str, parent: &str| serde_json::json!({"id":id,"name":id,"role":role,"based_on":parent,"block":{},"character":{}});
    let value = serde_json::json!({"version":1,"block_styles":[block("First","Paragraph","Paragraph"),block("Second","Paragraph","First"),block("Quote","Quote","Paragraph")]});
    document
        .initialize_style_defaults(&serde_json::to_vec(&value).unwrap())
        .unwrap();
    let sheet = document.projection().style_sheet();
    let first = sheet.local_parent_candidates(StyleNamespace::Block, &StyleId::from("First"));
    assert!(first.contains(&StyleId::from("Paragraph")));
    for id in ["First", "Second", "Quote"] {
        assert!(!first.contains(&StyleId::from(id)), "{id}");
    }
    let quote = sheet.local_parent_candidates(StyleNamespace::Block, &StyleId::from("Quote"));
    assert!(quote.contains(&StyleId::from("Paragraph")));
    assert!(!quote.contains(&StyleId::from("First")));
    assert!(sheet
        .local_parent_candidates(StyleNamespace::Block, &StyleId::from("Paragraph"))
        .is_empty());
}

#[test]
fn keypad_literal_return_and_digits_keep_vim_character_values() {
    let (mut core, view) = fixture("");
    text(&mut core, view, "i");
    send(&mut core, view, Key::Ctrl('v'));
    send(
        &mut core,
        view,
        Key::Keypad {
            code: 30,
            modifiers: 0,
        },
    );
    send(&mut core, view, Key::Ctrl('v'));
    for code in [40, 46, 45] {
        send(&mut core, view, Key::Keypad { code, modifiers: 0 });
    }
    send(&mut core, view, Key::Ctrl('v'));
    send(
        &mut core,
        view,
        Key::Keypad {
            code: 26,
            modifiers: 0,
        },
    );
    send(&mut core, view, Key::Ctrl('v'));
    send(
        &mut core,
        view,
        Key::Keypad {
            code: 20,
            modifiers: 0,
        },
    );
    assert_eq!(core.document().text(), "\rA+<kHome>");
}

#[test]
fn unsuccessful_native_search_preserves_insert_undo_group() {
    let (mut core, view) = fixture("x");
    text(&mut core, view, "/x");
    send(&mut core, view, Key::Enter);
    text(&mut core, view, "xiA");
    assert_eq!(
        send(&mut core, view, action(NativeActionKind::FindNext)),
        CommandStatus::SearchNotFound
    );
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    text(&mut core, view, "B");
    send(&mut core, view, Key::Escape);
    text(&mut core, view, "u");
    assert_eq!(core.document().text(), "");
}

#[test]
fn modified_keypad_enter_preserves_paragraph_and_literal_semantics() {
    use viem_core::document::{Encoding, Format};
    for (modifiers, ordinary) in [(1, Key::ShiftEnter), (4, Key::AltEnter)] {
        for mode in ["i", "R"] {
            let run = |key| {
                let document = Document::from_bytes(
                    b"- AlphaBeta\n- Tail".to_vec(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 180., 300.);
                text(&mut core, view, "5l");
                text(&mut core, view, mode);
                assert_eq!(send(&mut core, view, key), CommandStatus::Complete);
                (
                    core.document().source_bytes(),
                    core.command_state(view).unwrap().mode(),
                )
            };
            assert_eq!(
                run(Key::Keypad {
                    code: 30,
                    modifiers
                }),
                run(ordinary),
                "{modifiers} {mode}"
            );
        }
        let (mut core, view) = fixture("");
        text(&mut core, view, "i");
        send(&mut core, view, Key::Ctrl('v'));
        send(
            &mut core,
            view,
            Key::Keypad {
                code: 30,
                modifiers,
            },
        );
        assert_eq!(
            core.document().text(),
            if modifiers == 1 { "<S-CR>" } else { "<A-CR>" }
        );
    }
}
#[test]
fn repeating_native_selection_resumes_normal_from_visual_modes() {
    let (mut core, view) = fixture("one two\nlast");
    text(&mut core, view, "v");
    send(&mut core, view, action(NativeActionKind::SelectWord));
    send(&mut core, view, action(NativeActionKind::SelectLine));
    send(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.document().text(), "one two\nlast");
}
