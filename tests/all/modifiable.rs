//! Buffer editing policy must hold below every command and native edit path.
use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{DocumentError, Encoding, FileFormat, Format, HistoryNavigationRequest, ModelRequest, ModelTransactionError, SemanticInlineStyle, TextEdit};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
fn editor(source: &[u8], format: Format) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes_with_file_format(source.to_vec(), Encoding::Utf8, format, FileFormat::Dos).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
    (core, view)
}
fn key(core: &mut Editor, view: ViewId, key: Key) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap()
}
fn keys(core: &mut Editor, view: ViewId, text: &str) -> CoreOutcome {
    text.chars().map(|ch| key(core, view, Key::Char(ch))).last().unwrap()
}
fn ex(core: &mut Editor, view: ViewId, command: &str) -> CoreOutcome {
    core.stage_command_line(view, command).unwrap();
    key(core, view, Key::Enter)
}
fn blocked(value: impl std::fmt::Debug) {
    let message = format!("{value:?}");
    assert!(message.contains("E21") || message.contains("NotModifiable"), "expected editing policy failure: {message}");
}
fn same(core: &Editor, source: &[u8], history: &viem_core::document::HistoryStatus) {
    assert_eq!(core.document().source_bytes(), source);
    assert_eq!(&core.document().history_status(), history);
}

#[test]
fn option_is_buffer_local_script_visible_and_independent_of_history_and_save_policy() {
    let (mut core, a) = editor(b"one\r\ntwo\n", Format::PlainText);
    let b = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    keys(&mut core, a, "x");
    let bytes = core.document().source_bytes();
    let history = core.document().history_status();
    ex(&mut core, a, "setlocal noma");
    assert!(!core.document().is_modifiable());
    same(&core, &bytes, &history);
    blocked(keys(&mut core, b, "x"));
    let output = ex(&mut core, b, "echo &ma &l:modifiable exists('+ma') exists('&modifiable')");
    assert!(format!("{output:?}").contains("0 0 1 1"), "{output:?}");
    let output = ex(&mut core, b, "set ma?");
    assert!(format!("{output:?}").contains("nomodifiable"));
    ex(&mut core, b, "let &l:modifiable = 1");
    assert!(core.document().is_modifiable());
    key(&mut core, a, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), b"one\r\ntwo\n");
    assert!(core.document().is_modifiable());
    ex(&mut core, a, "setglobal ma!");
    assert!(!core.document().is_modifiable());
    ex(&mut core, a, "set ma&");
    assert!(core.document().is_modifiable());
    ex(&mut core, a, "set readonly");
    keys(&mut core, a, "x");
    assert_eq!(core.document().text(), "ne\ntwo\n");
    ex(&mut core, a, "set noma");
    assert!(core.document().is_read_only());
    let other = Document::new("new buffer");
    assert!(other.is_modifiable());
    ex(&mut core, a, "set noro");
    let write = ex(&mut core, a, "write /tmp/viem-modifiable-test.txt");
    assert!(!format!("{write:?}").contains("E21"));
    assert!(!format!("{write:?}").contains("ReadOnly"));
}

#[test]
fn normal_visual_operator_count_register_repeat_and_macro_edits_are_blocked() {
    for (prefix, command) in [
        ("", "i"), ("", "a"), ("", "A"), ("", "I"), ("", "R"), ("", "o"), ("", "O"),
        ("", "2x"), ("", "\"a2dw"), ("", "cc"), ("", "rZ"), ("", "2J"), ("", ">>"),
        ("", "gUw"), ("", "gqq"), ("", "p"), ("", "."), ("", "@a"),
        ("vll", "d"), ("V", "c"), ("v", "p"),
    ] {
        let (mut core, view) = editor(b"one two\r\nthree four\n", Format::PlainText);
        ex(&mut core, view, "let @a = 'x'");
        keys(&mut core, view, "x"); // establishes dot and the small-delete register
        key(&mut core, view, Key::Char('u'));
        ex(&mut core, view, "set noma");
        if !prefix.is_empty() { keys(&mut core, view, prefix); }
        let history = core.document().history_status();
        let registers = ['a', '"', '-', '1', '0'].map(|r| core.command_state(view).unwrap().register(r).cloned());
        let cursor = core.command_state(view).unwrap().cursor();
        let mode = core.command_state(view).unwrap().mode();
        blocked((prefix, command, keys(&mut core, view, command)));
        same(&core, b"one two\r\nthree four\n", &history);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor, "{prefix}{command}");
        assert_eq!(core.command_state(view).unwrap().mode(), mode, "{prefix}{command}");
        assert_eq!(['a', '"', '-', '1', '0'].map(|r| core.command_state(view).unwrap().register(r).cloned()), registers);
    }
}

