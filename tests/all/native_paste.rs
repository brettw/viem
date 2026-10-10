//! Native Paste follows Vim's GUI `"+gP` and selection paste helper, while
//! ordinary `p` and Insert Ctrl-R retain their register-command semantics.
use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{
    CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key, Mode, RegisterKind,
    RegisterReadError, RegisterValue,
};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, FileFormat, FontSlant, Format, SemanticInlineStyle,
    StyleApplication, StyleNamespace,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

fn plain(source: &str) -> Document {
    Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::PlainText,
    )
    .unwrap()
}

fn clipboard(content: ClipboardContent) -> ClipboardCommandContext {
    ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(17),
        content,
    ))
}

fn external(text: &str) -> ClipboardCommandContext {
    clipboard(ClipboardContent::from_plain_text(text))
}

fn accepted(output: CommandOutput) -> CommandOutput {
    assert!(
        matches!(
            output.status,
            CommandStatus::Complete | CommandStatus::Pending
        ),
        "{:?}",
        output.status
    );
    output
}

fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) -> CommandOutput {
    accepted(commands.handle(document, InputEvent::Key(key)).unwrap())
}

fn keys(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
    for character in text.chars() {
        key(commands, document, Key::Char(character));
    }
}

fn text(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
    accepted(commands.handle(document, InputEvent::text(text)).unwrap());
}

fn paste(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    context: &ClipboardCommandContext,
) -> CommandOutput {
    let before = context.clone();
    let output = accepted(
        commands
            .handle_with_clipboard_context(document, InputEvent::Key(Key::PasteClipboard), context)
            .unwrap(),
    );
    assert_eq!(context, &before);
    assert!(output.clipboard_writes.is_empty());
    output
}

fn exact_history(
    commands: &mut CommandInterpreter,
    document: &mut Document,
    before: &[u8],
    after: &[u8],
) {
    if matches!(commands.mode(), Mode::Insert | Mode::Replace) {
        key(commands, document, Key::Escape);
    }
    key(commands, document, Key::Char('u'));
    assert_eq!(document.source_bytes(), before);
    key(commands, document, Key::Ctrl('r'));
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn normal_paste_precedes_the_current_character_and_follows_inserted_graphemes() {
    for (source, at, payload, expected, caret) in [
        ("abc", 0, "XY", "XYabc", 2),
        ("abc", 2, "XY", "abXYc", 4),
        ("abc", 1, "X\nY", "aX\nYbc", 4),
        ("é👩‍💻tail", 2, "e\u{301}", "ée\u{301}👩‍💻tail", 5),
        ("", 0, "XY", "XY", 1),
    ] {
        let mut document = plain(source);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, at));
        paste(&mut commands, &mut document, &external(payload));
        assert_eq!(document.text(), expected);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), caret);
        exact_history(
            &mut commands,
            &mut document,
            source.as_bytes(),
            expected.as_bytes(),
        );
    }
}

#[test]
fn linewise_paste_precedes_empty_and_nonempty_lines_without_a_leading_blank() {
    for (source, at, payload, expected, caret) in [
        ("abc\ndef", 1, "X\nY\n", "X\nY\nabc\ndef", 4),
        ("\nabc", 0, "X\nY\n", "X\nY\n\nabc", 4),
        ("", 0, "X\nY\n", "X\nY\n", 4),
        ("abc", 1, "\n", "\nabc", 1),
        ("", 0, "\n", "\n", 1),
        ("abc", 1, "\n\n", "\n\nabc", 2),
        ("abc", 1, "X\r\nY\r", "X\nY\nabc", 4),
    ] {
        let mut document = plain(source);
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, at));
        paste(&mut commands, &mut document, &external(payload));
        assert_eq!(document.text(), expected, "{source:?}/{payload:?}");
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), caret);
        exact_history(
            &mut commands,
            &mut document,
            source.as_bytes(),
            expected.as_bytes(),
        );
    }
}

#[test]
fn private_character_shape_wins_over_a_trailing_newline() {
    let context = clipboard(ClipboardContent::from_register(
        RegisterValue::characterwise("X\n"),
    ));
    let mut document = plain("abc");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "l");
    paste(&mut commands, &mut document, &context);
    assert_eq!(document.text(), "aX\nbc");
    assert_eq!(commands.cursor(), 3);
    exact_history(&mut commands, &mut document, b"abc", b"aX\nbc");
}

