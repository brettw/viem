use viem_core::document::*;

const OPEN_V1: &str = "<style id=\"viem-styles\" data-viem-version=\"1\">";
const OPEN_V2: &str = "<style id=\"viem-styles\" data-viem-version=\"2\">";
// Frozen import fixture: this deliberately does not call the current writer.
const LEGACY_A: &str = r#".viem-c-41 {
  --viem-style-id: "A";
  --viem-style-name: "A \22 name\22 ";
  --viem-style-role: "character";
  --viem-based-on: "Character";
  --viem-prop-character-weight: "400";
  --viem-prop-character-underline: "false";
  --viem-prop-character-letter-spacing: "0";
  font-weight: 400;
  text-decoration-line: none;
  letter-spacing: 0pt;
}
"#;
const OPAQUE_OVERRIDE: &str = ".viem-c-41 {\n  font-weight: 100;\n}\n";
const BODY: &str = "<body><p><span class='viem-c-41'>First</span> <span class='viem-c-42'>Second</span></p><!--tail--></body>";

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn edit(document: &mut Document, change: StyleDefinitionEdit) -> CommittedModelTransaction {
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: change,
            }),
        ))
        .unwrap()
}

fn character(document: &Document, id: &str) -> CharacterStyle {
    document
        .projection()
        .style_sheet()
        .character_style(&id.into())
        .unwrap()
        .clone()
}

fn assert_noop_is_exact(document: &mut Document, source: &str) {
    assert_eq!(document.source_bytes(), source.as_bytes());
    let unchanged = character(document, "A");
    let result = edit(document, StyleDefinitionEdit::UpdateCharacter(unchanged));
    assert_eq!(result.summary().kind(), ModelChangeKind::NoOp);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

fn assert_reopen_and_history(document: &mut Document, original: &str, format: Format) {
    let saved = document.source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
    for style in document.projection().style_sheet().character_styles() {
        assert_eq!(
            reopened
                .projection()
                .style_sheet()
                .character_style(&style.id),
            Some(style),
        );
    }
    assert_eq!(reopened.source_bytes(), saved);
    assert_eq!(reopened.text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
}

fn assert_opaque_stays_v1(saved: &str, opaque: &str) {
    let at = saved
        .find(opaque)
        .unwrap_or_else(|| panic!("lost opaque bytes: {saved}"));
    let opening = saved[..at].rfind("<style ").unwrap();
    assert!(saved[opening..].starts_with(OPEN_V1), "{saved}");
    assert!(!saved[opening..at].contains("</style>"), "{saved}");
}

#[test]
fn migration_keeps_untouched_rule_before_opaque_override_and_later_v2_sheet() {
    let later_v2 = format!("{OPEN_V2}\ncode {{\n  font-size: 18pt;\n}}\n</style>");
    let opaque_sheet = format!("<style data-owner='external'>{OPAQUE_OVERRIDE}</style>");
    let source = format!(
        "<html><head>{OPEN_V1}\n{LEGACY_A}</style>{opaque_sheet}{later_v2}</head>{BODY}</html>"
    );
    for format in [Format::Html, Format::HtmlSource] {
        let mut document = open(&source, format);
        assert_noop_is_exact(&mut document, &source);
        let unchanged_a = character(&document, "A");
        edit(
            &mut document,
            StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: "B".into(),
                    based_on: Some("Character".into()),
                    properties: CharacterProperties {
                        weight: Some(700),
                        ..Default::default()
                    },
                },
                metadata: StyleDefinitionMetadata {
                    display_name: "B".into(),
                    origin: StyleDefinitionOrigin::SourceBacked,
                },
            },
        );
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert_eq!(character(&document, "A"), unchanged_a);
        let adopted_a = saved.find("--viem-style-id: \"A\"").unwrap();
        let opaque = saved.find(&opaque_sheet).unwrap();
        let later = saved.find("code {\n  font-size: 18pt;\n}").unwrap();
        assert!(adopted_a < opaque && opaque < later, "{saved}");
        assert!(!saved.contains(OPEN_V1), "{saved}");
        assert!(saved.ends_with(&format!("{BODY}</html>")));
        assert_reopen_and_history(&mut document, &source, format);
    }
}

#[test]
fn mixed_legacy_sheet_splits_in_place_around_each_opaque_gap() {
    let legacy_b = LEGACY_A
        .replace(".viem-c-41", ".viem-c-42")
        .replace("--viem-style-id: \"A\"", "--viem-style-id: \"B\"");
    let prefix = ".before {\n  color: teal;\n}\n";
    let suffix = ".after {\n  color: orange;\n}\n";
    for format in [Format::Html, Format::HtmlSource] {
        for (before, after) in [("", ""), (prefix, ""), ("", suffix), (prefix, suffix)] {
            let source = format!("<html><head>{OPEN_V1}\n{before}{LEGACY_A}{OPAQUE_OVERRIDE}{legacy_b}{after}</style></head>{BODY}</html>");
            let mut document = open(&source, format);
            assert_noop_is_exact(&mut document, &source);
            let unchanged_a = character(&document, "A");
            let mut changed_b = character(&document, "B");
            changed_b.properties.weight = Some(700);
            edit(
                &mut document,
                StyleDefinitionEdit::UpdateCharacter(changed_b),
            );
            let saved = String::from_utf8(document.source_bytes()).unwrap();
            assert_eq!(character(&document, "A"), unchanged_a);
            assert_eq!(character(&document, "B").properties.weight, Some(700));
            let adopted_a = saved.find("--viem-style-id: \"A\"").unwrap();
            let opaque = saved.find(OPAQUE_OVERRIDE).unwrap();
            let adopted_b = saved.find("--viem-style-id: \"B\"").unwrap();
            assert!(adopted_a < opaque && opaque < adopted_b, "{saved}");
            if !before.is_empty() {
                assert!(saved.find(before).unwrap() < adopted_a, "{saved}");
            }
            if !after.is_empty() {
                assert!(adopted_b < saved.find(after).unwrap(), "{saved}");
            }
            for opaque in [before, OPAQUE_OVERRIDE, after]
                .into_iter()
                .filter(|text| !text.is_empty())
            {
                assert_opaque_stays_v1(&saved, opaque);
            }
            assert_eq!(saved.matches(OPEN_V2).count(), 2, "{saved}");
            assert!(saved.ends_with(&format!("{BODY}</html>")));
            assert_reopen_and_history(&mut document, &source, format);
        }
    }
}
