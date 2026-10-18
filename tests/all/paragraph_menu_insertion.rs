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

#[test]
fn paragraph_styles_on_opened_empty_heading_boundary_reopen_and_restore_source() {
    for source in ["## Heading\n\nFollowing paragraph", "Heading\n-------\n\nFollowing paragraph"] {
        for keys in [vec![Key::Char('o')], vec![Key::Char('A'), Key::Enter], vec![Key::Char('j'), Key::Char('O')]] {
            for style in ["Heading1", "Code Block", "Block quote", "BulletedList1", "NumberedList1"] {
                let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
                let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
                for key in &keys {
                    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(*key))).unwrap();
                }
                assert_eq!(core.document().text(), "Heading\n\nFollowing paragraph");
                let opened = core.document().source_bytes();
                let expected = core.list_selection_identity(view).unwrap();
                let event = match style {
                    "BulletedList1" | "NumberedList1" => CoreEvent::SetListStyle {
                        expected, style: Some(if style == "BulletedList1" { viem_core::document::ListStyle::Bullet } else { viem_core::document::ListStyle::Numbered }),
                    },
                    _ => CoreEvent::AssignNamedStyle {
                        expected,
                        style_sheet_revision: core.document().projection().style_sheet().revision,
                        namespace: StyleNamespace::Block,
                        style: style.into(),
                    },
                };
                core.handle(view, event).unwrap_or_else(|error| panic!("{source:?} {keys:?} {style}: {error:?}"));
                assert_eq!(core.document().text(), "Heading\n\nFollowing paragraph");
                let blocks = core.document().projection().blocks();
                assert_eq!(blocks.len(), 3, "{source:?} {keys:?} {style}: {blocks:?}");
                assert_eq!(blocks[0].style.0, "Heading2");
                assert_eq!(blocks[2].style.0, "Paragraph");
                if style == "Block quote" { assert_eq!(blocks[1].quote_depth, 1); }
                else { assert_eq!(blocks[1].style.0, style); }
                let styled = core.document().source_bytes();
                let reopened = Document::from_bytes(styled.clone(), Encoding::Utf8, Format::Markdown).unwrap();
                assert_eq!(reopened.text(), core.document().text());
                assert_eq!(reopened.projection().blocks().iter().map(|block| (&block.range, &block.kind, &block.style, block.quote_depth)).collect::<Vec<_>>(),
                    blocks.iter().map(|block| (&block.range, &block.kind, &block.style, block.quote_depth)).collect::<Vec<_>>());
                core.handle(view, CoreEvent::Input(InputEvent::text("X"))).unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
                assert_eq!(core.document().text(), "Heading\nX\nFollowing paragraph");
                let changed = core.document().source_bytes();
                for expected in [&styled[..], &opened[..], source.as_bytes()] {
                    core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
                    assert_eq!(core.document().source_bytes(), expected, "{source:?} {keys:?} {style}: undo");
                }
                for expected in [&opened, &styled, &changed] {
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
                    assert_eq!(&core.document().source_bytes(), expected);
                }
            }
        }
    }
}
