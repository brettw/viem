use viem_core::command::{CommandInterpreter, InputEvent, Key, NavigationKey};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format, StyleNamespace, StyleProperty, StylePropertyValue, TextEdit};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn assert_saved(document: &Document, source: &str) {
    assert_eq!(document.source_bytes(), source.as_bytes());
    let reopened = html(source);
    assert_eq!(reopened.text(), document.text());
    assert_eq!(reopened.projection().blocks().len(), document.projection().blocks().len());
}

#[test]
fn deleting_the_last_character_removes_nested_character_scopes_in_the_same_undo_step() {
    for (before, after) in [
        ("<p><b>x</b></p>", "<p></p>"),
        ("<p><span class='Code'><b><i>x</i></b></span></p>", "<p></p>"),
        ("<p><span data-viem-character='none'>x</span></p>", "<p></p>"),
        ("<p><a href='https://example.test/'><sup>x</sup></a></p><!--tail-->", "<p></p><!--tail-->"),
        ("<p><b><!--keep-->x<!--also--></b></p>", "<p><!--keep--><!--also--></p>"),
        ("<p><b>x</b><i></i>y</p>", "<p><i></i>y</p>"),
    ] {
        let mut document = html(before);
        document.apply_edits(vec![TextEdit::new(0..1, "")]).unwrap();
        assert_saved(&document, after);
        assert!(document.undo());
        assert_saved(&document, before);
        assert!(document.redo());
        assert_saved(&document, after);
    }
}

#[test]
fn empty_authored_siblings_and_metadata_are_not_cleanup_targets() {
    for (before, after) in [
        ("<p><b></b>x<span style='color:red'></span></p>", "<p><b></b><span style='color:red'></span></p>"),
        ("<p><b data-id='keep'>x</b></p>", "<p><b data-id='keep'></b></p>"),
        ("<p><a id='bookmark'>x</a></p>", "<p><a id='bookmark'></a></p>"),
        ("<p><custom>x</custom></p>", "<p><custom></custom></p>"),
        ("<p><b>x<script>keep()</script></b></p>", "<p><b><script>keep()</script></b></p>"),
    ] {
        let mut document = html(before);
        document.apply_edits(vec![TextEdit::new(0..1, "")]).unwrap();
        assert_saved(&document, after);
    }
}

#[test]
fn partial_deletion_and_replacement_preserve_character_context() {
    for (range, replacement, after) in [
        (0..1, "", "<p><b>y</b></p>"),
        (0..2, "z", "<p><b>z</b></p>"),
    ] {
        let mut document = html("<p><b>xy</b></p>");
        document.apply_edits(vec![TextEdit::new(range, replacement)]).unwrap();
        assert_saved(&document, after);
        assert!(DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap().bold);
    }
}

#[test]
fn discontiguous_deletions_cleanup_only_scopes_without_retained_text() {
    let mut document = html("<p><b>x</b>a<i>y</i>b<u>cd</u></p>");
    document.apply_edits(vec![TextEdit::new(0..1, ""), TextEdit::new(2..3, ""), TextEdit::new(4..5, "")]).unwrap();
    assert_saved(&document, "<p>ab<u>d</u></p>");
}

#[test]
fn source_view_deletion_does_not_cleanup_markup() {
    let before = "<p><b>x</b></p>";
    let mut document = Document::from_bytes(before.as_bytes().to_vec(), Encoding::Utf8, Format::HtmlSource).unwrap();
    document.apply_edits(vec![TextEdit::new(6..7, "")]).unwrap();
    assert_eq!(document.source_bytes(), b"<p><b></b></p>");
}

#[test]
fn cleanup_respects_utf16_source_coordinates() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |source: &str| -> Vec<u8> {
            source.encode_utf16().flat_map(|word| if encoding == Encoding::Utf16Le { word.to_le_bytes() } else { word.to_be_bytes() }).collect()
        };
        let before = encode("<p><b>猫</b></p><!--keep-->");
        let after = encode("<p></p><!--keep-->");
        let mut document = Document::from_bytes(before.clone(), encoding, Format::Html).unwrap();
        document.apply_edits(vec![TextEdit::new(0..3, "")]).unwrap();
        assert_eq!(document.source_bytes(), after);
        assert_eq!(Document::from_bytes(after.clone(), encoding, Format::Html).unwrap().text(), "");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), after);
    }
}

