use viem_core::command::{CommandStatus, InputEvent, Key, NavigationKey};
use viem_core::document::{BoundaryAffinity, Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
fn editor(source: &str, format: Format, file_format: FileFormat) -> (Editor, ViewId) {
    let mut core = Core::new(
        Document::from_bytes_with_file_format(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            format,
            file_format,
        )
        .unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 1200.);
    (core, view)
}
fn input(core: &mut Editor, view: ViewId, event: InputEvent) {
    let output = core
        .handle_with_layout(view, CoreEvent::Input(event.clone()))
        .unwrap_or_else(|error| {
            panic!(
                "{event:?} in {:?}: {error:?}",
                core.document().text().chars().take(80).collect::<String>()
            )
        });
    let status = output.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending)
            || matches!(event, InputEvent::Key(Key::Escape)) && status == CommandStatus::Cancelled,
        "{event:?} {status:?} {:?} {:?}",
        core.document().format(),
        core.document().file_format()
    );
}
fn key(core: &mut Editor, view: ViewId, value: Key) {
    input(core, view, InputEvent::Key(value));
}
fn text(core: &mut Editor, view: ViewId, value: &str) {
    input(core, view, InputEvent::text(value));
}
fn place(core: &mut Editor, view: ViewId, offset: usize, extend: bool) {
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: extend,
        },
    )
    .unwrap();
}
fn fresh(core: &Editor, format: Format, file_format: FileFormat) {
    let reopened = Document::from_bytes_with_file_format(
        core.document().source_bytes(),
        Encoding::Utf8,
        format,
        file_format,
    )
    .unwrap();
    assert_eq!(reopened.text(), core.document().text());
    let blocks = |document: &Document| {
        document
            .projection()
            .blocks()
            .iter()
            .map(|block| {
                format!(
                    "{:?} {:?} {:?} {} {} {} {:?} {:?}",
                    block.range,
                    block.kind,
                    block.style,
                    block.quote_depth,
                    block.list_loose,
                    block.markdown_html,
                    block.thematic_break,
                    block.code_language
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(blocks(&reopened), blocks(core.document()));
    if format.is_markdown() {
        assert_eq!(
            reopened.projection().style_spans(),
            core.document().projection().style_spans()
        );
    }
}

#[test]
fn multiline_heading_typeover_keeps_closing_syntax_out_of_visible_prose() {
    for source in [
        "# Heading #",
        "# **Heading** ###",
        "> # Heading #",
        "# Heading #\n\nuntouched",
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        let mut expected = core.document().text().to_owned();
        expected.replace_range(0..3, "one\ntwo");
        key(&mut core, view, Key::Char('i'));
        core.handle_with_layout(
            view,
            CoreEvent::BeginPointerGesture {
                document_revision: core.document().revision(),
                text_offset: 0,
                affinity: BoundaryAffinity::Downstream,
            },
        )
        .unwrap();
        place(&mut core, view, 3, true);
        text(&mut core, view, "one\ntwo");
        assert_eq!(core.document().text(), expected);
        fresh(&core, Format::Markdown, FileFormat::Unix);
        let edited = core.document().source_bytes();
        if source.ends_with("untouched") {
            assert!(edited.ends_with(b"\n\nuntouched"));
        }
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), edited);
    }
}

#[test]
fn substitution_line_break_splits_inline_code_and_restores_exact_source() {
    let source = "`a b c`";
    let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
    text(&mut core, view, ":s/ b /\\r/");
    key(&mut core, view, Key::Enter);
    assert_eq!(core.document().text(), "a\nc");
    fresh(&core, Format::Markdown, FileFormat::Unix);
    let edited = core.document().source_bytes();
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), edited);
}

#[test]
fn unrepresentable_substitute_literal_lf_explains_the_format_constraint() {
    for format in [
        Format::Markdown,
        Format::MarkdownSource,
        Format::PlainText,
        Format::Code,
    ] {
        for file_format in [FileFormat::Unix, FileFormat::Dos] {
            let (mut core, view) = editor("abc", format, file_format);
            let revision = core.document().revision();
            text(&mut core, view, ":s/b/\\n/");
            let output = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap();
            let CommandStatus::ExError(error) = output.command.unwrap().status else {
                panic!("literal LF must be rejected");
            };
            let message = match error {
                viem_core::command::ExCommandError::Execute(error) => error.to_string(),
                error => panic!("{error:?}"),
            };
            assert!(
                message.contains("literal line-feed") && message.contains("\\r"),
                "{message}"
            );
            assert_eq!(core.document().source_bytes(), b"abc");
            assert_eq!(core.document().revision(), revision);
            key(&mut core, view, Key::Char('i'));
            text(&mut core, view, "x");
            assert_eq!(core.document().text(), "xabc");
        }
        if format == Format::MarkdownSource {
            continue;
        }
        let (mut core, view) = editor("abc", format, FileFormat::Mac);
        text(&mut core, view, ":s/b/\\n/");
        key(&mut core, view, Key::Enter);
        assert_eq!(core.document().text(), "a\nc");
        assert_eq!(core.document().line_count(), 1);
        fresh(&core, format, FileFormat::Mac);
    }
}

