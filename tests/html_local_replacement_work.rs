//! A short rich edit must not reparse a giant paragraph's untouched prefix.
use viem_core::document::{
    measure_document_work, Document, Encoding, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};

fn encode(source: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        _ => unreachable!(),
    }
}

fn check_local_edits(source: String, encoding: Encoding) {
    let bytes = encode(&source, encoding);
    let mut document = Document::from_bytes(bytes.clone(), encoding, Format::Html).unwrap();
    document.set_history_retention_policy(viem_core::document::HistoryRetentionPolicy::unlimited());
    let initial_style_count = document.projection().style_spans().len();
    let paragraph_ids = document
        .projection()
        .blocks()
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    for location in 0..3 {
        let length = document.projection().blocks()[0].range.end;
        let position = match location {
            0 => 0,
            1 => length / 2,
            _ => length.saturating_sub(8),
        };
        let at = (position..length)
            .find(|at| {
                document.text_point(*at).is_ok()
                    && document
                        .projection()
                        .text_tree()
                        .slice(*at..*at + 1)
                        .as_deref()
                        != Ok("\n")
            })
            .unwrap();
        let end = document
            .hard_line_snapshot()
            .next_grapheme_boundary(at)
            .unwrap();
        let (prepared, measured) = measure_document_work(|| {
            document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(at..end, "XYZ")],
                })
                .unwrap()
        });
        assert_eq!(
            measured.source_full_materializations, 0,
            "{encoding:?} at={at}: {measured:?}"
        );
        assert!(
            measured.source_decoded_bytes < 8192,
            "{encoding:?} at={at}: {measured:?}"
        );
        assert!(
            measured.html_tokenized_bytes < 8192,
            "{encoding:?} at={at}: {measured:?}"
        );
        let work = prepared.summary().projection_work();
        assert_eq!(
            work.scope(),
            ProjectionWorkScope::RegionalHardLines,
            "{encoding:?} at={at}: {work:?}"
        );
        assert!(
            work.decoded_source_bytes() < 4096,
            "{encoding:?} at={at}: {work:?}"
        );
        assert!(
            work.projected_formatted_bytes() < 1024,
            "{encoding:?} at={at}: {work:?}"
        );
        assert!(
            work.persistent_records_copied() < 4096,
            "{encoding:?} at={at}: {work:?}"
        );
        assert_eq!(work.full_text_bytes_materialized(), 0);
        document.commit_model_transaction(prepared).unwrap();
    }
    let length = document.projection().blocks()[0].range.end;
    let at = (length / 2..length)
        .find(|at| document.text_point(*at).is_ok())
        .unwrap();
    for count in 0..32 {
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at + count..at + count, "x")],
            })
            .unwrap();
        assert!(
            prepared
                .summary()
                .projection_work()
                .projected_formatted_bytes()
                < 1024
        );
        document.commit_model_transaction(prepared).unwrap();
    }
    assert_eq!(
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| block.id)
            .collect::<Vec<_>>(),
        paragraph_ids
    );
    assert!(
        document.projection().style_spans().len() <= initial_style_count + 8,
        "continued typing fragmented persistent style runs"
    );
    let fresh = Document::from_bytes(document.source_bytes(), encoding, Format::Html).unwrap();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
    while document.undo() {}
    assert_eq!(document.source_bytes(), bytes);
}

#[test]
fn large_paragraph_edits_are_bounded_across_plain_and_long_inherited_runs() {
    for length in [4096, 65_536, 2_000_000] {
        check_local_edits(
            format!(
                "<p><b><i>{}</i></b></p><p>tail</p>",
                "abcde".repeat(length / 5)
            ),
            Encoding::Utf8,
        );
    }
}

#[test]
fn bounded_windows_preserve_nested_runs_entities_and_utf16() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        check_local_edits(
            format!(
                "<p><b>{}</b></p>",
                "abc<i>def</i>ghi &amp; jkl ".repeat(256)
            ),
            encoding,
        );
        check_local_edits(format!("<p>{}</p>", "aé😺e\u{301}".repeat(512)), encoding);
    }
}

#[test]
fn long_multiline_paragraph_keeps_unaffected_hard_lines_and_style_runs() {
    check_local_edits(
        format!("<p><b>{}last line</b></p>", "word<br>".repeat(4096)),
        Encoding::Utf8,
    );
}

#[test]
fn long_preformatted_line_uses_the_same_bounded_window() {
    check_local_edits(
        format!("<pre>{}</pre>", "word  ".repeat(16_384)),
        Encoding::Utf8,
    );
}

#[test]
fn ordinary_typing_does_not_redecode_large_unchanged_ancestor_attributes() {
    check_local_edits(
        format!(
            "<p><span title='{}'><b>{}</b></span></p>",
            "x".repeat(1_000_000),
            "word".repeat(4096)
        ),
        Encoding::Utf8,
    );
}

#[test]
fn canonical_edits_retain_html5_recovered_character_context() {
    use viem_core::layout::DocumentLayoutStyles;
    for prefix in [String::new(), "x".repeat(2048)] {
        let source = format!("<p>{prefix}<b><i>A</b>B</i>C</p>");
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        let at = prefix.len() + 1;
        let original =
            DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at + 1, "X")],
            })
            .unwrap();
        assert_eq!(
            prepared.summary().projection_work().scope(),
            ProjectionWorkScope::RegionalHardLines
        );
        document.commit_model_transaction(prepared).unwrap();
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap(),
            original
        );
        assert_eq!(
            document.projection().style_spans(),
            reopened.projection().style_spans()
        );
    }
}

