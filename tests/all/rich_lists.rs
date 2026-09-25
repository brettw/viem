use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::{BlockKind, ListStyle};
use viem_core::{Document, Encoding, Format};

fn ordinals(document: &Document) -> Vec<u64> {
    document
        .projection()
        .list_structure()
        .lists
        .into_iter()
        .flat_map(|list| list.items.into_iter().map(|item| item.ordinal))
        .collect()
}

#[test]
fn html_lists_are_visible_editable_and_preserve_body_spelling() {
    let source = b"<p class='x'>One &amp; <b>two</b></p><!--keep--><p>Three</p>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "One & two\nThree");
    assert_eq!(ordinals(&document), vec![1, 2]);
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("class='x'>One &amp; <b>two</b>"));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("<!--keep-->"));
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "One & two\nThree");
    document
        .set_list_style(0..document.text().len(), None)
        .unwrap();
    assert_eq!(document.text(), "One & two\nThree");
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rtf_lists_use_scoped_numbering_and_undo_exact_source() {
    let source = br"{\rtf1\ansi One\par Two}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "One\nTwo");
    assert!(matches!(
        document.projection().blocks()[0].kind,
        BlockKind::ListItem { ordered: false, .. }
    ));
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "One\nTwo");
    assert_eq!(ordinals(&document), vec![1, 2]);
    document
        .set_list_style(0..document.text().len(), None)
        .unwrap();
    assert_eq!(document.text(), "One\nTwo");
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rtf_list_wrappers_keep_inline_groups_balanced() {
    let source = br"{\rtf1\ansi {\b One} two\par Three {\i four}}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "One two\nThree four");
    assert!(document
        .projection()
        .blocks()
        .iter()
        .all(|block| matches!(block.kind, BlockKind::ListItem { ordered: false, .. })));
    let text = document.text();
    let two = text.find("two").unwrap();
    assert!(!document.projection().style_spans().iter().any(|span|
        span.range.contains(&two) && matches!(&span.application, viem_core::document::StyleApplication::Direct(p) if p.weight == Some(700))));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rich_list_enter_continues_numbering_and_empty_enter_exits() {
    for (source, format) in [
        ("<p>First</p>", Format::Html),
        (r"{\rtf1 First}", Format::Rtf),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_list_style(0..0, Some(ListStyle::Numbered))
            .unwrap();
        let mut commands = CommandInterpreter::new();
        commands
            .handle(&mut document, InputEvent::key('A'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::Key(Key::Enter))
            .unwrap_or_else(|e| {
                panic!(
                    "{format:?}: {e:?} source {} spans {:?} blocks {:?}",
                    String::from_utf8_lossy(&document.source_bytes()),
                    document.projection().provenance(),
                    document.projection().blocks()
                )
            });
        assert_eq!(document.text(), "First\n", "{format:?}");
        commands
            .handle(&mut document, InputEvent::text("Second"))
            .unwrap();
        assert_eq!(document.text(), "First\nSecond", "{format:?}");
        assert_eq!(ordinals(&document), vec![1, 2]);
        commands
            .handle(&mut document, InputEvent::Key(Key::Enter))
            .unwrap_or_else(|e| {
                panic!(
                    "{format:?}: {e:?} source {} spans {:?} blocks {:?}",
                    String::from_utf8_lossy(&document.source_bytes()),
                    document.projection().provenance(),
                    document.projection().blocks()
                )
            });
        commands
            .handle(&mut document, InputEvent::Key(Key::Enter))
            .unwrap_or_else(|e| {
                panic!(
                    "{format:?}: {e:?} source {} spans {:?} blocks {:?}",
                    String::from_utf8_lossy(&document.source_bytes()),
                    document.projection().provenance(),
                    document.projection().blocks()
                )
            });
        assert_eq!(document.text(), "First\nSecond\n", "{format:?}");
        commands
            .handle(&mut document, InputEvent::text("Outside"))
            .unwrap_or_else(|e| {
                panic!(
                    "{format:?}: {e:?} source {} spans {:?} blocks {:?}",
                    String::from_utf8_lossy(&document.source_bytes()),
                    document.projection().provenance(),
                    document.projection().blocks()
                )
            });
        assert_eq!(document.text(), "First\nSecond\nOutside", "{format:?}");
        assert_eq!(ordinals(&document), vec![1, 2]);
    }
}

#[test]
fn numbered_list_enter_replays_semantically_for_counted_dot() {
    for (source, format) in [
        ("First", Format::PlainText),
        ("First", Format::Markdown),
        ("First", Format::MarkdownSource),
        ("<p>First</p>", Format::Html),
        (r"{\rtf1 First}", Format::Rtf),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_list_style(0..0, Some(ListStyle::Numbered))
            .unwrap();
        let mut commands = CommandInterpreter::new();
        for event in [
            InputEvent::key('A'),
            InputEvent::Key(Key::Enter),
            InputEvent::text("Next"),
            InputEvent::Key(Key::Escape),
            InputEvent::key('2'),
            InputEvent::key('.'),
        ] {
            commands
                .handle(&mut document, event)
                .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        }
        assert_eq!(
            document.text(),
            if matches!(format, Format::PlainText | Format::MarkdownSource) {
                "1. First\n2. Next\n3. Next\n4. Next"
            } else {
                "First\nNext\nNext\nNext"
            },
            "{format:?}"
        );
        if format != Format::PlainText {
            assert_eq!(ordinals(&document), vec![1, 2, 3, 4], "{format:?}");
        }
        commands
            .handle(&mut document, InputEvent::key('u'))
            .unwrap();
        assert_eq!(
            document.text(),
            if matches!(format, Format::PlainText | Format::MarkdownSource) {
                "1. First\n2. Next"
            } else {
                "First\nNext"
            },
            "{format:?}"
        );
    }
}

#[test]
fn changing_one_html_list_item_preserves_following_numbering() {
    let source = br#"<ol start="3" class='list'><li>A</li><li>B</li><li>C</li></ol>"#;
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "A\nB\nC");
    assert_eq!(ordinals(&document), vec![3, 4, 5]);
    document
        .set_list_style(2..3, Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "A\nB\nC");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("class='list'"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn rtf_list_enter_before_styled_following_paragraph_preserves_scopes() {
    let source = br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0 Helvetica;}}{\colortbl;\red128\green64\blue32;}\f0\fs28 RTF body\par {\b Bold words} and {\i italic}.\par {\cf1\fs40 Large colored text}\par{\*\unknown Opaque preserved}}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    document
        .set_list_style(0..0, Some(ListStyle::Numbered))
        .unwrap();
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('A'))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::Key(Key::Enter))
        .unwrap();
    assert_eq!(
        document.text(),
        "RTF body\n\nBold words and italic.\nLarge colored text\n"
    );
    commands
        .handle(&mut document, InputEvent::text("Second"))
        .unwrap();
    assert!(document.text().starts_with("RTF body\nSecond\nBold words"));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(r"{\b Bold words} and {\i italic}."));
}

