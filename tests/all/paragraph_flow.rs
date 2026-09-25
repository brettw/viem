use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit};
use viem_core::layout::{
    compute_layout_job, HardLineLayoutRegion, LayoutCancellationToken, LayoutEngine,
    LayoutExecutionContext, LayoutJobPriority, LayoutJobRegion, MockTextMeasurementProvider,
    MAX_LONG_LINE_LAYOUT_SLICE_BYTES,
};
use viem_core::{Core, CoreEvent, Document, ViewId};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for ch in input.chars() {
        let step = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
            .unwrap();
        assert!(matches!(
            step.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ));
    }
}

#[test]
fn markdown_prose_flows_and_explicit_breaks_share_a_paragraph() {
    let source = "First **strong\nwords** here.\nA continuation.\n\nSecond paragraph.\n\nHard  \nbreak.\n\n```\ncode\nlines\n```\nTail.";
    let document = open(source, Format::Markdown);
    assert_eq!(document.text(), "First strong words here. A continuation.\nSecond paragraph.\nHard\nbreak.\ncode\nlines\nTail.");
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .filter(|block| &document.text()[block.range.clone()] == "Hard\nbreak.")
            .count(),
        1
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
    let text = document.text();
    let start = text.find("strong").unwrap();
    assert!(document
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.range.start == start && span.range.end == start + "strong words".len()));
    let fenced = "```\ncode\n```\n";
    let document = open(fenced, Format::Markdown);
    assert_eq!(document.text(), "code");
    assert_eq!(document.source_bytes(), fenced.as_bytes());
}

#[test]
fn source_flow_is_view_local_reversible_and_retains_pre_code_rows() {
    for (format, source, expected_lines) in [
        (
            Format::MarkdownSource,
            "One\ntwo.\n\n```\ncode\nlines\n```\nLast.",
            6,
        ),
        (
            Format::HtmlSource,
            "<p>One\ntwo.</p>\n<pre>code\nlines</pre>\n<p>Last.</p>",
            4,
        ),
    ] {
        let mut core = Core::new(open(source, format));
        let first = core.add_view(MockTextMeasurementProvider::new(), 2000.0, 800.0);
        let second = core.add_view(MockTextMeasurementProvider::new(), 2000.0, 800.0);
        let history = core.document().history_status();
        let formatted = core.document().text().to_owned();
        let normal_rows = core.layout(first).unwrap().snapshot().unwrap().rows.len();
        let before = core.layout(first).unwrap().configuration_generation();
        core.handle(first, CoreEvent::SetParagraphFlow(true))
            .unwrap();
        let after = core.layout(first).unwrap().snapshot().unwrap();
        assert_eq!(after.rows.len(), expected_lines, "{format:?}");
        assert_eq!(after.rows[0].text_range.start, 0);
        assert!(after.rows[0].text_range.end > source.find('\n').unwrap());
        assert_eq!(
            core.layout(second).unwrap().snapshot().unwrap().rows.len(),
            normal_rows
        );
        assert!(core.layout(first).unwrap().configuration_generation() > before);
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.document().text(), formatted);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(first, CoreEvent::SetParagraphFlow(false))
            .unwrap();
        assert_eq!(
            core.layout(first).unwrap().snapshot().unwrap().rows.len(),
            normal_rows
        );
    }
}

#[test]
fn markdown_paragraph_enter_paste_and_undo_keep_source_exact() {
    let source = "**First** words with café and emoji 👩🏽‍💻.\n\nSecond paragraph.\n\nTail.";
    let mut core = Core::new(open(source, Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 170.0, 600.0);
    keys(&mut core, view, "G0o");
    core.handle(view, CoreEvent::Input(InputEvent::text("Changed é👩🏽‍💻")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert!(core.document().text().ends_with("Tail.\nChanged é👩🏽‍💻"));
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    keys(&mut core, view, "ggyyGp");
    assert_eq!(core.document().text().matches("First").count(), 2);
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn local_edits_reparse_only_the_affected_prose_paragraph_in_large_documents() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = (0..10_000)
            .map(|i| format!("Paragraph {i} first\ncontinuation.\n\n"))
            .collect::<String>();
        let mut document = open(&source, format);
        let original_id = document.projection().blocks()[100].id;
        let start = document.text().find("Paragraph 9000").unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(start..start + "Paragraph".len(), "Section")],
            })
            .unwrap();
        let statistics = prepared.summary().projection_work();
        assert_eq!(
            statistics.scope(),
            ProjectionWorkScope::RegionalHardLines,
            "{format:?}: {statistics:?}"
        );
        assert!(statistics.decoded_source_bytes() < 256, "{statistics:?}");
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.projection().blocks()[100].id, original_id);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 170.0, 120.0);
        if format == Format::MarkdownSource {
            core.handle(view, CoreEvent::SetParagraphFlow(true))
                .unwrap();
        }
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.rows.len() < 200);
        let pending = core.prepare_view_layout_job(
            view,
            LayoutJobPriority::Background,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(1..2).unwrap()),
            LayoutCancellationToken::new(),
        );
        assert!(pending.is_ok());
    }
}