#[test]
fn insert_and_replace_native_paste_insert_and_resume_at_the_following_boundary() {
    for (entry, payload, expected, caret, mode) in [
        ('i', "XY", "aXYbc", 3, Mode::Insert),
        ('R', "XY", "aXYbc", 3, Mode::Replace),
        ('i', "X\nY\n", "X\nY\nabc", 4, Mode::Insert),
        ('R', "X\nY\n", "X\nY\nabc", 4, Mode::Replace),
    ] {
        let mut document = plain("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "l");
        key(&mut commands, &mut document, Key::Char(entry));
        paste(&mut commands, &mut document, &external(payload));
        assert_eq!(document.text(), expected);
        assert_eq!(commands.cursor(), caret);
        assert_eq!(commands.mode(), mode);
        exact_history(&mut commands, &mut document, b"abc", expected.as_bytes());
    }

    let mut document = plain("abc");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "A");
    paste(&mut commands, &mut document, &external("XY"));
    assert_eq!(document.text(), "abcXY");
    assert_eq!(commands.cursor(), 5);
    assert_eq!(commands.mode(), Mode::Insert);
    text(&mut commands, &mut document, "Z");
    assert_eq!(document.text(), "abcXYZ");
}

#[test]
fn visual_paste_uses_change_then_put_and_preserves_its_deleted_register() {
    for (source, selection, payload, expected, caret, removed) in [
        ("abcdef", "lvl", "XY", "aXYdef", 3, "bc"),
        ("abcdef", "lvl", "X\nY", "aX\nYdef", 4, "bc"),
        ("abcdef", "lvl", "X\nY\n", "X\nY\nadef", 4, "bc"),
        ("abcdef", "4lv$", "XY", "abcdXY", 5, "ef"),
        ("abc\ndef\nghi", "V", "XY", "XY\ndef\nghi", 1, "abc\n"),
        (
            "abc\ndef\nghi",
            "V",
            "X\nY\n",
            "X\nY\n\ndef\nghi",
            4,
            "abc\n",
        ),
        ("abc\ndef", "VG", "X\nY\n", "X\nY\n", 4, "abc\ndef\n"),
    ] {
        let mut document = plain(source);
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, selection);
        paste(&mut commands, &mut document, &external(payload));
        assert_eq!(document.text(), expected, "{selection}/{payload:?}");
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), caret);
        assert_eq!(commands.register('-').unwrap().text, removed);
        exact_history(
            &mut commands,
            &mut document,
            source.as_bytes(),
            expected.as_bytes(),
        );
        assert_eq!(commands.register('-').unwrap().text, removed);
    }
}

#[test]
fn native_selection_replacement_is_half_open_in_both_directions() {
    for reverse in [false, true] {
        let mut document = plain("a👩‍💻éb");
        let mut commands = CommandInterpreter::new();
        let endpoints = if reverse { [14, 1] } else { [1, 14] };
        assert!(commands.set_cursor_from_pointer(
            &document,
            endpoints[0],
            BoundaryAffinity::Downstream,
            false
        ));
        assert!(commands.set_cursor_from_pointer(
            &document,
            endpoints[1],
            BoundaryAffinity::Upstream,
            true
        ));
        assert!(commands.is_native_selection());
        paste(&mut commands, &mut document, &external("XY"));
        assert_eq!(document.text(), "aXYb");
        assert_eq!(commands.cursor(), 3);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.register('-').unwrap().text, "👩‍💻é");
        exact_history(&mut commands, &mut document, "a👩‍💻éb".as_bytes(), b"aXYb");
    }
}

