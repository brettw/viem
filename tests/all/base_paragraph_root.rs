use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::*;

#[test]
fn base_paragraph_rejects_next_style_edits_without_publishing_changes() {
    for (format, source) in [
        (Format::Markdown, "Text"),
        (Format::Html, "<p>Text</p><!--keep-->"),
        (Format::Rtf, r"{\rtf1{\stylesheet{\s0 Normal;}}\s0 Text}"),
    ] {
        for next in ["Heading1", "Paragraph"] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let before = document.projection().style_sheet().clone();
            let history = document.history_status();
            let revision = document.revision();
            let mut root = before.block_style(&before.base_paragraph).unwrap().clone();
            root.next_paragraph_style = Some(next.into());
            let edit = StyleDefinitionEdit::UpdateBlock(root);
            let intent = if format.has_rich_source() {
                StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                    origin: StyleDefinitionOrigin::SourceBacked,
                    edit,
                })
            } else {
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit))
            };
            assert!(document.apply_style_request(StyleModelRequest::new(
                document.id(), revision, intent,
            )).is_err(), "{format:?}: {next}");
            assert_eq!(document.revision(), revision);
            assert_eq!(document.projection().style_sheet(), &before);
            assert_eq!(document.history_status(), history);
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn native_base_paragraph_next_style_is_same_without_rewriting_source() {
    let source = br"{\rtf1\deff3{\fonttbl{\f3 Georgia;}}{\stylesheet{\s0\snext5 Normal;}{\s5\sbasedon0\snext0\b Heading;}}\s0 Text{\*\unknown keep}}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let sheet = document.projection().style_sheet();
    let root = sheet.block_style(&sheet.base_paragraph).unwrap();
    assert_eq!(root.based_on, None);
    assert_eq!(root.next_paragraph_style, None);
    assert_eq!(sheet.next_paragraph_style(&root.id).unwrap(), &root.id);
    assert_eq!(sheet.next_paragraph_style(&"RtfP5".into()).unwrap(), &root.id);
    let effective = sheet.resolve_paragraph_style(
        &root.id, &root.id, None, &BlockProperties::default(), &CharacterProperties::default(),
    ).unwrap();
    assert_eq!(root.character.size, None, "Native declarations remain sparse");
    assert_eq!(effective.character.size, 12.0);
    assert_eq!(effective.character.font_families, vec!["Georgia"]);
    assert_eq!(effective.character.weight, 400);
    assert_eq!(effective.character.slant, FontSlant::Upright);
    assert_eq!(effective.line_spacing, LineSpacing::Normal);
    assert_eq!(effective.alignment, ParagraphAlignment::Start);
    assert_eq!(document.source_bytes(), source);

    let mut commands = CommandInterpreter::new();
    for event in [InputEvent::key('A'), InputEvent::Key(Key::Enter), InputEvent::text("Body")] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "Text\nBody");
    assert!(document.projection().blocks().iter().all(|block| block.style == "Paragraph".into()));
    assert!(String::from_utf8_lossy(&document.source_bytes()).contains(r"\s0\snext5 Normal;"));
    commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
    commands.handle(&mut document, InputEvent::key('u')).unwrap();
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn user_defaults_cannot_change_base_paragraph_next_style() {
    let mut document = Document::new("Text");
    let before = document.export_style_defaults().unwrap();
    let mut defaults: serde_json::Value = serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    let root = defaults["block_styles"].as_array_mut().unwrap().iter_mut()
        .find(|style| style["id"] == "Paragraph").unwrap();
    root["next_paragraph_style"] = "Heading1".into();
    let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap();
    assert!(diagnostics.iter().any(|message| message.contains("Paragraph") && message.contains("next_paragraph_style")));
    assert_eq!(document.export_style_defaults().unwrap(), before);
    assert_eq!(document.source_bytes(), b"Text");
}
