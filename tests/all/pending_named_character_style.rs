use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{CommandInterpreter, InputEvent, Key, LineMode, Mode};
use viem_core::document::{
    Document, Encoding, FontSlant, Format, SemanticInlineStyle, StyleApplication, StyleId,
    StyleNamespace, StyleProperty, StylePropertyValue,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};
use std::ops::Range;

type Editor = Core<MockTextMeasurementProvider>;

fn fixture(format: Format, source: &str) -> (Editor, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

fn text(core: &mut Editor, view: ViewId, value: &str) {
    core.handle(view, CoreEvent::Input(InputEvent::text(value)))
        .unwrap_or_else(|error| panic!("{error:?}, format={:?}, mode={:?}, input={value:?}, cursor={}, source={:?}, selected={:?}", core.document().format(), core.command_state(view).unwrap().mode(), core.command_state(view).unwrap().cursor(), String::from_utf8_lossy(&core.document().source_bytes()), core.selected_named_styles(view)));
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

fn assign(core: &mut Editor, view: ViewId, style: &str) {
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: style.into(),
        },
    )
    .unwrap();
}

fn named_at(document: &Document, range: Range<usize>, name: &str) {
    for (offset, _) in document.text()[range.clone()].char_indices() {
        let at = range.start + offset;
        assert!(
            document.projection().style_spans().iter().any(|span| {
                span.range.contains(&at)
                    && (span.application == StyleApplication::Named(name.into())
                        || name == "Code"
                            && document.format() == Format::Markdown
                            && span.application
                                == StyleApplication::Semantic(SemanticInlineStyle::Code))
            }),
            "{name} missing at {at}: {:?}",
            String::from_utf8_lossy(&document.source_bytes())
        );
    }
}

#[test]
fn named_choice_is_pending_in_normal_insert_and_replace_until_text_commits() {
    for (format, source, name) in [
        (Format::Html, "<p data-keep='x'>word</p><!--keep-->", "Code"),
        (Format::Markdown, "word", "Code"),
        (
            Format::Rtf,
            r"{\rtf1{\stylesheet{\*\cs2\i Accent;}}word}",
            "RtfC2",
        ),
    ] {
        for (before, entry) in [
            (None, Some('i')),
            (None, Some('a')),
            (None, Some('R')),
            (Some('i'), None),
            (Some('R'), None),
        ] {
            let (mut core, view) = fixture(format, source);
            if let Some(before) = before {
                key(&mut core, view, Key::Char(before));
            }
            let revision = core.document().revision();
            let history = core.document().history_status();
            let cursor = core.command_state(view).unwrap().cursor();
            let mode = core.command_state(view).unwrap().mode();
            assign(&mut core, view, name);
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status().current, history.current);
            assert_eq!(
                core.document().history_status().node_count,
                history.node_count
            );
            assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
            assert_eq!(core.command_state(view).unwrap().mode(), mode);
            assert_eq!(
                core.selected_named_styles(view).unwrap().character,
                Some(name.into())
            );
            assert!(!core.selected_named_styles(view).unwrap().character_mixed);
            if name == "RtfC2" {
                assert_eq!(
                    core.selected_typography(view).unwrap().0.slant,
                    FontSlant::Italic
                );
            }
            if let Some(entry) = entry {
                key(&mut core, view, Key::Char(entry));
            }
            let at = core.command_state(view).unwrap().cursor();
            text(&mut core, view, "é👩‍💻");
            named_at(core.document(), at..at + "é👩‍💻".len(), name);
            assert_eq!(
                core.selected_named_styles(view).unwrap().character,
                Some(name.into())
            );
            key(&mut core, view, Key::Escape);
            let after = core.document().source_bytes();
            let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(reopened.text(), core.document().text());
            named_at(&reopened, at..at + "é👩‍💻".len(), name);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), after);
        }
    }
}

