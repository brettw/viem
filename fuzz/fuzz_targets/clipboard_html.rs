//! Import untrusted clipboard payloads: external HTML, or Viem's private JSON
//! fragment (which another application can also place on the pasteboard),
//! then paste the result into a document through the command controller.
#![no_main]
use libfuzzer_sys::fuzz_target;
use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{InputEvent, Key, RegisterValue};
use viem_core::document::{ClipboardFragment, Document, Encoding, FileFormat, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    let fragment = if selector & 1 == 0 {
        let Ok((fragment, plain)) = ClipboardFragment::from_html_utf8(payload) else {
            return;
        };
        // Our own export must pass the external-payload validator.
        ClipboardFragment::from_json(fragment.json(), &plain)
            .expect("an imported HTML fragment must validate as a clipboard payload")
    } else {
        let Ok(json) = std::str::from_utf8(payload) else {
            return;
        };
        let plain = serde_json::from_str::<serde_json::Value>(json)
            .ok()
            .and_then(|value| value.get("plain_text")?.as_str().map(str::to_owned))
            .unwrap_or_default();
        let Ok(fragment) = ClipboardFragment::from_json(json, &plain) else {
            return;
        };
        fragment
    };
    let _ = fragment.table_cells();
    let Ok(register) = RegisterValue::from_clipboard_fragment(fragment) else {
        return;
    };
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard,
        ClipboardGeneration(1),
        ClipboardContent::from_register(register),
    ));
    let format = match selector >> 1 & 3 {
        0 => Format::Markdown,
        1 => Format::MarkdownSource,
        2 => Format::Code,
        _ => Format::PlainText,
    };
    let initial: &[u8] = match selector >> 3 & 3 {
        0 => b"",
        1 => b"- item\n- two",
        2 => b"> quote **bold**",
        _ => b"| a | b |\n|---|---|\n| c | d |",
    };
    let document =
        Document::from_bytes_with_file_format(initial.to_vec(), Encoding::Utf8, format, FileFormat::Unix)
            .expect("fixture opens");
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300., 200.);
    let inputs: &[InputEvent] = if selector & 0x20 == 0 {
        &[InputEvent::Key(Key::PasteClipboard)]
    } else {
        &[
            InputEvent::Key(Key::Char('i')),
            InputEvent::Key(Key::Ctrl('r')),
            InputEvent::Key(Key::Char('+')),
            InputEvent::Key(Key::Escape),
        ]
    };
    for input in inputs {
        let _ = core.handle(
            view,
            CoreEvent::InputWithClipboard { input: input.clone(), clipboard: context.clone() },
        );
    }
    let saved = core.document().source_bytes();
    if let Ok(reopened) =
        Document::from_bytes_with_file_format(saved, Encoding::Utf8, core.document().format(), FileFormat::Unix)
    {
        assert_eq!(
            core.document().text(),
            reopened.text(),
            "pasted source must reproduce its projection"
        );
    }
});
