use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, Mode};
use viem_core::document::*;

fn input(commands: &mut CommandInterpreter, document: &mut Document, event: InputEvent) {
    let output = commands
        .handle(document, event.clone())
        .unwrap_or_else(|error| {
            panic!(
                "{event:?} {error:?} from {}",
                String::from_utf8_lossy(&document.source_bytes())
            )
        });
    assert_eq!(output.status, CommandStatus::Complete);
}
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn styles(document: &Document) -> Vec<String> {
    document
        .projection()
        .blocks()
        .iter()
        .map(|block| block.style.0.clone())
        .collect()
}

#[test]
fn opening_above_and_below_uses_the_originating_heading_following_style() {
    for (format, source, heading) in [
        (Format::Markdown, "# Title", "Heading1"),

    ] {
        for (key, expected, expected_styles) in [
            ('o', "Title\nbody", vec![heading, "Paragraph"]),
            ('O', "body\nTitle", vec!["Paragraph", heading]),
        ] {
            let mut document = open(source, format);
            let mut commands = CommandInterpreter::new();
            input(&mut commands, &mut document, InputEvent::key(key));
            assert_eq!(commands.mode(), Mode::Insert);
            assert_eq!(
                styles(&document),
                expected_styles,
                "{format:?} {key}: empty opened paragraph"
            );
            input(&mut commands, &mut document, InputEvent::text("body"));
            input(&mut commands, &mut document, InputEvent::Key(Key::Escape));
            assert_eq!(document.text(), expected, "{format:?} {key}");
            assert_eq!(styles(&document), expected_styles, "{format:?} {key}");
            let changed = document.source_bytes();
            let reopened = Document::from_bytes(changed.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(styles(&reopened), expected_styles);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(!document.undo());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}

#[test]
fn code_open_adds_a_code_line_and_keeps_the_fence() {
    let source = "```rust\ncode\n```";
    for (key, expected, bytes) in [
        ('o', "code\nbody", "```rust\ncode\nbody\n```"),
        ('O', "body\ncode", "```rust\nbody\ncode\n```"),
    ] {
        let mut document = open(source, Format::Markdown);
        let mut commands = CommandInterpreter::new();
        input(&mut commands, &mut document, InputEvent::key(key));
        input(&mut commands, &mut document, InputEvent::text("body"));
        input(&mut commands, &mut document, InputEvent::Key(Key::Escape));
        assert_eq!(document.text(), expected, "{key}");
        assert_eq!(document.source_bytes(), bytes.as_bytes(), "{key}");
        assert_eq!(styles(&document), vec!["Code Block"], "{key}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn list_open_retains_item_style_including_empty_items() {
    for (format, source) in [
        (Format::Markdown, "- item"),
        (Format::Markdown, "- "),

    ] {
        for key in ['o', 'O'] {
            let mut document = open(source, format);
            let original = document.text().to_owned();
            let mut commands = CommandInterpreter::new();
            input(&mut commands, &mut document, InputEvent::key(key));
            input(&mut commands, &mut document, InputEvent::text("new"));
            input(&mut commands, &mut document, InputEvent::Key(Key::Escape));
            assert_eq!(
                document.text(),
                if key == 'o' {
                    format!("{original}\nnew")
                } else {
                    format!("new\n{original}")
                }
            );
            assert_eq!(
                styles(&document),
                vec!["BulletedList1", "BulletedList1"],
                "{format:?} {key}"
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn counted_open_and_dot_evaluate_each_new_paragraph_style_and_share_undo() {
    for (format, source, title, following) in [
        (Format::Markdown, "# Title", "Heading1", "Heading2"),

    ] {
        for key in ['o', 'O'] {
            let mut document = open(source, format);
            {
                let mut style = document
                    .projection()
                    .style_sheet()
                    .block_style(&title.into())
                    .unwrap()
                    .clone();
                style.next_paragraph_style = Some(following.into());
                let edit = StyleDefinitionEdit::UpdateBlock(style);
                let intent = {
                    StyleModelIntent::Configuration(ConfigurationStyleIntent::EditDefinition(edit))
                };
                document
                    .apply_style_request(StyleModelRequest::new(
                        document.id(),
                        document.revision(),
                        intent,
                    ))
                    .unwrap();
            }
            let baseline = document.source_bytes();
            let mut commands = CommandInterpreter::new();
            commands
                .handle(&mut document, InputEvent::key('2'))
                .unwrap();
            input(&mut commands, &mut document, InputEvent::key(key));
            input(&mut commands, &mut document, InputEvent::text("X"));
            input(&mut commands, &mut document, InputEvent::Key(Key::Escape));
            assert_eq!(
                document.text(),
                if key == 'o' {
                    "Title\nX\nX"
                } else {
                    "X\nX\nTitle"
                },
                "{format:?} {key}"
            );
            assert_eq!(
                styles(&document),
                if key == 'o' {
                    vec![title, following, "Paragraph"]
                } else {
                    vec![following, "Paragraph", title]
                },
                "{format:?} {key}"
            );
            let once = document.source_bytes();
            input(&mut commands, &mut document, InputEvent::key('.'));
            let twice = document.source_bytes();
            assert_ne!(twice, once);
            input(&mut commands, &mut document, InputEvent::key('u'));
            assert_eq!(document.source_bytes(), once);
            input(&mut commands, &mut document, InputEvent::key('u'));
            assert_eq!(document.source_bytes(), baseline);
        }
    }
}

#[test]
fn open_from_a_middle_code_row_stays_in_the_code_block() {
    let source = "```rust\none\ntwo\n```";
    let mut document = open(source, Format::Markdown);
    let mut commands = CommandInterpreter::new();
    input(&mut commands, &mut document, InputEvent::key('o'));
    input(&mut commands, &mut document, InputEvent::text("X"));
    input(&mut commands, &mut document, InputEvent::Key(Key::Escape));
    assert_eq!(document.text(), "one\nX\ntwo");
    assert_eq!(document.source_bytes(), b"```rust\none\nX\ntwo\n```");
    assert_eq!(styles(&document), vec!["Code Block"]);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn open_adds_lines_to_empty_and_unclosed_code_paragraphs() {
    for (source, below, above) in [
        ("```\n\n```", "```\n\nX\n```", "```\nX\n\n```"),
        ("```\n```", "```\n\nX\n```", "```\nX\n\n```"),
        ("```", "```\n\nX", "```\nX\n"),
        ("```\ncode", "```\ncode\nX", "```\nX\ncode"),
    ] {
        for (key, expected) in [('o', below), ('O', above)] {
            let mut document = open(source, Format::Markdown);
            let original = document.text().to_owned();
            let mut commands = CommandInterpreter::new();
            input(&mut commands, &mut document, InputEvent::key(key));
            input(&mut commands, &mut document, InputEvent::text("X"));
            assert_eq!(
                document.text(),
                if key == 'o' {
                    format!("{original}\nX")
                } else {
                    format!("X\n{original}")
                },
                "{source:?} {key}"
            );
            assert_eq!(String::from_utf8(document.source_bytes()).unwrap(), expected, "{source:?} {key}");
            assert_eq!(styles(&document), vec!["Code Block"], "{source:?} {key}");
        }
    }
}
