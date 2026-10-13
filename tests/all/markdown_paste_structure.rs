//! Markdown-looking text typed or pasted into formatted view keeps the
//! structure a reader expects: a pasted list stays one flat list, a repeated
//! quote prefix stays in its quote, and literal pastes keep every line literal.
use crate::support::{assert_exact_undo_redo, assert_reopens_as_displayed, shape, Editor};
use viem_core::command::Key;
use viem_core::document::{BlockKind, Format};

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
        // Continuation numbering wins over a repeated typed ordinal.
        ("1. one\n1. two-part\n1. three", true, "1. one\n2. two-part\n3. three"),
    ] {
        for pasted in [true, false] {
            let mut editor = Editor::open("", Format::Markdown);
            editor.type_keys("i");
            if pasted { editor.paste_plain(input) } else { editor.type_keys(input) }
            let context = format!("{input:?} pasted={pasted}");
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
    for pasted in [true, false] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.type_keys("i");
        if pasted { editor.paste_plain("> a\n> b") } else { editor.type_keys("> a\n> b") }
        assert_eq!(editor.source(), "> a\n>\n> b", "pasted={pasted}");
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
        ("- a\n- b", "\\- a\n\n\\- b"),
        ("# x\n> y", "\\# x\n\n\\> y"),
    ] {
        let mut editor = Editor::open("", Format::Markdown);
        editor.set_autoformat(false);
        editor.type_keys("i");
        editor.paste_plain(input);
        assert_eq!(editor.source(), expected, "{input:?}");
        assert_eq!(shape(editor.document()).0, input, "{input:?}");
        assert_reopens_as_displayed(editor.document(), input);
        assert_exact_undo_redo(&mut editor, "", expected, input);
    }
}

#[test]
fn a_nested_item_sharing_its_empty_parents_line_can_be_lifted() {
    // Imported or older files can nest an item on the same line as an empty
    // parent's marker. Backspace and Shift-Tab lift it beside its sibling.
    for (source, expected) in [("- a\n- - b", "- a\n- b"), ("1. a\n2. 1. b", "1. a\n2. b")] {
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
