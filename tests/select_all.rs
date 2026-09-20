use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardTarget};
use viem_core::command::{CommandStatus, InputEvent, Key, LineMode, Mode};
use viem_core::document::{BlockKind, DocumentId, Encoding, Format, Revision};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;
fn key(core: &mut TestCore, view: ViewId, value: char) {
    special(core, view, Key::Char(value));
}
fn special(core: &mut TestCore, view: ViewId, value: Key) {
    let out = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(value)))
        .unwrap();
    assert!(matches!(
        out.command.unwrap().status,
        CommandStatus::Complete | CommandStatus::Pending
    ));
}
fn select(core: &mut TestCore, view: ViewId) {
    let event = CoreEvent::SelectAll {
        document: core.document().id(),
        revision: core.document().revision(),
    };
    let out = core.handle(view, event).unwrap();
    assert_eq!(out.command.unwrap().status, CommandStatus::Complete);
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualCharacter);
    assert!(commands.is_text_selection());
    assert_eq!(
        core.list_selection_identity(view).unwrap().range(),
        0..core.document().text().len()
    );
    core.document().text_point(commands.cursor()).unwrap();
}
fn opened(source: &str, format: Format) -> (TestCore, ViewId) {
    let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(doc);
    let view = core.add_view(MockTextMeasurementProvider::new(), 90., 48.);
    (core, view)
}

#[test]
fn select_all_reaches_final_wrapped_paragraph_in_every_format_and_line_policy() {
    let tail = format!(
        "{}final العربية 👩‍💻 e\u{301}",
        "many wrapping words ".repeat(30)
    );
    for (source, format) in [
        (format!("first\nsecond\n{tail}"), Format::PlainText),
        (format!("# first\n\nsecond\n\n{tail}"), Format::Markdown),
        (
            format!("# first\n\nsecond\n\n{tail}"),
            Format::MarkdownSource,
        ),
        (
            format!("<h1>first</h1><p>second</p><p>{tail}</p>"),
            Format::Html,
        ),
        (
            format!("<h1>first</h1>\n<p>second</p>\n<p>{tail}</p>"),
            Format::HtmlSource,
        ),
        (
            format!("{{\\rtf1 first\\par second\\par {tail}}}"),
            Format::Rtf,
        ),
    ] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            if format == Format::Rtf && mode == LineMode::PhysicalSource {
                continue;
            }
            for flow in [false, true] {
                let (mut core, view) = opened(&source, format);
                if format.is_source_view() {
                    core.handle(view, CoreEvent::SetParagraphFlow(flow))
                        .unwrap();
                }
                core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
                key(&mut core, view, 'V');
                let before = core.document().source_bytes();
                select(&mut core, view);
                assert_eq!(core.document().source_bytes(), before);
                special(&mut core, view, Key::Ctrl('o'));
                for value in ['"', '+'] {
                    key(&mut core, view, value);
                }
                let clipboard =
                    ClipboardCommandContext::default().with_write(ClipboardTarget::Clipboard);
                let out = core
                    .handle_with_layout(
                        view,
                        CoreEvent::InputWithClipboard {
                            input: InputEvent::key('y'),
                            clipboard,
                        },
                    )
                    .unwrap();
                let output = out.command.unwrap();
                assert_eq!(output.status, CommandStatus::Complete);
                let expected = if format.is_source_view() {
                    source.as_str()
                } else {
                    core.document().text()
                };
                assert_eq!(
                    output.clipboard_writes[0].content().plain_text(),
                    expected,
                    "{format:?} {mode:?} flow={flow}"
                );
            }
        }
    }
}

#[test]
fn selecting_all_includes_trailing_hard_break_and_finishes_insert_undo_unit() {
    for text in ["last", "last\n", "last\n\n", ""] {
        let (mut core, view) = opened(text, Format::PlainText);
        key(&mut core, view, 'i');
        core.handle(view, CoreEvent::Input(InputEvent::Text("é".into())))
            .unwrap();
        let inserted = core.document().source_bytes();
        select(&mut core, view);
        special(&mut core, view, Key::Delete);
        assert_eq!(core.document().text(), "");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        special(&mut core, view, Key::Escape);
        key(&mut core, view, 'u');
        assert_eq!(core.document().source_bytes(), inserted);
        key(&mut core, view, 'u');
        assert_eq!(core.document().source_bytes(), text.as_bytes());
    }
}

