use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Encoding, FileFormat, Format, HistoryNavigationRequest, ModelRequest,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document};

#[test]
fn hard_breaks_inside_inline_scopes_and_visible_html_preserve_text_and_history() {
    let mut failures = Vec::new();
    let mut cases = [
        ("a b", &[1, 2][..]),
        ("a\tb", &[1, 2][..]),
        ("**a `b` c**", &[2, 3][..]),
        ("a <!-- inline --> b", &[5, 10, 15][..]),
        ("> <!-- inner -->", &[0, 3, 6, 12, 14][..]),
        ("- <!-- inner -->", &[0, 3, 6, 12, 14][..]),
        ("- a <!-- inner -->", &[10][..]),
        ("> - a <!-- inner -->", &[10][..]),
        ("- > a <!-- inner -->", &[10][..]),
        (
            "<details><summary>s</summary>\n\nbody\n\n</details>",
            &[0][..],
        ),
        ("# Title\n\nPara **bold** and _it_.\n\n- a\n- b\n\n> q\n\n```\nc\n```\n\n| a | b |\n| - | - |\n| 1 | 2 |", &[20][..]),
    ].into_iter().map(|(source, positions)| (source, positions.to_vec())).collect::<Vec<_>>();
    for source in [
        "**bold**",
        "x**bold**y",
        "~~strike~~",
        "**a *b* c**",
        "**bold *nested _deep `code` deep_ nested* bold**",
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        cases.push((source, (0..=document.text().len()).collect()));
    }
    for (source, positions) in cases {
        for at in positions {
            for affinity in [BoundaryAffinity::Downstream, BoundaryAffinity::Upstream] {
                let document = Document::from_bytes(
                    source.as_bytes().to_vec(),
                    Encoding::Utf8,
                    Format::Markdown,
                )
                .unwrap();
                let before = document.text().to_owned();
                let before_lines = document.line_count();
                let before_owners = document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| {
                        (
                            block.kind.clone(),
                            block.quote_depth,
                            block
                                .containers
                                .iter()
                                .map(|member| member.container.kind.clone())
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                let original_styles = before
                    .char_indices()
                    .map(|(offset, _)| {
                        (
                            offset,
                            DocumentLayoutStyles::semantic_character_at(
                                document.projection(),
                                offset,
                                false,
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut expected = before.clone();
                expected.insert(at, '\n');
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
                    .unwrap();
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
                let result = core
                    .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::ShiftEnter)));
                match result {
                    Ok(outcome)
                        if outcome.command.as_ref().map_or(true, |command| {
                            matches!(
                                command.status,
                                CommandStatus::Complete | CommandStatus::Pending
                            )
                        }) => {}
                    result => {
                        failures.push(format!(
                            "{source:?} at {at} {affinity:?}: {result:?}; blocks={:?}",
                            core.document().projection().blocks()
                        ));
                        continue;
                    }
                }
                assert_eq!(
                    core.document().text(),
                    expected,
                    "{source:?} at {at} {affinity:?}"
                );
                assert_eq!(
                    core.document().line_count(),
                    before_lines + 1,
                    "{source:?} at {at} {affinity:?}: inserted break must be a hard line"
                );
                assert_eq!(
                    core.document()
                        .projection()
                        .blocks()
                        .iter()
                        .map(|block| (
                            block.kind.clone(),
                            block.quote_depth,
                            block
                                .containers
                                .iter()
                                .map(|member| member.container.kind.clone())
                                .collect::<Vec<_>>()
                        ))
                        .collect::<Vec<_>>(),
                    before_owners,
                    "{source:?} at {at} {affinity:?}: paragraph owners must survive"
                );
                if source.contains("**") || source.contains("~~") {
                    for (offset, style) in &original_styles {
                        assert_eq!(
                            *style,
                            DocumentLayoutStyles::semantic_character_at(
                                core.document().projection(),
                                offset + usize::from(*offset >= at),
                                false
                            ),
                            "{source:?} at {at} {affinity:?}: retained character {offset}"
                        );
                    }
                }
                let changed = core.document().source_bytes();
                let reopened =
                    Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Markdown)
                        .unwrap();
                assert_eq!(reopened.text(), expected);
                assert_eq!(
                    core.document()
                        .projection()
                        .blocks()
                        .iter()
                        .map(|block| (&block.range, &block.attributes))
                        .collect::<Vec<_>>(),
                    reopened
                        .projection()
                        .blocks()
                        .iter()
                        .map(|block| (&block.range, &block.attributes))
                        .collect::<Vec<_>>()
                );
                core.handle(
                    view,
                    CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
                )
                .unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                assert_eq!(core.document().text(), before);
                core.handle(
                    view,
                    CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
                )
                .unwrap();
                assert_eq!(core.document().source_bytes(), changed);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn typing_after_an_emphasis_boundary_break_shares_exact_undo() {
    for (source, at) in [
        ("**a *b* c**", 0),
        ("**bold *nested _deep `code` deep_ nested* bold**", 0),
        ("~~strike~~", 0),
        ("# Title\n\nPara **bold** and _it_.\n\n- a\n- b\n\n> q\n\n```\nc\n```\n\n| a | b |\n| - | - |\n| 1 | 2 |", 20),
    ] {
        for affinity in [BoundaryAffinity::Downstream, BoundaryAffinity::Upstream] {
            let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let mut expected = document.text().to_owned();
            expected.insert_str(at, "\nx");
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at, affinity, extend_selection: false,
            }).unwrap();
            for key in [Key::ShiftEnter, Key::Char('x')] {
                let result = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                    .unwrap_or_else(|error| panic!("{source:?} at {at} {affinity:?}: {error:?}"));
                if let Some(command) = result.command {
                    assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending));
                }
            }
            assert_eq!(core.document().text(), expected);
            let changed = core.document().source_bytes();
            let reopened = Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), changed);
        }
    }
}

