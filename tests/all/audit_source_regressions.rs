use viem_core::command::{CommandStatus, InputEvent, Key, NavigationKey};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest, TableEditIntent};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    (core, view)
}

fn exact(core: &Core<MockTextMeasurementProvider>) {
    let document = core.document();
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, document.format()).unwrap();
    assert_eq!(document.text(), fresh.text(), "source {:?}", String::from_utf8_lossy(&document.source_bytes()));
    let blocks = |document: &Document| document.projection().blocks().iter()
        .map(|block| (block.kind.clone(), block.range.clone(), block.quote_depth, block.style.clone(), block.markdown_html, block.thematic_break))
        .collect::<Vec<_>>();
    assert_eq!(blocks(document), blocks(&fresh), "source {:?}", String::from_utf8_lossy(&document.source_bytes()));
    assert_eq!(document.projection().style_spans(), fresh.projection().style_spans());
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key.clone())))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}; source {:?}; text {:?}; cursor {}",
            String::from_utf8_lossy(&core.document().source_bytes()), core.document().text(), core.command_state(view).unwrap().cursor()));
    if let Some(command) = outcome.command {
        assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled), "{key:?}: {:?}", command.status);
    }
    exact(core);
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, offset: usize, extend: bool) {
    core.handle_with_layout(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: offset,
        affinity: BoundaryAffinity::Downstream, extend_selection: extend,
    }).unwrap();
}

#[test]
fn table_last_row_delete_and_final_break_match_reopen() {
    for source in ["| a |\n| - |\n| 1 |", "> | a |\n> | - |\n> | 1 |", "- | a |\n  | - |\n  | 1 |"] {
        let (mut core, view) = open(source);
        let table = core.document().projection().tables()[0].id;
        core.handle_with_layout(view, CoreEvent::TableEdit {
            document: core.document().id(), revision: core.document().revision(),
            intent: TableEditIntent::DeleteRow {table, row: 1},
        }).unwrap();
        exact(&core);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        let end = core.document().text().len();
        place(&mut core, view, end, false);
        key(&mut core, view, Key::Char('i'));
        key(&mut core, view, Key::ShiftEnter);
    }
}

#[test]
fn indent_does_not_treat_unrelated_nested_lists_as_siblings() {
    let source = "> - > - > a\n> - > - > b";
    let (mut core, view) = open(source);
    key(&mut core, view, Key::Char('i'));
    place(&mut core, view, 12, false);
    key(&mut core, view, Key::Tab);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.command_state(view).unwrap().cursor(), 12);
    assert_eq!(core.document().list_indent_capabilities(12..12), (false, false));
}

