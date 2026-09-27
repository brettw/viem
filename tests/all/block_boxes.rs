use viem_core::document::{Encoding, Format};
use viem_core::layout::{DecorationKind, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document};

fn core(source: &str) -> Core<MockTextMeasurementProvider> {
    Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap())
}
fn near(actual: f32, expected: f32) { assert!((actual - expected).abs() < 0.01, "{actual} != {expected}"); }

#[test]
fn empty_document_startup_accepts_canvas_insets_before_and_after_resize() {
    use viem_core::layout::EdgeInsets;
    for format in [Format::PlainText, Format::Markdown, Format::Html] {
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
fn nested_quotes_and_paragraphs_have_independent_css_boxes() {
    let source = "<blockquote style='margin:10pt 11pt 12pt 13pt;padding:3pt;border:2pt solid red;background-color:#eeeeee'><blockquote style='margin:5pt 6pt 7pt 8pt;padding:9pt;border:4pt solid blue;background-color:#dddddd'><p style='margin:1pt 2pt 3pt 4pt;padding:2pt;border:1pt solid green;background-color:#cccccc'>First</p><p style='margin:6pt 2pt 3pt 4pt;padding:2pt;border:1pt solid green;background-color:#cccccc'>Second</p></blockquote></blockquote>";
    let mut core = core(source);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 800.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let a = &snapshot.rows[0]; let b = &snapshot.rows[1];
    near(a.paragraph_content_x - snapshot.content_insets.left, 46.);
    near(a.y - snapshot.content_insets.top, 37.);
    near(b.y - a.y - a.line_advance, 12.); // 3 edge + max(3,6) margins + 3 edge
    let backgrounds = a.decorations.iter().filter(|d| d.kind == DecorationKind::BlockBackground).collect::<Vec<_>>();
    assert_eq!(backgrounds.len(), 3);
    for (d, x, y) in [(backgrounds[0],13.,10.),(backgrounds[1],26.,20.),(backgrounds[2],43.,34.)] {
        near(d.typographic_bounds.x - snapshot.content_insets.left, x);
        near(d.typographic_bounds.y - snapshot.content_insets.top, y);
    }
    let lower = b.decorations.iter().filter(|d| d.kind == DecorationKind::BlockBackground).collect::<Vec<_>>();
    near(backgrounds[0].typographic_bounds.y + backgrounds[0].typographic_bounds.height, lower[0].typographic_bounds.y);
    near(backgrounds[1].typographic_bounds.y + backgrounds[1].typographic_bounds.height, lower[1].typographic_bounds.y);
    near(lower[0].typographic_bounds.y + lower[0].typographic_bounds.height - b.y - b.line_advance, 31.);
    near(snapshot.total_height - b.y - b.line_advance - snapshot.content_insets.bottom,43.);
    assert!(a.decorations.iter().any(|d| d.paint.foreground.red == 1. && d.kind == DecorationKind::BlockQuoteBorder));
    assert!(a.decorations.iter().any(|d| d.paint.foreground.blue == 1. && d.kind == DecorationKind::BlockQuoteBorder));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn sibling_and_parent_child_margins_collapse_until_border_or_padding_separates_them() {
    for (separator, expected_top) in [("",20.),("padding-top:3pt",33.),("border-top:2pt solid red",32.)] {
        let source = format!("<blockquote style='margin:10pt 0 0;padding:0;border:0;{separator}'><p style='margin:20pt 0 12pt'>A</p><p style='margin:8pt 0 0'>B</p></blockquote>");
        let mut core = core(&source);
        let view = core.add_view(MockTextMeasurementProvider::new(),400.,400.);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        near(snapshot.rows[0].y - snapshot.content_insets.top, expected_top);
        near(snapshot.rows[1].y - snapshot.rows[0].y - snapshot.rows[0].line_advance,12.);
    }
    let mut core = core("<p style='margin:0 0 12pt'>A</p><p style='margin:-4pt 0 0'>B</p>");
    let view = core.add_view(MockTextMeasurementProvider::new(),400.,400.);
    let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
    near(rows[1].y - rows[0].y - rows[0].line_advance,8.);
}

#[test]
fn nested_boxes_reflow_after_resize_and_remain_bounded_in_large_documents() {
    let mut source = String::from("<blockquote style='margin:9pt;padding:5pt;border:2pt solid red'><blockquote style='margin:4pt;padding:3pt;border:1pt solid blue'>");
    for _ in 0..10_000 { source.push_str("<p>Words within nested boxes wrap onto several visual rows.</p>"); }
    source.push_str("</blockquote></blockquote>");
    let mut core = core(&source);
    let view = core.add_view(MockTextMeasurementProvider::new(),240.,120.);
    let old = core.layout(view).unwrap().snapshot().unwrap().rows[0].text_range.end;
    core.handle(view,CoreEvent::Resize { width:160.,height:120. }).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(snapshot.rows[0].text_range.end < old);
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    assert!(snapshot.rows.iter().all(|row| row.decorations.iter().filter(|d| d.kind == DecorationKind::BlockQuoteBorder).count() == 2));
}

#[test]
fn container_background_and_border_edits_invalidate_visible_boxes_and_geometry() {
    use viem_core::document::{Color, StyleNamespace, StyleDefinitionFieldEdit, StyleProperty, StylePropertyValue};
    let mut core = core("<blockquote><p>A</p><blockquote><p>B</p></blockquote></blockquote>");
    let view = core.add_view(MockTextMeasurementProvider::new(),400.,400.);
    let before = core.layout(view).unwrap().snapshot().unwrap().rows[1].paragraph_content_x;
    let color = Color {red:0.8,green:0.4,blue:0.2,alpha:1.};
    for (property,value) in [
        (StyleProperty::BlockBackground,StylePropertyValue::Color(color)),
        (StyleProperty::BlockBorderLeftWidth,StylePropertyValue::Float(6.)),
        (StyleProperty::BlockBorderLeftColor,StylePropertyValue::Color(color)),
    ] {
        let document = core.document();
        core.handle(view,CoreEvent::EditGeneratedStyle {document:document.id(),revision:document.revision(),
            style_sheet_revision:document.projection().style_sheet().revision,namespace:StyleNamespace::Block,
            style:"Block quote".into(),edit:StyleDefinitionFieldEdit::SetDeclaration {property,value}}).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows[1].decorations.iter().any(|d| d.kind == DecorationKind::BlockBackground && d.paint.foreground == color));
    }
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    near(snapshot.rows[1].paragraph_content_x-before,8.); // two levels, each grows 2 -> 6
    assert_eq!(snapshot.rows[1].decorations.iter().filter(|d| d.kind == DecorationKind::BlockQuoteBorder && d.paint.foreground == color).count(),2);
    let reopened = core.document().source_bytes();
    let defaults = core.document().export_style_defaults().unwrap();
    let mut document = Document::from_bytes(reopened,Encoding::Utf8,Format::Html).unwrap();
    document.initialize_style_defaults(&defaults).unwrap();
    let mut fresh = Core::new(document);
    let fresh_view = fresh.add_view(MockTextMeasurementProvider::new(),400.,400.);
    let fresh_rows = &fresh.layout(fresh_view).unwrap().snapshot().unwrap().rows;
    for (a,b) in snapshot.rows.iter().zip(fresh_rows.iter()) {
        near(a.y,b.y); near(a.paragraph_content_x,b.paragraph_content_x);
        let geometry = |row: &viem_core::layout::VisualRow| row.decorations.iter().cloned().map(|mut decoration| {
            decoration.owner = None; decoration
        }).collect::<Vec<_>>();
        assert_eq!(geometry(a),geometry(b));
    }
}

#[test]
fn code_inside_quote_has_one_box_around_literal_lines_and_zoom_scales_once() {
    let mut core = core("<blockquote style='margin:0;padding:5pt;border:2pt solid red'><pre style='margin:3pt;padding:4pt;border:1pt solid blue'>first\nsecond</pre></blockquote>");
    let view = core.add_view(MockTextMeasurementProvider::new(),600.,600.);
    for scale in [1.,2.,0.75] {
        core.handle(view,CoreEvent::SetScale(scale)).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.rows.len(),2);
        let a = &snapshot.rows[0]; let b = &snapshot.rows[1];
        near(a.paragraph_content_x-snapshot.content_insets.left,15.*scale);
        near(a.y-snapshot.content_insets.top,15.*scale);
        near(b.y-a.y,a.line_advance);
        let tops = a.decorations.iter().filter(|d| d.kind == DecorationKind::BlockBorder && d.typographic_bounds.width > 50. && d.typographic_bounds.y < a.y).count();
        let bottoms = b.decorations.iter().filter(|d| d.kind == DecorationKind::BlockBorder && d.typographic_bounds.width > 50. && d.typographic_bounds.y >= b.y+b.line_advance).count();
        assert_eq!(tops,2); assert_eq!(bottoms,2);
    }
}

#[test]
fn nested_container_text_defaults_inherit_without_inheriting_box_geometry() {
    use viem_core::layout::DocumentLayoutStyles;
    let mut core = core("<blockquote style='margin:0;padding:0;border:0;color:red;font-size:28pt'><blockquote style='margin:0;padding:0;border:2pt solid;font-size:42pt;text-align:center;line-height:2'><p>A</p><p style='text-align:end'>B</p></blockquote></blockquote>");
    let styles = DocumentLayoutStyles::resolve(core.document().projection()).unwrap();
    near(styles.paragraphs[0].default_shaping_style.size,42.);
    assert_eq!(styles.paragraphs[0].alignment,viem_core::document::ParagraphAlignment::Center);
    assert_eq!(styles.paragraphs[1].alignment,viem_core::document::ParagraphAlignment::End);
    near(styles.paragraphs[0].containers[1].style.foreground.red,1.);
    let view = core.add_view(MockTextMeasurementProvider::new(),600.,600.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    near(snapshot.rows[0].line_advance,42.*1.12*2.);
    assert!(snapshot.rows[0].decorations.iter().filter(|d| d.kind == DecorationKind::BlockBorder).all(|d| d.paint.foreground.red == 1. && d.paint.foreground.green == 0.));
}

#[test]
fn overlapping_children_paint_after_the_entire_parent_box_in_document_order() {
    let mut core = core("<blockquote style='margin:0;padding:2pt;border:1pt solid;background-color:#ff0000'><p style='margin:0;background-color:#00ff00'>A</p><p style='margin:-4pt 0 0;background-color:#0000ff'>B</p></blockquote><p style='background-color:#ffff00'>C</p>");
    let view = core.add_view(MockTextMeasurementProvider::new(),400.,400.);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let backgrounds = snapshot.decorations_in_paint_order().into_iter()
        .filter(|(_, decoration)| decoration.kind == DecorationKind::BlockBackground)
        .map(|(_, decoration)| decoration.paint.foreground).collect::<Vec<_>>();
    assert_eq!(backgrounds.len(),5);
    assert_eq!((backgrounds[0].red,backgrounds[1].red),(1.,1.)); // both parent slices first
    assert_eq!(backgrounds[2].green,1.);
    assert_eq!(backgrounds[3].blue,1.);
    assert_eq!((backgrounds[4].red,backgrounds[4].green),(1.,1.)); // following sibling last
}

#[test]
fn negative_margin_fallback_preserves_advancing_css_boundaries_in_full_and_regional_layout() {
    use viem_core::layout::{LayoutEngine, ViewLayout};
    for margin in [-100., -30., -4., 0., 20.] {
        let source = format!("<p style='margin:0'>A</p><p style='margin:{margin}pt 0 0'>B</p><p style='margin:0'>C</p>");
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut full = ViewLayout::new(400., 400.);
        engine.relayout(&document, &mut full).unwrap();
        let mut core = Core::new(document);
        let view = core.try_add_view(MockTextMeasurementProvider::new(), 400., 400.).unwrap();
        for snapshot in [full.snapshot().unwrap(), core.layout(view).unwrap().snapshot().unwrap()] {
            let requested = snapshot.rows[0].line_advance + margin;
            near(snapshot.rows[1].y - snapshot.rows[0].y, if requested <= 0. { 1. } else { requested });
            assert_eq!(snapshot.diagnostics.iter().any(|d| d.message.contains("reverse document flow")), requested <= 0.);
            assert!(snapshot.rows.windows(2).all(|rows| rows[1].y > rows[0].y));
        }
        core.handle(view, CoreEvent::SetScale(2.)).unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let requested = snapshot.rows[0].line_advance + margin * 2.;
        near(snapshot.rows[1].y - snapshot.rows[0].y, if requested <= 0. { 2. } else { requested });
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let source = "<blockquote style='margin:-100pt 0 0;padding:0;border:0'><p style='margin:0'>First</p></blockquote>";
    let mut core = core(source);
    let view = core.try_add_view(MockTextMeasurementProvider::new(), 400., 400.).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    near(snapshot.rows[0].y, 0.);
    assert!(snapshot.diagnostics.iter().any(|d| d.message.contains("reverse document flow")));
}

#[test]
fn extreme_negative_margins_keep_large_documents_editable_with_bounded_regional_work() {
    use viem_core::command::{InputEvent, Key};
    let source = "<p style='margin:-100pt 0 0'>Editable text.</p>".repeat(10_000);
    let mut core = core(&source);
    let view = core.try_add_view(MockTextMeasurementProvider::new(), 400., 120.).unwrap();
    for key in ['g', 'g', 'A'] { core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key)))).unwrap(); }
    core.handle(view, CoreEvent::Input(InputEvent::text(" More."))).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    assert!(core.document().text().starts_with("Editable text. More."));
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 1_000);
    assert!(snapshot.rows.windows(2).all(|rows| rows[1].y > rows[0].y));
    assert!(snapshot.diagnostics.iter().any(|d| d.message.contains("reverse document flow")));
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn reverse_flow_fallback_also_handles_wrapped_empty_and_unwrapped_rows() {
    use viem_core::layout::{LayoutEngine, ViewLayout};
    for source in [
        "<p style='margin:0'>Words that wrap onto several rows before the following paragraph.</p><p style='margin:-100pt 0 0'>Next</p>",
        "<p style='margin:0'></p><p style='margin:-100pt 0 0'></p><p style='margin:0'>Next</p>",
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        for wrap in [true, false] {
            let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
            let mut full = ViewLayout::new(120., 400.);
            full.set_wrap(wrap);
            engine.relayout(&document, &mut full).unwrap();
            let mut core = core(source);
            let view = core.try_add_view(MockTextMeasurementProvider::new(),120.,400.).unwrap();
            core.handle(view, CoreEvent::SetWrap(wrap)).unwrap();
            for snapshot in [full.snapshot().unwrap(), core.layout(view).unwrap().snapshot().unwrap()] {
                assert!(snapshot.rows.windows(2).all(|rows| rows[1].y > rows[0].y));
                assert!(snapshot.diagnostics.iter().any(|d| d.message.contains("reverse document flow")), "source={source} wrap={wrap} full={} rows={:?}", snapshot.coverage.is_full_document(), snapshot.rows.iter().map(|row| row.y).collect::<Vec<_>>());
            }
        }
    }
}

