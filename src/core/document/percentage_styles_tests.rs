use super::*;

fn edit(document: &mut Document, change: StyleDefinitionEdit) {
    let sheet = document.projection().style_sheet();
    let metadata = if change.is_block() {
        sheet.block_style_metadata(change.style_id())
    } else {
        sheet.character_style_metadata(change.style_id())
    }
    .unwrap();
    let intent = match metadata.origin {
        StyleDefinitionOrigin::SourceBacked => {
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: change,
            })
        }
        StyleDefinitionOrigin::GeneratedConfiguration => {
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(change))
        }
        StyleDefinitionOrigin::SyntheticReadOnly => panic!("fixture style is read-only"),
    };
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            intent,
        ))
        .unwrap();
}

fn sizes(document: &Document) -> (f32, f32) {
    (
        rich_text::resolved_character_at(document.projection(), 0)
            .unwrap()
            .size,
        rich_text::resolved_character_at(document.projection(), 8)
            .unwrap()
            .size,
    )
}

#[test]
fn percentage_html_styles_edit_save_reopen_and_undo_preserve_units_and_dynamic_sizes() {
    let original = b"<!--keep--><h1><code>Heading</code></h1><p><code>Body</code></p>";
    let mut doc = Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    doc.apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
        document: doc.id(),
        revision: doc.revision(),
        enabled: true,
    })
    .unwrap();
    let mut base = doc
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    base.character.size = Some(FontSize::Points(12.0));
    edit(&mut doc, StyleDefinitionEdit::UpdateBlock(base));
    let mut heading = doc
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .unwrap()
        .clone();
    heading.character.size = Some(FontSize::Percentage(200));
    edit(&mut doc, StyleDefinitionEdit::UpdateBlock(heading));
    let mut code = doc
        .projection()
        .style_sheet()
        .character_style(&"Code".into())
        .unwrap()
        .clone();
    code.properties.size = Some(FontSize::Percentage(90));
    edit(&mut doc, StyleDefinitionEdit::UpdateCharacter(code));
    assert_eq!(doc.text(), "Heading\nBody");
    assert_eq!(sizes(&doc), (21.6, 10.8));
    let saved = doc.source_bytes();
    let source = std::str::from_utf8(&saved).unwrap();
    assert!(source.contains("font-size: 90%"), "{source}");
    assert!(source.contains("font-size: 24pt"), "{source}");
    assert!(
        source.contains("--viem-prop-character-size: \"200%\""),
        "{source}"
    );
    assert!(source.ends_with(std::str::from_utf8(original).unwrap()));
    let mut reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(sizes(&reopened), (21.6, 10.8));
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .block_style(&"Heading1".into())
            .unwrap()
            .character
            .size,
        Some(FontSize::Percentage(200))
    );
    let mut base = reopened
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    base.character.size = Some(FontSize::Points(20.0));
    edit(&mut reopened, StyleDefinitionEdit::UpdateBlock(base));
    assert_eq!(sizes(&reopened), (36.0, 18.0));
    let changed = reopened.source_bytes();
    assert!(std::str::from_utf8(&changed)
        .unwrap()
        .contains("font-size: 40pt"));
    assert!(reopened.undo());
    assert_eq!(reopened.source_bytes(), saved);
    assert_eq!(sizes(&reopened), (21.6, 10.8));
    assert!(reopened.redo());
    assert_eq!(reopened.source_bytes(), changed);
    assert_eq!(
        sizes(&Document::from_bytes(changed, Encoding::Utf8, Format::Html).unwrap()),
        (36.0, 18.0)
    );
}

