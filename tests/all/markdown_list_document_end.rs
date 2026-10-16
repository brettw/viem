//! Structural edits in the final owner of a Markdown document stay regional
//! when the source ends with a line terminator, as most saved files do. A
//! complete reprojection would discard every view's retained geometry. A blank
//! final row before the terminator still uses complete reprojection, since
//! the terminator then pairs into a separator rather than remaining trivia.
use crate::support::{assert_exact_undo_redo, assert_reopens_as_displayed, explain_candidates, Editor};
use viem_core::command::Key;
use viem_core::document::{measure_document_work, trace_candidate_strategies, BlockKind, ContainerKind, Format};

/// List structure queries still copy the block records (tracked with the
/// other remaining flat consumers in docs/performance.md); neither the
/// projection nor the formatted text may be rebuilt for the whole document.
fn assert_not_reprojected(context: &str, operation: impl FnOnce()) {
    let ((_, events), work) = measure_document_work(|| trace_candidate_strategies(operation));
    assert_eq!(work.full_projection_candidates, 0, "{context}: {}", explain_candidates(&events));
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{context}: {}", explain_candidates(&events));
}

#[test]
fn continuing_or_ending_a_final_list_before_a_terminator_stays_regional() {
    let intro = "Intro paragraph.\n\n".repeat(200);
    for (tail, continued) in [
        ("- one\n- two\n", "- one\n- two\n- \n"),
        ("1. one\n2. two\n", "1. one\n2. two\n3. \n"),
        ("> quoted\n>\n> - one\n> - two\n", "> quoted\n>\n> - one\n> - two\n> - \n"),
    ] {
        let source = intro.clone() + tail;
        let mut editor = Editor::open(&source, Format::Markdown);
        editor.type_keys("i");
        editor.place(editor.document().text().rfind("two").unwrap() + 3);
        assert_not_reprojected(tail, || editor.key(Key::Enter));
        assert_eq!(editor.source(), intro.clone() + continued, "{tail:?}");
        assert_reopens_as_displayed(editor.document(), tail);
        // Enter on the new empty item ends the list.
        editor.key(Key::Enter);
        assert_reopens_as_displayed(editor.document(), tail);
        assert_not_reprojected(tail, || editor.type_keys("after"));
        assert_reopens_as_displayed(editor.document(), tail);
        let last = editor.document().projection().blocks().iter()
            .rfind(|block| !block.range.is_empty()).expect("typed paragraph").clone();
        assert_eq!(&editor.document().text()[last.range.clone()], "after", "{tail:?}");
        assert!(matches!(last.kind, BlockKind::Paragraph) && last.containers.iter()
            .all(|member| !matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem)),
            "{tail:?}: {:?}", &editor.source()[intro.len()..]);
        let typed = editor.source();
        assert_exact_undo_redo(&mut editor, &source, &typed, tail);
    }
}

#[test]
fn continuing_a_final_list_with_dos_endings_stays_regional() {
    let source = "Intro paragraph.\r\n\r\n".repeat(200) + "- one\r\n- two\r\n";
    let mut editor = Editor::open(&source, Format::Markdown);
    editor.type_keys("i");
    editor.place(editor.document().text().rfind("two").unwrap() + 3);
    assert_not_reprojected("dos", || editor.key(Key::Enter));
    assert!(editor.source().ends_with("- two\r\n- \r\n"), "{:?}", &editor.source()[source.len() - 20..]);
    assert_reopens_as_displayed(editor.document(), "dos");
    let continued = editor.source();
    assert_exact_undo_redo(&mut editor, &source, &continued, "dos");
}
