use evim_core::document::*;

fn apply(
    document: &mut Document,
    intent: PersistedStyleIntent,
) -> Result<CommittedModelTransaction, ModelTransactionError> {
    document.apply_style_request(StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Persisted(intent),
    ))
}
fn definition(
    document: &mut Document,
    edit: StyleDefinitionEdit,
) -> Result<CommittedModelTransaction, ModelTransactionError> {
    apply(
        document,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit,
        },
    )
}
fn metadata(name: &str) -> StyleDefinitionMetadata {
    StyleDefinitionMetadata {
        display_name: name.into(),
        origin: StyleDefinitionOrigin::SourceBacked,
    }
}

#[test]
fn deleting_each_builtin_heading_persists_default_assignment_and_undo() {
    for level in 1..=6 {
        let id = StyleId(format!("Heading{level}"));
        let hidden = format!("<template><h{level} data-hidden='keep'>Hidden</h{level}></template>");
        let original = format!("<!doctype html><h{level} data-keep='x' style='text-align:center'>Head<br>line</h{level}><p>Tail</p><!--keep-->{hidden}");
        let mut document = html(&original);
        enable_style_export(&mut document);
        let before_delete = document.source_bytes();
        let ids = document
            .projection()
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>();
        definition(&mut document, StyleDefinitionEdit::DeleteBlock(id.clone())).unwrap();
        assert_eq!(document.text(), "Head\nline\nTail");
        assert!(document
            .projection()
            .style_sheet()
            .block_style(&id)
            .is_none());
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>(),
            ids
        );
        let block = &document.projection().blocks()[0];
        assert_eq!(block.style, StyleId::from("Paragraph"));
        assert_eq!(
            block.direct_paragraph.alignment,
            Some(ParagraphAlignment::Center)
        );
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(saved.contains("--evim-style-deleted: \"true\";"));
        assert!(saved.contains(&format!("<h{level} data-keep='x' style='text-align:center' class=\"evim-p-506172616772617068\">Head<br>line</h{level}>")), "{saved}");
        assert!(saved.ends_with(&format!("<p>Tail</p><!--keep-->{hidden}")));
        let reopened = html(&saved);
        assert_eq!(reopened.source_bytes(), saved.as_bytes());
        assert!(reopened
            .projection()
            .style_sheet()
            .block_style(&id)
            .is_none());
        assert_eq!(
            reopened.projection().blocks()[0].style,
            StyleId::from("Paragraph")
        );
        document.replace(1..2, "E").unwrap();
        assert_eq!(document.text(), "HEad\nline\nTail");
        assert!(document
            .projection()
            .style_sheet()
            .block_style(&id)
            .is_none());
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Paragraph")
        );
        assert_eq!(
            document.projection().blocks()[0].direct_paragraph.alignment,
            Some(ParagraphAlignment::Center)
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), saved.as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before_delete);
        assert_eq!(document.projection().blocks()[0].style, id);
    }
}

#[test]
fn deleted_heading_rules_follow_default_edits_and_explicit_recreation() {
    let mut document = html("<h1>Title</h1><p>Body</p><!--keep-->");
    enable_style_export(&mut document);
    let heading = document
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .unwrap()
        .clone();
    definition(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: "Child".into(),
                based_on: Some("Heading1".into()),
                next_paragraph_style: Some("Heading1".into()),
                role: BlockRole::Paragraph,
                character: Default::default(),
                block: Default::default(),
            },
            metadata: metadata("Child"),
        },
    )
    .unwrap();
    definition(
        &mut document,
        StyleDefinitionEdit::DeleteBlock("Heading1".into()),
    )
    .unwrap();
    let child = document
        .projection()
        .style_sheet()
        .block_style(&"Child".into())
        .unwrap();
    assert_eq!(child.based_on, Some("Paragraph".into()));
    assert_eq!(child.next_paragraph_style, Some("Paragraph".into()));
    let mut paragraph = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    paragraph.character.size = Some(21.0);
    definition(&mut document, StyleDefinitionEdit::UpdateBlock(paragraph)).unwrap();
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    let reopened = html(&saved);
    assert!(reopened
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .is_none());
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .block_style(&"Paragraph".into())
            .unwrap()
            .character
            .size,
        Some(21.0)
    );
    definition(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: heading,
            metadata: metadata("Heading 1"),
        },
    )
    .unwrap();
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(!saved.contains("--evim-style-deleted"));
    let reopened = html(&saved);
    assert!(reopened
        .projection()
        .style_sheet()
        .block_style(&"Heading1".into())
        .is_some());
    assert_eq!(
        reopened.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    assert!(saved.ends_with("<p>Body</p><!--keep-->"));
}

