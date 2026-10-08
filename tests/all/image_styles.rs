use viem_core::document::*;
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider, DecorationKind};
use viem_core::{Core, CoreEvent};

fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn defaults() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"block_styles":[{
        "id":"Image","name":"Image","role":"Paragraph","based_on":"Paragraph",
        "next_paragraph_style":"Paragraph",
        "character":{"font_families":["Image Face"],"size":23,
            "foreground":{"red":0.8,"green":0.1,"blue":0.2,"alpha":1}},
        "block":{"margin_top":11,"margin_bottom":13,"padding_left":17,"padding_right":19,
            "padding_top":7,"padding_bottom":9,"border_left_width":2,"border_right_width":3,
            "border_top_width":4,"border_bottom_width":5}
    }]})).unwrap()
}

#[test]
fn image_only_paragraphs_use_image_style_in_both_views_without_rewriting_source() {
    let source = "![alone](local.png)\n\nProse ![inline](remote.png) text\n\n# ![heading](a.png)\n\n- ![list](a.png)\n\n![multiline\nalt](a.png)";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document(source, format);
        let styles = doc.projection().blocks().iter().map(|block|block.style.0.as_str()).collect::<Vec<_>>();
        assert_eq!(styles, ["Image","Paragraph","Heading1","BulletedList1","Image"], "{format:?}");
        assert!(doc.initialize_style_defaults(&defaults()).unwrap().is_empty());
        assert_eq!(doc.source_bytes(),source.as_bytes());
        let sheet=doc.projection().style_sheet();
        let definition=sheet.block_style(&"Image".into()).unwrap();
        assert_eq!(definition.role,BlockRole::Paragraph);
        assert_eq!(definition.next_paragraph_style,Some("Paragraph".into()));
        let original=doc.source_bytes();
        doc.insert(doc.text().len()," prose").unwrap();
        assert_eq!(doc.projection().blocks().last().unwrap().style.0,"Paragraph");
        assert!(doc.undo());
        assert_eq!(doc.projection().blocks().last().unwrap().style.0,"Image");
        assert_eq!(doc.source_bytes(),original);
    }
    for format in [Format::PlainText,Format::Code] {
        assert!(document(source,format).projection().style_sheet().block_style(&"Image".into()).is_none());
    }
}

#[test]
fn link_and_emphasis_wrappers_keep_image_paragraph_appearance_in_both_views() {
    for source in ["[![alt](a.png)](https://example.invalid)","**![alt](a.png)**",
        "~~![alt](a.png)~~", "![one](a.png) ![two](b.png)"] {
        for format in [Format::Markdown,Format::MarkdownSource] {
            let doc=document(source,format);
            assert_eq!(doc.projection().blocks()[0].style.0,"Image","{source} {format:?}");
            assert_eq!(doc.source_bytes(),source.as_bytes());
        }
    }
}

#[test]
fn image_font_and_paint_cover_only_objects_and_complete_source_syntax() {
    let source="Before [![alt](image.png)](https://example.invalid) after\n\n![alone](image.png)";
    for format in [Format::Markdown,Format::MarkdownSource] {
        let mut doc=document(source,format);
        doc.initialize_style_defaults(&defaults()).unwrap();
        let projection=doc.projection();
        let resolved=DocumentLayoutStyles::resolve(projection).unwrap();
        let images=projection.inline_images_for_region(&(0..doc.text().len()));
        assert_eq!(images.len(),2);
        for image in images {
            for at in [image.range.start,image.range.end-1] {
                let run=resolved.shaping_runs.iter().find(|run|run.text_range.contains(&at)).unwrap();
                assert_eq!(run.style.font_families,["Image Face"]);
                assert_eq!(run.style.size,23.);
                let paint=resolved.paint_runs.iter().find(|run|run.text_range.contains(&at)).unwrap();
                assert_eq!(paint.paint.foreground.red,0.8);
                assert!(!paint.paint.underline,"enclosing link must not recolor/underline image labels");
            }
            let caret=DocumentLayoutStyles::character_at(projection,image.range.start,false).unwrap();
            assert_eq!(caret.font_families,["Image Face"]);
            assert_eq!(caret.size,23.);
        }
        assert_ne!(DocumentLayoutStyles::character_at(projection,0,false).unwrap().font_families,["Image Face"]);
        assert_eq!(resolved.paragraphs[0].block_box.padding.left,0.);
        let image_paragraph=resolved.paragraphs.last().unwrap();
        assert_eq!((image_paragraph.margin_top,image_paragraph.margin_bottom),(11.,13.));
        assert_eq!((image_paragraph.block_box.padding.left,image_paragraph.block_box.padding.right),(17.,19.));
        assert_eq!((image_paragraph.block_box.padding.top,image_paragraph.block_box.padding.bottom),(7.,9.));
        assert_eq!((image_paragraph.block_box.border.left,image_paragraph.block_box.border.right),(2.,3.));
    }
}

