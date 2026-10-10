use std::ops::Range;
use viem_core::document::{Document, Encoding, Format, ModelRequest, StyleApplication, TextEdit};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn styles(document: &Document) -> Vec<Vec<StyleApplication>> {
    document
        .text()
        .char_indices()
        .map(|(at, ch)| {
            let mut styles = document
                .projection()
                .style_spans()
                .iter()
                .filter(|span| span.range.start <= at && at + ch.len_utf8() <= span.range.end)
                .map(|span| span.application.clone())
                .collect::<Vec<_>>();
            styles.sort_by_key(|style| format!("{style:?}"));
            styles.dedup();
            styles
        })
        .collect()
}
fn assert_reopened(document: &Document, format: Format) {
    let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
    assert_eq!(
        document.text(),
        reopened.text(),
        "{:?}",
        String::from_utf8_lossy(&document.source_bytes())
    );
    assert_eq!(styles(document), styles(&reopened));
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.kind, &block.style, block.quote_depth))
            .collect::<Vec<_>>(),
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|block| (&block.range, &block.kind, &block.style, block.quote_depth))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        document
            .projection()
            .tables()
            .iter()
            .map(|table| (
                &table.range,
                &table.columns,
                table
                    .rows
                    .iter()
                    .map(|row| (
                        &row.range,
                        row.cells
                            .iter()
                            .map(|cell| (&cell.range, cell.missing))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>(),
        reopened
            .projection()
            .tables()
            .iter()
            .map(|table| (
                &table.range,
                &table.columns,
                table
                    .rows
                    .iter()
                    .map(|row| (
                        &row.range,
                        row.cells
                            .iter()
                            .map(|cell| (&cell.range, cell.missing))
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );
}
fn edit_and_history(
    source: &str,
    format: Format,
    range: Range<usize>,
    replacement: &str,
    expected_source: &str,
    allowed: Range<usize>,
) {
    for direct in [false, true] {
        let mut document = open(source, format);
        let before_styles = styles(&document);
        let mut expected = document.text().to_owned();
        expected.replace_range(range.clone(), replacement);
        if direct {
            if range.is_empty() {
                document.insert(range.start, replacement).unwrap();
            } else {
                document.replace(range.clone(), replacement).unwrap();
            }
        } else {
            let committed = document
                .apply_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(range.clone(), replacement)],
                })
                .unwrap();
            for patch in committed.summary().source_patches() {
                assert!(
                    allowed.start <= patch.range().start && patch.range().end <= allowed.end,
                    "{patch:?}"
                );
            }
        }
        assert_eq!(document.text(), expected);
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        assert_reopened(&document, format);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(styles(&document), before_styles);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected_source.as_bytes());
        assert_eq!(document.text(), expected);
        assert_reopened(&document, format);
    }
}

#[test]
fn code_body_start_typing_keeps_the_reopened_character_context() {
    for (source, needle, text) in [
        ("  - foo\n\n    bar\n", "bar", "*"),
        ("  - foo\n\n\tbar\n", "bar", "*"),
        ("   > > 1.  one\n>>\n>>     two\n", "two", "*"),
        ("  - foo\n\n    **bar**\n", "bar", "x"),
        (
            "```ruby\ndef foo(x)\n  return 3\nend\n```\n",
            "  return 3",
            "x",
        ),
        ("<pre><code>ab\ncd</code></pre>\n", "cd", "x"),
    ] {
        let at = open(source, Format::Markdown).text().find(needle).unwrap();
        let source_at = source.find(needle).unwrap();
        let mut expected_source = source.to_owned();
        expected_source.insert_str(source_at, text);
        edit_and_history(
            source,
            Format::Markdown,
            at..at,
            text,
            &expected_source,
            source_at..source_at,
        );
    }
}

#[test]
fn multiline_code_span_start_typing_preserves_source_and_styles() {
    for source in ["``\nfoo\n``\n", "a ``\nfoo\n`` b\n", "``\r\nfoo\r\n``\r\n"] {
        let at = open(source, Format::Markdown).text().find("foo").unwrap();
        let source_at = source.find("foo").unwrap();
        let mut expected_source = source.to_owned();
        expected_source.insert(source_at, 'x');
        edit_and_history(
            source,
            Format::Markdown,
            at..at,
            "x",
            &expected_source,
            source_at..source_at,
        );
    }
}

#[test]
fn enter_after_indented_code_preserves_the_unselected_boundary() {
    for ending in ["\n", "\r\n"] {
        let source = format!("    foo{ending}bar{ending}");
        let expected = format!("    foo{}bar{ending}", ending.repeat(4));
        let at = open(&source, Format::Markdown).text().find("bar").unwrap();
        let source_at = source.find("bar").unwrap();
        edit_and_history(
            &source,
            Format::Markdown,
            at..at,
            "\n",
            &expected,
            source_at..source_at,
        );
    }
}

#[test]
fn deleting_a_table_cell_suffix_protects_retained_whitespace_locally() {
    for ending in ["\n", "\r\n"] {
        let source = format!("| Col 1 | Col 2 |{ending}|-------|-------|{ending}");
        let expected = format!("| Col&#32; | Col 2 |{ending}|-------|-------|{ending}");
        edit_and_history(&source, Format::Markdown, 4..5, "", &expected, 5..7);
    }
    let source = "| 1 Col | Other |\n| --- | --- |\n";
    edit_and_history(
        source,
        Format::Markdown,
        0..1,
        "",
        "| &#32;Col | Other |\n| --- | --- |\n",
        2..4,
    );
}

#[test]
fn typing_beside_a_lone_pipe_after_a_table_keeps_it_in_prose() {
    let source = "| Table | Header |\n|-------|--------|\n|\n";
    for at in [13, 14] {
        let source_at = source.rfind('|').unwrap();
        let expected = if at == 13 {
            "| Table | Header |\n|-------|--------|\n\nx|\n"
        } else {
            "| Table | Header |\n|-------|--------|\n\n|x\n"
        };
        edit_and_history(
            source,
            Format::Markdown,
            at..at,
            "x",
            expected,
            source_at..source_at + 1,
        );
    }
}

#[test]
fn source_header_pipe_deletion_reparses_the_preceding_prose_context() {
    let source = "Hello World\n| abc | def |\n| --- | --- |\n| bar | baz |\n";
    edit_and_history(
        source,
        Format::MarkdownSource,
        12..13,
        "",
        "Hello World\n abc | def |\n| --- | --- |\n| bar | baz |\n",
        12..13,
    );
    let mut document = open(source, Format::MarkdownSource);
    document.replace(12..13, "").unwrap();
    assert!(document.projection().tables().is_empty());
}

#[test]
fn typing_after_literal_table_backslashes_and_escaped_pipes_reopens() {
    // Valid GFM cases from pulldown-cmark's escaped-pipe table fixtures.
    for body in [
        r"`\`",
        r"`\\`",
        r"`\|`",
        r"`\|\|\`",
        r"`x\|y\|z\`",
        r"`\.\|\`",
        "abc\\",
    ] {
        for header in [false, true] {
            let source = if header {
                format!("| Description | {body} |\n|-------------|-----------|\n| Basic | z |\n")
            } else {
                format!("| Description | Test case |\n|-------------|-----------|\n| Basic | {body} |\n")
            };
            let document = open(&source, Format::Markdown);
            let cell = if header {
                &document.projection().tables()[0].rows[0].cells[1]
            } else {
                &document.projection().tables()[0].rows[1].cells[1]
            };
            let at = cell.range.end;
            let source_at = if body.starts_with('`') {
                source.rfind('`').unwrap()
            } else {
                source.find("abc\\").unwrap() + 4
            };
            let mut expected = source.clone();
            expected.insert(source_at, 'x');
            edit_and_history(
                &source,
                Format::Markdown,
                at..at,
                "x",
                &expected,
                source_at..source_at,
            );
        }
    }
}

#[test]
fn continuation_and_table_cell_repairs_do_not_decode_growing_neighbors() {
    for length in [20, 30_000] {
        let source = format!("  - {}\n\n    bar\n\nTail", "a".repeat(length));
        let mut document = open(&source, Format::Markdown);
        let at = document.text().find("bar").unwrap();
        let source_at = source.find("bar").unwrap();
        let committed = document
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at, "*")],
            })
            .unwrap();
        assert!(
            committed.summary().projection_work().decoded_source_bytes() < 256,
            "{:?}",
            committed.summary().projection_work()
        );
        assert_eq!(committed.summary().source_patches().len(), 1);
        assert_eq!(
            committed.summary().source_patches()[0].range(),
            source_at..source_at
        );
        assert_reopened(&document, Format::Markdown);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_reopened(&document, Format::Markdown);

        let source = format!(
            "| Col 1 | Col 2 |\n|-------|-------|\n\n{}\n",
            "a".repeat(length)
        );
        let mut document = open(&source, Format::Markdown);
        let committed = document
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(4..5, "")],
            })
            .unwrap();
        assert!(
            committed.summary().projection_work().decoded_source_bytes() < 256,
            "{:?}",
            committed.summary().projection_work()
        );
        assert!(committed
            .summary()
            .source_patches()
            .iter()
            .all(|patch| 5 <= patch.range().start && patch.range().end <= 7));
        assert_reopened(&document, Format::Markdown);
    }
}

