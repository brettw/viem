use viem_core::document::{
    ConversionWarning, Document, Encoding, Format, ModelRequest,
    ProjectionWorkScope, StyleApplication, TextEdit,
};
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
#[test]
fn explicit_latin1_conversion_reports_count_and_undo_restores_unicode() {
    let mut document = open("Café مرحبا 💚", Format::PlainText);
    let original = document.source_bytes();
    let prepared = document
        .prepare_model_request(ModelRequest::SetEncoding {
            document: document.id(),
            revision: document.revision(),
            target: Encoding::Latin1,
        })
        .unwrap();
    assert!(matches!(
        prepared.summary().conversion_warnings(),
        [ConversionWarning::UnrepresentableCharacters { count: 6, .. }]
    ));
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), "Café ????? ?");
    assert_eq!(document.encoding(), Encoding::Latin1);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.insert(0, "💚").is_ok());
}
#[test]
fn markdown_code_styles_preserve_source_while_editing() {
    for (source, format) in [("Use `x < y`.\n```rust\n  a\n  b\n```", Format::Markdown)] {
        let mut document = open(source, format);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.text(), "Use x < y.\n  a\n  b");
        assert!(document
            .projection()
            .style_spans()
            .iter()
            .any(|span| span.application == StyleApplication::Named("Code".into())));
        assert!(document
            .projection()
            .blocks()
            .iter()
            .any(|block| block.style.0 == "Code Block"));
        let at = document.text().find("  a").unwrap() + 2;
        document.replace(at..at + 1, "A").unwrap();
        assert!(String::from_utf8(document.source_bytes())
            .unwrap()
            .contains("  A"));
    }
}
#[test]
fn large_fenced_code_edit_is_local_and_fresh_projection_matches() {
    let source = format!(
        "intro\n```\n{}\n```\ntail",
        (0..10_000)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut document = open(&source, Format::Markdown);
    let at = document.text().find("line 5000").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 1, "L")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
    document.commit_model_transaction(prepared).unwrap();
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(
        document.projection().style_spans(),
        fresh.projection().style_spans()
    );
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
}
#[test]
fn named_code_edits_invalidate_only_code_and_keep_raw_source() {
    use viem_core::document::{
        ConfigurationStyleIntent, StyleDefinitionEdit, StyleInvalidationEffect, StyleModelIntent,
        StyleModelRequest,
    };
    let mut document = open("plain `code` tail\nother", Format::Markdown);
    let source = document.source_bytes();
    let mut code = document
        .projection()
        .style_sheet()
        .character_style(&"Code".into())
        .unwrap()
        .clone();
    code.properties.font_families = Some(vec!["Menlo".into()]);
    let request = StyleModelRequest::new(
        document.id(),
        document.revision(),
        StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
            StyleDefinitionEdit::UpdateCharacter(code),
        )),
    );
    let committed = document.apply_style_request(request).unwrap();
    let style = committed.summary().style_change().unwrap();
    assert!(style
        .invalidation_effects()
        .contains(&StyleInvalidationEffect::Shaping));
    assert_eq!(style.affected_ranges(), &[6..10]);
    assert_eq!(document.source_bytes(), source);
    let resolved =
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), 7, false)
            .unwrap();
    assert_eq!(resolved.font_families, ["Menlo"]);
    assert_eq!(resolved.foreground.green, 100.0 / 255.0);
}

#[test]
fn empty_code_blocks_accept_first_typing_inside_the_source_wrappers() {
    for (source, format) in [("```\n```", Format::Markdown)] {
        let mut document = open(source, format);
        assert_eq!(document.text(), "");
        document
            .insert(0, "word")
            .unwrap_or_else(|e| panic!("{format:?} {e:?}"));
        assert_eq!(document.text(), "word");
        assert!(document
            .projection()
            .blocks()
            .iter()
            .any(|block| block.style.0 == "Code Block"));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn ordinary_prose_after_a_fence_keeps_regional_editing_in_a_large_document() {
    let source = format!(
        "```\ncode\n```\n{}",
        (0..10_000)
            .map(|index| format!("prose {index}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let mut document = open(&source, Format::Markdown);
    let at = document.text().find("prose 5000").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 1, "P")],
        })
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 128);
    document.commit_model_transaction(prepared).unwrap();
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(
        document.projection().provenance(),
        fresh.projection().provenance()
    );
}
#[test]
fn literal_backticks_in_code_grow_only_required_delimiters() {
    for (source, at, insert, expected) in [
        ("Use `ab` now", 5, "`", "Use ``a`b`` now"),
        ("Use `ab` now", 4, "`", "Use `` `ab `` now"),
        ("```\na\n```\ntail", 1, "`", "```\na`\n```\ntail"),
        ("```\na\n```\ntail", 0, "```,", "```\n```,a\n```\ntail"),
    ] {
        let mut document = open(source, Format::Markdown);
        document
            .insert(at, insert)
            .unwrap_or_else(|e| panic!("{source:?} {e:?}"));
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let mut document = open("```\na\n```\ntail", Format::Markdown);
    document.replace(0..1, "```").unwrap();
    assert_eq!(document.source_bytes(), b"````\n```\n````\ntail");
    assert_eq!(document.text(), "```\ntail");
}
#[test]
fn grown_inline_code_delimiters_can_be_cleared_with_exact_visible_text() {
    let mut document = open("Use `` a`b `` now", Format::Markdown);
    assert_eq!(document.text(), "Use a`b now");
    document
        .set_semantic_style(4..7, viem_core::document::SemanticInlineStyle::Code, false)
        .unwrap();
    assert_eq!(document.text(), "Use a`b now");
    assert_eq!(document.source_bytes(), b"Use a`b now");
}
