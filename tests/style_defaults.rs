use evim_core::document::*;

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn lists(document: &Document) -> Vec<String> {
    document
        .projection()
        .style_sheet()
        .block_styles()
        .filter(|style| style.id.0.starts_with("List"))
        .map(|style| style.id.0.clone())
        .collect()
}

#[test]
fn rich_default_list_paragraph_styles_are_assignable_source_backed_and_undoable() {
    for (format, source) in [
        (Format::Html, "<p data-keep='x'>Words</p><!--keep-->"),
        (Format::Rtf, r"{\rtf1 Words{\*\unknown keep}}"),
    ] {
        for level in 1..=3 {
            let mut document = open(source, format);
            document
                .apply_model_request(ModelRequest::AssignNamedStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: 0..0,
                    namespace: StyleNamespace::Block,
                    style: StyleId(format!("List{level}")),
                })
                .unwrap();
            assert_eq!(document.text(), "Words");
            let saved = String::from_utf8(document.source_bytes()).unwrap();
            let reopened = open(&saved, format);
            let id = &reopened.projection().blocks()[0].style;
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style(id)
                    .unwrap()
                    .block
                    .leading_indent,
                Some((if format == Format::Html { 32.0 } else { 20.0 }) * level as f32)
            );
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style_metadata(id)
                    .unwrap()
                    .origin,
                StyleDefinitionOrigin::SourceBacked
            );
            assert!(saved.contains("keep"));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn deleting_html_list_defaults_does_not_regenerate_them_on_reopen_or_edit() {
    for level in [1, 3, 6] {
        let source = format!(
            "{}Words{}<!--keep-->",
            "<ul><li>".repeat(level),
            "</li></ul>".repeat(level)
        );
        let mut document = open(&source, Format::Html);
        let id = StyleId(format!("List{level}"));
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                    origin: StyleDefinitionOrigin::SourceBacked,
                    edit: StyleDefinitionEdit::DeleteBlock(id.clone()),
                }),
            ))
            .unwrap();
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(saved.ends_with("<!--keep-->"));
        assert!(
            saved.contains("<li class=\"evim-p-506172616772617068\">Words"),
            "{saved}"
        );
        let reopened = open(&saved, Format::Html);
        assert!(reopened
            .projection()
            .style_sheet()
            .block_style(&id)
            .is_none());
        assert_eq!(
            reopened.projection().blocks().last().unwrap().style,
            StyleId::from("Paragraph")
        );
        let at = document.text().find("Words").unwrap() + 1;
        document.replace(at..at + 1, "O").unwrap();
        assert!(document
            .projection()
            .style_sheet()
            .block_style(&id)
            .is_none());
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn defaults_define_only_three_list_levels_and_import_synthesizes_used_depth() {
    for format in [
        Format::PlainText,
        Format::Markdown,
        Format::MarkdownSource,
        Format::Html,
        Format::Rtf,
    ] {
        assert_eq!(
            lists(&open("", format)),
            ["List1", "List2", "List3"],
            "{format:?}"
        );
    }
    let markdown = (0..6)
        .map(|depth| format!("{}- Level {depth}", "  ".repeat(depth)))
        .collect::<Vec<_>>()
        .join("\n");
    let html = format!("{}Deep{}", "<ul><li>".repeat(6), "</li></ul>".repeat(6));
    let rtf = format!(r"{{\rtf1{{\*\listtable{{\list{}\listid42}}}}{{\*\listoverridetable{{\listoverride\listid42\listoverridecount0\ls1}}}}\pard\ls1\ilvl5 Deep}}",
        r"{\listlevel\levelnfc23\levelstartat1{\leveltext\'01\u8226?;}{\levelnumbers;}\li720\fi-360}".repeat(6));
    for (format, source) in [
        (Format::Markdown, markdown),
        (Format::Html, html),
        (Format::Rtf, rtf),
    ] {
        let document = open(&source, format);
        assert_eq!(
            lists(&document),
            ["List1", "List2", "List3", "List4", "List5", "List6"],
            "{format:?}: {:?}",
            document.projection().blocks()
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
        let sixth = document
            .projection()
            .style_sheet()
            .block_style(&"List6".into())
            .unwrap();
        assert_eq!(
            sixth.block.leading_indent,
            Some(if format == Format::Rtf { 120.0 } else { 192.0 })
        );
    }
}

#[test]
fn editing_list_depth_adds_missing_styles_and_undo_restores_prior_sheet() {
    let mut document = open("- One\n  - Two\n    - Three", Format::MarkdownSource);
    assert_eq!(lists(&document).len(), 3);
    let end = document.text().len();
    document.insert(end, "\n      - Four").unwrap();
    assert_eq!(lists(&document), ["List1", "List2", "List3", "List4"]);
    assert!(document.undo());
    assert_eq!(lists(&document), ["List1", "List2", "List3"]);
}

#[test]
fn generated_list_styles_continue_beyond_the_old_sixteen_level_table() {
    let html = format!("{}Deep{}", "<ul><li>".repeat(20), "</li></ul>".repeat(20));
    let markdown = (0..20)
        .map(|depth| format!("{}- Text", "  ".repeat(depth)))
        .collect::<Vec<_>>()
        .join("\n");
    for (format, source) in [(Format::Html, html), (Format::Markdown, markdown)] {
        let document = open(&source, format);
        assert!(
            document
                .projection()
                .style_sheet()
                .block_style(&"List20".into())
                .is_some(),
            "{format:?}"
        );
        assert_eq!(
            document.projection().blocks().last().unwrap().style,
            StyleId::from("List20")
        );
    }
}
