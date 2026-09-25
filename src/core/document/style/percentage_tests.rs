use super::*;

fn sample_sheet() -> StyleSheet {
    let mut sheet = StyleSheet::default();
    sheet
        .block_styles
        .get_mut(&sheet.base_paragraph.clone())
        .unwrap()
        .character
        .size = Some(FontSize::Points(12.0));
    sheet
        .block_styles
        .get_mut(&StyleId::from("Heading1"))
        .unwrap()
        .character
        .size = Some(FontSize::Percentage(200));
    sheet
        .character_styles
        .get_mut(&StyleId::from("Code"))
        .unwrap()
        .properties
        .size = Some(FontSize::Percentage(90));
    sheet
}

fn resolved(
    sheet: &StyleSheet,
    paragraph: &str,
    character: Option<&str>,
) -> ResolvedParagraphStyle {
    sheet
        .resolve_paragraph_style(
            &sheet.base_paragraph,
            &paragraph.into(),
            character.map(StyleId::from).as_ref(),
            &BlockProperties::default(),
            &CharacterProperties::default(),
        )
        .unwrap()
}

#[test]
fn percentage_paragraph_and_character_sizes_follow_their_distinct_contexts() {
    let mut sheet = sample_sheet();
    assert_eq!(resolved(&sheet, "Paragraph", None).character.size, 12.0);
    assert_eq!(resolved(&sheet, "Heading1", None).character.size, 24.0);
    assert_eq!(
        resolved(&sheet, "Paragraph", Some("Code")).character.size,
        10.8
    );
    assert_eq!(
        resolved(&sheet, "Heading1", Some("Code")).character.size,
        21.6
    );
    let old = resolved(&sheet, "Heading1", Some("Code"));
    sheet
        .block_styles
        .get_mut(&sheet.base_paragraph.clone())
        .unwrap()
        .character
        .size = Some(FontSize::Points(20.0));
    let new = resolved(&sheet, "Heading1", Some("Code"));
    assert_eq!(new.character.size, 36.0);
    assert_eq!(
        old.character.changed_properties(&new.character),
        BTreeSet::from([StyleProperty::CharacterSize])
    );
    assert_eq!(
        StyleProperty::CharacterSize.invalidation_effect(),
        StyleInvalidationEffect::Shaping
    );
    let traced = sheet
        .resolve_assigned_paragraph_style_with_contributions(
            &DocumentStyleAssignment::new(sheet.base_paragraph.clone()),
            &"Heading1".into(),
            &BlockProperties::default(),
            &CharacterProperties::default(),
            Some(&"Code".into()),
            &CharacterProperties::default(),
        )
        .unwrap();
    assert_eq!(traced.value, new);
    let size = traced.contribution(StyleProperty::CharacterSize).unwrap();
    assert_eq!(
        size.winner,
        StyleContributionOrigin::CharacterStyle("Code".into())
    );
    assert!(size
        .dependencies
        .contains(&StyleDependency::Block(sheet.base_paragraph.clone())));
    assert!(size
        .dependencies
        .contains(&StyleDependency::Block("Heading1".into())));
    assert!(size
        .dependencies
        .contains(&StyleDependency::Character("Code".into())));
}

#[test]
fn percentage_character_inheritance_selects_one_declaration_without_compounding() {
    let mut sheet = sample_sheet();
    for (id, parent, size) in [
        ("Parent", None, Some(FontSize::Points(30.0))),
        ("Child", Some("Parent"), Some(FontSize::Percentage(90))),
        ("Grandchild", Some("Child"), None),
        ("Override", Some("Child"), Some(FontSize::Percentage(50))),
    ] {
        sheet
            .insert_character_style(
                CharacterStyle {
                    id: id.into(),
                    based_on: parent.map(StyleId::from),
                    properties: CharacterProperties {
                        size,
                        ..Default::default()
                    },
                },
                StyleDefinitionMetadata::generated(id),
            )
            .unwrap();
    }
    for name in ["Child", "Grandchild"] {
        assert_eq!(
            resolved(&sheet, "Paragraph", Some(name)).character.size,
            10.8
        );
        assert_eq!(
            resolved(&sheet, "Heading1", Some(name)).character.size,
            21.6
        );
    }
    assert_eq!(
        resolved(&sheet, "Paragraph", Some("Override"))
            .character
            .size,
        6.0
    );
    assert_eq!(
        resolved(&sheet, "Paragraph", Some("Parent")).character.size,
        30.0
    );
    let inherited = sheet
        .named_character_declarations(Some(&"Grandchild".into()))
        .unwrap();
    assert_eq!(inherited.size, Some(FontSize::Percentage(90)));
}