#[test]
fn comment_body_break_preserves_encoded_source_and_owner_prefixes() {
    let encode = |source: &str, encoding| {
        let mut bytes = match encoding {
            Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => vec![0xff, 0xfe],
            Encoding::Utf16Be => vec![0xfe, 0xff],
            _ => unreachable!(),
        };
        bytes.extend(match encoding {
            Encoding::Utf8 => source.as_bytes().to_vec(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        });
        bytes
    };
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (prefix, continuation) in [("> ", "> "), ("- ", "  ")] {
            let source = format!("{prefix}<!-- αβ -->\r\n\r\nTail\r\n");
            let bytes = encode(&source, encoding);
            let mut document =
                Document::from_bytes(bytes.clone(), encoding, Format::Markdown).unwrap();
            assert_eq!(document.file_format(), FileFormat::Dos);
            let before = document.text().to_owned();
            let before_lines = document.line_count();
            let before_owners = document
                .projection()
                .blocks()
                .iter()
                .map(|block| {
                    (
                        block.kind.clone(),
                        block.quote_depth,
                        block
                            .containers
                            .iter()
                            .map(|member| member.container.kind.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let at = "<!-- α".len();
            let mut expected = before.clone();
            expected.insert(at, '\n');
            let prepared = document
                .prepare_model_request(ModelRequest::InsertHardBreak {
                    document: document.id(),
                    revision: document.revision(),
                    at,
                    affinity: BoundaryAffinity::Downstream,
                })
                .unwrap();
            assert_eq!(prepared.summary().source_patches().len(), 1);
            assert!(prepared.summary().source_patches()[0].range().is_empty());
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), expected);
            assert_eq!(document.line_count(), before_lines + 1);
            assert_eq!(
                document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (
                        block.kind.clone(),
                        block.quote_depth,
                        block
                            .containers
                            .iter()
                            .map(|member| member.container.kind.clone())
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
                before_owners
            );
            let changed = encode(
                &format!("{prefix}<!-- α\r\n{continuation}β -->\r\n\r\nTail\r\n"),
                encoding,
            );
            assert_eq!(document.source_bytes(), changed);
            let reopened =
                Document::from_bytes(changed.clone(), encoding, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            assert_eq!(reopened.line_count(), document.line_count());
            assert_eq!(
                reopened
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (&block.range, &block.attributes))
                    .collect::<Vec<_>>(),
                document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (&block.range, &block.attributes))
                    .collect::<Vec<_>>()
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
            assert_eq!(document.text(), before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}

#[test]
fn comment_continuations_preserve_whitespace_beyond_list_indentation() {
    for source in [
        "- <!-- a\n    b -->",
        "> - <!-- a\n>     b -->",
        "- <!-- a\n\tb -->",
        "> - <!-- a\n> \tb -->",
    ] {
        for at in [7, 8, 9] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(document.text(), "<!-- a\n  b -->", "{source:?}");
            assert_eq!(
                document.line_count(),
                2,
                "a partial tab is not a line ending"
            );
            assert_eq!(document.source_bytes(), source.as_bytes());
            let owners = document
                .projection()
                .blocks()
                .iter()
                .map(|block| {
                    (
                        block.kind.clone(),
                        block.quote_depth,
                        block
                            .containers
                            .iter()
                            .map(|member| member.container.kind.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let prepared = document
                .prepare_model_request(ModelRequest::InsertHardBreak {
                    document: document.id(),
                    revision: document.revision(),
                    at,
                    affinity: BoundaryAffinity::Downstream,
                })
                .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
            assert_eq!(prepared.summary().source_patches().len(), 1);
            let patch = &prepared.summary().source_patches()[0];
            if let Some(tab) = source.find('\t').filter(|_| at < 9) {
                assert_eq!(
                    patch.range(),
                    tab..tab + 1,
                    "only the shared tab is rewritten"
                );
            } else {
                assert!(patch.range().is_empty());
            }
            document.commit_model_transaction(prepared).unwrap();
            let mut expected = "<!-- a\n  b -->".to_owned();
            expected.insert(at, '\n');
            assert_eq!(document.text(), expected, "{source:?} at {at}");
            assert_eq!(document.line_count(), 3);
            assert_eq!(
                document
                    .projection()
                    .blocks()
                    .iter()
                    .map(|block| (
                        block.kind.clone(),
                        block.quote_depth,
                        block
                            .containers
                            .iter()
                            .map(|member| member.container.kind.clone())
                            .collect::<Vec<_>>(),
                    ))
                    .collect::<Vec<_>>(),
                owners
            );
            let changed = document.source_bytes();
            let reopened =
                Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), document.text());
            assert_eq!(reopened.line_count(), 3);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}