#[test]
fn portable_list_containers_retain_item_identities_and_labels_are_decorations() {
    let mut document = Document::from_bytes(
        b"- First\n  - Child\n- Last".to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let before = document.projection().list_structure();
    assert_eq!(before.lists.len(), 2);
    assert_eq!(before.lists[0].items.len(), 2);
    assert_eq!(
        before.lists[1].parent_item,
        Some(before.lists[0].items[0].paragraph_id)
    );
    document.replace(2..7, "Renamed").unwrap();
    let after = document.projection().list_structure();
    assert_eq!(before.lists[0].id, after.lists[0].id);
    assert_eq!(before.lists[1].id, after.lists[1].id);
    for (source, format) in [
        ("<p>A<br>B</p><p>C</p>", Format::Html),
        (r"{\rtf1 A\line B\par C}", Format::Rtf),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_list_style(2..3, Some(ListStyle::Bullet))
            .unwrap();
        assert_eq!(document.text(), "A\nB\nC", "{format:?}");
        let lists = document.projection().list_structure();
        assert_eq!(lists.lists.len(), 1);
        assert!(lists.lists[0].items[0].marker_is_synthetic);
        let unchanged = document.source_bytes();
        assert!(lists.lists[0].items[0].marker_range.is_empty());
        document.replace(0..1, "X").unwrap();
        assert_eq!(document.text(), "X\nB\nC");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), unchanged);
    }
}

