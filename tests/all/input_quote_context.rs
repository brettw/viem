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
fn empty_source_code_has_literal_single_quote_input() {
    for (source, at) in [("```\n\n```", 4)] {
        let document = open(source, Format::MarkdownSource);
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
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (encoding, bytes, at) in [
            (Encoding::Utf8, b"\r\n".to_vec(), 0),
            (
                Encoding::Utf16Le,
                "\r\n"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                0,
            ),
        ] {
            let document = Document::from_bytes(bytes.clone(), encoding, format).unwrap();
            let input = "\"first\"\r\n`\"code\"`\r\n\"last\"";
            assert_eq!(
                document
                    .transform_source_input(at..at, input, quote)
                    .unwrap(),
                "“first”\r\n`\"code\"`\r\n“last”"
            );
            assert_eq!(document.source_bytes(), bytes);
        }
    }
}

#[test]
fn source_input_can_leave_existing_code_and_enter_new_prose() {
    for (format, source, at, input, expected) in [
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
fn rich_fragments_transform_prose_and_rebase_styles_without_changing_original() {
    for (format, source) in [
        (Format::Markdown, "**\"prose\"** `\"code\"`"),

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

    }
}

#[test]
fn fragment_quotes_keep_configuration_only_typography_and_combining_graphemes() {
    let mut document = open("\"\u{301}word\"", Format::Markdown);
    let mut style = document
        .projection()
        .style_sheet()
        .block_style(&"Paragraph".into())
        .unwrap()
        .clone();
    style.character.size = Some((31.).into());
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(
                StyleDefinitionEdit::UpdateBlock(style),
            )),
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
fn source_fragment_quote_transformation_preserves_normalized_register_text() {
    let source = "\"one\"\r\n\"two\"";
    let document = open(source, Format::MarkdownSource);
    let fragment = document
        .clipboard_fragment(0..document.text().len())
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    // A source-view yank uses the visible, normalized register representation.
    value["plain_text"] = value["source_plain_text"].clone();
    let plain = value["plain_text"].as_str().unwrap().to_owned();
    let fragment = ClipboardFragment::from_json(&value.to_string(), &plain).unwrap();
    let (text, transformed) = fragment.transform_quotes(None, quote).unwrap();
    assert_eq!(text, "“one”\n“two”");
    let value: serde_json::Value = serde_json::from_str(transformed.json()).unwrap();
    assert_eq!(value["source_text"], "“one”\r\n“two”");
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
