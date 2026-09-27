use viem_core::document::*;

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn lists(document: &Document) -> Vec<String> {
    document
        .projection()
        .style_sheet()
        .block_styles()
        .filter(|style| style.id.is_internal_list())
        .map(|style| style.id.0.clone())
        .collect()
}

fn saved_block(id: &str, role: &str, parent: Option<&str>) -> serde_json::Value {
    serde_json::json!({"id":id,"name":id,"role":role,"based_on":parent,
        "next_paragraph_style":null,"block":{},"character":{}})
}

fn saved_character(id: &str, parent: Option<&str>) -> serde_json::Value {
    serde_json::json!({"id":id,"name":id,"based_on":parent,"properties":{}})
}

#[test]
fn saved_defaults_skip_only_invalid_entries_and_individual_declarations() {
    let mut base = saved_block("Paragraph", "Paragraph", Some("Heading1"));
    base["character"] = serde_json::json!({"size":21,"weight":1001,"slant":"Italic"});
    base["block"] = serde_json::json!({"padding_left":-8,"padding_right":6,"background":"bad color"});
    base["next_paragraph_style"] = "Heading1".into();
    let bad_quote = saved_block("Block quote", "Paragraph", Some("Paragraph"));
    let mut custom = saved_block("Pull quote", "Quote", Some("Block quote"));
    custom["next_paragraph_style"] = "Paragraph".into();
    custom["block"] = serde_json::json!({"padding_left":4});
    let mut emphasis = saved_character("Custom emphasis", None);
    emphasis["properties"] = serde_json::json!({"size":"too big","bold":true,"underline":true});
    let settings = serde_json::json!({"version":1,
        "block_styles":[base,bad_quote,42,custom,{"id":"Malformed","role":42}],
        "character_styles":[emphasis,{"id":false,"name":"Malformed"}]});
    let mut document = open("> Quoted text\n>\n> ~~~\n> code\n> ~~~", Format::Markdown);
    let source = document.source_bytes();
    let text = document.text().to_owned();
    let history = document.history_status();
    let revision = document.revision();
    let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&settings).unwrap()).unwrap();
    let sheet = document.projection().style_sheet();
    let base = sheet.block_style(&"Paragraph".into()).unwrap();
    assert_eq!(base.character.size, Some(FontSize::Points(21.)));
    assert_eq!(base.character.weight, None);
    assert_eq!(base.character.slant, Some(FontSlant::Italic));
    assert_eq!(base.block.padding_left, None);
    assert_eq!(base.block.padding_right, Some(6.));
    assert_eq!(base.block.background, None);
    assert_eq!(base.based_on, None);
    assert_eq!(base.next_paragraph_style, None);
    assert_eq!(sheet.block_style(&"Block quote".into()).unwrap().role, BlockRole::Quote);
    let custom = sheet.block_style(&"Pull quote".into()).unwrap();
    assert_eq!(custom.role, BlockRole::Quote);
    assert_eq!(custom.based_on, Some("Block quote".into()));
    assert_eq!(custom.next_paragraph_style, None);
    assert_eq!(custom.block.padding_left, Some(4.));
    let emphasis = sheet.character_style(&"Custom emphasis".into()).unwrap();
    assert_eq!(emphasis.properties.size, None);
    assert_eq!(emphasis.properties.bold, Some(true));
    assert_eq!(emphasis.properties.underline, Some(true));
    assert!(sheet.block_style(&"Malformed".into()).is_none());
    for property in ["padding_left", "weight", "background", "next_paragraph_style", "properties.size", "role"] {
        assert!(diagnostics.iter().any(|message| message.contains(property)), "{property}: {diagnostics:?}");
    }
    assert_eq!(document.source_bytes(), source);
    assert_eq!(document.text(), text);
    assert_eq!(document.revision(), revision);
    let after = document.history_status();
    assert_eq!(after, HistoryStatus {
        retained_memory_bytes: after.retained_memory_bytes,
        live_state_memory_bytes: after.live_state_memory_bytes,
        additional_history_memory_bytes: after.additional_history_memory_bytes,
        ..history
    });
    viem_core::layout::DocumentLayoutStyles::resolve(document.projection()).unwrap();
}

