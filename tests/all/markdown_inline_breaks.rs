use viem_core::document::{BoundaryAffinity, Encoding, Format, ModelRequest};
use viem_core::Document;

#[test]
fn trailing_break_spelling_is_only_consumed_inside_prose() {
    for (source, expected) in [
        ("foo\\\n", "foo\\"),
        ("### foo\\\n", "foo\\"),
        ("Foo\\\n----\n", "Foo\\"),
        ("- a\\\n- b\n", "a\\\nb"),
        ("a\\\n> b\n", "a\\\nb"),
        ("path C:\\\\\nnext\n", "path C:\\ next"),
        ("    foo  \n", "foo  "),
        ("    a\\\n", "a\\"),
        ("para\n\n    code\\\n", "para\ncode\\"),
    ] {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(doc.text(), expected, "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn inline_scopes_cross_physical_hard_breaks() {
    use viem_core::document::{SemanticInlineStyle, StyleApplication};
    for (source, expected, style) in [
        ("**bold across  \nbreak**", "bold across\nbreak", SemanticInlineStyle::Strong),
        ("*foo\\\nbar*", "foo\nbar", SemanticInlineStyle::Emphasis),
        ("`code  \nspan`", "code   span", SemanticInlineStyle::Code),
        ("``\nfoo\nbar  \nbaz\n``", "foo bar   baz", SemanticInlineStyle::Code),
    ] {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(doc.text(), expected, "{source:?}");
        assert!(doc.projection().style_spans().iter().any(|span|
            span.application == StyleApplication::Semantic(style) && span.range == (0..expected.len())), "{source:?}");
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    let doc = Document::from_bytes(b"[link  \ntext](u)".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(doc.text(), "link\ntext");
    assert_eq!(doc.link_at(doc.text_point(6).unwrap()).unwrap().as_deref(), Some("u"));
}

#[test]
fn typing_across_a_hard_break_reopens_and_restores_exact_bytes() {
    use viem_core::command::{CommandStatus, InputEvent, Key};
    use viem_core::document::HistoryNavigationRequest;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for source in ["**ab  \ncd**", "    ab\\\n"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut expected = document.text().to_owned();
        expected.insert(1, 'X');
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: 1,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in [Key::Char('i'), Key::Char('X'), Key::Escape] {
            let outcome = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            if let Some(command) = outcome.command {
                assert!(matches!(command.status, CommandStatus::Complete | CommandStatus::Pending), "{source:?}: {:?}", command.status);
            }
        }
        assert_eq!(core.document().text(), expected);
        let saved = core.document().source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), expected);
        for at in expected.char_indices().map(|(at, _)| at) {
            assert_eq!(DocumentLayoutStyles::semantic_character_at(core.document().projection(), at, false),
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), at, false));
        }
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}

#[test]
fn native_inline_breaks_preserve_paragraph_ownership_and_literal_contexts() {
    for (source, text, style) in [
        ("# a<br>b", "a\nb", "Heading1"),
        ("a<BR/>b", "a\nb", "Paragraph"),
        ("> a<br />b", "a\nb", "Paragraph"),
        ("- a<br\t/>b", "a\nb", "BulletedList1"),
        ("**a<br>b**", "a\nb", "Paragraph"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(document.text(), text, "{source}");
        assert_eq!(document.projection().blocks().len(), 1);
        assert_eq!(document.projection().blocks()[0].style.0, style);
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.source_bytes(), source.as_bytes());
        let source_view = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        assert_eq!(source_view.text(), source);
        assert_eq!(source_view.line_count(), 1);
    }
    let source = "\\<br> `<br>` <bracket> <br data-x='keep'>";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(document.text(), "<br> <br> <bracket> \n");
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn heading_break_is_a_local_encoded_insertion_and_deleting_it_restores_exact_source() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let source = "# αβ\r\n\r\nUntouched **tail**\r\n";
        let mut bytes = match encoding {
            Encoding::Utf8 => vec![0xef, 0xbb, 0xbf],
            Encoding::Utf16Le => vec![0xff, 0xfe],
            Encoding::Utf16Be => vec![0xfe, 0xff],
            _ => unreachable!(),
        };
        bytes.extend(match encoding {
            Encoding::Utf8 => source.as_bytes().to_vec(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        });
        let mut document = Document::from_bytes(bytes.clone(), encoding, Format::Markdown).unwrap();
        let before = document.text().to_owned();
        let at = "α".len();
        let prepared = document
            .prepare_model_request(ModelRequest::InsertHardBreak {
                document: document.id(),
                revision: document.revision(),
                at,
                affinity: BoundaryAffinity::Downstream,
            })
            .unwrap();
        assert_eq!(prepared.summary().source_patches().len(), 1);
        let patch = &prepared.summary().source_patches()[0];
        assert!(patch.range().is_empty());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(
            document.text(),
            format!("{}\n{}", &before[..at], &before[at..])
        );
        assert_eq!(document.projection().blocks()[0].style.0, "Heading1");
        let reopened =
            Document::from_bytes(document.source_bytes(), encoding, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        document.delete(at..at + 1).unwrap();
        assert_eq!(document.source_bytes(), bytes);
    }
}
