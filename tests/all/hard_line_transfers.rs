use viem_core::document::{
    Document, DocumentError, Encoding, FileFormat, Format, HardLineTransfer, ModelChangeKind,
    ModelRequest, ModelTransactionError, Revision,
};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|character| u8::try_from(character as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xff, 0xfe];
            bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = vec![0xfe, 0xff];
            bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            bytes
        }
    }
}

#[test]
fn transferring_quote_hard_lines_preserves_their_independent_container_depths() {
    let source = "> a\\\n> b\n\n>> c";
    for (operation, rows, destination, expected_text, depths) in [
        (
            HardLineTransfer::Copy,
            1..2,
            0,
            "b\na\nb\nc",
            vec![1, 1, 1, 2],
        ),
        (HardLineTransfer::Move, 0..1, 3, "b\nc\na", vec![1, 2, 1]),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let prepared = document
            .prepare_model_request(request(&document, operation, rows, destination))
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), expected_text);
        for (line, depth) in depths.into_iter().enumerate() {
            let range = document
                .hard_line_snapshot()
                .line(line)
                .unwrap()
                .content_range();
            let block = document
                .projection()
                .blocks()
                .iter()
                .find(|block| block.range.start <= range.start && range.end <= block.range.end)
                .unwrap();
            assert_eq!(block.quote_depth, depth);
        }
        let edited = document.source_bytes();
        let reopened =
            Document::from_bytes(edited.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), expected_text);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), edited);
    }
}

fn paragraph_source(text: &str, format: Format) -> String {
    if format == Format::Markdown {
        text.replace('\r', "\r\r")
    } else {
        text.to_owned()
    }
}

fn paragraph_lines(lines: &[&str], format: Format) -> String {
    lines.join(if format == Format::Markdown {
        "\r\r"
    } else {
        "\r"
    })
}

fn request(
    document: &Document,
    operation: HardLineTransfer,
    source_lines: std::ops::Range<usize>,
    destination: usize,
) -> ModelRequest {
    ModelRequest::TransferHardLines {
        document: document.id(),
        revision: document.revision(),
        operation,
        source_lines,
        destination,
    }
}

fn apply_patches(
    before: &[u8],
    prepared: &viem_core::document::PreparedModelTransaction,
) -> Vec<u8> {
    let mut result = before.to_vec();
    for patch in prepared.summary().source_patches().iter().rev() {
        result.splice(patch.range(), patch.replacement().iter().copied());
    }
    result
}

#[test]
fn mac_copy_preserves_markdown_source_literal_lf_and_all_encodings() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            let source = paragraph_source("# Hé\r**one**\nraw\r_last_", format);
            let expected_source =
                paragraph_source("**one**\nraw\r# Hé\r**one**\nraw\r_last_", format);
            let original = encoded(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                format,
                FileFormat::Mac,
            )
            .unwrap();
            let original_ids = (0..document.line_count())
                .map(|line| document.projection().hard_line_id(line).unwrap())
                .collect::<Vec<_>>();

            let prepared = document
                .prepare_model_request(request(&document, HardLineTransfer::Copy, 1..2, 0))
                .unwrap();
            assert_eq!(prepared.summary().kind(), ModelChangeKind::HardLineTransfer);
            assert_eq!(prepared.summary().source_patches().len(), 1);
            assert_eq!(
                apply_patches(&original, &prepared),
                encoded(&expected_source, encoding)
            );
            document.commit_model_transaction(prepared).unwrap();

            assert_eq!(document.source_bytes(), encoded(&expected_source, encoding));
            assert_eq!(document.line_count(), 4);
            let all = document
                .hard_line_snapshot()
                .capture(0..document.text().len())
                .unwrap();
            let expected_text = match format {
                Format::PlainText => "**one**\nraw\n# Hé\n**one**\nraw\n_last_",
                Format::Markdown => "one\nraw\nHé\none\nraw\nlast",
                _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
            };
            assert_eq!(all.text(), expected_text);
            // Only the three separators between authoritative lines are
            // marked. Both copied LF scalars remain ordinary line content.
            assert_eq!(all.break_offsets().len(), 3);
            assert_ne!(
                document.projection().hard_line_id(0).unwrap(),
                original_ids[1]
            );
            assert_eq!(
                document.projection().hard_line_id(2).unwrap(),
                original_ids[1]
            );

            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(!document.undo());
        }
    }
}

