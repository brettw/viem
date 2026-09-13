use viem_core::document::{Document, Encoding, FileFormat, Format};

#[test]
fn authored_marker_on_markdown_source_continuation_stays_literal() {
    for (encoding, endings, delimiter) in [
        (Encoding::Utf8, FileFormat::Unix, "\n"),
        (Encoding::Utf16Le, FileFormat::Mac, "\r"),
        (Encoding::Utf16Be, FileFormat::Dos, "\r\n"),
    ] {
        for replacement in ["- ", "+ ", "1. ", "2) "] {
            let source = format!("1. first{delimiter}   continuation{delimiter}2. second");
            let bytes = encode(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                bytes.clone(),
                encoding,
                Format::Markdown,
                endings,
            )
            .unwrap();
            document.replace(6..8, replacement).unwrap();
            let fresh = Document::from_bytes_with_file_format(
                document.source_bytes(),
                encoding,
                Format::Markdown,
                endings,
            )
            .unwrap();
            assert_eq!(
                document.text(),
                format!("first {replacement}ntinuation\nsecond")
            );
            assert_eq!(document.text(), fresh.text());
            assert_eq!(
                document.projection().provenance(),
                fresh.projection().provenance()
            );
            assert_eq!(document.projection().blocks().len(), 2);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn malformed_html_source_prefix_edit_recomputes_prose_flow_boundaries() {
    let source = "e\u{301}A <b>bold</אבx&am<; </p>\r\n<e\u{301}!--keep--&amp;<p>Ta\rl<e\u{301}p**>";
    let bytes = encode(source, Encoding::Utf16Be);
    let mut document = Document::from_bytes_with_file_format(
        bytes.clone(),
        Encoding::Utf16Be,
        Format::HtmlSource,
        FileFormat::Dos,
    )
    .unwrap();
    document.replace_physical_source(24..24, "\\").unwrap();
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        Encoding::Utf16Be,
        Format::HtmlSource,
        FileFormat::Dos,
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    let ranges = |document: &Document| {
        (0..document.projection().presentation_line_count(true))
            .map(|line| document.projection().presentation_line_range(line, true))
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(&document), ranges(&fresh));
    assert_eq!(ranges(&document).len(), 2);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), bytes);
}

#[test]
fn counted_open_lines_in_markdown_lists_keep_structural_ownership_and_repeat() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    for source in [
        "- first\n  continuation\n- second",
        "3. first\n   continuation\n1. second",
        "- \n- second",
    ] {
        for above in [false, true] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let original = document.source_bytes();
            let first = document.text().split('\n').next().unwrap().to_owned();
            let mut commands = CommandInterpreter::new();
            for key in [Key::Char('2'), Key::Char(if above { 'O' } else { 'o' })] {
                commands
                    .handle(&mut document, InputEvent::Key(key))
                    .unwrap_or_else(|error| panic!("{source:?} above={above}: {error:?}"));
            }
            commands
                .handle(&mut document, InputEvent::text("Next"))
                .unwrap();
            commands
                .handle(&mut document, InputEvent::Key(Key::Escape))
                .unwrap();
            let expected = if above {
                format!("Next\nNext\n{first}\nsecond")
            } else {
                format!("{first}\nNext\nNext\nsecond")
            };
            assert_eq!(document.text(), expected, "{source:?} above={above}");
            assert!(document.projection().blocks().iter().all(|block| matches!(
                block.kind,
                viem_core::document::BlockKind::ListItem { .. }
            )));
            let once = document.source_bytes();
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('.')))
                .unwrap();
            assert_eq!(document.text().matches("Next").count(), 4);
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('u')))
                .unwrap();
            assert_eq!(document.source_bytes(), once);
            commands
                .handle(&mut document, InputEvent::Key(Key::Char('u')))
                .unwrap();
            assert_eq!(document.source_bytes(), original);
        }
    }
}

