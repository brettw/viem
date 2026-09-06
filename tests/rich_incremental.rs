use evim_core::document::{
    Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};

#[test]
fn ordinary_rich_edits_reparse_one_line_and_share_unaffected_indexes() {
    for format in [Format::Html, Format::Rtf] {
        let mut source = if format == Format::Rtf {
            String::from("{\\rtf1\\ansi\n")
        } else {
            String::new()
        };
        for index in 0..20_000 {
            source.push_str(&if format == Format::Html {
                format!("<p><b>line {index:05}</b> body</p>\n")
            } else {
                format!("{{\\b line {index:05}}} body\\par\n")
            });
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
fn html_incremental_reparse_checks_character_reference_boundaries() {
    let mut document = Document::from_bytes(
        b"<p>&a</p><p>tail</p>".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    let original = document.source_bytes();
    // Literal insertion must not complete &amp; across the source boundary.
    document.insert(2, "mp;").unwrap();
    assert_eq!(document.text(), "&amp;\ntail");
    assert_eq!(document.source_bytes(), b"<p>&a&#x6D;p;</p><p>tail</p>");
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), reopened.text());
    assert_eq!(
        document.projection().provenance(),
        reopened.projection().provenance()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn multiline_paragraph_edits_reparse_only_the_affected_paragraph() {
    for format in [Format::Html, Format::Rtf] {
        let mut source = if format == Format::Rtf {
            String::from(r"{\rtf1{\stylesheet{\s0 Normal;}{\*\cs2\i Accent;}}")
        } else {
            String::new()
        };
        for index in 0..12_000 {
            source.push_str(&if format == Format::Html {
                format!("<p><b>line {index:05}</b><br>second<br>third</p>\n")
            } else {
                format!("{{\\cs2 line {index:05}\\line second\\line third}}\\par\n")
            });
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
    use evim_core::document::{
        BoundaryAffinity, FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };
    let source = (0..15_000)
        .map(|i| format!("<p><b>word {i:05}</b> tail</p>\n"))
        .collect::<String>();
    let mut document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
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
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
}