#[test]
fn whole_selection_delete_clears_quote_list_and_character_context_with_exact_history() {
    for (source, format) in [
        ("> **quoted**\n> continuation", Format::Markdown),
        ("> - **item**\n> - second", Format::Markdown),
        ("<!--keep--><blockquote><p><b>quoted</b></p></blockquote><!--end-->", Format::Html),
        ("<!doctype html><html><head><title>Keep</title><style>p{color:navy}</style></head><body><blockquote><ul><li>item</li></ul></blockquote></body></html>", Format::Html),
        ("{\\rtf1\\ansi{\\fonttbl{\\f0 Helvetica;}}{\\*\\unknown keep}\\li640\\b quoted}", Format::Rtf),
    ] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            if format == Format::Rtf && mode == LineMode::PhysicalSource { continue; }
            let (mut core, view) = opened(source, format);
            core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
            select(&mut core, view);
            special(&mut core, view, Key::Delete);
            assert_eq!(core.document().text(), "", "{format:?} {mode:?}");
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
            special(&mut core, view, Key::Escape);
            let cleared = core.document().source_bytes();
            assert!(!core.document().projection().blocks().iter().any(|b| b.style.0 == "Block quote" || matches!(b.kind, BlockKind::ListItem { .. })));
            key(&mut core, view, 'i');
            core.handle(view, CoreEvent::Input(InputEvent::Text("plain".into()))).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            assert_eq!(core.document().text(), "plain");
            assert_eq!(core.selected_named_styles(view).unwrap().paragraph.as_ref().map(|x| x.0.as_str()), Some("Paragraph"));
            key(&mut core, view, 'u');
            assert_eq!(core.document().source_bytes(), cleared);
            key(&mut core, view, 'u');
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
            assert_eq!(core.document().source_bytes(), cleared);
            let reopened = Document::from_bytes(cleared, Encoding::Utf8, format).unwrap();
            assert_eq!(reopened.text(), "");
        }
    }
}

#[test]
fn select_all_rejects_stale_identity_without_changing_pending_insert_state() {
    let (mut core, view) = opened("word", Format::Markdown);
    key(&mut core, view, 'i');
    let document = core.document().id();
    let revision = core.document().revision();
    for event in [
        CoreEvent::SelectAll {
            document,
            revision: Revision(revision.0 + 1),
        },
        CoreEvent::SelectAll {
            document: DocumentId(document.0 + 1),
            revision,
        },
    ] {
        assert!(core.handle(view, event).is_err());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.document().source_bytes(), b"word");
        assert_eq!(core.document().revision(), revision);
    }
    assert!(core
        .handle(
            ViewId(u64::MAX),
            CoreEvent::SelectAll { document, revision }
        )
        .is_err());
}

#[test]
fn whole_content_clear_preserves_bom_and_authored_envelopes_without_regeneration() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (source, cleared, format) in [
            ("<!DoCtYpE html><HTML lang='en'><HEAD><!--head--><title>Keep</title></HEAD><BODY><blockquote><b>word</b></blockquote><!--tail--></BODY></HTML>",
             "<!DoCtYpE html><HTML lang='en'><HEAD><!--head--><title>Keep</title></HEAD><BODY><p></p><!--tail--></BODY></HTML>", Format::Html),
            ("> - **word**", "", Format::Markdown),
            ("{\\rtf1\\ansi{\\fonttbl{\\f0 Helvetica;}}{\\*\\unknown Keep}\\li640\\b word}",
             "{\\rtf1\\ansi{\\fonttbl{\\f0 Helvetica;}}{\\*\\unknown Keep}}", Format::Rtf),
        ] {
            fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
                match encoding {
                    Encoding::Utf8 => [vec![0xef, 0xbb, 0xbf], text.as_bytes().to_vec()].concat(),
                    Encoding::Utf16Le => [vec![0xff, 0xfe], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
                    Encoding::Utf16Be => [vec![0xfe, 0xff], text.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
                    _ => unreachable!(),
                }
            }
            if format == Format::Rtf && encoding != Encoding::Utf8 { continue; }
            let original = if format == Format::Rtf { source.as_bytes().to_vec() } else { encoded(source, encoding) };
            let expected = if format == Format::Rtf { cleared.as_bytes().to_vec() } else { encoded(cleared, encoding) };
            let mut document = Document::from_bytes(original.clone(), encoding, format).unwrap();
            document.clear_document_content().unwrap();
            assert_eq!(document.source_bytes(), expected, "{format:?} {encoding:?}");
            assert_eq!(document.has_bom(), format != Format::Rtf);
            assert_eq!(document.text(), "");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected);
        }
    }
}