#[test]
fn source_open_lines_preserve_paired_separator_rows_and_code_breaks() {
    use viem_core::document::ModelRequest;
    for source in [
        "👩‍💻\n\na\n\n",
        "a\nb",
        "a\n\nb",
        "a\n\n\nb",
        "a\n\n\n\nb",
        "```\na\n\nb\n```",
        "```\na\n```\n\nTail",
        "    code\n\nTail",
        "- first\n- second",
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let original = encode(&source.replace('\n', "\r\n"), encoding);
            let before = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                Format::MarkdownSource,
                FileFormat::Dos,
            )
            .unwrap();
            let points = (0..before.projection().hard_line_count())
                .flat_map(|line| {
                    let range = before.projection().hard_line_range(line).unwrap();
                    [range.start, range.end]
                })
                .collect::<Vec<_>>();
            for at in points {
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::MarkdownSource,
                    FileFormat::Dos,
                )
                .unwrap();
                let mut expected = document.text().to_owned();
                expected.insert(at, '\n');
                document
                    .apply_model_request(ModelRequest::OpenLine {
                        document: document.id(),
                        revision: document.revision(),
                        at,
                        origin: at,
                        after: true,
                    })
                    .unwrap_or_else(|error| panic!("{source:?} at{at}: {error:?}"));
                assert_eq!(document.text(), expected);
                let fresh = Document::from_bytes_with_file_format(
                    document.source_bytes(),
                    encoding,
                    Format::MarkdownSource,
                    FileFormat::Dos,
                )
                .unwrap();
                assert_eq!(
                    document.projection().provenance(),
                    fresh.projection().provenance()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }
}

#[test]
fn contextual_markdown_sort_owners_return_precise_atomic_policy() {
    use viem_core::document::{DocumentError, ModelRequest, ModelTransactionError};
    for source in [
        "\tcode\n\nprose",
        "```\ncode\n```\n\nprose",
        "- b\n- a\n\nprose",
        "body\\\nnext\n\nprose",
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = document.source_bytes();
        let count = document.projection().hard_line_count();
        let revision = document.revision();
        let error = document
            .prepare_model_request(ModelRequest::ReorderHardLines {
                document: document.id(),
                revision,
                source_lines: 0..count,
                order: (0..count).rev().collect(),
            })
            .unwrap_err();
        assert!(
            matches!(
                error,
                ModelTransactionError::Document(DocumentError::UnsupportedFormatting)
            ),
            "{source:?}: {error:?}; blocks={:?}; text={:?}",
            document.projection().blocks(),
            document.text()
        );
        assert_eq!(document.source_bytes(), before);
        assert_eq!(document.revision(), revision);
    }
}

