use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format, ListStyle, SemanticInlineStyle};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    (core, view)
}
fn verify(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    source: &str,
    original: &str,
) {
    assert_eq!(core.document().text(), original);
    let result = core.document().source_bytes();
    let reopened = Document::from_bytes(result.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(reopened.text(), core.document().text());
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), result);
}

#[test]
fn headings_preserve_empty_and_deep_list_owners() {
    for source in ["-", "- ", "- - - deep", "> - > - deep", "1. 2. deep"] {
        let (mut core, view) = open(source);
        let original = core.document().text().to_owned();
        let containers = core.document().projection().blocks()[0].containers.len();
        let expected = core.list_selection_identity(view).unwrap();
        core.handle_with_layout(
            view,
            CoreEvent::SetParagraphStyle {
                expected,
                style: "Heading2".into(),
            },
        )
        .unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
        let block = &core.document().projection().blocks()[0];
        assert_eq!(block.style.0, "Heading2");
        assert_eq!(block.containers.len(), containers);
        verify(&mut core, view, source, &original);
    }
}

#[test]
fn quotes_keep_setext_underlines_and_list_code_owners() {
    for (source, target) in [
        ("Setext\n===", "Setext"),
        ("Setext\n---", "Setext"),
        ("Setext\nline two\n===", "Setext"),
        ("text before\n| a | b |\n| - | - |\n| 1 | 2 |", "text"),
        ("- Setext\n  ===", "Setext"),
        ("- - - deep", "deep"),
        ("```\none\n\nthree\n```", "one"),
        ("```js\ncode", "code"),
        ("- a\n  ```\n  code\n  ```", "code"),
        ("- ```\n  code\n  ```", "code"),
        ("- - ```\n    code\n\n    end\n    ```", "code"),
        ("para\n```\ncode\n```", "code"),
    ] {
        let (mut core, view) = open(source);
        let original = core.document().text().to_owned();
        let cursor = original.find(target).unwrap();
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: cursor,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let before = core
            .document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&cursor))
            .unwrap()
            .clone();
        let expected = core.list_selection_identity(view).unwrap();
        core.handle_with_layout(
            view,
            CoreEvent::SetBlockQuote {
                expected,
                enabled: true,
            },
        )
        .unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
        let after = core
            .document()
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&cursor))
            .unwrap();
        assert!(after.quote_depth > 0);
        assert_eq!(after.style, before.style, "{source:?}");
        assert_eq!(after.kind, before.kind, "{source:?}");
        verify(&mut core, view, source, &original);
    }
}

#[test]
fn special_owner_list_and_heading_assignments_reopen_and_undo() {
    for source in [
        "```\none\n\nthree\n```",
        "```js\ncode",
        "para\na | b\n- | -\n1 | 2",
    ] {
        for list in [None, Some(ListStyle::Bullet), Some(ListStyle::Numbered)] {
            let (mut core, view) = open(source);
            let original = core.document().text().to_owned();
            let expected = core.list_selection_identity(view).unwrap();
            let action = match list {
                Some(style) => CoreEvent::SetListStyle {
                    expected,
                    style: Some(style),
                },
                None => CoreEvent::SetParagraphStyle {
                    expected,
                    style: "Heading2".into(),
                },
            };
            core.handle_with_layout(view, action)
                .unwrap_or_else(|e| panic!("{source:?}, {list:?}: {e:?}"));
            verify(&mut core, view, source, &original);
        }
    }
}

#[test]
fn pending_emphasis_inside_and_after_table_code_preserves_columns() {
    let source = "| `a\\|b` | c |\n| --- | --- |\n| x | y |";
    for at in 0..=3 {
        for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
            let (mut core, view) = open(source);
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
                .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: at,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            let mut expected_text = core.document().text().to_owned();
            expected_text.insert(at, 'x');
            let expected = core.list_selection_identity(view).unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style,
                    enabled: true,
                },
            )
            .unwrap();
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::text("x")))
                .unwrap_or_else(|e| panic!("{at}, {style:?}: {e:?}"));
            assert_eq!(core.document().projection().tables()[0].columns.len(), 2);
            verify(&mut core, view, source, &expected_text);
        }
    }
}

