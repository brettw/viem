use serde_json::{json, Value};
use viem_core::document::{Color, Encoding, Format};
use viem_core::layout::{
    compute_layout_job, inspect_layout_provider, prepare_layout_job, DecorationKind,
    DecorationOwner, HardLineLayoutRegion, LayoutCancellationToken, LayoutEngine,
    LayoutExecutionContext, LayoutJobId, LayoutJobPriority, LayoutJobRegion, LayoutSnapshot,
    MockTextMeasurementProvider, ViewLayout,
};
use viem_core::Document;

const SOURCE: &str = "| Head | Other |\n| --- | --- |\n| body | next |";
fn document(source: &str, format: Format, body: Value, header: Value, table: Value) -> Document {
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    configure(&mut document, body, header, table);
    document
}
fn configure(document: &mut Document, body: Value, header: Value, table: Value) {
    let diagnostics = document
        .replace_style_defaults(
            &serde_json::to_vec(&json!({"version":1,"block_styles":[
                {"id":"Table cell","name":"Table cell","role":"Paragraph","based_on":"Paragraph","block":body},
                {"id":"Table header","name":"Table header","role":"Paragraph","based_on":"Table cell","block":header},
                {"id":"Table","name":"Table","role":"Table","based_on":"Paragraph","block":table}
            ]}))
            .unwrap(),
        )
        .unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}
fn layout(document: &Document, scale: f32) -> LayoutSnapshot {
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(1000., 1000.);
    view.set_scale(scale).unwrap();
    engine.relayout(document, &mut view).unwrap();
    view.snapshot().unwrap().clone()
}
fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
}
fn first_cell_row(
    snapshot: &LayoutSnapshot,
    row: usize,
    column: usize,
) -> &viem_core::layout::VisualRow {
    snapshot
        .rows
        .iter()
        .find(|line| {
            line.table_cell
                .as_ref()
                .is_some_and(|cell| cell.row == row && cell.column == column)
        })
        .unwrap()
}

#[test]
fn inherited_top_edges_precede_header_and_body_padding() {
    let document = document(
        SOURCE,
        Format::Markdown,
        json!({"padding_top":4,"padding_bottom":6,"border_top_width":20,"border_bottom_width":2}),
        json!({}),
        json!({"margin_top":0,"margin_bottom":0}),
    );
    for scale in [1., 2.] {
        let snapshot = layout(&document, scale);
        for index in [0, 1] {
            let row = first_cell_row(&snapshot, index, 0);
            let cell = row.table_cell.as_ref().unwrap();
            close(row.y - cell.rect.y, 24. * scale);
            close(
                cell.rect.height,
                row.natural_height() + (30. + if index == 1 { 2. } else { 0. }) * scale,
            );
            let border = row
                .decorations
                .iter()
                .find(|decoration| {
                    decoration.kind == DecorationKind::BlockBorder
                        && (decoration.typographic_bounds.y - cell.rect.y).abs() < 0.02
                        && (decoration.typographic_bounds.height - 20. * scale).abs() < 0.02
                })
                .unwrap();
            close(border.typographic_bounds.width, cell.rect.width);
        }
    }
    assert_eq!(document.source_bytes(), SOURCE.as_bytes());
}

#[test]
fn collapsed_header_bottom_wins_once_above_body_content() {
    let red = json!({"red":1,"green":0,"blue":0,"alpha":1});
    let blue = json!({"red":0,"green":0,"blue":1,"alpha":1});
    for header_width in [20., 31.] {
        let document = document(
            SOURCE,
            Format::Markdown,
            json!({"padding_top":4,"border_top_width":20,"border_top_color":blue,"border_bottom_width":2}),
            json!({"border_top_width":7,"border_bottom_width":header_width,"border_bottom_color":red}),
            json!({"margin_top":0,"margin_bottom":0}),
        );
        let snapshot = layout(&document, 1.);
        let header = first_cell_row(&snapshot, 0, 0);
        let body = first_cell_row(&snapshot, 1, 0);
        let header_cell = header.table_cell.as_ref().unwrap();
        let body_cell = body.table_cell.as_ref().unwrap();
        close(header.y - header_cell.rect.y, 11.);
        close(body.y - body_cell.rect.y, header_width + 4.);
        close(
            body_cell.rect.y,
            header_cell.rect.y + header_cell.rect.height,
        );
        let edge = body
            .decorations
            .iter()
            .find(|decoration| {
                decoration.kind == DecorationKind::BlockBorder
                    && (decoration.typographic_bounds.y - body_cell.rect.y).abs() < 0.02
                    && (decoration.typographic_bounds.height - header_width).abs() < 0.02
            })
            .unwrap();
        assert_eq!(
            edge.paint.foreground,
            Color {
                red: 1.,
                green: 0.,
                blue: 0.,
                alpha: 1.
            }
        );
        assert!(!header.decorations.iter().any(|decoration| decoration.kind
            == DecorationKind::BlockBorder
            && (decoration.typographic_bounds.y + decoration.typographic_bounds.height
                - body_cell.rect.y)
                .abs()
                < 0.02
            && decoration.typographic_bounds.width > 3.
            && decoration.typographic_bounds.height >= 20.));
    }
}

