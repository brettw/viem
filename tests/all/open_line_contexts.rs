//! `o` and `O` open a line or paragraph in the same owner that Enter at the
//! corresponding line edge would, in every Preview context and in Text and
//! Code. Open-line commands never fail: code (fenced, indented, quoted, in a
//! list item and front matter) gains a literal line inside its fence, table
//! cells gain a row inside the cell, and prose opens a paragraph.

use crate::support::{assert_regional, shape, Editor};
use viem_core::command::{InputEvent, Key, LineMode, Mode};
use viem_core::document::{Document, Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn open(source: &str, format: Format, file_format: FileFormat) -> Editor {
    let document = Document::from_bytes_with_file_format(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        format,
        file_format,
    )
    .expect("fixture opens");
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 800., 2000.);
    Editor { core, view }
}

/// The formatted offset of `needle`. `@Style` names the start of the first
/// block with that paragraph style, for owners without searchable text such
/// as empty code and images; an empty needle is the document start.
fn locate(document: &Document, needle: &str) -> usize {
    if let Some(style) = needle.strip_prefix('@') {
        return document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.style.0 == style)
            .unwrap_or_else(|| panic!("{style} block"))
            .range
            .start;
    }
    document.text().find(needle).unwrap_or_else(|| panic!("{needle:?} in text"))
}

fn place(editor: &mut Editor, needle: &str) {
    let at = locate(editor.document(), needle);
    editor.place(at);
}

/// Saved bytes reopen, in the same `fileformat`, as displayed.
fn assert_reopens(editor: &Editor, context: &str) {
    let document = editor.document();
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        document.format(),
        document.file_format(),
    )
    .expect("saved bytes reopen");
    assert_eq!(shape(document), shape(&fresh), "{context}: saved {:?}", editor.source());
}

fn press(editor: &mut Editor, keys: &[Key], context: &str) {
    for key in keys {
        editor
            .core
            .handle(editor.view, CoreEvent::Input(InputEvent::Key(*key)))
            .unwrap_or_else(|error| panic!("{context}: {key:?}: {error:?}"));
    }
}

fn mode(editor: &Editor) -> Mode {
    editor.core.command_state(editor.view).unwrap().mode()
}

/// Run `keys` from `needle`, then require an Insert caret on the opened row,
/// typed `X`, saved bytes that reopen as displayed and exact undo/redo.
fn edit(source: &str, format: Format, file_format: FileFormat, needle: &str, keys: &[Key]) -> Editor {
    let context = format!("{format:?} {file_format:?} {source:?} at {needle:?} {keys:?}");
    let mut editor = open(source, format, file_format);
    place(&mut editor, needle);
    press(&mut editor, keys, &context);
    assert_eq!(mode(&editor), Mode::Insert, "{context}");
    press(&mut editor, &[Key::Char('X'), Key::Escape], &context);
    assert_reopens(&editor, &context);
    let after = editor.source();
    editor.undo();
    assert_eq!(editor.source(), source, "{context}: undo");
    editor.redo();
    assert_eq!(editor.source(), after, "{context}: redo");
    editor
}

const O: [Key; 1] = [Key::Char('o')];
const ABOVE: [Key; 1] = [Key::Char('O')];
const ENTER_AT_END: [Key; 2] = [Key::Char('A'), Key::Enter];

fn style_at_x(editor: &Editor) -> String {
    let at = editor.document().text().find('X').expect("typed X");
    editor
        .document()
        .projection()
        .blocks()
        .iter()
        .find(|block| block.range.start <= at && at < block.range.end)
        .map(|block| block.style.0.clone())
        .expect("X has an owner")
}

