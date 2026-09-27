use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BlockKind, BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn input(core: &mut Editor, view: ViewId, event: InputEvent) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(event.clone()))
        .unwrap();
    assert!(
        matches!(
            result.command.as_ref().unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ) || event == InputEvent::Key(Key::Escape)
            && matches!(result.command.as_ref().unwrap().status, CommandStatus::Cancelled),
        "{event:?}: {result:?}; source={:?}",
        String::from_utf8_lossy(&core.document().source_bytes())
    );
}

fn backspace(source: &str, format: Format, at: usize) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 500.0);
    input(&mut core, view, InputEvent::key('i'));
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
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    (core, view)
}

fn verify_history(core: &mut Editor, view: ViewId, source: &str) {
    let bytes = core.document().source_bytes();
    let reopened =
        Document::from_bytes(bytes.clone(), Encoding::Utf8, core.document().format()).unwrap();
    assert_eq!(reopened.text(), core.document().text());
    let kinds = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| (block.range.clone(), block.kind.clone(), block.style.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(&reopened), kinds(core.document()));
    input(core, view, InputEvent::Key(Key::Escape));
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), bytes);
}

#[test]
fn continuation_backspace_joins_same_item_without_removing_label() {
    for (format, source) in [
        (Format::Markdown, "- a\n\n  b\n- c"),
        (Format::Markdown, "9) a\n\n   b\n1) c"),
        (Format::Markdown, "> - a\n> \n>   b\n> - c"),
    ] {
        let (mut core, view) = backspace(source, format, 2);
        assert_eq!(core.document().text(), "ab\nc", "{source}");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(
            matches!(
                core.document().projection().blocks()[0].kind,
                BlockKind::ListItem {
                    item_start: true,
                    ..
                }
            ),
            "{source}"
        );
        verify_history(&mut core, view, source);
    }
}

#[test]
fn first_body_backspace_removes_label_and_hidden_continuation_indent() {
    for source in [
        "- a\n\n  b\n- c",
        "9) a\n\n   b\n1) c",
        "> - a\n> \n>   b\n> - c",
    ] {
        let (mut core, view) = backspace(source, Format::Markdown, 0);
        assert_eq!(core.document().text(), "a\nb\nc", "{source}");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        let blocks = core.document().projection().blocks();
        assert!(
            !matches!(blocks[0].kind, BlockKind::ListItem { .. }),
            "{source}"
        );
        assert!(
            !matches!(blocks[1].kind, BlockKind::ListItem { .. }),
            "{source}"
        );
        assert!(
            matches!(
                blocks[2].kind,
                BlockKind::ListItem {
                    item_start: true,
                    ..
                }
            ),
            "{source}"
        );
        verify_history(&mut core, view, source);
    }
}

#[test]
fn first_body_reset_preserves_flowed_text_and_child_labels() {
    for (source, expected, child) in [
        ("- a\n  lazy\n\n  b\n- c", "a lazy\nb\nc", false),
        ("- a\n  - child\n\n  b\n- c", "a\nchild\nb\nc", true),
        ("- a\n\n  b\n\n  d\n- c", "a\nb\nd\nc", false),
        ("- a\n\n\tb\n- c", "a\nb\nc", false),
        ("- \n\n  b\n- c", "\nb\nc", false),
    ] {
        let (mut core, view) = backspace(source, Format::Markdown, 0);
        assert_eq!(core.document().text(), expected, "{source}");
        let blocks = core.document().projection().blocks();
        assert!(
            !matches!(blocks[0].kind, BlockKind::ListItem { .. }),
            "{source}"
        );
        if child {
            assert!(
                matches!(
                    blocks[1].kind,
                    BlockKind::ListItem {
                        item_start: true,
                        ..
                    }
                ),
                "{source}"
            );
        }
        assert!(
            matches!(
                blocks.last().unwrap().kind,
                BlockKind::ListItem {
                    item_start: true,
                    ..
                }
            ),
            "{source}"
        );
        assert!(String::from_utf8(core.document().source_bytes())
            .unwrap()
            .ends_with("- c"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn removing_selected_list_style_handles_continuation_and_single_paragraph_items_together() {
    for source in ["- a\n\n  b\n- c", "- a\n  - child\n\n  b\n- c"] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 500.0);
        let text = core.document().text().to_owned();
        input(&mut core, view, InputEvent::Key(Key::SelectAll));
        core.handle(
            view,
            CoreEvent::SetListStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style: None,
            },
        )
        .unwrap();
        assert_eq!(core.document().text(), text);
        assert!(core
            .document()
            .projection()
            .list_structure()
            .lists
            .is_empty());
        verify_history(&mut core, view, source);
    }
}
