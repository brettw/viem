use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::{Document, Encoding, Format, FormattedPayloadEdit, FormattedPayloadEditRequest,
    FormattedTextPayload, ModelRequest, TextEdit};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn assert_history(document: &mut Document, original: &[u8], expected: &[u8]) {
    assert_eq!(document.source_bytes(), expected);
    let reopened = Document::from_bytes(expected.to_vec(), document.encoding(), Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
    assert_eq!(reopened.projection().blocks().len(), document.projection().blocks().len());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected);
}

#[test]
fn typing_materializes_only_the_current_anonymous_paragraph() {
    for (source, at, expected) in [
        ("", 0, "<p>X</p>"),
        ("<!--keep-->", 0, "<!--keep--><p>X</p>"),
        ("<body></body>", 0, "<body><p>X</p></body>"),
        ("<html><body></body></html>", 0, "<html><body><p>X</p></body></html>"),
        ("<!doctype html><html><head><title>Title</title></head><body><!--keep--></body></html>", 0,
         "<!doctype html><html><head><title>Title</title></head><body><p>X</p><!--keep--></body></html>"),
        ("<div></div>", 0, "<div><p>X</p></div>"),
        ("<div>A</div><div>B</div>", 1, "<div><p>AX</p></div><div>B</div>"),
        ("<div>A</div><div>B</div>", 2, "<div>A</div><div><p>XB</p></div>"),
        ("<!--before--><div data-x='keep'><b>A</b><!--tail--></div>", 1,
         "<!--before--><div data-x='keep'><p><b>AX</b></p><!--tail--></div>"),
        ("<p>A</p><i>B</i><!--tail--><p>C</p>", 2,
         "<p>A</p><p><i>XB</i></p><!--tail--><p>C</p>"),
        ("<div><span title='<p>fake</p>'>A</span></div>", 0,
         "<div><p><span title='<p>fake</p>'>XA</span></p></div>"),
    ] {
        let mut document = html(source);
        document.insert(at, "X").unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_history(&mut document, source.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn existing_paragraph_owners_keep_their_original_source() {
    for source in ["<p>A</p>", "<h2>A</h2>", "<pre>A</pre>", "<ul><li>A</li></ul>",
        "<div class='viem-p-506172616772617068'>A</div>", "<p><b>A</b></p>"] {
        let mut document = html(source);
        document.insert(1, "X").unwrap();
        assert_eq!(document.source_bytes(), source.replace('A', "AX").as_bytes());
    }
}

#[test]
fn anonymous_replacement_with_hard_break_retains_neighboring_empty_paragraphs() {
    for (source, range, expected) in [
        ("<div>a</div><p></p>", 0..1, "<div><p><br></p></div><p></p>"),
        ("<p></p><div>a</div>", 1..2, "<p></p><div><p><br></p></div>"),
        ("<div>a<p></p></div>", 0..1, "<div><p><br></p><p></p></div>"),
    ] {
        let mut document = html(source);
        document.apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(), revision: document.revision(), edits: vec![TextEdit::new(range, "\n")],
        }).unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_history(&mut document, source.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn command_typing_creates_paragraph_in_the_same_undo_unit() {
    let mut document = html("");
    let mut command = CommandInterpreter::new();
    for event in [InputEvent::key('i'), InputEvent::text("Hello"), InputEvent::Key(Key::Escape)] {
        command.handle(&mut document, event).unwrap();
    }
    assert_history(&mut document, b"", b"<p>Hello</p>");
}

#[test]
fn paragraph_materialization_preserves_generated_space_identity() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |text: &str| match encoding {
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<_>>(),
            _ => text.as_bytes().to_vec(),
        };
        let mut document = Document::from_bytes(vec![], encoding, Format::Html).unwrap();
        let mut command = CommandInterpreter::new();
        command.handle(&mut document, InputEvent::key('i')).unwrap();
        command.handle(&mut document, InputEvent::text("A ")).unwrap();
        assert_eq!(document.source_bytes(), encode("<p>A&nbsp;</p>"));
        command.handle(&mut document, InputEvent::text("B")).unwrap();
        command.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        assert_eq!(document.text(), "A B");
        assert_history(&mut document, b"", &encode("<p>A B</p>"));
    }
}

#[test]
fn materialization_retains_source_encoding_and_envelope() {
    for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |text: &str| match encoding {
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<_>>(),
            _ => text.as_bytes().to_vec(),
        };
        let original = encode("<!DOCTYPE html><html><head><title>Keep</title></head><body><div>A</div><!--tail--></body></html>");
        let expected = encode("<!DOCTYPE html><html><head><title>Keep</title></head><body><div><p>AX</p></div><!--tail--></body></html>");
        let mut document = Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
        document.insert(1, "X").unwrap();
        assert_history(&mut document, &original, &expected);
    }
}

#[test]
fn empty_envelope_typing_stays_inside_the_body_and_reopens_with_exact_history() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |text: &str| match encoding {
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<_>>(),
            _ => text.as_bytes().to_vec(),
        };
        for source in ["<html><body></body></html>", "<!doctype html><html><head><title>Title</title></head><body><!--keep--></body></html>",
            "<BODY class='body'><!--before--><script>hidden</script><!--after--></BODY>"] {
            let original = encode(source);
            let mut document = Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
            let mut command = CommandInterpreter::new();
            for event in [InputEvent::key('i'), InputEvent::text("X"), InputEvent::Key(Key::Escape)] {
                command.handle(&mut document, event).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            }
            let saved = document.source_bytes();
            assert_eq!(document.text(), "X");
            let decoded = match encoding {
                Encoding::Utf16Le => String::from_utf16(&saved.chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>()).unwrap(),
                Encoding::Utf16Be => String::from_utf16(&saved.chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect::<Vec<_>>()).unwrap(),
                _ => String::from_utf8(saved.clone()).unwrap(),
            };
            assert!(decoded.contains("<p>X</p>"), "{decoded}");
            assert!(decoded.find("<p>").unwrap() > decoded.to_ascii_lowercase().find("<body").unwrap());
            assert_history(&mut document, &original, &saved);
        }
    }
}

#[test]
fn payload_replacement_materializes_anonymous_paragraph() {
    let source = "<div><b>AB</b></div><p>tail</p>";
    let mut document = html(source);
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "XY", vec![]).unwrap();
    let prepared = document.prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
        document.id(), document.revision(), vec![FormattedPayloadEdit::new(0..1, payload)],
    )).unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_history(&mut document, source.as_bytes(), b"<div><p><b>XYB</b></p></div><p>tail</p>");
}

#[test]
fn typing_next_to_atomic_table_does_not_create_invalid_paragraph_children() {
    let mut document = html("<table><tr><td>cell</td></tr></table>");
    document.insert(0, "X").unwrap();
    assert!(!String::from_utf8(document.source_bytes()).unwrap().contains("<p><table"));
    let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
}

#[test]
fn owned_paragraph_typing_in_large_document_remains_regional() {
    use viem_core::document::ProjectionWorkScope;
    let source = "<p><b>unchanged</b></p>".repeat(10_000);
    let mut document = html(&source);
    let before = document.projection().blocks()[5_000].id;
    let at = document.text().len();
    let prepared = document.prepare_model_request(ModelRequest::ApplyTextEdits {
        document: document.id(), revision: document.revision(), edits: vec![TextEdit::new(at..at, "X")],
    }).unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(prepared.summary().projection_work().scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[5_000].id, before);
}
