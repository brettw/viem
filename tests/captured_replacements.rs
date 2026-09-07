use evim_core::document::{
    Document, Encoding, FontSlant, Format, FormattedTextPayload, FragmentEdit, ModelRequest,
    ReplacementFragment,
};
use evim_core::layout::DocumentLayoutStyles;
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
#[test]
fn captured_runs_reorder_with_style_and_one_atomic_source_transaction() {
    for (format, source) in [
        (Format::Html, "<p><b>one</b> <i>two</i></p><!--keep-->"),
        (Format::Rtf, r"{\rtf1 {\b one} {\i two}{\*\unknown keep}}"),
        (Format::Markdown, "**one** *two*"),
    ] {
        let mut document = open(source, format);
        let space = FormattedTextPayload::new(&document.hard_line_snapshot(), " ", vec![]).unwrap();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyFragmentEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![FragmentEdit {
                    range: 0..7,
                    fragments: vec![
                        ReplacementFragment::Capture(4..7),
                        ReplacementFragment::Literal(space),
                        ReplacementFragment::Capture(0..3),
                    ],
                }],
            })
            .unwrap_or_else(|e| panic!("{format:?} {e:?}"));
        assert_eq!(document.source_bytes(), source.as_bytes());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.text(), "two one");
        let first = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
        let last = DocumentLayoutStyles::character_at(document.projection(), 4, false).unwrap();
        assert_eq!(first.slant, FontSlant::Italic, "{format:?}");
        assert!(!first.bold, "{format:?}");
        assert!(last.bold, "{format:?}");
        assert_eq!(last.slant, FontSlant::Upright, "{format:?}");
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            DocumentLayoutStyles::character_at(fresh.projection(), 0, false).unwrap(),
            first
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn captures_can_read_scalar_boundaries_inside_graphemes() {
    let mut document = Document::new("a\u{301}z");
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![FragmentEdit {
                range: 0..3,
                fragments: vec![ReplacementFragment::Capture(0..1)],
            }],
        })
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "az");
    assert!(document.undo());
    assert_eq!(document.text(), "a\u{301}z");
}
#[test]
fn captures_keep_literal_and_semantic_newlines_distinct() {
    use evim_core::document::FileFormat;
    let mut document = Document::from_bytes_with_file_format(
        b"a\nb\rc".to_vec(),
        Encoding::Utf8,
        Format::PlainText,
        FileFormat::Mac,
    )
    .unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![FragmentEdit {
                range: 0..5,
                fragments: vec![
                    ReplacementFragment::Capture(2..5),
                    ReplacementFragment::Capture(0..2),
                ],
            }],
        })
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), b"b\rca\n");
}
#[test]
fn unchanged_capture_does_not_author_resolved_defaults_or_change_source() {
    for (source, format) in [
        ("<p><b>word</b></p><!--keep-->", Format::Html),
        ("**word**", Format::Markdown),
        (r"{\rtf1 {\b word}}", Format::Rtf),
    ] {
        let mut document = open(source, format);
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyFragmentEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![FragmentEdit {
                    range: 0..4,
                    fragments: vec![ReplacementFragment::Capture(0..4)],
                }],
            })
            .unwrap();
        assert!(prepared.summary().source_patches().is_empty(), "{format:?}");
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn captures_restore_default_color_when_the_destination_inherits_a_source_override() {
    let mut document = open(
        "<p><span style='color:red'>red</span> plain</p><!--keep-->",
        Format::Html,
    );
    let space = FormattedTextPayload::new(&document.hard_line_snapshot(), " ", vec![]).unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![FragmentEdit {
                range: 0..9,
                fragments: vec![
                    ReplacementFragment::Capture(4..9),
                    ReplacementFragment::Literal(space),
                    ReplacementFragment::Capture(0..3),
                ],
            }],
        })
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "plain red");
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap()
            .foreground_is_default
    );
    assert!(
        !DocumentLayoutStyles::character_at(document.projection(), 6, false)
            .unwrap()
            .foreground_is_default
    );
}
#[test]
fn captured_plain_text_can_clear_only_part_of_a_surviving_code_span() {
    let mut document = open("`abcdef` plain", Format::Markdown);
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![FragmentEdit {
                range: 2..4,
                fragments: vec![ReplacementFragment::Capture(7..9)],
            }],
        })
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "abplef plain");
    let spans = document.projection().style_spans();
    assert!(spans.iter().any(|span| span.range == (0..2)
        && span.application == evim_core::document::StyleApplication::Named("Code".into())));
    assert!(spans.iter().any(|span| span.range == (4..6)
        && span.application == evim_core::document::StyleApplication::Named("Code".into())));
    assert!(!spans.iter().any(|span| span.range.contains(&2)));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"`abcdef` plain");
}

