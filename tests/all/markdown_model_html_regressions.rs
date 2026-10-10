use std::ops::Range;
use viem_core::document::{
    Document, Encoding, FileFormat, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};
use viem_core::layout::DocumentLayoutStyles;

fn reopen(document: &Document) {
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        Format::Markdown,
        document.file_format(),
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    for (at, _) in document.text().char_indices() {
        assert_eq!(
            DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap(),
            DocumentLayoutStyles::character_at(fresh.projection(), at, false).unwrap(),
            "at {at}"
        );
    }
    let blocks = |doc: &Document| {
        doc.projection()
            .blocks()
            .iter()
            .map(|block| {
                (
                    block.range.clone(),
                    block.kind.clone(),
                    block.style.clone(),
                    block.quote_depth,
                    block.markdown_html,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(document), blocks(&fresh));
}

fn edit(document: &mut Document, range: Range<usize>, replacement: &str) {
    let original = document.source_bytes();
    let before = document.text().to_owned();
    let mut expected = before.clone();
    expected.replace_range(range.clone(), replacement);
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(range, replacement)],
        })
        .unwrap();
    let patches = prepared.summary().source_patches();
    // These reports need only local scalar edits and syntax beside them. An
    // audit must not hide a serializer rewriting the complete HTML/comment.
    assert!(
        patches.iter().all(|patch| patch.range().len() <= 2 * 4),
        "{patches:?}"
    );
    let mut patched = original.clone();
    for patch in patches.iter().rev() {
        patched.splice(patch.range(), patch.replacement().iter().copied());
    }
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), expected);
    assert_eq!(document.source_bytes(), patched);
    reopen(document);
    let saved = document.source_bytes();
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert_eq!(document.text(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
    assert_eq!(document.text(), expected);
    reopen(document);
}

fn bytes(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}

#[test]
fn literal_comment_and_tag_edits_reopen_without_consuming_neighbors() {
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
            for (source, needle, delta) in [
                ("<!-- <dl> -->\n- **foo** (u8, u8)\n", "-->", 0),
                ("- foo\n- bar\n\n<!-- -->\n\n- baz\n- bim\n", "-->", 0),
                ("* <foo>\n\t<bar>\n", "<bar>", 2),
            ] {
                let original = bytes(&source.replace('\n', ending), encoding);
                let mut doc = Document::from_bytes_with_file_format(
                    original,
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                let at = doc.text().find(needle).unwrap() + delta;
                let footer = source
                    .find("- **foo**")
                    .map(|at| bytes(&source[at..].replace('\n', ending), encoding));
                edit(&mut doc, at..at + 1, "");
                if let Some(footer) = footer {
                    assert!(doc.source_bytes().ends_with(&footer));
                    let at = doc.text().find("foo").unwrap();
                    assert!(
                        DocumentLayoutStyles::character_at(doc.projection(), at, false)
                            .unwrap()
                            .bold
                    );
                }
            }
        }
    }
}

