use viem_core::document::*;

#[test]
fn base_paragraph_rejects_next_style_edits_without_publishing_changes() {
    for (format, source) in [
        (Format::Markdown, "Text"),

    ] {
        for next in ["Heading1", "Paragraph"] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let before = document.projection().style_sheet().clone();
            let history = document.history_status();
            let revision = document.revision();
            let mut root = before.block_style(&before.base_paragraph).unwrap().clone();
            root.next_paragraph_style = Some(next.into());
            let edit = StyleDefinitionEdit::UpdateBlock(root);
            let intent = {
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit))
            };
            assert!(document.apply_style_request(StyleModelRequest::new(
                document.id(), revision, intent,
            )).is_err(), "{format:?}: {next}");
            assert_eq!(document.revision(), revision);
            assert_eq!(document.projection().style_sheet(), &before);
            assert_eq!(document.history_status(), history);
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn user_defaults_cannot_change_base_paragraph_next_style() {
    let mut document = Document::new("Text");
    let before = document.export_style_defaults().unwrap();
    let mut defaults: serde_json::Value = serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let root = defaults["block_styles"].as_array_mut().unwrap().iter_mut()
        .find(|style| style["id"] == "Paragraph").unwrap();
    root["next_paragraph_style"] = "Heading1".into();
    let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap();
    assert!(diagnostics.iter().any(|message| message.contains("Paragraph") && message.contains("next_paragraph_style")));
    assert_eq!(document.export_style_defaults().unwrap(), before);
    assert_eq!(document.source_bytes(), b"Text");
}
