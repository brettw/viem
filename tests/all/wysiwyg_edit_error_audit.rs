//! Cross-adapter audit of legal visible-text replacements.
use viem_core::document::{Document, Encoding, Format, ModelRequest, TextEdit};

const CASES: &[(Format, &str)] = &[
    (Format::Markdown, "a **b** c"),
    (Format::Markdown, "a _b_ c"),
    (Format::Markdown, "a ~~b~~ c"),
    (Format::Markdown, "a [b](url) c"),
    (Format::Markdown, "a [bc](url \"title\") d"),
    (Format::Markdown, "a **[bc](url)** d"),
    (Format::Markdown, "a [**bc**](url) d"),
    (Format::Markdown, "a ![b](url) c"),
    (Format::Markdown, "a &amp; b"),
    (Format::Markdown, "# a\n\nb"),
    (Format::Markdown, "> a\n> b\n\nc"),
    (Format::Markdown, "- a\n  - b\n- c"),
    (Format::Markdown, "1. a\n2. b"),
    (Format::Markdown, "a  \nb\n\nc"),
    (Format::Markdown, "```\na\nb\n```\n\nc"),
    (Format::Markdown, "a\n\n---\n\nb"),
    (Format::Markdown, "a <b>b</b> c"),
    (Format::Markdown, "a <!--keep--> b"),
    (Format::Markdown, "[a]: url\n\n[a] b"),
    (Format::Markdown, "| a | b |\n| - | - |\n| c | d |"),
    (Format::Markdown, "a **é👩‍💻** b"),
    (Format::Rtf, "{\\rtf1 a {\\b b} c}"),
    (Format::Rtf, "{\\rtf1 a\\line b\\par c}"),
    (Format::Rtf, "{\\rtf1 a {\\*\\unknown keep}b}"),
    (
        Format::Rtf,
        "{\\rtf1 a{\\field{\\*\\fldinst HYPERLINK url}{\\fldrslt b}}c}",
    ),
    (Format::Rtf, "{\\rtf1 a{\\pict\\pngblip 00}b}"),
    (Format::Rtf, "{\\rtf1 a{\\object{\\*\\objdata 00}}b}"),
    (Format::Rtf, "{\\rtf1 a\\tab b\\par c}"),
    (Format::Rtf, "{\\rtf1 a\\u233?{\\b b}c}"),
    (Format::Rtf, "{\\rtf1 a\\u-10179?\\u-8704?b}"),
    (Format::Rtf, "{\\rtf1{\\b a\\par b}\\par c}"),
];

#[test]
fn legal_visible_replacement_ranges_preserve_requested_text() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &(format, source) in CASES {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let boundaries = (0..=document.text().len())
            .filter(|&at| document.text_point(at).is_ok())
            .collect::<Vec<_>>();
        for (index, &start) in boundaries.iter().enumerate() {
            for &end in &boundaries[index + 1..] {
                for replacement in ["", "x", " ", "\n"] {
                    checked += 1;
                    let mut expected = document.text().to_owned();
                    expected.replace_range(start..end, replacement);
                    match document.prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(start..end, replacement)],
                    }) {
                        Err(error) => failures.push(format!("{format:?} source={source:?} text={:?} {start}..{end} => {replacement:?}: {error:?}", document.text())),
                        Ok(prepared) => {
                            let no_op = prepared.is_no_op();
                            document.commit_model_transaction(prepared).unwrap();
                            // HTML intentionally protects authored edge spaces
                            // with NBSP; their displayed value remains a space.
                            let visible = |text: &str| { text.to_owned() };
                            if visible(document.text()) != visible(&expected) {
                                failures.push(format!("visible {format:?} {source:?} {start}..{end} => {replacement:?}: expected {expected:?}, got {:?}", document.text()));
                            }
                            let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
                            if reopened.text() != document.text() {
                                failures.push(format!("reopen {format:?} {source:?} {start}..{end} => {replacement:?}: expected {:?}, got {:?}; source={:?}", document.text(), reopened.text(), String::from_utf8_lossy(&document.source_bytes())));
                            }
                            if !no_op {
                                assert!(document.undo());
                                assert_eq!(document.source_bytes(), source.as_bytes());
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "checked {checked} legal replacements; {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn markdown_link_break_repairs_preserve_styles_destinations_source_and_history() {
    use viem_core::layout::DocumentLayoutStyles;
    for source in ["a **[bc](url \"title\")** d", "a [**bc**](url \"title\") d"] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let original = match encoding {
                Encoding::Utf8 | Encoding::Latin1 => source.as_bytes().to_vec(),
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            };
            for range in [2..3, 3..3, 3..4, 2..4] {
                let mut document =
                    Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
                let old_text = document.text().to_owned();
                let mut expected = old_text.clone();
                expected.replace_range(range.clone(), "\n");
                let retained = old_text
                    .char_indices()
                    .filter(|(at, ch)| !range.contains(at) && *ch != '\n')
                    .map(|(at, _)| {
                        (
                            at,
                            DocumentLayoutStyles::semantic_character_at(
                                document.projection(),
                                at,
                                false,
                            )
                            .unwrap(),
                            document.link_at(document.text_point(at).unwrap()).unwrap(),
                        )
                    })
                    .collect::<Vec<_>>();
                let prepared = document
                    .prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(range.clone(), "\n")],
                    })
                    .unwrap();
                let patches = prepared.summary().source_patches().to_vec();
                document.commit_model_transaction(prepared).unwrap();
                assert_eq!(document.text(), expected);
                let after = document.source_bytes();
                let (mut before_at, mut after_at) = (0, 0);
                for patch in patches {
                    let untouched = patch.range().start - before_at;
                    assert_eq!(
                        &original[before_at..before_at + untouched],
                        &after[after_at..after_at + untouched]
                    );
                    before_at = patch.range().end;
                    after_at += untouched + patch.replacement().len();
                }
                assert_eq!(&original[before_at..], &after[after_at..]);
                for (at, style, link) in retained {
                    let mapped = if at < range.start {
                        at
                    } else {
                        at + 1 - range.len()
                    };
                    assert_eq!(
                        DocumentLayoutStyles::semantic_character_at(
                            document.projection(),
                            mapped,
                            false
                        )
                        .unwrap(),
                        style
                    );
                    assert_eq!(
                        document
                            .link_at(document.text_point(mapped).unwrap())
                            .unwrap(),
                        link
                    );
                }
                let reopened =
                    Document::from_bytes(after.clone(), encoding, Format::Markdown).unwrap();
                assert_eq!(reopened.text(), expected);
                assert_eq!(
                    reopened.projection().style_spans(),
                    document.projection().style_spans()
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), after);
            }
        }
    }
}