#[test]
fn percentage_named_style_sizes_pending_typing_from_the_current_paragraph() {
    use viem_core::document::StyleDefinitionFieldEdit;
    for (source, points) in [("# heading", 21.6), ("body", 10.8)] {
        let (mut core, view) = fixture(Format::Markdown, source);
        for (namespace, id, value) in [
            (StyleNamespace::Block, "Paragraph", StylePropertyValue::Float(12.0)),
            (StyleNamespace::Block, "Heading1", StylePropertyValue::Percentage(200)),
            (StyleNamespace::Character, "Code", StylePropertyValue::Percentage(90)),
        ] {
            core.handle(view, CoreEvent::EditGeneratedStyle {
                document: core.document().id(), revision: core.document().revision(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace, style: id.into(),
                edit: StyleDefinitionFieldEdit::SetDeclaration { property: StyleProperty::CharacterSize, value },
            }).unwrap();
        }
        key(&mut core, view, Key::Char('i'));
        assign(&mut core, view, "Code");
        assert!((core.selected_typography(view).unwrap().0.size - points).abs() < 0.0001);
        text(&mut core, view, "new");
        named_at(core.document(), 0..3, "Code");
        assert!((DocumentLayoutStyles::character_at(core.document().projection(), 1, false).unwrap().size - points).abs() < 0.0001);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        key(&mut core, view, Key::Ctrl('r'));
        assert!((DocumentLayoutStyles::character_at(core.document().projection(), 1, false).unwrap().size - points).abs() < 0.0001);
    }
}

#[test]
fn named_identity_and_sparse_direct_overrides_remain_separate() {
    let source = r"{\rtf1{\stylesheet{\*\cs2\i Accent;}}word}";
    let (mut core, view) = fixture(Format::Rtf, source);
    key(&mut core, view, Key::Char('i'));
    assign(&mut core, view, "RtfC2");
    core.handle(
        view,
        CoreEvent::SetDirectCharacterProperties {
            expected: core.list_selection_identity(view).unwrap(),
            values: vec![(StyleProperty::CharacterSize, StylePropertyValue::Float(22.))],
        },
    )
    .unwrap();
    assert_eq!(core.selected_typography(view).unwrap().0.size, 22.);
    text(&mut core, view, "x");
    named_at(core.document(), 0..1, "RtfC2");
    assign(&mut core, view, "");
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        None
    );
    assert_eq!(
        core.selected_typography(view).unwrap().0.slant,
        FontSlant::Upright
    );
    assert_eq!(core.selected_typography(view).unwrap().0.size, 12.);
    text(&mut core, view, "y");
    let style = DocumentLayoutStyles::character_at(core.document().projection(), 1, false).unwrap();
    assert_eq!(style.slant, FontSlant::Upright);
    assert_eq!(style.size, 12.);
}

#[test]
fn failed_named_replace_retains_pending_identity_and_existing_restore_entry() {
    let source = "<p>word</p><!--keep-->";
    let (mut core, view) = fixture(Format::Html, source);
    key(&mut core, view, Key::Char('R'));
    assign(&mut core, view, "Code");
    text(&mut core, view, "x");
    let bytes = core.document().source_bytes();
    let revision = core.document().revision();
    let cursor = core.command_state(view).unwrap().cursor();
    assert!(core
        .handle(view, CoreEvent::Input(InputEvent::text("y\0")))
        .is_err());
    assert_eq!(core.document().source_bytes(), bytes);
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        Some("Code".into())
    );
    key(&mut core, view, Key::Backspace);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Replace);
}

#[test]
fn named_choice_replays_with_dot_and_resets_on_normal_movement() {
    let (mut core, view) = fixture(Format::Html, "<p>word</p>");
    assign(&mut core, view, "Code");
    key(&mut core, view, Key::Char('l'));
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        None
    );
    key(&mut core, view, Key::Char('h'));
    assign(&mut core, view, "Code");
    key(&mut core, view, Key::Char('i'));
    text(&mut core, view, "X");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::DocumentEnd);
    key(&mut core, view, Key::Char('.'));
    assert_eq!(core.document().text(), "XworXd");
    named_at(core.document(), 0..1, "Code");
    named_at(core.document(), 4..5, "Code");
}

#[test]
fn ime_commits_named_identity_and_retains_it_for_subsequent_typing() {
    let source = "<p>word</p>";
    let (mut core, view) = fixture(Format::Html, source);
    key(&mut core, view, Key::Char('i'));
    assign(&mut core, view, "Code");
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
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
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    named_at(core.document(), 0..3, "Code");
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        Some(StyleId::from("Code"))
    );
    let composed = core.document().source_bytes();
    text(&mut core, view, "é");
    named_at(core.document(), 0..5, "Code");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), composed);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn selected_text_assignment_changes_only_the_selection_and_does_not_start_typing() {
    let (mut core, view) = fixture(Format::Html, "<p>word</p>");
    key(&mut core, view, Key::Char('v'));
    key(&mut core, view, Key::Char('l'));
    assign(&mut core, view, "Code");
    named_at(core.document(), 0..2, "Code");
    assert_eq!(core.document().text(), "word");
    assert!(core
        .document()
        .projection()
        .style_spans()
        .iter()
        .all(|span| {
            span.application != StyleApplication::Named("Code".into()) || span.range.end <= 2
        }));
}

