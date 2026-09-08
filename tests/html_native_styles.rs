use evim_core::command::InputEvent;
use evim_core::document::*;
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}
fn assign(
    document: &mut Document,
    range: std::ops::Range<usize>,
    id: &str,
) -> CommittedModelTransaction {
    let source = String::from_utf8_lossy(&document.source_bytes()).into_owned();
    document
        .apply_model_request(ModelRequest::SetParagraphStyle {
            document: document.id(),
            revision: document.revision(),
            range,
            style: id.into(),
        })
        .unwrap_or_else(|error| panic!("{id} in {source:?}: {error:?}"))
}
fn assert_reopen(document: &Document) {
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
    let blocks = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| {
                let mut block = block.clone();
                block.id = 0;
                block
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(&reopened), blocks(document));
}
#[test]
fn blank_paragraph_accepts_native_list_assignment_then_typing() {
    for (source, styled, typed) in [
        ("", "<ul><li></li></ul>", "<ul><li>a</li></ul>"),
        ("<p></p>", "<ul><li></li></ul>", "<ul><li>a</li></ul>"),
        (
            "<p data-author='keep'></p><!--tail-->",
            "<ul><li data-author='keep'></li></ul><!--tail-->",
            "<ul><li data-author='keep'>a</li></ul><!--tail-->",
        ),
        (
            "<body><p></p></body>",
            "<body><ul><li></li></ul></body>",
            "<body><ul><li>a</li></ul></body>",
        ),
    ] {
        let mut document = html(source);
        assign(&mut document, 0..0, "BulletedList1");
        assert_eq!(document.source_bytes(), styled.as_bytes(), "{source}");
        assert_eq!(document.text(), "");
        assert!(matches!(
            document.projection().blocks()[0].kind,
            BlockKind::ListItem {
                ordered: false,
                level: 0,
                ..
            }
        ));
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("BulletedList1")
        );
        assert_reopen(&document);
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
        for event in [InputEvent::key('i'), InputEvent::text("a")] {
            core.handle(view, CoreEvent::Input(event)).unwrap();
        }
        assert_eq!(core.document().text(), "a");
        assert_eq!(core.document().source_bytes(), typed.as_bytes());
        assert_reopen(core.document());
        for (navigation, expected) in [
            (HistoryNavigationRequest::Undo, styled),
            (HistoryNavigationRequest::Undo, source),
            (HistoryNavigationRequest::Redo, styled),
            (HistoryNavigationRequest::Redo, typed),
        ] {
            core.handle(view, CoreEvent::NavigateHistory(navigation))
                .unwrap();
            assert_eq!(core.document().source_bytes(), expected.as_bytes());
        }
    }
}
#[test]
fn neighboring_paragraphs_share_one_native_list_and_retain_author_bytes() {
    let source = "<!--head--><P data-one='A'>one</P>\n<!--gap--><p data-two='B'><b>two</b></p><p>tail</p><!--tail-->";
    let mut document = html(source);
    let prepared = document
        .prepare_model_request(ModelRequest::SetParagraphStyle {
            document: document.id(),
            revision: document.revision(),
            range: 0..7,
            style: "BulletedList1".into(),
        })
        .unwrap();
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 4);
    for patch in patches {
        let old = &source[patch.range()];
        assert!(
            old.starts_with("<P") || old.starts_with("<p") || old == "</P>" || old == "</p>",
            "{old}"
        );
    }
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), b"<!--head--><ul><li data-one='A'>one</li>\n<!--gap--><li data-two='B'><b>two</b></li></ul><p>tail</p><!--tail-->");
    assert_eq!(document.text(), "one\ntwo\ntail");
    assert_reopen(&document);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_reopen(&document);
}
#[test]
fn native_list_assignment_retains_existing_numbering_and_nested_structure() {
    for (source, offset, style) in [
        (
            "<ol start='9'><li data-keep='x'>one</li><li>two</li></ol>",
            0,
            "NumberedList1",
        ),
        (
            "<ul><li>one<ul><li data-keep='x'>child</li></ul></li></ul>",
            4,
            "BulletedList2",
        ),
    ] {
        let mut document = html(source);
        assign(&mut document, offset..offset, style);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_reopen(&document);
    }
}
#[test]
fn basic_paragraph_and_code_styles_use_native_elements() {
    for (style, expected) in [
        ("Heading1", "<h1 data-keep='x'>word</h1>"),
        ("Code Block", "<pre data-keep='x'>word</pre>"),
    ] {
        let mut document = html("<p data-keep='x'>word</p>");
        assign(&mut document, 0..0, style);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_reopen(&document);
        assign(&mut document, 0..0, "Paragraph");
        assert_eq!(document.source_bytes(), b"<p data-keep='x'>word</p>");
        assert_reopen(&document);
    }
}

#[test]
fn list_to_paragraph_and_numbered_list_use_only_required_markup() {
    let mut document = html("<p data-keep='x'>one</p><p>two</p>");
    assign(&mut document, 0..7, "BulletedList1");
    document
        .set_list_style(0..7, Some(ListStyle::Numbered))
        .unwrap();
    assert_eq!(
        document.source_bytes(),
        b"<ol><li data-keep='x'>one</li><li>two</li></ol>"
    );
    assert_reopen(&document);
    assign(&mut document, 0..7, "Paragraph");
    assert_eq!(
        document.source_bytes(),
        b"<p data-keep='x'>one</p><p>two</p>"
    );
    assert_reopen(&document);
}

