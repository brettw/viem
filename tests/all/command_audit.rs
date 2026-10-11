use viem_core::command::{CommandStatus, InputEvent, Key, NavigationKey};
use viem_core::document::{Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn editor(source: &str, format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 700., 400.);
    (core, view)
}
fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap_or_else(|error| {
            panic!(
                "{key:?}: {error:?}; source {:?}",
                String::from_utf8_lossy(&core.document().source_bytes())
            )
        });
    let status = result.command.unwrap().status;
    assert!(
        matches!(status, CommandStatus::Complete | CommandStatus::Pending),
        "{key:?}: {status:?}"
    );
}
fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
    for ch in text.chars() {
        key(core, view, Key::Char(ch));
    }
}

#[test]
fn jump_operator_rebases_retained_multibyte_locations_before_deduplicating() {
    for format in [Format::PlainText, Format::MarkdownSource] {
        let (mut core, view) = editor(" 中文", format);
        keys(&mut core, view, "$gg=G");
        assert_eq!(core.document().text(), "中文");
        keys(&mut core, view, "ix");
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, "uu");
        assert_eq!(core.document().source_bytes(), " 中文".as_bytes());
    }
}

#[test]
fn source_line_put_after_empty_terminal_line_keeps_unicode_caret_valid() {
    for word in ["é", "か", "中文", "👩‍💻"] {
        let source = format!("{word}\nb");
        let (mut core, view) = editor(&source, Format::MarkdownSource);
        keys(&mut core, view, "yyGo");
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, "p");
        assert!(core
            .document()
            .text_point(core.command_state(view).unwrap().cursor())
            .is_ok());
        keys(&mut core, view, "u");
    }
}

#[test]
fn source_open_below_last_table_row_keeps_valid_cursor() {
    for source in [
        "| a | b |\n| - | - |\n| 1 | 2 |",
        "> | a | b |\n> | - | - |\n> | 1 | 2 |",
        "- | a | b |\n  | - | - |\n  | 1 | 2 |",
    ] {
        let (mut core, view) = editor(source, Format::MarkdownSource);
        let cursor = core.document().text().find('1').unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: cursor,
                affinity: viem_core::layout::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        keys(&mut core, view, "o");
        assert!(core
            .document()
            .text_point(core.command_state(view).unwrap().cursor())
            .is_ok());
        keys(&mut core, view, "x");
        assert!(
            String::from_utf8(core.document().source_bytes())
                .unwrap()
                .contains("\nx")
                || source.starts_with(['>', '-']),
            "typing follows the newly opened physical row"
        );
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_terminal_line_changes_keep_valid_ordered_ranges() {
    for format in [Format::PlainText, Format::MarkdownSource, Format::Code] {
        for change in ["cc", "S", "Vc"] {
            let (mut core, view) = editor("a\nb\n", format);
            keys(&mut core, view, &format!("G{change}x"));
            key(&mut core, view, Key::Escape);
            assert_eq!(
                core.document().source_bytes(),
                b"a\nb\nx",
                "{format:?} {change}"
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), b"a\nb\n");
        }
    }
}

#[test]
fn selecting_generated_indentation_accepts_it_before_navigation() {
    for format in [Format::PlainText, Format::MarkdownSource, Format::Code] {
        for nav in [
            NavigationKey::Left,
            NavigationKey::Right,
            NavigationKey::Up,
            NavigationKey::Down,
            NavigationKey::Home,
            NavigationKey::End,
        ] {
            let (mut core, view) = editor("fn f() {\n    foo();\n}", format);
            keys(&mut core, view, "jo");
            key(
                &mut core,
                view,
                Key::ModifiedNavigation {
                    key: nav,
                    modifiers: 1,
                },
            );
            assert!(core
                .document()
                .text_point(core.command_state(view).unwrap().cursor())
                .is_ok());
        }
    }
}

#[test]
fn word_end_advances_from_word_end_and_operators_include_next_word() {
    for format in [
        Format::PlainText,
        Format::MarkdownSource,
        Format::Code,
        Format::Markdown,
    ] {
        for motion in ['e', 'E'] {
            let (mut core, view) = editor("alpha beta gamma", format);
            keys(&mut core, view, &format!("{motion}{motion}"));
            assert_eq!(core.command_state(view).unwrap().cursor(), 9);
            keys(&mut core, view, &format!("gg{motion}d{motion}"));
            assert_eq!(core.document().text(), "alph gamma");
            keys(&mut core, view, "u");
            assert_eq!(core.document().text(), "alpha beta gamma");
        }
    }
}

#[test]
fn whitespace_word_objects_preserve_adjacent_line_breaks() {
    for (source, motion, expected) in [
        ("x\n   b\n", "jdiw", "x\nb\n"),
        ("x\n   b\n", "jdaw", "x\n\n"),
        ("x\n\ny\n", "jdiw", "x\n\ny\n"),
    ] {
        for format in [Format::PlainText, Format::Code] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, motion);
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {motion}"
            );
        }
    }
}

