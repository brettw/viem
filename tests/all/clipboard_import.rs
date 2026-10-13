//! External rich clipboard import through the command controller.
use viem_core::document::{ClipboardFragment, Document, Encoding, FileFormat, Format};

fn open(source: &[u8], format: Format) -> Document {
    Document::from_bytes_with_file_format(source.to_vec(), Encoding::Utf8, format, FileFormat::Unix)
        .unwrap()
}

#[test]
fn normalized_html_styles_survive_real_clipboard_paste_and_undo() {
    use viem_core::command::clipboard::{
        ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
        ClipboardTarget,
    };
    use viem_core::command::{InputEvent, Key, RegisterValue};
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    let (fragment, plain) = ClipboardFragment::from_html_utf8(
        b"<p style='text-align:center;margin-bottom:18pt'><b>Bold</b> <i style='color:#ff0000'>red</i></p>",
    ).unwrap();
    // The FFI/Windows clipboard path validates the private wire payload.
    let imported = ClipboardFragment::from_json(fragment.json(), &plain).unwrap();
    let register = RegisterValue::from_clipboard_fragment(imported).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        ClipboardContent::from_register(register),
    ));
    for format in [
        Format::Markdown,
        Format::MarkdownSource,
        Format::PlainText,
        Format::Code,
    ] {
        let mut core = Core::new(open(b"", format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
        for input in [
            InputEvent::key('i'),
            InputEvent::Key(Key::Ctrl('r')),
            InputEvent::key('+'),
            InputEvent::Key(Key::Escape),
        ] {
            core.handle(
                view,
                CoreEvent::InputWithClipboard {
                    input,
                    clipboard: context.clone(),
                },
            )
            .unwrap_or_else(|error| panic!("{format:?}: {error:?}"));
        }
        assert_eq!(core.document().text(), "Bold red", "{format:?}");
        let saved = core.document().source_bytes();
        let reopened = open(&saved, format);
        for document in [core.document(), &reopened] {
            if format.is_wysiwyg() {
                assert!(
                    DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
                        .unwrap()
                        .bold,
                    "{format:?}"
                );
                let red =
                    DocumentLayoutStyles::semantic_character_at(document.projection(), 5, false)
                        .unwrap();
                assert_eq!(red.slant, viem_core::document::FontSlant::Italic);

            }
        }
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert!(core.document().source_bytes().is_empty(), "{format:?}");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}

fn paste_private_fragment(
    document: &mut Document,
    fragment: &ClipboardFragment,
    at: usize,
) -> viem_core::command::CommandInterpreter {
    use viem_core::command::clipboard::{
        ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
        ClipboardTarget,
    };
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
    let register = RegisterValue::from_clipboard_fragment(fragment.clone()).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        ClipboardContent::from_register(register),
    ));
    let before = context.clone();
    let mut commands = CommandInterpreter::new();
    commands.handle(document, InputEvent::key('i')).unwrap();
    assert!(commands.set_cursor(document, at));
    assert_eq!(commands.cursor(), at);
    let output = commands
        .handle_with_clipboard_context(document, InputEvent::Key(Key::PasteClipboard), &context)
        .unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
    assert!(output.document_changed);
    assert!(output.clipboard_writes.is_empty());
    assert_eq!(context, before);
    commands
}

