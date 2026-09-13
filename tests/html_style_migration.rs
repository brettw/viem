use viem_core::document::*;

// Frozen input from the v1 grammar, independent of the current authoring code.
const LEGACY_CHARACTER: &str = ".viem-c-41 {\n  --viem-style-id: \"A\";\n  --viem-style-name: \"A \\22 name\\22 \";\n  --viem-style-role: \"character\";\n  --viem-prop-character-weight: \"400\";\n  --viem-prop-character-underline: \"false\";\n  --viem-prop-character-letter-spacing: \"0\";\n  font-weight: 400;\n  text-decoration-line: none;\n  letter-spacing: 0pt;\n}\n";
const OPEN_V1: &str = "<style id=\"viem-styles\" data-viem-version=\"1\">";
const OPEN_V2: &str = "<style id=\"viem-styles\" data-viem-version=\"2\">";
const BODY: &str =
    "<body><p data-keep='yes'><span class='viem-c-41'>Words</span></p><!--tail--></body>";

fn edit_character(document: &mut Document, weight: u16) -> CommittedModelTransaction {
    let mut style = document
        .projection()
        .style_sheet()
        .character_style(&"A".into())
        .unwrap()
        .clone();
    style.properties.weight = Some(weight);
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateCharacter(style),
            }),
        ))
        .unwrap()
}

fn encode(source: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Utf16Le => [0xff, 0xfe]
            .into_iter()
            .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
            .collect(),
        _ => unreachable!(),
    }
}

#[test]
fn legacy_open_noop_and_text_edits_keep_stylesheet_bytes_exact() {
    let original = format!("{OPEN_V1}\n{LEGACY_CHARACTER}</style>{BODY}");
    for format in [Format::Html, Format::HtmlSource] {
        let mut document =
            Document::from_bytes(original.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.source_bytes(), original.as_bytes());
        let noop = edit_character(&mut document, 400);
        assert_eq!(noop.summary().kind(), ModelChangeKind::NoOp);
        assert_eq!(document.source_bytes(), original.as_bytes());
        let at = document.text().find("Words").unwrap();
        document.insert(at, "!").unwrap();
        assert_eq!(
            document.source_bytes(),
            original.replace("Words", "!Words").as_bytes()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
    }
}

#[test]
fn explicit_style_edit_migrates_v1_atomically_and_preserves_opaque_source() {
    // This native-looking v2 rule must stay opaque in its original v1 wrapper.
    let opaque = "p {\n  font-size: 77pt;\n}\n/* keep this comment */\n";
    let existing_v2 = format!("{OPEN_V2}\ncode {{\n  font-family: monospace;\n}}\n</style>");
    let hidden = format!("<template>{OPEN_V1}\n{LEGACY_CHARACTER}</style></template>");
    for format in [Format::Html, Format::HtmlSource] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let original = format!(
                "<!doctype html><html><head><meta charset='UTF-8'>{OPEN_V1}\n{LEGACY_CHARACTER}{opaque}</style>{existing_v2}</head>{hidden}{BODY}</html>"
            ).replace('\n', "\r\n");
            let bytes = encode(&original, encoding);
            let mut document = Document::from_bytes(bytes.clone(), encoding, format).unwrap();
            let committed = edit_character(&mut document, 700);
            let saved = document.source_bytes();
            // Source patches reproduce the exact saved artifact, including BOM,
            // original CRLF spelling and every untouched byte between patches.
            let mut patched = bytes.clone();
            for patch in committed.summary().source_patches().iter().rev() {
                patched.splice(patch.range(), patch.replacement().iter().copied());
            }
            assert_eq!(saved, patched);
            let decoded = match encoding {
                Encoding::Utf8 => String::from_utf8(saved.clone()).unwrap(),
                Encoding::Utf16Le => String::from_utf16(
                    &saved[2..]
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
                _ => unreachable!(),
            };
            assert!(decoded.contains(&format!(
                "{OPEN_V1}{}</style>",
                opaque.replace('\n', "\r\n")
            )));
            assert!(decoded.contains(&hidden.replace('\n', "\r\n")));
            assert!(decoded.ends_with(&format!("{BODY}</html>")));
            assert_eq!(decoded.matches(OPEN_V2).count(), 2, "{decoded}");
            let reopened = Document::from_bytes(saved.clone(), encoding, format).unwrap();
            for projection in [document.projection(), reopened.projection()] {
                assert_eq!(
                    projection
                        .style_sheet()
                        .character_style(&"A".into())
                        .unwrap_or_else(|| panic!("lost A in {format:?}/{encoding:?}: {decoded}"))
                        .properties
                        .weight,
                    Some(700)
                );
                assert_eq!(
                    projection
                        .style_sheet()
                        .block_style(&"Paragraph".into())
                        .unwrap()
                        .character
                        .size,
                    Some(14.0)
                );
            }
            let rich = Document::from_bytes(saved.clone(), encoding, Format::Html).unwrap();
            assert_eq!(rich.text(), "Words");
            assert!(rich
                .projection()
                .style_spans()
                .iter()
                .any(|span| span.application == StyleApplication::Named("A".into())));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}

#[test]
fn changing_export_policy_migrates_unchanged_legacy_definitions() {
    let original = format!("{OPEN_V1}\n{LEGACY_CHARACTER}</style>{BODY}");
    for format in [Format::Html, Format::HtmlSource] {
        let mut document =
            Document::from_bytes(original.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let before = document
            .projection()
            .style_sheet()
            .character_style(&"A".into())
            .unwrap()
            .clone();
        document
            .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
                document: document.id(),
                revision: document.revision(),
                enabled: true,
            })
            .unwrap();
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(!saved.contains(OPEN_V1), "{saved}");
        assert!(saved.contains(OPEN_V2));
        assert!(saved.ends_with(BODY));
        let reopened = Document::from_bytes(saved.into_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            reopened
                .projection()
                .style_sheet()
                .character_style(&"A".into()),
            Some(&before)
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
    }
}
