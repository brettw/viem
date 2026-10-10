use viem_core::document::{
    BlockKind, Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};

fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn assert_structural_key_edit(
    source: &str,
    at: usize,
    key: viem_core::command::Key,
    expected: &str,
) {
    use viem_core::command::{CommandStatus, InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};

    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let outcome = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
    assert_eq!(
        outcome.command.unwrap().status,
        CommandStatus::Complete,
        "{source:?}"
    );
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(core.document().text(), expected, "{source:?}");
    let saved = core.document().source_bytes();
    let fresh = open(std::str::from_utf8(&saved).unwrap());
    assert_eq!(
        core.document().text(),
        fresh.text(),
        "saved {:?}",
        String::from_utf8_lossy(&saved)
    );
    assert_eq!(
        core.document().projection().style_spans(),
        fresh.projection().style_spans()
    );
    assert_eq!(
        core.document()
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.attributes))
            .collect::<Vec<_>>(),
        fresh
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.attributes))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        core.document()
            .hard_line_snapshot()
            .capture(0..expected.len())
            .unwrap()
            .break_offsets(),
        fresh
            .hard_line_snapshot()
            .capture(0..expected.len())
            .unwrap()
            .break_offsets()
    );
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), saved);
    assert_eq!(core.document().text(), expected);
}

#[test]
fn typing_in_bare_empty_list_item_retains_its_marker() {
    for source in [
        "1.\n  Text after.\n",
        "-\n  foo\n",
        "-\nparagraph",
        "text\n1.\nnext",
        "*\n      <div>\n     <div>\n",
        "* \n      <div>\n     <div>\n",
    ] {
        let doc = open(source);
        if source.starts_with('*') {
            assert_eq!(doc.text(), "\n<div>\n");
            assert_eq!(doc.projection().blocks()[1].style.0, "Code Block");
        }
        let at = doc
            .projection()
            .blocks()
            .iter()
            .find(|block| matches!(block.kind, BlockKind::ListItem { .. }))
            .unwrap()
            .range
            .start;
        let mut expected = doc.text().to_owned();
        expected.insert(at, 'x');
        assert_structural_key_edit(source, at, viem_core::command::Key::Char('x'), &expected);
    }
}

#[test]
fn typing_in_empty_eof_heading_stays_inside_heading() {
    for source in [
        "## \n#\n### ###\n",
        "### ###\n",
        "### \n",
        "### \n\nNext\n",
        "#\n",
        "### ###",
        "### ",
    ] {
        let doc = open(source);
        let at = doc
            .projection()
            .blocks()
            .iter()
            .rev()
            .find(|block| matches!(block.kind, BlockKind::Heading(_)) && block.range.is_empty())
            .unwrap()
            .range
            .start;
        let mut expected = doc.text().to_owned();
        expected.insert(at, 'x');
        assert_structural_key_edit(source, at, viem_core::command::Key::Char('x'), &expected);
    }
}

#[test]
fn bare_list_marker_typing_through_document_edits_matches_reopened_source() {
    for (source, at, expected) in [
        ("-\nparagraph", 0, "X paragraph"),
        ("text\n1.\nnext", 5, "text\nX next"),
    ] {
        let mut doc = open(source);
        doc.replace(at..at, "X").unwrap();
        assert_eq!(doc.text(), expected);
        let saved = doc.source_bytes();
        let fresh = open(std::str::from_utf8(&saved).unwrap());
        assert_eq!(fresh.text(), expected);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), saved);
        assert_eq!(doc.text(), expected);
    }
}

