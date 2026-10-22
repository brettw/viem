//! Edits at the edges of Markdown blocks keep the block's own presentation
//! and settings: a code block's new final row stays inside its box, joining
//! a passive `<pre lang>` keeps the language, and opening a paragraph beside
//! a reference definition keeps the definition.

use crate::support::{assert_exact_undo_redo, assert_reopens_as_displayed, Editor};
use viem_core::command::Key;
use viem_core::document::{
    BoundaryAffinity, Color, Document, Encoding, Format, StyleApplication, StyleDefinitionFieldEdit,
    StyleNamespace, StyleProperty, StylePropertyValue,
};
use viem_core::layout::DecorationKind;
use viem_core::CoreEvent;

fn set_block_style(editor: &mut Editor, style: &str, property: StyleProperty, value: StylePropertyValue) {
    let document = editor.core.document();
    editor
        .core
        .handle(editor.view, CoreEvent::EditGeneratedStyle {
            document: document.id(),
            revision: document.revision(),
            style_sheet_revision: document.projection().style_sheet().revision,
            namespace: StyleNamespace::Block,
            style: style.into(),
            edit: StyleDefinitionFieldEdit::SetDeclaration { property, value },
        })
        .unwrap();
}

/// The caret's row, its paragraph, and the vertical extent of that
/// paragraph's background on the row.
fn caret_row(editor: &mut Editor) -> (Option<u64>, f32, f32, Option<(f32, f32)>) {
    let at = editor.core.command_state(editor.view).unwrap().cursor();
    let layout = editor.core.layout(editor.view).unwrap().snapshot().unwrap().clone();
    let caret = layout.caret_point(at, BoundaryAffinity::Downstream).unwrap();
    let row = &layout.rows[layout.caret_geometry(caret).unwrap().row_index];
    let fill = row
        .decorations
        .iter()
        .find(|decoration| decoration.kind == DecorationKind::BlockBackground)
        .map(|decoration| (decoration.typographic_bounds.y, decoration.typographic_bounds.y + decoration.typographic_bounds.height));
    (row.paragraph_id, row.y, row.height(), fill)
}

#[test]
fn enter_at_the_end_of_a_code_block_keeps_the_new_row_inside_its_box() {
    let padding = 10.;
    for (source, needle, keys, expected) in [
        ("```rust\ncode\n```\n\nAfter\n", "code", vec![Key::Char('A'), Key::Enter], "```rust\ncode\n\n```\n\nAfter\n"),
        ("```rust\ncode\n```\n\nAfter\n", "code", vec![Key::Char('o')], "```rust\ncode\n\n```\n\nAfter\n"),
        ("> ```\n> code\n> ```\n\nAfter\n", "code", vec![Key::Char('A'), Key::Enter], "> ```\n> code\n> \n> ```\n\nAfter\n"),
        ("```rust\ncode\n```", "code", vec![Key::Char('A'), Key::Enter], "```rust\ncode\n\n```"),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        set_block_style(&mut editor, "Code Block", StyleProperty::BlockBackground,
            StylePropertyValue::Color(Color { red: 1., green: 0., blue: 0., alpha: 1. }));
        set_block_style(&mut editor, "Code Block", StyleProperty::BlockPaddingBottom, StylePropertyValue::Float(padding));
        editor.place_at(needle);
        let (code, ..) = caret_row(&mut editor);
        for key in &keys {
            editor.key(*key);
        }
        assert_eq!(editor.source(), expected, "{source:?} {keys:?}");
        for escape in [false, true] {
            if escape {
                editor.key(Key::Escape);
            }
            let (paragraph, y, height, fill) = caret_row(&mut editor);
            assert_eq!(paragraph, code, "{source:?} {keys:?}: the new row belongs to the code block");
            let (_, bottom) = fill.unwrap_or_else(|| panic!("{source:?} {keys:?}: the new row has the code background"));
            assert!((bottom - (y + height + padding)).abs() < 0.01, "{source:?} {keys:?}: bottom padding below the new row");
        }
        editor.key(Key::Char('a'));
        editor.key(Key::Char('X'));
        editor.key(Key::Escape);
        let at = editor.document().text().find('X').unwrap();
        let block = editor.document().projection().blocks().iter()
            .find(|block| block.range.start <= at && at < block.range.end).unwrap().clone();
        assert_eq!(block.style.0, "Code Block", "{source:?} {keys:?}");
        assert_reopens_as_displayed(editor.document(), source);
    }
}

