//! Deep black-box coverage for command families whose declarative matrix only
//! proves that their key grammar is accepted.

use viem_core::command::{CommandStatus, InputEvent, Key, Mode, RegisterKind};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

fn core_for(document: Document) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    (core, view)
}

fn plain(text: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    core_for(Document::new(text))
}

fn markdown(source: &[u8]) -> (Core<MockTextMeasurementProvider>, ViewId) {
    core_for(
        Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown)
            .expect("Markdown fixture must project"),
    )
}

fn input(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    event: InputEvent,
) -> CommandStatus {
    core.handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| panic!("command dispatch for {event:?} must succeed: {error:?}"))
        .command
        .expect("input must produce a command outcome")
        .status
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) -> CommandStatus {
    input(core, view, InputEvent::Key(key))
}

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, keys: &str) {
    for character in keys.chars() {
        let status = key(core, view, Key::Char(character));
        assert!(
            !matches!(
                status,
                CommandStatus::Unsupported(_)
                    | CommandStatus::Error(_)
                    | CommandStatus::CountError(_)
                    | CommandStatus::RegisterReadError(_)
                    | CommandStatus::RegisterWriteError(_)
                    | CommandStatus::ExError(_)
                    | CommandStatus::VisualBlockError(_)
            ),
            "{character:?} failed as {status:?}"
        );
    }
}

fn assert_register(
    core: &Core<MockTextMeasurementProvider>,
    view: ViewId,
    name: char,
    kind: RegisterKind,
    text: &str,
) {
    let register = core
        .command_state(view)
        .and_then(|commands| commands.register(name))
        .unwrap_or_else(|| panic!("register {name:?} must be populated"));
    assert_eq!(register.kind, kind, "register {name:?} kind");
    assert_eq!(register.text, text, "register {name:?} text");
}

fn assert_register_absent(core: &Core<MockTextMeasurementProvider>, view: ViewId, name: char) {
    assert!(
        core.command_state(view)
            .and_then(|commands| commands.register(name))
            .is_none(),
        "register {name:?} must remain empty"
    );
}

#[test]
fn search_and_substitute_cross_adjacent_markdown_style_boundaries() {
    let original = b"**ab**_cd_ tail";
    let (mut core, view) = markdown(original);
    assert_eq!(core.document().text(), "abcd tail");

    keys(&mut core, view, "/bc");
    assert_eq!(key(&mut core, view, Key::Enter), CommandStatus::Complete);
    assert_eq!(core.command_state(view).unwrap().cursor(), 1);
    assert_eq!(core.document().source_bytes(), original);
    assert_eq!(core.document().revision().0, 0);

    keys(&mut core, view, ":%s/bc/XY/");
    assert_eq!(key(&mut core, view, Key::Enter), CommandStatus::Complete);
    assert_eq!(core.document().text(), "aXYd tail");
    assert_eq!(core.document().source_bytes(), b"**aX**_Yd_ tail");

    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "abcd tail");
    assert_eq!(core.document().source_bytes(), original);
}

#[test]
fn visual_exact_mark_keeps_backward_direction_and_drives_named_delete() {
    let original = "one\ntwo\nthree";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "wmaGv`a");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualCharacter);
    assert_eq!(commands.visual_anchor(), Some(8));
    assert_eq!(commands.cursor(), 4);

    keys(&mut core, view, "\"bd");
    assert_eq!(core.document().text(), "one\nhree");
    assert_register(&core, view, 'b', RegisterKind::Characterwise, "two\nt");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "two\nt");

    keys(&mut core, view, "gv");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualCharacter);
    assert_eq!(commands.visual_anchor(), Some(4));
    assert_eq!(
        commands.cursor(),
        4,
        "cross-line deleted endpoints collapse"
    );
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn visual_line_mark_jump_is_linewise_and_named_yank_is_register_exact() {
    let original = "one\ntwo\nthree";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "jmaGV'a");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualLine);
    assert_eq!(commands.visual_anchor(), Some(8));
    assert_eq!(commands.cursor(), 4);

    let revision = core.document().revision();
    keys(&mut core, view, "\"cy");
    assert_eq!(core.document().revision(), revision, "yank is not an edit");
    assert_eq!(core.document().text(), original);
    assert_register(&core, view, 'c', RegisterKind::Linewise, "two\nthree\n");
}

#[test]
fn visual_block_exact_mark_preserves_rectangle_direction_and_undo() {
    let original = "one\ntwo\nthree";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "lmaGl");
    key(&mut core, view, Key::Ctrl('v'));
    keys(&mut core, view, "`a");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualBlock);
    let block = commands
        .visual_block()
        .expect("block selection must resolve");
    assert_eq!(block.anchor.text_offset, 9);
    assert_eq!(block.active.text_offset, 1);

    keys(&mut core, view, "\"dd");
    assert_eq!(core.document().text(), "oe\nto\ntree");
    assert_register(&core, view, 'd', RegisterKind::Blockwise, "n\nw\nh");
    assert_register(&core, view, '1', RegisterKind::Blockwise, "n\nw\nh");

    keys(&mut core, view, "gv");
    let block = core
        .command_state(view)
        .unwrap()
        .visual_block()
        .expect("remembered block must rebind after deletion");
    assert_eq!(block.anchor.text_offset, 7);
    assert_eq!(block.active.text_offset, 1);
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
}