#[test]
fn saved_defaults_validate_graphs_independently_of_entry_order() {
    let mut blocks = vec![
        saved_block("Forward child", "Paragraph", Some("Forward parent")),
        saved_block("Forward parent", "Paragraph", Some("Paragraph")),
        saved_block("Cycle A", "Quote", Some("Cycle B")),
        saved_block("Cycle B", "Quote", Some("Cycle A")),
    ];
    for index in 0..128 {
        blocks.push(saved_block(&format!("Broken {index:03}"), "Paragraph",
            Some(&format!("Broken {:03}", index + 1))));
    }
    let characters = vec![
        saved_character("Character cycle A", Some("Character cycle B")),
        saved_character("Character cycle B", Some("Character cycle A")),
        saved_character("Missing character parent", Some("Absent")),
    ];
    let mut outcomes = Vec::new();
    for reverse in [false, true] {
        let mut definitions = blocks.clone();
        if reverse { definitions.reverse(); }
        let settings = serde_json::json!({"version":1,"block_styles":definitions,"character_styles":characters});
        let mut document = open("> Text", Format::Markdown);
        let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let sheet = document.projection().style_sheet();
        assert!(sheet.block_style(&"Forward child".into()).is_some());
        assert!(sheet.block_style(&"Forward parent".into()).is_some());
        assert!(sheet.block_style(&"Cycle A".into()).is_none());
        assert!(sheet.block_style(&"Cycle B".into()).is_none());
        assert!(sheet.block_style(&"Broken 000".into()).is_none());
        assert_eq!(sheet.character_style(&"Character cycle A".into()).unwrap().based_on, None);
        assert_eq!(sheet.character_style(&"Character cycle B".into()).unwrap().based_on, Some("Character cycle A".into()));
        assert_eq!(sheet.character_style(&"Missing character parent".into()).unwrap().based_on, None);
        assert!(diagnostics.iter().any(|message| message.contains("inheritance cycle")));
        viem_core::layout::DocumentLayoutStyles::resolve(document.projection()).unwrap();
        outcomes.push((document.export_style_defaults().unwrap(), diagnostics));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn saved_defaults_keep_valid_sibling_arrays_and_reject_invalid_whole_files_atomically() {
    let mut document = open("Text", Format::PlainText);
    let mut valid = saved_character("Kept", None);
    valid["properties"] = serde_json::json!({"bold":true});
    let settings = serde_json::json!({"version":1,"block_styles":42,"character_styles":[valid]});
    let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert!(document.projection().style_sheet().character_style(&"Kept".into()).unwrap().properties.bold.unwrap());
    let mut base = saved_block("Paragraph", "Paragraph", None);
    base["character"] = serde_json::json!({"size":23});
    let settings = serde_json::json!({"version":1,"block_styles":[base],"character_styles":"bad"});
    assert_eq!(document.initialize_style_defaults(&serde_json::to_vec(&settings).unwrap()).unwrap().len(), 1);
    assert_eq!(document.projection().style_sheet().block_style(&"Paragraph".into()).unwrap().character.size, Some(FontSize::Points(23.)));
    let before = document.export_style_defaults().unwrap();
    for invalid in [b"{".as_slice(), b"{}", br#"{"version":99}"#, br#"{"version":"bad"}"#] {
        assert!(document.initialize_style_defaults(invalid).is_err());
        assert_eq!(document.export_style_defaults().unwrap(), before);
    }
    assert_eq!(document.text(), "Text");
}

#[test]
fn saved_character_percentage_inheritance_keeps_each_valid_declaration() {
    let styles = (0..40).map(|index| {
        let parent = (index > 0).then(|| format!("Relative {}", index - 1));
        let mut style = saved_character(&format!("Relative {index}"), parent.as_deref());
        style["properties"] = serde_json::json!({"size":{"percentage":1000}});
        style
    }).collect::<Vec<_>>();
    let mut document = open("Text", Format::PlainText);
    let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&serde_json::json!({
        "version":1,"character_styles":styles
    })).unwrap()).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for index in 0..40 {
        assert_eq!(document.projection().style_sheet().character_style(&StyleId(format!("Relative {index}"))).unwrap().properties.size,
            Some(FontSize::Percentage(1000)));
    }
}