#[test]
fn code_open_line_adds_a_literal_line_inside_the_code_like_enter() {
    let fence = "Para\n\n```rust\none\ntwo\nthree\n```\n\nAfter\n";
    let indented = "Para\n\n    one\n    two\n    three\n\nAfter\n";
    let front = "---\ntitle: x\nb: y\n---\n\nBody\n";
    // (source, needle, after `oX`, after `OX`)
    let cases = [
        (fence, "one", "Para\n\n```rust\none\nX\ntwo\nthree\n```\n\nAfter\n", "Para\n\n```rust\nX\none\ntwo\nthree\n```\n\nAfter\n"),
        (fence, "two", "Para\n\n```rust\none\ntwo\nX\nthree\n```\n\nAfter\n", "Para\n\n```rust\none\nX\ntwo\nthree\n```\n\nAfter\n"),
        (fence, "three", "Para\n\n```rust\none\ntwo\nthree\nX\n```\n\nAfter\n", "Para\n\n```rust\none\ntwo\nX\nthree\n```\n\nAfter\n"),
        ("```\ncode\n```\n", "code", "```\ncode\nX\n```\n", "```\nX\ncode\n```\n"),
        ("Text\n\n```\ncode\n```", "code", "Text\n\n```\ncode\nX\n```", "Text\n\n```\nX\ncode\n```"),
        ("~~~\none\n~~~\n", "one", "~~~\none\nX\n~~~\n", "~~~\nX\none\n~~~\n"),
        ("```\ncode", "code", "```\ncode\nX", "```\nX\ncode"),
        // An empty block has one empty code line; opening adds a second.
        ("```\n```\n", "@Code Block", "```\n\nX\n```\n", "```\nX\n\n```\n"),
        ("```\n\n```\n", "@Code Block", "```\n\nX\n```\n", "```\nX\n\n```\n"),
        ("Para\n\n```\n```\n", "@Code Block", "Para\n\n```\n\nX\n```\n", "Para\n\n```\nX\n\n```\n"),
        ("> ```\n> code\n> more\n> ```\n", "code", "> ```\n> code\n> X\n> more\n> ```\n", "> ```\n> X\n> code\n> more\n> ```\n"),
        ("- item\n\n  ```\n  code\n  ```\n", "code", "- item\n\n  ```\n  code\n  X\n  ```\n", "- item\n\n  ```\n  X\n  code\n  ```\n"),
        (indented, "two", "Para\n\n    one\n    two\n    X\n    three\n\nAfter\n", "Para\n\n    one\n    X\n    two\n    three\n\nAfter\n"),
        // An indented block cannot begin or end with a blank line, so an
        // opened line at either edge fences it, exactly as Enter there does.
        (indented, "one", "Para\n\n    one\n    X\n    two\n    three\n\nAfter\n", "Para\n\n```\nX\none\ntwo\nthree\n```\n\nAfter\n"),
        (indented, "three", "Para\n\n```\none\ntwo\nthree\nX\n```\n\nAfter\n", "Para\n\n    one\n    two\n    X\n    three\n\nAfter\n"),
        (front, "title", "---\ntitle: x\nX\nb: y\n---\n\nBody\n", "---\nX\ntitle: x\nb: y\n---\n\nBody\n"),
        (front, "b:", "---\ntitle: x\nb: y\nX\n---\n\nBody\n", "---\ntitle: x\nX\nb: y\n---\n\nBody\n"),
    ];
    for (source, needle, below, above) in cases {
        let opened = edit(source, Format::Markdown, FileFormat::Unix, needle, &O);
        assert_eq!(opened.source(), below, "o in {source:?} at {needle:?}");
        assert_eq!(style_at_x(&opened), "Code Block", "o in {source:?} at {needle:?}");
        let entered = edit(source, Format::Markdown, FileFormat::Unix, needle, &ENTER_AT_END);
        assert_eq!(entered.source(), below, "A<CR> in {source:?} at {needle:?}");
        let opened = edit(source, Format::Markdown, FileFormat::Unix, needle, &ABOVE);
        assert_eq!(opened.source(), above, "O in {source:?} at {needle:?}");
        assert_eq!(style_at_x(&opened), "Code Block", "O in {source:?} at {needle:?}");
    }
}

