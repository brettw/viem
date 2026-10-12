//! Blank lines (spaces and tabs only) follow one rule wherever they occur:
//! each pair of separator endings is a paragraph boundary, an unpaired ending
//! is trivia, and a blank line's whitespace is never paragraph text. Edits in
//! the resulting empty paragraphs must reopen exactly as displayed.
use viem_core::command::InputEvent;
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn paragraphs(source: &str, format: Format) -> Vec<String> {
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    document.projection().blocks().iter()
        .map(|block| document.text()[block.range.clone()].to_owned()).collect()
}

fn shape(document: &Document) -> (String, Vec<String>) {
    let blocks = document.projection().blocks().iter()
        .map(|block| format!("{:?} {:?} {:?} q{}", block.range, block.kind, block.style, block.quote_depth))
        .collect();
    (document.text().to_owned(), blocks)
}

#[test]
fn blank_line_runs_count_pairs_at_the_start_middle_and_end() {
    for (source, expected) in [
        // Between blocks one blank line separates; each further pair keeps
        // an editable empty paragraph.
        ("a\n\nb", &["a", "b"][..]),
        ("a\n\n\n\nb", &["a", "", "b"]),
        ("a\n \n \n \nb", &["a", "", "b"]),
        ("a\n\t\n\n \nb", &["a", "", "b"]),
        // Before the first block every ending is surplus, as after the last.
        ("\nb", &["b"]),
        (" \nb", &["b"]),
        ("\t\nb", &["b"]),
        ("\n\nb", &["", "b"]),
        (" \n \nb", &["", "b"]),
        ("\n\n\nb", &["", "b"]),
        ("\n\n\n\nb", &["", "", "b"]),
        ("a\n", &["a"]),
        ("a\n ", &["a"]),
        ("a\n\n", &["a", ""]),
        ("a\n \n ", &["a", ""]),
        ("a\n\n\n\n", &["a", "", ""]),
        // A document of blank lines.
        ("", &[""]),
        ("   ", &[""]),
        ("  \n  ", &[""]),
        (" \n \n ", &["", ""]),
        // NBSP is content, not a blank line.
        ("\u{a0}\nb", &["\u{a0} b"]),
        ("# h\n\u{a0}\nb", &["h", "\u{a0} b"]),
    ] {
        assert_eq!(paragraphs(source, Format::Markdown), expected, "{source:?}");
    }
}

#[test]
fn source_view_keeps_unpaired_edge_lines_and_literal_whitespace() {
    for (source, expected) in [
        ("\nb", &["", "b"][..]),
        (" \nb", &[" ", "b"]),
        ("\n\nb", &["", "b"]),
        ("a\n", &["a", ""]),
    ] {
        assert_eq!(paragraphs(source, Format::MarkdownSource), expected, "{source:?}");
    }
}

#[test]
fn typing_into_empty_paragraphs_reopens_as_displayed() {
    for source in [
        "\n\nb", " \n \nb", "    \n    \nb", "\t\n\t\nb", "a\n\n\n\nb", "a\n \n \n \nb",
        "a\n    \n    \n    \nb", "a\n\n", "a\n\n  \n  ", "   ", "  \n  ", " \n \n ",
        "- a\n\n  \n\n  \n\n  b", "> a\n>\n>\n>\n> b",
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let empties = document.projection().blocks().iter()
            .filter(|block| block.range.is_empty()).map(|block| block.range.start).collect::<Vec<_>>();
        assert!(!empties.is_empty(), "{source:?} has no empty paragraph");
        for at in empties {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::text("x")))
                .unwrap_or_else(|error| panic!("{source:?} at {at}: {error:?}"));
            let mut expected = core.document().text().to_owned();
            assert_eq!(expected.remove(at), 'x', "{source:?} at {at}");
            let saved = core.document().source_bytes();
            let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(core.document()), shape(&fresh),
                "{source:?} at {at}: saved {:?}", String::from_utf8_lossy(&saved));
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?} at {at}");
        }
    }
}

fn reopens_as_displayed(core: &Core<MockTextMeasurementProvider>, context: &str) {
    let saved = core.document().source_bytes();
    let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(shape(core.document()), shape(&fresh), "{context}: saved {:?}", String::from_utf8_lossy(&saved));
}

#[test]
fn emptying_the_first_paragraph_keeps_its_boundary_like_any_other() {
    // The emptied paragraph's boundary needs a pair of endings at the start
    // of the document, as it does between two blocks.
    for (source, keys) in [
        ("text before\n| a | b |\n| - | - |\n| 1 | 2 |", "dip"),
        ("text\n1. list after text", "dip"),
        ("text\n- list right after text", "vipd"),
        ("text\n```\ncode\n```\ntext", "D"),
        ("a\n***\nb", "x"),
        ("X\n\nb", "dip"),
        ("a\n\ntext\n| a | b |\n| - | - |\n| 1 | 2 |", "jjdip"),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        for key in keys.chars() {
            core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
        }
        assert_ne!(core.document().source_bytes(), source.as_bytes(), "{source:?} {keys}");
        reopens_as_displayed(&core, &format!("{source:?} {keys}"));
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?} {keys}");
    }
}

#[test]
fn authored_syntax_in_blank_line_paragraphs_reopens_as_displayed() {
    for source in [" \n \nb", "   \n  \n", "a\n\n  \n  \n\nb"] {
        let at = {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            document.projection().blocks().iter().find(|block| block.range.is_empty()).unwrap().range.start
        };
        for action in ["`", "~", "- ", "link", "image"] {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            let expected = core.list_selection_identity(view).unwrap();
            let range = expected.range();
            match action {
                "link" => {
                    core.edit_link(view, expected, viem_core::document::LinkEditIntent::Insert {
                        range, text: "t".into(), destination: "https://e.com".into(),
                    }).unwrap_or_else(|error| panic!("{source:?} {action}: {error:?}"));
                }
                "image" => {
                    core.edit_image(view, expected, viem_core::document::ImageEditIntent::Insert {
                        range, text: "alt".into(), destination: "pic.png".into(),
                    }).unwrap_or_else(|error| panic!("{source:?} {action}: {error:?}"));
                }
                text => {
                    core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                        .unwrap_or_else(|error| panic!("{source:?} {action}: {error:?}"));
                }
            }
            assert_ne!(core.document().source_bytes(), source.as_bytes(), "{source:?} {action}");
            reopens_as_displayed(&core, &format!("{source:?} {action}"));
            core.handle(view, CoreEvent::Input(InputEvent::Key(viem_core::command::Key::Escape))).unwrap();
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?} {action}");
        }
    }
}