#[test]
fn normal_dot_repeats_native_put_and_visual_dot_repeats_only_the_change_extent() {
    let context = external("XY");
    let mut document = plain("abc");
    let mut commands = CommandInterpreter::new();
    paste(&mut commands, &mut document, &context);
    accepted(
        commands
            .handle_with_clipboard_context(&mut document, InputEvent::key('.'), &context)
            .unwrap(),
    );
    assert_eq!(document.text(), "XYXYabc");
    assert_eq!(commands.cursor(), 4);
    key(&mut commands, &mut document, Key::Char('u'));
    assert_eq!(document.text(), "XYabc");

    let mut document = plain("abcdef");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "lvl");
    paste(&mut commands, &mut document, &context);
    key(&mut commands, &mut document, Key::Char('.'));
    assert_eq!(document.text(), "aXYf");
    assert_eq!(commands.register('-').unwrap().text, "de");
    key(&mut commands, &mut document, Key::Char('u'));
    assert_eq!(document.text(), "aXYdef");
    key(&mut commands, &mut document, Key::Char('u'));
    assert_eq!(document.text(), "abcdef");
}

#[test]
fn visual_line_native_paste_dot_repeats_change_and_retains_the_empty_line() {
    let mut document = plain("abc\ndef\nghi");
    let mut commands = CommandInterpreter::new();
    key(&mut commands, &mut document, Key::Char('V'));
    paste(&mut commands, &mut document, &external("XY"));
    assert_eq!(document.text(), "XY\ndef\nghi");
    key(&mut commands, &mut document, Key::Char('.'));
    assert_eq!(document.source_bytes(), b"\ndef\nghi");
    assert_eq!(commands.mode(), Mode::Normal);
    assert_eq!(commands.cursor(), 0);
    assert_eq!(commands.register('-').unwrap().text, "XY\n");
    for expected in [b"XY\ndef\nghi".as_slice(), b"abc\ndef\nghi".as_slice()] {
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.source_bytes(), expected);
    }
    for expected in [b"XY\ndef\nghi".as_slice(), b"\ndef\nghi".as_slice()] {
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(document.source_bytes(), expected);
    }
}

#[test]
fn native_paste_has_undo_breaks_on_both_sides_of_typing() {
    let mut document = plain("abc");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "i");
    text(&mut commands, &mut document, "T");
    paste(&mut commands, &mut document, &external("XY"));
    text(&mut commands, &mut document, "Z");
    key(&mut commands, &mut document, Key::Escape);
    assert_eq!(document.text(), "TXYZabc");
    for expected in ["TXYabc", "Tabc", "abc"] {
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
    for expected in ["Tabc", "TXYabc", "TXYZabc"] {
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn native_paste_completes_one_normal_command_and_resumes_insert_after_ctrl_o() {
    let mut document = plain("abc");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "i");
    text(&mut commands, &mut document, "T");
    key(&mut commands, &mut document, Key::Ctrl('o'));
    assert_eq!(commands.mode(), Mode::Normal);
    paste(&mut commands, &mut document, &external("XY"));
    assert_eq!(document.text(), "TXYabc");
    assert_eq!(commands.mode(), Mode::Insert);
    assert_eq!(commands.cursor(), 3);
    text(&mut commands, &mut document, "Z");
    assert_eq!(document.text(), "TXYZabc");
    key(&mut commands, &mut document, Key::Escape);
    for expected in ["TXYabc", "Tabc", "abc"] {
        key(&mut commands, &mut document, Key::Char('u'));
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn native_paste_commits_pending_rich_choices_on_the_actual_caret_affinity() {
    for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
        let source = b"a**b**c";
        let document =
            Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core_key(&mut core, view, Key::Char('i'));
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 1,
                affinity,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style: SemanticInlineStyle::Emphasis,
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source);
        core_paste(&mut core, view, &external("X"));
        assert_eq!(core.document().text(), "aXbc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        let inserted = core.document().source_bytes();
        let appearance =
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), 1, false)
                .unwrap();
        assert_eq!(appearance.slant, FontSlant::Italic);
        assert_eq!(appearance.bold, affinity == BoundaryAffinity::Downstream);
        accepted(
            core.handle(view, CoreEvent::Input(InputEvent::text("Y")))
                .unwrap()
                .command
                .unwrap(),
        );
        assert_eq!(core.document().text(), "aXYbc");
        for at in [1, 2] {
            let appearance = DocumentLayoutStyles::semantic_character_at(
                core.document().projection(),
                at,
                false,
            )
            .unwrap();
            assert_eq!(appearance.slant, FontSlant::Italic);
            assert_eq!(appearance.bold, affinity == BoundaryAffinity::Downstream);
        }
        let after = core.document().source_bytes();
        let reopened =
            Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), "aXYbc");
        core_key(&mut core, view, Key::Escape);
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), inserted);
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source);
        core_key(&mut core, view, Key::Ctrl('r'));
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), after);
    }

    let mut core = Core::new(
        Document::from_bytes(b"word".to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: "Code".into(),
        },
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), b"word");
    core_paste(&mut core, view, &external("X"));
    assert_eq!(core.document().text(), "Xword");
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        Some("Code".into())
    );
    assert!(
        core.document()
            .projection()
            .style_spans()
            .iter()
            .any(|span| {
                span.range.contains(&0)
                    && matches!(&span.application,
            StyleApplication::Named(name) if name.0 == "Code")
            })
            || core
                .document()
                .projection()
                .style_spans()
                .iter()
                .any(|span| {
                    span.range.contains(&0)
                        && span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                })
    );
    core_key(&mut core, view, Key::Escape);
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), b"word");
}

