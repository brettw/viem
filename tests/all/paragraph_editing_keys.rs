use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{
    BoundaryAffinity, Encoding, Format, HistoryNavigationRequest, ModelRequest,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn open(source: &str, format: Format) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 320.0, 500.0);
    (core, view)
}

fn input(core: &mut Editor, view: ViewId, event: InputEvent) {
    let outcome = core
        .handle_with_layout(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| {
            panic!(
                "{event:?}: {error:?}, source={:?}, cursor={}",
                String::from_utf8_lossy(&core.document().source_bytes()),
                core.command_state(view).unwrap().cursor()
            )
        });
    assert!(
        outcome.command.as_ref().map_or(true, |command| matches!(
            command.status,
            CommandStatus::Complete | CommandStatus::Pending
        )),
        "{event:?}: {outcome:?}"
    );
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

fn insert_at(core: &mut Editor, view: ViewId, at: usize) {
    input(core, view, InputEvent::key('i'));
    let affinity = if core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .caret_point(at, BoundaryAffinity::Downstream)
        .is_ok()
    {
        BoundaryAffinity::Downstream
    } else {
        BoundaryAffinity::Upstream
    };
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity,
            extend_selection: false,
        },
    )
    .unwrap();
}

#[test]
fn enter_uses_following_style_only_at_the_end_of_a_paragraph() {
    for (format, source, current) in [
        (Format::Markdown, "# Title", "Heading1"),

    ] {
        for (at, expected_text, expected_styles) in [
            (0, "\nTitle", vec![current, current]),
            (2, "Ti\ntle", vec![current, current]),
            (5, "Title\n", vec![current, "Paragraph"]),
        ] {
            let (mut core, view) = open(source, format);
            insert_at(&mut core, view, at);
            input(&mut core, view, InputEvent::Key(Key::Enter));
            assert_eq!(core.document().text(), expected_text, "{format:?} at {at}");
            assert_eq!(
                core.document()
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| block.style.0.as_str())
                    .collect::<Vec<_>>(),
                expected_styles,
                "{format:?} at {at}"
            );
            assert_eq!(core.command_state(view).unwrap().cursor(), at + 1);
            reopen_and_history(&mut core, view, source.as_bytes());
        }
    }
}

fn reopen_and_history(core: &mut Editor, view: ViewId, original: &[u8]) {
    let final_bytes = core.document().source_bytes();
    let reopened = Document::from_bytes(
        final_bytes.clone(),
        core.document().encoding(),
        core.document().format(),
    )
    .unwrap();
    assert_eq!(reopened.text(), core.document().text());
    assert_eq!(
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.range, &b.style))
            .collect::<Vec<_>>(),
        core.document()
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.range, &b.style))
            .collect::<Vec<_>>()
    );
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), original);
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), final_bytes);
}

