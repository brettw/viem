use super::*;
use crate::document::{Encoding, Format};
use crate::command::clipboard::{ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};

fn key(commands: &mut CommandInterpreter, document: &mut Document, key: Key) -> CommandOutput {
    commands.handle(document, InputEvent::Key(key)).unwrap()
}
fn register(commands: &mut CommandInterpreter, name: char, text: &str) {
    commands.registers.yank(Some(name), RegisterValue::characterwise(text));
}
fn insert(commands: &mut CommandInterpreter, document: &mut Document, selector: Key) -> CommandOutput {
    key(commands, document, Key::Ctrl('r'));
    key(commands, document, selector)
}

#[test]
fn named_numbered_unnamed_and_small_delete_registers_insert_into_each_prompt() {
    for prompt in [':', '/', '?'] {
        let mut document = Document::new("source");
        let mut commands = CommandInterpreter::new();
        for name in ['a', '0', '7', '"', '-'] { register(&mut commands, name, "α🙂"); }
        key(&mut commands, &mut document, Key::Char(prompt));
        for name in ['A', '0', '7', '"', '-'] {
            assert_eq!(insert(&mut commands, &mut document, Key::Char(name)).status, CommandStatus::Pending);
            assert!(!commands.command_line_register_pending());
        }
        assert_eq!(commands.command_line(), Some("α🙂α🙂α🙂α🙂α🙂"));
        assert_eq!(document.text(), "source");
        assert_eq!(document.revision(), Revision(0));
        assert_eq!(commands.mode(), Mode::CommandLine);
    }
}