#[test]
fn overlapping_tall_rows_remain_hittable_and_supply_visible_horizontal_extent() {
    use viem_core::layout::{LayoutEngine, LayoutPoint, LayoutSnapshot, ViewLayout};
    let source = "<p style='margin:0;font-size:100pt'>WWWWWWWWWWWWWWWWWWWW</p><p style='margin:-105pt 0 0;font-size:10pt'>Short</p><p style='margin:0;font-size:10pt'>Tail</p><p style='margin:200pt 0 0;font-size:10pt'>End</p>";
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut full = ViewLayout::new(160., 10.);
    full.set_wrap(false);
    engine.relayout(&document, &mut full).unwrap();
    let mut core = Core::new(document);
    let view = core.try_add_view(MockTextMeasurementProvider::new(),160.,400.).unwrap();
    core.handle(view, CoreEvent::SetWrap(false)).unwrap();
    let check = |snapshot: &LayoutSnapshot| {
        let rows = &snapshot.rows;
        assert_eq!(rows.len(),4);
        assert!(rows[0].y + rows[0].height() > rows[2].y + rows[2].height());
        for (y, expected) in [(rows[1].y + 2.,1), (rows[2].y + 2.,2), (rows[0].y + 60.,0), (rows[0].y + 130.,0)] {
            let point = snapshot.hit_test(LayoutPoint { x: rows[expected].paragraph_content_x, y }).unwrap();
            assert_eq!(point.text_offset, rows[expected].text_range.start, "y={y} should choose row{expected}");
        }
    };
    check(full.snapshot().unwrap());
    check(core.layout(view).unwrap().snapshot().unwrap());
    let top = full.snapshot().unwrap().rows[0].y;
    full.set_viewport_top(top + 60.).unwrap();
    assert!(full.maximum_viewport_left().unwrap() > 100.);
    full.set_viewport_top(top + 130.).unwrap();
    assert_eq!(full.maximum_viewport_left(),Some(0.));
    let top = core.layout(view).unwrap().snapshot().unwrap().rows[0].y;
    core.handle(view, CoreEvent::Resize { width:160., height:10. }).unwrap();
    core.handle(view, CoreEvent::SetViewportOrigin { left:0., top:Some(top + 60.) }).unwrap();
    assert!(core.viewport_state(view).unwrap().maximum_left().unwrap() > 100., "viewport={:?}; rows={:?}", core.viewport_state(view).unwrap(), core.layout(view).unwrap().snapshot().unwrap().rows.iter().map(|row| (row.hard_line_index,row.y,row.height(),row.width)).collect::<Vec<_>>());
    core.handle(view, CoreEvent::SetViewportOrigin { left:0., top:Some(top + 130.) }).unwrap();
    assert_eq!(core.viewport_state(view).unwrap().maximum_left(),Some(0.));
}

