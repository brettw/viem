use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::DocumentLayoutStyles;

#[test]
fn imported_kerning_switch_is_preserved_in_source_but_not_in_shaping_styles() {
    let source = "<p style='font-feature-settings: \"kern\" 0, \"liga\" 0'>AV tail</p><!--keep-->";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let character =
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
    assert_eq!(character.open_type_features.get("kern"), Some(&0));
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    for style in std::iter::once(&styles.default_shaping_style)
        .chain(styles.shaping_runs.iter().map(|run| &run.style))
        .chain(
            styles
                .paragraphs
                .iter()
                .map(|paragraph| &paragraph.default_shaping_style),
        )
    {
        assert!(style.features.iter().all(|feature| feature.tag != *b"kern"));
    }
    assert!(styles.shaping_runs.iter().any(|run| run
        .style
        .features
        .iter()
        .any(|feature| feature.tag == *b"liga" && feature.value == 0)));
    assert_eq!(document.source_bytes(), source.as_bytes());
}
