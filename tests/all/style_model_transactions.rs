use viem_core::document::{
    BlockProperties, BlockRole, BlockStyle, CharacterProperties, CharacterStyle,
    ConfigurationStyleIntent, Document, Encoding, FileFormat, Format, ModelChangeKind,
    ModelTransactionError, PersistedStyleIntent, PipelineCapabilityDecision, PipelineEditIntent,
    StyleDefinitionEdit, StyleDefinitionMetadata,
    StyleError, StyleId, StyleInvalidationEffect, StyleModelIntent, StyleModelRequest,
    StyleProperty, StyleTransactionError, TextEdit, TextRange, UnsupportedEditReason,
};
use std::collections::BTreeSet;

fn request(document: &Document, intent: StyleModelIntent) -> StyleModelRequest {
    StyleModelRequest::new(document.id(), document.revision(), intent)
}

fn configure(document: &Document, intent: ConfigurationStyleIntent) -> StyleModelRequest {
    request(document, StyleModelIntent::Configuration(intent))
}

fn update_heading(document: &Document, size: f32, margin_bottom: f32) -> StyleModelRequest {
    let mut heading = document
        .projection()
        .style_sheet()
        .block_style(&StyleId::from("Heading1"))
        .unwrap()
        .clone();
    heading.character.size = Some((size).into());
    heading.block.margin_bottom = Some(margin_bottom);
    configure(
        document,
        ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(heading)),
    )
}

fn whole_document(document: &Document) -> TextRange {
    TextRange::new(
        document.text_point(0).unwrap(),
        document
            .text_point(document.projection().text_tree().byte_len())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn configuration_definition_edit_is_atomic_source_unchanged_and_branching() {
    let mut document = Document::from_bytes_with_file_format(
        b"# Title\nbody".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Unix,
    )
    .unwrap();
    let original_source = document.source_bytes();
    let original_sheet = document.projection().style_sheet().clone();

    let committed = document
        .apply_style_request(update_heading(&document, 31.0, 12.0))
        .unwrap();
    assert_eq!(
        committed.summary().kind(),
        ModelChangeKind::ConfigurationStyle
    );
    assert!(committed.summary().source_patches().is_empty());
    assert!(committed.summary().formatted_splices().is_empty());
    assert_eq!(document.source_bytes(), original_source);
    let style_change = committed.summary().style_change().unwrap();
    assert_eq!(style_change.before_revision(), original_sheet.revision);
    assert_eq!(
        style_change.after_revision(),
        document.projection().style_sheet().revision
    );
    assert_eq!(
        style_change.changed_properties(),
        &BTreeSet::from([
            StyleProperty::BlockMarginBottom,
            StyleProperty::CharacterSize,
        ])
    );
    assert!(style_change
        .affected_block_styles()
        .contains(&StyleId::from("Heading1")));
    assert_eq!(
        style_change.affected_ranges(),
        std::iter::once(0..5).collect::<Vec<_>>()
    );
    assert!(style_change
        .invalidation_effects()
        .contains(&StyleInvalidationEffect::ParagraphLayout));
    assert!(style_change
        .invalidation_effects()
        .contains(&StyleInvalidationEffect::Shaping));

    assert!(document.undo());
    assert_eq!(document.projection().style_sheet(), &original_sheet);
    assert!(document.redo());
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .block_style(&StyleId::from("Heading1"))
            .unwrap()
            .character
            .size,
        Some((31.0).into())
    );

    assert!(document.undo());
    document
        .apply_style_request(update_heading(&document, 28.0, 8.0))
        .unwrap();
    assert!(
        !document.redo(),
        "a new style edit creates a history branch"
    );
    assert_eq!(document.source_bytes(), original_source);
}

#[test]
fn failed_and_stale_configuration_edits_publish_nothing() {
    let mut document = Document::new("unchanged");
    let before_revision = document.revision();
    let before_sheet = document.projection().style_sheet().clone();
    let before_history = document.history_status();
    let invalid = CharacterStyle {
        id: StyleId::from("Broken"),
        based_on: Some(StyleId::from("Missing")),
        properties: CharacterProperties::default(),
    };
    let error = document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::InsertCharacter {
                style: invalid,
                metadata: StyleDefinitionMetadata::generated("Broken"),
            }),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ModelTransactionError::Style(StyleTransactionError::Definition(
            StyleError::MissingParent(id)
        )) if id == StyleId::from("Missing")
    ));
    assert_eq!(document.revision(), before_revision);
    assert_eq!(document.projection().style_sheet(), &before_sheet);
    assert_eq!(document.history_status(), before_history);

    let prepared = document
        .prepare_style_request(update_heading_for_plain(&document, 18.0))
        .unwrap();
    document.insert(0, "X").unwrap();
    let after_text_sheet = document.projection().style_sheet().clone();
    assert!(matches!(
        document.commit_model_transaction(prepared),
        Err(ModelTransactionError::StaleRevision { .. })
    ));
    assert_eq!(document.projection().style_sheet(), &after_text_sheet);
}

