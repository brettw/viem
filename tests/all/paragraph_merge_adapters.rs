use viem_core::document::{
    FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
};
use viem_core::{Document, Encoding, Format};

#[test]
fn paragraph_boundary_and_selection_merges_keep_the_preceding_style() {
    for (format, source) in [
        (Format::Markdown, "alpha\n\n# bravo\n\ncharlie"),
        (Format::Markdown, "# alpha\n\nbravo\n\ncharlie"),
        (Format::Markdown, "alpha\n\n> bravo\n\ncharlie"),
        (Format::Markdown, "alpha\n\n- bravo\n- charlie"),
        (Format::Markdown, "- alpha\n- bravo\n- charlie"),
        (Format::Markdown, "alpha\n\n```\nbravo\n```\n\ncharlie"),
        (Format::Markdown, "```\nalpha\n```\n\nbravo\n\ncharlie"),

    ] {
        for selection in [false, true] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let start = document.text().find('\n').unwrap();
            let range = if selection {
                start - 2..start + 3
            } else {
                start..start + 1
            };
            let before = document.projection().blocks().to_vec();
            let expected = format!(
                "{}{}",
                &document.text()[..range.start],
                &document.text()[range.end..]
            );
            document.replace(range, "").unwrap_or_else(|error| {
                panic!("{format:?} selection={selection} {source:?}: {error:?}")
            });
            assert_eq!(document.text(), expected, "{source}");
            let after = document.projection().blocks();
            assert_eq!(after.len(), before.len() - 1, "{source}");
            assert_eq!(after[0].style, before[0].style, "{source}");
            assert_eq!(
                after[0].direct_paragraph, before[0].direct_paragraph,
                "{source}"
            );
            assert_eq!(
                after.last().unwrap().style,
                before.last().unwrap().style,
                "{source}"
            );
            assert_eq!(
                after.last().unwrap().direct_paragraph,
                before.last().unwrap().direct_paragraph,
                "{source}"
            );
            check_reopen_and_history(&mut document, source.as_bytes());
        }
    }
}

fn check_reopen_and_history(document: &mut Document, original: &[u8]) {
    let bytes = document.source_bytes();
    let reopened =
        Document::from_bytes(bytes.clone(), document.encoding(), document.format()).unwrap();
    assert_eq!(reopened.text(), document.text());
    let properties = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| {
                (
                    block.range.clone(),
                    block.style.clone(),
                    block.direct_paragraph.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(properties(&reopened), properties(document));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), bytes);
}

#[test]
fn markdown_code_merges_convert_literal_lines_and_keep_unselected_inline_syntax() {
    for source in [
        "**alpha**\n\n```\nbravo *literal*\nsecond <br>\n```\n\ncharlie",
        "# **alpha**\n\n```\nbravo *literal*\nsecond <br>\n```\n\ncharlie",
        "```\nfirst\nalpha\n```\n\n**bravo**\n\ncharlie",
        "```\nalpha\n```\n\n# remove\n\n**bravo**\n\ncharlie",
        "**alpha**\n\n```\nremove\n```\n\n```\nbravo\n```\n\ncharlie",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let at = document.text().find("alpha").unwrap() + 3;
        let end = document.text().find("bravo").unwrap() + 2;
        let expected = format!("{}{}", &document.text()[..at], &document.text()[end..]);
        let style = document.projection().blocks()[0].style.clone();
        document
            .delete(at..end)
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_eq!(document.text(), expected);
        assert_eq!(document.projection().blocks()[0].style, style);
        assert_eq!(document.projection().blocks().len(), 2);
        check_reopen_and_history(&mut document, source.as_bytes());
    }
}