#[test]
fn successful_native_paste_retires_pending_operator_count_register_and_mapping() {
    let mut document = plain("alpha beta");
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "\"a2d");
    paste(&mut commands, &mut document, &external("XY"));
    assert_eq!(document.text(), "XYalpha beta");
    key(&mut commands, &mut document, Key::Char('w'));
    assert_eq!(document.text(), "XYalpha beta");
    assert!(commands.register('a').is_none());

    let mut document = plain("alpha beta");
    let mut commands = CommandInterpreter::new();
    text(&mut commands, &mut document, ":nnoremap zx dw");
    key(&mut commands, &mut document, Key::Enter);
    assert_eq!(
        key(&mut commands, &mut document, Key::Char('z')).status,
        CommandStatus::Pending
    );
    paste(&mut commands, &mut document, &external("XY"));
    key(&mut commands, &mut document, Key::Char('x'));
    assert_eq!(document.text(), "XYlpha beta");
}

#[test]
fn failed_native_paste_preserves_pending_operator_mapping_and_selection() {
    let unavailable = ClipboardCommandContext::new();
    for pending_mapping in [false, true] {
        let mut document = plain("alpha beta");
        let mut commands = CommandInterpreter::new();
        if pending_mapping {
            text(&mut commands, &mut document, ":nnoremap zx dw");
            key(&mut commands, &mut document, Key::Enter);
            key(&mut commands, &mut document, Key::Char('z'));
        } else {
            keys(&mut commands, &mut document, "d");
        }
        let history = document.history_status();
        let revision = document.revision();
        let output = commands
            .handle_with_clipboard_context(
                &mut document,
                InputEvent::Key(Key::PasteClipboard),
                &unavailable,
            )
            .unwrap();
        assert_eq!(
            output.status,
            CommandStatus::RegisterReadError(RegisterReadError::ClipboardUnavailable(
                ClipboardTarget::Clipboard
            ))
        );
        assert_eq!(document.source_bytes(), b"alpha beta");
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status(), history);
        assert_eq!(commands.mode(), Mode::Normal);
        assert_eq!(commands.cursor(), 0);
        key(
            &mut commands,
            &mut document,
            Key::Char(if pending_mapping { 'x' } else { 'w' }),
        );
        assert_eq!(document.text(), "beta");
    }

    let mut document = plain("abcdef");
    let mut commands = CommandInterpreter::new();
    assert!(commands.set_cursor_from_pointer(&document, 1, BoundaryAffinity::Downstream, false));
    assert!(commands.set_cursor_from_pointer(&document, 3, BoundaryAffinity::Upstream, true));
    let output = commands
        .handle_with_clipboard_context(
            &mut document,
            InputEvent::Key(Key::PasteClipboard),
            &unavailable,
        )
        .unwrap();
    assert!(matches!(output.status, CommandStatus::RegisterReadError(_)));
    assert!(commands.is_native_selection());
    assert_eq!(commands.cursor(), 3);
    assert_eq!(document.source_bytes(), b"abcdef");
    text(&mut commands, &mut document, "Z");
    assert_eq!(document.text(), "aZdef");
}