#[test]
fn percentage_rtf_styles_edit_save_reopen_and_undo_preserve_relative_sizes() {
    let original = br"{\rtf1\ansi{\stylesheet{\s0\fs24 Base;}{\s1\sbasedon0 Heading;}{\*\cs1 Code;}}\s1{\cs1 Heading}\par\s0{\cs1 Body}}";
    let mut doc = Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let mut heading = doc
        .projection()
        .style_sheet()
        .block_style(&"RtfP1".into())
        .unwrap()
        .clone();
    heading.character.size = Some(FontSize::Percentage(200));
    edit(&mut doc, StyleDefinitionEdit::UpdateBlock(heading));
    let mut code = doc
        .projection()
        .style_sheet()
        .character_style(&"RtfC1".into())
        .unwrap()
        .clone();
    code.properties.size = Some(FontSize::Percentage(90));
    edit(&mut doc, StyleDefinitionEdit::UpdateCharacter(code));
    assert_eq!(doc.text(), "Heading\nBody");
    assert_eq!(sizes(&doc), (21.6, 10.8));
    let saved = doc.source_bytes();
    let source = std::str::from_utf8(&saved).unwrap();
    assert!(source.contains("\\fs48\\viemsizepercent200"), "{source}");
    assert!(source.contains("\\viemsizepercent90"), "{source}");
    let mut reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(sizes(&reopened), (21.6, 10.8));
    let mut base = reopened
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    base.character.size = Some(FontSize::Points(20.0));
    edit(&mut reopened, StyleDefinitionEdit::UpdateBlock(base));
    assert_eq!(sizes(&reopened), (36.0, 18.0));
    let changed = reopened.source_bytes();
    assert!(std::str::from_utf8(&changed)
        .unwrap()
        .contains("\\fs80\\viemsizepercent200"));
    assert!(reopened.undo());
    assert_eq!(reopened.source_bytes(), saved);
    assert!(reopened.redo());
    assert_eq!(reopened.source_bytes(), changed);
    assert_eq!(
        sizes(&Document::from_bytes(changed, Encoding::Utf8, Format::Rtf).unwrap()),
        (36.0, 18.0)
    );
}

#[test]
fn percentage_font_size_is_rejected_for_direct_formatting_without_mutation() {
    for (format, bytes) in [
        (Format::Html, b"<p>Text</p>".as_slice()),
        (Format::Rtf, br"{\rtf1 Text}".as_slice()),
    ] {
        let mut document = Document::from_bytes(bytes.to_vec(), Encoding::Utf8, format).unwrap();
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(4).unwrap(),
        )
        .unwrap();
        let result = document.apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                range,
                properties: CharacterProperties {
                    size: Some(FontSize::Percentage(90)),
                    ..Default::default()
                },
            }),
        ));
        assert!(result.is_err());
        assert_eq!(document.source_bytes(), bytes);
        assert_eq!(document.text(), "Text");
    }
}

#[test]
fn percentage_html_native_character_style_retains_generated_parent_without_saved_defaults() {
    for original in [
        b"<p>Text</p>".as_slice(),
        b"<p><code>Text</code></p>".as_slice(),
    ] {
        let mut document =
            Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Html).unwrap();
        assert!(!document.include_style_definitions_in_file());
        let mut base = document
            .projection()
            .style_sheet()
            .block_style(&"Paragraph".into())
            .unwrap()
            .clone();
        base.character.size = Some(FontSize::Points(20.0));
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(base));
        let mut link = document
            .projection()
            .style_sheet()
            .character_style(&"Link".into())
            .unwrap()
            .clone();
        link.properties.size = Some(FontSize::Points(100.0));
        edit(&mut document, StyleDefinitionEdit::UpdateCharacter(link));
        let mut code = document
            .projection()
            .style_sheet()
            .character_style(&"Code".into())
            .unwrap()
            .clone();
        code.based_on = Some("Link".into());
        edit(
            &mut document,
            StyleDefinitionEdit::UpdateCharacter(code.clone()),
        );
        code.properties.size = Some(FontSize::Percentage(90));
        edit(&mut document, StyleDefinitionEdit::UpdateCharacter(code));
        let sheet = document.projection().style_sheet();
        assert_eq!(
            sheet
                .character_style(&"Link".into())
                .unwrap()
                .properties
                .size,
            Some(FontSize::Points(100.0))
        );
        assert_eq!(
            sheet
                .resolve_paragraph_style(
                    &sheet.base_paragraph,
                    &sheet.base_paragraph,
                    Some(&"Code".into()),
                    &BlockProperties::default(),
                    &CharacterProperties::default()
                )
                .unwrap()
                .character
                .size,
            18.0
        );
        let mut base = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        base.character.size = Some(FontSize::Points(12.0));
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(base));
        let sheet = document.projection().style_sheet();
        assert_eq!(
            sheet
                .resolve_paragraph_style(
                    &sheet.base_paragraph,
                    &sheet.base_paragraph,
                    Some(&"Code".into()),
                    &BlockProperties::default(),
                    &CharacterProperties::default()
                )
                .unwrap()
                .character
                .size,
            10.8
        );
        let expected_sheet = sheet.clone();
        assert_eq!(document.source_bytes(), original);
        assert!(document.undo());
        assert!(document.redo());
        assert_eq!(document.projection().style_sheet(), &expected_sheet);
    }
}

