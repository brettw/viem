use viem_core::document::*;

fn documents() -> Vec<Document> {
    [
        (Format::PlainText, "one two"),
        (Format::Markdown, "one **two**"),
        (Format::MarkdownSource, "one **two**"),
        (Format::Html, "<p>one <b>two</b></p>"),
        (Format::HtmlSource, "<p>one <b>two</b></p>"),
        (Format::Rtf, r"{\rtf1 one {\b two}}"),
        (Format::Code, "one two"),
    ]
    .into_iter()
    .map(|(format, source)| {
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
    })
    .collect()
}

fn configure(document: &mut Document, properties: CharacterProperties) {
    let mut style = document
        .projection()
        .style_sheet()
        .character_style(&StyleId::incremental_match())
        .unwrap()
        .clone();
    style.properties = properties;
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                StyleDefinitionEdit::UpdateCharacter(style),
            )),
        ))
        .unwrap();
}

#[test]
fn built_in_is_internal_in_every_format_and_only_declares_a_background() {
    for document in documents() {
        let sheet = document.projection().style_sheet();
        let id = StyleId::incremental_match();
        assert!(id.is_internal());
        let style = sheet.character_style(&id).unwrap();
        assert_eq!(style.based_on, None);
        let metadata = sheet.character_style_metadata(&id).unwrap();
        assert_eq!(metadata.display_name, "Incremental match");
        assert_eq!(
            metadata.origin,
            StyleDefinitionOrigin::GeneratedConfiguration
        );
        let background = style.properties.background.unwrap();
        assert_eq!(background.red, 1.0);
        assert!(background.green > 0.8 && background.blue < 0.1 && background.alpha > 0.0);
        assert_eq!(
            style.properties,
            CharacterProperties {
                background: Some(background),
                ..Default::default()
            }
        );
    }
    assert!(code_style::default_sheet()
        .character_style(&StyleId::incremental_match())
        .is_some());
}

#[test]
fn default_overlay_preserves_the_complete_underlying_character_style() {
    let sheet = StyleSheet::default();
    let mut underlying = ResolvedCharacterStyle::default();
    underlying.font_families = vec!["Georgia".into()];
    underlying.size = 27.0;
    underlying.weight = 700;
    underlying.bold = true;
    underlying.slant = FontSlant::Italic;
    underlying.foreground = Color {
        red: 0.2,
        green: 0.4,
        blue: 0.8,
        alpha: 1.0,
    };
    underlying.underline = true;
    underlying.language = Some("fr".into());
    let actual = sheet.overlay_incremental_match(&underlying).unwrap();
    let mut expected = underlying;
    expected.background = sheet.incremental_match_properties().unwrap().background;
    assert_eq!(actual, expected);
}

#[test]
fn customized_search_style_never_enters_exported_html_definitions() {
    let mut document =
        Document::from_bytes(b"<p>Words</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
    configure(
        &mut document,
        CharacterProperties {
            background: Some(Color {
                red: 0.2,
                green: 0.4,
                blue: 0.7,
                alpha: 0.9,
            }),
            size: Some((19.0).into()),
            ..Default::default()
        },
    );
    document
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: document.id(),
            revision: document.revision(),
            enabled: true,
        })
        .unwrap();
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.contains("data-viem-version=\"2\""));
    assert!(!source.contains("Incremental match"));
    let encoded_id = StyleId::incremental_match()
        .0
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert!(!source.contains(&encoded_id));
    let reopened =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.source_bytes(), source.as_bytes());
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .incremental_match_properties()
            .unwrap(),
        StyleSheet::default()
            .incremental_match_properties()
            .unwrap()
    );
}

