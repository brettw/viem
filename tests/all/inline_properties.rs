use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, Format, ModelRequest, SemanticInlineStyle, StyleProperty,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

fn apply(
    document: &mut Document,
    range: std::ops::Range<usize>,
    property: StyleProperty,
    enabled: bool,
) {
    document
        .apply_model_request(ModelRequest::SetInlineProperty {
            document: document.id(),
            revision: document.revision(),
            range,
            property,
            enabled,
        })
        .unwrap();
}

#[test]
fn underline_and_scripts_preserve_source_locality_and_history_in_both_views() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (property, tag) in [
            (StyleProperty::CharacterUnderline, "ins"),
            (StyleProperty::CharacterSuperscript, "sup"),
            (StyleProperty::CharacterSubscript, "sub"),
        ] {
            let original = b"before\n\nword\n\nafter".to_vec();
            let mut document =
                Document::from_bytes(original.clone(), Encoding::Utf8, format).unwrap();
            let start = document.text().find("word").unwrap();
            apply(&mut document, start..start + 4, property, true);
            let expected = format!("before\n\n<{tag}>word</{tag}>\n\nafter");
            assert_eq!(document.source_bytes(), expected.as_bytes());
            let reloaded =
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let at = reloaded.text().find("word").unwrap();
            let style =
                DocumentLayoutStyles::character_at(reloaded.projection(), at, false).unwrap();
            assert!(match property {
                StyleProperty::CharacterUnderline => style.underline,
                StyleProperty::CharacterSuperscript => style.superscript,
                _ => style.subscript,
            });
            assert!(document.undo());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected.as_bytes());
            let start = document
                .text()
                .find(if format.is_source_view() { "<" } else { "word" })
                .unwrap();
            let end = start
                + if format.is_source_view() {
                    4 + tag.len() * 2 + 5
                } else {
                    4
                };
            apply(&mut document, start..end, property, false);
            assert_eq!(document.source_bytes(), original);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn partial_underline_removal_and_script_switch_preserve_unselected_content() {
    let mut document =
        Document::from_bytes(b"<u>word</u>".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    apply(
        &mut document,
        1..3,
        StyleProperty::CharacterUnderline,
        false,
    );
    assert_eq!(document.source_bytes(), b"<u>w</u>or<u>d</u>");
    let mut document = Document::from_bytes(
        b"<sup>word</sup>".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    apply(&mut document, 1..3, StyleProperty::CharacterSubscript, true);
    assert_eq!(document.text(), "word");
    let styles: Vec<_> = (0..4)
        .map(|at| DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap())
        .collect();
    assert!(styles[0].superscript && styles[3].superscript);
    assert!(styles[1].subscript && styles[2].subscript);
    assert!(!styles[1].superscript && !styles[2].superscript);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"<sup>word</sup>");
}

#[test]
fn native_caret_effects_enter_insert_commit_one_undo_and_reject_literal_owners() {
    for property in [
        StyleProperty::CharacterUnderline,
        StyleProperty::CharacterSuperscript,
        StyleProperty::CharacterSubscript,
    ] {
        let mut core =
            Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetInlineProperty {
                expected,
                property,
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(core.document().source_bytes().is_empty());
        core.handle(view, CoreEvent::Input(InputEvent::Text("word".into())))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        assert!(core.document().text() == "word");
        core.handle(view, CoreEvent::Input(InputEvent::Text("u".into())))
            .unwrap();
        assert!(core.document().source_bytes().is_empty());
        let mut core = Core::new(
            Document::from_bytes(b"```\nword\n```".to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 0,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        assert!(core
            .handle(
                view,
                CoreEvent::SetInlineProperty {
                    expected,
                    property,
                    enabled: true
                }
            )
            .is_err());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.document().source_bytes(), b"```\nword\n```");
    }
}

#[test]
fn gfm_combined_and_nested_emphasis_project_and_native_formatting_adds_each_role() {
    for source in [
        "***foo***",
        "**This text is _extremely_ important**",
        "*This text is **extremely** important*",
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let at = if document.text() == "foo" {
            0
        } else {
            document.text().find("extremely").unwrap()
        };
        let style = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
        assert!(style.bold);
        assert_ne!(style.slant, viem_core::document::FontSlant::Upright);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    for (source, role) in [
        ("*foo*", SemanticInlineStyle::Strong),
        ("**foo**", SemanticInlineStyle::Emphasis),
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let start = document.text().find("foo").unwrap();
            document
                .apply_model_request(ModelRequest::SetSemanticStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: start..start + 3,
                    style: role,
                    enabled: true,
                })
                .unwrap();
            let reopened =
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(reopened.text(), "foo");
            let style =
                DocumentLayoutStyles::character_at(reopened.projection(), 0, false).unwrap();
            assert!(style.bold);
            assert_ne!(style.slant, viem_core::document::FontSlant::Upright);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn scripts_change_shaping_size_and_baseline_and_remain_local_on_large_documents() {
    let source = format!(
        "<sup>up</sup> <sub>down</sub> plain\n\n{}",
        "unrelated\n\n".repeat(10000)
    );
    let mut document =
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let styles = DocumentLayoutStyles::resolve_region(document.projection(), 0..13).unwrap();
    let superscript = styles
        .shaping_runs
        .iter()
        .find(|run| run.text_range.contains(&0))
        .unwrap();
    let subscript = styles
        .shaping_runs
        .iter()
        .find(|run| run.text_range.contains(&3))
        .unwrap();
    assert!(superscript.style.baseline_offset > 0.0);
    assert!(subscript.style.baseline_offset < 0.0);
    assert_eq!(superscript.style.size, subscript.style.size);
    let prepared = document
        .prepare_model_request(ModelRequest::SetInlineProperty {
            document: document.id(),
            revision: document.revision(),
            range: 8..13,
            property: StyleProperty::CharacterUnderline,
            enabled: true,
        })
        .unwrap();
    assert!(
        prepared
            .summary()
            .projection_work()
            .projected_formatted_bytes()
            < 256,
        "{:?}",
        prepared.summary().projection_work()
    );
    document.commit_model_transaction(prepared).unwrap();
}

#[test]
fn nested_inline_effects_keep_inner_treatment_and_clear_every_requested_layer() {
    for source in [
        "<u>A<sup>B</sup>C</u>",
        "<u>A<sup>XBZ</sup>C</u>",
        "<u>A<u>B</u>C</u>",
        "<u>A<del>B</del>C</u>",
        "<u>A<i data-note=\"keep\">B</i>C</u>",
        "<u>A<b>B</b>C</u>",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let at = document.text().find('B').unwrap();
        let before = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
        apply(
            &mut document,
            at..at + 1,
            StyleProperty::CharacterUnderline,
            false,
        );
        let after = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
        assert!(!after.underline, "{source}");
        assert_eq!(after.superscript, before.superscript, "{source}");
        assert_eq!(after.strikethrough, before.strikethrough, "{source}");
        assert_eq!(after.bold, before.bold, "{source}");
        assert_eq!(after.slant, before.slant, "{source}");
        assert_eq!(document.text().as_bytes()[at], b'B');
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let after = DocumentLayoutStyles::character_at(reopened.projection(), at, false).unwrap();
        assert!(!after.underline);
        assert_eq!(after.superscript, before.superscript);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let document = Document::from_bytes(
        b"<sup><sub>foo</sub></sup>".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    let style = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    assert!(style.subscript && !style.superscript);
}

#[test]
fn pending_script_switches_are_mutually_exclusive_and_clear_without_resurrecting_old_choices() {
    for choices in [
        vec![
            (StyleProperty::CharacterSuperscript, true),
            (StyleProperty::CharacterSubscript, true),
            (StyleProperty::CharacterSuperscript, true),
        ],
        vec![
            (StyleProperty::CharacterSuperscript, true),
            (StyleProperty::CharacterSubscript, true),
            (StyleProperty::CharacterSubscript, false),
        ],
    ] {
        let mut core =
            Core::new(Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        for (property, enabled) in choices.clone() {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetInlineProperty {
                    expected,
                    property,
                    enabled,
                },
            )
            .unwrap();
            let style = core.selected_character_style(view).unwrap();
            assert!(!(style.superscript && style.subscript));
        }
        let before = core.selected_character_style(view).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Text("word".into())))
            .unwrap();
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let after = DocumentLayoutStyles::character_at(reopened.projection(), 0, false).unwrap();
        assert_eq!(
            (after.superscript, after.subscript),
            (before.superscript, before.subscript)
        );
    }
}

#[test]
fn cross_paragraph_toggle_state_ignores_structural_boundaries() {
    for (property, tag) in [
        (StyleProperty::CharacterUnderline, "ins"),
        (StyleProperty::CharacterSuperscript, "sup"),
        (StyleProperty::CharacterSubscript, "sub"),
    ] {
        let source = format!("<{tag}>A</{tag}>\n\n<{tag}>B</{tag}>");
        let mut core = Core::new(
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
        core.handle(
            view,
            CoreEvent::SelectAll {
                document: core.document().id(),
                revision: core.document().revision(),
            },
        )
        .unwrap();
        assert_eq!(
            core.selection_inline_property_state(view, property)
                .unwrap(),
            viem_core::SemanticStyleState::On
        );
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetInlineProperty {
                expected,
                property,
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(
            core.selection_inline_property_state(view, property)
                .unwrap(),
            viem_core::SemanticStyleState::Off
        );
    }
}
