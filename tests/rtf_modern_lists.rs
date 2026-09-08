use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::{BlockKind, ListStyle};
use viem_core::{Document, Encoding, Format};

fn source() -> String {
    concat!(
        r"{\rtf1\ansi{\*\listtable",
        r"{\list\listtemplateid42\listhybrid",
        r"{\listlevel\levelnfc0\levelstartat3{\leveltext\'02\'00.;}{\levelnumbers\'01;}\li720\fi-360}",
        r"{\listlevel\levelnfc23\levelstartat1{\leveltext\'01\u8226?;}{\levelnumbers;}\li1440\fi-360}\listid42}",
        r"}{\*\listoverridetable{\listoverride\listid42\listoverridecount0\ls1}",
        r"{\listoverride\listid42\listoverridecount1\ls2{\lfolevel\listoverridestartat\levelstartat8}}}",
        r"\pard\ls1\ilvl0{\listtext 3.\tab}One\par ",
        r"\pard\ls1\ilvl1{\listtext\u8226?\tab}{\b Child}\par ",
        r"\pard\ls1\ilvl0{\listtext 4.\tab}Two\par ",
        r"\pard\ls2\ilvl0{\listtext 8.\tab}Restart\par\pard Tail{\*\unknown preserved}}"
    ).to_owned()
}
fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap()
}
fn numbering(document: &Document) -> Vec<u64> {
    document
        .projection()
        .blocks()
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::ListItem { ordinal, .. } => Some(ordinal),
            _ => None,
        })
        .collect()
}
#[test]
fn word_list_tables_preserve_bytes_and_project_nested_numbering_and_overrides() {
    let source = source();
    let document = open(&source);
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert_eq!(document.text(), "One\nChild\nTwo\nRestart\nTail");
    assert_eq!(numbering(&document), vec![3, 1, 4, 8]);
    let lists = document.projection().list_structure();
    assert_eq!(lists.lists.len(), 3);
    assert_eq!(lists.lists[0].items.len(), 2);
    assert_eq!(lists.lists[0].start, 3);
    assert_eq!(
        lists.lists[1].parent_item,
        Some(lists.lists[0].items[0].paragraph_id)
    );
    assert_eq!(lists.lists[2].start, 8);
    assert!(lists
        .lists
        .iter()
        .flat_map(|list| &list.items)
        .all(|item| item.marker_is_synthetic && item.marker_range.is_empty()));
    assert!(matches!(
        document.projection().blocks()[1].kind,
        BlockKind::ListItem {
            level: 1,
            ordered: false,
            ..
        }
    ));
}
#[test]
fn text_and_list_edits_preserve_modern_tables_and_reopen_exactly() {
    let source = source();
    let mut document = open(&source);
    let start = document.text().find("Child").unwrap();
    document.replace(start..start + 5, "Kid").unwrap();
    assert_eq!(document.text(), "One\nKid\nTwo\nRestart\nTail");
    assert_eq!(
        document.source_bytes(),
        source.replace("Child", "Kid").as_bytes()
    );
    assert_eq!(
        open(&String::from_utf8(document.source_bytes()).unwrap()).text(),
        document.text()
    );
    assert!(document.undo());
    let at = document.text().find("Two").unwrap();
    document.set_list_style(at..at + 3, None).unwrap();
    assert_eq!(document.text(), "One\nChild\nTwo\nRestart\nTail");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(r"{\*\listtable"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    document
        .set_list_style(at..at + 3, Some(ListStyle::Bullet))
        .unwrap();
    assert_eq!(document.text(), "One\nChild\nTwo\nRestart\nTail");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn enter_continues_imported_decimal_list_as_one_undo_unit() {
    let source = source();
    let mut document = open(&source);
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('A'))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::Key(Key::Enter))
        .unwrap();
    for character in "Next".chars() {
        commands
            .handle(&mut document, InputEvent::text(character.to_string()))
            .unwrap();
    }
    commands
        .handle(&mut document, InputEvent::Key(Key::Escape))
        .unwrap();
    assert_eq!(document.text(), "One\nNext\nChild\nTwo\nRestart\nTail");
    assert!(String::from_utf8_lossy(&document.source_bytes()).contains(r"\par\ls0 {\*\pn"));
    assert_eq!(numbering(&document), vec![3, 4, 1, 5, 8]);
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn large_imported_lists_keep_text_edits_regional_and_distant_identities_stable() {
    use viem_core::document::{ModelRequest, ProjectionWorkScope, TextEdit};
    let original = source();
    let header = &original[..original.find(r"\pard").unwrap()];
    let mut large = header.to_owned();
    for index in 0..10_000 {
        large.push_str(&format!("\\pard\\ls1\\ilvl0 Line {index:05}\\par\n"));
    }
    large.push('}');
    let mut document = open(&large);
    let distant = document.projection().hard_line_id(9000).unwrap();
    let at = document.projection().hard_line_range(5000).unwrap().start + 6;
    let transaction = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 1, "X")],
        })
        .unwrap();
    let work = transaction.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(transaction).unwrap();
    assert_eq!(document.projection().hard_line_id(9000).unwrap(), distant);
    let reopened = open(&String::from_utf8(document.source_bytes()).unwrap());
    assert_eq!(document.text(), reopened.text());
    let a = document.projection().provenance();
    let b = reopened.projection().provenance();
    assert!(
        a == b,
        "first provenance difference: {:?}; lengths {} {}",
        a.iter()
            .zip(b.iter())
            .enumerate()
            .find(|(_, (a, b))| a != b),
        a.len(),
        b.len()
    );
    assert_eq!(
        document.projection().style_spans(),
        reopened.projection().style_spans()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), large.as_bytes());
}

