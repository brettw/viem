use viem_core::document::{
    Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};

#[test]
fn removing_last_rtf_group_character_retains_empty_typing_context() {
    use viem_core::document::FontSlant;
    use viem_core::layout::DocumentLayoutStyles;
    for encoding in [Encoding::Utf8, Encoding::Latin1] {
        let original = r"{\rtf1 Before {\i x} after}";
        let bytes: Vec<u8> = match encoding {
            Encoding::Utf8 | Encoding::Latin1 => original.as_bytes().to_vec(),
            _ => unreachable!(),
        };
        let mut document = Document::from_bytes(bytes.clone(), encoding, Format::Rtf).unwrap();
        document.replace(7..8, "").unwrap();
        let fresh = Document::from_bytes(document.source_bytes(), encoding, Format::Rtf).unwrap();
        assert_eq!(document.text(), "Before  after");
        assert_eq!(
            document.projection().provenance(),
            fresh.projection().provenance()
        );
        assert!(document
            .projection()
            .provenance()
            .iter()
            .any(|span| span.formatted == (7..7)));
        document.insert(7, "é").unwrap();
        let style =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 7, false).unwrap();
        assert_eq!(style.slant, FontSlant::Italic);
        assert_eq!(document.text(), "Before é after");
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
    }
}

#[test]
fn ordinary_rtf_group_interior_deletion_remains_regional() {
    let source = format!("{{\\rtf1 {}}}", "{\\i many words}\\par ".repeat(10_000));
    let mut document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    let at = document.projection().hard_line_range(5_000).unwrap().start + 1;
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 1, "")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 256);
    document.commit_model_transaction(prepared).unwrap();
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
}

#[test]
fn ordinary_rich_edits_reparse_one_line_and_share_unaffected_indexes() {
    for format in [Format::Rtf] {
        let mut source = if format == Format::Rtf {
            String::from("{\\rtf1\\ansi\n")
        } else {
            String::new()
        };
        for index in 0..20_000 {
            source.push_str(&format!("{{\\b line {index:05}}} body\\par\n"));
        }
        if format == Format::Rtf {
            source.push('}');
        }
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        let original_block = document.projection().blocks()[15_000].id;
        for index in 0..12 {
            let line = document.projection().hard_line_range(10_000).unwrap();
            let at = line.start + 2;
            let replacement = if index % 2 == 0 { "X" } else { "n" };
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at + 1, replacement)],
                })
                .unwrap();
            let work = prepared.summary().projection_work();
            assert_eq!(
                work.scope(),
                ProjectionWorkScope::RegionalHardLines,
                "{format:?}"
            );
            assert_eq!(work.projected_hard_lines(), 1);
            assert!(work.decoded_source_bytes() < 256, "{format:?}: {work:?}");
            assert_eq!(work.full_text_bytes_materialized(), 0);
            document.commit_model_transaction(prepared).unwrap();
        }
        assert_eq!(document.projection().blocks()[15_000].id, original_block);
        let fresh =
            Document::from_bytes(document.source_bytes(), document.encoding(), format).unwrap();
        assert_eq!(document.text(), fresh.text());
        assert_eq!(
            document.projection().provenance(),
            fresh.projection().provenance()
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
    }
}

#[test]
fn multiline_paragraph_edits_reparse_only_the_affected_paragraph() {
    for format in [Format::Rtf] {
        let mut source = if format == Format::Rtf {
            String::from(r"{\rtf1{\stylesheet{\s0 Normal;}{\*\cs2\i Accent;}}")
        } else {
            String::new()
        };
        for index in 0..12_000 {
            source.push_str(&format!(
                "{{\\cs2 line {index:05}\\line second\\line third}}\\par\n"
            ));
        }
        if format == Format::Rtf {
            source.push('}');
        }
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, format).unwrap();
        let at = document.projection().hard_line_range(18_001).unwrap().start + 1;
        let paragraphs = document
            .projection()
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();
        let lines = (0..document.projection().hard_line_count())
            .map(|line| document.projection().hard_line_id(line).unwrap())
            .collect::<Vec<_>>();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at + 1, "XYZ")],
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(
            work.scope(),
            ProjectionWorkScope::RegionalHardLines,
            "{format:?}: {work:?}"
        );
        assert_eq!(work.projected_hard_lines(), 3);
        assert!(work.decoded_source_bytes() < 256);
        assert_eq!(work.full_text_bytes_materialized(), 0);
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>(),
            paragraphs
        );
        assert_eq!(
            (0..document.projection().hard_line_count())
                .map(|line| document.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>(),
            lines
        );
        let fresh =
            Document::from_bytes(document.source_bytes(), document.encoding(), format).unwrap();
        assert_eq!(document.text(), fresh.text());
        assert_eq!(
            document.projection().provenance(),
            fresh.projection().provenance()
        );
        assert_eq!(
            document.projection().style_spans(),
            fresh.projection().style_spans()
        );
    }
}

#[test]
fn native_typed_payloads_use_the_bounded_rich_edit_path() {
    use viem_core::document::{
        BoundaryAffinity, FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };
    let source = format!(
        "{{\\rtf1 {}}}",
        (0..15_000)
            .map(|i| format!("{{\\b word {i:05}}} tail\\par\n"))
            .collect::<String>()
    );
    let mut document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    let at = document.projection().hard_line_range(7_500).unwrap().start + 10;
    let payload =
        FormattedTextPayload::new(&document.hard_line_snapshot(), "X", Vec::new()).unwrap();
    let request = FormattedPayloadEditRequest::new(
        document.id(),
        document.revision(),
        vec![FormattedPayloadEdit::new(at..at, payload)
            .with_boundary_affinity(BoundaryAffinity::Upstream)],
    );
    let prepared = document.prepare_formatted_payload_request(request).unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(prepared).unwrap();
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
}
