use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn fixture(source: &str, at: usize, insert: bool) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
    if insert {
        input(&mut core, view, InputEvent::key('i'));
    }
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: if at == core.document().text().len() {
                BoundaryAffinity::Upstream
            } else {
                BoundaryAffinity::Downstream
            },
            extend_selection: false,
        },
    )
    .unwrap();
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let detail = format!(
        "{event:?} at {} in {:?}",
        core.command_state(view).unwrap().cursor(),
        String::from_utf8_lossy(&core.document().source_bytes())
    );
    let update = core
        .handle(view, CoreEvent::Input(event))
        .unwrap_or_else(|error| panic!("{detail}: {error:?}"));
    if let Some(command) = update.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
            ),
            "{detail}: {:?}",
            command.status
        );
    }
}

fn verify(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    original: &str,
    text: &str,
    cursor: usize,
    source: &str,
) {
    assert_eq!(core.document().text(), text);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    core.document().text_point(cursor).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!source.contains("white-space"));
    let reopened =
        Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), text);
    input(core, view, InputEvent::Key(Key::Escape));
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().text(), text);
}

#[test]
fn deleting_text_protects_exposed_spaces_and_maps_carets() {
    for (body, at, insert, event, text, cursor, expected) in [
        (
            "<p>A B</p>",
            3,
            true,
            InputEvent::Key(Key::Backspace),
            "A\u{a0}",
            3,
            "<p>A&nbsp;</p>",
        ),
        (
            "<p>A B</p>",
            0,
            false,
            InputEvent::key('x'),
            "\u{a0}B",
            0,
            "<p>&nbsp;B</p>",
        ),
        (
            "<p>A B C</p>",
            2,
            false,
            InputEvent::key('x'),
            "A \u{a0}C",
            2,
            "<p>A &nbsp;C</p>",
        ),
        (
            "<p>A B</p>",
            2,
            true,
            InputEvent::Key(Key::Delete),
            "A\u{a0}",
            3,
            "<p>A&nbsp;</p>",
        ),
    ] {
        let source = format!("<!--keep-->{body}<unknown data-x='same'></unknown>");
        let expected = format!("<!--keep-->{expected}<unknown data-x='same'></unknown>");
        let (mut core, view) = fixture(&source, at, insert);
        input(&mut core, view, event);
        verify(&mut core, view, &source, text, cursor, &expected);
    }
}

#[test]
fn paragraph_and_list_enter_use_nbsp_at_new_edges_and_keep_cursor_after_break() {
    for (open, close) in [("<p>", "</p>"), ("<ul><li>", "</li></ul>")] {
        for (at, text, cursor, left, right) in [
            (1, "A\n\u{a0}B", 2, "A", "&nbsp;B"),
            (2, "A\u{a0}\nB", 4, "A&nbsp;", "B"),
        ] {
            let source = format!("{open}A B{close}<!--tail-->");
            let expected = if open == "<p>" {
                format!("<p>{left}</p><p>{right}</p><!--tail-->")
            } else {
                format!("<ul><li>{left}</li><li>{right}</li></ul><!--tail-->")
            };
            let (mut core, view) = fixture(&source, at, true);
            input(&mut core, view, InputEvent::Key(Key::Enter));
            verify(&mut core, view, &source, text, cursor, &expected);
        }
    }
}

#[test]
fn normal_and_visual_put_protect_spaces_without_losing_the_cursor() {
    for (events, text, cursor, source) in [
        ("yl$p", "A B\u{a0}", 3, "<p>A B&nbsp;</p>"),
        ("yl0P", "\u{a0}A B", 0, "<p>&nbsp;A B</p>"),
        ("yl$vp", "A \u{a0}", 2, "<p>A &nbsp;</p>"),
    ] {
        let original = "<p>A B</p>";
        let (mut core, view) = fixture(original, 1, false);
        for event in events.chars() {
            input(&mut core, view, InputEvent::key(event));
        }
        verify(&mut core, view, original, text, cursor, source);
    }
}

#[test]
fn normal_replace_and_substitution_use_protected_spaces() {
    let original = "<p>A B</p>";
    let (mut core, view) = fixture(original, 2, false);
    input(&mut core, view, InputEvent::key('r'));
    input(&mut core, view, InputEvent::key(' '));
    verify(&mut core, view, original, "A \u{a0}", 2, "<p>A &nbsp;</p>");

    let (mut core, view) = fixture(original, 0, false);
    input(&mut core, view, InputEvent::key(':'));
    input(&mut core, view, InputEvent::text("s/B/ /"));
    input(&mut core, view, InputEvent::Key(Key::Enter));
    verify(&mut core, view, original, "A \u{a0}", 2, "<p>A &nbsp;</p>");
}