#[test]
fn base_paragraph_relative_bold_preserves_selected_system_light_face() {
    let mut document = html("<p>Words</p>");
    enable_style_export(&mut document);
    let mut paragraph = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    paragraph.character.font_families = Some(vec![".SFNS-Light".into()]);
    paragraph.character.weight = Some(274);
    definition(
        &mut document,
        StyleDefinitionEdit::UpdateBlock(paragraph.clone()),
    )
    .unwrap();
    paragraph.character.bold = Some(true);
    definition(&mut document, StyleDefinitionEdit::UpdateBlock(paragraph)).unwrap();
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    let reopened = html(&saved);
    let paragraph = reopened
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap();
    assert_eq!(
        paragraph.character.font_families,
        Some(vec![".SFNS-Light".into()])
    );
    assert_eq!(paragraph.character.weight, Some(274));
    assert_eq!(paragraph.character.bold, Some(true));
}

#[test]
fn deleted_heading_rules_stay_passive_in_opaque_content_and_bases_remain_protected() {
    let mut seed = html("<h1>Title</h1>");
    enable_style_export(&mut seed);
    definition(
        &mut seed,
        StyleDefinitionEdit::DeleteBlock("Heading1".into()),
    )
    .unwrap();
    let saved = String::from_utf8(seed.source_bytes()).unwrap();
    let sheet = &saved[..saved.find("<h1").unwrap()];
    for source in [
        format!("<template>{sheet}</template><h1>Title</h1>"),
        format!("{}<h1>Title</h1>", sheet.replace("\nh1 {\n", "\nh01 {\n")),
    ] {
        let document = html(&source);
        assert!(document
            .projection()
            .style_sheet()
            .block_style(&"Heading1".into())
            .is_some());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    for edit in [
        StyleDefinitionEdit::DeleteBlock("Paragraph".into()),
        StyleDefinitionEdit::DeleteBlock("Document".into()),
        StyleDefinitionEdit::DeleteCharacter("Character".into()),
    ] {
        let before = seed.source_bytes();
        assert!(definition(&mut seed, edit).is_err());
        assert_eq!(seed.source_bytes(), before);
    }
}
fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn enable_style_export(document: &mut Document) {
    document
        .apply_model_request(ModelRequest::SetIncludeStyleDefinitionsInFile {
            document: document.id(),
            revision: document.revision(),
            enabled: true,
        })
        .unwrap();
}

#[test]
fn owned_styles_in_templates_atomic_content_and_script_text_are_not_adopted_or_edited() {
    let mut seed = html("<p>Body</p>");
    enable_style_export(&mut seed);
    let mut paragraph = seed
        .projection()
        .style_sheet()
        .block_style(&StyleId::from("Paragraph"))
        .unwrap()
        .clone();
    paragraph.character.size = Some(40.0);
    definition(&mut seed, StyleDefinitionEdit::UpdateBlock(paragraph)).unwrap();
    let seed = String::from_utf8(seed.source_bytes()).unwrap();
    let sheet = &seed[..seed.find("<p>").unwrap()];
    // The semantic traversal balances void elements too; a preceding opaque
    // image must not suppress a later, independently active owned sheet.
    let after_void = html(&format!("<img>{sheet}<p>Body</p>"));
    assert_eq!(
        after_void
            .projection()
            .style_sheet()
            .block_style(&StyleId::from("Paragraph"))
            .unwrap()
            .character
            .size,
        Some(40.0)
    );
    for hidden in [
        format!("<template><head>{sheet}</head></template>"),
        format!("<svg>{sheet}</svg>"),
        format!("<script><!--<script></script>{sheet}</script>"),
    ] {
        let original = format!("{hidden}<p>Body</p>");
        let mut document = html(&original);
        assert_eq!(
            document
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Paragraph"))
                .unwrap()
                .character
                .size,
            None,
            "{hidden}"
        );
        enable_style_export(&mut document);
        let before_edits = document.source_bytes();
        let text = document.text().to_owned();
        for size in [18.0, 22.0] {
            let mut paragraph = document
                .projection()
                .style_sheet()
                .block_style(&StyleId::from("Paragraph"))
                .unwrap()
                .clone();
            paragraph.character.size = Some(size);
            definition(&mut document, StyleDefinitionEdit::UpdateBlock(paragraph)).unwrap();
            assert_eq!(document.text(), text);
            let source = String::from_utf8(document.source_bytes()).unwrap();
            assert!(
                source.ends_with(&original),
                "hidden bytes must stay exact: {source}"
            );
            assert_eq!(source.matches("<style id=\"evim-styles\"").count(), 2);
            let reopened = html(&source);
            assert_eq!(
                reopened
                    .projection()
                    .style_sheet()
                    .block_style(&StyleId::from("Paragraph"))
                    .unwrap()
                    .character
                    .size,
                Some(size)
            );
        }
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before_edits);
    }
}