#[test]
fn replacing_only_a_paragraph_separator_with_unicode_keeps_positions_valid() {
    for replacement in ["é", "العربية", "👩‍💻"] {
        let source = "\n\n\n\n";
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        key(&mut core, view, Key::Char('i'));
        place(&mut core, view, 1, false);
        key(
            &mut core,
            view,
            Key::ModifiedNavigation {
                key: NavigationKey::Left,
                modifiers: 1,
            },
        );
        text(&mut core, view, replacement);
        assert_eq!(core.document().text(), format!("{replacement}\n"));
        fresh(&core, Format::Markdown, FileFormat::Unix);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn pending_source_emphasis_enter_maps_the_existing_unicode_insert_floor() {
    let source = "- x\n- y\n<ins>unclosed";
    let (mut core, view) = editor(source, Format::MarkdownSource, FileFormat::Unix);
    place(&mut core, view, 8, false);
    key(&mut core, view, Key::Char('O'));
    text(&mut core, view, "العربية");
    place(&mut core, view, 1, false);
    let expected = core.list_selection_identity(view).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::SetSelectionSemanticStyle {
            expected,
            style: viem_core::document::SemanticInlineStyle::Emphasis,
            enabled: true,
        },
    )
    .unwrap();
    key(&mut core, view, Key::Enter);
    assert!(core
        .document()
        .text_point(core.command_state(view).unwrap().cursor())
        .is_ok());
    text(&mut core, view, "é");
    fresh(&core, Format::MarkdownSource, FileFormat::Unix);
    let edited = core.document().source_bytes();
    key(&mut core, view, Key::Escape);
    key(&mut core, view, Key::Char('u'));
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), edited);
}

#[test]
fn structural_edits_keep_live_markdown_equal_to_reopened_source() {
    for (source, target, command, inserted) in [
        ("-\tfoo\n\n\tbar", "bar", "cc", ""),
        ("-\tfoo\n\n\tbar", "bar", "S", ""),
        ("-\tfoo\n\n\tbar", "bar", "ciw", ""),
        (
            "- a\n\n      indented code in list\n- b",
            "indented",
            "o",
            "x",
        ),
        (
            "- a\n\n      indented code in list\n- b",
            "indented",
            "O",
            "x",
        ),
        (
            "> ```\n> unclosed fence in quote\n\nafter",
            "unclosed",
            "dip",
            "",
        ),
        (
            "> ```\n> unclosed fence in quote\n\nafter",
            "unclosed",
            "das",
            "",
        ),
        (
            "> ```\n> unclosed fence in quote\n\nafter",
            "unclosed",
            "vipd",
            "",
        ),
        (
            "1. one\n1. one\n1. one\n\n   para in third",
            "one",
            "Vjc",
            "x",
        ),
        ("foot[^1] note\n\n[^1]: The note.", "The", "x", ""),
        ("foot[^1] note\n\n[^1]: The note.", "The", "s", "x"),
        ("[ref]:\n/url\n\n[ref]", "/url", "C", "x"),
        ("<!-- unclosed comment", "<!--", "o", "---"),
        ("[ref]\n\n[ref]: /url", "[ref]", "A", "]é"),
        ("a\n- \nb", "", "jdw", ""),
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        let offset = core.document().text().find(target).unwrap();
        place(&mut core, view, offset, false);
        for ch in command.chars() {
            key(&mut core, view, Key::Char(ch));
        }
        if !inserted.is_empty() {
            text(&mut core, view, inserted);
        }
        fresh(&core, Format::Markdown, FileFormat::Unix);
        key(&mut core, view, Key::Escape);
        let edited = core.document().source_bytes();
        key(&mut core, view, Key::Char('u'));
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{source:?} {command}"
        );
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), edited);
    }
}

#[test]
fn copying_a_heading_ignores_unrelated_table_source_mapping() {
    let source = "# Heading\n\nParagraph\n\n| a | b |\n| - | - |\n| x | y |\n\nAfter";
    let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
    let before = core.document().text().to_owned();
    text(&mut core, view, ":t.");
    key(&mut core, view, Key::Enter);
    assert_eq!(core.document().text(), format!("Heading\n{before}"));
    fresh(&core, Format::Markdown, FileFormat::Unix);
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn deleting_the_final_prose_body_preserves_its_unselected_boundary() {
    for source in [
        "a\n- \nb",
        "a\n===\nb",
        "# a\nb",
        "a\n\nb",
        "```\ncode\n```\nb",
        "> # a\n> b",
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        let before = core.document().text().to_owned();
        place(&mut core, view, before.len() - 1, false);
        key(&mut core, view, Key::Char('d'));
        key(&mut core, view, Key::Char('w'));
        assert_eq!(
            core.document().text(),
            &before[..before.len() - 1],
            "{source:?}"
        );
        fresh(&core, Format::Markdown, FileFormat::Unix);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn repository_markdown_copy_move_and_sort_preserve_reopened_projection() {
    for (source, command) in [
        (include_str!("../../docs/markdown_demo.md"), ":t."),
        (include_str!("../../docs/markdown_demo.md"), ":m+1"),
        (include_str!("../../README.md"), ":sort"),
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        text(&mut core, view, command);
        key(&mut core, view, Key::Enter);
        fresh(&core, Format::Markdown, FileFormat::Unix);
        let edited = core.document().source_bytes();
        key(&mut core, view, Key::Char('u'));
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{command}"
        );
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), edited);
    }
}

