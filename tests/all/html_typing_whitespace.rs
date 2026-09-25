use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, Format, ModelRequest, ProjectionWorkScope,
    SemanticInlineStyle, StyleApplication, TextEdit,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

fn fixture(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(html(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 160.);
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: InputEvent) {
    let description = format!(
        "{input:?} at {} in {}",
        core.command_state(view).unwrap().cursor(),
        String::from_utf8_lossy(&core.document().source_bytes())
    );
    let update = core
        .handle(view, CoreEvent::Input(input))
        .unwrap_or_else(|error| panic!("{description}: {error:?}"));
    if let Some(command) = update.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{description}: {:?}",
            command.status
        );
    }
}

fn assert_reopens(core: &Core<MockTextMeasurementProvider>, expected: &str) {
    assert_eq!(core.document().text(), expected);
    let source = String::from_utf8(core.document().source_bytes()).unwrap();
    assert_eq!(html(&source).text(), expected, "{source}");
}

fn assert_typed_whitespace(core: &Core<MockTextMeasurementProvider>, view: ViewId, visible: &str) {
    let actual = core.document().text();
    assert_eq!(actual.replace('\u{a0}', " "), visible);
    assert_reopens(core, actual);
    assert_eq!(core.command_state(view).unwrap().cursor(), actual.len());
}

fn assert_undo_redo(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, original: &str) {
    input(core, view, InputEvent::Key(Key::Escape));
    let saved = core.document().source_bytes();
    let text = core.document().text().to_owned();
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), saved);
    assert_reopens(core, &text);
}

#[test]
fn ordinary_word_spaces_have_the_same_simple_source_for_scalar_and_whole_text_input() {
    for text in ["Hello, world", "Hello, world!", "one two three"] {
        for scalar_input in [false, true] {
            let original = "<p></p>";
            let (mut core, view) = fixture(original);
            input(&mut core, view, InputEvent::key('i'));
            if scalar_input {
                let mut typed = String::new();
                for ch in text.chars() {
                    input(&mut core, view, InputEvent::text(ch.to_string()));
                    typed.push(ch);
                    assert_typed_whitespace(&core, view, &typed);
                }
            } else {
                input(&mut core, view, InputEvent::text(text));
                assert_reopens(&core, text);
            }
            assert_eq!(
                core.document().source_bytes(),
                format!("<p>{text}</p>").as_bytes(),
                "scalar_input={scalar_input}"
            );
            assert_undo_redo(&mut core, view, original);
        }
    }
}

