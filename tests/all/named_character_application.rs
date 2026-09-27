use viem_core::document::*;
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
        .unwrap_or_else(|| "".into())
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
fn markdown_code_assignment_splits_blocks_and_clears_nested_emphasis() {
    let source = b"# Heading\n\na **bold** c\n\n- item\n- tail\n\n```\ncode body\n```";
    let mut document = open(source, Format::Markdown);
    let text = document.text().to_owned();
    assign(&mut document, 0..text.len(), "Code").unwrap();
    assert_eq!(document.text(), text);
    let after = document.source_bytes();
    let syntax = String::from_utf8(after.clone()).unwrap();
    assert!(!syntax.contains("**"), "{syntax}");
    assert!(syntax.ends_with("```\ncode body\n```"), "{syntax}");
    assert!(!document.projection().style_spans().iter().any(|span|
        span.application == StyleApplication::Semantic(SemanticInlineStyle::Strong)));
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
    assign(&mut document, start..start + 3, "").unwrap();
    assert_eq!(document.text(), text);
    assert_eq!(named(&document, start).0, "");
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
    let mut source = "line\n\n".repeat(10_000);
    source.push_str("`last`");
    let mut document = open(source.as_bytes(), Format::Markdown);
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
    for (style, text) in [("Code", " "), ("", " "), ("Code", " x ")] {
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
    let source = open(b"word", Format::MarkdownSource);
    let internal = source
        .projection()
        .style_sheet()
        .character_styles()
        .find(|style| style.id.is_internal())
        .unwrap();
    assert!(source.validate_typing_named_style(&internal.id).is_err());
}

#[test]
fn default_paragraph_clears_inline_code_without_changing_code_paragraphs() {
    let mut document = open(b"`inline`\n\n```\nblock\n```", Format::Markdown);
    let end = document.text().len();
    assign(&mut document, 0..end, "").unwrap();
    assert_eq!(document.text(), "inline\nblock");
    assert_eq!(document.source_bytes(), b"inline\n\n```\nblock\n```");
    assert!(!document.projection().style_spans().iter().any(|span| matches!(span.application, StyleApplication::Named(_))));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), b"`inline`\n\n```\nblock\n```");
}

#[test]
fn style_choice_clears_all_inline_traits_and_preserves_paragraph_defaults_and_neighbors() {
    use viem_core::layout::DocumentLayoutStyles;
    for (format, source, chosen) in [
        (Format::Rtf,
         r"{\rtf1{\colortbl;\red51\green102\blue153;\red255\green0\blue0;}{\stylesheet{\s1\fs40\cf1 Heading;}{\*\cs2\i Accent;}}\s1 {\fs60\cf2\highlight2\b\i\ul\strike\super\expndtw40\rtlch\lang1036 word}{\*\opaque keep}}",
         "RtfC2"),
    ] {
        for style in [chosen, ""] {
            let mut document = open(source.as_bytes(), format);
            let expected = document.typing_named_style_at(1, BoundaryAffinity::Downstream, &style.into()).unwrap();
            assert_eq!(expected.size, 20.0);
            assert!(!expected.bold);
            assert_eq!(expected.script_position, ScriptPosition::Normal);
            assert!(expected.background.is_none());
            let before = document.source_bytes();
            let left = DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
            let right = DocumentLayoutStyles::semantic_character_at(document.projection(), 3, false).unwrap();
            let changed = assign(&mut document, 1..3, style).unwrap_or_else(|error| panic!("{format:?} {style}: {error:?}"));
            let after = document.source_bytes();
            unchanged_bytes(&before, &after, changed.summary().source_patches());
            let reopened = open(&after, format);
            assert_eq!(reopened.text(), "word");
            for at in [1, 2] {
                assert_eq!(DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false).unwrap(), expected, "{format:?} {style}");
            }
            assert_eq!(DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false).unwrap(), left);
            assert_eq!(DocumentLayoutStyles::semantic_character_at(reopened.projection(), 3, false).unwrap(), right);
            assert!(String::from_utf8_lossy(&after).contains("keep"));
            assert!(document.undo());
            assert_eq!(document.source_bytes(), before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}

#[test]
fn markdown_partial_style_choice_clears_emphasis_only_in_selection() {
    use viem_core::layout::DocumentLayoutStyles;
    for chosen in ["Code", ""] {
        let source = b"# Heading\n\nleft ***word*** right";
        let mut document = open(source, Format::Markdown);
        let start = document.text().find("word").unwrap() + 1;
        assign(&mut document, start..start + 2, chosen).unwrap();
        for at in [start, start + 1] {
            let value = DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
            assert!(!value.bold);
            assert_eq!(value.slant, FontSlant::Upright);
            assert_eq!(named(&document, at).0, chosen);
        }
        for at in [start - 1, start + 2] {
            let value = DocumentLayoutStyles::semantic_character_at(document.projection(), at, false).unwrap();
            assert!(value.bold);
            assert_eq!(value.slant, FontSlant::Italic);
        }
        assert_eq!(document.text(), "Heading\nleft word right");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}

#[test]
fn source_view_style_choices_clear_inline_traits_without_changing_visible_words() {
    use viem_core::layout::DocumentLayoutStyles;
    for (format, source) in [
        (Format::MarkdownSource, "left ***word*** right"),
    ] {
        for chosen in ["Code", ""] {
            let mut document = open(source.as_bytes(), format);
            let start = document.text().find("word").unwrap();
            assign(&mut document, start..start + 4, chosen).unwrap_or_else(|error| panic!("{format:?} {chosen}: {error:?}"));
            let source_after = document.source_bytes();
            let rich = open(&source_after, Format::Markdown);
            let start = rich.text().find("word").unwrap();
            let value = DocumentLayoutStyles::semantic_character_at(rich.projection(), start, false).unwrap();
            assert!(!value.bold);
            assert_eq!(value.slant, FontSlant::Upright);
            assert_eq!(named(&rich, start).0, chosen);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}
