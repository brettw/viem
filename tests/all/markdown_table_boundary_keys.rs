use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

#[test]
fn editing_keys_preserve_table_edges_without_backend_errors() {
    for source in [
        "before\n\n| h | j |\n| - | - |\n| b | c |\n\nafter",
        "before\n\n> | h | j |\n> | - | - |\n> | b | c |\n\nafter",
        "before\n\n| h | j |\n| - | - |\n| b<br>c | d |\n\nafter",
    ] {
        for with_layout in [false, true] {
            for (backward, key) in [
                (false, Key::Delete),
                (true, Key::Backspace),
                (true, Key::Ctrl('w')),
                (true, Key::Ctrl('u')),
            ] {
                let document = Document::from_bytes(
                    source.as_bytes().to_vec(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                let at = if backward {
                    document.text().find("after").unwrap()
                } else {
                    "before".len()
                };
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
                let send = |core: &mut Core<MockTextMeasurementProvider>, event| {
                    if with_layout {
                        core.handle_with_layout(view, event)
                    } else {
                        core.handle(view, event)
                    }
                };
                send(&mut core, CoreEvent::Input(InputEvent::key('i'))).unwrap();
                let revision = core.document().revision();
                send(
                    &mut core,
                    CoreEvent::PlaceCursor {
                        document_revision: revision,
                        text_offset: at,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                let history = core.document().history_status();
                for _ in 0..2 {
                    let output = send(&mut core, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                    assert!(matches!(
                        output.command.unwrap().status,
                        CommandStatus::Complete
                    ));
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                    assert_eq!(core.document().revision(), revision);
                    assert_eq!(core.document().history_status(), history);
                    assert_eq!(core.command_state(view).unwrap().cursor(), at);
                }
            }
        }
    }
}

#[test]
fn insert_delete_motions_stop_at_cell_boundaries_but_delete_internal_breaks() {
    let source = "| h | j |\n| - | - |\n| b<br>c | d |";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let starts = document.projection().tables()[0]
        .rows
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.range.start))
        .collect::<Vec<_>>();
    for at in starts {
        for key in [Key::Backspace, Key::Ctrl('w'), Key::Ctrl('u')] {
            let mut core = Core::new(
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap(),
            );
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
                .unwrap();
            let revision = core.document().revision();
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: revision,
                    text_offset: at,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
            assert!(matches!(
                output.command.unwrap().status,
                CommandStatus::Complete
            ));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_eq!(core.document().revision(), revision);
        }
    }
    let at = document.text().find('c').unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
        .unwrap();
    let revision = core.document().revision();
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: revision,
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
        .unwrap();
    assert_eq!(
        core.document().source_bytes(),
        source.replace("<br>", "").as_bytes()
    );
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn backspace_after_a_table_still_removes_list_or_code_treatment() {
    use viem_core::document::BlockKind;
    let table = "| h |\n| - |\n| b |\n\n";
    for body in ["- item", "```\nitem\n```"] {
        let source = format!("{table}{body}");
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        let text = core.document().text().to_owned();
        let at = text.find("item").unwrap();
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(core.document().text(), text);
        assert_eq!(
            core.document().source_bytes(),
            format!("{table}item").as_bytes()
        );
        let block = core.document().projection().blocks().last().unwrap();
        assert_eq!(block.kind, BlockKind::Paragraph);
        let saved = core.document().source_bytes();
        let reopened =
            Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), text);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}