#[test]
fn container_perimeter_padding_and_background_enclose_the_grid() {
    let purple = Color {
        red: 0.6,
        green: 0.2,
        blue: 0.8,
        alpha: 1.,
    };
    let document = document(
        SOURCE,
        Format::Markdown,
        json!({"padding_top":3,"padding_right":3,"padding_bottom":3,"padding_left":3,
            "border_top_width":2,"border_right_width":2,"border_bottom_width":2,"border_left_width":2,
            "background":{"red":1,"green":1,"blue":1,"alpha":1}}),
        json!({}),
        json!({"margin_top":0,"margin_bottom":0,"padding_top":11,"padding_right":13,
            "padding_bottom":17,"padding_left":19,"border_top_width":3,"border_right_width":7,
            "border_bottom_width":5,"border_left_width":9,"border_top_color":purple,
            "border_right_color":purple,"border_bottom_color":purple,"border_left_color":purple,
            "background":{"red":0.2,"green":0.3,"blue":0.4,"alpha":1}}),
    );
    let snapshot = layout(&document, 1.);
    let table = &snapshot.tables()[0];
    let cells = snapshot.table_cells();
    close(cells[0].rect.x - table.rect.x, 28.);
    close(cells[0].rect.y - table.rect.y, 14.);
    close(
        table.rect.x + table.rect.width - cells[1].rect.x - cells[1].rect.width,
        20.,
    );
    close(
        table.rect.y + table.rect.height - cells[2].rect.y - cells[2].rect.height,
        22.,
    );
    close(
        first_cell_row(&snapshot, 0, 0).paragraph_content_x - cells[0].rect.x,
        5.,
    );
    close(first_cell_row(&snapshot, 0, 0).y - cells[0].rect.y, 5.);
    let owned = snapshot.decorations_in_paint_order();
    let table_owner = DecorationOwner::Table(table.table_id);
    let first_cell = owned
        .iter()
        .position(|(_, decoration)| matches!(decoration.owner, Some(DecorationOwner::Paragraph(_))))
        .unwrap();
    assert!(owned[..first_cell]
        .iter()
        .all(|(_, decoration)| decoration.owner == Some(table_owner)));
    let perimeter = owned
        .iter()
        .filter(|(_, decoration)| {
            decoration.owner == Some(table_owner) && decoration.kind == DecorationKind::BlockBorder
        })
        .map(|(_, decoration)| *decoration)
        .collect::<Vec<_>>();
    assert_eq!(perimeter.len(), 6);
    assert!(perimeter.iter().all(|edge| edge.paint.foreground == purple));
    let top = perimeter
        .iter()
        .find(|edge| (edge.typographic_bounds.height - 3.).abs() < 0.02)
        .unwrap();
    close(top.typographic_bounds.x, table.rect.x);
    close(top.typographic_bounds.y, table.rect.y);
    close(top.typographic_bounds.width, table.rect.width);
}

#[test]
fn zero_container_perimeter_retains_independent_outer_cell_edges() {
    let document = document(
        SOURCE,
        Format::Markdown,
        json!({"padding_top":4,"border_top_width":20}),
        json!({}),
        json!({"margin_top":0,"margin_bottom":0,"border_top_width":0}),
    );
    let snapshot = layout(&document, 1.);
    let header = first_cell_row(&snapshot, 0, 0);
    close(header.y - header.table_cell.as_ref().unwrap().rect.y, 24.);
    let body = first_cell_row(&snapshot, 1, 0);
    close(body.y - body.table_cell.as_ref().unwrap().rect.y, 24.);
}