#[test]
fn native_defaults_do_not_replace_authored_font_requests() {
    for (format, source) in [
        (
            Format::Rtf,
            r"{\rtf1\ansi\deff0{\fonttbl{\f0 SF Pro;}}\f0 Text}",
        ),
    ] {
        let document = open(source, format);
        let style = viem_core::layout::DocumentLayoutStyles::semantic_character_at(
            document.projection(),
            0,
            false,
        )
        .unwrap();
        assert_eq!(style.font_families, ["SF Pro"], "{format:?}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn rich_default_list_paragraph_styles_are_assignable_source_backed_and_undoable() {
    for (format, source) in [
        (Format::Rtf, r"{\rtf1 Words{\*\unknown keep}}"),
    ] {
        for level in 1..=3 {
            let mut document = open(source, format);
            document
                .apply_model_request(ModelRequest::AssignNamedStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: 0..0,
                    namespace: StyleNamespace::Block,
                    style: StyleId(format!("BulletedList{level}")),
                })
                .unwrap();
            assert_eq!(document.text(), "Words");
            let saved = String::from_utf8(document.source_bytes()).unwrap();
            let reopened = open(&saved, format);
            let id = &reopened.projection().blocks()[0].style;
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style(id)
                    .unwrap()
                    .block
                    .leading_indent,
                Some(0.0)
            );
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style_metadata(id)
                    .unwrap()
                    .origin,
                StyleDefinitionOrigin::SourceBacked
            );
            assert!(saved.contains("keep"));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn defaults_define_two_four_level_families_independent_of_used_depth() {
    let expected = [
        "BulletedList1",
        "BulletedList2",
        "BulletedList3",
        "BulletedList4",
        "NumberedList1",
        "NumberedList2",
        "NumberedList3",
        "NumberedList4",
    ];
    for format in [
        Format::PlainText,
        Format::Markdown,
        Format::MarkdownSource,
        Format::Rtf,
    ] {
        assert_eq!(lists(&open("", format)), expected, "{format:?}");
    }
    let markdown = (0..20)
        .map(|depth| format!("{}- Text", "  ".repeat(depth)))
        .collect::<Vec<_>>()
        .join("\n");
    for (format, source) in [(Format::Markdown, markdown),] {
        let document = open(&source, format);
        assert_eq!(lists(&document), expected);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            document.projection().blocks().last().unwrap().style,
            StyleId::from("BulletedList4")
        );
    }
}

#[test]
fn editing_list_depth_reuses_fourth_style_and_undo_restores_source() {
    let original = "- One\n  - Two\n    - Three";
    let mut document = open(original, Format::MarkdownSource);
    let styles = lists(&document);
    let end = document.text().len();
    document.insert(end, "\n      - Four").unwrap();
    assert_eq!(lists(&document), styles);
    assert_eq!(
        document.projection().blocks().last().unwrap().style,
        StyleId::from("BulletedList4")
    );
    assert!(document.undo());
    assert_eq!(lists(&document), styles);
    assert_eq!(document.source_bytes(), original.as_bytes());
}

fn clear_definition(document: &mut Document, id: &StyleId, character: bool) {
    let sheet = document.projection().style_sheet();
    let (edit, origin) = if character {
        let mut style = sheet.character_style(id).unwrap().clone();
        style.properties = CharacterProperties::default();
        (
            StyleDefinitionEdit::UpdateCharacter(style),
            sheet.character_style_metadata(id).unwrap().origin,
        )
    } else {
        let mut style = sheet.block_style(id).unwrap().clone();
        style.character = CharacterProperties::default();
        style.block = BlockProperties::default();
        (
            StyleDefinitionEdit::UpdateBlock(style),
            sheet.block_style_metadata(id).unwrap().origin,
        )
    };
    let intent = if origin == StyleDefinitionOrigin::GeneratedConfiguration {
        StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit))
    } else {
        StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition { origin, edit })
    };
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            intent,
        ))
        .unwrap();
}