#[test]
fn ex_edits_and_history_navigation_fail_without_source_or_option_changes() {
    for command in ["%delete", "%s/o/X/g", "%s/o/X/gc", "g/o/s/o/X/c", "g/o/delete", "%sort!", "1copy 2", "1move 2", "normal! x", "call setline(1, 'X')", "call append(1, 'X')", "undo", "redo", "earlier 1", "set ff=unix", "set fenc=latin1", "set bomb"] {
        let (mut core, view) = editor(b"one\r\ntwo\n", Format::PlainText);
        keys(&mut core, view, "x");
        key(&mut core, view, Key::Char('u'));
        ex(&mut core, view, "set noma");
        let history = core.document().history_status();
        blocked((command, ex(&mut core, view, command)));
        same(&core, b"one\r\ntwo\n", &history);
        assert!(core.command_state(view).unwrap().substitute_confirmation_prompt().is_none());
        assert_eq!(core.document().encoding(), Encoding::Utf8);
        assert_eq!(core.document().file_format(), FileFormat::Dos);
        assert!(!core.document().has_bom());
    }
}

#[test]
fn model_policy_rejects_preparation_and_late_publication() {
    let mut doc = Document::new("one\ntwo");
    let request = ModelRequest::ApplyTextEdits { document: doc.id(), revision: doc.revision(), edits: vec![TextEdit::new(0..1, "X")] };
    let prepared = doc.prepare_model_request(request.clone()).unwrap();
    let history = doc.history_status();
    doc.set_modifiable(false);
    assert!(matches!(doc.prepare_model_request(request), Err(ModelTransactionError::Document(DocumentError::NotModifiable))));
    assert!(matches!(doc.commit_model_transaction(prepared), Err(ModelTransactionError::Document(DocumentError::NotModifiable))));
    assert_eq!(doc.history_status(), history);
    assert_eq!(doc.text(), "one\ntwo");
    assert_eq!(doc.insert(0, "x"), Err(DocumentError::NotModifiable));
    doc.set_modifiable(true);
    doc.insert(0, "x").unwrap();
    let target = doc.history_status().current;
    doc.set_modifiable(false);
    blocked(doc.try_undo());
    blocked(doc.try_redo());
    blocked(doc.select_history_node(target.node));
    blocked(doc.select_history_change(target.change));
    doc.set_modifiable(true);
    assert!(doc.undo());
    assert_eq!(doc.text(), "one\ntwo");
}

#[test]
fn block_insertion_and_existing_insert_replace_sessions_obey_other_views_policy() {
    for entry in [Key::Char('i'), Key::Char('R')] {
        let (mut core, a) = editor(b"one\r\ntwo\n", Format::PlainText);
        let b = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
        key(&mut core, a, entry);
        ex(&mut core, b, "set noma");
        let history = core.document().history_status();
        blocked(key(&mut core, a, Key::Char('X')));
        same(&core, b"one\r\ntwo\n", &history);
        key(&mut core, a, Key::Escape);
        assert_eq!(core.command_state(a).unwrap().mode(), Mode::Normal);
    }
    for entry in ['I', 'A', 'c', 'd', 'r'] {
        let (mut core, view) = editor(b"one\r\ntwo\n", Format::PlainText);
        ex(&mut core, view, "set noma");
        key(&mut core, view, Key::Ctrl('v'));
        key(&mut core, view, Key::Char('j'));
        let history = core.document().history_status();
        let output = key(&mut core, view, Key::Char(entry));
        if entry == 'r' { blocked(key(&mut core, view, Key::Char('X'))); } else { blocked(output); }
        same(&core, b"one\r\ntwo\n", &history);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::VisualBlock);
    }
}