#[test]
fn assigning_a_named_style_to_an_empty_html_buffer_preserves_owned_style_placement() {
    let mut document = html("");
    definition(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: "Custom".into(),
                based_on: Some("Paragraph".into()),
                next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: Default::default(),
                block: Default::default(),
            },
            metadata: metadata("Custom"),
        },
    )
    .unwrap();
    document
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 0..0,
            namespace: StyleNamespace::Block,
            style: "Custom".into(),
        })
        .unwrap();
    assert_eq!(document.text(), "");
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Custom")
    );
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.starts_with("<style id=\"evim-styles\""));
    assert!(source.ends_with("<p class=\"evim-p-437573746f6d\"></p>"));
}

#[test]
fn anonymous_paragraph_assignment_adds_minimal_wrappers_around_original_inline_source() {
    for body in [
        "Words<!--tail-->",
        "<b data-x='keep'>Words</b><!--tail-->",
        "<div data-block='keep'><b data-x='keep'>Words</b><!--tail--></div>",
    ] {
        let mut document = html(body);
        definition(
            &mut document,
            StyleDefinitionEdit::InsertBlock {
                style: BlockStyle {
                    id: "Custom".into(),
                    based_on: Some("Paragraph".into()),
                    next_paragraph_style: None,
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(22.0),
                        ..Default::default()
                    },
                    block: Default::default(),
                },
                metadata: metadata("Custom"),
            },
        )
        .unwrap();
        let before = document.source_bytes();
        document
            .apply_model_request(ModelRequest::AssignNamedStyle {
                document: document.id(),
                revision: document.revision(),
                range: 0..0,
                namespace: StyleNamespace::Block,
                style: "Custom".into(),
            })
            .unwrap();
        let source = String::from_utf8(document.source_bytes()).unwrap();
        assert_eq!(document.text(), "Words");
        assert!(source.contains("<p class=\"evim-p-437573746f6d\">"));
        assert!(source.contains("</p><!--tail-->"));
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Custom")
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before);
        document
            .apply_model_request(ModelRequest::SetParagraphStyle {
                document: document.id(),
                revision: document.revision(),
                range: 0..0,
                style: "Heading2".into(),
            })
            .unwrap();
        let source = String::from_utf8(document.source_bytes()).unwrap();
        assert!(source.contains("<h2>"));
        assert!(source.contains("</h2><!--tail-->"));
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Heading2")
        );
    }
}

