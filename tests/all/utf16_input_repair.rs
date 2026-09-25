use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, ConversionWarning, Document, DocumentError, Encoding, FileFormat, Format,
    FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload, ModelRequest,
    ModelTransactionError, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    text.encode_utf16().flat_map(|unit| match encoding {
        Encoding::Utf16Le => unit.to_le_bytes(),
        Encoding::Utf16Be => unit.to_be_bytes(),
        _ => unreachable!(),
    }).collect()
}

fn fixture(prefix: &str, encoding: Encoding, format: Format) -> (Document, Vec<u8>) {
    let mut source = encoded(prefix, encoding);
    source.push(0x61);
    let document = Document::from_bytes_with_file_format(
        source.clone(), encoding, format, FileFormat::Unix,
    ).unwrap();
    (document, source)
}

#[test]
fn append_preserves_visible_diagnostic_and_repairs_only_its_source_byte() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        // WYSIWYG HTML also authors the anonymous paragraph's `p` owner, so its
        // patches are not confined to the repaired byte; HTML Source covers it.
        for format in [Format::PlainText, Format::Code, Format::Markdown, Format::MarkdownSource, Format::HtmlSource] {
            let (mut document, original) = fixture("a", encoding, format);
            let before = document.text().to_owned();
            assert_eq!(before, "a\u{fffd}");
            let at = before.len();
            let prepared = document.prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(), revision: document.revision(),
                edits: vec![TextEdit::new(at..at, "😀x")],
            }).unwrap_or_else(|error| panic!("{encoding:?}/{format:?}: {error:?}"));
            assert_eq!(prepared.summary().conversion_warnings(), &[ConversionWarning::RepairedTruncatedUtf16]);
            assert_eq!(prepared.summary().formatted_splices().len(), 1);
            assert!(prepared.summary().source_patches().iter().all(|patch| patch.range().start >= original.len() - 1));
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), "a\u{fffd}😀x");
            assert!(document.decoding_diagnostics().is_empty());
            let saved = document.source_bytes();
            assert_eq!(&saved[..original.len() - 1], &original[..original.len() - 1]);
            let reopened = Document::from_bytes_with_file_format(saved.clone(), encoding, format, FileFormat::Unix).unwrap();
            assert_eq!(reopened.text(), document.text());
            assert!(reopened.decoding_diagnostics().is_empty());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
            let at = document.text().len();
            let continued = document.prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(), revision: document.revision(),
                edits: vec![TextEdit::new(at..at, "Y")],
            }).unwrap();
            assert!(continued.summary().conversion_warnings().is_empty());
            document.commit_model_transaction(continued).unwrap();
            assert_eq!(document.text(), "a\u{fffd}😀xY");
        }
    }
}

#[test]
fn structured_paste_repairs_and_reopens_with_requested_breaks() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        for format in [Format::PlainText, Format::Markdown, Format::Html] {
            let (mut document, original) = fixture("a", encoding, format);
            let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "x\ny", vec![1]).unwrap();
            let prepared = document.prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                document.id(), document.revision(), vec![FormattedPayloadEdit::new(4..4, payload)],
            )).unwrap_or_else(|error| panic!("{encoding:?}/{format:?}: {error:?}"));
            assert_eq!(prepared.summary().conversion_warnings(), &[ConversionWarning::RepairedTruncatedUtf16]);
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), "a\u{fffd}x\ny");
            let reopened = Document::from_bytes_with_file_format(document.source_bytes(), encoding, format, FileFormat::Unix).unwrap();
            assert_eq!(reopened.text(), document.text());
            assert_eq!(reopened.line_count(), 2);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn unrelated_edits_and_failed_batches_never_repair_malformed_bytes() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let (mut document, original) = fixture("ab", encoding, Format::PlainText);
        document.insert(0, "X").unwrap();
        assert_eq!(document.source_bytes().last(), Some(&0x61));
        assert_eq!(document.decoding_diagnostics().len(), 1);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        let revision = document.revision();
        assert!(matches!(document.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(), revision,
            edits: vec![TextEdit::new(5..5, "new"), TextEdit::new(3..4, "bad")],
        }), Err(ModelTransactionError::Document(DocumentError::NotGraphemeBoundary(_)))));
        assert_eq!(document.source_bytes(), original);
        assert_eq!(document.revision(), revision);
        // Deleting a diagnostic in the same batch as append needs no repair.
        let prepared = document.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(), revision,
            edits: vec![TextEdit::new(2..5, ""), TextEdit::new(5..5, "X")],
        }).unwrap();
        assert!(prepared.summary().conversion_warnings().is_empty());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), "abX");
        assert_eq!(document.source_bytes(), encoded("abX", encoding));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn native_typing_and_composition_repair_once_and_undo_exact_bytes() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        for format in [Format::PlainText, Format::Markdown, Format::Html] {
        for composition in [false, true] {
            let (document, original) = fixture("a", encoding, format);
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: 4,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            if composition {
                let target = CompositionTarget::at_offsets(core.document(), 4..4).unwrap();
                core.handle(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
                core.handle(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3)))).unwrap();
                assert_eq!(core.document().source_bytes(), original);
                core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel)).unwrap();
                assert_eq!(core.document().source_bytes(), original);
                let target = CompositionTarget::at_offsets(core.document(), 4..4).unwrap();
                core.handle(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
                core.handle(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3)))).unwrap();
                core.handle(view, CoreEvent::Composition(CompositionEvent::Commit)).unwrap();
            } else {
                core.handle(view, CoreEvent::Input(InputEvent::text("猫"))).unwrap();
            }
            assert_eq!(core.document().text(), "a\u{fffd}猫");
            assert_eq!(core.command_state(view).unwrap().cursor(), 7);
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
            assert_eq!(core.document().source_bytes(), original);
        }
        }
    }
}

#[test]
fn repair_work_is_local_in_a_large_document() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let prefix = format!("{}tail", "word\n".repeat(10_000));
        let (document, original) = fixture(&prefix, encoding, Format::PlainText);
        let at = document.projection().text_tree().byte_len();
        let prepared = document.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(), revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "x")],
        }).unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.full_text_bytes_materialized(), 0, "{work:?}");
        assert!(work.decoded_source_bytes() < 128, "{work:?}");
        assert_eq!(prepared.summary().source_patches()[0].range(), original.len() - 1..original.len());
    }
}
