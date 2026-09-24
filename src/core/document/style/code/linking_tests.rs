use super::*;

fn id(name: &str) -> StyleId {
    StyleId(format!("syntax:{name}"))
}

fn resolved(sheet: &StyleSheet, name: &str) -> ResolvedCharacterStyle {
    sheet
        .resolve_paragraph_style(
            &sheet.base_paragraph,
            &sheet.base_paragraph,
            resolve_name(sheet, name),
            &BlockProperties::default(),
            &CharacterProperties::default(),
        )
        .unwrap()
        .character
}

/// Reproduce the sparse version-1 file, whose changed entries nevertheless
/// stored entire definitions, including their copied default foreground.
fn legacy_json(sheet: &StyleSheet) -> Vec<u8> {
    let defaults = default_sheet();
    serde_json::to_vec(&File {
        version: 1,
        block_styles: vec![],
        character_styles: sheet
            .character_styles
            .values()
            .filter(|s| {
                defaults.character_styles.get(&s.id) != Some(s)
                    || defaults.character_metadata.get(&s.id) != sheet.character_metadata.get(&s.id)
            })
            .map(|s| CharacterEntry {
                name: sheet.character_metadata[&s.id].display_name.clone(),
                style: s.clone(),
            })
            .collect(),
        suppressed_character_ids: defaults
            .character_styles
            .keys()
            .filter(|id| !sheet.character_styles.contains_key(*id))
            .cloned()
            .collect(),
    })
    .unwrap()
}

#[test]
fn links_preserve_the_default_palette_and_exact_name_lookup() {
    let linked = default_sheet();
    validate(&linked).unwrap();
    for (name, color) in [("Comment", (115, 123, 130)), ("Keyword.function", (170, 65, 153)), ("String.escape", (38, 132, 77))] {
        assert_eq!(resolved(&linked, name).foreground, Color { red: color.0 as f32 / 255., green: color.1 as f32 / 255., blue: color.2 as f32 / 255., alpha: 1. });
    }
    for (child, parent) in [
        ("Comment.documentation", "Comment"),
        ("Keyword.function", "Keyword"),
        ("Keyword", "Statement"),
        ("Function.macro.builtin", "Function.macro"),
        ("Function.macro", "Function"),
        ("Number.float", "Number"),
        ("Number", "Constant"),
        ("Text.uri", "Text"),
        ("Text", "String"),
        ("Punctuation.delimiter", "Punctuation"),
        ("Punctuation", "Delimiter"),
        ("Preproc", "PreProc"),
        ("Storageclass", "StorageClass"),
    ] {
        let style = &linked.character_styles[&id(child)];
        assert_eq!(style.based_on, Some(id(parent)), "{child}");
        assert_eq!(style.properties, CharacterProperties::default(), "{child}");
    }
    assert!(resolve_name(&linked, "Comment.unknown").is_none());
    assert!(resolve_name(&linked, "comment").is_none());
    assert!(resolve_name(&linked, "@comment").is_none());
    assert!(linked.character_metadata.values().all(|m| !m.display_name.starts_with('@')));
    assert!(linked.character_styles.keys().all(|id| !id.0.starts_with("syntax:@")));
    for (id, metadata) in &linked.character_metadata {
        if let Some((parent, _)) = metadata.display_name.rsplit_once('.') {
            assert_eq!(linked.character_styles[id].based_on.as_ref(), resolve_name(&linked, parent));
        }
    }
}

#[test]
fn parent_changes_cascade_until_an_individual_property_is_overridden() {
    let mut sheet = default_sheet();
    let parent = &mut sheet
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .properties;
    parent.size = Some(23.0.into());
    parent.bold = Some(true);
    parent.foreground = Some(Color {
        red: 0.2,
        green: 0.4,
        blue: 0.6,
        alpha: 1.,
    });
    assert_eq!(
        resolved(&sheet, "Comment"),
        resolved(&sheet, "Comment.documentation")
    );

    sheet
        .character_styles
        .get_mut(&id("Comment.documentation"))
        .unwrap()
        .properties
        .size = Some(31.0.into());
    sheet
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .properties
        .size = Some(26.0.into());
    assert_eq!(resolved(&sheet, "Comment").size, 26.);
    assert_eq!(resolved(&sheet, "Comment.documentation").size, 31.);
    assert_eq!(
        resolved(&sheet, "Comment.documentation").foreground,
        resolved(&sheet, "Comment").foreground
    );
    sheet
        .character_styles
        .get_mut(&id("Comment.documentation"))
        .unwrap()
        .properties
        .size = None;
    assert_eq!(resolved(&sheet, "Comment.documentation").size, 26.);
}

#[test]
fn obsolete_version_one_is_rejected_without_migration() {
    let sheet = default_sheet();
    assert!(parse_json(&legacy_json(&sheet)).is_err());
    let restored = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
}