#[test]
fn replace_backspace_and_word_delete_never_remove_preexisting_prefix() {
    for format in [
        Format::PlainText,
        Format::MarkdownSource,
        Format::Code,
        Format::Markdown,
    ] {
        let (mut core, view) = editor("First paragraph.", format);
        keys(&mut core, view, "w6lRxyz");
        for _ in 0..5 {
            key(&mut core, view, Key::Backspace);
        }
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), "First paragraph.");
        keys(&mut core, view, "ggw6lRxyz");
        key(&mut core, view, Key::Ctrl('w'));
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().text(), "First paragraph.");
    }
}

#[test]
fn source_replace_restores_exact_markup_that_changes_projection() {
    for (source, prefix) in [("*it*", "l"), ("ab\n\nb", ""), ("a\nb\n\nc", "j")] {
        let (mut core, view) = editor(source, Format::MarkdownSource);
        keys(&mut core, view, &format!("{prefix}R "));
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    for (source, prefix) in [("```\n\n\n```", "ll"), ("    a\n\n    b", "jl")] {
        let (mut core, view) = editor(source, Format::MarkdownSource);
        keys(&mut core, view, &format!("{prefix}Rxy"));
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn command_text_with_multiple_mutations_is_rejected_without_changing_history() {
    let (mut core, view) = editor("abcdef\nghij", Format::PlainText);
    keys(&mut core, view, "iq");
    key(&mut core, view, Key::Escape);
    for text in ["xx", "~~"] {
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::Text(text.into())))
            .unwrap();
        assert!(matches!(
            output.command.unwrap().status,
            CommandStatus::Unsupported(_)
        ));
        assert_eq!(core.document().text(), "qabcdef\nghij");
    }
    keys(&mut core, view, "u");
    assert_eq!(core.document().text(), "abcdef\nghij");
}

#[test]
fn change_line_preserves_indentation_and_undo() {
    for format in [Format::PlainText, Format::MarkdownSource, Format::Code] {
        for command in ["cc", "S", "Vc"] {
            let (mut core, view) = editor("if a {\n    b();\n}\n", format);
            keys(&mut core, view, &format!("j{command}x"));
            key(&mut core, view, Key::Escape);
            assert_eq!(
                core.document().source_bytes(),
                b"if a {\n    x\n}\n",
                "{format:?} {command}"
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), b"if a {\n    b();\n}\n");
        }
    }
}

