use viem_core::command::{CommandInterpreter, InputEvent};
use viem_core::document::{Document, Encoding, Format};
#[test]
fn empty_markdown_list_accepts_whitespace() {
    for value in [" ", "\t"] {
        let mut document =
            Document::from_bytes(b"- ".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .handle(&mut document, InputEvent::key('i'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::text(value))
            .unwrap();
        assert_eq!(document.text(), value);
    }
}

#[test]
fn direct_empty_markdown_list_accepts_whitespace() {
    for value in [" ", "\t"] {
        let mut document =
            Document::from_bytes(b"- ".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        document.insert(0, value).unwrap();
        assert_eq!(document.text(), value);
    }
}

#[test]
fn list_body_whitespace_reopens_without_rewriting_labels_or_neighbor_source() {
    for source in ["- ", "-", "1. ", "- first\n  - ", "- item"] {
        for value in [" ", "\t", " \t", "  body", "&#32;", "&#9;"] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let block = document.projection().blocks().last().unwrap();
            let at = block.range.start;
            let mut expected = document.text().to_owned();
            expected.insert_str(at, value);
            document
                .insert(at, value)
                .unwrap_or_else(|error| panic!("{source:?} {value:?}: {error:?}"));
            assert_eq!(document.text(), expected);
            let changed = document.source_bytes();
            let reopened = Document::from_bytes(changed, Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn whitespace_references_are_literal_in_source_and_code() {
    let source = "- &#32;&#x09;";
    let formatted =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(formatted.text(), " \t");
    assert_eq!(formatted.source_bytes(), source.as_bytes());
    for (source, expected, format) in [
        (source, source, Format::MarkdownSource),
        ("`&#32;`", "&#32;", Format::Markdown),
        ("```\n&#32;\n```", "&#32;", Format::Markdown),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), expected);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn whitespace_in_large_list_preserves_unaffected_layout_and_block_identity() {
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let source = (0..10_000)
        .map(|index| {
            if index == 5000 {
                "- \n".to_owned()
            } else {
                format!("- Item {index}\n")
            }
        })
        .collect::<String>();
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document.projection().blocks()[5000].range.start;
    let later_id = document.projection().blocks()[9000].id;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600.0, 300.0);
    engine.set_cache_capacity(10_010);
    engine.relayout(&document, &mut view).unwrap();
    let requests = engine.provider().request_calls();
    document.insert(at, "\t ").unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(document.projection().blocks()[9000].id, later_id);
    assert!(engine.provider().request_calls() - requests <= 2);
    assert_eq!(document.text()[at..at + 2].as_bytes(), b"\t ");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn list_continuation_body_whitespace_matches_reopened_projection() {
    let source = "- ```\n  A\n  ```";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    document.insert(4, " ").unwrap();
    assert_eq!(document.text(), "```  A\n");
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(reopened.text(), document.text());
}