#[test]
fn paragraph_join_uses_plain_separator_and_preserves_comments() {
    for (events, text) in [("gJ", "AB"), ("J", "A B")] {
        let original = "<p>A \t</p>\n<!--keep--><p> \n B</p>";
        let (mut core, view) = fixture(original, 0, false);
        for event in events.chars() {
            input(&mut core, view, InputEvent::key(event));
        }
        assert_eq!(core.document().text(), text);
        let source = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(source.contains("<!--keep-->"));
        assert!(!source.contains("white-space"), "{source}");
        assert!(!source.contains("&nbsp;"), "{source}");
        verify(&mut core, view, original, text, 1, &source);
    }
}

#[test]
fn substituting_away_a_word_maps_over_the_retained_nonbreaking_space() {
    let original = "<p>A B</p>";
    let (mut core, view) = fixture(original, 0, false);
    input(&mut core, view, InputEvent::key(':'));
    input(&mut core, view, InputEvent::text("s/B//"));
    input(&mut core, view, InputEvent::Key(Key::Enter));
    verify(&mut core, view, original, "A\u{a0}", 1, "<p>A&nbsp;</p>");
}

#[test]
fn ctrl_r_and_repeated_put_keep_protected_spaces_and_valid_carets() {
    let original = "<p>A B</p>";
    let (mut core, view) = fixture(original, 1, false);
    for event in "ylA".chars() {
        input(&mut core, view, InputEvent::key(event));
    }
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    input(&mut core, view, InputEvent::key('"'));
    verify(
        &mut core,
        view,
        original,
        "A B\u{a0}",
        5,
        "<p>A B&nbsp;</p>",
    );

    let (mut core, view) = fixture(original, 1, false);
    for event in "yl$3p".chars() {
        input(&mut core, view, InputEvent::key(event));
    }
    verify(
        &mut core,
        view,
        original,
        "A B \u{a0}\u{a0}",
        6,
        "<p>A B &nbsp;&nbsp;</p>",
    );
}

#[test]
fn substitution_captures_keep_style_after_space_normalization() {
    let original = "<p>A <b>B</b></p>";
    let (mut core, view) = fixture(original, 0, false);
    input(&mut core, view, InputEvent::key(':'));
    input(&mut core, view, InputEvent::text(r"s/(A) (B)/ \2 \1 /"));
    input(&mut core, view, InputEvent::Key(Key::Enter));
    assert_eq!(core.document().text(), "\u{a0}B A\u{a0}");
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert!(source.contains("&nbsp;"));
    assert!(!source.contains("white-space"), "{source}");
    let projection = core.document().projection();
    assert!(
        viem_core::layout::DocumentLayoutStyles::semantic_character_at(projection, 2, false)
            .unwrap()
            .bold,
        "{source}"
    );
    assert!(
        !viem_core::layout::DocumentLayoutStyles::semantic_character_at(projection, 4, false)
            .unwrap()
            .bold,
        "{source}"
    );
    verify(&mut core, view, original, "\u{a0}B A\u{a0}", 0, &source);
}

#[test]
fn typing_after_backspace_simplifies_the_new_protective_space() {
    let original = "<p>A B</p><!--keep-->";
    let (mut core, view) = fixture(original, 3, true);
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().text(), "A\u{a0}");
    input(&mut core, view, InputEvent::text("C"));
    verify(&mut core, view, original, "A C", 3, "<p>A C</p><!--keep-->");
}

#[test]
fn completing_text_before_a_generated_right_space_uses_simple_html() {
    for (original, events, inserted, expected, cursor) in [
        (
            "<p>old B</p><!--keep-->",
            "vec",
            "C",
            "<p>C B</p><!--keep-->",
            1,
        ),
        (
            "<p><b>Bold words</b> and &amp; text.</p>",
            "wvec",
            "ORDS",
            "<p><b>Bold </b><b>ORDS</b> and &amp; text.</p>",
            9,
        ),
    ] {
        let (mut core, view) = fixture(original, 0, false);
        for event in events.chars() {
            input(&mut core, view, InputEvent::key(event));
        }
        assert!(String::from_utf8_lossy(&core.document().source_bytes()).contains("&nbsp;"));
        if original.contains("<b>") {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style: viem_core::document::SemanticInlineStyle::Strong,
                    enabled: true,
                },
            )
            .unwrap();
        }
        input(&mut core, view, InputEvent::text(inserted));
        let text = Document::from_bytes(expected.as_bytes().to_vec(), Encoding::Utf8, Format::Html)
            .unwrap()
            .text()
            .to_owned();
        verify(&mut core, view, original, &text, cursor, expected);
    }
}