#[test]
fn literal_next_is_retired_only_when_native_paste_succeeds() {
    for succeeds in [false, true] {
        let mut document = plain("abc");
        let mut commands = CommandInterpreter::new();
        keys(&mut commands, &mut document, "i");
        key(&mut commands, &mut document, Key::Ctrl('v'));
        keys(&mut commands, &mut document, "06");
        assert!(commands.literal_input_pending());
        if succeeds {
            paste(&mut commands, &mut document, &external("XY"));
            assert!(!commands.literal_input_pending());
            text(&mut commands, &mut document, "5");
            assert_eq!(document.text(), "XY5abc");
        } else {
            let history = document.history_status();
            let output = commands
                .handle_with_clipboard_context(
                    &mut document,
                    InputEvent::Key(Key::PasteClipboard),
                    &ClipboardCommandContext::new(),
                )
                .unwrap();
            assert!(matches!(output.status, CommandStatus::RegisterReadError(_)));
            assert!(commands.literal_input_pending());
            assert_eq!(document.history_status(), history);
            assert_eq!(document.text(), "abc");
            keys(&mut commands, &mut document, "5");
            assert_eq!(document.text(), "Aabc");
        }
        assert_eq!(commands.mode(), Mode::Insert);
    }
}

#[test]
fn encoding_rejection_preserves_source_selection_registers_and_history() {
    let mut document =
        Document::from_bytes(b"abcdef".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "lvl");
    let revision = document.revision();
    let history = document.history_status();
    let context = external("😀");
    assert!(commands
        .handle_with_clipboard_context(
            &mut document,
            InputEvent::Key(Key::PasteClipboard),
            &context,
        )
        .is_err());
    assert_eq!(document.source_bytes(), b"abcdef");
    assert_eq!(document.revision(), revision);
    assert_eq!(document.history_status(), history);
    assert_eq!(commands.mode(), Mode::VisualCharacter);
    assert_eq!(commands.cursor(), 2);
    assert!(commands.register('-').is_none());
    key(&mut commands, &mut document, Key::Char('d'));
    assert_eq!(document.text(), "adef");
}

type Editor = Core<MockTextMeasurementProvider>;

fn core_key(core: &mut Editor, view: ViewId, key: Key) {
    accepted(
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
            .unwrap()
            .command
            .unwrap(),
    );
}

fn core_text(core: &mut Editor, view: ViewId, value: &str) {
    accepted(
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::text(value)))
            .unwrap()
            .command
            .unwrap(),
    );
}

fn core_paste(core: &mut Editor, view: ViewId, context: &ClipboardCommandContext) {
    let before = context.clone();
    let output = accepted(
        core.handle_with_layout(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::PasteClipboard),
                clipboard: context.clone(),
            },
        )
        .unwrap()
        .command
        .unwrap(),
    );
    assert_eq!(context, &before);
    assert!(output.clipboard_writes.is_empty());
}

fn begin_deferred_block(core: &mut Editor, view: ViewId, entry: char) {
    for key in [
        Key::Char('l'),
        Key::Ctrl('v'),
        Key::Char('j'),
        Key::Char('l'),
        Key::Char(entry),
    ] {
        core_key(core, view, key);
    }
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .visual_block_insert_payload(),
        Some("")
    );
}

#[test]
fn native_paste_finishes_deferred_block_insert_append_and_change_on_the_first_row() {
    let original = "abcd\nefgh\nijkl";
    for (entry, pasted, typed, prior, caret) in [
        (
            'I',
            "aXYbcd\nefgh\nijkl",
            "aXYZbcd\nefgh\nijkl",
            original,
            3,
        ),
        (
            'A',
            "abcXYd\nefgh\nijkl",
            "abcXYZd\nefgh\nijkl",
            original,
            5,
        ),
        ('c', "aXYd\neh\nijkl", "aXYZd\neh\nijkl", "ad\neh\nijkl", 3),
    ] {
        let mut core = Core::new(plain(original));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        begin_deferred_block(&mut core, view, entry);
        core_paste(&mut core, view, &external("XY"));
        assert_eq!(core.document().source_bytes(), pasted.as_bytes());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.command_state(view).unwrap().cursor(), caret);
        assert!(core
            .command_state(view)
            .unwrap()
            .visual_block_insert_payload()
            .is_none());
        core_text(&mut core, view, "Z");
        core_key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), typed.as_bytes());
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), pasted.as_bytes());
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), prior.as_bytes());
        if entry == 'c' {
            core_key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), original.as_bytes());
            core_key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), prior.as_bytes());
        }
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), pasted.as_bytes());
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), typed.as_bytes());
    }
}