#[test]
fn shift_enter_keeps_rich_paragraph_style_and_text_through_reopen_and_history() {
    for (format, source) in [
        (Format::Markdown, "αβ👩‍💻xy\n\ntail"),
        (Format::Markdown, "> αβ👩‍💻xy\n\ntail"),
        (Format::Markdown, "- αβ👩‍💻xy\n\ntail"),
        (Format::Markdown, "4. αβ👩‍💻xy\n\ntail"),
        (Format::Markdown, "**αβ👩‍💻xy**\n\ntail"),
        (Format::Markdown, "`αβ👩‍💻xy`\n\ntail"),
        (Format::Markdown, "# αβ👩‍💻xy\n\ntail"),
        (Format::Markdown, "## **αβ👩‍💻xy**\n\ntail"),
        (Format::Markdown, "### `αβ👩‍💻xy`\n\ntail"),
        (Format::Markdown, "```\nαβ👩‍💻xy\n```\n\ntail"),
        (Format::Markdown, "> ```\n> αβ👩‍💻xy\n> ```\n\ntail"),

    ] {
        let (mut core, view) = open(source, format);
        let before_text = core.document().text().to_owned();
        let at = "αβ".len();
        let original_style = core.document().projection().blocks()[0].style.clone();
        let count = core.document().projection().blocks().len();
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
        assert_eq!(
            core.document().text(),
            format!("{}\n{}", &before_text[..at], &before_text[at..]),
            "{source}"
        );
        assert_eq!(
            core.document().projection().blocks().len(),
            count,
            "{source}"
        );
        assert_eq!(
            core.document().projection().blocks()[0].style,
            original_style,
            "{source}"
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), at + 1);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        let bytes = core.document().source_bytes();
        let generated = String::from_utf8(bytes).unwrap();
        if !source.contains("```") {
            assert!(
                generated.contains(match format {
                    _ if source.starts_with('#') => "<br>",
                    _ => "\\\n",
                }),
                "{generated}"
            );
        }
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn shift_enter_at_paragraph_edges_keeps_one_paragraph_and_source_modes_insert_literal_endings() {
    for (format, source) in [
        (Format::Markdown, "abc"),
        (Format::Markdown, "# abc"),

    ] {
        for at in [0, 3] {
            let (mut core, view) = open(source, format);
            insert_at(&mut core, view, at);
            input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
            assert_eq!(
                core.document().projection().blocks().len(),
                1,
                "{format:?} at {at}"
            );
            assert_eq!(
                core.document().text(),
                if at == 0 { "\nabc" } else { "abc\n" }
            );
            reopen_and_history(&mut core, view, source.as_bytes());
        }
    }
    for (format, source) in [
        (Format::Markdown, ""),
        (Format::Markdown, "# "),
        (Format::Markdown, "> "),
        (Format::Markdown, "- "),

    ] {
        let (mut core, view) = open(source, format);
        insert_at(&mut core, view, 0);
        for count in 1..=3 {
            input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
            assert_eq!(
                core.document().text(),
                "\n".repeat(count),
                "{format:?}, {source:?}"
            );
            assert_eq!(
                core.document().projection().blocks().len(),
                1,
                "{format:?}, {source:?}"
            );
        }
        input(&mut core, view, InputEvent::text("tail"));
        assert_eq!(core.document().text(), "\n\n\ntail");
        reopen_and_history(&mut core, view, source.as_bytes());
    }
    for (format, source) in [
        (Format::PlainText, "abc"),
        (Format::MarkdownSource, "**abc**"),
    ] {
        let (mut core, view) = open(source, format);
        insert_at(&mut core, view, 2);
        input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
        assert_eq!(
            core.document().source_bytes(),
            format!("{}\n{}", &source[..2], &source[2..]).as_bytes()
        );
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn empty_quote_enter_and_list_backspace_reset_while_other_block_backspace_joins() {
    for (format, source, at, enter, expected) in [
        (Format::Markdown, "previous\n\n> body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Markdown, "previous\n\n- body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Markdown, "previous\n\n1. body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Markdown, "previous\n\n> \n\nnext", 9, true, "previous\n\nnext"),
        (Format::Markdown, "> ", 0, true, ""),
    ] {
        let (mut core, view) = open(source, format);
        assert_eq!(core.document().text(), expected, "fixture {source}");
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(if enter { Key::Enter } else { Key::Backspace }));
        let joins_quote = !enter && (source.contains("cite='keep'") || source.contains("\n\n> body"));
        let cursor = if joins_quote { at - 1 } else { at };
        let after = if joins_quote { expected.replacen('\n', "", 1) } else { expected.to_owned() };
        assert_eq!(core.document().text(), after, "{source}");
        let block = core.document().projection().blocks().into_iter()
            .find(|block| block.range.start <= cursor && cursor <= block.range.end).unwrap();
        assert_eq!(block.style.0, "Paragraph", "{source}");
        assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn imported_list_quote_combinations_accept_typing_before_and_after_a_hard_break() {
    for source in [
        "- > body\n- other",
        "> 1. body\n> 2. other",
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, 2);
        input(&mut core, view, InputEvent::text("é👩‍💻"));
        input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
        input(&mut core, view, InputEvent::text("العربية"));
        assert_eq!(core.document().text(), "boé👩‍💻\nالعربيةdy\nother");
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn backspace_restores_heading_and_empty_item_hard_break_source_exactly() {
    for source in ["# ", "> # ", "# `abc`", "> ", "- ", "1. "] {
        let text =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap()
                .text()
                .to_owned();
        let offsets = if text.is_empty() {
            vec![0]
        } else {
            vec![0, text.len()]
        };
        for at in offsets {
            for affinity in [BoundaryAffinity::Downstream, BoundaryAffinity::Upstream] {
                let (mut core, view) = open(source, Format::Markdown);
                insert_at(&mut core, view, at);
                if core
                    .layout(view)
                    .unwrap()
                    .snapshot()
                    .unwrap()
                    .caret_point(at, affinity)
                    .is_err()
                {
                    continue;
                }
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: at,
                        affinity,
                        extend_selection: false,
                    },
                )
                .unwrap();
                input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
                input(&mut core, view, InputEvent::Key(Key::Backspace));
                assert_eq!(core.document().text(), text, "{source}, {at}, {affinity:?}");
                assert_eq!(
                    core.document().source_bytes(),
                    source.as_bytes(),
                    "{source}, {at}, {affinity:?}"
                );
            }
        }
    }
}

#[test]
fn shift_enter_counts_dot_and_macros_replay_semantic_breaks() {
    for (format, source) in [ (Format::Markdown, "ab")] {
        for counted in [false, true] {
            let (mut core, view) = open(source, format);
            input(&mut core, view, InputEvent::key('l'));
            if counted {
                input(&mut core, view, InputEvent::key('2'));
            }
            input(&mut core, view, InputEvent::key('i'));
            input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
            input(&mut core, view, InputEvent::text("X"));
            input(&mut core, view, InputEvent::Key(Key::Escape));
            if !counted {
                input(&mut core, view, InputEvent::key('$'));
                input(&mut core, view, InputEvent::key('.'));
            }
            assert_eq!(core.document().text(), "a\nX\nXb");
            assert_eq!(core.document().projection().blocks().len(), 1);
        }
        let (mut core, view) = open(source, format);
        for event in [
            InputEvent::key('l'),
            InputEvent::key('q'),
            InputEvent::key('a'),
            InputEvent::key('i'),
            InputEvent::Key(Key::ShiftEnter),
            InputEvent::text("X"),
            InputEvent::Key(Key::Escape),
            InputEvent::key('q'),
            InputEvent::key('$'),
            InputEvent::key('@'),
            InputEvent::key('a'),
        ] {
            input(&mut core, view, event);
        }
        assert_eq!(core.document().text(), "a\nX\nXb");
        assert_eq!(core.document().projection().blocks().len(), 1);
    }
}

#[test]
fn hard_break_preparation_rejects_invalid_boundary_without_mutation() {
    let document = Document::from_bytes(
        "👩‍💻body".as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let source = document.source_bytes();
    let revision = document.revision();
    let result = document.prepare_model_request(ModelRequest::InsertHardBreak {
        document: document.id(),
        revision,
        at: 1,
        affinity: BoundaryAffinity::Downstream,
    });
    assert!(result.is_err());
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.revision(), revision);
    assert!(!document.history_status().can_undo);
}

#[test]
fn escape_keeps_the_cursor_in_a_new_terminal_empty_paragraph() {
    for (format, source) in [
        (Format::PlainText, "body"),
        (Format::Markdown, "body"),
        (Format::Markdown, "# body"),

    ] {
        let (mut core, view) = open(source, format);
        input(&mut core, view, InputEvent::key('A'));
        input(&mut core, view, InputEvent::Key(Key::Enter));
        let at = core.document().text().len();
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
        let before = core.document().source_bytes();
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_eq!(core.document().source_bytes(), before);
        assert_eq!(core.command_state(view).unwrap().cursor(), at, "{format:?}");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        let layout = core.layout(view).unwrap().snapshot().unwrap();
        let caret = layout
            .caret_point(at, BoundaryAffinity::Downstream)
            .unwrap();
        let geometry = layout.caret_geometry(caret).unwrap();
        assert_eq!(layout.rows[geometry.row_index].text_range, at..at);
        assert!(geometry.rect.height > 0.0);
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::text("next"));
        assert_eq!(core.document().text(), "body\nnext", "{format:?}");
    }
}

#[test]
fn revisiting_a_new_empty_list_item_keeps_backspace_semantic() {
    for (format, source) in [
        (Format::Markdown, "- **body**"),
        (Format::Markdown, "1. **body**\n2. tail"),
    ] {
        let (mut core, view) = open(source, format);
        input(&mut core, view, InputEvent::key('A'));
        input(&mut core, view, InputEvent::Key(Key::Enter));
        let at = core.command_state(view).unwrap().cursor();
        input(&mut core, view, InputEvent::Key(Key::Up));
        input(&mut core, view, InputEvent::Key(Key::Down));
        assert_eq!(core.command_state(view).unwrap().cursor(), at, "{source}");
        let before = core.document().text().to_owned();
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), before, "{source}");
        let block = core
            .document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range == (at..at))
            .unwrap();
        assert_eq!(block.style.0, "Paragraph", "{source}");
    }
}