#[test]
fn native_paste_formatting_conversion_and_ime_cannot_bypass_policy() {
    let (mut core, a) = editor(b"one\r\n\r\ntwo", Format::Markdown);
    let b = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
    key(&mut core, a, Key::Char('i'));
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
    core.handle(a, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
    core.handle(a, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3)))).unwrap();
    ex(&mut core, b, "set noma");
    let history = core.document().history_status();
    blocked(core.handle(a, CoreEvent::Composition(CompositionEvent::Commit)));
    same(&core, b"one\r\n\r\ntwo", &history);
    let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_plain_text("pasted"))).with_write(ClipboardTarget::Clipboard);
    blocked(core.handle_with_layout(b, CoreEvent::InputWithClipboard { input: InputEvent::Key(Key::PasteClipboard), clipboard }));
    same(&core, b"one\r\n\r\ntwo", &history);
    keys(&mut core, b, "vll");
    let expected = core.list_selection_identity(b).unwrap();
    blocked(core.handle(b, CoreEvent::SetSelectionSemanticStyle { expected, style: SemanticInlineStyle::Strong, enabled: true }));
    same(&core, b"one\r\n\r\ntwo", &history);
    for event in [
        CoreEvent::SetEncoding { document: core.document().id(), revision: core.document().revision(), target: Encoding::Latin1 },
        CoreEvent::SetFileFormat { document: core.document().id(), revision: core.document().revision(), target: FileFormat::Unix },
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    ] { blocked(core.handle(b, event)); same(&core, b"one\r\n\r\ntwo", &history); }
}

#[test]
fn navigation_yank_search_prompts_and_preview_remain_available() {
    let (mut core, view) = editor(b"**one** two\r\n\r\nthree", Format::Markdown);
    ex(&mut core, view, "set noma");
    let history = core.document().history_status();
    keys(&mut core, view, "wbyiw");
    assert_eq!(core.command_state(view).unwrap().register('0').unwrap().text, "one");
    keys(&mut core, view, "/two");
    assert_eq!(key(&mut core, view, Key::Enter).command.unwrap().status, CommandStatus::Complete);
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    ex(&mut core, view, "set nowrap");
    same(&core, b"**one** two\r\n\r\nthree", &history);
    let document = core.document().id();
    let revision = core.document().revision();
    core.handle(view, CoreEvent::SetDocumentMode { document, revision, mode: viem_core::DocumentMode::CodeFormat }).unwrap();
    assert!(!core.document().is_modifiable());
    assert_eq!(core.document().source_bytes(), b"**one** two\r\n\r\nthree");
}

#[test]
fn excluded_options_are_recognized_but_never_report_working_support() {
    let (mut core, view) = editor(b"text", Format::PlainText);
    for option in ["guicursor", "cursorline", "cursorlineopt", "colorcolumn", "relativenumber", "statusline", "wrapmargin", "foldclose", "foldcolumn", "foldenable", "foldexpr", "foldignore", "foldlevel", "foldlevelstart", "foldmarker", "foldmethod", "foldminlines", "foldnestmax", "foldopen", "foldtext", "spell", "spellcapcheck", "spellfile", "spelllang", "spelloptions", "spellsuggest", "mkspellmem"] {
        let output = ex(&mut core, view, &format!("echo exists('&{option}') exists('+{option}')"));
        assert!(format!("{output:?}").contains("1 0"), "{option}: {output:?}");
    }
    for setting in ["cursorline", "relativenumber", "colorcolumn=80", "statusline=test", "wrapmargin=4", "foldmethod=expr", "foldexpr=throw('must_not_run')", "spell", "spelllang=fr", "guicursor=n:hor10"] {
        let output = ex(&mut core, view, &format!("set {setting}"));
        assert!(format!("{output:?}").contains("does not support"), "{setting}: {output:?}");
        assert_eq!(core.document().source_bytes(), b"text");
    }
}
