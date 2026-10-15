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
fn physical_source_quotes_track_encoding_and_original_line_endings() {
    for format in [Format::Markdown] {
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

