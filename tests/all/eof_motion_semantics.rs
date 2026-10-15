use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn editor(text: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::new(text));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 300.0);
    (core, view)
}

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for key in input.chars() {
        let outcome = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .expect("command dispatch succeeds");
        let status = outcome.command.expect("input produces a command").status;
        assert!(
            matches!(status, CommandStatus::Complete | CommandStatus::Pending),
            "{key:?} returned {status:?}"
        );
    }
}

#[test]
fn terminal_word_motion_clamps_cursor_but_delete_uses_exclusive_eof() {
    let text = "a\u{301}bc";
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, "w");
    assert_eq!(normal.command_state(normal_view).unwrap().cursor(), 4);

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "dw");
    assert_eq!(operator.document().text(), "");
}

#[test]
fn terminal_sentence_motion_clamps_to_punctuation_but_operator_consumes_eof() {
    let text = "One sentence!";
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, ")");
    assert_eq!(
        normal.command_state(normal_view).unwrap().cursor(),
        text.find('!').unwrap()
    );

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "d)");
    assert_eq!(operator.document().text(), "");
}

#[test]
fn terminal_paragraph_motion_clamps_to_final_grapheme_but_operator_consumes_eof() {
    let text = "aaa\nbbb\n終\u{301}";
    let final_grapheme = text.find('終').unwrap();
    let (mut normal, normal_view) = editor(text);
    keys(&mut normal, normal_view, "}");
    assert_eq!(
        normal.command_state(normal_view).unwrap().cursor(),
        final_grapheme
    );

    let (mut operator, operator_view) = editor(text);
    keys(&mut operator, operator_view, "d}");
    assert_eq!(operator.document().text(), "");
}

#[test]
fn literal_final_terminator_is_not_an_extra_normal_or_ex_line() {
    use viem_core::command::LineMode;
    use viem_core::document::{Encoding, FileFormat, Format};
    for format in [Format::PlainText, Format::Code] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let ending = match file_format { FileFormat::Unix => "\n", FileFormat::Dos => "\r\n", FileFormat::Mac => "\r" };
                let original = format!("a{ending}b{ending}c{ending}");
                for (command, expected) in [
                    ("$d", format!("a{ending}b{ending}")),
                    ("2m$", format!("a{ending}c{ending}b{ending}")),
                    ("%norm Ax", format!("ax{ending}bx{ending}cx{ending}")),
                    ("g/^$/d", original.clone()),
                ] {
                    let document = Document::from_bytes_with_file_format(original.as_bytes().to_vec(), Encoding::Utf8, format, file_format).unwrap();
                    let mut core = Core::new(document);
                    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
                    core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
                    keys(&mut core, view, "G");
                    assert_eq!(core.command_state(view).unwrap().cursor(), 4, "{format:?}, {mode:?}");
                    keys(&mut core, view, ":");
                    keys(&mut core, view, command);
                    let output = core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
                    let status = output.command.unwrap().status;
                    assert!(matches!(status, CommandStatus::Complete)
                        || command == "g/^$/d" && matches!(&status, CommandStatus::ExError(
                            viem_core::command::ExCommandError::Execute(
                                viem_core::command::ex_execute::ExExecuteError::PatternNotFound(pattern)
                            )) if pattern == "^$"), "{command}: {status:?}");
                    assert_eq!(core.document().source_bytes(), expected.as_bytes(), "{format:?}, {mode:?}, {command}");
                    if expected != original {
                        keys(&mut core, view, "u");
                        assert_eq!(core.document().source_bytes(), original.as_bytes());
                    }
                }
            }
        }
    }
}

#[test]
fn literal_empty_last_content_line_remains_addressable_and_insert_can_add_one() {
    use viem_core::document::{Encoding, Format};
    for format in [Format::PlainText, Format::Code] {
        let mut core = Core::new(Document::from_bytes(b"a\n\n".to_vec(), Encoding::Utf8, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
        keys(&mut core, view, "G");
        assert_eq!(core.command_state(view).unwrap().cursor(), 2);
        keys(&mut core, view, "i");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert_eq!(core.document().source_bytes(), b"a\n\n\n");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), b"a\n\n");
    }
}

#[test]
fn literal_normal_vertical_motions_skip_the_final_insertion_boundary() {
    use viem_core::command::LineMode;
    use viem_core::document::{Encoding, Format};
    for format in [Format::PlainText, Format::Code] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            for motion in [vec![Key::Down], vec![Key::Char('j')], vec![Key::Char('g'), Key::Char('j')]] {
                let mut core = Core::new(Document::from_bytes(b"abc\ndef\n".to_vec(), Encoding::Utf8, format).unwrap());
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
                core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
                keys(&mut core, view, "Gl");
                for _ in 0..3 {
                    for key in &motion {
                        core.handle(view, CoreEvent::Input(InputEvent::Key(*key))).unwrap();
                    }
                    let state = core.command_state(view).unwrap();
                    assert_eq!(state.cursor(), 5, "{format:?} {mode:?} {motion:?}");
                    assert!(state.visual_position().is_none_or(|point| point.text_offset < 8));
                }
                assert_eq!(core.document().source_bytes(), b"abc\ndef\n");
            }
        }
    }
}