#[test]
fn selected_emphasis_keeps_passive_html_and_table_cell_boundaries() {
    for source in [
        "<div>\ntext *literal*\n</div>",
        "<div>\n<ins>under</ins> text\n</div>",
        "<!-- comment -->",
        "---\ntitle: hello\n---\n\nbody",
        "| a | b |\n| - | - |\n| one | two |",
        "| `a\\|b` | c |\n| --- | --- |\n| x | y |",
    ] {
        for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
            let (mut core, view) = open(source);
            let original = core.document().text().to_owned();
            // Front matter is literal YAML code. Exercise prose formatting
            // beside that owner without trying to style its literal body.
            let selection_start = if source.starts_with("---\n") {
                original.find("body").unwrap()
            } else { 0 };
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
                .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::BeginPointerGesture {
                    document_revision: core.document().revision(),
                    text_offset: selection_start,
                    affinity: BoundaryAffinity::Downstream,
                },
            )
            .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: original.len(),
                    affinity: BoundaryAffinity::Upstream,
                    extend_selection: true,
                },
            )
            .unwrap();
            let expected = core.list_selection_identity(view).unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::SetSelectionSemanticStyle {
                    expected,
                    style,
                    enabled: true,
                },
            )
            .unwrap_or_else(|e| panic!("{source:?}, {style:?}: {e:?}"));
            if selection_start > 0 {
                assert!(core.document().source_bytes().starts_with(b"---\ntitle: hello\n---\n\n"));
            }
            assert!(core
                .document()
                .projection()
                .style_spans()
                .iter()
                .any(|span| match &span.application {
                    viem_core::document::StyleApplication::Semantic(candidate) =>
                        *candidate == style,
                    viem_core::document::StyleApplication::Direct(properties) => match style {
                        SemanticInlineStyle::Strong => properties.bold == Some(true),
                        SemanticInlineStyle::Emphasis =>
                            properties.slant == Some(viem_core::document::FontSlant::Italic),
                        _ => false,
                    },
                    _ => false,
                }));
            verify(&mut core, view, source, &original);
        }
    }
}

#[test]
fn repository_global_delete_preserves_completed_prefix_on_supported_targets() {
    use viem_core::command::ex_execute::ExExecuteError;
    use viem_core::command::{CommandStatus, ExCommandError};
    use viem_core::document::DocumentError;
    let source = include_str!("../../docs/markdown_demo.md");
    let (mut core, view) = open(source);
    const MAX_ZERO_PROGRESS_RETRIES: usize = 16;
    let max_attempts = core.document().hard_line_snapshot().line_count()
        + 1
        + MAX_ZERO_PROGRESS_RETRIES;
    let mut zero_progress_retries = 0;
    for _ in 0..max_attempts {
        let before = core.document().source_bytes();
        let before_revision = core.document().revision();
        for key in ":g/a/d".chars() {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key)))
                .unwrap();
        }
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap();
        let status = output.command.unwrap().status;
        let table_policy = matches!(
            &status,
            CommandStatus::ExError(ExCommandError::Execute(ExExecuteError::Document(
                DocumentError::UnsupportedTableEdit(_)
            )))
        );
        let work_limit =
            matches!(&status, CommandStatus::Unsupported(message) if message.contains("work limit"));
        assert!(
            table_policy
                || matches!(&status, CommandStatus::Complete)
                || work_limit,
            "{status:?}"
        );
        let result = core.document().source_bytes();
        let reopened =
            Document::from_bytes(result.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        if result == before {
            let remaining = core.document().text().match_indices('a')
                .map(|(at, _)| at).collect::<Vec<_>>();
            if work_limit {
                // The wall-clock replay budget can expire before the first
                // target when other tests contend for CPU. Retry that pause
                // without treating it as a completed undoable edit.
                assert_eq!(core.document().revision(), before_revision);
                zero_progress_retries += 1;
                assert!(
                    zero_progress_retries <= MAX_ZERO_PROGRESS_RETRIES,
                    ":global repeatedly exhausted its budget without progress: {status:?}"
                );
                continue;
            }
            // Whole visual-row table deletion remains the existing open
            // policy. Repeated continuation must end at that explicit refusal,
            // never an internal verification error or silently accepted edit.
            assert!(table_policy, "{status:?}");
            assert!(!remaining.is_empty());
            assert!(remaining.iter().all(|at| core.document().projection().table_cell_at(*at).is_some()));
            return;
        }
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), before);
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), result);
    }
    panic!("Repeated global deletion did not reach its remaining table-policy boundary");
}