#[test]
fn level_overrides_and_nested_restarts_follow_the_word_numbering_stream() {
    let source = concat!(
        r"{\rtf1{\*\listtable{\list\listid7",
        r"{\listlevel\levelnfc0\levelstartat1{\leveltext\'02\'00.;}}",
        r"{\listlevel\levelnfc23\levelnfcn0\levelstartat5{\leveltext\'02\'01.;}}}}",
        r"{\*\listoverridetable{\listoverride\listid7\ls1\listoverridecount0}",
        r"{\listoverride\listid7\ls2\listoverridecount1{\lfolevel\listoverrideformat1",
        r"{\listlevel\levelnfc23\levelstartat1{\leveltext\'01\u8226?;}}}}}",
        r"\pard\ls1\ilvl0 A\par\pard\ls1\ilvl1 B\par\pard\ls1\ilvl1 C\par",
        r"\pard\ls1\ilvl0 D\par\pard\ls1\ilvl1 E\par\pard\ls2\ilvl0 F}"
    );
    let document = open(source);
    assert_eq!(document.text(), "A\nB\nC\nD\nE\nF");
    assert_eq!(numbering(&document), vec![1, 5, 6, 2, 5, 1]);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn list_definitions_in_opaque_destinations_do_not_affect_body_text() {
    let source = source();
    let boundary = source.find(r"\pard").unwrap();
    let source = format!(
        "{{\\rtf1{{\\*\\opaque {}}}\\ls1 Text}}",
        &source[11..boundary]
    );
    let document = open(&source);
    assert_eq!(document.text(), "Text");
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn nested_rtf_document_in_opaque_destination_cannot_supply_body_list_tables() {
    let fixture = source();
    let header = &fixture[..fixture.find(r"\pard").unwrap()];
    let source = format!("{{\\rtf1{{\\*\\opaque {header}}}}}\\ls1 Text}}");
    let document = open(&source);
    assert_eq!(document.text(), "Text");
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn modern_empty_and_control_only_paragraphs_retain_list_properties() {
    let fixture = source();
    let header = &fixture[..fixture.find(r"\pard").unwrap()];
    for (body, text) in [(r"\pard\ls1\ilvl0\u233?}", "é"), (r"\pard\ls1\ilvl0}", "")] {
        let document = open(&format!("{header}{body}"));
        assert_eq!(document.text(), text, "{body}");
        assert_eq!(numbering(&document), vec![3]);
        assert_eq!(
            document.projection().blocks()[0]
                .direct_paragraph
                .leading_indent,
            Some(36.0),
            "{body}"
        );
        assert_eq!(
            document.projection().blocks()[0]
                .direct_paragraph
                .first_line_indent,
            Some(-18.0),
            "{body}"
        );
    }
}

#[test]
fn whole_item_delete_preserves_surviving_word_list_labels_and_tables() {
    let original = source();
    for count in [1, 2, 3] {
        let mut document = open(&original);
        let mut commands = CommandInterpreter::new();
        if count > 1 {
            commands
                .handle(&mut document, InputEvent::key(char::from(b'0' + count)))
                .unwrap();
        }
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('d'))
            .unwrap_or_else(|error| panic!("{error:?} count={count}"));
        assert_eq!(
            document.text(),
            match count {
                1 => "Child\nTwo\nRestart\nTail",
                2 => "Two\nRestart\nTail",
                _ => "Restart\nTail",
            }
        );
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        let boundary = original.find(r"\pard").unwrap();
        assert_eq!(&saved[..boundary], &original[..boundary]);
        assert!(saved.contains(r"{\*\unknown preserved}"));
        assert_eq!(open(&saved).text(), document.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
    }
}

#[test]
fn deleting_all_imported_items_leaves_no_generated_marker() {
    let original = source();
    let header = &original[..original.find(r"\pard").unwrap()];
    let original = format!("{header}\\pard\\ls1\\ilvl0 One\\par\\pard\\ls1\\ilvl0 Two}}");
    let mut document = open(&original);
    let mut commands = CommandInterpreter::new();
    for key in ['2', 'd', 'd'] {
        commands
            .handle(&mut document, InputEvent::key(key))
            .unwrap();
    }
    assert_eq!(document.text(), "");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
}

#[test]
fn inserted_item_restores_inherited_modern_selector_for_following_nested_items() {
    let original = source()
        .replace(r"\pard\ls1\ilvl1", r"\ilvl1")
        .replace(r"\pard\ls1\ilvl0{\listtext 4.", r"\ilvl0{\listtext 4.");
    let mut document = open(&original);
    let mut commands = CommandInterpreter::new();
    for event in [
        InputEvent::key('A'),
        InputEvent::Key(Key::Enter),
        InputEvent::text("Next"),
        InputEvent::Key(Key::Escape),
    ] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "One\nNext\nChild\nTwo\nRestart\nTail");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
}
