use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    let status = outcome.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending)
            || key == Key::Escape && status == CommandStatus::Cancelled,
        "{key:?}: {status:?}"
    );
}

#[test]
fn smart_quotes_are_view_input_policy_and_do_not_rewrite_source_on_toggle() {
    let mut core = Core::new(Document::new("word"));
    let first = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    let revision = core.document().revision();
    core.handle(first, CoreEvent::SetSmartQuotes(true)).unwrap();
    assert_eq!(core.document().revision(), revision);
    assert!(!core.document().is_dirty());
    assert!(core.command_state(first).unwrap().smart_quotes());
    assert!(!core.command_state(second).unwrap().smart_quotes());
    key(&mut core, first, Key::Char('i'));
    core.handle(first, CoreEvent::Input(InputEvent::text("\"")))
        .unwrap();
    assert_eq!(core.document().text(), "“word");
    key(&mut core, first, Key::Escape);
    key(&mut core, first, Key::Char('u'));
    assert_eq!(core.document().text(), "word");
    key(&mut core, second, Key::Char('i'));
    core.handle(second, CoreEvent::Input(InputEvent::text("\"")))
        .unwrap();
    assert_eq!(core.document().text(), "\"word");
}

fn smart_fixture(format: Format, source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
    core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
    (core, view)
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize) {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: viem_core::document::BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
}

#[test]
fn replacement_and_batches_share_quote_context_and_atomic_undo() {
    for (keys, input, expected) in [
        ("r", "\"", "“xxxxx"),
        ("3r", "\"", "“““xxx"),
        ("R", "\"a\"", "“a”xxx"),
        ("i", "\"a\" isn't 'b'\n\"c\"", "“a” isn’t ‘b’\n“c”xxxxxx"),
    ] {
        let (mut core, view) = smart_fixture(Format::PlainText, "xxxxxx");
        for ch in keys.chars() {
            key(&mut core, view, Key::Char(ch));
        }
        core.handle(view, CoreEvent::Input(InputEvent::text(input)))
            .unwrap();
        assert_eq!(core.document().text(), expected, "{keys}");
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().text(), "xxxxxx", "{keys}");
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().text(), expected, "{keys}");
    }
}

#[test]
fn every_text_command_preserves_quotes_in_code_spans_and_paragraphs() {
    for (format, source) in [
        (Format::Markdown, "before `xxxxxx` after"),
        (Format::MarkdownSource, "before `xxxxxx` after"),
        (Format::Markdown, "```\nxxxxxx\n```"),
        (Format::MarkdownSource, "```\nxxxxxx\n```"),
    ] {
        for command in ['i', 'r', 'R'] {
            let (mut core, view) = smart_fixture(format, source);
            let at = core.document().text().find("xxxxxx").unwrap();
            place(&mut core, view, at);
            key(&mut core, view, Key::Char(command));
            core.handle(view, CoreEvent::Input(InputEvent::text("\"")))
                .unwrap();
            assert!(
                !core.document().text().contains('“'),
                "{format:?} {command}: {}",
                core.document().text()
            );
            assert!(core.document().text().contains('"'), "{format:?} {command}");
            key(&mut core, view, Key::Escape);
            key(&mut core, view, Key::Char('u'));
            assert_eq!(
                core.document().source_bytes(),
                source.as_bytes(),
                "{format:?} {command}"
            );
        }
    }
}

#[test]
fn source_batches_protect_their_own_code_and_attribute_syntax() {
    for (format, input, expected) in [
        (
            Format::MarkdownSource,
            "\"prose\" `\"code\"`\n\n```\n'code'\n```\n\n'prose'",
            "“prose” `\"code\"`\n\n```\n'code'\n```\n\n‘prose’",
        ),
    ] {
        let (mut core, view) = smart_fixture(format, "");
        key(&mut core, view, Key::Char('i'));
        core.handle(view, CoreEvent::Input(InputEvent::text(input)))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            expected.as_bytes(),
            "{format:?}"
        );
        let reopened =
            Document::from_bytes(expected.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(core.document().text(), reopened.text(), "{format:?}");
    }
}