fn update_heading_for_plain(document: &Document, size: f32) -> StyleModelRequest {
    let mut heading = document
        .projection()
        .style_sheet()
        .block_style(&StyleId::from("Heading1"))
        .unwrap()
        .clone();
    heading.character.size = Some((size).into());
    configure(
        document,
        ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(heading)),
    )
}

#[test]
fn configured_sheet_and_document_root_survive_later_reprojection() {
    let mut document = Document::from_bytes_with_file_format(
        b"# heading\nbody".to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Unix,
    )
    .unwrap();
    document
        .apply_style_request(update_heading(&document, 33.0, 14.0))
        .unwrap();
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::SetDocumentCanvas(BlockProperties {
                padding_left: Some(19.0),
                ..BlockProperties::default()
            }),
        ))
        .unwrap();
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::SetDocumentDefaultCharacter(CharacterProperties {
                font_families: Some(vec!["Configured Serif".to_owned()]),
                size: Some((17.0).into()),
                ..CharacterProperties::default()
            }),
        ))
        .unwrap();
    let configured_sheet = document.projection().style_sheet().clone();
    let configured_root = document.projection().document_style().clone();
    let source_before_edit = document.source_bytes();

    document
        .apply_edits(vec![TextEdit::new(0..0, "preface\n")])
        .unwrap();
    assert_ne!(document.source_bytes(), source_before_edit);
    assert_eq!(document.projection().style_sheet(), &configured_sheet);
    assert_eq!(document.projection().document_style(), &configured_root);
    document.set_file_format(FileFormat::Dos).unwrap();
    assert_eq!(document.projection().style_sheet(), &configured_sheet);
    assert_eq!(document.projection().document_style(), &configured_root);
}

#[test]
fn dependency_summary_names_descendants_but_not_unrelated_styles() {
    let mut document = Document::new("plain");
    let base = StyleId::from("Code");
    for (id, parent) in [
        ("Parent", base.clone()),
        ("Child", StyleId::from("Parent")),
        ("Unrelated", base.clone()),
    ] {
        document
            .apply_style_request(configure(
                &document,
                ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: StyleId::from(id),
                        based_on: Some(parent),
                        properties: CharacterProperties::default(),
                    },
                    metadata: StyleDefinitionMetadata::generated(id),
                }),
            ))
            .unwrap();
    }
    let mut parent = document
        .projection()
        .style_sheet()
        .character_style(&StyleId::from("Parent"))
        .unwrap()
        .clone();
    parent.properties.weight = Some(650);
    let committed = document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateCharacter(parent)),
        ))
        .unwrap();
    let change = committed.summary().style_change().unwrap();
    assert_eq!(
        change.affected_character_styles(),
        &[StyleId::from("Child"), StyleId::from("Parent")]
    );
    assert!(!change
        .affected_character_styles()
        .contains(&StyleId::from("Unrelated")));
    assert!(change.affected_ranges().is_empty());
    assert!(change.invalidation_effects().is_empty());
}