#[test]
fn html_whole_content_clear_retains_only_unrelated_empty_scopes() {
    for (source, expected) in [
        ("<div data-keep='before'></div>\n<p>word<b></b></p>\n<div data-keep='after'><i></i></div>",
         "<div data-keep='before'></div>\n<p></p>\n<div data-keep='after'><i></i></div>"),
        ("<div><b>word<i></i></b></div>", "<p></p>"),
        ("<b></b><p>word</p>", "<b></b><p></p>"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        document.clear_document_content().unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), "");
        assert_eq!(reopened.projection().blocks().len(), 1);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn html_clearing_an_already_empty_normal_paragraph_does_not_rewrite_it() {
    for source in ["", "<p></p>", "<div></div><p></p>", "<p id='keep'><!--inside--><b></b></p>"] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let revision = document.revision();
        document.clear_document_content().unwrap();
        document.delete_lines(0..0).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision(), revision);
        assert!(!document.history_status().can_undo);
    }
}

#[test]
fn ordinary_last_character_deletion_and_partial_selection_keep_quote_context() {
    for (source, format) in [
        ("> word", Format::Markdown),
        (
            "<!--keep--><blockquote>word</blockquote><!--end-->",
            Format::Html,
        ),
    ] {
        let (mut core, view) = opened(source, format);
        key(&mut core, view, 'v');
        key(&mut core, view, 'l');
        key(&mut core, view, 'd');
        assert_eq!(core.document().text(), "rd");
        assert_eq!(
            core.selected_named_styles(view)
                .unwrap()
                .paragraph
                .unwrap()
                .0,
            "Block quote"
        );
        let modified = String::from_utf8(core.document().source_bytes()).unwrap();
        assert_eq!(modified, source.replace("word", "rd"));
        let len = core.document().text().len();
        // A model body-text deletion is deliberately distinct from ClearDocumentContent.
        let mut document =
            Document::from_bytes(modified.into_bytes(), Encoding::Utf8, format).unwrap();
        document.delete(0..len).unwrap();
        assert_eq!(document.text(), "");
        assert_eq!(document.projection().blocks()[0].style.0, "Block quote");
    }
}

#[test]
fn native_select_all_records_one_replayable_intention_and_honors_counted_insert() {
    let (mut core, view) = opened("original", Format::PlainText);
    for value in "qa".chars() {
        key(&mut core, view, value);
    }
    select(&mut core, view);
    special(&mut core, view, Key::Delete);
    special(&mut core, view, Key::Escape);
    key(&mut core, view, 'q');
    let recorded = core.command_state(view).unwrap().register('a').unwrap();
    assert_eq!(recorded.text, "<SelectAll><Del><Esc>");
    key(&mut core, view, 'i');
    core.handle(
        view,
        CoreEvent::Input(InputEvent::Text("different longer ending 👩‍💻".into())),
    )
    .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    for value in "@a".chars() {
        key(&mut core, view, value);
    }
    assert_eq!(core.document().text(), "");
    key(&mut core, view, 'u');
    assert_eq!(core.document().text(), "different longer ending 👩‍💻");

    let (mut core, view) = opened("", Format::PlainText);
    for value in "3i".chars() {
        key(&mut core, view, value);
    }
    core.handle(view, CoreEvent::Input(InputEvent::Text("é".into())))
        .unwrap();
    select(&mut core, view);
    assert_eq!(core.document().text(), "ééé");
    special(&mut core, view, Key::Delete);
    special(&mut core, view, Key::Escape);
    key(&mut core, view, 'u');
    assert_eq!(core.document().text(), "ééé");
    key(&mut core, view, 'u');
    assert_eq!(core.document().text(), "");
}
