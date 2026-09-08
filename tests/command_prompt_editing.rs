use evim_core::command::{CommandLineEditAction, CommandLineEditRequest, InputEvent, Key};
use evim_core::document::Document;
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, ViewId};
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
        "evim-completion-macro-{}-{}",
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
