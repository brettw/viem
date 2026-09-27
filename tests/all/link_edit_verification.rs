use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;



#[test]
fn link_queries_survive_deleted_appearance_and_text_reprojection() {
    for (source, format) in [
        ("[link](https://example.test/) tail", Format::Markdown),
        ("[link](https://example.test/) tail", Format::MarkdownSource),
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