#[test]
fn authored_or_explicit_right_nbsp_keeps_its_nonbreaking_semantics() {
    for source in [
        "<p>&nbsp;B</p>",
        "<p>&#160;B</p>",
        "<p>&#xA0;B</p>",
        "<p>\u{a0}B</p>",
    ] {
        let (mut core, view) = fixture(source, 0, true);
        input(&mut core, view, InputEvent::text("C"));
        let expected = source.replacen("<p>", "<p>C", 1);
        verify(&mut core, view, source, "C\u{a0}B", 1, &expected);
    }
    for literal in [false, true] {
        let mut document =
            Document::from_bytes(b"<p>B</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
        document
            .insert(0, if literal { "\u{a0}" } else { " " })
            .unwrap();
        let source = document.source_bytes();
        if !literal {
            document = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Html).unwrap();
        }
        document.insert(0, "C").unwrap();
        assert_eq!(document.text(), "C\u{a0}B");
        let expected = String::from_utf8(source.clone())
            .unwrap()
            .replacen("<p>", "<p>C", 1);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn styled_typing_before_protective_right_space_keeps_caret_after_authored_text() {
    use viem_core::document::{
        FormattedPayloadEdit, FormattedTextPayload, StyleProperty, StylePropertyValue,
    };
    let mut document = Document::from_bytes(
        b"<p><b>B</b></p><!--keep-->".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    document.insert(0, " ").unwrap();
    let before = document.source_bytes();
    let payload = FormattedTextPayload::new(&document.hard_line_snapshot(), "C", vec![]).unwrap();
    let caret = document
        .insert_with_typing_properties(
            FormattedPayloadEdit::new(0..0, payload),
            &[(
                StyleProperty::CharacterBold,
                StylePropertyValue::Boolean(true),
            )],
        )
        .unwrap();
    assert_eq!(document.text(), "C B");
    assert_eq!(caret, 1);
    assert_eq!(document.source_bytes(), b"<p><b>C B</b></p><!--keep-->");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), b"<p><b>C B</b></p><!--keep-->");
}

#[test]
fn core_styled_typing_before_right_space_keeps_the_caret_and_style() {
    let original = "<p><b>old B</b></p><!--keep-->";
    let (mut core, view) = fixture(original, 0, false);
    for event in "vec".chars() {
        input(&mut core, view, InputEvent::key(event));
    }
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style: viem_core::document::SemanticInlineStyle::Strong,
            enabled: true,
        },
    )
    .unwrap();
    input(&mut core, view, InputEvent::text("C"));
    verify(
        &mut core,
        view,
        original,
        "C B",
        1,
        "<p><b>C B</b></p><!--keep-->",
    );
}

#[test]
fn right_space_compaction_declares_only_the_insert_and_entity_patch() {
    use viem_core::document::{ModelRequest, TextEdit};
    let mut document = Document::from_bytes(
        b"<p>old B</p><!--keep-->".to_vec(),
        Encoding::Utf8,
        Format::Html,
    )
    .unwrap();
    document.delete(0..3).unwrap();
    let protected = document.source_bytes();
    assert_eq!(protected, b"<p>&nbsp;B</p><!--keep-->");
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(0..0, "C")],
        })
        .unwrap();
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 2);
    assert_eq!(patches[0].range(), 3..3);
    assert_eq!(patches[0].replacement(), b"C");
    assert_eq!(patches[1].range(), 3..9);
    assert_eq!(&protected[patches[1].range()], b"&nbsp;");
    assert_eq!(patches[1].replacement(), b" ");
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.source_bytes(), b"<p>C B</p><!--keep-->");
    assert_eq!(document.text(), "C B");
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
    assert_eq!(reopened.text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), protected);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), b"<p>C B</p><!--keep-->");
}

#[test]
fn ime_before_a_protective_right_space_keeps_caret_before_the_space() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    let original = "<p>old B</p><!--keep-->";
    let (mut core, view) = fixture(original, 0, false);
    for event in "vec".chars() {
        input(&mut core, view, InputEvent::key(event));
    }
    let protected = String::from_utf8(core.document().source_bytes()).unwrap();
    let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Begin(target)),
    )
    .unwrap();
    core.handle(
        view,
        CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("C", 1..1))),
    )
    .unwrap();
    core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
        .unwrap();
    verify(
        &mut core,
        view,
        &protected,
        "C B",
        1,
        "<p>C B</p><!--keep-->",
    );
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), protected.as_bytes());
    input(&mut core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), protected.as_bytes());
    input(&mut core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), b"<p>C B</p><!--keep-->");
}