#[test]
fn visual_jump_list_navigation_preserves_direction_in_all_visual_modes() {
    for (entry, mode, expected_kind, expected_register, expected_after) in [
        (
            Key::Char('v'),
            Mode::VisualCharacter,
            RegisterKind::Characterwise,
            "one\ntwo\nt",
            "hree",
        ),
        (
            Key::Char('V'),
            Mode::VisualLine,
            RegisterKind::Linewise,
            "one\ntwo\nthree\n",
            "",
        ),
        (
            Key::Ctrl('v'),
            Mode::VisualBlock,
            RegisterKind::Blockwise,
            "o\nt\nt",
            "ne\nwo\nhree",
        ),
    ] {
        let original = "one\ntwo\nthree";
        let (mut core, view) = plain(original);
        keys(&mut core, view, "G");
        key(&mut core, view, entry);

        key(&mut core, view, Key::Ctrl('o'));
        let commands = core.command_state(view).unwrap();
        assert_eq!(commands.mode(), mode);
        assert_eq!(commands.cursor(), 0, "Ctrl-O moves the active endpoint");
        match mode {
            Mode::VisualBlock => {
                let block = commands.visual_block().unwrap();
                assert_eq!(block.anchor.text_offset, 8);
                assert_eq!(block.active.text_offset, 0);
            }
            _ => assert_eq!(commands.visual_anchor(), Some(8)),
        }

        key(&mut core, view, Key::Ctrl('i'));
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            8,
            "Ctrl-I restores the forward jump without leaving Visual mode"
        );
        assert_eq!(core.command_state(view).unwrap().mode(), mode);

        key(&mut core, view, Key::Ctrl('o'));
        keys(&mut core, view, "\"ed");
        assert_eq!(core.document().text(), expected_after, "{mode:?} delete");
        assert_register(&core, view, 'e', expected_kind, expected_register);
        assert_register(&core, view, '1', expected_kind, expected_register);

        keys(&mut core, view, "gv");
        let commands = core.command_state(view).unwrap();
        match mode {
            Mode::VisualBlock => {
                let block = commands.visual_block().unwrap();
                assert_eq!(block.anchor.text_offset, 6);
                assert_eq!(block.active.text_offset, 0);
            }
            _ => {
                assert_eq!(commands.visual_anchor(), Some(0));
                assert_eq!(commands.cursor(), 0);
            }
        }
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), original, "{mode:?} undo");
    }
}

#[test]
fn counted_visual_sentence_object_updates_named_register_and_undo_atomically() {
    let original = "One.  Two? Three!";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "v2is\"ad");
    assert_eq!(core.document().text(), " Three!");
    assert_register(&core, view, 'a', RegisterKind::Characterwise, "One.  Two?");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "One.  Two?");
    assert_register_absent(&core, view, '-');

    keys(&mut core, view, "gv");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.visual_anchor(), Some(0));
    assert_eq!(commands.cursor(), 6);
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn counted_visual_paragraph_object_yanks_exact_semantic_breaks() {
    let original = "first\nline\n\n  \nsecond\n\nthird";
    let expected = "first\nline\n\n  \nsecond";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "v2ip\"by");
    assert_eq!(core.document().text(), original);
    assert_register(&core, view, 'b', RegisterKind::Characterwise, expected);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('b')
            .unwrap()
            .hard_break_offsets(),
        &[5, 10, 11, 14]
    );
    assert_eq!(core.document().revision().0, 0, "yank creates no history");
}

#[test]
fn counted_visual_quote_object_uses_vim_quote_count_and_named_yank() {
    let original = "aa  \"one\"  bb";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "/one");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "v2i\"\"cy");
    assert_eq!(core.document().text(), original);
    assert_register(&core, view, 'c', RegisterKind::Characterwise, "\"one\"");
}

#[test]
fn counted_visual_pair_object_climbs_outward_and_deletes_as_one_unit() {
    let original = "outer(a + (b * c)) tail";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "/b");
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "v2a(\"dd");
    assert_eq!(core.document().text(), "outer tail");
    assert_register(
        &core,
        view,
        'd',
        RegisterKind::Characterwise,
        "(a + (b * c))",
    );
    assert_register(
        &core,
        view,
        '1',
        RegisterKind::Characterwise,
        "(a + (b * c))",
    );

    keys(&mut core, view, "gv");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.visual_anchor(), Some(5));
    assert_eq!(commands.cursor(), 9);
    key(&mut core, view, Key::Escape);
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
}

#[test]
fn same_line_backward_visual_delete_retains_vim_shape_and_rotates_explicit_register() {
    let original = "abcdef";
    let (mut core, view) = plain(original);

    keys(&mut core, view, "3lv2h\"fd");
    assert_eq!(core.document().text(), "aef");
    assert_register(&core, view, 'f', RegisterKind::Characterwise, "bcd");
    assert_register(&core, view, '1', RegisterKind::Characterwise, "bcd");
    assert_register_absent(&core, view, '-');

    keys(&mut core, view, "gv");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualCharacter);
    assert_eq!(commands.visual_anchor(), Some(2));
    assert_eq!(commands.cursor(), 1);
    key(&mut core, view, Key::Escape);

    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), original);
}

#[test]
fn visual_put_and_unicode_replace_remember_the_produced_text_for_gv() {
    let (mut core, view) = plain("abcdef");
    keys(&mut core, view, "vly2lv2lp");
    assert_eq!(core.document().text(), "ababf");
    keys(&mut core, view, "gv");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.visual_anchor(), Some(2));
    assert_eq!(commands.cursor(), 3);

    let (mut core, view) = plain("😀bc");
    keys(&mut core, view, "vlrXgv");
    assert_eq!(core.document().text(), "XXc");
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.visual_anchor(), Some(0));
    assert_eq!(commands.cursor(), 1);
}