#[test]
fn command_delete_backspace_and_native_selections_cleanup_consumed_styles() {
    for path in 0..4 {
        let mut document = html("<p><b>x</b>y</p>");
        let mut commands = CommandInterpreter::new();
        match path {
            0 => { commands.handle(&mut document, InputEvent::key('x')).unwrap(); }
            1 => {
                commands.handle(&mut document, InputEvent::key('a')).unwrap();
                commands.handle(&mut document, InputEvent::Key(Key::Backspace)).unwrap();
            }
            _ => {
                commands.handle(&mut document, InputEvent::key('i')).unwrap();
                let (start, end) = if path == 2 { (0, 1) } else { (1, 0) };
                assert!(commands.set_cursor_from_pointer(&document, start, BoundaryAffinity::Downstream, false));
                assert!(commands.set_cursor_from_pointer(&document, end, BoundaryAffinity::Downstream, true));
                commands.handle(&mut document, InputEvent::Key(Key::Backspace)).unwrap();
            }
        }
        assert_saved(&document, "<p>y</p>");
    }
}

#[test]
fn abandoned_pending_bold_and_named_choices_never_create_source_or_history() {
    for named in [false, true] {
        for movement in [Key::Right, Key::ModifiedNavigation { key: NavigationKey::Right, modifiers: 1 }] {
            let before = "<p>ab</p>";
            let mut core = Core::new(html(before));
            let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
            core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            let revision = core.document().revision();
            let history = core.document().history_status();
            let expected = core.list_selection_identity(view).unwrap();
            let choice = if named {
                CoreEvent::AssignNamedStyle { expected,
                    style_sheet_revision: core.document().projection().style_sheet().revision,
                    namespace: StyleNamespace::Character, style: "Code".into() }
            } else {
                CoreEvent::SetDirectCharacterProperties { expected,
                    values: vec![(StyleProperty::CharacterBold, StylePropertyValue::Boolean(true))] }
            };
            core.handle(view, choice).unwrap();
            assert_saved(core.document(), before);
            core.handle(view, CoreEvent::Input(InputEvent::Key(movement))).unwrap();
            assert_saved(core.document(), before);
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status().node_count, history.node_count);
            assert_eq!(core.document().history_status().current, history.current);
            assert_eq!(core.selected_named_styles(view).unwrap().character, None);
            assert!(!core.selected_typography(view).unwrap().0.bold);
            core.handle(view, CoreEvent::Input(InputEvent::text("z"))).unwrap();
            let at = core.command_state(view).unwrap().cursor().saturating_sub(1);
            assert!(!DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false).unwrap().bold);
        }
    }
}

#[test]
fn disabling_a_style_at_its_start_does_not_create_an_empty_split_scope() {
    for (before, after) in [
        ("<p><b>xy</b></p>", "<p>z<b>xy</b></p>"),
        ("<p><i><b>xy</b></i></p>", "<p><i>z<b>xy</b></i></p>"),
        ("<p><b><i>xy</i></b></p>", "<p><i>z</i><b><i>xy</i></b></p>"),
    ] {
        let mut document = html(before);
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        commands.set_typing_properties(&document, vec![(StyleProperty::CharacterBold, StylePropertyValue::Boolean(false))]).unwrap();
        assert_saved(&document, before);
        commands.handle(&mut document, InputEvent::text("z")).unwrap();
        assert_saved(&document, after);
        assert!(!DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap().bold);
        assert!(DocumentLayoutStyles::semantic_character_at(document.projection(), 1, false).unwrap().bold);
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_saved(&document, before);
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_saved(&document, after);
    }
}

#[test]
fn deleting_newly_typed_bold_removes_its_scope_while_the_pending_choice_stays_at_the_caret() {
    let mut document = html("<p>ab</p>");
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut document, InputEvent::key('i')).unwrap();
    commands.set_typing_properties(&document, vec![(StyleProperty::CharacterBold, StylePropertyValue::Boolean(true))]).unwrap();
    commands.handle(&mut document, InputEvent::text("x")).unwrap();
    commands.handle(&mut document, InputEvent::Key(Key::Backspace)).unwrap();
    assert_saved(&document, "<p>ab</p>");
    commands.handle(&mut document, InputEvent::text("y")).unwrap();
    assert!(DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap().bold);
    commands.handle(&mut document, InputEvent::Key(Key::Backspace)).unwrap();
    assert!(commands.set_cursor_from_pointer(&document, 1, BoundaryAffinity::Downstream, false));
    commands.handle(&mut document, InputEvent::text("z")).unwrap();
    assert_saved(&document, "<p>azb</p>");
}
