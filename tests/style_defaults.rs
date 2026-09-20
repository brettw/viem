use viem_core::document::*;

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn lists(document: &Document) -> Vec<String> {
    document
        .projection()
        .style_sheet()
        .block_styles()
        .filter(|style| style.id.is_internal_list())
        .map(|style| style.id.0.clone())
        .collect()
}

#[test]
fn native_defaults_do_not_replace_authored_font_requests() {
    for (format, source) in [
        (Format::Html, "<p style=\"font-family: 'SF Pro'\">Text</p>"),
        (Format::Rtf, r"{\rtf1\ansi\deff0{\fonttbl{\f0 SF Pro;}}\f0 Text}"),
    ] {
        let document = open(source, format);
        let style = viem_core::layout::DocumentLayoutStyles::semantic_character_at(
            document.projection(), 0, false).unwrap();
        assert_eq!(style.font_families, ["SF Pro"], "{format:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
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
                    style: StyleId(format!("BulletedList{level}")),
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
                Some(32.0 * level as f32)
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
    for level in [1, 3, 4] {
        let source = format!(
            "{}Words{}<!--keep-->",
            "<ul><li>".repeat(level),
            "</li></ul>".repeat(level)
        );
        let mut document = open(&source, Format::Html);
        document
            .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
                document: document.id(),
                revision: document.revision(),
                enabled: true,
            })
            .unwrap();
        let definitions_enabled = document.source_bytes();
        let id = StyleId(format!("BulletedList{level}"));
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
            saved.contains("<li class=\"viem-p-506172616772617068\">Words"),
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
        assert_eq!(document.source_bytes(), definitions_enabled);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn defaults_define_two_four_level_families_independent_of_used_depth() {
    let expected = [
        "BulletedList1",
        "BulletedList2",
        "BulletedList3",
        "BulletedList4",
        "NumberedList1",
        "NumberedList2",
        "NumberedList3",
        "NumberedList4",
    ];
    for format in [
        Format::PlainText,
        Format::Markdown,
        Format::MarkdownSource,
        Format::Html,
        Format::Rtf,
    ] {
        assert_eq!(lists(&open("", format)), expected, "{format:?}");
    }
    let markdown = (0..20)
        .map(|depth| format!("{}- Text", "  ".repeat(depth)))
        .collect::<Vec<_>>()
        .join("\n");
    let html = format!("{}Deep{}", "<ul><li>".repeat(20), "</li></ul>".repeat(20));
    for (format, source) in [(Format::Markdown, markdown), (Format::Html, html)] {
        let document = open(&source, format);
        assert_eq!(lists(&document), expected);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            document.projection().blocks().last().unwrap().style,
            StyleId::from("BulletedList4")
        );
    }
}

#[test]
fn editing_list_depth_reuses_fourth_style_and_undo_restores_source() {
    let original = "- One\n  - Two\n    - Three";
    let mut document = open(original, Format::MarkdownSource);
    let styles = lists(&document);
    let end = document.text().len();
    document.insert(end, "\n      - Four").unwrap();
    assert_eq!(lists(&document), styles);
    assert_eq!(
        document.projection().blocks().last().unwrap().style,
        StyleId::from("BulletedList4")
    );
    assert!(document.undo());
    assert_eq!(lists(&document), styles);
    assert_eq!(document.source_bytes(), original.as_bytes());
}