#[test]
fn character_assignment_spans_existing_named_and_semantic_wrappers_without_touching_them() {
    let mut document=html("<p><span class='evim-c-41' data-x='keep'>ab</span><b foo='bar'>x</b><span class='evim-c-41'>cd</span></p>");
    for id in ["A", "B"] {
        definition(
            &mut document,
            StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: id.into(),
                    based_on: Some("Character".into()),
                    properties: CharacterProperties {
                        size: Some(if id == "A" { 12.0 } else { 20.0 }),
                        ..Default::default()
                    },
                },
                metadata: metadata(id),
            },
        )
        .unwrap();
    }
    let before = document.source_bytes();
    let range = TextRange::new(
        document.text_point(1).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    let committed = apply(
        &mut document,
        PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: "B".into(),
        },
    )
    .unwrap();
    assert_eq!(committed.summary().source_patches().len(), 6);
    assert_eq!(document.text(), "abxcd");
    let after = String::from_utf8(document.source_bytes()).unwrap();
    assert!(after.contains("<span class='evim-c-41' data-x='keep'>a<span class=\"evim-c-42\">b</span></span><b foo='bar'><span class=\"evim-c-42\">x</span></b><span class='evim-c-41'><span class=\"evim-c-42\">c</span>d</span>"));
    for offset in 1..4 {
        assert!(document
            .projection()
            .style_spans()
            .iter()
            .any(|span| span.range.contains(&offset)
                && span.application == StyleApplication::Named("B".into())));
    }
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
}

#[test]
fn canonical_character_styles_preserve_sparse_properties_ids_and_rule_locality() {
    let original="<!doctype html><html><head><meta charset='utf-8'><style>.foreign { color: red }</style></head><body><p data-id='keep'>Alpha &amp; Beta</p></body></html>";
    let mut document = html(original);
    let id: StyleId = "Accent α".into();
    definition(
        &mut document,
        StyleDefinitionEdit::InsertCharacter {
            style: CharacterStyle {
                id: id.clone(),
                based_on: Some("Character".into()),
                properties: CharacterProperties {
                    weight: Some(700),
                    underline: Some(false),
                    letter_spacing: Some(0.0),
                    ..Default::default()
                },
            },
            metadata: metadata("Accent \"quoted\" <name>"),
        },
    )
    .unwrap();
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(
        source.contains("<meta charset='utf-8'><style id=\"evim-styles\" data-evim-version=\"2\">")
    );
    assert!(source.contains(".evim-c-416363656e7420ceb1 {\n"));
    assert!(source.contains("  --evim-prop-character-underline: \"false\";\n"));
    assert!(source.contains("  --evim-prop-character-letter-spacing: \"0\";\n"));
    assert!(source.contains("<style>.foreign { color: red }</style>"));
    assert!(source.contains("<p data-id='keep'>Alpha &amp; Beta</p>"));
    let reopened = html(&source);
    assert_eq!(
        reopened.projection().style_sheet().character_style(&id),
        document.projection().style_sheet().character_style(&id)
    );
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .character_style_metadata(&id)
            .unwrap()
            .display_name,
        "Accent \"quoted\" <name>"
    );
    let before = document.source_bytes();
    let committed = definition(
        &mut document,
        StyleDefinitionEdit::UpdateMetadata {
            namespace: StyleNamespace::Character,
            id: id.clone(),
            metadata: metadata("Renamed"),
        },
    )
    .unwrap();
    assert_eq!(committed.summary().source_patches().len(), 1);
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(".evim-c-416363656e7420ceb1"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
}