/// A background job may capture just the empty caret row. Its owner must not
/// depend on a preceding nonempty row happening to be in the same request.
fn assert_isolated_code_caret_row(editor: &Editor, padding: f32, context: &str) {
    use viem_core::layout::{
        compute_layout_job, inspect_layout_provider, prepare_layout_job, HardLineLayoutRegion,
        LayoutCancellationToken, LayoutEngine, LayoutExecutionContext, LayoutJobId,
        LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider, ViewLayout,
    };
    let document = editor.document();
    let at = editor.core.command_state(editor.view).unwrap().cursor();
    let owner = document.projection().blocks().iter()
        .filter(|block| block.range.start <= at && at <= block.range.end)
        .max_by_key(|block| block.range.start).unwrap();
    assert_eq!(owner.style.0, "Code Block", "{context}");
    let line = document.projection().hard_line_at_offset(at).unwrap();
    assert_eq!(document.projection().hard_line_range(line), Some(at..at), "{context}");
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let requirements = inspect_layout_provider(&engine);
    let mut view = ViewLayout::new(800., 2000.);
    let request = prepare_layout_job(document, &mut view, requirements, LayoutJobId(1),
        LayoutJobPriority::ChangedVisibleRows,
        LayoutJobRegion::HardLines(HardLineLayoutRegion::new(line..line + 1).unwrap()),
        LayoutCancellationToken::new()).unwrap();
    let candidate = compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
    let rows = candidate.regional_snapshot().lines()[0].rows();
    assert_eq!(rows.len(), 1, "{context}");
    let row = &rows[0];
    assert_eq!(row.paragraph_id, Some(owner.id), "{context}: isolated empty row keeps its owner");
    let background = row.decorations.iter().find(|decoration|
        decoration.kind == DecorationKind::BlockBackground
            && decoration.paint.foreground == Color { red: 1., green: 0., blue: 0., alpha: 1. })
        .unwrap_or_else(|| panic!("{context}: isolated empty row has its code background"));
    let bottom = background.typographic_bounds.y + background.typographic_bounds.height;
    assert!((bottom - row.y - row.height() - padding).abs() < 0.01,
        "{context}: bottom padding remains below the empty code row: {row:?}");
    let snapshot = editor.core.layout(editor.view).unwrap().snapshot().unwrap();
    let caret = snapshot.caret_point(at, BoundaryAffinity::Downstream).unwrap();
    let complete = &snapshot.rows[snapshot.caret_geometry(caret).unwrap().row_index];
    assert_eq!(row.ascent, complete.ascent, "{context}: code font ascent");
    assert_eq!(row.descent, complete.descent, "{context}: code font descent");
    assert_eq!(row.height(), complete.height(), "{context}: code line height");
    assert_eq!(row.carets[0].x, complete.carets[0].x, "{context}: code indentation");
}

#[test]
fn isolated_background_jobs_keep_both_trailing_code_rows_and_their_bottom_padding() {
    let padding = 12.;
    for (source, body, retained_enters) in [
        ("Before\n\n```rust\ncode\n```", "code", 2),
        ("Before\n\n```rust\ncode\n```\n\nAfter", "code", 2),
        ("Before\n\n```rust\ncode", "code", 2),
        ("Before\n\n> ```rust\n> code\n> ```", "code", 2),
        ("---\nkey: value\n---\n\nAfter", "key: value", 1),
    ] {
        let mut editor = Editor::open(source, Format::Markdown);
        set_block_style(&mut editor, "Code Block", StyleProperty::BlockBackground,
            StylePropertyValue::Color(Color { red: 1., green: 0., blue: 0., alpha: 1. }));
        set_block_style(&mut editor, "Code Block", StyleProperty::BlockPaddingBottom, StylePropertyValue::Float(padding));
        set_block_style(&mut editor, "Code Block", StyleProperty::BlockPaddingLeft, StylePropertyValue::Float(24.));
        editor.place_at(body);
        editor.key(Key::Char('A'));
        for enter in 1..=retained_enters {
            editor.key(Key::Enter);
            assert_isolated_code_caret_row(&editor, padding, &format!("{source:?}: Enter {enter}"));
            assert_reopens_as_displayed(editor.document(), source);
        }
    }
}