#[test]
fn simplifying_a_typed_word_space_preserves_surrounding_source_bytes() {
    let original = "<!--before--><P data-note='&amp;' CLASS=keep></P>\r\n<!--after--><p title='untouched'>Tail&#33;</p>";
    let (mut core, view) = fixture(original);
    input(&mut core, view, InputEvent::key('i'));
    for ch in "Hello, world!".chars() {
        input(&mut core, view, InputEvent::text(ch.to_string()));
    }
    let expected = "<!--before--><P data-note='&amp;' CLASS=keep>Hello, world!</P>\r\n<!--after--><p title='untouched'>Tail&#33;</p>";
    assert_eq!(core.document().source_bytes(), expected.as_bytes());
    assert_reopens(&core, "Hello, world!\nTail!");
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn scalar_typing_protects_collapsible_spaces_and_normalizes_tabs() {
    for text in [
        " Hello",
        "Hello ",
        "Hello  world",
        "\tHello",
        "Hello\tworld",
        " \t  ",
    ] {
        let original = "<p></p><!--keep-->";
        let (mut core, view) = fixture(original);
        input(&mut core, view, InputEvent::key('i'));
        let mut typed = String::new();
        for ch in text.chars() {
            input(&mut core, view, InputEvent::text(ch.to_string()));
            typed.push(if ch == '\t' { ' ' } else { ch });
            assert_typed_whitespace(&core, view, &typed);
        }
        let source = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(source.ends_with("</p><!--keep-->"), "{source}");
        assert!(!source.contains("white-space"), "{source}");
        assert!(!source.contains("&#32;"), "{source}");
        if text.starts_with([' ', '\t']) || text.ends_with([' ', '\t']) || text.contains("  ") {
            assert!(source.contains("&nbsp;"), "{source}");
        }
        assert_undo_redo(&mut core, view, original);
    }
}

#[test]
fn typing_does_not_simplify_an_existing_authored_pre_wrap_span() {
    for opening in [
        "<span style=\"white-space: pre-wrap\">",
        "<span data-keep='value' style='white-space: pre-wrap; color: red'>",
    ] {
        let original = format!("<p>Hello,{opening}&#32;</span></p><!--keep-->");
        let (mut core, view) = fixture(&original);
        input(&mut core, view, InputEvent::key('A'));
        for ch in "world!".chars() {
            input(&mut core, view, InputEvent::text(ch.to_string()));
        }
        let expected = format!("<p>Hello,{opening}&#32;world!</span></p><!--keep-->");
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        assert_reopens(&core, "Hello, world!");
        assert_undo_redo(&mut core, view, &original);
    }
}

#[test]
fn batched_text_after_a_generated_space_preserves_its_own_trailing_space() {
    let original = "<p></p>";
    let (mut core, view) = fixture(original);
    input(&mut core, view, InputEvent::key('i'));
    let mut typed = String::new();
    for text in ["Hello,", " ", "world ", "again"] {
        input(&mut core, view, InputEvent::text(text));
        typed.push_str(text);
        assert_typed_whitespace(&core, view, &typed);
    }
    assert_eq!(core.document().source_bytes(), b"<p>Hello, world again</p>");
    assert_undo_redo(&mut core, view, original);
}

#[test]
fn typing_at_block_and_inline_boundaries_uses_only_necessary_nonbreaking_spaces() {
    for (source, at, inserted, expected) in [
        ("<p></p><!--keep-->", 0, " Hello ", "\u{a0}Hello\u{a0}"),
        ("<p>AB</p>", 1, " ", "A B"),
        ("<p>A<span>B</span>C</p>", 1, " ", "A BC"),
        ("<p>A<span>B</span>C</p>", 2, " ", "AB C"),
        ("<p>A<span></span>B</p>", 1, " ", "A B"),
        ("<p>A<br>B</p>", 1, " ", "A\u{a0}\nB"),
        ("<p>A<br>B</p>", 2, " ", "A\n\u{a0}B"),
        ("<p>A</p><p>B</p>", 1, " ", "A\u{a0}\nB"),
        ("<p>A</p><p>B</p>", 2, " ", "A\n\u{a0}B"),
    ] {
        let (mut core, view) = fixture(source);
        input(&mut core, view, InputEvent::key('i'));
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: if core.document().text()[at..].starts_with('\n') {
                    BoundaryAffinity::Upstream
                } else {
                    BoundaryAffinity::Downstream
                },
                extend_selection: false,
            },
        )
        .unwrap_or_else(|error| panic!("{source} at {at}: {error:?}"));
        input(&mut core, view, InputEvent::text(inserted));
        assert_reopens(&core, expected);
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(!saved.contains("white-space"), "{source}: {saved}");
        assert!(!saved.contains("&#32;"), "{source}: {saved}");
        assert_eq!(
            saved.contains("&nbsp;"),
            expected.contains('\u{a0}'),
            "{saved}"
        );
        core.document()
            .text_point(core.command_state(view).unwrap().cursor())
            .unwrap();
        assert_undo_redo(&mut core, view, source);
    }
}

#[test]
fn explicit_and_authored_nonbreaking_spaces_remain_nonbreaking_when_a_word_follows() {
    for (source, input_text, expected_source) in [
        ("<p>A&nbsp;</p>", "B", "<p>A&nbsp;B</p>"),
        ("<p>A&#160;</p>", "B", "<p>A&#160;B</p>"),
        ("<p>A</p>", "\u{a0}B", "<p>A&#160;B</p>"),
    ] {
        let (mut core, view) = fixture(source);
        input(&mut core, view, InputEvent::key('A'));
        for ch in input_text.chars() {
            input(&mut core, view, InputEvent::text(ch.to_string()));
        }
        assert_reopens(&core, "A\u{a0}B");
        assert_eq!(core.document().source_bytes(), expected_source.as_bytes());
        assert_undo_redo(&mut core, view, source);
    }
}