#[test]
fn list_prose_remains_visible_before_indented_block_starts() {
    for (source, expected) in [
        ("- item one\n  <div>block</div>\n", "item one\nblock"),
        ("- item one\n  ***\n- two\n", "item one\n\ntwo"),
        ("1. Step one\n   ## Sub heading\n", "Step one\nSub heading"),
        ("- _t\n  # test\n  t_\n", "_t\ntest\nt_"),
        (
            "* A Heading:\n  # inside a list item\n",
            "A Heading:\ninside a list item",
        ),
    ] {
        let doc = open(source);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn parser_setext_headings_keep_ownership_of_list_like_underlines() {
    for (source, rendered, source_text) in [
        ("bbb\n- ", "bbb", "bbb\n- "),
        ("\nbbb\n- ", "\nbbb", "\nbbb\n- "),
        ("aaa\n\nbbb\n- ", "aaa\nbbb", "aaa\nbbb\n- "),
        ("> aaa\n\nbbb\n- ", "aaa\nbbb", "> aaa\nbbb\n- "),
        (
            "> ```\n> aaa\n\nbbb\n- ",
            "aaa\n\nbbb",
            "> ```\n> aaa\n\nbbb\n- ",
        ),
        (
            "```\naaa\n```\n\nbbb\n- ",
            "aaa\nbbb",
            "```\naaa\n```\nbbb\n- ",
        ),
    ] {
        for (format, expected) in [
            (Format::Markdown, rendered),
            (Format::MarkdownSource, source_text),
        ] {
            let doc =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            assert_eq!(doc.text(), expected, "{format:?} {source:?}");
            let heading = doc.projection().blocks().last().unwrap();
            assert_eq!(heading.kind, BlockKind::Heading(2), "{format:?} {source:?}");
            assert_eq!(heading.style.0, "Heading2");
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn empty_nested_dash_items_keep_list_ownership_after_keys_and_replacements() {
    use viem_core::command::Key;
    use viem_core::document::FileFormat;
    let source = "- a\n  - b\n    - c\n      deep\n- tail";
    for (at, key, expected) in [
        (2, Key::Enter, "a\n\nb\nc deep\ntail"),
        (4, Key::Enter, "a\nb\n\nc deep\ntail"),
        (2, Key::Delete, "a\n\nc deep\ntail"),
        (3, Key::Backspace, "a\n\nc deep\ntail"),
    ] {
        assert_structural_key_edit(source, at, key, expected);
    }
    for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (file_format, ending) in [
            (FileFormat::Unix, "\n"), (FileFormat::Dos, "\r\n"), (FileFormat::Mac, "\r"),
        ] {
            let source = "- a\n  - b\n  - c\n- d".replace('\n', ending);
            let bytes = match encoding {
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => source.as_bytes().to_vec(),
            };
            for (range, replacement, expected) in [
                (2..3, "", "a\n\nc\nd"),
                (1..3, "\n", "a\n\nc\nd"),
                (2..7, "", "a\n"),
            ] {
                let mut document = Document::from_bytes_with_file_format(
                    bytes.clone(), encoding, Format::Markdown, file_format,
                ).unwrap();
                document.replace(range, replacement).unwrap();
                assert_eq!(document.text(), expected);
                let saved = document.source_bytes();
                let fresh = Document::from_bytes_with_file_format(
                    saved.clone(), encoding, Format::Markdown, file_format,
                ).unwrap();
                assert_eq!(fresh.text(), expected);
                assert_eq!(
                    document.projection().blocks().iter().map(|block| (&block.range, &block.attributes)).collect::<Vec<_>>(),
                    fresh.projection().blocks().iter().map(|block| (&block.range, &block.attributes)).collect::<Vec<_>>(),
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), bytes);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), saved);
            }
        }
    }
    let source = "- a\n\n  - b\n- c";
    let mut document = open(source);
    document.replace(2..3, "").unwrap();
    assert_eq!(document.text(), "a\n\nc");
    assert_eq!(document.source_bytes(), b"- a\n\n  - \n- c");
    assert_eq!(open("- a\n\n  - \n- c").text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), b"- a\n\n  - \n- c");
}

#[test]
fn enter_at_heading_start_keeps_closing_sequence_and_setext_body() {
    for source in ["## foo ##\n", "Foo *bar*\n=========\n"] {
        let expected = format!("\n{}", open(source).text());
        assert_structural_key_edit(source, 0, viem_core::command::Key::Enter, &expected);
    }
}