#[test]
fn empty_documents_accept_pending_identity_and_invalid_choices_leave_it_unchanged() {
    for format in [Format::Html, Format::Markdown] {
        for entry in ['i', 'R'] {
            let (mut core, view) = fixture(format, "");
            assign(&mut core, view, "Code");
            let history = core.document().history_status();
            let revision = core.document().revision();
            assert!(core
                .handle(
                    view,
                    CoreEvent::AssignNamedStyle {
                        expected: core.list_selection_identity(view).unwrap(),
                        style_sheet_revision: core.document().projection().style_sheet().revision,
                        namespace: StyleNamespace::Character,
                        style: "Missing style".into(),
                    }
                )
                .is_err());
            assert_eq!(
                core.selected_named_styles(view).unwrap().character,
                Some("Code".into())
            );
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status().current, history.current);
            assert!(core.document().source_bytes().is_empty());
            key(&mut core, view, Key::Char(entry));
            text(&mut core, view, "猫");
            named_at(core.document(), 0..3, "Code");
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert!(core.document().source_bytes().is_empty());
        }
    }
}

#[test]
fn clearing_direct_typing_properties_keeps_the_pending_named_identity() {
    let mut document = Document::from_bytes(
        br"{\rtf1{\stylesheet{\*\cs2\i Accent;}}word}".to_vec(),
        Encoding::Utf8,
        Format::Rtf,
    )
    .unwrap();
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('i'))
        .unwrap();
    commands
        .set_typing_named_style(&document, "RtfC2".into())
        .unwrap();
    commands
        .set_typing_properties(
            &document,
            vec![(StyleProperty::CharacterSize, StylePropertyValue::Float(22.))],
        )
        .unwrap();
    commands.clear_typing_properties();
    commands
        .handle(&mut document, InputEvent::text("x"))
        .unwrap();
    named_at(&document, 0..1, "RtfC2");
    let style = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    assert_eq!(style.slant, FontSlant::Italic);
    assert_ne!(style.size, 22.);
}

#[test]
fn vertical_navigation_resets_pending_name_in_both_line_policies() {
    for policy in [LineMode::Visual, LineMode::PhysicalSource] {
        let (mut core, view) = fixture(Format::Html, "<p>word</p>\n<p>tail</p>");
        core.handle(view, CoreEvent::SetLineMode(policy)).unwrap();
        assign(&mut core, view, "Code");
        key(&mut core, view, Key::Char('j'));
        assert_eq!(
            core.selected_named_styles(view).unwrap().character,
            None
        );
        key(&mut core, view, Key::Char('i'));
        text(&mut core, view, "x");
        assert!(core
            .document()
            .projection()
            .style_spans()
            .iter()
            .all(|span| { span.application != StyleApplication::Named("Code".into()) }));
    }
}