#[test]
fn percentage_character_context_overflow_rejects_configuration_and_source_edits_atomically() {
    for persisted in [false, true] {
        let mut document = Document::from_bytes(
            b"<h1><code>Text</code></h1>".to_vec(),
            Encoding::Utf8,
            Format::Html,
        )
        .unwrap();
        if persisted {
            document
                .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
                    document: document.id(),
                    revision: document.revision(),
                    enabled: true,
                })
                .unwrap();
        }
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&"Heading1".into())
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(3e38));
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
        let mut code = document
            .projection()
            .style_sheet()
            .character_style(&"Code".into())
            .unwrap()
            .clone();
        code.properties.size = Some(FontSize::Percentage(10));
        edit(
            &mut document,
            StyleDefinitionEdit::UpdateCharacter(code.clone()),
        );
        assert_eq!(
            rich_text::resolved_character_at(document.projection(), 0)
                .unwrap()
                .size,
            FontSize::Percentage(10).resolve(3e38)
        );
        let before_source = document.source_bytes();
        let before_sheet = document.projection().style_sheet().clone();
        let before_revision = document.revision();
        let before_history = document.history_status();
        code.properties.size = Some(FontSize::Percentage(1000));
        let mut invalid_sheet = before_sheet.clone();
        let change = StyleDefinitionEdit::UpdateCharacter(code.clone());
        let revision = StyleSheetRevision(invalid_sheet.revision.0 + 1);
        if persisted {
            invalid_sheet
                .apply_source_edit(&change, revision, true)
                .unwrap();
        } else {
            invalid_sheet
                .apply_configuration_edit(&change, revision, true)
                .unwrap();
        }
        assert!(rich_text::resolved_character_at_with_style_context(document.projection(), 0,
            &invalid_sheet, document.projection().document_style()).is_none(),
            "The actual rendered interval overflows even though the Base Paragraph preview is finite");
        let intent = if persisted {
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateCharacter(code),
            })
        } else {
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                StyleDefinitionEdit::UpdateCharacter(code),
            ))
        };
        assert!(document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                intent
            ))
            .is_err());
        assert_eq!(document.source_bytes(), before_source);
        assert_eq!(document.projection().style_sheet(), &before_sheet);
        assert_eq!(document.revision(), before_revision);
        assert_eq!(document.history_status().current, before_history.current);
        assert_eq!(
            document.history_status().node_count,
            before_history.node_count
        );
    }
}

#[test]
fn percentage_context_validation_respects_absolute_inline_overrides() {
    for persisted in [false, true] {
        let mut document = Document::from_bytes(
            b"<h1><code><span style='font-size:12pt'>Text</span></code></h1>".to_vec(),
            Encoding::Utf8,
            Format::Html,
        )
        .unwrap();
        if persisted {
            document
                .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
                    document: document.id(),
                    revision: document.revision(),
                    enabled: true,
                })
                .unwrap();
        }
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&"Heading1".into())
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(3e38));
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
        let mut code = document
            .projection()
            .style_sheet()
            .character_style(&"Code".into())
            .unwrap()
            .clone();
        code.properties.size = Some(FontSize::Percentage(1000));
        edit(&mut document, StyleDefinitionEdit::UpdateCharacter(code));
        assert_eq!(
            rich_text::resolved_character_at(document.projection(), 0)
                .unwrap()
                .size,
            12.0
        );
    }
}
