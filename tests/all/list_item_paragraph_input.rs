use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
fn editor(source: &str, format: Format) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}
fn key(core: &mut Editor, view: ViewId, key: Key) {
    let outcome = core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert!(matches!(outcome.command.as_ref().unwrap().status, CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled), "{key:?}: {outcome:?}");
}
fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for c in text.chars() { key(core, view, Key::Char(c)); }
}
fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
}
fn reopen(core: &Editor) {
    let doc = core.document();
    let reopened = Document::from_bytes(doc.source_bytes(), doc.encoding(), doc.format()).unwrap();
    assert_eq!(reopened.text(), doc.text());
    let structure = |doc: &Document| doc.projection().blocks().iter().map(|b| (b.range.clone(), b.kind.clone(), b.style.clone())).collect::<Vec<_>>();
    assert_eq!(structure(&reopened), structure(doc));
}
fn history(core: &mut Editor, view: ViewId, before: &str) {
    let after = core.document().source_bytes();
    key(core, view, Key::Escape);
    key(core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), before.as_bytes());
    key(core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), after);
}

#[test]
fn keyboard_and_native_action_split_at_caret_and_share_typing_undo() {
    for mode in [Mode::Normal, Mode::Insert, Mode::Replace] {
        for native in [false, true] {
            let source = "- AlphaBeta\n- Tail";
            let (mut core, view) = editor(source, Format::Markdown);
            if mode != Mode::Normal { keys(&mut core, view, if mode == Mode::Insert { "i" } else { "R" }); }
            place(&mut core, view, 5);
            if native {
                let expected = core.list_selection_identity(view).unwrap();
                assert!(core.can_insert_list_paragraph(view).unwrap());
                core.handle(view, CoreEvent::InsertListParagraph { expected }).unwrap();
            } else { key(&mut core, view, Key::AltEnter); }
            assert_eq!(core.document().source_bytes(), b"- Alpha\n\n  Beta\n- Tail");
            assert_eq!(core.command_state(view).unwrap().cursor(), 6);
            assert_eq!(core.command_state(view).unwrap().mode(), if mode == Mode::Replace { Mode::Replace } else { Mode::Insert });
            keys(&mut core, view, "x");
            reopen(&core);
            history(&mut core, view, source);
        }
    }
}

#[test]
fn new_empty_continuation_types_joins_and_continues_as_same_item() {
    for source in ["- first", "7) first", "> - first", "- parent\n  - first"] {
        let (mut core, view) = editor(source, Format::Markdown);
        keys(&mut core, view, "GA");
        let before = core.document().text().to_owned();
        key(&mut core, view, Key::AltEnter);
        assert_eq!(core.document().text(), format!("{before}\n"));
        reopen(&core);
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().text(), before);
        key(&mut core, view, Key::AltEnter);
        keys(&mut core, view, "second");
        key(&mut core, view, Key::Enter);
        keys(&mut core, view, "next");
        assert_eq!(core.document().text(), format!("{before}\nsecond\nnext"));
        reopen(&core);
        history(&mut core, view, source);
    }
}

#[test]
fn replace_backspace_restores_exact_source_for_keyboard_and_menu() {
    for native in [false, true] {
        let source = "- **AlphaBeta**\n- Tail";
        let (mut core, view) = editor(source, Format::Markdown);
        keys(&mut core, view, "R"); place(&mut core, view, 5);
        if native {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(view, CoreEvent::InsertListParagraph { expected }).unwrap();
        } else { key(&mut core, view, Key::AltEnter); }
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.command_state(view).unwrap().cursor(), 5);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Replace);
    }
}

#[test]
fn counted_insert_dot_macro_and_last_insert_keep_paragraph_intention() {
    let (mut core, view) = editor("- a", Format::Markdown);
    keys(&mut core, view, "3A"); key(&mut core, view, Key::AltEnter); keys(&mut core, view, "x"); key(&mut core, view, Key::Escape);
    assert_eq!(core.document().text(), "a\nx\nx\nx");
    assert_eq!(core.document().projection().list_structure().lists[0].items.len(), 1);
    key(&mut core, view, Key::Char('.'));
    assert_eq!(core.document().text(), "a\nx\nx\nx\nx\nx\nx");
    reopen(&core);
    let (mut core, view) = editor("- a", Format::Markdown);
    keys(&mut core, view, "qaA"); key(&mut core, view, Key::AltEnter); keys(&mut core, view, "x"); key(&mut core, view, Key::Escape); keys(&mut core, view, "q");
    assert!(core.command_state(view).unwrap().register('a').unwrap().text.contains("<A-Enter>"));
    assert_eq!(core.command_state(view).unwrap().register('.').unwrap().text, "\nx");
    keys(&mut core, view, "@a");
    assert_eq!(core.document().text(), "a\nx\nx");
    assert_eq!(core.document().projection().list_structure().lists[0].items.len(), 1);
    reopen(&core);
}

#[test]
fn mapping_and_literal_next_keep_alt_enter_identity() {
    let (mut core, view) = editor("- a", Format::Markdown);
    keys(&mut core, view, ":imap <A-CR> X"); key(&mut core, view, Key::Enter);
    keys(&mut core, view, "A"); key(&mut core, view, Key::AltEnter);
    assert_eq!(core.document().text(), "aX");
    key(&mut core, view, Key::Ctrl('v')); key(&mut core, view, Key::AltEnter);
    assert_eq!(core.document().text(), "aX<A-CR>");
    reopen(&core);
}

