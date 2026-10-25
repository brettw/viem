use viem_core::document::CharacterProperties;
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

fn open(source: &str, format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 500.);
    (core, view)
}

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: InputEvent) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(input.clone()))
        .unwrap_or_else(|error| {
            let source = String::from_utf8_lossy(&core.document().source_bytes())
                .chars()
                .take(200)
                .collect::<String>();
            let cursor = core.command_state(view).unwrap().cursor();
            panic!("{input:?} at {cursor}, source prefix {source:?}: {error:?}")
        });
    if let Some(command) = result.command {
        assert!(
            matches!(
                command.status,
                CommandStatus::Complete | CommandStatus::Pending
            ) || matches!(input, InputEvent::Key(Key::Escape))
                && command.status == CommandStatus::Cancelled,
            "{input:?}: {:?}",
            command.status
        );
    }
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, offset: usize) {
    core.handle_with_layout(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
}

fn verify_history(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, source: &str) {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
        .unwrap();
    let bytes = core.document().source_bytes();
    let reopened =
        Document::from_bytes(bytes.clone(), Encoding::Utf8, core.document().format()).unwrap();
    assert_eq!(core.document().text(), reopened.text());
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), bytes);
}

fn verification_rich_case(source: &str, keys: &str, expected: &str) {
    let (mut core, view) = open(source, Format::Markdown);
    for key in keys.chars() { input(&mut core, view, InputEvent::key(key)); }
    assert_eq!(core.document().text(), expected, "{source:?}: {keys}");
    let bytes = core.document().source_bytes();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains("[]()"), "{text:?}");
    if source.starts_with("| abc") { assert!(!text.contains("**"), "{text:?}"); }
    if source.starts_with("p\n") { assert_eq!(text, "```\nc2\n```\n\nz"); }
    let fresh = Document::from_bytes(bytes, Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(core.document().text(), fresh.text());
    if keys != "xp" { verify_history(&mut core, view, source); }
}

#[test]
fn verification_setext_put() { verification_rich_case("Setext\n===", "yyp", "Setext\nSetext"); }
#[test]
fn verification_code_adjacent_put() { verification_rich_case("a `b` c `d` e", "xp", " ab c d e"); }
#[test]
fn verification_bold_link_row_delete() { verification_rich_case("**[bold link](u)**\n\nx", "dd", "x"); }
#[test]
fn verification_delete_into_code() { verification_rich_case("p\n\n```\nc1\nc2\n```\n\nz", "2dd", "c2\nz"); }
#[test]
fn verification_autolink_change() { verification_rich_case("<https://auto.link>", "ccx", "x"); }
#[test]
fn verification_final_autolink_delete() { verification_rich_case("x\n\n<https://auto.link>", "Gdd", "x"); }
#[test]
fn verification_empty_link_change() { verification_rich_case("[text]()", "Cx", "x"); }
#[test]
fn verification_header_cell_change() { verification_rich_case("| abc |\n| --- |", "ciwz", "z"); }