#[test]
fn open_line_matches_enter_in_every_context() {
    let markdown = [
        ("One\n\nTwo\n", "One"),
        ("One\n\nTwo\n", "Two"),
        ("# Head\n\nTwo\n", "Head"),
        ("Intro\n\n## Head\n", "Head"),
        ("> quote\n> more\n\nAfter\n", "quote"),
        ("> > deep\n\nAfter\n", "deep"),
        ("> quote", "quote"),
        ("- a\n  - b\n- c\n", "b"),
        ("- a\n  - b\n- c\n", "c"),
        ("- a\n", "a"),
        ("1. a\n2. b\n", "a"),
        ("1. a\n2. b\n", "b"),
        ("- [ ] task\n- [x] done\n", "task"),
        ("- [ ] task\n- [x] done\n", "done"),
        ("| A | B |\n| - | - |\n| x | y |\n", "A"),
        ("| A | B |\n| - | - |\n| x | y |\n", "y"),
        ("<div>\nhello\n</div>\n\nAfter\n", "hello"),
        ("See [x].\n\n[x]: https://e.com\n\nAfter\n", "[x]:"),
        ("![alt](img.png)\n\nText\n", "@Image"),
        ("Text\n\n![alt](img.png)", "@Image"),
        ("", ""),
        ("---\ntitle: x\n---\n\nBody\n", "Body"),
        ("```\ncode\n```\n\nAfter\n", "After"),
    ];
    for (source, needle) in markdown {
        let context = format!("{source:?} at {needle:?}");
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let original = document.text().to_owned();
        let at = locate(&document, needle);
        let line_start = original[..at].rfind('\n').map_or(0, |at| at + 1);
        let line_end = original[at..].find('\n').map_or(original.len(), |end| at + end);

        let opened = edit(source, Format::Markdown, FileFormat::Unix, needle, &O);
        let entered = edit(source, Format::Markdown, FileFormat::Unix, needle, &ENTER_AT_END);
        assert_eq!(opened.source(), entered.source(), "o versus A<CR>: {context}");
        let mut expected = original.clone();
        expected.insert_str(line_end, "\nX");
        assert_eq!(opened.document().text(), expected, "o: {context}");

        let opened = edit(source, Format::Markdown, FileFormat::Unix, needle, &ABOVE);
        let mut expected = original.clone();
        expected.insert_str(line_start, "X\n");
        assert_eq!(opened.document().text(), expected, "O: {context}");
    }
    // A final delimiter ends the last literal line, so opening below an
    // unterminated last line (or above an empty one) adds the delimiter that
    // keeps the original line, as Enter there does.
    for (source, needle, below, above) in [
        ("a\nb\n", "a", "a\nX\nb\n", "X\na\nb\n"),
        ("a\nb\n", "b", "a\nb\nX\n", "a\nX\nb\n"),
        ("a\nb", "a", "a\nX\nb", "X\na\nb"),
        ("a\nb", "b", "a\nb\nX\n", "a\nX\nb"),
        ("", "", "\nX\n", "X\n\n"),
        ("\n", "", "\nX\n", "X\n\n"),
    ] {
        for format in [Format::PlainText, Format::Code] {
            let context = format!("{format:?} {source:?} at {needle:?}");
            let opened = edit(source, format, FileFormat::Unix, needle, &O);
            assert_eq!(opened.source(), below, "o: {context}");
            let entered = edit(source, format, FileFormat::Unix, needle, &ENTER_AT_END);
            assert_eq!(entered.source(), below, "A<CR>: {context}");
            let opened = edit(source, format, FileFormat::Unix, needle, &ABOVE);
            assert_eq!(opened.source(), above, "O: {context}");
        }
    }
}

#[test]
fn counted_open_line_in_code_repeats_lines_dot_and_undo_units() {
    let source = "Para\n\n```\none\ntwo\n```\n";
    for (needle, keys, once, twice) in [
        ("one", "3oX", "Para\n\n```\none\nX\nX\nX\ntwo\n```\n", "Para\n\n```\none\nX\nX\nX\nX\nX\nX\ntwo\n```\n"),
        ("two", "2OY", "Para\n\n```\none\nY\nY\ntwo\n```\n", "Para\n\n```\none\nY\nY\nY\nY\ntwo\n```\n"),
    ] {
        let context = format!("{keys} at {needle}");
        let mut editor = open(source, Format::Markdown, FileFormat::Unix);
        editor.place_at(needle);
        let keys = keys.chars().map(Key::Char).chain([Key::Escape]).collect::<Vec<_>>();
        press(&mut editor, &keys, &context);
        assert_eq!(editor.source(), once, "{context}");
        press(&mut editor, &[Key::Char('.')], &context);
        assert_eq!(editor.source(), twice, "{context}: dot");
        assert_reopens(&editor, &context);
        press(&mut editor, &[Key::Char('u')], &context);
        assert_eq!(editor.source(), once, "{context}: undo dot");
        press(&mut editor, &[Key::Char('u')], &context);
        assert_eq!(editor.source(), source, "{context}: undo count");
        press(&mut editor, &[Key::Ctrl('r')], &context);
        assert_eq!(editor.source(), once, "{context}: redo");
    }
}

