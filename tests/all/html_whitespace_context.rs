use viem_core::document::{Document, Encoding, Format, HistoryRetentionPolicy};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn encoded(encoding: Encoding, source: &str) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => source.as_bytes().to_vec(),
        Encoding::Latin1 => source
            .chars()
            .map(|ch| u8::try_from(ch as u32).unwrap())
            .collect(),
        Encoding::Utf16Le => [0xff, 0xfe]
            .into_iter()
            .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
            .collect(),
        Encoding::Utf16Be => [0xfe, 0xff]
            .into_iter()
            .chain(source.encode_utf16().flat_map(u16::to_be_bytes))
            .collect(),
    }
}

fn assert_reopens(document: &Document, expected: &str) {
    assert_eq!(document.text(), expected);
    let reopened =
        Document::from_bytes(document.source_bytes(), document.encoding(), Format::Html).unwrap();
    assert_eq!(reopened.text(), expected);
}

#[test]
fn generated_space_compacts_after_its_creation_history_is_pruned() {
    let mut document = html("<p>Hello,</p><!--keep-->");
    document.insert(6, " ").unwrap();
    document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
    assert_eq!(document.history_status().node_count, 1);
    assert!(!document.history_status().can_undo);

    for ch in "world!".chars() {
        document
            .insert(document.text().len(), &ch.to_string())
            .unwrap();
    }
    assert_eq!(document.source_bytes(), b"<p>Hello, world!</p><!--keep-->");
    assert_reopens(&document, "Hello, world!");
}

#[test]
fn source_authored_canonical_whitespace_span_survives_switching_to_wysiwyg() {
    for content in [" ", "&#32;"] {
        let mut document = Document::from_bytes(
            b"<p>Hello,</p><!--keep-->".to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap();
        let wrapper = format!("<span style=\"white-space: pre-wrap\">{content}</span>");
        document.insert(9, &wrapper).unwrap();
        document
            .set_format(
                Format::Html,
                viem_core::document::FormatOperation::Reinterpret,
            )
            .unwrap();
        assert_reopens(&document, "Hello, ");
        document.insert(7, "world!").unwrap();

        let expected = format!(
            "<p>Hello,<span style=\"white-space: pre-wrap\">{content}world!</span></p><!--keep-->"
        );
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_reopens(&document, "Hello, world!");
    }
}

#[test]
fn scalar_space_compaction_preserves_source_encoding_and_bom() {
    for encoding in [Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
        let original = "<!--café--><p>Café,</p><!--keep-->";
        let mut document =
            Document::from_bytes(encoded(encoding, original), encoding, Format::Html).unwrap();
        for ch in " monde!".chars() {
            document
                .insert(document.text().len(), &ch.to_string())
                .unwrap();
        }
        assert_eq!(
            document.source_bytes(),
            encoded(encoding, "<!--café--><p>Café, monde!</p><!--keep-->"),
            "{encoding:?}"
        );
        assert_reopens(&document, "Café, monde!");
    }
}

#[test]
fn replacing_the_last_word_with_a_space_preserves_the_new_trailing_space() {
    let mut document = html("<p>Hello,world</p><!--keep-->");
    document.replace(6..11, " ").unwrap();
    assert_reopens(&document, "Hello,\u{a0}");
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert_eq!(source, "<p>Hello,&nbsp;</p><!--keep-->");

    document.insert(8, "world!").unwrap();
    assert_eq!(document.source_bytes(), b"<p>Hello, world!</p><!--keep-->");
    assert_reopens(&document, "Hello, world!");
}

#[test]
fn interior_space_insertions_and_replacements_use_literal_text() {
    let mut insertion = html("<p>Hello,world!</p><!--keep-->");
    insertion.insert(6, " ").unwrap();
    assert_eq!(insertion.source_bytes(), b"<p>Hello, world!</p><!--keep-->");
    assert_reopens(&insertion, "Hello, world!");

    let mut replacement = html("<p>Hello,world!</p><!--keep-->");
    replacement.replace(6..11, " ").unwrap();
    assert_eq!(replacement.source_bytes(), b"<p>Hello, !</p><!--keep-->");
    assert_reopens(&replacement, "Hello, !");
}

#[test]
fn reopening_a_generated_nbsp_makes_its_existing_spelling_authoritative() {
    let mut generated = html("<p>Hello,</p><!--keep-->");
    generated.insert(6, " ").unwrap();
    let original = String::from_utf8(generated.source_bytes()).unwrap();
    let mut reopened = html(&original);
    reopened.insert(8, "world!").unwrap();
    assert_eq!(
        reopened.source_bytes(),
        original.replace("</p>", "world!</p>").as_bytes()
    );
    assert_reopens(&reopened, "Hello,\u{a0}world!");
}

#[test]
fn deleting_beside_collapsed_source_whitespace_uses_nbsp_in_each_encoding() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (body, expected) in [
            ("<p>A \t\n B</p>", "<p>A&nbsp;</p>"),
            (
                "<p>A<span data-keep='x'> \t\n </span>B</p>",
                "<p>A<span data-keep='x'>&nbsp;</span></p>",
            ),
        ] {
            let original = format!("<!--café-->{body}<!--tail-->");
            let before = encoded(encoding, &original);
            let mut document =
                Document::from_bytes(before.clone(), encoding, Format::Html).unwrap();
            assert_eq!(document.text(), "A B");
            document.replace(2..3, "").unwrap();
            assert_reopens(&document, "A\u{a0}");
            let after = encoded(encoding, &format!("<!--café-->{expected}<!--tail-->"));
            assert_eq!(document.source_bytes(), after);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), before);
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}
