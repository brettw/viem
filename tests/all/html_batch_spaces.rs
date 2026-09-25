use viem_core::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, Encoding, Format,
    FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload, ModelRequest,
    TextEdit,
};

fn check(
    source: &str,
    edits: Vec<TextEdit>,
    expected_text: &str,
    expected_source: &str,
    payloads: bool,
) {
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let original_end = document.text_point(document.text().len()).unwrap();
    let prepared = if payloads {
        let snapshot = document.hard_line_snapshot();
        let edits = edits
            .into_iter()
            .map(|edit| {
                let breaks = edit
                    .replacement
                    .match_indices('\n')
                    .map(|(at, _)| at)
                    .collect();
                let payload =
                    FormattedTextPayload::new(&snapshot, edit.replacement, breaks).unwrap();
                FormattedPayloadEdit::new(edit.range, payload)
            })
            .collect();
        document
            .prepare_formatted_payload_request(FormattedPayloadEditRequest::new(
                document.id(),
                document.revision(),
                edits,
            ))
            .unwrap()
    } else {
        document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits,
            })
            .unwrap()
    };
    let end = prepared
        .text_position_map()
        .map_text_point(
            original_end,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )
        .unwrap()
        .value()
        .unwrap()
        .offset();
    assert_eq!(end, expected_text.len());
    let mut old_at = 0;
    let mut new_at = 0;
    for patch in prepared.summary().source_patches() {
        let range = patch.range();
        let unchanged = range.start - old_at;
        assert_eq!(
            &source.as_bytes()[old_at..range.start],
            &expected_source.as_bytes()[new_at..new_at + unchanged]
        );
        new_at += unchanged;
        assert_eq!(
            patch.replacement(),
            &expected_source.as_bytes()[new_at..new_at + patch.replacement().len()]
        );
        new_at += patch.replacement().len();
        old_at = range.end;
    }
    assert_eq!(
        &source.as_bytes()[old_at..],
        &expected_source.as_bytes()[new_at..]
    );
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), expected_text);
    assert_eq!(document.source_bytes(), expected_source.as_bytes());
    assert!(!expected_source.contains("pre-wrap"));
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), expected_text);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected_source.as_bytes());
    assert_eq!(document.text(), expected_text);
}

#[test]
fn adjacent_batch_edits_protect_surviving_and_inserted_spaces() {
    for payloads in [false, true] {
        for (body, edits, text, result) in [
            (
                "ABC",
                vec![
                    TextEdit::new(0..1, ""),
                    TextEdit::new(1..2, " "),
                    TextEdit::new(2..3, ""),
                ],
                "\u{a0}",
                "&nbsp;",
            ),
            (
                "AB C",
                vec![TextEdit::new(0..1, ""), TextEdit::new(1..2, "")],
                "\u{a0}C",
                "&nbsp;C",
            ),
            (
                "A BC",
                vec![TextEdit::new(2..3, ""), TextEdit::new(3..4, "")],
                "A\u{a0}",
                "A&nbsp;",
            ),
            (
                "A B C",
                vec![TextEdit::new(0..1, ""), TextEdit::new(2..3, "")],
                "\u{a0} C",
                "&nbsp; C",
            ),
            (
                "ABC",
                vec![
                    TextEdit::new(0..1, ""),
                    TextEdit::new(1..2, " "),
                    TextEdit::new(2..3, "z"),
                ],
                "\u{a0}z",
                "&nbsp;z",
            ),
            (
                "ABC",
                vec![
                    TextEdit::new(0..1, "z"),
                    TextEdit::new(1..2, " "),
                    TextEdit::new(2..3, ""),
                ],
                "z\u{a0}",
                "z&nbsp;",
            ),
            (
                "AB CD",
                vec![
                    TextEdit::new(0..1, ""),
                    TextEdit::new(1..2, "\n"),
                    TextEdit::new(3..4, ""),
                ],
                "\n\u{a0}D",
                "<br>&nbsp;D",
            ),
            (
                "ABCD",
                vec![
                    TextEdit::new(0..1, ""),
                    TextEdit::new(1..3, "  "),
                    TextEdit::new(3..4, ""),
                ],
                "\u{a0}\u{a0}",
                "&nbsp;&nbsp;",
            ),
            (
                "ABC",
                vec![
                    TextEdit::new(0..1, ""),
                    TextEdit::new(1..2, " X\nZ"),
                    TextEdit::new(2..3, ""),
                ],
                "\u{a0}X\nZ",
                "&nbsp;X<br>Z",
            ),
        ] {
            for reverse in [false, true] {
                let mut edits = edits.clone();
                if reverse {
                    edits.reverse();
                }
                check(
                    &format!("<!--keep--><p>{body}</p>"),
                    edits,
                    text,
                    &format!("<!--keep--><p>{result}</p>"),
                    payloads,
                );
            }
        }
    }
}

#[test]
fn batch_edits_keep_whitespace_preserving_source_context() {
    for payloads in [false, true] {
        check(
            "<pre>ABC</pre><!--keep-->",
            vec![
                TextEdit::new(0..1, ""),
                TextEdit::new(1..2, " "),
                TextEdit::new(2..3, ""),
            ],
            " ",
            "<pre> </pre><!--keep-->",
            payloads,
        );
    }
}

#[test]
fn protecting_surviving_space_keeps_its_combining_cluster_intact() {
    for payloads in [false, true] {
        check(
            "<p>AB \u{301}C</p>",
            vec![TextEdit::new(0..1, ""), TextEdit::new(1..2, "")],
            "\u{a0}\u{301}C",
            "<p>&nbsp;\u{301}C</p>",
            payloads,
        );
    }
}

#[test]
fn preserved_newline_still_exposes_a_block_edge_for_normal_spaces() {
    for payloads in [false, true] {
        check(
            "<p><span style='white-space:pre'>A\n</span>B C</p>",
            vec![TextEdit::new(2..3, "")],
            "A\n\u{a0}C",
            "<p><span style='white-space:pre'>A\n</span>&nbsp;C</p>",
            payloads,
        );
    }
}
