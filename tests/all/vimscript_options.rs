//! Option declarations must change the editing behavior exposed to scripts.
use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{CommandInterpreter, CommandOutput, CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

fn keys(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandOutput {
    let mut output = None;
    for ch in text.chars() { output = Some(c.handle(d, InputEvent::key(ch)).unwrap()); }
    output.expect("nonempty input")
}
fn ex(c: &mut CommandInterpreter, d: &mut Document, text: &str) -> CommandOutput {
    c.handle(d, InputEvent::key(':')).unwrap();
    for ch in text.chars() { c.handle(d, InputEvent::key(ch)).unwrap(); }
    c.handle(d, InputEvent::Key(Key::Enter)).unwrap()
}
fn messages(output: &CommandOutput) -> Vec<String> {
    output.ex_outcome.as_ref().map(|ex| ex.frontend_requests.iter().filter_map(|request| match request {
        ExFrontendRequest::Info(ExInfoRequest::Message(message)) => Some(message.clone()), _ => None,
    }).collect()).unwrap_or_default()
}
fn editor(text: &str) -> (CommandInterpreter, Document) { (CommandInterpreter::new(), Document::new(text)) }

#[test]
fn gdefault_toggles_explicit_g_and_repeats_effective_flags() {
    let (mut c, mut d) = editor("a a a\na a a\na a a\n");
    assert_eq!(ex(&mut c, &mut d, "set gdefault | s/a/x/").status, CommandStatus::Complete);
    assert_eq!(d.text(), "x x x\na a a\na a a\n");
    ex(&mut c, &mut d, "2s/a/y/g");
    assert_eq!(d.text(), "x x x\ny a a\na a a\n");
    ex(&mut c, &mut d, "3&&");
    assert_eq!(d.text(), "x x x\ny a a\ny a a\n");
    keys(&mut c, &mut d, "u");
    assert_eq!(d.text(), "x x x\ny a a\na a a\n");
}

#[test]
fn joinspaces_changes_normal_ex_and_bang_joins() {
    for (setting, expected) in [("joinspaces", "Hello.  World!  Again"), ("nojoinspaces", "Hello. World! Again")] {
        let (mut c, mut d) = editor("Hello.\nWorld!\nAgain");
        ex(&mut c, &mut d, &format!("set {setting}"));
        keys(&mut c, &mut d, "3J");
        assert_eq!(d.text(), expected);
        keys(&mut c, &mut d, "u");
        ex(&mut c, &mut d, "%join");
        assert_eq!(d.text(), expected);
        keys(&mut c, &mut d, "u");
        ex(&mut c, &mut d, "%join!");
        assert_eq!(d.text(), "Hello.World!Again");
    }
}

#[test]
fn shiftround_uses_indent_levels_for_normal_visual_and_ex() {
    for (command, expected) in [(">>", "    one\n      two"), ("2>>", "    one\n        two"), ("Vj>", "    one\n        two")] {
        let (mut c, mut d) = editor("  one\n      two");
        ex(&mut c, &mut d, "set sw=4 et shiftround");
        keys(&mut c, &mut d, command);
        assert_eq!(d.text(), expected, "{command}");
    }
    let (mut c, mut d) = editor("  one\n      two");
    ex(&mut c, &mut d, "set sw=4 et shiftround | %>");
    assert_eq!(d.text(), "    one\n        two");
    ex(&mut c, &mut d, "%<");
    assert_eq!(d.text(), "one\n    two");
    keys(&mut c, &mut d, "u");
    assert_eq!(d.text(), "    one\n        two");
}

#[test]
fn keyword_option_controls_motions_operators_text_objects_and_insert_deletion() {
    let (mut c, mut d) = editor("one-two three");
    ex(&mut c, &mut d, "set iskeyword+=-");
    keys(&mut c, &mut d, "w");
    assert_eq!(c.cursor(), 8);
    keys(&mut c, &mut d, "bdw");
    assert_eq!(d.text(), "three");
    keys(&mut c, &mut d, "u0diw");
    assert_eq!(d.text(), " three");
    keys(&mut c, &mut d, "uA");
    c.handle(&mut d, InputEvent::Key(Key::Ctrl('w'))).unwrap();
    c.handle(&mut d, InputEvent::Key(Key::Ctrl('w'))).unwrap();
    assert_eq!(d.text(), "");
    c.handle(&mut d, InputEvent::Key(Key::Escape)).unwrap();
    keys(&mut c, &mut d, "u");
    assert_eq!(d.text(), "one-two three");
}

#[test]
fn invalid_keywords_and_numeric_values_keep_prior_effective_values() {
    let (mut c, mut d) = editor("one-two");
    ex(&mut c, &mut d, "set isk+=-");
    assert!(matches!(ex(&mut c, &mut d, "set isk=300-400").status, CommandStatus::ExError(_) | CommandStatus::Error(_)));
    assert_eq!(messages(&ex(&mut c, &mut d, "echo &isk")), vec!["@,48-57,_,192-255,-"]);
    assert!(matches!(ex(&mut c, &mut d, "set history=-1").status, CommandStatus::ExError(_) | CommandStatus::Error(_)));
    assert_eq!(messages(&ex(&mut c, &mut d, "echo &history")), vec!["200"]);
}

#[test]
fn history_zero_disables_retention_and_limits_deduplicate_entries() {
    let (mut c, mut d) = editor("text");
    let output = ex(&mut c, &mut d, "set history=2 | call histadd('/', 'a') | call histadd('/', 'b') | call histadd('/', 'c') | echo histget('/', -2) histget('/', -1)");
    assert_eq!(messages(&output), vec!["b c"]);
    let output = ex(&mut c, &mut d, "call histadd('/', 'b') | echo histget('/', -2) histget('/', -1)");
    assert_eq!(messages(&output), vec!["c b"]);
    let output = ex(&mut c, &mut d, "set history=0 | call histadd('/', 'ignored') | echo empty(histget('/')) empty(histget(':'))");
    assert_eq!(messages(&output), vec!["1 1"]);
}

#[test]
fn whichwrap_and_startofline_change_explicit_vim_motion_keys() {
    let (mut c, mut d) = editor("abcd\n  efgh");
    ex(&mut c, &mut d, "set whichwrap=h,l nostartofline");
    keys(&mut c, &mut d, "3lG");
    assert_eq!(c.cursor(), 8);
    keys(&mut c, &mut d, "gg");
    assert_eq!(c.cursor(), 3);
    keys(&mut c, &mut d, "l");
    assert_eq!(c.cursor(), 5);
    keys(&mut c, &mut d, "h");
    assert_eq!(c.cursor(), 3);
    ex(&mut c, &mut d, "set startofline whichwrap=");
    keys(&mut c, &mut d, "lG");
    assert_eq!(c.cursor(), 7);
}

#[test]
fn clipboard_unnamed_reads_and_yanks_preserve_numbered_registers() {
    let (mut c, mut d) = editor("one\ntwo");
    ex(&mut c, &mut d, "set clipboard=unnamed,unnamedplus");
    keys(&mut c, &mut d, "y");
    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard).with_write(ClipboardTarget::Primary);
    let output = c.handle_with_clipboard_context(&mut d, InputEvent::key('y'), &context).unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
    assert_eq!(output.clipboard_writes.len(), 2);
    assert_eq!(c.register('0').unwrap().text, "one\n");
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_plain_text("CLIP")));
    c.handle_with_clipboard_context(&mut d, InputEvent::key('p'), &context).unwrap();
    assert_eq!(d.text(), "oCLIPne\ntwo");
    let before = d.text().to_owned();
    let output = keys(&mut c, &mut d, "x");
    assert!(matches!(output.status, CommandStatus::RegisterWriteError(_)));
    assert_eq!(d.text(), before);
    keys(&mut c, &mut d, "\"_x");
    assert_ne!(d.text(), before);
}

