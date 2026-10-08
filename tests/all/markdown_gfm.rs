use viem_core::document::{BlockKind, Document, Encoding, Format, StyleApplication, SemanticInlineStyle};
fn open(source: &str) -> Document { Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap() }

#[test]
fn gfm_inline_recognition_and_literal_references() {
    for (source, expected) in [
        ("foo_bar_baz", "foo_bar_baz"),
        ("a * b * c", "a * b * c"),
        ("***both*** **strong *nested***", "both strong nested"),
        ("~~old~~ and ~also~", "old and also"),
        ("![*alt*](picture.png)", "\u{fffc}"),
        ("[label][ref]\n\n[ref]: https://example.com", "[label][ref]\n[ref]: https://example.com"),
        ("<https://example.com> <me@example.com>", "https://example.com me@example.com"),
        ("&amp; &copy; &#0; &#x80; &NotEqualTilde;", "& © � € ≂̸"),
        ("`&amp; ~~code~~`", "&amp; ~~code~~"),
        ("before <!-- *comment* --> after", "before <!-- *comment* --> after"),
    ] {
        let doc = open(source);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    let doc = open("![alt](image.png) <!-- comment --> ~~old~~");
    for name in ["Comment", "Strikethrough"] {
        assert!(doc.projection().style_spans().iter().any(|span| span.application == StyleApplication::Automatic(name.into())), "{name}");
    }
    assert!(open("***both***").projection().style_spans().iter().any(|s| s.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)));
}

