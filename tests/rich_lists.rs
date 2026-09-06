use evim_core::command::{CommandInterpreter, InputEvent, Key};
use evim_core::document::{BlockKind, ListStyle};
use evim_core::{Document, Encoding, Format};

#[test]
fn html_lists_are_visible_editable_and_preserve_body_spelling() {
    let source = b"<p class='x'>One &amp; <b>two</b></p><!--keep--><p>Three</p>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "• One & two\n• Three");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("class='x'>One &amp; <b>two</b>"));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("<!--keep-->"));
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "1. One & two\n2. Three");
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
    assert_eq!(document.text(), "• One\n• Two");
    assert!(matches!(
        document.projection().blocks()[0].kind,
        BlockKind::ListItem { ordered: false, .. }
    ));
    document
        .set_list_style(0..document.text().len(), Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "1. One\n2. Two");
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
    assert_eq!(document.text(), "• One two\n• Three four");
    assert!(document
        .projection()
        .blocks()
        .iter()
        .all(|block| matches!(block.kind, BlockKind::ListItem { ordered: false, .. })));
    let text = document.text();
    let two = text.find("two").unwrap();
    assert!(!document.projection().style_spans().iter().any(|span|
        span.range.contains(&two) && matches!(&span.application, evim_core::document::StyleApplication::Direct(p) if p.weight == Some(700))));
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
        assert_eq!(document.text(), "1. First\n2. ", "{format:?}");
        commands
            .handle(&mut document, InputEvent::text("Second"))
            .unwrap();
        assert_eq!(document.text(), "1. First\n2. Second", "{format:?}");
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
        assert_eq!(document.text(), "1. First\n2. Second\n", "{format:?}");
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
        assert_eq!(
            document.text(),
            "1. First\n2. Second\nOutside",
            "{format:?}"
        );
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
            "1. First\n2. Next\n3. Next\n4. Next",
            "{format:?}"
        );
        commands
            .handle(&mut document, InputEvent::key('u'))
            .unwrap();
        assert_eq!(document.text(), "1. First\n2. Next", "{format:?}");
    }
}

#[test]
fn changing_one_html_list_item_preserves_following_numbering() {
    let source = br#"<ol start="3" class='list'><li>A</li><li>B</li><li>C</li></ol>"#;
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "3. A\n4. B\n5. C");
    document
        .set_list_style(5..9, Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "3. A\n• B\n5. C");
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
        "1. RTF body\n2. \nBold words and italic.\nLarge colored text\n"
    );
    commands
        .handle(&mut document, InputEvent::text("Second"))
        .unwrap();
    assert!(document
        .text()
        .starts_with("1. RTF body\n2. Second\nBold words"));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(r"{\b Bold words} and {\i italic}."));
}

#[test]
fn portable_list_containers_retain_item_identities_and_markers_are_synthetic() {
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
        assert_eq!(document.text(), "• A\nB\nC", "{format:?}");
        let lists = document.projection().list_structure();
        assert_eq!(lists.lists.len(), 1);
        assert!(lists.lists[0].items[0].marker_is_synthetic);
        let unchanged = document.source_bytes();
        assert!(document.replace(0..4, "X").is_err());
        assert_eq!(document.source_bytes(), unchanged);
    }
}

#[test]
fn independent_html_lists_and_multiparagraph_items_keep_structural_membership() {
    let source = b"<ul><li><p>A</p><p>B</p></li><li>C</li></ul><ul><li>D</li></ul>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "• A\nB\n• C\n• D");
    let structure = document.projection().list_structure();
    assert_eq!(structure.lists.len(), 2);
    assert_eq!(structure.lists[0].items.len(), 2);
    assert_eq!(structure.lists[0].items[0].paragraph_ids.len(), 2);
    assert_eq!(structure.lists[1].items.len(), 1);
    document
        .set_list_style(6..7, Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(document.text(), "1. A\nB\n• C\n• D");
    document.set_list_style(5..6, None).unwrap();
    assert_eq!(document.text(), "A\nB\n• C\n• D");
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn nested_html_list_children_belong_to_the_containing_multiparagraph_item() {
    let source = b"<ul><li>A<p>B</p><ul><li>C</li></ul><p>D</p></li><li>E</li></ul>";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(document.text(), "• A\nB\n• C\nD\n• E");
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
    assert_eq!(document.text(), "1. A\nB\n• C\nD\n• E");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn continuation_paragraphs_align_with_list_body_and_preserve_local_layout_queries() {
    use evim_core::layout::DocumentLayoutStyles;
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
    assert_eq!(paragraph.leading_indent, 20.0);
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
    for (source, expected) in [
        (
            "<ol><li><p>A</p><p>B</p></li><li>C</li></ol>",
            "1. A\nB\n2. Next\n3. C",
        ),
        (
            "<ol><li value='5'><p>A</p><p>B</p></li><li>C</li></ol>",
            "5. A\nB\n6. Next\n7. C",
        ),
        (
            "<ol><li><p>A</p><p>B</p></li><li value='2'>C</li><li>D</li></ol>",
            "1. A\nB\n2. Next\n2. C\n3. D",
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
            commands
                .handle(&mut document, event)
                .unwrap_or_else(|error| panic!("{error:?}: {source}"));
        }
        assert_eq!(document.text(), expected);
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
                "2. Second item\n3. Third"
            } else {
                "3. Third"
            }
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
        assert_eq!(document.text(), "1. One body\n2. Second item");
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
            "8. Restart"
        } else {
            "Tail"
        };
        assert_eq!(document.text(), format!("1. One\n2. Next\n3. Two\n{tail}"));
        for key in ['2', '.'] {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap();
        }
        assert_eq!(
            document.text(),
            format!("1. One\n2. Next\n3. Next\n4. Next\n5. Two\n{tail}")
        );
        assert!(document.undo());
        assert_eq!(document.text(), format!("1. One\n2. Next\n3. Two\n{tail}"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
