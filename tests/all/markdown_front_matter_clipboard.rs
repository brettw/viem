use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget,
};
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
use viem_core::document::{ClipboardFragment, Document, Encoding, FileFormat, Format};

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        Encoding::Latin1 => text.chars().map(|ch| ch as u8).collect(),
    }
}

fn paste(document: &mut Document, at: usize, fragment: &ClipboardFragment) {
    let value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), value["plain_text"].as_str().unwrap()).unwrap();
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1),
        ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap()),
    ));
    let mut commands = CommandInterpreter::new();
    commands.handle(document, InputEvent::key('i')).unwrap();
    assert!(commands.set_cursor(document, at));
    let result = commands.handle_with_clipboard_context(
        document, InputEvent::Key(Key::PasteClipboard), &context,
    ).unwrap_or_else(|error| panic!(
        "paste failed: {error:?}; encoding={} file_format={} plain_text={} source_text={} source_prefix={:?} destination={:?}",
        value["encoding"], value["file_format"], value["plain_text"], value["source_text"],
        value["source_bytes"].as_array().map(|bytes| &bytes[..bytes.len().min(8)]), document.text(),
    ));
    assert_eq!(result.status, CommandStatus::Complete);
    assert!(result.document_changed);
    commands.handle(document, InputEvent::Key(Key::Escape)).unwrap();
}

fn assert_code_copy(document: &Document, expected: &str) {
    assert_eq!(document.text(), expected);
    assert!(document.projection().blocks().iter().all(|block| !block.front_matter));
    let block = &document.projection().blocks()[0];
    assert_eq!(block.style.0, "Code Block");
    assert_eq!(block.code_language.as_deref(), Some("yaml"));
}

#[test]
fn whole_front_matter_clipboard_pastes_as_yaml_code_with_original_encoded_body() {
    let body = "name: café\nvalue: ````literal````";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (file_format, ending) in [(FileFormat::Unix, "\n"), (FileFormat::Dos, "\r\n"), (FileFormat::Mac, "\r")] {
            for bom in [false, true] {
                let prefix = if bom { "\u{feff}" } else { "" };
                let source = format!("{prefix}---{ending}{}{ending}---", body.replace('\n', ending));
                let original = encoded(&source, encoding);
                let donor = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Markdown, file_format).unwrap();
                let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
                let mut target = Document::from_bytes_with_file_format(Vec::new(), encoding, Format::Markdown, file_format).unwrap();
                paste(&mut target, 0, &fragment);
                assert_code_copy(&target, body);
                let expected = encoded(&format!("{prefix}`````yaml{ending}{}{ending}`````", body.replace('\n', ending)), encoding);
                assert_eq!(target.source_bytes(), expected, "{encoding:?} {file_format:?} bom={bom}");
                assert_eq!(donor.source_bytes(), original);
                assert!(!donor.is_dirty());
                let reopened = Document::from_bytes_with_file_format(expected.clone(), encoding, Format::Markdown, file_format).unwrap();
                assert_code_copy(&reopened, body);
                assert!(target.undo());
                assert!(target.source_bytes().is_empty());
                assert!(target.redo());
                assert_eq!(target.source_bytes(), expected);
            }
        }
    }
}

#[test]
fn whole_document_clipboard_preserves_prose_after_the_converted_metadata_owner() {
    let source = "--- \t\nname: example\n---\t\n\n# Heading\n\n__untouched__ prose\n";
    let donor = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
    let mut target = Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap();
    paste(&mut target, 0, &fragment);
    assert_eq!(target.text(), donor.text());
    assert_eq!(target.source_bytes(), b"```yaml\nname: example\n```\n\n# Heading\n\n__untouched__ prose\n");
    assert!(target.projection().blocks().iter().all(|block| !block.front_matter));
    assert_eq!(donor.source_bytes(), source.as_bytes());
}

