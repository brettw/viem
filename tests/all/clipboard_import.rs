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
        Format::Rtf,
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
                if format == Format::Rtf {
                    assert_eq!(red.foreground.red, 1.0);
                    assert_eq!(red.foreground.green, 0.0);
                    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
                    assert_eq!(
                        styles.paragraphs[0].alignment,
                        viem_core::document::ParagraphAlignment::Center
                    );
                    assert_eq!(styles.paragraphs[0].margin_bottom, 18.0);
                }
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

#[test]
fn normalized_html_unrepresentable_style_uses_plain_paste_and_exact_undo() {
    use viem_core::command::clipboard::{
        ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
        ClipboardTarget,
    };
    use viem_core::command::{InputEvent, Key, RegisterValue};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let (fragment, _) = ClipboardFragment::from_html_utf8(
        b"<p><span style='font-family:Arial,sans-serif'>fallback</span></p>",
    )
    .unwrap();
    let initial = br"{\rtf1 }";
    let document = open(initial, Format::Rtf);
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap()),
    ));
    let mut core = Core::new(document);
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
        .unwrap();
    }
    assert_eq!(core.document().text(), "fallback");
    let saved = core.document().source_bytes();
    assert_eq!(open(&saved, Format::Rtf).text(), "fallback");
    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
        .unwrap();
    assert_eq!(core.document().source_bytes(), initial);
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
        .unwrap();
    assert_eq!(core.document().source_bytes(), saved);
}
