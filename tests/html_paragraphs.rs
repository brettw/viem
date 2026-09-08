use viem_core::command::{CommandInterpreter, InputEvent, Key};
use viem_core::document::*;

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}
fn enter(document: &mut Document, at: usize) {
    document
        .apply_model_request(ModelRequest::ContinueList {
            document: document.id(),
            revision: document.revision(),
            at,
        })
        .unwrap();
}
fn direct(document: &Document, at: usize) -> CharacterProperties {
    let mut result = CharacterProperties::default();
    for span in document
        .projection()
        .style_spans()
        .iter()
        .filter(|span| span.range.start <= at && at < span.range.end)
    {
        if let StyleApplication::Direct(properties) = &span.application {
            if properties.bold.is_some() {
                result.bold = properties.bold;
            }
            if properties.weight.is_some() {
                result.weight = properties.weight;
            }
            if properties.slant.is_some() {
                result.slant = properties.slant;
            }
            if properties.size.is_some() {
                result.size = properties.size;
            }
        }
    }
    result
}
#[test]
fn middle_split_preserves_named_style_direct_paragraph_and_inline_scopes() {
    let source="<h2 data-id='keep' style='margin-inline-start:12pt;font-size:19pt'><b data-x='original'>abcd</b><i>tail</i></h2><!--outside--><p>last</p>";
    let mut document = html(source);
    enter(&mut document, 2);
    assert_eq!(document.text(), "ab\ncdtail\nlast");
    let paragraphs = document.projection().blocks();
    assert_eq!(paragraphs[0].style, StyleId::from("Heading2"));
    assert_eq!(paragraphs[1].style, StyleId::from("Heading2"));
    assert_eq!(
        paragraphs[0].direct_paragraph,
        paragraphs[1].direct_paragraph
    );
    assert_eq!(direct(&document, 0).bold, Some(true));
    assert_eq!(direct(&document, 3).bold, Some(true));
    assert_eq!(direct(&document, 5).weight, None);
    assert_eq!(direct(&document, 5).slant, Some(FontSlant::Italic));
    assert_eq!(direct(&document, 5).size, Some(19.0));
    let bytes = document.source_bytes();
    let changed = String::from_utf8(bytes.clone()).unwrap();
    assert!(changed.starts_with("<h2 data-id='keep' style='margin-inline-start:12pt;font-size:19pt'><b data-x='original'>ab</b></h2>"));
    assert!(changed.ends_with("cd</b><i>tail</i></h2><!--outside--><p>last</p>"));
    let reopened = Document::from_bytes(bytes, Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
    assert_eq!(
        reopened.projection().style_spans(),
        document.projection().style_spans()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn terminal_split_uses_next_style_and_middle_hard_line_keeps_current_style() {
    for (source, at, expected) in [
        ("<h2><b>Word</b></h2><p>tail</p>", 4, "Word\n\ntail"),
        ("<h2>Word</h2>", 4, "Word\n"),
        ("<h2>Word", 4, "Word\n"),
    ] {
        let mut document = html(source);
        enter(&mut document, at);
        assert_eq!(document.text(), expected);
        assert_eq!(
            document.projection().blocks()[1].style,
            StyleId::from("Paragraph")
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
    let mut document = html("<h2>one<br>two</h2>");
    enter(&mut document, 3);
    assert_eq!(document.text(), "one\n\ntwo");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Heading2")
    );
}
#[test]
fn anonymous_and_empty_paragraph_splits_keep_containers_and_opaque_source() {
    for (source, at, expected) in [
        ("", 0, "\n"),
        ("<p></p>", 0, "\n"),
        ("<h2></h2>", 0, "\n"),
        ("Words<!--tail-->", 2, "Wo\nrds"),
        (
            "<div><b data-x='keep'>Words</b><!--tail--></div>",
            2,
            "Wo\nrds",
        ),
        ("Words", 0, "\nWords"),
        ("Words", 5, "Words\n"),
    ] {
        let mut document = html(source);
        enter(&mut document, at);
        assert_eq!(document.text(), expected, "{source}");
        assert_eq!(document.projection().blocks().len(), 2, "{source}");
        let changed = String::from_utf8(document.source_bytes()).unwrap();
        if source.contains("<!--tail-->") {
            assert!(changed.contains("</p><!--tail-->"));
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn enter_typing_is_one_insert_undo_and_dot_replays_paragraph_split() {
    let mut document = html("<h2>Title</h2>");
    let original = document.source_bytes();
    let mut commands = CommandInterpreter::new();
    for event in [
        InputEvent::key('A'),
        InputEvent::Key(Key::Enter),
        InputEvent::text("body"),
        InputEvent::Key(Key::Escape),
    ] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "Title\nbody");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Paragraph")
    );
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), original);
    commands
        .handle(&mut document, InputEvent::key('.'))
        .unwrap();
    assert_eq!(document.text(), "Title\nbody");
}

#[test]
fn source_named_next_style_survives_split_reopen_and_redo() {
    let mut document = html("<p>Title</p>");
    for (id, next) in [("Body", None), ("Title", Some("Body"))] {
        document
            .apply_model_request(ModelRequest::EditNamedStyleDefinition {
                document: document.id(),
                revision: document.revision(),
                edit: StyleDefinitionEdit::InsertBlock {
                    style: BlockStyle {
                        id: id.into(),
                        based_on: Some("Paragraph".into()),
                        next_paragraph_style: next.map(Into::into),
                        role: BlockRole::Paragraph,
                        character: Default::default(),
                        block: Default::default(),
                    },
                    metadata: StyleDefinitionMetadata {
                        display_name: id.into(),
                        origin: StyleDefinitionOrigin::SourceBacked,
                    },
                },
            })
            .unwrap();
    }
    document
        .apply_model_request(ModelRequest::AssignNamedStyle {
            document: document.id(),
            revision: document.revision(),
            range: 0..0,
            namespace: StyleNamespace::Block,
            style: "Title".into(),
        })
        .unwrap();
    let original = document.source_bytes();
    enter(&mut document, 5);
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Body")
    );
    let changed = document.source_bytes();
    let reopened = Document::from_bytes(changed.clone(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(
        reopened.projection().blocks()[1].style,
        StyleId::from("Body")
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), changed);
}

#[test]
fn large_document_split_preserves_unaffected_identities_and_cached_shaping() {
    use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
    let source = (0..10_000)
        .map(|index| format!("<p>Paragraph {index:05}</p>\n"))
        .collect::<String>();
    let mut document = html(&source);
    let before = document.projection().blocks()[9000].id;
    let at = document.projection().blocks()[5000].range.start + 5;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    let mut view = ViewLayout::new(600.0, 300.0);
    engine.set_cache_capacity(10_010);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    enter(&mut document, at);
    assert_eq!(document.projection().blocks()[9001].id, before);
    engine.relayout(&document, &mut view).unwrap();
    assert!(
        engine.provider().request_calls() - shaped <= 2,
        "unaffected paragraph shaping must remain cached: {} new requests",
        engine.provider().request_calls() - shaped
    );
    assert_eq!(view.snapshot().unwrap().rows.len(), 10_001);
    assert_eq!(
        view.snapshot().unwrap().document_revision,
        document.revision()
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn whole_paragraph_delete_preserves_neighbor_source_and_undo() {
    for (source, commands, expected) in [
        (
            "<p data-x='old'>one</p><!--keep--><h2>two</h2>",
            "dd",
            "two",
        ),
        ("<p><b>one</b></p><h2>two</h2>", "dd", "two"),
        ("<p>one</p><h2>two</h2>", "jdd", "one"),
    ] {
        let mut document = html(source);
        let mut interpreter = CommandInterpreter::new();
        for c in commands.chars() {
            interpreter
                .handle(&mut document, InputEvent::key(c))
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        }
        assert_eq!(document.text(), expected, "{source}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn whole_list_item_delete_consumes_structure_and_preserves_following_ordinals() {
    for (source, commands, expected) in [
        (
            "<ul><li data-x='old'><b>one</b><!--keep--></li><li>two</li></ul>",
            "dd",
            "two",
        ),
        (
            "<ol start='3'><li>one</li><li data-x='keep'>two</li><li>three</li></ol>",
            "dd",
            "two\nthree",
        ),
        (
            "<ol><li>one</li><li>two</li><li>three</li></ol>",
            "jdd",
            "one\nthree",
        ),
        ("<ol><li>one</li><li>two</li></ol>", "jdd", "one"),
        ("<ul><li>one</li></ul>", "dd", ""),
        (
            "<ul><li><p>one</p><p>more</p></li><li>two</li></ul>",
            "2dd",
            "two",
        ),
    ] {
        let mut document = html(source);
        let mut interpreter = CommandInterpreter::new();
        for c in commands.chars() {
            interpreter
                .handle(&mut document, InputEvent::key(c))
                .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        }
        assert_eq!(document.text(), expected, "{source}");
        let changed = String::from_utf8(document.source_bytes()).unwrap();
        if source.contains("<!--keep-->") {
            assert!(changed.contains("<!--keep-->"));
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn paragraph_join_and_backspace_keep_first_style_and_second_direct_characters() {
    let source="<h2 style='margin-inline-start:12pt'><b>one</b></h2><!--keep--><p style='font-size:17pt;margin-inline-start:33pt'><i>two</i></p><p>last</p>";
    let mut document = html(source);
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('J'))
        .unwrap();
    assert_eq!(document.text(), "one two\nlast");
    assert_eq!(document.projection().blocks().len(), 2);
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Heading2")
    );
    assert_eq!(
        document.projection().blocks()[0]
            .direct_paragraph
            .leading_indent,
        Some(12.0)
    );
    assert_eq!(direct(&document, 4).size, Some(17.0));
    assert_eq!(direct(&document, 4).slant, Some(FontSlant::Italic));
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("<!--keep-->"));
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), source.as_bytes());
    for event in [
        InputEvent::key('j'),
        InputEvent::key('0'),
        InputEvent::key('i'),
        InputEvent::Key(Key::Backspace),
        InputEvent::Key(Key::Escape),
    ] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "onetwo\nlast");
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("Heading2")
    );
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), source.as_bytes());
    for source in ["<h2>one<p>two", "<h2>one</h2><p>two</p>"] {
        let mut document = html(source);
        document.replace(1..6, "X").unwrap_or_else(|error| {
            panic!(
                "{source}: {error:?} {:?}",
                document.projection().provenance()
            )
        });
        assert_eq!(document.text(), "oXo");
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Heading2")
        );
    }
}

#[test]
fn counted_and_visual_joins_keep_first_assignment_in_one_undo() {
    for keys in ["3J", "VjjJ", "3gJ"] {
        let source = "<h2>one</h2><p style='font-size:17pt'>two</p><h3>three</h3>";
        let mut document = html(source);
        let mut commands = CommandInterpreter::new();
        for key in keys.chars() {
            commands
                .handle(&mut document, InputEvent::key(key))
                .unwrap();
        }
        assert_eq!(
            document.text(),
            if keys == "3gJ" {
                "onetwothree"
            } else {
                "one two three"
            }
        );
        assert_eq!(
            document.projection().blocks()[0].style,
            StyleId::from("Heading2")
        );
        assert_eq!(document.projection().blocks().len(), 1);
        commands
            .handle(&mut document, InputEvent::key('u'))
            .unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        commands
            .handle(&mut document, InputEvent::key('g'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('g'))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('.'))
            .unwrap();
        assert_eq!(document.projection().blocks().len(), 1);
    }
}