#[test]
fn layout_line_indentation_uses_buffer_options() {
    for format in [Format::PlainText, Format::Code] {
        for (source, command, expected) in [
            ("a", ">>", "  a"),
            ("\tb", ">>", "    b"),
            ("\t\tb", "<<", "  b"),
            ("  a\n      b", "j==", "  a\n  b"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {command}"
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn insert_indentation_changes_keep_ctrl_u_floor_and_history_valid() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource] {
        for indent in [4, 8] {
            for actions in [
                vec![Key::Ctrl('d')],
                vec![Key::Ctrl('t')],
                vec![Key::Backspace],
                vec![Key::Char('0'), Key::Ctrl('d')],
                vec![Key::Char('^'), Key::Ctrl('d')],
            ] {
                let source = format!("{}abcdefgh\nnext", " ".repeat(indent));
                let (mut core, view) = editor(&source, format);
                keys(&mut core, view, ":set sw=4 sts=4 et");
                key(&mut core, view, Key::Enter);
                core.handle_with_layout(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: indent,
                        affinity: viem_core::document::BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                key(&mut core, view, Key::Char('i'));
                for action in &actions {
                    key(&mut core, view, action.clone());
                }
                key(&mut core, view, Key::Char('x'));
                key(&mut core, view, Key::Ctrl('u'));
                assert_eq!(
                    core.document().source_bytes(),
                    b"abcdefgh\nnext",
                    "{format:?} {indent} {actions:?}"
                );
                key(&mut core, view, Key::Char('y'));
                key(&mut core, view, Key::Escape);
                assert_eq!(core.document().source_bytes(), b"yabcdefgh\nnext");
                key(&mut core, view, Key::Char('u'));
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                key(&mut core, view, Key::Ctrl('r'));
                assert_eq!(core.document().source_bytes(), b"yabcdefgh\nnext");
            }
        }
    }
}

#[test]
fn global_deletes_every_adjacent_empty_target() {
    for format in [Format::PlainText, Format::Code] {
        for (source, command, expected) in [
            ("a\n\n\nb", ":g/^$/d", "a\nb"),
            ("a\n\n\nb\n", ":g/^$/d", "a\nb\n"),
            ("a\n\nb\n\n\nc\n", ":v/a/d", "a\n"),
            ("a\n\nb\n\n\nc", ":v/a/d", "a"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            key(&mut core, view, Key::Enter);
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {command}"
            );
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn literal_word_operator_edges_preserve_separators_and_put_on_empty_line() {
    for format in [Format::PlainText, Format::Code] {
        for (source, command, expected) in [
            ("one two\nthree\n", "wdw", "one \nthree\n"),
            ("one two\n  three\n", "j0db", "one \n  three\n"),
            ("alpha\n", "$dl", "alph\n"),
            ("ab\n\ncd\n", "yljp", "ab\na\ncd\n"),
            ("{\nab\n", "xp", "{\nab\n"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {command}"
            );
        }
    }
}

#[test]
fn partial_last_visual_row_deletion_retains_the_following_separator() {
    use viem_core::document::BoundaryAffinity;
    for command in ["dd", "Vd", "cc", "S"] {
        let (mut core, view) = editor("First paragraph.\nSecond line.", Format::PlainText);
        core.handle(
            view,
            CoreEvent::Resize {
                width: 140.,
                height: 400.,
            },
        )
        .unwrap();
        let row = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.hard_line_index == 0 && !row.wraps_to_next)
            .unwrap()
            .text_range
            .clone();
        assert!(row.start > 0);
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: row.start,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        keys(&mut core, view, command);
        if command == "cc" || command == "S" {
            keys(&mut core, view, "x");
            key(&mut core, view, Key::Escape);
        }
        assert!(
            core.document().text().contains("\nSecond line."),
            "{command}: {:?}",
            core.document().text()
        );
    }
}

#[test]
fn visual_join_reaches_its_handler_in_physical_line_mode() {
    for format in [Format::PlainText, Format::Code] {
        for command in ["VjJ", "VJ", "vJ", "vlJ"] {
            let (mut core, view) = editor("one\ntwo\nthree\n", format);
            core.handle(
                view,
                CoreEvent::SetLineMode(viem_core::command::LineMode::PhysicalSource),
            )
            .unwrap();
            keys(&mut core, view, command);
            assert_eq!(
                core.document().source_bytes(),
                b"one two\nthree\n",
                "{format:?} {command}"
            );
            assert_eq!(
                core.command_state(view).unwrap().mode(),
                viem_core::command::Mode::Normal
            );
        }
    }
}

#[test]
fn line_case_changes_only_letters_and_keeps_source_break_spelling() {
    for (format, source, keys_value, expected) in [
        (Format::MarkdownSource, "a\n\nb", "gUU", "A\n\nb"),
        (Format::MarkdownSource, "a\n\nb", "gUj", "A\n\nB"),
        (Format::Markdown, "x\ny\n\nb", "gUU", "X\nY\n\nb"),
        (
            Format::Markdown,
            "- a\n  ```\n  code\n  ```\n- b",
            "jgUU",
            "- a\n  ```\n  CODE\n  ```\n- b",
        ),
    ] {
        let (mut core, view) = editor(source, format);
        keys(&mut core, view, keys_value);
        assert_eq!(
            core.document().source_bytes(),
            expected.as_bytes(),
            "{format:?} {keys_value}"
        );
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn replace_structural_enter_and_tab_restore_original_source() {
    for source in ["*it*", "**bold**", "a\nb\n\nc", "\n", "   \n  \n"] {
        for replacement in [Key::Enter, Key::Tab] {
            let (mut core, view) = editor(source, Format::MarkdownSource);
            if source.starts_with('*') {
                keys(&mut core, view, "l");
            }
            if source == "a\nb\n\nc" {
                keys(&mut core, view, "j");
            }
            keys(&mut core, view, "R");
            key(&mut core, view, replacement);
            key(&mut core, view, Key::Backspace);
            key(&mut core, view, Key::Escape);
            assert_eq!(
                core.document().source_bytes(),
                source.as_bytes(),
                "{source:?} {replacement:?}"
            );
        }
    }
    for (source, prefix) in [("- a\n  b\n  c", "j2l"), ("> a\n>\n> b", "j")] {
        let (mut core, view) = editor(source, Format::MarkdownSource);
        keys(&mut core, view, &format!("{prefix}R"));
        key(&mut core, view, Key::Tab);
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    for source in ["&amp;", "\\*", "\\[", "a\nb", "a  \nb"] {
        let (mut core, view) = editor(source, Format::Markdown);
        if source.starts_with('a') {
            keys(&mut core, view, "l");
        }
        keys(&mut core, view, "Rx");
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
    for source in ["*it*", "\na", "# Heading #"] {
        let (mut core, view) = editor(source, Format::Markdown);
        keys(&mut core, view, "R");
        key(&mut core, view, Key::Enter);
        key(&mut core, view, Key::Backspace);
        key(&mut core, view, Key::Escape);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn global_literal_line_deletions_publish_one_bounded_transaction() {
    for format in [Format::PlainText, Format::Code] {
        let source = "alpha line\nbeta line\n".repeat(1000);
        let (mut core, view) = editor(&source, format);
        core.handle(
            view,
            CoreEvent::SetLineMode(viem_core::command::LineMode::PhysicalSource),
        )
        .unwrap();
        let before = core.document().revision();
        keys(&mut core, view, ":g/al/d");
        key(&mut core, view, Key::Enter);
        assert_eq!(
            core.document().source_bytes(),
            "beta line\n".repeat(1000).as_bytes()
        );
        assert_eq!(core.document().revision().0, before.0 + 1);
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('1')
                .unwrap()
                .text,
            "alpha line\n"
        );
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn editing_last_nonempty_line_preserves_existing_final_terminator() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource] {
        for (command, expected) in [
            ("jjdd", "a\nb\n"),
            ("jdj", "a\n"),
            ("jddp", "a\nc\nb\n"),
            ("jjyyp", "a\nb\nc\nc\n"),
            ("jjccx", "a\nb\nx\n"),
        ] {
            let (mut core, view) = editor("a\nb\nc\n", format);
            keys(&mut core, view, command);
            if command.contains('x') {
                key(&mut core, view, Key::Escape);
            }
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {command}"
            );
        }
        let (mut core, view) = editor("if a {\n    b();\n}\n", format);
        keys(&mut core, view, "j2ccx");
        key(&mut core, view, Key::Escape);
        assert_eq!(
            core.document().source_bytes(),
            b"if a {\n    x\n",
            "{format:?} counted change"
        );
    }
}

#[test]
fn whitespace_objects_and_insert_word_delete_keep_session_and_line_boundaries() {
    for format in [Format::PlainText, Format::Code] {
        for (source, prefix, expected) in [
            ("x\n\ny\n", "jviwd", "x\ny\n"),
            ("a\n\n\nb\n", "jjciwx", "a\n\nx\nb\n"),
            ("a\n\n\nb\n", "jjcwx", "a\n\nx\nb\n"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, prefix);
            if prefix.contains('x') {
                key(&mut core, view, Key::Escape);
            }
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {prefix}"
            );
        }
        let (mut core, view) = editor("paragraph", format);
        keys(&mut core, view, "5lixyz");
        key(&mut core, view, Key::Ctrl('w'));
        assert_eq!(core.document().text(), "paragraph");
        key(&mut core, view, Key::Ctrl('w'));
        assert_eq!(core.document().text(), "raph");
    }
    let (mut core, view) = editor("x\n\n   y", Format::MarkdownSource);
    keys(&mut core, view, "j0diw");
    assert_eq!(core.document().source_bytes(), b"x\n\ny");
    let (mut core, view) = editor("a<br>b", Format::MarkdownSource);
    keys(&mut core, view, "3lRxyz");
    key(&mut core, view, Key::Ctrl('w'));
    key(&mut core, view, Key::Escape);
    assert_eq!(core.document().source_bytes(), b"a<br>b");
}

#[test]
fn literal_remaining_boundary_commands_follow_vim_without_touching_terminal_breaks() {
    for format in [Format::PlainText, Format::Code] {
        for (source, command, expected) in [
            ("one.\ntwo\n", "v$d", "two\n"),
            ("p1 a.\np1 b.\n\np2\n", "dip", "\np2\n"),
            ("a\n   \n", "jIx", "a\n   x\n"),
            ("ab\ncd\n", "l~x", "a\ncd\n"),
            ("ab\ncd\n", "l~.", "ab\ncd\n"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            if command == "jIx" {
                key(&mut core, view, Key::Escape);
            }
            assert_eq!(
                core.document().source_bytes(),
                expected.as_bytes(),
                "{format:?} {command}"
            );
        }
        let (mut core, view) = editor("alpha beta\n", format);
        keys(&mut core, view, "4lr");
        key(&mut core, view, Key::Enter);
        assert_eq!(core.document().source_bytes(), b"alph\nbeta\n");
        let (mut core, view) = editor("ab\n\ncd\n", format);
        keys(&mut core, view, "w");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        keys(&mut core, view, "wb");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
    }
}

#[test]
fn rich_open_above_preserves_consecutive_heading_and_reference_owners() {
    for (source, target) in [
        ("# A\n## B\n### C\n#### D\n##### E\n###### F", "B"),
        ("\na", "a"),
        ("[ref]: /url \"title\"\n[other]: <x y>", "[other]"),
    ] {
        let (mut core, view) = editor(source, Format::Markdown);
        let cursor = core.document().text().find(target).unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: cursor,
                affinity: viem_core::layout::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        keys(&mut core, view, "Ox");
        key(&mut core, view, Key::Escape);
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(reopened.text(), core.document().text(), "{source:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn rich_linewise_put_preserves_markdown_owners_and_exact_undo() {
    for (source, operation) in [
        ("```\ncode\n```", "ddp"),
        ("**[bold link](u)**", "ddp"),
        ("# Heading #", "yyP"),
        ("# Heading #", "yyp"),
        ("# Heading #", "ddP"),
        ("``code with ` tick``", "yyp"),
        ("\na", "yyp"),
        ("`**not bold**`", "yyp"),
    ] {
        let (mut core, view) = editor(source, Format::Markdown);
        let original = core.document().text().to_owned();
        keys(&mut core, view, operation);
        let expected = if operation.starts_with("dd") {
            original.clone()
        } else if source == "\na" {
            "\n\na".to_owned()
        } else {
            format!("{original}\n{original}")
        };
        assert_eq!(core.document().text(), expected, "{source:?} {operation}");
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(
            reopened.text(),
            core.document().text(),
            "{source:?} {operation}"
        );
        if source.starts_with("# ") {
            assert!(reopened
                .projection()
                .blocks()
                .iter()
                .filter(|block| !block.range.is_empty())
                .all(|block| block.style.0 == "Heading1"));
        }
        if source.starts_with('`') && !source.starts_with("```") {
            assert_eq!(
                reopened
                    .projection()
                    .style_spans()
                    .iter()
                    .filter(|span| span.application
                        == viem_core::document::StyleApplication::Semantic(
                            viem_core::document::SemanticInlineStyle::Code
                        ))
                    .map(|span| span.range.len())
                    .sum::<usize>(),
                original.len() * 2
            );
        }
        keys(
            &mut core,
            view,
            if operation.starts_with("dd") {
                "uu"
            } else {
                "u"
            },
        );
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{source:?} {operation}"
        );
    }
}

#[test]
fn rich_indentation_preserves_literal_reference_definitions() {
    for source in [
        "[ref]: /url \"title\"\n[other]: <x y>",
        "[ref]\n\n[ref]: /url",
        "[a][ref]\n\n[ref]: https://example.com",
    ] {
        let (mut core, view) = editor(source, Format::Markdown);
        keys(&mut core, view, ">>");
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(reopened.text(), core.document().text(), "{source:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn rich_linewise_register_count_and_repeat_preserve_copied_treatments() {
    for source in ["# Heading #\n\nuntouched", "*italic*\n\nuntouched"] {
        let (mut core, view) = editor(source, Format::Markdown);
        let first = core
            .document()
            .text()
            .split('\n')
            .next()
            .unwrap()
            .to_owned();
        keys(&mut core, view, "\"ayy\"a2p");
        assert_eq!(
            core.document().text(),
            format!("{first}\n{first}\n{first}\nuntouched")
        );
        keys(&mut core, view, ".");
        assert_eq!(
            core.document().text(),
            format!("{first}\n{first}\n{first}\n{first}\n{first}\nuntouched")
        );
        keys(&mut core, view, "uu");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn global_independent_edits_batch_once_and_preserve_registers_history() {
    for (format, separator, body, expected_word) in [
        (Format::PlainText, "\n", "d", "beta"),
        (Format::Code, "\n", "d", "beta"),
        (Format::MarkdownSource, "\n", "d", "beta"),
        (Format::Markdown, "\n\n", "d", "beta"),
        (Format::PlainText, "\n", "s/a/z/g", "zlphz"),
    ] {
        let source = (0..1000)
            .flat_map(|_| ["alpha", "beta"])
            .collect::<Vec<_>>()
            .join(separator);
        let (mut core, view) = editor(&source, format);
        keys(&mut core, view, ":set nowrap");
        key(&mut core, view, Key::Enter);
        let before = core.document().revision();
        keys(&mut core, view, &format!(":g/alpha/{body}"));
        key(&mut core, view, Key::Enter);
        assert_eq!(
            core.document().revision().0,
            before.0 + 1,
            "{format:?} {body}"
        );
        if body == "d" {
            assert_eq!(
                core.document().text().split('\n').collect::<Vec<_>>(),
                vec![expected_word; 1000]
            );
            assert_eq!(
                core.command_state(view)
                    .unwrap()
                    .register('"')
                    .unwrap()
                    .text,
                "alpha\n"
            );
        } else {
            assert_eq!(
                core.document().text().split('\n').next(),
                Some(expected_word)
            );
        }
        let reopened =
            Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(reopened.text(), core.document().text());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn rich_linewise_put_inside_multiline_code_retains_the_code_owner() {
    for (target, command, expected) in [
        ("one", "yyp", "one\none\ntwo"),
        ("two", "yyp", "one\ntwo\ntwo"),
        ("one", "ddp", "two\none"),
    ] {
        let source = "```\none\ntwo\n```";
        let (mut core, view) = editor(source, Format::Markdown);
        let cursor = core.document().text().find(target).unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: cursor,
                affinity: viem_core::layout::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        keys(&mut core, view, command);
        assert_eq!(core.document().text(), expected);
        let reopened = Document::from_bytes(
            core.document().source_bytes(),
            Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert_eq!(reopened.text(), expected);
        assert_eq!(reopened.projection().blocks().len(), 1);
        assert_eq!(reopened.projection().blocks()[0].style.0, "Code Block");
        keys(
            &mut core,
            view,
            if command.starts_with("dd") { "uu" } else { "u" },
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}


#[test]
fn fix_verification_word_changes_keep_the_current_word_and_line_ending() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        for (source, command, expected) in [
            ("I saw a cat.\n", "wwcwx", "I saw x cat.\n"),
            ("ab\ncd\nef\n", "lcwx", "ax\ncd\nef\n"),
            ("ab\ncd\nef\n", "lc2wx", "ax\nef\n"),
            ("a\n", "cwx", "x\n"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            key(&mut core, view, Key::Escape);
            assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{format:?} {command}");
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn fix_verification_literal_operators_preserve_paragraph_and_eof_boundaries() {
    for format in [Format::PlainText, Format::Code] {
        for (source, command, expected) in [
            ("First paragraph.\nSecond line.\n\nLast line.", "Gdap", "First paragraph.\nSecond line."),
            ("a\nb\n\nc\n", "Gkdap", "a\nb\n"),
            ("ab\n", "$de", "a\n"),
            ("ab\n", "$dE", "a\n"),
            ("word   \n", "4lde", "word\n"),
            ("one\n\n", "2de", ""),
            ("ab\n", "$yeP", "abb\n"),
            ("ab\ncd\n", "jv$d", "ab\n\n"),
            ("s = '''\nmulti\nline\n'''\n", "jd2w", "s = '''\n'''\n"),
            ("s = '''\nmulti\nline\n'''\n", "jjdb", "s = '''\nline\n'''\n"),
            ("plain\n\ttext\n", "jdd.", ""),
            ("P.\nS.\n\nL.\n", "2jdw", "P.\nS.\nL.\n"),
        ] {
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{format:?} {command}");
        }
    }
}

#[test]
fn fix_verification_ex_unicode_global_and_jump_targets() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        for command in [":d", ":.,$d", ":t0", ":m0"] {
            let (mut core, view) = editor("a\n\nbé", format);
            keys(&mut core, view, "G");
            keys(&mut core, view, command);
            key(&mut core, view, Key::Enter);
        }
        let (mut core, view) = editor("xyz", format);
        keys(&mut core, view, ":v/a/d");
        key(&mut core, view, Key::Enter);
        keys(&mut core, view, "Aéé");
        key(&mut core, view, Key::Escape);
        keys(&mut core, view, ":s/^/#/");
        key(&mut core, view, Key::Enter);
        assert!(core.document().text_point(core.command_state(view).unwrap().cursor()).is_ok());
    }
    for format in [Format::PlainText, Format::Code] {
        let (mut core, view) = editor("a\nb\na", format);
        keys(&mut core, view, ":g/a/d");
        key(&mut core, view, Key::Enter);
        assert_eq!(core.document().source_bytes(), b"b");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), b"a\nb\na");
    }
}

#[test]
fn fix_verification_replace_enter_keeps_indentation_and_source_cursor() {
    for (source, format, command, expected) in [
        (" * star\n", Format::PlainText, "lr", " \n star\n"),
        ("a\né👩‍💻", Format::MarkdownSource, "G0r", "a\n\n👩‍💻"),
    ] {
        let (mut core, view) = editor(source, format);
        keys(&mut core, view, command);
        key(&mut core, view, Key::Enter);
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        assert!(core.document().text_point(core.command_state(view).unwrap().cursor()).is_ok());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn fix_verification_markdown_paragraph_objects_use_semantic_owners() {
    let source = "# Title\n\nFirst para line one\nline two.\n\n- item a\n- item b\n\nLast para.";
    for format in [Format::Markdown, Format::MarkdownSource] {
        for command in ["dap", "dip", "cipX", "d}", ">ap", "yap", "}"] {
            let (mut core, view) = editor(source, format);
            let at = core.document().text().find("First").unwrap();
            core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                text_offset: at, affinity: viem_core::document::BoundaryAffinity::Downstream,
                extend_selection: false }).unwrap();
            keys(&mut core, view, command);
            let after = String::from_utf8(core.document().source_bytes()).unwrap();
            assert!(after.contains("item a") && after.contains("Last para."), "{format:?} {command}: {after:?}");
            assert!(core.command_state(view).unwrap().cursor() < core.document().text().len() - 10);
        }
    }
}


#[test]
fn fix_verification_global_reveals_its_last_affected_line() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        let source = if format == Format::Markdown { "alpha\n\nbeta\n\n".repeat(100) }
            else { "alpha\nbeta\n".repeat(100) };
        let (mut core, view) = editor(&source, format);
        keys(&mut core, view, ":g/al/d");
        key(&mut core, view, Key::Enter);
        let state = core.command_state(view).unwrap();
        let layout = core.layout(view).unwrap();
        let geometry = layout.snapshot().unwrap()
            .logical_endpoint_geometry(state.cursor(), state.boundary_affinity()).unwrap();
        assert!(geometry.rect.y >= layout.viewport_top() && geometry.rect.y < layout.viewport_top() + layout.height());
    }
}

#[test]
fn fix_verification_page_keys_move_when_document_fits_the_viewport() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource, Format::Markdown] {
        for insert in [false, true] {
            let (mut core, view) = editor("a\n\nb\n\nc", format);
            if insert { keys(&mut core, view, "i"); }
            key(&mut core, view, Key::PageDown);
            assert!(core.command_state(view).unwrap().cursor() > 0);
            key(&mut core, view, Key::PageUp);
            assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        }
    }
}


#[test]
fn fix_verification_wrapped_heading_transfer_accepts_before_document_destination() {
    let source = "# A heading containing enough words to wrap across multiple rows\n\nend";
    for command in [":m0", ":t0"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 140., 400.);
        keys(&mut core, view, command);
        key(&mut core, view, Key::Enter);
        assert!(core.document().text_point(core.command_state(view).unwrap().cursor()).is_ok());
        let reopened = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(core.document().text(), reopened.text());
        if command == ":t0" {
            assert_eq!(core.document().projection().blocks().iter()
                .filter(|block| block.style.0 == "Heading1").count(), 2);
            let changed = core.document().source_bytes();
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            key(&mut core, view, Key::Ctrl('r'));
            assert_eq!(core.document().source_bytes(), changed);
        } else {
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn fix_verification_wide_table_ex_endpoints_finish_layout_resolution() {
    let source = "| one | two | three | four | five | six |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |\n| alpha | beta | gamma | delta | epsilon | zeta |";
    for command in [":$d", ":.,$d", ":m$", ":t$"] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 140., 400.);
        keys(&mut core, view, command);
        let result = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert!(!matches!(result.command.unwrap().status, CommandStatus::NeedsMoreLayout(_)));
        assert!(core.document().text_point(core.command_state(view).unwrap().cursor()).is_ok());
    }
}


#[test]
fn markdown_counted_paragraph_objects_keep_exact_undo_units() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for command in ["2dap", "d2ap"] {
            let source = "one\n\ntwo\n\nthree";
            let (mut core, view) = editor(source, format);
            keys(&mut core, view, command);
            assert_eq!(core.document().text(), "three", "{format:?} {command}");
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}