#[test]
fn open_commands_keep_empty_code_rows_in_isolated_background_jobs() {
    let padding = 12.;
    for source in ["Before\n\n```rust\ncode\n```", "Before\n\n```rust\n```"] {
        for command in ['o', 'O'] {
            let mut editor = Editor::open(source, Format::Markdown);
            set_block_style(&mut editor, "Code Block", StyleProperty::BlockBackground,
                StylePropertyValue::Color(Color { red: 1., green: 0., blue: 0., alpha: 1. }));
            set_block_style(&mut editor, "Code Block", StyleProperty::BlockPaddingBottom, StylePropertyValue::Float(padding));
            let code = editor.document().projection().blocks().iter()
                .find(|block| block.style.0 == "Code Block").unwrap().range.clone();
            editor.place(code.start);
            editor.key(Key::Char(command));
            // O before nonempty content has another code row below it, so
            // only a final row owns bottom padding.
            let at = editor.core.command_state(editor.view).unwrap().cursor();
            let end = editor.document().projection().blocks().iter()
                .find(|block| block.style.0 == "Code Block").unwrap().range.end;
            assert_isolated_code_caret_row(&editor, if at == end { padding } else { 0. },
                &format!("{source:?}: {command}"));
            assert_reopens_as_displayed(editor.document(), source);
        }
    }
}

#[test]
fn a_trailing_hard_break_row_belongs_to_its_paragraph() {
    for source in ["> quote\n\nAfter\n", "- item\n- next\n", "Para\n\nAfter\n"] {
        let mut editor = Editor::open(source, Format::Markdown);
        let needle = source.split(['>', '-', ' ', '\n']).find(|word| !word.is_empty()).unwrap();
        editor.place_at(needle);
        let (owner, ..) = caret_row(&mut editor);
        editor.key(Key::Char('A'));
        editor.key(Key::ShiftEnter);
        let (paragraph, ..) = caret_row(&mut editor);
        assert_eq!(paragraph, owner, "{source:?}");
        assert_reopens_as_displayed(editor.document(), source);
    }
}

fn languages(document: &Document) -> Vec<Option<String>> {
    document
        .projection()
        .blocks()
        .iter()
        .filter(|block| block.style.0 == "Code Block")
        .map(|block| block.code_language.clone())
        .collect()
}

#[test]
fn joining_a_code_block_with_the_following_text_keeps_its_settings() {
    let pre = "Intro\n\n<pre lang=\"rust\" class=\"x\">fn main() {\n}</pre>\n\n## Next\n";
    let fence = "Intro\n\n```rust title=\"x\"\nfn main() {\n}\n```\n\nNext\n";
    for (source, keys, expected) in [
        (pre, vec![Key::Char('J')], "Intro\n\n<pre lang=\"rust\" class=\"x\">fn main() {\n} Next</pre>\n"),
        (pre, vec![Key::Char('j'), Key::Char('I'), Key::Backspace], "Intro\n\n<pre lang=\"rust\" class=\"x\">fn main() {\n}Next</pre>\n"),
        (pre, vec![Key::Char('A'), Key::Delete], "Intro\n\n<pre lang=\"rust\" class=\"x\">fn main() {\n}Next</pre>\n"),
        // The joined text is literal code, so HTML-significant characters
        // are escaped and inline Markdown syntax is not carried along.
        ("Intro\n\n<pre lang=\"rust\">fn main() {\n}</pre>\n\nNext **bold** a<b & c\nmore\n", vec![Key::Char('J')],
            "Intro\n\n<pre lang=\"rust\">fn main() {\n} Next bold a&lt;b &amp; c more</pre>\n"),
        (fence, vec![Key::Char('J')], "Intro\n\n```rust title=\"x\"\nfn main() {\n} Next\n```\n"),
        (fence, vec![Key::Char('j'), Key::Char('I'), Key::Backspace], "Intro\n\n```rust title=\"x\"\nfn main() {\n}Next\n```\n"),
    ] {
        let context = format!("{source:?} {keys:?}");
        let mut editor = Editor::open(source, Format::Markdown);
        let at = editor.document().text().rfind('}').unwrap();
        editor.place(at);
        for key in &keys {
            editor.key(*key);
        }
        editor.key(Key::Escape);
        assert_eq!(editor.source(), expected, "{context}");
        assert_eq!(languages(editor.document()), vec![Some("rust".to_owned())], "{context}");
        assert_reopens_as_displayed(editor.document(), &context);
        assert_exact_undo_redo(&mut editor, source, expected, &context);
    }
}

#[test]
fn joining_the_markdown_demo_rust_blocks_keeps_their_languages() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/markdown_demo.md")).unwrap();
    let original = languages(&Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    for needle in ["println!(\"Hello, Viem!\");\n}", "println!(\"answer: {answer}\");\n}"] {
        let mut editor = Editor::open(&source, Format::Markdown);
        let at = editor.document().text().find(needle).unwrap() + needle.len() - 1;
        editor.place(at);
        editor.key(Key::Char('J'));
        assert_eq!(languages(editor.document()), original, "J after {needle:?}");
    }
}

/// The text of every resolved reference link and definition.
fn references(document: &Document) -> Vec<String> {
    document
        .projection()
        .style_spans()
        .iter()
        .filter(|span| span.application == StyleApplication::Automatic("Markdown reference".into()))
        .map(|span| document.text()[span.range.clone()].to_owned())
        .collect()
}