#[test]
fn filetype_and_syntax_select_code_language_without_changing_bytes() {
    let document = Document::from_bytes(b"let n = 1;\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    core.initialize_code_detection("example.rs", false).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1000.);
    for line in ["set ft=python", "set syntax=javascript", "set syntax=OFF"] {
        core.stage_command_line(view, line).unwrap();
        let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
        let expected = if line.ends_with("python") { Some("python") } else if line.ends_with("javascript") { Some("javascript") } else { None };
        assert_eq!(core.code_language_detection().unwrap().language.as_deref(), expected);
        assert_eq!(core.document().source_bytes(), b"let n = 1;\n");
    }
    core.stage_command_line(view, "filetype").unwrap();
    let output = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap().command.unwrap();
    assert!(messages(&output).iter().any(|message| message == "filetype detection:ON  plugin:OFF  indent:OFF"));
}

#[test]
fn explicit_keywords_preserve_editor_search_extraction_and_regex_boundaries() {
    for setting in ["set isk+=-,+", "set isk="] {
        let (mut c, mut d) = editor("-foo -fooX -foo");
        ex(&mut c, &mut d, setting);
        keys(&mut c, &mut d, "l*");
        assert_eq!(c.cursor(), "-foo -fooX -".len());
        keys(&mut c, &mut d, "n");
        assert_eq!(c.cursor(), 1);
        let result = ex(&mut c, &mut d, r"%s/\b-foo\b/MATCH/g");
        assert!(matches!(result.status, CommandStatus::ExError(_) | CommandStatus::Error(_)));
        assert_eq!(d.text(), "-foo -fooX -foo");
        ex(&mut c, &mut d, r"%s/\<foo\>/MATCH/g");
        assert_eq!(d.text(), "-MATCH -fooX -MATCH");
    }
    let (mut c, mut d) = editor("café café_ café");
    ex(&mut c, &mut d, "set isk=");
    keys(&mut c, &mut d, "*");
    assert_eq!(c.cursor(), "café café_ ".len());
}

#[test]
fn startofline_retains_column_after_linewise_edits_and_ex_addresses() {
    let (mut c, mut d) = editor("abcdef\n  ghijkl\n  mnopqr");
    ex(&mut c, &mut d, "set nosol sw=2 et");
    keys(&mut c, &mut d, "4l");
    ex(&mut c, &mut d, "2");
    assert_eq!(c.cursor(), "abcdef\n  gh".len());
    keys(&mut c, &mut d, ">>");
    assert_eq!(c.cursor(), "abcdef\n    ".len());
    keys(&mut c, &mut d, "dd");
    assert_eq!(c.cursor(), "abcdef\n  mn".len());
    assert_eq!(d.text(), "abcdef\n  mnopqr");
}

#[test]
fn whichwrap_flags_are_independent_and_wrapped_delete_counts_newline() {
    let (mut c, mut d) = editor("abc\ndef");
    ex(&mut c, &mut d, "set ww=l");
    keys(&mut c, &mut d, "$ ");
    assert_eq!(c.cursor(), 2, "space does not inherit the l flag");
    keys(&mut c, &mut d, "l");
    assert_eq!(c.cursor(), 4);
    ex(&mut c, &mut d, "set ww=h");
    keys(&mut c, &mut d, "dh");
    assert_eq!(d.text(), "abcdef");
    keys(&mut c, &mut d, "u0");
    ex(&mut c, &mut d, "set ww=~");
    keys(&mut c, &mut d, "4~");
    assert_eq!(d.text(), "ABC\nDef");
    assert_eq!(c.cursor(), 5);
}

#[test]
fn clipboard_combined_flags_duplicate_yanks_but_not_deletes() {
    let (mut c, mut d) = editor("one\ntwo");
    ex(&mut c, &mut d, "set cb=unnamed,unnamedplus");
    let only_clipboard = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
    keys(&mut c, &mut d, "y");
    let failed = c.handle_with_clipboard_context(&mut d, InputEvent::key('y'), &only_clipboard).unwrap();
    assert!(matches!(failed.status, CommandStatus::RegisterWriteError(_)));
    assert!(failed.clipboard_writes.is_empty());
    assert!(c.register('0').is_none());
    let deleted = c.handle_with_clipboard_context(&mut d, InputEvent::key('x'), &only_clipboard).unwrap();
    assert_eq!(deleted.status, CommandStatus::Complete);
    assert_eq!(deleted.clipboard_writes.len(), 1);
    assert_eq!(deleted.clipboard_writes[0].target(), ClipboardTarget::Clipboard);
    assert_eq!(d.text(), "ne\ntwo");
    assert_eq!(c.register('-').unwrap().text, "o");
}

#[test]
fn keyword_option_applies_to_deferred_visual_block_insert_deletion() {
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new("one\ntwo"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1000.);
    core.stage_command_line(view, "set isk+=-").unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
    let mut send = |key| core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    send(Key::Ctrl('v'));
    for ch in "jIfoo-bar".chars() { send(Key::Char(ch)); }
    send(Key::Ctrl('w'));
    send(Key::Char('X'));
    send(Key::Escape);
    assert_eq!(core.document().text(), "Xone\nXtwo");
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
    assert_eq!(core.document().text(), "one\ntwo");
}