#[test]
fn ex_relative_visual_addresses_cannot_reach_a_final_terminator_row() {
    use viem_core::command::LineMode;
    use viem_core::document::{Encoding, Format};
    for format in [Format::PlainText, Format::Code] {
        for command in ["$+1d", ".+1d"] {
            let mut core = Core::new(Document::from_bytes(b"abc\ndef\n".to_vec(), Encoding::Utf8, format).unwrap());
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
            core.handle(view, CoreEvent::SetLineMode(LineMode::Visual)).unwrap();
            keys(&mut core, view, "G:");
            keys(&mut core, view, command);
            let output = core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
            assert!(matches!(output.command.unwrap().status, CommandStatus::ExError(_)), "{format:?} {command}");
            assert_eq!(core.document().source_bytes(), b"abc\ndef\n");
            assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        }
    }
}

#[test]
fn counted_line_end_change_retains_the_final_file_terminator() {
    use viem_core::command::LineMode;
    use viem_core::document::{Encoding, Format};
    for format in [Format::PlainText, Format::Code] {
        for mode in [LineMode::Visual, LineMode::PhysicalSource] {
            let mut core = Core::new(Document::from_bytes(b"abc\ndef\n".to_vec(), Encoding::Utf8, format).unwrap());
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
            core.handle(view, CoreEvent::SetLineMode(mode)).unwrap();
            keys(&mut core, view, "G2Cz");
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            assert_eq!(core.document().source_bytes(), b"abc\nz\n", "{format:?} {mode:?}");
            keys(&mut core, view, "u");
            assert_eq!(core.document().source_bytes(), b"abc\ndef\n");
        }
    }
}

#[test]
fn interpreter_line_motions_resolve_the_last_content_row_before_its_column() {
    use viem_core::command::CommandInterpreter;
    use viem_core::document::{Encoding, Format};

    // No layout is installed here: this exercises the public interpreter fallback.
    for format in [Format::PlainText, Format::Code] {
        for visual in [None, Some('v'), Some('V')] {
            for (motion, expected) in [
                ("jjj", 6),
                ("99j", 6),
                ("↓↓↓", 6),
                ("99+", 5),
                ("99\n", 5),
                ("2g_", 7),
                ("99g_", 7),
                ("2$", 9),
                ("99$", 9),
                ("$jj", 9),
            ] {
                let source = b"abc\n def  \n";
                let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
                let mut commands = CommandInterpreter::new();
                assert!(commands.set_cursor(&document, 6));
                for character in visual.into_iter().chain(motion.chars()) {
                    let key = match character {
                        '↓' => Key::Down,
                        '\n' => Key::Enter,
                        character => Key::Char(character),
                    };
                    let output = commands.handle(&mut document, InputEvent::Key(key)).unwrap();
                    assert!(matches!(output.status, CommandStatus::Complete | CommandStatus::Pending),
                        "{format:?}, {visual:?}, {motion:?}: {:?}", output.status);
                }
                assert_eq!(commands.cursor(), expected, "{format:?}, {visual:?}, {motion:?}");
                assert_eq!(document.source_bytes(), source);
            }

            let mut document = Document::from_bytes(b"abcdef\nxy\n".to_vec(), Encoding::Utf8, format).unwrap();
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, 4));
            if let Some(key) = visual {
                commands.handle(&mut document, InputEvent::key(key)).unwrap();
            }
            for (key, expected) in [('j', 8), ('j', 8), ('k', 4)] {
                commands.handle(&mut document, InputEvent::key(key)).unwrap();
                assert_eq!(commands.cursor(), expected, "preferred column: {format:?}, {visual:?}, {key}");
            }
        }
    }
}

#[test]
fn interpreter_line_motions_keep_insertion_rows_and_explicit_source_paragraphs() {
    use viem_core::command::CommandInterpreter;
    use viem_core::document::{Encoding, Format};

    for format in [Format::PlainText, Format::Code] {
        for insert in ['i', 'R'] {
            let source = b"abc\ndef\n";
            let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
            let mut commands = CommandInterpreter::new();
            assert!(commands.set_cursor(&document, 5));
            commands.handle(&mut document, InputEvent::key(insert)).unwrap();
            commands.handle(&mut document, InputEvent::Key(Key::Down)).unwrap();
            assert_eq!(commands.cursor(), document.text().len(), "{format:?}, {insert}");
            assert_eq!(document.source_bytes(), source);
        }
    }
}
