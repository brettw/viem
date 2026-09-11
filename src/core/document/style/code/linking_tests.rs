use super::*;

fn id(name: &str) -> StyleId {
    StyleId(format!("syntax:{name}"))
}

fn resolved(sheet: &StyleSheet, name: &str) -> ResolvedCharacterStyle {
    sheet
        .resolve_paragraph_style(
            &sheet.base_document,
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
    let defaults = default_sheet_with_links(false);
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
    let legacy = default_sheet_with_links(false);
    let linked = default_sheet();
    validate(&linked).unwrap();
    for metadata in legacy.character_metadata.values() {
        assert_eq!(
            resolved(&linked, &metadata.display_name),
            resolved(&legacy, &metadata.display_name),
            "{} changed its default appearance",
            metadata.display_name
        );
    }
    for (child, parent) in [
        ("@comment.documentation", "@comment"),
        ("@comment", "Comment"),
        ("@keyword.function", "@keyword"),
        ("@keyword", "Keyword"),
        ("Keyword", "Statement"),
        ("@function.macro.builtin", "@function.macro"),
        ("@function.macro", "@function"),
        ("@function", "Function"),
        ("@number.float", "@number"),
        ("@number", "Number"),
        ("Number", "Constant"),
    ] {
        let style = &linked.character_styles[&id(child)];
        assert_eq!(style.based_on, Some(id(parent)), "{child}");
        assert_eq!(style.properties, CharacterProperties::default(), "{child}");
    }
    assert!(resolve_name(&linked, "@comment.unknown").is_none());
    assert!(resolve_name(&linked, "comment").is_none());
}

#[test]
fn parent_changes_cascade_until_an_individual_property_is_overridden() {
    let mut sheet = default_sheet();
    let parent = &mut sheet
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .properties;
    parent.size = Some(23.);
    parent.bold = Some(true);
    parent.foreground = Some(Color {
        red: 0.2,
        green: 0.4,
        blue: 0.6,
        alpha: 1.,
    });
    assert_eq!(
        resolved(&sheet, "Comment"),
        resolved(&sheet, "@comment.documentation")
    );

    sheet
        .character_styles
        .get_mut(&id("@comment.documentation"))
        .unwrap()
        .properties
        .size = Some(31.);
    sheet
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .properties
        .size = Some(26.);
    assert_eq!(resolved(&sheet, "@comment").size, 26.);
    assert_eq!(resolved(&sheet, "@comment.documentation").size, 31.);
    assert_eq!(
        resolved(&sheet, "@comment.documentation").foreground,
        resolved(&sheet, "Comment").foreground
    );
    sheet
        .character_styles
        .get_mut(&id("@comment.documentation"))
        .unwrap()
        .properties
        .size = None;
    assert_eq!(resolved(&sheet, "@comment.documentation").size, 26.);
}

#[test]
fn legacy_migration_separates_copied_defaults_from_local_changes_and_renames() {
    let mut legacy = default_sheet_with_links(false);
    let comment_id = id("Comment");
    let color = Color {
        red: 0.7,
        green: 0.2,
        blue: 0.3,
        alpha: 1.,
    };
    legacy
        .character_styles
        .get_mut(&comment_id)
        .unwrap()
        .properties
        .foreground = Some(color);
    legacy
        .character_metadata
        .get_mut(&comment_id)
        .unwrap()
        .display_name = "Project comments".into();
    legacy
        .character_styles
        .get_mut(&id("@comment"))
        .unwrap()
        .properties
        .size = Some(27.);
    let local = legacy
        .character_styles
        .get_mut(&id("@comment.documentation"))
        .unwrap();
    local.properties.slant = Some(FontSlant::Italic);
    local.properties.foreground = Some(Color {
        red: 0.,
        green: 0.,
        blue: 1.,
        alpha: 1.,
    });
    // Explicitly detached parents must stay detached, including their paint.
    legacy
        .character_styles
        .get_mut(&id("Todo"))
        .unwrap()
        .based_on = None;
    let loaded = parse_json(&legacy_json(&legacy)).unwrap();
    assert_eq!(
        loaded.character_styles[&id("@comment")].based_on,
        Some(comment_id.clone())
    );
    assert_eq!(
        loaded.character_styles[&id("@comment")]
            .properties
            .foreground,
        None
    );
    assert_eq!(resolved(&loaded, "@comment").foreground, color);
    assert_eq!(resolved(&loaded, "@comment").size, 27.);
    assert_eq!(resolved(&loaded, "@comment.documentation").size, 27.);
    assert_eq!(
        resolved(&loaded, "@comment.documentation").slant,
        FontSlant::Italic
    );
    assert_eq!(
        loaded.character_styles[&id("@comment.documentation")]
            .properties
            .foreground,
        legacy.character_styles[&id("@comment.documentation")]
            .properties
            .foreground
    );
    assert_eq!(
        loaded.character_styles[&id("Todo")],
        legacy.character_styles[&id("Todo")]
    );
    assert!(resolve_name(&loaded, "Comment").is_none());
    assert_eq!(resolve_name(&loaded, "Project comments"), Some(&comment_id));
    let encoded = export_snapshot(&loaded).unwrap();
    assert_eq!(serde_json::from_slice::<File>(&encoded).unwrap().version, 2);
    let restored = parse_json(&encoded).unwrap();
    assert_eq!(restored.character_styles, loaded.character_styles);
    assert_eq!(restored.character_metadata, loaded.character_metadata);
}

#[test]
fn migration_preserves_suppression_and_avoids_cycles_with_authored_parents() {
    let mut legacy = default_sheet_with_links(false);
    legacy
        .remove_character_style(&id("Comment"), false)
        .unwrap();
    let loaded = parse_json(&legacy_json(&legacy)).unwrap();
    assert!(resolve_name(&loaded, "Comment").is_none());
    assert_eq!(
        loaded.character_styles[&id("@comment")].based_on,
        Some(loaded.base_character.clone())
    );
    assert_eq!(resolved(&loaded, "@comment"), resolved(&legacy, "@comment"));
    assert_eq!(
        loaded.character_styles[&id("@comment.documentation")].based_on,
        Some(id("@comment"))
    );
    let restored = parse_json(&export_snapshot(&loaded).unwrap()).unwrap();
    assert_eq!(restored.character_styles, loaded.character_styles);

    let mut legacy = default_sheet_with_links(false);
    legacy
        .character_styles
        .get_mut(&id("Comment"))
        .unwrap()
        .based_on = Some(id("@comment.documentation"));
    validate(&legacy).unwrap();
    let loaded = parse_json(&legacy_json(&legacy)).unwrap();
    validate(&loaded).unwrap();
    assert_eq!(
        loaded.character_styles[&id("Comment")],
        legacy.character_styles[&id("Comment")]
    );
    for name in ["Comment", "@comment", "@comment.documentation"] {
        assert_eq!(resolved(&loaded, name), resolved(&legacy, name));
    }
}

#[test]
fn version_two_preserves_explicit_default_colored_and_detached_overrides() {
    let mut sheet = default_sheet();
    let copied_color = sheet.character_styles[&id("Comment")].properties.foreground;
    sheet
        .character_styles
        .get_mut(&id("@comment"))
        .unwrap()
        .properties
        .foreground = copied_color;
    sheet
        .character_styles
        .get_mut(&id("@comment.documentation"))
        .unwrap()
        .based_on = Some(sheet.base_character.clone());
    let restored = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
    assert_eq!(restored.character_metadata, sheet.character_metadata);
}