#[test]
fn explicit_overlay_properties_win_over_direct_named_and_paragraph_styles() {
    let mut document = Document::from_bytes(
        b"<h1><b><span style='font-size:31pt;color:blue'>one</span></b></h1>".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    let color = Color {
        red: 0.7,
        green: 0.1,
        blue: 0.3,
        alpha: 1.0,
    };
    let properties = CharacterProperties {
        size: Some((18.0).into()),
        weight: Some(300),
        foreground: Some(color),
        underline: Some(false),
        ..Default::default()
    };
    let before = document.source_bytes();
    configure(&mut document, properties.clone());
    assert_eq!(document.source_bytes(), before);
    assert!(!document.is_dirty());
    let sheet = document.projection().style_sheet();
    let ordinary = sheet
        .resolve_paragraph_style(
            &sheet.base_paragraph,
            &StyleId::from("Heading1"),
            Some(&StyleId::from("Code")),
            &BlockProperties::default(),
            &CharacterProperties {
                size: Some((31.0).into()),
                underline: Some(true),
                bold: Some(true),
                ..Default::default()
            },
        )
        .unwrap()
        .character;
    let matched = sheet.overlay_incremental_match(&ordinary).unwrap();
    assert_eq!(matched.size, 18.0);
    assert_eq!(matched.weight, 300);
    assert_eq!(matched.foreground, color);
    assert!(!matched.underline);
    assert_eq!(matched.font_families, ordinary.font_families);
    assert_eq!(sheet.incremental_match_properties().unwrap(), properties);
}

#[test]
fn user_defaults_round_trip_and_configuration_edits_never_author_source() {
    let properties = CharacterProperties {
        background: Some(Color {
            red: 0.3,
            green: 0.5,
            blue: 0.9,
            alpha: 0.7,
        }),
        strikethrough: Some(true),
        size: Some((19.0).into()),
        ..Default::default()
    };
    for mut document in documents()
        .into_iter()
        .filter(|document| !document.format().is_code())
    {
        let source = document.source_bytes();
        let format = document.format();
        configure(&mut document, properties.clone());
        let saved = document.export_style_defaults().unwrap();
        assert_eq!(document.source_bytes(), source);
        assert!(!document.is_dirty());
        let mut reopened = Document::from_bytes(source.clone(), Encoding::Utf8, format).unwrap();
        reopened.initialize_style_defaults(&saved).unwrap();
        assert_eq!(reopened.source_bytes(), source);
        assert!(!reopened.is_dirty());
        assert_eq!(
            reopened
                .projection()
                .style_sheet()
                .incremental_match_properties()
                .unwrap(),
            properties
        );
        reopened.insert(0, "X").unwrap();
        assert_eq!(
            reopened
                .projection()
                .style_sheet()
                .incremental_match_properties()
                .unwrap(),
            properties
        );
        assert!(reopened.undo());
        assert_eq!(reopened.source_bytes(), source);
        assert_eq!(
            reopened
                .projection()
                .style_sheet()
                .incremental_match_properties()
                .unwrap(),
            properties
        );
    }
    let mut sheet = code_style::default_sheet();
    let mut style = sheet
        .character_style(&StyleId::incremental_match())
        .unwrap()
        .clone();
    style.properties = properties.clone();
    sheet
        .insert_character_style(
            style,
            StyleDefinitionMetadata::generated("Incremental match"),
        )
        .unwrap();
    let encoded = code_style::export_snapshot(&sheet).unwrap();
    let reopened = code_style::parse_json(&encoded).unwrap();
    assert_eq!(reopened.incremental_match_properties().unwrap(), properties);
}

#[test]
fn internal_overlay_cannot_be_assigned_through_any_editing_entry_point() {
    for mut document in documents() {
        let source = document.source_bytes();
        let revision = document.revision();
        let id = StyleId::incremental_match();
        assert!(document.validate_typing_named_style(&id).is_err());
        assert!(document
            .apply_model_request(ModelRequest::AssignNamedStyle {
                document: document.id(),
                revision,
                range: 0..1,
                namespace: StyleNamespace::Character,
                style: id.clone(),
            })
            .is_err());
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(1).unwrap(),
        )
        .unwrap();
        assert!(document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                revision,
                StyleModelIntent::Persisted(PersistedStyleIntent::AssignCharacterStyle {
                    range,
                    style: id
                }),
            ))
            .is_err());
        assert_eq!(document.revision(), revision);
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn internal_identity_cannot_be_renamed_removed_or_reparented() {
    let mut document = Document::new("one");
    let id = StyleId::incremental_match();
    let mut child = document
        .projection()
        .style_sheet()
        .character_style(&id)
        .unwrap()
        .clone();
    child.based_on = Some(StyleId::from("Code"));
    for edit in [
        StyleDefinitionEdit::DeleteCharacter(id.clone()),
        StyleDefinitionEdit::UpdateCharacter(child),
        StyleDefinitionEdit::UpdateMetadata {
            namespace: StyleNamespace::Character,
            id,
            metadata: StyleDefinitionMetadata::generated("Renamed"),
        },
    ] {
        assert!(document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit)),
            ))
            .is_err());
    }
    assert_eq!(document.source_bytes(), b"one");
    assert!(!document.is_dirty());
}
