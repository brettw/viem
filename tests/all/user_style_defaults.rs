use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn defaults() -> Vec<u8> {
    let document = Document::new("");
    let mut value: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in value["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["character"]["size"] = 31.into();
            style["character"]["font_families"] = serde_json::json!(["Georgia"]);
        }
    }
    serde_json::to_vec(&value).unwrap()
}
#[test]
fn defaults_are_visible_declarations_and_source_declarations_override_them() {
    for (format, source, offset, expected) in [
        (Format::PlainText, "Text", 0, 31.),
        (Format::Markdown, "Text", 0, 31.),
        (Format::MarkdownSource, "Text", 0, 31.),

    ] {
        let mut doc = open(source, format);
        let before = doc.history_status();
        let prior_style_revision = doc.projection().style_sheet().revision;
        doc.initialize_style_defaults(&defaults()).unwrap();
        assert!(doc.projection().style_sheet().revision > prior_style_revision);
        assert_eq!(doc.revision(), Revision(0));
        let after = doc.history_status();
        // Defaults add retained configuration allocations, which must count
        // toward the byte budget without creating an edit or moving history.
        assert!(after.retained_memory_bytes > before.retained_memory_bytes);
        assert!(after.live_state_memory_bytes > before.live_state_memory_bytes);
        // Independent accounting tables may retain a few different spare
        // buckets after reprojection; no old document state is retained.
        assert!(after.additional_history_memory_bytes <= 4096);
        assert_eq!(
            after,
            HistoryStatus {
                retained_memory_bytes: after.retained_memory_bytes,
                live_state_memory_bytes: after.live_state_memory_bytes,
                additional_history_memory_bytes: after.additional_history_memory_bytes,
                ..before
            }
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
        let style =
            DocumentLayoutStyles::semantic_character_at(doc.projection(), offset, false).unwrap();
        assert_eq!(style.size, expected, "{format:?}");
        assert_eq!(
            doc.projection()
                .style_sheet()
                .block_style(&"Paragraph".into())
                .unwrap()
                .character
                .size,
            Some((31.).into())
        );
        let original = doc.source_bytes();
        doc.insert(offset, "X").unwrap();
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), offset, false)
                .unwrap()
                .size,
            expected,
            "after edit {format:?}"
        );
        let saved = String::from_utf8(doc.source_bytes()).unwrap();
        assert!(!saved.contains("Georgia"), "{saved}");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original);
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(doc.projection(), offset, false)
                .unwrap()
                .size,
            expected
        );
    }
}

#[test]
fn default_files_skip_invalid_definitions_and_export_current_overrides() {
    let mut doc = Document::new("Text");
    let before = doc.source_bytes();
    assert!(doc.initialize_style_defaults(br#"{"version":2}"#).is_err());
    let diagnostics = doc.initialize_style_defaults(br#"{"version":1,"block_styles":[{"id":"Paragraph","name":"Bad","role":"Document","based_on":null,"next_paragraph_style":null,"character":{},"block":{}}]}"#).unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("Paragraph") && diagnostics[0].contains("role"));
    assert_eq!(doc.projection().style_sheet().block_style(&"Paragraph".into()).unwrap().role, BlockRole::Paragraph);
    assert_eq!(doc.source_bytes(),before);
    assert_eq!(doc.revision(),Revision(0));
    doc.initialize_style_defaults(&defaults()).unwrap();
    let exported = doc.export_style_defaults().unwrap();
    let mut reopened = Document::new("Other");
    reopened.initialize_style_defaults(&exported).unwrap();
    assert_eq!(
        DocumentLayoutStyles::character_at(reopened.projection(), 0, false)
            .unwrap()
            .size,
        31.
    );
    doc.insert(0, "X").unwrap();
    assert!(doc.initialize_style_defaults(&defaults()).is_err());
}

#[test]
fn defaults_do_not_invalidate_untouched_large_document_projection() {
    let source = (0..10_001)
        .map(|i| format!("Paragraph {i}\n\n"))
        .collect::<String>();
    let mut doc = open(&source, Format::Markdown);
    let work = doc.open_work_statistics();
    let id = doc.projection().blocks()[9_000].id;
    doc.initialize_style_defaults(&defaults()).unwrap();
    assert_eq!(doc.open_work_statistics(), work);
    let range = doc.projection().hard_line_range(5_000).unwrap();
    let prepared = doc
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(),
            revision: doc.revision(),
            edits: vec![TextEdit::new(range.start..range.start + 1, "p")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert_eq!(
        prepared.summary().projection_work().projected_hard_lines(),
        1
    );
    assert_eq!(
        prepared
            .summary()
            .projection_work()
            .full_text_bytes_materialized(),
        0
    );
    doc.commit_model_transaction(prepared).unwrap();
    assert_eq!(doc.projection().blocks()[9_000].id, id);
    assert_eq!(
        DocumentLayoutStyles::character_at(doc.projection(), range.start, false)
            .unwrap()
            .size,
        31.
    );
}