#[test]
fn default_paragraph_typing_in_unstyled_html_keeps_ordinary_source() {
    let source = "<p data-keep='x'>word</p><!--keep-->";
    let (mut core, view) = fixture(Format::Html, source);
    key(&mut core, view, Key::Char('i'));
    assign(&mut core, view, "");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    text(&mut core, view, "a");
    text(&mut core, view, "b");
    text(&mut core, view, "c");
    assert_eq!(core.document().source_bytes(), b"<p data-keep='x'>abcword</p><!--keep-->");
    assert_eq!(core.selected_named_styles(view).unwrap().character, None);
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn pending_style_choice_discards_surrounding_traits_before_normal_insert_or_replace() {
    for (format, source, named) in [
        (Format::Html, "<p><span style='font-size:30pt;color:red'><b><sup>word</sup></b></span></p><!--keep-->", "Code"),
        (Format::Rtf, r"{\rtf1{\stylesheet{\*\cs2\i Accent;}}{\fs60\b\super word}{\*\opaque keep}}", "RtfC2"),
        (Format::Markdown, "***word***", "Code"),
    ] {
        for chosen in [named, ""] {
            for entry in ['i', 'a', 'R'] {
                let (mut core, view) = fixture(format, source);
                let expected = core.document().typing_named_style_at(0, viem_core::document::BoundaryAffinity::Downstream, &chosen.into()).unwrap();
                assign(&mut core, view, chosen);
                assert_eq!(core.selected_typography(view).unwrap().0, expected);
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                key(&mut core, view, Key::Char(entry));
                let start = core.command_state(view).unwrap().cursor();
                text(&mut core, view, "XY");
                for at in start..start + 2 {
                    assert_eq!(DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false).unwrap(), expected,
                        "{format:?} {chosen} {entry}: {:?}", String::from_utf8_lossy(&core.document().source_bytes()));
                }
                key(&mut core, view, Key::Escape);
                key(&mut core, view, Key::Char('u'));
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn reselecting_pending_style_resets_new_overrides_and_preserves_named_identity() {
    let (mut core, view) = fixture(Format::Html, "<p><b>word</b></p>");
    key(&mut core, view, Key::Char('i'));
    assign(&mut core, view, "Code");
    core.handle(view, CoreEvent::SetDirectCharacterProperties {
        expected: core.list_selection_identity(view).unwrap(),
        values: vec![(StyleProperty::CharacterSize, StylePropertyValue::Float(22.))],
    }).unwrap();
    assert_eq!(core.selected_typography(view).unwrap().0.size, 22.);
    assign(&mut core, view, "Code");
    assert_eq!(core.selected_typography(view).unwrap().0.size, 14.);
    assert!(!core.selected_typography(view).unwrap().0.bold);
    text(&mut core, view, "X");
    named_at(core.document(), 0..1, "Code");
    let resolved = DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false).unwrap();
    assert_eq!(resolved.size, 14.);
    assert!(!resolved.bold);
}

#[test]
fn select_replacement_keeps_later_direct_edits_after_a_clean_named_choice() {
    use viem_core::command::NavigationKey;
    use viem_core::document::{ParagraphAlignment, ScriptPosition};
    for chosen in ["Code", ""] {
        for operation in 0..5 {
            let source = "<p><span style='color:red;font-size:30pt'><b><sup>word</sup></b></span></p>";
            let (mut core, view) = fixture(Format::Html, source);
            key(&mut core, view, Key::ModifiedNavigation { key: NavigationKey::Right, modifiers: 1 });
            assign(&mut core, view, chosen);
            let expected = core.list_selection_identity(view).unwrap();
            let event = match operation {
                0 => CoreEvent::EditDirectProperty { expected, property: StyleProperty::CharacterSize,
                    value: Some(StylePropertyValue::Float(21.)) },
                1 => CoreEvent::SetDirectCharacterProperties { expected, values: vec![(
                    StyleProperty::CharacterScriptPosition,
                    StylePropertyValue::ScriptPosition(ScriptPosition::Subscript),
                )] },
                2 => CoreEvent::EditDirectProperties { expected, values: vec![
                    (StyleProperty::CharacterSize, Some(StylePropertyValue::Float(22.))),
                    (StyleProperty::ParagraphAlignment, Some(StylePropertyValue::ParagraphAlignment(ParagraphAlignment::Center))),
                ] },
                3 => CoreEvent::SetSelectionSemanticStyle { expected, style: SemanticInlineStyle::Strong, enabled: true },
                _ => {
                    core.handle(view, CoreEvent::EditDirectProperty { expected,
                        property: StyleProperty::CharacterSize, value: Some(StylePropertyValue::Float(23.)),
                    }).unwrap();
                    CoreEvent::EditDirectProperties { expected: core.list_selection_identity(view).unwrap(),
                        values: vec![(StyleProperty::CharacterSize, None)] }
                }
            };
            core.handle(view, event).unwrap();
            let expected_style = DocumentLayoutStyles::semantic_character_at(core.document().projection(), 0, false).unwrap();
            let after_formatting = core.document().source_bytes();
            let revision = core.document().revision();
            assert!(core.handle(view, CoreEvent::SetDirectCharacterProperties {
                expected: core.list_selection_identity(view).unwrap(),
                values: vec![(StyleProperty::CharacterSize, StylePropertyValue::Float(f32::NAN))],
            }).is_err());
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().source_bytes(), after_formatting);
            text(&mut core, view, "XY");
            assert_eq!(core.document().text(), "XYord");
            for at in 0..2 {
                assert_eq!(DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false).unwrap(),
                    expected_style, "choice={chosen}, operation={operation}");
            }
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), after_formatting);
        }
    }
}
