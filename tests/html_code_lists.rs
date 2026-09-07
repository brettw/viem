use evim_core::document::{BlockKind, Document, Encoding, Format, ModelRequest, StyleId};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn pre_is_one_paragraph_with_whitespace_and_entities_preserved_by_local_edits() {
    let source = "<p>Before</p><pre data-keep='yes'><code>A\r\n  B\t&amp;\r\n\r\nC</code></pre><!--keep--><p>After</p>";
    let mut document = html(source);
    assert_eq!(document.text(), "Before\nA\n  B\t&\n\nC\nAfter");
    assert_eq!(document.projection().blocks().len(), 3);
    let code = &document.projection().blocks()[1];
    assert_eq!(code.style, StyleId::from("Code Block"));
    assert_eq!(&document.text()[code.range.clone()], "A\n  B\t&\n\nC");
    assert_eq!(document.projection().hard_line_count(), 6);
    let at = document.text().find("  B").unwrap() + 2;
    document.replace(at..at + 1, "Z").unwrap();
    assert_eq!(
        document.source_bytes(),
        source.replace("  B", "  Z").as_bytes()
    );
    assert_eq!(document.projection().blocks().len(), 3);
    assert_eq!(
        html(&String::from_utf8(document.source_bytes()).unwrap()).text(),
        document.text()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn html_list_continuation_source_lines_and_hard_breaks_keep_item_paragraphs() {
    let source =
        "<ol start='3'><li>A\n  <b>B</b><br>C</li><li value='8'>D\r\n E</li><li>F</li></ol>";
    let document = html(source);
    assert_eq!(document.text(), "A B\nC\nD E\nF");
    assert_eq!(document.projection().blocks().len(), 3);
    assert_eq!(document.projection().hard_line_count(), 4);
    let structure = document.projection().list_structure();
    // A declared ordinal restart begins a new logical numbering run.
    assert_eq!(structure.lists.len(), 2);
    assert_eq!(
        structure
            .lists
            .iter()
            .flat_map(|list| &list.items)
            .map(|item| item.ordinal)
            .collect::<Vec<_>>(),
        [3, 8, 9]
    );
    assert!(structure
        .lists
        .iter()
        .flat_map(|list| &list.items)
        .all(|item| item.paragraph_ids.len() == 1));
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn code_and_div_children_do_not_make_standalone_list_labels() {
    for (source, expected, paragraphs) in [
        ("<ul><li><div>A\n B</div></li><li>C</li></ul>", "A B\nC", 2),
        (
            "<ul><li><div>A</div><div>B</div></li><li>C</li></ul>",
            "A\nB\nC",
            3,
        ),
        (
            "<ul><li><pre>A\n B</pre></li><li>C</li></ul>",
            "A\n B\nC",
            2,
        ),
        ("<ul><li><div><p>A</p></div></li><li>C</li></ul>", "A\nC", 2),
        ("<ul><li><h2>A</h2></li><li>C</li></ul>", "A\nC", 2),
    ] {
        let document = html(source);
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(document.projection().blocks().len(), paragraphs, "{source}");
        assert!(
            document
                .projection()
                .blocks()
                .iter()
                .all(|block| matches!(block.kind, BlockKind::ListItem { .. })),
            "{source}"
        );
        let lists = document.projection().list_structure();
        assert_eq!(lists.lists.len(), 1);
        assert_eq!(lists.lists[0].items.len(), 2);
    }
}

#[test]
fn block_descendants_in_pre_still_belong_to_one_code_paragraph() {
    for (source, text) in [
        ("<pre>A<div>B</div>C</pre>", "A\nB\nC"),
        ("<pre><p>A</p><p>B</p></pre>", "A\nB"),
        ("<pre>A\n<div>B</div>\nC</pre>", "A\nB\n\nC"),
    ] {
        let document = html(source);
        assert_eq!(document.text(), text, "{source}");
        assert_eq!(document.projection().blocks().len(), 1, "{source}");
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Code Block")
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn enter_in_pre_adds_an_internal_break_and_keeps_the_source_wrapper() {
    for (source, at, expected, replacement) in [
        ("<pre>AB</pre>", 1, "A\nB", "<pre>A<br>B</pre>"),
        (
            "<pre><code>AB</code></pre>",
            1,
            "A\nB",
            "<pre><code>A<br>B</code></pre>",
        ),
        ("<pre></pre>", 0, "\n", "<pre><br></pre>"),
        ("<pre>AB</pre>", 0, "\nAB", "<pre><br>AB</pre>"),
        ("<pre>AB</pre>", 2, "AB\n", "<pre>AB<br></pre>"),
        (
            "<ol><li><pre>AB</pre></li></ol>",
            1,
            "A\nB",
            "<ol><li><pre>A<br>B</pre></li></ol>",
        ),
    ] {
        let mut document = html(source);
        document
            .apply_model_request(ModelRequest::ContinueList {
                document: document.id(),
                revision: document.revision(),
                at,
            })
            .unwrap();
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(document.projection().blocks().len(), 1);
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Code Block")
        );
        assert_eq!(document.source_bytes(), replacement.as_bytes(), "{source}");
        assert_eq!(html(replacement).text(), expected);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), replacement.as_bytes());
        assert_eq!(document.text(), expected);
    }
}

#[test]
fn source_pre_keeps_exact_source_hard_lines_in_one_paragraph() {
    let source =
        "<p>Before</p>\r\n<pre><code>A\r\n  B\t&amp;\r\n\r\nC</code></pre>\r\n<p>After</p>";
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    assert_eq!(document.text(), source.replace("\r\n", "\n"));
    assert_eq!(document.projection().blocks().len(), 3);
    let code = document.projection().blocks()[1].clone();
    assert_eq!(code.style, StyleId::from("Code Block"));
    assert_eq!(
        &document.text()[code.range],
        "<pre><code>A\n  B\t&amp;\n\nC</code></pre>"
    );
    assert_eq!(document.projection().hard_line_count(), 6);
    let at = document.text().find("  B").unwrap() + 2;
    document.replace(at..at + 1, "Z").unwrap();
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::HtmlSource).unwrap();
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| (block.range.clone(), block.style.clone()))
            .collect::<Vec<_>>(),
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|block| (block.range.clone(), block.style.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        document.source_bytes(),
        source.replace("  B", "  Z").as_bytes()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn source_pre_partition_ignores_pre_spelling_in_inert_and_opaque_content() {
    let source = "<script>'<pre>A\nB</pre>'</script>\n<template><pre>C\nD</pre></template>\n<object><pre>E\nF</pre></object>\n<pre>G\nH</pre>\n<p>Tail</p>";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    assert_eq!(document.text(), source);
    assert_eq!(document.source_bytes(), source.as_bytes());
    let code = document
        .projection()
        .blocks()
        .iter()
        .filter(|block| block.style == StyleId::from("Code Block"))
        .collect::<Vec<_>>();
    assert_eq!(code.len(), 1);
    assert_eq!(&document.text()[code[0].range.clone()], "<pre>G\nH</pre>");
}

#[test]
fn large_pre_edits_preserve_paragraph_identity_and_bounded_invalidation_in_both_views() {
    use evim_core::document::{ProjectionWorkScope, TextEdit};
    use evim_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let source = "<p>Before</p>\n<pre><code>".to_owned()
        + &"Code body line\n".repeat(10_000)
        + "Last</code></pre>\n<p>After</p>";
    for format in [Format::Html, Format::HtmlSource] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let code_id = document.projection().blocks()[1].id;
        let before_id = document.projection().blocks()[0].id;
        let at = document.text().find("Code body").unwrap() + 5;
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        engine.set_cache_capacity(10_010);
        let mut view = ViewLayout::new(300.0, 100.0);
        engine.relayout(&document, &mut view).unwrap();
        let shaped = engine.provider().request_calls();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at + 4, "BODY")],
            })
            .unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::RegionalHardLines,
            "{format:?}"
        );
        assert!(
            prepared.summary().projection_work().decoded_source_bytes() < 256,
            "{format:?}"
        );
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.projection().blocks().len(), 3);
        assert_eq!(document.projection().blocks()[1].id, code_id);
        assert_eq!(document.projection().blocks()[0].id, before_id);
        engine.relayout(&document, &mut view).unwrap();
        assert!(
            engine.provider().request_calls() - shaped <= 2,
            "{format:?}"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
