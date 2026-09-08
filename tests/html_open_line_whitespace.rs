//! Minimized from fuzz campaign 20260907T193459Z-15419, seed 1, action 1100.
//! The malformed input was not essential: O at a wrapped row can move its
//! insertion boundary when the preceding ordinary space becomes NBSP.
use evim_core::command::{CommandStatus, InputEvent, Key, Mode};
use evim_core::document::{BoundaryAffinity, Document, Encoding, Format, SourceArtifactDigest};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent, ViewId};

fn open(source: &str, width: f32, at: usize) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 300.);
    place(&mut core, view, at);
    (core, view)
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    let description = format!(
        "{event:?} at {} in {:?}",
        core.command_state(view).unwrap().cursor(),
        String::from_utf8_lossy(&before)
    );
    let outcome = core
        .handle(view, CoreEvent::Input(event))
        .unwrap_or_else(|error| panic!("{description}: {error:?}"));
    if let Some(command) = outcome.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            ),
            "{description}: {:?}",
            command.status
        );
    }
    let document = core.document();
    document
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
    if document.revision() == revision || !outcome.document_changed {
        return;
    }
    let after = document.source_bytes();
    let details = document
        .history_node_details(document.history_status().current.node)
        .unwrap();
    let Some(transaction) = details
        .transactions
        .last()
        .filter(|transaction| transaction.before_revision() == revision)
    else {
        return;
    };
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

fn assert_reopens(core: &Core<MockTextMeasurementProvider>, text: &str, source: &str) {
    assert_eq!(core.document().text(), text);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    let fresh =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(fresh.text(), text);
    assert_eq!(
        fresh.projection().hard_line_count(),
        core.document().projection().hard_line_count()
    );
}

fn history_roundtrip(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    original: &str,
    changed: &str,
) {
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), changed.as_bytes());
}

#[test]
fn open_above_a_wrapped_row_maps_past_the_protected_space() {
    for (word, width) in [("A", 20.), ("é", 20.), ("e\u{301}", 20.), ("中", 36.)] {
        let source = format!("<p data-keep='x'>{word} B</p><!--keep-->");
        let at = word.len() + 1;
        let (mut core, view) = open(&source, width, at);
        input(&mut core, view, InputEvent::key('O'));
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.command_state(view).unwrap().cursor(), word.len() + 2);
        assert_reopens(
            &core,
            &format!("{word}\u{a0}\nB"),
            &format!("<p data-keep='x'>{word}&nbsp;<br>B</p><!--keep-->"),
        );
        input(&mut core, view, InputEvent::text("C"));
        let changed = format!("<p data-keep='x'>{word} C<br>B</p><!--keep-->");
        assert_reopens(&core, &format!("{word} C\nB"), &changed);
        assert_eq!(core.command_state(view).unwrap().cursor(), word.len() + 2);
        input(&mut core, view, InputEvent::Key(Key::Escape));
        history_roundtrip(&mut core, view, &source, &changed);
    }
}

#[test]
fn open_below_a_wrapped_row_stays_after_its_own_break() {
    for (width, text, caret, source) in [
        (20., "A\u{a0}\nB", 4, "<p>A&nbsp;<br>B</p><!--keep-->"),
        (12., "A\u{a0}\nB", 4, "<p>A&nbsp;<br>B</p><!--keep-->"),
    ] {
        let original = "<p>A B</p><!--keep-->";
        let (mut core, view) = open(original, width, 0);
        input(&mut core, view, InputEvent::key('o'));
        assert_eq!(core.command_state(view).unwrap().cursor(), caret);
        assert_reopens(&core, text, source);
        input(&mut core, view, InputEvent::text("C"));
        // Even at 12px, the space stays with its preceding word instead of
        // introducing an emergency break before the whitespace.
        let (text, changed) = ("A\u{a0}\nCB", "<p>A&nbsp;<br>CB</p><!--keep-->");
        assert_reopens(&core, text, changed);
        assert_eq!(core.command_state(view).unwrap().cursor(), caret + 1);
        input(&mut core, view, InputEvent::Key(Key::Escape));
        history_roundtrip(&mut core, view, original, changed);
    }
}

#[test]
fn wrapped_open_line_keeps_other_view_anchors_valid() {
    let original = "<p>A B</p><!--keep-->";
    let (mut core, view) = open(original, 20., 2);
    let other = core.add_view(MockTextMeasurementProvider::new(), 200., 300.);
    place(&mut core, other, 2);
    input(&mut core, view, InputEvent::key('O'));
    assert_eq!(core.command_state(view).unwrap().cursor(), 3);
    assert_eq!(core.command_state(other).unwrap().cursor(), 4);
    core.document()
        .text_point(core.command_state(other).unwrap().cursor())
        .unwrap();
    input(&mut core, view, InputEvent::text("C"));
    assert_eq!(core.command_state(other).unwrap().cursor(), 4);
    assert_reopens(&core, "A C\nB", "<p>A C<br>B</p><!--keep-->");
    input(&mut core, view, InputEvent::Key(Key::Escape));
    history_roundtrip(&mut core, view, original, "<p>A C<br>B</p><!--keep-->");
    core.document()
        .text_point(core.command_state(other).unwrap().cursor())
        .unwrap();
}

#[test]
fn counted_open_lines_and_dot_repeat_keep_whitespace_and_undo_groups() {
    for (key, at, text, changed, repeated_text, repeated) in [
        (
            'O',
            2,
            "A C\nC\nB",
            "<p>A C<br>C<br>B</p>",
            "A C\nC\nC\nC\nB",
            "<p>A C<br>C<br>C<br>C<br>B</p>",
        ),
        (
            'o',
            0,
            "A\u{a0}\nC\nCB",
            "<p>A&nbsp;<br>C<br>CB</p>",
            "A\u{a0}\nC\nCB\nC\nC",
            "<p>A&nbsp;<br>C<br>CB<br>C<br>C</p>",
        ),
    ] {
        let original = "<p>A B</p>";
        let (mut core, view) = open(original, 24., at);
        input(&mut core, view, InputEvent::key('2'));
        input(&mut core, view, InputEvent::key(key));
        input(&mut core, view, InputEvent::text("C"));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_reopens(&core, text, changed);
        input(&mut core, view, InputEvent::key('.'));
        assert_reopens(&core, repeated_text, repeated);
        history_roundtrip(&mut core, view, changed, repeated);
        input(&mut core, view, InputEvent::key('u'));
        input(&mut core, view, InputEvent::key('u'));
        assert_eq!(core.document().source_bytes(), original.as_bytes());
        input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
        assert_eq!(core.document().source_bytes(), changed.as_bytes());
        input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
        assert_eq!(core.document().source_bytes(), repeated.as_bytes());
    }
}