#[test]
fn percentage_paragraph_inheritance_compounds_declared_parent_sizes_once() {
    let mut sheet = sample_sheet();
    for (id, parent, size) in [
        ("Subheading", "Heading1", Some(FontSize::Percentage(150))),
        ("Inherited", "Subheading", None),
    ] {
        sheet
            .insert_block_style(
                BlockStyle {
                    id: id.into(),
                    based_on: Some(parent.into()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size,
                        ..Default::default()
                    },
                    block: BlockProperties::default(),
                },
                StyleDefinitionMetadata::generated(id),
            )
            .unwrap();
    }
    assert_eq!(resolved(&sheet, "Subheading", None).character.size, 36.0);
    assert_eq!(resolved(&sheet, "Inherited", None).character.size, 36.0);
    assert_eq!(
        resolved(&sheet, "Inherited", Some("Code")).character.size,
        32.4
    );
}

#[test]
fn percentage_size_validation_is_atomic_and_base_paragraph_disallows_relative_sizes() {
    let mut sheet = sample_sheet();
    let before = sheet.clone();
    for percent in [0, 9, 1001, u16::MAX] {
        assert!(sheet
            .prepare_generated_field_edit(
                StyleNamespace::Character,
                &"Code".into(),
                &StyleDefinitionFieldEdit::SetDeclaration {
                    property: StyleProperty::CharacterSize,
                    value: StylePropertyValue::Percentage(percent)
                }
            )
            .is_err());
    }
    for percent in [10, 1000] {
        assert!(sheet
            .prepare_generated_field_edit(
                StyleNamespace::Character,
                &"Code".into(),
                &StyleDefinitionFieldEdit::SetDeclaration {
                    property: StyleProperty::CharacterSize,
                    value: StylePropertyValue::Percentage(percent)
                }
            )
            .is_ok());
    }
    let mut base = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
    base.character.size = Some(FontSize::Percentage(100));
    assert!(matches!(
        sheet.replace_block_style(base),
        Err(StyleError::InvalidStylePropertyValue {
            property: StyleProperty::CharacterSize,
            ..
        })
    ));
    assert_eq!(sheet, before);
}

