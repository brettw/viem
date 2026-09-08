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
    for (format, source, code_indent) in [
        (
            Format::Markdown,
            "Before.\n\n```\n  one\n\n  two\n```\n\nAfter.",
            32.,
        ),
        (
            Format::Html,
            "<p>Before.</p><pre>  one\n\n  two</pre><p>After.</p>",
            0.,
        ),
    ] {
        let document = open(source, format);
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs.len(), 3);
        for paragraph in &styles.paragraphs {
            assert_eq!(paragraph.spacing_before, 7.);
            assert_eq!(paragraph.spacing_after, 7.);
        }
        assert_eq!(styles.paragraphs[1].leading_indent, code_indent);
        assert_eq!(document.text(), "Before.\n  one\n\n  two\nAfter.");
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 800., 800.);
        let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].paragraph_id, rows[2].paragraph_id);
        assert_eq!(rows[2].paragraph_id, rows[3].paragraph_id);
        assert_ne!(rows[0].paragraph_id, rows[1].paragraph_id);
        assert_ne!(rows[3].paragraph_id, rows[4].paragraph_id);
        for (row, next) in rows.iter().zip(rows.iter().skip(1)) {
            let expected_gap = if row.paragraph_id == next.paragraph_id {
                0.
            } else {
                14.
            };
            assert!((next.y - row.y - row.line_advance - expected_gap).abs() < 0.01);
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    for format in [Format::PlainText, Format::Rtf] {
        let source = if format == Format::Rtf {
            r"{\rtf1 Text}"
        } else {
            "Text"
        };
        let document = open(source, format);
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs[0].spacing_before, 0.);
        assert_eq!(styles.paragraphs[0].spacing_after, 0.);
    }
}

#[test]
fn paragraph_defaults_remain_overridable_without_serializing_on_open() {
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&Document::new("").export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["block"]["spacing_before"] = 11.into();
            style["block"]["spacing_after"] = 13.into();
        }
    }
    for (format, source, expected_before) in [
        (Format::Markdown, "Text", 11.),
        (Format::Html, "<p>Text</p>", 11.),
        (
            Format::Html,
            "<p style='margin-block-start:19pt'>Text</p>",
            19.,
        ),
    ] {
        let mut document = open(source, format);
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(styles.paragraphs[0].spacing_before, expected_before);
        assert_eq!(styles.paragraphs[0].spacing_after, 13.);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision().0, 0);
    }
}

#[test]
fn list_body_indents_are_signed_and_derived_list_styles_do_not_double_the_inset() {
    for indent in [-10., 0., 10.] {
        let source = format!("<ul><li style='text-indent:{indent}pt'>First words with enough text to wrap across several rows.</li></ul>");
        let mut core = Core::new(open(&source, Format::Html));
        let body = core.document().text().find("First").unwrap();
        let view = core.add_view(MockTextMeasurementProvider::new(), 175., 800.);
        let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
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
    let document = open("<ul><li>Words</li></ul>", Format::Html);
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
    assert_eq!(styles.paragraphs[0].leading_indent, 32.);
}

#[test]
fn wrapped_list_labels_hang_and_end_of_row_is_stable_through_edit_and_undo() {
    for (format, source) in [
        (Format::Markdown, "1. First words on the source line\n   continuation words with enough text to wrap.\n2. Second item."),
        (Format::Markdown, "99) First words on the source line\n    continuation words with enough text to wrap.\n1) Second item."),
        (Format::Markdown, "- First words on the source line\n  continuation words with enough text to wrap.\n- Second item."),
        (Format::Html, "<ol start='99'><li>First words on the source line\ncontinuation words with enough text to wrap.</li><li>Second item.</li></ol>"),
        (Format::Html, "<ul><li>First words on the source line\ncontinuation words with enough text to wrap.</li><li>Second item.</li></ul>"),
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
