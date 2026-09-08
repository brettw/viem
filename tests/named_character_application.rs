use evim_core::document::*;
use std::ops::Range;

fn open(source: &[u8], format: Format) -> Document {
    Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap()
}
fn assign(
    document: &mut Document,
    range: Range<usize>,
    style: &str,
) -> Result<CommittedModelTransaction, ModelTransactionError> {
    document.apply_model_request(ModelRequest::AssignNamedStyle {
        document: document.id(),
        revision: document.revision(),
        range,
        namespace: StyleNamespace::Character,
        style: style.into(),
    })
}
fn named(document: &Document, at: usize) -> StyleId {
    document
        .projection()
        .style_spans()
        .iter()
        .filter_map(|span| {
            if span.range.contains(&at) {
                if let StyleApplication::Named(id) = &span.application {
                    return Some(id.clone());
                }
            }
            None
        })
        .last()
        .unwrap_or_else(|| "Character".into())
}
fn unchanged_bytes(before: &[u8], after: &[u8], patches: &[SourcePatch]) {
    let mut old = 0;
    let mut new = 0;
    for patch in patches {
        let length = patch.range().start - old;
        assert_eq!(&before[old..old + length], &after[new..new + length]);
        old = patch.range().end;
        new += length + patch.replacement().len();
    }
    assert_eq!(&before[old..], &after[new..]);
}