#[test]
fn context_replay_keeps_persisted_character_definitions_and_source_line_boundaries() {
    use viem_core::command::InputEvent;
    use viem_core::document::*;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for multiline in [false, true] {
        let source = if multiline {
            "<p><a\n href='/x'>ab</a>&amp;</p><p>tail</p>"
        } else {
            "<p><a href='/x'>ab</a>&amp;</p><p>tail</p>"
        };
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let id: StyleId = "Accent".into();
        let apply = |document: &mut Document, intent| {
            document
                .apply_style_request(StyleModelRequest::new(
                    document.id(),
                    document.revision(),
                    StyleModelIntent::Persisted(intent),
                ))
                .unwrap();
        };
        apply(
            &mut document,
            PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: id.clone(),
                        based_on: None,
                        properties: CharacterProperties {
                            size: Some(27.),
                            ..Default::default()
                        },
                    },
                    metadata: StyleDefinitionMetadata {
                        display_name: "Accent".into(),
                        origin: StyleDefinitionOrigin::SourceBacked,
                    },
                },
            },
        );
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(3).unwrap(),
        )
        .unwrap();
        apply(
            &mut document,
            PersistedStyleIntent::AssignCharacterStyle { range, style: id },
        );
        let before = document.source_bytes();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        for (at, extend) in [(2, false), (3, true)] {
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: extend,
                },
            )
            .unwrap();
        }
        let (_, measured) = measure_document_work(|| {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("XY")))
                .unwrap();
        });
        if !multiline {
            assert_eq!(measured.source_full_materializations, 0, "{measured:?}");
        }
        assert_eq!(core.document().text(), "abXY\ntail");
        let reopened =
            Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html)
                .unwrap();
        for at in 0..core.document().text().len() {
            if core.document().text().as_bytes()[at] != b'\n' {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(
                        core.document().projection(),
                        at,
                        false
                    ),
                    DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false)
                );
            }
        }
        assert_eq!(
            core.document()
                .projection()
                .provenance()
                .iter()
                .filter(|span| !span.formatted.is_empty())
                .collect::<Vec<_>>(),
            reopened
                .projection()
                .provenance()
                .iter()
                .filter(|span| !span.formatted.is_empty())
                .collect::<Vec<_>>()
        );
        for at in [2, 3] {
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false
                )
                .unwrap()
                .size,
                27.
            );
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(at).unwrap())
                    .unwrap(),
                None
            );
        }
        core.handle_with_layout(
            view,
            CoreEvent::Input(InputEvent::Key(viem_core::command::Key::Escape)),
        )
        .unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), before);
    }
}

#[test]
fn supporting_whitespace_batch_keeps_separate_patches_and_exact_reopen() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = "<p>alpha <b>bold</b> omega</p>\n<p>tail</p>";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(6..10, "")],
        })
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 2);
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "alpha \u{a0}omega\ntail");
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        document.projection().provenance(),
        reopened.projection().provenance()
    );
    for at in document.text().char_indices().map(|(at, _)| at) {
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), at, false),
            DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false)
        );
    }
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn contextual_windows_decline_unrecoverable_lexical_formatting_ancestry() {
    use viem_core::command::InputEvent;
    use viem_core::document::{BoundaryAffinity, DocumentWorkFallback};
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    let source = format!(
        "<p><b><i>A</b>{}<a href='/x'>C</a>&amp;</i>D</p>",
        "B".repeat(2048)
    );
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
    let at = 2050;
    let expected =
        DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    for (offset, extend_selection) in [(at, false), (at + 1, true)] {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection,
            },
        )
        .unwrap();
    }
    let (_, work) = measure_document_work(|| {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
    });
    assert!(
        work.fallback_counts[DocumentWorkFallback::HtmlRecoveryContext as usize] > 0,
        "{work:?}"
    );
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false)
            .unwrap(),
        expected
    );
    let reopened =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false).unwrap(),
        expected
    );
    assert_eq!(core.document().text(), reopened.text());
}

#[test]
fn emptied_leading_inline_scope_keeps_its_exact_typing_anchor() {
    use viem_core::layout::DocumentLayoutStyles;
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for trivia in ["", "<!--preserve-->"] {
            let source =
                format!("<p><i><b foo='bar'>{trivia}Bold words</b></i> and <u>tail</u>.</p>");
            let bytes = encode(&source, encoding);
            let mut document = Document::from_bytes(bytes.clone(), encoding, Format::Html).unwrap();
            document.replace(0..10, "").unwrap();
            let reopened =
                Document::from_bytes(document.source_bytes(), encoding, Format::Html).unwrap();
            assert_eq!(
                document.projection().provenance(),
                reopened.projection().provenance(),
                "{encoding:?} {trivia}"
            );
            document.insert(0, "New words").unwrap();
            assert_eq!(document.text(), "New words and tail.");
            let reopened =
                Document::from_bytes(document.source_bytes(), encoding, Format::Html).unwrap();
            for at in [0, 8, 10, 14] {
                assert_eq!(
                    DocumentLayoutStyles::semantic_character_at(document.projection(), at, false),
                    DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false)
                );
            }
            assert!(
                DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
                    .unwrap()
                    .bold
            );
            assert!(document.undo());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn filling_empty_inline_owners_replaces_only_real_caret_annotations() {
    use viem_core::layout::DocumentLayoutStyles;
    for body in ["", "<b></b>", "<b></b><i></i>", "<b><i></i><u></u></b>"] {
        let source = format!("<p>{body}</p><p>tail</p>");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        document.insert(0, "X").unwrap();
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(
            document.projection().provenance(),
            reopened.projection().provenance(),
            "{body}"
        );
        let mut reopened = reopened;
        document.insert(1, "Y").unwrap();
        reopened.insert(1, "Y").unwrap();
        assert_eq!(document.source_bytes(), reopened.source_bytes(), "{body}");
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), 1, false),
            DocumentLayoutStyles::semantic_character_at(reopened.projection(), 1, false)
        );
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