#[test]
fn gfm_block_recognition() {
    for (source, expected) in [
        ("Title\n=====\n\nTail", "Title\nTail"),
        ("Title\n-----", "Title"),
        ("# Title ##", "Title"),
        ("  ## Title ###  ", "Title"),
        ("- # Title\n- tail", "Title\ntail"),
        ("- a\n\n  > quote", "a\nquote"),
        ("paragraph\n    continuation", "paragraph continuation"),
        ("  ```\n  code\n  ```", "code"),
        ("```\na\n    ```\nb\n```", "a\n    ```\nb"),
        ("Before\n\n---\n\nAfter", "Before\n\nAfter"),
    ] {
        let doc = open(source);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    let doc = open("- # Title\n- tail");
    assert_eq!(doc.projection().blocks()[0].style.0, "Heading1");
    assert!(matches!(doc.projection().blocks()[0].kind, BlockKind::ListItem { .. }));
    let doc = open("> outer\n>\n>> inner");
    assert_eq!(doc.projection().blocks()[1].quote_depth, 2);
    let doc = open("- a\n\n- b");
    assert!(doc.projection().blocks()[0].list_loose);
}

#[test]
fn passive_html_and_comments() {
    for (source, expected) in [
        ("<b>bold</b> and <em>italic</em>", "bold and italic"),
        ("x<sup>2</sup> H<sub>2</sub>O", "x<sup>2</sup> H<sub>2</sub>O"),
        ("<div>x<sup>2</sup> H<sub>2</sub>O</div>", "x<sup>2</sup> H<sub>2</sub>O"),
        ("<div>\n*literal*\n</div>", "*literal*"),
        ("<div><p>one</p><p>two</p></div>", "one\ntwo"),
        ("before <!-- &amp; comment --> after", "before <!-- &amp; comment --> after"),
        ("<!-- a comment -->", "<!-- a comment -->"),
        ("<pre>one\n  two</pre>", "one\n  two"),
        ("<span style=\"display:none\">visible</span>", "visible"),
        ("<script>alert('literal')</script>", "<script>alert('literal')</script>"),
    ] {
        let doc = open(source);
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn autolinks_resolve_and_stop_at_punctuation() {
    for (source, label, destination) in [
        ("<https://example.com>", "https://example.com", "https://example.com"),
        ("<me@example.com>", "me@example.com", "mailto:me@example.com"),
        ("See https://example.com/a_(b).", "https://example.com/a_(b)", "https://example.com/a_(b)"),
        ("(www.example.com)", "www.example.com", "http://www.example.com"),
        ("Email me@example.com.", "me@example.com", "mailto:me@example.com"),
        (r"\[label](https://example.test/)", "https://example.test/", "https://example.test/"),
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let at = doc.text().find(label).unwrap();
            assert_eq!(doc.link_at(doc.text_point(at).unwrap()).unwrap().as_deref(), Some(destination), "{source:?} {format:?}");
        }
    }
}

#[test]
fn new_syntax_accepts_edits_reopens_and_undoes_exactly() {
    for source in ["~~old~~", "# Title ##", "Title\n=====", "<b>bold</b>", "<div>\n*literal*\n</div>", "![alt](image.png)", "<!-- comment -->", "&copy; text", "- # Title\n- tail", "> ```\n> code\n> ```", "---", "Before\n\n---\n\nAfter", "Before\n***\nAfter", "> Before\n>\n> ---\n>\n> After"] {
        let original = open(source);
        for at in original.text().char_indices().map(|(at, _)| at).chain(std::iter::once(original.text().len())) {
            let mut doc = open(source);
            let mut expected = doc.text().to_owned(); expected.insert_str(at, "X");
            doc.replace(at..at, "X").unwrap_or_else(|error| panic!("{source:?} at {at}: {error:?}"));
            assert_eq!(doc.text(), expected);
            let reopened = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo()); assert_eq!(doc.text(), expected);
        }
    }
}

#[test]
fn quote_rules_and_loose_lists_invalidate_layout_and_keep_large_documents_local() {
    use viem_core::{Core, CoreEvent};
    use viem_core::command::InputEvent;
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{DecorationKind, DocumentLayoutStyles, MockTextMeasurementProvider};
    let source = format!("> outer\n>\n>> inner\n\n---\n\n{}", "ordinary paragraph\n\n".repeat(10_000));
    let mut core = Core::new(open(&source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 500., 400.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let borders = |row: &viem_core::layout::VisualRow| row.decorations.iter().filter(|d| d.kind == DecorationKind::BlockQuoteBorder).count();
    assert_eq!(borders(&snapshot.rows[0]), 1);
    assert_eq!(borders(&snapshot.rows[1]), 2);
    let width = snapshot.rows.iter().flat_map(|r| &r.decorations).find(|d| d.kind == DecorationKind::ThematicBreak).unwrap().advance;
    core.handle(view, CoreEvent::Resize { width: 300., height: 400. }).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(snapshot.rows.iter().flat_map(|r| &r.decorations).find(|d| d.kind == DecorationKind::ThematicBreak).unwrap().advance < width);
    let at = core.document().text().find("inner").unwrap();
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(), text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    core.handle(view, CoreEvent::SetParagraphStyle { expected: core.list_selection_identity(view).unwrap(), style: "Paragraph".into() }).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(borders(&snapshot.rows[1]), 1);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text("X"))).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    let tight = DocumentLayoutStyles::resolve(open("- a\n- b").projection()).unwrap();
    let loose = DocumentLayoutStyles::resolve(open("- a\n\n- b").projection()).unwrap();
    assert!(loose.paragraphs[0].margin_bottom > tight.paragraphs[0].margin_bottom);
}

#[test]
fn reference_style_is_light_purple_and_definition_edits_refresh_other_paragraphs() {
    use viem_core::layout::DocumentLayoutStyles;
    let doc = open("[alt][reference]\n\n[reference]: x.png");
    let color = DocumentLayoutStyles::character_at(doc.projection(), 2, false).unwrap().foreground;
    assert_eq!((color.red, color.green, color.blue), (0.65, 0.45, 0.82));
    let mut doc = Document::from_bytes(b"[label]\n\n[label]: /url".to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    let at = doc.text().rfind("label").unwrap();
    doc.replace(at..at + 5, "other").unwrap();
    let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    assert_eq!(doc.projection().style_spans(), fresh.projection().style_spans());
}

#[test]
fn multi_character_entities_remain_editable_and_share_one_source_rewrite() {
    use viem_core::document::{ModelRequest, TextEdit};
    let mut doc = open("a &fjlig; b");
    assert_eq!(doc.text(), "a fj b");
    doc.replace(3..3, "X").unwrap();
    assert_eq!(doc.text(), "a fXj b");
    assert!(doc.undo());
    doc.apply_model_request(ModelRequest::ApplyTextEdits { document: doc.id(), revision: doc.revision(), edits: vec![TextEdit::new(2..3, "F"), TextEdit::new(3..4, "J")] }).unwrap();
    assert_eq!(doc.text(), "a FJ b");
    assert_eq!(open(&String::from_utf8(doc.source_bytes()).unwrap()).text(), "a FJ b");
    assert!(doc.undo()); assert_eq!(doc.source_bytes(), b"a &fjlig; b");
}

#[test]
fn demo_loads_in_both_views_and_lays_out_without_changing_source() {
    use viem_core::Core;
    use viem_core::layout::MockTextMeasurementProvider;
    let source = include_str!("../../docs/markdown_demo.md");
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 800., 600.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.rows.is_empty());
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(core.document().text().contains("Unsupported GitHub features"));
        let images = core.document().projection().inline_images_for_region(&(0..core.document().text().len()));
        assert!(!images.is_empty());
        if format.is_source_view() {
            assert!(images.iter().all(|image| {
                let notation = &core.document().text()[image.range.clone()];
                notation.starts_with("![") || notation.starts_with("<img")
            }));
        }
    }
}

#[test]
fn editing_a_rule_preserves_surrounding_paragraph_boundaries() {
    for source in ["---", "Before\n***\nAfter", "Before\n\n---\n\nAfter", "> Before\n>\n> ---\n>\n> After"] {
        let original = open(source);
        let at = original.projection().blocks().iter().find(|block| block.thematic_break).unwrap().range.start;
        for inserted in ["X", " ", "\n", "x\ny"] {
            let mut doc = open(source);
            let mut expected = original.text().to_owned(); expected.insert_str(at, inserted);
            doc.replace(at..at, inserted).unwrap_or_else(|error| panic!("{source:?} {inserted:?}: {error:?}"));
            assert_eq!(doc.text(), expected);
            assert_eq!(open(&String::from_utf8(doc.source_bytes()).unwrap()).text(), expected);
            assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo()); assert_eq!(doc.text(), expected);
        }
    }
}