#[test]
fn pasted_whitespace_preserves_visible_count_and_semantic_newlines() {
    for inserted in ["  A   B  ", "\tA\t\tB\t", " A \n B ", "\rA\r\rB\r"] {
        let source = "<p></p><!--keep-->";
        let (mut core, view) = fixture(source);
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::text(inserted));
        assert_typed_whitespace(&core, view, &inserted.replace(['\t', '\r'], " "));
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(!saved.contains("white-space"), "{saved}");
        assert!(!saved.contains("&#32;"), "{saved}");
        assert_undo_redo(&mut core, view, source);
    }
}

#[test]
fn styled_typing_and_ime_use_normalized_whitespace_and_legal_carets() {
    for styled in [false, true] {
        let source = "<p></p><!--keep-->";
        let (mut core, view) = fixture(source);
        input(&mut core, view, InputEvent::key('i'));
        if styled {
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style: SemanticInlineStyle::Strong,
                    enabled: true,
                },
            )
            .unwrap();
        }
        let target = CompositionTarget::at_offsets(core.document(), 0..0).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                " Hello ",
                7..7,
            ))),
        )
        .unwrap();
        core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        assert_typed_whitespace(&core, view, " Hello ");
        let saved = String::from_utf8(core.document().source_bytes()).unwrap();
        assert!(saved.contains("&nbsp;"), "{saved}");
        assert!(!saved.contains("white-space"), "{saved}");
        input(&mut core, view, InputEvent::text("world"));
        assert_typed_whitespace(&core, view, " Hello world");
        assert_eq!(core.document().text(), "\u{a0}Hello world");
    }
}

#[test]
fn replace_backspace_restores_source_and_caret_across_nonbreaking_space_compaction() {
    let source = "<p>AB</p><!--keep-->";
    let (mut core, view) = fixture(source);
    input(&mut core, view, InputEvent::key('$'));
    input(&mut core, view, InputEvent::key('R'));
    input(&mut core, view, InputEvent::text(" "));
    assert_typed_whitespace(&core, view, "A ");
    let after_space = core.document().source_bytes();
    input(&mut core, view, InputEvent::text("C"));
    assert_typed_whitespace(&core, view, "A C");
    assert_eq!(core.document().source_bytes(), b"<p>A C</p><!--keep-->");
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), after_space);
    assert_typed_whitespace(&core, view, "A ");
    input(&mut core, view, InputEvent::text("D"));
    assert_eq!(core.document().source_bytes(), b"<p>A D</p><!--keep-->");
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), after_space);
    input(&mut core, view, InputEvent::Key(Key::Backspace));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.command_state(view).unwrap().cursor(), 1);
}

#[test]
fn deleting_a_normalized_space_retains_the_authored_repeat_frontier() {
    for whitespace in [" ", "\t"] {
        let (mut core, view) = fixture("<p></p><p>X</p>");
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, InputEvent::text("Hello"));
        input(&mut core, view, InputEvent::text(whitespace));
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        input(&mut core, view, InputEvent::Key(Key::Escape));
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('.')
                .unwrap()
                .text,
            "Hello"
        );
        input(&mut core, view, InputEvent::key('j'));
        input(&mut core, view, InputEvent::key('0'));
        input(&mut core, view, InputEvent::key('.'));
        assert_reopens(&core, "Hello\nHelloX");
    }
}

