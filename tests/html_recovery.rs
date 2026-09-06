use evim_core::document::{CharacterProperties, FontSlant, StyleApplication};
use evim_core::{Document, Encoding, Format};

fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}
fn direct(document: &Document, at: usize) -> CharacterProperties {
    document
        .projection()
        .style_spans()
        .iter()
        .find_map(|span| {
            if span.range.contains(&at) {
                if let StyleApplication::Direct(properties) = &span.application {
                    return Some(properties.clone());
                }
            }
            None
        })
        .unwrap_or_default()
}

#[test]
fn active_formatting_reconstructs_after_explicit_and_implied_paragraph_closes() {
    for source in [
        "<p><b data-x='keep'>one</p>two</b>three",
        "<p><b data-x='keep'>one<p>two</b>three",
    ] {
        let mut document = open(source);
        assert_eq!(document.text(), "one\ntwothree");
        assert_eq!(direct(&document, 0).weight, Some(700));
        assert_eq!(direct(&document, 4).weight, Some(700));
        assert_eq!(direct(&document, 7).weight, None);
        assert_eq!(document.source_bytes(), source.as_bytes());
        document.replace(4..7, "TWO").unwrap();
        assert_eq!(document.text(), "one\nTWOthree");
        assert_eq!(
            document.source_bytes(),
            source.replace("two", "TWO").as_bytes()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn misnested_formatting_end_preserves_open_paragraph_and_its_direct_properties() {
    let source = "<b><p style='font-size:24pt;margin-inline-start:8pt'><i>one</b>two</i>three</p>";
    let document = open(source);
    assert_eq!(document.text(), "onetwothree");
    assert_eq!(direct(&document, 0).weight, Some(700));
    assert_eq!(direct(&document, 3).weight, None);
    assert_eq!(direct(&document, 3).slant, Some(FontSlant::Italic));
    assert_eq!(direct(&document, 6).slant, None);
    assert_eq!(direct(&document, 6).size, Some(24.0));
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .leading_indent,
        Some(8.0)
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn inline_css_recovers_declarations_and_applies_only_valid_supported_values() {
    let source = r#"<p style="font-family: Times   New   Roman, serif; font-family: inherit; font-feature-settings: 'liga' OFF; font-feature-settings: liga 1; text-decoration-line: underline; text-decoration-line: none underline; color: #ff0000; color: rgb(0 0 0 0.5); font-size:12pt ! /*keep*/ IMPORTANT; font-size:99pt; @unknown { font-weight:900; }; f\6f nt-weight:600">Text</p>"#;
    let document = open(source);
    let properties = direct(&document, 0);
    assert_eq!(
        properties.font_families,
        Some(vec!["Times New Roman".into(), "serif".into()])
    );
    assert_eq!(properties.open_type_features.unwrap().get("liga"), Some(&0));
    assert_eq!(properties.underline, Some(true));
    assert_eq!(properties.foreground.unwrap().red, 1.0);
    assert_eq!(properties.size, Some(12.0));
    assert_eq!(properties.weight, Some(600));
    assert_eq!(document.source_bytes(), source.as_bytes());
    let document = open("<p style=\"font-family:'unterminated\n; font-size:21pt\">Text</p>");
    assert_eq!(direct(&document, 0).font_families, None);
    assert_eq!(direct(&document, 0).size, Some(21.0));
}

#[test]
fn pre_wrap_uses_the_same_comment_and_important_cascade_as_other_properties() {
    let document=open("<p><span style='white-space:pre-wrap ! /*x*/ important;white-space:normal'> a  b </span></p>");
    assert_eq!(document.text(), " a  b ");
}

#[test]
fn super_and_sub_normalize_against_effective_size_and_reopen_as_exact_points() {
    for (css, expected) in [
        ("vertical-align:super;font-size:30pt", 10.0),
        ("font-size:30pt;vertical-align:super", 10.0),
        ("vertical-align:sub;font-size:30pt", -6.0),
        ("vertical-align:super;vertical-align:2pt", 2.0),
        ("vertical-align:super;vertical-align:baseline", 0.0),
    ] {
        let source = format!("<p><span style='{css}'>Text</span></p>");
        let document = open(&source);
        assert_eq!(direct(&document, 0).baseline_shift, Some(expected));
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let mut document = open("<h2><span style='vertical-align:super'>Text</span></h2>");
    assert_eq!(direct(&document, 0).baseline_shift, Some(22.0 / 3.0));
    use evim_core::document::*;
    let mut heading = document
        .projection()
        .style_sheet()
        .block_style(&"Heading2".into())
        .unwrap()
        .clone();
    heading.character.size = Some(30.0);
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(heading),
            }),
        ))
        .unwrap();
    assert_eq!(direct(&document, 0).baseline_shift, Some(10.0));
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                range,
                properties: CharacterProperties {
                    baseline_shift: Some(4.5),
                    ..Default::default()
                },
            }),
        ))
        .unwrap();
    let reopened = open(&String::from_utf8(document.source_bytes()).unwrap());
    assert_eq!(direct(&reopened, 0).baseline_shift, Some(4.5));
}

#[test]
fn html5_adoption_and_nested_anchors_use_recovered_ancestry() {
    let source="<b>one<p data-x='keep'>two</b>three</p><a style='font-weight:600'>four<a style='font-style:italic'>five</a>six</a>";
    let document = open(source);
    assert_eq!(document.text(), "one\ntwothree\nfourfivesix");
    assert_eq!(direct(&document, 0).weight, Some(700));
    assert_eq!(direct(&document, 4).weight, Some(700));
    assert_eq!(direct(&document, 7).weight, None);
    assert_eq!(direct(&document, 13).weight, Some(600));
    assert_eq!(direct(&document, 17).weight, None);
    assert_eq!(direct(&document, 17).slant, Some(FontSlant::Italic));
    assert_eq!(direct(&document, 21).slant, None);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn html5_foster_parenting_keeps_unique_text_provenance_and_atomic_table_source() {
    let source = "<table data-x='keep'>before<tr><td>cell</td></tr>after</table>tail";
    let mut document = open(source);
    assert_eq!(document.text(), "beforeafter\u{fffc}tail");
    document.replace(0..6, "BEFORE").unwrap();
    assert_eq!(
        document.source_bytes(),
        source.replace("before", "BEFORE").as_bytes()
    );
    assert_eq!(document.text(), "BEFOREafter\u{fffc}tail");
    let before = document.source_bytes();
    assert!(
        document.replace(6..14, "").is_err(),
        "opaque table source cannot be deleted by a visible hull"
    );
    assert_eq!(document.source_bytes(), before);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn html5_script_double_escape_templates_and_foreign_cdata_are_passive() {
    let source="<script><!--<script>inner</script>--></script><template><p>hidden</p></template><svg><![CDATA[foreign]]></svg><p>Visible &NotEqualTilde;</p>";
    let document = open(source);
    assert_eq!(document.text(), "\u{fffc}\nVisible ≂̸");
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn html5_deeply_nested_unsupported_containers_do_not_recurse_on_projection_or_drop() {
    let source = format!("{}Text{}", "<span>".repeat(1200), "</span>".repeat(1200));
    let document = open(&source);
    assert_eq!(document.text(), "Text");
    assert_eq!(document.source_bytes(), source.as_bytes());
}