#[test]
fn html_named_character_assignments_stay_inside_blocks_and_preserve_irregular_source() {
    let original = b"<!--keep--><h2>Head</h2><p>A <i>nested</i><br>hard</p><ul><li>list<p>child</p><pre>code\nbody</pre></li><li>tail</li></ul><p>x\n\0\n y</p><!--end-->";
    for style in ["Code", "Accent"] {
        let mut document = open(original, Format::Html);
        if style == "Accent" {
            document
                .apply_style_request(StyleModelRequest::new(
                    document.id(),
                    document.revision(),
                    StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                        origin: StyleDefinitionOrigin::SourceBacked,
                        edit: StyleDefinitionEdit::InsertCharacter {
                            style: CharacterStyle {
                                id: "Accent".into(),
                                based_on: Some("Character".into()),
                                properties: CharacterProperties {
                                    underline: Some(true),
                                    ..Default::default()
                                },
                            },
                            metadata: StyleDefinitionMetadata {
                                display_name: "Accent".into(),
                                origin: StyleDefinitionOrigin::SourceBacked,
                            },
                        },
                    }),
                ))
                .unwrap();
        }
        let before = document.source_bytes();
        let text = document.text().to_owned();
        let breaks = document
            .hard_line_snapshot()
            .capture(0..text.len())
            .unwrap()
            .break_offsets()
            .to_vec();
        let changed = assign(&mut document, 0..text.len(), style).unwrap();
        let after = document.source_bytes();
        unchanged_bytes(&before, &after, changed.summary().source_patches());
        assert_eq!(document.text(), text);
        assert_eq!(
            document
                .hard_line_snapshot()
                .capture(0..text.len())
                .unwrap()
                .break_offsets(),
            breaks
        );
        let reopened = open(&after, Format::Html);
        assert_eq!(reopened.text(), text);
        for (at, c) in text.char_indices().filter(|(_, c)| *c != '\n') {
            assert_eq!(named(&reopened, at).0, style, "{style} {at} {c:?}");
        }
        let syntax = String::from_utf8(after.clone()).unwrap();
        for structural in ["<h2>", "<p>", "<ul>", "<li>", "<pre>"] {
            assert!(!syntax.contains(&format!("<code>{structural}")), "{syntax}");
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), before);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), after);
        if style == "Accent" {
            let start = document.text().find("nested").unwrap();
            assign(&mut document, start..start + "nested".len(), "Code").unwrap();
            assert_eq!(named(&document, start).0, "Code");
            assert_eq!(named(&document, start - 1).0, "Accent");
            assert_eq!(named(&document, start + "nested".len() + 1).0, "Accent");
            assert!(document.undo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}

#[test]
fn html_reassigns_code_and_custom_styles_only_inside_selected_inline_text() {
    let mut document = open(b"<p>left <code>a<i>bc</i>d</code> right</p>", Format::Html);
    let original = document.source_bytes();
    let at = document.text().find("bc").unwrap();
    assign(&mut document, at..at + 2, "Character").unwrap();
    assert_eq!(named(&document, at).0, "Character");
    assert_eq!(named(&document, at - 1).0, "Code");
    assert_eq!(named(&document, at + 2).0, "Code");
    assign(&mut document, at..at + 2, "Code").unwrap();
    assert_eq!(named(&document, at).0, "Code");
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn markdown_code_assignment_splits_blocks_and_preserves_nested_emphasis() {
    let source = b"# Heading\n\na **bold** c\n\n- item\n- tail\n\n```\ncode body\n```";
    let mut document = open(source, Format::Markdown);
    let text = document.text().to_owned();
    let strong = document
        .projection()
        .style_spans()
        .iter()
        .filter(|span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong))
        .cloned()
        .collect::<Vec<_>>();
    assign(&mut document, 0..text.len(), "Code").unwrap();
    assert_eq!(document.text(), text);
    let after = document.source_bytes();
    let syntax = String::from_utf8(after.clone()).unwrap();
    assert!(syntax.contains("**`bold`**"), "{syntax}");
    assert!(syntax.ends_with("```\ncode body\n```"), "{syntax}");
    assert_eq!(
        document
            .projection()
            .style_spans()
            .iter()
            .filter(
                |span| span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)
            )
            .cloned()
            .collect::<Vec<_>>(),
        strong
    );
    assert_eq!(open(&after, Format::Markdown).text(), text);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn markdown_base_character_removes_partial_code_with_literal_markers_losslessly() {
    let mut document = open(b"prefix ``a *b* ` c`` suffix", Format::Markdown);
    let before = document.source_bytes();
    let text = document.text().to_owned();
    let start = text.find("*b*").unwrap();
    assign(&mut document, start..start + 3, "Character").unwrap();
    assert_eq!(document.text(), text);
    assert_eq!(named(&document, start).0, "Character");
    assert_eq!(named(&document, start - 1).0, "Code");
    assert_eq!(named(&document, start + 3).0, "Code");
    let after = document.source_bytes();
    assert_eq!(open(&after, Format::Markdown).text(), text);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn continuing_named_typing_uses_one_local_literal_patch_in_a_large_document() {
    let mut source = "<p>line</p>".repeat(10_000);
    source.push_str("<p><code>last</code></p><!--keep-->");
    let mut document = open(source.as_bytes(), Format::Html);
    for input in ["b", "é", "👩‍💻"] {
        let at = document.text().len();
        let payload =
            FormattedTextPayload::new(&document.hard_line_snapshot(), input, vec![]).unwrap();
        let (prepared, caret) = document
            .prepare_insertion_with_typing_style(
                FormattedPayloadEdit::new(at..at, payload)
                    .with_boundary_affinity(BoundaryAffinity::Upstream),
                Some(&"Code".into()),
                &[],
            )
            .unwrap();
        assert_eq!(caret, at + input.len());
        assert_eq!(prepared.summary().source_patches().len(), 1);
        assert_eq!(
            prepared.summary().source_patches()[0].replacement(),
            input.as_bytes()
        );
        assert!(prepared.summary().projection_work().projected_hard_lines() <= 2);
        document.commit_model_transaction(prepared).unwrap();
    }
}

#[test]
fn named_typing_preserves_space_only_markdown_and_rejects_internal_styles() {
    let mut document = open(b"tail", Format::Markdown);
    let before = document.source_bytes();
    let mut at = 0;
    for (style, text) in [("Code", " "), ("Character", " "), ("Code", " x ")] {
        let payload =
            FormattedTextPayload::new(&document.hard_line_snapshot(), text, vec![]).unwrap();
        let start = at;
        at = document
            .insert_with_typing_style(
                FormattedPayloadEdit::new(at..at, payload)
                    .with_boundary_affinity(BoundaryAffinity::Upstream),
                Some(&style.into()),
                &[],
            )
            .unwrap();
        assert_eq!(at, start + text.len());
        assert_eq!(named(&document, start).0, style);
    }
    assert_eq!(document.text(), "   x tail");
    let after = document.source_bytes();
    assert_eq!(open(&after, Format::Markdown).text(), document.text());
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.source_bytes(), before);
    let source = open(b"<p>word</p>", Format::HtmlSource);
    let internal = source
        .projection()
        .style_sheet()
        .character_styles()
        .find(|style| style.id.is_internal())
        .unwrap();
    assert!(source.validate_typing_named_style(&internal.id).is_err());
}

#[test]
fn html_named_assignment_preserves_existing_direct_properties() {
    use evim_core::layout::DocumentLayoutStyles;
    let mut document = open(
        b"<p><span style='font-size:30pt;font-weight:450'><b>word</b></span></p>",
        Format::Html,
    );
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: "Accent".into(),
                        based_on: Some("Character".into()),
                        properties: CharacterProperties {
                            size: Some(22.0),
                            weight: Some(600),
                            ..Default::default()
                        },
                    },
                    metadata: StyleDefinitionMetadata {
                        display_name: "Accent".into(),
                        origin: StyleDefinitionOrigin::SourceBacked,
                    },
                },
            }),
        ))
        .unwrap();
    let before = document.source_bytes();
    assign(&mut document, 1..3, "Accent").unwrap();
    let after = document.source_bytes();
    let reopened = open(&after, Format::Html);
    let resolved = DocumentLayoutStyles::character_at(reopened.projection(), 1, false).unwrap();
    assert_eq!(resolved.size, 30.0);
    assert_eq!(resolved.base_weight, 450);
    assert!(resolved.bold);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), after);
}
