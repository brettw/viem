use viem_core::document::{BoundaryAffinity, Encoding, Format, ModelRequest, StyleNamespace};
use viem_core::layout::{DecorationKind, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

fn assign(document: &mut Document, range: std::ops::Range<usize>, style: &str) {
    document
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range,
            namespace: StyleNamespace::Block,
            style: style.into(),
        })
        .unwrap_or_else(|error| panic!("{style}: {error:?}; source={:?}", String::from_utf8_lossy(&document.source_bytes())));
}

#[test]
fn native_html_quotes_project_paragraphs_without_extra_container_lines() {
    for (source, expected, quote_count) in [
        (
            "<blockquote cite='keep'>one <b>two</b></blockquote><p>outside</p>",
            "one two\noutside",
            1,
        ),
        (
            "<BLOCKQUOTE><p>one<br>two</p><p>three</p></BLOCKQUOTE><p>outside</p>",
            "one\ntwo\nthree\noutside",
            2,
        ),
        ("<blockquote></blockquote><p>outside</p>", "\noutside", 1),
    ] {
        let document = open(source, Format::Html);
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(
            document
                .projection()
                .blocks()
                .iter()
                .filter(|block| block.style.0 == "Block quote")
                .count(),
            quote_count
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            document.projection().blocks().last().unwrap().style.0,
            "Paragraph"
        );
    }
}

#[test]
fn quote_assignment_uses_native_html_and_round_trips_exact_source() {
    for source in [
        "<p data-keep='yes'>one <b>two</b></p><!--keep-->",
        "<p></p>",
    ] {
        let mut document = open(source, Format::Html);
        let text = document.text().to_owned();
        assign(&mut document, 0..0, "Block quote");
        assert_eq!(document.text(), text);
        let saved = document.source_bytes();
        let syntax = String::from_utf8(saved.clone()).unwrap();
        assert!(syntax.contains("<blockquote"), "{syntax}");
        assert!(!syntax.contains("class="), "{syntax}");
        let reopened = open(&syntax, Format::Html);
        assert_eq!(reopened.text(), text);
        assert_eq!(reopened.projection().blocks()[0].style.0, "Block quote");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
        assign(&mut document, 0..0, "Paragraph");
        assert_eq!(document.text(), text);
        assert_eq!(document.projection().blocks()[0].style.0, "Paragraph");
    }
}

#[test]
fn removing_quote_from_one_contained_paragraph_preserves_its_siblings_and_attributes() {
    let source = "<!--keep--><blockquote cite='original'><p>one</p><h2>two</h2><p>three</p></blockquote><!--end-->";
    let mut document = open(source, Format::Html);
    assign(&mut document, 4..7, "Paragraph");
    assert_eq!(document.text(), "one\ntwo\nthree");
    let saved = document.source_bytes();
    let syntax = String::from_utf8(saved.clone()).unwrap();
    assert!(
        syntax.contains("</blockquote><p>two</p><blockquote cite='original'>"),
        "{syntax}"
    );
    let reopened = open(&syntax, Format::Html);
    assert_eq!(
        reopened
            .projection()
            .blocks()
            .iter()
            .map(|block| block.style.0.as_str())
            .collect::<Vec<_>>(),
        ["Block quote", "Paragraph", "Block quote"]
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
}

#[test]
fn source_quote_style_includes_visible_tags_and_border_has_no_text_positions() {
    let source = "<blockquote>one <b>two</b></blockquote>";
    for format in [Format::Html, Format::HtmlSource] {
        let mut core = Core::new(open(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 140., 300.);
        let text = core.document().text().to_owned();
        let selected = core.selected_named_styles(view).unwrap();
        assert_eq!(selected.paragraph.unwrap().0, "Block quote");
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows.len() > 1 || format == Format::Html);
        for row in snapshot.rows.iter() {
            let border = row
                .decorations
                .iter()
                .find(|item| item.kind == DecorationKind::BlockQuoteBorder)
                .unwrap();
            assert!(border.text.is_empty());
            assert!(border.ink_bounds.x < row.paragraph_content_x);
            assert_eq!(border.ink_bounds.height, row.height());
            for caret in &row.carets {
                core.document().text_point(caret.point.text_offset).unwrap();
            }
        }
        assert_eq!(core.document().text(), text);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        if format == Format::HtmlSource {
            assert_eq!(text, source);
        }
    }
}

#[test]
fn quote_style_change_invalidates_local_layout_and_undo_restores_border() {
    let source = format!(
        "<p>outside</p>{}<blockquote>last quoted paragraph</blockquote>",
        "<p>body</p>".repeat(10_000)
    );
    let mut core = Core::new(open(&source, Format::Html));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240., 200.);
    let at = core.document().text().find("last quoted").unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let before = core.layout(view).unwrap().regional_cache_statistics();
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Block,
            style: "Paragraph".into(),
        },
    )
    .unwrap();
    let after = core.layout(view).unwrap().regional_cache_statistics();
    assert!(before.hard_line_count() < 500);
    assert!(after.hard_line_count() < 500);
    assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 500);
    assert!(core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .all(|row| row
            .decorations
            .iter()
            .all(|decoration| decoration.kind != DecorationKind::BlockQuoteBorder)));
    core.handle(
        view,
        CoreEvent::Input(viem_core::command::InputEvent::key('u')),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .any(|row| row
            .decorations
            .iter()
            .any(|decoration| decoration.kind == DecorationKind::BlockQuoteBorder)));
}