#[test]
fn backspace_new_empty_markdown_item_becomes_an_ordinary_empty_paragraph() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    for source in [
        "- first\n- second",
        "5. first\n6. second",
        "- first\n- **second**",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let original = document.source_bytes();
        let before = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        for key in [Key::Char('A'), Key::Enter, Key::Backspace, Key::Escape] {
            commands
                .handle(&mut document, InputEvent::Key(key))
                .unwrap();
        }
        assert_eq!(document.text(), before.replacen('\n', "\n\n", 1));
        let blocks = document.projection().blocks();
        assert_eq!(blocks[1].style.0, "Paragraph");
        assert!(blocks[1].range.is_empty());
        let after = document.source_bytes();
        let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        assert_eq!(reopened.projection().blocks()[1].style.0, "Paragraph");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), after);
    }
}

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text
            .chars()
            .map(|ch| u8::try_from(ch as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn source_multi_return_replacement_preserves_each_requested_visible_row() {
    for (source, range, replacement) in [
        (" first\n2. second\n3. ", 12..15, "\n\n"),
        ("1. *f*irst\n2. second\n3. ", 9..12, "\n\n"),
        ("a\n\nb", 1..1, "\n\n"),
        ("a\n\nb", 2..2, "\n"),
        ("a\n\n\nb", 1..1, "\n"),
        ("abc", 1..2, "\n\n\n"),
        ("```\na\nb\n```", 5..5, "\n\n"),
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for (endings, delimiter) in [
                (FileFormat::Unix, "\n"),
                (FileFormat::Dos, "\r\n"),
                (FileFormat::Mac, "\r"),
            ] {
                let original = encode(&source.replace('\n', delimiter), encoding);
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::MarkdownSource,
                    endings,
                )
                .unwrap();
                let mut expected = document.text().to_owned();
                expected.replace_range(range.clone(), replacement);
                document
                    .replace(range.clone(), replacement)
                    .unwrap_or_else(|error| {
                        panic!("{source:?} {range:?} {encoding:?} {endings:?}: {error:?}")
                    });
                let fresh = Document::from_bytes_with_file_format(
                    document.source_bytes(),
                    encoding,
                    Format::MarkdownSource,
                    endings,
                )
                .unwrap();
                assert_eq!(document.text(), expected);
                assert_eq!(document.text(), fresh.text());
                assert_eq!(
                    document.projection().provenance(),
                    fresh.projection().provenance()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }
}

#[test]
fn html_join_of_paragraphs_in_one_list_item_retains_list_and_inline_scopes() {
    for source in [
        "<ol start='3'><li>One<p>Second</p></li><li><i>Two</i></li></ol><!--keep-->",
        "<ul><li><p>One</p><p>Second</p></li><li>Two</li></ul>",
        "<ol><li>One<p><b>Second</b></p></li><li>Two</li></ol>",
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for replacement in ["", "אב", "<"] {
                let original = encode(source, encoding);
                let mut document =
                    Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
                assert_eq!(document.text(), "One\nSecond\nTwo");
                document.replace(3..5, replacement).unwrap();
                assert_eq!(document.text(), format!("One{replacement}econd\nTwo"));
                let fresh =
                    Document::from_bytes(document.source_bytes(), encoding, Format::Html).unwrap();
                assert_eq!(
                    document.projection().provenance(),
                    fresh.projection().provenance()
                );
                assert_eq!(document.projection().blocks().len(), 2);
                assert!(document.projection().blocks().iter().all(|block| matches!(
                    block.kind,
                    viem_core::document::BlockKind::ListItem { .. }
                )));
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }
}

#[test]
fn source_tail_paragraph_body_edit_keeps_terminal_source_ending() {
    for source in ["a\n\nTail.\n", "a\n\nTail.\n\n", "```\na\n```\n\nTail."] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for (endings, delimiter) in [
                (FileFormat::Unix, "\n"),
                (FileFormat::Dos, "\r\n"),
                (FileFormat::Mac, "\r"),
            ] {
                let original = encode(&source.replace('\n', delimiter), encoding);
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::MarkdownSource,
                    endings,
                )
                .unwrap();
                let at = document.text().find("Tail").unwrap();
                let mut expected = document.text().to_owned();
                expected.replace_range(at..at + 3, "x");
                document.replace(at..at + 3, "x").unwrap_or_else(|error| {
                    panic!("{source:?} {encoding:?} {endings:?}: {error:?}")
                });
                let fresh = Document::from_bytes_with_file_format(
                    document.source_bytes(),
                    encoding,
                    Format::MarkdownSource,
                    endings,
                )
                .unwrap();
                assert_eq!(document.text(), expected);
                assert_eq!(
                    document.projection().provenance(),
                    fresh.projection().provenance()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
            }
        }
    }
}

#[test]
fn source_body_edit_after_large_code_block_keeps_projection_work_regional() {
    use viem_core::document::{ModelRequest, ProjectionWorkScope, TextEdit};
    let source = format!("```\n{}```\n\nTail.\n", "code line\n".repeat(10_000));
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let at = document.text().find("Tail").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 3, "x")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(
        work.scope(),
        ProjectionWorkScope::RegionalHardLines,
        "{work:?}"
    );
    assert!(work.decoded_source_bytes() < 128, "{work:?}");
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(prepared).unwrap();
    let fresh = Document::from_bytes(
        document.source_bytes(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn markdown_source_opening_fence_prefix_can_be_replaced() {
    use viem_core::document::{ModelRequest, TextEdit};

    for source in [
        "```",
        "```\n",
        "```\na",
        "```\na\n```",
        "```\na\n```\nTail.",
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let mut document = Document::from_bytes_with_file_format(
                encode(source, encoding),
                encoding,
                Format::MarkdownSource,
                FileFormat::Unix,
            )
            .unwrap();
            assert_eq!(document.text(), source);
            let mut expected = source.to_owned();
            expected.replace_range(0..2, "\\");
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(0..2, "\\")],
                })
                .unwrap_or_else(|error| panic!("{source:?} {encoding:?}: {error:?}"));
            assert_fence_prefix_edit(&mut document, prepared, &expected, &expected);
        }
    }
}