#[test]
fn current_schema_rejects_dangling_parents_and_cycles_without_repair() {
    let mut sheet = default_sheet();
    sheet.character_styles.get_mut(&id("Comment")).unwrap().based_on = Some(id("Comment.documentation"));
    assert!(parse_json(&export_snapshot(&sheet).unwrap()).is_err());
    let mut sheet = default_sheet();
    sheet.character_styles.remove(&id("Comment"));
    sheet.character_metadata.remove(&id("Comment"));
    assert!(parse_json(&export_snapshot(&sheet).unwrap()).is_err());
}

#[test]
fn version_three_preserves_explicit_default_colored_and_detached_overrides() {
    let mut sheet = default_sheet();
    let copied_color = sheet.character_styles[&id("Comment")].properties.foreground;
    sheet
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .properties
        .foreground = copied_color;
    sheet
        .character_styles
        .get_mut(&id("Comment.documentation"))
        .unwrap()
        .based_on = None;
    let restored = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
    assert_eq!(restored.character_metadata, sheet.character_metadata);
}

#[test]
fn version_two_merges_capture_overrides_and_rewrites_parents_and_suppressions() {
    let legacy = br#"{
        "version": 2,
        "character_styles": [
            {"id":"syntax:Comment","name":"Project comments","based_on":null,
             "properties":{"size":21,"bold":true}},
            {"id":"syntax:@comment","name":"@comment","based_on":"syntax:Comment",
             "properties":{"size":27}},
            {"id":"syntax:@comment.documentation","name":"@comment.documentation",
             "based_on":"syntax:@comment","properties":{"underline":true}},
            {"id":"custom:notes","name":"Notes","based_on":"syntax:@comment.documentation",
             "properties":{"slant":"Italic"}}
        ],
        "suppressed_character_ids":["syntax:@spell"]
    }"#;
    let sheet = parse_json(legacy).unwrap();
    let comment = &sheet.character_styles[&id("Comment")];
    assert_eq!(comment.based_on, None);
    assert_eq!(comment.properties.size, Some(27.0.into()));
    assert_eq!(comment.properties.bold, Some(true));
    assert_eq!(sheet.character_metadata[&id("Comment")].display_name, "Comment");
    let preserved = resolve_name(&sheet, "Project comments").unwrap();
    assert_eq!(sheet.character_styles[preserved].properties.size, Some(21.0.into()));
    assert_eq!(sheet.character_styles[preserved].properties.bold, Some(true));
    assert_eq!(sheet.character_styles[preserved].based_on, None);
    assert_eq!(sheet.character_styles[&id("Comment.documentation")].based_on, Some(id("Comment")));
    assert_eq!(sheet.character_styles[&StyleId("custom:notes".into())].based_on, Some(id("Comment.documentation")));
    assert!(!sheet.character_styles.contains_key(&id("Spell")));
    assert!(!sheet.character_styles.keys().any(|id| id.0.contains('@')));
    let bytes = export_snapshot(&sheet).unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["version"], 3);
    let restored = parse_json(&bytes).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
    assert_eq!(restored.character_metadata, sheet.character_metadata);
}

#[test]
fn version_two_preserves_detached_captures_and_renamed_dotted_definitions() {
    let sheet = parse_json(br#"{
        "version":2,
        "character_styles":[
            {"id":"syntax:@comment","name":"@comment","based_on":null,"properties":{}},
            {"id":"syntax:@comment.documentation","name":"Project documentation",
             "based_on":"syntax:@comment","properties":{"size":24}}
        ]
    }"#).unwrap();
    let comment = &sheet.character_styles[&id("Comment")];
    assert_eq!(comment.based_on, None);
    assert_eq!(comment.properties, CharacterProperties::default());
    assert_eq!(resolve_name(&sheet, "Project documentation"), Some(&id("Comment.documentation")));
    assert!(resolve_name(&sheet, "Comment.documentation").is_none());
    assert_eq!(resolved(&sheet, "Project documentation").size, 24.);
}

#[test]
fn version_two_does_not_repair_duplicate_ids_or_self_inheritance() {
    for characters in [
        r#"[{"id":"syntax:@comment","name":"@comment","properties":{}},
             {"id":"syntax:@comment","name":"Other","properties":{}}]"#,
        r#"[{"id":"syntax:@comment","name":"@comment","based_on":"syntax:@comment","properties":{}}]"#,
    ] {
        let json = format!(r#"{{"version":2,"character_styles":{characters}}}"#);
        assert!(parse_json(json.as_bytes()).is_err());
    }
}

