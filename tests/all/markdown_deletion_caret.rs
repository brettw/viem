//! The deletion target is a surviving character, not a screen-row number.
//! Reflow may move it upward. Only ambiguous wrap placement prefers the next
//! row; code/quote containment is a separate exception. See the matching
//! contract in docs/markdown-feature-contracts.md before changing expectations.
use viem_core::command::{InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

fn open(source: &str, width: f32) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 1200.);
    (core, view)
}

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, keys: &str) {
    for key in keys.chars() {
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::key(key)))
            .unwrap();
        assert!(
            output.command.as_ref().is_none_or(|command| matches!(
                command.status,
                viem_core::command::CommandStatus::Complete
                    | viem_core::command::CommandStatus::Pending
            )),
            "{output:?}"
        );
    }
}

fn place(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    at: usize,
    affinity: BoundaryAffinity,
) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity,
            extend_selection: false,
        },
    )
    .unwrap();
}

#[test]
fn line_deletion_stays_in_surviving_code_and_quote_owners() {
    for source in [
        "```rust\nfirst\nsecond\nlast\n```\n\nFollowing",
        "~~~~\nfirst\nsecond\nlast\n~~~~\n\nFollowing",
        "    first\n    second\n    last\n\nFollowing",
        "<pre><code>first\nsecond\nlast</code></pre>\n\nFollowing",
        "---\nfirst\nsecond\nlast\n---\n\nFollowing",
        "> first\n>\n> second\n>\n> last\n\nFollowing",
        "> ```\n> first\n> second\n> last\n> ```\n\nFollowing",
        "- intro\n\n  ```\n  first\n  second\n  last\n  ```\n\nFollowing",
    ] {
        for command in [
            "dd",
            "dk",
            "d-up",
            "Vd",
            "Vkd",
            "native-delete",
            "native-backspace",
            "gutter-delete",
            "gutter-cut",
        ] {
            let (mut core, view) = open(source, 600.);
            let at = core.document().text().find("last").unwrap();
            place(&mut core, view, at, BoundaryAffinity::Downstream);
            if command == "d-up" {
                keys(&mut core, view, "d");
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Up)))
                    .unwrap();
            } else if command.starts_with("gutter") {
                let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                let row = snapshot
                    .rows
                    .iter()
                    .find(|row| row.text_range.start == at)
                    .unwrap();
                let event = CoreEvent::SelectPointerRow {
                    document_revision: core.document().revision(),
                    layout_revision: snapshot.revision,
                    y: row.y + row.height() / 2.,
                    extend_selection: false,
                };
                core.handle(view, event).unwrap();
                if command == "gutter-cut" {
                    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardTarget};
                    for key in "\"+d".chars() {
                        core.handle(
                            view,
                            CoreEvent::InputWithClipboard {
                                input: InputEvent::key(key),
                                clipboard: ClipboardCommandContext::new()
                                    .with_write(ClipboardTarget::Clipboard),
                            },
                        )
                        .unwrap();
                    }
                } else {
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Delete)))
                        .unwrap();
                }
            } else if command.starts_with("native") {
                keys(&mut core, view, "V");
                core.set_selection_origin(view, SelectionOrigin::Mouse, Mode::Normal)
                    .unwrap();
                core.handle(
                    view,
                    CoreEvent::Input(InputEvent::Key(if command == "native-delete" {
                        Key::Delete
                    } else {
                        Key::Backspace
                    })),
                )
                .unwrap();
            } else {
                keys(&mut core, view, command);
            }
            let retained = if matches!(command, "dk" | "d-up" | "Vkd") {
                "first"
            } else {
                "second"
            };
            let expected = core.document().text().find(retained).unwrap() + retained.len() - 1;
            let state = core.command_state(view).unwrap();
            assert_eq!(
                state.cursor(),
                expected,
                "{source:?} {command}: {:?}",
                core.document().text()
            );
            assert_eq!(state.boundary_affinity(), BoundaryAffinity::Downstream);
            // History must restore the recorded caret, not reapply containment.
            let after = core.document().source_bytes();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            let recorded = core.command_state(view).unwrap().cursor();
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                .unwrap();
            assert_eq!(core.document().source_bytes(), after);
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                recorded,
                "history: {source:?} {command}"
            );
        }
    }
}

#[test]
fn explicit_crossing_whole_owners_and_lists_keep_forward_recovery() {
    // Counts and selections can explicitly include outside rows. Containment
    // must not turn a single 2dd into two independent, backward-stepping dd's.
    for command in ["dj", "2dd", "Vjd"] {
        let (mut core, view) = open("```\nfirst\nlast\n```\n\nFollowing\n\nAfter", 600.);
        let at = core.document().text().find("last").unwrap();
        place(&mut core, view, at, BoundaryAffinity::Downstream);
        keys(&mut core, view, command);
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            core.document().text().find("After").unwrap(),
            "{command}"
        );
    }
    for source in [
        "```\nlast\n```\n\nFollowing",
        "> last\n\nFollowing",
        "- first\n- last\n- Following",
    ] {
        let (mut core, view) = open(source, 600.);
        let at = core.document().text().find("last").unwrap();
        place(&mut core, view, at, BoundaryAffinity::Downstream);
        keys(&mut core, view, "dd");
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            core.document().text().find("Following").unwrap(),
            "{source}"
        );
    }
}

