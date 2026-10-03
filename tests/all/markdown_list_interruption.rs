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
