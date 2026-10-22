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
fn enter_replaces_a_folded_source_break_but_preserves_literal_spaces() {
    for (source, at, expected, saved) in [
        ("a\nb", 1, "a\nb", "a\n\nb"),
        ("a\nb\nc", 1, "a\nb c", "a\n\nb\nc"),
        ("**a\nb**", 1, "a\nb", "**a**\n\n**b**"),
        ("a b", 1, "a\n b", "a\n\n&#32;b"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), expected, "{source:?}");
        assert_eq!(core.document().source_bytes(), saved.as_bytes(), "{source:?}");
        assert_eq!(core.command_state(view).unwrap().cursor(), at + 1);
        reopen_and_history(&mut core, view, source.as_bytes());
    }
    for source in ["a\nb", "**a\nb**"] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, 1);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), "a\nb", "{source:?}");
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?}");
    }
    for source in ["> a\n> b", "- a\n  b"] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, 1);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), "a\nb", "{source:?}");
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn deleting_a_continuation_lines_last_character_keeps_its_folded_space() {
    for (source, expected, saved) in [
        ("ab\nc", "ab ", "ab "),
        ("ab\nc\nd", "ab  d", "ab&#32;\nd"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, 4);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), expected);
        assert_eq!(core.document().source_bytes(), saved.as_bytes());
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn overflowing_ordered_continuations_follow_commonmark_literal_text() {
    let source = "999999999. big";
    for format in [Format::Markdown] {
        for above in [false, true] {
            let (mut core, view) = open(source, format);
            input(&mut core, view, InputEvent::key(if above { 'O' } else { 'o' }));
            let saved = if above { "999999999. \n1000000000. big" }
                else { "999999999. big\n1000000000. " };
            assert_eq!(core.document().source_bytes(), saved.as_bytes(), "{format:?} above={above}");
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
            let cursor = core.command_state(view).unwrap().cursor();
            let expected = if above { 0 } else { core.document().text().len() };
            assert_eq!(cursor, expected, "{format:?} above={above}");
            reopen_and_history(&mut core, view, source.as_bytes());
        }
        let (mut core, view) = open(source, format);
        input(&mut core, view, InputEvent::key('A'));
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().source_bytes(), b"999999999. big\n1000000000. ");
        reopen_and_history(&mut core, view, source.as_bytes());
    }
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
fn empty_structural_owner_keyboard_edits_preserve_neighbors_and_history() {
    let mut failures = Vec::new();
    for owner in ["##", "## ###", "##\t###", ">", "-", "1.", "```\n```", "> ##", "> -", "- ##", "- ```\n  ```", "<span></span>", "> <span></span>", "- <span></span>", "- a\n\n  \n\n  b", "> - a\n> \n>   \n> \n>   b", "- parent\n  - a\n\n    \n\n    b"] {
        for key in [Key::Char('o'), Key::Char('O'), Key::Enter, Key::ShiftEnter] {
            let result = std::panic::catch_unwind(|| {
            let source = format!("Before\n\n{owner}\n\nAfter");
            let (mut core, view) = open(&source, Format::Markdown);
            let original = core.document().projection().blocks().iter().find(|block| block.range.is_empty())
                .unwrap_or_else(|| panic!("missing empty owner in {source:?}")).clone();
            let at = original.range.start;
            core.handle_with_layout(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            if matches!(key, Key::Enter | Key::ShiftEnter) { input(&mut core, view, InputEvent::key('i')); }
            input(&mut core, view, InputEvent::Key(key));
            if key != Key::Enter {
                // Opening a line in code adds a literal line to the same owner.
                let code = original.style.0 == "Code Block";
                let range = match key {
                    Key::ShiftEnter => at..at + 1,
                    Key::Char('o' | 'O') if code => at..at + 1,
                    Key::Char('O') => at + 1..at + 1,
                    _ => at..at,
                };
                let retained = core.document().projection().blocks().iter().find(|block| block.range == range)
                    .unwrap_or_else(|| panic!("missing retained owner in {source:?} after {key:?}"));
                let original_lists = original.containers.iter().filter_map(|member| {
                    if let viem_core::document::ContainerKind::List { ordered } = member.container.kind { Some(ordered) }
                    else { None }
                }).collect::<Vec<_>>();
                if original.kind == viem_core::document::BlockKind::Paragraph && !original_lists.is_empty() {
                    let level = original_lists.len() as u8 - 1;
                    assert!(matches!(retained.kind, viem_core::document::BlockKind::ListItem {
                        ordered, level: retained_level, item_start: false, ..
                    } if Some(&ordered) == original_lists.last() && retained_level == level), "{source:?} {key:?}: {retained:?}");
                    assert_eq!(retained.style, core.document().projection().style_sheet().list_style_id(*original_lists.last().unwrap(), level));
                    assert_eq!(retained.containers.iter().map(|member| &member.container).collect::<Vec<_>>(),
                        original.containers.iter().map(|member| &member.container).collect::<Vec<_>>(), "{source:?} {key:?}");
                    assert_eq!(retained.quote_depth, original.quote_depth, "{source:?} {key:?}");
                } else {
                    assert_eq!((&retained.style, std::mem::discriminant(&retained.kind), retained.quote_depth),
                        (&original.style, std::mem::discriminant(&original.kind), original.quote_depth), "{source:?} {key:?}");
                }
            }
            let empty_bytes = core.document().source_bytes();
            let reopened = Document::from_bytes(empty_bytes, Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), core.document().text(), "{source:?} {key:?}");
            assert_eq!(reopened.projection().blocks().iter().map(|block| (&block.range, &block.kind, &block.style, block.quote_depth)).collect::<Vec<_>>(),
                core.document().projection().blocks().iter().map(|block| (&block.range, &block.kind, &block.style, block.quote_depth)).collect::<Vec<_>>(), "{source:?} {key:?}");
            input(&mut core, view, InputEvent::text("X"));
            assert!(core.document().text().starts_with("Before\n"), "{source:?} {key:?}");
            assert!(core.document().text().ends_with("\nAfter"), "{source:?} {key:?}");
            assert!(core.document().source_bytes().starts_with(b"Before\n\n"), "{source:?} {key:?}");
            assert!(core.document().source_bytes().ends_with(b"\n\nAfter"), "{source:?} {key:?}: {:?}", String::from_utf8_lossy(&core.document().source_bytes()));
            reopen_and_history(&mut core, view, source.as_bytes());
            });
            if let Err(error) = result {
                let detail = error.downcast_ref::<String>().cloned()
                    .or_else(|| error.downcast_ref::<&str>().map(|value| (*value).to_owned()))
                    .unwrap_or_else(|| "unexpected panic".to_owned());
                failures.push(format!("{owner:?} {key:?}: {detail}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
        let at = core.command_state(view).unwrap().cursor();
        assert_eq!(at, core.document().text().len() - usize::from(format.is_literal()));
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
        assert_eq!(core.document().text(), if format.is_literal() { "body\nnext\n" } else { "body\nnext" }, "{format:?}");
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

fn caret_x(core: &Editor, view: ViewId) -> f32 {
    let at = core.command_state(view).unwrap().cursor();
    let layout = core.layout(view).unwrap().snapshot().unwrap();
    let caret = layout.caret_point(at, BoundaryAffinity::Downstream).unwrap();
    layout.caret_geometry(caret).unwrap().rect.x
}

#[test]
fn exiting_an_empty_list_item_immediately_unindents_the_caret() {
    use viem_core::document::ContainerKind;
    for source in [
        "intro\n\n- body",
        "intro\n\n- body\n",
        "intro\n\n- body\n\n",
        "intro\n\n- body\n- tail",
        "intro\n\n- body\n\nsuffix",
        "intro\n\n* body",
        "intro\n\n9) body\n10) tail",
        "intro\r\n\r\n- body\r\n",
        "> intro\n>\n> - body",
    ] {
        for exit in [Key::Enter, Key::Backspace] {
            let (mut core, view) = open(source, Format::Markdown);
            insert_at(&mut core, view, 0);
            let outside_x = caret_x(&core, view);
            let at = core.document().text().find("body").unwrap() + 4;
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            input(&mut core, view, InputEvent::Key(Key::Enter));
            assert!(caret_x(&core, view) > outside_x, "{source:?} {exit:?}: new item");
            let empty = core.command_state(view).unwrap().cursor();
            input(&mut core, view, InputEvent::Key(exit));
            assert_eq!(core.command_state(view).unwrap().cursor(), empty, "{source:?} {exit:?}");
            assert_eq!(caret_x(&core, view), outside_x, "{source:?} {exit:?}: exit");
            let block = core.document().projection().blocks().iter()
                .find(|block| block.range == (empty..empty)).unwrap();
            assert!(!block.containers.iter().any(|member|
                matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem)),
                "{source:?} {exit:?}: {block:?}");
            let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            let fresh = reopened.projection().blocks().iter()
                .find(|block| block.range == (empty..empty)).unwrap();
            assert!(!fresh.containers.iter().any(|member|
                matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem)),
                "{source:?} {exit:?}: reopened {fresh:?}");
            reopen_and_history(&mut core, view, source.as_bytes());
        }
    }
}

#[test]
fn option_enter_keeps_empty_continuation_carets_indented() {
    for source in ["intro\n\n- body", "intro\n\n- body\n- tail", "intro\n\n1. body", "> intro\n>\n> - body"] {
        let (mut core, view) = open(source, Format::Markdown);
        insert_at(&mut core, view, 0);
        let outside_x = caret_x(&core, view);
        let at = core.document().text().find("body").unwrap() + 4;
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let owner = core.document().projection().blocks().iter()
            .find(|block| block.range.start <= at && at <= block.range.end).unwrap().clone();
        for _ in 0..2 {
            input(&mut core, view, InputEvent::Key(Key::AltEnter));
            let empty = core.command_state(view).unwrap().cursor();
            assert!(caret_x(&core, view) > outside_x, "{source:?}");
            let block = core.document().projection().blocks().iter()
                .find(|block| block.range == (empty..empty)).unwrap();
            assert!(matches!(block.kind, viem_core::document::BlockKind::ListItem { item_start: false, .. }), "{source:?}");
            assert!(block.containers.iter().map(|member| &member.container)
                .eq(owner.containers.iter().map(|member| &member.container)), "{source:?}");
        }
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn exiting_an_empty_item_reuses_its_existing_explicit_body() {
    for (source, expected) in [
        ("- first\n- <span></span>", "- first\n\n<span></span>"),
        ("- first\n- <span></span>\n- tail", "- first\n\n<span></span>\n\n- tail"),
    ] {
        for exit in [Key::Enter, Key::Backspace] {
            let (mut core, view) = open(source, Format::Markdown);
            insert_at(&mut core, view, "first\n".len());
            input(&mut core, view, InputEvent::Key(exit));
            assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{source:?} {exit:?}");
            assert_eq!(core.command_state(view).unwrap().cursor(), "first\n".len(), "{source:?} {exit:?}");
            reopen_and_history(&mut core, view, source.as_bytes());
        }
    }
}

#[test]
fn removing_a_whole_list_needs_no_empty_body_between_its_prose_paragraphs() {
    let source = "- first\n- \n- last";
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    document.set_list_style(0..document.text().len(), None).unwrap();
    assert_eq!(document.source_bytes(), b"first\n\n\n\nlast");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), b"first\n\n\n\nlast");
}

#[test]
fn enter_in_an_unclosed_empty_code_block_replaces_its_eof_caret_locally() {
    use crate::markdown_test_support::place_insert_boundary;
    use viem_core::command::CommandInterpreter;
    use viem_core::document::measure_document_work;

    let mut measured = None;
    for paragraphs in [100, 5000] {
        let prefix = "Unrelated *styled* prose.\n\n".repeat(paragraphs);
        let source = format!("{prefix}Guard.\n\n```rust\n");
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let code = document.projection().blocks().last().unwrap().clone();
        assert!(code.range.is_empty());
        assert_eq!(code.style.0, "Code Block");
        assert!(document.projection().provenance().iter().any(|span|
            span.formatted == code.range && span.source == (source.len()..source.len())),
            "fixture exercises the empty code owner's exact EOF caret");
        let at = code.range.end;
        let mut commands = CommandInterpreter::new();
        place_insert_boundary(&mut commands, &mut document, at);

        let (result, work) = measure_document_work(||
            commands.handle(&mut document, InputEvent::Key(Key::Enter)));
        result.unwrap();
        assert_eq!(work.full_projection_candidates, 0, "{work:?}");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
        assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{work:?}");
        assert!(work.source_decoded_bytes < 8192, "{work:?}");
        assert!(work.projected_formatted_bytes < 4096, "{work:?}");
        let current = (work.source_decoded_bytes, work.projected_formatted_bytes, work.model_requests);
        if let Some(previous) = measured {
            assert_eq!(previous, current, "unrelated prefix changes empty code Enter work: {work:?}");
        }
        measured = Some(current);
        assert_eq!(commands.cursor(), at + 1);
        let code = document.projection().blocks().last().unwrap();
        assert_eq!(code.style.0, "Code Block");
        assert_eq!(&document.text()[code.range.clone()], "\n");
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(fresh.projection().provenance(), document.projection().provenance(),
            "the old EOF caret must not survive beside the new code boundary");
        crate::support::assert_reopens_as_displayed(&document, "empty code Enter");

        commands.handle(&mut document, InputEvent::text("next")).unwrap();
        let code = document.projection().blocks().last().unwrap();
        assert_eq!(code.style.0, "Code Block");
        assert_eq!(&document.text()[code.range.clone()], "\nnext");
        let saved = document.source_bytes();
        assert!(saved.starts_with(prefix.as_bytes()));
        crate::support::assert_reopens_as_displayed(&document, "typing after empty code Enter");
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
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

