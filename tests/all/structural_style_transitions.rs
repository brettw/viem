use viem_core::command::{InputEvent, Key, Mode};
use viem_core::document::{
    BoundaryAffinity, Encoding, Format, ListStyle, ModelRequest, StyleNamespace,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document};

fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn list_and_quote_styles_compose_and_leave_typing_usable() {
    for (format, source) in [
        (Format::Html, "<!--keep--><ul data-x='list'><li>before</li><li><b>é العربية</b></li><li>after</li></ul><!--end-->"),
        (Format::Markdown, "- before\n- **é العربية**\n- after"),
    ] {
        let mut core = Core::new(open(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 400.);
        let at = core.document().text().find('é').unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::AssignNamedStyle {
            expected: core.list_selection_identity(view).unwrap(),
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Block, style: "Block quote".into(),
        }).unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        let quote_source = core.document().source_bytes();
        let block = core.document().projection().blocks().iter().find(|block| block.range.contains(&at)).unwrap();
        assert_eq!(block.quote_depth, 1);
        assert!(core.document().projection().list_structure().lists.iter().any(|list|
            list.items.iter().any(|item| item.paragraph_ids.contains(&block.id))));
        core.handle(view, CoreEvent::Input(InputEvent::text("X"))).unwrap();
        let cursor = core.command_state(view).unwrap().cursor();
        core.document().text_point(cursor).unwrap();
        assert!(core.document().text().contains("Xé العربية"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        let typed = core.document().source_bytes();
        let reopened = Document::from_bytes(typed.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert_eq!(reopened.projection().hard_line_count(), core.document().projection().hard_line_count());
        core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
        assert_eq!(core.document().source_bytes(), quote_source);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
        assert_eq!(core.document().source_bytes(), typed);
        core.handle(view, CoreEvent::SetListStyle {
            expected: core.list_selection_identity(view).unwrap(), style: Some(ListStyle::Numbered),
        }).unwrap_or_else(|error| panic!("reverse {format:?}: {error:?}"));
        let after = core.document().source_bytes();
        assert!(core.document().projection().blocks().iter().any(|block| block.quote_depth == 1));
        assert_eq!(open(std::str::from_utf8(&after).unwrap(), format).text(), core.document().text());
        core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
        assert_eq!(core.document().source_bytes(), typed);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
        assert_eq!(core.document().source_bytes(), after);
    }
}

#[test]
fn selecting_middle_list_items_changes_only_their_structural_syntax() {
    for (format, source) in [
        (Format::Html, "<!--start--><ul id='keep'><li data-x='a'>before</li><li><b>middle</b></li><li><em>second</em></li><li data-x='z'>after</li></ul><!--end-->"),
        (Format::Markdown, "- before\n- **middle**\n- *second*\n- after"),
    ] {
        let mut doc = open(source, format);
        let text = doc.text().to_owned();
        let start = text.find("middle").unwrap();
        let end = text.find("after").unwrap() - 1;
        let prepared = doc.prepare_model_request(ModelRequest::SetParagraphStyle {
            document: doc.id(), revision: doc.revision(), range: start..end,
            style: "Block quote".into(),
        }).unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        let first = prepared.summary().source_patches().first().unwrap().range().start;
        let last = prepared.summary().source_patches().last().unwrap().range().end;
        assert!(first > source.find("before").unwrap());
        assert!(last < source.find("after").unwrap());
        doc.commit_model_transaction(prepared).unwrap();
        assert_eq!(doc.text(), text);
        let saved = doc.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), text);
        assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo()); assert_eq!(doc.source_bytes(), saved);
    }
}

#[test]
fn edge_list_items_do_not_leave_empty_list_containers() {
    for (source, needle) in [
        (
            "<ul data-keep='list'><li data-keep='item'>one <b>two</b></li><li>other</li></ul>",
            "one",
        ),
        (
            "<ol start='3'><li>other</li><li data-keep='item'>one <b>two</b></li></ol>",
            "one",
        ),
        ("<ul><li></li><li>other</li></ul>", ""),
    ] {
        let mut doc = open(source, Format::Html);
        let before = doc.text().to_owned();
        let at = before.find(needle).unwrap();
        doc.set_paragraph_style(at..at, "Block quote".into())
            .unwrap();
        assert_eq!(doc.text(), before);
        let saved = doc.source_bytes();
        let source_after = std::str::from_utf8(&saved).unwrap();
        assert!(!source_after.contains("<ul data-keep='list'></ul>"));
        assert!(!source_after.contains("<ol start='3'></ol>"));
        assert_eq!(open(source_after, Format::Html).text(), before);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), saved);
    }
}

