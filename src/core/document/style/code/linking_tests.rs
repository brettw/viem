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

#[test]
fn links_follow_the_explicit_family_palette_and_exact_name_lookup() {
    let mut linked = default_sheet();
    for (name, color) in [("Comment", (115, 123, 130)), ("Statement", (170, 65, 153)), ("String", (38, 132, 77))] {
        linked.character_styles.get_mut(&id(name)).unwrap().properties.foreground = Some(Color {
            red: color.0 as f32 / 255., green: color.1 as f32 / 255., blue: color.2 as f32 / 255., alpha: 1.,
        });
    }
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
fn diff_groups_have_sparse_aliases_that_follow_explicit_family_colors() {
    let mut sheet = parse_json(br#"{"version":3}"#).unwrap();
    for (index, (child, parent)) in [("Added", "DiffAdd"), ("Removed", "DiffDelete"), ("Changed", "DiffChange")].into_iter().enumerate() {
        let root = &sheet.character_styles[&id(parent)];
        assert!(root.properties.foreground.is_some(), "{parent} needs a visible syntax color");
        assert_eq!(root.properties.background, None);
        let alias = &sheet.character_styles[&id(child)];
        assert_eq!(alias.based_on, Some(id(parent)));
        assert_eq!(alias.properties, CharacterProperties::default());
        let color = Color { red: 0.1 + index as f32 * 0.1, green: 0.4, blue: 0.7, alpha: 1. };
        sheet.character_styles.get_mut(&id(parent)).unwrap().properties.foreground = Some(color);
        assert_eq!(resolved(&sheet, child).foreground, color);
        assert_eq!(resolved(&sheet, parent).foreground, color);
    }
    let reloaded = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(reloaded.character_styles, sheet.character_styles);
}

#[test]
fn legacy_diff_definitions_keep_their_saved_identity_color_and_clears() {
    let color = Color { red: 0.3, green: 0.5, blue: 0.7, alpha: 1. };
    for (name, saved_id, alias) in [
        ("DiffAdd", "implicit:DiffAdd", Some("Added")),
        ("DiffDelete", "custom:removals", Some("Removed")),
        ("DiffChange", "implicit:DiffChange", Some("Changed")),
        ("Added", "implicit:Added", None),
        ("Removed", "custom:Removed", None),
        ("Changed", "implicit:Changed", None),
    ] {
        for foreground in [None, Some(color)] {
            let saved = CharacterStyle {
                id: StyleId(saved_id.into()),
                based_on: None,
                properties: CharacterProperties { foreground, bold: Some(false), ..Default::default() },
            };
            let file = File {
                version: 3, block_styles: vec![],
                character_styles: vec![CharacterEntry { name: name.into(), style: saved.clone() }],
                suppressed_character_ids: BTreeSet::new(),
            };
            let sheet = parse_json(&serde_json::to_vec(&file).unwrap()).unwrap();
            assert_eq!(resolve_name(&sheet, name), Some(&saved.id));
            assert_eq!(sheet.character_styles[&saved.id], saved, "{name}");
            assert!(!sheet.character_styles.contains_key(&id(name)));
            if let Some(alias) = alias {
                assert_eq!(sheet.character_styles[&id(alias)].based_on, Some(saved.id.clone()));
                assert_eq!(resolved(&sheet, alias), resolved(&sheet, name));
            }
            let reloaded = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
            assert_eq!(sheet.character_styles, reloaded.character_styles);
            assert_eq!(sheet.character_metadata, reloaded.character_metadata);
        }
    }
}

#[test]
fn diff_defaults_preserve_suppressed_names_and_reject_duplicate_saved_definitions() {
    let file = File {
        version: 3, block_styles: vec![], character_styles: vec![],
        suppressed_character_ids: [id("Added")].into_iter().collect(),
    };
    let sheet = parse_json(&serde_json::to_vec(&file).unwrap()).unwrap();
    assert!(resolve_name(&sheet, "Added").is_none());
    assert!(resolve_name(&sheet, "DiffAdd").is_some());
    let mut file = file;
    file.suppressed_character_ids = [id("DiffAdd")].into_iter().collect();
    let sheet = parse_json(&serde_json::to_vec(&file).unwrap()).unwrap();
    assert!(resolve_name(&sheet, "DiffAdd").is_none());
    assert_eq!(sheet.character_styles[&id("Added")].based_on, None);
    assert_eq!(sheet.character_styles[&id("Added")].properties, CharacterProperties::default());
    file.suppressed_character_ids.clear();
    for saved_id in ["implicit:Added", "syntax:Added"] {
        file.character_styles.push(CharacterEntry {
            name: "Added".into(),
            style: CharacterStyle { id: StyleId(saved_id.into()), based_on: None, properties: Default::default() },
        });
    }
    assert!(parse_json(&serde_json::to_vec(&file).unwrap()).is_err());
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
fn obsolete_stylesheet_versions_are_rejected_without_migration() {
    let sheet = default_sheet();
    let current = export_snapshot(&sheet).unwrap();
    let mut file: serde_json::Value = serde_json::from_slice(&current).unwrap();
    for version in [1, 2] {
        file["version"] = version.into();
        assert_eq!(
            parse_json(&serde_json::to_vec(&file).unwrap()).unwrap_err(),
            "Unsupported Code stylesheet"
        );
    }
    let restored = parse_json(&current).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
    assert_eq!(restored.character_metadata, sheet.character_metadata);
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