#[test]
fn markdown_terminal_newline_is_trivia_and_repeated_enter_authors_empty_paragraphs() {
    for source in ["", "word", "word\n", "**Word**", "one\nsoft line."] {
        let mut core = Core::new(open(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 170.0, 400.0);
        let prefix = core.document().text().to_owned();
        if source == "word\n" {
            assert_eq!(prefix, "word");
        }
        keys(&mut core, view, "GA");
        for count in 1..=3 {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap();
            assert_eq!(
                core.document().text(),
                format!("{prefix}{}", "\n".repeat(count))
            );
        }
        core.handle(view, CoreEvent::Input(InputEvent::text("Tail")))
            .unwrap();
        assert_eq!(core.document().text(), format!("{prefix}\n\n\nTail"));
        for _ in 0..5 {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
                .unwrap();
        }
        assert_eq!(core.document().text(), format!("{prefix}\n\n"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn flow_toggle_cancels_stale_work_and_long_source_paragraph_navigation_stays_bounded() {
    let source = format!("{}Tail.", "one two three four.\n".repeat(5_000));
    let mut core = Core::new(open(&source, Format::MarkdownSource));
    let view = core.add_view(MockTextMeasurementProvider::new(), 170.0, 120.0);
    let old = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::Background,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..2).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
    let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
    let stale = compute_layout_job(&mut worker, &old, LayoutExecutionContext::WorkerPool).unwrap();
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    assert!(core.install_view_layout_job(view, stale).is_err());
    let request = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::Background,
            LayoutJobRegion::HardLines(HardLineLayoutRegion::new(0..1).unwrap()),
            LayoutCancellationToken::new(),
        )
        .unwrap();
    assert!(request.captured_text_len() <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES + 4096);
    let at = source.len() - 2;
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    let snapshot = core.layout(view).unwrap().snapshot().unwrap();
    assert!(snapshot
        .rows
        .iter()
        .any(|row| row.text_range.start <= at && at <= row.text_range.end));
    assert!(
        snapshot.rows.last().unwrap().text_range.end - snapshot.rows[0].text_range.start
            <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES
    );
    assert_eq!(core.command_state(view).unwrap().cursor(), at);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn local_source_marker_edits_split_and_rejoin_flow_groups_without_scanning_the_paragraph() {
    let source = "ordinary prose line\n".repeat(10_000);
    let mut document = open(&source, Format::MarkdownSource);
    let at = document.line_start(5_000).unwrap();
    for (range, text) in [(at..at, "# "), (at..at + 2, "")] {
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(range, text)],
            })
            .unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::RegionalHardLines
        );
        assert_eq!(
            prepared.summary().projection_work().projected_hard_lines(),
            3
        );
        assert!(prepared.summary().projection_work().decoded_source_bytes() < 100);
        document.commit_model_transaction(prepared).unwrap();
        let fresh = Document::from_bytes(
            document.source_bytes(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(
            document.projection().presentation_line_count(true),
            fresh.projection().presentation_line_count(true)
        );
        for index in 0..fresh.projection().presentation_line_count(true) {
            assert_eq!(
                document.projection().presentation_line_range(index, true),
                fresh.projection().presentation_line_range(index, true)
            );
        }
    }
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn long_source_flow_retains_first_paragraph_geometry_after_checkpoints_and_resize() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    use viem_core::document::BoundaryAffinity;
    let source = ("word ".repeat(30) + "\n").repeat(2_000);
    let mut document = open(&source, Format::MarkdownSource);
    let mut defaults: serde_json::Value =
        serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
    for style in defaults["block_styles"].as_array_mut().unwrap() {
        if style["id"] == "Paragraph" {
            style["block"]["leading_indent"] = 40.into();
        }
    }
    document
        .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
        .unwrap();
    let paragraph = document.projection().blocks()[0].id;
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 120.0);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    for at in [0, 151_000] {
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
        for width in [240.0, 320.0] {
            core.handle(
                view,
                CoreEvent::Resize {
                    width,
                    height: 120.0,
                },
            )
            .unwrap();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            assert!(snapshot
                .rows
                .iter()
                .all(|row| row.paragraph_id == Some(paragraph) && row.paragraph_content_x == 40.0));
            assert!(
                snapshot.rows.last().unwrap().text_range.end - snapshot.rows[0].text_range.start
                    <= MAX_LONG_LINE_LAYOUT_SLICE_BYTES
            );
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
            assert!(composed
                .rows
                .iter()
                .all(|row| row.paragraph_id == Some(paragraph) && row.paragraph_content_x == 40.0));
            core.handle(view, CoreEvent::Composition(CompositionEvent::Cancel))
                .unwrap();
        }
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