#[test]
fn image_style_configuration_tracks_inline_dependencies_and_undo_without_source_changes() {
    for format in [Format::Markdown,Format::MarkdownSource] {
        let source="Before ![inline](a.png) after\n\n![alone](b.png)\n\nUnaffected";
        let mut doc=document(source,format);
        let images=doc.projection().inline_images_for_region(&(0..doc.text().len()));
        let mut definition=doc.projection().style_sheet().block_style(&"Image".into()).unwrap().clone();
        definition.character.size=Some(29.0.into());
        let committed=doc.apply_style_request(StyleModelRequest::new(doc.id(),doc.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(StyleDefinitionEdit::UpdateBlock(definition))))).unwrap();
        let change=committed.summary().style_change().unwrap();
        assert_eq!(change.affected_ranges(),images.iter().map(|image|image.range.clone()).collect::<Vec<_>>());
        assert!(change.invalidation_effects().contains(&StyleInvalidationEffect::Shaping));
        assert_eq!(doc.source_bytes(),source.as_bytes());
        assert_eq!(DocumentLayoutStyles::character_at(doc.projection(),images[0].range.start,false).unwrap().size,29.);
        assert!(doc.undo());
        assert_ne!(DocumentLayoutStyles::character_at(doc.projection(),images[0].range.start,false).unwrap().size,29.);
        assert!(doc.redo());
        assert_eq!(DocumentLayoutStyles::character_at(doc.projection(),images[0].range.start,false).unwrap().size,29.);
        let saved=doc.export_style_defaults().unwrap();
        let mut reopened=document(source,format);
        reopened.initialize_style_defaults(&saved).unwrap();
        assert_eq!(DocumentLayoutStyles::character_at(reopened.projection(),images[0].range.start,false).unwrap().size,29.);
    }
}

#[test]
fn configured_image_style_does_not_leak_into_replacement_prose() {
    let source="![image](a.png)";
    let mut doc=document(source,Format::Markdown);
    doc.initialize_style_defaults(&defaults()).unwrap();
    let mut core=Core::new(doc);
    let view=core.add_view(MockTextMeasurementProvider::new(),800.,600.);
    let doc=core.document();
    core.select_image(view,doc.id(),doc.revision(),0).unwrap();
    core.handle(view,CoreEvent::Input(viem_core::command::InputEvent::text("ordinary"))).unwrap();
    assert_eq!(core.document().source_bytes(),b"ordinary");
    assert_eq!(core.document().projection().blocks()[0].style.0,"Paragraph");
}

#[test]
fn live_image_border_changes_refresh_cached_boxes_in_both_views() {
    let source="Before\n\n![image](a.png)\n\nAfter";
    for format in [Format::Markdown,Format::MarkdownSource] {
        let mut doc=document(source,format);
        doc.initialize_style_defaults(&defaults()).unwrap();
        let mut core=Core::new(doc);
        let view=core.add_view(MockTextMeasurementProvider::new(),600.,600.);
        for color in [Color { red:0.3,green:0.7,blue:0.2,alpha:1. },Color { red:0.9,green:0.2,blue:0.5,alpha:1. }] {
            let doc=core.document();
            core.handle(view,CoreEvent::EditGeneratedStyle {document:doc.id(),revision:doc.revision(),
                style_sheet_revision:doc.projection().style_sheet().revision,namespace:StyleNamespace::Block,
                style:"Image".into(),edit:StyleDefinitionFieldEdit::SetDeclaration {
                    property:StyleProperty::BlockBorderLeftColor,value:StylePropertyValue::Color(color)}}).unwrap();
            let snapshot=core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot.rows.iter().flat_map(|row|&row.decorations)
                .any(|decoration|decoration.kind==DecorationKind::BlockBorder && decoration.paint.foreground==color));
        }
        assert_eq!(core.document().source_bytes(),source.as_bytes());
    }
}

#[test]
fn large_image_document_local_edit_and_style_resolution_remain_regional() {
    let mut source="paragraph\n\n".repeat(10_000);
    source.push_str("Text ![image](local.png) after");
    let mut doc=document(&source,Format::Markdown);
    doc.initialize_style_defaults(&defaults()).unwrap();
    let at=doc.text().rfind("Text").unwrap();
    let (_,work)=measure_document_work(||doc.insert(at,"new ").unwrap());
    assert_eq!(work.full_projection_candidates,0,"{work:?}");
    assert!(work.projected_formatted_bytes<256,"{work:?}");
    let (_,work)=measure_document_work(||DocumentLayoutStyles::resolve_region(doc.projection(),at..doc.text().len()).unwrap());
    assert_eq!(work.formatted_full_materialized_bytes,0,"{work:?}");
    assert!(doc.source_bytes().ends_with(b"new Text ![image](local.png) after"));
}
