use evim_core::document::*;
fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap()
}
fn direct(document: &Document, at: usize) -> CharacterProperties {
    document
        .projection()
        .style_spans()
        .iter()
        .find_map(|span| match &span.application {
            StyleApplication::Direct(properties) if span.range.contains(&at) => {
                Some(properties.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}
fn apply(document: &mut Document, intent: PersistedStyleIntent) {
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(intent),
        ))
        .unwrap();
}
#[test]
fn ignored_destinations_cannot_change_font_codepages_or_document_defaults() {
    let table = r"{\fonttbl{\f0\fcharset0 Arial;}{\f1\fcharset204 Courier;}}";
    for hidden in [
        r"{\*\unknown\deff1\deflang1041{\fonttbl{\f0\fcharset204 Wrong;}}}",
        r"{\*\unknown{\rtf1\deff1\deflang1041{\fonttbl{\f0\fcharset204 Wrong;}}}}",
        r"{\info{\deff1\deflang1041{\fonttbl{\f0\fcharset204 Wrong;}}}}",
    ] {
        for before in [true, false] {
            let source = format!(
                "{{\\rtf1\\ansi\\deff0\\deflang1033{}{}\\plain Caf\\'e9}}",
                if before { hidden } else { table },
                if before { table } else { hidden }
            );
            let document = open(&source);
            assert_eq!(document.text(), "Café", "{source}");
            assert_eq!(
                direct(&document, 0).font_families,
                Some(vec!["Arial".to_owned()])
            );
            assert_eq!(direct(&document, 0).language.as_deref(), Some("en-US"));
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}
#[test]
fn ignored_color_tables_and_nested_color_destinations_do_not_change_indexes() {
    let source = r"{\rtf1{\*\unknown{\colortbl;\red0\green255\blue0;}}{\colortbl;\red255\green0\blue0;{\*\unknown\red0\green255\blue0;}\red0\green0\blue255;}\cf1 Red {\cf2 Blue}}";
    let document = open(source);
    assert_eq!(document.text(), "Red Blue");
    assert_eq!(
        direct(&document, 0).foreground,
        Some(Color {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0
        })
    );
    assert_eq!(
        direct(&document, 4).foreground,
        Some(Color {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            alpha: 1.0
        })
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn only_document_stylesheet_definitions_supply_named_styles() {
    let source = r"{\rtf1{\*\unknown{\stylesheet{\s0\b Wrong;}{\*\cs7\fs80 Bogus;}}}{\stylesheet{\s0 Normal;}{\*\unknown\s2 Evil;}{\s1\i Real{\*\unknown\b Injected};}}\s1 text}";
    let document = open(source);
    let sheet = document.projection().style_sheet();
    assert_eq!(document.text(), "text");
    assert_eq!(
        sheet
            .block_style_metadata(&"Paragraph".into())
            .unwrap()
            .display_name,
        "Normal"
    );
    assert_eq!(
        sheet
            .block_style_metadata(&"RtfP1".into())
            .unwrap()
            .display_name,
        "Real"
    );
    let properties = &sheet.block_style(&"RtfP1".into()).unwrap().character;
    assert_eq!(properties.weight, None);
    assert_eq!(properties.slant, Some(FontSlant::Italic));
    assert!(sheet.character_style(&"RtfC7".into()).is_none());
    assert!(sheet.block_style(&"RtfP2".into()).is_none());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn formatting_writers_create_owned_header_tables_without_touching_opaque_tables() {
    let hidden = r"{\*\unknown{\fonttbl{\f7\fcharset0 Private;}}{\colortbl;\red7\green8\blue9;}{\stylesheet{\s9 Private;}}}";
    let source = format!("{{\\rtf1{hidden}text}}");
    let mut document = open(&source);
    let selected = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    apply(
        &mut document,
        PersistedStyleIntent::SetDirectCharacterProperties {
            range: selected,
            properties: CharacterProperties {
                font_families: Some(vec!["Georgia".to_owned()]),
                foreground: Some(Color {
                    red: 1.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                }),
                ..Default::default()
            },
        },
    );
    let changed = String::from_utf8(document.source_bytes()).unwrap();
    assert!(changed.contains(hidden));
    assert_eq!(
        direct(&document, 0).font_families,
        Some(vec!["Georgia".to_owned()])
    );
    apply(
        &mut document,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit: StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: "RtfC2".into(),
                    based_on: Some("Character".into()),
                    properties: CharacterProperties {
                        slant: Some(FontSlant::Italic),
                        ..Default::default()
                    },
                },
                metadata: StyleDefinitionMetadata {
                    display_name: "Accent".to_owned(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            },
        },
    );
    let changed = String::from_utf8(document.source_bytes()).unwrap();
    assert!(changed.contains(hidden));
    assert!(open(&changed)
        .projection()
        .style_sheet()
        .character_style(&"RtfC2".into())
        .is_some());
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
