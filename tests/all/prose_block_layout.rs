use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, DocumentStyleInput, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, ViewId};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for ch in input.chars() {
        let step = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
            .unwrap();
        assert!(
            matches!(
                step.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "key {ch:?}"
        );
    }
}

#[test]
fn rich_defaults_separate_paragraphs_and_code_without_changing_source() {
    for (format, source, code_indent) in [(
        Format::Markdown,
        "Before.\n\n```\n  one\n\n  two\n```\n\nAfter.",
        32.,
    )] {
        let mut document = open(source, format);
        let mut defaults: serde_json::Value =
            serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        for style in defaults["block_styles"].as_array_mut().unwrap() {
            if style["id"] == "Paragraph" || style["id"] == "Code Block" {
                style["block"]["margin_top"] = 7.into();
                style["block"]["margin_bottom"] = 7.into();
            }
            if style["id"] == "Code Block" {
                for property in [
                    "margin_left",
                    "padding_top",
                    "padding_bottom",
                    "border_top_width",
                    "border_bottom_width",
                    "border_left_width",
                ] {
                    style["block"][property] = 0.into();
                }
                style["block"]["padding_left"] = code_indent.into();
            }
        }
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs.len(), 3);
        for (index, paragraph) in styles.paragraphs.iter().enumerate() {
            assert_eq!(paragraph.margin_top, if index == 1 { 0. } else { 7. });
            assert_eq!(paragraph.margin_bottom, if index == 1 { 0. } else { 7. });
        }
        assert_eq!(
            styles.paragraphs[1]
                .containers
                .iter()
                .map(|c| c.style.left())
                .sum::<f32>(),
            code_indent
        );
        assert_eq!(document.text(), "Before.\n  one\n\n  two\nAfter.");
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 800., 800.);
        let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].paragraph_id, rows[2].paragraph_id);
        assert_eq!(rows[2].paragraph_id, rows[3].paragraph_id);
        assert_ne!(rows[0].paragraph_id, rows[1].paragraph_id);
        assert_ne!(rows[3].paragraph_id, rows[4].paragraph_id);
        let caption = rows[1]
            .decorations
            .iter()
            .find(|decoration| decoration.kind == viem_core::layout::DecorationKind::CodeLanguage)
            .unwrap();
        assert_eq!(caption.text, "None ▾");
        assert_eq!(caption.typographic_bounds.height, 16.);
        assert!(
            (caption.typographic_bounds.y - rows[0].y - rows[0].line_advance - 9.).abs() < 0.01,
            "the caption begins inside the code box, below the collapsed paragraph margin"
        );
        assert!(
            (caption.typographic_bounds.y + caption.typographic_bounds.height - rows[1].y).abs()
                < 0.01,
            "the first body row starts after the caption's reserved header"
        );
        for (index, (row, next)) in rows.iter().zip(rows.iter().skip(1)).enumerate() {
            let expected_gap = if row.paragraph_id == next.paragraph_id {
                0.
            } else if index == 0 {
                7. + 18.
            } else {
                7.
            };
            assert!((next.y - row.y - row.line_advance - expected_gap).abs() < 0.01);
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    for format in [Format::PlainText] {
        let source = "Text";
        let document = open(source, format);
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs[0].margin_top, 0.);
        assert_eq!(styles.paragraphs[0].margin_bottom, 0.);
    }
}

#[test]
fn paragraph_defaults_remain_overridable_without_serializing_on_open() {
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&Document::new("").export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["block"]["margin_top"] = 11.into();
            style["block"]["margin_bottom"] = 13.into();
        }
    }
    for (format, source, expected_before) in [(Format::Markdown, "Text", 11.)] {
        let mut document = open(source, format);
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs[0].margin_top, expected_before);
        assert_eq!(styles.paragraphs[0].margin_bottom, 13.);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision().0, 0);
    }
}