#[test]
fn heading_and_code_to_quote_to_heading_preserve_text_and_allow_typing() {
    for (format, source, needle) in [
        (
            Format::Html,
            "<h2 data-keep='heading'>one <b>two</b></h2><p>tail</p>",
            "one",
        ),
        (
            Format::Html,
            "<pre data-keep='code'>one\n # two</pre><p>tail</p>",
            "one",
        ),
        (Format::Markdown, "# one **two**\n\ntail", "one"),
        (
            Format::Markdown,
            "```language\none\n# two\n```\n\ntail",
            "one",
        ),
    ] {
        let mut doc = open(source, format);
        let text = doc.text().to_owned();
        let at = text.find(needle).unwrap();
        let original_style = doc.projection().blocks()[0].style.clone();
        doc.set_paragraph_style(at..at, "Block quote".into())
            .unwrap_or_else(|error| panic!("quote {format:?} {source:?}: {error:?}"));
        assert_eq!(doc.text(), text);
        assert_eq!(doc.projection().blocks()[0].style, original_style);
        assert_eq!(doc.projection().blocks()[0].quote_depth, 1);
        let quote_source = doc.source_bytes();
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), quote_source);
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 400.);
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
        core.document()
            .text_point(core.command_state(view).unwrap().cursor())
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        let typed = core.document().source_bytes();
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected: core.list_selection_identity(view).unwrap(),
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Block,
                style: "Heading2".into(),
            },
        )
        .unwrap_or_else(|error| panic!("heading {format:?} {source:?}: {error:?}"));
        let final_source = core.document().source_bytes();
        let block = &core.document().projection().blocks()[0];
        assert_eq!(block.style.0, "Heading2");
        assert_eq!(block.quote_depth, 1);
        let reopened = Document::from_bytes(final_source.clone(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        assert_eq!(
            reopened.projection().hard_line_count(),
            core.document().projection().hard_line_count()
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), typed);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), final_source);
    }
}

#[test]
fn explicitly_reassigning_imported_nested_quote_retains_inner_styles() {
    for (format, source) in [
        (
            Format::Html,
            "<ul><li><blockquote><h2>one</h2></blockquote></li><li>tail</li></ul>",
        ),
        (Format::Markdown, "> - one\n\ntail"),
    ] {
        let mut doc = open(source, format);
        assert_eq!(doc.source_bytes(), source.as_bytes());
        let text = doc.text().to_owned();
        let original_style = doc.projection().blocks()[0].style.clone();
        let original_kind = doc.projection().blocks()[0].kind.clone();
        doc.set_paragraph_style(0..0, "Block quote".into())
            .unwrap_or_else(|error| panic!("{format:?} {error:?}"));
        assert_eq!(doc.text(), text);
        let first = &doc.projection().blocks()[0];
        assert_eq!(first.style, original_style);
        assert_eq!(first.kind, original_kind);
        assert!(first.quote_depth > 0);
        assert!(doc.projection().list_structure().lists.iter().any(|list|
            list.items.iter().any(|item| item.paragraph_ids.contains(&first.id))));
        let after = doc.source_bytes();
        assert_eq!(
            Document::from_bytes(after.clone(), Encoding::Utf8, format)
                .unwrap()
                .text(),
            text
        );
        if after != source.as_bytes() {
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), after);
        }
    }
}

#[test]
fn mixed_selection_preserves_existing_quote_attributes() {
    let source = "<blockquote cite='keep' class='custom'><p>quoted</p></blockquote><ul><li>item</li><li>tail</li></ul>";
    let mut doc = open(source, Format::Html);
    let before = doc.text().to_owned();
    let end = before.find("tail").unwrap() - 1;
    doc.set_paragraph_style(0..end, "Block quote".into())
        .unwrap();
    assert_eq!(doc.text(), before);
    let after = doc.source_bytes();
    assert!(after.starts_with(b"<blockquote cite='keep' class='custom'><p>quoted</p></blockquote>"));
    assert_eq!(
        Document::from_bytes(after.clone(), Encoding::Utf8, Format::Html)
            .unwrap()
            .text(),
        before
    );
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), source.as_bytes());
    assert!(doc.redo());
    assert_eq!(doc.source_bytes(), after);
}
