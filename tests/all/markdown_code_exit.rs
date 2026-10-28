//! Ordinary code exits after two blank rows and front matter after one. The
//! resulting paragraph is usable before typing and survives reopening.
use crate::support::{assert_exact_undo_redo, assert_reopens_as_displayed, Editor};
use crate::markdown_test_support::{place_insert_boundary, EncodedFixture};
use viem_core::command::{CommandInterpreter, InputEvent, Key, Mode};
use viem_core::document::{
    measure_document_work, Block, BlockKind, BoundaryAffinity, ContainerIdentity, ContainerKind,
    Document, Encoding, FileFormat, Format, ModelRequest,
};
use viem_core::CoreEvent;

#[track_caller]
fn press(editor: &mut Editor, key: Key) {
    let source = editor.source();
    let cursor = editor.core.command_state(editor.view).unwrap().cursor();
    editor.core.handle(editor.view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| panic!("{key:?} at {cursor}, source={source:?}: {error:?}"));
}

fn is_code(block: &Block) -> bool {
    block.style.0 == "Code Block"
        || block.containers.iter().any(|member| member.container.kind == ContainerKind::CodeBlock)
}

fn list_depth(block: &Block) -> usize {
    block.containers.iter().filter(|member| member.container.kind == ContainerKind::ListItem).count()
}

fn list_owner(block: &Block) -> Option<ContainerIdentity> {
    block.containers.iter().rev().find(|member| member.container.kind == ContainerKind::ListItem)
        .map(|member| member.container.id)
}

fn code(editor: &Editor) -> Block {
    editor.document().projection().blocks().iter().find(|block| is_code(block))
        .expect("fixture has a code owner").clone()
}

fn caret_owner(editor: &Editor) -> Block {
    let at = editor.core.command_state(editor.view).unwrap().cursor();
    editor.document().projection().blocks().iter()
        .filter(|block| block.range.start <= at && at <= block.range.end)
        .max_by_key(|block| block.range.start).expect("caret has a paragraph").clone()
}

fn insert_at(editor: &mut Editor, at: usize) {
    press(editor, Key::Char('i'));
    editor.place(at);
    let state = editor.core.command_state(editor.view).unwrap();
    assert_eq!(state.mode(), Mode::Insert);
    assert_eq!(state.cursor(), at, "fixture must place an actual insertion boundary");
}

fn assert_exited(editor: &mut Editor, original: &Block, retained: &str, context: &str) {
    let previous = code(editor);
    assert_eq!(&editor.document().text()[previous.range.clone()], retained, "{context}");
    assert_eq!(previous.code_language, original.code_language, "{context}");
    assert_eq!(previous.code_kind, original.code_kind, "{context}");
    let paragraph = caret_owner(editor);
    assert!(paragraph.range.is_empty() && !is_code(&paragraph), "{context}: {paragraph:?}");
    assert_eq!(paragraph.quote_depth, original.quote_depth, "{context}");
    assert_eq!(list_depth(&paragraph), list_depth(original), "{context}");
    if list_depth(original) > 0 {
        assert_eq!(list_owner(&paragraph), list_owner(&previous), "{context}");
        assert!(matches!(paragraph.kind, BlockKind::ListItem { item_start: false, .. }), "{context}");
    }
    let at = editor.core.command_state(editor.view).unwrap().cursor();
    assert_eq!(at, previous.range.end + 1, "{context}: caret is in the following paragraph");
    let layout = editor.core.layout(editor.view).unwrap().snapshot().unwrap();
    let point = layout.caret_point(at, BoundaryAffinity::Downstream).unwrap();
    let geometry = layout.caret_geometry(point).unwrap();
    assert_eq!(layout.rows[geometry.row_index].paragraph_id, Some(paragraph.id), "{context}: caret row owns new paragraph");
    assert_reopens_as_displayed(editor.document(), context);
}

