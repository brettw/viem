use viem_core::command::{CommandStatus, InputEvent, Key, LineMode, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str, format: Format, width: f32) -> (Editor, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 300.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    assert!(
        matches!(
            output.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ),
        "{key:?}"
    );
    core.document()
        .text_point(core.command_state(view).unwrap().cursor())
        .unwrap();
}

fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for character in text.chars() {
        key(core, view, Key::Char(character));
    }
}

#[test]
fn delete_keys_remove_native_unicode_selection_and_round_trip_source() {
    for (format, source) in [
        (Format::PlainText, "left 👩‍💻éright"),
        (Format::Markdown, "left **👩‍💻é**right"),
        (
            Format::Html,
            "<p data-keep='yes'>left <b>👩‍💻é</b>right</p><!--keep-->",
        ),
    ] {
        for delete in [Key::Backspace, Key::Delete] {
            for reverse in [false, true] {
                let (mut core, view) = editor(source, format, 300.);
                assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
                let endpoints = if reverse { [18, 5] } else { [5, 18] };
                for (index, text_offset) in endpoints.into_iter().enumerate() {
                    core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset,
                            affinity: BoundaryAffinity::Downstream,
                            extend_selection: index == 1,
                        },
                    )
                    .unwrap();
                }
                assert_eq!(core.list_selection_identity(view).unwrap().range(), 5..18);
                key(&mut core, view, delete);
                assert_eq!(core.document().text(), "left right");
                assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
                assert_eq!(
                    core.command_state(view)
                        .unwrap()
                        .register('"')
                        .unwrap()
                        .text,
                    "👩‍💻é"
                );
                let saved = core.document().source_bytes();
                let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
                assert_eq!(reopened.text(), "left right");
                key(&mut core, view, Key::Escape);
                key(&mut core, view, Key::Char('u'));
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                key(&mut core, view, Key::Ctrl('r'));
                assert_eq!(core.document().source_bytes(), saved);
            }
        }
    }
}

#[test]
fn delete_keys_share_visual_delete_counts_registers_shapes_and_repeat() {
    let original = "abcdefghij\nklmnopqrst\nuvwxyz";
    for (line_mode, block, selection, width) in [
        (LineMode::Visual, false, "vll", 300.),
        (LineMode::Visual, false, "Vj", 55.),
        (LineMode::PhysicalSource, false, "Vj", 300.),
        (LineMode::Visual, true, "lj", 300.),
    ] {
        let mut results = Vec::new();
        for delete in [Key::Char('d'), Key::Backspace, Key::Delete] {
            let (mut core, view) = editor(original, Format::PlainText, width);
            core.handle(view, CoreEvent::SetLineMode(line_mode))
                .unwrap();
            if block {
                key(&mut core, view, Key::Ctrl('v'));
            }
            keys(&mut core, view, selection);
            keys(&mut core, view, "\"a2");
            key(&mut core, view, delete);
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
            let saved = core.document().source_bytes();
            let register = core
                .command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .clone();
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), original.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), saved);
            key(&mut core, view, Key::Char('.'));
            results.push((saved, register, core.document().source_bytes()));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], results[2]);
    }
}

#[test]
fn without_selection_backspace_still_moves_and_forward_delete_deletes_at_caret() {
    let (mut core, view) = editor("abc", Format::PlainText, 300.);
    key(&mut core, view, Key::Char('l'));
    key(&mut core, view, Key::Backspace);
    assert_eq!(core.document().text(), "abc");
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert!(!core.document().history_status().can_undo);
    key(&mut core, view, Key::Delete);
    assert_eq!(core.document().text(), "bc");
}
