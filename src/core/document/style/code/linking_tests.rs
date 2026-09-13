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
    for (name, color) in [("@comment", (115, 123, 130)), ("@keyword.function", (170, 65, 153)), ("@string.escape", (38, 132, 77))] {
        assert_eq!(resolved(&linked, name).foreground, Color { red: color.0 as f32 / 255., green: color.1 as f32 / 255., blue: color.2 as f32 / 255., alpha: 1. });
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
fn obsolete_version_one_is_rejected_without_migration() {
    let sheet = default_sheet();
    assert!(parse_json(&legacy_json(&sheet)).is_err());
    let restored = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
}

#[test]
fn current_schema_rejects_dangling_parents_and_cycles_without_repair() {
    let mut sheet = default_sheet();
    sheet.character_styles.get_mut(&id("Comment")).unwrap().based_on = Some(id("@comment.documentation"));
    assert!(parse_json(&export_snapshot(&sheet).unwrap()).is_err());
    let mut sheet = default_sheet();
    sheet.character_styles.remove(&id("Comment"));
    sheet.character_metadata.remove(&id("Comment"));
    assert!(parse_json(&export_snapshot(&sheet).unwrap()).is_err());
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
        .based_on = None;
    let restored = parse_json(&export_snapshot(&sheet).unwrap()).unwrap();
    assert_eq!(restored.character_styles, sheet.character_styles);
    assert_eq!(restored.character_metadata, sheet.character_metadata);
}
