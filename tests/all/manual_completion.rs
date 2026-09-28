//! Public-core acceptance tests for the portable completion/menu contract.

use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, FontSlant};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn fixture(document: Document) -> (Editor, ViewId) {
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 180.0);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

fn text(core: &mut Editor, view: ViewId, text: &str) {
    core.handle(view, CoreEvent::Input(InputEvent::text(text)))
        .unwrap();
}

fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: if at == 0 {
                BoundaryAffinity::Downstream
            } else {
                BoundaryAffinity::Upstream
            },
            extend_selection: false,
        },
    )
    .unwrap();
}

fn insert_at(core: &mut Editor, view: ViewId, at: usize) {
    key(core, view, Key::Char('i'));
    place(core, view, at);
}

fn finish_search(core: &mut Editor, view: ViewId) {
    for _ in 0..100_000 {
        if !core
            .completion_presentation(view)
            .unwrap()
            .is_some_and(|menu| menu.searching)
        {
            return;
        }
        core.poll_completion(view).unwrap();
    }
    panic!("completion failed to settle");
}

fn selected(core: &Editor, view: ViewId) -> Option<&str> {
    let menu = core.completion_presentation(view).unwrap()?;
    menu.selected_index.map(|index| menu.items[index].as_str())
}

fn preview(core: &Editor, view: ViewId) -> String {
    core.composition_overlay(view).unwrap().map_or_else(
        || core.document().text().to_owned(),
        |overlay| overlay.text_in_range(0..overlay.utf8_len()).unwrap(),
    )
}

