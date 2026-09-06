use evim_core::command::{CommandStatus, InputEvent, Key};
use evim_core::document::{Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, Document, ViewId};

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    let status = outcome.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending),
        "{key:?}: {status:?}"
    );
}

#[test]
fn coordinator_routes_authored_html_completion_through_one_atomic_insert_group() {
    let mut core =
        Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::HtmlSource).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    key(&mut core, view, Key::Char('i'));
    for (input, expected, cursor) in [("<", "<>", 1), ("b", "<b></b>", 2), ("r", "<br>", 3)] {
        let result = core
            .handle(view, CoreEvent::Input(InputEvent::text(input)))
            .unwrap();
        assert!(result.document_changed);
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    }
    key(&mut core, view, Key::Backspace);
    assert_eq!(core.document().text(), "<b></b>");
    key(&mut core, view, Key::Char('>'));
    core.handle(view, CoreEvent::Input(InputEvent::text("hello")))
        .unwrap();
    assert_eq!(core.document().text(), "<b>hello</b>");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), b"");
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), b"<b>hello</b>");
}

#[test]
fn smart_quotes_are_view_input_policy_and_do_not_rewrite_source_on_toggle() {
    let mut core = Core::new(Document::new("word"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    let revision = core.document().revision();
    core.handle(first, CoreEvent::SetSmartQuotes(true)).unwrap();
    assert_eq!(core.document().revision(), revision);
    assert!(!core.document().is_dirty());
    assert!(core.command_state(first).unwrap().smart_quotes());
    assert!(!core.command_state(second).unwrap().smart_quotes());
    key(&mut core, first, Key::Char('i'));
    core.handle(first, CoreEvent::Input(InputEvent::text("\"")))
        .unwrap();
    assert_eq!(core.document().text(), "“word");
    key(&mut core, first, Key::Escape);
    key(&mut core, first, Key::Char('u'));
    assert_eq!(core.document().text(), "word");
    key(&mut core, second, Key::Char('i'));
    core.handle(second, CoreEvent::Input(InputEvent::text("\"")))
        .unwrap();
    assert_eq!(core.document().text(), "\"word");
}

#[test]
fn source_completion_preserves_original_encoding_and_mixed_ending_bytes() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        let text = "<p>first</p>\r\n<p>last</p>\n";
        let original = match encoding {
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => text.as_bytes().to_vec(),
        };
        let mut core = Core::new(
            Document::from_bytes(original.clone(), encoding, Format::HtmlSource).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
        key(&mut core, view, Key::Char('i'));
        for character in "<b>x".chars() {
            key(&mut core, view, Key::Char(character));
        }
        key(&mut core, view, Key::Escape);
        let reopened =
            Document::from_bytes(core.document().source_bytes(), encoding, Format::HtmlSource)
                .unwrap();
        assert_eq!(reopened.text(), core.document().text());
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), original);
    }
}
