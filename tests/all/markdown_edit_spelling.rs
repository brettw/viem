use viem_core::document::{Document, Encoding, Format};

fn markdown(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn assert_source(document: &Document, source: &str) {
    assert_eq!(document.source_bytes(), source.as_bytes());
    let reopened = markdown(source);
    assert_eq!(document.text(), reopened.text());
    assert_eq!(
        document.projection().style_spans(),
        reopened.projection().style_spans()
    );
    assert_eq!(
        document
            .hard_line_snapshot()
            .capture(0..document.text().len())
            .unwrap()
            .break_offsets(),
        reopened
            .hard_line_snapshot()
            .capture(0..reopened.text().len())
            .unwrap()
            .break_offsets()
    );
    assert!(document
        .projection()
        .blocks()
        .iter()
        .zip(reopened.projection().blocks())
        .all(|(a, b)| a.range == b.range && a.attributes == b.attributes));
}

#[test]
fn spaces_protected_during_typing_become_literal_after_following_text() {
    for (source, expected) in [
        (
            "first\n\nrest&#32; untouched",
            "first next\n\nrest&#32; untouched",
        ),
        ("# first\n\nrest", "# first next\n\nrest"),
        ("- first\n- rest", "- first next\n- rest"),
        ("> first\n\nrest", "> first next\n\nrest"),
    ] {
        let mut document = markdown(source);
        document.insert(5, " ").unwrap();
        document.insert(6, "next").unwrap();
        assert_source(&document, expected);
        assert!(document.undo());
        assert!(document.undo());
        assert_source(&document, source);
        assert!(document.redo());
        assert!(document.redo());
        assert_source(&document, expected);
    }
}

#[test]
fn editing_next_to_existing_whitespace_references_cleans_only_that_run() {
    for (source, range, input, expected) in [
        ("first&#32;", 6..6, "next", "first next"),
        ("a&#32;&#x20;b", 3..4, "c", "a  c"),
        ("a&#9;b", 2..3, "c", "a\tc"),
        ("&#32;&#32;&#32;&#32;b", 4..5, "c", "&#32;   c"),
        (
            "a&#32;old&#32;b\n\nuntouched&#32;x",
            2..5,
            "NEW",
            "a NEW b\n\nuntouched&#32;x",
        ),
        ("a&#32;old&#32;b", 2..5, "", "a  b"),
        ("*word&#32;* tail", 6..7, "x", "*word&#32;* xail"),
    ] {
        let mut document = markdown(source);
        document.replace(range, input).unwrap();
        assert_source(&document, expected);
        assert!(document.undo());
        assert_source(&document, source);
    }
}

#[test]
fn cleanup_keeps_owner_sensitive_whitespace_at_list_and_html_boundaries() {
    use viem_core::document::{
        BoundaryAffinity, FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };
    for (source, at, expected) in [
        ("- first\n\n", 6, "- first\n\n&#32;"),
        ("- first\n\nTail", 6, "- first\n\n&#32;Tail"),
        (
            "- first\n\n  continued\n- tail",
            6,
            "- first\n\n  &#32;continued\n- tail",
        ),
        (
            "- first\n\n  second\n\n  third\n- tail",
            13,
            "- first\n\n  second\n\n  &#32;third\n- tail",
        ),
        (
            "> - first\n>\n>   continued\n> - tail",
            6,
            "> - first\n>\n>   &#32;continued\n> - tail",
        ),
        (
            "<div>\nx\n</div>\n\nTail",
            0,
            "<div>\n&#32;x\n</div>\n\nTail",
        ),
        (
            "<div>\nx\n</div>\n\nTail",
            1,
            "<div>\nx&#32;\n</div>\n\nTail",
        ),
    ] {
        let mut document = markdown(source);
        let payload =
            FormattedTextPayload::new(&document.hard_line_snapshot(), " ", vec![]).unwrap();
        let request = FormattedPayloadEditRequest::new(
            document.id(),
            document.revision(),
            vec![FormattedPayloadEdit::new(at..at, payload)
                .with_boundary_affinity(BoundaryAffinity::Downstream)],
        );
        document
            .apply_formatted_payload_request(request)
            .unwrap_or_else(|error| panic!("{source:?} at {at}: {error:?}"));
        assert_source(&document, expected);
        assert!(document.undo());
        assert_source(&document, source);
    }
}

#[test]
fn command_typing_and_payloads_cleanup_in_the_same_undo_transaction() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    use viem_core::document::{
        FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };
    let mut document = markdown("first&#32;");
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('A'))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::text("next"))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::Key(Key::Escape))
        .unwrap();
    assert_source(&document, "first next");
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_source(&document, "first&#32;");

    let payload =
        FormattedTextPayload::new(&document.hard_line_snapshot(), "next", vec![]).unwrap();
    let request = FormattedPayloadEditRequest::new(
        document.id(),
        document.revision(),
        vec![FormattedPayloadEdit::new(6..6, payload)],
    );
    document.apply_formatted_payload_request(request).unwrap();
    assert_source(&document, "first next");
    assert!(document.undo());
    assert_source(&document, "first&#32;");
}

