//! Vim script through the core coordinator: typed command lines, `:source`,
//! editor builtins, nested `:normal`, messages and undo grouping.
use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, CoreOutcome, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let document =
        Document::from_bytes_with_file_format(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText, FileFormat::Unix)
            .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    (core, view)
}

fn press(core: &mut Editor, view: ViewId, event: InputEvent) -> CoreOutcome {
    core.handle_with_layout(view, CoreEvent::Input(event)).unwrap()
}

/// Type `:{line}<CR>` and return the outcome of Enter.
fn ex(core: &mut Editor, view: ViewId, line: &str) -> CoreOutcome {
    press(core, view, InputEvent::Key(Key::Char(':')));
    for character in line.chars() {
        press(core, view, InputEvent::Key(Key::Char(character)));
    }
    press(core, view, InputEvent::Key(Key::Enter))
}

fn messages(outcome: &CoreOutcome) -> Vec<String> {
    outcome
        .command
        .as_ref()
        .and_then(|command| command.ex_outcome.as_ref())
        .map(|ex| {
            ex.frontend_requests
                .iter()
                .filter_map(|request| match request {
                    ExFrontendRequest::Info(ExInfoRequest::Message(text)) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn insert_abbreviations_expand_only_newly_typed_words_and_undo_together() {
    let (mut core, view) = editor("teh");
    ex(&mut core, view, "iabbrev teh the");
    press(&mut core, view, InputEvent::key('A'));
    press(&mut core, view, InputEvent::key(' '));
    for c in "teh".chars() { press(&mut core, view, InputEvent::key(c)); }
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().text(), "teh the");
    press(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().text(), "teh");
}

#[test]
fn abbreviation_shapes_custom_keywords_expression_and_command_line() {
    let (mut core, view) = editor("");
    ex(&mut core, view, "iabbrev #i include");
    ex(&mut core, view, "iabbrev <expr> teh toupper('the')");
    ex(&mut core, view, "set iskeyword+=-");
    ex(&mut core, view, "iabbrev foo-bar joined");
    press(&mut core, view, InputEvent::key('i'));
    for c in "x#i #i teh foo-bar ".chars() { press(&mut core, view, InputEvent::key(c)); }
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().text(), "xinclude include THE joined ");
    ex(&mut core, view, "cabbrev say echo");
    assert_eq!(messages(&ex(&mut core, view, "say 'expanded'")), vec!["expanded"]);
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('teh', 'i', 1, 1).expr")), vec!["1"]);
}

#[test]
fn buffer_local_maps_and_abbreviations_preserve_global_definitions() {
    let (mut core, view) = editor("");
    ex(&mut core, view, "inoremap q G");
    ex(&mut core, view, "inoremap <buffer> q L");
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('q', 'i')")), vec!["L"]);
    ex(&mut core, view, "iunmap <buffer> q");
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('q', 'i')")), vec!["G"]);
    ex(&mut core, view, "iabbrev teh global");
    ex(&mut core, view, "iabbrev <buffer> teh local");
    press(&mut core, view, InputEvent::key('i'));
    for c in "teh ".chars() { press(&mut core, view, InputEvent::key(c)); }
    press(&mut core, view, InputEvent::Key(Key::Escape));
    assert_eq!(core.document().text(), "local ");
    ex(&mut core, view, "iunabbrev <buffer> teh");
    assert_eq!(messages(&ex(&mut core, view, "echo maparg('teh', 'i', 1)")), vec!["global"]);
}

#[test]
fn abbreviation_remaps_rhs_and_control_bracket_only_expands() {
    let (mut core, view) = editor("");
    ex(&mut core, view, "imap x X");
    ex(&mut core, view, "iabbrev aa x");
    press(&mut core, view, InputEvent::key('i'));
    press(&mut core, view, InputEvent::key('a'));
    press(&mut core, view, InputEvent::key('a'));
    press(&mut core, view, InputEvent::Key(Key::Ctrl(']')));
    assert_eq!(core.document().text(), "X");
    press(&mut core, view, InputEvent::Key(Key::Escape));
}