#[test]
fn open_line_writes_the_current_fileformat() {
    for (file_format, ending) in [(FileFormat::Unix, "\n"), (FileFormat::Dos, "\r\n"), (FileFormat::Mac, "\r")] {
        let spelled = |text: &str| text.replace('\n', ending);
        for (source, needle, keys, expected) in [
            ("```\none\ntwo\n```\n", "one", O, "```\none\nX\ntwo\n```\n"),
            ("```\none\ntwo\n```\n", "two", ABOVE, "```\none\nX\ntwo\n```\n"),
            ("---\na: b\n---\n\nBody\n", "a:", O, "---\na: b\nX\n---\n\nBody\n"),
            ("Para\n\nNext\n", "Para", O, "Para\n\nX\n\nNext\n"),
            ("Para\n\nNext\n", "Next", ABOVE, "Para\n\nX\n\nNext\n"),
        ] {
            let opened = edit(&spelled(source), Format::Markdown, file_format, needle, &keys);
            assert_eq!(opened.source(), spelled(expected), "{file_format:?} {keys:?} {source:?}");
        }
        for format in [Format::PlainText, Format::Code] {
            let opened = edit(&spelled("a\nb\n"), format, file_format, "a", &O);
            assert_eq!(opened.source(), spelled("a\nX\nb\n"), "{format:?} {file_format:?}");
            let opened = edit(&spelled("a\nb\n"), format, file_format, "b", &ABOVE);
            assert_eq!(opened.source(), spelled("a\nX\nb\n"), "{format:?} {file_format:?}");
        }
    }
}

#[test]
fn physical_line_open_in_preview_code_stays_in_the_fence() {
    let source = "Para\n\n```\none\ntwo\n```\n";
    for (needle, key, expected) in [
        ("one", 'o', "Para\n\n```\none\nX\ntwo\n```\n"),
        ("two", 'O', "Para\n\n```\none\nX\ntwo\n```\n"),
        ("two", 'o', "Para\n\n```\none\ntwo\nX\n```\n"),
    ] {
        let mut editor = open(source, Format::Markdown, FileFormat::Unix);
        editor.core.handle(editor.view, CoreEvent::SetLineMode(LineMode::PhysicalSource)).unwrap();
        editor.place_at(needle);
        press(&mut editor, &[Key::Char(key), Key::Char('X'), Key::Escape], needle);
        assert_eq!(editor.source(), expected, "{key} at {needle}");
        assert_eq!(style_at_x(&editor), "Code Block", "{key} at {needle}");
        editor.undo();
        assert_eq!(editor.source(), source, "{key} at {needle}");
    }
}

#[test]
fn open_line_in_code_stays_regional_in_a_large_document() {
    let filler = "Paragraph text.\n\n".repeat(2_000);
    let source = format!("{filler}```\none\ntwo\n```\n\n{filler}");
    for key in ['o', 'O'] {
        let mut editor = open(&source, Format::Markdown, FileFormat::Unix);
        editor.place_at("one");
        assert_regional(&format!("{key} in code"), || press(&mut editor, &[Key::Char(key)], "open"));
        assert_regional(&format!("{key} typing"), || press(&mut editor, &[Key::Char('X'), Key::Escape], "type"));
        let expected = if key == 'o' { "```\none\nX\ntwo\n```" } else { "```\nX\none\ntwo\n```" };
        assert_eq!(editor.source(), format!("{filler}{expected}\n\n{filler}"), "{key}");
    }
}

fn loose(editor: &Editor) -> Vec<bool> {
    editor.document().projection().blocks().iter().map(|block| block.list_loose).collect()
}