#[test]
fn html_capture_styles_follow_normalized_spaces_and_prior_batch_edits() {
    let original = "<p><b>A</b> x <i>B</i></p><!--keep-->";
    let mut document = open(original, Format::Html);
    let space = FormattedTextPayload::new(&document.hard_line_snapshot(), " ", vec![]).unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![
                FragmentEdit {
                    range: 0..1,
                    fragments: vec![
                        ReplacementFragment::Literal(space.clone()),
                        ReplacementFragment::Capture(4..5),
                    ],
                },
                FragmentEdit {
                    range: 4..5,
                    fragments: vec![
                        ReplacementFragment::Capture(0..1),
                        ReplacementFragment::Literal(space),
                    ],
                },
            ],
        })
        .unwrap();
    assert_eq!(document.source_bytes(), original.as_bytes());
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "\u{a0}B x A\u{a0}");
    let italic = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    let bold = DocumentLayoutStyles::character_at(document.projection(), 6, false).unwrap();
    assert_eq!(italic.slant, FontSlant::Italic);
    assert!(!italic.bold);
    assert!(bold.bold);
    assert_eq!(bold.slant, FontSlant::Upright);
    let changed = document.source_bytes();
    let source = String::from_utf8(changed.clone()).unwrap();
    assert_eq!(source.matches("&nbsp;").count(), 2);
    assert!(!source.contains("white-space"));
    assert!(source.ends_with("<!--keep-->"));
    let reopened = open(&source, Format::Html);
    assert_eq!(reopened.text(), document.text());
    assert_eq!(
        DocumentLayoutStyles::character_at(reopened.projection(), 2, false).unwrap(),
        italic
    );
    assert_eq!(
        DocumentLayoutStyles::character_at(reopened.projection(), 6, false).unwrap(),
        bold
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), changed);
}

#[test]
fn html_capture_offsets_follow_simplification_of_a_previous_protected_space() {
    let mut document = open("<p><b>A</b> B</p><!--keep-->", Format::Html);
    document.insert(3, " ").unwrap();
    assert_eq!(document.text(), "A B\u{a0}");
    let original = document.source_bytes();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyFragmentEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![FragmentEdit {
                range: 5..5,
                fragments: vec![ReplacementFragment::Capture(0..1)],
            }],
        })
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "A B A");
    assert!(
        DocumentLayoutStyles::character_at(document.projection(), 4, false)
            .unwrap()
            .bold
    );
    assert!(
        !DocumentLayoutStyles::character_at(document.projection(), 2, false)
            .unwrap()
            .bold
    );
    let changed = document.source_bytes();
    let source = String::from_utf8(changed.clone()).unwrap();
    assert!(!source.contains("&nbsp;"));
    assert!(!source.contains("white-space"));
    let reopened = open(&source, Format::Html);
    assert_eq!(reopened.text(), document.text());
    assert!(
        DocumentLayoutStyles::character_at(reopened.projection(), 4, false)
            .unwrap()
            .bold
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), changed);
}

#[test]
fn html_capture_offsets_include_space_protection_caused_by_an_adjacent_edit() {
    for with_break in [false, true] {
        let original = "<p>A<b>B</b>C</p><!--keep-->";
        let mut document = open(original, Format::Html);
        let lines = document.hard_line_snapshot();
        let space = FormattedTextPayload::new(&lines, " ", vec![]).unwrap();
        let mut fragments = vec![
            ReplacementFragment::Literal(space),
            ReplacementFragment::Capture(1..2),
        ];
        if with_break {
            fragments.push(ReplacementFragment::Literal(
                FormattedTextPayload::new(&lines, "\n", vec![0]).unwrap(),
            ));
        }
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyFragmentEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![
                    FragmentEdit {
                        range: 0..1,
                        fragments: Vec::new(),
                    },
                    FragmentEdit {
                        range: 1..2,
                        fragments,
                    },
                ],
            })
            .unwrap();
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document.text(),
            if with_break { "\u{a0}B\nC" } else { "\u{a0}BC" }
        );
        assert!(
            DocumentLayoutStyles::character_at(document.projection(), 2, false)
                .unwrap()
                .bold
        );
        let changed = document.source_bytes();
        let source = String::from_utf8(changed.clone()).unwrap();
        assert!(!source.contains("white-space"), "{source}");
        assert_eq!(source.matches("&nbsp;").count(), 1);
        let reopened = open(&source, Format::Html);
        assert_eq!(reopened.text(), document.text());
        assert!(
            DocumentLayoutStyles::character_at(reopened.projection(), 2, false)
                .unwrap()
                .bold
        );
        let reopened_payload = reopened
            .hard_line_snapshot()
            .capture(0..reopened.text().len())
            .unwrap();
        assert_eq!(reopened_payload.break_offsets(), if with_break { &[3][..] } else { &[] });
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}
