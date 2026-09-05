use evim_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, HardLineTransfer,
    HistorySemanticChangeKind, ModelChangeKind, ModelRequest, ModelTransactionError,
    SemanticInlineStyle,
};

fn encode_source(encoding: Encoding, text: &str) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|ch| u8::try_from(u32::from(ch)).expect("fixture is Latin-1 representable"))
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xff, 0xfe];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = vec![0xfe, 0xff];
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            bytes
        }
    }
}

fn encode_fragment(encoding: Encoding, text: &str) -> Vec<u8> {
    let mut bytes = encode_source(encoding, text);
    if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
        bytes.drain(..2);
    }
    bytes
}

#[test]
fn source_image_restores_markdown_spelling_when_formatted_text_is_identical() {
    let original = b"__bold__\r\nnext".to_vec();
    let mut document =
        Document::from_bytes(original.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.file_format(), FileFormat::Dos);
    let image = document.capture_hard_line_source_image(0).unwrap();

    document
        .set_semantic_style(0..4, SemanticInlineStyle::Strong, false)
        .unwrap();
    document
        .set_semantic_style(0..4, SemanticInlineStyle::Strong, true)
        .unwrap();
    let alternate = b"**bold**\r\nnext".to_vec();
    assert_eq!(document.source_bytes(), alternate);
    assert_eq!(document.text(), "bold\nnext");

    let prepared = document
        .prepare_model_request(ModelRequest::RestoreHardLineSource {
            document: document.id(),
            revision: document.revision(),
            target_line: 0,
            image: image.clone(),
        })
        .unwrap();
    assert_eq!(
        prepared.summary().kind(),
        ModelChangeKind::HardLineSourceRestoration
    );
    assert!(prepared.summary().formatted_splices().is_empty());
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(prepared.summary().source_patches()[0].range(), 0..8);
    assert_eq!(
        prepared.summary().source_patches()[0].replacement(),
        b"__bold__"
    );
    assert_eq!(
        document.source_bytes(),
        alternate,
        "prepare is non-mutating"
    );

    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.text(), "bold\nnext");
    assert_eq!(
        document.history_status().undo_summary.unwrap().changes(),
        [HistorySemanticChangeKind::HardLineSourceRestoration]
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), alternate);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn source_image_restores_markdown_crlf_and_bom_exactly_in_every_encoding() {
    let source_text = "# __Hé__\r\nnext";
    let changed_text = "# __Yo__\r\nnext";
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let original = encode_source(encoding, source_text);
        let changed = encode_source(encoding, changed_text);
        let mut document =
            Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
        assert_eq!(document.file_format(), FileFormat::Dos);
        let image = document.capture_hard_line_source_image(0).unwrap();
        assert!(image.is_terminated());
        assert_eq!(
            image.source_byte_len(),
            encode_fragment(encoding, "# __Hé__\r\n").len()
        );

        document.replace(0.."Hé".len(), "Yo").unwrap();
        assert_eq!(document.source_bytes(), changed);
        let prepared = document
            .prepare_model_request(ModelRequest::RestoreHardLineSource {
                document: document.id(),
                revision: document.revision(),
                target_line: 0,
                image: image.clone(),
            })
            .unwrap_or_else(|error| panic!("{encoding:?}: {error:?}"));
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(
            prepared.summary().source_patches()[0].replacement(),
            encode_fragment(encoding, "Hé")
        );
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), original);
        assert_eq!(document.text(), "Hé\nnext");

        assert!(document.undo());
        assert_eq!(document.source_bytes(), changed);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn source_image_restores_malformed_opaque_bytes_and_diagnostics() {
    let original = b"**a\xffb**\r\nnext".to_vec();
    let mut document =
        Document::from_bytes(original.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), "a\u{fffd}b\nnext");
    assert_eq!(document.decoding_diagnostics().len(), 1);
    let image = document.capture_hard_line_source_image(0).unwrap();

    document.replace(0.."a\u{fffd}b".len(), "clean").unwrap();
    let changed = b"**clean**\r\nnext".to_vec();
    assert_eq!(document.source_bytes(), changed);
    assert!(document.decoding_diagnostics().is_empty());

    document.restore_hard_line_source_image(0, image).unwrap();
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.text(), "a\u{fffd}b\nnext");
    assert_eq!(document.decoding_diagnostics().len(), 1);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), changed);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn source_image_rejects_wrong_document_stale_request_and_incompatible_pipeline() {
    let first = Document::new("baseline");
    let image = first.capture_hard_line_source_image(0).unwrap();
    let mut other = Document::new("changed");
    let before = other.source_bytes();
    assert_eq!(
        other.restore_hard_line_source_image(0, image.clone()),
        Err(DocumentError::WrongDocument)
    );
    assert_eq!(other.source_bytes(), before);

    let mut stale = Document::new("baseline");
    let image = stale.capture_hard_line_source_image(0).unwrap();
    let request = ModelRequest::RestoreHardLineSource {
        document: stale.id(),
        revision: stale.revision(),
        target_line: 0,
        image,
    };
    stale.replace(0..8, "changed").unwrap();
    let before = stale.source_bytes();
    let before_history = stale.history_status();
    assert!(matches!(
        stale.prepare_model_request(request),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(stale.source_bytes(), before);
    assert_eq!(stale.history_status(), before_history);

    let mut incompatible = Document::from_bytes_with_file_format(
        b"baseline\n".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Unix,
    )
    .unwrap();
    let image = incompatible.capture_hard_line_source_image(0).unwrap();
    incompatible.set_file_format(FileFormat::Dos).unwrap();
    let before = incompatible.source_bytes();
    let before_history = incompatible.history_status();
    assert_eq!(
        incompatible.restore_hard_line_source_image(0, image),
        Err(DocumentError::IncompatibleHardLineSourceImage)
    );
    assert_eq!(incompatible.source_bytes(), before);
    assert_eq!(incompatible.history_status(), before_history);
}

#[test]
fn source_image_rejects_changed_or_ambiguous_hard_line_topology() {
    let mut split = Document::new("one\ntwo");
    let image = split.capture_hard_line_source_image(0).unwrap();
    split.insert(1, "\n").unwrap();
    let before = split.source_bytes();
    let before_history = split.history_status();
    assert_eq!(
        split.restore_hard_line_source_image(0, image),
        Err(DocumentError::HardLineSourceImageTopologyChanged {
            captured_line_count: 2,
            current_line_count: 3,
        })
    );
    assert_eq!(split.source_bytes(), before);
    assert_eq!(split.history_status(), before_history);

    let mut moved = Document::new("one\ntwo\nthree");
    let image = moved.capture_hard_line_source_image(0).unwrap();
    let expected_id = image.hard_line_id();
    moved
        .transfer_hard_lines(HardLineTransfer::Move, 0..1, 2)
        .unwrap();
    let actual_id = moved.projection().hard_line_id(0).unwrap();
    assert_ne!(actual_id, expected_id);
    let before = moved.source_bytes();
    let before_history = moved.history_status();
    assert_eq!(
        moved.restore_hard_line_source_image(0, image),
        Err(DocumentError::StaleHardLineSourceImage {
            target_line: 0,
            expected_id,
            actual_id,
        })
    );
    assert_eq!(moved.source_bytes(), before);
    assert_eq!(moved.history_status(), before_history);

    let mut moved_to_final = Document::new("one\ntwo\nthree");
    let image = moved_to_final.capture_hard_line_source_image(1).unwrap();
    let target_id = image.hard_line_id();
    moved_to_final
        .transfer_hard_lines(HardLineTransfer::Move, 1..2, 3)
        .unwrap();
    let target_line = (0..moved_to_final.line_count())
        .find(|&line| moved_to_final.projection().hard_line_id(line) == Some(target_id))
        .unwrap();
    assert_eq!(target_line + 1, moved_to_final.line_count());
    let before = moved_to_final.source_bytes();
    assert_eq!(
        moved_to_final.restore_hard_line_source_image(target_line, image),
        Err(DocumentError::HardLineSourceImageTerminatorShapeChanged {
            captured_terminated: true,
            current_terminated: false,
        })
    );
    assert_eq!(moved_to_final.source_bytes(), before);
}

#[test]
fn source_image_can_follow_its_stable_identity_to_a_new_nonfinal_ordinal() {
    let mut document = Document::new("one\ntwo\nthree\nfour");
    let image = document.capture_hard_line_source_image(1).unwrap();
    let target_id = image.hard_line_id();
    document.replace(4..7, "TWO").unwrap();
    document
        .transfer_hard_lines(HardLineTransfer::Move, 1..2, 3)
        .unwrap();
    let target_line = (0..document.line_count())
        .find(|&line| document.projection().hard_line_id(line) == Some(target_id))
        .unwrap();
    assert_ne!(target_line, image.captured_line());
    assert!(target_line + 1 < document.line_count());

    document
        .restore_hard_line_source_image(target_line, image)
        .unwrap();
    assert_eq!(document.text(), "one\nthree\ntwo\nfour");
    assert_eq!(document.source_bytes(), b"one\nthree\ntwo\nfour");
}