#[test]
fn verification_reference_and_indented_code_transfers() {
    for (source, prefix, expected) in [
        // The copied definition is an inactive duplicate; the use still
        // resolves and shows its label.
        ("[ref]\n\n[ref]: /url", "G", "ref\n[ref]: /url\n[ref]: /url"),
        ("    a\n\n    b", "", "a\na\n\nb"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        for key in format!("{prefix}:t.").chars() { input(&mut core, view, InputEvent::key(key)); }
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn emptied_fence_can_be_typed_again() {
    for source in [
        "```\nx\n```",
        "```rust\nx\n```",
        "before\n\n```\nx\n```\n\nafter",
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        let end = core.document().text().find('x').unwrap() + 1;
        place(&mut core, view, end);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        input(&mut core, view, InputEvent::text("y"));
        assert_eq!(
            core.document().source_bytes(),
            source.replace('x', "y").as_bytes()
        );
        verify_history(&mut core, view, source);
    }
}

#[test]
fn semantic_selection_formatting_handles_paragraphs_and_edge_spaces() {
    use viem_core::document::{ModelRequest, SemanticInlineStyle, StyleApplication};
    for (source, range) in [
        ("one\n\ntwo", 0..7),
        ("# A\n## B", 0..3),
        ("- a\n- b", 0..3),
        ("one two", 0..4),
        ("one two", 3..7),
        ("one two three", 4..8),
        ("Line one\nLine two", 1..14),
        ("a &amp; b", 0..5),
        ("a<br>b", 0..3),
        ("<ins>under</ins>", 0..3),
    ] {
        for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let text = document.text().to_owned();
            document
                .apply_model_request(ModelRequest::SetSemanticStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: range.clone(),
                    style,
                    enabled: true,
                })
                .unwrap_or_else(|error| panic!("{source:?} {range:?} {style:?}: {error:?}"));
            assert_eq!(document.text(), text);
            for (offset, ch) in text
                .char_indices()
                .filter(|(at, ch)| range.contains(at) && *ch != '\n')
            {
                assert!(
                    document
                        .projection()
                        .style_spans()
                        .iter()
                        .any(|span| span.range.contains(&offset)
                            && span.application == StyleApplication::Semantic(style)),
                    "{source:?} at {offset}: {ch}"
                );
            }
            let after = document.source_bytes();
            let fresh =
                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(fresh.text(), text);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}

#[test]
fn empty_list_items_can_indent_before_typing() {
    for source in ["- a\n- ", "- a\n- \n- c", "> - a\n> - \n> - c"] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 2);
        input(&mut core, view, InputEvent::Key(Key::Tab));
        input(&mut core, view, InputEvent::text("child"));
        let text = core.document().text().to_owned();
        assert!(text.starts_with("a\nchild"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn hard_break_before_folded_source_ending_preserves_retained_space() {
    for (source, at) in [
        ("a\nb", 1),
        ("a\nb\nc", 3),
        ("> a\n> b", 1),
        ("- a\n  b", 1),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let mut expected = core.document().text().to_owned();
        expected.insert(at, '\n');
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn code_change_line_retains_its_existing_empty_fence() {
    for source in [
        "```\ncode\n```",
        "```rust\ncode\n```",
        "before\n\n```\ncode\n```\n\nafter",
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let at = core.document().text().find("code").unwrap();
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::key('c'));
        input(&mut core, view, InputEvent::key('c'));
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(
            core.document().source_bytes(),
            source.replace("code", "x").as_bytes()
        );
        verify_history(&mut core, view, source);
    }
}

#[test]
fn enter_splits_inline_code_scope_and_can_undo_exactly() {
    for source in ["`code`", "before `code` after", "`a\nb`"] {
        let (mut core, view) = open(source, Format::Markdown);
        let at = if source.contains("code") {
            core.document().text().find("code").unwrap() + 1
        } else {
            1
        };
        let mut expected = core.document().text().to_owned();
        expected.insert(at, '\n');
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn paragraph_joins_preserve_emphasis_and_literal_delimiters() {
    for (source, at) in [("*i*\n\n*t*", 2), ("_i_\n\n_t_", 2), ("**a\n\nb**", 4)] {
        let (mut core, view) = open(source, Format::Markdown);
        let mut expected = core.document().text().to_owned();
        expected.remove(at - 1);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn final_empty_table_cell_line_keeps_its_insertion_inside_the_cell() {
    for suffix in ["\n\ntext after", "\n\n| x | y |\n| - | - |\n| q | r |", ""] {
        let source = format!("| a | b |\n| - | - |\n| 1 | 2 |{suffix}");
        let (mut core, view) = open(&source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 7);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(
            core.document().source_bytes(),
            source.replace("2 |", "2<br>x |").as_bytes()
        );
        verify_history(&mut core, view, &source);
    }
}

#[test]
fn empty_heading_list_items_accept_text_and_empty_quotes_accept_spaces() {
    for (source, typed) in [
        ("- # ", "x"),
        ("- #", "x"),
        ("1. # ", "x"),
        ("- ## ", "x"),
        (">", " "),
        ("> ", "\t"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 0);
        input(&mut core, view, InputEvent::text(typed));
        assert_eq!(core.document().text(), typed, "{source:?}");
        verify_history(&mut core, view, source);
    }
}

#[test]
fn upstream_after_fold_keeps_hidden_container_prefix_outside_text() {
    for source in ["> a\n> b", "- a\n  b", "a\n  b", "> a\n> *b*"] {
        let (mut core, view) = open(source, Format::Markdown);
        let mut expected = core.document().text().to_owned();
        expected.insert(2, 'x');
        input(&mut core, view, InputEvent::key('i'));
        core.handle_with_layout(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: 2,
                affinity: BoundaryAffinity::Upstream,
                extend_selection: false,
            },
        )
        .unwrap();
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn retained_whitespace_survives_typing_into_otherwise_blank_lines() {
    for (source, at) in [
        (" ", 1),
        ("\t", 1),
        ("   \n  \n", 2),
        ("a\n   ", 5),
        ("abc\n\nend\n  ", 10),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let at = at.min(core.document().text().len());
        let mut expected = core.document().text().to_owned();
        expected.insert(at, 'x');
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(core.document().text(), expected, "{source:?}");
        verify_history(&mut core, view, source);
    }
}

#[test]
fn empty_heading_and_list_markers_accept_hard_breaks() {
    for source in ["#", "##", "-", "- #", "> #", "1."] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 0);
        input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(core.document().text(), "\nx", "{source:?}");
        verify_history(&mut core, view, source);
    }
}

#[test]
fn deleting_last_list_continuation_character_preserves_retained_folded_space() {
    for (source, at) in [("- a\n  b", 3), ("- ab\n  c\n- d", 4), ("> - a\n>   b", 3)] {
        let (mut core, view) = open(source, Format::Markdown);
        let mut expected = core.document().text().to_owned();
        expected.remove(at - 1);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn change_line_does_not_duplicate_retained_quote_markers() {
    for source in ["> # h", "1. > heading", "- > # h"] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('c'));
        input(&mut core, view, InputEvent::key('c'));
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(core.document().text(), "x", "{source:?}");
        assert_eq!(
            core.document().source_bytes(),
            source
                .replace(
                    if source.ends_with("heading") {
                        "heading"
                    } else {
                        "h"
                    },
                    "x"
                )
                .as_bytes()
        );
        verify_history(&mut core, view, source);
    }
    for source in ["> # > x", "- > # > x"] {
        let (core, _) = open(source, Format::Markdown);
        assert_eq!(core.document().text(), "> x", "{source:?}");
    }
}

#[test]
fn punctuation_typed_after_inline_code_fold_remains_literal_code() {
    for typed in [">", "#", "*", "[", "_", "-", "`", "&", "<", "~", "!"] {
        let source = "`a\nb`";
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 2);
        input(&mut core, view, InputEvent::text(typed));
        assert_eq!(core.document().text(), format!("a {typed}b"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn setext_heading_full_text_change_retains_heading_and_deletion_retains_following_prose() {
    for source in [
        "Setext\n===",
        "Setext\n---",
        "Setext\n===\n\nBody text.",
        "Setext\nline two\n===",
    ] {
        for command in ["cc", "cw", "dd"] {
            let (mut core, view) = open(source, Format::Markdown);
            for key in command.chars() {
                input(&mut core, view, InputEvent::key(key));
            }
            if command != "dd" {
                input(&mut core, view, InputEvent::text("x"));
                assert!(core.document().text().starts_with('x'));
                assert!(core.document().projection().blocks()[0]
                    .style
                    .0
                    .starts_with("Heading"));
            } else if source.contains("Body") {
                assert_eq!(core.document().text(), "Body text.");
                assert_eq!(
                    core.document().projection().blocks()[0].style.0,
                    "Paragraph"
                );
            } else {
                assert_eq!(core.document().text(), "");
            }
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn typing_space_beside_atx_closing_hashes_keeps_it_visible() {
    for source in ["# Heading #", "- # Heading #", "> # Heading #"] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 7);
        input(&mut core, view, InputEvent::text("#"));
        input(&mut core, view, InputEvent::text(" "));
        assert_eq!(core.document().text(), "Heading# ");
        verify_history(&mut core, view, source);
    }
}

#[test]
fn unsupported_visual_table_joins_never_panic_or_leave_partial_source_edits() {
    for body in ["*c*", "**c**", "c**c**"] {
        for join in ["J", "gJ"] {
            let source = format!("| a | b |\n| - | - |\n| {body} | d |");
            let (mut core, view) = open(&source, Format::Markdown);
            place(&mut core, view, 2);
            input(&mut core, view, InputEvent::key('v'));
            input(&mut core, view, InputEvent::key('j'));
            for key in join.chars() {
                let _ = core.handle_with_layout(view, CoreEvent::Input(InputEvent::key(key)));
            }
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            input(&mut core, view, InputEvent::Key(Key::Escape));
            input(&mut core, view, InputEvent::key('i'));
            input(&mut core, view, InputEvent::text("x"));
        }
    }
}

#[test]
fn retiring_first_reference_definition_keeps_following_definition_separate() {
    let source = "[ref]: /url \"title\"\n[other]: <x y>";
    for event in [
        InputEvent::text("x"),
        InputEvent::text(" "),
        InputEvent::Key(Key::Delete),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let expected = if matches!(event, InputEvent::Key(Key::Delete)) {
            core.document().text()[1..].to_owned()
        } else if matches!(event, InputEvent::Text(ref value) if value == " ") {
            format!(" {}", core.document().text())
        } else {
            format!("x{}", core.document().text())
        };
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, event);
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn heading_and_list_assignment_preserve_body_and_container_content() {
    use viem_core::document::ListStyle;
    for source in [
        "trailing spaces   ",
        "a\\\nb",
        "a  \nb",
        "a<br>b",
        "> a\n> b",
        "> > nested\n> back",
        "- a\n  b\n  c",
        "- > quote in list",
        "text\n    not code (lazy)",
        "`a\nb`",
        "Setext\n===",
        "# Heading #",
        " ",
        "\t",
        "` `",
    ] {
        for list in [None, Some(ListStyle::Bullet), Some(ListStyle::Numbered)] {
            let (mut core, view) = open(source, Format::Markdown);
            let text = core.document().text().to_owned();
            let expected = core.list_selection_identity(view).unwrap();
            let event = if let Some(style) = list {
                CoreEvent::SetListStyle {
                    expected,
                    style: Some(style),
                }
            } else {
                CoreEvent::SetParagraphStyle {
                    expected,
                    style: "Heading2".into(),
                }
            };
            core.handle_with_layout(view, event)
                .unwrap_or_else(|error| panic!("{source:?}, {list:?}: {error:?}"));
            assert_eq!(core.document().text(), text, "{source:?}, {list:?}");
            if core.document().source_bytes() != source.as_bytes() {
                verify_history(&mut core, view, source);
            }
        }
    }
}

#[test]
fn whitespace_before_visible_comments_stays_visible() {
    for source in [
        "<!-- comment -->",
        "<!-- unclosed comment",
        "<!--\ncomment\n\nspanning\n-->",
        "<script>alert(1)</script>",
    ] {
        for typed in [" ", "\t"] {
            let (mut core, view) = open(source, Format::Markdown);
            let expected = format!("{typed}{}", core.document().text());
            input(&mut core, view, InputEvent::key('i'));
            input(&mut core, view, InputEvent::text(typed));
            assert_eq!(core.document().text(), expected);
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn splitting_after_a_leading_empty_paragraph_preserves_both_boundaries() {
    // A leading empty paragraph needs a pair of endings; one is trivia.
    for source in ["\n\na", "\n\nab", "\n\n\na", " \n \na"] {
        let (mut core, view) = open(source, Format::Markdown);
        let original = core.document().text().to_owned();
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 1);
        input(&mut core, view, InputEvent::Key(Key::Enter));
        assert_eq!(core.document().text(), format!("\n{original}"));
        input(&mut core, view, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document().text(), original);
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(core.document().text(), format!("\nx{}", &original[1..]));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn typing_at_the_start_of_a_retained_trailing_whitespace_row_has_no_extra_fold() {
    // The whitespace row is retained trivia, never visible paragraph text.
    let source = "abc\n\nend\n  ";
    let (mut core, view) = open(source, Format::Markdown);
    assert_eq!(core.document().text(), "abc\nend");
    input(&mut core, view, InputEvent::key('i'));
    place(&mut core, view, 7);
    input(&mut core, view, InputEvent::text("x"));
    assert_eq!(core.document().text(), "abc\nendx");
    verify_history(&mut core, view, source);
}

#[test]
fn native_whole_paragraph_replacement_preserves_nested_owners_and_literal_punctuation() {
    for source in [
        "# abc",
        "> abc",
        "- # heading",
        "1. > # heading",
        "1. > heading",
        "Setext\n===",
    ] {
        for typed in ["x", "*", "-", "#", "> "] {
            let (mut core, view) = open(source, Format::Markdown);
            let before = core.document().projection().blocks()[0].clone();
            core.handle_with_layout(view, CoreEvent::SetMarkdownAutodetect(false))
                .unwrap();
            input(&mut core, view, InputEvent::key('i'));
            let end = core.document().text().len();
            place(&mut core, view, 0);
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: end,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: true,
                },
            )
            .unwrap();
            input(&mut core, view, InputEvent::text(typed));
            assert_eq!(core.document().text(), typed, "{source:?}");
            let after = &core.document().projection().blocks()[0];
            assert_eq!(after.style, before.style, "{source:?}");
            assert_eq!(after.quote_depth, before.quote_depth, "{source:?}");
            assert_eq!(
                after
                    .containers
                    .iter()
                    .map(|item| item.container.kind)
                    .collect::<Vec<_>>(),
                before
                    .containers
                    .iter()
                    .map(|item| item.container.kind)
                    .collect::<Vec<_>>(),
                "{source:?}"
            );
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn table_join_after_column_insert_and_native_selection_cannot_reverse_source_slices() {
    use viem_core::command::NavigationKey;
    use viem_core::document::TableEditIntent;
    let (mut core, view) = open("| a |\n| - |\n| 1 |\n| 2 |\n| 3 |\n| 4 |", Format::Markdown);
    core.handle_with_layout(
        view,
        CoreEvent::Resize {
            width: 300.,
            height: 60.,
        },
    )
    .unwrap();
    let table = core.document().projection().tables()[0].id;
    core.handle_with_layout(
        view,
        CoreEvent::TableEdit {
            document: core.document().id(),
            revision: core.document().revision(),
            intent: TableEditIntent::InsertColumn {
                table,
                column: 0,
                after: false,
            },
        },
    )
    .unwrap();
    for event in [
        InputEvent::Key(Key::Down),
        InputEvent::text("**"),
        InputEvent::Key(Key::ModifiedNavigation {
            key: NavigationKey::Up,
            modifiers: 1,
        }),
        InputEvent::Key(Key::WordRight),
        InputEvent::Key(Key::Escape),
        InputEvent::key('v'),
        InputEvent::key('j'),
    ] {
        let _ = core.handle_with_layout(view, CoreEvent::Input(event));
    }
    let before = core.document().source_bytes();
    let _ = core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('J')));
    assert_eq!(core.document().source_bytes(), before);
    let _ = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Escape)));
    input(&mut core, view, InputEvent::key('i'));
    input(&mut core, view, InputEvent::text("x"));
}

#[test]
fn pending_emphasis_can_style_new_text_inside_inline_code() {
    use viem_core::document::{SemanticInlineStyle, StyleApplication};
    for source in [
        "`code`",
        "# `code` heading",
        "[`code` link](u)",
        "``code with ` tick``",
        "`**not bold**`",
        "<https://auto.link>",
        "https://bare.link.example",
        "trailing backslash\\",
    ] {
        for style in [SemanticInlineStyle::Strong, SemanticInlineStyle::Emphasis] {
            let (mut core, view) = open(source, Format::Markdown);
            let original = core.document().text().to_owned();
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
            input(&mut core, view, InputEvent::text("x"));
            assert_eq!(core.document().text(), format!("x{original}"));
            assert!(core
                .document()
                .projection()
                .style_spans()
                .iter()
                .any(|span| span.range.contains(&0)
                    && span.application == StyleApplication::Semantic(style)));
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn consecutive_reference_definitions_accept_tab_and_shift_enter() {
    let source = "[ref]: /url \"title\"\n[other]: <x y>";
    for event in [InputEvent::Key(Key::Tab), InputEvent::Key(Key::ShiftEnter)] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        input(&mut core, view, event);
        input(&mut core, view, InputEvent::text("x"));
        assert!(core.document().text().ends_with("[other]: <x y>"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn code_typing_choice_preserves_literal_punctuation_and_passive_inline_contexts() {
    use viem_core::document::{SemanticInlineStyle, StyleApplication, StyleNamespace};
    for (source, at) in [
        ("`unclosed code", 0),
        ("trailing backslash\\", 19),
        ("<p align=\"center\">x</p>", 0),
        ("<ins>under</ins>", 0),
        ("**`code in bold`**", 1),
        ("https://bare.link.example", 8),
        ("- a\n  - nested", 3),
        ("<div>\n**md inside html**\n</div>", 1),
        ("| a \\| b |\n| --- |", 2),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let original = core.document().text().to_owned();
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        let expected = core.list_selection_identity(view).unwrap();
        core.handle_with_layout(
            view,
            CoreEvent::AssignNamedStyle {
                expected,
                style_sheet_revision: core.document().projection().style_sheet().revision,
                namespace: StyleNamespace::Character,
                style: "Code".into(),
            },
        )
        .unwrap();
        input(&mut core, view, InputEvent::text("x"));
        assert_eq!(
            core.document().text(),
            format!("{}x{}", &original[..at], &original[at..])
        );
        assert!(core
            .document()
            .projection()
            .style_spans()
            .iter()
            .any(|span| span.range.contains(&at)
                && (span.application == StyleApplication::Semantic(SemanticInlineStyle::Code)
                    || span.application == StyleApplication::Named("Code".into()))));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn literal_html_and_multiline_comments_accept_ordinary_typing_and_breaks() {
    let comment = "<!--\ncomment\n\nspanning\n-->";
    for (source, at, typed) in [
        ("</closing-only>", 0, "\\"),
        ("<unknown-tag attr>", 0, "\\"),
        ("<script>alert(1)</script>", 17, "&"),
        ("<script>alert(1)</script>", 17, "<"),
        (comment, 0, "```"),
        (comment, 0, "#"),
        (comment, 0, "<"),
        (comment, 4, "\n"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        let original = core.document().text().to_owned();
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, at);
        input(&mut core, view, InputEvent::text(typed));
        assert_eq!(
            core.document().text(),
            format!("{}{typed}{}", &original[..at], &original[at..])
        );
        input(&mut core, view, InputEvent::text("y"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn native_selection_replacement_preserves_single_tilde_strikethrough() {
    use viem_core::document::StyleApplication;
    for (source, start) in [("~strike~", 0), ("a ~strike~ b", 2)] {
        for count in [1, 2] {
            let (mut core, view) = open(source, Format::Markdown);
            input(&mut core, view, InputEvent::key('i'));
            let mut expected = core.document().text().to_owned();
            expected.replace_range(start..start + count, "x");
            core.handle_with_layout(
                view,
                CoreEvent::BeginPointerGesture {
                    document_revision: core.document().revision(),
                    text_offset: start,
                    affinity: BoundaryAffinity::Downstream,
                },
            )
            .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: start + count,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: true,
                },
            )
            .unwrap();
            input(&mut core, view, InputEvent::text("x"));
            assert_eq!(core.document().text(), expected);
            assert!(core
                .document()
                .projection()
                .style_spans()
                .iter()
                .any(|span| span.range.contains(&start)
                    && span.application == StyleApplication::direct(CharacterProperties { strikethrough: Some(true), ..Default::default() })));
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn change_commands_preserve_deep_container_markers() {
    for (source, keys) in [
        ("> - > - > a\n> - > - > b", "C"),
        ("1. > # heading\n   > text\n2. b", "cc"),
        ("1. > # heading\n   > text\n2. b", "S"),
        ("1. > # heading\n   > text\n2. b", "ciw"),
        ("1. > # heading\n   > text\n2. b", "Vjc"),
        ("1. > # heading\n   > text\n2. b", "O"),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        for key in keys.chars() {
            input(&mut core, view, InputEvent::key(key));
        }
        input(&mut core, view, InputEvent::text("x"));
        assert!(
            !core.document().text().contains(['>', '#']),
            "{keys}: {:?}",
            core.document().text()
        );
        verify_history(&mut core, view, source);
    }
}

#[test]
fn list_assignment_wraps_code_without_clearing_its_leaf_style() {
    use viem_core::document::{ListStyle, ModelRequest};
    for source in [
        "```js\nx\ny\n```",
        "```\nx\n\ny",
        "> ```js\n> x\n> ```",
        "    x\n    y",
    ] {
        for style in [ListStyle::Bullet, ListStyle::Numbered] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let text = document.text().to_owned();
            document
                .apply_model_request(ModelRequest::SetListStyle {
                    document: document.id(),
                    revision: document.revision(),
                    range: 0..0,
                    style: Some(style),
                })
                .unwrap_or_else(|error| panic!("{source:?} {style:?}: {error:?}"));
            assert_eq!(document.text(), text);
            assert_eq!(document.projection().blocks()[0].style.0, "Code Block");
            assert!(matches!(
                document.projection().blocks()[0].kind,
                viem_core::document::BlockKind::ListItem { .. }
            ));
            let after = document.source_bytes();
            assert_eq!(
                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown)
                    .unwrap()
                    .text(),
                text
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), after);
        }
    }
}

#[test]
fn reference_definition_interior_hard_breaks_keep_literal_text_editable() {
    for source in ["[ref]: /url", "[ref]: /url \"title\"\n[other]: <x y>"] {
        let initial =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap()
                .text()
                .to_owned();
        for at in 0..=initial.len() {
            let (mut core, view) = open(source, Format::Markdown);
            input(&mut core, view, InputEvent::key('i'));
            place(&mut core, view, at);
            input(&mut core, view, InputEvent::Key(Key::ShiftEnter));
            input(&mut core, view, InputEvent::text("x"));
            assert_eq!(
                core.document().text(),
                format!("{}\nx{}", &initial[..at], &initial[at..]),
                "{source:?}@{at}"
            );
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn passive_pre_blocks_accept_literal_break_then_typing() {
    let source = "<pre>\npre block\n\n</pre>\n\nafter";
    for key in [
        InputEvent::text("```"),
        InputEvent::text("#"),
        InputEvent::text("<"),
        InputEvent::Key(Key::Tab),
        InputEvent::Key(Key::ShiftEnter),
    ] {
        let (mut core, view) = open(source, Format::Markdown);
        input(&mut core, view, InputEvent::key('i'));
        place(&mut core, view, 11);
        input(&mut core, view, key);
        input(&mut core, view, InputEvent::text("x"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn typing_in_a_blank_paragraph_after_an_unclosed_comment_matches_reopening() {
    for source in ["<!-- unclosed comment", "<!-- unclosed **literal"] {
        let (mut core, view) = open(source, Format::Markdown);
        core.handle_with_layout(view, CoreEvent::SetMarkdownAutodetect(false))
            .unwrap();
        input(&mut core, view, InputEvent::key('o'));
        input(&mut core, view, InputEvent::text("---"));
        assert_eq!(core.document().text(), format!("{source}\n---"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn reference_definition_edits_keep_following_definitions_separate() {
    let source = "[ref]: /url \"title\"\n[other]: <x y>";
    for at in 0..=source.len() {
        for event in [
            InputEvent::text("one\ntwo"),
            InputEvent::Key(Key::Backspace),
        ] {
            let (mut core, view) = open(source, Format::Markdown);
            input(&mut core, view, InputEvent::key('i'));
            place(&mut core, view, at);
            let mut expected = core.document().text().to_owned();
            match &event {
                InputEvent::Text(text) => expected.insert_str(at, text),
                _ if at > 0 => {
                    expected.remove(at - 1);
                }
                _ => {}
            }
            input(&mut core, view, event);
            assert_eq!(core.document().text(), expected, "at {at}");
            input(&mut core, view, InputEvent::text("x"));
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn visible_multiline_comments_keep_literal_internal_breaks() {
    let source = "<!--\ncomment\n\nspanning\n-->";
    for at in [0, 4, 5, 8, 12, 13, 14, 21, source.len()] {
        for event in [
            InputEvent::Key(Key::Enter),
            InputEvent::Key(Key::ShiftEnter),
            InputEvent::text("one\ntwo"),
        ] {
            let (mut core, view) = open(source, Format::Markdown);
            input(&mut core, view, InputEvent::key('i'));
            place(&mut core, view, at);
            let mut expected = source.to_owned();
            expected.insert_str(
                at,
                match &event {
                    InputEvent::Text(text) => text,
                    _ => "\n",
                },
            );
            let hard_break = matches!(event, InputEvent::Key(Key::ShiftEnter));
            input(&mut core, view, event);
            assert_eq!(core.document().text(), expected, "comment break at {at}");
            if hard_break {
                assert_eq!(core.document().projection().blocks().len(), 1);
            }
            input(&mut core, view, InputEvent::text("x"));
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn native_selection_typeover_and_enter_preserve_special_boundaries() {
    for (source, range) in [
        ("\n\na", 1..2),
        (" \n \na", 1..2),
        ("# Heading #", 0..3),
        ("Title\n-----", 0..3),
        ("[ref]: /url \"title\"\n[other]: <x y>", 0..3),
    ] {
        for event in [
            InputEvent::text("x"),
            InputEvent::text("one\ntwo"),
            InputEvent::Key(Key::Enter),
        ] {
            let (mut core, view) = open(source, Format::Markdown);
            input(&mut core, view, InputEvent::key('i'));
            core.handle_with_layout(
                view,
                CoreEvent::BeginPointerGesture {
                    document_revision: core.document().revision(),
                    text_offset: range.start,
                    affinity: BoundaryAffinity::Downstream,
                },
            )
            .unwrap();
            core.handle_with_layout(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: range.end,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: true,
                },
            )
            .unwrap();
            input(&mut core, view, event);
            input(&mut core, view, InputEvent::text("y"));
            verify_history(&mut core, view, source);
        }
    }
}

#[test]
fn deleting_code_rows_before_an_indented_block_preserves_remaining_text() {
    let source = "  ```\n  This fence is indented.\n    Two additional spaces.\n  ```\n\n    Four spaces code.";
    for row in [0, 1] {
        let (mut core, view) = open(source, Format::Markdown);
        let initial = core.document().text().to_owned();
        let first_end = initial.find('\n').unwrap() + 1;
        let range = if row == 0 {
            0..first_end
        } else {
            first_end..initial[first_end..].find('\n').unwrap() + first_end + 1
        };
        let mut expected = initial.clone();
        expected.replace_range(range.clone(), "");
        place(&mut core, view, range.start);
        input(&mut core, view, InputEvent::key('d'));
        input(&mut core, view, InputEvent::key('d'));
        assert_eq!(core.document().text(), expected);
        verify_history(&mut core, view, source);
    }
}

#[test]
fn repository_paragraph_deletion_preserves_later_code_paragraphs() {
    let source = include_str!("../../docs/markdown_demo.md");
    for keys in ["dip", "dap", "vipd"] {
        let (mut core, view) = open(source, Format::Markdown);
        for key in keys.chars() {
            input(&mut core, view, InputEvent::key(key));
        }
        assert!(core.document().text().contains("This fence is indented by two spaces."));
        assert!(core.document().text().contains("Four spaces"));
        assert!(!core.document().text().contains("Viem Markdown demo"));
        verify_history(&mut core, view, source);
    }
}

#[test]
fn repository_fence_line_commands_use_current_content_positions() {
    let source = include_str!("../../docs/markdown_demo.md");
    for target in [
        "This fence is indented by two spaces.",
        "  These two additional spaces",
    ] {
        for keys in ["dd", "D", "J", "Vjd", ":d"] {
            let (mut core, view) = open(source, Format::Markdown);
            let at = core.document().text().find(target).unwrap();
            place(&mut core, view, at);
            for key in keys.chars() {
                input(&mut core, view, InputEvent::key(key));
            }
            if keys.starts_with(':') {
                input(&mut core, view, InputEvent::Key(Key::Enter));
            }
            verify_history(&mut core, view, source);
        }
    }
}