#[test]
fn fence_and_html_delimiter_edits_match_reopen() {
    for source in ["```\ncode\n```", "~~~\ncode\n~~~", "<div>\nblock\n</div>", "```\n\n\n```", "text\n```\ncode\n```\ntext"] {
        let (mut core, view) = open(source);
        let offset = source.find(['`', '~', '<']).unwrap() + 1;
        place(&mut core, view, offset, false);
        key(&mut core, view, Key::Char('i'));
        key(&mut core, view, Key::Enter);
        key(&mut core, view, Key::Backspace);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let source = "```\n\n\n```";
    let (mut core, view) = open(source);
    place(&mut core, view, 2, false);
    for action in [Key::Char('i'), Key::Char('x'), Key::Char('y'), Key::Backspace, Key::Backspace] {
        key(&mut core, view, action);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn list_table_boundary_enter_and_edits_match_reopen() {
    for source in ["1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b", "- item\n\n  | a | b | c | d |\n  |---|---|---|---|\n  | 1 | 2 | 3 | 4 |\n\n- next"] {
        for sequence in [vec![Key::Char('i'), Key::Enter], vec![Key::Char('x')], vec![Key::Char('X')],
            vec![Key::Char('3'),Key::Char('x')], vec![Key::Char('d'), Key::Char('w')],
            vec![Key::Char('d'), Key::Char('e')], vec![Key::Char('c'),Key::Char('w')],
            vec![Key::Char('s')], vec![Key::Char('v'),Key::Char('d')],
            vec![Key::Char('<'),Key::Char('<')], vec![Key::Char('='),Key::Char('=')],
            vec![Key::Char('i'),Key::Backspace], vec![Key::Char('a'),Key::Delete],
            vec![Key::Ctrl('v'),Key::Char('d')]] {
            let (mut core, view) = open(source);
            let at = if sequence.len() == 2 && sequence[1] == Key::Enter {8} else {14};
            place(&mut core, view, at, false);
            for action in sequence { key(&mut core, view, action); }
            key(&mut core, view, Key::Escape);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            exact(&core);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            exact(&core);
        }
    }
}

#[test]
fn nested_table_structural_edits_match_reopen_in_both_views() {
    let source = "1. a\n\n   | x | y |\n   | - | - |\n   | 1 | 2 |\n\n2. b";
    for format in [Format::Markdown,Format::MarkdownSource] {
        for operation in 0..4 {
            let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap());
            let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
            let table = core.document().projection().tables()[0].id;
            let intent = match operation {
                0 => TableEditIntent::InsertRow {table,row: 1,after: true},
                1 => TableEditIntent::DeleteRow {table,row: 1},
                2 => TableEditIntent::InsertColumn {table,column: 0,after: true},
                _ => TableEditIntent::DeleteColumn {table,column: 0},
            };
            core.handle_with_layout(view,CoreEvent::TableEdit {
                document: core.document().id(),revision: core.document().revision(),intent,
            }).unwrap();
            exact(&core);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(),source.as_bytes());
            exact(&core);
        }
    }
}

#[test]
fn indented_code_enter_then_backspace_matches_reopen() {
    let (mut core, view) = open("    a\n\n    b");
    let end = core.document().text().len();
    place(&mut core, view, end, false);
    key(&mut core, view, Key::Char('i'));
    key(&mut core, view, Key::Enter);
    key(&mut core, view, Key::Backspace);
}

#[test]
fn selections_across_source_syntax_split_without_reversed_ranges() {
    for (source, start, end) in [("a\n- \nb",4,6), ("*\ta\n*\tb",1,7), ("*it*",1,3), ("**bold**",2,6)] {
        let (mut core, view) = open(source);
        place(&mut core, view, start, false);
        place(&mut core, view, end, true);
        key(&mut core, view, Key::Enter);
    }
}

#[test]
fn indent_nonlist_source_selections_never_reports_invalid_ranges() {
    for (source,start,end) in [("a  \nb",2,3),("a<br>b",2,3),("x<br/>\ny",2,3),("![a](u \"t\")![b](v)",1,3)] {
        let (mut core, view) = open(source);
        place(&mut core, view, start, false);
        place(&mut core, view, end, true);
        let expected = core.list_selection_identity(view).unwrap();
        let result = core.handle_with_layout(view, CoreEvent::IndentList {expected, unindent: false});
        if let Err(error) = result {
            let error = format!("{error:?}");
            assert!(!error.contains("InvalidRange") && !error.contains("NotGraphemeBoundary"), "{source}: {error}");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
        exact(&core);
    }
}

#[test]
fn ordinary_paragraph_before_table_accepts_structural_reset() {
    let source = "text before\n| a | b |\n| - | - |\n| 1 | 2 |";
    for list in [false, true] {
        let (mut core, view) = open(source);
        let expected = core.list_selection_identity(view).unwrap();
        let event = if list { CoreEvent::SetListStyle {expected, style:None} }
            else { CoreEvent::SetParagraphStyle {expected,style:"Paragraph".into()} };
        core.handle_with_layout(view, event).unwrap();
        exact(&core);
        assert_eq!(core.document().source_bytes(),source.as_bytes());
    }
}

#[test]
fn indent_source_emoji_and_nested_quote_selection_uses_source_mapping() {
    let source = "- a\n\n- 👩‍💻\n  > q\n\n  ```\n  c\n  ```\n\n- b";
    for selection in 0..3 {
        let (mut core, view) = open(source);
        let start = core.document().text().find('👩').unwrap() + "👩‍💻".len();
        let end = core.document().text().find('q').unwrap() + 1;
        place(&mut core, view, start, false);
        if selection > 0 { key(&mut core, view, Key::Char('i')); }
        if selection == 2 {
            key(&mut core, view, Key::ModifiedNavigation {key: NavigationKey::Down, modifiers:1});
        } else {
            place(&mut core, view, end, true);
        }
        key(&mut core, view, Key::Tab);
        exact(&core);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(),source.as_bytes());
    }
}

#[test]
fn emptied_source_line_retains_its_typing_destination() {
    for (source, at, keys, expected) in [
        ("first line\nsecond line\nthird line",11,"ccnew", "first line\nnew\nthird line"),
        ("first line\nsecond line\nthird line",11,"Snew", "first line\nnew\nthird line"),
        ("first line\nsecond line\nthird line",11,"0Cnew", "first line\nnew\nthird line"),
        ("first line\nsecond line\nthird line",11,"0d$inew", "first line\nnew\nthird line"),
        ("a\nbb\nc",2,"xxxiz", "a\nz\nc"),
        ("a\nbb\n\nc",2,"ccz", "a\nz\n\nc"),
        ("> a\n> bb\n> c",4,"ccz", "> a\nz\n> c"),
        ("- a\n  bb\n  c",4,"ccz", "- a\n  z\n  c"),
        ("a\nb\nc\n",4,"ccx", "a\nb\nx\n"),
        ("- a\n  bb\n",4,"ccx", "- a\n  x\n"),
    ] {
        let (mut core,view) = open(source);
        place(&mut core,view,at,false);
        for action in keys.chars() { key(&mut core,view,Key::Char(action)); }
        assert_eq!(core.document().source_bytes(),expected.as_bytes(),"{source:?}: {keys}");
        key(&mut core,view,Key::Escape);
        let mut undos = 0;
        while core.document().source_bytes() != source.as_bytes() && undos < keys.len() {
            core.handle(view,CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            exact(&core);
            undos += 1;
        }
        assert_eq!(core.document().source_bytes(),source.as_bytes());
        for _ in 0..undos {
            core.handle(view,CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            exact(&core);
        }
        assert_eq!(core.document().source_bytes(),expected.as_bytes());
    }
    let (mut core,view) = open("a\nbb\n\nc");
    place(&mut core,view,2,false);
    for action in [Key::Char('A'),Key::Backspace,Key::Backspace] {key(&mut core,view,action);}
    assert_eq!(core.document().source_bytes(),b"a\n\n\nc");
}
