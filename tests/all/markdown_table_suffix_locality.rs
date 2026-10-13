//! Table metadata shifts through the persistent table index. Editing prose or
//! a table cell must not visit and copy every following table and row store.
use viem_core::{Document, Encoding, Format};
use viem_core::document::measure_document_work;

fn source(tables: usize) -> String {
    let mut source = "Editable paragraph.\n\n".to_owned();
    for _ in 0..tables {
        source.push_str("| first | second |\n| --- | --- |\n| alpha | beta |\n\n");
    }
    source
}

fn same_table_positions(document: &Document, reopened: &Document) {
    assert_eq!(document.text(), reopened.text());
    assert_eq!(document.projection().tables().len(), reopened.projection().tables().len());
    for (actual, expected) in document.projection().tables().iter().zip(reopened.projection().tables()) {
        assert_eq!(actual.range, expected.range);
        assert_eq!(actual.source_range, expected.source_range);
        assert_eq!(actual.columns, expected.columns);
        for (actual, expected) in actual.rows.iter().zip(&expected.rows) {
            assert_eq!(actual.range, expected.range);
            assert_eq!(actual.source_range, expected.source_range);
            for (actual, expected) in actual.cells.iter().zip(&expected.cells) {
                assert_eq!(actual.range, expected.range);
                assert_eq!(actual.source_range, expected.source_range);
            }
        }
        for (actual, expected) in actual.source_rows.iter().zip(&expected.source_rows) {
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn prose_edits_shift_table_suffix_with_bounded_work_and_exact_history() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let source = source(4096);
            let encode = |text: &str| match encoding {
                Encoding::Utf8 => text.as_bytes().to_vec(),
                Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                _ => unreachable!(),
            };
            let original = encode(&source);
            let mut document = Document::from_bytes(original.clone(), encoding, format).unwrap();
            let first_id = document.projection().tables()[0].id;
            let last_id = document.projection().tables().last().unwrap().id;
            let (result, work) = measure_document_work(|| document.replace(1..1, "λ"));
            result.unwrap();
            // Source prose still has a pre-existing full-projection fallback;
            // exercise its coordinate and history correctness without claiming
            // that separate regional path is supported.
            if format == Format::Markdown {
                assert_eq!(work.full_projection_candidates, 0, "{work:?}");
                assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
                assert!(work.source_decoded_bytes < 4096, "{work:?}");
                assert!(work.retained_memory_allocation_visits < 4096, "{work:?}");
                assert!(work.projection_persistent_nodes_copied < 512, "{work:?}");
            }
            assert_eq!(document.projection().tables()[0].id, first_id);
            assert_eq!(document.projection().tables().last().unwrap().id, last_id);
            let expected = encode(&format!("Eλ{}", &source[1..]));
            assert_eq!(document.source_bytes(), expected);
            let reopened = Document::from_bytes(expected.clone(), encoding, format).unwrap();
            same_table_positions(&document, &reopened);
            document.try_undo().unwrap();
            assert_eq!(document.source_bytes(), original);
            document.try_redo().unwrap();
            assert_eq!(document.source_bytes(), expected);
            same_table_positions(&document, &reopened);
        }
    }
}