#[test]
fn literal_boundary_repairs_remain_local_in_a_large_markdown_document() {
    let source = "Untouched paragraph.\n\n".repeat(10_000) + "a ![b](url) c";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let untouched = document.projection().blocks()[5000].id;
    let at = document.projection().text_tree().byte_len() - "![b](url) c".len();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 1, "")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert!(work.decoded_source_bytes() < 128, "{work:?}");
    assert_eq!(work.full_text_bytes_materialized(), 0);
    assert_eq!(prepared.summary().source_patches().len(), 2);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[5000].id, untouched);
    assert!(document.text().ends_with("a [b](url) c"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn deleting_complete_emphasis_removes_empty_markers_and_replacement_keeps_its_style() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for body in [
        "**bold**",
        "__bold__",
        "*bold*",
        "_bold_",
        "***bold***",
        "**_bold_**",
        "**`bold`**",
    ] {
        let source = format!("plain {body} tail");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let expected =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 6, false).unwrap();
        document.replace(6..12, "").unwrap();
        assert_eq!(document.source_bytes(), b"plain ail", "{body}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 150.);
        for (text_offset, extend_selection) in [(6, false), (12, true)] {
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection,
                },
            )
            .unwrap();
        }
        for text in ["X", "Y"] {
            core.handle(view, CoreEvent::Input(InputEvent::text(text)))
                .unwrap_or_else(|error| {
                    panic!(
                        "{body} typing {text}: {error:?}, {:?}",
                        String::from_utf8_lossy(&core.document().source_bytes())
                    )
                });
        }
        assert_eq!(core.document().text(), "plain XYail");
        for at in [6, 7] {
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false
                )
                .unwrap(),
                expected,
                "{body}"
            );
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn crossing_markdown_inline_breaks_keeps_retained_emphasis_and_links() {
    use viem_core::layout::DocumentLayoutStyles;
    for source in [
        "**ab**<br>*cd*",
        "a **bc**  \nd",
        "[ab](https://example.test/)<br>**cd**",
        "**ab**<br>[cd](https://example.test/)",
    ] {
        let original =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let before = original.text().to_owned();
        for start in 0..before.len() {
            for end in start + 1..=before.len() {
                if !before[start..end].contains('\n') {
                    continue;
                }
                for replacement in ["", "X", "\n", "X\nY"] {
                    let mut document = Document::from_bytes(
                        source.as_bytes().to_vec(),
                        Encoding::Utf8,
                        Format::Markdown,
                    )
                    .unwrap();
                    let mut expected = before.clone();
                    expected.replace_range(start..end, replacement);
                    document
                        .replace(start..end, replacement)
                        .unwrap_or_else(|error| {
                            panic!("{source:?} {start}..{end} {replacement:?}: {error:?}")
                        });
                    assert_eq!(document.text(), expected);
                    for old in (0..start)
                        .chain(end..before.len())
                        .filter(|&at| before.as_bytes()[at] != b'\n')
                    {
                        let new = if old < start {
                            old
                        } else {
                            old - (end - start) + replacement.len()
                        };
                        assert_eq!(
                            DocumentLayoutStyles::semantic_character_at(
                                document.projection(),
                                new,
                                false
                            )
                            .unwrap(),
                            DocumentLayoutStyles::semantic_character_at(
                                original.projection(),
                                old,
                                false
                            )
                            .unwrap(),
                            "{source:?} {start}..{end} {replacement:?}: {:?}",
                            String::from_utf8_lossy(&document.source_bytes())
                        );
                        assert_eq!(
                            document.link_at(document.text_point(new).unwrap()).unwrap(),
                            original.link_at(original.text_point(old).unwrap()).unwrap()
                        );
                    }
                    let reopened = Document::from_bytes(
                        document.source_bytes(),
                        Encoding::Utf8,
                        Format::Markdown,
                    )
                    .unwrap();
                    assert_eq!(reopened.text(), expected);
                    assert_eq!(
                        reopened.projection().style_spans(),
                        document.projection().style_spans()
                    );
                    if expected != before {
                        assert!(document.undo());
                        assert_eq!(document.source_bytes(), source.as_bytes());
                    }
                }
            }
        }
    }
}
