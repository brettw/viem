use viem_core::command::{CommandStatus, InputEvent, LineMode};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn open(source: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 140., 1000.);
    (core, view)
}

fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
}

fn keys(core: &mut Editor, view: ViewId, keys: &str, with_layout: bool) {
    for character in keys.chars() {
        let event = CoreEvent::Input(InputEvent::key(character));
        let result = if with_layout { core.handle_with_layout(view, event) } else { core.handle(view, event) }.unwrap();
        assert!(matches!(result.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending), "{keys}");
    }
}

#[test]
fn wrapped_list_shifts_match_the_buttons_and_preserve_whole_items() {
    for unindent in [false, true] {
        let source = if unindent {
            "- parent\n  - Target has many words that wrap onto several display rows\n    - descendant\n- end"
        } else {
            "- parent\n- Target has many words that wrap onto several display rows\n  - descendant\n- end"
        };
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            for with_layout in [false, true] {
                for visual in [false, true] {
                    let (mut core, view) = open(source);
                    let (mut native, native_view) = open(source);
                    let start = core.document().text().find("Target").unwrap();
                    let at = core.layout(view).unwrap().snapshot().unwrap().rows.iter()
                        .find(|row| start < row.text_range.start && row.text_range.start < start + 30).unwrap().text_range.start;
                    for (core, view) in [(&mut core, view), (&mut native, native_view)] {
                        core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
                        place(core, view, at);
                    }
                    let expected = native.list_selection_identity(native_view).unwrap();
                    native.handle(native_view, CoreEvent::IndentList { expected, unindent }).unwrap();
                    let command = match (visual, unindent) { (false, false) => ">>", (false, true) => "<<", (true, false) => "V>", (true, true) => "V<" };
                    keys(&mut core, view, command, with_layout);
                    assert_eq!(core.document().source_bytes(), native.document().source_bytes(), "{mode:?} {command}");
                    assert_eq!(core.document().text(), native.document().text());
                    assert_eq!(core.command_state(view).unwrap().cursor(), at);
                    let edited = core.document().source_bytes();
                    let fresh = Document::from_bytes(edited.clone(), Encoding::Utf8, Format::Markdown).unwrap();
                    assert_eq!(core.document().projection().blocks(), fresh.projection().blocks());
                    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                    assert_eq!(core.document().source_bytes(), edited);
                }
            }
        }
    }
}

#[test]
fn preview_shifts_are_no_ops_where_indent_buttons_are_disabled() {
    for source in [
        "The quick brown fox jumps over the lazy dog and wraps onto several rows",
        "# The quick brown fox jumps over the lazy dog and wraps onto several rows",
        "```\nThe quick brown fox jumps over the lazy dog and wraps onto several rows\n```",
        "| a | b |\n| - | - |\n| c | d |",
        "- The first list item has enough words to wrap onto several display rows",
    ] {
        for command in [">>", "<<", "V>", "V<"] {
            let (mut core, view) = open(source);
            let at = core.layout(view).unwrap().snapshot().unwrap().rows[1].text_range.start;
            place(&mut core, view, at);
            assert_eq!(core.document().list_indent_capabilities(at..at), (false, false));
            let history = core.document().history_status();
            let revision = core.document().revision();
            keys(&mut core, view, command, true);
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{command}: {source}");
            assert_eq!(core.document().revision(), revision);
            assert_eq!(core.document().history_status(), history);
            assert_eq!(core.command_state(view).unwrap().cursor(), at);
        }
    }
}

#[test]
fn counted_preview_shifts_repeat_button_presses_in_one_undo_unit() {
    let source = "- parent\n  - sibling\n    - child\n      - leaf\n  - Target words that wrap onto several display rows\n- end";
    for (command, presses) in [("V2>", 2), ("V999>", 2), ("2>>", 1)] {
        let (mut core, view) = open(source);
        let (mut native, native_view) = open(source);
        let at = core.document().text().find("Target").unwrap();
        place(&mut core, view, at);
        place(&mut native, native_view, at);
        for _ in 0..presses {
            let expected = native.list_selection_identity(native_view).unwrap();
            native.handle(native_view, CoreEvent::IndentList { expected, unindent: false }).unwrap();
        }
        keys(&mut core, view, command, true);
        assert_eq!(core.document().source_bytes(), native.document().source_bytes(), "{command}");
        keys(&mut core, view, "u", true);
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{command}");
    }
    let (mut core, view) = open("- parent\n  - child\n    - Target\n- end");
    let at = core.document().text().find("Target").unwrap();
    place(&mut core, view, at);
    keys(&mut core, view, "<<.", true);
    assert_eq!(core.document().source_bytes(), b"- parent\n  - child\n- Target\n- end");
    keys(&mut core, view, "uu", true);
    assert_eq!(core.document().source_bytes(), b"- parent\n  - child\n    - Target\n- end");
}

#[test]
fn preview_visual_character_shifts_match_the_buttons_for_selected_items() {
    let source = "- parent\n- two\n  - descendant\n- three\n- end";
    for reverse in [false, true] {
        let (mut core, view) = open(source);
        let (mut native, native_view) = open(source);
        let first = core.document().text().find("two").unwrap();
        let last = core.document().text().find("three").unwrap() + 4;
        for (core, view) in [(&mut core, view), (&mut native, native_view)] {
            place(core, view, if reverse { last } else { first });
            let revision = core.document().revision();
            core.handle(view, CoreEvent::PlaceCursor { document_revision: revision,
                text_offset: if reverse { first } else { last }, affinity: BoundaryAffinity::Downstream,
                extend_selection: true }).unwrap();
        }
        let expected = native.list_selection_identity(native_view).unwrap();
        native.handle(native_view, CoreEvent::IndentList { expected, unindent: false }).unwrap();
        keys(&mut core, view, ">", true);
        assert_eq!(core.document().source_bytes(), native.document().source_bytes());
        keys(&mut core, view, "u", true);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