#[test]
fn table_pipe_escapes_after_multiple_backslashes_keep_code_editable() {
    for count in 2..=5 {
        let body = format!("`{}|`", "\\".repeat(count));
        for header in [false, true] {
            let source = if header {
                format!("| {body} | b |\n| --- | --- |\n| z | q |\n")
            } else {
                format!("| a | b |\n| --- | --- |\n| {body} | q |\n")
            };
            let document = open(&source, Format::Markdown);
            let cell = &document.projection().tables()[0].rows[usize::from(!header)].cells[0];
            let text = &document.text()[cell.range.clone()];
            assert_eq!(text, format!("{}|", "\\".repeat(count - 1)));
            let at = cell.range.start + 1;
            let source_at = source.find(&body).unwrap() + 2;
            let mut expected = source.clone();
            expected.insert(source_at, 'x');
            edit_and_history(
                &source,
                Format::Markdown,
                at..at,
                "x",
                &expected,
                source_at..source_at,
            );
        }
    }
}

#[test]
fn table_pipe_escape_is_applied_before_ordinary_inline_escape_pairs() {
    for count in 1..=5 {
        let body = format!("{}|", "\\".repeat(count));
        let source = format!("| a | b |\n| --- | --- |\n| {body} | q |\n");
        let document = open(&source, Format::Markdown);
        let cell = &document.projection().tables()[0].rows[1].cells[0];
        assert_eq!(
            &document.text()[cell.range.clone()],
            format!("{}|", "\\".repeat((count - 1) / 2))
        );
        assert_reopened(&document, Format::Markdown);
        let source_document = open(&source, Format::MarkdownSource);
        assert_eq!(source_document.source_bytes(), source.as_bytes());
        assert_eq!(
            source_document.projection().tables()[0].rows[1].cells.len(),
            2
        );
    }
}