#[test]
fn tab_and_backtab_inside_a_list_item_change_structure_and_stay_in_the_insert_undo_unit() {
    use viem_core::document::BlockKind;
    for (format, source) in [
        (Format::Markdown, "- parent\n- **body**\n- tail"),
        (Format::Markdown, "4. parent\n5. body\n6. tail"),

    ] {
        let (mut core, view) = open(source, format);
        let at = core.document().text().find("body").unwrap() + 2;
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Tab));
        let block = &core.document().projection().blocks()[1];
        assert!(
            matches!(block.kind, BlockKind::ListItem { level: 1, .. }),
            "{format:?}: {block:?}"
        );
        assert_eq!(core.document().text(), "parent\nbody\ntail");
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
        input(&mut core, view, InputEvent::Key(Key::BackTab));
        let block = &core.document().projection().blocks()[1];
        assert!(
            matches!(block.kind, BlockKind::ListItem { level: 0, .. }),
            "{format:?}: {block:?}"
        );
        input(&mut core, view, InputEvent::Key(Key::Tab));
        assert!(matches!(
            core.document().projection().blocks()[1].kind,
            BlockKind::ListItem { level: 1, .. }
        ));
        input(&mut core, view, InputEvent::text("new "));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_eq!(core.document().text(), "parent\nbonew dy\ntail");
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn unavailable_list_indentation_is_a_noop_and_tab_elsewhere_inserts_text() {
    for (format, source) in [
        (Format::Markdown, "- body"),
    ] {
        let (mut core, view) = open(source, format);
        insert_at(&mut core, view, 2);
        let revision = core.document().revision();
        input(&mut core, view, InputEvent::Key(Key::Tab));
        input(&mut core, view, InputEvent::Key(Key::BackTab));
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }

    let (mut core, view) = open("ordinary", Format::Markdown);
    insert_at(&mut core, view, 2);
    input(&mut core, view, InputEvent::Key(Key::Tab));
    assert_ne!(core.document().text(), "ordinary");
}