#[test]
fn three_enters_exit_a_final_code_block_before_any_text_is_typed() {
    for (source, retained) in [
        ("```rust\ncode\n```", "code"),
        ("```rust\ncode\n```\n", "code"),
        ("~~~rust title=\"keep\"\ncode\n~~~~~ \t", "code"),
        ("~~~rust title=\"keep\"\ncode\n~~~~~ \t\n", "code"),
        ("```rust\ncode", "code"),
        ("~~~rust\ncode", "code"),
        ("    code", "code"),
        ("    code\n", "code"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        insert_at(&mut editor, original.range.end);
        press(&mut editor, Key::Enter);
        assert!(is_code(&caret_owner(&editor)), "{source:?}: first Enter stays in code");
        press(&mut editor, Key::Enter);
        assert!(is_code(&caret_owner(&editor)), "{source:?}: second Enter stays in code");
        press(&mut editor, Key::Enter);
        assert_exited(&mut editor, &original, retained, source);
        if source.starts_with("~~~rust title") {
            assert!(editor.source().starts_with("~~~rust title=\"keep\"\ncode\n~~~~~ \t"), "closer spelling: {:?}", editor.source());
        }
        editor.type_keys("after");
        assert!(!is_code(&caret_owner(&editor)), "{source:?}: typing stays outside code");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn a_single_blank_line_at_the_end_of_code_can_be_kept_and_typed_into() {
    let source = "```rust\ncode\n```";
    let mut editor = Editor::open(source, Format::Markdown);
    let end = code(&editor).range.end;
    insert_at(&mut editor, end);
    press(&mut editor, Key::Enter);
    press(&mut editor, Key::Escape);
    assert_eq!(editor.source(), "```rust\ncode\n\n```");
    assert_reopens_as_displayed(editor.document(), source);
    let end = code(&editor).range.end;
    insert_at(&mut editor, end);
    editor.type_keys("next");
    assert_eq!(editor.source(), "```rust\ncode\nnext\n```");
    assert!(is_code(&caret_owner(&editor)));
    assert_reopens_as_displayed(editor.document(), source);
}

#[test]
fn replace_mode_enter_exits_and_groups_following_typing_with_the_exit() {
    let source = "```rust\ncode\n\n\n```";
    let mut editor = Editor::open(source, Format::Markdown);
    let original = code(&editor);
    press(&mut editor, Key::Char('R'));
    editor.place(original.range.end);
    press(&mut editor, Key::Enter);
    assert_eq!(editor.core.command_state(editor.view).unwrap().mode(), Mode::Replace);
    assert_exited(&mut editor, &original, "code", source);
    editor.type_keys("after");
    assert!(!is_code(&caret_owner(&editor)));
    assert_reopens_as_displayed(editor.document(), source);
    let saved = editor.source();
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn dot_repeats_the_semantic_enter_exit_after_undo() {
    let source = "```rust\ncode\n\n\n```";
    let mut editor = Editor::open(source, Format::Markdown);
    let original = code(&editor);
    insert_at(&mut editor, original.range.end);
    press(&mut editor, Key::Enter);
    press(&mut editor, Key::Escape);
    let saved = editor.source();
    press(&mut editor, Key::Char('u'));
    assert_eq!(editor.source(), source);
    editor.place(original.range.end);
    press(&mut editor, Key::Char('.'));
    assert_eq!(editor.source(), saved);
    assert_exited(&mut editor, &original, "code", source);
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn unclosed_fence_with_existing_final_endings_gains_its_matching_closer() {
    for source in ["```rust\ncode\n", "~~~~rust\ncode\n\n"] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        let body = &editor.document().text()[original.range.clone()];
        let trailing = body.bytes().rev().take_while(|byte| *byte == b'\n').count();
        assert!(trailing <= 2, "fixture starts at most two rows after content");
        insert_at(&mut editor, original.range.end);
        for _ in 0..3 - trailing { press(&mut editor, Key::Enter); }
        assert_exited(&mut editor, &original, "code", source);
        let marker = if source.starts_with('~') { "~~~~" } else { "```" };
        assert!(editor.source().contains(&format!("code\n{marker}")), "{:?}", editor.source());
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn autoindented_blank_rows_and_existing_whitespace_rows_allow_exit() {
    for (source, enters, retained) in [
        ("```rust\n  code\n```", 3, "  code"),
        ("```rust\ncode\n \t\n\t \n```", 1, "code"),
        ("```rust\ncode\n\n\n\n```", 1, "code\n"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        insert_at(&mut editor, original.range.end);
        for _ in 0..enters { press(&mut editor, Key::Enter); }
        assert_exited(&mut editor, &original, retained, source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn exiting_nested_code_keeps_its_surrounding_quote_and_list_item() {
    for source in [
        "> ```rust\n> code\n> ```",
        ">   ```rust\n>   code\n>   ```",
        "  ```rust\n  code\n  ```",
        "- ```rust\n  code\n  ```",
        "-\t```rust\n\tcode\n\t```",
        "- item\n\n  ```rust\n  code\n  ```",
        "> - ```rust\n>   code\n>   ```",
        "- > ```rust\n  > code\n  > ```",
        "- parent\n  - ```rust\n    code\n    ```",
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        insert_at(&mut editor, original.range.end);
        for body in ["code\n", "code\n\n"] {
            press(&mut editor, Key::Enter);
            let owner = code(&editor);
            assert_eq!(&editor.document().text()[owner.range], body, "{source:?}: {:?}", editor.source());
            assert_reopens_as_displayed(editor.document(), source);
        }
        press(&mut editor, Key::Enter);
        assert_exited(&mut editor, &original, "code", source);
        editor.type_keys("after");
        let paragraph = caret_owner(&editor);
        assert!(!is_code(&paragraph), "{source:?}");
        assert_eq!(paragraph.quote_depth, original.quote_depth, "{source:?}");
        assert_eq!(list_depth(&paragraph), list_depth(&original), "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn exiting_code_before_following_prose_retains_that_prose_and_its_source() {
    let source = "```rust\ncode\n\n\n```\n\n**Tail** &amp; [link](target)\n";
    let mut editor = Editor::open(source, Format::Markdown);
    let original = code(&editor);
    insert_at(&mut editor, original.range.end);
    press(&mut editor, Key::Enter);
    assert_exited(&mut editor, &original, "code", source);
    assert!(editor.source().ends_with("\n\n**Tail** &amp; [link](target)\n"));
    let saved = editor.source();
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn blank_rows_inside_code_do_not_exit_the_owner() {
    let source = "```rust\ncode\n\n\nmore\n```";
    let mut editor = Editor::open(source, Format::Markdown);
    let at = editor.document().text().find("more").unwrap() - 1;
    insert_at(&mut editor, at);
    press(&mut editor, Key::Enter);
    assert_eq!(editor.source(), "```rust\ncode\n\n\n\nmore\n```");
    assert!(is_code(&caret_owner(&editor)));
    assert_eq!(editor.document().projection().blocks().len(), 1);
    let saved = editor.source();
    assert_reopens_as_displayed(editor.document(), source);
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn explicit_hard_break_and_open_line_keep_trailing_blank_code_rows() {
    let source = "```rust\ncode\n\n\n```";
    for key in [Key::ShiftEnter, Key::Char('o')] {
        let mut editor = Editor::open(source, Format::Markdown);
        let end = code(&editor).range.end;
        if key == Key::ShiftEnter {
            insert_at(&mut editor, end);
        } else {
            editor.place(end);
        }
        press(&mut editor, key);
        assert!(is_code(&caret_owner(&editor)), "{key:?}");
        assert_eq!(editor.document().projection().blocks().len(), 1, "{key:?}");
        assert!(editor.document().text().ends_with("\n\n\n"), "{key:?}: {:?}", editor.document().text());
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn initially_empty_code_block_exits_when_a_second_blank_row_already_exists() {
    let source = "```rust\n```";
    let mut editor = Editor::open(source, Format::Markdown);
    let original = code(&editor);
    insert_at(&mut editor, original.range.end);
    press(&mut editor, Key::Enter);
    assert!(is_code(&caret_owner(&editor)));
    press(&mut editor, Key::Enter);
    assert_exited(&mut editor, &original, "", source);
    let saved = editor.source();
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn passive_pre_keeps_its_literal_enter_behavior() {
    for source in ["<pre lang=\"rust\">code\n\n</pre>", "<pre>code\n\n</pre>"] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        insert_at(&mut editor, original.range.end);
        press(&mut editor, Key::Enter);
        let after = code(&editor);
        assert_eq!(after.range.len(), original.range.len() + 1, "{source:?}");
        assert!(is_code(&caret_owner(&editor)), "{source:?}");
        assert_eq!(after.code_kind, original.code_kind, "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn two_enters_exit_front_matter_before_any_text_is_typed() {
    for source in [
        "---\nname: example\n---",
        "---\nname: example\n---\n",
        "--- \t\nname: example\n---\t ",
        "---\nname: example\n---\n\n**Tail** &amp; [link](target)\n",
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        assert!(original.is_front_matter());
        insert_at(&mut editor, original.range.end);
        press(&mut editor, Key::Enter);
        assert!(caret_owner(&editor).is_front_matter(), "first Enter: {source:?}");
        assert_eq!(&editor.document().text()[code(&editor).range], "name: example\n");
        assert_reopens_as_displayed(editor.document(), source);
        press(&mut editor, Key::Enter);
        assert_exited(&mut editor, &original, "name: example", source);
        if source.contains("**Tail**") {
            assert!(editor.source().ends_with("\n\n**Tail** &amp; [link](target)\n"));
        }
        editor.type_keys("after");
        assert!(!is_code(&caret_owner(&editor)), "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn enter_on_an_existing_blank_front_matter_row_exits_and_retains_its_envelope() {
    for (source, retained) in [
        ("---\n---", ""),
        ("---\n\n---", ""),
        ("---\nname: example\n\n---", "name: example"),
        ("---\nname: example\n \t\n---", "name: example"),
        ("---\nname: example\n\n\n---", "name: example\n"),
        ("---\n\n---\n\n**Tail**", ""),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        let original = code(&editor);
        insert_at(&mut editor, original.range.end);
        press(&mut editor, Key::Enter);
        assert_exited(&mut editor, &original, retained, source);
        assert!(code(&editor).is_front_matter());
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn whitespace_only_front_matter_row_exits_on_enter() {
    let source = "---\n  name: example\n---";
    let mut editor = Editor::open(source, Format::Markdown);
    let original = code(&editor);
    insert_at(&mut editor, original.range.end);
    press(&mut editor, Key::Enter);
    editor.type_keys("  ");
    assert_eq!(&editor.document().text()[code(&editor).range], "  name: example\n  ");
    press(&mut editor, Key::Enter);
    assert_exited(&mut editor, &original, "  name: example", source);
    let saved = editor.source();
    assert_exact_undo_redo(&mut editor, source, &saved, source);
}

#[test]
fn front_matter_internal_blank_rows_and_explicit_line_commands_stay_literal() {
    for (source, at, key, expected) in [
        ("---\nfirst: 1\n\nlast: 2\n---", "first: 1\n".len(), Key::Enter, "first: 1\n\n\nlast: 2"),
        ("---\nname: example\n\n---", "name: example\n".len(), Key::ShiftEnter, "name: example\n\n"),
        ("---\nname: example\n\n---", "name: example\n".len(), Key::Char('o'), "name: example\n\n"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        if key == Key::Char('o') { editor.place(at); }
        else { insert_at(&mut editor, at); }
        press(&mut editor, key);
        assert!(caret_owner(&editor).is_front_matter(), "{source:?}, {key:?}");
        assert_eq!(editor.document().text(), expected, "{source:?}, {key:?}");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn backspace_deletes_empty_front_matter_and_ordinary_code_envelopes() {
    for envelope in ["---\n---", "--- \t\n\n---\t", "```rust\n```", "~~~rust\n\n~~~~ "] {
        for suffix in ["", "\n", "\n\n**Tail** &amp; [link](target)\n"] {
            let source = format!("{envelope}{suffix}");
            let mut editor = Editor::open(&source, Format::Markdown);
            let original = code(&editor);
            assert!(original.range.is_empty(), "{source:?}");
            insert_at(&mut editor, original.range.start);
            press(&mut editor, Key::Backspace);
            assert_eq!(editor.source(), suffix, "{source:?}");
            assert!(editor.document().projection().blocks().iter().all(|block| !is_code(block)), "{source:?}");
            assert_eq!(editor.core.command_state(editor.view).unwrap().cursor(), 0);
            assert!(!is_code(&caret_owner(&editor)));
            assert_reopens_as_displayed(editor.document(), &source);
            let saved = editor.source();
            assert_exact_undo_redo(&mut editor, &source, &saved, &source);
        }
    }
}

#[test]
fn backspace_removes_empty_nested_code_envelopes_and_retains_parent_containers() {
    for (opening_prefix, continuation) in [
        ("> ", "> "),
        ("-\t", "\t"),
        ("- > ", "  > "),
        ("> - ", ">   "),
        ("> - > ", ">   > "),
    ] {
        for closed in [false, true] {
            let before = "__Before__ &amp; [link](before)\n\n";
            let after = if closed { "\n\n**Tail** &amp; [link](after)\n" } else { "" };
            let closer = if closed { format!("\n{continuation}```") } else { String::new() };
            let source = format!("{before}{opening_prefix}```rust{closer}{after}");
            let expected = format!("{before}{opening_prefix}{after}");
            let mut editor = Editor::open(&source, Format::Markdown);
            let original = code(&editor);
            assert!(original.range.is_empty(), "{source:?}");
            let parents = original.containers.iter()
                .filter(|member| member.container.kind != ContainerKind::CodeBlock)
                .map(|member| member.container.kind).collect::<Vec<_>>();
            assert!(!parents.is_empty(), "fixture must exercise a nested owner: {source:?}");

            insert_at(&mut editor, original.range.start);
            press(&mut editor, Key::Backspace);

            assert_eq!(editor.source(), expected, "{source:?}");
            assert_eq!(editor.core.command_state(editor.view).unwrap().cursor(), original.range.start, "{source:?}");
            assert!(editor.document().projection().blocks().iter().all(|block| !is_code(block)), "{source:?}");
            let paragraph = caret_owner(&editor);
            assert!(paragraph.range.is_empty(), "{source:?}: {paragraph:?}");
            assert_eq!(paragraph.kind, original.kind, "{source:?}");
            assert_eq!(paragraph.quote_depth, original.quote_depth, "{source:?}");
            assert_eq!(list_depth(&paragraph), list_depth(&original), "{source:?}");
            assert_eq!(paragraph.containers.iter().map(|member| member.container.kind).collect::<Vec<_>>(), parents, "{source:?}");
            assert_reopens_as_displayed(editor.document(), &source);
            assert_exact_undo_redo(&mut editor, &source, &expected, &source);
        }
    }
}

#[test]
fn front_matter_exit_and_empty_deletion_preserve_encoding_bom_and_endings() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            for bom in [false, true] {
                if encoding == Encoding::Latin1 && bom { continue; }
                let fixture = EncodedFixture { encoding, file_format, bom };
                for (source, expected, key) in [
                    ("--- \t\nname: 'café'\n\n---\t", "--- \t\nname: 'café'\n---\t\n\n", Key::Enter),
                    ("---\n---", "---\n---\n\n", Key::Enter),
                    ("--- \t\n\n---\t\n\n**Tail** &amp; [link](target)\n", "\n\n**Tail** &amp; [link](target)\n", Key::Backspace),
                ] {
                    let before = fixture.bytes(source);
                    let after = fixture.bytes(expected);
                    let mut document = fixture.open(source);
                    let at = document.projection().blocks()[0].range.end;
                    let mut commands = CommandInterpreter::new();
                    place_insert_boundary(&mut commands, &mut document, at);
                    commands.handle(&mut document, InputEvent::Key(key))
                        .unwrap_or_else(|error| panic!("{fixture:?}, {source:?}, {key:?}: {error:?}"));
                    assert_eq!(document.source_bytes(), after, "{fixture:?}, {source:?}, {key:?}");
                    if key == Key::Enter { assert!(document.projection().blocks()[0].is_front_matter()); }
                    else { assert!(document.projection().blocks().iter().all(|block| !is_code(block))); }
                    let owner = document.projection().blocks().iter()
                        .find(|block| block.range == (commands.cursor()..commands.cursor())).unwrap();
                    assert!(!is_code(owner), "{fixture:?}, {key:?}");
                    commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
                    fixture.assert_history(&mut document, &before, &after);
                }
            }
        }
    }
}

#[test]
fn literal_formats_keep_three_enters_as_literal_text() {
    for format in [Format::PlainText, Format::Code] {
        let source = "```rust\ncode\n```";
        let mut editor = Editor::open(source, format);
        insert_at(&mut editor, 12);
        for _ in 0..3 { press(&mut editor, Key::Enter); }
        assert_eq!(editor.source(), "```rust\ncode\n\n\n\n```", "{format:?}");
        assert_reopens_as_displayed(editor.document(), source);
        let saved = editor.source();
        assert_exact_undo_redo(&mut editor, source, &saved, source);
    }
}

#[test]
fn exiting_preserves_bom_encoding_and_untouched_mixed_source_endings() {
    let source = "Intro\r\n\r\n```rust title=x\ncafé\r\n\n\r\n```` \t";
    let expected = "Intro\r\n\r\n```rust title=x\ncafé\r\n```` \t\r\n\r\n";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        for bom in [false, true] {
            if encoding == Encoding::Latin1 && bom { continue; }
            let fixture = EncodedFixture { encoding, file_format: FileFormat::Dos, bom };
            // The encoding helper keeps this fixture's already mixed endings.
            let spelling = EncodedFixture { file_format: FileFormat::Unix, ..fixture };
            let before = spelling.bytes(source);
            let after = spelling.bytes(expected);
            let mut document = Document::from_bytes_with_file_format(
                before.clone(), encoding, Format::Markdown, FileFormat::Dos,
            ).unwrap();
            let at = document.projection().blocks().iter().find(|block| is_code(block)).unwrap().range.end;
            let mut commands = CommandInterpreter::new();
            place_insert_boundary(&mut commands, &mut document, at);
            commands.handle(&mut document, InputEvent::Key(Key::Enter))
                .unwrap_or_else(|error| panic!("{fixture:?}: {error:?}"));
            let owner = document.projection().blocks().iter()
                .find(|block| block.range == (commands.cursor()..commands.cursor())).unwrap();
            assert!(!is_code(owner), "{fixture:?}");
            commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
            fixture.assert_history(&mut document, &before, &after);
        }
    }
}

#[test]
fn interpreter_exit_work_does_not_grow_with_unrelated_paragraphs() {
    for (code_source, body, closed) in [
        ("```rust\ncode\n\n\n```", "code\n\n", true),
        ("```rust\ncode\n\n\n```\n", "code\n\n", true),
        ("```rust\n\n\n```", "\n", true),
        ("```rust\ncode\n\n", "code\n\n", false),
        ("```rust\ncode\n\n\n", "code\n\n\n", false),
    ] {
    for before_code in [false, true] {
        // Source after an unclosed fence belongs to its body, so unrelated
        // paragraphs can precede that case only.
        if !closed && !before_code { continue; }
        let mut measured = None;
        for paragraphs in [100, 1000] {
            let unrelated = "Unrelated *styled* prose.\n\n".repeat(paragraphs);
            let source = if before_code {
                format!("{unrelated}Guard.\n\n{code_source}")
            } else {
                format!("{code_source}\n\nGuard.\n\n{unrelated}")
            };
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let code = document.projection().blocks().iter().find(|block| is_code(block)).unwrap();
            assert_eq!(document.projection().text_tree().slice(code.range.clone()).unwrap(), body, "{code_source:?}");
            let at = code.range.end;
            let mut commands = CommandInterpreter::new();
            place_insert_boundary(&mut commands, &mut document, at);
            let (result, work) = measure_document_work(|| commands.handle(&mut document, InputEvent::Key(Key::Enter)));
            result.unwrap_or_else(|error| panic!("prefix={before_code}, paragraphs={paragraphs}: {error:?}"));
            assert_eq!(work.full_projection_candidates, 0, "{work:?}");
            assert_eq!(work.source_full_materializations, 0, "{work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
            assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{work:?}");
            assert!(work.source_decoded_bytes < 8192, "{work:?}");
            assert!(work.projected_formatted_bytes < 4096, "{work:?}");
            let current = (work.source_decoded_bytes, work.projected_formatted_bytes, work.model_requests);
            if let Some(previous) = measured {
                assert_eq!(previous, current, "unrelated paragraph count changes code-exit work: {work:?}");
            }
            measured = Some(current);
            let owner = document.projection().blocks().iter()
                .find(|block| block.range == (commands.cursor()..commands.cursor())).unwrap();
            assert!(!is_code(owner));
            let saved = document.source_bytes();
            if before_code {
                assert!(saved.starts_with(unrelated.as_bytes()));
            } else {
                assert!(saved.ends_with(unrelated.as_bytes()));
            }
            assert_reopens_as_displayed(&document, &format!("prefix={before_code}, paragraphs={paragraphs}"));
            commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
    }
}

#[test]
fn ineligible_code_exit_requests_leave_source_revision_and_history_unchanged() {
    for (source, format, at) in [
        ("```rust\ncode\n```", Format::Markdown, 4),
        ("```rust\ncode\n\n```", Format::Markdown, 5),
        ("```rust\ncode\n\n\nmore\n```", Format::Markdown, 6),
        ("---\ncode\n---", Format::Markdown, 4),
        ("<pre>code\n\n</pre>", Format::Markdown, 6),
        ("code\n\n", Format::PlainText, 6),
        ("code\n\n", Format::Code, 6),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let revision = document.revision();
        let history = document.history_status();
        let request = ModelRequest::ExitCodeBlock { document: document.id(), revision, at };
        assert!(document.apply_model_request(request).is_err(), "{source:?}, {format:?}");
        assert_eq!(document.source_bytes(), source.as_bytes(), "{source:?}, {format:?}");
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status(), history);
    }
}

#[test]
fn stale_foreign_and_nonmodifiable_code_exit_requests_are_atomic() {
    for (source, at) in [("```rust\ncode\n\n\n```", 6), ("---\ncode\n\n---", 5), ("---\n---", 0)] {
    for failure in ["foreign", "stale", "nonmodifiable", "late policy", "late revision"] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut request = ModelRequest::ExitCodeBlock { document: document.id(), revision: document.revision(), at };
        let prepared = if failure.starts_with("late") {
            Some(document.prepare_model_request(request.clone()).unwrap())
        } else { None };
        if failure == "foreign" {
            let foreign = Document::new("other");
            request = ModelRequest::ExitCodeBlock { document: foreign.id(), revision: document.revision(), at };
        }
        if matches!(failure, "stale" | "late revision") {
            document.insert(0, "x").unwrap();
        }
        if matches!(failure, "nonmodifiable" | "late policy") { document.set_modifiable(false); }
        let before = document.source_bytes();
        let revision = document.revision();
        let history = document.history_status();
        let result = if let Some(prepared) = prepared { document.commit_model_transaction(prepared) }
            else { document.apply_model_request(request) };
        assert!(result.is_err(), "{failure}");
        assert_eq!(document.source_bytes(), before, "{failure}");
        assert_eq!(document.revision(), revision, "{failure}");
        assert_eq!(document.history_status(), history, "{failure}");
    }
    }
}

#[test]
fn empty_envelope_deletion_rejects_foreign_stale_and_nonmodifiable_requests_atomically() {
    for source in ["---\n---", "```rust\n```"] {
        for failure in ["foreign", "stale", "nonmodifiable", "late policy", "late revision"] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let style = document.projection().style_sheet().base_paragraph.clone();
            let mut request = ModelRequest::SetParagraphStyle {
                document: document.id(), revision: document.revision(), range: 0..0, style: style.clone(),
            };
            let prepared = if failure.starts_with("late") {
                Some(document.prepare_model_request(request.clone()).unwrap())
            } else { None };
            if failure == "foreign" {
                let foreign = Document::new("other");
                request = ModelRequest::SetParagraphStyle {
                    document: foreign.id(), revision: document.revision(), range: 0..0, style,
                };
            }
            if matches!(failure, "stale" | "late revision") { document.insert(0, "x").unwrap(); }
            if matches!(failure, "nonmodifiable" | "late policy") { document.set_modifiable(false); }
            let before = document.source_bytes();
            let revision = document.revision();
            let history = document.history_status();
            let result = if let Some(prepared) = prepared { document.commit_model_transaction(prepared) }
                else { document.apply_model_request(request) };
            assert!(result.is_err(), "{source:?}, {failure}");
            assert_eq!(document.source_bytes(), before, "{source:?}, {failure}");
            assert_eq!(document.revision(), revision, "{source:?}, {failure}");
            assert_eq!(document.history_status(), history, "{source:?}, {failure}");
        }
    }
}

#[test]
fn code_exit_and_empty_deletion_do_not_scan_unrelated_following_prose() {
    for (envelope, key) in [
        ("---\nname: example\n\n---", Key::Enter),
        ("---\n---", Key::Enter),
        ("---\n---", Key::Backspace),
        ("```rust\n```", Key::Backspace),
    ] {
        let mut measured = None;
        for paragraphs in [100, 1000] {
            let unrelated = "Unrelated *styled* prose.\n\n".repeat(paragraphs);
            let source = format!("{envelope}\n\nGuard.\n\n{unrelated}");
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let at = document.projection().blocks()[0].range.end;
            let mut commands = CommandInterpreter::new();
            place_insert_boundary(&mut commands, &mut document, at);
            let (result, work) = measure_document_work(|| commands.handle(&mut document, InputEvent::Key(key)));
            result.unwrap_or_else(|error| panic!("{envelope:?}, {key:?}, paragraphs={paragraphs}: {error:?}"));
            assert_eq!(work.full_projection_candidates, 0, "{envelope:?}, {key:?}: {work:?}");
            assert_eq!(work.source_full_materializations, 0, "{envelope:?}, {key:?}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{envelope:?}, {key:?}: {work:?}");
            assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{envelope:?}, {key:?}: {work:?}");
            assert!(work.source_decoded_bytes < 8192, "{envelope:?}, {key:?}: {work:?}");
            assert!(work.projected_formatted_bytes < 4096, "{envelope:?}, {key:?}: {work:?}");
            let current = (work.source_decoded_bytes, work.projected_formatted_bytes, work.model_requests);
            if let Some(previous) = measured { assert_eq!(previous, current, "{envelope:?}, {key:?}: {work:?}"); }
            measured = Some(current);
            let saved = document.source_bytes();
            assert!(saved.ends_with(unrelated.as_bytes()));
            assert_reopens_as_displayed(&document, &format!("{envelope:?}, {key:?}"));
            commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}
