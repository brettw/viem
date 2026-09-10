use viem_core::document::*;

fn open(text: &str, format: Format) -> Document {
    Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn quote(character: char, previous: Option<char>) -> char {
    let opening = previous.map_or(true, |value| {
        value.is_whitespace()
            || matches!(value, '(' | '[' | '{' | '<' | '-' | '–' | '—' | '“' | '‘')
    });
    match (character, opening) {
        ('"', true) => '“',
        ('"', false) => '”',
        ('\'', true) => '‘',
        _ => '’',
    }
}

#[test]
fn source_batch_classifies_new_syntax_and_code_before_quote_conversion() {
    for (format, input, expected) in [
        (
            Format::HtmlSource,
            "<p title=\"literal\">\"prose\" <code>\"code\"</code></p><pre>'code'</pre>",
            "<p title=\"literal\">“prose” <code>\"code\"</code></p><pre>'code'</pre>",
        ),
        (
            Format::MarkdownSource,
            "\"prose\" `\"code\"`\n\n```\n'code'\n```\n\n\"prose\"",
            "“prose” `\"code\"`\n\n```\n'code'\n```\n\n“prose”",
        ),
    ] {
        let document = open("", format);
        assert_eq!(
            document
                .transform_text_input(0..0, BoundaryAffinity::Downstream, input, quote)
                .unwrap(),
            expected,
            "{format:?}"
        );
        assert_eq!(document.source_bytes(), b"");
    }
}

#[test]
fn empty_html_code_uses_the_actual_innermost_insertion_context() {
    for source in [
        "<p><code></code></p>",
        "<p><b><code></code></b></p>",
        "<pre></pre>",
    ] {
        let document = open(source, Format::Html);
        assert!(
            document
                .is_code_at(0, BoundaryAffinity::Downstream)
                .unwrap(),
            "{source}"
        );
        assert_eq!(
            document
                .transform_text_input(0..0, BoundaryAffinity::Downstream, "\"code\"", quote)
                .unwrap(),
            "\"code\""
        );
    }
}

#[test]
fn empty_source_code_has_literal_single_quote_input() {
    for (source, at) in [("<p><code></code></p>", 9), ("<pre></pre>", 5)] {
        let document = open(source, Format::HtmlSource);
        assert!(
            document
                .is_code_at(at, BoundaryAffinity::Downstream)
                .unwrap(),
            "{source}"
        );
        assert_eq!(
            document
                .transform_text_input(at..at, BoundaryAffinity::Downstream, "\"", quote)
                .unwrap(),
            "\""
        );
    }
}

#[test]
fn physical_source_quotes_track_encoding_and_original_line_endings() {
    for format in [Format::Html, Format::HtmlSource] {
        for (encoding, bytes, at) in [
            (Encoding::Utf8, b"<p></p>\r\n".to_vec(), 3),
            (
                Encoding::Utf16Le,
                "<p></p>\r\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                6,
            ),
        ] {
            let document = Document::from_bytes(bytes.clone(), encoding, format).unwrap();
            let input = "\"first\"\r\n<code>\"code\"</code>\r\n\"last\"";
            assert_eq!(
                document
                    .transform_source_input(at..at, input, quote)
                    .unwrap(),
                "“first”\r\n<code>\"code\"</code>\r\n“last”"
            );
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn source_input_can_leave_existing_code_and_enter_new_prose() {
    for (format, source, at, input, expected) in [
        (
            Format::HtmlSource,
            "<p><code></code></p>",
            9,
            "\"code\"</code> \"prose\"<code>",
            "\"code\"</code> “prose”<code>",
        ),
        (
            Format::MarkdownSource,
            "```\n\n```",
            4,
            "\"code\"\n```\n\n\"prose\"\n```\n",
            "\"code\"\n```\n\n“prose”\n```\n",
        ),
    ] {
        let document = open(source, format);
        assert_eq!(
            document
                .transform_text_input(at..at, BoundaryAffinity::Downstream, input, quote)
                .unwrap(),
            expected,
            "{format:?}"
        );
    }
}

#[test]
fn named_code_inheritance_is_semantic_and_direct_monospace_remains_prose() {
    let source = "{\\rtf1{\\stylesheet{\\s0 Normal;}{\\s1\\sbasedon0 Code Block;}{\\s2\\sbasedon1 Derived;}{\\cs1 Code;}{\\cs2\\sbasedon1 Derived Code;}}\\s2 Block\\par\\s0 {\\cs2 Inline} ordinary}";
    let document = open(source, Format::Rtf);
    assert!(document
        .is_code_at(0, BoundaryAffinity::Downstream)
        .unwrap());
    let inline = document.text().find("Inline").unwrap();
    assert!(document
        .is_code_at(inline, BoundaryAffinity::Downstream)
        .unwrap());
    assert!(document.character_style_is_code(&"RtfC2".into()));
    let ordinary = document.text().find("ordinary").unwrap();
    assert!(!document
        .is_code_at(ordinary, BoundaryAffinity::Downstream)
        .unwrap());
    let document = open(
        "<p style='font-family:monospace'>ordinary</p>",
        Format::Html,
    );
    assert!(!document
        .is_code_at(0, BoundaryAffinity::Downstream)
        .unwrap());
}

#[test]
fn rich_fragments_transform_prose_and_rebase_styles_without_changing_original() {
    for (format, source) in [
        (
            Format::Html,
            "<p><b>\"prose\"</b> <code>\"code\"</code></p><!--keep-->",
        ),
        (Format::Markdown, "**\"prose\"** `\"code\"`"),
        (
            Format::Rtf,
            "{\\rtf1{\\stylesheet{\\cs1 Code;}}{\\b \"prose\"} {\\cs1 \"code\"}}",
        ),
    ] {
        let document = open(source, format);
        let fragment = document
            .clipboard_fragment(0..document.text().len())
            .unwrap();
        let original = fragment.json().to_owned();
        let (text, transformed) = fragment.transform_quotes(None, quote).unwrap();
        assert_eq!(text, "“prose” \"code\"", "{format:?}");
        assert_eq!(fragment.json(), original);
        ClipboardFragment::from_json(transformed.json(), &text).unwrap();
        let before: serde_json::Value = serde_json::from_str(&original).unwrap();
        let after: serde_json::Value = serde_json::from_str(transformed.json()).unwrap();
        assert_eq!(
            before["character_runs"][0]["bold"],
            after["character_runs"][0]["bold"]
        );
        assert_eq!(after["character_runs"][0]["end"], 11);
        assert_eq!(after["source_plain_text"], text);
        if format == Format::Html {
            assert!(after["source_text"]
                .as_str()
                .unwrap()
                .ends_with("<!--keep-->"));
        }
    }
}

#[test]
fn fragment_quotes_keep_configuration_only_typography_and_combining_graphemes() {
    let mut document = open("<p>\"\u{301}word\"</p>", Format::Html);
    let mut style = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    style.character.size = Some(31.);
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(style),
            }),
        ))
        .unwrap();
    let fragment = document
        .clipboard_fragment(0..document.text().len())
        .unwrap();
    let (text, transformed) = fragment.transform_quotes(None, quote).unwrap();
    assert_eq!(text, "“\u{301}word”");
    let value: serde_json::Value = serde_json::from_str(transformed.json()).unwrap();
    assert!(value["character_runs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|run| run["size"] == 31.));
}

#[test]
fn physical_rtf_source_only_transforms_quotes_in_visible_noncode_text() {
    let document = open("{\\rtf1 Before }", Format::Rtf);
    let at = document.source_bytes().len() - 1;
    let input = "\"prose\"{\\*\\unknown \"syntax\"}";
    assert_eq!(
        document
            .transform_source_input(at..at, input, quote)
            .unwrap(),
        "“prose”{\\*\\unknown \"syntax\"}"
    );
}

#[test]
fn source_fragment_quote_transformation_preserves_normalized_register_text() {
    let source = "<p>\"one\"</p>\r\n<p>\"two\"</p>";
    let document = open(source, Format::HtmlSource);
    let fragment = document
        .clipboard_fragment(0..document.text().len())
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    // A source-view yank uses the visible, normalized register representation.
    value["plain_text"] = value["source_plain_text"].clone();
    let plain = value["plain_text"].as_str().unwrap().to_owned();
    let fragment = ClipboardFragment::from_json(&value.to_string(), &plain).unwrap();
    let (text, transformed) = fragment.transform_quotes(None, quote).unwrap();
    assert_eq!(text, "<p>“one”</p>\n<p>“two”</p>");
    let value: serde_json::Value = serde_json::from_str(transformed.json()).unwrap();
    assert_eq!(value["source_text"], "<p>“one”</p>\r\n<p>“two”</p>");
    ClipboardFragment::from_json(transformed.json(), &text).unwrap();
}

#[test]
fn escaped_markdown_backticks_are_prose_but_unfinished_code_is_literal() {
    for (source, expected) in [
        ("\\`prose ", "“"),
        ("``code ", "\""),
        ("`code ", "\""),
        ("`closed` ", "“"),
    ] {
        let document = open(source, Format::MarkdownSource);
        let at = document.text().len();
        assert_eq!(
            document
                .transform_text_input(at..at, BoundaryAffinity::Downstream, "\"", quote)
                .unwrap(),
            expected,
            "{source}"
        );
    }
}