#[test]
fn backend_owns_directional_items_selection_and_original_text_cycle() {
    let source = "alpha alpine alchemy al almanac alpha";
    let at = "alpha alpine alchemy al".len();
    for (control, expected) in [
        ('n', vec!["almanac", "alpha", "alpine", "alchemy"]),
        ('p', vec!["alchemy", "alpine", "alpha", "almanac"]),
    ] {
        let (mut core, view) = fixture(Document::new(source));
        insert_at(&mut core, view, at);
        let history = core.document().history_status();
        key(&mut core, view, Key::Ctrl(control));
        finish_search(&mut core, view);
        let menu = core.completion_presentation(view).unwrap().unwrap();
        assert_eq!(menu.items, expected);
        assert_eq!(menu.selected_index, Some(0));
        assert_eq!(menu.document_id, core.document().id());
        assert_eq!(menu.revision, core.document().revision());
        let session = menu.session_id;
        for (index, word) in expected.iter().enumerate() {
            assert_eq!(selected(&core, view), Some(*word));
            assert_eq!(
                preview(&core, view),
                format!("{}{}{}", &source[..at - 2], word, &source[at..])
            );
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            assert_eq!(core.document().history_status().current, history.current);
            assert_eq!(
                core.completion_presentation(view)
                    .unwrap()
                    .unwrap()
                    .session_id,
                session
            );
            assert_eq!(
                core.completion_presentation(view)
                    .unwrap()
                    .unwrap()
                    .selected_index,
                Some(index)
            );
            key(&mut core, view, Key::Ctrl(control));
        }
        assert_eq!(selected(&core, view), None);
        assert_eq!(preview(&core, view), source);
        key(
            &mut core,
            view,
            Key::Ctrl(if control == 'n' { 'p' } else { 'n' }),
        );
        assert_eq!(selected(&core, view), expected.last().copied());
        key(&mut core, view, Key::Ctrl(control));
        assert_eq!(selected(&core, view), None);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(core.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn ordinary_keys_accept_then_keep_their_editing_effect() {
    for (accept, expected, mode) in [
        (
            InputEvent::Key(Key::Escape),
            "alpha alphasuffix",
            Mode::Normal,
        ),
        (InputEvent::text("!"), "alpha alpha!suffix", Mode::Insert),
        (
            InputEvent::Key(Key::Enter),
            "alpha alpha\nsuffix",
            Mode::Insert,
        ),
        (
            InputEvent::Key(Key::Backspace),
            "alpha alphsuffix",
            Mode::Insert,
        ),
        (
            InputEvent::Key(Key::Left),
            "alpha alphasuffix",
            Mode::Insert,
        ),
    ] {
        let (mut core, view) = fixture(Document::new("alpha alsuffix"));
        insert_at(&mut core, view, 8);
        key(&mut core, view, Key::Ctrl('p'));
        finish_search(&mut core, view);
        assert_eq!(selected(&core, view), Some("alpha"));
        core.handle(view, CoreEvent::Input(accept.clone())).unwrap();
        // Existing text after the caret is retained; only the missing prefix
        // suffix is inserted before the triggering key is interpreted.
        assert_eq!(core.document().text(), expected, "{accept:?}");
        assert_eq!(core.command_state(view).unwrap().mode(), mode);
        assert!(core.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn remaining_keys_match_ordinary_input_after_the_accepted_suffix() {
    for trigger in [
        Key::Tab,
        Key::BackTab,
        Key::Delete,
        Key::Right,
        Key::Up,
        Key::Down,
        Key::Home,
        Key::End,
        Key::Ctrl('w'),
        Key::Ctrl('u'),
        Key::Ctrl('c'),
        Key::Ctrl('o'),
        Key::Ctrl('v'),
        Key::Ctrl('r'),
        Key::Char('x'),
    ] {
        let (mut completed, view) = fixture(Document::new("alpha alsuffix"));
        insert_at(&mut completed, view, 8);
        key(&mut completed, view, Key::Ctrl('p'));
        finish_search(&mut completed, view);
        key(&mut completed, view, trigger);

        let (mut ordinary, baseline) = fixture(Document::new("alpha alsuffix"));
        insert_at(&mut ordinary, baseline, 8);
        text(&mut ordinary, baseline, "pha");
        key(&mut ordinary, baseline, trigger);
        assert_eq!(
            completed.document().source_bytes(),
            ordinary.document().source_bytes(),
            "{trigger:?}"
        );
        assert_eq!(
            completed.command_state(view).unwrap().mode(),
            ordinary.command_state(baseline).unwrap().mode(),
            "{trigger:?}"
        );
        assert_eq!(
            completed.command_state(view).unwrap().cursor(),
            ordinary.command_state(baseline).unwrap().cursor(),
            "{trigger:?}"
        );
        assert!(completed.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn literal_next_retains_control_n_ownership_and_replace_mode_does_not_start_completion() {
    let (mut core, view) = fixture(Document::new("alpha al"));
    insert_at(&mut core, view, 8);
    key(&mut core, view, Key::Ctrl('v'));
    key(&mut core, view, Key::Ctrl('n'));
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert_eq!(core.document().text(), "alpha al\u{e}");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('R'));
    key(&mut core, view, Key::Ctrl('p'));
    assert!(core.completion_presentation(view).unwrap().is_none());
}

#[test]
fn failing_accepting_key_reports_the_successful_completion_and_preserves_undo() {
    let source = b"alphabet al";
    let document =
        Document::from_bytes(source.to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
    let (mut core, view) = fixture(document);
    insert_at(&mut core, view, source.len());
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    let before = core.document().revision();
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::text("😀")))
        .unwrap();
    assert!(outcome.document_changed);
    let map = outcome.position_map.unwrap();
    assert_eq!(map.source_revision(), before);
    assert_eq!(map.target_revision(), core.document().revision());
    assert!(matches!(
        outcome.command.unwrap().status,
        CommandStatus::Error(_)
    ));
    assert_eq!(core.document().source_bytes(), b"alphabet alphabet");
    assert!(core.completion_presentation(view).unwrap().is_none());
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source);
}

#[test]
fn layout_dependent_accepting_keys_keep_the_completion_map_and_normal_motion() {
    let source = format!("al alphabet\n{}tail", "later line\n".repeat(10_000));
    for trigger in [Key::PageDown, Key::DocumentEnd] {
        let (mut core, view) = fixture(Document::new(source.clone()));
        insert_at(&mut core, view, 2);
        key(&mut core, view, Key::Ctrl('n'));
        assert_eq!(selected(&core, view), Some("alphabet"));
        let before = core.document().revision();
        let outcome = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(trigger)))
            .unwrap();
        assert!(outcome.document_changed, "{trigger:?}");
        let map = outcome.position_map.unwrap();
        assert_eq!(map.source_revision(), before);
        assert_eq!(map.target_revision(), core.document().revision());
        assert!(matches!(
            outcome.command.unwrap().status,
            CommandStatus::Complete
        ));
        assert_eq!(core.document().text(), format!("alphabet{}", &source[2..]));
        let cursor = core.command_state(view).unwrap().cursor();
        assert!(
            cursor > "alphabet".len(),
            "{trigger:?} should perform its motion after accepting"
        );
        if trigger == Key::DocumentEnd {
            assert_eq!(cursor, core.document().text().len());
        }
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(core.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn control_y_and_e_accept_then_copy_from_the_adjacent_line() {
    for (control, expected) in [('y', "alphabet8"), ('e', "alphabeti")] {
        let (mut core, view) = fixture(Document::new("0123456789\nal\nabcdefghij\nalphabet"));
        insert_at(&mut core, view, 13);
        key(&mut core, view, Key::Ctrl('n'));
        finish_search(&mut core, view);
        assert_eq!(selected(&core, view), Some("alphabet"));
        key(&mut core, view, Key::Ctrl(control));
        assert_eq!(core.document().text().lines().nth(1), Some(expected));
        assert!(core.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn accepted_completion_joins_insert_undo_and_last_insert_register() {
    let original = "alphabet\n";
    let (mut core, view) = fixture(Document::new(original));
    let unnamed = core.command_state(view).unwrap().register('"').cloned();
    insert_at(&mut core, view, original.len());
    text(&mut core, view, "al");
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    text(&mut core, view, "!");
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "alphabet\nalphabet!");
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('.')
            .unwrap()
            .text,
        "alphabet!"
    );
    assert_eq!(
        core.command_state(view).unwrap().register('"'),
        unnamed.as_ref()
    );
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().text(), "alphabet\nalphabet!");
}

#[test]
fn change_operator_completion_undoes_as_one_change_and_preserves_delete_register() {
    let original = "alphabet\nword";
    let (mut core, view) = fixture(Document::new(original));
    place(&mut core, view, 9);
    for ch in "cw".chars() {
        key(&mut core, view, Key::Char(ch));
    }
    text(&mut core, view, "al");
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "alphabet\nalphabet");
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('"')
            .unwrap()
            .text,
        "word"
    );
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn counts_dot_and_macro_replay_use_the_accepted_payload() {
    let (mut core, view) = fixture(Document::new("alphabet\n"));
    place(&mut core, view, 9);
    for ch in "3i".chars() {
        key(&mut core, view, Key::Char(ch));
    }
    text(&mut core, view, "al");
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "alphabet\nalphabetalphabetalphabet");
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().text(), "alphabet\n");

    for macro_replay in [false, true] {
        let (mut core, view) = fixture(Document::new("alphabet\n\n"));
        place(&mut core, view, 9);
        if macro_replay {
            for ch in "qa".chars() {
                key(&mut core, view, Key::Char(ch));
            }
        }
        key(&mut core, view, Key::Char('i'));
        text(&mut core, view, "al");
        key(&mut core, view, Key::Ctrl('p'));
        finish_search(&mut core, view);
        key(&mut core, view, Key::Escape);
        if macro_replay {
            key(&mut core, view, Key::Char('q'));
        }
        key(&mut core, view, Key::Char('G'));
        for ch in if macro_replay { "@a" } else { "." }.chars() {
            key(&mut core, view, Key::Char(ch));
        }
        assert_eq!(
            core.document().text(),
            "alphabet\nalphabet\nalphabet",
            "macro={macro_replay}"
        );
        assert!(core.completion_presentation(view).unwrap().is_none());
    }
}

#[test]
fn rich_previews_preserve_source_and_acceptance_inherits_destination_style() {
    for (format, source) in [
        (Format::Markdown, "alphabet\n\n*al*"),

    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let at = document.text().len();
        let original_text = document.text().to_owned();
        let (mut core, view) = fixture(document);
        insert_at(&mut core, view, at);
        key(&mut core, view, Key::Ctrl('p'));
        finish_search(&mut core, view);
        assert_eq!(selected(&core, view), Some("alphabet"), "{format:?}");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(preview(&core, view), format!("{original_text}phabet"));
        text(&mut core, view, "!");
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), format!("{original_text}phabet!"));
        for offset in at - 2..at + "phabet!".len() {
            assert_ne!(
                DocumentLayoutStyles::character_at(core.document().projection(), offset, false)
                    .unwrap()
                    .slant,
                FontSlant::Upright,
                "{format:?} style at {offset}"
            );
        }

        key(&mut core, view, Key::Char('u'));
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}"
        );
    }
}

