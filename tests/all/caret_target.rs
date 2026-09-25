//! What the caret occupies is a model decision, not a drawing detail.
//!
//! These fixtures pin the decision itself, because a frontend that re-derives
//! it from the mode, the cursor offset, and the boundary affinity drew the
//! Normal-mode block one grapheme early after `$` and `<End>`.

use viem_core::command::caret::CaretTarget;
use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn new_core(text: &str) -> (TestCore, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500.0, 2_000.0);
    (core, view)
}

fn press(core: &mut TestCore, view: ViewId, keys: &[Key]) {
    for key in keys {
        core.handle(view, CoreEvent::Input(InputEvent::Key(*key)))
            .unwrap_or_else(|error| panic!("{key:?}: {error:?}"));
    }
}

fn chars(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

fn target(core: &TestCore, view: ViewId) -> CaretTarget {
    core.command_state(view)
        .expect("view is attached")
        .caret_target(core.document())
}

fn cell(core: &TestCore, view: ViewId) -> std::ops::Range<usize> {
    match target(core, view) {
        CaretTarget::Cell { range } => range,
        other => panic!("expected a character cell, got {other:?}"),
    }
}

#[test]
fn end_of_line_covers_the_last_character_not_the_one_before_it() {
    for keys in [vec![Key::Char('$')], vec![Key::End]] {
        let (mut core, view) = new_core("abcdef\nnext");
        press(&mut core, view, &keys);
        let state = core.command_state(view).unwrap();
        assert_eq!(state.cursor(), 5, "{keys:?}");
        assert_eq!(
            state.boundary_affinity(),
            BoundaryAffinity::Upstream,
            "{keys:?}: the model keeps its upstream boundary affinity"
        );
        assert_eq!(cell(&core, view), 5..6, "{keys:?}: the cell covers `f`");
    }
}

#[test]
fn a_character_cell_never_depends_on_boundary_affinity() {
    // The same offset reached with each affinity must present the same cell.
    let (mut upstream, upstream_view) = new_core("abcdef\nnext");
    press(&mut upstream, upstream_view, &chars("$"));
    let (mut downstream, downstream_view) = new_core("abcdef\nnext");
    press(&mut downstream, downstream_view, &chars("5l"));
    assert_eq!(
        upstream.command_state(upstream_view).unwrap().cursor(),
        downstream.command_state(downstream_view).unwrap().cursor()
    );
    assert_ne!(
        upstream
            .command_state(upstream_view)
            .unwrap()
            .boundary_affinity(),
        downstream
            .command_state(downstream_view)
            .unwrap()
            .boundary_affinity()
    );
    assert_eq!(
        cell(&upstream, upstream_view),
        cell(&downstream, downstream_view)
    );
    assert_eq!(
        target(&upstream, upstream_view).affinity(),
        BoundaryAffinity::Downstream,
        "a cell reports the neutral affinity rather than a row choice"
    );
}

#[test]
fn every_normal_mode_position_covers_exactly_the_grapheme_under_the_cursor() {
    let text = "ab cd\n\n  e\u{301}f g👩🏽‍💻h\nlast";
    // Walk every reachable position with the motions a user actually types.
    for keys in [
        chars("gg0"),
        chars("$"),
        chars("w"),
        chars("2w"),
        chars("3w"),
        chars("j"),
        chars("jj$"),
        chars("G"),
        chars("G$"),
        chars("ggl"),
        chars("gg3l"),
    ] {
        let (mut core, view) = new_core(text);
        press(&mut core, view, &keys);
        let document_text = core.document().text().to_owned();
        let state = core.command_state(view).unwrap();
        let cursor = state.cursor();
        let snapshot = core.document().hard_line_snapshot();
        let line = snapshot.line_at_offset(cursor).unwrap();
        let expected = snapshot.grapheme_range_at(cursor);
        match state.caret_target(core.document()) {
            CaretTarget::Cell { range } => {
                assert_eq!(Some(range.clone()), expected, "{keys:?}");
                assert!(range.contains(&cursor), "{keys:?}");
                assert!(range.end <= line.content_range().end, "{keys:?}");
                assert!(
                    !document_text[range].contains('\n'),
                    "{keys:?}: a cell never covers a hard break"
                );
            }
            CaretTarget::Boundary { offset, .. } => {
                assert_eq!(offset, cursor, "{keys:?}");
                assert!(
                    cursor >= line.content_range().end,
                    "{keys:?}: a boundary only where the line has no character"
                );
            }
        }
    }
}

#[test]
fn multi_scalar_graphemes_are_covered_whole() {
    for (text, expected) in [("e\u{301}x", 0..3), ("👩🏽‍💻x", 0..15), ("🇳🇿x", 0..8)]
    {
        let (core, view) = new_core(text);
        assert_eq!(cell(&core, view), expected, "{text:?}");
    }
    let (mut core, view) = new_core("xe\u{301}");
    press(&mut core, view, &chars("$"));
    assert_eq!(
        cell(&core, view),
        1..4,
        "the trailing cluster is covered whole"
    );
}

#[test]
fn positions_with_no_character_present_a_boundary() {
    // An empty line, and the end of a document with no final terminator.
    let (mut core, view) = new_core("ab\n\ncd");
    press(&mut core, view, &chars("j"));
    assert_eq!(
        target(&core, view),
        CaretTarget::Boundary {
            offset: 3,
            affinity: BoundaryAffinity::Downstream
        }
    );
    let (core, view) = new_core("");
    assert_eq!(
        target(&core, view),
        CaretTarget::Boundary {
            offset: 0,
            affinity: BoundaryAffinity::Downstream
        }
    );
}

#[test]
fn insert_and_command_line_present_boundaries_that_keep_their_affinity() {
    let (mut core, view) = new_core("abcdef\nnext");
    press(&mut core, view, &chars("$"));
    assert!(target(&core, view).is_cell());
    press(&mut core, view, &chars("i"));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    match target(&core, view) {
        CaretTarget::Boundary { offset, .. } => assert_eq!(offset, 5),
        other => panic!("Insert mode addresses gaps, got {other:?}"),
    }
    press(&mut core, view, &[Key::Escape]);
    press(&mut core, view, &chars("A"));
    match target(&core, view) {
        CaretTarget::Boundary { offset, .. } => {
            assert_eq!(offset, 6, "appending past the last character")
        }
        other => panic!("Insert mode addresses gaps, got {other:?}"),
    }
    press(&mut core, view, &[Key::Escape]);
    press(&mut core, view, &chars(":"));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::CommandLine);
    assert!(
        !target(&core, view).is_cell(),
        "the command line addresses gaps"
    );
}