#[test]
fn fenced_code_is_one_paragraph_with_literal_internal_breaks() {
    for source in [
        "```rust\n  first\n\nsecond\n```",
        "~~~\n  first\n\nsecond\n~~~\n",
        "```\n  first\n\nsecond",
    ] {
        let mut document = open(source);
        assert_eq!(document.text(), "  first\n\nsecond");
        assert_eq!(document.projection().blocks().len(), 1);
        assert_eq!(document.projection().blocks()[0].style.0, "Code Block");
        assert_eq!(document.projection().hard_line_count(), 3);
        let id = document.projection().blocks()[0].id;
        let at = document.text().find("second").unwrap();
        document.replace(at..at + 1, "S").unwrap();
        assert_eq!(document.projection().blocks()[0].id, id);
        assert_eq!(document.projection().blocks().len(), 1);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn list_continuations_flow_and_ordered_markers_use_list_start() {
    for (source, expected) in [
        (
            "- first\n  continuation\n- second",
            "first continuation\nsecond",
        ),
        (
            "3. first\n   continuation\n1. second",
            "first continuation\nsecond",
        ),
        (
            "10) first\nlazy continuation\n2) second",
            "first lazy continuation\nsecond",
        ),
        ("- first  \n  explicit\n- second", "first\nexplicit\nsecond"),
    ] {
        let document = open(source);
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(document.projection().blocks().len(), 2, "{source}");
        assert_eq!(document.projection().list_structure().lists.len(), 1);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn nested_lists_and_second_paragraphs_keep_item_membership() {
    let source = "- first\n  continuation\n\n  second paragraph\n\n  - child\n    more child\n\n  back to first\n- next";
    let document = open(source);
    assert_eq!(
        document.text(),
        "first continuation\nsecond paragraph\nchild more child\nback to first\nnext"
    );
    let blocks = document.projection().blocks();
    assert_eq!(blocks.len(), 5);
    assert!(matches!(
        blocks[1].kind,
        BlockKind::ListItem {
            item_start: false,
            level: 0,
            ..
        }
    ));
    assert!(matches!(
        blocks[2].kind,
        BlockKind::ListItem {
            item_start: true,
            level: 1,
            ..
        }
    ));
    assert!(matches!(
        blocks[3].kind,
        BlockKind::ListItem {
            item_start: false,
            level: 0,
            ..
        }
    ));
    let lists = document.projection().list_structure();
    assert_eq!(lists.lists.len(), 2);
    assert_eq!(lists.lists[0].items[0].paragraph_ids.len(), 3);
}

#[test]
fn bullet_spellings_project_canonically_and_edits_keep_original_markers() {
    for marker in ["-", "+", "*"] {
        let source = format!("{marker} first\n{marker} second");
        let mut document = open(&source);
        assert_eq!(document.text(), "first\nsecond");
        let at = document.text().find("second").unwrap();
        document.replace(at..at + 1, "S").unwrap();
        assert_eq!(
            document.source_bytes(),
            format!("{marker} first\n{marker} Second").as_bytes()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        let raw = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(raw.text(), source);
    }
}

#[test]
fn fenced_code_inside_a_list_keeps_item_membership_and_literal_indentation() {
    let source = "- item\n\n  ```rust\n    code\n  second\n  ```\n\n  after\n- next";
    let document = open(source);
    assert_eq!(document.text(), "item\n  code\nsecond\nafter\nnext");
    let blocks = document.projection().blocks();
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[1].style.0, "Code Block");
    assert!(matches!(
        blocks[1].kind,
        BlockKind::ListItem {
            item_start: false,
            level: 0,
            ..
        }
    ));
    assert_eq!(
        document.projection().list_structure().lists[0].items[0]
            .paragraph_ids
            .len(),
        3
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn large_code_paragraph_edits_keep_regional_work_and_identity() {
    let source = format!(
        "```\n{}\n```\n\nTail",
        (0..20_000)
            .map(|n| format!("code {n}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut document = open(&source);
    let paragraph = document.projection().blocks()[0].id;
    let at = document.text().find("code 18000").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 4, "CODE")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert_eq!(work.projected_hard_lines(), 1);
    assert!(work.projected_formatted_bytes() < 100);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[0].id, paragraph);
    assert_eq!(document.projection().blocks().len(), 2);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn list_body_edits_preserve_normalized_ordinals_and_continuation_identity() {
    for source in [
        "3. first\n1. second",
        "- first\n\n  continuation paragraph\n- next",
    ] {
        let mut document = open(source);
        let before = document
            .projection()
            .blocks()
            .iter()
            .map(|block| block.kind.clone())
            .collect::<Vec<_>>();
        let at = document
            .text()
            .find(if source.starts_with('3') {
                "second"
            } else {
                "continuation"
            })
            .unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at + 1, "X")],
            })
            .unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::RegionalHardLines
        );
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .map(|block| block.kind.clone())
                .collect::<Vec<_>>(),
            before
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn ordered_list_enter_renumbers_siblings_and_undo_restores_literal_markers() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for source in ["3. first\n1. second", "- parent\n\n  3) first\n  1) second"] {
        let mut core = Core::new(open(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 180.0);
        let at = core.document().text().find("first").unwrap() + 5;
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: viem_core::document::BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a'))))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("added")))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert!(core.document().text().contains("first\nadded\nsecond"));
        let ordinals = core
            .document()
            .projection()
            .blocks()
            .iter()
            .filter_map(|block| {
                if let BlockKind::ListItem {
                    ordered: true,
                    ordinal,
                    marker_is_decoration: true,
                    ..
                } = block.kind
                {
                    Some(ordinal)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(ordinals, vec![3, 4, 5]);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn code_enter_at_body_end_preserves_one_paragraph_and_exact_undo() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for source in [
        "```\ncode\n```",
        "- item\n\n  ```\n  code\n  ```\n\n  after",
    ] {
        let mut core = Core::new(open(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 120.0, 180.0);
        let at = core.document().text().find("code").unwrap() + 4;
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: viem_core::document::BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('a'))))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("next")))
            .unwrap_or_else(|error| {
                panic!(
                    "{error:?}: {} {:?}",
                    String::from_utf8_lossy(&core.document().source_bytes()),
                    core.document().projection().blocks()
                )
            });
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert!(
            core.document().text().contains("code\nnext"),
            "{}",
            core.document().text()
        );
        if source.starts_with('-') {
            assert!(String::from_utf8_lossy(&core.document().source_bytes())
                .contains("  code\n  next\n  ```"));
        }
        let blocks = core.document().projection().blocks();
        assert_eq!(
            blocks
                .iter()
                .filter(|block| block.style.0 == "Code Block")
                .count(),
            1
        );
        assert!(blocks.iter().any(|block| block.style.0 == "Code Block"
            && core.document().text()[block.range.clone()] == *"code\nnext"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn assigning_paragraph_style_retains_list_container_without_source_regeneration() {
    let source = "+ First\n+ second\n\nTail";
    let mut document = open(source);
    let end = document.text().find("\nTail").unwrap();
    document
        .set_paragraph_style(0..end, viem_core::document::StyleId::from("Heading2"))
        .unwrap();
    assert_eq!(document.text(), "First\nsecond\nTail");
    assert_eq!(document.source_bytes(), b"+ ## First\n+ ## second\n\nTail");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn source_list_edits_keep_enclosing_depth_and_match_fresh_projection() {
    let source = "- Root\n  3. Child\n     - Grandchild\n       continuation\n  1. Sibling\n\nTail";
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let at = document.text().find("continuation").unwrap();
    document.replace(at..at + 1, "C").unwrap();
    let reopened = Document::from_bytes(
        document.source_bytes(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.kind, &block.style))
            .collect::<Vec<_>>(),
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.kind, &block.style))
            .collect::<Vec<_>>()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn late_source_list_body_edit_keeps_bounded_work_and_flow_membership() {
    let source = format!("- Root\n{}", "  continuation body\n".repeat(20_000));
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let at = document.projection().hard_line_range(18_000).unwrap().start + 2;
    let old_flow = document.projection().presentation_line_count(true);
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 12, "replacement")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.projected_hard_lines() <= 3);
    assert!(work.projected_formatted_bytes() < 100);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(
        document.projection().presentation_line_count(true),
        old_flow
    );
    assert!(matches!(
        document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&at))
            .unwrap()
            .kind,
        BlockKind::ListItem {
            item_start: true,
            level: 0,
            ..
        }
    ));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn decorated_labels_are_absent_from_text_and_empty_item_boundaries_keep_source() {
    let encode = |encoding, source: &str| -> Vec<u8> {
        match encoding {
            Encoding::Utf8 | Encoding::Latin1 => source.as_bytes().to_vec(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        }
    };
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for source in ["+ ", "01) ", "- First\n- ", "- First\n  + "] {
            let bytes = encode(encoding, source);
            let mut document =
                Document::from_bytes(bytes.clone(), encoding, Format::Markdown).unwrap();
            assert!(!document.text().contains(['+', ')']));
            let end = document.text().len();
            assert!(document.projection().blocks().iter().any(|block| matches!(
                block.kind,
                BlockKind::ListItem {
                    marker_is_decoration: true,
                    ..
                }
            ) && block.range.end == end));
            document.insert(end, "Added").unwrap();
            assert_eq!(
                document.source_bytes(),
                encode(encoding, &format!("{source}Added"))
            );
            assert!(document.text().ends_with("Added"));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn list_type_changes_are_source_only_and_do_not_remove_literal_body_prefixes() {
    use viem_core::document::ListStyle;
    let source = "+ \\- literal\n+ Second";
    let mut document = open(source);
    let before = document.text().to_owned();
    assert_eq!(before, "- literal\nSecond");
    document
        .set_list_style(0..before.len(), Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), before);
    assert_eq!(document.source_bytes(), b"1. \\- literal\n2. Second");
    document.set_list_style(0..before.len(), None).unwrap();
    assert_eq!(document.text(), before);
    assert_eq!(document.source_bytes(), b"\\- literal\n\nSecond");
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn late_decorated_item_edit_is_regional_and_retains_label_metadata() {
    let source = (0..10_000)
        .map(|n| format!("1. Body {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut document = open(&source);
    let at = document.text().find("Body 9000").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 4, "Changed")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert_eq!(work.projected_hard_lines(), 1);
    assert!(work.projected_formatted_bytes() < 100);
    document.commit_model_transaction(prepared).unwrap();
    assert!(matches!(
        document.projection().blocks()[9000].kind,
        BlockKind::ListItem {
            ordinal: 9001,
            marker_is_decoration: true,
            ..
        }
    ));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn decorated_markdown_list_delete_and_put_keep_body_registers_and_source_ownership() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (keys, expected, expected_source) in [
        ("dd", "Second\nThird", "3) Second\n4) Third"),
        ("2dd", "Third", "3) Third"),
        ("Gdd", "First\nSecond", "3) First\n4) Second"),
        (
            "yyp",
            "First\nFirst\nSecond\nThird",
            "03) First\n4) First\n01) Second\n01) Third",
        ),
    ] {
        let source = "03) First\n01) Second\n01) Third";
        let mut core = Core::new(open(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 180.0);
        for ch in keys.chars() {
            core.handle(view, CoreEvent::Input(InputEvent::key(ch)))
                .unwrap_or_else(|error| panic!("{keys}: {error:?}"));
        }
        assert_eq!(core.document().text(), expected, "{keys}");
        let register = core.command_state(view).unwrap().register('"').unwrap();
        assert!(!register.text.contains(')'), "{keys}: {}", register.text);
        assert_eq!(
            core.document().source_bytes(),
            expected_source.as_bytes(),
            "{keys}"
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_decorated_markdown_item_enter_exits_without_inserting_text() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for source in ["- ", "-", "1.", "- First\n- "] {
        let mut core = Core::new(open(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 180.0);
        core.handle(view, CoreEvent::Input(InputEvent::key('G')))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('A')))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        let blocks = core.document().projection().blocks();
        assert_eq!(
            blocks.last().unwrap().kind,
            BlockKind::Paragraph,
            "{source}"
        );
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn linewise_delete_owns_hidden_label_but_character_delete_preserves_empty_item() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (source, keys, expected_source, list_remains) in [
        ("+ x", "x", "+ ", true),
        ("+ x", "dd", "", false),
        ("+ ", "dd", "", false),
        ("+ First\n+ ", "Gdd", "+ First", true),
    ] {
        let mut core = Core::new(open(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 160.0);
        for ch in keys.chars() {
            core.handle(view, CoreEvent::Input(InputEvent::key(ch)))
                .unwrap_or_else(|error| panic!("{source} {keys}: {error:?}"));
        }
        assert_eq!(
            core.document().source_bytes(),
            expected_source.as_bytes(),
            "{source} {keys}"
        );
        assert_eq!(
            core.document()
                .projection()
                .blocks()
                .iter()
                .any(|block| matches!(block.kind, BlockKind::ListItem { .. })),
            list_remains
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_labels_without_source_spacing_accept_text_and_undo_exactly() {
    for source in ["-", "+", "*", "01)"] {
        let mut document = open(source);
        assert_eq!(document.text(), "");
        document.insert(0, "Body").unwrap();
        assert_eq!(document.text(), "Body");
        assert_eq!(document.source_bytes(), format!("{source} Body").as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn repeated_line_delete_and_end_change_keep_distinct_list_intentions() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "3. First\n1. Second\n1. Third";
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 160.0);
    for ch in "dd.".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(ch)))
            .unwrap();
    }
    assert_eq!(core.document().source_bytes(), b"3. Third");
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('"')
            .unwrap()
            .text,
        "Second\n"
    );
    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"3. Second\n4. Third");
    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    for ch in "c$".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(ch)))
            .unwrap();
    }
    core.handle(view, CoreEvent::Input(InputEvent::text("Changed")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(
        core.document().source_bytes(),
        b"3. Changed\n1. Second\n1. Third"
    );
    assert!(matches!(
        core.document().projection().blocks()[0].kind,
        BlockKind::ListItem {
            ordinal: 3,
            marker_is_decoration: true,
            ..
        }
    ));
    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
