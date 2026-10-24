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
    for format in [Format::Markdown] {
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
    identities_in_tables(document.projection().tables())
}

fn identities_in_tables(tables: &viem_core::document::TableRows<viem_core::document::MarkdownTable>) -> Vec<(u64, Vec<(u64, Vec<u64>)>)> {
    tables.iter().map(|table| {
        (table.id, table.rows.iter().map(|row| {
            (row.id, row.cells.iter().map(|cell| cell.id).collect())
        }).collect())
    }).collect()
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

fn whitespace_rich_clipboard() -> viem_core::command::clipboard::ClipboardCommandContext {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::RegisterValue;
    use viem_core::document::{ClipboardFragment, FileFormat};
    let donor = Document::from_bytes_with_file_format(
        encode_fixture("line  \r\n  \r\nafter **double** space", Encoding::Utf16Le),
        Encoding::Utf16Le, Format::Markdown, FileFormat::Dos,
    ).unwrap();
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), copied).unwrap();
    let register = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_register(register),
    ))
}

fn cold_whitespace_paste(
    original: Vec<u8>,
    encoding: Encoding,
    at: usize,
    unchanged_prefix: &[u8],
    unchanged_suffix: &[u8],
    clipboard: &viem_core::command::clipboard::ClipboardCommandContext,
) -> viem_core::document::DocumentWorkStatistics {
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
    use viem_core::document::FileFormat;
    use viem_core::layout::DocumentLayoutStyles;
    let copied = "  \nafter double spac";
    let mut document = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
    // Clone persistent roots without materializing compatibility text or table
    // lookups. Identity and text comparisons happen only after the cold paste.
    let original_tables = document.projection().tables().clone();
    let original_text = document.projection().text_tree().clone();
    let clipboard_before = clipboard.clone();
    let mut commands = CommandInterpreter::new();
    let (result, work) = measure_document_work(|| {
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        assert!(commands.set_cursor(&document, at));
        commands.handle_with_clipboard_context(
            &mut document, InputEvent::Key(Key::PasteClipboard), clipboard,
        )
    });
    let output = result.unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
    assert!(output.document_changed);
    assert!(output.clipboard_writes.is_empty());
    assert_eq!(clipboard, &clipboard_before);
    let saved = document.source_bytes();
    assert!(saved.starts_with(unchanged_prefix));
    assert!(saved.ends_with(unchanged_suffix));
    let identities = identities_in_tables(&original_tables);
    assert_eq!(table_identities(&document), identities);
    let mut expected_text = original_text.slice(0..original_text.byte_len()).unwrap();
    expected_text.insert_str(at, copied);
    let reopened = Document::from_bytes_with_file_format(saved.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
    assert_eq!(reopened.source_bytes(), saved);
    for candidate in [&document, &reopened] {
        assert_eq!(candidate.text(), expected_text);
        let style = DocumentLayoutStyles::semantic_character_at(candidate.projection(), at + copied.find("double").unwrap(), false).unwrap();
        assert!(style.bold);
    }
    assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
    let block_shape = |document: &Document| {
        let mut owners = std::collections::HashMap::new();
        document.projection().blocks().iter().map(|block| {
            let containers = block.containers.iter().map(|member| {
                let next = owners.len();
                let owner = *owners.entry(member.container.id).or_insert(next);
                (owner, member.container.kind, member.starts_here, member.ends_here)
            }).collect::<Vec<_>>();
            (block.range.clone(), block.kind.clone(), block.style.clone(), block.list_loose, containers)
        }).collect::<Vec<_>>()
    };
    assert_eq!(block_shape(&document), block_shape(&reopened));
    same_table_positions(&document, &reopened);
    commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
    commands.handle(&mut document, InputEvent::key('u')).unwrap();
    assert_eq!(document.source_bytes(), original);
    assert_eq!(table_identities(&document), identities);
    commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
    assert_eq!(document.source_bytes(), saved);
    assert_eq!(table_identities(&document), identities);
    assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
    same_table_positions(&document, &reopened);
    work
}

fn assert_rich_paste_is_regional(work: viem_core::document::DocumentWorkStatistics) {
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.source_full_materializations, 0, "{work:?}");
    assert_eq!(work.source_full_materialized_bytes, 0, "{work:?}");
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 8192, "{work:?}");
    assert!(work.projected_formatted_bytes < 4096, "{work:?}");
    assert!(work.projection_persistent_nodes_copied < 1024, "{work:?}");
    assert!(work.retained_memory_allocation_visits < 4096, "{work:?}");
    assert!(work.clipboard_style_blocks_visited < 128, "{work:?}");
    assert!(work.clipboard_style_text_bytes < 4096, "{work:?}");
}

