use viem_core::document::{Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit};
use viem_core::layout::{
    compute_layout_job, DocumentLayoutStyles, HardLineLayoutRegion, LayoutCancellationToken,
    LayoutEngine, LayoutExecutionContext, LayoutJobPriority, LayoutJobRegion,
    MockTextMeasurementProvider, ViewLayout,
};
use viem_core::{Core, CoreEvent, Document};

#[test]
fn flow_separates_same_source_line_blocks_and_applies_their_paragraph_geometry() {
    let source = "<h1>Hello, world</h1><p style='text-align: center; margin-block-start: 30px'>Now is the time</p><p></p>";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 2_000., 600.);
    assert_eq!(core.layout(view).unwrap().snapshot().unwrap().rows.len(), 1);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    let texts = snapshot
        .rows
        .iter()
        .map(|row| &source[row.text_range.clone()])
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        [
            "<h1>Hello, world</h1>",
            "<p style='text-align: center; margin-block-start: 30px'>Now is the time</p>",
            "<p></p>"
        ]
    );
    assert!(snapshot.rows[0].height() > snapshot.rows[1].height());
    assert!(
        snapshot.rows[1].clusters[0].x > 300.,
        "{:?}",
        snapshot.rows[1]
    );
    assert!(
        snapshot.rows[1].y - snapshot.rows[0].y >= snapshot.rows[0].height() + 22.5,
        "{:?}",
        snapshot
            .rows
            .iter()
            .map(|row| (row.y, row.height()))
            .collect::<Vec<_>>()
    );
    assert_ne!(snapshot.rows[0].paragraph_id, snapshot.rows[1].paragraph_id);
    assert_ne!(snapshot.rows[1].paragraph_id, snapshot.rows[2].paragraph_id);
    assert_eq!(core.document().text(), source);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::SetParagraphFlow(false))
        .unwrap();
    assert_eq!(core.layout(view).unwrap().snapshot().unwrap().rows.len(), 1);
}

#[test]
fn source_flow_keeps_container_markup_and_preformatted_rows_with_their_blocks() {
    for (source, expected) in [
        (
            "<div><p>One</p><p>Two</p></div>",
            vec!["<div><p>One</p>", "<p>Two</p></div>"],
        ),
        (
            "<h2>Title</h2><pre>one\ntwo</pre><p>Tail</p>",
            vec!["<h2>Title</h2>", "<pre>one", "two</pre>", "<p>Tail</p>"],
        ),
        (
            "<ul><li>One</li><li>Two</li></ul>",
            vec!["<ul><li>One</li>", "<li>Two</li></ul>"],
        ),
        (
            "<div>One</div><div>Two</div>",
            vec!["<div>One</div>", "<div>Two</div>"],
        ),
        ("<p>One<br>Two</p>", vec!["<p>One<br>", "Two</p>"]),
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 2_000., 600.);
        core.handle(view, CoreEvent::SetParagraphFlow(true))
            .unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| &source[row.text_range.clone()])
                .collect::<Vec<_>>(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn empty_source_flow_paragraph_uses_its_own_style_after_a_heading() {
    let source = "<h1>Title</h1><p></p>";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 2_000., 600.);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.rows.len(), 2);
    assert!(snapshot.rows[1].natural_height() < snapshot.rows[0].natural_height());
    let paragraph = snapshot.rows[1].paragraph_id;
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    let at = source.rfind("</p>").unwrap();
    let target = CompositionTarget::at_offsets(core.document(), at..at).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("é", 2..2))),
    )
    .unwrap();
    let composed = core.presentation_layout(view).unwrap().snapshot().unwrap();
    assert_eq!(composed.rows.len(), 2);
    assert_eq!(composed.rows[1].paragraph_id, paragraph);
    assert!(composed.rows[1].natural_height() < composed.rows[0].natural_height());
    core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn source_flow_row_navigation_delete_insert_and_undo_preserve_source_blocks() {
    use viem_core::command::{CommandStatus, InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    let source = "<h1>Title</h1><p>Body</p><p>Tail</p>";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 2_000., 600.);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let keys = |core: &mut Core<MockTextMeasurementProvider>, input: &str| {
        for ch in input.chars() {
            let outcome = core
                .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
                .unwrap();
            assert!(matches!(
                outcome.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ));
        }
    };
    let ids = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .map(|row| row.paragraph_id)
        .collect::<Vec<_>>();
    keys(&mut core, "$j0");
    let at = source.find("<p>").unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
    keys(&mut core, "dd");
    assert_eq!(core.document().text(), "<h1>Title</h1><p>Tail</p>");
    assert_eq!(
        core.layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| row.paragraph_id)
            .collect::<Vec<_>>(),
        [ids[0], ids[2]]
    );
    keys(&mut core, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity,
                extend_selection: false,
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap().snapshot().unwrap();
        let geometry = layout.logical_endpoint_geometry(at, affinity).unwrap();
        let expected = if affinity == BoundaryAffinity::Upstream {
            layout.rows[0].y
        } else {
            layout.rows[1].y
        };
        assert_eq!(geometry.rect.y, expected);
    }
    keys(&mut core, "i");
    core.handle(view, CoreEvent::Input(InputEvent::text("<p>New</p>")))
        .unwrap();
    assert_eq!(
        core.document().text(),
        "<h1>Title</h1><p>New</p><p>Body</p><p>Tail</p>"
    );
    let inserted = core.layout(view).unwrap().snapshot().unwrap();
    assert_eq!(inserted.rows.len(), 4);
    assert_eq!(inserted.rows[2].paragraph_id, ids[1]);
    assert_eq!(inserted.rows[3].paragraph_id, ids[2]);
    assert!(!ids.contains(&inserted.rows[1].paragraph_id));
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    keys(&mut core, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn source_flow_edit_preserves_untouched_paragraphs_and_shaping_cache() {
    let source = (0..128)
        .map(|index| format!("<h2>Title {index}</h2><p>Body {index}</p>\n"))
        .collect::<String>();
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(400);
    let mut view = ViewLayout::new(2_000., 600.);
    view.set_paragraph_flow(true);
    engine.relayout(&document, &mut view).unwrap();
    let ids = view
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .map(|row| row.paragraph_id)
        .collect::<Vec<_>>();
    let calls = engine.provider().request_calls();
    let at = document.text().find("Body 64").unwrap() + 5;
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "new ")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 256);
    document.commit_model_transaction(prepared).unwrap();
    engine.relayout(&document, &mut view).unwrap();
    assert_eq!(
        view.snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| row.paragraph_id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(engine.provider().request_calls() - calls <= 1);
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::HtmlSource).unwrap();
    let mut fresh_view = ViewLayout::new(2_000., 600.);
    fresh_view.set_paragraph_flow(true);
    engine.relayout(&fresh, &mut fresh_view).unwrap();
    let geometry = |view: &ViewLayout| {
        view.snapshot()
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                (
                    row.text_range.clone(),
                    row.y,
                    row.height(),
                    row.paragraph_content_x,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(geometry(&view), geometry(&fresh_view));

    let at = document.text().find("<h2>Title 64</h2>").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "<p>Added</p>")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(prepared).unwrap();
    let calls = engine.provider().request_calls();
    engine.relayout(&document, &mut view).unwrap();
    let next_ids = view
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .map(|row| row.paragraph_id)
        .collect::<Vec<_>>();
    assert_eq!(next_ids.len(), ids.len() + 1);
    assert_eq!(&next_ids[..128], &ids[..128]);
    assert_eq!(&next_ids[129..], &ids[128..]);
    assert!(!ids.contains(&next_ids[128]));
    assert!(engine.provider().request_calls() - calls <= 1);
}