#[test]
fn percentage_size_configuration_round_trip_preserves_sparse_units_and_legacy_points() {
    let sheet = sample_sheet();
    let bytes = sheet
        .default_configuration_json(&DocumentStyleAssignment::new(sheet.base_paragraph.clone()))
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let blocks = json["block_styles"].as_array().unwrap();
    assert_eq!(
        blocks.iter().find(|s| s["id"] == "Paragraph").unwrap()["character"]["size"],
        12.0
    );
    assert_eq!(
        blocks.iter().find(|s| s["id"] == "Heading1").unwrap()["character"]["size"]["percentage"],
        200
    );
    let reopened = StyleSheet::default().with_default_json(&bytes).unwrap();
    assert_eq!(
        resolved(&reopened, "Heading1", Some("Code")).character.size,
        21.6
    );
    assert_eq!(
        reopened
            .block_style(&"Heading1".into())
            .unwrap()
            .character
            .size,
        Some(FontSize::Percentage(200))
    );
    for invalid in [9, 1001] {
        let mut invalid_json = json.clone();
        invalid_json["character_styles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["id"] == "Code")
            .unwrap()["properties"]["size"]["percentage"] = invalid.into();
        assert!(StyleSheet::default()
            .with_default_json(&serde_json::to_vec(&invalid_json).unwrap())
            .is_err());
    }
}

#[test]
fn percentage_paragraph_uses_named_parent_while_character_uses_underlying_text() {
    let sheet = sample_sheet();
    let mut document = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
    document.direct_default_character.size = Some(FontSize::Points(30.0));
    for (paragraph, expected) in [("Heading1", 21.6), ("Paragraph", 27.0)] {
        let traced = sheet
            .resolve_assigned_paragraph_style_with_contributions(
                &document,
                &paragraph.into(),
                &BlockProperties::default(),
                &CharacterProperties::default(),
                Some(&"Code".into()),
                &CharacterProperties::default(),
            )
            .unwrap();
        let plain = sheet
            .resolve_assigned_paragraph_style(
                &document,
                &paragraph.into(),
                &BlockProperties::default(),
                &CharacterProperties::default(),
                Some(&"Code".into()),
                &CharacterProperties::default(),
            )
            .unwrap();
        assert_eq!(plain, traced.value);
        assert_eq!(plain.character.size, expected);
    }
    let direct_paragraph = CharacterProperties {
        size: Some(FontSize::Points(40.0)),
        ..Default::default()
    };
    assert_eq!(
        sheet
            .resolve_assigned_paragraph_style(
                &document,
                &"Heading1".into(),
                &BlockProperties::default(),
                &direct_paragraph,
                Some(&"Code".into()),
                &CharacterProperties::default()
            )
            .unwrap()
            .character
            .size,
        36.0
    );
}

#[test]
fn percentage_code_configuration_retains_units_and_rejects_relative_base() {
    let mut sheet = code::default_sheet();
    sheet
        .block_styles
        .get_mut(&sheet.base_paragraph.clone())
        .unwrap()
        .character
        .size = Some(FontSize::Points(12.0));
    let id = code::resolve_name(&sheet, "Comment").unwrap().clone();
    sheet.character_styles.get_mut(&id).unwrap().properties.size = Some(FontSize::Percentage(90));
    let bytes = code::export_snapshot(&sheet).unwrap();
    let imported = code::parse_json(&bytes).unwrap();
    assert_eq!(
        imported.character_styles[&id].properties.size,
        Some(FontSize::Percentage(90))
    );
    let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    invalid["block_styles"][0]["character"]["size"] = serde_json::json!({"percentage":200});
    assert!(code::parse_json(&serde_json::to_vec(&invalid).unwrap()).is_err());
}

#[test]
fn percentage_paragraph_overflow_or_underflow_is_rejected_before_insertion() {
    for percent in [10, 1000] {
        let mut sheet = sample_sheet();
        let mut parent = sheet.base_paragraph.clone();
        let mut rejected = false;
        for index in 0..200 {
            let previous = sheet.clone();
            let id = StyleId(format!("Relative{index}"));
            let result = sheet.insert_block_style(
                BlockStyle {
                    id: id.clone(),
                    based_on: Some(parent),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(FontSize::Percentage(percent)),
                        ..Default::default()
                    },
                    block: BlockProperties::default(),
                },
                StyleDefinitionMetadata::generated(&id.0),
            );
            if result.is_err() {
                assert_eq!(sheet, previous);
                rejected = true;
                break;
            }
            parent = id;
        }
        assert!(rejected);
    }
}

#[test]
fn percentage_scaling_avoids_intermediate_overflow_and_rejects_invalid_final_size() {
    assert_eq!(
        FontSize::Percentage(10).resolve(f32::MAX),
        (f64::from(f32::MAX) / 10.0) as f32
    );
    let mut sheet = sample_sheet();
    sheet
        .character_styles
        .get_mut(&StyleId::from("Code"))
        .unwrap()
        .properties
        .size = Some(FontSize::Percentage(1000));
    let paragraph = CharacterProperties {
        size: Some(FontSize::Points(f32::MAX)),
        ..Default::default()
    };
    let root = DocumentStyleAssignment::new(sheet.base_paragraph.clone());
    assert!(sheet
        .resolve_assigned_paragraph_style(
            &root,
            &sheet.base_paragraph,
            &BlockProperties::default(),
            &paragraph,
            Some(&"Code".into()),
            &CharacterProperties::default()
        )
        .is_err());
    assert!(sheet
        .resolve_assigned_paragraph_style_with_contributions(
            &root,
            &sheet.base_paragraph,
            &BlockProperties::default(),
            &paragraph,
            Some(&"Code".into()),
            &CharacterProperties::default()
        )
        .is_err());
}
