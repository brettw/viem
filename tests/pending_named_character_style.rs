use evim_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use evim_core::command::{CommandInterpreter, InputEvent, Key, LineMode, Mode};
use evim_core::document::{
    Document, Encoding, FontSlant, Format, SemanticInlineStyle, StyleApplication, StyleId,
    StyleNamespace, StyleProperty, StylePropertyValue,
};
use evim_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use evim_core::{Core, CoreEvent, ViewId};
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
    assign(&mut core, view, "Character");
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        Some("Character".into())
    );
    assert_eq!(
        core.selected_typography(view).unwrap().0.slant,
        FontSlant::Upright
    );
    assert_eq!(core.selected_typography(view).unwrap().0.size, 22.);
    text(&mut core, view, "y");
    let style = DocumentLayoutStyles::character_at(core.document().projection(), 1, false).unwrap();
    assert_eq!(style.slant, FontSlant::Upright);
    assert_eq!(style.size, 22.);
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
        Some("Character".into())
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
            Some("Character".into())
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
