use evim_core::command::{InputEvent, Key};
use evim_core::document::{
    BlockKind, Document, Encoding, FileFormat, Format, ModelRequest, ProjectionWorkScope, TextEdit,
};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent};

fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    }
}
fn open(source: &str) -> Document {
    Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap()
}
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: evim_core::ViewId, text: &str) {
    for ch in text.chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(ch)))
            .unwrap();
    }
}

#[test]
fn source_blank_separator_lines_share_semantic_boundaries_but_code_stays_literal() {
    for (source, expected) in [
        (
            "# Heading\n\nFirst **body**\ncontinuation.\n\nLast.",
            "# Heading\nFirst **body**\ncontinuation.\nLast.",
        ),
        ("First\n\n\n\nLast", "First\n\nLast"),
        ("First\n  \nLast", "First\nLast"),
        ("First\n\n", "First\n"),
        (
            "```\nfirst\n\nsecond\n```\n\nTail",
            "```\nfirst\n\nsecond\n```\nTail",
        ),
        (
            "- Item\n\n  ```\n  code\n\n  more\n  ```\n\nTail",
            "- Item\n  ```\n  code\n\n  more\n  ```\nTail",
        ),
    ] {
        let document = open(source);
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn source_empty_list_exit_then_typing_makes_a_plain_paragraph_in_both_views() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for marker in ["3. ", "- ", "+ ", "  3) "] {
            let source = format!("1. First\r\n{marker}");
            let original = encode(&source, encoding);
            let document =
                Document::from_bytes(original.clone(), encoding, Format::MarkdownSource).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 250.0);
            keys(&mut core, view, "GA");
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap();
            assert_eq!(core.document().text(), "1. First\n");
            core.handle(view, CoreEvent::Input(InputEvent::text("Plain")))
                .unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            assert_eq!(
                core.document().source_bytes(),
                encode("1. First\r\n\r\nPlain", encoding)
            );
            assert_eq!(
                core.document().projection().blocks().last().unwrap().kind,
                BlockKind::Paragraph
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), original);
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                .unwrap();
            core.handle(
                view,
                CoreEvent::SetFormat {
                    document: core.document().id(),
                    revision: core.document().revision(),
                    target: Format::Markdown,
                },
            )
            .unwrap();
            assert_eq!(core.document().text(), "First\nPlain");
            assert_eq!(
                core.document().projection().blocks().last().unwrap().kind,
                BlockKind::Paragraph
            );
        }
    }
}

#[test]
fn edits_and_formatting_after_mapped_separator_preserve_exact_source() {
    let source = "First\r\n \t\r\nSecond\r\n\r\nTail";
    let mut document = open(source);
    assert_eq!(document.text(), "First\nSecond\nTail");
    document.insert(6, "New ").unwrap();
    assert_eq!(
        document.source_bytes(),
        b"First\r\n \t\r\nNew Second\r\n\r\nTail"
    );
    document
        .set_paragraph_style(6..6, "Heading2".into())
        .unwrap();
    assert_eq!(
        document.source_bytes(),
        b"First\r\n \t\r\n## New Second\r\n\r\nTail"
    );
    document
        .set_list_style(6..6, Some(evim_core::document::ListStyle::Bullet))
        .unwrap();
    assert_eq!(
        document.source_bytes(),
        b"First\r\n \t\r\n- New Second\r\n\r\nTail"
    );
    assert!(document.undo());
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    document.replace(5..6, "").unwrap();
    assert_eq!(document.text(), "FirstSecond\nTail");
    assert_eq!(document.source_bytes(), b"FirstSecond\r\n\r\nTail");
    assert!(document.undo());
    document.set_file_format(FileFormat::Unix).unwrap();
    assert_eq!(document.text(), "First\nSecond\nTail");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn source_flow_toggle_joins_only_original_ordinary_source_endings() {
    let source = "First\ncontinuation\n\nSecond\ncontinuation";
    let mut core = Core::new(open(source));
    let first = core.add_view(MockTextMeasurementProvider::new(), 900.0, 400.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 900.0, 400.0);
    let revision = core.document().revision();
    assert_eq!(
        core.layout(first).unwrap().snapshot().unwrap().rows.len(),
        4
    );
    core.handle(first, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    assert_eq!(
        core.layout(first).unwrap().snapshot().unwrap().rows.len(),
        2
    );
    assert_eq!(
        core.layout(second).unwrap().snapshot().unwrap().rows.len(),
        4
    );
    assert_eq!(
        core.document().text(),
        "First\ncontinuation\nSecond\ncontinuation"
    );
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(first, CoreEvent::SetParagraphFlow(false))
        .unwrap();
    assert_eq!(
        core.layout(first).unwrap().snapshot().unwrap().rows.len(),
        4
    );
}

#[test]
fn late_source_paragraph_edit_keeps_regional_work_after_many_hidden_separators() {
    let source = (0..10_000)
        .map(|n| format!("Paragraph {n}\ncontinuation\n\n"))
        .collect::<String>();
    let mut document = open(&source);
    let at = document.text().find("Paragraph 9000").unwrap();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at + 9, "Section")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.projected_hard_lines() <= 3);
    assert!(work.decoded_source_bytes() < 200);
    document.commit_model_transaction(prepared).unwrap();
    let fresh = Document::from_bytes(
        document.source_bytes(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(
        document.projection().presentation_line_count(true),
        fresh.projection().presentation_line_count(true)
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn source_empty_enter_promotes_separator_then_keeps_intentional_empty_paragraphs_editable() {
    let source = "Word";
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 250.0, 250.0);
    keys(&mut core, view, "A");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"Word\n");
    assert_eq!(core.document().text(), "Word\n");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"Word\n\n");
    assert_eq!(core.document().text(), "Word\n");
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"Word\n\n\n\n");
    assert_eq!(core.document().text(), "Word\n\n");
    core.handle(view, CoreEvent::Input(InputEvent::text("Tail")))
        .unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    assert_eq!(core.document().text(), "Word\n\nTail");
    keys(&mut core, view, "u");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn source_line_undo_restores_only_its_line_in_a_grouped_paragraph() {
    let source = "First\nsecond\n\nTail";
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 250.0);
    assert_eq!(core.document().projection().blocks().len(), 2);
    assert_eq!(core.document().projection().hard_line_count(), 3);
    keys(&mut core, view, "jxU");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