#[test]
fn literal_variants_preserve_controls_and_typed_variant_edits_only_the_prompt() {
    for prefix in [None, Some(Key::Ctrl('r')), Some(Key::Ctrl('o'))] {
        let mut document = Document::new("source");
        let mut commands = CommandInterpreter::new();
        register(&mut commands, 'a', "xy\u{8}z\n\u{1b}\u{3}\t");
        key(&mut commands, &mut document, Key::Char(':'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        if let Some(prefix) = prefix { key(&mut commands, &mut document, prefix); }
        key(&mut commands, &mut document, Key::Char('a'));
        assert_eq!(commands.command_line(), Some(if prefix.is_some() { "xy\u{8}z\r\u{1b}\u{3}\t" } else { "xz\r\u{1b}\u{3}\t" }));
        assert_eq!(commands.mode(), Mode::CommandLine);
        assert_eq!(document.text(), "source");
        assert_eq!(document.revision(), Revision(0));
    }
}

#[test]
fn linewise_separators_are_literal_carriage_returns_without_submitting() {
    let mut document = Document::new("one\ntwo");
    let mut commands = CommandInterpreter::new();
    commands.registers.yank(Some('a'), RegisterValue::linewise("one\ntwo\n"));
    key(&mut commands, &mut document, Key::Char(':'));
    insert(&mut commands, &mut document, Key::Char('a'));
    assert_eq!(commands.command_line(), Some("one\rtwo\r"));
    assert_eq!(document.revision(), Revision(0));
}

#[test]
fn literal_mac_line_feeds_remain_distinct_from_semantic_register_breaks() {
    for literal in [false, true] {
        let mut document = Document::from_bytes_with_file_format(
            b"source".to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Mac,
        ).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.registers.yank(Some('a'), RegisterValue::try_new("literal\nline\nbreak", RegisterKind::Characterwise, vec![12]).unwrap());
        key(&mut commands, &mut document, Key::Char(':'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        if literal { key(&mut commands, &mut document, Key::Ctrl('r')); }
        key(&mut commands, &mut document, Key::Char('a'));
        assert_eq!(commands.command_line(), Some("literal\nline\rbreak"));
        assert_eq!(document.source_bytes(), b"source");
        assert_eq!(document.revision(), Revision(0));
    }
}

#[test]
fn selectors_cancel_without_closing_prompt_and_literal_next_has_precedence() {
    for cancel in [Key::Escape, Key::Ctrl('c'), Key::Ctrl('[')] {
        let mut document = Document::new("source");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char(':'));
        key(&mut commands, &mut document, Key::Char('x'));
        insert(&mut commands, &mut document, cancel);
        assert_eq!(commands.command_line(), Some("x"));
        assert_eq!(commands.mode(), Mode::CommandLine);
        assert!(!commands.command_line_register_pending());
        key(&mut commands, &mut document, Key::Ctrl('v'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        assert_eq!(commands.command_line(), Some("x\u{12}"));
        assert!(!commands.command_line_register_pending());
    }
}

#[test]
fn empty_unsupported_and_unavailable_registers_preserve_prompt_and_source() {
    for selector in [Key::Char('z'), Key::Char('='), Key::Char('#'), Key::Char('+')] {
        let mut document = Document::new("source");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char(':'));
        key(&mut commands, &mut document, Key::Char('x'));
        let output = insert(&mut commands, &mut document, selector);
        assert!(command_status_stops_compound(&output.status));
        assert_eq!(commands.command_line(), Some("x"));
        assert!(!commands.command_line_register_pending());
        assert_eq!(commands.mode(), Mode::CommandLine);
        assert_eq!(document.revision(), Revision(0));
    }
}

#[test]
fn selection_replacement_and_combining_suffix_keep_legal_grapheme_boundaries() {
    let mut document = Document::new("source");
    let mut commands = CommandInterpreter::new();
    register(&mut commands, 'a', "α🙂");
    key(&mut commands, &mut document, Key::Char(':'));
    commands.handle(&mut document, InputEvent::Text("e\u{301}Z".into())).unwrap();
    let expected = commands.command_line_snapshot().unwrap();
    commands.edit_command_line(&document, CommandLineEditRequest {
        document: document.id(), revision: document.revision(), expected,
        action: CommandLineEditAction::Select { anchor: 0, active: 3 },
    }).unwrap();
    insert(&mut commands, &mut document, Key::Char('a'));
    assert_eq!(commands.command_line(), Some("α🙂Z"));
    assert_eq!(commands.command_line_cursor(), Some(6));
    commands.command_line_state.as_mut().unwrap().buffer.set("\u{301}x".into());
    key(&mut commands, &mut document, Key::Home);
    register(&mut commands, 'a', "e");
    insert(&mut commands, &mut document, Key::Char('a'));
    assert_eq!(commands.command_line(), Some("e\u{301}x"));
    assert_eq!(commands.command_line_cursor(), Some(3));
}

#[test]
fn clipboard_reads_are_captured_for_controller_only_plans_and_not_retained() {
    for target in [ClipboardTarget::Clipboard, ClipboardTarget::Primary] {
        let mut document = Document::new("source");
        let mut commands = CommandInterpreter::new();
        key(&mut commands, &mut document, Key::Char(':'));
        key(&mut commands, &mut document, Key::Ctrl('r'));
        let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            target, ClipboardGeneration(1), ClipboardContent::from_plain_text("from clipboard"),
        ));
        let context = CommandContext::with_clipboard(&document, &clipboard);
        let CommandResolution::Planned(plan) = commands.resolve(&context, InputEvent::key(target.register_name())).unwrap() else { panic!("register insertion must remain controller-only"); };
        assert!(plan.model_request().is_none());
        assert_eq!(commands.command_line(), Some(""));
        let result = plan.publish_success(&mut commands, &document, false);
        assert!(result.output.clipboard_writes.is_empty());
        assert_eq!(commands.command_line(), Some("from clipboard"));
        assert!(commands.clipboard_context.read(target).is_none());
        assert_eq!(document.revision(), Revision(0));
    }
}

#[test]
fn objects_and_search_and_command_registers_use_current_buffer_context() {
    let mut document = Document::new("hello-world αβ\nlast");
    let mut commands = CommandInterpreter::new();
    commands.last_search = Some((SearchDirection::Forward, "αβ".into()));
    commands.last_command_line = Some("set ai".into());
    key(&mut commands, &mut document, Key::Char(':'));
    for selector in [Key::Ctrl('w'), Key::Char('/'), Key::Ctrl('a'), Key::Char(':'), Key::Ctrl('l')] {
        insert(&mut commands, &mut document, selector);
    }
    assert_eq!(commands.command_line(), Some("helloαβhello-worldset aihello-world αβ"));
    assert_eq!(document.revision(), Revision(0));
}

#[test]
fn word_objects_skip_nonword_text_but_do_not_cross_the_current_line() {
    for (offset, key_value, expected) in [
        (5, 'w', "world"), (5, 'a', "hello-world"),
        (11, 'w', "rest"), (11, 'a', "rest"),
    ] {
        let mut document = Document::new("hello-world  rest\nnext");
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor(&document, offset));
        key(&mut commands, &mut document, Key::Char(':'));
        insert(&mut commands, &mut document, Key::Ctrl(key_value));
        assert_eq!(commands.command_line(), Some(expected));
        assert_eq!(document.revision(), Revision(0));
    }
}

#[test]
fn submitted_failed_ex_is_the_last_command_register_and_is_shared() {
    let mut document = Document::new("source");
    let mut commands = CommandInterpreter::new();
    key(&mut commands, &mut document, Key::Char(':'));
    commands.handle(&mut document, InputEvent::Text("bogus_command".into())).unwrap();
    key(&mut commands, &mut document, Key::Enter);
    let mut other = CommandInterpreter::new();
    other.install_buffer_state(&commands.export_buffer_state());
    key(&mut other, &mut document, Key::Char(':'));
    insert(&mut other, &mut document, Key::Char(':'));
    assert_eq!(other.command_line(), Some("bogus_command"));
}

#[test]
fn recorded_register_insertion_replays_once_and_preserves_the_source() {
    let mut document = Document::new("source");
    let mut commands = CommandInterpreter::new();
    register(&mut commands, 'b', "set ai");
    for input in [Key::Char('q'), Key::Char('a'), Key::Char(':'), Key::Ctrl('r'), Key::Char('b'), Key::Escape, Key::Char('q')] {
        key(&mut commands, &mut document, input);
    }
    for input in [Key::Char('@'), Key::Char('a')] { key(&mut commands, &mut document, input); }
    assert_eq!(commands.mode(), Mode::Normal);
    assert_eq!(document.text(), "source");
    assert_eq!(document.revision(), Revision(0));
    assert_eq!(commands.registers.macro_events('a').unwrap(), [InputEvent::key(':'), InputEvent::Key(Key::Ctrl('r')), InputEvent::key('b'), InputEvent::Key(Key::Escape)]);
}

#[test]
fn nested_register_expansion_is_bounded_and_never_leaves_prompt_mode() {
    let mut document = Document::new("source");
    let mut commands = CommandInterpreter::new();
    register(&mut commands, 'a', "\u{12}a");
    key(&mut commands, &mut document, Key::Char(':'));
    let output = insert(&mut commands, &mut document, Key::Char('a'));
    assert!(command_status_stops_compound(&output.status));
    assert_eq!(commands.mode(), Mode::CommandLine);
    assert_eq!(document.revision(), Revision(0));
}

#[test]
fn batched_native_text_resolves_any_remaining_register_selector_before_inserting_suffix() {
    let mut document = Document::new("source");
    let mut commands = CommandInterpreter::new();
    register(&mut commands, 'a', "\u{12}");
    register(&mut commands, 'b', "value");
    key(&mut commands, &mut document, Key::Char(':'));
    key(&mut commands, &mut document, Key::Ctrl('r'));
    commands.handle(&mut document, InputEvent::Text("ab tail".into())).unwrap();
    assert_eq!(commands.command_line(), Some("value tail"));
    assert!(!commands.command_line_register_pending());
    assert_eq!(document.revision(), Revision(0));
}