#[test]
fn copy_final_unterminated_line_synthesizes_only_the_required_boundary() {
    let mut document = Document::from_bytes_with_file_format(
        b"# one\r\r**two**".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Mac,
    )
    .unwrap();
    let prepared = document
        .prepare_model_request(request(&document, HardLineTransfer::Copy, 1..2, 0))
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    let patch = &prepared.summary().source_patches()[0];
    assert_eq!(patch.range(), 0..0);
    assert_eq!(patch.replacement(), b"**two**\r\r");
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), b"**two**\r\r# one\r\r**two**");
    assert_eq!(document.text(), "two\none\ntwo");
}

#[test]
fn copy_to_eof_rotates_an_existing_separator_and_handles_trailing_empty_lines() {
    let mut unterminated = Document::from_bytes_with_file_format(
        b"# one\r\r**two**".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Mac,
    )
    .unwrap();
    let prepared = unterminated
        .prepare_model_request(request(&unterminated, HardLineTransfer::Copy, 0..1, 2))
        .unwrap();
    assert_eq!(prepared.summary().source_patches()[0].range(), 14..14);
    assert_eq!(
        prepared.summary().source_patches()[0].replacement(),
        b"\r\r# one"
    );
    unterminated.commit_model_transaction(prepared).unwrap();
    assert_eq!(unterminated.source_bytes(), b"# one\r\r**two**\r\r# one");

    let mut trailing = Document::from_bytes_with_file_format(
        b"a\r".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    trailing
        .transfer_hard_lines(HardLineTransfer::Copy, 1..2, 2)
        .unwrap();
    assert_eq!(trailing.source_bytes(), b"a\r\r");
    assert_eq!(trailing.line_count(), 3);
    assert_eq!(trailing.text(), "a\n\n");
}

#[test]
fn move_final_unterminated_line_earlier_uses_two_local_patches() {
    let original = b"# a\r\r**b**\nraw".to_vec();
    let mut document = Document::from_bytes_with_file_format(
        original.clone(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Mac,
    )
    .unwrap();
    let moved_id = document.projection().hard_line_id(1).unwrap();
    let prepared = document
        .prepare_model_request(request(&document, HardLineTransfer::Move, 1..2, 0))
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 2);
    assert_eq!(prepared.summary().source_patches()[0].range(), 0..0);
    assert_eq!(
        prepared.summary().source_patches()[0].replacement(),
        b"**b**\nraw\r\r"
    );
    assert_eq!(prepared.summary().source_patches()[1].range(), 3..14);
    assert_eq!(prepared.summary().source_patches()[1].replacement(), b"");
    assert_eq!(apply_patches(&original, &prepared), b"**b**\nraw\r\r# a");

    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), b"**b**\nraw\r\r# a");
    assert_eq!(document.text(), "b\nraw\na");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.projection().hard_line_id(0), Some(moved_id));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn move_to_eof_preserves_syntax_and_global_final_line_state() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            let source = paragraph_source("# a\r**bé**\r_last_", format);
            let expected = paragraph_source("**bé**\r_last_\r# a", format);
            let original = encoded(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                format,
                FileFormat::Mac,
            )
            .unwrap();
            let first_id = document.projection().hard_line_id(0).unwrap();
            let prepared = document
                .prepare_model_request(request(&document, HardLineTransfer::Move, 0..1, 3))
                .unwrap();
            assert_eq!(prepared.summary().source_patches().len(), 2);
            assert_eq!(
                apply_patches(&original, &prepared),
                encoded(&expected, encoding)
            );
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.source_bytes(), encoded(&expected, encoding));
            assert_eq!(document.projection().hard_line_id(2), Some(first_id));
            assert_eq!(document.history_status().node_count, 2);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
        }
    }

    let mut trailing = Document::from_bytes_with_file_format(
        b"a\rb\r".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    trailing
        .transfer_hard_lines(HardLineTransfer::Move, 0..1, 3)
        .unwrap();
    assert_eq!(trailing.source_bytes(), b"b\r\ra");
    assert_eq!(trailing.text(), "b\n\na");
}

