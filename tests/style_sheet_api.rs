use evim_core::document::{
    BlockProperties, BlockRole, BlockStyle, CharacterProperties, CharacterStyle, StyleId,
    StyleSheet,
};

#[test]
fn style_definitions_have_ergonomic_read_only_lookup_and_iteration() {
    let mut sheet = StyleSheet::default();
    let paragraph_id = StyleId::from("Body");
    let paragraph = BlockStyle {
        id: paragraph_id.clone(),
        based_on: Some(sheet.base_paragraph.clone()),
        next_paragraph_style: Some(paragraph_id.clone()),
        role: BlockRole::Paragraph,
        character: CharacterProperties::default(),
        block: BlockProperties::default(),
    };
    sheet.insert_block_style(paragraph.clone()).unwrap();

    let character_id = StyleId::from("Comment");
    let character = CharacterStyle {
        id: character_id.clone(),
        based_on: Some(sheet.base_character.clone()),
        properties: CharacterProperties::default(),
    };
    sheet.insert_character_style(character.clone()).unwrap();

    assert_eq!(sheet.block_style(&paragraph_id), Some(&paragraph));
    assert_eq!(sheet.character_style(&character_id), Some(&character));
    assert_eq!(sheet.block_styles().len(), sheet.block_style_count());
    assert_eq!(
        sheet.character_styles().len(),
        sheet.character_style_count()
    );
    assert!(sheet.block_styles().any(|style| style.id == paragraph_id));
    assert!(sheet
        .character_styles()
        .any(|style| style.id == character_id));
}