#[test]
fn markdown_source_fence_edit_reparses_immediately_through_either_path() {
    use viem_core::document::{ModelRequest, TextEdit};

    // Markdown Source keeps delimiters editable, and editing one reparses and
    // updates formatting immediately. Removing the old opener makes the old
    // closer open an unclosed code block: its following paragraph separator
    // becomes two literal code breaks. In the second case, old code blank rows
    // also become folded prose separators. Editing the delimiter through the
    // formatted text and through explicit physical source must commit the same
    // minimal patch and reach the same authoritative reprojection.
    for (source, before_text, after_text) in [
        (
            "```\na\n```\n\nTail.",
            "```\na\n```\nTail.",
            "\\`\na\n```\n\nTail.",
        ),
        (
            "```\nline one\n\nli\r two\n```\n\nTail.",
            "```\nline one\n\nli\r two\n```\nTail.",
            "\\`\nline one\nli\r two\n```\n\nTail.",
        ),
    ] {
        let mut expected_source = source.to_owned();
        expected_source.replace_range(0..2, "\\");
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for formatted in [true, false] {
                let mut document = Document::from_bytes_with_file_format(
                    encode(source, encoding),
                    encoding,
                    Format::MarkdownSource,
                    FileFormat::Unix,
                )
                .unwrap();
                assert_eq!(document.text(), before_text);
                let revision = document.revision();
                let request = if formatted {
                    ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision,
                        edits: vec![TextEdit::new(0..2, "\\")],
                    }
                } else {
                    ModelRequest::ReplacePhysicalSource {
                        document: document.id(),
                        revision,
                        range: 0..encode("``", encoding).len(),
                        replacement: "\\".into(),
                    }
                };
                let prepared = document.prepare_model_request(request).unwrap_or_else(|error| {
                    panic!("{source:?} {encoding:?} formatted={formatted}: {error:?}")
                });
                assert_fence_prefix_edit(&mut document, prepared, &expected_source, after_text);
            }
        }
    }
}

fn assert_fence_prefix_edit(
    document: &mut Document,
    prepared: viem_core::document::PreparedModelTransaction,
    expected_source: &str,
    expected_text: &str,
) {
    let original = document.source_bytes();
    let before = document.projection().clone();
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].range(), 0..encode("``", document.encoding()).len());
    assert_eq!(patches[0].replacement(), encode("\\", document.encoding()));
    document.commit_model_transaction(prepared).unwrap();
    let after = document.projection().clone();
    let edited = encode(expected_source, document.encoding());
    assert_eq!(document.source_bytes(), edited);
    assert_eq!(document.text(), expected_text);
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        Format::MarkdownSource,
        FileFormat::Unix,
    )
    .unwrap();
    assert_eq!(fresh.text(), expected_text);
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.projection(), &before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), edited);
    assert_eq!(document.projection(), &after);
}