#[test]
fn native_paste_flushes_prior_block_typing_before_its_own_undo_unit() {
    let original = "abcd\nefgh\nijkl";
    let mut core = Core::new(plain(original));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    begin_deferred_block(&mut core, view, 'I');
    core_text(&mut core, view, "T");
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    core_paste(&mut core, view, &external("XY"));
    assert_eq!(core.document().source_bytes(), b"aXYTbcd\neTfgh\nijkl");
    core_text(&mut core, view, "Z");
    core_key(&mut core, view, Key::Escape);
    assert_eq!(core.document().source_bytes(), b"aXYZTbcd\neTfgh\nijkl");
    for expected in ["aXYTbcd\neTfgh\nijkl", "aTbcd\neTfgh\nijkl", original] {
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
    for expected in [
        "aTbcd\neTfgh\nijkl",
        "aXYTbcd\neTfgh\nijkl",
        "aXYZTbcd\neTfgh\nijkl",
    ] {
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}

#[test]
fn rejected_native_paste_keeps_deferred_block_input_and_history_usable() {
    let original = b"abcd\nefgh\nijkl";
    for (encoding, context, is_encoding_error) in [
        (Encoding::Utf8, external("X\nY"), false),
        (Encoding::Latin1, external("😀"), true),
        (Encoding::Utf8, ClipboardCommandContext::new(), false),
    ] {
        let mut core = Core::new(
            Document::from_bytes(original.to_vec(), encoding, Format::PlainText).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        begin_deferred_block(&mut core, view, 'I');
        core_text(&mut core, view, "T");
        let history = core.document().history_status();
        let revision = core.document().revision();
        let outcome = core.handle_with_layout(
            view,
            CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::PasteClipboard),
                clipboard: context,
            },
        );
        if is_encoding_error {
            assert!(outcome.is_err());
        } else {
            let output = outcome.unwrap().command.unwrap();
            assert!(!matches!(
                output.status,
                CommandStatus::Complete | CommandStatus::Pending
            ));
            assert!(output.clipboard_writes.is_empty());
        }
        assert_eq!(core.document().source_bytes(), original);
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .visual_block_insert_payload(),
            Some("T")
        );
        core_text(&mut core, view, "Z");
        core_key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), b"aTZbcd\neTZfgh\nijkl");
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), original);
    }
}

#[test]
fn native_table_matrix_paste_resumes_replace_and_accepts_an_empty_single_cell() {
    let original = b"| A | B |\n| - | - |\n| x | y |\n";
    for (value, expected, pasted_source, typed_source) in [
        (
            "XY",
            "XY\nB\nx\ny",
            b"|XY  | B |\n| - | - |\n| x | y |\n".as_slice(),
            b"|ZY  | B |\n| - | - |\n| x | y |\n".as_slice(),
        ),
        (
            "",
            "\nB\nx\ny",
            b"|  | B |\n| - | - |\n| x | y |\n".as_slice(),
            b"|Z  | B |\n| - | - |\n| x | y |\n".as_slice(),
        ),
    ] {
        let donor = Document::from_bytes(
            format!("| {value} |\n| - |\n").into_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let (fragment, plain_text) = donor
            .table_clipboard_fragment(donor.projection().tables()[0].id, 0..1, 0..1)
            .unwrap();
        assert_eq!(plain_text, value);
        let context = clipboard(ClipboardContent::from_register(
            RegisterValue::from_clipboard_fragment(fragment).unwrap(),
        ));
        let mut core = Core::new(
            Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core_key(&mut core, view, Key::Char('R'));
        assert!(core.table_selection(view).unwrap().is_none());
        core_paste(&mut core, view, &context);
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.document().source_bytes(), pasted_source);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Replace);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        accepted(
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("Z")))
                .unwrap()
                .command
                .unwrap(),
        );
        assert_eq!(core.document().source_bytes(), typed_source);
        core_key(&mut core, view, Key::Escape);
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), pasted_source);
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), original);
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), pasted_source);
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), typed_source);
    }
}