#[test]
fn source_geometry_ignores_table_and_cell_box_declarations() {
    let document = document(
        SOURCE,
        Format::MarkdownSource,
        json!({}),
        json!({}),
        json!({}),
    );
    let first = layout(&document, 1.);
    let mut changed = Document::from_bytes(
        document.source_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    configure(
        &mut changed,
        json!({"padding_top":20,"padding_left":50,"border_top_width":40}),
        json!({}),
        json!({"padding_top":50,"padding_left":70,"border_left_width":30,"border_top_width":60}),
    );
    let second = layout(&changed, 1.);
    assert!(second.tables().is_empty());
    for (before, after) in first.rows.iter().zip(second.rows.iter()) {
        close(before.y, after.y);
        close(before.paragraph_content_x, after.paragraph_content_x);
        close(before.width, after.width);
        assert!(after
            .decorations
            .iter()
            .all(|decoration| decoration.kind != DecorationKind::BlockBorder));
    }
    assert_eq!(changed.source_bytes(), SOURCE.as_bytes());
}

#[test]
fn table_box_style_changes_invalidate_geometry_with_bounded_discovery() {
    let source = "| A | B |\n| - | - |\n".to_owned()
        + &(0..10_000)
            .map(|index| format!("| row {index} | value |\n"))
            .collect::<String>();
    let mut document = document(&source, Format::Markdown, json!({}), json!({}), json!({}));
    let mut view = ViewLayout::new(500., 150.);
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut height = None;
    for (id, top) in [(1, 1.), (2, 20.)] {
        configure(
            &mut document,
            json!({"border_top_width":top}),
            json!({}),
            json!({"padding_top":9}),
        );
        let request = prepare_layout_job(
            &document,
            &mut view,
            inspect_layout_provider(&engine),
            LayoutJobId(id),
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..3).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
        let candidate =
            compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let snapshot = candidate.regional_snapshot();
        assert!(snapshot.work_statistics().table_measured_cells() < 150);
        let actual = snapshot.lines()[0].rows()[0]
            .table_cell
            .as_ref()
            .unwrap()
            .rect
            .height;
        if let Some(previous) = height {
            close(actual - previous, 19.);
        }
        height = Some(actual);
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}

fn edit_style(
    core: &mut viem_core::Core<MockTextMeasurementProvider>,
    view: viem_core::ViewId,
    name: &str,
    edit: viem_core::document::StyleDefinitionFieldEdit,
) {
    let document = core.document();
    core.handle(
        view,
        viem_core::CoreEvent::EditGeneratedStyle {
            document: document.id(),
            revision: document.revision(),
            style_sheet_revision: document.projection().style_sheet().revision,
            namespace: viem_core::document::StyleNamespace::Block,
            style: name.into(),
            edit,
        },
    )
    .unwrap();
}

#[test]
fn border_colors_survive_live_style_changes_and_clear_to_theme_foreground() {
    use viem_core::document::{StyleDefinitionFieldEdit, StyleProperty, StylePropertyValue};
    let document = document(
        SOURCE,
        Format::Markdown,
        json!({
            "border_top_width":1,"border_right_width":1,"border_bottom_width":1,"border_left_width":1
        }),
        json!({}),
        json!({
            "border_top_width":2,"border_right_width":2,"border_bottom_width":2,"border_left_width":2
        }),
    );
    let mut core = viem_core::Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 1000.);
    let properties = [
        StyleProperty::BlockBorderTopColor,
        StyleProperty::BlockBorderRightColor,
        StyleProperty::BlockBorderBottomColor,
        StyleProperty::BlockBorderLeftColor,
    ];
    let purple = Color {
        red: 0.6,
        green: 0.2,
        blue: 0.8,
        alpha: 1.,
    };
    for property in properties {
        edit_style(
            &mut core,
            view,
            "Table",
            StyleDefinitionFieldEdit::SetDeclaration {
                property,
                value: StylePropertyValue::Color(purple),
            },
        );
    }
    for alpha in [1., 0.4, 0.] {
        let color = Color {
            red: 0.9,
            green: 0.1,
            blue: 0.3,
            alpha,
        };
        for property in properties {
            edit_style(
                &mut core,
                view,
                "Table cell",
                StyleDefinitionFieldEdit::SetDeclaration {
                    property,
                    value: StylePropertyValue::Color(color),
                },
            );
        }
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let mut cells = 0;
        let mut containers = 0;
        for (_, decoration) in snapshot.decorations_in_paint_order() {
            if decoration.kind != DecorationKind::BlockBorder {
                continue;
            }
            assert!(!decoration.paint.foreground_is_default);
            match decoration.owner {
                Some(DecorationOwner::Table(_)) => {
                    containers += 1;
                    assert_eq!(decoration.paint.foreground, purple);
                }
                Some(DecorationOwner::Paragraph(_)) => {
                    cells += 1;
                    assert_eq!(decoration.paint.foreground, color);
                }
                _ => panic!("unexpected border owner"),
            }
        }
        assert!(cells > 0 && containers > 0);
    }
    for property in properties {
        edit_style(
            &mut core,
            view,
            "Table cell",
            StyleDefinitionFieldEdit::ClearDeclaration(property),
        );
    }
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    for (_, decoration) in snapshot.decorations_in_paint_order() {
        if decoration.kind == DecorationKind::BlockBorder
            && matches!(decoration.owner, Some(DecorationOwner::Paragraph(_)))
        {
            assert!(
                decoration.paint.foreground_is_default,
                "cleared cell border must follow theme foreground"
            );
        }
    }
    assert_eq!(core.document().source_bytes(), SOURCE.as_bytes());
}

#[test]
fn collapsed_edge_without_color_uses_the_winning_styles_foreground() {
    use viem_core::document::{StyleDefinitionFieldEdit, StyleProperty, StylePropertyValue};
    let document = document(
        SOURCE,
        Format::Markdown,
        json!({}),
        json!({"border_bottom_width":12}),
        json!({"border_top_width":3}),
    );
    let mut core = viem_core::Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 1000.);
    let initial = core.layout(view).unwrap().snapshot().unwrap();
    assert!(initial
        .decorations_in_paint_order()
        .iter()
        .filter(|(_, decoration)| decoration.kind == DecorationKind::BlockBorder)
        .all(|(_, decoration)| decoration.paint.foreground_is_default));
    let blue = Color {
        red: 0.,
        green: 0.,
        blue: 1.,
        alpha: 1.,
    };
    let red = Color {
        red: 1.,
        green: 0.,
        blue: 0.,
        alpha: 1.,
    };
    for (name, color) in [("Table cell", red), ("Table header", blue)] {
        edit_style(
            &mut core,
            view,
            name,
            StyleDefinitionFieldEdit::SetDeclaration {
                property: StyleProperty::CharacterForeground,
                value: StylePropertyValue::Color(color),
            },
        );
    }
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let row = first_cell_row(snapshot, 1, 0);
    let cell = row.table_cell.as_ref().unwrap();
    let shared = row
        .decorations
        .iter()
        .find(|decoration| {
            decoration.kind == DecorationKind::BlockBorder
                && (decoration.typographic_bounds.y - cell.rect.y).abs() < 0.01
                && (decoration.typographic_bounds.height - 12.).abs() < 0.01
        })
        .unwrap();
    assert_eq!(shared.paint.foreground, blue);
    assert!(!shared.paint.foreground_is_default);
}

