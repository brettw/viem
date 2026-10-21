use viem_core::command::clipboard::ClipboardCommandContext;
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{
    ArtifactBinding, ArtifactIdentity, ArtifactPath, Encoding, Format, LineEndingOpenPolicy,
    LoadedArtifact,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};
type Editor = Core<MockTextMeasurementProvider>;
fn editor(text: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    (core, view)
}
fn key(core: &mut Editor, view: ViewId, key: Key) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap()
}
fn keys(core: &mut Editor, view: ViewId, text: &str) -> CoreOutcome {
    text.chars()
        .map(|ch| key(core, view, Key::Char(ch)))
        .last()
        .unwrap()
}
fn ex(core: &mut Editor, view: ViewId, command: &str) -> CoreOutcome {
    core.stage_command_line(view, command).unwrap();
    key(core, view, Key::Enter)
}
fn cursor(core: &Editor, view: ViewId) -> usize {
    core.command_state(view).unwrap().cursor()
}
#[test]
fn search_command_and_filename_registers_share_all_read_paths() {
    let (mut core, view) = editor("one two");
    ex(&mut core, view, "let @/ = 'two'");
    keys(&mut core, view, "\"/2p");
    assert_eq!(core.document().text(), "otwotwone two");
    keys(&mut core, view, "u");
    ex(&mut core, view, "put /");
    assert_eq!(core.document().text(), "one two\ntwo");
    keys(&mut core, view, "u");
    ex(&mut core, view, "echo 'hello'");
    keys(&mut core, view, "\":p");
    assert!(core.document().text().contains("echo 'hello'"));
    let document = Document::from_loaded_artifact(
        LoadedArtifact::new(
            ArtifactBinding::new(
                ArtifactPath::from_utf8("/tmp/notes.txt"),
                ArtifactIdentity::new(b"notes".to_vec()),
            ),
            b"text".to_vec(),
        ),
        Encoding::Utf8,
        Format::PlainText,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    let output = ex(&mut core, view, "echo @% getreg('%')");
    assert!(
        format!("{output:?}").contains("/tmp/notes.txt /tmp/notes.txt"),
        "{output:?}"
    );
    core.set_alternate_artifact(view, Some(ArtifactPath::from_utf8("/tmp/previous.txt")))
        .unwrap();
    assert!(format!("{:?}", ex(&mut core, view, "echo @#")).contains("/tmp/previous.txt"));
    keys(&mut core, view, "\"#p");
    assert!(core.document().text().contains("/tmp/previous.txt"));
}
#[test]
fn expression_register_prompts_put_insert_and_nest_without_losing_outer_prompt() {
    let (mut core, view) = editor("ab");
    keys(&mut core, view, "2\"=1 + 2");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "p");
    assert_eq!(core.document().text(), "a33b");
    keys(&mut core, view, "u");
    keys(&mut core, view, "i");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "='é'");
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "éab");
    ex(&mut core, view, "let g:answer = 6");
    keys(&mut core, view, ":echo ");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "=g:answer * 7");
    key(&mut core, view, Key::Enter);
    assert_eq!(
        core.command_state(view).unwrap().command_line(),
        Some("echo 42")
    );
    assert!(format!("{:?}", key(&mut core, view, Key::Enter)).contains("42"));
    keys(&mut core, view, ":echo 'keep'");
    key(&mut core, view, Key::Ctrl('r'));
    keys(&mut core, view, "=bad(");
    key(&mut core, view, Key::Escape);
    assert_eq!(
        core.command_state(view).unwrap().command_line(),
        Some("echo 'keep'")
    );
    key(&mut core, view, Key::Escape);
    let before = core.document().source_bytes();
    keys(&mut core, view, "\"=1 /");
    let failed = key(&mut core, view, Key::Enter);
    assert!(matches!(
        failed.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    assert_eq!(core.document().source_bytes(), before);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
}
#[test]
fn affected_marks_last_insert_and_previous_position_are_usable_and_rebased() {
    let (mut core, view) = editor("one\n  two\nthree");
    keys(&mut core, view, "j0y$");
    keys(&mut core, view, "gg`]");
    assert_eq!(cursor(&core, view), 8);
    keys(&mut core, view, "`[");
    assert_eq!(cursor(&core, view), 4);
    keys(&mut core, view, "ggG``");
    assert_eq!(cursor(&core, view), 0);
    keys(&mut core, view, "``");
    assert_eq!(cursor(&core, view), 10);
    keys(&mut core, view, "ggiXY");
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "G`^");
    assert_eq!(cursor(&core, view), 2);
    keys(&mut core, view, "`[");
    assert_eq!(cursor(&core, view), 0);
    keys(&mut core, view, "`]");
    assert_eq!(cursor(&core, view), 1);
    keys(&mut core, view, "`.");
    assert_eq!(cursor(&core, view), 1);
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    keys(&mut core, other, "`]");
    assert_eq!(cursor(&core, other), 1);
    keys(&mut core, view, "ggd`]");
    assert_eq!(core.document().text(), "Yone\n  two\nthree");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "XYone\n  two\nthree");
}
#[test]
fn read_only_register_destinations_fail_without_changing_source_or_unnamed() {
    for name in ['/', ':', '#', '~', '%'] {
        let (mut core, view) = editor("one\ntwo");
        keys(&mut core, view, "yy");
        let revision = core.document().revision();
        let before = core.command_state(view).unwrap().register('"').cloned();
        let result = keys(&mut core, view, &format!("\"{name}dd"));
        assert!(
            matches!(
                result.command.unwrap().status,
                CommandStatus::RegisterWriteError(_)
            ),
            "{name}"
        );
        assert_eq!(core.document().revision(), revision);
        assert_eq!(
            core.command_state(view).unwrap().register('"'),
            before.as_ref()
        );
    }
}
#[test]
fn drop_and_alternate_registers_do_not_share_view_context() {
    let (mut core, a) = editor("ab");
    let b = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    core.set_alternate_artifact(a, Some(ArtifactPath::from_utf8("first")))
        .unwrap();
    core.set_alternate_artifact(b, Some(ArtifactPath::from_utf8("second")))
        .unwrap();
    core.set_drop_register(
        a,
        viem_core::command::RegisterValue::characterwise("dropped"),
    )
    .unwrap();
    let read = |view, name| {
        core.command_state(view)
            .unwrap()
            .register_with_context(core.document(), &ClipboardCommandContext::new(), name)
            .unwrap()
            .unwrap()
            .text
    };
    assert_eq!(read(a, '#'), "first");
    assert_eq!(read(b, '#'), "second");
    assert_eq!(read(b, '~'), "dropped");
}
#[test]
fn expression_lists_ex_put_repeat_history_and_failed_textlock() {
    let (mut core, view) = editor("one");
    let output = ex(&mut core, view, "put =['two', 'three']");
    assert!(matches!(
        output.command.unwrap().status,
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "one\ntwo\nthree");
    keys(&mut core, view, "u");
    ex(&mut core, view, "let @= = '21 * 2'");
    assert!(format!(
        "{:?}",
        ex(&mut core, view, "echo getreg('=', 1) getreg('=')")
    )
    .contains("21 * 2 42"));
    keys(&mut core, view, "\"=");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "p.");
    assert_eq!(core.document().text(), "o4242ne");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "o42ne");
    let before = core.document().source_bytes();
    keys(&mut core, view, "\"=setline(1, 'bad')");
    let outcome = key(&mut core, view, Key::Enter);
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    assert_eq!(core.document().source_bytes(), before);
}
#[test]
fn expression_errors_clear_prefix_and_recursive_reads_are_bounded() {
    let (mut core, view) = editor("abcdef");
    keys(&mut core, view, "3\"=unknown(");
    assert!(matches!(
        key(&mut core, view, Key::Enter).command.unwrap().status,
        CommandStatus::Error(_)
    ));
    keys(&mut core, view, "x");
    assert_eq!(
        core.document().text(),
        "bcdef",
        "failed expression must not leave count three pending"
    );
    ex(&mut core, view, "let @= = \"getregtype('==')\"");
    keys(&mut core, view, "\"=");
    let outcome = key(&mut core, view, Key::Enter);
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    keys(&mut core, view, "2\"=1");
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "x");
    assert_eq!(core.document().text(), "cdef");
}
#[test]
fn expression_register_macro_and_normal_execution_keep_one_undo_unit() {
    let (mut core, view) = editor("abc");
    keys(&mut core, view, "qq\"=7");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "pq@q");
    assert_eq!(core.document().text(), "a77bc");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a7bc");
    ex(&mut core, view, "normal! \"/p"); // Empty search register is a no-op.
    ex(&mut core, view, "let @/ = 'search'");
    ex(&mut core, view, "normal! \"/p");
    assert_eq!(core.document().text(), "asearch7bc");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "a7bc");
}
#[test]
fn visual_mark_setting_and_script_positions_use_current_locations() {
    let (mut core, view) = editor("one\n  two");
    keys(&mut core, view, "jllvmA");
    key(&mut core, view, Key::Escape);
    let output = ex(&mut core, view, "echo getpos(\"'A\") col(\"'A\")");
    assert!(
        format!("{output:?}").contains("[0, 2, 3, 0] 3"),
        "{output:?}"
    );
    ex(&mut core, view, "delmarks A");
    let output = ex(&mut core, view, "echo getpos(\"'A\")");
    assert!(format!("{output:?}").contains("[0, 0, 0, 0]"), "{output:?}");
}
#[test]
fn expression_put_inside_global_uses_the_same_evaluator_and_undo_unit() {
    let (mut core, view) = editor("one\ntwo");
    let outcome = ex(&mut core, view, "g/o/put =6 * 7");
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Complete
    ));
    assert_eq!(core.document().text(), "one\n42\ntwo\n42");
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "one\ntwo");
}
#[test]
fn global_deletion_rebases_the_new_previous_jump_mark() {
    for format in [Format::PlainText, Format::Code, Format::Markdown] {
        let document = Document::from_bytes(
            b"First paragraph.\nSecond line.\n\nLast line.".to_vec(),
            Encoding::Utf8,
            format,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        keys(&mut core, view, "G");
        let before = core.document().source_bytes();
        let result = ex(&mut core, view, "g/a/d");
        assert!(matches!(
            result.command.unwrap().status,
            CommandStatus::Complete
        ));
        keys(&mut core, view, "``");
        assert!(cursor(&core, view) <= core.document().text().len());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), before);
    }
    let mut document = Document::new("First paragraph.\nSecond line.\n\nLast line.");
    let mut commands = viem_core::command::CommandInterpreter::new();
    for ch in "G:g/a/d".chars() {
        commands.handle(&mut document, InputEvent::key(ch)).unwrap();
    }
    let output = commands
        .handle(&mut document, InputEvent::Key(Key::Enter))
        .unwrap();
    assert!(matches!(output.status, CommandStatus::Complete));
    for ch in "``".chars() {
        commands.handle(&mut document, InputEvent::key(ch)).unwrap();
    }
    assert!(commands.cursor() <= document.text().len());
}
