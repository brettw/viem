//! Structural large-file regressions: work and fidelity rather than timing.
use super::*;

fn prepare(document: &Document, edits: Vec<TextEdit>) -> PreparedModelTransaction {
    document.prepare_model_request(ModelRequest::ApplyTextEdits {
        document: document.id(), revision: document.revision(), edits,
    }).unwrap()
}

fn assert_local(work: ProjectionWorkStatistics, max_bytes: usize) {
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines, "{work:?}");
    assert!(work.decoded_source_bytes() <= max_bytes, "{work:?}");
    assert_eq!(work.full_text_bytes_materialized(), 0, "{work:?}");
    assert!(work.persistent_nodes_copied() < 2048, "{work:?}");
}

#[test]
fn literal_newlines_and_joins_are_local_and_undoable_with_default_budget() {
    for format in [Format::PlainText, Format::Code] {
        let source = "alpha beta gamma\n".repeat(65_536).into_bytes();
        let mut document = Document::from_bytes(source.clone(), Encoding::Utf8, format).unwrap();
        let old_tail = document.projection().hard_line_id(65_535).unwrap();
        let prepared = prepare(&document, vec![TextEdit::new(5..5, "\nnew")]);
        assert_local(prepared.summary().projection_work(), 8192);
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.projection().hard_line_count(), 65_538);
        assert_eq!(document.projection().hard_line_id(65_536), Some(old_tail));
        assert!(!document.projection().compatibility_text_is_materialized());
        assert!(document.history_status().can_undo);
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), source);
        document.try_redo().unwrap();
        let prepared = prepare(&document, vec![TextEdit::new(5..6, "")]);
        assert_local(prepared.summary().projection_work(), 8192);
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.projection().hard_line_count(), 65_537);
        let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), reopened.text());
    }
}

