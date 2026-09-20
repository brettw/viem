use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}
fn include(document: &mut Document, enabled: bool) -> CommittedModelTransaction {
    document
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: document.id(),
            revision: document.revision(),
            enabled,
        })
        .unwrap()
}
fn update_paragraph(document: &mut Document, size: f32) {
    let mut paragraph = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    paragraph.character.size = Some(size);
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(paragraph),
            }),
        ))
        .unwrap();
}

#[test]
fn builtin_edits_without_export_are_configuration_and_survive_typing_and_history() {
    let source = "<p data-keep='x'>A</p><!--opaque-->";
    let mut document = html(source);
    assert!(!document.include_style_definitions_in_file());
    update_paragraph(&mut document, 23.);
    assert_eq!(document.source_bytes(), source.as_bytes());
    document.insert(1, "B").unwrap();
    assert_eq!(
        document.source_bytes(),
        b"<p data-keep='x'>AB</p><!--opaque-->"
    );
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        23.
    );
    assert!(document.undo());
    assert!(document.undo());
    assert_ne!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        23.
    );
    assert!(document.redo());
    assert!(document.redo());
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        23.
    );
}

#[test]
fn export_toggle_preserves_content_customized_appearance_and_exact_history() {
    let source = "<p data-keep='x'>A</p><!--opaque-->";
    let mut document = html(source);
    update_paragraph(&mut document, 23.);
    let transaction = include(&mut document, true);
    let exported = document.source_bytes();
    let markup = String::from_utf8(exported.clone()).unwrap();
    assert!(markup.ends_with(source));
    let family = if DEFAULT_FONT_FAMILY == "system-ui" { "system-ui".to_owned() }
        else { format!("'{DEFAULT_FONT_FAMILY}'") };
    assert!(markup.contains(&format!("p {{\n  font-family: {family};\n  font-size: 23pt;")), "{markup}");
    assert!(!markup.contains("text-indent:"), "{markup}");
    assert!(!markup.contains("vertical-align:"), "{markup}");
    assert!(!markup.contains("letter-spacing:"), "{markup}");
    assert!(!transaction.summary().source_patches().is_empty());
    let reopened = html(&markup);
    assert!(reopened.include_style_definitions_in_file());
    assert_eq!(reopened.text(), "A");
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false)
            .unwrap()
            .size,
        23.
    );
    include(&mut document, false);
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        23.
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), exported);
    assert!(document.include_style_definitions_in_file());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(!document.include_style_definitions_in_file());
}

#[test]
fn exporting_saved_defaults_preserves_appearance_without_settings_on_reopen() {
    let mut settings: serde_json::Value =
        serde_json::from_slice(&html("").export_style_defaults().unwrap()).unwrap();
    for style in settings["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["character"]["size"] = 31.into();
            style["character"]["font_families"] = serde_json::json!(["Georgia"]);
        }
    }
    let mut document = html("<p>A</p><ul><li>B</li></ul>");
    document
        .initialize_style_defaults(&serde_json::to_vec(&settings).unwrap())
        .unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        31.
    );
    include(&mut document, true);
    let reopened = html(&String::from_utf8(document.source_bytes()).unwrap());
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false)
            .unwrap()
            .size,
        31.
    );
    assert_eq!(reopened.text(), document.text());
}

#[test]
fn disabling_export_keeps_relative_source_formatting_at_the_configured_size() {
    let mut document = html("<p><span style='vertical-align:super'>A</span></p>");
    update_paragraph(&mut document, 30.);
    for enabled in [true, false] {
        include(&mut document, enabled);
        let resolved =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
        assert_eq!(resolved.size, 30.);
        assert_eq!(resolved.baseline_shift, 10.);
    }
    document.insert(1, "B").unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .baseline_shift,
        10.
    );
}

#[test]
fn deleting_a_native_style_with_export_off_rebases_assignments_and_retains_configuration() {
    let mut document = html("<h1>A</h1><p>B</p><!--keep-->");
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::DeleteBlock("Heading1".into()),
            }),
        ))
        .unwrap();
    assert!(document
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .is_none());
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(!source.contains("<style"), "{source}");
    assert_eq!(document.text(), "A\nB");
    document.insert(1, "C").unwrap();
    assert!(document
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .is_none());
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"<h1>A</h1><p>B</p><!--keep-->");
}