#[test]
fn inline_code_uses_the_native_character_element() {
    let mut document = html("<p>one two</p>");
    document
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 4..7,
            namespace: StyleNamespace::Character,
            style: "Code".into(),
        })
        .unwrap();
    assert_eq!(document.source_bytes(), b"<p>one <code>two</code></p>");
    assert_reopen(&document);
}

#[test]
fn empty_paragraph_among_other_blocks_has_its_own_source_anchor() {
    for (source, offset, expected) in [
        (
            "<p>one</p><p></p><p>two</p>",
            4,
            "<p>one</p><ul><li></li></ul><p>two</p>",
        ),
        ("<p></p><p>two</p>", 0, "<ul><li></li></ul><p>two</p>"),
        ("<p>one</p><p></p>", 4, "<p>one</p><ul><li></li></ul>"),
    ] {
        let mut document = html(source);
        assign(&mut document, offset..offset, "BulletedList1");
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_reopen(&document);
        document.insert(offset, "a").unwrap();
        assert_reopen(&document);
    }
}

#[test]
fn blank_paragraph_heading_and_code_assignments_replace_the_owning_element() {
    for (style, expected) in [("Heading2", "<h2></h2>"), ("Code Block", "<pre></pre>")] {
        let mut document = html("<p></p>");
        assign(&mut document, 0..0, style);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_reopen(&document);
        document.insert(0, "a").unwrap();
        assert_eq!(document.text(), "a");
        assert_reopen(&document);
    }
}

#[test]
fn omitted_paragraph_end_tags_become_native_list_items_without_rewriting_content() {
    let mut document = html("<p data-keep='a'>one<p data-keep='b'>two");
    assign(&mut document, 0..7, "BulletedList1");
    assert_eq!(
        document.source_bytes(),
        b"<ul><li data-keep='a'>one</li><li data-keep='b'>two</li></ul>"
    );
    assert_reopen(&document);
}

#[test]
fn deeper_list_assignment_uses_native_ancestor_items_without_phantom_paragraphs() {
    for level in 2..=3 {
        for (source, text) in [("<p>one</p>", "one"), ("<p></p>", "")] {
            let mut document = html(source);
            assign(&mut document, 0..0, &format!("BulletedList{level}"));
            let expected = format!(
                "{}{text}{}",
                "<ul><li>".repeat(level),
                "</li></ul>".repeat(level)
            );
            assert_eq!(document.source_bytes(), expected.as_bytes());
            assert_eq!(document.text(), text);
            assert_eq!(document.projection().blocks().len(), 1);
            assert_eq!(
                document.projection().blocks()[0].style,
                StyleId(format!("BulletedList{level}"))
            );
            assert_reopen(&document);
            document.insert(text.len(), "a").unwrap();
            assert_eq!(document.text(), format!("{text}a"));
            assert_reopen(&document);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), expected.as_bytes());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected.as_bytes());
            assign(&mut document, 0..text.len(), "Paragraph");
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert_reopen(&document);
        }
    }
}

#[test]
fn generated_native_code_style_edits_recompute_relative_source_formatting_without_dirtying_source()
{
    use evim_core::layout::DocumentLayoutStyles;
    for (source, character) in [
        (
            "<p><code><span style='vertical-align:super'>x</span></code></p>",
            true,
        ),
        (
            "<pre><span style='vertical-align:super'>x</span></pre>",
            false,
        ),
    ] {
        let mut document = html(source);
        assert!(!document.is_dirty());
        let before =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
        let edit = if character {
            let mut style = document
                .projection()
                .style_sheet()
                .character_style(&"Code".into())
                .unwrap()
                .clone();
            style.properties.size = Some(28.);
            StyleDefinitionEdit::UpdateCharacter(style)
        } else {
            let mut style = document
                .projection()
                .style_sheet()
                .block_style(&"Code Block".into())
                .unwrap()
                .clone();
            style.character.size = Some(28.);
            StyleDefinitionEdit::UpdateBlock(style)
        };
        let committed = document
            .apply_style_request(StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit)),
            ))
            .unwrap();
        let after =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
        assert_eq!(after.size, 28.);
        assert_eq!(after.baseline_shift, 28. / 3.);
        assert_ne!(after.baseline_shift, before.baseline_shift);
        assert!(committed.summary().source_patches().is_empty());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(!document.is_dirty());
        let mut reopened = html(source);
        reopened
            .initialize_style_defaults(&document.export_style_defaults().unwrap())
            .unwrap();
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false),
            Ok(after.clone())
        );
        assert!(document.undo());
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false),
            Ok(before)
        );
        assert!(!document.is_dirty());
        assert!(document.redo());
        assert_eq!(
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false),
            Ok(after)
        );
        assert!(!document.is_dirty());
    }
}

#[test]
fn native_source_style_intent_with_export_off_keeps_source_clean() {
    use evim_core::layout::DocumentLayoutStyles;
    let source = "<p><span style='vertical-align:super'>x</span></p>";
    let mut document = html(source);
    assert!(!document.is_dirty());
    let mut style = document
        .projection()
        .style_sheet()
        .block_style(&"Document".into())
        .unwrap()
        .clone();
    style.character.size = Some(28.);
    let committed = document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(PersistedStyleIntent::EditStyleDefinition {
                origin: StyleDefinitionOrigin::SourceBacked,
                edit: StyleDefinitionEdit::UpdateBlock(style),
            }),
        ))
        .unwrap();
    assert!(committed.summary().source_patches().is_empty());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(!document.is_dirty());
    let after =
        DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
    assert_eq!(after.size, 28.);
    assert_eq!(after.baseline_shift, 28. / 3.);
    assert!(document.undo());
    assert!(!document.is_dirty());
    assert!(document.redo());
    assert!(!document.is_dirty());
}