#[test]
fn repeated_deletion_walks_back_inside_but_can_remove_the_whole_owner() {
    for command in ["dd.", "qaddq@a"] {
        let (mut core, view) = open("```\nfirst\nsecond\nlast\n```\n\nFollowing", 600.);
        let at = core.document().text().find("last").unwrap();
        place(&mut core, view, at, BoundaryAffinity::Downstream);
        keys(&mut core, view, command);
        assert_eq!(core.document().text(), "first\nFollowing");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, ".");
        assert_eq!(core.document().text(), "Following");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    }
}

#[test]
fn wrapped_prose_follows_the_next_character_with_downstream_affinity() {
    // Starting on $ intentionally supplies upstream affinity. Deleting the
    // row must discard it. Shorter following words can reflow into the old
    // preceding row: moving to a later row would skip surviving characters.
    let mut moved_up = false;
    let mut shared_boundary = false;
    for source in [
        "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa",
        "longword longword longword longword a b c d e f g h i j k l m n o p q r s t u v w x y z",
        "aa bbbbbbbbbbbbbbbbbbbbbbbb x y z alpha bravo charlie delta echo foxtrot golf hotel india juliet",
    ] {
        for width in [100., 140., 180.] {
            let (mut core, view) = open(source, width);
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot.rows.len() >= 4);
            let row = snapshot.rows[1].text_range.clone();
            let expected_text = format!("{}{}", &source[..row.start], &source[row.end..]);
            let next = source[row.end..].chars().next().unwrap();
            place(&mut core, view, row.start, BoundaryAffinity::Downstream);
            keys(&mut core, view, "$");
            assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Upstream);
            keys(&mut core, view, "dd");
            assert_eq!(core.document().text(), expected_text);
            let state = core.command_state(view).unwrap();
            assert_eq!(state.cursor(), row.start);
            assert_eq!(state.boundary_affinity(), BoundaryAffinity::Downstream);
            assert_eq!(core.document().text()[state.cursor()..].chars().next(), Some(next));
            let at = state.cursor();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            moved_up |= snapshot.rows[0].text_range.contains(&at);
            shared_boundary |= snapshot.rows.iter().skip(1).any(|row| row.text_range.start == at);
        }
    }
    assert!(
        moved_up,
        "fixture must exercise real upward reflow, not only affinity"
    );
    assert!(
        shared_boundary,
        "fixture must exercise both sides of a shared wrap boundary"
    );
}

#[test]
fn deleting_final_wrapped_prose_row_advances_across_the_retained_separator() {
    let prose = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa";
    // This differs from deleting a middle wrapped row: its next surviving
    // character is in another paragraph. Normal-mode normalization must not
    // back up over the retained separator and reverse repeated dd's direction.
    for tail in ["\n\nFollowing", "\n\n## Following", "\n\n\n\nFollowing", ""] {
        for command in ["dd", "Vd", "gutter-delete"] {
            let source = format!("{prose}{tail}");
            let (mut core, view) = open(&source, 140.);
            let before = core.document().text().to_owned();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.text_range.end == prose.len())
                .unwrap();
            assert!(row.wrapped_from_previous);
            let selected = row.text_range.clone();
            let gutter = CoreEvent::SelectPointerRow {
                document_revision: core.document().revision(),
                layout_revision: snapshot.revision,
                y: row.y + row.height() / 2.,
                extend_selection: false,
            };
            place(
                &mut core,
                view,
                selected.start,
                BoundaryAffinity::Downstream,
            );
            if command == "gutter-delete" {
                core.handle(view, gutter).unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Delete)))
                    .unwrap();
            } else {
                keys(&mut core, view, command);
            }
            let expected_text = format!("{}{}", &before[..selected.start], &before[selected.end..]);
            assert_eq!(core.document().text(), expected_text);
            let expected = if tail.is_empty() {
                selected.start - 1
            } else {
                selected.start + 1
            };
            assert_eq!(
                core.command_state(view).unwrap().cursor(),
                expected,
                "{tail:?} {command}"
            );
            assert_eq!(
                core.command_state(view).unwrap().boundary_affinity(),
                BoundaryAffinity::Downstream
            );
            if tail == "\n\n\n\nFollowing" {
                assert!(
                    core.document()
                        .paragraph_range_at(expected)
                        .unwrap()
                        .is_empty(),
                    "the following empty paragraph is a stop, not disposable whitespace"
                );
            }
            if command != "gutter-delete" {
                let after = core.document().source_bytes();
                keys(&mut core, view, "u");
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), after);
                assert_eq!(core.command_state(view).unwrap().cursor(), expected);
            }
        }
    }
    // The same retained-separator situation in a quote still obeys containment.
    let (mut core, view) = open(&format!("> {prose}\n\nFollowing"), 140.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let row = snapshot
        .rows
        .iter()
        .find(|row| row.text_range.end == prose.len())
        .unwrap();
    let at = row.text_range.start;
    assert!(row.wrapped_from_previous);
    place(&mut core, view, at, BoundaryAffinity::Downstream);
    keys(&mut core, view, "dd");
    assert_eq!(core.command_state(view).unwrap().cursor(), at - 1);
}

