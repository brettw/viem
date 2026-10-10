use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, ContainerKind, Document, Encoding, Format, HistoryNavigationRequest,
    ModelRequest, ProjectionWorkScope, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn input(core: &mut Core<MockTextMeasurementProvider>, view: viem_core::ViewId, event: InputEvent) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(event))
        .unwrap();
    if let Some(command) = result.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{:?}",
            command.status
        );
    }
}

fn assert_reopened(document: &Document) {
    let fresh = Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        document.format(),
        document.file_format(),
    )
    .unwrap();
    assert_eq!(
        document.text(),
        fresh.text(),
        "source: {:?}",
        String::from_utf8_lossy(&document.source_bytes())
    );
    // Container anchors belong to a document instance. Compare their roles
    // and membership while checking all other parsed attributes exactly.
    let blocks = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| {
                let mut attributes = (*block.attributes).clone();
                for member in std::sync::Arc::make_mut(&mut attributes.containers) {
                    std::sync::Arc::make_mut(&mut member.container).id.anchor = 0;
                }
                (block.range.clone(), attributes)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(document), blocks(&fresh));
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
}

fn check(source: &str, format: Format, at: usize, action: InputEvent, expected: &str) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let quotes = document
        .projection()
        .container_structure()
        .containers
        .iter()
        .filter(|node| node.attributes.kind == ContainerKind::Quote)
        .count();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    input(&mut core, view, InputEvent::key('i'));
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
    input(&mut core, view, action);
    if format == Format::Markdown {
        assert_eq!(
            core.document()
                .projection()
                .container_structure()
                .containers
                .iter()
                .filter(|node| node.attributes.kind == ContainerKind::Quote)
                .count(),
            quotes
        );
    }
    assert_eq!(
        core.document().text(),
        expected,
        "source: {:?}",
        String::from_utf8_lossy(&core.document().source_bytes())
    );
    assert_reopened(core.document());
    let saved = core.document().source_bytes();
    input(&mut core, view, InputEvent::Key(Key::Escape));
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(
        view,
        CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
    )
    .unwrap();
    assert_eq!(core.document().source_bytes(), saved);
    assert_reopened(core.document());
}

#[test]
fn typing_before_a_setext_like_paragraph_keeps_its_text_and_structure() {
    check(
        "\n====\n",
        Format::Markdown,
        0,
        InputEvent::text("x"),
        "x\n====",
    );
}

#[test]
fn typing_before_a_thematic_rule_keeps_its_interrupting_boundary() {
    check(
        "Before\n***\nAfter",
        Format::Markdown,
        0,
        InputEvent::text("x"),
        "xBefore\n\nAfter",
    );
}

#[test]
fn empty_paragraph_typing_keeps_separator_patches_and_neighbor_work_bounded() {
    for neighbor_len in [16, 40_000, 100_000] {
        for endings in [1, 2, 3] {
            let neighbor = "a".repeat(neighbor_len);
            let source = format!("{}{neighbor}\n", "\n".repeat(endings));
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let neighbor_id = document.projection().blocks()[1].id;
            let committed = document
                .apply_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(0..0, "x")],
                })
                .unwrap();
            let patches = committed
                .summary()
                .source_patches()
                .iter()
                .map(|patch| (patch.range(), patch.replacement().to_vec()))
                .collect::<Vec<_>>();
            let expected = if endings == 1 {
                vec![(0..0, b"x".to_vec()), (1..1, b"\n".to_vec())]
            } else {
                vec![(0..0, b"x".to_vec())]
            };
            assert_eq!(patches, expected);
            let saved = format!("x{}{neighbor}\n", "\n".repeat(endings.max(2)));
            assert_eq!(document.source_bytes(), saved.as_bytes());
            assert_eq!(document.text(), format!("x\n{neighbor}"));
            assert_eq!(document.projection().blocks()[1].id, neighbor_id);
            assert_reopened(&document);
            let work = committed.summary().projection_work();
            assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
            assert!(work.decoded_source_bytes() <= 4, "{work:?}");
            check(
                &source,
                Format::Markdown,
                0,
                InputEvent::text("x"),
                &format!("x\n{neighbor}"),
            );
        }
    }
}

#[test]
fn typing_into_a_blank_separator_keeps_the_following_paragraph_separate() {
    for (source, at, expected) in [
        ("> ```\n> aaa\n\nbbb\n", 4, "aaa\nx\nbbb"),
        ("\n<foo>\n", 0, "x\n<foo>"),
        ("\n <foo>\n", 0, "x\n<foo>"),
        ("\n   <foo>\n", 0, "x\n<foo>"),
        (">\n> foo\n>  \n", 0, "x\nfoo\n"),
    ] {
        check(
            source,
            Format::Markdown,
            at,
            InputEvent::text("x"),
            expected,
        );
    }
}

#[test]
fn typing_into_an_empty_fenced_code_row_keeps_its_literal_boundary() {
    check(
        "```\n\nb\n```\n",
        Format::Markdown,
        0,
        InputEvent::text("x"),
        "x\nb",
    );
}

#[test]
fn source_backspace_after_a_table_reprojects_the_joined_row() {
    let source = "| a | b |\n| - | - |\n| c | d |\n> q\n";
    check(
        source,
        Format::MarkdownSource,
        source.find("> q").unwrap(),
        InputEvent::Key(Key::Backspace),
        "| a | b |\n| - | - |\n| c | d |> q\n",
    );
}

#[test]
fn table_terminal_source_ending_deletion_matches_reopening() {
    let source = "| a | b |\n| - | - |\n| c | d |\n";
    check(
        source,
        Format::MarkdownSource,
        source.len(),
        InputEvent::Key(Key::Backspace),
        source.trim_end_matches('\n'),
    );
}

#[test]
fn typing_before_an_indented_setext_neighbor_matches_reopening() {
    let source = "# Heading\n    foo\nHeading\n------\n    foo\n----\n";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document
        .projection()
        .hard_line_at_offset(document.text().rfind("foo").unwrap())
        .and_then(|index| document.projection().hard_line_range(index))
        .unwrap()
        .start;
    let mut expected = document.text().to_owned();
    expected.insert_str(at, "x");
    check(
        source,
        Format::Markdown,
        at,
        InputEvent::text("x"),
        &expected,
    );
    for use_request in [false, true] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        if use_request {
            document
                .apply_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..at, "x")],
                })
                .unwrap();
        } else {
            document.insert(at, "x").unwrap();
        }
        assert_eq!(document.text(), expected);
        assert_eq!(
            document.source_bytes(),
            b"# Heading\n    foo\nHeading\n------\n    xfoo\n----\n"
        );
        assert_reopened(&document);
    }
}

#[test]
fn source_typing_a_setext_marker_after_a_quoted_fence_matches_reopening() {
    let source = "> ```\n> aaa\n\nbbb\n";
    let document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let mut expected = document.text().to_owned();
    let at = expected.len();
    expected.push_str("- ");
    check(
        source,
        Format::MarkdownSource,
        at,
        InputEvent::text("- "),
        &expected,
    );
    let mut document = document;
    document.insert(at, "- ").unwrap();
    assert_eq!(document.text(), expected);
    assert_reopened(&document);
    let saved = document.source_bytes();
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
    assert_reopened(&document);
}
