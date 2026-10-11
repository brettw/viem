use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn setup(source: &str, offset: usize) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    key(&mut core, view, Key::Char('i'));
    core.handle_with_layout(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: offset,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    (core, view)
}

fn exact(core: &Core<MockTextMeasurementProvider>) {
    let document = core.document();
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), fresh.text());
    let blocks = |document: &Document| document.projection().blocks().iter()
        .map(|block| (block.range.clone(), block.kind.clone(), block.style.clone(), block.quote_depth, block.thematic_break))
        .collect::<Vec<_>>();
    assert_eq!(blocks(document), blocks(&fresh));
    assert_eq!(document.projection().style_spans(), fresh.projection().style_spans());
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key.clone())))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}, source {:?}", String::from_utf8_lossy(&core.document().source_bytes())));
    if let Some(command) = result.command {
        assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled), "{key:?}: {:?}", command.status);
    }
    exact(core);
}

fn undo_redo(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, initial: &str) {
    key(core, view, Key::Escape);
    let edited = core.document().source_bytes();
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), initial.as_bytes());
    exact(core);
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
    assert_eq!(core.document().source_bytes(), edited);
    exact(core);
}

#[test]
fn thematic_and_setext_boundaries_accept_backspace_and_delete() {
    for (source, at, keys, expected) in [
        ("a\n***\nb", 3, vec![Key::Backspace, Key::Backspace], "ab"),
        ("a\n=\n\nb", 1, vec![Key::Backspace], "\nb"),
        ("a\n=\n\nb", 0, vec![Key::Delete], "\nb"),
    ] {
        let (mut core, view) = setup(source, at);
        for input in keys { key(&mut core, view, input); }
        assert_eq!(core.document().text(), expected);
        undo_redo(&mut core, view, source);
    }
}

#[test]
fn hard_break_at_rules_and_whitespace_retains_following_typing() {
    for source in ["---", "***", "___", " ", "   ", "\t", "\n", "\n\n"] {
        let initial = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap().text().to_owned();
        for at in 0..=initial.len() {
            let (mut core, view) = setup(source, at);
            key(&mut core, view, Key::ShiftEnter);
            key(&mut core, view, Key::Char('z'));
            assert_eq!(core.document().text(), format!("{}\nz{}", &initial[..at], &initial[at..]), "{source:?} at {at}");
            undo_redo(&mut core, view, source);
        }
    }
}

#[test]
fn hard_break_at_setext_and_table_adjacent_paragraphs_matches_reopen() {
    for source in ["Title\n===", "Title\n---",
        "before\n\n| a | b |\n|-|-|\n| c | d |\n\nafter"] {
        let initial = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap().text().to_owned();
        for at in 0..=initial.len() {
            let (mut core, view) = setup(source, at);
            key(&mut core, view, Key::ShiftEnter);
            key(&mut core, view, Key::Char('z'));
            assert_eq!(core.document().text(), format!("{}\nz{}", &initial[..at], &initial[at..]), "{source:?} at {at}");
            undo_redo(&mut core, view, source);
        }
    }
}

#[test]
fn hard_break_replaces_rule_furniture_without_changing_adjacent_paragraphs() {
    for source in ["before\n***\nafter", "before\n\n---\n\nafter", "> ---", "<hr>"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.projection().blocks().iter().find(|block| block.thematic_break).unwrap().range.start;
        let expected = format!("{}\nz{}", &document.text()[..at], &document.text()[at..]);
        let (mut core, view) = setup(source, at);
        key(&mut core, view, Key::ShiftEnter);
        key(&mut core, view, Key::Char('z'));
        assert_eq!(core.document().text(), expected, "{source:?}");
        undo_redo(&mut core, view, source);
    }
}

#[test]
fn upstream_hard_break_at_setext_end_retains_following_paragraph() {
    let source = "a\n- \nb";
    let (mut core, view) = setup(source, 1);
    core.handle_with_layout(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: 1,
        affinity: BoundaryAffinity::Upstream, extend_selection: false,
    }).unwrap();
    key(&mut core, view, Key::ShiftEnter);
    key(&mut core, view, Key::Char('x'));
    assert_eq!(core.document().text(), "a\nx\nb");
    undo_redo(&mut core, view, source);
}

#[test]
fn nested_list_quote_fences_accept_enter_and_backspace() {
    for source in ["- > ```\n  > code\n  > ```", "> - ```\n>   code\n>   ```", "> - > ```\n>   > code\n>   > ```",
        "- > ```\n  > - literal\n  >   indented\n  > ```"] {
        for action in [Key::Enter, Key::Backspace] {
            let (mut core, view) = setup(source, 0);
            let initial = core.document().text().to_owned();
            let kind = core.document().projection().blocks()[0].kind.clone();
            let quote_depth = core.document().projection().blocks()[0].quote_depth;
            key(&mut core, view, action.clone());
            assert_eq!(core.document().text(), if action == Key::Enter { format!("\n{initial}") } else { initial });
            let block = &core.document().projection().blocks()[0];
            if action == Key::Backspace {
                assert_ne!(block.style.0, "Code Block");
                assert_eq!(block.kind, kind);
                assert_eq!(block.quote_depth, quote_depth);
            }
            undo_redo(&mut core, view, source);
        }
    }
}

#[test]
fn nested_list_quote_fence_language_changes_only_the_annotation() {
    let source = "- > ```\n  > code\n  > ```";
    let (mut core, view) = setup(source, 0);
    core.set_markdown_code_language(view,
        core.document().id(), core.document().revision(), 0, Some("rust")).unwrap();
    assert_eq!(core.document().source_bytes(), b"- > ```rust\n  > code\n  > ```");
    exact(&core);
    undo_redo(&mut core, view, source);
}