#[test]
fn native_selection_replacements_preserve_nested_list_owners() {
    for (source, word) in [
        ("1. > # heading\n   > text\n2. b", true),
        ("1. one\n1. one\n1. one\n\n   para in third", false),
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        if word {
            core.handle_with_layout(
                view,
                CoreEvent::SelectPointerWord {
                    document_revision: core.document().revision(),
                    text_offset: 1,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            key(&mut core, view, Key::Char('O'));
        } else {
            place(&mut core, view, 0, false);
            place(&mut core, view, 11, true);
            text(&mut core, view, "x");
        }
        fresh(&core, Format::Markdown, FileFormat::Unix);
        let edited = core.document().source_bytes();
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        key(&mut core, view, Key::Ctrl('r'));
        assert_eq!(core.document().source_bytes(), edited);
        fresh(&core, Format::Markdown, FileFormat::Unix);
    }
}

#[test]
fn repository_markdown_structural_actions_use_current_content_positions() {
    let source = include_str!("../../docs/markdown_demo.md");
    for action in [
        "heading",
        "bullets",
        "numbers",
        "code-enter",
        "comment-break",
    ] {
        let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
        let target = match action {
            "code-enter" => "../examples/markdown-image-flow.png",
            "comment-break" => "A standalone comment",
            _ => "Setext heading level two",
        };
        let offset = core.document().text().find(target).unwrap();
        place(&mut core, view, offset, false);
        if matches!(action, "code-enter" | "comment-break") {
            key(&mut core, view, Key::Char('i'));
            place(&mut core, view, offset + 5, false);
            let event = InputEvent::Key(if action == "code-enter" {
                Key::Enter
            } else {
                Key::ShiftEnter
            });
            core.handle_with_layout(view, CoreEvent::Input(event))
                .unwrap_or_else(|error| panic!("{action}: {error:?}"));
        } else {
            let expected = core.list_selection_identity(view).unwrap();
            let event = if action == "heading" {
                CoreEvent::SetParagraphStyle {
                    expected,
                    style: "Heading1".into(),
                }
            } else {
                CoreEvent::SetListStyle {
                    expected,
                    style: Some(if action == "numbers" {
                        viem_core::document::ListStyle::Numbered
                    } else {
                        viem_core::document::ListStyle::Bullet
                    }),
                }
            };
            core.handle_with_layout(view, event)
                .unwrap_or_else(|error| panic!("{action}: {error:?}"));
        }
        fresh(&core, Format::Markdown, FileFormat::Unix);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{action}"
        );
    }
}

#[test]
fn source_selection_enter_and_table_typeover_preserve_literal_editing() {
    for (source, range, action) in [
        ("*it*", 1..4, "enter"),
        ("**bold**", 1..8, "enter"),
        ("a | b\n- | -\n1 | 2", 0..14, "delete"),
        ("a | b\n- | -\n1 | 2", 0..14, "replace"),
    ] {
        let (mut core, view) = editor(source, Format::MarkdownSource, FileFormat::Unix);
        place(&mut core, view, range.start, false);
        place(&mut core, view, range.end, true);
        match action {
            "enter" => key(&mut core, view, Key::Enter),
            "delete" => key(&mut core, view, Key::Backspace),
            _ => text(&mut core, view, "x"),
        }
        fresh(&core, Format::MarkdownSource, FileFormat::Unix);
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{source:?} {action}"
        );
    }
}

#[test]
fn default_character_choice_on_partial_underline_is_verified() {
    let source = "<ins>under</ins> after";
    let (mut core, view) = editor(source, Format::Markdown, FileFormat::Unix);
    place(&mut core, view, 0, false);
    place(&mut core, view, 3, true);
    let expected = core.list_selection_identity(view).unwrap();
    core.handle_with_layout(
        view,
        CoreEvent::AssignNamedStyle {
            expected,
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: viem_core::document::StyleNamespace::Character,
            style: "".into(),
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "under after");
    fresh(&core, Format::Markdown, FileFormat::Unix);
    if core.document().source_bytes() != source.as_bytes() {
        key(&mut core, view, Key::Escape);
        key(&mut core, view, Key::Char('u'));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
