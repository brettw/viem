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
            if format.is_markdown() {
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
    for format in [Format::PlainText, Format::Code] {
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

#[test]
fn whitespace_rich_paste_retains_styles_on_both_sides_of_its_changed_paragraphs() {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterValue};
    use viem_core::document::{BoundaryAffinity, FontSlant};
    use viem_core::layout::DocumentLayoutStyles;
    let donor = open(b"line  \n  \nafter **double** space", Format::Markdown);
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1),
        ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap()),
    ));
    for (source, selected, expected, suffix) in [
        ("__before__ target _after_", 7..7, format!("before {copied}target after"), "target _after_"),
        ("__before__ left\n\nright _after_", 7..17, format!("before {copied} after"), " _after_"),
    ] {
        let mut document = open(source.as_bytes(), Format::Markdown);
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        assert!(commands.set_cursor_from_pointer(&document, selected.start, BoundaryAffinity::Downstream, false));
        if !selected.is_empty() {
            assert!(commands.set_cursor_from_pointer(&document, selected.end, BoundaryAffinity::Upstream, true));
        }
        let output = commands.handle_with_clipboard_context(&mut document,
            InputEvent::Key(Key::PasteClipboard), &clipboard).unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.document_changed);
        let saved = document.source_bytes();
        assert!(saved.starts_with(b"__before__ "));
        assert!(saved.ends_with(suffix.as_bytes()));
        let reopened = open(&saved, Format::Markdown);
        for candidate in [&document, &reopened] {
            assert_eq!(candidate.text(), expected);
            let style_at = |at| DocumentLayoutStyles::semantic_character_at(candidate.projection(), at, false).unwrap();
            assert!(style_at(0).bold);
            assert!(style_at(7 + copied.find("double").unwrap()).bold);
            assert_eq!(style_at(expected.len() - "after".len()).slant, FontSlant::Italic);
        }
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
        exact_paste_history(&mut document, &mut commands, source.as_bytes(), &saved);
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
    }
}

