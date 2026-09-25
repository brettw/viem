use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn range(document: &Document, start: usize, end: usize) -> TextRange {
    TextRange::new(
        document.text_point(start).unwrap(),
        document.text_point(end).unwrap(),
    )
    .unwrap()
}

#[test]
fn html_link_character_edits_keep_destination_and_exact_undo() {
    for source in [
        "<p>a <a href='https://example.test/path?x=1&amp;y=2'>link</a> z</p><!--keep-->",
        "<p>a <a href='https://example.test/path?x=1&amp;y=2' style='color:red;text-decoration:none'>link</a> z</p><!--keep-->",
        "<p>a <a href='https://example.test/path?x=1&amp;y=2'><em>link</em></a> z</p><!--keep-->",
    ] {
        for deleted in [false, true] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
            if deleted {
                document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
                    StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                        StyleDefinitionEdit::DeleteCharacter("Link".into()))))).unwrap();
            }
            let before = document.source_bytes();
            let selected = range(&document, 2, 6);
            document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                    range: selected,
                    properties: CharacterProperties { bold: Some(true), underline: Some(false), ..Default::default() },
                }))).unwrap_or_else(|error| panic!("{source:?}, deleted={deleted}: {error:?}"));
            assert_eq!(document.text(), "a link z");
            assert_eq!(document.link_at(document.text_point(3).unwrap()).unwrap().as_deref(), Some("https://example.test/path?x=1&y=2"));
            let styled = DocumentLayoutStyles::character_at(document.projection(), 3, false).unwrap();
            assert!(styled.bold);
            assert!(!styled.underline);
            let source_after = document.source_bytes();
            assert!(String::from_utf8(source_after.clone()).unwrap().ends_with("<!--keep-->"));
            let selected = range(&document, 2, 6);
            document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::ClearDirectCharacterProperties {
                    range: selected, properties: std::collections::BTreeSet::from([StyleProperty::CharacterBold]),
                }))).unwrap_or_else(|error| panic!("clear {source:?} after {:?} deleted={deleted}: {error:?}", String::from_utf8_lossy(&source_after)));
            assert!(!DocumentLayoutStyles::character_at(document.projection(), 3, false).unwrap().bold);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source_after);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), before);
        }
    }
}

#[test]
fn link_queries_survive_deleted_appearance_and_text_reprojection() {
    for (source, format) in [
        ("[link](https://example.test/) tail", Format::Markdown),
        ("[link](https://example.test/) tail", Format::MarkdownSource),
        (
            "<p><a href='https://example.test/'>link</a> tail</p>",
            Format::Html,
        ),
        (
            "<p><a href='https://example.test/'>link</a> tail</p>",
            Format::HtmlSource,
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                    StyleDefinitionEdit::DeleteCharacter("Link".into()),
                )),
            ))
            .unwrap();
        let at = document.text().find("link").unwrap();
        assert_eq!(
            document
                .link_at(document.text_point(at).unwrap())
                .unwrap()
                .as_deref(),
            Some("https://example.test/")
        );
        DocumentLayoutStyles::resolve(document.projection()).unwrap();
        document.replace(at..at + 1, "L").unwrap();
        assert_eq!(
            document
                .link_at(document.text_point(at).unwrap())
                .unwrap()
                .as_deref(),
            Some("https://example.test/")
        );
        DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(
            DocumentLayoutStyles::character_at(document.projection(), at, false)
                .unwrap()
                .underline
        );
    }
}

#[test]
fn clearing_html_base_weight_and_relative_bold_are_independent() {
    for (source, property, expected_bold) in [
        ("<p><b>abc</b></p>", StyleProperty::CharacterWeight, true),
        ("<p><b>abc</b></p>", StyleProperty::CharacterBold, false),
        (
            "<p><b style='font-weight:500'>abc</b></p>",
            StyleProperty::CharacterWeight,
            false,
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let selected = range(&document, 0, 3);
        document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::ClearDirectCharacterProperties {
                    range: selected,
                    properties: std::collections::BTreeSet::from([property]),
                }),
            ))
            .unwrap_or_else(|error| panic!("{source:?} {property:?}: {error:?}"));
        assert_eq!(
            DocumentLayoutStyles::character_at(document.projection(), 1, false)
                .unwrap()
                .bold,
            expected_bold
        );
    }
}