#[test]
fn deleting_across_multiple_paragraphs_uses_the_same_rule_for_payloads() {
    for (format, source) in [
        (Format::Markdown, "# alpha\n\n> bravo\n\n- charlie\n\ndelta"),

    ] {
        for payload in [false, true] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let end = document.text().find("charlie").unwrap() + 2;
            let before = document.projection().blocks().to_vec();
            if payload {
                let content =
                    FormattedTextPayload::new(&document.hard_line_snapshot(), "", vec![]).unwrap();
                let prepared = document
                    .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                        document.id(),
                        document.revision(),
                        vec![FormattedPayloadEdit::new(3..end, content)],
                    ))
                    .unwrap();
                document.commit_model_transaction(prepared).unwrap();
            } else {
                document.delete(3..end).unwrap();
            }
            assert_eq!(document.text(), "alparlie\ndelta");
            assert_eq!(document.projection().blocks().len(), 2);
            assert_eq!(document.projection().blocks()[0].style, before[0].style);
            assert_eq!(
                document.projection().blocks()[0].direct_paragraph,
                before[0].direct_paragraph
            );
            assert_eq!(
                document.projection().blocks()[1].direct_paragraph,
                before[3].direct_paragraph
            );
            check_reopen_and_history(&mut document, source.as_bytes());
        }
    }
}

#[test]
fn markdown_empty_and_adjacent_code_paragraphs_merge_at_visible_boundaries() {
    for (source, index) in [
        ("alpha\n\n```\n```\n\ncharlie", 0),
        ("alpha\n\n```\n\n```\n\ncharlie", 0),
        ("```\nalpha\n```\n\n```\nbravo\n```\n\ncharlie", 0),
        ("```\nalpha\n```\n\n```\n```\n\ncharlie", 0),
        ("```\n```\n\nbravo\n\ncharlie", 0),
        ("alpha\n\n> ```\n> bravo\n> ```\n\ncharlie", 0),
        ("> ```\n> alpha\n> ```\n\nbravo\n\ncharlie", 0),
        ("- alpha\n\n  ```\n  bravo\n  ```\n- charlie", 0),
        ("- prefix\n\n  ```\n  alpha\n  ```\n\n  bravo\n- charlie", 1),
        ("alpha\n\n> ```\n> \n> ```\n\ncharlie", 0),
        ("> ```\n> \n> ```\n\nbravo\n\ncharlie", 0),
        ("- alpha\n\n  ```\n  \n  ```\n- charlie", 0),
        ("- prefix\n\n  ```\n  \n  ```\n\n  bravo\n- charlie", 1),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let count = document.projection().blocks().len();
        let boundary = document.projection().blocks()[index].range.end;
        let expected = format!(
            "{}{}",
            &document.text()[..boundary],
            &document.text()[boundary + 1..]
        );
        let style = document.projection().blocks()[index].style.clone();
        document
            .delete(boundary..boundary + 1)
            .unwrap_or_else(|error| {
                panic!(
                    "{source}: {error:?} {:?}",
                    document
                        .projection()
                        .blocks()
                        .iter()
                        .map(|b| (
                            &b.range,
                            &b.style,
                            &b.kind,
                            document
                                .projection()
                                .provenance()
                                .iter()
                                .filter(|p| p.formatted.start >= b.range.start
                                    && p.formatted.end <= b.range.end)
                                .map(|p| (&p.formatted, &p.source))
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(
            document.projection().blocks()[index].style,
            style,
            "{source}"
        );
        assert_eq!(document.projection().blocks().len(), count - 1, "{source}");
        check_reopen_and_history(&mut document, source.as_bytes());
    }
}

#[test]
fn every_selection_deletes_across_markdown_paragraph_boundaries() {
    let mut failures = Vec::new();
    for (format, source) in [
        (Format::Markdown, "# abc\n\n> def\n\n- ghi\n- jkl"),
        (Format::Markdown, "# **ab**\n\n> _cd_\n\n- `ef`"),
        (Format::Markdown, "**ab**\n\n```\ncd\n```\n\n_ef_"),
        (Format::Markdown, "abc\n\n> ```\n> def\n> ghi\n> ```\n\nend"),
        (Format::Markdown, "> ```\n> abc\n> def\n> ```\n\nend"),
        (Format::Markdown, "- abc\n\n  ```\n  def\n  ```\n- ghi"),

    ] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = original.text().to_owned();
        for start in 0..text.len() {
            for end in start + 1..=text.len() {
                let mut document =
                    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format)
                        .unwrap();
                let expected = format!("{}{}", &text[..start], &text[end..]);
                match document.delete(start..end) {
                    Ok(()) if document.text() == expected => {}
                    outcome => failures.push(format!(
                        "{format:?} {source:?} {start}..{end}: {outcome:?}, text={:?}",
                        document.text()
                    )),
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(100)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
