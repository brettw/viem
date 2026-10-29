use viem_core::document::*;

fn open(source: &[u8]) -> Document {
    Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}

fn states(document: &Document) -> Vec<Option<bool>> {
    document.projection().blocks().iter().map(|block| block.task_checkbox).collect()
}

fn assert_roundtrip(document: &mut Document, before: &[u8], text: &str) {
    let after = document.source_bytes();
    assert_eq!(document.text(), text);
    let reopened = open(&after);
    assert_eq!(reopened.text(), text);
    assert_eq!(states(&reopened), states(document));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn changing_task_leaf_treatment_removes_only_its_hidden_checkbox() {
    for source in [b"- [X] task\n- [x] other".as_slice(),
        b"- [X] task\n  - [x] nested done\n- [ ] upper"] {
        for style in ["Paragraph", "Heading2", "Code Block"] {
            let mut document = open(source);
            let text = document.text().to_owned();
            document.set_paragraph_style(0..1, style.into()).unwrap();
            assert_eq!(states(&document)[0], None, "{style}");
            assert_eq!(states(&document)[1..], states(&open(source))[1..], "{style}");
            assert_roundtrip(&mut document, source, &text);
        }
    }
}

#[test]
fn changing_task_container_keeps_checked_spelling_and_undo() {
    let source = b"- [X] task\n- [ ] other";
    for style in [Some(ListStyle::Bullet), Some(ListStyle::Numbered), None] {
        let mut document = open(source);
        document.set_list_style(0..1, style).unwrap();
        assert_eq!(states(&document), [None, Some(false)]);
        assert_roundtrip(&mut document, source, "task\nother");
    }
    let mut document = open(source);
    document.set_paragraph_style(0..1, "Block quote".into()).unwrap();
    assert_eq!(states(&document), [Some(true), Some(false)]);
    assert!(String::from_utf8(document.source_bytes()).unwrap().contains("[X] task"));
    assert_roundtrip(&mut document, source, "task\nother");
}

#[test]
fn task_indentation_keeps_markers_and_rejects_the_first_sibling() {
    let source = b"- [ ] parent\n- [X] child\n- [x] sibling";
    let mut document = open(source);
    assert_eq!(document.list_indent_capabilities(0..0), (false, false));
    assert!(document.prepare_list_indent(0..0, false).is_err());
    assert_eq!(document.source_bytes(), source);
    let at = document.text().find("child").unwrap();
    assert_eq!(document.list_indent_capabilities(at..at), (true, false));
    let prepared = document.prepare_list_indent(at..at, false).unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(states(&document), [Some(false), Some(true), Some(true)]);
    assert_eq!(document.list_indent_capabilities(at..at), (false, true));
    assert!(String::from_utf8(document.source_bytes()).unwrap().contains("[X] child"));
    assert_roundtrip(&mut document, source, "parent\nchild\nsibling");
    let nested = document.source_bytes();
    let prepared = document.prepare_list_indent(at..at, true).unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(states(&document), [Some(false), Some(true), Some(true)]);
    assert_roundtrip(&mut document, &nested, "parent\nchild\nsibling");
}

#[test]
fn nested_task_quotes_and_code_preserve_text_and_enclosing_items() {
    for source in [b"- [ ] task\n  - [x] nested done\n- [X] upper".as_slice(),
        b"- task\n  - nested done\n- upper"] {
        for range in [5..5, 11..11, 6..16, 8..19, 4..18] {
            for style in ["Block quote", "Code Block"] {
                let mut document = open(source);
                let text = document.text().to_owned();
                document.set_paragraph_style(range.clone(), style.into())
                    .unwrap_or_else(|error| panic!("{source:?} {range:?} {style}: {error:?}"));
                assert_eq!(states(&document)[1], None);
                let nested = &document.projection().blocks()[1];
                assert!(matches!(nested.kind, BlockKind::ListItem { .. }), "{style}: {nested:?}");
                if style == "Code Block" {
                    assert_eq!(nested.style.0, "Code Block");
                } else {
                    assert_eq!(nested.quote_depth, 1);
                }
                assert_roundtrip(&mut document, source, &text);
            }
        }
    }
}

#[test]
fn task_indent_capabilities_match_unmarked_lists_at_the_same_visible_range() {
    let task = open(b"- [ ] task\n  - [x] nested done\n- [X] upper");
    let plain = open(b"- task\n  - nested done\n- upper");
    assert_eq!(task.text(), plain.text());
    for (range, expected) in [
        (0..0, (false, false)), (5..5, (false, true)), (11..11, (false, true)),
        (17..17, (true, false)), (1..11, (false, false)), (1..12, (false, false)),
        (3..4, (false, false)), (3..17, (false, false)), (4..18, (false, false)),
        (6..16, (false, true)), (8..19, (false, false)), (8..20, (false, false)),
        (11..14, (false, true)), (11..15, (false, true)), (14..19, (false, false)),
        (14..20, (false, false)),
    ] {
        assert_eq!(plain.list_indent_capabilities(range.clone()), expected, "{range:?}");
        assert_eq!(task.list_indent_capabilities(range.clone()), expected, "{range:?}");
        for (unindent, available) in [(false, expected.0), (true, expected.1)] {
            let plain_result = plain.prepare_list_indent(range.clone(), unindent);
            let task_result = task.prepare_list_indent(range.clone(), unindent);
            assert_eq!(plain_result.is_ok(), available, "plain {range:?} unindent={unindent}");
            assert_eq!(task_result.is_ok(), available, "task {range:?} unindent={unindent}");
            if !available {
                assert!(matches!(plain_result,
                    Err(ModelTransactionError::Document(DocumentError::UnsupportedFormatting))));
                assert!(matches!(task_result,
                    Err(ModelTransactionError::Document(DocumentError::UnsupportedFormatting))));
            }
        }
    }
}

#[test]
fn task_style_creation_and_mixed_conversion_preserve_existing_task_source() {
    for (source, expected) in [
        ("ordinary", "- [ ] ordinary"),
        ("3) ordinary", "- [ ] ordinary"),
        ("+ ordinary", "- [ ] ordinary"),
        ("3) [X] imported", "3) [X] imported"),
        ("- [X] First\n- Second\n- [x] Third", "- [X] First\n- [ ] Second\n- [x] Third"),
    ] {
        let mut document = open(source.as_bytes());
        let text = document.text().to_owned();
        assert!(document.can_set_task_list(0..text.len()));
        document.set_list_style(0..text.len(), Some(ListStyle::Task)).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        if source == expected {
            assert!(!document.is_dirty());
            assert!(!document.undo());
        } else { assert_roundtrip(&mut document, source.as_bytes(), &text); }
    }
}

#[test]
fn task_style_switching_to_numbered_renumbers_the_selected_run() {
    let source = b"- [X] First\n- [ ] Second\n- [x] Third";
    let mut document = open(source);
    let end = document.text().len();
    document.set_list_style(0..end, Some(ListStyle::Numbered)).unwrap();
    assert_eq!(document.source_bytes(), b"1. First\n2. Second\n3. Third");
    assert_eq!(states(&document), [None, None, None]);
    assert_roundtrip(&mut document, source, "First\nSecond\nThird");
    let source = b"3) [X] First\n4) [ ] Second\n5) [x] Third";
    let mut document = open(source);
    document.set_list_style(6..12, Some(ListStyle::Numbered)).unwrap();
    assert_eq!(document.source_bytes(), b"3) [X] First\n4) Second\n5) [x] Third");
    assert_roundtrip(&mut document, source, "First\nSecond\nThird");
}

#[test]
fn task_list_controls_in_continuations_target_the_whole_owning_item() {
    let source = b"- [X] parent\n\n  continuation\n\n  - [x] child\n- sibling";
    let mut document = open(source);
    let text = document.text().to_owned();
    let at = text.find("continuation").unwrap();
    assert!(document.can_set_task_list(at..at));
    document.set_list_style(at..at, Some(ListStyle::Task)).unwrap();
    assert_eq!(document.source_bytes(), source);
    document.set_list_style(at..at, Some(ListStyle::Bullet)).unwrap();
    assert_eq!(document.source_bytes(), b"- parent\n\n  continuation\n\n  - [x] child\n- sibling");
    assert_roundtrip(&mut document, source, &text);
    let mut document = open(source);
    document.set_list_style(at..at, None).unwrap();
    assert_eq!(document.text(), text);
    assert!(String::from_utf8(document.source_bytes()).unwrap().contains("- [x] child"));
    assert_eq!(states(&document).into_iter().filter_map(|state| state).collect::<Vec<_>>(), [true]);
    assert_roundtrip(&mut document, source, &text);
}

#[test]
fn task_list_family_changes_reindent_complete_fenced_descendants() {
    for (source, style, expected) in [
        ("1000. [x] parent\n\n      ```\n      code\n      ```", ListStyle::Bullet,
         "- parent\n\n  ```\n  code\n  ```"),
        ("1000. parent\n\n      ```\n      code\n      ```", ListStyle::Task,
         "- [ ] parent\n\n  ```\n  code\n  ```"),
        ("- > 1000. [x] parent\n  >\n  >       more", ListStyle::Bullet,
         "- > - parent\n  >\n  >   more"),
    ] {
        let mut document = open(source.as_bytes());
        let text = document.text().to_owned();
        document.set_list_style(0..0, Some(style)).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        if source.contains("```") {
            assert_eq!(document.projection().blocks()[1].style.0, "Code Block");
        } else {
            assert!(matches!(document.projection().blocks()[1].kind, BlockKind::ListItem { item_start: false, .. }));
            assert_eq!(document.projection().blocks()[1].quote_depth, 1);
        }
        assert_roundtrip(&mut document, source.as_bytes(), &text);
    }
}

#[test]
fn ordered_task_enter_and_open_lines_create_unchecked_siblings() {
    use super::support::Editor;
    use viem_core::command::Key;
    for (keys, expected, expected_states) in [
        ("oX", "3) [X] task\n4) [ ] X", vec![Some(true), Some(false)]),
        ("A\nX", "3) [X] task\n4) [ ] X", vec![Some(true), Some(false)]),
        ("OX", "3) [ ] X\n4) [X] task", vec![Some(false), Some(true)]),
        ("i\nX", "3) [ ] \n4) [X] Xtask", vec![Some(false), Some(true)]),
    ] {
        let source = "3) [X] task";
        let mut editor = Editor::open(source, Format::Markdown);
        editor.type_keys(keys);
        editor.key(Key::Escape);
        assert_eq!(editor.source(), expected, "{keys}");
        assert_eq!(states(editor.document()), expected_states, "{keys}");
        assert_eq!(open(editor.source().as_bytes()).text(), editor.document().text());
        editor.undo();
        assert_eq!(editor.source(), source, "{keys}");
        editor.redo();
        assert_eq!(editor.source(), expected, "{keys}");
    }
}

#[test]
fn removing_task_treatment_keeps_newly_exposed_body_syntax_literal() {
    for body in ["# heading", "> quote", "---", "```", "[ ] literal", "[X] literal", "1. literal"] {
        let source = format!("- [x] {body}");
        for style in [Some(ListStyle::Bullet), Some(ListStyle::Numbered), None] {
            let mut document = open(source.as_bytes());
            assert_eq!(document.text(), body);
            document.set_list_style(0..0, style)
                .unwrap_or_else(|error| panic!("{body:?} {style:?}: {error:?}"));
            assert_eq!(states(&document), [None]);
            assert_roundtrip(&mut document, source.as_bytes(), body);
        }
    }
}
