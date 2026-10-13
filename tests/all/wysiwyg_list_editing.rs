use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for key in input.chars() {
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
        assert!(
            matches!(
                output.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{key:?}"
        );
    }
}

fn fixtures() -> [(Format, &'static str); 2] {
    [
        (Format::Markdown, "- First words continue onto several wrapped rows with more prose.\n- Second item."),
        (Format::Markdown, "9) First words continue onto several wrapped rows with more prose.\n1) Second item."),
    ]
}

#[test]
fn start_end_and_counted_edits_address_list_body_only() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            doc.text(),
            "First words continue onto several wrapped rows with more prose.\nSecond item."
        );
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 900., 600.);
        keys(&mut core, view, "0");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        keys(&mut core, view, "2x");
        assert!(
            core.document().text().starts_with("rst words"),
            "{format:?}"
        );
        keys(&mut core, view, "u$");
        let expected = core.document().text().find('\n').unwrap();
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .visual_position()
                .unwrap()
                .text_offset,
            expected
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), expected - 1);
        keys(&mut core, view, "x");
        assert!(core
            .document()
            .text()
            .starts_with("First words continue onto several wrapped rows with more prose\n"));
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn wrapped_list_end_targets_final_letter_and_append_stays_before_wrap_space() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 165., 800.);
        let before = core.document().text().to_owned();
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        assert!(row.wraps_to_next);
        let content_end = row.text_range.start
            + before[row.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "$$");
        let state = core.command_state(view).unwrap();
        assert_eq!(
            state.visual_position().unwrap().text_offset,
            content_end,
            "{format:?}"
        );
        assert_eq!(state.boundary_affinity(), BoundaryAffinity::Upstream);
        assert!(!before[state.cursor()..]
            .chars()
            .next()
            .unwrap()
            .is_whitespace());
        keys(&mut core, view, "A");
        core.handle(view, CoreEvent::Input(InputEvent::text("XYZ")))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        let mut expected = before.clone();
        expected.insert_str(content_end, "XYZ");
        assert_eq!(core.document().text(), expected, "{format:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn wrapped_list_operators_and_registers_contain_body_text_only() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 165., 800.);
        let before = core.document().text().to_owned();
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        let end = row.text_range.start
            + before[row.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "\"ay$");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            before[..end],
            "{format:?}"
        );
        keys(&mut core, view, "0d$");
        let remaining = &before[end..];
        let expected = {
            remaining.to_owned()
        };
        assert_eq!(core.document().text(), expected, "{format:?}");
        assert!(!core
            .document()
            .projection()
            .list_structure()
            .lists
            .is_empty());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        let second = &core.layout(view).unwrap().snapshot().unwrap().rows[1];
        let end = second.text_range.start
            + before[second.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "02$");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .visual_position()
                .unwrap()
                .text_offset,
            end,
            "{format:?}"
        );
    }
}

#[test]
fn unicode_list_body_end_deletes_one_whole_grapheme() {
    for (format, source) in [
        (Format::Markdown, "- café 👩🏽‍💻"),
    ] {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 900., 600.);
        keys(&mut core, view, "$x");
        assert_eq!(core.document().text(), { "café " });

        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('"')
                .unwrap()
                .text,
            "👩🏽‍💻"
        );
        keys(&mut core, view, "u0x");
        assert_eq!(core.document().text(), "afé 👩🏽‍💻");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}


fn assert_list_reopens(document: &Document) {
    use std::collections::HashMap;
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
    let reopened = Document::from_bytes_with_file_format(document.source_bytes(), document.encoding(),
        document.format(), document.file_format()).unwrap();
    assert_eq!(document.text(), reopened.text());
    assert_eq!(shape(document), shape(&reopened));
}