#[test]
fn table_cell_edits_retain_distant_table_ids_and_reopen_positions() {
    let original = source(4096).into_bytes();
    let mut document = Document::from_bytes(original.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    let first = &document.projection().tables()[0];
    let at = first.rows[1].cells[0].range.start + 1;
    let table_id = first.id;
    let last_id = document.projection().tables().last().unwrap().id;
    let (result, work) = measure_document_work(|| document.replace(at..at, "é"));
    result.unwrap();
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert!(work.retained_memory_allocation_visits < 4096, "{work:?}");
    assert!(work.source_decoded_bytes < 4096, "{work:?}");
    assert_eq!(document.projection().tables()[0].id, table_id);
    assert_eq!(document.projection().tables().last().unwrap().id, last_id);
    let saved = document.source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    same_table_positions(&document, &reopened);
    document.try_undo().unwrap();
    assert_eq!(document.source_bytes(), original);
    document.try_redo().unwrap();
    assert_eq!(document.source_bytes(), saved);
}

fn table_identities(document: &Document) -> Vec<(u64, Vec<(u64, Vec<u64>)>)> {
    document.projection().tables().iter().map(|table| {
        (table.id, table.rows.iter().map(|row| {
            (row.id, row.cells.iter().map(|cell| cell.id).collect())
        }).collect())
    }).collect()
}

#[test]
fn source_full_reprojection_retains_all_shifted_table_identities() {
    for (range, replacement) in [(1..1, "λ"), (1..2, "")] {
        let original = source(3);
        let mut document = Document::from_bytes(original.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
        let identities = table_identities(&document);
        let mut expected = original.clone();
        expected.replace_range(range.clone(), replacement);
        let (result, work) = measure_document_work(|| document.replace(range, replacement));
        result.unwrap();
        assert!(work.full_projection_candidates > 0, "exercise the complete Source reconciliation: {work:?}");
        assert_eq!(table_identities(&document), identities);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        let reopened = Document::from_bytes(expected.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
        same_table_positions(&document, &reopened);
        document.try_undo().unwrap();
        assert_eq!(table_identities(&document), identities);
        assert_eq!(document.source_bytes(), original.as_bytes());
        document.try_redo().unwrap();
        assert_eq!(table_identities(&document), identities);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        same_table_positions(&document, &reopened);
    }
}

fn encode_fixture(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        _ => unreachable!(),
    }
}

#[test]
fn list_paragraph_split_keeps_dense_table_suffix_regional_and_identical() {
    use viem_core::document::{BoundaryAffinity, FileFormat, ModelRequest};
    let tables = "| first | second |\n| --- | --- |\n| alpha | beta |\n\n".repeat(4096);
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let original = encode_fixture(&format!("- first\n- second\n\n{tables}").replace('\n', "\r\n"), encoding);
        let expected = encode_fixture(&format!("- fi\n\n  rst\n- second\n\n{tables}").replace('\n', "\r\n"), encoding);
        let mut document = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        let identities = table_identities(&document);
        let request = ModelRequest::InsertListParagraph {
            document: document.id(), revision: document.revision(), at: 2,
            affinity: BoundaryAffinity::Downstream,
        };
        let (result, work) = measure_document_work(|| document.apply_model_request(request));
        result.unwrap();
        assert_eq!(work.full_projection_candidates, 0, "{encoding:?}: {work:?}");
        assert_eq!(work.source_full_materializations, 0, "{encoding:?}: {work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{encoding:?}: {work:?}");
        assert!(work.formatted_compatibility_materialized_bytes < 4096, "{encoding:?}: {work:?}");
        assert!(work.source_decoded_bytes < 8192, "{encoding:?}: {work:?}");
        assert!(work.projection_persistent_nodes_copied < 1024, "{encoding:?}: {work:?}");
        assert_eq!(document.source_bytes(), expected);
        assert_eq!(table_identities(&document), identities);
        let reopened = Document::from_bytes_with_file_format(expected.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        same_table_positions(&document, &reopened);
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), original);
        assert_eq!(table_identities(&document), identities);
        document.try_redo().unwrap();
        assert_eq!(document.source_bytes(), expected);
        assert_eq!(table_identities(&document), identities);
        same_table_positions(&document, &reopened);
    }
}

#[test]
fn whitespace_rich_paste_keeps_encoded_table_suffix_and_history_exact() {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
    use viem_core::document::{ClipboardFragment, FileFormat};
    use viem_core::layout::DocumentLayoutStyles;
    let donor = Document::from_bytes_with_file_format(
        encode_fixture("line  \r\n  \r\nafter **double** space", Encoding::Utf16Le),
        Encoding::Utf16Le, Format::Markdown, FileFormat::Dos,
    ).unwrap();
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), copied).unwrap();
    let register = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_register(register),
    ));
    let tables = "| first | second |\r\n| --- | --- |\r\n| alpha | beta |\r\n\r\n".repeat(4096);
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let original = encode_fixture(&format!("target\r\n\r\n{tables}"), encoding);
        let mut document = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        let identities = table_identities(&document);
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        let (result, work) = measure_document_work(|| commands.handle_with_clipboard_context(
            &mut document, InputEvent::Key(Key::PasteClipboard), &clipboard,
        ));
        assert_eq!(result.unwrap().status, CommandStatus::Complete);
        // Whitespace-rich paste still decodes the whole source during fallback
        // preparation. This guards the table splice and source/history
        // semantics without claiming that the entire paste is regional.
        assert!(work.projection_persistent_nodes_copied < 1024, "{encoding:?}: {work:?}");
        assert!(work.retained_memory_allocation_visits < 4096, "{encoding:?}: {work:?}");
        let saved = document.source_bytes();
        assert!(saved.starts_with(&encode_fixture("&#32;", encoding)));
        assert!(saved.ends_with(&original));
        assert_eq!(table_identities(&document), identities);
        let reopened = Document::from_bytes_with_file_format(saved.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
        assert_eq!(reopened.source_bytes(), saved);
        for candidate in [&document, &reopened] {
            assert!(candidate.text().starts_with(&format!("{copied}target\n")));
            let style = DocumentLayoutStyles::semantic_character_at(candidate.projection(), copied.find("double").unwrap(), false).unwrap();
            assert!(style.bold);
        }
        same_table_positions(&document, &reopened);
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), original);
        assert_eq!(table_identities(&document), identities);
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_eq!(document.source_bytes(), saved);
        assert_eq!(table_identities(&document), identities);
    }
}

#[test]
fn opening_shares_one_block_grammar_pass_across_projection_classifiers() {
    let source = "# Heading\n\n- list one\n- list two\n\n> quoted **text**\n> next line\n\n    indented code\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n| - | - |\n| x | y |\n\n<div>passive</div>\n\n".repeat(256);
    for format in [Format::Markdown, Format::MarkdownSource] {
        let (document, work) = measure_document_work(|| Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format));
        let document = document.unwrap();
        assert_eq!(work.markdown_block_parse_calls, 1, "{work:?}");
        assert_eq!(work.markdown_block_parsed_bytes, source.len(), "{work:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