#[test]
fn markdown_and_html_share_quote_geometry_and_source_flow_controls_decoration() {
    let mut geometry = Vec::new();
    for (format, source) in [
        (Format::Html, "<blockquote>one two</blockquote>"),
        (Format::Markdown, "> one two"),
    ] {
        let mut core = Core::new(open(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 240., 200.);
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        let border = row
            .decorations
            .iter()
            .find(|item| item.kind == DecorationKind::BlockQuoteBorder)
            .unwrap();
        geometry.push((
            row.paragraph_content_x,
            row.paragraph_content_width,
            border.ink_bounds,
        ));
    }
    assert_eq!(geometry[0], geometry[1]);

    let source = "> first\n> second";
    let mut core = Core::new(open(source, Format::MarkdownSource));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let other = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let original = core.layout(view).unwrap().snapshot().unwrap().clone();
    assert!(original.rows.iter().all(|row| row.decorations.is_empty()));
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let flowed = core.layout(view).unwrap().snapshot().unwrap();
    assert!(flowed.rows[0].paragraph_content_x > original.rows[0].paragraph_content_x);
    assert!(flowed.rows.iter().any(|row| row
        .decorations
        .iter()
        .any(|item| item.kind == DecorationKind::BlockQuoteBorder)));
    assert!(core
        .layout(other)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .all(|row| row.decorations.is_empty()));
    assert_eq!(core.document().text(), source);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::SetParagraphFlow(false))
        .unwrap();
    let restored = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(
        restored.rows[0].paragraph_content_x,
        original.rows[0].paragraph_content_x
    );
    assert!(restored.rows.iter().all(|row| row.decorations.is_empty()));
}

#[test]
fn enter_and_typing_keep_native_quote_paragraphs_and_exact_history() {
    let source = "<blockquote data-keep='yes'>word</blockquote><!--keep-->";
    let mut core = Core::new(open(source, Format::Html));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240., 200.);
    for event in [
        viem_core::command::InputEvent::key('A'),
        viem_core::command::InputEvent::Key(viem_core::command::Key::Enter),
        viem_core::command::InputEvent::text("tail"),
        viem_core::command::InputEvent::Key(viem_core::command::Key::Escape),
    ] {
        core.handle(view, CoreEvent::Input(event)).unwrap();
    }
    assert_eq!(core.document().text(), "word\ntail");
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), "word\ntail");
    assert!(reopened
        .projection()
        .blocks()
        .iter()
        .all(|block| block.style.0 == "Block quote"));
    core.handle(
        view,
        CoreEvent::Input(viem_core::command::InputEvent::key('u')),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(
        view,
        CoreEvent::Input(viem_core::command::InputEvent::Key(
            viem_core::command::Key::Ctrl('r'),
        )),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), saved);
}

#[test]
fn quote_assignment_replaces_native_heading_code_and_list_styles() {
    for format in [Format::Html, Format::HtmlSource] {
        for source in [
            "<h2 data-keep='heading'>one</h2>",
            "<pre data-keep='code'>one\n two</pre>",
            "<ul data-keep='list'><li data-keep='item'>one <b>two</b></li><li>other</li></ul>",
            "<ol start='3'><li><p data-keep='para'>one</p><ul><li>nested</li></ul></li></ol>",
        ] {
            let mut document = open(source, format);
            let original_text = open(source, Format::Html).text().to_owned();
            let at = document.text().find("one").unwrap();
            assign(&mut document, at..at, "Block quote");
            let saved = document.source_bytes();
            let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(reopened.text(), document.text());
            let visible = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(visible.text(), original_text);
            let block = &visible.projection().blocks()[0];
            assert_eq!(block.style.0, "Block quote");
            assert!(!visible.projection().list_structure().lists.iter().any(|list| list.items.iter().any(|item| item.paragraph_ids.contains(&block.id))));
            assert!(!String::from_utf8(saved.clone()).unwrap().contains("<pre"));
            assert!(!String::from_utf8(saved.clone()).unwrap().contains("<h2"));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
            let at = document.text().find("one").unwrap();
            assign(&mut document, at..at, "Paragraph");
            let plain = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
            assert_eq!(plain.projection().blocks()[0].style.0, "Paragraph");
            assert_eq!(plain.text(), original_text);
        }
    }
}