#[test]
fn transfer_requests_report_foreign_stale_range_and_destination_errors_atomically() {
    let document = Document::new("a\nb\nc");
    let other = Document::new("other");

    let foreign = ModelRequest::TransferHardLines {
        document: other.id(),
        revision: document.revision(),
        operation: HardLineTransfer::Copy,
        source_lines: 0..1,
        destination: 0,
    };
    assert_eq!(
        document.prepare_model_request(foreign).unwrap_err(),
        ModelTransactionError::WrongDocument {
            expected: document.id(),
            actual: other.id(),
        }
    );

    let stale = ModelRequest::TransferHardLines {
        document: document.id(),
        revision: Revision(99),
        operation: HardLineTransfer::Copy,
        source_lines: 0..1,
        destination: 0,
    };
    assert_eq!(
        document.prepare_model_request(stale).unwrap_err(),
        ModelTransactionError::StaleRevision {
            expected: Revision(99),
            actual: Revision(0),
        }
    );

    for (source_lines, destination, expected) in [
        (
            1..1,
            0,
            DocumentError::InvalidHardLineTransferRange {
                start: 1,
                end: 1,
                line_count: 3,
            },
        ),
        (
            0..4,
            0,
            DocumentError::InvalidHardLineTransferRange {
                start: 0,
                end: 4,
                line_count: 3,
            },
        ),
        (
            0..1,
            4,
            DocumentError::InvalidHardLineTransferDestination {
                destination: 4,
                line_count: 3,
            },
        ),
    ] {
        assert_eq!(
            document
                .prepare_model_request(request(
                    &document,
                    HardLineTransfer::Copy,
                    source_lines,
                    destination,
                ))
                .unwrap_err(),
            ModelTransactionError::Document(expected)
        );
    }

    assert_eq!(
        document
            .prepare_model_request(request(&document, HardLineTransfer::Move, 1..3, 2))
            .unwrap_err(),
        ModelTransactionError::Document(DocumentError::HardLineTransferDestinationInsideSource {
            destination: 2,
            source: 1..3,
        })
    );
    for destination in [1, 3] {
        let no_op = document
            .prepare_model_request(request(
                &document,
                HardLineTransfer::Move,
                1..3,
                destination,
            ))
            .unwrap();
        assert!(no_op.is_no_op());
        assert_eq!(no_op.after_revision(), document.revision());
        assert!(no_op.summary().source_patches().is_empty());
    }
    assert_eq!(document.source_bytes(), b"a\nb\nc");
    assert_eq!(document.revision(), Revision(0));
    assert_eq!(document.history_status().node_count, 1);
}

