use viem_core::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, Encoding, Format, ModelRequest,
    SemanticInlineStyle, StyleNamespace, StyleProperty, StylePropertyValue,
};
use viem_core::layout::DocumentLayoutStyles;

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn styling_one_character_of_an_entity_changes_only_its_complete_source_contributor() {
    for (source, enabled) in [
        (
            "<!--before--><p data-keep='x'>left <i>&fjlig;</i> right</p><!--after-->",
            true,
        ),
        (
            "<!--before--><p data-keep='x'>left <b><i>&fjlig;</i></b> right</p><!--after-->",
            false,
        ),
    ] {
        for selected_first in [true, false] {
            let mut document = html(source);
            let before_text = document.text().to_owned();
            let entity_at = before_text.find("fj").unwrap();
            let at = entity_at + usize::from(!selected_first);
            let range = at..at + 1;
            let raw_start = source.find("&fjlig;").unwrap();
            let raw_end = raw_start + "&fjlig;".len();
            let before_styles = (0..before_text.len())
                .map(|at| {
                    DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap()
                })
                .collect::<Vec<_>>();
            let prepared = document
                .prepare_model_request(ModelRequest::SetSemanticStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: range.clone(),
                    style: SemanticInlineStyle::Strong,
                    enabled,
                })
                .unwrap();
            assert!(prepared.summary().formatted_splices().is_empty());
            assert!(prepared.summary().source_patches().iter().all(|patch| {
                let range = patch.range();
                raw_start <= range.start && range.end <= raw_end
            }));
            for at in 0..=before_text.len() {
                let mapped = prepared
                    .text_position_map()
                    .map_text_point(
                        document.text_point(at).unwrap(),
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )
                    .unwrap();
                assert_eq!(mapped.value().unwrap().offset(), at);
            }
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), before_text);
            let after = document.source_bytes();
            assert!(after.starts_with(source[..raw_start].as_bytes()));
            assert!(after.ends_with(source[raw_end..].as_bytes()));
            let reopened =
                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Html).unwrap();
            for (position, before) in before_styles.iter().enumerate() {
                let actual =
                    DocumentLayoutStyles::character_at(document.projection(), position, false)
                        .unwrap();
                assert_eq!(
                    actual,
                    DocumentLayoutStyles::character_at(reopened.projection(), position, false)
                        .unwrap()
                );
                if range.contains(&position) {
                    assert_eq!(actual.bold, enabled);
                    assert_eq!(actual.slant, before.slant);
                } else {
                    assert_eq!(&actual, before, "unselected position {position}");
                }
            }
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}

#[test]
fn an_already_satisfied_partial_entity_style_is_a_byte_exact_noop() {
    for (source, enabled) in [("<p><b>&fjlig;</b></p>", true), ("<p>&fjlig;</p>", false)] {
        let mut document = html(source);
        let revision = document.revision();
        document
            .set_semantic_style(1..2, SemanticInlineStyle::Strong, enabled)
            .unwrap();
        assert_eq!(document.revision(), revision);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.history_status().can_undo);
    }
}

#[test]
fn direct_character_properties_share_entity_materialization_and_preserve_the_other_character() {
    let mut document = html("<p><b>&fjlig;</b></p>");
    let before = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    document
        .apply_model_request(ModelRequest::SetDirectCharacterProperties {
            document: document.id(),
            revision: document.revision(),
            range: 1..2,
            values: vec![(
                StyleProperty::CharacterSize,
                StylePropertyValue::Float(27.0),
            )],
        })
        .unwrap();
    assert_eq!(document.text(), "fj");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap(),
        before
    );
    let selected = DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap();
    assert_eq!(selected.size, 27.0);
    assert!(selected.bold);
}

#[test]
fn both_partial_entity_endpoints_materialize_without_losing_intervening_style() {
    let mut document = html("<p>&fjlig;<i>mid</i>&fjlig;</p><!--keep-->");
    document
        .set_semantic_style(1..6, SemanticInlineStyle::Strong, true)
        .unwrap();
    assert_eq!(document.text(), "fjmidfj");
    for at in 0..7 {
        let style = DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap();
        assert_eq!(style.bold, (1..6).contains(&at));
    }
    let after = document.source_bytes();
    let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
    assert!(after.ends_with(b"</p><!--keep-->"));
}

#[test]
fn named_character_assignment_and_direct_style_clear_share_materialization() {
    let source = "<p><b>&fjlig;</b></p><!--keep-->";
    let mut document = html(source);
    let before = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    document
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 1..2,
            namespace: StyleNamespace::Character,
            style: "Code".into(),
        })
        .unwrap();
    assert_eq!(document.text(), "fj");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap(),
        before
    );
    assert!(document.projection().style_spans().iter().any(|span| {
        span.range == (1..2)
            && span.application == viem_core::document::StyleApplication::Named("Code".into())
    }));
    assert!(!
        DocumentLayoutStyles::character_at(document.projection(), 1, false)
            .unwrap()
            .bold
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    let mut document = html("<p><span style='font-size:27pt'>&fjlig;</span></p><!--keep-->");
    let before = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
    document
        .apply_model_request(ModelRequest::EditDirectProperty {
            document: document.id(),
            revision: document.revision(),
            range: 1..2,
            property: StyleProperty::CharacterSize,
            value: None,
        })
        .unwrap();
    assert_eq!(document.text(), "fj");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap(),
        before
    );
    assert_ne!(
        DocumentLayoutStyles::character_at(document.projection(), 1, false)
            .unwrap()
            .size,
        before.size
    );
}

#[test]
fn partial_entity_visual_selection_keeps_the_style_menu_enabled_and_editable() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(html("<p>&fjlig;</p>"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 100.0);
    for key in ['l', 'v'] {
        core.handle(view, CoreEvent::Input(InputEvent::key(key)))
            .unwrap();
    }
    let presentation = core
        .selection_semantic_style_presentation(view, SemanticInlineStyle::Strong)
        .unwrap();
    assert!(presentation.can_set());
    assert!(presentation.can_clear());
    let selected = presentation.selection().unwrap().clone();
    assert_eq!(selected.range(), 1..2);
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected: selected,
            style: SemanticInlineStyle::Strong,
            enabled: true,
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "fj");
    assert!(
        !DocumentLayoutStyles::character_at(core.document().projection(), 0, false)
            .unwrap()
            .bold
    );
    assert!(
        DocumentLayoutStyles::character_at(core.document().projection(), 1, false)
            .unwrap()
            .bold
    );
}