#[test]
fn capture_canonicalization_preserves_dotted_case_and_only_uppercases_first_letter() {
    for (raw, canonical) in [
        ("@comment.documentation", "Comment.documentation"),
        ("comment.documentation", "Comment.documentation"),
        ("@custom.HTTPHeader", "Custom.HTTPHeader"),
        ("@storageclass", "Storageclass"),
        ("@preproc", "Preproc"),
    ] {
        assert_eq!(canonical_capture_name(raw), canonical);
    }
}

fn version_two_json(sheet: &StyleSheet) -> Vec<u8> {
    let defaults = default_sheet_with_capture_aliases(true);
    serde_json::to_vec(&File {
        version: 2,
        block_styles: vec![],
        character_styles: sheet.character_styles.values().filter(|style| {
            defaults.character_styles.get(&style.id) != Some(style)
                || defaults.character_metadata.get(&style.id) != sheet.character_metadata.get(&style.id)
        }).map(|style| CharacterEntry {
            name: sheet.character_metadata[&style.id].display_name.clone(),
            style: style.clone(),
        }).collect(),
        suppressed_character_ids: defaults.character_styles.keys()
            .filter(|id| !sheet.character_styles.contains_key(*id)).cloned().collect(),
    }).unwrap()
}

#[test]
fn version_two_default_settings_upgrade_to_exact_current_defaults() {
    let old = default_sheet_with_capture_aliases(true);
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    let defaults = default_sheet();
    assert_eq!(migrated.character_styles, defaults.character_styles);
    assert_eq!(migrated.character_metadata, defaults.character_metadata);
}

#[test]
fn version_two_deleting_either_collapsed_definition_keeps_the_surviving_one() {
    for removed in ["@comment", "Comment"] {
        let mut old = default_sheet_with_capture_aliases(true);
        let removed_id = id(removed);
        let parent = old.character_styles[&removed_id].based_on.clone();
        for child in old.character_styles.values_mut().filter(|style| style.based_on.as_ref() == Some(&removed_id)) {
            child.based_on = parent.clone();
        }
        old.character_styles.remove(&removed_id);
        old.character_metadata.remove(&removed_id);
        let migrated = parse_json(&version_two_json(&old)).unwrap();
        let comment = resolve_name(&migrated, "Comment").unwrap();
        assert_eq!(migrated.character_styles[&id("Comment.documentation")].based_on.as_ref(), Some(comment));
        assert!(!migrated.character_styles.keys().any(|id| id.0.contains('@')));
        assert_eq!(migrated.character_styles[comment].properties.foreground.is_some(), removed == "@comment");
        let round_trip = parse_json(&export_snapshot(&migrated).unwrap()).unwrap();
        assert_eq!(round_trip.character_styles, migrated.character_styles);
    }
}

#[test]
fn version_two_custom_parent_between_alias_and_vim_style_keeps_acyclic_identities() {
    let mut old = default_sheet_with_capture_aliases(true);
    let notes = StyleId("custom:notes".into());
    old.character_styles.insert(notes.clone(), CharacterStyle {
        id: notes.clone(), based_on: Some(id("Comment")),
        properties: CharacterProperties { size: Some(23.0.into()), ..Default::default() },
    });
    old.character_metadata.insert(notes.clone(), StyleDefinitionMetadata::generated("Notes"));
    old.character_styles.get_mut(&id("@comment")).unwrap().based_on = Some(notes.clone());
    old.character_styles.get_mut(&id("@comment")).unwrap().properties.bold = Some(true);
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    validate(&migrated).unwrap();
    assert_eq!(resolve_name(&migrated, "Notes"), Some(&notes));
    assert_eq!(migrated.character_styles[&id("Comment")].based_on, Some(notes.clone()));
    let preserved_parent = migrated.character_styles[&notes].based_on.as_ref().unwrap();
    assert!(preserved_parent.0.starts_with("migrated-code:"));
    assert_eq!(migrated.character_styles[preserved_parent].based_on, None);
    assert_eq!(resolved(&migrated, "Comment"), resolved(&old, "@comment"));
    assert_eq!(resolved(&migrated, "Notes"), resolved(&old, "Notes"));
    assert_eq!(resolved(&migrated, "Todo"), resolved(&old, "Todo"));
    let encoded = export_snapshot(&migrated).unwrap();
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(restored.character_styles, migrated.character_styles);
    assert_eq!(restored.character_metadata, migrated.character_metadata);
    assert_eq!(export_snapshot(&restored).unwrap(), encoded);
}