#[test]
fn nested_empty_and_adjacent_owners_use_identity_and_surviving_content() {
    for (source, marker, expected_text, expected_cursor) in [
        // Removing an entire inner code owner still leaves the enclosing quote.
        (
            "> intro\n>\n> ```\n> last\n> ```\n\nFollowing",
            "last",
            "intro\nFollowing",
            4,
        ),
        // A different code owner with the same style is not a surviving prefix.
        (
            "```\nfirst\n```\n\n```\nlast\n```\n\nFollowing",
            "last",
            "first\nFollowing",
            6,
        ),
        (
            "> first\n\nplain\n\n> last\n\nFollowing",
            "last",
            "first\nplain\nFollowing",
            12,
        ),
        // Empty tail and empty surviving rows still have an editable position.
        (
            "```\nfirst\n\n```\n\nFollowing",
            "\nFollowing",
            "first\nFollowing",
            4,
        ),
        ("```\n\nlast\n```\n\nFollowing", "last", "\nFollowing", 0),
        (
            "> ```\n>\n> last\n> ```\n\nFollowing",
            "last",
            "\nFollowing",
            0,
        ),
        (
            "- intro\n\n  ```\n\n  last\n  ```\n\nFollowing",
            "last",
            "intro\n\nFollowing",
            6,
        ),
        (
            "```\n👩‍💻\nlast\n```\n\nFollowing",
            "last",
            "👩‍💻\nFollowing",
            0,
        ),
    ] {
        let (mut core, view) = open(source, 600.);
        let before = core.document().projection().blocks().to_vec();
        let at = core.document().text().find(marker).unwrap();
        place(&mut core, view, at, BoundaryAffinity::Downstream);
        keys(&mut core, view, "dd");
        assert_eq!(core.document().text(), expected_text, "{source}");
        assert_eq!(
            core.command_state(view).unwrap().cursor(),
            expected_cursor,
            "{source}: before={before:?} after={:?}",
            core.document().projection().blocks()
        );
        if expected_text.starts_with('\n') {
            // The empty code owner survived; source-based reprojection must use
            // the requested edit, not an equivalent replace-all text diff that
            // would retire its identity and break containment.
            assert_eq!(core.document().projection().blocks()[0].id, before[0].id);
        }
    }
}

#[test]
fn physical_and_layout_free_line_deletes_share_containment() {
    let source = "```\nfirst\nlast\n```\n\nFollowing";
    let (mut core, view) = open(source, 600.);
    core.handle(
        view,
        CoreEvent::SetLineMode(viem_core::command::LineMode::PhysicalSource),
    )
    .unwrap();
    place(&mut core, view, 6, BoundaryAffinity::Downstream);
    keys(&mut core, view, "dd");
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);

    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut commands = viem_core::command::CommandInterpreter::new();
    assert!(commands.set_cursor(&document, 6));
    for key in "dd".chars() {
        commands
            .handle(&mut document, InputEvent::key(key))
            .unwrap();
    }
    assert_eq!(commands.cursor(), 4);
    assert_eq!(commands.boundary_affinity(), BoundaryAffinity::Downstream);
}

#[test]
fn containment_is_view_local_and_does_not_override_explicit_ex_navigation() {
    let source = "```\nfirst\nlast\n```\n\nFollowing";
    let (mut core, view) = open(source, 600.);
    let other = core.add_view(MockTextMeasurementProvider::new(), 100., 600.);
    let following = core.document().text().find("Following").unwrap();
    place(&mut core, other, following, BoundaryAffinity::Downstream);
    place(&mut core, view, 6, BoundaryAffinity::Downstream);
    keys(&mut core, view, "dd");
    assert_eq!(core.command_state(view).unwrap().cursor(), 4);
    assert_eq!(
        core.command_state(other).unwrap().cursor(),
        core.document().text().find("Following").unwrap()
    );

    let (mut core, view) = open(source, 600.);
    place(&mut core, view, 6, BoundaryAffinity::Downstream);
    keys(&mut core, view, ":delete");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        core.document().text().find("Following").unwrap()
    );
}