#[test]
fn passive_html_emphasis_scope_can_be_cleared_without_changing_other_markup() {
    for (source, style, expected) in [
        (
            "<div>\n<strong>text *literal*</strong>\n</div>",
            SemanticInlineStyle::Strong,
            "<div>\ntext *literal*\n</div>",
        ),
        (
            "<div>\n<em>text *literal*</em>\n</div>",
            SemanticInlineStyle::Emphasis,
            "<div>\ntext *literal*\n</div>",
        ),
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let range = 0..document.text().len();
        let unchanged = document.source_bytes();
        document
            .set_semantic_style(range.clone(), style, true)
            .unwrap();
        assert_eq!(document.source_bytes(), unchanged);
        document.set_semantic_style(range, style, false).unwrap();
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(document.text(), "text *literal*");
    }
}

#[test]
fn passive_html_emphasis_toggles_independently_across_existing_tags() {
    for source in [
        "<div>\n<ins>under</ins> text\n</div>",
        "<div>\nleft &amp; right\n</div>",
    ] {
        for order in [
            [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis],
            [SemanticInlineStyle::Emphasis, SemanticInlineStyle::Strong],
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let original = document.text().to_owned();
            let range = 0..original.len();
            for style in order {
                document
                    .set_semantic_style(range.clone(), style, true)
                    .unwrap();
            }
            for style in order {
                document
                    .set_semantic_style(range.clone(), style, false)
                    .unwrap();
            }
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert_eq!(document.text(), original);
        }
    }
}

#[test]
fn october11_block_actions_preserve_special_source_owners() {
    for (source, at, action, expected) in [
        ("# Heading #", 0, 0, "Heading"),
        ("- > quote in list", 0, 1, "- quote in list"),
        ("- a\n  - b\n    - c\n      - d", 6, 2, "- a\n  - b\n    - c\n      - > d"),
        ("para\n| a | b |\n| - | - |\n| 1 | 2 |", 0, 3, "- para\n\n| a | b |\n| - | - |\n| 1 | 2 |"),
    ] {
        let (mut core, view) = open(source);
        let original = core.document().text().to_owned();
        core.handle_with_layout(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let expected_identity = core.list_selection_identity(view).unwrap();
        let event = match action {
            0 => CoreEvent::SetParagraphStyle { expected: expected_identity, style: "Paragraph".into() },
            1 | 2 => CoreEvent::SetBlockQuote { expected: expected_identity, enabled: action == 2 },
            _ => CoreEvent::SetListStyle { expected: expected_identity, style: Some(ListStyle::Bullet) },
        };
        core.handle_with_layout(view, event).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        verify(&mut core, view, source, &original);
    }
}

#[test]
fn ex_line_deletion_preserves_following_markdown_owners() {
    use viem_core::command::CommandStatus;
    for (deleted, retained) in [
        ("# a", "| b |\n| --- |\n| c |"),
        ("## a ##", "keep"),
        ("a\n===", "```\nb\n```"),
        ("a&amp; &#x1F642;", "### b ###"),
        ("[](url)a <https://example.com> and <writer@example.com>.", "b"),
        ("<!-- a -->", "<div>\n<p>b</p>\n</div>"),
        ("# a", "<section>\n<div>\n<p>b</p>\n</div>\n</section>"),
        ("<pre>a</pre>", "keep"),
        ("<div>\n<p>a</p>\n</div>", "b"),
    ] {
        for ending in ["\n", "\r\n"] {
            let source = format!("{deleted}\n\n{retained}").replace('\n', ending);
            let retained = retained.replace('\n', ending);
            for command in [":d", ":g/a/d"] {
                let (mut core, view) = open(&source);
                for key in command.chars() {
                    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key)))
                        .unwrap();
                }
                let output = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                    .unwrap();
                let status = output.command.unwrap().status;
                assert!(matches!(status, CommandStatus::Complete), "{command}: {source}: {status:?}");
                assert_eq!(core.document().source_bytes(), retained.as_bytes(), "{command}: {source}");
                let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
                assert_eq!(reopened.text(), core.document().text());
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), retained.as_bytes());
            }
        }
    }
}

