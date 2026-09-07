use evim_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use evim_core::document::{
    Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn typing_after_trailing_empty_elements_and_list_exit_keeps_exact_projection() {
    for source in [
        "",
        "<p></p>",
        "<html><body></body></html>",
        "<!doctype html><html><head><title>Title</title></head><body><!--keep--></body></html>",
        "<p>first</p>",
        "<p>first</p>\n",
        "<p>first</p><p></p>",
        "<p>first<br></p>",
        "<p>first<br><br></p>",
        "<p>first</p><br>",
        "<p>first</p><br><br>",
        "<p>first</p><p><br></p>",
        "<!doctype html><html><head><title>Title</title></head><body><p>first</p></body></html>\n",
        "<body><p>first</p><p></p></body>",
        "<body><p>first<br></p></body>\n",
        "<body><p>first</p><br></body>\n",
        "<body><p>first</p><!--tail--></body>",
        "<p>first</p>tail",
        "<p>first</p>\n<!--tail-->",
        "<ul><li>first</li></ul>",
        "<ul><li>first</li><li></li></ul>",
        "<p>first<br></p><ul></ul>",
        "<p>first<br></p><div></div>",
        "<p>first<br></p><span></span>",
        "<p>first</p><div></div>",
        "<p>first</p><span></span>",
        "<p>first</p><ul></ul>",
        "<div></div>",
        "<pre>first\n</pre>",
        "<p><b>first</b></p>",
    ] {
        for enters in 0..=3 {
            let mut document = html(source);
            let mut commands = CommandInterpreter::new();
            let mut events = vec![InputEvent::key('G'), InputEvent::key('A')];
            events.extend((0..enters).map(|_| InputEvent::Key(Key::Enter)));
            events.push(InputEvent::text("-"));
            for event in events {
                let outcome = commands
                    .handle(&mut document, event.clone())
                    .unwrap_or_else(|error| {
                        panic!("{source:?} enters={enters} event={event:?}: {error:?}")
                    });
                assert!(
                    matches!(
                        outcome.status,
                        CommandStatus::Complete | CommandStatus::Pending
                    ),
                    "{source:?} enters={enters} event={event:?}: {:?}",
                    outcome.status
                );
            }
            let changed = document.source_bytes();
            let reopened = html(&String::from_utf8(changed.clone()).unwrap());
            assert_eq!(
                reopened.text(),
                document.text(),
                "{source:?} enters={enters}"
            );
            assert!(document.text().ends_with('-'), "{source:?} enters={enters}");
            commands
                .handle(&mut document, InputEvent::Key(Key::Escape))
                .unwrap();
            assert!(document.undo(), "{source:?} enters={enters}");
            assert_eq!(
                document.source_bytes(),
                source.as_bytes(),
                "{source:?} enters={enters}"
            );
            assert!(document.redo());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}

#[test]
fn paragraph_splits_preserve_visible_whitespace_at_both_edges() {
    for source in [
        "<p>First paragraph.</p><p>Second paragraph.</p>",
        "<p>Second \n\t paragraph.</p>",
        "<p>Second<b> </b>paragraph.</p>",
        "<p>Second <b>paragraph.</b></p>",
        "<p><b>Second</b> paragraph.</p>",
        "<p>Second&#32;paragraph.</p>",
        "<p style='white-space:pre-wrap'>Second \t paragraph.</p>",
    ] {
        let original = html(source).text().to_owned();
        for (offset, c) in original
            .char_indices()
            .filter(|(_, c)| matches!(c, ' ' | '\t'))
        {
            for at in [offset, offset + c.len_utf8()] {
                let mut document = html(source);
                document
                    .apply_model_request(evim_core::document::ModelRequest::ContinueList {
                        document: document.id(),
                        revision: document.revision(),
                        at,
                    })
                    .unwrap_or_else(|error| panic!("{source:?} at {at}: {error:?}"));
                let mut expected = original.clone();
                expected.insert(at, '\n');
                assert_eq!(document.text(), expected, "{source:?} at {at}");
                let bytes = document.source_bytes();
                assert_eq!(
                    html(&String::from_utf8(bytes.clone()).unwrap()).text(),
                    expected
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), bytes);
            }
        }
    }
}

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn trailing_empty_source_keeps_its_bytes_and_does_not_steal_typing_context() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (body, expected_body) in [
            ("<p>first</p>", "<p>first-</p>"),
            ("<p>first<br></p>", "<p>first<br>-</p>"),
            ("<p><b>first</b></p>", "<p><b>first-</b></p>"),
            ("<p>first<b></b></p>", "<p>first<b>-</b></p>"),
            ("<p>first</p><p><b></b></p>", "<p>first</p><p><b>-</b></p>"),
        ] {
            let tail = "<div data-keep='unchanged'><span></span></div><ul></ul><!--tail-->\r\n</body></html>\r\n";
            let source = format!("<html><body>{body}{tail}");
            let original = encode(&source, encoding);
            let mut document =
                Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
            let before = document.text().to_owned();
            document
                .insert(before.len(), "-")
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
            assert_eq!(document.text(), format!("{before}-"));
            assert_eq!(
                document.source_bytes(),
                encode(&format!("<html><body>{expected_body}{tail}"), encoding)
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert_eq!(document.text(), format!("{before}-"));
        }
    }
}

#[test]
fn eof_typing_keeps_large_document_projection_and_layout_local() {
    use evim_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let source = "<p>Unchanged paragraph.</p>\n".repeat(10_000)
        + "<p>Tail</p><div><span></span></div><!--keep-->";
    let mut document = html(&source);
    let at = document.text().len();
    let unaffected = document.projection().blocks()[5000].id;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(10_010);
    let mut view = ViewLayout::new(600., 300.);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "-")],
        })
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(prepared.summary().source_patches()[0].replacement(), b"-");
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[5000].id, unaffected);
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
    assert_eq!(
        document.source_bytes(),
        source.replace("<p>Tail</p>", "<p>Tail-</p>").as_bytes()
    );
    assert_eq!(view.snapshot().unwrap().rows.len(), 10_001);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
}
