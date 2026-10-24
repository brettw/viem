use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{Association, BoundaryAffinity, DeletionRecovery, FileFormat, HistoryNavigationRequest, ModelRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, DocumentMode, Encoding, Format, ViewId};
use super::markdown_test_support::EncodedFixture;

type Editor = Core<MockTextMeasurementProvider>;
fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
}
fn keys(core: &mut Editor, view: ViewId, keys: &str) {
    for key in keys.chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
    }
}
fn gutter(core: &mut Editor, view: ViewId, at: usize) {
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let row = snapshot.rows.iter().find(|row| row.text_range == (at..at)).unwrap();
    let event = CoreEvent::SelectPointerRow { document_revision: core.document().revision(),
        layout_revision: snapshot.revision, y: row.y + row.height() / 2., extend_selection: false };
    core.handle(view, event).unwrap();
    let commands = core.command_state(view).unwrap();
    assert_eq!(commands.mode(), Mode::VisualLine);
    assert_eq!(commands.cursor(), at);
    let caret = commands.caret_target(core.document());
    assert!(core.layout(view).unwrap().snapshot().unwrap()
        .logical_endpoint_geometry(caret.offset(), caret.affinity()).is_ok());
}

#[test]
fn deleting_empty_rows_preserves_unselected_structure_and_exact_history() {
    for neighbor in ["## Heading", "Heading\n-------", "- item\n  - child", "> Quoted", "```rust\ncode\n```", "---", "| a | b |\n| - | - |\n| c | d |", "<div>HTML</div>"] {
        for count in [4, 5, 6] {
            for action in ["dd", "Vd", "gutter-d", "gutter-delete", "ex"] {
                let source = format!("before{}{}", "\n".repeat(count), neighbor);
                let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
                place(&mut core, view, 7);
                match action {
                    "gutter-d" | "gutter-delete" => {
                        gutter(&mut core, view, 7);
                        if action == "gutter-d" { keys(&mut core, view, "d"); }
                        else { core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Delete))).unwrap(); }
                    },
                    "ex" => {
                        keys(&mut core, view, ":delete");
                        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
                    },
                    _ => keys(&mut core, view, action),
                }
                let after = core.document().source_bytes();
                let expected = format!("before{}{}", "\n".repeat(if count == 6 { 4 } else { 2 }), neighbor);
                assert_eq!(after, expected.as_bytes(), "{action} count={count} neighbor={neighbor:?}");
                let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
                assert_eq!(core.document().text(), reopened.text());
                let signature = |document: &Document| document.projection().blocks().iter()
                    .map(|block| (block.range.clone(), block.kind.clone(), block.style.clone(), block.thematic_break)).collect::<Vec<_>>();
                assert_eq!(signature(core.document()), signature(&reopened));
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                assert_eq!(core.document().source_bytes(), after);
            }
        }
    }
}

#[test]
fn empty_row_deletion_preserves_encoding_and_mixed_endings_while_join_keeps_its_policy() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let fixture = EncodedFixture { encoding, file_format, bom: encoding != Encoding::Latin1 };
            let source = "before\n\n\n\n\n## Heading";
            let mut document = fixture.open(source);
            document.delete_lines(7..8).unwrap();
            fixture.assert_history(&mut document, &fixture.bytes(source), &fixture.bytes("before\n\n## Heading"));
        }
    }
    let source = b"before\r\n\n\r\n\n\r\n## Heading\r\n";
    let mut document = Document::from_bytes_with_file_format(source.to_vec(), Encoding::Utf8, Format::Markdown, FileFormat::Dos).unwrap();
    document.delete_lines(7..8).unwrap();
    assert_eq!(document.source_bytes(), b"before\r\n\n## Heading\r\n");
    let mut core = Core::new(Document::from_bytes(b"before\n\n\n\n\n## Heading".to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    place(&mut core, view, 7);
    keys(&mut core, view, "J");
    assert_eq!(core.document().source_bytes(), b"before\n\nHeading");
}

#[test]
fn preview_switches_retain_empty_owner_boundaries_in_all_views_and_history() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let fixture = EncodedFixture { encoding, file_format, bom: true };
            let source = "before\n\n\n\n\n## Heading";
            let mut core = Core::new(fixture.open(source));
            let first = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
            let second = core.add_view(MockTextMeasurementProvider::new(), 400., 400.);
            place(&mut core, first, 7);
            keys(&mut core, second, "i");
            place(&mut core, second, 7);
            for (mode, at) in [(DocumentMode::Code("markdown".into()), 8), (DocumentMode::Markdown, 7)] {
                core.handle(first, CoreEvent::SetDocumentMode { document: core.document().id(),
                    revision: core.document().revision(), mode }).unwrap();
                for view in [first, second] { assert_eq!(core.command_state(view).unwrap().cursor(), at, "{fixture:?}"); }
                assert_eq!(core.document().source_bytes(), fixture.bytes(source));
                assert!(!core.document().is_dirty());
            }
            core.handle(first, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.command_state(first).unwrap().cursor(), 8);
            core.handle(first, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.command_state(first).unwrap().cursor(), 7);
        }
    }
}

#[test]
fn empty_owner_maps_compose_and_invert_without_deletion_recovery() {
    let mut document = Document::from_bytes(b"before\n\n\n\n\n## Heading".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let anchors = [Association::BeforeInsertion, Association::AfterInsertion].into_iter().flat_map(|association| {
        [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream].into_iter().map(move |affinity| (association, affinity))
    }).map(|(association, affinity)| document.text_anchor(document.text_point(7).unwrap(), association, affinity,
        DeletionRecovery::Unresolvable).unwrap()).collect::<Vec<_>>();
    let prepared = document.prepare_model_request(ModelRequest::SetViewFormat { document: document.id(), revision: document.revision(), target: Format::Code }).unwrap();
    let forward = prepared.text_position_map().clone();
    document.commit_model_transaction(prepared).unwrap();
    let prepared = document.prepare_model_request(ModelRequest::SetViewFormat { document: document.id(), revision: document.revision(), target: Format::Markdown }).unwrap();
    let composed = forward.then(prepared.text_position_map()).unwrap();
    for anchor in anchors {
        let literal = *forward.map_text_anchor(anchor).unwrap().value().unwrap();
        assert_eq!(literal.offset(), 8);
        assert_eq!(forward.inverted().unwrap().map_text_anchor(literal).unwrap().value().unwrap().offset(), 7);
        assert_eq!(composed.map_text_anchor(anchor).unwrap().value().unwrap().offset(), 7);
    }
}

#[test]
fn leading_trailing_and_whitespace_empty_rows_keep_their_neighbors() {
    for (source, at, expected) in [
        ("\n\n## Heading", 0, "## Heading"),
        ("## Heading\n\n\n\n", 8, "## Heading\n\n"),
        ("## Heading\n\n\n\n", 9, "## Heading\n\n"),
        ("before\n\n  \n \n\n## Heading", 7, "before\n\n## Heading"),
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        place(&mut core, view, at);
        keys(&mut core, view, "dd");
        assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{source:?} at={at}");
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn deleting_final_wrapped_row_keeps_the_preceding_hard_line_and_wraps() {
    let source = "first\nseveral words wrap across the final hard line";
    for action in ["dd", "Vd"] {
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 130., 600.);
        let at = core.layout(view).unwrap().snapshot().unwrap().rows.last().unwrap().text_range.start;
        assert!(at > source.find('\n').unwrap() + 1);
        place(&mut core, view, at);
        keys(&mut core, view, action);
        assert_eq!(core.document().source_bytes(), source[..at].as_bytes(), "{action}");
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