#[test]
fn named_parent_edits_update_only_dependent_materialized_css_rules() {
    let mut document = html("<p>Text</p>");
    let parent: StyleId = "Parent".into();
    let child: StyleId = "Child".into();
    let unrelated: StyleId = "Other".into();
    for (id, based_on, properties) in [
        (
            parent.clone(),
            Some("Character".into()),
            CharacterProperties {
                weight: Some(400),
                ..Default::default()
            },
        ),
        (
            child.clone(),
            Some(parent.clone()),
            CharacterProperties {
                underline: Some(true),
                ..Default::default()
            },
        ),
        (
            unrelated.clone(),
            Some("Character".into()),
            CharacterProperties {
                slant: Some(FontSlant::Italic),
                ..Default::default()
            },
        ),
    ] {
        definition(
            &mut document,
            StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id,
                    based_on,
                    properties,
                },
                metadata: metadata("Name"),
            },
        )
        .unwrap();
    }
    let mut style = document
        .projection()
        .style_sheet()
        .character_style(&parent)
        .unwrap()
        .clone();
    style.properties.weight = Some(800);
    let committed = definition(&mut document, StyleDefinitionEdit::UpdateCharacter(style)).unwrap();
    assert_eq!(committed.summary().source_patches().len(), 2);
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert_eq!(source.matches("font-weight: 800;").count(), 2);
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .character_style(&child)
            .unwrap()
            .properties
            .weight,
        None
    );
    let reopened = html(&source);
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .character_style(&child)
            .unwrap()
            .based_on,
        Some(parent)
    );
}

#[test]
fn first_applicable_class_wins_and_assigning_preserves_other_attributes() {
    let mut document = html("<p class='foreign' data-keep='yes'>Alpha</p><p>Beta</p>");
    for (id, size) in [("First", 18.0), ("Second", 30.0)] {
        definition(
            &mut document,
            StyleDefinitionEdit::InsertBlock {
                style: BlockStyle {
                    id: id.into(),
                    based_on: Some("Paragraph".into()),
                    next_paragraph_style: Some("Paragraph".into()),
                    role: BlockRole::Paragraph,
                    character: CharacterProperties {
                        size: Some(size),
                        ..Default::default()
                    },
                    block: Default::default(),
                },
                metadata: metadata(id),
            },
        )
        .unwrap();
    }
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    apply(
        &mut document,
        PersistedStyleIntent::AssignBlockStyle {
            target: StyleBlockTarget::Paragraphs(range),
            style: "First".into(),
        },
    )
    .unwrap();
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("First")
    );
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.contains("class=\"evim-p-4669727374 foreign\" data-keep='yes'"));
    let both = source.replace(
        "evim-p-4669727374 foreign",
        "foreign evim-p-5365636f6e64 evim-p-4669727374",
    );
    assert_eq!(
        html(&both).projection().blocks()[0].style,
        StyleId::from("Second")
    );
    assert!(document.undo());
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
}

#[test]
fn character_assignments_use_named_layers_and_inline_overrides_win() {
    let mut document = html("<p><b>Alpha</b> Beta</p>");
    let id: StyleId = "Quiet".into();
    definition(
        &mut document,
        StyleDefinitionEdit::InsertCharacter {
            style: CharacterStyle {
                id: id.clone(),
                based_on: Some("Character".into()),
                properties: CharacterProperties {
                    weight: Some(400),
                    foreground: Some(Color {
                        red: 1.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    }),
                    ..Default::default()
                },
            },
            metadata: metadata("Quiet"),
        },
    )
    .unwrap();
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(5).unwrap(),
    )
    .unwrap();
    apply(
        &mut document,
        PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: id.clone(),
        },
    )
    .unwrap();
    assert!(document.projection().style_spans().iter().any(
        |span| span.range == (0..5) && span.application == StyleApplication::Named(id.clone())
    ));
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.contains("<b><span class=\"evim-c-5175696574\">Alpha</span></b>"));
    let inline = source.replace(
        "class=\"evim-c-5175696574\"",
        "class=\"evim-c-5175696574\" style='font-weight: 800'",
    );
    let parsed = html(&inline);
    assert!(parsed.projection().style_spans().iter().any(|span|matches!(&span.application,StyleApplication::Direct(properties)if properties.weight==Some(800))));
    assert!(document.undo());
    assert!(!document
        .projection()
        .style_spans()
        .iter()
        .any(|span| matches!(span.application, StyleApplication::Named(_))));
    let before = document.source_bytes();
    definition(
        &mut document,
        StyleDefinitionEdit::DeleteCharacter(id.clone()),
    )
    .unwrap();
    assert!(document
        .projection()
        .style_sheet()
        .character_style(&id)
        .is_none());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
}