#[test]
fn large_html_source_flow_captures_only_visible_semantic_blocks_and_rejects_stale_jobs() {
    let source = "<h2>Title</h2><p>Body text</p>".repeat(2_000);
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 640., 150.);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let request = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::Background,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(2_000..2_002).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
    assert!(request.captured_text_len() < 128);
    let statistics = request.capture_statistics();
    assert!(statistics.document_paragraph_styles() < 5, "{statistics:?}");
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let stale =
        compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool).unwrap();
    core.handle(view, CoreEvent::SetParagraphFlow(false))
        .unwrap();
    assert!(core.install_view_layout_job(view, stale).is_err());
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    for width in [200., 800.] {
        core.handle(
            view,
            CoreEvent::Resize {
                width,
                height: 150.,
            },
        )
        .unwrap();
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.rows.len() < 100);
        assert!(snapshot.coverage.hard_lines().len() < 100);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn consecutive_source_breaks_never_create_empty_semantic_style_spans() {
    for source in [
        "<pre>first\n\nsecond</pre>",
        "<pre>first\r\n\r\n\r\nsecond</pre>",
        "<p title='first\n\nsecond'>café &amp; text</p>\n\n<!--one\n\ntwo-->",
        "<pre>first<br><span style='white-space: pre-wrap'>&#32;</span>added\n\nlast</pre>",
    ] {
        let document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap();
        assert!(
            document
                .projection()
                .style_spans()
                .iter()
                .all(|span| !span.range.is_empty()),
            "{source:?}"
        );
        DocumentLayoutStyles::resolve(document.projection()).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn large_source_code_with_blank_lines_keeps_resize_and_zoom_layout_bounded() {
    let source = "<pre>first\n\nlast</pre>\n".repeat(10_000);
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::HtmlSource,
    )
    .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 640., 150.);
    let original_configuration = core.layout(view).unwrap().configuration_generation();
    core.handle(view, CoreEvent::SetScale(1.75)).unwrap();
    for width in [200., 800.] {
        core.handle(
            view,
            CoreEvent::Resize {
                width,
                height: 150.,
            },
        )
        .unwrap();
        let layout = core.layout(view).unwrap();
        assert!(layout.configuration_generation() > original_configuration);
        let snapshot = layout.snapshot().unwrap();
        assert!(!snapshot.rows.is_empty());
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.coverage.hard_lines().len() < 100);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision().0, 0);
}