#[test]
fn legacy_insert_interpreter_tracks_utf8_carets_through_protection_and_compaction() {
    let mut document = html("<p></p>");
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('i'))
        .unwrap();
    for (typed, expected) in [
        ("Hello", "Hello"),
        (" ", "Hello\u{a0}"),
        ("world", "Hello world"),
        ("\t", "Hello world\u{a0}"),
    ] {
        commands
            .handle(&mut document, InputEvent::text(typed))
            .unwrap();
        assert_eq!(document.text(), expected);
        assert_eq!(commands.cursor(), expected.len());
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), expected);
    }
    let mut document = html("<p>🇨🇦</p>");
    let mut commands = CommandInterpreter::new();
    commands
        .handle(&mut document, InputEvent::key('i'))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::text("🇫"))
        .unwrap();
    assert_eq!(document.text(), "🇫🇨🇦");
    assert_eq!(commands.cursor(), "🇫🇨".len());
    commands
        .handle(&mut document, InputEvent::text(" "))
        .unwrap();
    assert_eq!(document.text(), "🇫🇨 🇦");
    assert_eq!(commands.cursor(), "🇫🇨 ".len());
}

#[test]
fn disabling_bold_before_a_typed_space_preserves_protective_space_metadata() {
    let (mut core, view) = fixture("<p><b>A</b></p><!--keep-->");
    input(&mut core, view, InputEvent::key('A'));
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style: SemanticInlineStyle::Strong,
            enabled: false,
        },
    )
    .unwrap();
    input(&mut core, view, InputEvent::text(" "));
    assert_typed_whitespace(&core, view, "A ");
    let saved = String::from_utf8(core.document().source_bytes()).unwrap();
    assert!(saved.contains("&nbsp;"), "{saved}");
    assert!(!saved.contains("white-space"), "{saved}");
    input(&mut core, view, InputEvent::text("B"));
    assert_reopens(&core, "A B");
    assert_eq!(
        core.document().source_bytes(),
        b"<p><b>A</b> B</p><!--keep-->"
    );
}

#[test]
fn completing_a_word_space_in_a_large_document_uses_one_local_patch() {
    let prefix = "<p>unchanged words</p>\n".repeat(2_500);
    let suffix = "\n<p>unchanged tail</p>".repeat(2_500);
    let original = format!("{prefix}<p>Hello,</p>{suffix}");
    let mut document = html(&original);
    let at = "unchanged words\n".len() * 2_500 + "Hello,".len();
    document.insert(at, " ").unwrap();
    let before = document.source_bytes();
    let request = ModelRequest::ApplyTextEdits {
        document: document.id(),
        revision: document.revision(),
        edits: vec![TextEdit::new(at + 2..at + 2, "world!")],
    };
    let prepared = document.prepare_model_request(request).unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256, "{work:?}");
    assert_eq!(work.projected_hard_lines(), 1);
    assert_eq!(work.full_text_bytes_materialized(), 0);
    let patches = prepared.summary().source_patches();
    assert_eq!(patches.len(), 1);
    let patch = &patches[0];
    assert_eq!(&before[patch.range()], b"&nbsp;");
    assert_eq!(patch.replacement(), b" world!");
    document.commit_model_transaction(prepared).unwrap();
    let expected = format!("{prefix}<p>Hello, world!</p>{suffix}");
    assert_eq!(document.source_bytes(), expected.as_bytes());
    let reopened = html(&expected);
    assert_eq!(reopened.text(), document.text());
    let content_end = prefix.len() + "<p>Hello, world!".len();
    let separator = document
        .projection()
        .provenance()
        .iter()
        .find(|span| span.formatted == (at + 7..at + 8))
        .unwrap();
    assert_eq!(separator.source, content_end..content_end);
    for source_at in [content_end, content_end + 8, content_end + 9] {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            let mapped = document
                .projection()
                .map_source_boundary(document.revision(), source_at, affinity)
                .map(|point| point.formatted_offset);
            let reparsed = reopened
                .projection()
                .map_source_boundary(reopened.revision(), source_at, affinity)
                .map(|point| point.formatted_offset);
            assert_eq!(mapped, reparsed);
        }
    }
    assert!(document
        .projection()
        .style_spans()
        .iter()
        .all(|span| !span.range.contains(&at)
            || span.application != StyleApplication::SourcePreservedWhitespace));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), before);
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
}