#[test]
fn in_use_delete_and_incompatible_document_assignment_are_structured() {
    let mut markdown =
        Document::from_bytes(b"# used".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let before = markdown.revision();
    markdown
        .apply_style_request(configure(
            &markdown,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::DeleteBlock(
                StyleId::from("Heading1"),
            )),
        ))
        .unwrap();
    assert!(markdown
        .projection()
        .style_sheet()
        .block_style(&StyleId::from("Heading1"))
        .is_none());
    assert_eq!(
        markdown.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    assert_eq!(markdown.source_bytes(), b"# used");
    markdown.replace(0..1, "U").unwrap();
    assert_eq!(
        markdown.projection().blocks()[0].style,
        StyleId::from("Paragraph")
    );
    assert!(markdown.undo());
    assert!(markdown.undo());
    assert_eq!(markdown.revision(), before);
    assert_eq!(
        markdown.projection().blocks()[0].style,
        StyleId::from("Heading1")
    );

    let error = markdown
        .apply_style_request(configure(
            &markdown,
            ConfigurationStyleIntent::AssignDocumentStyle(StyleId::from("Heading1")),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ModelTransactionError::Style(StyleTransactionError::Definition(
            StyleError::IncompatibleBlockRole { .. }
        ))
    ));
    assert_eq!(markdown.revision(), before);
}

#[test]
fn document_root_configuration_is_undoable_and_reports_precise_layers() {
    let mut document = Document::new("content");
    let source = document.source_bytes();
    let committed = document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::SetDocumentCanvas(BlockProperties {
                padding_left: Some(23.0),
                background: Some(viem_core::document::Color {
                    red: 0.1,
                    green: 0.2,
                    blue: 0.3,
                    alpha: 1.0,
                }),
                ..BlockProperties::default()
            }),
        ))
        .unwrap();
    let change = committed.summary().style_change().unwrap();
    assert_eq!(
        change.changed_properties(),
        &BTreeSet::from([
            StyleProperty::CanvasBackground,
            StyleProperty::CanvasPaddingLeft,
        ])
    );
    assert_eq!(
        change.invalidation_effects(),
        &BTreeSet::from([
            StyleInvalidationEffect::Paint,
            StyleInvalidationEffect::ViewUsableWidth,
        ])
    );
    assert_eq!(
        change.affected_ranges(),
        std::iter::once(0..7).collect::<Vec<_>>()
    );
    assert_eq!(document.source_bytes(), source);
    assert_eq!(
        document
            .projection()
            .document_style()
            .direct_canvas
            .padding_left,
        Some(23.0)
    );
    assert!(document.undo());
    assert_eq!(
        document
            .projection()
            .document_style()
            .direct_canvas
            .padding_left,
        None
    );
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn document_configuration_clear_is_canonical_no_op_and_domain_checked() {
    let mut document = Document::new("content");
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::SetDocumentDefaultCharacter(CharacterProperties {
                weight: Some(725),
                ..CharacterProperties::default()
            }),
        ))
        .unwrap();
    let before_clear = document.revision();
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::ClearDocumentDefaultCharacter(BTreeSet::from([
                StyleProperty::CharacterWeight,
            ])),
        ))
        .unwrap();
    assert_ne!(document.revision(), before_clear);
    assert_eq!(
        document
            .projection()
            .document_style()
            .direct_default_character
            .weight,
        None
    );

    let no_op_revision = document.revision();
    let prepared = document
        .prepare_style_request(configure(
            &document,
            ConfigurationStyleIntent::ClearDocumentDefaultCharacter(BTreeSet::from([
                StyleProperty::CharacterWeight,
            ])),
        ))
        .unwrap();
    assert!(prepared.is_no_op());
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.revision(), no_op_revision);

    let sheet = document.projection().style_sheet().clone();
    let error = document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::ClearDocumentCanvas(BTreeSet::from([
                StyleProperty::CharacterWeight,
            ])),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        ModelTransactionError::Style(StyleTransactionError::InvalidPropertyTarget {
            property: StyleProperty::CharacterWeight,
            ..
        })
    ));
    assert_eq!(document.projection().style_sheet(), &sheet);
    assert_eq!(document.revision(), no_op_revision);
}

