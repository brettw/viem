use viem_core::document::*;
use viem_core::layout::DocumentLayoutStyles;

fn open(source: &str) -> Document {
    Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap()
}

fn edit_paragraph_size(document: &mut Document, size: f32) {
    let mut style = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    style.character.size = Some((size).into());
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(style),
            }),
        ))
        .unwrap();
}

fn size_at_word(document: &Document) -> f32 {
    let at = document.text().find("Words").unwrap();
    DocumentLayoutStyles::semantic_character_at(document.projection(), at, false)
        .unwrap()
        .size
}

fn underline_word(document: &mut Document) {
    let at = document.text().find("Words").unwrap();
    let range = TextRange::new(
        document.text_point(at).unwrap(),
        document.text_point(at + 5).unwrap(),
    )
    .unwrap();
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                range,
                properties: CharacterProperties {
                    underline: Some(true),
                    ..Default::default()
                },
            }),
        ))
        .unwrap();
}

#[test]
fn native_definition_without_source_patches_commits_and_survives_source_editing() {
    let original = "<p data-keep='yes'>Words</p><!--keep-->";
    let mut document = open(original);
    let before_size = size_at_word(&document);
    let revision = document.revision();
    edit_paragraph_size(&mut document, 23.);
    assert!(document.revision() > revision);
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert_eq!(size_at_word(&document), 23.);
    assert!(!document.include_style_definitions_in_file());
    let size_revision = document.projection().style_sheet().revision;

    underline_word(&mut document);
    assert_eq!(size_at_word(&document), 23.);
    let styled = document.source_bytes();
    let styled_text = String::from_utf8(styled.clone()).unwrap();
    assert!(
        styled_text.contains("text-decoration-line: underline"),
        "{styled_text}"
    );
    assert!(!styled_text.contains("<style"), "{styled_text}");
    assert!(styled_text.contains("data-keep='yes'") && styled_text.contains("<!--keep-->"));
    let at = document.text().find("Words").unwrap() + 5;
    document.insert(at, "!").unwrap();
    assert_eq!(size_at_word(&document), 23.);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), styled);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert_eq!(document.projection().style_sheet().revision, size_revision);
    assert_eq!(size_at_word(&document), 23.);
    assert!(document.undo());
    assert_eq!(size_at_word(&document), before_size);
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.redo());
    assert_eq!(size_at_word(&document), 23.);
}

#[test]
fn saved_defaults_survive_source_style_translation_without_becoming_css() {
    let mut template = open("<p>Words</p>");
    edit_paragraph_size(&mut template, 29.);
    let defaults = template.export_style_defaults().unwrap();
    let original = "<p data-keep='yes'>Words</p><!--keep-->";
    let mut document = open(original);
    document.initialize_style_defaults(&defaults).unwrap();
    assert_eq!(size_at_word(&document), 29.);
    underline_word(&mut document);
    assert_eq!(size_at_word(&document), 29.);
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(
        !saved.contains("font-size") && !saved.contains("<style"),
        "{saved}"
    );
    let at = document.text().find("Words").unwrap();
    document.insert(at + 5, "!").unwrap();
    assert_eq!(size_at_word(&document), 29.);
}

#[test]
fn enabled_source_style_translation_writes_native_css_and_reopens() {
    let mut document = open("<p>Words</p><!--keep-->");
    document
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: document.id(),
            revision: document.revision(),
            enabled: true,
        })
        .unwrap();
    assert!(document.include_style_definitions_in_file());
    edit_paragraph_size(&mut document, 23.);
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(
        saved.contains("p {") && saved.contains("font-size: 23pt"),
        "{saved}"
    );
    assert!(!saved.contains("class="), "{saved}");
    assert_eq!(size_at_word(&document), 23.);
    let reopened = open(&saved);
    assert!(reopened.include_style_definitions_in_file());
    assert_eq!(size_at_word(&reopened), 23.);
    underline_word(&mut document);
    assert_eq!(size_at_word(&document), 23.);
    assert!(document.include_style_definitions_in_file());
}

#[test]
fn source_script_position_retains_configured_paragraph_size() {
    for (value, expected) in [("super", ScriptPosition::Superscript), ("sub", ScriptPosition::Subscript)] {
        let mut document = open("<p>Words</p><!--keep-->");
        edit_paragraph_size(&mut document, 24.);
        let at = document.text().find("Words").unwrap();
        let markup = format!("<span style='vertical-align:{value}'>Words</span>");
        document.replace(at..at + 5, &markup).unwrap();
        let at = document.text().find("Words").unwrap();
        let resolved =
            DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
        assert_eq!(resolved.size, 24.);
        assert_eq!(resolved.script_position, expected);
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(
            !saved.contains("font-size") && !saved.contains("<style"),
            "{saved}"
        );
    }
}

#[test]
fn saved_defaults_apply_to_existing_source_script_positions_on_open() {
    let mut template = open("<p>Words</p>");
    edit_paragraph_size(&mut template, 24.);
    let defaults = template.export_style_defaults().unwrap();
    let source = "<p><span style='vertical-align:super'>Words</span></p><!--keep-->";
    let mut document = open(source);
    document.initialize_style_defaults(&defaults).unwrap();
    let at = document.text().find("Words").unwrap();
    let resolved =
        DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
    assert_eq!(resolved.size, 24.);
    assert_eq!(resolved.script_position, ScriptPosition::Superscript);
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(!document.history_status().is_dirty);
    assert!(!document.history_status().can_undo);
}
