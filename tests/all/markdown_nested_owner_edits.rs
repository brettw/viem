//! Native and Vim edits retain the complete path through interleaved containers.
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    Block, BlockKind, BoundaryAffinity, ContainerKind, Encoding, Format, HistoryNavigationRequest,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

const DEEP: &str = "- a\n  - b\n    - c\n      - d\n        - e";
const MIXED: &str = "> - > - > a\n> - > - > b";

fn shape(document: &Document) -> Vec<Block> {
    document
        .projection()
        .blocks()
        .iter()
        .cloned()
        .map(|mut block| {
            block.id = 0;
            block
        })
        .collect()
}

fn list_level(block: &Block) -> Option<u8> {
    if let BlockKind::ListItem { level, .. } = block.kind {
        Some(level)
    } else {
        None
    }
}

fn owner_path(block: &Block) -> Vec<ContainerKind> {
    block
        .containers
        .iter()
        .map(|member| member.container.kind)
        .collect()
}

fn exercise(
    source: &str,
    at: usize,
    events: Vec<InputEvent>,
    expected: &str,
) -> Result<Vec<Block>, String> {
    exercise_selection(source, at, None, events, expected).map(|(blocks, _)| blocks)
}

fn exercise_selection(
    source: &str,
    at: usize,
    end: Option<usize>,
    events: Vec<InputEvent>,
    expected: &str,
) -> Result<(Vec<Block>, Vec<u8>), String> {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
    if end.is_some() {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
    }
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    if let Some(end) = end {
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: end,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: true,
            },
        )
        .unwrap();
    }
    let context = format!("{source:?} at{at} end{end:?}: {events:?}");
    for event in events {
        let (result, trace) = viem_core::document::trace_candidate_strategies(|| {
            core.handle_with_layout(view, CoreEvent::Input(event.clone()))
        });
        let outcome = result.map_err(|error| {
            format!(
                "{context}: {event:?}: {error:?}, source={:?}, trace={trace:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        })?;
        if outcome.command.as_ref().is_some_and(|command| {
            !matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            )
        }) {
            return Err(format!("{context}: {outcome:?}"));
        }
    }
    if core.document().text() != expected {
        return Err(format!(
            "{context}: expected{expected:?}, actual{:?}, source={:?}",
            core.document().text(),
            String::from_utf8_lossy(&core.document().source_bytes())
        ));
    }
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(reopened.text(), expected, "{context}");
    assert_eq!(
        shape(core.document()),
        shape(&reopened),
        "{context}: reopened owners"
    );
    core.handle_with_layout(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(
        core.document().source_bytes(),
        source.as_bytes(),
        "{context}: exact undo"
    );
    core.handle_with_layout(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(
        core.document().source_bytes(),
        saved,
        "{context}: exact redo"
    );
    Ok((shape(core.document()), saved))
}

#[test]
fn deleting_nested_item_rows_preserves_surviving_descendants() {
    let mut failures = Vec::new();
    let original =
        Document::from_bytes(DEEP.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    for (at, expected) in [(0, "c\nd\ne"), (4, "a\nb\ne")] {
        match exercise(
            DEEP,
            at,
            vec![
                InputEvent::key('2'),
                InputEvent::key('d'),
                InputEvent::key('d'),
            ],
            expected,
        ) {
            Ok(blocks) => {
                assert_eq!(blocks.len(), expected.lines().count());
                for block in &blocks {
                    let retained = &expected[block.range.clone()];
                    let old_at = original.text().find(retained).unwrap();
                    let old = original
                        .projection()
                        .blocks()
                        .iter()
                        .find(|block| block.range.start <= old_at && old_at < block.range.end)
                        .unwrap();
                    assert_eq!(
                        list_level(block),
                        list_level(old),
                        "2dd at {at}: {retained:?} retains its original list level"
                    );
                    assert_eq!(
                        owner_path(block),
                        owner_path(old),
                        "2dd at {at}: {retained:?} retains its ancestor containers"
                    );
                }
            }
            Err(error) => failures.push(error),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn interleaved_quote_list_editing_keeps_empty_bodies_and_container_paths() {
    let mut failures = Vec::new();
    let original =
        Document::from_bytes(MIXED.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let old = &original.projection().blocks()[0];
    for (at, events, expected) in [
        (0, vec![InputEvent::key('x')], "\nb"),
        (2, vec![InputEvent::key('x')], "a\n"),
        (0, vec![InputEvent::key('J')], "a b"),
        (
            0,
            vec![InputEvent::key('o'), InputEvent::Key(Key::Escape)],
            "a\n\nb",
        ),
        (
            0,
            vec![InputEvent::key('O'), InputEvent::Key(Key::Escape)],
            "\na\nb",
        ),
        (
            2,
            vec![
                InputEvent::key('i'),
                InputEvent::Key(Key::Enter),
                InputEvent::Key(Key::Escape),
            ],
            "a\n\nb",
        ),
        (
            0,
            vec![
                InputEvent::key('o'),
                InputEvent::text("x"),
                InputEvent::Key(Key::Escape),
            ],
            "a\nx\nb",
        ),
        (
            0,
            vec![
                InputEvent::key('O'),
                InputEvent::text("x"),
                InputEvent::Key(Key::Escape),
            ],
            "x\na\nb",
        ),
    ] {
        match exercise(MIXED, at, events, expected) {
            Ok(blocks) => {
                for block in &blocks {
                    assert_eq!(owner_path(block), owner_path(old), "{expected:?}: retained and newly empty paragraphs keep the interleaved container path");
                    assert_eq!(block.quote_depth, old.quote_depth, "{expected:?}");
                    assert_eq!(list_level(block), list_level(old), "{expected:?}");
                    assert_eq!(block.style, old.style, "{expected:?}");
                }
            }
            Err(error) => failures.push(error),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn removing_list_treatment_preserves_a_contained_table() {
    let source =
        "- item\n\n  | a | b | c | d |\n  |---|---|---|---|\n  | 1 | 2 | 3 | 4 |\n\n- next";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let expected = document.text().to_owned();
    for insert in ['i', 'I'] {
        exercise(
            source,
            0,
            vec![
                InputEvent::key(insert),
                InputEvent::Key(Key::Backspace),
                InputEvent::Key(Key::Escape),
            ],
            &expected,
        )
        .unwrap();
    }
}

#[test]
fn native_selection_edits_keep_descendants_of_joined_list_owners() {
    let original =
        Document::from_bytes(DEEP.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut failures = Vec::new();
    for end in [6, 8] {
        for reverse in [false, true] {
            for (replacement, event) in [
                ("", InputEvent::Key(Key::Backspace)),
                ("", InputEvent::Key(Key::Delete)),
                ("x", InputEvent::text("x")),
            ] {
                let expected = format!("{replacement}{}", &original.text()[end..]);
                let (anchor, cursor) = if reverse { (end, 0) } else { (0, end) };
                match exercise_selection(
                    DEEP,
                    anchor,
                    Some(cursor),
                    vec![event, InputEvent::Key(Key::Escape)],
                    &expected,
                ) {
                    Ok((blocks, _)) => {
                        assert_eq!(
                            list_level(&blocks[0]),
                            Some(0),
                            "joined body keeps the first selected paragraph's owner"
                        );
                        if end == 6 {
                            let retained = blocks.last().unwrap();
                            let old = original.projection().blocks().last().unwrap();
                            assert_eq!(
                                list_level(retained),
                                list_level(old),
                                "unjoined descendant retains its original list level"
                            );
                            assert_eq!(owner_path(retained), owner_path(old));
                        }
                    }
                    Err(error) => failures.push(error),
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn descendant_retention_uses_each_removed_marker_once_and_keeps_inline_spelling() {
    let mut failures = Vec::new();
    for (source, at, command, expected, suffix) in [
        ("- a\n\n  b\n\n  - c", 0, "2dd", "c", "- c"),
        ("- a\n\n  b\n\n  - c", 2, "dd", "a\nc", "- c"),
        ("- a\n\n  b\n\n  - c", 0, "J", "a b\nc", "- c"),
        ("- **a**\n  - b", 0, "dd", "b", "- b"),
        ("- a\n  - **b**", 0, "dd", "b", "- **b**"),
        ("- <ins>a</ins>\n  - b", 0, "dd", "b", "- b"),
        ("- a\n  - <ins>b</ins>", 0, "dd", "b", "- <ins>b</ins>"),
        ("- [a](where)\n  - b", 0, "dd", "b", "- b"),
        ("- a\n  - [b](where)", 0, "dd", "b", "- [b](where)"),
    ] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let old = original.projection().blocks().last().unwrap();
        match exercise_selection(
            source,
            at,
            None,
            command.chars().map(InputEvent::key).collect(),
            expected,
        ) {
            Ok((blocks, saved)) => {
                let retained = blocks.last().unwrap();
                assert_eq!(
                    list_level(retained),
                    list_level(old),
                    "{source:?} {command}: descendant level"
                );
                assert_eq!(
                    owner_path(retained),
                    owner_path(old),
                    "{source:?} {command}: descendant path"
                );
                assert!(
                    saved.ends_with(suffix.as_bytes()),
                    "{source:?} {command}: retained source {:?}",
                    String::from_utf8_lossy(&saved)
                );
                let reopened =
                    Document::from_bytes(saved, Encoding::Utf8, Format::Markdown).unwrap();
                let before_style = viem_core::layout::DocumentLayoutStyles::semantic_character_at(
                    original.projection(),
                    old.range.start,
                    false,
                )
                .unwrap();
                let after_style = viem_core::layout::DocumentLayoutStyles::semantic_character_at(
                    reopened.projection(),
                    retained.range.start,
                    false,
                )
                .unwrap();
                assert_eq!(
                    before_style, after_style,
                    "{source:?} {command}: descendant character style"
                );
            }
            Err(error) => failures.push(error),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn changing_nested_quote_bodies_avoids_redundant_empty_support() {
    for source in [
        "1. > heading",
        "- > # h",
        "> - > - > h",
        "- > # h\n\n- next",
    ] {
        let expected = if source.ends_with("next") {
            "x\nnext"
        } else {
            "x"
        };
        let (_, saved) = exercise_selection(
            source,
            0,
            None,
            vec![
                InputEvent::key('c'),
                InputEvent::key('c'),
                InputEvent::text("x"),
                InputEvent::Key(Key::Escape),
            ],
            expected,
        )
        .unwrap();
        let body = if source.starts_with("1.") {
            "heading"
        } else {
            "h"
        };
        assert_eq!(
            saved,
            source.replacen(body, "x", 1).as_bytes(),
            "{source:?}"
        );
    }
    for hidden in ["<span></span>", "<span data-note='keep'></span>"] {
        let source = format!("1. > {hidden}h");
        let (_, saved) = exercise_selection(
            &source,
            0,
            None,
            vec![
                InputEvent::key('c'),
                InputEvent::key('c'),
                InputEvent::text("x"),
                InputEvent::Key(Key::Escape),
            ],
            "x",
        )
        .unwrap();
        let saved = String::from_utf8(saved).unwrap();
        assert!(
            saved.contains(hidden),
            "authored hidden source survives: {saved:?}"
        );
        assert_eq!(
            saved.matches("<span").count(),
            1,
            "no additional empty support: {saved:?}"
        );
    }
}