#[test]
fn stale_and_unsupported_native_requests_leave_state_unchanged() {
    for (source, format) in [("ordinary", Format::Markdown), ("- item", Format::MarkdownSource),
        ("- item", Format::PlainText), ("- ```\n  code\n  ```", Format::Markdown)] {
        let (mut core, view) = editor(source, format);
        let expected = core.list_selection_identity(view).unwrap();
        assert!(!core.can_insert_list_paragraph(view).unwrap());
        assert!(core.handle(view, CoreEvent::InsertListParagraph { expected }).is_err());
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    }
    let (mut core, view) = editor("- item", Format::Markdown);
    let expected = core.list_selection_identity(view).unwrap();
    keys(&mut core, view, "l");
    let cursor = core.command_state(view).unwrap().cursor();
    assert!(core.handle(view, CoreEvent::InsertListParagraph { expected }).is_err());
    assert_eq!(core.document().source_bytes(), b"- item");
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
}

#[test]
fn other_views_keep_mode_and_rebase_their_carets() {
    let (mut core, view) = editor("- first\n- tail", Format::Markdown);
    let other = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
    keys(&mut core, other, "GA");
    keys(&mut core, view, "A"); key(&mut core, view, Key::AltEnter);
    assert_eq!(core.command_state(other).unwrap().mode(), Mode::Insert);
    assert_eq!(core.command_state(other).unwrap().cursor(), "first\n\ntail".len());
    keys(&mut core, other, "!");
    assert_eq!(core.document().text(), "first\n\ntail!");
    reopen(&core);
}

#[test]
fn native_abi_checks_regions_and_exact_caret_before_editing() {
    use std::ptr;
    use viem_core::ffi::*;
    let fixture = super::ffi_core_surface::table_ffi_fixture(b"- first", VIEM_FORMAT_MARKDOWN);
    let (core, view) = (fixture.handle, fixture.view);
    let mut expected = ViemLogicalSelectionIdentityV1::default();
    let mut outcome = ViemCoreOutcomeV1::default();
    let mut flags = 0;
    assert_eq!(unsafe { viem_core_view_list_selection(core, view, &mut expected) }, ViemStatus::Ok);
    assert_eq!(unsafe { viem_core_view_list_indent_capabilities(core, view, &expected, &mut flags) }, ViemStatus::Ok);
    assert_ne!(flags & VIEM_LIST_CAN_INSERT_PARAGRAPH, 0);
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, ptr::null(), &mut outcome) }, ViemStatus::NullPointer);
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, &expected, ptr::null_mut()) }, ViemStatus::NullPointer);
    let mut overlapping = [0u64; 64];
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view,
        overlapping.as_ptr().cast(), overlapping.as_mut_ptr().cast()) }, ViemStatus::InvalidArgument);
    let mut invalid = expected;
    invalid.struct_size = 0;
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, &invalid, &mut outcome) }, ViemStatus::InvalidArgument);
    invalid = expected; invalid.text_start += 1;
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, &invalid, &mut outcome) }, ViemStatus::StaleRevision);
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, &expected, &mut outcome) }, ViemStatus::Ok);
    assert_eq!(unsafe { viem_core_view_insert_list_paragraph(core, view, &expected, &mut outcome) }, ViemStatus::StaleRevision);
    assert_eq!(unsafe { viem_core_view_list_indent_capabilities(core, view, &expected, &mut flags) }, ViemStatus::StaleRevision);
    assert_eq!(flags, 0);
}

#[test]
fn folded_source_break_becomes_paragraph_and_caret_follows_it() {
    for source in [
        "- first\n  second\n- tail",
        "- **first\n  second**\n- tail",
        "- [first\n  second](target)\n- tail",
        "> - first\n>   second\n> - tail",
        "- > first\n  > second\n- tail",
    ] {
    for native in [false, true] {
        let (mut core, view) = editor(source, Format::Markdown);
        keys(&mut core, view, "i"); place(&mut core, view, 5);
        if native {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(view, CoreEvent::InsertListParagraph { expected }).unwrap();
        } else { key(&mut core, view, Key::AltEnter); }
        assert_eq!(core.document().text(), "first\nsecond\ntail");
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
        keys(&mut core, view, "x");
        assert_eq!(core.document().text(), "first\nxsecond\ntail");
        reopen(&core);
        history(&mut core, view, source);
    }
    }
}

#[test]
fn loose_list_layout_after_split_matches_fresh_projection() {
    use viem_core::layout::{LayoutEngine, ViewLayout};
    let source = "- first\n- second\n- third\n\nOutside";
    let mut doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut layout = ViewLayout::new(600., 400.);
    engine.relayout(&doc, &mut layout).unwrap();
    doc.apply_model_request(viem_core::document::ModelRequest::InsertListParagraph {
        document: doc.id(), revision: doc.revision(), at: 9, affinity: BoundaryAffinity::Downstream,
    }).unwrap();
    engine.relayout(&doc, &mut layout).unwrap();
    let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut fresh_engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut fresh_layout = ViewLayout::new(600., 400.);
    fresh_engine.relayout(&fresh, &mut fresh_layout).unwrap();
    let geometry = |layout: &ViewLayout| layout.snapshot().unwrap().rows.iter().map(|row|
        (row.text_range.clone(), row.y, row.baseline, row.height(), row.paragraph_content_x,
         row.decorations.iter().map(|marker| marker.text.clone()).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert_eq!(geometry(&layout), geometry(&fresh_layout));
}
