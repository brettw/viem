use viem_core::command::{InputEvent, Key};
use viem_core::document::{Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

#[test]
fn empty_paragraph_owns_typing_after_an_empty_container() {
    for prefix in [
        "<div></div>", "<span></span>", "<div><b></b></div>",
        "<div style='white-space:pre'></div>", "<b></b><!--keep-->",
    ] {
        for body in ["", "tail"] {
            let source = format!("{prefix}<p>{body}</p>");
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(document.text(), body);
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::text("hello")))
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
            let expected = format!("{prefix}<p>hello{body}</p>");
            assert_eq!(core.document().source_bytes(), expected.as_bytes());
            let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(reopened.text(), format!("hello{body}"));
            for at in 0..reopened.text().len() {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false),
                    DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false),
                );
            }
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn new_html_typing_authors_a_paragraph() {
    let mut core = Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Html).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    for text in ["h", "ello"] {
        core.handle(view, CoreEvent::Input(InputEvent::text(text))).unwrap();
    }
    assert_eq!(core.document().source_bytes(), b"<p>hello</p>");
}

#[test]
fn preceding_empty_whitespace_scope_does_not_change_paragraph_typing() {
    let source = "<div style='white-space:pre'></div><p></p>";
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text(" "))).unwrap();
    assert_eq!(core.document().source_bytes(), b"<div style='white-space:pre'></div><p>&nbsp;</p>");
    assert_eq!(core.document().text(), "\u{a0}");
    let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), core.document().text());
}
