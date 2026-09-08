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
        (Format::Html, "<p>αβ👩‍💻xy</p><p>tail</p>"),
        (
            Format::Html,
            "<blockquote cite='keep'><p>αβ👩‍💻xy</p></blockquote><p>tail</p>",
        ),
        (Format::Html, "<ul><li>αβ👩‍💻xy</li></ul><p>tail</p>"),
        (
            Format::Html,
            "<ol start='4'><li>αβ👩‍💻xy</li></ol><p>tail</p>",
        ),
        (
            Format::Html,
            "<ul><li><blockquote>αβ👩‍💻xy</blockquote></li></ul><p>tail</p>",
        ),
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
        (Format::Rtf, "{\\rtf1 abcd\\par tail}"),
    ] {
        let (mut core, view) = open(source, format);
        let before_text = core.document().text().to_owned();
        let at = if format == Format::Rtf {
            2
        } else {
            "αβ".len()
        };
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
                    Format::Html => "<br>",
                    Format::Rtf => "\\line ",
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
        (Format::Html, "<p>abc</p>"),
        (Format::Markdown, "abc"),
        (Format::Markdown, "# abc"),
        (Format::Rtf, "{\\rtf1 abc}"),
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
        (Format::Html, "<p></p>"),
        (Format::Html, ""),
        (Format::Html, "<blockquote></blockquote>"),
        (Format::Html, "<ul><li></li></ul>"),
        (Format::Markdown, ""),
        (Format::Markdown, "# "),
        (Format::Markdown, "> "),
        (Format::Markdown, "- "),
        (Format::Rtf, "{\\rtf1 }"),
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
        (Format::HtmlSource, "<p>abc</p>"),
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
fn empty_quote_enter_and_first_backspace_remove_style_without_joining_previous_paragraph() {
    for (format, source, at, enter, expected) in [
        (Format::Html, "<p>previous</p><blockquote cite='keep'><p>body</p></blockquote><p>next</p>", 9, false, "previous\nbody\nnext"),
        (Format::Html, "<p>previous</p><ul><li>body</li></ul><p>next</p>", 9, false, "previous\nbody\nnext"),
        (Format::Html, "<p>previous</p><ol><li>body</li></ol><p>next</p>", 9, false, "previous\nbody\nnext"),
        (Format::Html, "<p>previous</p><ul><li><blockquote>body</blockquote></li><li>other</li></ul><p>next</p>", 9, false, "previous\nbody\nother\nnext"),
        (Format::Markdown, "previous\n\n> body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Markdown, "previous\n\n- body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Markdown, "previous\n\n1. body\n\nnext", 9, false, "previous\nbody\nnext"),
        (Format::Html, "<p>previous</p><blockquote></blockquote><p>next</p>", 9, true, "previous\n\nnext"),
        (Format::Markdown, "previous\n\n> \n\nnext", 9, true, "previous\n\nnext"),
        (Format::Html, "<blockquote></blockquote>", 0, true, ""),
        (Format::Markdown, "> ", 0, true, ""),
    ] {
        let (mut core, view) = open(source, format);
        assert_eq!(core.document().text(), expected, "fixture {source}");
        insert_at(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(if enter { Key::Enter } else { Key::Backspace }));
        assert_eq!(core.document().text(), expected, "{source}");
        let block = core.document().projection().blocks().into_iter().find(|block| block.range.start == at).unwrap();
        assert_eq!(block.style.0, "Paragraph", "{source}");
        assert_eq!(core.command_state(view).unwrap().cursor(), at);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        reopen_and_history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn imported_list_quote_combinations_accept_typing_before_and_after_a_hard_break() {
    for source in [
        "<ul><li><blockquote>body</blockquote></li><li>other</li></ul>",
        "<blockquote><ol><li>body</li><li>other</li></ol></blockquote>",
    ] {
        let (mut core, view) = open(source, Format::Html);
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
    for (format, source) in [(Format::Html, "<p>ab</p>"), (Format::Markdown, "ab")] {
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
        "<p>👩‍💻body</p>".as_bytes().to_vec(),
        Encoding::Utf8,
        Format::Html,
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