#[test]
fn completion_works_in_literal_formats_and_keeps_original_encoding() {
    for format in [
        Format::PlainText,
        Format::Code,
        Format::MarkdownSource,
    ] {
        let (mut core, view) =
            fixture(Document::from_bytes(b"alphabet al".to_vec(), Encoding::Utf8, format).unwrap());
        insert_at(&mut core, view, 11);
        key(&mut core, view, Key::Ctrl('p'));
        finish_search(&mut core, view);
        key(&mut core, view, Key::Escape);
        assert_eq!(
            core.document().source_bytes(),
            b"alphabet alphabet",
            "{format:?}"
        );
    }
    let source = b"caf\xe9 ca";
    let (mut core, view) = fixture(
        Document::from_bytes(source.to_vec(), Encoding::Latin1, Format::PlainText).unwrap(),
    );
    insert_at(&mut core, view, 8);
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    assert_eq!(core.document().source_bytes(), source);
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().source_bytes(), b"caf\xe9 caf\xe9");
}

#[test]
fn another_views_edit_retires_results_and_a_new_session_has_a_new_identity() {
    let (mut core, view) = fixture(Document::new("alphabet al"));
    let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 180.0);
    insert_at(&mut core, view, 11);
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    let session = core
        .completion_presentation(view)
        .unwrap()
        .unwrap()
        .session_id;
    key(&mut core, second, Key::Char('i'));
    text(&mut core, second, "!");
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert!(!core.poll_completion(view).unwrap());
    assert!(core.composition_overlay(view).unwrap().is_none());
    key(&mut core, view, Key::Ctrl('p'));
    finish_search(&mut core, view);
    assert_ne!(
        core.completion_presentation(view)
            .unwrap()
            .unwrap()
            .session_id,
        session
    );
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "!alphabet alphabet");
}

