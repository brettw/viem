use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, FontSlant, Format, SemanticInlineStyle,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

#[test]
fn marked_text_commits_pending_style_atomically_and_retains_it_for_subsequent_typing() {
    for (format, source, at) in [
        (Format::Markdown, "word", 0),
        (Format::MarkdownSource, "word", 0),

    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
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
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style: SemanticInlineStyle::Emphasis,
                enabled: true,
            },
        )
        .unwrap();
        let target = CompositionTarget::at_offsets(core.document(), at..at).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3))),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: marked text must remain an overlay"
        );
        let outcome = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        assert!(outcome.document_changed);
        let caret = core.command_state(view).unwrap().cursor();
        core.document().text_point(caret).unwrap();
        assert_ne!(
            DocumentLayoutStyles::character_at(core.document().projection(), caret, true)
                .unwrap()
                .slant,
            FontSlant::Upright,
            "{format:?}"
        );
        assert_ne!(
            core.selected_character_style(view).unwrap().slant,
            FontSlant::Upright
        );
        let after_composition = core.document().source_bytes();
        core.handle(view, CoreEvent::Input(InputEvent::text("é")))
            .unwrap();
        assert_ne!(
            core.selected_character_style(view).unwrap().slant,
            FontSlant::Upright
        );
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            after_composition,
            "{format:?}: next typing has its own undo unit"
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: composition text and formatting undo together"
        );
    }
}

#[test]
fn consecutive_compositions_over_native_selection_keep_separate_valid_history() {
    use viem_core::document::HistoryNavigationRequest;
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        let mut core = Core::new(Document::from_bytes(b"hello world".to_vec(), Encoding::Utf8, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
        for (offset, extend) in [(0, false), (5, true)] {
            core.handle_with_layout(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: offset,
                affinity: BoundaryAffinity::Downstream, extend_selection: extend,
            }).unwrap();
        }
        for (range, text) in [(0..5, "é"), (2..2, "ñ")] {
            let target = CompositionTarget::at_offsets(core.document(), range).unwrap();
            core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
            core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(text, text.len()..text.len())))).unwrap();
            core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Commit)).unwrap();
        }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("x"))).unwrap();
        assert_eq!(core.document().text(), "éñx world", "{format:?}");
        for expected in ["éñ world", "é world", "hello world"] {
            core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().text(), expected, "{format:?}");
        }
        for expected in ["é world", "éñ world", "éñx world"] {
            core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().text(), expected, "{format:?}");
        }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("z"))).unwrap();
    }
}

#[test]
fn compositions_adjacent_to_images_preserve_atomic_geometry_and_history() {
    use viem_core::document::HistoryNavigationRequest;
    for source in ["![img](local.png)", "a ![img](i.png) b"] {
        for after in [false, true] {
            for commit in [false, true] {
                let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
                let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
                let original = core.document().text().to_owned();
                let images = core.document().projection().inline_images_for_region(&(0..core.document().text().len()));
                let image = &images[0];
                let at = if after { image.range.end } else { image.range.start };
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
                let target = CompositionTarget::at_offsets(core.document(), at..at).unwrap();
                core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
                for text in ["e", "é"] {
                    core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(text, text.len()..text.len())))).unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                }
                core.handle_with_layout(view, CoreEvent::Composition(if commit { CompositionEvent::Commit } else { CompositionEvent::Cancel })).unwrap();
                let mut expected = original;
                if commit { expected.insert_str(at, "é"); }
                assert_eq!(core.document().text(), expected);
                assert_eq!(core.document().projection().inline_images_for_region(&(0..core.document().text().len()))[0].range.len(), 3);
                if commit {
                    core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                }
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn press_and_hold_replacement_keeps_inline_context_and_exact_history() {
    use viem_core::document::HistoryNavigationRequest;
    let mut failures = Vec::new();
    for (source, at) in [("**`code in bold`**", 2), ("<ins>unclosed", 2),
        ("<mailto:a@b.c>", 2), ("[a]: b\n[c]: d", 0), ("   ", 0)] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
        let original = core.document().text().to_owned();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle_with_layout(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let result = core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("e")));
        if let Err(error) = result { failures.push(format!("{source:?} initial typing: {error:?}")); continue; }
        let typed = core.document().source_bytes();
        let target = CompositionTarget::at_offsets(core.document(), at..at + 1).unwrap();
        core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Begin(target))).unwrap();
        core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("é", 2..2)))).unwrap();
        if let Err(error) = core.handle_with_layout(view, CoreEvent::Composition(CompositionEvent::Commit)) {
            failures.push(format!("{source:?} replacing composition: {error:?}")); continue;
        }
        let mut expected = original;
        expected.insert_str(at, "é");
        assert_eq!(core.document().text(), expected);
        let after = core.document().source_bytes();
        let fresh = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(fresh.text(), expected);
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), typed);
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), typed);
        core.handle_with_layout(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), after);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
