use viem_core::document::*;
use super::markdown_test_support::EncodedFixture;

fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}
fn insert(document: &mut Document, at: usize) {
    document.apply_model_request(ModelRequest::SetListStyle { document: document.id(),
        revision: document.revision(), range: at..at, style: Some(ListStyle::Task) }).unwrap();
}
fn toggle(document: &mut Document, at: usize) {
    let (block_id, _) = document.checkbox_at(at, BoundaryAffinity::Downstream).unwrap();
    document.apply_model_request(ModelRequest::ToggleCheckbox { document: document.id(),
        revision: document.revision(), at, block_id }).unwrap();
}
fn states(document: &Document) -> Vec<Option<bool>> {
    document.projection().blocks().iter().map(|block| block.task_checkbox).collect()
}

#[test]
fn recognizes_only_gfm_task_markers_and_preserves_source() {
    for (source, text, expected) in [
        ("- [ ] Todo", "Todo", vec![Some(false)]),
        ("- [\t] Todo", "Todo", vec![Some(false)]),
        ("- [\u{000b}] Todo", "Todo", vec![Some(false)]),
        ("- [\u{000c}] Todo", "Todo", vec![Some(false)]),
        ("- [x] Done", "Done", vec![Some(true)]),
        ("1. [X] Done", "Done", vec![Some(true)]),
        ("> - [ ] **Todo**", "Todo", vec![Some(false)]),
        ("- Parent\n  - [ ] Child", "Parent\nChild", vec![None, Some(false)]),
        ("[ ] ordinary", "[ ] ordinary", vec![None]),
        ("- prefix [ ] text", "prefix [ ] text", vec![None]),
        ("- [ ]no space", "[ ]no space", vec![None]),
        ("- \\[ ] escaped", "[ ] escaped", vec![None]),
        ("- `[ ] code`", "[ ] code", vec![None]),
        ("```\n- [ ] literal\n```", "- [ ] literal", vec![None]),
    ] {
        let document = open(source);
        assert_eq!(document.text(), text, "{source:?}");
        assert_eq!(states(&document), expected, "{source:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn imported_task_whitespace_markers_keep_source_ownership_during_edits() {
    for blank in ['\t', '\u{000b}', '\u{000c}'] {
        let source = format!("- [ ] first\n- [{blank}] second");
        let mut document = open(&source);
        document.insert(6, " ").unwrap();
        assert_eq!(document.text(), "first\n second");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        document.delete(5..6).unwrap();
        assert_eq!(document.text(), "firstsecond");
        assert_eq!(states(&document), [Some(false)]);
        assert_eq!(document.source_bytes(), b"- [ ] firstsecond");
    }
}

#[test]
fn inserts_into_paragraphs_and_existing_items_without_changing_text() {
    for (source, at, expected) in [
        ("", 0, "- [ ] "),
        ("Hello world", 5, "- [ ] Hello world"),
        ("first\nsecond", 8, "- [ ] first\nsecond"),
        ("**Bold**", 2, "- [ ] **Bold**"),
        ("- existing", 3, "- [ ] existing"),
        ("3) existing", 3, "- [ ] existing"),
        ("> paragraph", 2, "> - [ ] paragraph"),
        ("> - **quoted item**", 2, "> - [ ] **quoted item**"),
        ("- first\n\n  second", 8, "- [ ] first\n\n  second"),
    ] {
        let mut document = open(source);
        let text = document.text().to_owned();
        assert!(document.can_set_task_list(at..at), "{source:?}");
        insert(&mut document, at);
        assert_eq!(document.source_bytes(), expected.as_bytes(), "{source:?}");
        assert_eq!(document.text(), text, "{source:?}");
        assert_eq!(document.checkbox_at(at, BoundaryAffinity::Downstream).map(|(_, checked)| checked), Some(false));
        assert!(document.can_set_task_list(at..at));
    }
}

#[test]
fn unavailable_contexts_do_not_offer_checkbox_insertion() {
    for source in ["# Heading", "```\nbody\n```", "---\nkey: value\n---", "<div>literal</div>",
        "| One |\n| --- |\n| Two |", "- # Heading", "- > quoted", "- ```\n  code\n  ```"] {
        let document = open(source);
        assert!(!document.can_set_task_list(0..0), "{source:?}");
    }
    let mut document = open("ordinary");
    document.set_read_only(true);
    assert!(document.can_set_task_list(0..0));
    document.set_read_only(false);
    document.set_modifiable(false);
    assert!(!document.can_set_task_list(0..0));
    let literal = Document::from_bytes(b"- [ ] literal".to_vec(), Encoding::Utf8, Format::PlainText);
    assert!(!literal.unwrap().can_set_task_list(0..0));
}

#[test]
fn toggle_changes_only_marker_state_and_history_restores_exact_bytes() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let fixture = EncodedFixture { encoding, file_format, bom: encoding != Encoding::Latin1 };
            let mut document = fixture.open("+ [X] **Done**\n+ Untouched");
            let before = document.source_bytes();
            toggle(&mut document, 0);
            let after = fixture.bytes("+ [ ] **Done**\n+ Untouched");
            fixture.assert_history(&mut document, &before, &after);
            assert_eq!(states(&document)[0], Some(false));
        }
    }
}