#[test]
fn unsupported_owned_version_and_foreign_css_are_passive_and_unchanged() {
    let source="<style id=\"evim-styles\" data-evim-version=\"2\">.evim-c-41 { --evim-style-id: \"A\"; color: red; }</style><p class='evim-c-41'>Text</p>";
    let mut document = html(source);
    assert_eq!(document.text(), "Text");
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document
        .projection()
        .style_sheet()
        .character_style(&StyleId::from("A"))
        .is_none());
    document.insert(4, "!").unwrap();
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .starts_with(source.split("<p").next().unwrap()));
}

#[test]
fn source_heading_inspector_field_edits_write_a_rule_and_are_undoable() {
    use evim_core::layout::MockTextMeasurementProvider;
    use evim_core::{Core, CoreEvent};
    let mut core = Core::new(html("<h2 data-keep='yes'>Heading</h2><p>Body</p>"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let document = core.document();
    let id = document.id();
    let revision = document.revision();
    let sheet_revision = document.projection().style_sheet().revision;
    core.handle(
        view,
        CoreEvent::EditGeneratedStyle {
            document: id,
            revision,
            style_sheet_revision: sheet_revision,
            namespace: StyleNamespace::Block,
            style: "Heading2".into(),
            edit: StyleDefinitionFieldEdit::SetDeclaration {
                property: StyleProperty::CharacterSize,
                value: StylePropertyValue::Float(27.0),
            },
        },
    )
    .unwrap();
    assert_eq!(
        core.document()
            .projection()
            .style_sheet()
            .block_style(&StyleId::from("Heading2"))
            .unwrap()
            .character
            .size,
        Some(27.0)
    );
    assert!(String::from_utf8(core.document().source_bytes())
        .unwrap()
        .contains("<h2 data-keep='yes'>Heading</h2>"));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(
        core.document().source_bytes(),
        b"<h2 data-keep='yes'>Heading</h2><p>Body</p>"
    );
}

#[test]
fn deleting_assigned_style_removes_only_its_rule_and_class_and_rebases_children() {
    let mut document = html("<p data-keep='yes'>Text</p>");
    for (id, parent) in [("Parent", "Character"), ("Child", "Parent")] {
        definition(
            &mut document,
            StyleDefinitionEdit::InsertCharacter {
                style: CharacterStyle {
                    id: id.into(),
                    based_on: Some(parent.into()),
                    properties: Default::default(),
                },
                metadata: metadata(id),
            },
        )
        .unwrap();
    }
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    apply(
        &mut document,
        PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: "Parent".into(),
        },
    )
    .unwrap();
    let before = document.source_bytes();
    let committed = definition(
        &mut document,
        StyleDefinitionEdit::DeleteCharacter("Parent".into()),
    )
    .unwrap();
    assert_eq!(document.text(), "Text");
    assert_eq!(committed.summary().source_patches().len(), 3);
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.contains("<p data-keep='yes'>"));
    assert!(!source.contains("evim-c-506172656e74"));
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .character_style(&StyleId::from("Child"))
            .unwrap()
            .based_on,
        Some("Character".into())
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
}

