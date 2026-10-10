use viem_core::document::{
    Document, Encoding, FileFormat, Format, ModelRequest, StyleSpan, TextEdit,
};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        _ => text.as_bytes().to_vec(),
    }
}

fn inline_context(styles: &[StyleSpan], at: usize) -> Vec<String> {
    let mut context = styles
        .iter()
        .filter(|span| span.range.contains(&at))
        .map(|span| format!("{:?}", span.application))
        .collect::<Vec<_>>();
    context.sort();
    context
}

fn assert_model_edit(source: &str, edit: TextEdit, expected_source: &str) {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (file_format, ending) in [
            (FileFormat::Unix, "\n"),
            (FileFormat::Dos, "\r\n"),
            (FileFormat::Mac, "\r"),
        ] {
            let source = source.replace('\n', ending);
            let bytes = encoded(&source, encoding);
            let mut document = Document::from_bytes_with_file_format(
                bytes.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            let before = document.text().to_owned();
            let styles = document.projection().style_spans().to_vec();
            let mut expected = before.clone();
            expected.replace_range(edit.range.clone(), &edit.replacement);
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![edit.clone()],
                })
                .unwrap_or_else(|error| panic!("{source:?} {edit:?}: {error:?}"));
            for patch in prepared.summary().source_patches() {
                assert_ne!(patch.range(), 0..bytes.len(), "{source:?} {edit:?}");
            }
            assert_eq!(document.source_bytes(), bytes);
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), expected, "{source:?} {edit:?}");
            let saved = document.source_bytes();
            assert_eq!(
                saved,
                encoded(&expected_source.replace('\n', ending), encoding)
            );
            let fresh = Document::from_bytes_with_file_format(
                saved.clone(),
                encoding,
                Format::Markdown,
                file_format,
            )
            .unwrap();
            assert_eq!(document.text(), fresh.text());
            assert_eq!(
                document.projection().style_spans(),
                fresh.projection().style_spans()
            );
            assert_eq!(
                document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (&block.range, &block.attributes))
                    .collect::<Vec<_>>(),
                fresh
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (&block.range, &block.attributes))
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                document
                    .hard_line_snapshot()
                    .capture(0..expected.len())
                    .unwrap()
                    .break_offsets(),
                fresh
                    .hard_line_snapshot()
                    .capture(0..expected.len())
                    .unwrap()
                    .break_offsets()
            );
            for (at, _) in before
                .char_indices()
                .filter(|(at, ch)| !edit.range.contains(at) && *ch != '\n')
            {
                let after = if at < edit.range.start {
                    at
                } else {
                    at - edit.range.len() + edit.replacement.len()
                };
                assert_eq!(
                    inline_context(&styles, at),
                    inline_context(document.projection().style_spans(), after),
                    "retained context at {at} in {source:?}"
                );
            }
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
            assert_eq!(document.text(), before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
            assert_eq!(document.text(), expected);
        }
    }
}

#[test]
fn heading_body_whitespace_stays_visible_inside_list_owners() {
    for (source, at, text, expected_source) in [
        ("- # Foo\n- Bar\n", 0, " ", "- # &#32;Foo\n- Bar\n"),
        ("- # Foo\n- Bar\n", 0, "\t", "- # &#9;Foo\n- Bar\n"),
        (
            "- # **Foo**\n- *Bar*\n",
            0,
            "\t",
            "- # <strong>\tFoo</strong>\n- *Bar*\n",
        ),
        (
            "- _t\n  # test\n  t_\n",
            3,
            " ",
            "- _t\n  # &#32;test\n  t_\n",
        ),
        ("* > foo", 0, " ", "* > &#32;foo"),
    ] {
        assert_model_edit(source, TextEdit::new(at..at, text), expected_source);
    }
}

#[test]
fn indentation_typing_keeps_an_outside_code_block_outside_its_list() {
    assert_model_edit(
        " -    one\n\n     two\n",
        TextEdit::new(4..4, " "),
        " -    one\n\n```\n  two\n```\n",
    );
    assert_model_edit(
        " -    one\n\n     two\n",
        TextEdit::new(4..5, "\t"),
        " -    one\n\n```\n\ttwo\n```\n",
    );
}

#[test]
fn joining_alternate_atx_headings_keeps_the_first_heading_style() {
    for (source, expected_source) in [
        ("## foo ##\n  ###   bar    ###\n", "## foobar    ###\n"),
        (
            "# foo ##################################\n##### foo ##\n",
            "# foofoo ##\n",
        ),
        (" ### foo\n  ## foo\n   # foo\n", " ### foofoo\n   # foo\n"),
        ("## **foo** ##\n  ### *bar* ###\n", "## **foo***bar* ###\n"),
        ("## *foo* ##\n  ### **bar** ###\n", "## *foo***bar** ###\n"),
        ("## foo ##\n  ### **bar** ###\n", "## foo**bar** ###\n"),
        ("## **foo** ##\n  ### bar ###\n", "## **foo**bar ###\n"),
    ] {
        assert_model_edit(source, TextEdit::new(3..4, ""), expected_source);
    }
    let long_padding = format!("## foo ##\n  ###{}**bar** ###\n", " ".repeat(600));
    assert_model_edit(
        &long_padding,
        TextEdit::new(3..4, ""),
        "## foo**bar** ###\n",
    );
}

#[test]
fn empty_item_typing_preserves_its_break_and_unrelated_caret_seeds() {
    assert_model_edit(
        "-   \n  foo\n",
        TextEdit::new(0..0, "x"),
        "-   \n  xfoo\n",
    );
    assert_model_edit(
        "- foo\n-   \n- bar\n",
        TextEdit::new(0..0, "x"),
        "- xfoo\n-   \n- bar\n",
    );
}

#[test]
fn list_paragraph_splits_keep_tab_indented_code_and_nested_markers() {
    assert_model_edit(
        "- foo\n\n\t\tbar\n",
        TextEdit::new(0..0, "\n"),
        "- \n- foo\n\n\t\tbar\n",
    );
    assert_model_edit(
        " - foo\n   - bar\n\t - baz\n",
        TextEdit::new(4..4, "\n"),
        " - foo\n\n   - \n   - bar\n\t - baz\n",
    );
}

#[test]
fn regional_list_parses_retain_captured_lazy_continuation_indentation() {
    let source = "- parent\n  9. child\n     continued\n  10. next\n- tail";
    assert_model_edit(
        source,
        TextEdit::new(10..11, ""),
        "- parent\n  9. hild\n     continued\n  10. next\n- tail",
    );
    assert_model_edit(
        source,
        TextEdit::new(30..31, ""),
        "- parent\n  9. child\n     continued\n  10. ext\n- tail",
    );
}