#[test]
fn quote_assignment_stops_at_the_next_implicit_paragraph_opener() {
    for format in [Format::Html, Format::HtmlSource] {
        let source = "<p data-x='keep'>one<P>two";
        let mut document = open(source, format);
        let at = document.text().find("one").unwrap();
        assign(&mut document, at..at, "Block quote");
        assert_eq!(
            document.source_bytes(),
            b"<blockquote><p data-x='keep'>one</blockquote><P>two"
        );
        let visible = open(
            &String::from_utf8(document.source_bytes()).unwrap(),
            Format::Html,
        );
        assert_eq!(visible.text(), "one\ntwo");
        assert_eq!(visible.projection().blocks()[0].style.0, "Block quote");
        assert_eq!(visible.projection().blocks()[1].style.0, "Paragraph");
        let at = document.text().find("one").unwrap();
        assign(&mut document, at..at, "Paragraph");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn quote_container_survives_inner_named_paragraphs_and_list_headings() {
    let source = "<blockquote><p class='viem-p-48656164696e6732'>one <b>bold</b></p><ul><li><h2>heading</h2></li></ul><pre>code</pre></blockquote>";
    for format in [Format::Html, Format::HtmlSource] {
        let document = open(source, format);
        assert!(
            document
                .projection()
                .blocks()
                .iter()
                .all(|block| block.style.0 == "Block quote"),
            "{format:?}: {:?}",
            document.projection().blocks()
        );
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let document = open(source, Format::Html);
    let bold = document.text().find("bold").unwrap();
    let code = document.text().find("code").unwrap();
    assert!(
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), bold, false)
            .unwrap()
            .bold
    );
    let expected = open("<pre>code</pre>", Format::Html);
    assert_eq!(
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), code, false)
            .unwrap()
            .font_families,
        viem_core::layout::DocumentLayoutStyles::character_at(expected.projection(), 0, false)
            .unwrap()
            .font_families
    );
}

#[test]
fn enter_in_quoted_pre_keeps_code_and_exits_an_empty_quote_with_exact_history() {
    for source in [
        "<blockquote><pre data-x='keep'>code</pre></blockquote>",
        "<blockquote><pre></pre></blockquote>",
    ] {
        let mut core = Core::new(open(source, Format::Html));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 250.);
        let before = core.document().text().to_owned();
        for event in [
            viem_core::command::InputEvent::key('A'),
            viem_core::command::InputEvent::Key(viem_core::command::Key::Enter),
            viem_core::command::InputEvent::text("tail"),
            viem_core::command::InputEvent::Key(viem_core::command::Key::Escape),
        ] {
            let description = format!("{event:?}");
            core.handle(view, CoreEvent::Input(event))
                .unwrap_or_else(|error| {
                    panic!(
                        "{error:?}: {source:?}, {description}, bytes={:?}",
                        core.document().source_bytes()
                    )
                });
        }
        assert_eq!(core.document().text(), if before.is_empty() { "tail".to_owned() } else { format!("{before}\ntail") });
        assert_eq!(core.document().projection().blocks().len(), 1);
        assert_eq!(
            core.document().projection().blocks()[0].style.0,
            if before.is_empty() { "Paragraph" } else { "Block quote" }
        );
        let saved = core.document().source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert_eq!(reopened.projection().blocks().len(), 1);
        core.handle(
            view,
            CoreEvent::Input(viem_core::command::InputEvent::key('u')),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(
            view,
            CoreEvent::Input(viem_core::command::InputEvent::Key(
                viem_core::command::Key::Ctrl('r'),
            )),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}

#[test]
fn source_edits_inside_quoted_code_keep_the_reopened_paragraph_partition() {
    let source =
        "<p>before</p>\n<blockquote>\n<pre>first\nmiddle\nlast</pre>\n</blockquote>\n<p>after</p>";
    let mut document = open(source, Format::HtmlSource);
    let at = document.text().find("middle").unwrap();
    document
        .apply_edits(vec![viem_core::document::TextEdit::new(at..at, "new ")])
        .unwrap();
    let saved = document.source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::HtmlSource).unwrap();
    assert_eq!(document.text(), reopened.text());
    let paragraphs = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| (block.range.clone(), block.style.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(paragraphs(&document), paragraphs(&reopened));
    assert!(document
        .projection()
        .blocks()
        .iter()
        .any(|block| block.range.contains(&at) && block.style.0 == "Block quote"));
    assert_eq!(saved, source.replacen("middle", "new middle", 1).as_bytes());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
}
