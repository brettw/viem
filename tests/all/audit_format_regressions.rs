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
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('i')))
                .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::BeginPointerGesture {
                    document_revision: core.document().revision(),
                    text_offset: 0,
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
    let max_attempts = core.document().hard_line_snapshot().line_count() + 1;
    for _ in 0..max_attempts {
        let before = core.document().source_bytes();
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
        assert!(
            table_policy
                || matches!(&status, CommandStatus::Complete)
                || matches!(&status, CommandStatus::Unsupported(message) if message.contains("work limit")),
            "{status:?}"
        );
        let result = core.document().source_bytes();
        let reopened =
            Document::from_bytes(result.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        if result == before {
            // Whole visual-row table deletion remains the existing open
            // policy. Repeated continuation must end at that explicit refusal,
            // never an internal verification error or silently accepted edit.
            assert!(table_policy);
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