#[test]
fn literal_edits_inside_multimegabyte_lines_decode_only_mapping_chunks() {
    for format in [Format::PlainText, Format::Code] {
        let original = vec![b'a'; 2 * 1024 * 1024];
        for offset in [0, 1024 * 1024 + 13, original.len()] {
            let mut document = Document::from_bytes(original.clone(), Encoding::Utf8, format).unwrap();
            let prepared = prepare(&document, vec![TextEdit::new(offset..offset, "x\ny")]);
            assert_local(prepared.summary().projection_work(), 16 * 1024);
            document.commit_model_transaction(prepared).unwrap();
            assert!(!document.projection().compatibility_text_is_materialized());
            assert_eq!(document.projection().hard_line_count(), 2);
            let mut expected = original.clone();
            expected.splice(offset..offset, b"x\ny".iter().copied());
            assert_eq!(document.source_bytes(), expected);
            assert_eq!(document.projection().hard_line_range(0), Some(0..offset + 1));
            assert_eq!(document.projection().hard_line_range(1), Some(offset + 2..original.len() + 3));
            document.try_undo().unwrap();
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn disjoint_edits_in_one_long_literal_line_keep_separate_projection_windows() {
    for format in [Format::PlainText, Format::Code] {
        let original = vec![b'a'; 2 * 1024 * 1024];
        let mut document = Document::from_bytes(original.clone(), Encoding::Utf8, format).unwrap();
        let last = original.len() - 11;
        let prepared = prepare(&document, vec![TextEdit::new(10..11, "X"), TextEdit::new(last..last + 1, "Y")]);
        assert_local(prepared.summary().projection_work(), 32 * 1024);
        document.commit_model_transaction(prepared).unwrap();
        let mut expected = original.clone();
        expected[10] = b'X'; expected[last] = b'Y';
        assert_eq!(document.source_bytes(), expected);
        assert!(document.history_status().can_undo);
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), original);
    }
}

#[test]
fn distant_group_undo_copies_only_changed_source_bytes() {
    let mut document = Document::new(&"a".repeat(8 * 1024 * 1024));
    document.begin_edit_group();
    document.replace(10..11, "XX").unwrap();
    document.replace(8 * 1024 * 1024 - 10..8 * 1024 * 1024 - 9, "YYY").unwrap();
    document.end_edit_group();
    for navigation in [HistoryNavigationRequest::Undo, HistoryNavigationRequest::Redo] {
        let prepared = document.prepare_model_request(ModelRequest::NavigateHistory {
            document: document.id(), revision: document.revision(), navigation,
        }).unwrap();
        let patches = prepared.summary().source_patches();
        assert_eq!(patches.len(), 2);
        assert!(patches.iter().map(|patch| patch.replacement().len()).sum::<usize>() <= 5);
        document.commit_model_transaction(prepared).unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
    }
}

#[test]
fn additional_history_budget_excludes_current_only_bookkeeping_at_large_sizes() {
    for megabytes in [1, 16] {
        let row = format!("{}\n", "a".repeat(127));
        let mut document = Document::new(row.repeat(megabytes * 1024 * 1024 / row.len()));
        let cold = document.history_status();
        assert!(cold.additional_history_memory_bytes <= 4096, "{megabytes}MiB: {cold:?}");
        let allowance = 512 * 1024;
        document.set_history_retention_policy(HistoryRetentionPolicy::additional_history(100, allowance));
        document.replace(10..11, "X").unwrap();
        assert!(document.history_status().can_undo, "{megabytes}MiB must retain a small edit above its live state");
        assert!(document.history_status().additional_history_memory_bytes <= allowance);
        document.history.assert_memory_matches_full_recount();

        let before = document.history_status();
        document.history.begin_command_checkpoint();
        document.replace(20..21, "Y").unwrap();
        document.history.rollback_command_checkpoint();
        assert_eq!(document.history_status(), before);
        document.history.assert_memory_matches_full_recount();

        document.history.begin_command_checkpoint();
        document.replace(20..21, "Y").unwrap();
        document.history.commit_command_checkpoint();
        document.history.assert_memory_matches_full_recount();
        assert!(document.history_status().can_undo);
        assert!(document.history_status().additional_history_memory_bytes <= allowance);
        assert!(!document.projection().compatibility_text_is_materialized());
    }
}

#[test]
fn literal_chunk_edits_preserve_encoding_bom_and_line_endings() {
    for format in [Format::PlainText, Format::Code] {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let body = "caf\u{e9} ".repeat(8192);
                let mut source = encoding.bom_bytes().to_vec();
                source.extend(encoding.encode_fragment(&body).unwrap());
                let mut document = Document::from_bytes_with_file_format(source, encoding, format, file_format).unwrap();
                let offset = "caf\u{e9} ".len() * 4096;
                let prepared = prepare(&document, vec![TextEdit::new(offset..offset, "\nZ")]);
                assert_local(prepared.summary().projection_work(), 32 * 1024);
                document.commit_model_transaction(prepared).unwrap();
                let mut expected_text = body.clone();
                expected_text.insert_str(offset, "\nZ");
                let mut expected_source = encoding.bom_bytes().to_vec();
                expected_source.extend(encoding.encode_fragment(&expected_text.replace('\n', file_format.spelling())).unwrap());
                assert_eq!(document.source_bytes(), expected_source, "{format:?} {encoding:?} {file_format:?}");
                assert_eq!(document.text(), expected_text);
                let reopened = Document::from_bytes_with_file_format(expected_source, encoding, format, file_format).unwrap();
                assert_eq!(document.state().source_hard_lines, reopened.state().source_hard_lines);
            }
        }
    }
}

#[test]
fn mac_literal_payload_typing_and_newlines_keep_bounded_projection_work() {
    for format in [Format::PlainText, Format::Code] {
        for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
            // Forced Mac leaves the LF as ordinary content in one long hard
            // line. Inserted payload newlines must instead serialize as CR.
            let body = format!("{}\n{}", "a".repeat(65_536), "b".repeat(65_536));
            let mut source = encoding.bom_bytes().to_vec();
            source.extend(encoding.encode_fragment(&body).unwrap());
            let mut document = Document::from_bytes_with_file_format(
                source, encoding, format, FileFormat::Mac,
            ).unwrap();
            assert!(!document.projection().compatibility_text_is_materialized());
            for (text, breaks) in [("X", vec![]), ("\nZ", vec![0])] {
                let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), text, breaks).unwrap();
                let prepared = document.prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                    document.id(), document.revision(), vec![FormattedPayloadEdit::new(70_000..70_000, payload)],
                )).unwrap();
                assert_local(prepared.summary().projection_work(), 32 * 1024);
                document.commit_model_transaction(prepared).unwrap();
                assert!(!document.projection().compatibility_text_is_materialized());
            }
            assert_eq!(document.projection().hard_line_count(), 2);
            assert_eq!(document.projection().hard_line_range(0), Some(0..70_000));
            let reopened = Document::from_bytes_with_file_format(
                document.source_bytes(), encoding, format, FileFormat::Mac,
            ).unwrap();
            assert_eq!(document.state().source_hard_lines, reopened.state().source_hard_lines);
            assert_eq!(document.text(), reopened.text());
        }
    }
}