fn assert_suffix_independent_paste_bytes(works: &[viem_core::document::DocumentWorkStatistics]) {
    assert_eq!(works.len(), 2);
    for work in works {
        assert_rich_paste_is_regional(*work);
    }
    assert_eq!(works[0].source_decoded_bytes, works[1].source_decoded_bytes, "{works:?}");
    assert_eq!(works[0].source_range_materialized_bytes, works[1].source_range_materialized_bytes, "{works:?}");
    assert_eq!(works[0].projected_formatted_bytes, works[1].projected_formatted_bytes, "{works:?}");
    assert_eq!(works[0].clipboard_style_blocks_visited, works[1].clipboard_style_blocks_visited, "{works:?}");
    assert_eq!(works[0].clipboard_style_text_bytes, works[1].clipboard_style_text_bytes, "{works:?}");
}

#[test]
fn whitespace_rich_paste_keeps_encoded_table_suffix_and_history_exact() {
    let clipboard = whitespace_rich_clipboard();
    let mut measurements = Vec::new();
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let mut works = Vec::new();
        for table_count in [4, 4096] {
            let tables = "| first | second |\r\n| --- | --- |\r\n| alpha | beta |\r\n\r\n".repeat(table_count);
            let original = encode_fixture(&format!("target\r\n\r\n{tables}"), encoding);
            let work = cold_whitespace_paste(original.clone(), encoding, 0, &encode_fixture("&#32;", encoding), &original, &clipboard);
            works.push(work);
        }
        measurements.push(works);
    }
    for works in measurements {
        assert_suffix_independent_paste_bytes(&works);
    }
}

#[test]
fn whitespace_rich_paste_into_list_continuation_stays_regional_with_growing_table_suffix() {
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
    use viem_core::document::FileFormat;
    let clipboard = whitespace_rich_clipboard();
    let mut measurements = Vec::new();
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        // Author the empty continuation through the command path, then reopen
        // with each suffix so neither prior editing nor inspection warms it.
        let mut prefix = Document::from_bytes_with_file_format(
            encode_fixture("- first\r\n- tail", encoding), encoding, Format::Markdown, FileFormat::Dos,
        ).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut prefix, InputEvent::key('A')).unwrap();
        assert_eq!(commands.cursor(), 5);
        assert_eq!(commands.handle(&mut prefix, InputEvent::Key(Key::AltEnter)).unwrap().status, CommandStatus::Complete);
        let at = commands.cursor();
        assert_eq!(at, 6);
        let authored = prefix.source_bytes();
        let mut works = Vec::new();
        for table_count in [4, 4096] {
            let tables = "| first | second |\r\n| --- | --- |\r\n| alpha | beta |\r\n\r\n".repeat(table_count);
            let mut original = authored.clone();
            original.extend_from_slice(&encode_fixture(&format!("\r\n\r\n{tables}"), encoding));
            let unchanged_suffix = encode_fixture(&format!("- tail\r\n\r\n{tables}"), encoding);
            let work = cold_whitespace_paste(original, encoding, at, &encode_fixture("- first\r\n\r\n", encoding), &unchanged_suffix, &clipboard);
            works.push(work);
        }
        measurements.push(works);
    }
    for works in measurements {
        assert_suffix_independent_paste_bytes(&works);
    }
}