#[test]
fn base_definition_and_document_style_assignment_require_configuration_authority() {
    let mut document = Document::new("content");
    let source = document.source_bytes();
    let mut base_document = document
        .projection()
        .style_sheet()
        .block_style(&document.projection().style_sheet().base_paragraph)
        .unwrap()
        .clone();
    base_document.character.size = Some((18.0).into());
    let committed = document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(
                base_document,
            )),
        ))
        .unwrap();
    assert_eq!(
        committed
            .summary()
            .style_change()
            .unwrap()
            .affected_ranges(),
        std::iter::once(0..7).collect::<Vec<_>>()
    );

    let custom = BlockStyle {
        id: StyleId::from("WritingCanvas"),
        based_on: Some(document.projection().style_sheet().base_paragraph.clone()),
        next_paragraph_style: None,
        role: BlockRole::Document,
        character: CharacterProperties {
            font_families: Some(vec!["Writing Sans".to_owned()]),
            ..CharacterProperties::default()
        },
        block: BlockProperties {
            padding_right: Some(11.0),
            ..BlockProperties::default()
        },
    };
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::InsertBlock {
                style: custom,
                metadata: StyleDefinitionMetadata::generated("Writing Canvas"),
            }),
        ))
        .unwrap();
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::AssignDocumentStyle(StyleId::from("WritingCanvas")),
        ))
        .unwrap();
    assert_eq!(
        document.projection().document_style().style,
        StyleId::from("WritingCanvas")
    );
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn persisted_style_capabilities_are_explicit_for_plain_and_markdown() {
    for (format, bytes) in [
        (Format::PlainText, b"plain".as_slice()),
        (Format::Markdown, b"markdown".as_slice()),
        (Format::MarkdownSource, b"markdown".as_slice()),
    ] {
        let mut document = Document::from_bytes(bytes.to_vec(), Encoding::Utf8, format).unwrap();
        let range = whole_document(&document);
        let pipeline = document.transformation_pipeline_snapshot();
        let plain_reason = UnsupportedEditReason::PlainTextHasNoRichStyleStorage;
        for (intent, markdown_reason) in [
            (
                PipelineEditIntent::AssignBlockStyle {
                    style: StyleId::from("Paragraph"),
                },
                UnsupportedEditReason::FormatHasNoNamedStyleStorage,
            ),
            (
                PipelineEditIntent::AssignCharacterStyle {
                    style: StyleId::from(""),
                },
                UnsupportedEditReason::FormatHasNoNamedStyleStorage,
            ),
        ] {
            if format != Format::PlainText
                && (matches!(&intent, PipelineEditIntent::AssignBlockStyle { style } if style.0 == "Paragraph")
                    || matches!(&intent, PipelineEditIntent::AssignCharacterStyle { style } if style.0 == ""))
            {
                assert_eq!(
                    pipeline.capabilities(range, &intent).unwrap().decision,
                    PipelineCapabilityDecision::Supported
                );
                continue;
            }
            let expected_reason = if format == Format::PlainText {
                plain_reason
            } else {
                markdown_reason
            };
            assert!(matches!(
                pipeline.capabilities(range, &intent).unwrap().decision,
                PipelineCapabilityDecision::Unsupported { reason, .. }
                    if reason == expected_reason
            ));
        }
        assert_eq!(
            pipeline
                .capabilities(
                    range,
                    &PipelineEditIntent::EditConfigurationBlockStyleDefinition {
                        style: StyleId::from("Paragraph"),
                    },
                )
                .unwrap()
                .decision,
            PipelineCapabilityDecision::Supported
        );

        let persisted = PersistedStyleIntent::AssignCharacterStyle {
            range,
            style: StyleId::from(""),
        };
        if format != Format::PlainText {
            document
                .apply_style_request(request(&document, StyleModelIntent::Persisted(persisted)))
                .unwrap();
            assert_eq!(document.source_bytes(), bytes);
            continue;
        }
        let error = document
            .apply_style_request(request(&document, StyleModelIntent::Persisted(persisted)))
            .unwrap_err();
        let expected = if format == Format::PlainText {
            UnsupportedEditReason::PlainTextHasNoRichStyleStorage
        } else {
            UnsupportedEditReason::FormatHasNoNamedStyleStorage
        };
        assert!(matches!(
            error,
            ModelTransactionError::Style(StyleTransactionError::Unsupported {
                reason,
                ..
            }) if reason == expected
        ));
    }
}

