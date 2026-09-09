use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::{Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn html_boundary_deletion_joins_into_the_preceding_paragraph() {
    let mut failures = Vec::new();
    for source in [
        "<p>A</p><h1>B</h1>",
        "<h1>A</h1><p>B</p>",
        "<p>A</p><p></p><p>B</p>",
        "<ul><li>A</li><li>B</li></ul>",
        "<p>A</p><ul><li>B</li><li>C</li></ul>",
        "<ul><li>A</li></ul><p>B</p>",
        "<p>A</p><blockquote><p>B</p><p>C</p></blockquote>",
        "<blockquote><p>A</p></blockquote><p>B</p>",
        "<p>A</p><pre>B</pre><p>C</p>",
        "<pre>A</pre><p>B</p>",
        "<div><p>A</p></div><section><p>B</p><p>C</p></section>",
        "<ol><li>A<ul><li>B</li></ul></li><li>C</li></ol>",
        "<p><b>A</b></p><!--keep--><p><i>B</i></p>",
    ] {
        let before = html(source);
        for (at, _) in before.text().match_indices('\n') {
            let mut document = html(source);
            let first = document
                .projection()
                .blocks()
                .iter()
                .find(|block| block.range.end == at)
                .unwrap()
                .clone();
            let mut expected = document.text().to_owned();
            expected.remove(at);
            match document.replace(at..at + 1, "") {
                Ok(_) => {
                    assert_eq!(document.text(), expected, "{source} at {at}");
                    let merged = document
                        .projection()
                        .blocks()
                        .iter()
                        .find(|block| block.range.start <= at && at <= block.range.end)
                        .unwrap();
                    assert_eq!(merged.style, first.style, "{source} at {at}");
                    assert!(document.undo());
                    assert_eq!(document.source_bytes(), source.as_bytes());
                }
                Err(error) => failures.push(format!("{source} at {at}: {error:?}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn html_selection_deletion_can_cross_multiple_different_blocks() {
    for source in [
        "<h1>AB</h1><p>CD</p><h2>EF</h2>",
        "<p><b>AB</b></p><!--keep--><ul><li>CD</li><li>EF</li><li>GH</li></ul>",
        "<div><p>AB</p></div><blockquote><p>CD</p></blockquote><pre>EF</pre>",
    ] {
        let mut document = html(source);
        let original = document.text().to_owned();
        document
            .replace(1..7, "")
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_eq!(
            document.text(),
            format!("{}{}", &original[..1], &original[7..])
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn backspace_resets_list_and_code_but_joins_other_block_styles() {
    for (source, expected, reset) in [
        ("<p>A</p><h1>B</h1>", "AB", false),
        ("<p>A</p><blockquote><p>B</p></blockquote>", "AB", false),
        ("<p>A</p><pre>B</pre>", "A\nB", true),
        ("<p>A</p><ol><li>B</li></ol>", "A\nB", true),
    ] {
        let mut document = html(source);
        let mut commands = CommandInterpreter::new();
        for key in "j0i".chars() {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap();
        }
        commands
            .handle(&mut document, InputEvent::Key(Key::Backspace))
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(
            document.projection().blocks()[usize::from(reset)].style.0,
            "Paragraph"
        );
    }
}

#[test]
fn every_legal_html_selection_can_be_deleted_or_replaced() {
    let mut failures = Vec::new();
    for source in [
        "<p>AB</p><p>CD</p><p>EF</p>",
        "<p><b>AB</b></p><!--keep--><h1><i>CD</i></h1><p>EF</p>",
        "<p>A</p><p></p><p>B</p>",
        "<ol start='3'><li>AB</li><li>CD</li><li>EF</li></ol>",
        "<ol><li>A<ul><li>B</li><li>C</li></ul></li><li>D</li></ol>",
        "<blockquote><p>AB</p></blockquote><p>CD</p><pre>EF</pre>",
        "<pre>AB\nCD</pre><p>EF</p>",
        "<p>AB</p><pre>C\n D</pre><p>EF</p>",
        "<pre>AB</pre><p> C   D </p><p>EF</p>",
        "<div>A<div></div>B</div>",
        "<h2>AB<p>CD",
        "<p> A </p><!--keep--><p> B </p>",
        "<p>&fjlig;</p><p><b>CD</b></p>",
        "<p>A<img src='keep'></p><p>B</p>",
        "<p>A</p><table><b>B</b><tr><td>keep</td></tr></table><p>C</p>",
    ] {
        let original = html(source);
        let text = original.text().to_owned();
        for start in 0..text.len() {
            for end in start + 1..=text.len() {
                if original.text_point(start).is_err() || original.text_point(end).is_err() {
                    continue;
                }
                for replacement in ["", "X"] {
                    let mut document = html(source);
                    match document.replace(start..end, replacement) {
                        Ok(_) => {
                            let expected =
                                format!("{}{replacement}{}", &text[..start], &text[end..]);
                            assert_eq!(
                                document.text().replace('\u{a0}', " "),
                                expected.replace('\u{a0}', " "),
                                "{source} {start}..{end}"
                            );
                        }
                        Err(error) => failures.push(format!(
                            "{source} {start}..{end} -> {replacement:?}: {error:?}"
                        )),
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn hard_break_insertion_at_anonymous_container_boundaries_is_editable() {
    for source in [
        "<div>A<div></div>B</div>",
        "<table><tr><td>keep</td></tr></table>",
    ] {
        let original = html(source);
        for at in (0..=original.text().len()).filter(|at| original.text_point(*at).is_ok()) {
            let mut document = html(source);
            let mut expected = document.text().to_owned();
            expected.insert(at, '\n');
            document
                .replace(at..at, "\n")
                .unwrap_or_else(|error| panic!("{source} at {at}: {error:?}"));
            assert_eq!(document.text(), expected);
        }
    }
}

#[test]
fn structural_batches_keep_empty_insertions_before_same_start_replacements() {
    use viem_core::document::TextEdit;
    for (format, source) in [
        (Format::Html, "<p>A</p><p>B</p><p>C</p>"),
        (Format::Markdown, "A\n\nB\n\nC"),
        (Format::Rtf, r"{\rtf1\ansi A\par B\par C}"),
    ] {
        for (edits, expected) in [
            (
                vec![
                    TextEdit::new(1..2, ""),
                    TextEdit::new(2..2, "X"),
                    TextEdit::new(2..3, "Y"),
                ],
                "AXY\nC",
            ),
            (
                vec![
                    TextEdit::new(0..0, "X"),
                    TextEdit::new(0..1, "Y"),
                    TextEdit::new(1..2, ""),
                ],
                "XYB\nC",
            ),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            document
                .apply_edits(edits)
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
            assert_eq!(document.text(), expected);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}