#[test]
fn independent_html_lists_and_multiparagraph_items_keep_structural_membership() {
    let source = b"<ul><li><p>A</p><p>B</p></li><li>C</li></ul><ul><li>D</li></ul>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "A\nB\nC\nD");
    let structure = document.projection().list_structure();
    assert_eq!(structure.lists.len(), 2);
    assert_eq!(structure.lists[0].items.len(), 2);
    assert_eq!(structure.lists[0].items[0].paragraph_ids.len(), 2);
    assert_eq!(structure.lists[1].items.len(), 1);
    document
        .set_list_style(2..3, Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "A\nB\nC\nD");
    document.set_list_style(2..3, None).unwrap();
    assert_eq!(document.text(), "A\nB\nC\nD");
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn nested_html_list_children_belong_to_the_containing_multiparagraph_item() {
    let source = b"<ul><li>A<p>B</p><ul><li>C</li></ul><p>D</p></li><li>E</li></ul>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "A\nB\nC\nD\nE");
    let structure = document.projection().list_structure();
    assert_eq!(structure.lists.len(), 2);
    assert_eq!(structure.lists[0].items[0].paragraph_ids.len(), 3);
    assert_eq!(
        structure.lists[0].items[0].child_lists,
        vec![structure.lists[1].id]
    );
    let at = document.text().find('D').unwrap();
    document
        .set_list_style(at..at + 1, Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "A\nB\nC\nD\nE");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn continuation_paragraphs_align_with_list_body_and_preserve_local_layout_queries() {
    use viem_core::layout::DocumentLayoutStyles;
    let mut source = String::from("<ul>");
    for _ in 0..2000 {
        source.push_str("<li><p>First</p><p>Continuation</p></li>");
    }
    source.push_str("</ul>");
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let continuation = document.projection().blocks()[1].range.clone();
    let styles =
        DocumentLayoutStyles::resolve_region(document.projection(), continuation.clone()).unwrap();
    let paragraph = styles
        .paragraphs
        .iter()
        .find(|paragraph| paragraph.text_range == continuation)
        .unwrap();
    assert_eq!(paragraph.leading_indent, 32.0);
    assert_eq!(paragraph.first_line_indent, 0.0);
    let first_id = document.projection().blocks()[0].id;
    document
        .replace(continuation.start..continuation.start + 1, "K")
        .unwrap();
    let updated = document.projection().blocks()[1].range.clone();
    let styles =
        DocumentLayoutStyles::resolve_region(document.projection(), updated.clone()).unwrap();
    assert_eq!(
        styles
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.text_range == updated)
            .unwrap()
            .first_line_indent,
        0.0
    );
    assert_eq!(document.projection().blocks()[0].id, first_id);
}

#[test]
fn html_ordered_enter_renumbers_following_items_until_an_explicit_restart() {
    for (source, expected, expected_ordinals) in [
        (
            "<ol><li><p>A</p><p>B</p></li><li>C</li></ol>",
            "A\nB\nNext\nC",
            vec![1, 2, 3],
        ),
        (
            "<ol><li value='5'><p>A</p><p>B</p></li><li>C</li></ol>",
            "A\nB\nNext\nC",
            vec![5, 6, 7],
        ),
        (
            "<ol><li><p>A</p><p>B</p></li><li value='2'>C</li><li>D</li></ol>",
            "A\nB\nNext\nC\nD",
            vec![1, 2, 2, 3],
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let mut commands = CommandInterpreter::new();
        for event in [
            InputEvent::key('j'),
            InputEvent::key('A'),
            InputEvent::Key(Key::Enter),
            InputEvent::text("Next"),
            InputEvent::Key(Key::Escape),
        ] {
            let event_name = format!("{event:?}");
            commands
                .handle(&mut document, event)
                .unwrap_or_else(|error| {
                    panic!(
                        "{error:?}: {source}: {event_name}; {} {:?}",
                        document.text(),
                        document.projection().blocks()
                    )
                });
        }
        assert_eq!(document.text(), expected);
        assert_eq!(ordinals(&document), expected_ordinals);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn decorated_html_empty_item_paragraph_has_only_one_empty_body_boundary() {
    let source = br#"<ol><li><p>A</p><p>B</p></li><li value="2"><p></p></li><li>C</li></ol>"#;
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        document.text(),
        "A\nB\n\nC",
        "{:?}",
        document.projection().blocks()
    );
    assert_eq!(ordinals(&document), vec![1, 2, 3]);
    document
        .insert(4, "Next")
        .unwrap_or_else(|e| panic!("{e:?}: {:?}", document.projection().provenance()));
    assert_eq!(document.text(), "A\nB\nNext\nC");
}

#[test]
fn empty_decorated_items_type_inside_the_innermost_formatting_context() {
    for (format, source) in [
        (
            Format::Html,
            "<ol start='9'><li><p><b></b></p></li></ol><!--keep-->",
        ),
        (
            Format::Rtf,
            r"{\rtf1{\*\pn\pnlvlbody\pndec\pnstart9{\pntxta .}}{\b }{\*\unknown keep}}",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.text(), "");
        assert_eq!(ordinals(&document), vec![9]);
        let list = document.projection().list_structure();
        assert_eq!(list.lists[0].items[0].marker_range, 0..0);
        document.insert(0, "é").unwrap();
        assert_eq!(document.text(), "é");
        assert!(document.projection().style_spans().iter().any(|span|
            span.range == (0..2) && matches!(&span.application,
                viem_core::document::StyleApplication::Direct(properties) if properties.bold == Some(true))),
            "{format:?}: {}", String::from_utf8_lossy(&document.source_bytes()));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.text(), "é");
        assert_eq!(ordinals(&document), vec![9]);
    }
}

#[test]
fn user_authored_label_looking_text_remains_editable_body_through_list_changes() {
    for (format, source) in [
        (
            Format::Html,
            "<p>12. Actual text</p><p>• Actual bullet</p><!--keep-->",
        ),
        (
            Format::Rtf,
            r"{\rtf1 12. Actual text\par\bullet Actual bullet{\*\unknown keep}}",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = document.text().to_owned();
        for style in [Some(ListStyle::Numbered), Some(ListStyle::Bullet), None] {
            document.set_list_style(0..text.len(), style).unwrap();
            assert_eq!(document.text(), text, "{format:?}");
        }
        for _ in 0..3 {
            assert!(document.undo());
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn deleting_or_changing_item_body_keeps_the_list_while_dd_removes_it() {
    for (format, source) in [
        (
            Format::Html,
            "<ol start='3'><li><b>Word</b></li></ol><!--keep-->",
        ),
        (
            Format::Rtf,
            r"{\rtf1{\*\pn\pnlvlbody\pndec\pnstart3{\pntxta .}}{\b Word}{\*\unknown keep}}",
        ),
    ] {
        for action in ["diw", "ciw", "dd"] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let mut commands = CommandInterpreter::new();
            for key in action.chars() {
                commands
                    .handle(&mut document, InputEvent::key(key))
                    .unwrap();
            }
            if action == "ciw" {
                commands
                    .handle(&mut document, InputEvent::text("Next"))
                    .unwrap();
                commands
                    .handle(&mut document, InputEvent::Key(Key::Escape))
                    .unwrap();
                assert_eq!(document.text(), "Next");
            } else {
                assert_eq!(document.text(), "");
            }
            assert_eq!(
                ordinals(&document),
                if action == "dd" { vec![] } else { vec![3] },
                "{format:?} {action}"
            );
            assert!(document.undo());
            assert_eq!(
                document.source_bytes(),
                source.as_bytes(),
                "{format:?} {action}"
            );
            assert!(document.redo());
            assert_eq!(
                ordinals(&document),
                if action == "dd" { vec![] } else { vec![3] }
            );
        }
    }
}

#[test]
fn whole_line_delete_of_an_empty_decorated_item_changes_only_structure() {
    for (format, source) in [
        (Format::Html, "<ul><li><p></p></li></ul><!--keep-->"),
        (
            Format::Rtf,
            r"{\rtf1{\*\pn\pnlvlbody\pndec\pnstart3{\pntxta .}}{\b }{\*\unknown keep}}",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut commands = CommandInterpreter::new();
        assert_eq!(document.text(), "");
        assert!(!document.projection().list_structure().lists.is_empty());
        document
            .prepare_model_request(viem_core::document::ModelRequest::DeleteLines {
                document: document.id(),
                revision: document.revision(),
                range: 0..0,
            })
            .unwrap_or_else(|error| panic!("prepare {format:?}: {error:?}"));
        for key in ['d', 'd'] {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap_or_else(|error| {
                    panic!(
                        "{format:?}: {error:?}: {:?}",
                        document.projection().provenance()
                    )
                });
        }
        assert_eq!(document.text(), "");
        assert!(
            document.projection().list_structure().lists.is_empty(),
            "{format:?}: {}",
            String::from_utf8_lossy(&document.source_bytes())
        );
        assert!(String::from_utf8_lossy(&document.source_bytes()).contains("keep"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn rtf_whole_item_delete_preserves_survivor_labels_and_opaque_source() {
    for count in [1, 2] {
        let source = r"{\rtf1 {{\*\pn\pnlvlbody\pndec\pnstart1{\pntxta .}}{\b One} body}\par{{\*\pn\pnlvlbody\pndec\pnstart2{\pntxta .}}Second {\i item}}\par{{\*\pn\pnlvlbody\pndec\pnstart3{\pntxta .}}Third{\*\unknown Opaque}}}";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
        let listed = document.source_bytes();
        let mut commands = CommandInterpreter::new();
        if count == 2 {
            commands
                .handle(&mut document, InputEvent::key('2'))
                .unwrap();
        }
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap_or_else(|error| {
                panic!(
                    "{error:?}: {}",
                    String::from_utf8_lossy(&document.source_bytes())
                )
            });
        assert_eq!(
            document.text(),
            if count == 1 {
                "Second item\nThird"
            } else {
                "Third"
            }
        );
        assert_eq!(
            ordinals(&document),
            if count == 1 { vec![2, 3] } else { vec![3] }
        );
        assert!(String::from_utf8_lossy(&document.source_bytes()).contains(r"{\*\unknown Opaque}"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), listed);
        commands
            .handle(&mut document, InputEvent::key('G'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap();
        assert_eq!(document.text(), "One body\nSecond item");
        assert_eq!(ordinals(&document), vec![1, 2]);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), listed);
    }
}

#[test]
fn rtf_ordered_enter_and_counted_repeat_renumber_same_container_suffix() {
    for source in [
        r"{\rtf1 {{\*\pn\pnlvlbody\pndec\pnstart1{\pntxta .}}One}\par{{\*\pn\pnlvlbody\pndec\pnstart2{\pntxta .}}Two}\par{{\*\pn\pnlvlbody\pndec\pnstart8{\pntxta .}}Restart}}",
        r"{\rtf1{\*\listtable{\list\listid1{\listlevel\levelnfc0\levelstartat1{\leveltext\'02\'00.;}}}}{\*\listoverridetable{\listoverride\listid1\listoverridecount0\ls1}}\pard\ls1 One\par\pard\ls1 Two\par\pard Tail}",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
        let mut commands = CommandInterpreter::new();
        for event in [
            InputEvent::key('A'),
            InputEvent::Key(Key::Enter),
            InputEvent::text("Next"),
            InputEvent::Key(Key::Escape),
        ] {
            commands.handle(&mut document, event).unwrap();
        }
        let tail = if source.contains("Restart") {
            "Restart"
        } else {
            "Tail"
        };
        assert_eq!(document.text(), format!("One\nNext\nTwo\n{tail}"));
        assert_eq!(
            ordinals(&document),
            if source.contains("Restart") {
                vec![1, 2, 3, 8]
            } else {
                vec![1, 2, 3]
            }
        );
        for key in ['2', '.'] {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap();
        }
        assert_eq!(
            document.text(),
            format!("One\nNext\nNext\nNext\nTwo\n{tail}")
        );
        assert_eq!(
            ordinals(&document),
            if source.contains("Restart") {
                vec![1, 2, 3, 4, 5, 8]
            } else {
                vec![1, 2, 3, 4, 5]
            },
            "{}",
            String::from_utf8_lossy(&document.source_bytes())
        );
        assert!(document.undo());
        assert_eq!(document.text(), format!("One\nNext\nTwo\n{tail}"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
