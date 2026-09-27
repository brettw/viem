use viem_core::document::{ContainerKind, Document, Encoding, Format};
fn open(text: &str, format: Format) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn path(doc: &Document, index: usize) -> Vec<ContainerKind> {
    doc.projection().blocks()[index].containers.iter().map(|member| member.container.kind).collect()
}

#[test]
fn markdown_quotes_own_multiple_independent_paragraphs_and_nested_blocks() {
    let source = "> first\n>\n> ## Heading\n>\n> > inner one\n> >\n> > inner two\n>\n> last\n\nOutside";
    let doc = open(source, Format::Markdown);
    assert_eq!(doc.text(), "first\nHeading\ninner one\ninner two\nlast\nOutside");
    assert_eq!(doc.projection().blocks()[0].style.0, "Paragraph");
    assert_eq!(doc.projection().blocks()[1].style.0, "Heading2");
    assert_eq!(path(&doc, 0), [ContainerKind::Quote]);
    assert_eq!(path(&doc, 2), [ContainerKind::Quote, ContainerKind::Quote]);
    assert_eq!(path(&doc, 5), []);
    let structure = doc.projection().container_structure();
    assert_eq!(structure.containers.len(), 2);
    assert_eq!(structure.containers[0].paragraph_ids.len(), 5);
    assert_eq!(structure.containers[1].parent, Some(structure.containers[0].attributes.id));
    assert_eq!(structure.containers[1].paragraph_ids.len(), 2);
    assert!(doc.projection().blocks()[0].containers[0].starts_here);
    assert!(!doc.projection().blocks()[2].containers[0].starts_here);
    assert!(doc.projection().blocks()[4].containers[0].ends_here);
    assert_eq!(doc.source_bytes(), source.as_bytes());
}

#[test]
fn literal_code_and_list_item_are_distinct_nested_owners() {
    let doc = open("> - first\n>\n>   second\n>\n>   ```rust\n>   a\n>\n>   b\n>   ```", Format::Markdown);
    let owners = doc.projection().container_structure();
    assert!(owners.containers.iter().any(|node| node.attributes.kind == ContainerKind::CodeBlock));
    let code = owners.containers.iter().find(|node| node.attributes.kind == ContainerKind::CodeBlock).unwrap();
    let parent = owners.containers.iter().find(|node| Some(node.attributes.id) == code.parent).unwrap();
    assert_eq!(parent.attributes.kind, ContainerKind::ListItem);
    assert_eq!(&doc.text()[code.range.clone()], "a\n\nb");
    assert!(owners.containers.iter().any(|node| node.attributes.kind == ContainerKind::List { ordered: false }));
}

#[test]
fn container_identity_survives_typing_deleting_first_paragraph_and_history() {
    for (format, source) in [
        (Format::Markdown, "> first\n>\n> second\n>\n> > inner"),
    ] {
        let mut doc = open(source, format);
        let initial = doc.projection().container_structure();
        let id = initial.containers[0].attributes.id;
        doc.replace(8..8, "X").unwrap();
        assert_eq!(doc.projection().container_structure().containers[0].attributes.id, id);
        assert!(doc.undo()); assert_eq!(doc.projection().container_structure(), initial);
        assert!(doc.redo());
        doc.replace(0..6, "").unwrap();
        assert_eq!(doc.projection().container_structure().containers[0].attributes.id, id);
        let reopened = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(doc.text(), reopened.text());
        assert_eq!(path(&doc, 0), path(&reopened, 0));
    }
}