#[test]
fn block_clipboard_inserts_each_row_and_follows_the_last_row_in_each_mode() {
    let context = clipboard(ClipboardContent::from_register(RegisterValue::blockwise(
        "XY\nZZ",
    )));
    for (entry, expected_mode) in [
        (None, Mode::Normal),
        (Some('i'), Mode::Insert),
        (Some('R'), Mode::Replace),
    ] {
        let mut core = Core::new(plain("abcd\nefgh"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core_key(&mut core, view, Key::Char('l'));
        if let Some(entry) = entry {
            core_key(&mut core, view, Key::Char(entry));
        }
        core_paste(&mut core, view, &context);
        assert_eq!(core.document().text(), "aXYbcd\neZZfgh");
        assert_eq!(core.command_state(view).unwrap().mode(), expected_mode);
        assert_eq!(core.command_state(view).unwrap().cursor(), 10);
        if entry.is_some() {
            core_key(&mut core, view, Key::Escape);
        }
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"abcd\nefgh");
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), b"aXYbcd\neZZfgh");
    }
}

#[test]
fn visual_block_paste_deletes_the_rectangle_then_applies_the_donor_shape() {
    for (content, expected, caret) in [
        (ClipboardContent::from_plain_text("XY"), "aXYd\neh", 3),
        (
            ClipboardContent::from_plain_text("X\nY\n"),
            "X\nY\nad\neh",
            4,
        ),
        (
            ClipboardContent::from_register(RegisterValue::blockwise("XY\nZZ")),
            "aXYd\neZZh",
            8,
        ),
    ] {
        let mut core = Core::new(plain("abcd\nefgh"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        for event in [
            Key::Char('l'),
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('l'),
        ] {
            core_key(&mut core, view, event);
        }
        core_paste(&mut core, view, &clipboard(content));
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().cursor(), caret);
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('-')
                .unwrap()
                .text,
            "bc\nfg"
        );
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), b"abcd\nefgh");
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}