#[test]
fn prepared_transfer_rechecks_revision_before_commit() {
    let mut document = Document::new("a\nb");
    let prepared = document
        .prepare_model_request(request(&document, HardLineTransfer::Copy, 0..1, 2))
        .unwrap();
    document.insert(0, "!").unwrap();
    let before = document.source_bytes();
    let history = document.history_status();
    assert!(matches!(
        document.commit_model_transaction(prepared),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(document.source_bytes(), before);
    assert_eq!(document.history_status(), history);
}

#[test]
fn every_valid_gap_reorders_source_lines_without_regeneration() {
    for format in [Format::PlainText, Format::Markdown] {
        for trailing_break in [false, true] {
            let mut lines = match format {
                Format::PlainText => vec!["a", "bb", "ccc"],
                Format::Markdown => vec!["# a", "**bb**", "_ccc_"],
                _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
            };
            if trailing_break {
                lines.push("");
            }
            let separator = if format == Format::Markdown {
                "\r\r"
            } else {
                "\r"
            };
            let source = lines.join(separator);
            let line_count = lines.len();

            for start in 0..line_count {
                for end in start + 1..=line_count {
                    for destination in 0..=line_count {
                        let mut expected_copy = lines.clone();
                        let copied = lines[start..end].to_vec();
                        expected_copy.splice(destination..destination, copied);
                        let mut copy = Document::from_bytes_with_file_format(
                            source.as_bytes().to_vec(),
                            Encoding::Utf8,
                            format,
                            FileFormat::Mac,
                        )
                        .unwrap();
                        copy.transfer_hard_lines(HardLineTransfer::Copy, start..end, destination)
                            .unwrap_or_else(|error| panic!("copy {start}..{end} to {destination}, {format:?}, trailing={trailing_break}: {error:?}"));
                        assert_eq!(
                            copy.source_bytes(),
                            paragraph_lines(&expected_copy, format).as_bytes(),
                            "copy {start}..{end} to {destination}, {format:?}, trailing={trailing_break}"
                        );

                        if start < destination && destination < end {
                            let unchanged = Document::from_bytes_with_file_format(
                                source.as_bytes().to_vec(),
                                Encoding::Utf8,
                                format,
                                FileFormat::Mac,
                            )
                            .unwrap();
                            assert!(matches!(
                                unchanged.prepare_model_request(request(
                                    &unchanged,
                                    HardLineTransfer::Move,
                                    start..end,
                                    destination,
                                )),
                                Err(ModelTransactionError::Document(
                                    DocumentError::HardLineTransferDestinationInsideSource { .. }
                                ))
                            ));
                            continue;
                        }
                        if destination == start || destination == end {
                            let mut no_op = Document::from_bytes_with_file_format(
                                source.as_bytes().to_vec(),
                                Encoding::Utf8,
                                format,
                                FileFormat::Mac,
                            )
                            .unwrap();
                            no_op
                                .transfer_hard_lines(
                                    HardLineTransfer::Move,
                                    start..end,
                                    destination,
                                )
                                .unwrap();
                            assert_eq!(no_op.source_bytes(), source.as_bytes());
                            assert_eq!(no_op.revision(), Revision(0));
                            assert_eq!(no_op.history_status().node_count, 1);
                            continue;
                        }
                        let mut expected_move = lines.clone();
                        let moved = expected_move.drain(start..end).collect::<Vec<_>>();
                        let adjusted = if destination > end {
                            destination - (end - start)
                        } else {
                            destination
                        };
                        expected_move.splice(adjusted..adjusted, moved);
                        let mut moved_document = Document::from_bytes_with_file_format(
                            source.as_bytes().to_vec(),
                            Encoding::Utf8,
                            format,
                            FileFormat::Mac,
                        )
                        .unwrap();
                        moved_document
                            .transfer_hard_lines(HardLineTransfer::Move, start..end, destination)
                            .unwrap_or_else(|error| panic!("move {start}..{end} to {destination}, {format:?}, trailing={trailing_break}: {error:?}"));
                        assert_eq!(
                            moved_document.source_bytes(),
                            paragraph_lines(&expected_move, format).as_bytes(),
                            "move {start}..{end} to {destination}, {format:?}, trailing={trailing_break}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_dos_delimiter_spellings_are_carried_without_normalization() {
    let mut copy = Document::from_bytes_with_file_format(
        b"# a\r\n\r\n**b**\n\n_c_".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    copy.transfer_hard_lines(HardLineTransfer::Copy, 1..2, 0)
        .unwrap();
    assert_eq!(copy.source_bytes(), b"**b**\n\n# a\r\n\r\n**b**\n\n_c_");

    let mut moved = Document::from_bytes_with_file_format(
        b"# a\r\n\r\n**b**\n\n_c_".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    moved
        .transfer_hard_lines(HardLineTransfer::Move, 0..1, 3)
        .unwrap();
    assert_eq!(moved.source_bytes(), b"**b**\n\n_c_\r\n\r\n# a");
}

#[test]
fn markdown_transfers_complete_wrapped_paragraphs_and_explicit_hard_lines() {
    for (source, expected, line) in [
        (
            "First soft\ncontinued\n\nSecond",
            "First soft continued\nSecond\nFirst soft continued",
            0,
        ),
        ("**one**\\\ntwo\n\nthird", "one\ntwo\nthird\none", 0),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let ids = (0..document.line_count())
            .map(|index| document.projection().hard_line_id(index).unwrap())
            .collect::<Vec<_>>();
        let count = document.line_count();
        let prepared = document
            .prepare_model_request(request(
                &document,
                HardLineTransfer::Copy,
                line..line + 1,
                count,
            ))
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(
            prepared.summary().source_patches()[0].range(),
            source.len()..source.len()
        );
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), expected);
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(document.projection().hard_line_id(index), Some(*id));
        }
        assert_ne!(document.projection().hard_line_id(count), Some(ids[line]));
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(reopened.text(), document.text());
        assert_eq!(
            reopened.projection().style_spans(),
            document.projection().style_spans()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn unrelated_fences_do_not_block_prose_transfers_and_complete_code_owners_travel() {
    for (source, operation, lines, destination, expected) in [
        (
            "```rust\nfn x() {}\n```\n\npara",
            HardLineTransfer::Copy,
            1..2,
            2,
            "```rust\nfn x() {}\n```\n\npara\n\npara",
        ),
        (
            "```rust\nfn x() {}\n```\n\npara",
            HardLineTransfer::Move,
            1..2,
            0,
            "para\n\n```rust\nfn x() {}\n```",
        ),
        (
            "```rust\nfn x() {}\n```\n\npara",
            HardLineTransfer::Copy,
            0..1,
            1,
            "```rust\nfn x() {}\n```\n\n```rust\nfn x() {}\n```\n\npara",
        ),
        (
            "```rust\na\nb\n```\n\npara",
            HardLineTransfer::Move,
            2..3,
            0,
            "para\n\n```rust\na\nb\n```",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let prepared = document
            .prepare_model_request(request(&document, operation, lines.clone(), destination))
            .unwrap_or_else(|e| {
                panic!("{source:?} {operation:?} {lines:?} -> {destination}: {e:?}")
            });
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
            .unwrap();
        assert_eq!(document.text(), fresh.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn contextual_ex_copy_and_move_work_beside_fenced_code() {
    use viem_core::command::{CommandStatus, InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "```rust\nfn x() {}\n```\n\npara";
    for (ex, expected) in [
        (":t.", "```rust\nfn x() {}\n```\n\npara\n\npara"),
        (":m0", "para\n\n```rust\nfn x() {}\n```"),
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: core.document().text().find("para").unwrap(),
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        for event in ex
            .chars()
            .map(InputEvent::key)
            .chain([InputEvent::Key(Key::Enter)])
        {
            let result = core
                .handle_with_layout(view, CoreEvent::Input(event))
                .unwrap();
            let status = result.command.unwrap().status;
            assert!(
                matches!(status, CommandStatus::Complete | CommandStatus::Pending),
                "{ex}: {status:?}"
            );
        }
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn markdown_alternate_structures_copy_complete_visible_lines() {
    let mut failures = Vec::new();
    for source in [
        "\n",
        "\na",
        "a<br>b",
        "Setext\n===",
        "a\n=\n\nb",
        "1) a\n2) b",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.text().to_owned();
        let mut lines: Vec<_> = before.split('\n').collect();
        lines.insert(1, lines[0]);
        let expected = lines.join("\n");
        let prepared = match document.prepare_model_request(request(
            &document,
            HardLineTransfer::Copy,
            0..1,
            1,
        )) {
            Ok(prepared) => prepared,
            Err(error) => {
                failures.push(format!("{source:?}: {error:?}"));
                continue;
            }
        };
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), expected, "{source:?}");
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
            .unwrap();
        assert_eq!(fresh.text(), expected);
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn markdown_alternate_structures_move_complete_visible_lines() {
    for source in ["\na", "a<br>b", "a\n=\n\nb", "1) a\n2) b"] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.text().to_owned();
        let mut lines: Vec<_> = before.split('\n').collect();
        let first = lines.remove(0);
        lines.push(first);
        let expected = lines.join("\n");
        let prepared = document
            .prepare_model_request(request(
                &document,
                HardLineTransfer::Move,
                0..1,
                lines.len(),
            ))
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), expected, "{source:?}");
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
            .unwrap();
        assert_eq!(fresh.text(), expected);
        document.try_undo().unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