#[test]
fn version_two_existing_custom_canonical_name_keeps_identity_and_appearance() {
    let mut old = default_sheet_with_capture_aliases(true);
    old.character_metadata.get_mut(&id("Comment")).unwrap().display_name = "Project comments".into();
    let custom = StyleId("custom:comment".into());
    old.character_styles.insert(custom.clone(), CharacterStyle {
        id: custom.clone(), based_on: None,
        properties: CharacterProperties { size: Some(31.0.into()), bold: Some(true), ..Default::default() },
    });
    old.character_metadata.insert(custom.clone(), StyleDefinitionMetadata::generated("Comment"));
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    validate(&migrated).unwrap();
    assert_eq!(resolve_name(&migrated, "Comment"), Some(&custom));
    assert_eq!(resolved(&migrated, "Comment"), resolved(&old, "Comment"));
    assert_eq!(resolved(&migrated, "Project comments"), resolved(&old, "Project comments"));
    assert_eq!(resolve_name(&migrated, "Comment (previous capture)"), Some(&id("Comment")));
    assert_eq!(resolved(&migrated, "Comment (previous capture)"), resolved(&old, "@comment"));
    let encoded = export_snapshot(&migrated).unwrap();
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(restored.character_styles, migrated.character_styles);
    assert_eq!(restored.character_metadata, migrated.character_metadata);
    assert_eq!(export_snapshot(&restored).unwrap(), encoded);
}

#[test]
fn version_two_authored_intermediate_names_are_reused_for_new_dotted_defaults() {
    let mut old = default_sheet_with_capture_aliases(true);
    let text = StyleId("custom:text".into());
    old.character_styles.insert(text.clone(), CharacterStyle {
        id: text.clone(), based_on: Some(id("String")),
        properties: CharacterProperties { size: Some(29.0.into()), ..Default::default() },
    });
    old.character_metadata.insert(text.clone(), StyleDefinitionMetadata::generated("Text"));
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    assert_eq!(resolve_name(&migrated, "Text"), Some(&text));
    assert_eq!(migrated.character_styles[&id("Text.uri")].based_on, Some(text));
    assert!(!migrated.character_styles.contains_key(&id("Text")));
    assert_eq!(resolved(&migrated, "Text"), resolved(&old, "Text"));
    assert_eq!(resolved(&migrated, "Text.uri").size, 29.);
    let encoded = export_snapshot(&migrated).unwrap();
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(export_snapshot(&restored).unwrap(), encoded);
}

#[test]
fn version_two_authored_intermediate_descendant_keeps_previous_child_parent() {
    for custom_id in ["custom:text", "syntax:Text"] {
    let mut old = default_sheet_with_capture_aliases(true);
    let text = StyleId(custom_id.into());
    old.character_styles.insert(text.clone(), CharacterStyle {
        id: text.clone(), based_on: Some(id("@text.uri")),
        properties: CharacterProperties { size: Some(29.0.into()), ..Default::default() },
    });
    old.character_metadata.insert(text.clone(), StyleDefinitionMetadata::generated("Text"));
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    validate(&migrated).unwrap();
    assert_eq!(resolve_name(&migrated, "Text"), Some(&text));
    assert_eq!(migrated.character_styles[&text].based_on, Some(id("Text.uri")));
    assert_eq!(migrated.character_styles[&id("Text.uri")].based_on, Some(id("String")));
    assert_eq!(migrated.character_styles.contains_key(&id("Text")), custom_id == "syntax:Text");
    assert_eq!(resolved(&migrated, "Text"), resolved(&old, "Text"));
    assert_eq!(resolved(&migrated, "Text.uri"), resolved(&old, "@text.uri"));
    let encoded = export_snapshot(&migrated).unwrap();
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(export_snapshot(&restored).unwrap(), encoded);
    }
}

#[test]
fn version_two_renamed_base_keeps_existing_custom_children_on_its_preserved_identity() {
    let mut old = default_sheet_with_capture_aliases(true);
    old.character_metadata.get_mut(&id("Comment")).unwrap().display_name = "Project comments".into();
    old.character_styles.get_mut(&id("Comment")).unwrap().properties.size = Some(21.0.into());
    old.character_styles.get_mut(&id("@comment")).unwrap().properties.size = Some(27.0.into());
    let notes = StyleId("custom:notes".into());
    old.character_styles.insert(notes.clone(), CharacterStyle {
        id: notes.clone(), based_on: Some(id("Comment")), properties: Default::default(),
    });
    old.character_metadata.insert(notes.clone(), StyleDefinitionMetadata::generated("Notes"));
    let migrated = parse_json(&version_two_json(&old)).unwrap();
    let project = resolve_name(&migrated, "Project comments").unwrap();
    assert_eq!(resolve_name(&migrated, "Notes"), Some(&notes));
    assert_eq!(migrated.character_styles[&notes].based_on.as_ref(), Some(project));
    assert_eq!(resolved(&migrated, "Notes"), resolved(&old, "Notes"));
    assert_eq!(resolved(&migrated, "Notes").size, 21.);
    assert_eq!(resolved(&migrated, "Comment").size, 27.);
    assert_eq!(migrated.character_styles[&id("Comment.documentation")].based_on, Some(id("Comment")));
    let encoded = export_snapshot(&migrated).unwrap();
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(export_snapshot(&restored).unwrap(), encoded);
}