#[test]
fn large_document_search_yields_and_dismissed_work_never_reopens_the_popup() {
    let original = format!("al {} alphabet", "x ".repeat(100_000));
    let (mut core, view) = fixture(Document::new(original.clone()));
    insert_at(&mut core, view, 2);
    key(&mut core, view, Key::Ctrl('n'));
    for _ in 0..3 {
        let menu = core.completion_presentation(view).unwrap().unwrap();
        assert!(menu.searching);
        assert!(menu.items.is_empty());
        assert_eq!(menu.selected_index, None);
        core.poll_completion(view).unwrap();
    }
    key(&mut core, view, Key::Escape);
    assert!(!core.poll_completion(view).unwrap());
    assert!(core.completion_presentation(view).unwrap().is_none());
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}

#[test]
fn preview_layout_invalidates_its_carets_and_preserves_other_views_and_large_document_caches() {
    let original = format!(
        "al alphabet alphanumeric\n{}tail",
        "unchanged line\n".repeat(100_000)
    );
    let mut core = Core::new(Document::new(original.clone()));
    let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 120.0);
    let other = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    insert_at(&mut core, view, 2);
    let base_revision = core.layout(view).unwrap().snapshot().unwrap().revision;
    let other_revision = core.layout(other).unwrap().snapshot().unwrap().revision;
    key(&mut core, view, Key::Ctrl('n'));
    assert_eq!(selected(&core, view), Some("alphabet"));
    let first = core.presentation_layout(view).unwrap().snapshot().unwrap();
    assert!(!first.coverage.is_full_document());
    assert!(first.coverage.hard_lines().end < 100);
    let first_revision = first.revision;
    let first_caret = first.caret_point(8, BoundaryAffinity::Upstream).unwrap();
    key(&mut core, view, Key::Ctrl('n'));
    assert_eq!(selected(&core, view), Some("alphanumeric"));
    let next = core.presentation_layout(view).unwrap().snapshot().unwrap();
    assert_ne!(next.revision, first_revision);
    assert!(next.caret_geometry(first_caret).is_err());
    assert!(!next.coverage.is_full_document());
    assert!(next.coverage.hard_lines().end < 100);
    assert_eq!(
        core.layout(view).unwrap().snapshot().unwrap().revision,
        base_revision
    );
    assert_eq!(
        core.layout(other).unwrap().snapshot().unwrap().revision,
        other_revision
    );
    key(&mut core, view, Key::Ctrl('p'));
    key(&mut core, view, Key::Ctrl('p'));
    assert_eq!(selected(&core, view), None);
    assert!(core.composition_overlay(view).unwrap().is_none());
    assert_eq!(
        core.presentation_layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .revision,
        base_revision
    );
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().source_bytes(), original.as_bytes());
}
