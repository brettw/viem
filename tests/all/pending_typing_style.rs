use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest, SemanticInlineStyle,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};
fn key(key: Key) -> CoreEvent {
    CoreEvent::Input(InputEvent::Key(key))
}
fn fixture(format: Format, source: &str) -> (Core<MockTextMeasurementProvider>, viem_core::ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    (core, view)
}
#[test]
fn source_italic_input_before_existing_strong_word_keeps_unicode_and_whitespace() {
    let source = "A **word** and more prose.\nA continuation.\n\nLast paragraph.";
    let (mut core, view) = fixture(Format::MarkdownSource, source);
    core.handle(view, key(Key::Char('i'))).unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: source.find("word").unwrap(),
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
    core.handle(view, CoreEvent::Input(InputEvent::text("é👩🏽‍💻 ")))
        .unwrap();
    assert!(String::from_utf8_lossy(&core.document().source_bytes()).contains("é👩🏽‍💻 "));
}
#[test]
fn wys_italic_cycle_before_existing_strong_word_in_flowed_paragraph() {
    let source = "A **word** and more prose.\nA continuation.\n\nLast paragraph.";
    let (mut core, view) = fixture(Format::Markdown, source);
    core.handle(view, key(Key::Char('i'))).unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 2,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
    core.handle(view, CoreEvent::Input(InputEvent::text("é👩🏽‍💻 ")))
        .expect("italic text");
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, false);
    core.handle(view, CoreEvent::Input(InputEvent::text("plain ")))
        .expect("plain text");
    core.handle(view, key(Key::Escape)).unwrap();
    core.handle(view, key(Key::Char('u'))).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, key(Key::Ctrl('r'))).unwrap();
    assert_eq!(
        core.document().text(),
        "A é👩🏽‍💻 plain word and more prose. A continuation.\nLast paragraph."
    );
}
fn toggle(
    core: &mut Core<MockTextMeasurementProvider>,
    view: viem_core::ViewId,
    style: SemanticInlineStyle,
    enabled: bool,
) {
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style,
            enabled,
        },
    )
    .unwrap();
}
#[test]
fn pending_italic_is_clean_until_text_and_undo_restores_exact_source() {
    for (format, source, at) in [
        (Format::Markdown, "word", 0),
        (Format::MarkdownSource, "word", 0),

    ] {
        let (mut core, view) = fixture(format, source);
        core.handle(view, key(Key::Char('i'))).unwrap();
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
        let before = core.document().revision();
        toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
        assert_eq!(core.document().revision(), before, "{format:?}");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(
            core.selected_character_style(view).unwrap().slant
                != viem_core::document::FontSlant::Upright
        );
        let out = core
            .handle(view, CoreEvent::Input(InputEvent::Text("Hi".into())))
            .unwrap();
        assert!(out.document_changed, "{format:?}: {out:?}");
        let caret = core.command_state(view).unwrap().cursor();
        assert!(
            DocumentLayoutStyles::character_at(core.document().projection(), caret, true)
                .unwrap()
                .slant
                != viem_core::document::FontSlant::Upright,
            "{format:?}: {:?}",
            core.document().text()
        );
        core.handle(view, key(Key::Escape)).unwrap();
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}"
        );
    }
}
#[test]
fn escape_and_pointer_cancel_pending_without_source_changes() {
    let (mut core, view) = fixture(Format::Markdown, "word");
    core.handle(view, key(Key::Char('i'))).unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Strong, true);
    core.handle(view, key(Key::Escape)).unwrap();
    assert_eq!(core.document().source_bytes(), b"word");
    core.handle(view, key(Key::Char('i'))).unwrap();
    assert!(!core.selected_character_style(view).unwrap().bold);
    toggle(&mut core, view, SemanticInlineStyle::Strong, true);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 2,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert!(!core.selected_character_style(view).unwrap().bold);
}
#[test]
fn typing_can_disable_existing_style_and_continue_inherited_run() {
    for (format, source, at) in [
        (Format::Markdown, "**word**", 2),
        (Format::MarkdownSource, "**word**", 4),

    ] {
        let (mut core, view) = fixture(format, source);
        core.handle(view, key(Key::Char('i'))).unwrap();
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
        toggle(&mut core, view, SemanticInlineStyle::Strong, false);
        for input in ["a", "b"] {
            let out = core
                .handle(view, CoreEvent::Input(InputEvent::Text(input.into())))
                .unwrap_or_else(|e| {
                    panic!(
                        "{format:?} {input}: {e:?} source {:?}",
                        String::from_utf8_lossy(&core.document().source_bytes())
                    )
                });
            assert!(out.document_changed, "{format:?} {out:?}");
            let caret = core.command_state(view).unwrap().cursor();
            assert!(
                !DocumentLayoutStyles::character_at(core.document().projection(), caret, true)
                    .unwrap()
                    .bold,
                "{format:?} {:?}",
                core.document().text()
            );
        }
        core.handle(view, key(Key::Escape)).unwrap();
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
#[test]
fn combined_bold_italic_repeats_and_is_one_insert_undo() {
    for format in [
        Format::Markdown,
        Format::MarkdownSource,
        ] {
        let source = match format {

            _ => "x",
        };
        let (mut core, view) = fixture(format, source);
        core.handle(view, key(Key::Char('i'))).unwrap();
        toggle(&mut core, view, SemanticInlineStyle::Strong, true);
        toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
        for input in ["a", "b"] {
            let out = core
                .handle(view, CoreEvent::Input(InputEvent::Text(input.into())))
                .unwrap_or_else(|e| {
                    panic!(
                        "{format:?} {input}: {e:?} source {:?}",
                        String::from_utf8_lossy(&core.document().source_bytes())
                    )
                });
            assert!(out.document_changed, "{format:?} {out:?}");
        }
        let caret = core.command_state(view).unwrap().cursor();
        let style =
            DocumentLayoutStyles::character_at(core.document().projection(), caret, true).unwrap();
        assert!(
            style.bold && style.slant != viem_core::document::FontSlant::Upright,
            "{format:?} {:?}",
            core.document().text()
        );
        core.handle(view, key(Key::Escape)).unwrap();
        let before_repeat = core.document().source_bytes();
        let out = core.handle(view, key(Key::Char('.'))).unwrap();
        assert!(out.document_changed, "{format:?}: {out:?}");
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), before_repeat);
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
#[test]
fn stale_formatting_targets_leave_state_unchanged() {
    let (mut core, view) = fixture(Format::Markdown, "word");
    core.handle(view, key(Key::Char('i'))).unwrap();
    let stale = core.list_selection_identity(view).unwrap();
    core.handle(view, key(Key::Right)).unwrap();
    assert!(core.handle(view, CoreEvent::SetStrikethrough {
        expected: stale.clone(), enabled: true,
    }).is_err());
    assert!(core
        .handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected: stale,
                style: SemanticInlineStyle::Strong,
                enabled: true
            }
        )
        .is_err());
    assert_eq!(core.document().source_bytes(), b"word");
    assert!(!core.selected_character_style(view).unwrap().bold);

}
#[test]
fn pending_state_is_view_local_and_external_reprojection_cancels_it() {
    let (mut core, view) = fixture(Format::Markdown, "word");
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    core.handle(view, key(Key::Char('i'))).unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
    assert_eq!(
        core.selected_character_style(other).unwrap().slant,
        viem_core::document::FontSlant::Upright
    );
    core.handle(other, key(Key::Char('i'))).unwrap();
    core.handle(other, CoreEvent::Input(InputEvent::Text("a".into())))
        .unwrap();
    assert_eq!(
        core.selected_character_style(view).unwrap().slant,
        viem_core::document::FontSlant::Upright
    );
}
#[test]
fn whitespace_does_not_create_empty_markdown_spans_and_counts_replay_style() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let (mut core, view) = fixture(format, "x");
        core.handle(view, key(Key::Char('3'))).unwrap();
        core.handle(view, key(Key::Char('i'))).unwrap();
        toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
        core.handle(view, CoreEvent::Input(InputEvent::Text(" ".into())))
            .unwrap();
        assert_eq!(core.document().source_bytes(), if format == Format::Markdown { b"&#32;x".as_slice() } else { b" x".as_slice() });
        core.handle(view, CoreEvent::Input(InputEvent::Text("é".into())))
            .unwrap();
        core.handle(view, key(Key::Escape)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&core.document().source_bytes())
                .matches('é')
                .count(),
            3
        );
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), b"x");
    }
}
#[test]
fn continuing_style_uses_local_projection_and_one_literal_source_patch_in_large_document() {
    use viem_core::document::{
        FontSlant, FormattedPayloadEdit, FormattedTextPayload, StyleProperty, StylePropertyValue,
    };
    let mut source = "line\n\n".repeat(10_000);
    source.push_str("*last*");
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document.projection().text_tree().byte_len();
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "b", vec![]).unwrap();
    let (prepared, caret) = document
        .prepare_insertion_with_typing_properties(
            FormattedPayloadEdit::new(at..at, payload)
                .with_boundary_affinity(BoundaryAffinity::Upstream),
            &[(
                StyleProperty::CharacterSlant,
                StylePropertyValue::FontSlant(FontSlant::Italic),
            )],
        )
        .unwrap();
    assert_eq!(caret, at + 1);
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(prepared.summary().source_patches()[0].replacement(), b"b");
    assert!(
        prepared.summary().projection_work().projected_hard_lines() <= 2,
        "{:?}",
        prepared.summary().projection_work()
    );
}

