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
    for _ in [()] {
        let (source, format, heading_id, code_id) = (b"# `Text`".as_slice(), Format::Markdown, "Heading1", "Code");
        let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
        let mut heading = document
            .projection()
            .style_sheet()
            .block_style(&heading_id.into())
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(3e38));

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
        {
            invalid_sheet
                .apply_configuration_edit(&change, revision, true)
                .unwrap();
        }
        assert!(rich_text::resolved_character_at_with_style_context(document.projection(), 0,
            &invalid_sheet, document.projection().document_style()).is_none(),
            "The actual rendered interval overflows even though the Base Paragraph preview is finite");
        let intent = {
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