#[test]
fn tab_in_a_continuation_paragraph_moves_the_complete_item() {
    use viem_core::document::BlockKind;
    let source =
        "- parent\n- body\n\n  continuation text\n- tail";
    let (mut core, view) = open(source, Format::Markdown);
    let at = core.document().text().find("continuation").unwrap() + 5;
    insert_at(&mut core, view, at);
    input(&mut core, view, InputEvent::Key(Key::Tab));
    assert_eq!(
        core.document().text(),
        "parent\nbody\ncontinuation text\ntail"
    );
    assert!(matches!(
        core.document().projection().blocks()[1].kind,
        BlockKind::ListItem { level: 1, .. }
    ));
    assert!(matches!(
        core.document().projection().blocks()[2].kind,
        BlockKind::ListItem {
            level: 1,
            item_start: false,
            ..
        }
    ));
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
    input(&mut core, view, InputEvent::Key(Key::BackTab));
    assert!(matches!(
        core.document().projection().blocks()[1].kind,
        BlockKind::ListItem { level: 0, .. }
    ));
    input(&mut core, view, InputEvent::Key(Key::Escape));
    reopen_and_history(&mut core, view, source.as_bytes());
}

#[test]
fn backspace_unindents_nested_items_before_removing_the_top_level_marker() {
    use viem_core::document::BlockKind;
    for (format, source) in [
        (Format::Markdown, "1. top\n   1. middle\n      1. body"),
    ] {
        let (mut core, view) = open(source, format);
        let at = core.document().text().find("body").unwrap();
        insert_at(&mut core, view, at);
        for expected_level in [1, 0] {
            input(&mut core, view, InputEvent::Key(Key::Backspace));
            assert_eq!(core.document().text(), "top\nmiddle\nbody", "{format:?}");
            assert_eq!(core.command_state(view).unwrap().cursor(), at, "{format:?}");
            let block = core
                .document()
                .projection()
                .blocks()
                .iter()
                .find(|block| block.range.start == at)
                .unwrap();
            assert!(
                matches!(block.kind, BlockKind::ListItem { level, .. } if level == expected_level),
                "{format:?}: {block:?}"
            );
        }
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), "top\nmiddle\nbody", "{format:?}");
        let block = core
            .document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.start == at)
            .unwrap();
        assert!(
            matches!(block.kind, BlockKind::Paragraph),
            "{format:?}: {block:?}"
        );
        assert_eq!(block.style.0, "Paragraph", "{format:?}");
        input(&mut core, view, InputEvent::Key(Key::Escape));
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn list_indentation_is_replayed_as_structure_by_dot_and_macros() {
    use viem_core::document::BlockKind;
    for record in [false, true] {
        let (mut core, view) = open("- parent\n- body\n- tail", Format::Markdown);
        input(&mut core, view, InputEvent::key('j'));
        input(&mut core, view, InputEvent::key('0'));
        if record {
            input(&mut core, view, InputEvent::key('q'));
            input(&mut core, view, InputEvent::key('a'));
        }
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::Key(Key::Tab));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        if record {
            input(&mut core, view, InputEvent::key('q'));
        }
        input(&mut core, view, InputEvent::key('j'));
        input(&mut core, view, InputEvent::key('0'));
        if record {
            input(&mut core, view, InputEvent::key('@'));
            input(&mut core, view, InputEvent::key('a'));
        } else {
            input(&mut core, view, InputEvent::key('.'));
        }
        let levels = core
            .document()
            .projection()
            .blocks()
            .iter()
            .filter_map(|block| match block.kind {
                BlockKind::ListItem { level, .. } => Some(level),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(levels, [0, 1, 1], "record={record}");
        assert_eq!(core.document().text(), "parent\nbody\ntail");
    }
}

#[test]
fn source_list_body_uses_structural_tab_anywhere_and_preserves_literal_backspace() {
    use viem_core::document::BlockKind;
    for (format, source) in [
        (Format::MarkdownSource, "- parent\n- **body**\n- tail"),
    ] {
        let (mut core, view) = open(source, format);
        let at = core.document().text().find("body").unwrap() + 2;
        insert_at(&mut core, view, at);
        for key in [Key::Tab, Key::BackTab] {
            input(&mut core, view, InputEvent::Key(key));
            let semantic = Document::from_bytes(
                core.document().source_bytes(),
                Encoding::Utf8,
                {
                    Format::Markdown
                },
            )
            .unwrap();
            assert_eq!(semantic.text(), "parent\nbody\ntail");
            let block = &semantic.projection().blocks()[1];
            assert!(
                matches!(block.kind, BlockKind::ListItem { level, .. } if level == if key == Key::Tab { 1 } else { 0 }),
                "{format:?}: {block:?}"
            );
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                core.document().text().find("body").unwrap() + 2
            );
        }
        input(&mut core, view, InputEvent::text("new "));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        reopen_and_history(&mut core, view, source.as_bytes());

        let (mut core, view) = open(source, format);
        let at = core.document().text().find("body").unwrap() + 2;
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        let mut expected = source.to_owned();
        expected.remove(at - 1);
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}

#[test]
fn markdown_source_tab_recognizes_marker_start_and_body_boundary() {
    use viem_core::document::BlockKind;
    let source = "- parent\n- **body**\n- tail";
    for at in [9, 11] {
        let (mut core, view) = open(source, Format::MarkdownSource);
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Tab));
        let semantic = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert!(matches!(
            semantic.projection().blocks()[1].kind,
            BlockKind::ListItem { level: 1, .. }
        ));
    }
}
