use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn configured(source: &str, format: Format, weight: u16) -> Document {
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let request = StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Configuration(ConfigurationStyleIntent::SetDocumentDefaultCharacter(
            CharacterProperties {
                weight: Some(weight),
                ..Default::default()
            },
        )),
    );
    document.apply_style_request(request).unwrap();
    document
}

#[test]
fn semantic_bold_adds_three_hundred_to_base_weight_in_all_rich_adapters() {
    for (format, source) in [
        (Format::Markdown, "word"),

    ] {
        let mut document = configured(source, format, 200);
        document
            .set_semantic_style(0..4, SemanticInlineStyle::Strong, true)
            .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
        assert_eq!(
            (style.base_weight, style.weight, style.bold),
            (200, 500, true),
            "{format:?}"
        );
        let after = document.source_bytes();
        document
            .set_semantic_style(0..4, SemanticInlineStyle::Strong, false)
            .unwrap();
        let style = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
        assert_eq!(
            (style.base_weight, style.weight, style.bold),
            (200, 200, false),
            "{format:?}"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), after);
    }
}
