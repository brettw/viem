use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, DocumentError, Encoding, Format, TextEdit};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

fn document(source: &str, format: Format) -> Document {
    let bytes = source
        .chars()
        .map(|ch| u8::try_from(ch as u32).unwrap())
        .collect();
    Document::from_bytes(bytes, Encoding::Latin1, format).unwrap()
}
fn reopened(document: &Document) -> Document {
    Document::from_bytes(
        document.source_bytes(),
        document.encoding(),
        document.format(),
    )
    .unwrap()
}

#[test]
fn markdown_latin1_prose_escapes_unicode_without_changing_encoding_or_neighbors() {
    for (source, bold, link) in [
        ("left middle right", false, None),
        ("left **middle** right", true, None),
        (
            "left [middle](https://example.test/)",
            false,
            Some("https://example.test/"),
        ),
        (
            "left **[middle](https://example.test/)** right",
            true,
            Some("https://example.test/"),
        ),
    ] {
        let mut document = document(source, Format::Markdown);
        let original = document.source_bytes();
        let before = document.text().to_owned();
        document.replace(5..11, "中🙂é").unwrap();
        assert_eq!(
            document.text(),
            format!("{}中🙂é{}", &before[..5], &before[11..])
        );
        assert_eq!(document.encoding(), Encoding::Latin1);
        let changed = document.source_bytes();
        assert!(changed.starts_with(b"left "));
        assert!(changed
            .windows(b"&#x4E2D;&#x1F642;".len())
            .any(|value| value == b"&#x4E2D;&#x1F642;"));
        assert!(changed.contains(&0xe9));
        assert_eq!(reopened(&document).text(), document.text());
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), 5, false)
                .unwrap()
                .bold,
            bold
        );
        assert_eq!(
            document
                .link_at(document.text_point(5).unwrap())
                .unwrap()
                .as_deref(),
            link
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), changed);
    }
}

#[test]
fn native_markdown_replacement_and_continued_typing_preserve_unicode_and_link() {
    for (source, bold, link) in [
        ("left **middle** right", true, None),
        (
            "left [middle](https://example.test/&#x4E2D;) right",
            false,
            Some("https://example.test/中"),
        ),
    ] {
        let original = document(source, Format::Markdown).source_bytes();
        let mut core = Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        for (text_offset, extend_selection) in [(5, false), (11, true)] {
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
        for value in ["中", "🙂"] {
            core.handle(view, CoreEvent::Input(InputEvent::text(value)))
                .unwrap();
        }
        assert_eq!(core.document().text(), "left 中🙂 right");
        assert_eq!(reopened(core.document()).text(), core.document().text());
        for at in [5, 8] {
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false
                )
                .unwrap()
                .bold,
                bold
            );
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(at).unwrap())
                    .unwrap()
                    .as_deref(),
                link
            );
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), original);
    }
}

#[test]
fn markdown_multiline_payloads_and_batch_edits_escape_prose_segments() {
    for source in [
        "before middle after",
        "before **middle** after",
        "- before middle after",
        "> before middle after",
    ] {
        let mut document = document(source, Format::Markdown);
        let at = document.text().find("middle").unwrap();
        document
            .replace(at..at + 6, "中\n🙂")
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert_eq!(reopened(&document).text(), document.text());
        assert!(document.text().contains("中\n🙂"));
    }
    let mut document = document("a &#x4E2D; b", Format::Markdown);
    document
        .apply_edits(vec![TextEdit::new(0..1, "🙂"), TextEdit::new(2..5, "界")])
        .unwrap();
    assert_eq!(document.text(), "🙂 界 b");
    assert_eq!(reopened(&document).text(), document.text());
}

#[test]
fn literal_and_markdown_code_keep_exact_encoding_failures_atomic() {
    for (format, source, range) in [
        (Format::PlainText, "middle", 0..6),
        (Format::Code, "middle", 0..6),
        (Format::MarkdownSource, "**middle**", 2..8),
        (Format::Markdown, "`middle`", 0..6),
        (Format::Markdown, "```\nmiddle\n```", 0..6),
    ] {
        let mut document = document(source, format);
        let original = document.source_bytes();
        let revision = document.revision();
        assert!(
            matches!(
                document.replace(range, "中"),
                Err(DocumentError::UnrepresentableCharacter {
                    encoding: Encoding::Latin1,
                    character: '中'
                })
            ),
            "{format:?}: {source}"
        );
        assert_eq!(document.source_bytes(), original);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.encoding(), Encoding::Latin1);
        assert!(!document.undo());
    }
}

#[test]
fn imported_numeric_references_are_editable_without_rewriting_neighbor_spelling() {
    let source = "a &#x4e2d;&#128578; &amp; &#x65;&#x301; z";
    let mut document = document(source, Format::Markdown);
    assert_eq!(document.text(), "a 中🙂 & e\u{301} z");
    assert_eq!(document.source_bytes(), source.as_bytes());
    document.replace(5..9, "界").unwrap();
    assert_eq!(document.text(), "a 中界 & e\u{301} z");
    assert_eq!(
        document.source_bytes(),
        b"a &#x4e2d;&#x754C; &amp; &#x65;&#x301; z"
    );
    let cluster = document.text().find("e\u{301}").unwrap();
    let original = document.source_bytes();
    assert!(matches!(
        document.replace(cluster..cluster + 1, "X"),
        Err(DocumentError::NotGraphemeBoundary(_))
    ));
    assert_eq!(document.source_bytes(), original);
    document.replace(cluster..cluster + 3, "🙂").unwrap();
    assert_eq!(reopened(&document).text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn references_follow_gfm_replacements_without_creating_hard_lines_or_decoding_code() {
    for source in [
        "&#0; &#10; &#13; &#x7F; &#xD800; &#x110000;",
        "`&#x4E2D;`",
        "```\n&#x4E2D;\n```",
    ] {
        let document = document(source, Format::Markdown);
        assert_eq!(document.source_bytes(), source.as_bytes());
        if source.starts_with('`') {
            assert_eq!(document.text(), "&#x4E2D;");
        } else {
            assert_eq!(document.text(), "� \n \r \u{7f} � �");
        }
        assert_eq!(document.hard_line_snapshot().line_count(), 1);
    }
    let source = "**&#x4E2D;**";
    assert_eq!(document(source, Format::MarkdownSource).text(), source);
}

#[test]
fn typed_reference_spelling_stays_literal_beside_generated_unicode_escapes() {
    let mut document = document("before after", Format::Markdown);
    document.replace(7..7, "&#65; 中 ").unwrap();
    assert_eq!(document.text(), "before &#65; 中 after");
    assert_eq!(document.source_bytes(), b"before \\&#65; &#x4E2D; after");
    assert_eq!(reopened(&document).text(), document.text());
}