#[test]
fn ex_line_deletion_preserves_retained_code_rows() {
    use viem_core::command::CommandStatus;
    for source in [
        "keep\n\n    a\n\n        b\n\nend",
        "keep\n\n<pre>a\n  b</pre>\n\nend",
        "keep\n\n<pre lang=\"rust\">a\nb\n}</pre>\n\nend",
        "keep\n\n```text\na\n\nb\n```\n\nend",
    ] {
        for command in [":d", ":g/^a/d"] {
            let (mut core, view) = open(source);
            let original = core.document().text().to_owned();
            let at = original.find('a').unwrap();
            let range = core.document().hard_line_snapshot().line_at_offset(at).unwrap().linewise_range();
            let mut expected = original.clone();
            expected.replace_range(range, "");
            core.handle_with_layout(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            for key in command.chars() {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
            }
            let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap().command.unwrap().status;
            assert!(matches!(status, CommandStatus::Complete), "{command}: {source}: {status:?}");
            assert_eq!(core.document().text(), expected, "{command}: {source}");
            let result = core.document().source_bytes();
            let reopened = Document::from_bytes(result.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            let retained = expected.find('b').unwrap();
            assert_eq!(reopened.projection().blocks().iter()
                .find(|block| block.range.contains(&retained)).unwrap().style.0, "Code Block");
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
            assert_eq!(core.document().source_bytes(), result);
        }
    }
}

#[test]
fn ex_final_line_deletion_preserves_preceding_html_owner() {
    use viem_core::command::CommandStatus;
    for retained in ["<div>\n<p>b</p>\n</div>", "<pre>b</pre>"] {
        let source = format!("{retained}\n\n# a");
        let (mut core, view) = open(&source);
        for key in ":g/a/d".chars() {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
        }
        let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap().command.unwrap().status;
        assert!(matches!(status, CommandStatus::Complete), "{source}: {status:?}");
        assert_eq!(core.document().source_bytes(), retained.as_bytes());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
        assert_eq!(core.document().source_bytes(), retained.as_bytes());
    }
}

#[test]
fn global_delete_does_not_clear_a_different_table_cell_than_the_match() {
    use viem_core::command::ex_execute::ExExecuteError;
    use viem_core::command::{CommandStatus, ExCommandError};
    use viem_core::document::DocumentError;
    let source = "| keep | a |\n| --- | --- |\n| b | c |";
    let (mut core, view) = open(source);
    let revision = core.document().revision();
    for key in ":g/a/d".chars() {
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
    }
    let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
        .unwrap().command.unwrap().status;
    assert!(matches!(status, CommandStatus::ExError(ExCommandError::Execute(
        ExExecuteError::Document(DocumentError::UnsupportedTableEdit(_))
    ))), "{status:?}");
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().revision(), revision);
}

#[test]
fn ex_deletion_preserves_nested_list_children() {
    use viem_core::command::CommandStatus;
    let source = "- a\n  - b\n    - c\n      - d\n        - e";
    for (source, command, expected) in [
        (source, ":2d", "a\nc\nd\ne"),
        ("- a\n  - b\n    - c\n    - d\n  - e", ":2,3d", "a\nd\ne"),
    ] {
        for offset in [0, 4] {
            let (mut core, view) = open(source);
            core.handle_with_layout(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: offset,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            for key in command.chars() {
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
            }
            let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                .unwrap().command.unwrap().status;
            assert!(matches!(status, CommandStatus::Complete), "{command}: {offset}: {status:?}");
            assert_eq!(core.document().text(), expected);
            let result = core.document().source_bytes();
            let reopened = Document::from_bytes(result.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            assert!(reopened.projection().blocks().iter().all(|block|
                matches!(block.kind, viem_core::document::BlockKind::ListItem { .. })
                    && block.style.0 != "Code Block"));
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
            assert_eq!(core.document().source_bytes(), result);
        }
    }
}

#[test]
fn ex_empty_list_line_keeps_its_existing_deletion_policy() {
    use viem_core::command::CommandStatus;
    for (source, expected) in [("- ", "- "), ("keep\n\n- ", "keep")] {
        let (mut core, view) = open(source);
        for key in ":$d".chars() {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
        }
        let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap().command.unwrap().status;
        assert!(matches!(status, CommandStatus::Complete), "{source}: {status:?}");
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
    }
}

#[test]
fn ex_final_line_deletion_preserves_preceding_table() {
    use viem_core::command::CommandStatus;
    for retained in ["| b |\n| - |\n| c |", "> | b |\n> | - |\n> | c |"] {
        for deleted in ["a", "- a", "- [](#target)a"] {
            let source = format!("{retained}\n\n{deleted}");
            for command in [":$d", ":g/a/d"] {
                let (mut core, view) = open(&source);
                for key in command.chars() {
                    core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
                }
                let status = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
                    .unwrap().command.unwrap().status;
                assert!(matches!(status, CommandStatus::Complete), "{command}: {source}: {status:?}");
                assert_eq!(core.document().source_bytes(), retained.as_bytes());
                let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
                assert_eq!(reopened.text(), core.document().text());
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
                assert_eq!(core.document().source_bytes(), retained.as_bytes());
            }
        }
    }
}