#[test]
fn opening_a_first_nested_item_removes_its_generated_blank_line_once_it_has_text() {
    // (source, keys, expected source); every list stays tight once typed.
    for (source, keys, expected) in [
        ("- a\n  - b\n- c\n", "OX", "- a\n  - X\n  - b\n- c\n"),
        ("- a\n  - b\n- c\n", "2OX", "- a\n  - X\n  - X\n  - b\n- c\n"),
        ("1. a\n   1. b\n", "OX", "1. a\n   1. X\n   2. b\n"),
        ("> - a\n>   - b\n", "OX", "> - a\n>   - X\n>   - b\n"),
        ("- a\r\n  - b\r\n- c\r\n", "OX", "- a\r\n  - X\r\n  - b\r\n- c\r\n"),
    ] {
        let context = format!("{source:?} {keys}");
        let mut editor = Editor::open(source, Format::Markdown);
        editor.place_at("b");
        let keys = keys.chars().map(Key::Char).chain([Key::Escape]).collect::<Vec<_>>();
        press(&mut editor, &keys, &context);
        assert_eq!(editor.source(), expected, "{context}");
        assert!(loose(&editor).iter().all(|loose| !loose), "{context}");
        assert_reopens(&editor, &context);
        assert_exact_undo(&mut editor, source, expected, &context);
    }
}

fn assert_exact_undo(editor: &mut Editor, before: &str, after: &str, context: &str) {
    editor.undo();
    assert_eq!(editor.source(), before, "{context}: one undo unit");
    editor.redo();
    assert_eq!(editor.source(), after, "{context}: redo");
}

#[test]
fn a_nested_item_separator_stays_while_it_is_needed_or_authored() {
    for (source, keys, expected) in [
        // An empty item needs the blank line to interrupt its parent.
        ("- a\n  - b\n- c\n", vec![Key::Char('O')], "- a\n\n  - \n  - b\n- c\n"),
        ("- a\n  - b\n- c\n", vec![Key::Char('O'), Key::Char('X'), Key::Backspace], "- a\n\n  - \n  - b\n- c\n"),
        // An authored blank line is the author's loose list.
        ("- a\n\n  - b\n- c\n", vec![Key::Char('O'), Key::Char('X')], "- a\n\n  - X\n  - b\n- c\n"),
    ] {
        let context = format!("{source:?} {keys:?}");
        let mut editor = Editor::open(source, Format::Markdown);
        editor.place_at("b");
        press(&mut editor, &keys, &context);
        press(&mut editor, &[Key::Escape], &context);
        assert_eq!(editor.source(), expected, "{context}");
        assert_reopens(&editor, &context);
        assert_exact_undo(&mut editor, source, expected, &context);
    }
}

#[test]
fn dot_repeats_a_first_nested_item_open_with_its_cleanup() {
    let source = "- a\n  - b\n- c\n";
    let mut editor = Editor::open(source, Format::Markdown);
    editor.place_at("b");
    press(&mut editor, &[Key::Char('O'), Key::Char('X'), Key::Escape], "first");
    let once = "- a\n  - X\n  - b\n- c\n";
    assert_eq!(editor.source(), once);
    press(&mut editor, &[Key::Char('.')], "dot");
    assert_eq!(editor.source(), "- a\n  - X\n  - X\n  - b\n- c\n");
    assert!(loose(&editor).iter().all(|loose| !loose));
    press(&mut editor, &[Key::Char('u')], "undo dot");
    assert_eq!(editor.source(), once);
    press(&mut editor, &[Key::Char('u')], "undo open");
    assert_eq!(editor.source(), source);
}

