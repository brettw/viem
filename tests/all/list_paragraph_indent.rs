//! Tab and Shift-Tab at a paragraph's first position move that paragraph into
//! the list before it, spelled as Option-Enter would have, or out of every list.
use super::markdown_test_support::EncodedFixture;
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    measure_document_work, BlockKind, BoundaryAffinity, Encoding, FileFormat, Format, ModelRequest,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}

#[track_caller]
fn key(core: &mut Editor, view: ViewId, key: Key) {
    let outcome = core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    assert!(matches!(outcome.command.as_ref().unwrap().status, CommandStatus::Complete | CommandStatus::Pending),
        "{key:?}: {outcome:?}");
}

#[track_caller]
fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for c in text.chars() { key(core, view, Key::Char(c)); }
}

fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
}

fn source(core: &Editor) -> String {
    String::from_utf8(core.document().source_bytes()).unwrap()
}

fn last_kind(core: &Editor) -> BlockKind {
    core.document().projection().blocks().last().unwrap().kind.clone()
}

#[track_caller]
fn reopen(core: &Editor) {
    let doc = core.document();
    let reopened = Document::from_bytes(doc.source_bytes(), doc.encoding(), doc.format()).unwrap();
    assert_eq!(reopened.text(), doc.text());
    let structure = |doc: &Document| doc.projection().blocks().iter()
        .map(|b| (b.range.clone(), b.kind.clone(), b.style.clone())).collect::<Vec<_>>();
    assert_eq!(structure(&reopened), structure(doc));
}

/// Press the key with the caret at `at` in Insert mode, returning the caret.
#[track_caller]
fn press_at(source: &str, at: usize, pressed: Key) -> (Editor, usize) {
    let (mut core, view) = editor(source);
    keys(&mut core, view, "i");
    place(&mut core, view, at);
    key(&mut core, view, pressed);
    let cursor = core.command_state(view).unwrap().cursor();
    (core, cursor)
}

/// A paragraph move keeps the caret at the paragraph's start.
#[track_caller]
fn move_at(source: &str, at: usize, pressed: Key) -> Editor {
    let (core, cursor) = press_at(source, at, pressed);
    assert_eq!(cursor, at, "{source:?}");
    core
}

fn request(document: &Document, at: usize, unindent: bool) -> ModelRequest {
    ModelRequest::IndentListParagraph { document: document.id(), revision: document.revision(), at, unindent }
}

#[test]
fn tab_after_leaving_a_list_matches_option_enter_and_shift_tab_leaves_again() {
    // Each source exits its last item with one Backspace per nesting level.
    // Leaving the nested ordered item spells its paragraph with blank rows,
    // and moving it keeps the document's final terminator.
    for (original, exits, terminated) in [("- one\n- two", 1, false), ("- one\n  - two", 2, false),
        ("1. one\n2. two", 1, false), ("- one\n  1) two", 2, true)]
    {
        // Option-Enter at the end of the last item is the reference spelling.
        let (mut core, view) = editor(original);
        keys(&mut core, view, "GA");
        key(&mut core, view, Key::AltEnter);
        keys(&mut core, view, "x");
        let continued = source(&core) + if terminated { "\n" } else { "" };
        let (mut core, view) = editor(original);
        keys(&mut core, view, "GA");
        key(&mut core, view, Key::Enter);
        for _ in 0..exits { key(&mut core, view, Key::Backspace); }
        keys(&mut core, view, "x");
        let (outside, outside_text) = (source(&core), core.document().text().to_owned());

        let (mut core, view) = editor(original);
        keys(&mut core, view, "GA");
        key(&mut core, view, Key::Enter);
        for _ in 0..exits { key(&mut core, view, Key::Backspace); }
        assert_eq!(last_kind(&core), BlockKind::Paragraph, "{original:?}");
        key(&mut core, view, Key::Tab);
        assert!(matches!(last_kind(&core), BlockKind::ListItem { item_start: false, .. }), "{original:?}");
        keys(&mut core, view, "x");
        assert_eq!(source(&core), continued, "{original:?}");
        reopen(&core);
        key(&mut core, view, Key::Left);
        key(&mut core, view, Key::BackTab);
        let lifted = source(&core);
        // Typing into bare blank rows spells the paragraph without the
        // passive body that the moved paragraph retains.
        if !terminated { assert_eq!(lifted, outside, "{original:?}"); }
        assert_eq!(core.document().text(), outside_text, "{original:?}");
        assert_eq!(last_kind(&core), BlockKind::Paragraph, "{original:?}");
        reopen(&core);
        // Explicit movement closed the typing unit before Shift-Tab.
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, "u");
        assert_eq!(source(&core), continued, "{original:?}");
        keys(&mut core, view, "u");
        assert_eq!(source(&core), original, "{original:?}");
        key(&mut core, view, Key::Ctrl('r'));
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(source(&core), lifted, "{original:?}");
    }
}

