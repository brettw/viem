use viem_core::command::{CommandLineEditAction, CommandLineEditRequest, InputEvent, Key};
use viem_core::document::Document;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};
fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}
fn edit(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, action: CommandLineEditAction) {
    let request = CommandLineEditRequest {
        document: core.document().id(),
        revision: core.document().revision(),
        expected: core
            .command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap(),
        action,
    };
    core.handle(view, CoreEvent::EditCommandLine(request))
        .unwrap();
}
#[test]
fn prompt_selection_replaces_graphemes_without_changing_source_or_history() {
    let mut core = Core::new(Document::new("untouched"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    key(&mut core, view, Key::Char(':'));
    core.handle(
        view,
        CoreEvent::Input(InputEvent::Text("a👩‍👩‍👧‍👦éz".into())),
    )
    .unwrap();
    let end = "a👩‍👩‍👧‍👦é".len();
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: end,
            active: 1,
        },
    );
    edit(
        &mut core,
        view,
        CommandLineEditAction::Replace {
            range: 1..end,
            text: "B".into(),
        },
    );
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        "aBz"
    );
    assert_eq!(core.document().text(), "untouched");
    assert!(!core.document().is_dirty());
}
#[test]
fn stale_selection_and_interior_unicode_offsets_are_rejected() {
    let mut core = Core::new(Document::new("untouched"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    key(&mut core, view, Key::Char(':'));
    core.handle(view, CoreEvent::Input(InputEvent::Text("éx".into())))
        .unwrap();
    let mut stale = CommandLineEditRequest {
        document: core.document().id(),
        revision: core.document().revision(),
        expected: core
            .command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap(),
        action: CommandLineEditAction::Select {
            anchor: 1,
            active: 3,
        },
    };
    assert!(core
        .handle(view, CoreEvent::EditCommandLine(stale.clone()))
        .is_err());
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: 0,
            active: 2,
        },
    );
    stale.action = CommandLineEditAction::Replace {
        range: 0..2,
        text: "wrong".into(),
    };
    assert!(core
        .handle(view, CoreEvent::EditCommandLine(stale))
        .is_err());
    key(&mut core, view, Key::Backspace);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        "x"
    );
    assert_eq!(core.document().text(), "untouched");
}
#[test]
fn selected_prompt_text_is_deleted_by_delete_and_replaced_by_portable_text_input() {
    let mut core = Core::new(Document::new("untouched"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    key(&mut core, view, Key::Char('/'));
    core.handle(view, CoreEvent::Input(InputEvent::Text("abcdef".into())))
        .unwrap();
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: 1,
            active: 4,
        },
    );
    key(&mut core, view, Key::Delete);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        "aef"
    );
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: 1,
            active: 3,
        },
    );
    core.handle(view, CoreEvent::Input(InputEvent::Text("Z".into())))
        .unwrap();
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        "aZ"
    );
}

#[test]
fn recorded_macro_replays_prompt_replacement_after_pointer_selection() {
    let mut core = Core::new(Document::new("text"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    for value in "qa:".chars() {
        key(&mut core, view, Key::Char(value));
    }
    core.handle(view, CoreEvent::Input(InputEvent::Text("set wrap".into())))
        .unwrap();
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: 8,
            active: 4,
        },
    );
    edit(
        &mut core,
        view,
        CommandLineEditAction::Replace {
            range: 4..8,
            text: "nowrap".into(),
        },
    );
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Char('q'));
    assert!(!core.command_state(view).unwrap().wrap_option());
    for value in ":set wrap".chars() {
        key(&mut core, view, Key::Char(value));
    }
    key(&mut core, view, Key::Enter);
    assert!(core.command_state(view).unwrap().wrap_option());
    key(&mut core, view, Key::Char('@'));
    key(&mut core, view, Key::Char('a'));
    assert!(!core.command_state(view).unwrap().wrap_option());
    assert_eq!(core.document().text(), "text");
}

#[test]
fn recorded_macros_replay_backtab_native_directory_acceptance_and_caret_movement() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "viem-completion-macro-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    for name in ["Alpha", "Zebra"] {
        std::fs::create_dir_all(directory.0.join(name)).unwrap();
    }
    std::fs::write(directory.0.join("Zebra/Child.txt"), b"").unwrap();
    let prefix = format!("e {}/", directory.0.display());
    let expected = format!("{prefix}Zebra/Child.txt");
    let mut core = Core::new(Document::new("untouched"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    for value in "qa:".chars() {
        key(&mut core, view, Key::Char(value));
    }
    core.handle(view, CoreEvent::Input(InputEvent::Text(prefix.clone())))
        .unwrap();
    key(&mut core, view, Key::BackTab);
    let cursor = core
        .command_state(view)
        .unwrap()
        .command_line_snapshot()
        .unwrap()
        .active;
    edit(
        &mut core,
        view,
        CommandLineEditAction::Replace {
            range: cursor..cursor,
            text: "/".into(),
        },
    );
    key(&mut core, view, Key::Tab);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        expected
    );
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Char('q'));
    assert!(core
        .command_state(view)
        .unwrap()
        .register('a')
        .unwrap()
        .text
        .contains("<S-Tab>/<Tab>"));

    // Make replay observable even if it were to stop before executing Ex.
    for value in ":pwd".chars() {
        key(&mut core, view, Key::Char(value));
    }
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Char('@'));
    key(&mut core, view, Key::Char('a'));
    key(&mut core, view, Key::Char(':'));
    key(&mut core, view, Key::Up);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        expected
    );
    assert_eq!(core.document().text(), "untouched");
    assert!(!core.document().is_dirty());

    key(&mut core, view, Key::Escape);
    for value in "qb:".chars() {
        key(&mut core, view, Key::Char(value));
    }
    core.handle(
        view,
        CoreEvent::Input(InputEvent::text(format!("{prefix}ze.keep"))),
    )
    .unwrap();
    let cursor = prefix.len() + 2;
    edit(
        &mut core,
        view,
        CommandLineEditAction::Select {
            anchor: cursor,
            active: cursor,
        },
    );
    key(&mut core, view, Key::Tab);
    let expected = format!("{prefix}Zebra/.keep");
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        expected
    );
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Char('q'));
    for value in ":pwd".chars() {
        key(&mut core, view, Key::Char(value));
    }
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Char('@'));
    key(&mut core, view, Key::Char('b'));
    key(&mut core, view, Key::Char(':'));
    key(&mut core, view, Key::Up);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .command_line_snapshot()
            .unwrap()
            .text,
        expected
    );
    assert_eq!(core.document().text(), "untouched");
    assert!(!core.document().is_dirty());
}


