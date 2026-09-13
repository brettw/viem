use viem_core::document::{
    ConversionWarning, Document, Encoding, Format, FormatOperation, ModelRequest, ProjectionWorkScope,
    StyleApplication, TextEdit,
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
fn html_markdown_conversion_is_semantic_explicit_and_undoable() {
    let original = "<h2>Heading</h2><p>A <b>bold</b> and <i>soft</i> &amp; <code>x</code>.</p>";
    let mut document = open(original, Format::Html);
    let before = document.text().to_owned();
    let prepared = document
        .prepare_model_request(ModelRequest::SetFormat {
            document: document.id(),
            revision: document.revision(),
            target: Format::Markdown,
            operation: viem_core::document::FormatOperation::Convert,
        })
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(
        prepared.summary().source_patches()[0].range(),
        0..original.len()
    );
    assert!(prepared.summary().conversion_warnings().is_empty());
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.text(), before);
    assert_eq!(
        String::from_utf8(document.source_bytes()).unwrap(),
        "## Heading\nA **bold** and *soft* & `x`."
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert_eq!(document.format(), Format::Html);
    assert!(document.redo());
    convert(&mut document, Format::Html);
    assert_eq!(document.text(), before);
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("<b>bold</b>"));
}
#[test]
fn reinterpretation_keeps_source_bytes_and_conversion_uses_semantics() {
    let original = "<p><b>Bold</b></p><!-- comment -->";
    let mut document = open(original, Format::HtmlSource);
    convert(&mut document, Format::Html);
    assert_eq!(document.source_bytes(), original.as_bytes());
    document
        .set_format(Format::PlainText, FormatOperation::Reinterpret)
        .unwrap();
    assert_eq!(document.text(), original);
    document
        .set_format(Format::HtmlSource, FormatOperation::Reinterpret)
        .unwrap();
    let warnings = convert(&mut document, Format::MarkdownSource);
    assert!(!warnings.is_empty());
    assert_eq!(document.format(), Format::Markdown);
    assert_eq!(document.text(), "Bold");
    assert_eq!(document.source_bytes(), b"**Bold**");
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
    for (source, format) in [
        (
            "<p>Use <code>x &lt; y</code>.</p><pre><code>  a\n  b</code></pre>",
            Format::Html,
        ),
        ("Use `x < y`.\n```rust\n  a\n  b\n```", Format::Markdown),
    ] {
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
        let before = document.text().to_owned();
        convert(
            &mut document,
            if format == Format::Html {
                Format::Markdown
            } else {
                Format::Html
            },
        );
        assert_eq!(document.text(), before);
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
fn nested_lists_and_code_delimiters_survive_supported_conversion() {
    let mut document = open(
        "- parent\n  4. child\n- tail\n\nUse ``a`b``.",
        Format::Markdown,
    );
    convert(&mut document, Format::Html);
    assert_eq!(document.text(), "parent\nchild\ntail\nUse a`b.");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains(
            "<ul><li>parent<ol start=\"4\"><li value=\"4\">child</li></ol></li><li>tail</li></ul>"
        ));
    convert(&mut document, Format::Markdown);
    assert_eq!(document.text(), "parent\nchild\ntail\nUse a`b.");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("  4. child"));
}
#[test]
fn simple_direct_emphasis_writes_tags_without_private_css_flags() {
    use viem_core::document::{
        CharacterProperties, FontSlant, PersistedStyleIntent, StyleModelIntent, StyleModelRequest,
        TextRange,
    };
    let mut document = open("<p style='font-weight:200'>word</p>", Format::Html);
    let range = TextRange::new(
        document.text_point(0).unwrap(),
        document.text_point(4).unwrap(),
    )
    .unwrap();
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::SetDirectCharacterProperties {
                range,
                properties: CharacterProperties {
                    bold: Some(true),
                    slant: Some(FontSlant::Italic),
                    ..Default::default()
                },
            }),
        ))
        .unwrap();
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.contains("<b><i>word</i></b>"), "{source}");
    assert!(!source.contains("--viem"));
    let resolved =
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap();
    assert_eq!((resolved.base_weight, resolved.weight), (200, 500));
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
    let mut core = Core::new(open("<p>Text</p><!--lost-->", Format::Html));
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
    for (source, format) in [
        ("<pre></pre>", Format::Html),
        ("```\n```", Format::Markdown),
    ] {
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
fn conversion_preserves_first_words_when_list_labels_are_decorations() {
    for (source, from, to) in [
        (
            "- **First** words stay intact.\n- Second item.",
            Format::Markdown,
            Format::Html,
        ),
        (
            "<ol start='9'><li><b>First</b> words stay intact.</li><li>Second item.</li></ol>",
            Format::Html,
            Format::Markdown,
        ),
    ] {
        let mut document = open(source, from);
        let before = document.text().to_owned();
        convert(&mut document, to);
        assert_eq!(document.text(), before);
        convert(&mut document, from);
        assert_eq!(document.text(), before);
        assert_eq!(
            document.projection().list_structure().lists[0].items.len(),
            2
        );
    }
}

#[test]
fn text_paragraphs_and_hard_breaks_convert_to_html_and_back() {
    let original = "One line\ncontinued\n\n# literal & <tag>\n\nLast";
    let mut document = open(original, Format::PlainText);
    assert!(convert(&mut document, Format::HtmlSource).is_empty());
    assert_eq!(document.format(), Format::Html);
    assert_eq!(
        document.source_bytes(),
        b"<p>One line<br>continued</p>\n<p># literal &amp; &lt;tag&gt;</p>\n<p>Last</p>"
    );
    assert_eq!(
        document.text(),
        "One line\ncontinued\n# literal & <tag>\nLast"
    );
    convert(&mut document, Format::PlainText);
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.undo());
    assert_eq!(document.format(), Format::Html);
    assert!(document.undo());
    assert_eq!(document.format(), Format::PlainText);
    assert_eq!(document.source_bytes(), original.as_bytes());
    assert!(document.redo());
    assert_eq!(document.format(), Format::Html);
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
fn rich_text_conversion_flattens_styles_and_keeps_ordinary_visible_content() {
    for format in [Format::Html, Format::HtmlSource] {
        let original = "<h2>Heading</h2><p><a href='https://example.com'><span style='font-size:30pt;text-decoration:underline'>Visible</span></a> text</p><ul><li>First</li><li>Second</li></ul><pre>  code\n  next</pre><!--opaque-->";
        let mut document = open(original, format);
        assert!(!convert(&mut document, Format::PlainText).is_empty());
        assert_eq!(
            document.source_bytes(),
            b"Heading\n\nVisible text\n\nFirst\n\nSecond\n\n  code\n  next"
        );
        assert_eq!(document.format(), Format::PlainText);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original.as_bytes());
        assert_eq!(document.format(), format);
    }
}