#[test]
fn every_paragraph_row_moves_by_the_item_indentation_in_each_encoding() {
    let outside = "- one\n  - two\n\n**para**\nmore\n\nafter";
    let inside = "- one\n  - two\n\n    **para**\n    more\n\nafter";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let fixture = EncodedFixture { encoding, file_format, bom: encoding != Encoding::Latin1 };
            let mut document = fixture.open(outside);
            let at = document.text().find("para").unwrap();
            document.apply_model_request(request(&document, at, false)).unwrap();
            fixture.assert_history(&mut document, &fixture.bytes(outside), &fixture.bytes(inside));
            document.apply_model_request(request(&document, at, true)).unwrap();
            fixture.assert_history(&mut document, &fixture.bytes(inside), &fixture.bytes(outside));
        }
    }
}

#[test]
fn quotes_around_the_list_or_inside_the_paragraph_are_retained() {
    for (outside, inside) in [
        ("> - one\n>\n> para", "> - one\n>\n>   para"),
        ("> - one\n>   - two\n>\n> para\n> more", "> - one\n>   - two\n>\n>     para\n>     more"),
        ("- one\n\n> para\n> more", "- one\n\n  > para\n  > more"),
    ] {
        let at = editor(outside).0.document().text().find("para").unwrap();
        let core = move_at(outside, at, Key::Tab);
        assert_eq!(source(&core), inside);
        reopen(&core);
        let core = move_at(inside, at, Key::BackTab);
        assert_eq!(source(&core), outside);
        reopen(&core);
    }
    // A quote marker without its optional space still reaches the item body.
    let core = move_at("> - one\n>\n>para", 4, Key::Tab);
    assert_eq!(source(&core), "> - one\n>\n>   para");
    reopen(&core);
}

#[test]
fn lifting_a_paragraph_splits_its_list_before_the_following_item() {
    for (inside, outside, rejoined) in [
        ("- one\n\n  para\n- two", "- one\n\npara\n\n- two", "- one\n\n  para\n\n- two"),
        // Splitting restarts the later run, as removing any list treatment
        // does; rejoining continues the numbering again.
        ("1. one\n\n   para\n2. two", "1. one\n\npara\n\n1. two", "1. one\n\n   para\n\n2. two"),
        ("> 1. one\n>\n>    para\n> 2. two", "> 1. one\n>\n> para\n>\n> 1. two", "> 1. one\n>\n>    para\n>\n> 2. two"),
        // Blank rows after the list are not list content.
        ("- one\n\n  para\n\n\n", "- one\n\npara\n\n\n", "- one\n\n  para\n\n\n"),
    ] {
        let core = move_at(inside, 4, Key::BackTab);
        assert_eq!(source(&core), outside);
        reopen(&core);
        let core = move_at(outside, 4, Key::Tab);
        assert_eq!(source(&core), rejoined);
        reopen(&core);
    }
}