#[test]
fn whitespace_rich_paste_preserves_neighboring_independent_lists() {
    use viem_core::layout::DocumentLayoutStyles;
    let donor = open(b"line  \n  \nafter **double** space", Format::Markdown);
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let source = b"+ before\n\n- first\n\n  <span></span>\n- tail\n\n+ after";
    let mut document = open(source, Format::Markdown);
    let before = document.text().to_owned();
    let at = "before\nfirst\n".len();
    let mut commands = paste_private_fragment(&mut document, &fragment, at);
    let saved = document.source_bytes();
    assert!(saved.starts_with(b"+ before\n\n- first\n\n"));
    assert!(saved.ends_with(b"- tail\n\n+ after"));
    let reopened = open(&saved, Format::Markdown);
    let mut expected = before;
    expected.insert_str(at, copied);
    let list_items = |document: &Document| {
        let blocks = document.projection().blocks();
        document.projection().list_structure().lists.iter().map(|list| {
            list.items.iter().map(|item| {
                item.paragraph_ids.iter().map(|id| {
                    let block = blocks.iter().find(|block| block.id == *id).unwrap();
                    document.projection().text_tree().slice(block.range.clone()).unwrap()
                }).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        }).collect::<Vec<_>>()
    };
    for candidate in [&document, &reopened] {
        assert_eq!(candidate.text(), expected);
        assert!(DocumentLayoutStyles::semantic_character_at(candidate.projection(),
            at + copied.find("double").unwrap(), false).unwrap().bold);
        let lists = list_items(candidate);
        let owner = |text| lists.iter().position(|list| list.iter().any(|item| item[0] == text)).unwrap();
        assert_ne!(owner("before"), owner("first"));
        assert_ne!(owner("tail"), owner("after"));
        assert_ne!(owner("before"), owner("after"));
    }
    assert_eq!(list_items(&document), list_items(&reopened));
    assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
    exact_paste_history(&mut document, &mut commands, source, &saved);
    assert_eq!(list_items(&document), list_items(&reopened));
}

#[test]
fn multiline_rich_paste_uses_the_first_touching_list_owner() {
    let donor = open(b"X\n\nY", Format::Markdown);
    let fragment = donor.clipboard_fragment(0..donor.text().len()).unwrap();
    for (source, at, expected_source) in [
        ("- - - body", 0, "- - - X\n- Ybody"),
        ("> - parent\n>   - child\n> - tail", 7,
            "> - parent\n>   - X\n>   - Ychild\n> - tail"),
        ("- parent\n  - child\n\n  continuation\n- tail", 13,
            "- parent\n  - child\n\n  X\n- Ycontinuation\n- tail"),
        ("- first\n\n  <span></span>\n- tail", 6,
            "- first\n\n  X\n- Y<span></span>\n- tail"),
        ("-\n- tail", 0, "- X\n- Y\n- tail"),
    ] {
        let mut document = open(source.as_bytes(), Format::Markdown);
        let mut expected = document.text().to_owned();
        expected.insert_str(at, "X\nY");
        let mut commands = paste_private_fragment(&mut document, &fragment, at);
        let saved = document.source_bytes();
        assert_eq!(saved, expected_source.as_bytes(), "{source:?}");
        let reopened = open(&saved, Format::Markdown);
        assert_eq!(document.text(), expected, "{source:?}");
        assert_eq!(reopened.text(), expected, "{source:?}");
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
        let shape = |document: &Document| {
            let mut owners = std::collections::HashMap::new();
            document.projection().blocks().iter().map(|block| {
                let containers = block.containers.iter().map(|member| {
                    let next = owners.len();
                    let owner = *owners.entry(member.container.id).or_insert(next);
                    (owner, member.container.kind, member.starts_here, member.ends_here)
                }).collect::<Vec<_>>();
                (block.range.clone(), block.kind.clone(), block.style.clone(), block.list_loose, containers)
            }).collect::<Vec<_>>()
        };
        assert_eq!(shape(&document), shape(&reopened), "{source:?}");
        exact_paste_history(&mut document, &mut commands, source.as_bytes(), &saved);
        assert_eq!(shape(&document), shape(&reopened), "{source:?}");
    }
}

#[test]
fn linewise_rich_put_restores_deleted_empty_paragraphs_between_whitespace_separators() {
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key, RegisterKind};
    use viem_core::document::BoundaryAffinity;
    for (source, before_put, after_put) in [
        ("a\n \n \n \nb", "a\n \nb", "a\n \n\n\nb"),
        ("a\n\t\n \n\n\nb", "a\n\t\nb", "a\n\t\n\n\nb"),
        // The surviving list keeps its marker; P consequently uses the
        // destination list's ordinary empty-item insertion policy.
        ("# h\n \n \n \n- b", "# h\n \n- b", "# h\n \n- \n- b"),
    ] {
        let mut document = open(source.as_bytes(), Format::Markdown);
        let expected_text = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor_from_pointer(&document, 2, BoundaryAffinity::Downstream, false));
        commands.handle(&mut document, InputEvent::key('d')).unwrap();
        assert_eq!(commands.handle(&mut document, InputEvent::key('d')).unwrap().status, CommandStatus::Complete);
        assert_eq!(document.source_bytes(), before_put.as_bytes());
        let register = commands.register('"').unwrap().clone();
        assert_eq!(register.text, "\n");
        assert_eq!(register.kind, RegisterKind::Linewise);
        assert!(register.clipboard_fragment().is_some());
        let output = commands.handle(&mut document, InputEvent::key('P')).unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.document_changed);
        assert_eq!(commands.register('"'), Some(&register));
        assert_eq!(document.source_bytes(), after_put.as_bytes());
        assert_eq!(document.text(), expected_text);
        let reopened = open(after_put.as_bytes(), Format::Markdown);
        assert_eq!(document.text(), reopened.text());
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), before_put.as_bytes());
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_eq!(document.source_bytes(), after_put.as_bytes());
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn enter_replacing_an_empty_paragraph_retains_whitespace_separator_boundaries() {
    use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    for (source, expected) in [
        ("a\n \n \n \nb", "a\n \n \n\nb"),
        ("a\n\t\n \n\n\nb", "a\n\t\n \n\nb"),
        ("# h\n \n \n \n- b", "# h\n \n \n\nb"),
    ] {
        let mut document = open(source.as_bytes(), Format::Markdown);
        let expected_text = document.text().to_owned();
        let mut commands = CommandInterpreter::new();
        assert!(commands.set_cursor_from_pointer(&document, 2, BoundaryAffinity::Downstream, false));
        assert!(commands.set_cursor_from_pointer(&document, 3, BoundaryAffinity::Downstream, true));
        let output = commands.handle(&mut document, InputEvent::Key(Key::Enter)).unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.document_changed);
        assert_eq!(document.source_bytes(), expected.as_bytes());
        assert_eq!(document.text(), expected_text);
        let reopened = open(expected.as_bytes(), Format::Markdown);
        assert_eq!(document.text(), reopened.text());
        assert_eq!(document.projection().style_spans(), reopened.projection().style_spans());
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        exact_paste_history(&mut document, &mut commands, source.as_bytes(), expected.as_bytes());
    }
}
