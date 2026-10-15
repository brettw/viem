//! Markdown-looking text typed or pasted into Markdown Preview keeps the
//! structure a reader expects. Paste interprets external text as Markdown:
//! complete pasted blocks keep their own top-level structure, a partial
//! paragraph joins the destination, and code keeps literal text. Paste and
//! Match Style keeps every line literal.
use crate::support::{assert_exact_undo_redo, assert_reopens_as_displayed, assert_regional, shape, Editor};
use viem_core::command::{Key, Mode};
use viem_core::document::{BlockKind, Format, SemanticInlineStyle, StyleApplication};

fn list_levels(editor: &Editor) -> Vec<(String, u8, bool)> {
    let document = editor.document();
    document
        .projection()
        .blocks()
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::ListItem { level, ordered, .. } => {
                Some((document.text()[block.range.clone()].to_owned(), level, ordered))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn pasted_and_typed_lists_stay_flat_with_unescaped_words() {
    for (input, ordered, expected) in [
        ("- one\n- two-part\n- three", false, "- one\n- two-part\n- three"),
        ("* one\n* two-part\n* three", false, "* one\n* two-part\n* three"),
        ("1. one\n2. two-part\n3. three", true, "1. one\n2. two-part\n3. three"),
        // Continuation numbering wins over a repeated typed ordinal; pasted
        // Markdown keeps its authored (equivalent) ordinals.
        ("1. one\n1. two-part\n1. three", true, "1. one\n2. two-part\n3. three"),
    ] {
        for pasted in [true, false] {
            let mut editor = Editor::open("", Format::Markdown);
            editor.type_keys("i");
            if pasted { editor.paste_plain(input) } else { editor.type_keys(input) }
            let context = format!("{input:?} pasted={pasted}");
            let expected = if pasted { input } else { expected };
            assert_eq!(editor.source(), expected, "{context}");
            assert_eq!(
                list_levels(&editor),
                vec![("one".into(), 0, ordered), ("two-part".into(), 0, ordered), ("three".into(), 0, ordered)],
                "{context}"
            );
            assert_reopens_as_displayed(editor.document(), &context);
            if pasted {
                assert_exact_undo_redo(&mut editor, "", expected, &context);
            }
        }
    }
}

#[test]
fn a_pasted_list_supports_backspace_and_indentation_at_its_second_item() {
    let pasted = "- Give outcomes, including path-specific variants.\n- Make a regional-proof object.\n- Make materialization explicit.";
    for (key, expected) in [
        (Key::Backspace, "- Give outcomes, including path-specific variants.\n\nMake a regional-proof object.\n\n- Make materialization explicit."),
        (Key::Tab, "- Give outcomes, including path-specific variants.\n  - Make a regional-proof object.\n- Make materialization explicit."),
    ] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.type_keys("i");
        editor.paste_plain(pasted);
        editor.place_at("Make a");
        editor.key(key);
        assert_eq!(editor.source(), expected, "{key:?}");
        assert_reopens_as_displayed(editor.document(), &format!("{key:?}"));
    }
}

#[test]
fn a_repeated_quote_prefix_continues_the_quote() {
    // Typing Enter makes paragraphs; pasted Markdown keeps its one quoted
    // paragraph with a soft line ending.
    for (pasted, expected) in [(true, "> a\n> b"), (false, "> a\n>\n> b")] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.type_keys("i");
        if pasted { editor.paste_plain("> a\n> b") } else { editor.type_keys("> a\n> b") }
        assert_eq!(editor.source(), expected, "pasted={pasted}");
        assert!(editor.document().projection().blocks().iter().all(|block| block.quote_depth == 1), "pasted={pasted}");
        assert_reopens_as_displayed(editor.document(), &format!("pasted={pasted}"));
    }
}

#[test]
fn a_marker_typed_before_item_text_still_nests() {
    // Only a marker that is an empty first paragraph's whole body continues
    // its item; before existing text it keeps CommonMark's nesting.
    let mut editor = Editor::open("- a\n- b", Format::Markdown);
    editor.place_at("b");
    editor.type_keys("i- ");
    assert_eq!(editor.source(), "- a\n- - b");
    assert_reopens_as_displayed(editor.document(), "marker before body text");
}

#[test]
fn block_punctuation_after_line_content_is_not_escaped() {
    for (typed, expected) in [
        ("path-specific", "path-specific"),
        ("a+b and c#d", "a+b and c#d"),
        ("- item-with-dash", "- item-with-dash"),
        // At a line start the same characters still need protection until
        // their construct is complete.
        ("-", "\\-"),
        ("#", "\\#"),
    ] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.type_keys("i");
        editor.type_keys(typed);
        assert_eq!(editor.source(), expected, "{typed:?}");
        assert_reopens_as_displayed(editor.document(), typed);
    }
    // A heading's closing sequence is syntax: a typed trailing `#` stays text.
    let mut editor = Editor::open("# Title", Format::Markdown);
    editor.place(editor.document().text().len());
    editor.type_keys("a #");
    assert_eq!(editor.document().text(), "Title #");
    assert_reopens_as_displayed(editor.document(), "heading closing sequence");
}

