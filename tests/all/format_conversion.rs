use viem_core::document::{
    ConversionWarning, Document, Encoding, Format, FormatOperation, ModelRequest,
    ProjectionWorkScope, StyleApplication, TextEdit,
};
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn convert(document: &mut Document, target: Format) -> Vec<ConversionWarning> {
    let prepared = document
        .prepare_model_request(ModelRequest::SetFormat {
            document: document.id(),
            revision: document.revision(),
            target,
            operation: viem_core::document::FormatOperation::Convert,
        })
        .unwrap();
    let warnings = prepared.summary().conversion_warnings().to_vec();
    document.commit_model_transaction(prepared).unwrap();
    warnings
}

#[test]
fn reinterpretation_keeps_source_bytes_and_conversion_uses_semantics() {
    let original = "**Bold**";
    let mut document = open(original, Format::MarkdownSource);
    convert(&mut document, Format::Markdown);
    assert_eq!(document.source_bytes(), original.as_bytes());
    document
        .set_format(Format::Code, FormatOperation::Reinterpret)
        .unwrap();
    assert_eq!(document.text(), original);
    document
        .set_format(Format::MarkdownSource, FormatOperation::Reinterpret)
        .unwrap();
    let warnings = convert(&mut document, Format::PlainText);
    assert!(!warnings.is_empty());
    assert_eq!(document.format(), Format::PlainText);
    assert_eq!(document.text(), "Bold");
    assert_eq!(document.source_bytes(), b"Bold");
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
fn code_imports_are_named_preserved_and_convert() {
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
fn native_conversions_deliver_loss_warning_as_readonly_output_message() {
    use viem_core::command::ex_execute::{ExFrontendRequest, ExInfoRequest};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(open(r"{\rtf1 Text}", Format::Rtf));
    let view = core.add_view(MockTextMeasurementProvider::new(), 240.0, 100.0);
    let outcome = core
        .handle(
            view,
            CoreEvent::SetFormat {
                operation: viem_core::FormatOperation::Convert,
                document: core.document().id(),
                revision: core.document().revision(),
                target: Format::Markdown,
            },
        )
        .unwrap();
    assert!(
        matches!(outcome.command.unwrap().ex_outcome.unwrap().frontend_requests.as_slice(), [ExFrontendRequest::Info(ExInfoRequest::Message(text))] if text.starts_with("Warning:"))
    );
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

#[test]
fn text_to_markdown_preserves_literal_syntax_and_paragraph_hard_breaks() {
    let original =
        "# literal\nsecond line\n\n- ordinary\n\n1. ordinary\n\n> ordinary\n\n*literal* and <br>";
    let mut document = open(original, Format::PlainText);
    convert(&mut document, Format::MarkdownSource);
    assert_eq!(document.format(), Format::Markdown);
    assert_eq!(
        document.text(),
        "# literal\nsecond line\n- ordinary\n1. ordinary\n> ordinary\n*literal* and <br>"
    );
    assert!(document
        .projection()
        .blocks()
        .iter()
        .all(|block| block.kind == viem_core::document::BlockKind::Paragraph));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("literal<br>second"));
    convert(&mut document, Format::PlainText);
    assert_eq!(document.text(), original);
}

#[test]
fn rtf_source_can_convert_to_each_supported_destination() {
    for target in [Format::PlainText, Format::Markdown] {
        let original = "{\\rtf1\\ansi One \\b bold\\b0\\par Next}";
        let mut document = open(original, Format::Rtf);
        convert(&mut document, target);
        assert_eq!(document.format(), target);
        assert_eq!(
            document.text(),
            if target == Format::PlainText {
                "One bold\n\nNext"
            } else {
                "One bold\nNext"
            }
        );
        if target == Format::Markdown {
            assert!(String::from_utf8(document.source_bytes())
                .unwrap()
                .contains("**bold**"));
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert_eq!(document.format(), Format::Rtf);
    }
}

#[test]
fn reinterpretation_is_byte_exact_even_across_markup_families_and_bad_decoding() {
    let original = b"<h1># Raw</h1>\r\n<!--keep-->\xff";
    let mut document =
        Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Code).unwrap();
    for target in [Format::Markdown, Format::PlainText] {
        let prepared = document
            .prepare_model_request(ModelRequest::SetFormat {
                document: document.id(),
                revision: document.revision(),
                target,
                operation: FormatOperation::Reinterpret,
            })
            .unwrap();
        assert!(prepared.summary().source_patches().is_empty());
        assert!(prepared.summary().conversion_warnings().is_empty());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.source_bytes(), original);
        assert_eq!(document.format(), target);
    }
    for _ in 0..2 {
        assert!(document.undo());
    }
    assert_eq!(document.format(), Format::Code);
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn adjacent_code_paragraphs_keep_boundaries_separate_from_internal_hard_breaks() {
    for (source, from) in [("```\none\ninside\n```\n```\ntwo\n```", Format::Markdown)] {
        for target in [Format::PlainText] {
            let mut document = open(source, from);
            assert_eq!(document.projection().blocks().len(), 2);
            convert(&mut document, target);
            if target == Format::PlainText {
                assert_eq!(document.text(), "one\ninside\n\ntwo");
            } else {
                assert_eq!(document.text(), "one\ninside\ntwo");
                assert_eq!(document.projection().blocks().len(), 2);
                assert!(document
                    .projection()
                    .blocks()
                    .iter()
                    .all(|block| block.style.0 == "Code Block"));
            }
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn literal_text_padding_and_whitespace_reference_spellings_stay_visible() {
    for target in [Format::Markdown] {
        let original = "  leading  space\tend  \n next\n\n&#32; & plain";
        let mut document = open(original, Format::PlainText);
        convert(&mut document, target);
        assert_eq!(
            document.text(),
            "  leading  space\tend  \n next\n&#32; & plain",
            "{target:?}: {}",
            String::from_utf8(document.source_bytes()).unwrap()
        );
        convert(&mut document, Format::PlainText);
        assert_eq!(document.text(), original);
    }
}

#[test]
fn paired_text_breaks_preserve_empty_paragraphs_and_unpaired_hard_breaks() {
    for text in [
        "A\n\nB",
        "A\n\n\nB",
        "A\n\n\n\nB",
        "\n\nA\n\n",
        "A\n\n\n\n\nB",
    ] {
        let mut document = open(text, Format::PlainText);
        convert(&mut document, Format::Markdown);
        let markdown = document.source_bytes();
        assert_eq!(
            document.projection().blocks().len(),
            text.matches("\n\n").count() + 1
        );
        convert(&mut document, Format::PlainText);
        assert_eq!(
            document.text(),
            text,
            "{}",
            String::from_utf8(markdown).unwrap()
        );
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), text.as_bytes());
    }
}

#[test]
fn text_conversion_to_markdown_retains_surplus_paragraph_delimiters() {
    let mut document = open("A\n\n\n\nB", Format::PlainText);
    let warnings = convert(&mut document, Format::Markdown);
    assert_eq!(document.source_bytes(), b"A\n\n\n\nB");
    assert_eq!(document.projection().blocks().len(), 3, "{warnings:?}");
    convert(&mut document, Format::PlainText);
    assert_eq!(document.text(), "A\n\n\n\nB");
}

#[test]
fn opaque_rtf_conversion_has_readable_placeholders_and_no_rtf_authoring() {
    let source = "{\\rtf1 Before {\\pict 0000} after}";
    for target in [Format::PlainText, Format::Markdown] {
        let mut document = open(source, Format::Rtf);
        assert!(!convert(&mut document, target).is_empty());
        assert_eq!(document.text(), "Before [Object] after");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let mut document = open("Literal", Format::PlainText);
    assert!(document
        .set_format(Format::Rtf, FormatOperation::Convert)
        .is_err());
    assert_eq!(document.format(), Format::PlainText);
    assert_eq!(document.source_bytes(), b"Literal");
}

#[test]
fn code_to_markdown_keeps_literal_html_entities_and_undo_state() {
    let source = "# literal\n<b>tags &amp; &copy; &#169; &notAnEntity;</b>";
    let mut document = open(source, Format::Code);
    convert(&mut document, Format::Markdown);
    assert_eq!(document.text(), source);
    let saved = document.source_bytes();
    assert_eq!(
        Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown)
            .unwrap()
            .text(),
        source
    );
    assert!(document.undo());
    assert_eq!(document.format(), Format::Code);
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.format(), Format::Markdown);
    assert_eq!(document.source_bytes(), saved);
}
