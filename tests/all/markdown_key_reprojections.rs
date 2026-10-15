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
        "\n\n====\n",
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
        // One leading ending is trivia; a pair retains the empty paragraph.
        for endings in [2, 3] {
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
            assert_eq!(patches, vec![(0..0, b"x".to_vec())]);
            let saved = format!("x{}{neighbor}\n", "\n".repeat(endings));
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
        ("\n\n<foo>\n", 0, "x\n<foo>"),
        ("\n\n <foo>\n", 0, "x\n<foo>"),
        ("\n\n   <foo>\n", 0, "x\n<foo>"),
        (">\n>\n> foo\n>  \n", 0, "x\nfoo\n"),
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
fn october11_typing_boundaries_reopen_and_undo_exactly() {
    for (source, at, value, expected) in [
        ("# \\\\a", 0, "q", "q\\a"),
        ("- </ins>", 0, "q", "q"),
        ("<!-- unclosed comment", 21, "\\", "<!-- unclosed comment\\"),
        ("<https://auto.link>", 17, " ", "https://auto.link "),
        ("<mailto:a@b.c>", 12, " ", "mailto:a@b.c "),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = at.min(document.text().len());
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        input(&mut core, view, InputEvent::key('i'));
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: BoundaryAffinity::Upstream, extend_selection: false }).unwrap();
        input(&mut core, view, InputEvent::text(value));
        assert_eq!(core.document().text(), expected, "{source:?}");
        assert_reopened(core.document());
        input(&mut core, view, InputEvent::Key(Key::Escape));
        input(&mut core, view, InputEvent::key('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn october11_list_prefixes_and_authored_rules_reopen() {
    for source in ["0. zero", "1) a\n2) b", "- a\n1. b", "-\ta\n-\tb"] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::text("1"));
        input(&mut core, view, InputEvent::text(". "));
        assert!(core.document().text().starts_with("1. "));
        assert_reopened(core.document());
    }
    for source in ["- a\n- b", "- "] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        if source == "- " {
            input(&mut core, view, InputEvent::key('A'));
            input(&mut core, view, InputEvent::text("---"));
        } else {
            input(&mut core, view, InputEvent::key('r'));
            input(&mut core, view, InputEvent::key('-'));
        }
        assert_reopened(core.document());
    }
}

#[test]
fn october11_typing_after_emptied_setext_successor() {
    let mut document = Document::from_bytes(b"a\n=\n\n".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document.text().len();
    document.insert(at, "x").unwrap();
    assert_eq!(document.source_bytes(), b"a\n=\n\nx");
    assert_reopened(&document);
}

#[test]
fn october11_setext_successor_change_types_and_restores_exact_source() {
    let source = "a\n=\n\nb";
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    for key in "Gccx".chars() { input(&mut core, view, InputEvent::key(key)); }
    assert_eq!(core.document().source_bytes(), b"a\n=\n\nx");
    assert_reopened(core.document());
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn october11_list_substitution_and_empty_item_break_keep_projection() {
    for (source, at) in [
        ("- a\n- b\n- c", 2),
        ("1. a\n2. b\n3. c", 2),
        ("- a\n\n- b", 2),
        ("1) a\n2) b", 2),
        ("- a\n\n  continued\n- b", 2),
    ] {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        for key in ":s/^/#/".chars() { input(&mut core, view, InputEvent::key(key)); }
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_reopened(core.document());
        input(&mut core, view, InputEvent::key('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    let mut core = Core::new(Document::from_bytes(b"- \n- \n- ".to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    input(&mut core, view, InputEvent::key('i'));
    core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
        text_offset: 1, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
    input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
    input(&mut core, view, InputEvent::text("x"));
    assert_eq!(core.document().source_bytes(), b"- \n- <br>x\n- ");
    assert_reopened(core.document());
}

#[test]
fn october11_enter_before_image_has_ordinary_empty_paragraph() {
    let source = "![](local.png)";
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    input(&mut core, view, InputEvent::key('i'));
    input(&mut core, view, InputEvent::Key(Key::Enter));
    let blocks = core.document().projection().blocks();
    assert_eq!(blocks[0].style.0, "Paragraph");
    assert_eq!(blocks[1].style.0, "Image");
    assert_reopened(core.document());
    input(&mut core, view, InputEvent::Key(Key::Escape));
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn october11_nested_list_style_final_projection_stays_local() {
    use viem_core::document::{FormattedPayloadEdit, FormattedTextPayload, StyleProperty, StylePropertyValue};
    let source = format!("{}- a\n  - b\n    - c\n      - d\n{}", "- before\n".repeat(1_000), "- after\n".repeat(1_000));
    for code in [false, true] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find("a\nb\nc\nd").unwrap() + 6;
        let untouched = document.projection().blocks()[1_500].id;
        let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "x", vec![]).unwrap();
        let code_style = "Code".into();
        let values = [(StyleProperty::CharacterBold, StylePropertyValue::Boolean(true))];
        let (prepared, _) = document.prepare_insertion_with_typing_style(
            FormattedPayloadEdit::new(at..at, payload), code.then_some(&code_style),
            if code { &[] } else { &values },
        ).unwrap();
        let work = prepared.summary().projection_work();
        assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
        assert!(work.decoded_source_bytes() < 128, "Code={code}: {work:?}");
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.projection().blocks()[1_500].id, untouched);
        assert_reopened(&document);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn october11_removing_quote_preserves_literal_code_prefixes() {
    for (source, expected) in [
        (">     > literal", "    > literal"),
        ("> ```\n> - > literal\n> ```", "```\n- > literal\n```"),
        ("> >     > literal", ">     > literal"),
        ("- > ```\n  > - > literal\n  > ```", "- ```\n  - > literal\n  ```"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let before_text = document.text().to_owned();
        let prepared = document.prepare_model_request(ModelRequest::SetBlockQuote {
            document: document.id(), revision: document.revision(), range: 0..0, enabled: false,
        }).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes(), "{source:?}");
        assert_eq!(document.text(), before_text);
        assert_reopened(&document);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn enter_replacing_a_folded_break_preserves_unrelated_source_and_bounded_projection() {
    let source = format!("{}a\nb\n\n{}", "before\n\n".repeat(2_000), "after\n\n".repeat(2_000));
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document.text().find("a b").unwrap() + 1;
    let prepared = document.prepare_model_request(ModelRequest::ContinueList {
        document: document.id(), revision: document.revision(), at,
    }).unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 128, "{work:?}");
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), source.replacen("a\nb", "a\n\nb", 1).as_bytes());
    assert_reopened(&document);
    let saved = document.source_bytes();
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
}

#[test]
fn prose_splits_next_to_lists_retain_their_full_source_owner_context() {
    for source in ["- first\n\nTail", "Tail\n\n- next"] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find("Tail").unwrap();
        let prepared = document.prepare_model_request(ModelRequest::ContinueList {
            document: document.id(), revision: document.revision(), at,
        }).unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_reopened(&document);
        let saved = document.source_bytes();
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
        assert_reopened(&document);
    }
}