#[test]
fn literal_multiline_paste_protects_every_line_start() {
    for (input, expected) in [
        ("a\n- b", "a\n\n\\- b"),
        ("a\n2. b", "a\n\n2\\. b"),
        ("a\n    code", "a\n\n&#32;   code"),
        ("a\n  - b", "a\n\n&#32; - b"),
        ("- a\n- b", "\\- a\n\n\\- b"),
        ("# x\n> y", "\\# x\n\n\\> y"),
    ] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.type_keys("i");
        editor.paste_matching_style(input);
        assert_eq!(editor.source(), expected, "{input:?}");
        assert_eq!(shape(editor.document()).0, input, "{input:?}");
        assert_reopens_as_displayed(editor.document(), input);
        assert_exact_undo_redo(&mut editor, "", expected, input);
    }
}

#[test]
fn a_nested_item_sharing_its_empty_parents_line_can_be_lifted() {
    // Imported or older files can nest an item on the same line as an empty
    // parent's marker. Backspace and Shift-Tab lift it beside its sibling, or
    // to the parent's place when no item precedes it.
    for (source, expected) in [
        ("- a\n- - b", "- a\n- b"), ("1. a\n2. 1. b", "1. a\n2. b"),
        ("- - b", "- b"), ("1. 1. b", "1. b"), ("> - - b", "> - b"), ("- - b\n- c", "- b\n- c"),
    ] {
        for key in [Key::Backspace, Key::BackTab] {
            let mut editor = Editor::open(source, Format::Markdown);
            editor.type_keys("i");
            editor.place_at("b");
            editor.key(key);
            assert_eq!(editor.source(), expected, "{source:?} {key:?}");
            assert!(list_levels(&editor).iter().all(|(_, level, _)| *level == 0), "{source:?} {key:?}");
            assert_reopens_as_displayed(editor.document(), &format!("{source:?} {key:?}"));
            assert_exact_undo_redo(&mut editor, source, expected, &format!("{source:?} {key:?}"));
        }
    }
}

const REPORT: &str = "## Work\n\n- **Tests:** pass.\n- **Memory:** two limits fail.\n  - nested detail\n\nA closing *paragraph*.\n\n1. first\n2. second\n";

/// Every heading is top level, and the pasted lists remain their own lists.
fn assert_report_structure(editor: &Editor, context: &str) {
    let document = editor.document();
    let text = document.text();
    let blocks = document.projection().blocks();
    let block = |needle: &str| blocks.iter()
        .find(|block| text[block.range.clone()] == *needle)
        .unwrap_or_else(|| panic!("{context}: block {needle:?} in {:?}", shape(document)));
    let heading = block("Work");
    assert_eq!(heading.kind, BlockKind::Heading(2), "{context}");
    assert!(heading.quote_depth == 0 && heading.containers.is_empty(), "{context}: {:?}", shape(document));
    let closing = block("A closing paragraph.");
    assert!(matches!(closing.kind, BlockKind::Paragraph) && closing.quote_depth == 0 && closing.containers.is_empty(),
        "{context}: {:?}", shape(document));
    for (item, level, ordered) in [("Tests: pass.", 0, false), ("nested detail", 1, false), ("first", 0, true), ("second", 0, true)] {
        assert!(matches!(block(item).kind, BlockKind::ListItem { level: actual, ordered: actual_ordered, .. }
            if actual == level && actual_ordered == ordered), "{context}: {item:?} {:?}", shape(document));
    }
}