#[test]
fn surplus_empty_paragraphs_keep_their_list_item_ownership() {
    use viem_core::document::BlockKind;
    for source in ["- a\n\n\n\n- b", "1. a\n\n\n\n2. b\n3. c", "> - a\n>\n>\n>\n> - b"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let blocks = document.projection().blocks();
        let empty = blocks.iter().find(|block| block.range.is_empty()).unwrap();
        if source.starts_with('>') {
            assert!(matches!(empty.kind, BlockKind::ListItem { item_start: false, .. }));
        } else {
            assert_eq!(empty.kind, BlockKind::Paragraph);
        }
        let lists = document.projection().list_structure().lists;
        assert_eq!(lists.len(), 1, "{source:?}");
        assert_eq!(lists[0].items[0].paragraph_ids, [blocks[0].id, empty.id]);
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    for source in ["- a\n\n\n\n+ b", "- a\n\n&nbsp;\n\n- b", "- a\n\nprose\n\n- b"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(document.projection().list_structure().lists.len(), 2, "{source:?}");
    }
}

#[test]
fn insert_tab_crosses_owned_empty_paragraphs_and_keeps_undo() {
    use viem_core::document::HistoryNavigationRequest;
    for source in [
        "- a\n\n\n\n- b", "- a\n- b\n\n\n\n- c",
        "1. a\n\n\n\n2. b\n3. c", "- parent\n  - a\n\n\n\n  - b",
        "- parent\n\n  8. a\n\n\n\n  9. b",
        "> - a\n>\n>\n>\n> - b",
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        let at = core.document().text().find('b').unwrap();
        keys(&mut core, view, "i");
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        assert!(core.document().list_indent_capabilities(at..at).0, "{source:?}");
        let before = core.document().text().to_owned();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Tab))).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        let nested = core.document().source_bytes();
        assert_ne!(nested, source.as_bytes(), "{source:?}");
        assert_eq!(core.document().text(), before);
        assert!(core.document().projection().blocks().iter().any(|block| block.range.is_empty()));
        assert_list_reopens(core.document());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), nested);
        keys(&mut core, view, "i");
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::BackTab)))
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        assert_eq!(core.document().text(), before);
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?}");
        assert_list_reopens(core.document());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), nested);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), nested);
    }
}

#[test]
fn deleting_a_list_row_preserves_an_unselected_empty_edge_paragraph() {
    use viem_core::document::{BlockKind, HistoryNavigationRequest};
    for (source, expected) in [
        ("1. a\n\n\n\n2. b\n3. c", "\n\n1. b\n2. c"),
        ("- a\n\n\n\n- b\n- c", "\n\n- b\n- c"),
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        let empty_id = core.document().projection().blocks().iter()
            .find(|block| block.range.is_empty()).unwrap().id;
        keys(&mut core, view, "\"add");
        assert_eq!(core.document().text(), "\nb\nc");
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        let first = &core.document().projection().blocks()[0];
        assert!(first.range.is_empty());
        assert_eq!(first.kind, BlockKind::Paragraph);
        assert_eq!(first.id, empty_id);
        assert_eq!(core.command_state(view).unwrap().register('a').unwrap().text, "a\n");
        assert_list_reopens(core.document());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        keys(&mut core, view, "ggdd");
        assert_eq!(core.document().text(), "b\nc");
        assert_list_reopens(core.document());
    }
}

#[test]
fn deleting_a_list_row_after_an_implicit_boundary_retains_the_following_empty() {
    use viem_core::document::{FileFormat, HistoryNavigationRequest};
    for (source, expected) in [
        ("- parent\n  1. a\n\n\n\n  2. b\n  3. c", "- parent\n\n\n\n  1. b\n  2. c"),
        ("- before\n- a\n\n\n\n- b", "- before\n\n\n\n- b"),
        ("- before\n- a\n\n\n\n\n\n- b", "- before\n\n\n\n\n\n- b"),
        ("before\n\n1. a\n\n\n\n2. b", "before\n\n\n\n1. b"),
    ] {
        for (encoding, file_format) in [(Encoding::Utf8, FileFormat::Unix),
            (Encoding::Utf8, FileFormat::Dos), (Encoding::Utf16Le, FileFormat::Dos)] {
            let encode = |text: &str| {
                let ending = if file_format == FileFormat::Dos { "\r\n" } else { "\n" };
                let text = text.replace('\n', ending);
                match encoding {
                    Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                    _ => text.into_bytes(),
                }
            };
            let source = encode(source);
            let expected = encode(expected);
            let mut core = Core::new(Document::from_bytes_with_file_format(source.clone(), encoding,
                Format::Markdown, file_format).unwrap());
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
            let at = core.document().text().find("\na\n").unwrap() + 1;
            let before = core.document().text().to_owned();
            let text = format!("{}{}", &before[..at], &before[at + 2..]);
            core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
            keys(&mut core, view, "\"add");
            assert_eq!(core.document().text(), text, "{encoding:?} {file_format:?}");
            assert_eq!(core.document().source_bytes(), expected, "{encoding:?} {file_format:?}");
            assert_eq!(core.command_state(view).unwrap().register('a').unwrap().text, "a\n");
            assert_list_reopens(core.document());
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), expected);
        }
    }
}

#[test]
fn numbered_items_with_empty_continuations_survive_marker_width_changes() {
    use viem_core::document::HistoryNavigationRequest;
    for (source, expected, opening) in [
        ("9. a\n10. b\n\n\n\n11. c", "9. b\n\n\n\n10. c", false),
        ("9. a\n9. b\n\n\n\n9. c", "9. a\n10. x\n11. b\n\n\n\n12. c", true),
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        if opening {
            keys(&mut core, view, "ox");
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        } else {
            keys(&mut core, view, "dd");
        }
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        assert_list_reopens(core.document());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}