#[test]
fn quote_wrap_and_heading_assignment_keep_inner_and_outer_structure() {
    for format in [Format::Markdown,] {
        let source = if format == Format::Markdown { "## Heading" } else { "<h2>Heading</h2>" };
        let mut doc = open(source, format);
        doc.set_paragraph_style(0..0, "Block quote".into()).unwrap();
        assert_eq!(doc.text(), "Heading");
        assert_eq!(doc.projection().blocks()[0].style.0, "Heading2");
        assert_eq!(path(&doc, 0), [ContainerKind::Quote]);
        doc.set_paragraph_style(0..0, "Heading3".into()).unwrap();
        assert_eq!(doc.projection().blocks()[0].style.0, "Heading3");
        assert_eq!(path(&doc, 0), [ContainerKind::Quote]);
        let reopened = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(path(&doc, 0), path(&reopened, 0));
        assert!(doc.undo()); assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_owners_keep_a_real_editable_leaf() {
    for (format, source, expected) in [
        (Format::Markdown, "> ", ContainerKind::Quote),
        (Format::Markdown, "```\n```", ContainerKind::CodeBlock),
    ] {
        let mut doc = open(source, format);
        assert_eq!(doc.text(), "");
        assert_eq!(path(&doc, 0), [expected], "{format:?}: {source}");
        doc.replace(0..0, "new").unwrap();
        assert_eq!(doc.text(), "new");
        assert_eq!(path(&doc, 0), [expected], "{format:?}: {source}");
        assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn ordinary_edits_keep_container_paths_and_projection_work_local() {
    use viem_core::document::{ModelRequest, TextEdit};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = format!("> first\n>\n{}> target\n>\n> > inner", "> body\n>\n".repeat(10_000));
        let mut doc = open(&source, format);
        let at = doc.text().find("target").unwrap() + 2;
        let before = doc.projection().blocks().iter().find(|block| block.range.contains(&at)).unwrap().containers[0].container.id;
        let prepared = doc.prepare_model_request(ModelRequest::ApplyTextEdits {
            document: doc.id(), revision: doc.revision(), edits: vec![TextEdit::new(at..at, "x")],
        }).unwrap();
        assert!(prepared.summary().projection_work().decoded_source_bytes() < 1024, "{format:?}: {:?}", prepared.summary().projection_work());
        doc.commit_model_transaction(prepared).unwrap();
        assert_eq!(doc.projection().blocks().iter().find(|block| block.range.contains(&at)).unwrap().containers[0].container.id, before);
        assert_eq!(doc.projection().container_structure().containers.len(), 2, "{format:?} {:?}", doc.projection().blocks().last());
    }
}

#[test]
fn rtf_nested_lists_use_portable_owners_and_preserve_authored_absolute_indents() {
    let source = concat!(
        r"{\rtf1{\*\listtable{\list{\listlevel\levelnfc0\levelstartat1{\leveltext\'02\'00.;}{\levelnumbers\'01;}\li720}",
        r"{\listlevel\levelnfc23\levelstartat1{\leveltext\'01\u8226?;}{\levelnumbers;}\li1440}\listid1}}",
        r"{\*\listoverridetable{\listoverride\listid1\listoverridecount0\ls1}}",
        r"\pard\ls1\ilvl0 Parent\par\pard\ls1\ilvl1 Child\par\pard\ls1\ilvl0 Tail}");
    let mut doc = open(source, Format::Rtf);
    assert_eq!(path(&doc, 0), [ContainerKind::List { ordered: true }, ContainerKind::ListItem]);
    assert_eq!(path(&doc, 1), [ContainerKind::List { ordered: true }, ContainerKind::ListItem,
        ContainerKind::List { ordered: false }, ContainerKind::ListItem]);
    assert_eq!(doc.projection().blocks()[0].direct_paragraph.leading_indent, Some(36.));
    assert_eq!(doc.projection().blocks()[1].direct_paragraph.leading_indent, Some(72.));
    for member in doc.projection().blocks()[1].containers.iter().filter(|member| matches!(member.container.kind, ContainerKind::List { .. })) {
        assert_eq!(member.container.direct_formatting.as_ref().unwrap().direct_paragraph.padding_left, Some(0.));
    }
    let before = doc.projection().container_structure();
    doc.insert(8, "X").unwrap();
    assert_eq!(doc.projection().container_structure().containers[0].attributes.id, before.containers[0].attributes.id);
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
    assert_eq!(doc.projection().container_structure(), before);
    let legacy = open(r"{\rtf1{\*\pn\pnlvlblt}One\par Two}", Format::Rtf);
    assert!(legacy.projection().blocks()[0].containers[0].container.direct_formatting.is_none());
}

#[test]
fn changing_a_container_kind_never_aliases_a_surviving_sibling_owner() {
    let mut doc = open("> - first\n>\n>   continued\n> - tail", Format::MarkdownSource);
    let quote = doc.projection().blocks()[0].containers[0].container.id;
    doc.delete(0..1).unwrap();
    assert_eq!(doc.projection().blocks()[1].containers[0].container.id, quote);
    assert_ne!(doc.projection().blocks()[0].containers[0].container.id, quote);
    doc.insert(0, "z").unwrap();
    let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    for (actual, expected) in doc.projection().blocks().iter().zip(fresh.projection().blocks()) {
        assert_eq!(actual.attributes, expected.attributes);
    }
}

#[test]
fn plain_style_clears_the_active_leaf_or_one_quote_level() {
    for (format, source) in [
        (Format::Markdown, "> > ## Heading"),
        (Format::Markdown, "> > ```\n> > literal\n> > ```"),
        (Format::Markdown, "> > - item"),
    ] {
        let mut doc = open(source, format);
        let text = doc.text().to_owned();
        doc.set_paragraph_style(0..0, "Paragraph".into()).unwrap_or_else(|error| panic!("{format:?} {source}: {error:?}"));
        assert_eq!(doc.text(), text);
        assert_eq!(path(&doc, 0), [ContainerKind::Quote, ContainerKind::Quote], "{format:?} {source}: {}", String::from_utf8_lossy(&doc.source_bytes()));
        assert_eq!(doc.projection().blocks()[0].style.0, "Paragraph");
        doc.set_paragraph_style(0..0, "Paragraph".into()).unwrap();
        assert_eq!(path(&doc, 0), [ContainerKind::Quote]);
        assert_eq!(doc.text(), text);
        let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(path(&fresh, 0), [ContainerKind::Quote]);
        assert!(doc.undo()); assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn source_code_unwrap_keeps_literal_quote_characters_inside_outer_quotes() {
    let mut doc = open("> ```\n> > literal\n> ```", Format::MarkdownSource);
    doc.set_paragraph_style(0..0, "Paragraph".into()).unwrap();
    let visible = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(visible.text(), "> literal");
    assert_eq!(visible.projection().blocks()[0].quote_depth, 1);
}

#[test]
fn adjacent_empty_quote_owners_remain_distinct_when_edited() {
    let source = ">\n\n>";
    let mut doc = open(source, Format::Markdown);
    assert_eq!(doc.text(), "\n");
    let before = doc.projection().container_structure();
    assert_eq!(before.containers.len(), 2);
    assert_ne!(before.containers[0].attributes.id, before.containers[1].attributes.id);
    doc.insert(1, "second").unwrap();
    assert_eq!(doc.text(), "\nsecond");
    let after = doc.projection().container_structure();
    assert_eq!(before.containers[0].attributes.id, after.containers[0].attributes.id);
    assert_eq!(before.containers[1].attributes.id, after.containers[1].attributes.id);
    assert!(doc.undo());
    assert_eq!(doc.projection().container_structure(), before);
}