#[test]
fn rtf_source_can_convert_to_each_supported_destination() {
    for target in [Format::PlainText, Format::Markdown, Format::Html] {
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
fn quotes_and_supported_markup_convert_semantically_in_both_directions() {
    let original = "> Quoted *words*\n\n## Heading\n\n- First\n- Second";
    let mut document = open(original, Format::MarkdownSource);
    convert(&mut document, Format::Html);
    let html = String::from_utf8(document.source_bytes()).unwrap();
    assert!(
        html.contains("<blockquote><p>Quoted <i>words</i></p></blockquote>"),
        "{html}"
    );
    assert!(html.contains("<h2>Heading</h2>"));
    assert!(html.contains("<ul><li>First</li><li>Second</li></ul>"));
    let visible = document.text().to_owned();
    convert(&mut document, Format::Markdown);
    assert_eq!(document.text(), visible);
    assert_eq!(document.projection().blocks()[0].style.0, "Block quote");
}

#[test]
fn reinterpretation_is_byte_exact_even_across_markup_families_and_bad_decoding() {
    let original = b"<h1># Raw</h1>\r\n<!--keep-->\xff";
    let mut document =
        Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Html).unwrap();
    for target in [Format::Markdown, Format::PlainText, Format::HtmlSource] {
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
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.format(), Format::Html);
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn adjacent_code_paragraphs_keep_boundaries_separate_from_internal_hard_breaks() {
    for (source, from) in [
        ("<pre>one\ninside</pre><pre>two</pre>", Format::Html),
        ("```\none\ninside\n```\n```\ntwo\n```", Format::Markdown),
    ] {
        for target in [
            Format::PlainText,
            if from == Format::Html {
                Format::Markdown
            } else {
                Format::Html
            },
        ] {
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
fn embedded_html_conversion_keeps_alt_and_descendant_text_with_surrounding_content() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for format in [Format::Html, Format::HtmlSource] {
            for target in [Format::PlainText, Format::Markdown] {
                for (body, expected) in [
                    (
                        "<img src='unloaded' alt='A &amp; &lt;br&gt; *cat*'>",
                        "A & <br> *cat*",
                    ),
                    (
                        "<object><p>Fallback &amp; one</p><p>next</p></object>",
                        "Fallback & one\nnext",
                    ),
                    (
                        "<table><tr><td>Cell one</td><td>Cell two</td></tr></table>",
                        "Cell one\nCell two",
                    ),
                    (
                        "<table title='Summary'><tr><td>Cell text</td></tr></table>",
                        "Cell text",
                    ),
                    (
                        "<object title='Summary'><p>Visible text</p></object>",
                        "Visible text",
                    ),
                    ("<img alt=''>", ""),
                    ("<svg></svg>", "[Object]"),
                ] {
                    let source = format!("<p>Before {body} after</p>");
                    let bytes = match encoding {
                        Encoding::Utf8 => source.as_bytes().to_vec(),
                        Encoding::Utf16Le => [0xff, 0xfe]
                            .into_iter()
                            .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
                            .collect(),
                        Encoding::Utf16Be => [0xfe, 0xff]
                            .into_iter()
                            .chain(source.encode_utf16().flat_map(u16::to_be_bytes))
                            .collect(),
                        _ => unreachable!(),
                    };
                    let mut document =
                        Document::from_bytes(bytes.clone(), encoding, format).unwrap();
                    assert!(!convert(&mut document, target).is_empty());
                    assert_eq!(
                        document.text(),
                        format!("Before {expected} after"),
                        "{source} -> {target:?}"
                    );
                    let converted = document.source_bytes();
                    let reopened =
                        Document::from_bytes(converted.clone(), encoding, target).unwrap();
                    assert_eq!(reopened.text(), document.text());
                    assert!(document.undo());
                    assert_eq!(document.source_bytes(), bytes);
                    assert!(document.redo());
                    assert_eq!(document.source_bytes(), converted);
                }
            }
        }
    }
}

#[test]
fn source_backed_named_styles_convert_the_same_from_source_and_rich_views() {
    use viem_core::document::{
        CharacterProperties, CharacterStyle, PersistedStyleIntent, StyleDefinitionEdit,
        StyleDefinitionMetadata, StyleDefinitionOrigin, StyleModelIntent, StyleModelRequest,
        StyleNamespace,
    };
    let mut authored = open("<p>Word</p>", Format::Html);
    authored
        .apply_style_request(StyleModelRequest::new(
            authored.id(),
            authored.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::InsertCharacter {
                    style: CharacterStyle {
                        id: "Accent".into(),
                        based_on: None,
                        properties: CharacterProperties {
                            weight: Some(700),
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
    authored
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: authored.id(),
            revision: authored.revision(),
            range: 0..4,
            namespace: StyleNamespace::Character,
            style: "Accent".into(),
        })
        .unwrap();
    for format in [Format::Html, Format::HtmlSource] {
        let mut document =
            Document::from_bytes(authored.source_bytes(), Encoding::Utf8, format).unwrap();
        convert(&mut document, Format::Markdown);
        assert_eq!(document.source_bytes(), b"**Word**");
        assert_eq!(document.text(), "Word");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), authored.source_bytes());
        assert_eq!(document.format(), format);
    }
}

#[test]
fn literal_text_padding_and_whitespace_reference_spellings_stay_visible() {
    for target in [Format::Html, Format::Markdown] {
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
        convert(&mut document, Format::Html);
        let html = document.source_bytes();
        assert_eq!(
            document.projection().blocks().len(),
            text.matches("\n\n").count() + 1
        );
        convert(&mut document, Format::PlainText);
        assert_eq!(
            document.text(),
            text,
            "{}",
            String::from_utf8(html).unwrap()
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
    for target in [Format::PlainText, Format::Markdown, Format::Html] {
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
fn quoted_literal_block_markers_remain_visible_when_converting_to_markdown() {
    for text in [
        "- literal",
        "+ literal",
        "1. literal",
        "2) literal",
        "# literal",
        "---",
    ] {
        let source = format!("<blockquote><p>{text}</p></blockquote>");
        let mut document = open(&source, Format::Html);
        convert(&mut document, Format::Markdown);
        assert_eq!(document.text(), text);
        assert_eq!(document.projection().blocks().len(), 1);
        assert_eq!(document.projection().blocks()[0].style.0, "Block quote");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn object_fallback_text_cannot_close_surrounding_markdown_code_delimiters() {
    for (source, expected) in [
        ("<p><code><img alt='`literal`'></code></p>", "`literal`"),
        (
            "<p><code><img alt='before`literal`after'></code></p>",
            "before`literal`after",
        ),
        ("<p><code><img alt=' `literal` '></code></p>", " `literal` "),
        (
            "<pre><img alt='```\ninside\n```'></pre>",
            "```\ninside\n```",
        ),
    ] {
        let mut document = open(source, Format::Html);
        convert(&mut document, Format::Markdown);
        assert_eq!(
            document.text(),
            expected,
            "{}",
            String::from_utf8(document.source_bytes()).unwrap()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