#[test]
fn composition_transforms_only_committed_prose_and_undoes_atomically() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    for (format, source, expected) in [
        (Format::PlainText, "xxxxxx", "“a”"),
        (Format::Markdown, "`xxxxxx`", "\"a\""),
    ] {
        let (mut core, view) = smart_fixture(format, source);
        key(&mut core, view, Key::Char('i'));
        let target = CompositionTarget::at_offsets(core.document(), 0..6).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new(
                "\"a\"",
                3..3,
            ))),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        assert_eq!(core.document().text(), expected, "{format:?}");
        assert_eq!(core.command_state(view).unwrap().cursor(), expected.len());
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn pending_code_style_suppresses_smart_quotes_before_any_source_span_exists() {
    use viem_core::document::StyleNamespace;
    for (format, source) in [(Format::Markdown, "word"),] {
        let (mut core, view) = smart_fixture(format, source);
        key(&mut core, view, Key::Char('i'));
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::AssignNamedStyle {
                expected,
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: "Code".into(),
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("\"code\"")))
            .unwrap();
        assert_eq!(core.document().text(), "\"code\"word", "{format:?}");
    }
}

#[test]
fn replacement_crossing_code_boundaries_only_curves_prose_quotes() {
    for command in ["3r", "R"] {
        let (mut core, view) = smart_fixture(Format::Markdown, "x`x`x");
        for ch in command.chars() {
            key(&mut core, view, Key::Char(ch));
        }
        let input = if command == "3r" { "\"" } else { "\"\"\"" };
        core.handle(view, CoreEvent::Input(InputEvent::text(input)))
            .unwrap();
        assert_eq!(core.document().text(), "“\"”", "{command}");
    }
}

#[test]
fn smart_quotes_off_and_explicit_curly_quotes_remain_literal_for_batches() {
    let (mut core, view) = smart_fixture(Format::PlainText, "");
    core.handle(view, CoreEvent::SetSmartQuotes(false)).unwrap();
    key(&mut core, view, Key::Char('i'));
    core.handle(
        view,
        CoreEvent::Input(InputEvent::text("\"straight\" ‘explicit’")),
    )
    .unwrap();
    assert_eq!(core.document().text(), "\"straight\" ‘explicit’");
    core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text(" “explicit”")))
        .unwrap();
    assert_eq!(core.document().text(), "\"straight\" ‘explicit’ “explicit”");
}

#[test]
fn empty_code_areas_and_code_span_ends_keep_quotes_literal() {
    for (format, source, at, command, expected) in [
        (Format::Markdown, "```\n\n```", 0, 'i', "\"code\""),
        (Format::Markdown, "`x` after", 0, 'a', "x\"code\" after"),
    ] {
        let (mut core, view) = smart_fixture(format, source);
        place(&mut core, view, at);
        key(&mut core, view, Key::Char(command));
        core.handle(view, CoreEvent::Input(InputEvent::text("\"code\"")))
            .unwrap();
        assert_eq!(core.document().text(), expected, "{format:?} {source}");
    }
}

#[test]
fn implicit_quotes_do_not_make_encodable_input_fail() {
    for (format, source) in [
        (Format::PlainText, "x"),
        (Format::Markdown, "x"),
        (Format::MarkdownSource, "x"),
    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Latin1, format).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 120.0);
        core.handle(view, CoreEvent::SetSmartQuotes(true)).unwrap();
        let at = core.document().text().find('x').unwrap();
        place(&mut core, view, at);
        key(&mut core, view, Key::Char('i'));
        core.handle(view, CoreEvent::Input(InputEvent::text("\"word\"")))
            .unwrap();
        assert!(core.document().text().contains("\"word\""), "{format:?}");
    }
}

#[test]
fn pending_code_does_not_change_the_context_of_normal_replace() {
    use viem_core::document::StyleNamespace;
    let (mut core, view) = smart_fixture(Format::Markdown, "x");
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected,
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: StyleNamespace::Character,
            style: "Code".into(),
        },
    )
    .unwrap();
    key(&mut core, view, Key::Char('r'));
    key(&mut core, view, Key::Char('"'));
    assert_eq!(core.document().text(), "“");
}

#[test]
fn replace_mode_extending_past_code_text_keeps_new_quotes_literal() {
    let (mut core, view) = smart_fixture(Format::Markdown, "x`x`");
    key(&mut core, view, Key::Char('R'));
    core.handle(view, CoreEvent::Input(InputEvent::text("a\"b\"")))
        .unwrap();
    assert_eq!(core.document().text(), "a\"b\"");
}
