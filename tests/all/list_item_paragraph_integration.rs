//! Same-item paragraph authoring through existing list editing and rich paste.
use std::collections::HashMap;
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BlockKind, BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn editor(source: &str) -> (Editor, ViewId) {
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    let source = core.document().source_bytes();
    let outcome = core.handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}; source={:?}", String::from_utf8_lossy(&source)));
    assert!(matches!(outcome.command.unwrap().status, CommandStatus::Complete | CommandStatus::Pending), "{key:?}");
}

fn keys(core: &mut Editor, view: ViewId, text: &str) {
    for ch in text.chars() { key(core, view, Key::Char(ch)); }
}

fn place(core: &mut Editor, view: ViewId, at: usize) {
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
}

fn reopen(document: &Document) {
    let fresh = Document::from_bytes(document.source_bytes(), document.encoding(), document.format()).unwrap();
    assert_eq!(fresh.text(), document.text());
    let shape = |document: &Document| {
        let mut owners = HashMap::new();
        document.projection().blocks().iter().map(|block| {
            let path = block.containers.iter().map(|member| {
                let next = owners.len();
                let owner = *owners.entry(member.container.id).or_insert(next);
                (owner, member.container.kind, member.starts_here, member.ends_here)
            }).collect::<Vec<_>>();
            (block.range.clone(), block.kind.clone(), block.style.clone(), block.list_loose, path)
        }).collect::<Vec<_>>()
    };
    assert_eq!(shape(&fresh), shape(document));
    let item_paragraphs = |document: &Document| {
        let blocks = document.projection().blocks();
        document.projection().list_structure().lists.iter().map(|list| {
            list.items.iter().map(|item| item.paragraph_ids.iter().map(|id| {
                blocks.iter().position(|block| block.id == *id).unwrap()
            }).collect::<Vec<_>>()).collect::<Vec<_>>()
        }).collect::<Vec<_>>()
    };
    assert_eq!(item_paragraphs(&fresh), item_paragraphs(document));
}

fn history(core: &mut Editor, view: ViewId, before: &[u8]) {
    let after = core.document().source_bytes();
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), before);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), after);
}

#[test]
fn new_and_raw_empty_continuations_move_together_with_tab_and_backtab() {
    for source in [
        "- before\n- first\n\n\n\n- tail",
        "8. before\n9. first\n\n\n\n10. tail",
        "- parent\n  - before\n  - first\n\n\n\n  - tail",
    ] {
        let (mut core, view) = editor(source);
        let first = core.document().text().find("first").unwrap();
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, first + 5);
        key(&mut core, view, Key::AltEnter);
        key(&mut core, view, Key::Escape);
        let authored = core.document().source_bytes();
        let text = core.document().text().to_owned();
        let blocks = core.document().projection().blocks();
        assert!(blocks.iter().any(|block| block.range.is_empty() && block.kind == BlockKind::Paragraph));
        let continuation = blocks.iter().find(|block| block.range.start == first + 6).unwrap();
        assert!(matches!(continuation.kind, BlockKind::ListItem { item_start: false, .. }));
        let first_id = blocks.iter().find(|block| block.range.start == first).unwrap().id;
        let owned = core.document().projection().list_structure().lists.iter().flat_map(|list| &list.items)
            .find(|item| item.paragraph_ids.contains(&first_id)).unwrap().paragraph_ids.clone();
        assert_eq!(owned.len(), 3, "{source:?}");
        for action in [Key::Tab, Key::BackTab] {
            key(&mut core, view, Key::Char('i'));
            place(&mut core, view, first + 6);
            let before = core.document().source_bytes();
            key(&mut core, view, action);
            key(&mut core, view, Key::Escape);
            assert_eq!(core.document().text(), text, "{source:?} {action:?}");
            assert!(core.document().projection().list_structure().lists.iter().flat_map(|list| &list.items)
                .any(|item| item.paragraph_ids == owned), "{source:?} {action:?}");
            reopen(core.document());
            history(&mut core, view, &before);
        }
        assert_eq!(core.document().source_bytes(), authored, "{source:?}");
    }
}

#[test]
fn dd_retains_both_unselected_empty_continuations_after_same_item_split() {
    for source in ["- first\n\n\n\n- tail", "8. first\n\n\n\n9. tail"] {
        let (mut core, view) = editor(source);
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, 5);
        key(&mut core, view, Key::AltEnter);
        key(&mut core, view, Key::Escape);
        let before = core.document().source_bytes();
        let empty_ids = core.document().projection().blocks().iter().filter(|block| block.range.is_empty())
            .map(|block| block.id).collect::<Vec<_>>();
        assert_eq!(empty_ids.len(), 2);
        place(&mut core, view, 0);
        keys(&mut core, view, "\"add");
        assert_eq!(core.document().text(), "\n\ntail", "{source:?}");
        assert_eq!(core.command_state(view).unwrap().register('a').unwrap().text, "first\n");
        assert_eq!(core.document().projection().blocks().iter().filter(|block| block.range.is_empty())
            .map(|block| block.id).collect::<Vec<_>>(), empty_ids);
        reopen(core.document());
        history(&mut core, view, &before);
    }
}

