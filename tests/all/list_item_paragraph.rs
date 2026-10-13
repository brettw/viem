use viem_core::document::{BlockKind, BoundaryAffinity, HistoryNavigationRequest, ModelRequest};
use viem_core::{Document, Encoding, Format};

fn split(document: &mut Document, at: usize) {
    document.apply_model_request(ModelRequest::InsertListParagraph {
        document: document.id(), revision: document.revision(), at,
        affinity: BoundaryAffinity::Downstream,
    }).unwrap_or_else(|error| panic!("{error:?}; at={at}; source={:?}", String::from_utf8_lossy(&document.source_bytes())));
}

fn verify_reopen(document: &Document) {
    let reopened = Document::from_bytes(document.source_bytes(), document.encoding(), Format::Markdown).unwrap();
    assert_eq!(reopened.text(), document.text());
    let blocks = |doc: &Document| doc.projection().blocks().iter().map(|block|
        (block.range.clone(), block.kind.clone(), block.style.clone(), block.quote_depth)).collect::<Vec<_>>();
    assert_eq!(blocks(&reopened), blocks(document));
}

#[test]
fn list_paragraph_split_retains_owner_and_roundtrips() {
    for (source, at, expected) in [
        ("- first", 5, "first\n"),
        ("- first", 0, "\nfirst"),
        ("- first", 2, "fi\nrst"),
        ("- ", 0, "\n"),
        ("-", 0, "\n"),
        ("- first\n- second", 5, "first\n\nsecond"),
        ("- first\n\n  <span></span>", 6, "first\n\n"),
        ("- - first", 2, "fi\nrst"),
        ("-\tfirst", 2, "fi\nrst"),
        ("9) first\n1) second", 2, "fi\nrst\nsecond"),
        ("> - first\n> - second", 5, "first\n\nsecond"),
        ("- parent\n  - child\n- next", 12, "parent\nchild\n\nnext"),
        ("> - parent\n>   - child\n> - next", 9, "parent\nch\nild\nnext"),
        ("- first\n\n  second\n- next", 9, "first\nsec\nond\nnext"),
        ("- **first**", 2, "fi\nrst"),
        ("- [first](target)", 2, "fi\nrst"),
        ("- # first", 2, "fi\nrst"),
        ("- # first ##", 2, "fi\nrst"),
        ("- > first", 2, "fi\nrst"),
        ("- > - first", 2, "fi\nrst"),
    ] {
        let mut doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let before = doc.projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>();
        split(&mut doc, at);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>(), before, "{source:?}");
        assert!(matches!(doc.projection().blocks().iter().find(|block| block.range.start == at+1).unwrap().kind,
            BlockKind::ListItem { item_start: false, .. }));
        verify_reopen(&doc);
        let result = doc.source_bytes();
        doc.apply_model_request(ModelRequest::NavigateHistory { document: doc.id(), revision: doc.revision(), navigation: HistoryNavigationRequest::Undo }).unwrap();
        assert_eq!(doc.source_bytes(), source.as_bytes());
        doc.apply_model_request(ModelRequest::NavigateHistory { document: doc.id(), revision: doc.revision(), navigation: HistoryNavigationRequest::Redo }).unwrap();
        assert_eq!(doc.source_bytes(), result);
    }
}