#[test]
fn list_body_indents_are_signed_and_derived_list_styles_do_not_double_the_inset() {
    for indent in [-10., 0., 10.] {
        let document = open(
            "- First words with enough text to wrap across several rows.",
            Format::Markdown,
        );
        let mut styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        styles.paragraphs[0].first_line_indent = indent;
        let body = document.text().find("First").unwrap();
        let mut engine = viem_core::layout::LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = viem_core::layout::ViewLayout::new(175., 800.);
        engine
            .relayout_styled_text(
                document.id(),
                document.revision(),
                document.text(),
                styles,
                &mut view,
            )
            .unwrap();
        let rows = &view.snapshot().unwrap().rows;
        let body_x = rows[0]
            .clusters
            .iter()
            .find(|c| c.text_range.start == body)
            .unwrap()
            .x;
        assert!(
            (body_x - rows[1].clusters[0].x - indent).abs() < 0.01,
            "{indent}"
        );
    }
    let document = open("- Words", Format::Markdown);
    let projection = document.projection();
    let mut sheet = projection.style_sheet().clone();
    let mut style = sheet.block_style(&"BulletedList1".into()).unwrap().clone();
    style.id = "CustomList".into();
    style.based_on = Some("BulletedList1".into());
    style.block = Default::default();
    sheet
        .insert_block_style(
            style,
            viem_core::document::StyleDefinitionMetadata::generated("Custom List"),
        )
        .unwrap();
    let mut blocks = projection.blocks().to_vec();
    blocks[0].style = "CustomList".into();
    let styles = DocumentLayoutStyles::resolve_input(DocumentStyleInput {
        text: projection.text(),
        blocks: &blocks,
        style_spans: projection.style_spans(),
        style_sheet: &sheet,
        document_style: projection.document_style(),
    })
    .unwrap();
    assert_eq!(
        styles.paragraphs[0].leading_indent
            + styles.paragraphs[0]
                .containers
                .iter()
                .map(|c| c.style.left())
                .sum::<f32>(),
        32.
    );
}

#[test]
fn wrapped_list_labels_hang_and_end_of_row_is_stable_through_edit_and_undo() {
    for (format, source) in [
        (Format::Markdown, "1. First words on the source line\n   continuation words with enough text to wrap.\n2. Second item."),
        (Format::Markdown, "99) First words on the source line\n    continuation words with enough text to wrap.\n1) Second item."),
        (Format::Markdown, "- First words on the source line\n  continuation words with enough text to wrap.\n- Second item."),
    ] {
        let mut core = Core::new(open(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 175., 800.);
        let before = core.document().text().to_owned();
        let body_start = before.find("First").unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        let first = &snapshot.rows[0];
        let continuation = &snapshot.rows[1];
        assert_eq!(first.paragraph_id, continuation.paragraph_id, "{format:?}");
        let body_x = first.clusters.iter().find(|c| c.text_range.start == body_start).unwrap().x;
        assert!((body_x - continuation.clusters[0].x).abs() < 0.01, "{format:?}: {body_x} vs {}", continuation.clusters[0].x);
        let row_end = first.text_range.start + before[first.text_range.clone()].trim_end_matches([' ', '\t']).len();
        keys(&mut core, view, "$$");
        assert_eq!(core.command_state(view).unwrap().visual_position().unwrap().text_offset, row_end, "{format:?}");
        assert_eq!(core.command_state(view).unwrap().boundary_affinity(), BoundaryAffinity::Upstream);
        keys(&mut core, view, "A");
        core.handle(view, CoreEvent::Input(InputEvent::text("XYZ"))).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        let mut expected = before.clone();
        expected.insert_str(row_end, "XYZ");
        assert_eq!(core.document().text(), expected, "{format:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Resize {width: 130., height: 800.}).unwrap();
        keys(&mut core, view, "gg$$");
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        let end = row.text_range.start + before[row.text_range.clone()].trim_end_matches([' ', '\t']).len();
        assert_eq!(core.command_state(view).unwrap().visual_position().unwrap().text_offset, end);
    }
}

#[test]
fn list_geometry_invalidates_locally_after_resize_and_marker_edit_in_large_document() {
    let source = (1..=10_000)
        .map(|i| format!("{i}. Item with enough words to wrap onto multiple rows.\n"))
        .collect::<String>();
    let mut core = Core::new(open(&source, Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 120.);
    let old_generation = core.layout(view).unwrap().configuration_generation();
    let old_end = core.layout(view).unwrap().snapshot().unwrap().rows[0]
        .text_range
        .end;
    core.handle(
        view,
        CoreEvent::Resize {
            width: 100.,
            height: 120.,
        },
    )
    .unwrap();
    assert!(core.layout(view).unwrap().configuration_generation() > old_generation);
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(snapshot.rows[0].text_range.end < old_end);
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    keys(&mut core, view, "gg$A");
    core.handle(view, CoreEvent::Input(InputEvent::text("new ")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(!snapshot.coverage.is_full_document());
    assert!(snapshot.coverage.hard_lines().len() < 100);
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