#[test]
fn pending_style_insertion_keeps_extended_graphemes_indivisible() {
    let (mut core, view) = fixture(Format::Markdown, "a");
    core.handle(view, key(Key::Char('A'))).unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
    let out = core
        .handle(view, CoreEvent::Input(InputEvent::Text("\u{301}".into())))
        .unwrap();
    assert!(out.document_changed);
    assert_eq!(core.document().text(), "a\u{301}");
    assert!(core
        .document()
        .text_point(core.command_state(view).unwrap().cursor())
        .is_ok());
}
#[test]
fn markdown_typing_uses_canonical_asterisks_and_keeps_nested_intraword_styles() {
    for (source, enable, style) in [
        ("word", SemanticInlineStyle::Emphasis, false),
        ("**word**", SemanticInlineStyle::Emphasis, true),
        ("*word*", SemanticInlineStyle::Strong, true),
    ] {
        for at in [0, 2, 4] {
            let (mut core, view) = fixture(Format::Markdown, source);
            core.handle(view, key(Key::Char('i'))).unwrap();
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: if at == 4 {
                        BoundaryAffinity::Upstream
                    } else {
                        BoundaryAffinity::Downstream
                    },
                    extend_selection: false,
                },
            )
            .unwrap();
            toggle(&mut core, view, enable, true);
            core.handle(view, CoreEvent::Input(InputEvent::Text("X".into())))
                .unwrap();
            let mut expected = "word".to_owned();
            expected.insert(at, 'X');
            assert_eq!(core.document().text(), expected, "{source} at{at}");
            let caret = core.command_state(view).unwrap().cursor();
            let active =
                DocumentLayoutStyles::character_at(core.document().projection(), caret, true)
                    .unwrap();
            assert!(active.slant != viem_core::document::FontSlant::Upright);
            assert_eq!(
                active.bold,
                style,
                "{source} at{at}: {}",
                String::from_utf8_lossy(&core.document().source_bytes())
            );
            assert!(!core.document().source_bytes().contains(&b'_'));
        }
    }
}
