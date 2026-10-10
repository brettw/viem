use viem_core::command::{InputEvent, Key, Mode, NavigationKey};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn fixture(source: &str, format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 120., 500.);
    (core, view)
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    assert!(matches!(
        result.command.unwrap().status,
        viem_core::command::CommandStatus::Complete
    ));
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

fn word(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize, extend: bool) {
    core.handle(
        view,
        CoreEvent::SelectPointerWord {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: extend,
        },
    )
    .unwrap();
}

fn extent(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> std::ops::Range<usize> {
    let state = core.command_state(view).unwrap();
    let anchor = state.visual_anchor().unwrap();
    anchor.min(state.cursor())..anchor.max(state.cursor())
}

#[test]
fn word_drag_keeps_the_initial_word_when_reversing_and_after_release() {
    let source = "first naïve e\u{301}lan final";
    let (mut core, view) = fixture(source, Format::PlainText);
    key(&mut core, view, Key::Char('i'));
    word(&mut core, view, 8, false);
    assert_eq!(extent(&core, view), 6..12);
    word(&mut core, view, 16, true);
    assert_eq!(extent(&core, view), 6..19);
    word(&mut core, view, 2, true);
    assert_eq!(extent(&core, view), 0..12);
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    word(&mut core, view, 10, true);
    assert_eq!(extent(&core, view), 6..12);
    key(&mut core, view, Key::Right);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    // A fresh character gesture must not reuse the old word anchor.
    place(&mut core, view, 7);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 10,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: true,
        },
    )
    .unwrap();
    assert_eq!(extent(&core, view), 7..10);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn word_selection_handles_punctuation_whitespace_empty_lines_and_eof() {
    for (source, at, expected) in [
        ("one!!!two", 4, 3..6),
        ("one  two", 3, 3..5),
        ("one 👩‍💻 last", 4, 4..15),
        ("one\n\nlast", 4, 4..4),
        ("one last", 8, 4..8),
        ("", 0, 0..0),
    ] {
        let (mut core, view) = fixture(source, Format::PlainText);
        word(&mut core, view, at, false);
        assert_eq!(extent(&core, view), expected, "{source:?}");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn counted_and_mapped_paragraph_navigation_replays_through_macros() {
    use viem_core::command::{CommandInterpreter, CommandStatus};
    let mut document = Document::new("one\ntwo\nthree\nfour");
    let mut commands = CommandInterpreter::new();
    let send = |commands: &mut CommandInterpreter, document: &mut Document, input: InputEvent| {
        let output = commands.handle(document, input).unwrap();
        assert!(
            matches!(
                output.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{:?}",
            output.status
        );
    };
    send(
        &mut commands,
        &mut document,
        InputEvent::text(":nnoremap <C-Down> <ParagraphEnd>"),
    );
    send(&mut commands, &mut document, InputEvent::Key(Key::Enter));
    send(
        &mut commands,
        &mut document,
        InputEvent::Key(Key::ModifiedNavigation {
            key: NavigationKey::NextParagraph,
            modifiers: 2,
        }),
    );
    assert_eq!(commands.cursor(), 3);
    for character in "qq2".chars() {
        send(&mut commands, &mut document, InputEvent::key(character));
    }
    send(
        &mut commands,
        &mut document,
        InputEvent::Key(Key::ParagraphEnd),
    );
    send(&mut commands, &mut document, InputEvent::key('q'));
    assert_eq!(commands.cursor(), 13);
    commands.set_cursor(&document, 0);
    send(&mut commands, &mut document, InputEvent::text("@q"));
    assert_eq!(commands.cursor(), 7);
    assert_eq!(document.text(), "one\ntwo\nthree\nfour");
}

#[test]
fn pointer_words_preserve_visual_policy_and_reject_stale_hits_atomically() {
    let (mut core, view) = fixture("one two three", Format::PlainText);
    key(&mut core, view, Key::Char('v'));
    word(&mut core, view, 5, false);
    assert!(!core.command_state(view).unwrap().is_native_selection());
    assert_eq!(core.command_state(view).unwrap().visual_anchor(), Some(4));
    assert_eq!(core.command_state(view).unwrap().cursor(), 6);
    let revision = core.document().revision();
    let other = core.add_view(MockTextMeasurementProvider::new(), 120., 500.);
    key(&mut core, other, Key::Char('i'));
    key(&mut core, other, Key::Char('X'));
    let before = extent(&core, view);
    assert!(core
        .handle(
            view,
            CoreEvent::SelectPointerWord {
                document_revision: revision,
                text_offset: 12,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: true,
            }
        )
        .is_err());
    assert_eq!(extent(&core, view), before);
    // The original word follows a shared-view edit via stable anchors.
    word(&mut core, view, 12, true);
    assert_eq!(core.command_state(view).unwrap().visual_anchor(), Some(5));
    assert_eq!(core.command_state(view).unwrap().cursor(), 13);
}

#[test]
fn paragraph_navigation_uses_semantic_paragraphs_not_wrapped_or_hard_rows() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let (mut core, view) = fixture(
            "first paragraph with many wrapped words\\\ncontinuation\n\nsecond paragraph\n\nlast",
            format,
        );
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, 4);
        let first = core.document().paragraph_range_at(4).unwrap();
        key(&mut core, view, Key::ParagraphEnd);
        assert_eq!(core.command_state(view).unwrap().cursor(), first.end);
        assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Upstream);
        let next_start = core.document().text().find("second").unwrap();
        key(&mut core, view, Key::NextParagraph);
        assert_eq!(core.command_state(view).unwrap().cursor(), next_start);
        assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Downstream);
        key(&mut core, view, Key::ParagraphStart);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Downstream);
        key(&mut core, view, Key::ParagraphStart);
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    }
}

#[test]
fn paragraph_shift_selection_replaces_and_undoes_exactly() {
    let source = "first paragraph\n\nsecond\nlast";
    let (mut core, view) = fixture(source, Format::PlainText);
    place(&mut core, view, 4);
    key(
        &mut core,
        view,
        Key::ModifiedNavigation {
            key: NavigationKey::ParagraphEnd,
            modifiers: 5,
        },
    );
    assert_eq!(extent(&core, view), 4..15);
    assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Upstream);
    key(
        &mut core,
        view,
        Key::ModifiedNavigation {
            key: NavigationKey::ParagraphEnd,
            modifiers: 5,
        },
    );
    assert_eq!(extent(&core, view), 4..16); // The empty paragraph is a real stop.
    key(
        &mut core,
        view,
        Key::ModifiedNavigation {
            key: NavigationKey::ParagraphEnd,
            modifiers: 5,
        },
    );
    assert_eq!(extent(&core, view), 4..23);
    key(&mut core, view, Key::Char('X'));
    assert_eq!(core.document().text(), "firsX\nlast");
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn distant_word_and_paragraph_queries_leave_unrelated_text_unmaterialized() {
    let source = format!(
        "{}\n\nlast target words",
        "unrelated paragraph\n\n".repeat(20_000)
    );
    let (mut core, view) = fixture(&source, Format::Markdown);
    let at = core.document().projection().text_tree().byte_len() - "target words".len();
    word(&mut core, view, at + 2, false);
    assert_eq!(extent(&core, view), at..at + 6);
    key(
        &mut core,
        view,
        Key::ModifiedNavigation {
            key: NavigationKey::ParagraphStart,
            modifiers: 3,
        },
    );
    assert_eq!(extent(&core, view), at - 5..at);
    assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 150);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