fn nested_table_document(source: &str, quote_left: f32, alignment: &str) -> Document {
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    configure_nested_table(&mut document, quote_left, alignment);
    document
}

fn configure_nested_table(document: &mut Document, quote_left: f32, alignment: &str) {
    let diagnostics = document.replace_style_defaults(&serde_json::to_vec(&json!({
        "version":1,"block_styles":[
            {"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","block":{}},
            {"id":"Block quote","name":"Block quote","role":"Quote","based_on":"Paragraph",
                "block":{"margin_left":quote_left,"padding_left":7,"border_left_width":3,"margin_right":19,"leading_indent":0}},
            {"id":"Bulleted List","name":"Bulleted List","role":"List","based_on":"Paragraph",
                "block":{"leading_indent":31,"trailing_indent":11}},
            {"id":"List item","name":"List item","role":"ListItem","based_on":"Paragraph",
                "block":{"padding_left":5,"padding_right":2}},
            {"id":"Table","name":"Table","role":"Table","based_on":"Paragraph",
                "block":{"margin_left":13,"margin_right":17,"alignment":alignment}},
            {"id":"Table cell","name":"Table cell","role":"Paragraph","based_on":"Paragraph","block":{}},
            {"id":"Table header","name":"Table header","role":"Paragraph","based_on":"Table cell","block":{}}
        ]
    })).unwrap()).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn nested_table_uses_enclosing_canvas_in_full_regional_and_large_cell_layout() {
    for large in [false, true] {
        let body = if large { "x".repeat(70_000) } else { "body".into() };
        for (source, left, right) in [
            (format!("> | Head | Other |\n> | --- | --- |\n> | {body} | next |"), 27., 19.),
            (format!("- lead\n\n  | Head | Other |\n  | --- | --- |\n  | {body} | next |"), 36., 13.),
            (format!("> - lead\n>\n>   | Head | Other |\n>   | --- | --- |\n>   | {body} | next |"), 63., 32.),
        ] {
            for alignment in ["Start", "Center", "End"] {
                let document = nested_table_document(&source, 17., alignment);
                let mut full_view = ViewLayout::new(1000., 1000.);
                full_view.set_insets(viem_core::layout::EdgeInsets { left: 23., right: 29., ..Default::default() });
                let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
                engine.relayout(&document, &mut full_view).unwrap();
                let full = full_view.snapshot().unwrap();
                let table = &full.tables()[0];
                let spare = (1000. - 23. - left - 29. - right - 13. - 17. - table.rect.width).max(0.);
                close(table.rect.x, 23. + left + 13. + match alignment { "Center" => spare / 2., "End" => spare, _ => 0. });
                let mut core = viem_core::Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 1000.);
                core.set_view_insets(view, viem_core::layout::EdgeInsets { left:23., right:29., ..Default::default() }).unwrap();
                let regional = core.layout(view).unwrap().snapshot().unwrap();
                close(regional.tables()[0].rect.x, table.rect.x);
                for snapshot in [full, regional] {
                    for row_index in 0..2 {
                        let borders: Vec<_> = snapshot.rows.iter().filter(|row| {
                            row.table_cell.as_ref().is_some_and(|cell| cell.row == row_index)
                        }).flat_map(|row| &row.decorations).filter(|decoration| {
                            decoration.kind == DecorationKind::BlockQuoteBorder
                        }).collect();
                        assert_eq!(borders.len(), usize::from(source.starts_with('>')),
                            "One enclosing quote border per table row, regardless of cell alignment or size");
                        if let Some(border) = borders.first() {
                            assert!(matches!(border.owner, Some(DecorationOwner::Container(_))));
                            close(border.typographic_bounds.x, 23. + 17.);
                            close(border.typographic_bounds.width, 3.);
                            let cell = first_cell_row(snapshot, row_index, 0).table_cell.as_ref().unwrap();
                            close(border.typographic_bounds.y, cell.rect.y);
                            close(border.typographic_bounds.height, cell.rect.height);
                        }
                    }
                }
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn enclosing_table_style_change_invalidates_bounded_regional_geometry() {
    let source = format!("> | H | V |\n> | - | - |\n{}", "> | body | value |\n".repeat(10_000));
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(1000., 150.);
    let mut document = nested_table_document(&source, 17., "Start");
    for (id, left) in [(1, 17.), (2, 57.)] {
        configure_nested_table(&mut document, left, "Start");
        let request = prepare_layout_job(&document, &mut view, inspect_layout_provider(&engine),
            LayoutJobId(id), LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..3).unwrap()), LayoutCancellationToken::new()).unwrap();
        let result = compute_layout_job(&mut engine, &request, LayoutExecutionContext::WorkerPool).unwrap();
        let snapshot = result.regional_snapshot();
        assert!(snapshot.work_statistics().table_measured_cells() < 150);
        close(snapshot.lines()[0].rows()[0].table_cell.as_ref().unwrap().rect.x, left + 10. + 13.);
        for line in snapshot.lines() {
            let borders: Vec<_> = line.rows().iter().flat_map(|row| &row.decorations)
                .filter(|decoration| decoration.kind == DecorationKind::BlockQuoteBorder).collect();
            assert_eq!(borders.len(), 1);
            close(borders[0].typographic_bounds.x, left);
            close(borders[0].typographic_bounds.width, 3.);
        }
    }
}

#[test]
fn quoted_tables_paint_continuous_parent_boxes_behind_the_grid() {
    let table = "| Quoted item | Value |\n| --- | ---: |\n| Inside the quotation<br>second line | 7 |";
    for depth in [1, 2] {
        for surrounding_prose in [false, true] {
            let prefix = ">".repeat(depth);
            let table = table.lines().map(|line| format!("{prefix} {line}")).collect::<Vec<_>>().join("\n");
            let source = if surrounding_prose {
                format!("{prefix} before\n{prefix}\n{table}\n{prefix}\n{prefix} after")
            } else { table };
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let diagnostics = document.replace_style_defaults(&serde_json::to_vec(&json!({
                "version":1,"block_styles":[
                    {"id":"Paragraph","name":"Base Paragraph","role":"Paragraph","block":{}},
                    {"id":"Block quote","name":"Block quote","role":"Quote","based_on":"Paragraph",
                        "block":{"margin_left":17,"padding_left":7,"border_left_width":3,
                            "padding_top":8,"padding_bottom":10,
                            "background":{"red":1,"green":0,"blue":0,"alpha":0.5}}},
                    {"id":"Table","name":"Table","role":"Table","based_on":"Paragraph",
                        "block":{"margin_top":9,"margin_bottom":11,"padding_top":5,"padding_bottom":7,
                            "border_top_width":2,"border_bottom_width":2,
                            "background":{"red":0,"green":0,"blue":1,"alpha":1}}},
                    {"id":"Table cell","name":"Table cell","role":"Paragraph","based_on":"Paragraph",
                        "block":{"padding_top":6,"padding_bottom":4,"border_top_width":2,"border_bottom_width":2}},
                    {"id":"Table header","name":"Table header","role":"Paragraph","based_on":"Table cell","block":{}}
                ]
            })).unwrap()).unwrap();
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            let mut core = viem_core::Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 1000.);
            for scale in [1., 1.25, 2.] {
                let full = layout(core.document(), scale);
                core.handle(view, viem_core::CoreEvent::SetScale(scale)).unwrap();
                let regional = core.layout(view).unwrap().snapshot().unwrap();
                for snapshot in [&full, regional] {
                    let mut owners = std::collections::BTreeMap::new();
                    for row in snapshot.rows.iter() {
                        for border in row.decorations.iter().filter(|d| d.kind == DecorationKind::BlockQuoteBorder) {
                            assert!(matches!(border.owner, Some(DecorationOwner::Container(_))));
                            let bounds = border.typographic_bounds;
                            close(bounds.width, 3. * scale);
                            owners.entry(border.owner.unwrap()).or_insert_with(Vec::new).push(bounds);
                        }
                    }
                    assert_eq!(owners.len(), depth);
                    for bounds in owners.values() {
                        assert_eq!(bounds.len(), if surrounding_prose { 4 } else { 2 });
                        for pair in bounds.windows(2) {
                            close(pair[0].x, pair[1].x);
                            close(pair[0].y + pair[0].height, pair[1].y);
                        }
                        let table = &snapshot.tables()[0].rect;
                        assert!(bounds[0].x + bounds[0].width < table.x);
                        assert!(bounds[0].y < table.y);
                        let last = bounds.last().unwrap();
                        assert!(last.y + last.height > table.y + table.height);
                    }
                    let ordered = snapshot.decorations_in_paint_order();
                    let first_table = ordered.iter().position(|(_, d)| matches!(d.owner, Some(DecorationOwner::Table(_)))).unwrap();
                    assert!(ordered[..first_table].iter().all(|(_, d)| matches!(d.owner, Some(DecorationOwner::Container(_)))));
                    assert!(ordered[first_table..].iter().all(|(_, d)| !matches!(d.owner, Some(DecorationOwner::Container(_)))));
                }
                let geometry = |snapshot: &LayoutSnapshot| snapshot.rows.iter().flat_map(|r| &r.decorations)
                    .filter(|d| matches!(d.owner, Some(DecorationOwner::Container(_))))
                    .map(|d| (d.owner, d.kind, d.typographic_bounds)).collect::<Vec<_>>();
                let full_geometry = geometry(&full);
                let regional_geometry = geometry(regional);
                assert_eq!(full_geometry.len(), regional_geometry.len());
                for (full, regional) in full_geometry.iter().zip(&regional_geometry) {
                    assert_eq!((full.0, full.1), (regional.0, regional.1));
                    close(full.2.x, regional.2.x);
                    close(full.2.y, regional.2.y);
                    close(full.2.width, regional.2.width);
                    close(full.2.height, regional.2.height);
                }
                assert_eq!(core.document().source_bytes(), source.as_bytes());
            }
        }
    }
}
