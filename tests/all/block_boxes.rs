use viem_core::document::{Encoding, Format};
use viem_core::layout::{DecorationKind, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document};

fn near(actual: f32, expected: f32) { assert!((actual - expected).abs() < 0.01, "{actual} != {expected}"); }

#[test]
fn empty_document_startup_accepts_canvas_insets_before_and_after_resize() {
    use viem_core::layout::EdgeInsets;
    for format in [Format::PlainText, Format::Markdown,] {
        for (width, height) in [(0., 0.), (1., 1.), (800., 600.)] {
            for wrap in [false, true] {
                let document = Document::from_bytes(Vec::new(), Encoding::Utf8, format).unwrap();
                let mut core = Core::new(document);
                let view = core.try_add_view(MockTextMeasurementProvider::new(), width, height).unwrap();
                core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
                for insets in [
                    EdgeInsets { top: 8., left: 8., bottom: 8., right: 8. },
                    EdgeInsets { top: 18., left: 18., bottom: 18., right: 18. },
                    EdgeInsets::default(),
                ] {
                    core.set_view_insets(view, insets).unwrap_or_else(|error| {
                        panic!("{format:?} {width}x{height} wrap={wrap} insets={insets:?}: {error:?}")
                    });
                    core.handle(view, CoreEvent::Resize { width: 800., height: 600. }).unwrap();
                    core.set_view_insets(view, insets).unwrap();
                    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                    assert_eq!(snapshot.rows.len(), 1);
                    assert!(snapshot.total_height > 0.);
                    assert_eq!(core.document().text(), "");
                }
            }
        }
    }
}

#[test]
fn invalid_saved_defaults_keep_valid_styles_and_builtin_markdown_containers_usable() {
    use viem_core::document::{BlockRole, FontSize};
    use viem_core::layout::EdgeInsets;
    let source = b"> Quoted paragraph.\n>\n> ~~~\n> code\n> ~~~\n";
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (name, expected) in [("Block quote", BlockRole::Quote), ("Code Block", BlockRole::CodeBlock)] {
            let defaults = serde_json::json!({"version":1,"block_styles":[
                {"id":"Heading1","name":"Changed heading","role":"Paragraph","based_on":"Paragraph",
                 "next_paragraph_style":"Paragraph","block":{},"character":{"size":99}},
                {"id":name,"name":name,"role":"Paragraph","based_on":"Paragraph",
                 "next_paragraph_style":"Paragraph","block":{"padding_left":3},"character":{"size":19}}
            ]});
            let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
            let before = document.projection().style_sheet().block_style(&name.into()).unwrap().clone();
            let revision = document.revision();
            let diagnostics = document.initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap()).unwrap();
            assert!(diagnostics.iter().any(|message| message.contains(name) && message.contains("role")));
            let sheet = document.projection().style_sheet();
            assert_eq!(sheet.block_style(&name.into()).unwrap(), &before);
            assert_eq!(sheet.block_style(&name.into()).unwrap().role, expected);
            assert_eq!(sheet.block_style(&"Heading1".into()).unwrap().character.size, Some(FontSize::Points(99.)));
            assert_eq!(document.revision(), revision);
            let mut core = Core::new(document);
            let view = core.try_add_view(MockTextMeasurementProvider::new(), 920., 655.).unwrap();
            core.set_view_insets(view, EdgeInsets { top:10., left:10., bottom:10., right:10. }).unwrap();
            assert!(core.layout(view).unwrap().snapshot().is_some());
            assert_eq!(core.document().source_bytes(), source);
        }
    }
}

#[test]
fn code_background_changes_refresh_cached_boxes_in_both_markdown_views() {
    use viem_core::document::{Color, StyleNamespace, StyleDefinitionFieldEdit, StyleProperty, StylePropertyValue};
    let source = "Before\n\n```\nfirst line\n\nlast line\n```\n\nAfter";
    for format in [Format::Markdown, Format::MarkdownSource] {
        for flow in [false, true] {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 500., 500.);
            if format.is_source_view() { core.handle(view, CoreEvent::SetParagraphFlow(flow)).unwrap(); }
            assert!(core.layout(view).unwrap().snapshot().is_some());
            for alpha in [1., 0.5, 0.] {
                let color = Color { red: 1., green: 0., blue: 0., alpha };
                let document = core.document();
                core.handle(view, CoreEvent::EditGeneratedStyle { document: document.id(), revision: document.revision(),
                    style_sheet_revision: document.projection().style_sheet().revision, namespace: StyleNamespace::Block,
                    style: "Code Block".into(), edit: StyleDefinitionFieldEdit::SetDeclaration {
                        property: StyleProperty::BlockBackground, value: StylePropertyValue::Color(color),
                    }}).unwrap();
                let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                let fills = snapshot.rows.iter().flat_map(|row| &row.decorations)
                    .filter(|item| item.kind == DecorationKind::BlockBackground).collect::<Vec<_>>();
                assert!(!fills.is_empty(), "{format:?} flow={flow}");
                assert!(fills.iter().all(|fill| fill.paint.foreground == color));
            }
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn relative_container_font_defaults_resolve_at_each_nesting_level() {
    use viem_core::document::{StyleDefinitionFieldEdit, StyleNamespace, StyleProperty, StylePropertyValue};
    use viem_core::layout::DocumentLayoutStyles;
    let document = Document::from_bytes(b"> A\n>\n> > B".to_vec(),Encoding::Utf8,Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(),400.,400.);
    let document = core.document();
    core.handle(view,CoreEvent::EditGeneratedStyle {document:document.id(),revision:document.revision(),
        style_sheet_revision:document.projection().style_sheet().revision,namespace:StyleNamespace::Block,
        style:"Block quote".into(),edit:StyleDefinitionFieldEdit::SetDeclaration {
            property:StyleProperty::CharacterSize,value:StylePropertyValue::Percentage(200)}}).unwrap();
    let projection = core.document().projection();
    let styles = DocumentLayoutStyles::resolve(projection).unwrap();
    near(styles.paragraphs[0].default_shaping_style.size,28.);
    near(styles.paragraphs[1].default_shaping_style.size,56.);
    near(DocumentLayoutStyles::semantic_character_at(projection,projection.text().find('B').unwrap(),false).unwrap().size,56.);
}
