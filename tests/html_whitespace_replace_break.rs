use evim_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use evim_core::document::{Document, Encoding, Format};
use evim_core::layout::MockTextMeasurementProvider;
use evim_core::{Core, CoreEvent};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn normal_replace_enter_places_cursor_after_break_and_before_protected_following_space() {
    for (source, prefix, expected, cursor) in [
        ("<p>A B</p>", "$r", "A\u{a0}\n", 4),
        ("<p>A BC</p>", "ll2r", "A\u{a0}\n", 4),
        ("<p>A BC D</p>", "ll2r", "A\u{a0}\n\u{a0}D", 4),
        ("<p>é B</p>", "$r", "é\u{a0}\n", 5),
    ] {
        let mut events = prefix.chars().map(InputEvent::key).collect::<Vec<_>>();
        events.push(InputEvent::Key(Key::Enter));
        for planned in [false, true] {
            let (mut document, actual_cursor) = if planned {
                let mut core = Core::new(html(source));
                let view = core.add_view(MockTextMeasurementProvider::new(), 300., 100.);
                for event in &events {
                    let update = core
                        .handle(view, CoreEvent::Input(event.clone()))
                        .unwrap_or_else(|error| panic!("{source} {prefix} {event:?}: {error:?}"));
                    if let Some(command) = update.command {
                        assert!(matches!(
                            command.status,
                            CommandStatus::Complete | CommandStatus::Pending
                        ));
                    }
                }
                let cursor = core.command_state(view).unwrap().cursor();
                let document = html(&String::from_utf8(core.document().source_bytes()).unwrap());
                assert_eq!(core.document().text(), document.text());
                (document, cursor)
            } else {
                let mut document = html(source);
                let mut commands = CommandInterpreter::new();
                for event in &events {
                    let output = commands
                        .handle(&mut document, event.clone())
                        .unwrap_or_else(|error| panic!("{source} {prefix} {event:?}: {error:?}"));
                    assert!(matches!(
                        output.status,
                        CommandStatus::Complete | CommandStatus::Pending
                    ));
                }
                (document, commands.cursor())
            };
            assert_eq!(document.text(), expected, "{source}, planned={planned}");
            assert_eq!(actual_cursor, cursor, "{source}, planned={planned}");
            document.text_point(actual_cursor).unwrap();
            let line = document
                .hard_line_snapshot()
                .line_at_offset(actual_cursor)
                .unwrap();
            assert_eq!(line.content_range().start, actual_cursor);
            if !planned {
                let saved = document.source_bytes();
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), saved);
            }
        }
    }
}
