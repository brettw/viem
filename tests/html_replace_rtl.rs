//! A collapsed HTML space can have whitespace contributors on both sides of
//! an ignored NUL. Replace must consume that whole space while preserving the
//! ignored source bytes and its per-grapheme restoration journal.
use evim_core::command::{CommandStatus, InputEvent, Key, Mode};
use evim_core::document::{Document, DocumentError, Encoding, Format, SourceArtifactDigest};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreError, CoreEvent, ViewId};

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 80., 700.);
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    let cursor = core.command_state(view).unwrap().cursor();
    let outcome = core
        .handle(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| {
            panic!(
                "{event:?} at {cursor} in {:?}: {error:?}",
                String::from_utf8_lossy(&before)
            )
        });
    assert_eq!(outcome.command.unwrap().status, CommandStatus::Complete);
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();

    // Check source bytes outside the declared patches for each transaction,
    // including the atomic transaction composed from a native text batch.
    if revision == core.document().revision() {
        return;
    }
    let details = core
        .document()
        .history_node_details(core.document().history_status().current.node)
        .unwrap();
    let Some(transaction) = details
        .transactions
        .last()
        .filter(|transaction| transaction.before_revision() == revision)
    else {
        return; // Undo/redo navigate recorded states rather than publish an edit.
    };
    let after = core.document().source_bytes();
    let mut old_at = 0;
    let mut new_at = 0;
    for patch in transaction.source_patches() {
        let range = patch.range();
        let unchanged = range.start - old_at;
        assert_eq!(
            &before[old_at..range.start],
            &after[new_at..new_at + unchanged]
        );
        new_at += unchanged;
        assert_eq!(
            patch.replacement_digest(),
            SourceArtifactDigest::from_bytes(&after[new_at..new_at + patch.replacement_len()])
        );
        new_at += patch.replacement_len();
        old_at = range.end;
    }
    assert_eq!(&before[old_at..], &after[new_at..]);
}

fn assert_reopens(core: &Core<MockTextMeasurementProvider>, expected: &str) {
    assert_eq!(core.document().text(), expected);
    let fresh =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(fresh.text(), expected);
    let actual_lines = core.document().hard_line_snapshot();
    let reopened_lines = fresh.hard_line_snapshot();
    assert_eq!(actual_lines.line_count(), reopened_lines.line_count());
    for index in 0..actual_lines.line_count() {
        let actual = actual_lines.line(index).unwrap();
        let reopened = reopened_lines.line(index).unwrap();
        assert_eq!(actual.content_range(), reopened.content_range());
        assert_eq!(actual.separator_range(), reopened.separator_range());
    }
}

fn replace_prefix(core: &mut Core<MockTextMeasurementProvider>, view: ViewId) {
    input(core, view, InputEvent::key('R'));
    input(core, view, InputEvent::text("👩‍💻"));
    input(core, view, InputEvent::text("é"));
    input(core, view, InputEvent::Key(Key::Enter));
    let source = core.document().source_bytes();
    let text = core.document().text().to_owned();
    let cursor = core.command_state(view).unwrap().cursor();
    input(core, view, InputEvent::text("中"));
    input(core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), source);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    assert_reopens(core, &text);
}

#[test]
fn html_replace_enter_backspace_and_rtl_batch_restore_exact_source() {
    let original = "<p data-keep='x'>abربي\n\0\n-العربية</p><!--keep-->";
    let (mut core, view) = open(original);
    assert_reopens(&core, "abربي -العربية");
    replace_prefix(&mut core, view);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Replace);
    assert_eq!(core.command_state(view).unwrap().cursor(), "👩‍💻é\n".len());
    assert_reopens(&core, "👩‍💻é\nربي -العربية");
    let before_batch = core.document().source_bytes();
    let revision = core.document().revision();

    // This native text event used to fail on its fourth speculative grapheme,
    // which replaces the visible space backed by "\n\0\n".
    input(&mut core, view, InputEvent::text("العربية"));
    assert_eq!(core.document().revision().0, revision.0 + 1);
    assert_eq!(
        core.command_state(view).unwrap().cursor(),
        "👩‍💻é\nالعربية".len()
    );
    assert_reopens(&core, "👩‍💻é\nالعربيةعربية");
    assert_eq!(core.document().hard_line_snapshot().line_count(), 2);
    let final_source = core.document().source_bytes();
    assert!(final_source.starts_with(b"<p data-keep='x'>"));
    assert!(final_source.ends_with(b"</p><!--keep-->"));
    assert_eq!(final_source.iter().filter(|byte| **byte == 0).count(), 1);

    // Compare every batch journal frontier with the exact bytes and caret
    // produced when those same graphemes arrive as individual text events.
    let (mut individual, individual_view) = open(original);
    replace_prefix(&mut individual, individual_view);
    let mut frontiers = vec![(
        individual.document().source_bytes(),
        individual.document().text().to_owned(),
        individual.command_state(individual_view).unwrap().cursor(),
    )];
    for ch in "العربية".chars() {
        input(
            &mut individual,
            individual_view,
            InputEvent::text(ch.to_string()),
        );
        frontiers.push((
            individual.document().source_bytes(),
            individual.document().text().to_owned(),
            individual.command_state(individual_view).unwrap().cursor(),
        ));
    }
    assert_eq!(individual.document().source_bytes(), final_source);
    for (source, text, cursor) in frontiers[..frontiers.len() - 1].iter().rev() {
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(&core.document().source_bytes(), source);
        assert_eq!(core.command_state(view).unwrap().cursor(), *cursor);
        assert_reopens(&core, text);
    }
    assert_eq!(core.document().source_bytes(), before_batch);
    input(&mut core, view, InputEvent::text("العربية"));
    assert_eq!(core.document().source_bytes(), final_source);
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    assert_reopens(&core, "abربي -العربية");
    assert!(!core.document().history_status().can_undo);
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), final_source);
    assert_reopens(&core, "👩‍💻é\nالعربيةعربية");
    assert!(!core.document().history_status().can_redo);
}

#[test]
fn rejected_html_replace_batch_preserves_source_revision_cursor_and_journal() {
    let original = "<p data-keep='x'>abcd</p><!--keep-->";
    let (mut core, view) = open(original);
    input(&mut core, view, InputEvent::key('R'));
    input(&mut core, view, InputEvent::text("👩‍💻"));
    let source = core.document().source_bytes();
    let revision = core.document().revision();
    let cursor = core.command_state(view).unwrap().cursor();
    let history = core.document().history_status();

    // The first grapheme prepares successfully on scratch; NUL cannot be
    // represented as visible HTML text, so the entire native event must fail.
    assert!(matches!(
        core.handle(view, CoreEvent::Input(InputEvent::text("x\0"))),
        Err(CoreError::Document(
            DocumentError::UnrepresentableFormattedCharacter {
                format: Format::Html,
                character: '\0',
            }
        ))
    ));
    assert_eq!(core.document().source_bytes(), source);
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Replace);
    assert_reopens(&core, "👩‍💻bcd");
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert_reopens(&core, "abcd");
}