#[test]
fn prompt_arrows_filter_original_prefix_while_control_keys_visit_all_history() {
    for prompt in [':', '/', '?'] {
        let mut core = Core::new(Document::new("alpha beta alpha2"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        let entries = if prompt == ':' { ["set wrap", "set nowrap", "set number"] } else { ["alpha", "beta", "alpha2"] };
        for entry in entries {
            key(&mut core, view, Key::Char(prompt));
            core.handle(view, CoreEvent::Input(InputEvent::text(entry))).unwrap();
            key(&mut core, view, Key::Enter);
        }
        key(&mut core, view, Key::Char(prompt));
        let prefix = if prompt == ':' { "set n" } else { "al" };
        core.handle(view, CoreEvent::Input(InputEvent::text(prefix))).unwrap();
        let text = |core: &Core<MockTextMeasurementProvider>| core.command_state(view).unwrap().command_line_snapshot().unwrap().text;
        key(&mut core, view, Key::Up);
        assert_eq!(text(&core), entries[2]);
        key(&mut core, view, Key::Up);
        assert_eq!(text(&core), if prompt == ':' { entries[1] } else { entries[0] });
        key(&mut core, view, Key::Up);
        assert_eq!(text(&core), if prompt == ':' { entries[1] } else { entries[0] });
        key(&mut core, view, Key::Down);
        assert_eq!(text(&core), entries[2]);
        key(&mut core, view, Key::Down);
        assert_eq!(text(&core), prefix);
        key(&mut core, view, Key::Ctrl('p'));
        key(&mut core, view, Key::Ctrl('p'));
        assert_eq!(text(&core), entries[1]);
        key(&mut core, view, Key::Ctrl('n'));
        assert_eq!(text(&core), entries[2]);
        key(&mut core, view, Key::Down);
        assert_eq!(text(&core), prefix);
        core.handle(view, CoreEvent::Input(InputEvent::text("unmatched"))).unwrap();
        key(&mut core, view, Key::Up);
        assert_eq!(text(&core), format!("{prefix}unmatched"));
        assert_eq!(core.document().text(), "alpha beta alpha2");
        assert!(!core.document().is_dirty());
    }
}

#[test]
fn history_prefix_stops_at_caret_and_editing_starts_a_new_prefix() {
    let mut core = Core::new(Document::new("éclair élève other"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    for entry in ["éclair", "élève", "other"] {
        key(&mut core, view, Key::Char('/'));
        core.handle(view, CoreEvent::Input(InputEvent::text(entry))).unwrap();
        key(&mut core, view, Key::Enter);
    }
    key(&mut core, view, Key::Char('/'));
    core.handle(view, CoreEvent::Input(InputEvent::text("é draft"))).unwrap();
    edit(&mut core, view, CommandLineEditAction::Select { anchor: 2, active: 2 });
    key(&mut core, view, Key::Up);
    assert_eq!(core.command_state(view).unwrap().command_line_snapshot().unwrap().text, "élève");
    key(&mut core, view, Key::Down);
    assert_eq!(core.command_state(view).unwrap().command_line_snapshot().unwrap().text, "é draft");
    key(&mut core, view, Key::Ctrl('u'));
    core.handle(view, CoreEvent::Input(InputEvent::text("oth"))).unwrap();
    key(&mut core, view, Key::Up);
    assert_eq!(core.command_state(view).unwrap().command_line_snapshot().unwrap().text, "other");
}

#[test]
fn prompt_word_register_uses_iskeyword_and_preserves_graphemes() {
    for (setting, word, expected) in [
        ("iskeyword+=-", "one-two", "one-two"),
        ("iskeyword+=-", "e\u{301}lan-deux", "e\u{301}lan-deux"),
        ("iskeyword-=_", "one_two", "one"),
    ] {
        let mut core = Core::new(Document::new(word));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        key(&mut core, view, Key::Char(':'));
        core.handle(view, CoreEvent::Input(InputEvent::text(format!("set {setting}")))).unwrap();
        key(&mut core, view, Key::Enter);
        key(&mut core, view, Key::Char(':'));
        key(&mut core, view, Key::Ctrl('r'));
        key(&mut core, view, Key::Ctrl('w'));
        assert_eq!(core.command_state(view).unwrap().command_line_snapshot().unwrap().text, expected);
        assert_eq!(core.document().text(), word);
        assert!(!core.document().is_dirty());
    }
}
