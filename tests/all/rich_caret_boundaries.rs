use viem_core::command::{InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn document(format: Format, source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn every_visible_rich_caret_boundary_accepts_typing_with_either_affinity() {
    let fixtures = [

        (Format::Markdown, "```\n\n```"),
        (Format::Markdown, "> ```\n> \n> ```"),
        (Format::Markdown, "- ```\n  A\n  ```"),
    ];
    let mut failures = Vec::new();
    for (format, source) in fixtures {
        let original = document(format, source);
        for at in (0..=original.text().len()).filter(|at| original.text_point(*at).is_ok()) {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                for input in ["X", " ", "é"] {
                    let mut core = Core::new(document(format, source));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
                    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                        .unwrap();
                    let placement = core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset: at,
                            affinity,
                            extend_selection: false,
                        },
                    );
                    // Only the shaping sides that actually exist are visual
                    // caret locations (e.g. the first glyph has no upstream side).
                    if matches!(
                        placement,
                        Err(viem_core::CoreError::Layout(
                            viem_core::layout::LayoutError::NotACaretStop { .. }
                        ))
                    ) {
                        continue;
                    }
                    placement.unwrap();
                    if let Err(error) = core.handle(view, CoreEvent::Input(InputEvent::text(input)))
                    {
                        failures.push(format!(
                            "{format:?} {source:?} at {at} {affinity:?} {input:?}: {error:?}"
                        ));
                        continue;
                    }
                    let mut expected = original.text().to_owned();
                    expected.insert_str(at, input);
                    let actual = core.document().text();
                    // HTML's authored edge spaces intentionally use NBSP.
                    assert_eq!(
                        actual.replace('\u{a0}', " "),
                        expected,
                        "{format:?} {source:?} at {at} {affinity:?} {input:?}"
                    );
                    assert_eq!(
                        core.command_state(view).unwrap().cursor(),
                        at + actual.len() - original.text().len()
                    );
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                        .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn upstream_markdown_list_code_start_inserts_inside_the_fence() {
    use viem_core::document::{
        FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };

    let source = "- ```\n  A\n  ```";
    for (input, encoded) in [("X", "X"), (" ", " "), ("é", "é")] {
        let mut doc = document(Format::Markdown, source);
        let before = doc.text().to_owned();
        let payload = FormattedTextPayload::new(&doc.hard_line_snapshot(), input, vec![]).unwrap();
        let prepared = doc
            .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                doc.id(),
                doc.revision(),
                vec![FormattedPayloadEdit::new(0..0, payload)
                    .with_boundary_affinity(BoundaryAffinity::Upstream)],
            ))
            .unwrap();
        let patches = prepared.summary().source_patches();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].range(), 8..8);
        assert_eq!(patches[0].replacement(), encoded.as_bytes());
        doc.commit_model_transaction(prepared).unwrap();
        assert_eq!(doc.text(), format!("{input}{before}"));
        assert_eq!(
            doc.source_bytes(),
            format!("- ```\n  {encoded}A\n  ```").as_bytes()
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.text(), format!("{input}{before}"));
    }
}

#[test]
fn markdown_code_body_edge_payloads_preserve_their_hidden_container_syntax() {
    use viem_core::document::{
        FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    };

    for source in [
        "```\nA\n```",
        "- item\n\n  ```\n  A\n  ```",
        "- item\n\n  ```\n  A\n  B\n  ```",
        "> ```\n> A\n> ```",
    ] {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            for input in ["X", " ", "é"] {
                for after in [false, true] {
                    let mut doc = document(Format::Markdown, source);
                    let before = doc.text().to_owned();
                    let at = before.find('A').unwrap() + usize::from(after);
                    let source_at = source.find('A').unwrap() + usize::from(after);
                    let mut expected_source = source.to_owned();
                    expected_source.insert_str(source_at, input);
                    let mut expected_text = before.clone();
                    expected_text.insert_str(at, input);
                    let payload =
                        FormattedTextPayload::new(&doc.hard_line_snapshot(), input, vec![])
                            .unwrap();
                    let prepared = doc
                        .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                            doc.id(),
                            doc.revision(),
                            vec![FormattedPayloadEdit::new(at..at, payload)
                                .with_boundary_affinity(affinity)],
                        ))
                        .unwrap_or_else(|error| {
                            panic!("{source:?} at {at} {affinity:?} {input:?}: {error:?}")
                        });
                    let patches = prepared.summary().source_patches();
                    assert_eq!(patches.len(), 1);
                    assert_eq!(patches[0].range(), source_at..source_at);
                    assert_eq!(
                        patches[0].replacement(),
                        input.as_bytes(),
                        "{source:?} at {at} {affinity:?} {input:?}"
                    );
                    doc.commit_model_transaction(prepared).unwrap();
                    assert_eq!(doc.text(), expected_text);
                    assert_eq!(doc.source_bytes(), expected_source.as_bytes());
                    assert!(doc.undo());
                    assert_eq!(doc.text(), before);
                    assert_eq!(doc.source_bytes(), source.as_bytes());
                    assert!(doc.redo());
                    assert_eq!(doc.text(), expected_text);
                    assert_eq!(doc.source_bytes(), expected_source.as_bytes());
                }
            }
        }
    }
}

#[test]
fn empty_code_bodies_preserve_existing_line_endings_and_literal_delimiters() {
    for (source, expected) in [
        ("```\n\n```", "```\nX\n```"),
        ("```\n```", "```\nX\n```"),
        ("```", "```\nX"),
        ("> ```\n> \n> ```", "> ```\n> X\n> ```"),
        ("> ```\n> ```", "> ```\n> X\n> ```"),
        ("```\r\n\r\n```", "```\r\nX\r\n```"),
    ] {
        let mut doc = document(Format::Markdown, source);
        doc.replace(0..0, "X").unwrap();
        assert_eq!(doc.text(), "X");
        assert_eq!(doc.source_bytes(), expected.as_bytes());
    }
    for source in ["```\n\n```", "> ```\n> \n> ```"] {
        let mut doc = document(Format::Markdown, source);
        doc.replace(0..0, "```").unwrap();
        assert_eq!(doc.text(), "```");
        assert!(String::from_utf8(doc.source_bytes()).unwrap().starts_with(
            if source.starts_with('>') {
                "> ````"
            } else {
                "````"
            }
        ));
    }
}

#[test]
fn empty_insertions_leave_unfinished_source_constructs_byte_exact() {
    for (format, source) in [

        (Format::Markdown, "```"),
        (Format::Markdown, "```\n```"),
    ] {
        let mut doc = document(format, source);
        let end = doc.text().len();
        doc.replace(end..end, "").unwrap();
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}