#[test]
fn pasted_markdown_blocks_keep_their_own_top_level_structure() {
    // (destination, caret needle and offset, expected source)
    for (source, needle, offset, expected) in [
        ("", "", 0, REPORT.trim_end().to_owned()),
        ("Intro here.", "here", 0, format!("Intro \n\n{}\n\nhere.", REPORT.trim_end())),
        ("- one\n- ", "", 4, format!("- one\n\n{}", REPORT.trim_end())),
        ("- one\n- two", "two", 0, format!("- one\n\n{}\n\n- two", REPORT.trim_end())),
        ("- one\n- two", "two", 3, format!("- one\n- two\n\n{}", REPORT.trim_end())),
        ("> quoted here", "here", 0, format!("> quoted \n\n{}\n\n>\n> here", REPORT.trim_end())),
        // Same-kind lists join, as CommonMark requires; the join stays tight.
        ("1. a\n2. b\n3. c", "b", 1, format!("1. a\n2. b\n\n{}\n3. c", REPORT.trim_end())),
        ("- a\n  - b\n    - c\n    - e\n", "c", 1, format!("- a\n  - b\n    - c\n\n{}\n\n- e\n", REPORT.trim_end())),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        editor.type_keys("i");
        let at = editor.document().text().find(needle).unwrap() + offset;
        editor.place(at);
        editor.paste_plain(REPORT);
        let context = format!("{source:?} at {needle:?}+{offset}");
        assert_eq!(editor.source(), expected, "{context}");
        assert_report_structure(&editor, &context);
        assert_eq!(editor.core.command_state(editor.view).unwrap().mode(), Mode::Insert, "{context}");
        assert_reopens_as_displayed(editor.document(), &context);
        assert_exact_undo_redo(&mut editor, source, &expected, &context);
    }
}

#[test]
fn a_partial_pasted_paragraph_joins_the_destination_inline() {
    let mut editor = Editor::open("Hello world", Format::Markdown);
    editor.type_keys("i");
    editor.place_at("world");
    editor.paste_plain("**bold** text ");
    assert_eq!(editor.document().text(), "Hello bold text world");
    assert_eq!(editor.document().projection().blocks().len(), 1);
    assert!(editor.document().projection().style_spans().iter().any(|span| span.range == (6..10)
        && span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)));
    assert_eq!(editor.core.command_state(editor.view).unwrap().cursor(), 16);
    assert_reopens_as_displayed(editor.document(), "inline");
    let after = editor.source();
    assert_exact_undo_redo(&mut editor, "Hello world", &after, "inline");

    // The first and last paragraphs join the text around the caret.
    let mut editor = Editor::open("Hello world", Format::Markdown);
    editor.type_keys("i");
    editor.place_at("world");
    editor.paste_plain("first\n\n## Middle\n\nlast ");
    assert_eq!(editor.source(), "Hello first\n\n## Middle\n\nlast world");
    assert_eq!(editor.document().text(), "Hello first\nMiddle\nlast world");
    assert_reopens_as_displayed(editor.document(), "joined edges");
}

#[test]
fn code_destinations_keep_pasted_markdown_literal() {
    for (source, needle, expected_text) in [
        ("some `code` here", "ode", "some c**b**\n\n## Hode here"),
        ("```\ncode\n```", "ode", "c**b**\n\n## Hode"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        editor.type_keys("i");
        editor.place_at(needle);
        editor.paste_plain("**b**\n\n## H");
        assert_eq!(editor.document().text(), expected_text, "{source:?}");
        assert!(!editor.document().projection().blocks().iter().any(|block| matches!(block.kind, BlockKind::Heading(_))),
            "{source:?}: {:?}", shape(editor.document()));
        assert_reopens_as_displayed(editor.document(), source);
    }
    // A table cell takes inline Markdown but keeps block syntax literal.
    let table = "| a | b |\n| - | - |\n| c | d |";
    let mut editor = Editor::open(table, Format::Markdown);
    editor.type_keys("i");
    editor.place(editor.document().text().find('c').unwrap() + 1);
    editor.paste_plain("**x** y");
    assert_eq!(editor.document().text(), "a\nb\ncx y\nd");
    let mut editor = Editor::open(table, Format::Markdown);
    editor.type_keys("i");
    editor.place(editor.document().text().find('c').unwrap() + 1);
    editor.paste_plain("## H\n\npara");
    assert_eq!(editor.document().text(), "a\nb\nc## H\n\npara\nd");
    assert_reopens_as_displayed(editor.document(), "table cell");
}

#[test]
fn paste_and_match_style_keeps_every_line_literal() {
    let mut editor = Editor::open("", Format::Markdown);
    editor.type_keys("i");
    editor.paste_matching_style(REPORT);
    assert_eq!(editor.document().text(), REPORT);
    assert!(editor.document().projection().blocks().iter()
        .all(|block| matches!(block.kind, BlockKind::Paragraph) && block.containers.is_empty()));
    assert_reopens_as_displayed(editor.document(), "matching style");
    let after = editor.source();
    assert_exact_undo_redo(&mut editor, "", &after, "matching style");

    // Indented continuation lines inside a reopened inline scope keep their
    // leading whitespace and the scope.
    for text in ["l1\n  l2", "l1\n  - l2"] {
        let mut editor = Editor::open("*it*", Format::Markdown);
        editor.place(1);
        editor.type_keys("v");
        editor.paste_matching_style(text);
        assert_eq!(editor.document().text(), format!("i{text}"), "{text:?}");
        assert!(editor.document().projection().style_spans().iter()
            .any(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Emphasis)
                && span.range.end == editor.document().text().len()), "{text:?}");
        assert_reopens_as_displayed(editor.document(), text);
    }
}

