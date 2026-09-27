use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{Document, Encoding, Format, StyleNamespace};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

#[test]
fn quote_menu_at_end_inserts_an_empty_paragraph_and_preserves_existing_prose() {
    for (format, source, before, after) in [
        (Format::Markdown, "one **two**", "one two", "one two\n"),
        (Format::Markdown, "one\\\ntwo", "one\ntwo", "one\ntwo\n"),
        (Format::Markdown, "one<br>two<br>", "one\ntwo\n", "one\ntwo\n"),
        (Format::Markdown, "one\n", "one", "one\n"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), before);
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        for event in [Key::Char('G'), Key::Char('A')] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(event)))
                .unwrap();
        }
        assert_eq!(core.command_state(view).unwrap().cursor(), before.len());
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Block,
                style: "Block quote".into(),
            },
        )
        .unwrap_or_else(|error| panic!("{format:?}: {source:?}: {error:?}"));
        assert_eq!(core.document().text(), after, "{source}");
        let blocks = core.document().projection().blocks();
        assert_eq!(blocks.len(), 2, "{source}: {blocks:?}");
        assert_eq!(blocks[0].style.0, "Paragraph");
        assert_eq!(blocks[1].quote_depth, 1);
        assert_eq!(blocks[1].range, after.len()..after.len());
        let cursor = core.command_state(view).unwrap().cursor();
        assert_eq!(cursor, after.len());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(
            core.selected_named_styles(view).unwrap().paragraph,
            Some("Block quote".into())
        );
        core.document().text_point(cursor).unwrap();
        let quoted = core.document().source_bytes();
        let reopened = Document::from_bytes(quoted.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), after);
        assert_eq!(reopened.projection().blocks()[1].quote_depth, 1);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), quoted);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("العربية")))
            .unwrap();
        assert_eq!(core.document().text(), format!("{after}العربية"));
        assert_eq!(
            core.document().projection().blocks()[1].quote_depth,
            1
        );
    }
}

#[test]
fn quote_menu_uses_an_existing_empty_paragraph() {
    for (format, source, before) in [
        (Format::Markdown, "Words\n\n", "Words\n"),
        (Format::Markdown, "", ""),
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        for key in [Key::Char('G'), Key::Char('A')] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Block,
                style: "Block quote".into(),
            },
        )
        .unwrap();
        assert_eq!(core.document().text(), before);
        assert_eq!(
            core.document()
                .projection()
                .blocks()
                .last()
                .unwrap()
                .quote_depth,
            1
        );
        if !before.is_empty() {
            assert_eq!(
                core.document().projection().blocks()[0].style.0,
                "Paragraph"
            );
        }
    }
}