fn assert_inherits_base(document: &Document, id: &StyleId, character: bool) {
    let sheet = document.projection().style_sheet();
    let resolve = |paragraph: &StyleId, character| {
        sheet
            .resolve_assigned_paragraph_style(
                document.projection().document_style(),
                paragraph,
                &BlockProperties::default(),
                &CharacterProperties::default(),
                character,
                &CharacterProperties::default(),
            )
            .unwrap()
    };
    let mut base = resolve(&sheet.base_paragraph, None);
    if !character && sheet.block_style(id).is_some_and(|style| style.role.is_container()) {
        base = ResolvedParagraphStyle { character: base.character, ..Default::default() };
    }
    let actual = if character {
        assert_eq!(
            sheet.character_style(id).unwrap().properties,
            CharacterProperties::default()
        );
        resolve(&sheet.base_paragraph, Some(id))
    } else {
        let style = sheet.block_style(id).unwrap();
        assert_eq!(style.character, CharacterProperties::default());
        assert_eq!(style.block, BlockProperties::default());
        resolve(id, None)
    };
    assert_eq!(actual, base, "{}", id.0);
}

#[test]
fn loading_defaults_keeps_all_builtin_declarations_visible() {
    for format in [
        Format::PlainText,
        Format::Markdown,
        Format::Rtf,
    ] {
        let mut document = open("", format);
        let before = document.projection().style_sheet().clone();
        document
            .initialize_style_defaults(br#"{"version":1}"#)
            .unwrap();
        let after = document.projection().style_sheet();
        for style in before.block_styles() {
            assert_eq!(
                after.block_style(&style.id),
                Some(style),
                "{format:?} {}",
                style.id.0
            );
        }
        for style in before.character_styles() {
            assert_eq!(
                after.character_style(&style.id),
                Some(style),
                "{format:?} {}",
                style.id.0
            );
        }
    }
}

#[test]
fn clearing_all_builtin_deltas_inherits_base_and_survives_settings_reopen() {
    let original = open("# Title\n\nText", Format::Markdown);
    let original_defaults = original.export_style_defaults().unwrap();
    let sheet = original.projection().style_sheet();
    let ids = sheet
        .block_styles()
        .filter(|style| style.id != sheet.base_paragraph)
        .map(|style| (style.id.clone(), false))
        .chain(
            sheet
                .character_styles()
                .map(|style| (style.id.clone(), true)),
        )
        .collect::<Vec<_>>();
    for (id, character) in ids {
        let mut document = open("# Title\n\nText", Format::Markdown);
        document
            .initialize_style_defaults(&original_defaults)
            .unwrap();
        let before = document.projection().style_sheet().clone();
        clear_definition(&mut document, &id, character);
        let changed = document.projection().style_sheet() != &before;
        assert_inherits_base(&document, &id, character);
        let settings = document.export_style_defaults().unwrap();
        document.insert(document.text().len(), " appended").unwrap();
        assert_inherits_base(&document, &id, character);
        assert!(document.undo());
        if changed { assert!(document.undo()); }
        assert_eq!(document.projection().style_sheet(), &before);
        if changed { assert!(document.redo()); }
        assert_inherits_base(&document, &id, character);
        let mut reopened = open("# Title\n\nText", Format::Markdown);
        reopened.initialize_style_defaults(&settings).unwrap();
        assert_inherits_base(&reopened, &id, character);
    }
}