#[test]
fn checkbox_capability_does_not_scan_source_or_large_lists() {
    for count in [32, 12_000] {
        let source = format!("- Parent\n{}  - Target\n- Following", "  - Child\n".repeat(count));
        let document = open(&source);
        let at = document.text().find("Target").unwrap();
        let (available, work) = measure_document_work(|| document.can_set_task_list(at..at));
        assert!(available);
        assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
        assert_eq!(work.markdown_block_parse_calls, 0, "{work:?}");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
        assert_eq!(work.scratch_documents, 0, "{work:?}");
        assert!(work.list_capability_nodes_visited < 200, "{work:?}");
        assert!(work.list_capability_blocks_visited < 400, "{work:?}");
    }
    let document = open(&format!("- {}", "a".repeat(128 * 1024)));
    let (_, work) = measure_document_work(|| document.can_set_task_list(64 * 1024..64 * 1024));
    assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
    assert_eq!(work.markdown_block_parse_calls, 0, "{work:?}");
}

#[test]
fn enter_continues_a_checked_task_with_an_unchecked_item() {
    let mut document = open("- [x] Done");
    document.apply_model_request(ModelRequest::ContinueList { document: document.id(),
        revision: document.revision(), at: 4 }).unwrap();
    assert_eq!(document.source_bytes(), b"- [x] Done\n- [ ] ");
    assert_eq!(states(&document), [Some(true), Some(false)]);
}

#[test]
fn editing_task_text_matches_fresh_projection() {
    let mut document = open("- [x] One\n- [ ] Two\n\nUnrelated");
    document.insert(1, "!").unwrap();
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(states(&document), states(&fresh));
}

#[test]
fn ordinary_task_body_edits_preserve_hidden_markers_and_round_trip() {
    let source = "- [ ] task\n  - [x] nested done\n- [X] upper";
    for (range, replacement) in [
        (0..0, " "), (5..5, " "), (17..17, " "),
        (0..4, ""), (5..16, ""), (17..22, ""),
        (4..5, ""), (4..5, " "), (16..17, ""), (16..17, " "),
        (1..11, ""), (4..17, "x"), (6..16, ""), (8..19, "x"), (14..19, ""),
    ] {
        let mut document = open(source);
        let mut expected = document.text().to_owned();
        expected.replace_range(range.clone(), replacement);
        document.replace(range.clone(), replacement)
            .unwrap_or_else(|error| panic!("{range:?} {replacement:?}: {error:?}"));
        assert_eq!(document.text(), expected, "{range:?} {replacement:?}");
        let after = document.source_bytes();
        let fresh = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(fresh.text(), expected, "{range:?} {replacement:?}");
        assert_eq!(states(&document), states(&fresh));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), after);
    }
}

