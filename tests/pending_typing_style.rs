use evim_core::command::{InputEvent, Key};
use evim_core::document::{
    BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest, SemanticInlineStyle,
};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent};
fn key(key: Key) -> CoreEvent {
    CoreEvent::Input(InputEvent::Key(key))
}
fn fixture(format: Format, source: &str) -> (Core<MockTextMeasurementProvider>, evim_core::ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    (core, view)
}
fn toggle(
    core: &mut Core<MockTextMeasurementProvider>,
    view: evim_core::ViewId,
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
        (Format::Html, "<p>word</p><!--keep-->", 0),
        (Format::HtmlSource, "<p>word</p><!--keep-->", 3),
        (Format::Rtf, "{\\rtf1 word}{\\*\\unknown keep}", 0),
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
            core.selected_typography(view).unwrap().0.slant
                != evim_core::document::FontSlant::Upright
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
                != evim_core::document::FontSlant::Upright,
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
    assert!(!core.selected_typography(view).unwrap().0.bold);
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
    assert!(!core.selected_typography(view).unwrap().0.bold);
}
#[test]
fn typing_can_disable_existing_style_and_continue_inherited_run() {
    for (format, source, at) in [
        (Format::Markdown, "**word**", 2),
        (Format::MarkdownSource, "**word**", 4),
        (Format::Html, "<p><b>word</b></p>", 2),
        (Format::Rtf, "{\\rtf1 {\\b word}}", 2),
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
        Format::Html,
        Format::Rtf,
    ] {
        let source = match format {
            Format::Html => "<p>x</p>",
            Format::Rtf => "{\\rtf1 x}",
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
            style.bold && style.slant != evim_core::document::FontSlant::Upright,
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
fn unsupported_pending_properties_and_stale_targets_leave_state_unchanged() {
    use evim_core::document::{StyleProperty, StylePropertyValue};
    let (mut core, view) = fixture(Format::Markdown, "word");
    core.handle(view, key(Key::Char('i'))).unwrap();
    let stale = core.list_selection_identity(view).unwrap();
    assert!(core
        .handle(
            view,
            CoreEvent::EditDirectProperty {
                expected: stale.clone(),
                property: StyleProperty::CharacterSize,
                value: Some(StylePropertyValue::Float(18.))
            }
        )
        .is_err());
    core.handle(view, key(Key::Right)).unwrap();
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
    assert!(!core.selected_typography(view).unwrap().0.bold);
    let (mut core, view) = fixture(Format::HtmlSource, "<p title='keep'>word</p>");
    core.handle(view, key(Key::Char('i'))).unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 6,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    assert!(core
        .handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style: SemanticInlineStyle::Emphasis,
                enabled: true
            }
        )
        .is_err());
    assert!(!core
        .selection_semantic_style_presentation(view, SemanticInlineStyle::Emphasis)
        .unwrap()
        .can_set());
}
#[test]
fn pending_state_is_view_local_and_external_reprojection_cancels_it() {
    let (mut core, view) = fixture(Format::Html, "<p>word</p>");
    let other = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
    core.handle(view, key(Key::Char('i'))).unwrap();
    toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
    assert_eq!(
        core.selected_typography(other).unwrap().0.slant,
        evim_core::document::FontSlant::Upright
    );
    core.handle(other, key(Key::Char('i'))).unwrap();
    core.handle(other, CoreEvent::Input(InputEvent::Text("a".into())))
        .unwrap();
    assert_eq!(
        core.selected_typography(view).unwrap().0.slant,
        evim_core::document::FontSlant::Upright
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
        assert_eq!(core.document().source_bytes(), b" x");
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
    use evim_core::document::{
        FontSlant, FormattedPayloadEdit, FormattedTextPayload, StyleProperty, StylePropertyValue,
    };
    let mut source = "<p>line</p>".repeat(10_000);
    source.push_str("<p><i>last</i></p><!--keep-->");
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
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
fn direct_typing_properties_are_atomic_and_visible_in_pending_presentation() {
    use evim_core::document::{Color, StyleProperty as P, StylePropertyValue as V};
    for (format, source) in [
        (Format::Html, "<p>word</p>"),
        (Format::HtmlSource, "word"),
        (Format::Rtf, "{\\rtf1 word}"),
    ] {
        let (mut core, view) = fixture(format, source);
        core.handle(view, key(Key::Char('i'))).unwrap();
        let values = vec![
            (P::CharacterUnderline, V::Boolean(true)),
            (P::CharacterSize, V::Float(18.)),
            (
                P::CharacterForeground,
                V::Color(Color {
                    red: 1.,
                    green: 0.,
                    blue: 0.,
                    alpha: 1.,
                }),
            ),
        ];
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetDirectCharacterProperties { expected, values },
        )
        .unwrap();
        let pending = core.selected_typography(view).unwrap().0;
        assert!(pending.underline);
        assert_eq!(pending.size, 18.);
        assert_eq!(pending.foreground.red, 1.);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Text("é".into())))
            .unwrap();
        let caret = core.command_state(view).unwrap().cursor();
        let style =
            DocumentLayoutStyles::semantic_character_at(core.document().projection(), caret, true)
                .unwrap();
        assert!(style.underline);
        assert_eq!(style.size, 18.);
        assert_eq!(style.foreground.red, 1.);
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
fn pending_style_insertion_keeps_extended_graphemes_indivisible() {
    let (mut core, view) = fixture(Format::Html, "<p>a</p>");
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
            assert!(active.slant != evim_core::document::FontSlant::Upright);
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
#[test]
fn rtf_scalar_typing_keeps_group_depth_bounded_and_undo_exact() {
    for (source, count) in [
        ("{\\rtf1}", 1000),
        ("{\\rtf1{\\b}}", 16),
        ("{\\rtf1\\uc0 word}", 16),
    ] {
        let (mut core, view) = fixture(Format::Rtf, source);
        core.handle(view, key(Key::Char('i'))).unwrap();
        toggle(&mut core, view, SemanticInlineStyle::Emphasis, true);
        let original = core.document().text().to_owned();
        for _ in 0..count {
            for text in ["a", "é"] {
                core.handle(view, CoreEvent::Input(InputEvent::Text(text.into())))
                    .unwrap();
            }
        }
        assert_eq!(
            core.document().text(),
            format!("{}{original}", "aé".repeat(count))
        );
        let caret = core.command_state(view).unwrap().cursor();
        let active =
            DocumentLayoutStyles::character_at(core.document().projection(), caret, true).unwrap();
        assert_ne!(active.slant, evim_core::document::FontSlant::Upright);
        let serialized = String::from_utf8(core.document().source_bytes()).unwrap();
        let mut depth = 0;
        let mut max = 0;
        for ch in serialized.chars() {
            if ch == '{' {
                depth += 1;
                max = max.max(depth);
            } else if ch == '}' {
                depth -= 1;
            }
        }
        assert!(max <= 5, "depth{max}: {serialized}");
        core.handle(view, key(Key::Escape)).unwrap();
        core.handle(
            view,
            CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