#[test]
fn unequal_block_rows_pad_only_before_retained_suffixes() {
    let context = clipboard(ClipboardContent::from_register(RegisterValue::blockwise(
        "XY\nZ",
    )));
    for (source, entry, expected, caret, mode) in [
        ("abcd\nefgh", None, "aXYbcd\neZ fgh", 10, Mode::Normal),
        ("abcd\nefgh", Some('i'), "aXYbcd\neZ fgh", 10, Mode::Insert),
        ("abcd\ne", None, "aXYbcd\neZ", 8, Mode::Normal),
        ("abcd\ne", Some('i'), "aXYbcd\neZ", 9, Mode::Insert),
    ] {
        let mut core = Core::new(plain(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        core_key(&mut core, view, Key::Char('l'));
        if let Some(entry) = entry {
            core_key(&mut core, view, Key::Char(entry));
        }
        core_paste(&mut core, view, &context);
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.command_state(view).unwrap().mode(), mode);
        assert_eq!(core.command_state(view).unwrap().cursor(), caret);
        if entry.is_some() {
            core_key(&mut core, view, Key::Escape);
        }
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core_key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
    for (source, expected, caret) in [
        ("abcd\nefgh", "aXYd\neZ h", 8),
        ("abcd\nefg", "aXYd\neZ", 6),
    ] {
        let mut core = Core::new(plain(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        for event in [
            Key::Char('l'),
            Key::Ctrl('v'),
            Key::Char('j'),
            Key::Char('l'),
        ] {
            core_key(&mut core, view, event);
        }
        core_paste(&mut core, view, &context);
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.command_state(view).unwrap().cursor(), caret);
        core_key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

fn selected_block_donor_round_trip(
    source: &str,
    selection: &[Key],
    expected: &str,
    caret: usize,
    removed: &str,
) {
    let context = clipboard(ClipboardContent::from_register(RegisterValue::blockwise(
        "XY\nZ",
    )));
    let mut core = Core::new(plain(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    for event in selection {
        core_key(&mut core, view, *event);
    }
    core_paste(&mut core, view, &context);
    assert_eq!(core.document().text(), expected);
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    assert_eq!(core.command_state(view).unwrap().cursor(), caret);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('-')
            .unwrap()
            .text,
        removed
    );
    core_key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core_key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('-')
            .unwrap()
            .text,
        removed
    );
}

#[test]
fn native_block_donor_replaces_character_selection_then_inserts_rows_at_its_column() {
    selected_block_donor_round_trip(
        "abcdef\nuvwxyz",
        &[Key::Char('l'), Key::Char('v'), Key::Char('l')],
        "aXYdef\nuZ vwxyz",
        10,
        "bc",
    );
}

#[test]
fn native_block_donor_replaces_line_selection_using_the_retained_empty_line() {
    selected_block_donor_round_trip(
        "abc\ndef\nghi",
        &[Key::Char('V')],
        "XY\nZ def\nghi",
        5,
        "abc\n",
    );
}

#[test]
fn native_block_donor_uses_surviving_rows_after_a_cross_line_selection_is_joined() {
    selected_block_donor_round_trip(
        "abcdef\nuvwxyz\nlast",
        &[
            Key::Char('l'),
            Key::Char('v'),
            Key::Char('j'),
            Key::Char('l'),
        ],
        "aXYxyz\nlZ ast",
        10,
        "bcdef\nuvw",
    );
}

#[test]
fn native_rich_markdown_paste_retains_authored_spelling_and_exact_history() {
    let source = b"__bold__ and *italic*";
    let donor = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let value = RegisterValue::from_clipboard_fragment(
        donor.clipboard_fragment(0..donor.text().len()).unwrap(),
    )
    .unwrap();
    let context = clipboard(ClipboardContent::from_register(value));
    for entry in [None, Some('i'), Some('R')] {
        let mut document = Document::from_bytes(vec![], Encoding::Utf8, Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        if let Some(entry) = entry {
            key(&mut commands, &mut document, Key::Char(entry));
        }
        paste(&mut commands, &mut document, &context);
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.text(), "bold and italic");
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(reopened.text(), document.text());
        exact_history(&mut commands, &mut document, b"", source);
    }
}

#[test]
fn native_linewise_rich_donor_replaces_character_selection_above_the_joined_line() {
    let mut donor =
        Document::from_bytes(b"__X__\n\n*Y*".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut donor_commands = CommandInterpreter::new();
    keys(&mut donor_commands, &mut donor, "VG\"+");
    let output = accepted(
        donor_commands
            .handle_with_clipboard_context(
                &mut donor,
                InputEvent::key('y'),
                &ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
            )
            .unwrap(),
    );
    assert_eq!(output.clipboard_writes.len(), 1);
    let content = output.clipboard_writes[0].content().clone();
    let value = content.portable_register().unwrap();
    assert_eq!(value.kind, RegisterKind::Linewise);
    assert_eq!(value.text, "X\nY\n");
    assert!(value.clipboard_fragment().is_some());
    let context = clipboard(content);

    let mut document =
        Document::from_bytes(b"abcdef".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut commands = CommandInterpreter::new();
    keys(&mut commands, &mut document, "lvl");
    paste(&mut commands, &mut document, &context);
    let expected = b"__X__\n\n*Y*\n\nadef";
    assert_eq!(document.text(), "X\nY\nadef");
    assert_eq!(document.source_bytes(), expected);
    assert_eq!(commands.mode(), Mode::Normal);
    assert_eq!(commands.cursor(), 4);
    assert_eq!(commands.register('-').unwrap().text, "bc");
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(reopened.text(), document.text());
    exact_history(&mut commands, &mut document, b"abcdef", expected);
}

#[test]
fn native_source_paste_normalizes_external_breaks_and_preserves_unselected_source_bytes() {
    let original = b"# original\r\n\r\n__untouched__";
    let mut document = Document::from_bytes_with_file_format(
        original.to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
        FileFormat::Dos,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();
    paste(&mut commands, &mut document, &external("**new**\r\n"));
    let expected = b"**new**\r\n# original\r\n\r\n__untouched__";
    assert_eq!(document.source_bytes(), expected);
    exact_history(&mut commands, &mut document, original, expected);
}