fn exact_paste_history(
    document: &mut Document,
    commands: &mut viem_core::command::CommandInterpreter,
    before: &[u8],
    after: &[u8],
) {
    use viem_core::command::{InputEvent, Key};
    commands.handle(document, InputEvent::Key(Key::Escape)).unwrap();
    commands.handle(document, InputEvent::key('u')).unwrap();
    assert_eq!(document.source_bytes(), before);
    commands.handle(document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
    assert_eq!(document.source_bytes(), after);
}

#[test]
fn private_paste_protects_a_copied_whitespace_paragraph_without_losing_later_styles() {
    use viem_core::layout::DocumentLayoutStyles;
    for (source, copied) in [
        ("line  \n  \nafter double space", "  \nafter double spac"),
        ("line  \n  \nafter **double** space", "  \nafter double spac"),
        ("line    \n  \nafter **double** space", "    \nafter double spac"),
        ("line\t \n  \nafter **double** space", "\t \nafter double spac"),
    ] {
        let donor = open(source.as_bytes(), Format::Markdown);
        let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
        let fragment = ClipboardFragment::from_json(fragment.json(), copied).unwrap();
        for (target, at, protect) in [
            ("target", 0, true),
            ("before\n\ntarget", 7, true),
            ("target", 3, false),
            ("", 0, true),
        ] {
            let mut document = open(target.as_bytes(), Format::Markdown);
            let original = document.text().to_owned();
            let mut commands = paste_private_fragment(&mut document, &fragment, at);
            let expected = format!("{}{}{}", &original[..at], copied, &original[at..]);
            let saved = document.source_bytes();
            let syntax = String::from_utf8(saved.clone()).unwrap();
            assert_eq!(syntax.contains("&#32;") || syntax.contains("&#9;"), protect, "{target:?} at {at}: {syntax:?}");
            if source.contains("**double**") {
                assert!(syntax.contains("**double**"), "{syntax:?}");
            }
            let reopened = open(&saved, Format::Markdown);
            for candidate in [&document, &reopened] {
                assert_eq!(candidate.text(), expected);
                let double = DocumentLayoutStyles::semantic_character_at(
                    candidate.projection(), at + copied.find("double").unwrap(), false,
                ).unwrap();
                assert_eq!(double.bold, source.contains("**double**"));
            }
            exact_paste_history(&mut document, &mut commands, target.as_bytes(), &saved);
        }
    }
}

#[test]
fn private_whitespace_paste_keeps_literal_destinations_and_code_sources_literal() {
    let donor = open(b"line  \n  \nafter **double** space", Format::Markdown);
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource] {
        let mut document = open(b"", format);
        let mut commands = paste_private_fragment(&mut document, &fragment, 0);
        let saved = document.source_bytes();
        assert_eq!(saved, copied.as_bytes(), "{format:?}");
        assert_eq!(open(&saved, format).text(), copied);
        exact_paste_history(&mut document, &mut commands, b"", &saved);
    }
    for source in ["`  code`", "```\n  \n  code\n```"] {
        let donor = open(source.as_bytes(), Format::Markdown);
        let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
        let mut document = open(b"", Format::Markdown);
        let mut commands = paste_private_fragment(&mut document, &fragment, 0);
        let saved = document.source_bytes();
        assert_eq!(saved, source.as_bytes());
        let reopened = open(&saved, Format::Markdown);
        for candidate in [&document, &reopened] {
            assert_eq!(candidate.text(), donor.text());
            assert_eq!(candidate.projection().style_spans(), donor.projection().style_spans());
        }
        exact_paste_history(&mut document, &mut commands, b"", &saved);
    }
}

#[test]
fn private_whitespace_paste_protects_encoded_prefixes_and_preserves_dos_breaks() {
    let encode = |text: &str, encoding| match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        _ => unreachable!(),
    };
    for source_encoding in [Encoding::Utf8, Encoding::Utf16Le] {
        let donor = Document::from_bytes_with_file_format(
            encode("line  \r\n  \r\nafter **double** space", source_encoding),
            source_encoding, Format::Markdown, FileFormat::Dos,
        ).unwrap();
        let copied = "  \nafter double spac";
        let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
        for target_encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            let before = encode("target", target_encoding);
            let mut document = Document::from_bytes_with_file_format(
                before.clone(), target_encoding, Format::Markdown, FileFormat::Dos,
            ).unwrap();
            let mut commands = paste_private_fragment(&mut document, &fragment, 0);
            let saved = document.source_bytes();
            assert!(saved.starts_with(&encode("&#32;", target_encoding)));
            let reopened = Document::from_bytes_with_file_format(
                saved.clone(), target_encoding, Format::Markdown, FileFormat::Dos,
            ).unwrap();
            assert_eq!(document.text(), format!("{copied}target"));
            assert_eq!(document.text(), reopened.text());
            assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
            let source_view = Document::from_bytes_with_file_format(
                saved.clone(), target_encoding, Format::Code, FileFormat::Unix,
            ).unwrap();
            assert!(source_view.text().contains("\r\n  \r\nafter **double**"));
            exact_paste_history(&mut document, &mut commands, &before, &saved);
        }
    }
}
