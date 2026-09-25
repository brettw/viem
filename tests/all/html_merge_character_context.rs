use viem_core::document::{Color, FontSlant, WritingDirection};
use viem_core::layout::DocumentLayoutStyles;
use viem_core::{Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn merge_preserves_removed_ancestor_character_declarations_on_retained_text() {
    for source in [
        "<div style='color:red'><p>A</p></div><div style='color:blue'><p>B</p></div><p>C</p>",
        "<p>A</p><div style='font-style:italic'><p>B</p><p>C</p></div>",
        "<p>A</p><div lang='fr' dir='rtl'><p>B</p><p>C</p></div>",
        "<p>A</p><div style='color:red' lang='fr' dir='rtl'><section style='color:blue' lang='de' dir='ltr'><p>B</p></section></div><!--keep--><p>C</p>",
    ] {
        let mut document = html(source);
        let retained = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
        let following = DocumentLayoutStyles::character_at(document.projection(), 4, false).unwrap();
        document.delete(1..2).unwrap();
        assert_eq!(document.text(), "AB\nC");
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap(), retained, "{source}");
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 3, false).unwrap(), following, "{source}");
        let changed = document.source_bytes();
        let reopened = Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(DocumentLayoutStyles::character_at(reopened.projection(), 1, false).unwrap(), retained);
        if source.contains("<!--keep-->") {
            assert!(String::from_utf8_lossy(&changed).contains("<!--keep-->"));
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn retained_direct_context_overlays_left_named_style_without_copying_common_owners() {
    let source = "<main style='color:red' lang='en'><h2>A</h2><div style='color:blue' lang='fr' dir='rtl'><p><em>B</em></p></div><p>C</p></main>";
    let mut document = html(source);
    let left = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    document.delete(1..2).unwrap();
    let right = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
    assert_eq!(document.projection().blocks()[0].style.0, "Heading2");
    assert_eq!(right.size, left.size);
    assert_eq!(right.weight, left.weight);
    assert_eq!(
        right.foreground,
        Color {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            alpha: 1.0
        }
    );
    assert_eq!(right.slant, FontSlant::Italic);
    assert_eq!(right.language.as_deref(), Some("fr"));
    assert_eq!(right.direction, WritingDirection::RightToLeft);
    let changed = String::from_utf8(document.source_bytes()).unwrap();
    assert_eq!(changed.matches("lang=\"en\"").count(), 0);
    assert!(changed.starts_with("<main style='color:red' lang='en'><h2>A"));
    assert!(changed.ends_with("<p>C</p></main>"));
}