#[test]
fn task_list_matching_style_paste_uses_the_visible_paragraph_owner() {
    use super::support::Editor;
    use viem_core::{CoreEvent};
    let source = "- [ ] task\n  - [x] nested done\n- [X] upper";
    for (start, end, affinity, pasted) in [
        (5, 5, BoundaryAffinity::Upstream, "## H\n  - x\n**y**"),
        (17, 17, BoundaryAffinity::Upstream, "## H\n  - x\n**y**"),
        (3, 17, BoundaryAffinity::Downstream, "l1\n  - l2"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        editor.type_keys("i");
        editor.place(start);
        editor.core.handle(editor.view, CoreEvent::PlaceCursor {
            document_revision: editor.document().revision(), text_offset: end,
            affinity, extend_selection: end != start }).unwrap();
        let mut expected = editor.document().text().to_owned();
        expected.replace_range(start..end, pasted);
        editor.paste_matching_style(pasted);
        assert_eq!(editor.document().text(), expected);
        let after = editor.source();
        assert_eq!(open(&after).text(), expected);
        editor.undo();
        assert_eq!(editor.source(), source);
        editor.redo();
        assert_eq!(editor.source(), after);
    }
}

#[test]
fn character_put_across_task_items_keeps_the_first_body_inline() {
    use super::support::Editor;
    let source = "- [ ] task\n  - [x] nested done\n- [X] upper";
    for at in [0, 11] {
        let mut editor = Editor::open(source, Format::Markdown);
        editor.place(at);
        editor.type_keys("veey$p");
        let after = editor.source();
        let fresh = open(&after);
        assert_eq!(editor.document().text(), fresh.text());
        assert_eq!(states(editor.document()), states(&fresh));
        editor.undo();
        assert_eq!(editor.source(), source);
        editor.redo();
        assert_eq!(editor.source(), after);
    }
}

#[test]
fn task_markers_do_not_change_cross_item_link_or_image_eligibility() {
    let task = open("- [ ] task\n  - [x] nested done\n- [X] upper");
    let plain = open("- task\n  - nested done\n- upper");
    assert_eq!(task.text(), plain.text());
    for range in [1..11, 1..12, 3..17, 4..18, 8..19, 8..20, 14..19, 14..20] {
        assert_eq!(task.can_insert_link(range.clone()), plain.can_insert_link(range.clone()));
        assert_eq!(task.can_insert_image(range.clone()), plain.can_insert_image(range.clone()));
        assert!(!task.can_insert_link(range.clone()));
        assert!(!task.can_insert_image(range));
    }
}

#[test]
fn typing_into_terminal_empty_task_stays_before_the_final_ending() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            let fixture = EncodedFixture { encoding, file_format, bom: encoding != Encoding::Latin1 };
            let mut document = fixture.open("- [ ] \n");
            let before = document.source_bytes();
            document.insert(0, "X").unwrap();
            assert_eq!(document.text(), "X");
            fixture.assert_history(&mut document, &before, &fixture.bytes("- [ ] X\n"));
            assert_eq!(states(&document), [Some(false)]);
        }
    }
}

#[test]
fn enter_on_empty_task_exits_without_exposing_the_marker() {
    let mut document = open("- [ ] ");
    assert_eq!(document.text(), "");
    assert_eq!(states(&document), [Some(false)]);
    document.apply_model_request(ModelRequest::ContinueList { document: document.id(),
        revision: document.revision(), at: 0 }).unwrap();
    assert_eq!(document.text(), "");
    assert_eq!(states(&document), [None]);
    assert!(!String::from_utf8(document.source_bytes()).unwrap().contains("[ ]"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"- [ ] ");
}

#[test]
fn stale_checkbox_requests_leave_source_and_history_unchanged() {
    let mut document = open("- [x] Task");
    let (block_id, _) = document.checkbox_at(0, BoundaryAffinity::Downstream).unwrap();
    let request = ModelRequest::ToggleCheckbox { document: document.id(), revision: document.revision(),
        at: 0, block_id };
    document.insert(1, "!").unwrap();
    let before = document.source_bytes();
    assert!(document.apply_model_request(request).is_err());
    assert_eq!(document.source_bytes(), before);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"- [x] Task");
}

