use super::super::*;
use super::style_arena_text;
use crate::document::{ConfigurationStyleIntent, ModelRequest, StyleDefinitionEdit,
    StyleDefinitionMetadata, StyleModelIntent, StyleModelRequest, StyleNamespace};

#[test]
fn markdown_paragraph_assignment_capabilities_match_model_support() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut document = Document::from_bytes(b"plain".to_vec(), Encoding::Utf8, format).unwrap();
        let mut custom = document.projection().style_sheet()
            .block_style(&"Paragraph".into()).unwrap().clone();
        custom.id = "Custom".into();
        custom.based_on = Some("Paragraph".into());
        document.apply_style_request(StyleModelRequest::new(document.id(), document.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                StyleDefinitionEdit::InsertBlock { style: custom, metadata: StyleDefinitionMetadata::generated("Custom") }
            )))).unwrap();
        let export = export_style_sheet(&document).unwrap();
        for definition in export.definitions.iter().filter(|definition| definition.namespace == VIEM_STYLE_NAMESPACE_BLOCK) {
            let id = style_arena_text(&export.strings, definition.stable_id);
            let supported = matches!(id, "Paragraph" | "Heading1" | "Heading2" | "Heading3" | "Heading4"
                | "Heading5" | "Heading6" | "Block quote" | "Code Block");
            assert_eq!(definition.capabilities & VIEM_STYLE_CAPABILITY_ASSIGN != 0, supported, "{format:?} {id}");
            let prepared = document.prepare_model_request(ModelRequest::AssignNamedStyle {
                document: document.id(), revision: document.revision(), range: 0..5,
                namespace: StyleNamespace::Block, style: id.into(),
            });
            assert_eq!(prepared.is_ok(), supported, "{format:?} {id}: {prepared:?}");
        }
        for id in ["Table", "Table cell", "Table header", "Image", "Custom"] {
            assert!(export.definitions.iter().any(|definition|
                style_arena_text(&export.strings, definition.stable_id) == id), "Missing fixture style {id}");
        }
        assert_eq!(document.source_bytes(), b"plain");
    }
}
