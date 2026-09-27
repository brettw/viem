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
fn percentage_markdown_character_style_retains_generated_parent_without_saved_defaults() {
    for original in [
        b"Text".as_slice(),
        b"`Text`".as_slice(),
    ] {
        let mut document =
            Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
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
        let (source, format, heading_id, code_id) = if persisted {
            (br"{\rtf1{\stylesheet{\s0 Base;}{\s1\sbasedon0 Heading;}{\*\cs1 Code;}}\s1{\cs1 Text}}".as_slice(), Format::Rtf, "RtfP1", "RtfC1")
        } else { (b"# `Text`".as_slice(), Format::Markdown, "Heading1", "Code") };
        let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&heading_id.into())
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(3e38));
        if persisted {
            // RTF's finite half-point source grammar rejects this declaration
            // before serialization; configuration-only Markdown can reach the
            // relative-size overflow guard exercised below.
            let source = document.source_bytes();
            let history = document.history_status();
            assert!(document.apply_style_request(StyleModelRequest::new(
                document.id(), document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                    origin: StyleDefinitionOrigin::SourceBacked,
                    edit: StyleDefinitionEdit::UpdateBlock(heading),
                }),
            )).is_err());
            assert_eq!(document.source_bytes(), source);
            assert_eq!(document.history_status(), history);
            continue;
        }
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
        let mut code = document
            .projection()
            .style_sheet()
            .character_style(&code_id.into())
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
    for _ in [()] {
        let mut document = Document::from_bytes(
            br"{\rtf1{\stylesheet{\s0 Base;}{\s1\sbasedon0 Heading;}{\*\cs1 Code;}}\s1{\cs1{\fs24 Text}}}".to_vec(),
            Encoding::Utf8,
            Format::Rtf,
        )
        .unwrap();
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&"RtfP1".into())
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(6000.0));
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
        let mut code = document
            .projection()
            .style_sheet()
            .character_style(&"RtfC1".into())
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
