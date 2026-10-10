use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BlockKind, BoundaryAffinity, Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn open(source: &[u8], format: Format) -> Document {
    Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap()
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| {
            panic!(
                "{key:?}: {error:?}, source={:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
    if let Some(command) = outcome.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{key:?}: {:?}, source={:?}",
            command.status,
            String::from_utf8_lossy(&core.document().source_bytes())
        );
    }
}

#[test]
fn numbered_child_above_one_requires_a_paragraph_separator() {
    for (source, expected, levels) in [
        (
            "- parent\n  4. child\n- tail",
            "parent 4. child\ntail",
            vec![0, 0],
        ),
        (
            "- parent\n\n  4. child\n- tail",
            "parent\nchild\ntail",
            vec![0, 1, 0],
        ),
        (
            "- parent\n  1. child\n  4. sibling\n- tail",
            "parent\nchild\nsibling\ntail",
            vec![0, 1, 1, 0],
        ),
        (
            "- parent\n  continued\n  4) child\n- tail",
            "parent continued 4) child\ntail",
            vec![0, 0],
        ),
        (
            "> - parent\n>   4. child\n> - tail",
            "parent 4. child\ntail",
            vec![0, 0],
        ),
        (
            "- # parent\n  4. child\n- tail",
            "parent\nchild\ntail",
            vec![0, 1, 0],
        ),
    ] {
        let document = open(source.as_bytes(), Format::Markdown);
        assert_eq!(document.text(), expected, "{source:?}");
        let actual = document
            .projection()
            .blocks()
            .iter()
            .filter_map(|block| match block.kind {
                BlockKind::ListItem { level, .. } => Some(level),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, levels, "{source:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn literal_numbered_continuation_edits_reopen_and_restore_exact_bytes() {
    for source in [
        "- parent\n  4. child\n- tail",
        "- parent\n  4) child\n- tail",
        "> - parent\n>   4. child\n> - tail",
    ] {
        let original = open(source.as_bytes(), Format::Markdown);
        for at in 0..original.text().len() {
            for delete in [false, true] {
                let mut document = open(source.as_bytes(), Format::Markdown);
                let end = at + usize::from(delete);
                let mut expected = original.text().to_owned();
                expected.replace_range(at..end, if delete { "" } else { "X" });
                document
                    .replace(at..end, if delete { "" } else { "X" })
                    .unwrap_or_else(|error| panic!("{source:?}, {at}..{end}: {error:?}"));
                assert_eq!(document.text(), expected);
                let edited = document.source_bytes();
                assert_eq!(open(&edited, Format::Markdown).text(), expected);
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), edited);
            }
        }
    }
}

#[test]
fn literal_numbered_continuation_native_and_vim_commands_preserve_semantics() {
    for source in [
        "- parent\n  4. child\n- tail",
        "- before\n- parent\n  4. child\n- tail",
    ] {
        for action in [
            vec![Key::Char('i'), Key::Enter, Key::Escape],
            vec![Key::Char('i'), Key::Delete, Key::Escape],
            vec![Key::Char('i'), Key::Backspace, Key::Escape],
            vec![Key::Char('o'), Key::Escape],
            vec![Key::Char('i'), Key::Tab, Key::Escape],
        ] {
            let mut core = Core::new(open(source.as_bytes(), Format::Markdown));
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
            let at = core.document().text().find("4. child").unwrap();
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
            for input in &action {
                key(&mut core, view, *input);
            }
            let edited = core.document().source_bytes();
            assert_eq!(
                open(&edited, Format::Markdown).text(),
                core.document().text(),
                "{action:?}"
            );
            if edited != source.as_bytes() {
                key(&mut core, view, Key::Char('u'));
                assert_eq!(
                    core.document().source_bytes(),
                    source.as_bytes(),
                    "{action:?}"
                );
                key(&mut core, view, Key::Ctrl('r'));
                assert_eq!(core.document().source_bytes(), edited, "{action:?}");
            }
        }
    }
}

#[test]
fn unindent_repairs_retained_numbered_children_with_original_encoding_and_quotes() {
    for (prefix, ending, encoding) in [
        ("", "\n", Encoding::Utf8),
        ("> ", "\r\n", Encoding::Utf8),
        ("> ", "\r\n", Encoding::Utf16Le),
    ] {
        let source = ["- parent", "", "  9. first", "  10. second"]
            .map(|line| format!("{prefix}{line}"))
            .join(ending);
        let bytes = match encoding {
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            _ => source.as_bytes().to_vec(),
        };
        let file_format = if ending == "\r\n" {
            FileFormat::Dos
        } else {
            FileFormat::Unix
        };
        let mut document = Document::from_bytes_with_file_format(
            bytes.clone(),
            encoding,
            Format::Markdown,
            file_format,
        )
        .unwrap();
        let at = document.text().find("first").unwrap();
        let transaction = document.prepare_list_indent(at..at, true).unwrap();
        document.commit_model_transaction(transaction).unwrap();
        let edited = document.source_bytes();
        let reopened = Document::from_bytes_with_file_format(
            edited.clone(),
            encoding,
            Format::Markdown,
            file_format,
        )
        .unwrap();
        assert_eq!(reopened.text(), document.text());
        let levels = reopened
            .projection()
            .blocks()
            .iter()
            .filter_map(|block| match block.kind {
                BlockKind::ListItem { level, .. } => Some(level),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(levels, [0, 0, 1]);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), edited);
    }
}

#[test]
fn grammar_owned_list_markers_preserve_literal_continuations_and_nested_items() {
    let mut failures = Vec::new();
    for (source, expected, levels) in [
        ("foo\n*\n\nfoo\n1.\n", "foo *\nfoo 1.", vec![]),
        ("*foo bar\n*\n", "*foo bar *", vec![]),
        (";\n*\n%\n", "; * %", vec![]),
        (";\n* \n%\n", "; * %", vec![]),
        ("- a\n - b\n  - c\n   - d\n    - e\n", "a\nb\nc\nd - e", vec![0, 0, 0, 0]),
        ("1. a\n\n  2. b\n\n    3. c\n", "a\nb\n3. c", vec![0, 0]),
        ("1.\n  Text after.\n", "\nText after.", vec![0]),
        ("-\n  foo\n", "foo", vec![0]),
        ("- ```\n  code\n- next\n", "code\nnext", vec![0, 0]),
        ("- ```\na\n```\ntail", "\na\ntail", vec![0]),
        ("<div>\n```\n</div>\n\n- a\n- b\n", "```\na\nb", vec![0, 0]),
        ("Foo\n-\nbar\n", "Foo\nbar", vec![]),
        ("- - foo\n", "foo", vec![1]),
        ("1. - 2. foo\n", "foo", vec![2]),
        ("- *foo\n  - - \n  baz*\n", "*foo\n\nbaz*", vec![0, 2, 0]),
        (" - >*\n", "", vec![1]),
        ("- a\n  > - b\n  >   c\n", "a\nb c", vec![0, 1]),
    ] {
        let document = open(source.as_bytes(), Format::Markdown);
        if document.text() != expected { failures.push(format!("{source:?}: text {:?}, expected {expected:?}", document.text())); }
        let actual = document.projection().blocks().iter().filter_map(|block| {
            if let BlockKind::ListItem { level, .. } = block.kind { Some(level) } else { None }
        }).collect::<Vec<_>>();
        if actual != levels { failures.push(format!("{source:?}: levels {actual:?}, expected {levels:?}")); }
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn grammar_owned_quote_context_keeps_lazy_prose_and_retires_at_real_blocks() {
    for (source, expected, depths) in [
        ("> foo\nbar\n===\n", "foo bar ===", vec![1]),
        ("> a\n#\tfoo", "a\nfoo", vec![1, 0]),
        ("> a\n2. b", "a\nb", vec![1, 0]),
        ("- ```\n  code\n  ```\n\n> quote\n", "code\nquote", vec![0, 1]),
    ] {
        let document = open(source.as_bytes(), Format::Markdown);
        assert_eq!(document.text(), expected, "{source:?}");
        assert_eq!(document.projection().blocks().iter().map(|block| block.quote_depth).collect::<Vec<_>>(), depths, "{source:?}");
        let at = document.text().len() - 1;
        let mut edited = open(source.as_bytes(), Format::Markdown);
        edited.replace(at..at, "X").unwrap();
        let saved = edited.source_bytes();
        let fresh = open(&saved, Format::Markdown);
        assert_eq!(edited.text(), fresh.text(), "{source:?}");
        assert_eq!(edited.projection().blocks().iter().map(|block| (&block.kind, block.quote_depth)).collect::<Vec<_>>(), fresh.projection().blocks().iter().map(|block| (&block.kind, block.quote_depth)).collect::<Vec<_>>());
        assert!(edited.undo());
        assert_eq!(edited.source_bytes(), source.as_bytes());
        assert!(edited.redo());
        assert_eq!(edited.source_bytes(), saved);
    }
}
