use viem_core::command::ex::parse_ex;
use viem_core::command::ex_execute::{execute_ex, ExExecutionContext, ExExecutionState};
use viem_core::document::{Document, Encoding, FileFormat, Format, HardLineTransfer, ModelRequest};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect(),
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

fn check(
    source: &str,
    command: &str,
    expected_source: &str,
    encoding: Encoding,
    file_format: FileFormat,
) {
    let original = encoded(source, encoding);
    let mut document = Document::from_bytes_with_file_format(
        original.clone(),
        encoding,
        Format::MarkdownSource,
        file_format,
    )
    .unwrap();
    let expected = encoded(expected_source, encoding);
    let outcome = execute_ex(
        &mut document,
        &mut ExExecutionState::default(),
        &ExExecutionContext::default(),
        &parse_ex(command).unwrap(),
        &(),
    )
    .unwrap_or_else(|error| panic!("{command} on {source:?}: {error:?}"));
    assert!(outcome.register_effects.is_empty());
    assert_eq!(document.source_bytes(), expected, "{command} on {source:?}");
    let fresh = Document::from_bytes_with_file_format(
        expected.clone(),
        encoding,
        Format::MarkdownSource,
        file_format,
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(document.line_count(), fresh.line_count());
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.range, &b.kind, &b.style))
            .collect::<Vec<_>>(),
        fresh
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.range, &b.kind, &b.style))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.direct_paragraph, &b.direct_default_character))
            .collect::<Vec<_>>(),
        fresh
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.direct_paragraph, &b.direct_default_character))
            .collect::<Vec<_>>()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(!document.undo(), "one source transaction");
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected);
}

#[test]
fn copy_move_and_sort_address_visible_rows_after_hidden_source_separators() {
    let source = "## Hé\r\n \t\r\n__two__\r\nline\r\n\r\nTail";
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (command, expected) in [
            (
                ":2,3copy 4",
                "## Hé\r\n \t\r\n__two__\r\nline\r\n\r\nTail\r\n\r\n__two__\r\nline",
            ),
            (":4move 0", "Tail\r\n\r\n## Hé\r\n \t\r\n__two__\r\nline"),
            (":2,3sort!", "## Hé\r\n \t\r\nline\r\n__two__\r\n\r\nTail"),
        ] {
            check(source, command, expected, encoding, FileFormat::Dos);
        }
    }
}

#[test]
fn sort_keeps_mixed_hidden_separator_bytes_in_their_original_slots() {
    check(
        "z\r\n \t\n__a__\n\nb\r\n\r\n",
        ":sort",
        "__a__\r\n \t\nb\n\nz\r\n\r\n",
        Encoding::Utf8,
        FileFormat::Dos,
    );
    check(
        "z\r\r__a__\r\rb",
        ":sort",
        "__a__\r\rb\r\rz",
        Encoding::Utf8,
        FileFormat::Mac,
    );
    check(
        "b\n\n\n\na",
        ":sort",
        "\n\na\n\nb",
        Encoding::Utf8,
        FileFormat::Unix,
    );
    check(
        "z\n\na\n\na\n\nb",
        ":sort u",
        "a\n\nb\n\nz",
        Encoding::Utf8,
        FileFormat::Unix,
    );
}

#[test]
fn source_copy_may_change_contextual_code_styling_without_changing_literal_syntax() {
    check(
        "```\ncode\n```\n\nTail",
        ":2copy 4",
        "```\ncode\n```\n\nTail\ncode",
        Encoding::Utf8,
        FileFormat::Unix,
    );
}

#[test]
fn empty_source_row_copy_preserves_its_formatted_boundary() {
    for (source, expected) in [("a\n", "a\n\n\n\n"), ("a\n\n", "a\n\n\n\n")] {
        check(
            source,
            ":2copy 2",
            expected,
            Encoding::Utf8,
            FileFormat::Unix,
        );
    }
}

#[test]
fn every_copy_and_move_gap_preserves_empty_source_rows_and_line_identity() {
    for source in ["a\n\n\n\nb\n\n", "a\nb\n\n\n\n"] {
        let original = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        let lines = original.text().split('\n').collect::<Vec<_>>();
        for start in 0..lines.len() {
            for end in start + 1..=lines.len() {
                for destination in 0..=lines.len() {
                    for operation in [HardLineTransfer::Copy, HardLineTransfer::Move] {
                        if operation == HardLineTransfer::Move
                            && start <= destination
                            && destination <= end
                        {
                            continue;
                        }
                        let mut document = Document::from_bytes(
                            source.as_bytes().to_vec(),
                            Encoding::Utf8,
                            Format::MarkdownSource,
                        )
                        .unwrap();
                        let ids = (0..lines.len())
                            .map(|line| document.projection().hard_line_id(line).unwrap())
                            .collect::<Vec<_>>();
                        let mut expected_indices = (0..lines.len()).collect::<Vec<_>>();
                        let at = if operation == HardLineTransfer::Move {
                            expected_indices.drain(start..end);
                            if destination > end {
                                destination - (end - start)
                            } else {
                                destination
                            }
                        } else {
                            destination
                        };
                        expected_indices.splice(at..at, start..end);
                        let expected = expected_indices
                            .iter()
                            .map(|index| lines[*index])
                            .collect::<Vec<_>>()
                            .join("\n");
                        let prepared = document.prepare_model_request(ModelRequest::TransferHardLines {
                            document: document.id(), revision: document.revision(), operation, source_lines: start..end, destination,
                        }).unwrap_or_else(|error| panic!("{operation:?} {start}..{end} to {destination} on {source:?}: {error:?}"));
                        let mut patched = source.as_bytes().to_vec();
                        for patch in prepared.summary().source_patches().iter().rev() {
                            patched.splice(patch.range(), patch.replacement().iter().copied());
                        }
                        document.commit_model_transaction(prepared).unwrap();
                        assert_eq!(document.text(), expected);
                        assert_eq!(document.source_bytes(), patched);
                        if operation == HardLineTransfer::Move {
                            for (row, index) in expected_indices.iter().enumerate() {
                                assert_eq!(
                                    document.projection().hard_line_id(row),
                                    Some(ids[*index])
                                );
                            }
                        }
                        assert!(document.undo());
                        assert_eq!(document.source_bytes(), source.as_bytes());
                        assert!(document.redo());
                        assert_eq!(document.text(), expected);
                    }
                }
            }
        }
    }
}