#[test]
fn empty_paragraph_put_and_enter_keep_whitespace_boundary_work_regional() {
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
    use viem_core::document::{BoundaryAffinity, FileFormat};
    for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        for put in [true, false] {
            let mut works = Vec::new();
            for table_count in [4, 4096] {
                let suffix = format!("b\n\nafter\n\n{}", "| first | second |\n| --- | --- |\n| alpha | beta |\n\n".repeat(table_count));
                let original = encode_fixture(&format!("before\n\na\n \n \n \n{suffix}").replace('\n', "\r\n"), encoding);
                let blank = if put { "\n" } else { " \n" };
                let expected = encode_fixture(&format!("before\n\na\n \n{blank}\n{suffix}").replace('\n', "\r\n"), encoding);
                let mut document = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
                let before_tables = document.projection().tables().clone();
                let before_text = document.projection().text_tree().clone();
                let mut commands = CommandInterpreter::new();
                let at = "before\na\n".len();
                assert!(commands.set_cursor_from_pointer(&document, at, BoundaryAffinity::Downstream, false));
                let before_operation = if put {
                    commands.handle(&mut document, InputEvent::key('d')).unwrap();
                    assert_eq!(commands.handle(&mut document, InputEvent::key('d')).unwrap().status, CommandStatus::Complete);
                    encode_fixture(&format!("before\n\na\n \n{suffix}").replace('\n', "\r\n"), encoding)
                } else {
                    assert!(commands.set_cursor_from_pointer(&document, at + 1, BoundaryAffinity::Downstream, true));
                    original.clone()
                };
                let event = if put { InputEvent::key('P') } else { InputEvent::Key(Key::Enter) };
                let (result, work) = measure_document_work(|| commands.handle(&mut document, event));
                let output = result.unwrap();
                assert_eq!(output.status, CommandStatus::Complete);
                assert!(output.document_changed);
                assert_eq!(work.source_full_materializations, 0, "{encoding:?} put={put}: {work:?}");
                assert_eq!(work.full_projection_candidates, 0, "{encoding:?} put={put}: {work:?}");
                assert_eq!(work.formatted_full_materialized_bytes, 0, "{encoding:?} put={put}: {work:?}");
                assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{encoding:?} put={put}: {work:?}");
                assert!(work.source_decoded_bytes < 8192, "{encoding:?} put={put}: {work:?}");
                assert!(work.projected_formatted_bytes < 4096, "{encoding:?} put={put}: {work:?}");
                assert!(work.clipboard_style_blocks_visited < 128, "{encoding:?} put={put}: {work:?}");
                assert!(work.clipboard_style_text_bytes < 4096, "{encoding:?} put={put}: {work:?}");
                assert_eq!(document.source_bytes(), expected);
                let reopened = Document::from_bytes_with_file_format(expected.clone(), encoding, Format::Markdown, FileFormat::Dos).unwrap();
                assert_eq!(document.text(), before_text.slice(0..before_text.byte_len()).unwrap());
                assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
                same_table_positions(&document, &reopened);
                assert_eq!(table_identities(&document), identities_in_tables(&before_tables));
                commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
                commands.handle(&mut document, InputEvent::key('u')).unwrap();
                assert_eq!(document.source_bytes(), before_operation);
                commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
                assert_eq!(document.source_bytes(), expected);
                if put {
                    commands.handle(&mut document, InputEvent::key('u')).unwrap();
                    commands.handle(&mut document, InputEvent::key('u')).unwrap();
                    assert_eq!(document.source_bytes(), original);
                }
                works.push(work);
            }
            assert_suffix_independent_paste_bytes(&works);
        }
    }
}

#[test]
fn opening_shares_one_block_grammar_pass_across_projection_classifiers() {
    let source = "# Heading\n\n- list one\n- list two\n\n> quoted **text**\n> next line\n\n    indented code\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n| - | - |\n| x | y |\n\n<div>passive</div>\n\n".repeat(256);
    for format in [Format::Markdown] {
        let (document, work) = measure_document_work(|| Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format));
        let document = document.unwrap();
        assert_eq!(work.markdown_block_parse_calls, 1, "{work:?}");
        assert_eq!(work.markdown_block_parsed_bytes, source.len(), "{work:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