#[test]
fn cleanup_preserves_encoding_endings_and_exact_patch_locality() {
    use viem_core::document::{FileFormat, ModelRequest, TextEdit};
    fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
        match encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
            Encoding::Utf16Le => [
                vec![0xff, 0xfe],
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat(),
            Encoding::Utf16Be => [
                vec![0xfe, 0xff],
                text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            ]
            .concat(),
        }
    }
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (format, ending) in [
            (FileFormat::Unix, "\n"),
            (FileFormat::Dos, "\r\n"),
            (FileFormat::Mac, "\r"),
        ] {
            let source = format!("café&#x20;old{}{}keep&#32;spelling", ending, ending);
            let expected = format!("café NEW{}{}keep&#32;spelling", ending, ending);
            let original = encoded(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                Format::Markdown,
                format,
            )
            .unwrap();
            let at = document.text().find("old").unwrap();
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at + 3, "NEW")],
                })
                .unwrap();
            let byte_width = if matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be) {
                2
            } else {
                1
            };
            let prefix = encoded("café", encoding).len();
            let patches = prepared.summary().source_patches();
            assert_eq!(patches.len(), 1);
            assert_eq!(
                patches[0].range(),
                prefix..prefix + "&#x20;old".len() * byte_width
            );
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.source_bytes(), encoded(&expected, encoding));
            let reopened = Document::from_bytes_with_file_format(
                document.source_bytes(),
                encoding,
                Format::Markdown,
                format,
            )
            .unwrap();
            assert_eq!(document.text(), reopened.text());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn a_batch_cleans_each_edit_frontier_without_cleaning_between_them() {
    use viem_core::document::TextEdit;
    let source = "one&#32;x\n\nmiddle&#32;untouched\n\ntwo&#32;y";
    let mut document = markdown(source);
    let first = document.text().find('x').unwrap();
    let last = document.text().find('y').unwrap();
    document
        .apply_edits(vec![
            TextEdit::new(first..first + 1, "X"),
            TextEdit::new(last..last + 1, "Y"),
        ])
        .unwrap();
    assert_source(&document, "one X\n\nmiddle&#32;untouched\n\ntwo Y");
    assert!(document.undo());
    assert_source(&document, source);
}

#[test]
fn cleanup_is_regional_and_preserves_unaffected_layout_caches_in_large_documents() {
    use viem_core::document::measure_document_work;
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let source = (0..10_000)
        .map(|index| {
            if index == 5000 {
                "edited&#32;old\n\n".to_owned()
            } else {
                format!("Paragraph {index} unchanged&#32;spelling\n\n")
            }
        })
        .collect::<String>();
    let mut document = markdown(&source);
    let at = document.text().find("edited old").unwrap() + "edited ".len();
    let later_id = document.projection().blocks()[9000].id;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600., 300.);
    engine.set_cache_capacity(10_010);
    // Retain this fixture's longer paragraphs; the default byte budget would
    // deliberately evict entries independently of the edit.
    engine.set_shape_cache_byte_budgets(128 * 1024 * 1024, 4 * 1024 * 1024);
    engine.relayout(&document, &mut view).unwrap();
    let requests = engine.provider().request_calls();
    let (result, work) = measure_document_work(|| document.replace(at..at + 3, "NEW"));
    result.unwrap();
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.source_full_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 4096, "{work:?}");
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(document.projection().blocks()[9000].id, later_id);
    assert!(
        engine.provider().request_calls() - requests <= 2,
        "new measurement requests: {}",
        engine.provider().request_calls() - requests
    );
    assert_eq!(
        document.source_bytes(),
        source.replace("edited&#32;old", "edited NEW").as_bytes()
    );
}

#[test]
fn new_literal_text_uses_plain_spelling_where_syntax_is_inactive() {
    for (source, at, input, expected) in [
        ("word", 2, "#", "wo#rd"),
        ("word", 2, "_", "wo_rd"),
        ("word", 2, " > ", "wo > rd"),
        ("<p>A</p>", 1, " B", "<p>A B</p>"),
        ("<p>A</p>", 1, " <& B", "<p>A &lt;&amp; B</p>"),
    ] {
        let mut document = markdown(source);
        document.insert(at, input).unwrap();
        assert_source(&document, expected);
        assert!(document.undo());
        assert_source(&document, source);
    }
}

#[test]
fn necessary_escaping_and_untouched_literal_modes_are_preserved() {
    for (source, at, input) in [
        ("&#32;first", 6, "x"),
        ("first\nsecond", 5, " "),
        ("# first\n\nsecond", 5, " "),
        ("&#32;&#32;", 2, "x"),
        ("text", 0, "# "),
        ("**bold**", 2, "*"),
        ("`&#32;`", 5, "x"),
    ] {
        let mut document = markdown(source);
        let mut expected = document.text().to_owned();
        expected.insert_str(at, input);
        document.insert(at, input).unwrap();
        assert_eq!(document.text(), expected);
        let saved = document.source_bytes();
        assert_eq!(
            markdown(std::str::from_utf8(&saved).unwrap()).text(),
            expected
        );
        assert!(document.undo());
        assert_source(&document, source);
    }
    for format in [Format::MarkdownSource, Format::Code, Format::PlainText] {
        let mut document =
            Document::from_bytes(b"a&#32;".to_vec(), Encoding::Utf8, format).unwrap();
        document.insert(6, "x").unwrap();
        assert_eq!(document.source_bytes(), b"a&#32;x");
    }
}