#[test]
fn deleting_lone_backtick_retains_unselected_empty_paragraph() {
    let mut doc = Document::from_bytes(
        b"`Foo\n----\n`\n\n<a title=\"a lot\n---\nof dashes\"/>\n".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    edit(&mut doc, 5..6, "");
    assert_eq!(doc.text(), "`Foo\n\n<a title=\"a lot\nof dashes\"/>");
    assert!(doc
        .source_bytes()
        .ends_with(b"<a title=\"a lot\n---\nof dashes\"/>\n"));
}

#[test]
fn deleting_separated_paragraph_needs_no_added_empty_scope() {
    let mut doc =
        Document::from_bytes(b"a\n\nb\n\nc".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    edit(&mut doc, 2..3, "");
    assert_eq!(doc.source_bytes(), b"a\n\n\n\nc");
}

#[test]
fn stable_comment_body_typing_keeps_regional_projection_work() {
    let source = format!(
        "<!-- stable comment -->\n\n{}",
        "tail paragraph\n\n".repeat(1000)
    );
    let mut doc =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let prepared = doc
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(),
            revision: doc.revision(),
            edits: vec![TextEdit::new(8..8, "x")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 100, "{work:?}");
    assert_eq!(work.full_text_bytes_materialized(), 0);
    doc.commit_model_transaction(prepared).unwrap();
    reopen(&doc);
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn typing_before_literal_html_preserves_its_visible_rows_and_punctuation() {
    for source in ["<!-- foo -->*bar*\n", "<table><tr><td>\n..."] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        edit(&mut doc, 0..0, "x");
    }
}

#[test]
fn unsupported_inline_tag_name_cannot_activate_semantic_html() {
    let mut doc = Document::from_bytes(
        b"before <bravo> after".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
    )
    .unwrap();
    edit(&mut doc, 10..13, "");
    assert_eq!(doc.text(), "before <br> after");
    assert!(doc.source_bytes().starts_with(b"before "));
    assert!(doc.source_bytes().ends_with(b" after"));
    assert_eq!(doc.source_bytes().len(), b"before <br> after".len() + 1);

    let source = "<div>before <bravo title='&amp;'> after</div>";
    let mut doc =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = doc.text().find("bravo").unwrap() + 2;
    edit(&mut doc, at..at + 3, "");
    assert_eq!(doc.text(), "before <br title='&amp;'> after");
    assert_eq!(
        doc.source_bytes(),
        b"<div>before &lt;br title='&amp;amp;'> after</div>"
    );
}

#[test]
fn breaking_multiline_reference_definition_retains_visible_rows() {
    let source = "[Foo bar]:\n<my url>\n'title'\n\n[Foo bar]\n";
    let mut doc =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    edit(&mut doc, 9..9, "x");
    assert_eq!(doc.text(), "[Foo bar]x:\n<my url>\n'title'\n[Foo bar]");
}

#[test]
fn multiline_reference_rows_accept_boundary_punctuation_spaces_and_deletion() {
    for source in ["[foo]:\n/url\n'title'\n", "[Foo\n  bar]: /url\n"] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let text = original.text().to_owned();
        for at in text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
        {
            for replacement in ["x", " ", "*"] {
                let mut doc = Document::from_bytes(
                    source.as_bytes().to_vec(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                edit(&mut doc, at..at, replacement);
            }
            if let Some(ch) = text[at..].chars().next() {
                let mut doc = Document::from_bytes(
                    source.as_bytes().to_vec(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                edit(&mut doc, at..at + ch.len_utf8(), "");
            }
        }
    }
}

#[test]
fn breaking_definition_keeps_literal_label_punctuation() {
    for source in ["[*foo*]: /url\n", "[`foo`]: /url\n"] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        edit(&mut doc, 0..0, "x");
    }
}

#[test]
fn documented_html_list_and_comment_edits_reopen_and_undo_exactly() {
    for (source, range, replacement, expected) in [
        ("<ul><li>a</li><li>b</li></ul>", 0..3, "X", "X"),
        ("<ul><li>a</li><li>b</li></ul>", 1..2, "", "ab"),
        ("<div><!--x--></div>", 6..6, "X", "<!--x-X->"),
        ("<div><!--x--></div>", 7..7, "X", "<!--x--X>"),
    ] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        doc.replace(range, replacement).unwrap();
        assert_eq!(doc.text(), expected);
        reopen(&doc);
        let saved = doc.source_bytes();
        assert!(document_container_preserved(source, &saved));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), saved);
        reopen(&doc);
    }
}

fn document_container_preserved(source: &str, saved: &[u8]) -> bool {
    if source.starts_with("<ul>") {
        saved.starts_with(b"<ul><li>") && saved.ends_with(b"</li></ul>")
    } else {
        saved.starts_with(b"<div>") && saved.ends_with(b"</div>")
    }
}

#[test]
fn literal_comment_repair_preserves_malformed_bytes_and_alternate_markup() {
    let mut source = b"<!-- opaque ".to_vec();
    source.push(0xff);
    source.extend(b" -->\n- __keep__ &amp; spelling\n");
    let mut doc = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = doc.text().find("-->").unwrap();
    edit(&mut doc, at..at + 1, "");
    assert!(doc.source_bytes().contains(&0xff));
    assert!(doc.source_bytes().ends_with(b"- __keep__ &amp; spelling\n"));
}

#[test]
fn literal_grammar_repair_composes_atomically_with_an_unrelated_edit() {
    for source in [
        "<!-- foo -->*bar*\n\nTail",
        "<table><tr><td>\n...\n\nTail",
        "<!-- <dl> -->\n- **foo** (u8, u8)\n\nTail",
    ] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let original = doc.text().to_owned();
        let tail = original.find("Tail").unwrap();
        let first = if source.contains("<dl>") {
            let at = original.find("-->").unwrap();
            TextEdit::new(at..at + 1, "")
        } else {
            TextEdit::new(0..0, "x")
        };
        let mut expected = original.clone();
        expected.insert(tail, 'x');
        expected.replace_range(first.range.clone(), &first.replacement);
        doc.apply_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(),
            revision: doc.revision(),
            edits: vec![first, TextEdit::new(tail..tail, "x")],
        })
        .unwrap();
        assert_eq!(doc.text(), expected);
        reopen(&doc);
        let saved = doc.source_bytes();
        assert!(saved.ends_with(b"xTail"));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert_eq!(doc.text(), original);
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), saved);
        reopen(&doc);
    }
}