#[test]
fn unused_configuration_definitions_support_insert_update_and_delete() {
    let mut document = Document::new("text");
    let style = BlockStyle {
        id: StyleId::from("BodyIndented"),
        based_on: Some(document.projection().style_sheet().base_paragraph.clone()),
        next_paragraph_style: None,
        role: BlockRole::Paragraph,
        character: CharacterProperties::default(),
        block: BlockProperties {
            leading_indent: Some(12.0),
            ..BlockProperties::default()
        },
    };
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::InsertBlock {
                style: style.clone(),
                metadata: StyleDefinitionMetadata::generated("Body Indented"),
            }),
        ))
        .unwrap();
    assert!(document
        .projection()
        .style_sheet()
        .block_style(&style.id)
        .is_some());
    let mut updated = style.clone();
    updated.block.leading_indent = Some(18.0);
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(updated)),
        ))
        .unwrap();
    document
        .apply_style_request(configure(
            &document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::DeleteBlock(
                style.id.clone(),
            )),
        ))
        .unwrap();
    assert!(document
        .projection()
        .style_sheet()
        .block_style(&style.id)
        .is_none());
}

#[test]
fn clearing_a_character_override_reports_contextual_heading_metric_changes() {
    use viem_core::layout::DocumentLayoutStyles;
    for inherited_from_parent in [false, true] {
        let source = b"# `head`\n\n`body`";
        let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let target = if inherited_from_parent { "CodeParent" } else { "Code" };
        if inherited_from_parent {
            document.apply_style_request(configure(&document,
                ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle { id: target.into(), based_on: None, properties: CharacterProperties {
                        size: Some((14.0).into()), ..Default::default()
                    } },
                    metadata: StyleDefinitionMetadata::generated("Code parent"),
                }))).unwrap();
        }
        let mut code = document.projection().style_sheet().character_style(&"Code".into()).unwrap().clone();
        code.based_on = inherited_from_parent.then(|| target.into());
        code.properties.size = (!inherited_from_parent).then_some(14.0.into());
        document.apply_style_request(configure(&document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateCharacter(code)))).unwrap();
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap().size, 14.0);
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap().size, 14.0);
        let mut style = document.projection().style_sheet().character_style(&target.into()).unwrap().clone();
        style.properties.size = None;
        let committed = document.apply_style_request(configure(&document,
            ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateCharacter(style)))).unwrap();
        let change = committed.summary().style_change().unwrap();
        assert_eq!(change.changed_properties(), &BTreeSet::from([StyleProperty::CharacterSize]));
        assert_eq!(change.invalidation_effects(), &BTreeSet::from([StyleInvalidationEffect::Shaping]));
        assert!(change.affected_character_styles().contains(&StyleId::from("Code")));
        assert!(change.affected_ranges().iter().any(|range| range.start == 0 && range.end >= 4));
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap().size, 24.0);
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 5, false).unwrap().size, 14.0);
        assert_eq!(document.source_bytes(), source);
        assert!(document.undo());
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap().size, 14.0);
    }
}