#[test]
fn enter_from_authored_empty_continuation_keeps_ordered_run_with_raw_empty_rows() {
    let source = "8. first\n\n\n\n9. last\n10. tail";
    let (mut core, view) = editor(source);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 5);
    key(&mut core, view, Key::AltEnter);
    key(&mut core, view, Key::Enter);
    keys(&mut core, view, "added");
    key(&mut core, view, Key::Escape);
    let saved = String::from_utf8(core.document().source_bytes()).unwrap();
    assert!(saved.contains("\n9. added"), "{saved:?}");
    assert!(saved.contains("\n10. last"), "{saved:?}");
    assert!(saved.contains("\n11. tail"), "{saved:?}");
    let ordinals = core.document().projection().blocks().iter().filter_map(|block| match block.kind {
        BlockKind::ListItem { ordered: true, ordinal, item_start: true, .. } => Some(ordinal),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(ordinals, [8, 9, 10, 11]);
    reopen(core.document());
    history(&mut core, view, source.as_bytes());
}

#[test]
fn rich_whitespace_paste_into_new_empty_continuation_keeps_later_bold_text() {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::RegisterValue;
    let donor = Document::from_bytes(b"line  \n  \nafter **double** space".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let register = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_register(register),
    ));
    for source in ["- first\n- tail", "- first\n\n\n\n- tail"] {
        let (mut core, view) = editor(source);
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, 5);
        key(&mut core, view, Key::AltEnter);
        let before_text = core.document().text().to_owned();
        let authored = core.document().source_bytes();
        core.handle(view, CoreEvent::InputWithClipboard {
            input: InputEvent::Key(Key::PasteClipboard), clipboard: clipboard.clone(),
        }).unwrap();
        key(&mut core, view, Key::Escape);
        let expected = format!("{}{}{}", &before_text[..6], copied, &before_text[6..]);
        assert_eq!(core.document().text(), expected);
        let bold = 6 + copied.find("double").unwrap();
        assert!(DocumentLayoutStyles::semantic_character_at(core.document().projection(), bold, false).unwrap().bold);
        reopen(core.document());
        let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert!(DocumentLayoutStyles::semantic_character_at(fresh.projection(), bold, false).unwrap().bold);
        history(&mut core, view, &authored);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn same_item_split_accepts_a_raw_owned_empty_paragraph() {
    for source in [
        "- a\n\n\n\n- b",
        "8. a\n\n\n\n9. b",
        "- parent\n  - a\n\n\n\n  - b",
    ] {
        let (mut core, view) = editor(source);
        let at = core.document().projection().blocks().iter()
            .find(|block| block.range.is_empty() && block.kind == BlockKind::Paragraph).unwrap().range.start;
        let original = core.document().text().to_owned();
        let items = core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>();
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, at);
        assert!(core.can_insert_list_paragraph(view).unwrap(), "{source:?}");
        key(&mut core, view, Key::AltEnter);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), format!("{}\n{}", &original[..at], &original[at..]));
        assert_eq!(core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>(), items);
        let blocks = core.document().projection().blocks();
        for offset in [at, at + 1] {
            assert!(matches!(blocks.iter().find(|block| block.range.start == offset).unwrap().kind,
                BlockKind::ListItem { item_start: false, .. }));
        }
        reopen(core.document());
        history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn enter_on_empty_first_paragraph_retains_body_after_raw_owned_empty() {
    for source in [
        "- <span></span>\n\n\n\n  continued\n- next",
        "8. <span></span>\n\n\n\n   continued\n9. next",
        "- <span></span>\n\n\n\n  - child\n- next",
    ] {
        let (mut core, view) = editor(source);
        let original = core.document().text().to_owned();
        let items = core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>();
        key(&mut core, view, Key::Char('i'));
        key(&mut core, view, Key::Enter);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), format!("\n{original}"));
        assert_eq!(core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>(), items + 1);
        assert!(matches!(core.document().projection().blocks()[0].kind, BlockKind::ListItem { item_start: true, .. }));
        reopen(core.document());
        history(&mut core, view, source.as_bytes());
    }
}

#[test]
fn enter_from_raw_owned_empty_continuation_starts_the_next_item() {
    for source in [
        "- a\n\n\n\n- b",
        "8. a\n\n\n\n9. b",
        "- parent\n  - a\n\n\n\n  - b",
    ] {
        let (mut core, view) = editor(source);
        let at = core.document().projection().blocks().iter()
            .find(|block| block.range.is_empty() && block.kind == BlockKind::Paragraph).unwrap().range.start;
        let original = core.document().text().to_owned();
        let items = core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>();
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, at);
        key(&mut core, view, Key::Enter);
        keys(&mut core, view, "added");
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), format!("{}\nadded{}", &original[..at], &original[at..]));
        assert_eq!(core.document().projection().list_structure().lists.iter().map(|list| list.items.len()).sum::<usize>(), items + 1);
        let added = core.document().projection().blocks().iter().find(|block| block.range.start == at + 1).unwrap();
        assert!(matches!(added.kind, BlockKind::ListItem { item_start: true, .. }));
        reopen(core.document());
        history(&mut core, view, source.as_bytes());
    }
}