#[test]
fn normal_and_selected_markdown_paste_follow_native_paste_placement() {
    // Linewise text goes above the current line and the caret stays on it.
    let mut editor = Editor::open("first\n\nsecond line", Format::Markdown);
    editor.place_at("line");
    editor.paste_plain("## Pasted\n");
    assert_eq!(editor.source(), "first\n\n## Pasted\n\nsecond line");
    let state = editor.core.command_state(editor.view).unwrap();
    assert_eq!((state.mode(), state.cursor()), (Mode::Normal, editor.document().text().find("second").unwrap()));

    // A selection is replaced, ending in Normal with the deletion in `-`.
    let mut editor = Editor::open("keep this and that", Format::Markdown);
    editor.place_at("this");
    editor.type_keys("viw");
    editor.paste_plain("- x\n- y");
    assert_eq!(editor.source(), "keep \n\n- x\n- y\n\n&#32;and that");
    let state = editor.core.command_state(editor.view).unwrap();
    assert_eq!(state.mode(), Mode::Normal);
    assert_eq!(state.register('-').unwrap().text, "this");
    assert_reopens_as_displayed(editor.document(), "selection");
}

#[test]
fn markdown_paste_into_a_large_document_stays_regional() {
    let source = "Unchanged paragraph.\n\n".repeat(5_000) + "target here\n\n" + &"After.\n\n".repeat(5_000);
    let mut editor = Editor::open(&source, Format::Markdown);
    editor.type_keys("i");
    editor.place_at("here");
    assert_regional("block paste", || editor.paste_plain(REPORT));
    assert_report_structure(&editor, "large");
    assert!(editor.source().contains(&format!("target \n\n{}\n\nhere", REPORT.trim_end())));
}

#[test]
fn zz_scratch_one() {
    for (src, op) in [("\t", "type"), ("\t", "x"), ("\t", "a\nb"), ("  ", "a\nb"), ("\t\n\na", "a\nb"), ("a\n\n\t\n\nb", "a\nb")] {
        let mut editor = Editor::open(src, Format::Markdown);
        editor.key(Key::Char('i'));
        let at = if src.starts_with('a') { 2 } else { 0 };
        editor.place(at);
        let r = if op == "type" { editor.core.handle(editor.view, viem_core::CoreEvent::Input(viem_core::command::InputEvent::key('x'))).map(|_| ()) } else {
            let clipboard = viem_core::command::clipboard::ClipboardCommandContext::new().with_read(viem_core::command::clipboard::ClipboardSnapshot::new(
                viem_core::command::clipboard::ClipboardTarget::Clipboard,
                viem_core::command::clipboard::ClipboardGeneration(1),
                viem_core::command::clipboard::ClipboardContent::from_plain_text(op),
            ));
            editor.core.handle(editor.view, viem_core::CoreEvent::InputWithClipboard { input: viem_core::command::InputEvent::Key(Key::PasteClipboardMatchingStyle), clipboard }).map(|_| ())
        };
        eprintln!("RES {src:?} {op:?} -> {r:?} {:?}", editor.source());
    }
}

#[test]
fn zz_scratch_matching_style() {
    for source in ["", "\t", "#", "- a", "a*b*c", "<div>", "`` ` ``", "999999999. big", "<script>alert(1)</script>"] {
        for (key, text) in [("m", "## H\n  - x\n**y**"), ("p", "## H\n\n- a\n- b\n\npara"), ("p", "**md** | x\n\n- y"), ("p", "> q\n\n1. one\n2. two\n"), ("p", "line one\nline two\n")] {
            let len = source.len();
            for at in 0..=len.min(25) {
                let mut editor = Editor::open(source, Format::Markdown);
                let text_len = editor.document().text().len();
                if at > text_len { continue; }
                editor.key(Key::Char('i'));
                editor.place(at);
                let clipboard = viem_core::command::clipboard::ClipboardCommandContext::new().with_read(viem_core::command::clipboard::ClipboardSnapshot::new(
                    viem_core::command::clipboard::ClipboardTarget::Clipboard,
                    viem_core::command::clipboard::ClipboardGeneration(1),
                    viem_core::command::clipboard::ClipboardContent::from_plain_text(text),
                ));
                let k = if key == "m" { Key::PasteClipboardMatchingStyle } else { Key::PasteClipboard };
                let r = editor.core.handle(editor.view, viem_core::CoreEvent::InputWithClipboard { input: viem_core::command::InputEvent::Key(k), clipboard });
                if let Err(e) = r { eprintln!("FAIL {source:?} @{at} {key} {text:?}: {e:?}"); }
            }
        }
    }
}