#[test]
fn opening_a_paragraph_beside_a_reference_definition_keeps_the_definition() {
    for (source, needle, keys, expected) in [
        ("See [x].\n\n[x]: https://e.com\n\nAfter\n", "[x]:", vec![Key::Char('o')], "See [x].\n\n[x]: https://e.com\n\nX\n\nAfter\n"),
        ("See [x].\n\n[x]: https://e.com\n\nAfter\n", "[x]:", vec![Key::Char('O')], "See [x].\n\nX\n\n[x]: https://e.com\n\nAfter\n"),
        ("See [x].\n\n[x]: https://e.com\n\nAfter\n", "[x]:", vec![Key::Char('A'), Key::Enter], "See [x].\n\n[x]: https://e.com\n\nX\n\nAfter\n"),
        ("See [x].\n\n[x]: https://e.com 'title'\n", "[x]:", vec![Key::Char('o')], "See [x].\n\n[x]: https://e.com 'title'\n\nX\n"),
        ("See [x].\n\n[x]:\nhttps://e.com\n", "https", vec![Key::Char('o')], "See [x].\n\n[x]:\nhttps://e.com\n\nX\n"),
    ] {
        let context = format!("{source:?} {keys:?}");
        let mut editor = Editor::open(source, Format::Markdown);
        let links = references(editor.document());
        assert!(links.iter().any(|text| text.contains("[x]")) && !links.iter().all(|text| text.contains("]:")),
            "{context}: the fixture's reference resolves: {links:?}");
        editor.place_at(needle);
        for key in &keys {
            editor.key(*key);
        }
        editor.key(Key::Char('X'));
        editor.key(Key::Escape);
        assert_eq!(editor.source(), expected, "{context}");
        assert_eq!(references(editor.document()), links, "{context}: the reference still resolves");
        assert_reopens_as_displayed(editor.document(), &context);
        assert_exact_undo_redo(&mut editor, source, expected, &context);
    }
}

#[test]
fn a_copied_code_block_keeps_its_closing_delimiter_and_language_when_pasted() {
    use viem_core::command::clipboard::{
        ClipboardCommandContext, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget,
    };
    use viem_core::command::InputEvent;
    for (source, copy) in [
        ("Intro\n\n```rust\nfn x() {}\n```\n\nEnd\n", "Intro\n\n```rust\nfn x() {}\n```"),
        ("Intro\n\n~~~python title=x\nprint()\n~~~\n\nEnd\n", "Intro\n\n~~~python title=x\nprint()\n~~~"),
        ("Intro\n\n<pre lang=\"rust\">fn x() {}</pre>\n\nEnd\n", "Intro\n\n<pre lang=\"rust\">fn x() {}</pre>"),
    ] {
        for tail in ["", "Tail\n"] {
            let source = format!("{source}{}{tail}", if tail.is_empty() { "" } else { "\n" });
            let context = format!("{source:?}");
            let mut editor = Editor::open(&source, Format::Markdown);
            editor.key(Key::Char('i'));
            let end = editor.document().text().find("End").unwrap() - 1;
            for (offset, extend) in [(0, false), (end, true)] {
                editor.core.handle(editor.view, CoreEvent::PlaceCursor {
                    document_revision: editor.document().revision(), text_offset: offset,
                    affinity: BoundaryAffinity::Downstream, extend_selection: extend,
                }).unwrap();
            }
            let copied = editor.core.handle(editor.view, CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::CopySelection),
                clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
            }).unwrap().command.unwrap();
            let content = copied.clipboard_writes[0].content().clone();
            editor.key(Key::Escape);
            editor.place_at("End");
            editor.key(Key::Char('A'));
            editor.key(Key::Enter);
            editor.core.handle(editor.view, CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::PasteClipboard),
                clipboard: ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
                    ClipboardTarget::Clipboard, ClipboardGeneration(1), content)),
            }).unwrap_or_else(|error| panic!("{context}: {error:?}"));
            editor.key(Key::Escape);
            let pasted = editor.source();
            assert!(pasted.contains(&format!("End\n\n{copy}\n")), "{context}: {pasted:?}");
            assert!(pasted.ends_with(&format!("{copy}\n{}{tail}", if tail.is_empty() { "" } else { "\n" })), "{context}: {pasted:?}");
            let expected = if source.contains("python") { "python" } else { "rust" };
            assert_eq!(languages(editor.document()), vec![Some(expected.to_owned()); 2], "{context}");
            assert_reopens_as_displayed(editor.document(), &context);
        }
    }
}
