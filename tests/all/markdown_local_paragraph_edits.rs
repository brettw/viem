//! Splitting and joining ordinary paragraphs in formatted Markdown is local:
//! it must not reproject or flatten the whole document, and the result must
//! reopen exactly as displayed.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{measure_document_work, BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn shape(document: &Document) -> (String, Vec<String>) {
    let blocks = document.projection().blocks().iter()
        .map(|block| format!("{:?} {:?} {:?} q{}", block.range, block.kind, block.style, block.quote_depth))
        .collect();
    (document.text().to_owned(), blocks)
}

fn reopens_as_displayed(core: &Core<MockTextMeasurementProvider>, context: &str) {
    let saved = core.document().source_bytes();
    let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(shape(core.document()), shape(&fresh), "{context}: saved {:?}", String::from_utf8_lossy(&saved));
}

fn document(blocks: usize, middle: &str) -> String {
    let mut source = String::new();
    for index in 0..blocks {
        source.push_str(&format!("# Heading {index}\n\nParagraph {index} with several plain words.\n\n```\ncode {index}\n```\n\n"));
        if index == blocks / 2 {
            source.push_str(middle);
        }
    }
    source
}

#[test]
fn splitting_and_joining_independent_paragraphs_stays_regional() {
    let middle = "First half of the edited paragraph and its second half.\n\n```\ncode\n```\n\n";
    let source = document(400, middle);
    let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = original.text().find("and its second").unwrap();
    let mut core = Core::new(original);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    for (label, key) in [("Enter", Key::Enter), ("Backspace", Key::Backspace)] {
        let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::Key(key))));
        result.unwrap();
        assert_eq!(work.full_projection_candidates, 0, "{label}: {work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{label}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{label}: {work:?}");
        reopens_as_displayed(&core, label);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    for _ in 0..2 {
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn paragraphs_beside_lists_and_quotes_split_and_join_consistently() {
    for middle in [
        "- item\n- item\n\nAfter the list there is a paragraph here.\n\n",
        "Before the list there is a paragraph here.\n\n- item\n- item\n\n",
        "> quoted\n\nAfter the quote there is a paragraph here.\n\n",
        "    indented code\n\nAfter the code there is a paragraph here.\n\n",
        "Text before the rule there is a paragraph here.\n\n***\n\n",
    ] {
        let source = document(4, middle);
        let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find("there is").unwrap();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in [Key::Enter, Key::Backspace] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            reopens_as_displayed(&core, &format!("{middle:?} {key:?}"));
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{middle:?}");
    }
}

#[test]
fn enter_after_an_atx_heading_stays_regional_and_setext_stays_consistent() {
    for (middle, regional) in [
        ("## Section heading\n\nBody of the section.\n\n", true),
        ("Section heading\n---------------\n\nBody of the section.\n\n", false),
    ] {
        let source = document(300, middle);
        let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find("Section heading").unwrap() + "Section heading".len();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))));
        result.unwrap();
        if regional {
            assert_eq!(work.full_projection_candidates, 0, "{middle:?}: {work:?}");
        }
        reopens_as_displayed(&core, middle);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{middle:?}");
    }
}

#[test]
fn opening_beside_headings_without_blank_lines_keeps_new_paragraphs_separate() {
    // An ATX heading may interrupt a paragraph or be followed directly by
    // one; a single ending then separates them, and new paragraphs opened
    // there must not become lazy continuations.
    for (source, target, key) in [
        ("#heading-ish\n#\t tab heading", "tab", 'O'),
        ("#heading-ish\n#\t tab heading", "heading-ish", 'o'),
        ("# Heading\nbody text", "body", 'O'),
        ("# Heading\nbody text", "Heading", 'o'),
        ("body text\n# Heading", "body", 'o'),
    ] {
        let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find(target).unwrap();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in [key, 'x'] {
            core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
            reopens_as_displayed(&core, &format!("{source:?} {key}"));
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        reopens_as_displayed(&core, &format!("{source:?} Escape"));
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?}");
    }
}
