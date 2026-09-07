use evim_core::command::{InputEvent, Key};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreError, CoreEvent, Document, DocumentError, Encoding, Format};

#[test]
fn html_nul_typing_returns_an_explicit_policy_without_mutating_source() {
    let original = b"<p>hello</p>".to_vec();
    let document = Document::from_bytes(original.clone(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    core.handle(view, CoreEvent::Input(InputEvent::key('A'))).unwrap();
    let revision = core.document().revision();
    assert_eq!(
        core.handle(view, CoreEvent::Input(InputEvent::text("\0"))),
        Err(CoreError::Document(DocumentError::UnrepresentableFormattedCharacter {
            format: Format::Html,
            character: '\0',
        }))
    );
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), original);
    assert_eq!(core.document().text(), "hello");
    core.handle(view, CoreEvent::Input(InputEvent::text("!"))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    assert_eq!(core.document().text(), "hello!");
}

#[test]
fn html_source_and_plain_text_still_preserve_literal_nul_bytes() {
    for format in [Format::PlainText, Format::HtmlSource] {
        let mut document = Document::from_bytes(b"a".to_vec(), Encoding::Utf8, format).unwrap();
        document.insert(1, "\0").unwrap();
        assert_eq!(document.source_bytes(), b"a\0");
        assert_eq!(document.text(), "a\0");
    }
}

#[test]
fn html_join_reuses_collapsible_source_whitespace_exposed_by_break_removal() {
    for source in ["<p>prose<br>\n\0-next</p>", "<p>prose <br>\n\0-next</p>"] {
        let original = source.as_bytes().to_vec();
        let mut document =
            Document::from_bytes(original.clone(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(document.text(), "prose\n-next");
        let mut commands = evim_core::command::CommandInterpreter::new();
        assert!(commands.set_cursor(&document, 0));
        commands
            .handle(&mut document, InputEvent::key('J'))
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(document.text(), "prose -next");
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), document.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
    }
}