#[test]
fn metadata_owner_and_partial_copies_paste_into_empty_documents_as_yaml_code() {
    let source = "---\nname: `literal`\nvalue: **literal**\n---\n\nTail";
    let donor = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let owner = donor.projection().blocks()[0].range.clone();
    for range in [owner, 6..15, 6.."name: `literal`\nvalue: **literal**".len()] {
        let copied = donor.text()[range.clone()].to_owned();
        let fragment = donor.clipboard_fragment(range).unwrap();
        let mut target = Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap();
        paste(&mut target, 0, &fragment);
        assert_code_copy(&target, &copied);
        let saved = target.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_code_copy(&reopened, &copied);
        assert!(target.undo());
        assert!(target.source_bytes().is_empty());
        assert!(target.redo());
        assert_eq!(target.source_bytes(), saved);
    }
    assert_eq!(donor.source_bytes(), source.as_bytes());
}

#[test]
fn metadata_and_following_prose_partial_copy_keeps_the_code_owner() {
    let source = "---\nname: example\nvalue: **literal**\n---\n\nTail __styled__\n\nUnselected";
    let donor = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let end = donor.text().find("\nUnselected").unwrap();
    for start in [0, 6] {
        let copied = &donor.text()[start..end];
        let fragment = donor.clipboard_fragment(start..end).unwrap();
        let mut target = Document::from_bytes(Vec::new(), Encoding::Utf8, Format::Markdown).unwrap();
        paste(&mut target, 0, &fragment);
        assert_code_copy(&target, copied);
        assert!(String::from_utf8(target.source_bytes()).unwrap().ends_with("Tail __styled__"));
        let reopened = Document::from_bytes(target.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), copied);
    }
    assert_eq!(donor.source_bytes(), source.as_bytes());
}

#[test]
fn single_line_metadata_copies_preserve_literal_text_when_pasted_into_prose() {
    for copied in ["name: example", " `literal` ", "````", "**literal**"] {
        let source = format!("---\n{copied}\n---");
        let donor = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
        let mut target = Document::from_bytes(b"before after".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        paste(&mut target, 7, &fragment);
        assert_eq!(target.text(), format!("before {copied}after"));
        assert!(target.projection().blocks().iter().all(|block| !block.front_matter));
        let saved = target.source_bytes();
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), target.text());
        assert!(target.undo());
        assert_eq!(target.source_bytes(), b"before after");
        assert!(target.redo());
        assert_eq!(target.source_bytes(), saved);
    }
}

#[test]
fn source_view_clipboard_keeps_front_matter_bytes_literal() {
    let source = "\u{feff}---\r\nname: example\r\n---";
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        let original = encoded(source, encoding);
        let donor = Document::from_bytes_with_file_format(original.clone(), encoding, Format::Code, FileFormat::Dos).unwrap();
        let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
        let exported: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
        let bytes: Vec<u8> = serde_json::from_value(exported["source_bytes"].clone()).unwrap();
        assert_eq!(bytes, original);
        assert_eq!(donor.source_bytes(), original);
    }
}

#[test]
fn whole_metadata_clipboard_distinguishes_bom_metadata_from_visible_feff_content() {
    for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for (prefix, target_prefix) in [("", "\u{feff}"), ("\u{feff}", "")] {
            let body = "\u{feff}name: example";
            let source = encoded(&format!("{prefix}---\n{body}\n---"), encoding);
            let donor = Document::from_bytes(source, encoding, Format::Markdown).unwrap();
            let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
            let original = encoded(target_prefix, encoding);
            let mut target = Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
            paste(&mut target, 0, &fragment);
            assert_code_copy(&target, body);
            let expected = encoded(&format!("{prefix}```yaml\n{body}\n```"), encoding);
            assert_eq!(target.source_bytes(), expected);
            assert!(target.undo());
            assert_eq!(target.source_bytes(), original);
            assert!(target.redo());
            assert_eq!(target.source_bytes(), expected);
        }
    }
}