#[test]
fn deleting_custom_paragraph_on_heading_assigns_default_and_preserves_direct_properties() {
    let mut document = html("<h2 data-keep='yes' style='text-align:center'>Text</h2><!--opaque-->");
    definition(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: "Custom".into(),
                based_on: Some("Heading2".into()),
                next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: CharacterProperties {
                    size: Some(40.0),
                    ..Default::default()
                },
                block: Default::default(),
            },
            metadata: metadata("Custom"),
        },
    )
    .unwrap();
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    apply(
        &mut document,
        PersistedStyleIntent::AssignBlockStyle {
            target: StyleBlockTarget::Paragraphs(range),
            style: "Custom".into(),
        },
    )
    .unwrap();
    let original = document.source_bytes();
    definition(
        &mut document,
        StyleDefinitionEdit::DeleteBlock("Custom".into()),
    )
    .unwrap();
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    assert_eq!(
        document.projection().blocks()[0].direct_paragraph.alignment,
        Some(ParagraphAlignment::Center)
    );
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains("data-keep='yes'"));
    assert!(saved.ends_with("<!--opaque-->"));
    let reopened = html(&saved);
    assert_eq!(
        reopened.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    document.replace(0..1, "T").unwrap();
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn paragraph_heading_assignment_changes_only_tag_names_and_keeps_untouched_attributes() {
    for source in [
        "<P data-x='keep'>Heading</P><p>Body</p>",
        "<p data-x='keep'>Heading<p>Body</p>",
    ] {
        let mut document = html(source);
        document
            .apply_model_request(ModelRequest::SetParagraphStyle {
                document: document.id(),
                revision: document.revision(),
                range: 0..7,
                style: "Heading2".into(),
            })
            .unwrap();
        assert_eq!(document.text(), "Heading\nBody");
        assert_eq!(
            document.projection().blocks()[0].kind,
            BlockKind::Heading(2)
        );
        let changed = String::from_utf8(document.source_bytes()).unwrap();
        assert!(changed.starts_with("<h2 data-x='keep'>Heading</h2><p>Body</p>"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn named_definition_edit_invalidates_visible_metrics_in_a_large_document() {
    use evim_core::layout::MockTextMeasurementProvider;
    use evim_core::{Core, CoreEvent};
    let mut document = html("<p>Paragraph text</p>".repeat(2_000).as_str());
    definition(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: "BodyText".into(),
                based_on: Some("Paragraph".into()),
                next_paragraph_style: None,
                role: BlockRole::Paragraph,
                character: CharacterProperties {
                    size: Some(14.0),
                    ..Default::default()
                },
                block: Default::default(),
            },
            metadata: metadata("Body Text"),
        },
    )
    .unwrap();
    let source = String::from_utf8(document.source_bytes())
        .unwrap()
        .replace("<p>", "<p class=\"evim-p-426f647954657874\">");
    let mut core = Core::new(html(&source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let before = core.layout(view).unwrap().snapshot().unwrap().rows[0].clusters[0].advance;
    let first = core.document().projection().blocks()[0].id;
    let last = core.document().projection().blocks().last().unwrap().id;
    let document = core.document();
    let id = document.id();
    let revision = document.revision();
    let sheet_revision = document.projection().style_sheet().revision;
    core.handle(
        view,
        CoreEvent::EditGeneratedStyle {
            document: id,
            revision,
            style_sheet_revision: sheet_revision,
            namespace: StyleNamespace::Block,
            style: "BodyText".into(),
            edit: StyleDefinitionFieldEdit::SetDeclaration {
                property: StyleProperty::CharacterSize,
                value: StylePropertyValue::Float(24.0),
            },
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(snapshot.rows[0].clusters[0].advance > before);
    assert!(
        snapshot.rows.len() < 100,
        "only the viewport is materialized after the global definition edit"
    );
    assert_eq!(core.document().projection().blocks()[0].id, first);
    assert_eq!(
        core.document().projection().blocks().last().unwrap().id,
        last
    );
    assert_eq!(core.document().line_count(), 2_000);
}