#[test]
fn creating_a_task_preserves_encoded_source_and_exact_history() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        let fixture = EncodedFixture { encoding, file_format: FileFormat::Dos, bom: encoding != Encoding::Latin1 };
        let mut document = fixture.open("**Caf\u{e9}**\nsecond\n\nUntouched");
        let before = document.source_bytes();
        insert(&mut document, 1);
        fixture.assert_history(&mut document, &before, &fixture.bytes("- [ ] **Caf\u{e9}**\nsecond\n\nUntouched"));
        assert_eq!(states(&document)[0], Some(false));
    }
}

#[test]
fn typed_task_markers_share_insertion_history_and_literal_next_is_preserved() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    for marker in ["[ ]", "[x]", "[X]"] {
        let mut document = open("- ");
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        for ch in format!("{marker} Task").chars() {
            commands.handle(&mut document, InputEvent::key(ch)).unwrap();
        }
        assert_eq!(document.text(), "Task", "{marker}: {:?}", document.source_bytes());
        assert_eq!(states(&document), [Some(marker != "[ ]")]);
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        let after = document.source_bytes();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), b"- ");
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_eq!(document.source_bytes(), after);
    }
    let mut document = open("- ");
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut document, InputEvent::key('i')).unwrap();
    for ch in "[ ]".chars() { commands.handle(&mut document, InputEvent::key(ch)).unwrap(); }
    commands.handle(&mut document, InputEvent::Key(Key::Ctrl('v'))).unwrap();
    commands.handle(&mut document, InputEvent::key(' ')).unwrap();
    for ch in "literal".chars() { commands.handle(&mut document, InputEvent::key(ch)).unwrap(); }
    assert_eq!(document.text(), "[ ] literal");
    assert_eq!(states(&document), [None]);
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(fresh.text(), "[ ] literal");
    assert_eq!(states(&fresh), [None]);
}

fn paste_task_fragment(document: &mut Document, fragment: ClipboardFragment) {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent,
        ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
    let value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), value["plain_text"].as_str().unwrap()).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1),
        ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap()),
    ));
    let mut commands = CommandInterpreter::new();
    super::markdown_test_support::place_insert_boundary(&mut commands, document, 0);
    let result = commands.handle_with_clipboard_context(document,
        InputEvent::Key(Key::PasteClipboard), &context).unwrap();
    assert_eq!(result.status, CommandStatus::Complete);
    assert!(result.document_changed);
    commands.handle(document, InputEvent::Key(Key::Escape)).unwrap();
}

#[test]
fn clipboard_keeps_selected_task_items_and_leaves_donor_source_intact() {
    let source = "+ [X] **First**\n+ [ ] Second\n\nTail";
    let donor = open(source);
    for range in [0..5, 0..12, 0..donor.text().len()] {
        let copied = donor.text()[range.clone()].to_owned();
        let expected = if range.end == 5 { vec![Some(true)] }
            else if range.end == 12 { vec![Some(true), Some(false)] }
            else { vec![Some(true), Some(false), None] };
        let fragment = donor.clipboard_fragment(range).unwrap();
        let mut target = open("");
        paste_task_fragment(&mut target, fragment);
        assert_eq!(target.text(), copied);
        assert_eq!(states(&target), expected);
        let saved = target.source_bytes();
        assert!(String::from_utf8_lossy(&saved).contains("[X] **First**"));
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(states(&reopened), expected);
        assert!(target.undo());
        assert!(target.source_bytes().is_empty());
        assert!(target.redo());
        assert_eq!(target.source_bytes(), saved);
        assert_eq!(states(&target), expected);
        assert_eq!(donor.source_bytes(), source.as_bytes());
        assert!(!donor.is_dirty());
    }
}