#[test]
fn list_paragraph_stale_or_ineligible_request_preserves_source() {
    for (source, format) in [("ordinary", Format::Markdown), ("- item", Format::MarkdownSource), ("- item", Format::PlainText)] {
        let mut doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert!(!doc.can_insert_list_paragraph(1));
        assert!(doc.apply_model_request(ModelRequest::InsertListParagraph {
            document: doc.id(), revision: doc.revision(), at: 1, affinity: BoundaryAffinity::Downstream,
        }).is_err());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    let mut doc = Document::from_bytes(b"- item".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let request = ModelRequest::InsertListParagraph { document: doc.id(), revision: doc.revision(), at: 4, affinity: BoundaryAffinity::Downstream };
    split(&mut doc, 4);
    let source = doc.source_bytes();
    assert!(doc.apply_model_request(request).is_err());
    assert_eq!(doc.source_bytes(), source);
}

#[test]
fn enter_from_empty_continuation_creates_next_item() {
    let mut doc = Document::from_bytes(b"- first".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    split(&mut doc, 5);
    doc.apply_model_request(ModelRequest::ContinueList { document: doc.id(), revision: doc.revision(), at: 6 }).unwrap();
    assert_eq!(doc.text(), "first\n\n");
    let lists = doc.projection().list_structure();
    assert_eq!(lists.lists[0].items.len(), 2, "{:?}", String::from_utf8_lossy(&doc.source_bytes()));
    assert_eq!(lists.lists[0].items[0].paragraph_ids.len(), 2);
    verify_reopen(&doc);
}

#[test]
fn insertion_uses_original_encoding_and_current_fileformat() {
    use viem_core::document::FileFormat;
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let ending = match format { FileFormat::Unix => "\n", FileFormat::Dos => "\r\n", FileFormat::Mac => "\r" };
            let encode = |text: &str| match encoding {
                Encoding::Utf8 => text.as_bytes().to_vec(),
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
            };
            let source = format!("- café{ending}- next");
            let mut doc = Document::from_bytes_with_file_format(encode(&source), encoding, Format::Markdown, format).unwrap();
            split(&mut doc, "café".len());
            let expected = format!("- café{ending}{ending}  <span></span>{ending}- next");
            assert_eq!(doc.source_bytes(), encode(&expected), "{encoding:?} {format:?}");
            let reopened = Document::from_bytes_with_file_format(doc.source_bytes(), encoding, Format::Markdown, format).unwrap();
            assert_eq!(reopened.text(), doc.text());
        }
    }
}

#[test]
fn short_list_split_does_not_scan_unrelated_suffix() {
    use viem_core::document::measure_document_work;
    for size in [100, 10_000] {
        for (list, at) in [("- item\n\n", 2), ("- first\n  second\n\n", 5)] {
            let source = format!("{list}{}", "unrelated prose\n\n".repeat(size));
            let mut doc = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            let (_, work) = measure_document_work(|| split(&mut doc, at));
            assert_eq!(work.source_full_materializations, 0, "{size} {list:?}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{size} {list:?}: {work:?}");
            assert!(work.source_decoded_bytes < 20_000, "{size} {list:?}: {work:?}");
            assert!(work.projected_formatted_bytes < 2_000, "{size} {list:?}: {work:?}");
        }
    }
}

#[test]
fn list_paragraph_consumes_the_folded_source_ending() {
    let mut doc = Document::from_bytes(b"- first\n  second".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(doc.text(), "first second");
    split(&mut doc, 5);
    assert_eq!(doc.text(), "first\nsecond");
    assert_eq!(doc.projection().list_structure().lists[0].items.len(), 1);
    verify_reopen(&doc);
}

#[test]
fn list_paragraph_split_retains_inline_and_heading_treatment() {
    for (source, expected) in [
        ("- **first**", "- **fi**\n\n  **rst**"),
        ("- `first`", "- `fi`\n\n  `rst`"),
        ("- # first", "- # fi\n\n  # rst"),
    ] {
        let mut doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        split(&mut doc, 2);
        assert_eq!(doc.source_bytes(), expected.as_bytes());
        verify_reopen(&doc);
    }
}

#[test]
fn enter_at_continuation_start_keeps_empty_paragraph_with_original_item() {
    let mut doc = Document::from_bytes(b"- first\n\n  second".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    doc.apply_model_request(ModelRequest::ContinueList { document: doc.id(), revision: doc.revision(), at: 6 }).unwrap();
    assert_eq!(doc.text(), "first\n\nsecond");
    let lists = doc.projection().list_structure();
    assert_eq!(lists.lists[0].items.len(), 2);
    assert_eq!(lists.lists[0].items[0].paragraph_ids.len(), 2);
    verify_reopen(&doc);
}

#[test]
fn enter_on_empty_first_paragraph_does_not_exit_an_item_with_more_body() {
    for source in ["- <span></span>\n\n  first", "- <span></span>\n  - child"] {
        let mut doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let text = doc.text().to_owned();
        doc.apply_model_request(ModelRequest::ContinueList { document: doc.id(), revision: doc.revision(), at: 0 }).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(doc.text(), format!("\n{text}"));
        let lists = doc.projection().list_structure();
        assert_eq!(lists.lists[0].items.len(), 2, "{source}");
        verify_reopen(&doc);
    }
}

#[test]
fn folded_quote_prefix_uses_encoded_boundaries_and_preserves_inline_scopes() {
    use viem_core::document::FileFormat;
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        let encode = |text: &str| match encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
        };
        let source = "- > **first\r\n  > second**\r\n- tail";
        let mut doc = Document::from_bytes_with_file_format(encode(source), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        split(&mut doc, 5);
        assert_eq!(doc.text(), "first\nsecond\ntail");
        let result = doc.source_bytes();
        assert_eq!(result, encode("- > **first**\r\n  >\r\n  > **second**\r\n- tail"));
        let reopened = Document::from_bytes_with_file_format(result, encoding, Format::Markdown, FileFormat::Dos).unwrap();
        assert_eq!(reopened.text(), doc.text());
        assert_eq!(reopened.projection().style_spans(), doc.projection().style_spans());
    }
}