#[test]
fn earlier_tall_row_ink_remains_within_full_and_regional_document_scroll_bounds() {
    use viem_core::layout::{LayoutEngine, ViewLayout};
    let source = "<p style='margin:0;font-size:100pt'>Tall</p><p style='margin:-105pt 0 0;font-size:10pt'>End</p>";
    let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut full = ViewLayout::new(600.,10.);
    engine.relayout(&document,&mut full).unwrap();
    let mut core = Core::new(document);
    let view = core.try_add_view(MockTextMeasurementProvider::new(),600.,10.).unwrap();
    for snapshot in [full.snapshot().unwrap(), core.layout(view).unwrap().snapshot().unwrap()] {
        let tall = &snapshot.rows[0];
        assert!(snapshot.total_height >= tall.y + tall.height() + snapshot.content_insets.bottom);
        assert!(snapshot.maximum_viewport_top(10.).unwrap() >= tall.y + tall.height() - 10.);
    }
}

#[test]
fn returning_to_an_overlapping_tall_row_materializes_its_earlier_owner() {
    use viem_core::layout::LayoutPoint;
    let source = "<p style='margin:0;font-size:100pt'>WWWWWWWWWWWWWWWWWWWW</p><p style='margin:-105pt 0 0;font-size:10pt'>Short</p>".to_owned()
        + &"<p style='margin:0;font-size:10pt'>Following</p>".repeat(10_000);
    let mut core = core(&source);
    let view = core.try_add_view(MockTextMeasurementProvider::new(),160.,20.).unwrap();
    core.handle(view,CoreEvent::SetWrap(false)).unwrap();
    let top = core.layout(view).unwrap().snapshot().unwrap().rows[0].y;
    core.handle(view,CoreEvent::SetViewportOrigin {left:0.,top:Some(5_000.)}).unwrap();
    core.handle(view,CoreEvent::SetViewportOrigin {left:0.,top:Some(top + 60.)}).unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.coverage.hard_lines().start,0);
    assert!(snapshot.coverage.hard_lines().len() < 1_000);
    assert!(core.viewport_state(view).unwrap().maximum_left().unwrap() > 100.);
    // Later small rows may paint over the tall row at this y; hit testing must
    // still resolve the materialized overlap instead of requesting a gap.
    let point = snapshot.hit_test(LayoutPoint {x:snapshot.rows[0].paragraph_content_x,y:top + 60.}).unwrap();
    assert!(snapshot.rows.iter().any(|row| row.text_range.start == point.text_offset));
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