#[test]
fn other_positions_and_paragraphs_keep_their_tab_behavior() {
    // Inside a paragraph after a list, or at a paragraph without a list
    // before it, Tab is still text.
    let (core, _) = press_at("- one\n\npara", 6, Key::Tab);
    assert_eq!(core.document().text(), "one\npa\tra");
    let (core, _) = press_at("first\n\npara", 6, Key::Tab);
    assert_eq!(core.document().text(), "first\n\tpara");
    // A continuation paragraph's item still moves from inside the
    // paragraph, and from its start with Tab.
    let core = move_at("- a\n- b\n\n  para", 4, Key::Tab);
    assert_eq!(source(&core), "- a\n  - b\n\n    para");
    let (core, _) = press_at("- a\n  - b\n\n    para", 6, Key::BackTab);
    assert_eq!(source(&core), "- a\n- b\n\n  para");
    // Lifting would leave later content of the list without an owner, so
    // Shift-Tab still moves the owning item, as from inside the paragraph.
    for retained in ["- one\n\n  para\n\n  more", "- one\n  - two\n\n    para\n\n  - three",
        "- one\n  - two\n\n    para\n\n    more\n- four", "- one\n\n  para\n\n  - two"]
    {
        let at = editor(retained).0.document().text().find("para").unwrap();
        let (inside, _) = press_at(retained, at + 2, Key::BackTab);
        let start = move_at(retained, at, Key::BackTab);
        assert_eq!(source(&start), source(&inside), "{retained:?}");
    }
    // Code keeps literal indentation input.
    let (core, _) = press_at("- one\n\n```\ncode\n```", 4, Key::Tab);
    assert_eq!(core.document().text(), "one\n\tcode");
    assert_eq!(last_kind(&core), BlockKind::Paragraph);
}

#[test]
fn blank_rows_after_a_list_become_an_editable_continuation() {
    for (outside, inside, lifted) in [
        ("- one\n- two\n\n", "- one\n- two\n\n  <span></span>\n", "- one\n- two\n\n<span></span>\n"),
        ("- one\n- two\n\n\n", "- one\n- two\n\n  <span></span>\n", "- one\n- two\n\n<span></span>\n"),
        ("- one\r\n- two\r\n\r\n", "- one\r\n- two\r\n\r\n  <span></span>\r\n", "- one\r\n- two\r\n\r\n<span></span>\r\n"),
    ] {
        let core = move_at(outside, 8, Key::Tab);
        assert_eq!(source(&core), inside);
        assert_eq!(core.document().text(), "one\ntwo\n");
        assert!(matches!(last_kind(&core), BlockKind::ListItem { item_start: false, .. }));
        reopen(&core);
        let core = move_at(inside, 8, Key::BackTab);
        assert_eq!(source(&core), lifted);
        assert_eq!(last_kind(&core), BlockKind::Paragraph);
        reopen(&core);
    }
}

#[test]
fn repeat_resolves_the_paragraph_move_again() {
    let (mut core, view) = editor("- one\n\npara\n\nmore");
    place(&mut core, view, 4);
    keys(&mut core, view, "i");
    key(&mut core, view, Key::Tab);
    keys(&mut core, view, "x");
    key(&mut core, view, Key::Escape);
    assert_eq!(source(&core), "- one\n\n  xpara\n\nmore");
    let more = core.document().text().find("more").unwrap();
    place(&mut core, view, more);
    keys(&mut core, view, ".");
    assert_eq!(source(&core), "- one\n\n  xpara\n\n  xmore");
    reopen(&core);
    keys(&mut core, view, "u");
    assert_eq!(source(&core), "- one\n\n  xpara\n\nmore");
}

#[test]
fn moving_a_paragraph_stays_local_beside_a_large_suffix() {
    let mut bounds = Vec::new();
    for count in [16, 8_000] {
        let source = format!("- one\n\npara\n\n{}", "Unrelated paragraph text.\n\n".repeat(count));
        let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        for unindent in [false, true] {
            let (prepared, work) = measure_document_work(|| document.prepare_model_request(request(&document, 4, unindent)));
            document.commit_model_transaction(prepared.unwrap()).unwrap();
            assert_eq!(work.full_projection_candidates, 0, "{work:?}");
            assert_eq!(work.source_full_materializations, 0, "{work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
            bounds.push((work.source_decoded_bytes, work.projected_formatted_bytes));
        }
    }
    assert_eq!(bounds[..2], bounds[2..], "work must not grow with the suffix");
}

