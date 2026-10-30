use super::super::*;
use super::style_arena_text;
use crate::document::{ConfigurationStyleIntent, ModelRequest, StyleDefinitionEdit,
    StyleDefinitionMetadata, StyleModelIntent, StyleModelRequest, StyleNamespace};

#[test]
fn markdown_paragraph_assignment_capabilities_match_model_support() {
    for format in [Format::Markdown] {
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
                | "Heading5" | "Heading6" | "Block quote" | "Code Block"
                | "Note" | "Tip" | "Important" | "Warning" | "Caution");
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

#[test]
fn gutter_identity_is_independent_of_content_style_names_and_namespaces() {
    let mut document = Document::from_bytes(b"plain".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let defaults = serde_json::json!({"version":1,
        "block_styles":[
            {"id":"Heading1","name":"Line number","role":"Paragraph","based_on":"Paragraph",
                "character":{"size":23},"block":{}},
            {"id":"Line number","name":"Line number","role":"Paragraph","based_on":"Paragraph",
                "character":{"size":29},"block":{}},
            {"id":"Builtin line number","name":"Line number","role":"Paragraph","based_on":"Paragraph",
                "character":{"size":37},"block":{}}
        ],
        "character_styles":[
            {"id":"Code","name":"Line number","properties":{"size":17,"bold":true}},
            {"id":"Builtin line number","name":"Line number","properties":{"size":19}}
        ]});
    assert!(document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap().is_empty());
    let export = export_style_sheet(&document).unwrap();
    let chrome_flags = VIEM_STYLE_DEFINITION_LINE_NUMBER | VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER;
    let gutters = export.definitions.iter().filter(|d| d.flags & VIEM_STYLE_DEFINITION_LINE_NUMBER != 0).collect::<Vec<_>>();
    assert_eq!(gutters.len(), 2);
    for definition in &export.definitions {
        let id = style_arena_text(&export.strings, definition.stable_id);
        let gutter = definition.namespace == VIEM_STYLE_NAMESPACE_BLOCK
            && matches!(id, "Builtin line number" | "Builtin current line number");
        assert_eq!(definition.flags & VIEM_STYLE_DEFINITION_LINE_NUMBER != 0, gutter, "{} {id}", definition.namespace);
        if gutter {
            assert_ne!(definition.flags & VIEM_STYLE_DEFINITION_INTERNAL, 0);
            assert_eq!(definition.capabilities, VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS);
            assert_eq!(definition.flags & VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER != 0, id == "Builtin current line number");
            assert_eq!(style_arena_text(&export.strings, definition.display_name),
                if id == "Builtin current line number" { "Current line number" } else { "Line number" });
        } else if matches!(id, "Heading1" | "Code" | "Line number" | "Builtin line number") {
            assert_eq!(definition.flags & (chrome_flags | VIEM_STYLE_DEFINITION_INTERNAL), 0);
            assert_ne!(definition.capabilities & VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME, 0);
        }
    }
    for (namespace, id) in [(StyleNamespace::Block, "Heading1"), (StyleNamespace::Character, "Code")] {
        let definition = export.definitions.iter().find(|d| d.namespace == match namespace {
            StyleNamespace::Block => VIEM_STYLE_NAMESPACE_BLOCK, StyleNamespace::Character => VIEM_STYLE_NAMESPACE_CHARACTER
        } && style_arena_text(&export.strings, d.stable_id) == id).unwrap();
        assert_ne!(definition.capabilities & VIEM_STYLE_CAPABILITY_ASSIGN, 0);
        document.apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(), revision: document.revision(), range: 0..5, namespace, style: id.into(),
        }).unwrap();
    }
    let sheet = document.projection().style_sheet();
    let content = sheet.resolve_paragraph_style(&sheet.base_paragraph, &"Heading1".into(), Some(&"Code".into()),
        &crate::document::BlockProperties::default(), &crate::document::CharacterProperties::default()).unwrap();
    assert_eq!(content.character.size, 17.0);
    assert!(content.character.bold);
    let gutter = sheet.resolve_paragraph_style(&sheet.base_paragraph, &"Builtin line number".into(), None,
        &crate::document::BlockProperties::default(), &crate::document::CharacterProperties::default()).unwrap();
    assert_eq!(gutter.character.size, 37.0);
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"plain");
}

#[test]
fn syntax_named_line_number_keeps_its_content_appearance_and_never_identifies_the_gutter() {
    let source = b"plain";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Code).unwrap();
    // Deliberately reuse even the opaque block IDs in the character namespace.
    let sheet = crate::document::code_style::parse_json(br#"{"version":3,"character_styles":[
        {"id":"Builtin line number","name":"Line number","properties":{"size":19,"bold":true}},
        {"id":"Builtin current line number","name":"Current line number","properties":{"size":21}}
    ]}"#).unwrap();
    assert_eq!(crate::document::code_style::resolve_name(&sheet, "Line number"), Some(&"Builtin line number".into()));
    document.install_code_presentation(std::sync::Arc::new(sheet), &[crate::document::syntax::SyntaxRun {
        range: 0..5, name: crate::document::syntax::SyntaxStyleName("Line number".into()), origin: "test".into(), priority: 0,
    }]);
    let export = export_style_sheet(&document).unwrap();
    let mut syntax_count = 0;
    for definition in &export.definitions {
        let id = style_arena_text(&export.strings, definition.stable_id);
        if definition.namespace == VIEM_STYLE_NAMESPACE_CHARACTER
            && matches!(id, "Builtin line number" | "Builtin current line number") {
            syntax_count += 1;
            assert_eq!(definition.flags & (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER
                | VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER), 0);
            assert_ne!(definition.capabilities & VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME, 0);
        }
    }
    assert_eq!(syntax_count, 2);
    let sheet = document.projection().style_sheet();
    let content = sheet.resolve_paragraph_style(&sheet.base_paragraph, &sheet.base_paragraph,
        Some(&"Builtin line number".into()), &crate::document::BlockProperties::default(),
        &crate::document::CharacterProperties::default()).unwrap();
    assert_eq!(content.character.size, 19.0);
    assert!(content.character.bold);
    let gutter = sheet.resolve_paragraph_style(&sheet.base_paragraph, &"Builtin line number".into(), None,
        &crate::document::BlockProperties::default(), &crate::document::CharacterProperties::default()).unwrap();

    assert_eq!(gutter.character.size, 11.0);
    let current_gutter = sheet.resolve_paragraph_style(&sheet.base_paragraph, &"Builtin current line number".into(), None,
        &crate::document::BlockProperties::default(), &crate::document::CharacterProperties::default()).unwrap();
    assert_eq!(current_gutter.character.size, 11.0);
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.revision().0, 0);
}