#[test]
fn opening_at_the_end_of_a_document_stays_in_its_owner_and_keeps_the_terminator() {
    for (source, needle, keys, expected) in [
        ("> quote\n", "quote", O.to_vec(), "> quote\n>\n> X\n"),
        ("> quote\n", "quote", ENTER_AT_END.to_vec(), "> quote\n>\n> X\n"),
        ("> quote", "quote", O.to_vec(), "> quote\n>\n> X"),
        ("> > deep\n", "deep", O.to_vec(), "> > deep\n> >\n> > X\n"),
        ("One\n\nTwo\n", "Two", O.to_vec(), "One\n\nTwo\n\nX\n"),
        ("One\n\nTwo\n", "Two", ENTER_AT_END.to_vec(), "One\n\nTwo\n\nX\n"),
        ("One\n\nTwo", "Two", O.to_vec(), "One\n\nTwo\n\nX"),
        ("# H\n", "H", O.to_vec(), "# H\n\nX\n"),
        ("One\r\n\r\nTwo\r\n", "Two", O.to_vec(), "One\r\n\r\nTwo\r\n\r\nX\r\n"),
    ] {
        let opened = edit(source, Format::Markdown, if source.contains('\r') { FileFormat::Dos } else { FileFormat::Unix }, needle, &keys);
        assert_eq!(opened.source(), expected, "{source:?} {keys:?}");
        let at = opened.document().text().find('X').unwrap();
        let quote = opened.document().projection().blocks().iter()
            .find(|block| block.range.start <= at && at < block.range.end).unwrap().quote_depth;
        assert_eq!(quote, source.split(' ').take_while(|part| *part == ">").count(), "{source:?} {keys:?}");
    }
}

#[test]
fn a_final_blank_quote_row_projects_inside_the_quote() {
    for (source, quote_depths) in [
        ("> quote\n>\n> \n", vec![1, 1]),
        ("> quote\n>\n> ", vec![1, 1]),
        ("> quote\n>\n>\n", vec![1, 1]),
        ("> quote\n>\n> \n>\n", vec![1, 1, 1]),
        ("> > deep\n> >\n> > \n", vec![2, 2]),
        ("Para\n\n\n", vec![0, 0]),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let depths = document.projection().blocks().iter().map(|block| block.quote_depth).collect::<Vec<_>>();
        assert_eq!(depths, quote_depths, "{source:?}");
    }
}

#[test]
fn enter_at_a_first_nested_item_start_retires_its_blank_line_when_the_new_item_gets_text() {
    let source = "- a\n  - b\n- c\n";
    for (keys, expected) in [
        (vec![Key::Char('I'), Key::Enter, Key::Up, Key::Char('X')], "- a\n  - X\n  - b\n- c\n"),
        (vec![Key::Char('I'), Key::Enter, Key::Char('Y'), Key::Escape, Key::Char('k'), Key::Char('i'), Key::Char('X')], "- a\n  - X\n  - Yb\n- c\n"),
    ] {
        let context = format!("{keys:?}");
        let mut editor = Editor::open(source, Format::Markdown);
        editor.place_at("b");
        press(&mut editor, &keys, &context);
        press(&mut editor, &[Key::Escape], &context);
        assert_eq!(editor.source(), expected, "{context}");
        assert!(loose(&editor).iter().all(|loose| !loose), "{context}");
        assert_reopens(&editor, &context);
        // The typing in the new item is its own undo unit; undoing it restores
        // the separator the empty item needs.
        editor.undo();
        assert!(editor.source().starts_with("- a\n\n  - \n"), "{context}: {:?}", editor.source());
    }
}

#[test]
fn an_ime_commit_into_a_new_first_nested_item_retires_its_blank_line_in_the_commit() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    let mut editor = Editor::open("- a\n  - b\n- c\n", Format::Markdown);
    editor.place_at("b");
    press(&mut editor, &[Key::Char('O')], "O");
    let at = editor.core.command_state(editor.view).unwrap().cursor();
    let target = CompositionTarget::at_offsets(editor.core.document(), at..at).unwrap();
    for event in [
        CompositionEvent::Begin(target),
        CompositionEvent::Update(CompositionUpdate::new("猫", 3..3)),
    ] {
        editor.core.handle(editor.view, CoreEvent::Composition(event)).unwrap();
    }
    assert_eq!(editor.source(), "- a\n\n  - \n  - b\n- c\n", "composition changes no source");
    editor.core.handle(editor.view, CoreEvent::Composition(CompositionEvent::Commit)).unwrap();
    assert_eq!(editor.source(), "- a\n  - 猫\n  - b\n- c\n");
    assert_reopens(&editor, "IME");
    press(&mut editor, &[Key::Escape], "Escape");
    editor.undo();
    assert_eq!(editor.source(), "- a\n\n  - \n  - b\n- c\n", "the commit is its own undo unit");
}