#[test]
fn copying_and_moving_task_items_preserves_checked_state_and_exact_history() {
    let source = "- [X] First\n- [ ] Second";
    for (operation, expected_text, expected_states) in [
        (HardLineTransfer::Copy, "First\nSecond\nFirst", vec![Some(true), Some(false), Some(true)]),
        (HardLineTransfer::Move, "Second\nFirst", vec![Some(false), Some(true)]),
    ] {
        let mut document = open(source);
        document.apply_model_request(ModelRequest::TransferHardLines { document: document.id(),
            revision: document.revision(), operation, source_lines: 0..1, destination: 2 }).unwrap();
        assert_eq!(document.text(), expected_text);
        assert_eq!(states(&document), expected_states);
        let saved = document.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(states(&reopened), expected_states);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn preview_switching_preserves_task_source_and_body_anchor() {
    let source = "\u{feff}- [X] **First**\r\n- [ ] Second\r\n";
    let mut document = Document::from_bytes_with_file_format(source.as_bytes().to_vec(),
        Encoding::Utf8, Format::Markdown, FileFormat::Dos).unwrap();
    for target in [Format::Code, Format::Markdown] {
        let at = document.text().find("First").unwrap();
        let anchor = document.text_anchor(document.text_point(at).unwrap(), Association::AfterInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding).unwrap();
        let prepared = document.prepare_model_request(ModelRequest::SetViewFormat {
            document: document.id(), revision: document.revision(), target }).unwrap();
        assert!(prepared.summary().source_patches().is_empty());
        let mapped = match prepared.text_position_map().map_text_anchor(anchor).unwrap() {
            MappingOutcome::Exact(anchor) | MappingOutcome::Moved(anchor) => anchor,
            other => panic!("task body anchor lost: {other:?}"),
        };
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(mapped.offset(), document.text().find("First").unwrap());
        assert_eq!(document.source_bytes(), source.as_bytes());
        if target == Format::Markdown { assert_eq!(states(&document), [Some(true), Some(false)]); }
        else { assert!(states(&document).iter().all(Option::is_none)); }
    }
    assert!(document.undo());
    assert_eq!(document.format(), Format::Code);
    assert!(document.undo());
    assert_eq!(document.format(), Format::Markdown);
    assert_eq!(states(&document), [Some(true), Some(false)]);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn checkbox_edits_keep_unrelated_paragraphs_and_list_items_out_of_reprojection() {
    for count in [64, 640] {
        for source in [
            format!("Target\n\nGuard\n\n{}", "Unrelated paragraph.\n\n".repeat(count)),
            format!("{}- Target\n- Following", "- Sibling\n".repeat(count)),
        ] {
            let mut document = open(&source);
            let at = document.text().find("Target").unwrap();
            let (_, insert_work) = measure_document_work(|| insert(&mut document, at));
            let (_, toggle_work) = measure_document_work(|| toggle(&mut document, at));
            for work in [insert_work, toggle_work] {
                assert_eq!(work.full_projection_candidates, 0, "count={count}: {work:?}");
                assert_eq!(work.source_full_materializations, 0, "count={count}: {work:?}");
                assert_eq!(work.formatted_full_materialized_bytes, 0, "count={count}: {work:?}");
                assert_eq!(work.compatibility_record_materializations, 0, "count={count}: {work:?}");
                assert!(work.source_decoded_bytes < 8192, "count={count}: {work:?}");
                assert!(work.markdown_block_parsed_bytes < 8192, "count={count}: {work:?}");
            }
            let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(states(&document), states(&fresh));
        }
    }
}

#[test]
fn html_export_preserves_task_states_without_changing_source() {
    let source = "- [X] **Done**\n- [ ] Todo\n- Ordinary\n\n`[x] literal`";
    let document = open(source);
    let html = String::from_utf8(document.prepare_html_export().render().unwrap()).unwrap();
    assert_eq!(html.matches("<input type=\"checkbox\"").count(), 2);
    assert_eq!(html.matches("type=\"checkbox\" disabled checked").count(), 1);
    assert_eq!(html.matches("type=\"checkbox\" disabled style=").count(), 1);
    assert_eq!(html.matches("list-style-type:none").count(), 2);
    assert!(html.contains("width:1cap;height:1cap"));
    assert!(html.contains("Done") && html.contains("Todo") && html.contains("Ordinary"));
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(!document.is_dirty());
    let literal = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let literal_html = String::from_utf8(literal.prepare_html_export().render().unwrap()).unwrap();
    assert!(!literal_html.contains("<input"));
}