#[test]
fn visual_and_replace_modes_address_characters() {
    for (entry, mode) in [
        ("v", Mode::VisualCharacter),
        ("V", Mode::VisualLine),
        ("R", Mode::Replace),
    ] {
        let (mut core, view) = new_core("abcdef\nnext");
        press(&mut core, view, &chars("$"));
        press(&mut core, view, &chars(entry));
        assert_eq!(core.command_state(view).unwrap().mode(), mode, "{entry}");
        assert_eq!(cell(&core, view), 5..6, "{entry}");
    }
    // A Visual Block presents its moving corner.
    let (mut core, view) = new_core("abcdef\nghijkl");
    press(&mut core, view, &[Key::Ctrl('v')]);
    press(&mut core, view, &chars("jll"));
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::VisualBlock);
    let block = core.command_state(view).unwrap().visual_block().unwrap();
    let active = block.active.text_offset;
    assert_eq!(cell(&core, view), active..active + 1);
}

#[test]
fn a_wrapped_row_start_is_still_the_character_under_the_cursor() {
    let text = "wrapped words continue across several visual rows here";
    let document =
        Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 60.0, 2_000.0);
    let mut events = chars(":set wrap");
    events.push(Key::Enter);
    press(&mut core, view, &events);
    press(&mut core, view, &chars("gj"));
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let second = snapshot
        .rows
        .iter()
        .find(|row| row.wrapped_from_previous)
        .expect("the narrow view wraps this line");
    let start = second.text_range.start;
    press(&mut core, view, &chars("g0"));
    assert_eq!(core.command_state(view).unwrap().cursor(), start);
    let range = cell(&core, view);
    assert_eq!(
        range.start, start,
        "the cell is the character, not a row choice"
    );
    assert_eq!(range.end, start + 1);
}
