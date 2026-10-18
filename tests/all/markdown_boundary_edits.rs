//! Empty paragraph boundaries retain their source ownership across regional
//! edits, including encoded separators and opposite caret affinities.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, FileFormat, Format, HistoryNavigationRequest,
    ModelRequest, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

const FIXTURES: &[&str] = &[
    "\n\nb",
    "a\n\n\n\nb",
    "a\n \n\t\n \nb",
    "## h\n\n\n\npara",
    "a<br>\n\n\n\nb",
    "> a\n>\n>\n>\n> b",
    "- a\n\n  \n\n  \n\n  b",
    "\n\n***\n\nb",
];
const FORMATS: &[(Encoding, FileFormat)] = &[
    (Encoding::Utf8, FileFormat::Unix),
    (Encoding::Utf16Le, FileFormat::Dos),
    (Encoding::Utf16Be, FileFormat::Dos),
];

fn encoded(source: &str, encoding: Encoding, file_format: FileFormat) -> Vec<u8> {
    let source = source.replace('\n', if file_format == FileFormat::Dos { "\r\n" } else { "\n" });
    match encoding {
        Encoding::Utf16Le => [vec![0xff, 0xfe], source.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
        Encoding::Utf16Be => [vec![0xfe, 0xff], source.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
        _ => source.into_bytes(),
    }
}

fn open(bytes: Vec<u8>, encoding: Encoding, file_format: FileFormat) -> Document {
    Document::from_bytes_with_file_format(bytes, encoding, Format::Markdown, file_format).unwrap()
}

fn empty_paragraphs(document: &Document) -> Vec<usize> {
    document.projection().blocks().iter()
        .filter(|block| block.range.is_empty() && !block.thematic_break)
        .map(|block| block.range.start).collect()
}

fn shape(document: &Document) -> (String, Vec<viem_core::document::Block>, Vec<viem_core::document::StyleSpan>) {
    // BlockAttributes equality compares semantic container membership,
    // including starts/ends and declarations, without transient owner IDs.
    let blocks = document.projection().blocks().iter().map(|block| {
        let mut block = block.clone(); block.id = 0; block
    }).collect();
    (document.text().to_owned(), blocks, document.projection().style_spans().to_vec())
}

fn check_reopen(document: &Document, encoding: Encoding, file_format: FileFormat, context: &str) {
    let reopened = open(document.source_bytes(), encoding, file_format);
    assert_eq!(shape(document), shape(&reopened), "{context}: saved source must reopen with the same paragraph ownership");
    assert_eq!(document.hard_line_snapshot().capture(0..document.text().len()).unwrap().break_offsets(),
        reopened.hard_line_snapshot().capture(0..reopened.text().len()).unwrap().break_offsets(), "{context}");
}

#[test]
fn typing_in_empty_paragraphs_honors_both_affinities_and_encoded_separators() {
    let mut failures = Vec::new();
    for source in FIXTURES {
        for &(encoding, file_format) in FORMATS {
            let bytes = encoded(source, encoding, file_format);
            let original = open(bytes.clone(), encoding, file_format);
            let empties = empty_paragraphs(&original);
            assert!(!empties.is_empty(), "fixture has no empty paragraph: {source:?}");
            for at in empties {
                for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                    let context = format!("{source:?} {encoding:?} {file_format:?} at {at} {affinity:?}");
                    let mut core = Core::new(open(bytes.clone(), encoding, file_format));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
                    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
                    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                        text_offset: at, affinity, extend_selection: false }).unwrap();
                    let mut expected = original.text().to_owned(); expected.insert(at, 'x');
                    if let Err(error) = core.handle(view, CoreEvent::Input(InputEvent::text("x"))) {
                        failures.push(format!("{context}: {error:?}")); continue;
                    }
                    assert_eq!(core.document().text(), expected, "{context}");
                    assert_eq!(core.command_state(view).unwrap().cursor(), at + 1, "{context}: typing follows inserted text");
                    assert!(core.document().projection().blocks().iter().any(|block|
                        block.range.start == at && at < block.range.end), "{context}: insertion stays in its empty owner");
                    check_reopen(core.document(), encoding, file_format, &context);
                    let saved = core.document().source_bytes();
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
                    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                    assert_eq!(core.document().source_bytes(), bytes, "{context}: exact undo");
                    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                    assert_eq!(core.document().source_bytes(), saved, "{context}: exact redo");
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} typing failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn deleting_or_replacing_empty_paragraph_separators_preserves_projection_and_history() {
    let mut failures = Vec::new();
    for source in FIXTURES {
        for &(encoding, file_format) in FORMATS {
            let bytes = encoded(source, encoding, file_format);
            let original = open(bytes.clone(), encoding, file_format);
            for at in empty_paragraphs(&original) {
                let before = at.checked_sub(1).filter(|&start| original.text().as_bytes()[start] == b'\n').map(|start| start..at);
                let after = (original.text().as_bytes().get(at) == Some(&b'\n')).then_some(at..at + 1);
                for range in before.into_iter().chain(after) {
                    for replacement in ["", "x"] {
                        let context = format!("{source:?} {encoding:?} {file_format:?} {range:?} -> {replacement:?}");
                        let mut document = open(bytes.clone(), encoding, file_format);
                        let mut expected = original.text().to_owned(); expected.replace_range(range.clone(), replacement);
                        let prepared = match document.prepare_model_request(ModelRequest::ApplyTextEdits {
                            document: document.id(), revision: document.revision(), edits: vec![TextEdit::new(range.clone(), replacement)]
                        }) {
                            Ok(prepared) => prepared,
                            Err(error) => { failures.push(format!("{context}: {error:?}")); continue; }
                        };
                        assert_eq!(document.source_bytes(), bytes, "{context}: preparation is read-only");
                        let mut patched = bytes.clone();
                        for patch in prepared.summary().source_patches().iter().rev() {
                            patched.splice(patch.range(), patch.replacement().iter().copied());
                        }
                        document.commit_model_transaction(prepared).unwrap();
                        assert_eq!(document.text(), expected, "{context}");
                        assert_eq!(document.source_bytes(), patched, "{context}: only declared source patches change");
                        if *source == "a\n \n\t\n \nb" {
                            assert!(document.projection().blocks().iter().all(|block| block.style.0 == "Paragraph"),
                                "{context}: hidden blank-line whitespace must not turn retained prose into code");
                            assert!(!document.projection().style_spans().iter().any(|span|
                                span.application == viem_core::document::StyleApplication::Semantic(
                                    viem_core::document::SemanticInlineStyle::Code)), "{context}");
                        }
                        check_reopen(&document, encoding, file_format, &context);
                        let saved = document.source_bytes();
                        assert!(document.undo(), "{context}");
                        assert_eq!(document.source_bytes(), bytes, "{context}: exact undo");
                        assert_eq!(shape(&document), shape(&original), "{context}: undo restores paragraph ownership");
                        assert!(document.redo(), "{context}");
                        assert_eq!(document.source_bytes(), saved, "{context}: exact redo");
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} separator edit failures:\n{}", failures.len(), failures.join("\n"));
}
