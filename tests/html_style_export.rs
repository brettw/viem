use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn set_export(document: &mut Document, enabled: bool) -> CommittedModelTransaction {
    document
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: document.id(),
            revision: document.revision(),
            enabled,
        })
        .unwrap()
}

fn persisted(document: &mut Document, intent: PersistedStyleIntent) -> CommittedModelTransaction {
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(intent),
        ))
        .unwrap()
}

fn exact_patches(before: &[u8], after: &[u8], transaction: &CommittedModelTransaction) {
    let mut result = before.to_vec();
    for patch in transaction.summary().source_patches().iter().rev() {
        result.splice(patch.range().clone(), patch.replacement().iter().copied());
    }
    assert_eq!(result, after);
}

#[test]
fn native_style_export_is_optional_minimal_and_exactly_undoable() {
    let original = "<!doctype html><html><head><meta charset='utf-8'><style>.keep { color: red }</style></head><body><p>Words</p><!--opaque--></body></html>";
    let mut document = html(original);
    assert!(!document.include_style_definitions_in_file());
    let enabled = set_export(&mut document, true);
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains("data-viem-version=\"2\""), "{saved}");
    assert!(
        saved.contains("p {\n  font-family: 'SF Pro';\n  font-size: 14pt;\n  margin-block-start: 7pt;\n  margin-block-end: 7pt;\n}"),
        "{saved}"
    );
    for omitted in [
        "text-indent:",
        "vertical-align:",
        "letter-spacing:",
        "--viem-prop-",
    ] {
        assert!(!saved.contains(omitted), "unexpected {omitted}: {saved}");
    }
    assert!(
        saved.contains("<meta charset='utf-8'><style id=\"viem-styles\""),
        "{saved}"
    );
    assert!(saved.contains("<style>.keep { color: red }</style>"));
    assert!(saved.ends_with("<body><p>Words</p><!--opaque--></body></html>"));
    exact_patches(original.as_bytes(), saved.as_bytes(), &enabled);
    let reopened = html(&saved);
    assert_eq!(reopened.text(), document.text());
    assert!(reopened.include_style_definitions_in_file());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(!document.include_style_definitions_in_file());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved.as_bytes());
    assert!(document.include_style_definitions_in_file());
    let disabled = set_export(&mut document, false);
    assert_eq!(document.source_bytes(), original.as_bytes());
    exact_patches(saved.as_bytes(), original.as_bytes(), &disabled);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), saved.as_bytes());
}

#[test]
fn saved_defaults_stay_out_of_html_until_enabled_then_reopen_without_settings() {
    let original = "<p>Words</p><!--keep--><ul><li>Item</li></ul>";
    let mut document = html(original);
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["character"]["size"] = 21.into();
            style["character"]["font_families"] = serde_json::json!(["Georgia"]);
        }
    }
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(!document.include_style_definitions_in_file());
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        21.0
    );
    set_export(&mut document, true);
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains("font-family: 'Georgia';"), "{saved}");
    assert!(saved.contains("font-size: 21pt;"), "{saved}");
    assert!(!saved.contains("--viem-prop-character-size"), "{saved}");
    let reopened = html(&saved);
    let style =
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false).unwrap();
    assert_eq!(style.size, 21.0);
    assert_eq!(style.font_families, vec!["Georgia".to_owned()]);
    set_export(&mut document, false);
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap()
            .size,
        21.0
    );
}

#[test]
fn custom_styles_and_direct_formatting_persist_without_exporting_native_ancestors() {
    let original = "<p data-keep='yes'>Words</p><!--keep-->";
    let mut document = html(original);
    persisted(
        &mut document,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit: StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: "Accent".into(),
                    based_on: None,
                    properties: CharacterProperties {
                        size: Some(19.0),
                        ..Default::default()
                    },
                },
                metadata: StyleDefinitionMetadata {
                    display_name: "Accent".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            },
        },
    );
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    persisted(
        &mut document,
        PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: "Accent".into(),
        },
    );
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    persisted(
        &mut document,
        PersistedStyleIntent::SetDirectCharacterProperties {
            range,
            properties: CharacterProperties {
                bold: Some(true),
                ..Default::default()
            },
        },
    );
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(!document.include_style_definitions_in_file());
    assert!(!saved.contains("body {"), "{saved}");
    assert!(!saved.contains("p {"), "{saved}");
    assert!(!saved.contains("--viem-prop-character-size"), "{saved}");
    assert!(saved.contains("font-size: 19pt;"), "{saved}");
    assert!(saved.ends_with("</p><!--keep-->"), "{saved}");
    let reopened = html(&saved);
    assert!(!reopened.include_style_definitions_in_file());
    assert_eq!(reopened.text(), "Words");
    let style =
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false).unwrap();
    assert_eq!(style.size, 19.0);
    assert!(style.bold);
}

#[test]
fn toggling_export_and_deleting_custom_style_preserve_unrecognized_owned_css() {
    let original = "<p data-id='keep'>Words</p><!--keep-->";
    let mut seed = html(original);
    persisted(
        &mut seed,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit: StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: "Custom".into(),
                    based_on: None,
                    properties: CharacterProperties {
                        size: Some(23.0),
                        ..Default::default()
                    },
                },
                metadata: StyleDefinitionMetadata {
                    display_name: "Custom".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            },
        },
    );
    let source = String::from_utf8(seed.source_bytes()).unwrap();
    let opaque =
        "/* opaque exact bytes */\n.unknown { color: var(--keep); strange: 'untouched'; }\n";
    let source = source.replacen("</style>", &format!("{opaque}</style>"), 1);
    let mut document = html(&source);
    assert!(!document.include_style_definitions_in_file());
    set_export(&mut document, true);
    set_export(&mut document, false);
    assert_eq!(document.source_bytes(), source.as_bytes());
    let deleted = persisted(
        &mut document,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit: StyleDefinitionEdit::DeleteCharacter("Custom".into()),
        },
    );
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains(opaque), "{saved}");
    assert!(!saved.contains("font-size:"), "{saved}");
    assert!(saved.ends_with(original), "{saved}");
    exact_patches(source.as_bytes(), saved.as_bytes(), &deleted);
    assert!(!html(&saved).include_style_definitions_in_file());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved.as_bytes());
}

#[test]
fn an_empty_explicit_export_marker_reopens_enabled_and_toggle_off_removes_only_live_marker() {
    let marker = "<style id=\"viem-styles\" data-viem-version=\"2\"></style>";
    let retained = format!("<template>{marker}</template><p>Words</p><!--keep-->");
    let original = format!("{marker}{retained}");
    let mut document = html(&original);
    assert!(document.include_style_definitions_in_file());
    assert_eq!(document.source_bytes(), original.as_bytes());
    set_export(&mut document, false);
    assert_eq!(document.source_bytes(), retained.as_bytes());
    assert!(!document.include_style_definitions_in_file());
    assert!(!html(&retained).include_style_definitions_in_file());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.include_style_definitions_in_file());
}
