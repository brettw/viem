use viem_core::document::*;
fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn apply(doc: &mut Document, intent: ImageEditIntent) {
    let (edit, _) = doc
        .prepare_image_edit(doc.id(), doc.revision(), intent)
        .unwrap();
    doc.commit_model_transaction(edit).unwrap();
}
#[test]
fn html_image_location_edits_preserve_dimensions_and_other_attribute_bytes() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (source, expected) in [
            ("before <IMG width='0240' SRC = 'old.png' height=80 alt='a &amp; b' title=kept data-extra='x'> after",
             "before <IMG width='0240' SRC = \"new file.png?a=1&amp;b=2\" height=80 alt='a &amp; b' title=kept data-extra='x'> after"),
            ("<div><img src=old.png width=240 height='80' title='kept'></div>",
             "<div><img src=\"new file.png?a=1&amp;b=2\" width=240 height='80' title='kept'></div>"),
            ("<img\n  width='240'\n  src=old.png\n  height=80 />",
             "<img\n  width='240'\n  src=\"new file.png?a=1&amp;b=2\"\n  height=80 />"),
        ] {
            let mut doc = document(source, format);
            let image = doc.projection().inline_images_for_region(&(0..doc.text().len())).remove(0);
            let snapshot = doc.image_snapshot_at(doc.text_point(image.range.start).unwrap()).unwrap().unwrap();
            apply(&mut doc, ImageEditIntent::Edit { range: snapshot.range, text: snapshot.text, destination: "new file.png?a=1&b=2".into() });
            assert_eq!(doc.source_bytes(), expected.as_bytes(), "{format:?}: {source}");
            let reopened = document(expected, format);
            assert_eq!(doc.text(), reopened.text());
            assert_eq!(doc.projection().inline_images_for_region(&(0..doc.text().len())), reopened.projection().inline_images_for_region(&(0..reopened.text().len())));
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), expected.as_bytes());
        }
    }
}

#[test]
fn html_image_existing_alt_controls_keep_html_and_dimensions() {
    for (source, expected) in [
        ("a <img src=old.png width=120> z", "a <img alt=\"a &lt;cat&gt; &amp; &quot;dog&quot;\" src=old.png width=120> z"),
        ("a <img alt src=old.png height=60> z", "a <img alt=\"a &lt;cat&gt; &amp; &quot;dog&quot;\" src=old.png height=60> z"),
        ("a <img alt='old' src=old.png width=120 height=60/> z", "a <img alt=\"a &lt;cat&gt; &amp; &quot;dog&quot;\" src=old.png width=120 height=60/> z"),
    ] {
        let mut doc = document(source, Format::Markdown);
        let image = doc.image_snapshot_at(doc.text_point(2).unwrap()).unwrap().unwrap();
        apply(&mut doc, ImageEditIntent::Edit { range: image.range, text: "a <cat> & \"dog\"".into(), destination: image.destination });
        assert_eq!(doc.source_bytes(), expected.as_bytes());
        assert_eq!(doc.image_snapshot_at(doc.text_point(2).unwrap()).unwrap().unwrap().text, "a <cat> & \"dog\"");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn html_image_removal_and_normal_delete_restore_exact_source_on_undo() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for source in ["a <img src='cat.png' width=240 height=80> z", "<div>a <img src='cat.png' width=240 height=80> z</div>"] {
        let mut doc = document(source, Format::Markdown);
        let image = doc.image_snapshot_at(doc.text_point(2).unwrap()).unwrap().unwrap();
        apply(&mut doc, ImageEditIntent::Remove { range: image.range });
        assert_eq!(doc.text(), "a  z");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        core.handle(view, CoreEvent::Input(InputEvent::text("llx"))).unwrap();
        assert_eq!(core.document().text(), "a  z");
        core.handle(view, CoreEvent::Input(InputEvent::text("u"))).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn html_image_location_edits_copy_untouched_encoded_and_malformed_attributes() {
    for encoding in [Encoding::Utf8, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
        let encode = |value: &str| match encoding {
            Encoding::Utf8 => value.as_bytes().to_vec(),
            Encoding::Latin1 => value.chars().map(|ch| ch as u8).collect(),
            Encoding::Utf16Le => value.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => value.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        };
        let mut prefix = encode("a <img width='240' title='café ");
        prefix.extend(match encoding {
            Encoding::Utf8 => vec![0xff],
            Encoding::Latin1 => vec![0xfe],
            Encoding::Utf16Le => vec![0, 0xd8],
            Encoding::Utf16Be => vec![0xd8, 0],
        });
        prefix.extend(encode("' src="));
        let suffix = encode(" height=80> z");
        let mut original = prefix.clone();
        original.extend(encode("'old.png'"));
        original.extend(&suffix);
        let mut doc = Document::from_bytes(original.clone(), encoding, Format::Markdown).unwrap();
        let image = doc.image_snapshot_at(doc.text_point(2).unwrap()).unwrap().unwrap();
        apply(&mut doc, ImageEditIntent::Edit { range: image.range, text: image.text, destination: "new.png".into() });
        let mut expected = prefix;
        expected.extend(encode("\"new.png\""));
        expected.extend(suffix);
        assert_eq!(doc.source_bytes(), expected, "{encoding:?}");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original);
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), expected);
    }
}

#[test]
fn html_image_private_clipboard_retains_authored_dimensions() {
    use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
    use viem_core::command::{InputEvent, Key, RegisterValue};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let tag = "<img src='cat.png' alt='cat' width='240' height=80 title='kept'>";
    let doc = document(&format!("before {tag} after"), Format::Markdown);
    let fragment = doc.clipboard_fragment(7..10).unwrap();
    let content = ClipboardContent::from_register(RegisterValue::from_clipboard_fragment(fragment).unwrap());
    assert_eq!(content.plain_text(), "![cat](<cat.png>)");
    let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(ClipboardTarget::Clipboard, ClipboardGeneration(1), content));
    let mut core = Core::new(document("", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
    for input in [InputEvent::key('i'), InputEvent::Key(Key::Ctrl('r')), InputEvent::key('+'), InputEvent::Key(Key::Escape)] {
        core.handle(view, CoreEvent::InputWithClipboard { input, clipboard: context.clone() }).unwrap();
    }
    assert_eq!(core.document().source_bytes(), tag.as_bytes());
    let images = core.document().projection().inline_images_for_region(&(0..3));
    assert_eq!((images[0].width, images[0].height), (Some(240), Some(80)));
    core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
    assert!(core.document().source_bytes().is_empty());
}

#[test]
fn html_image_location_edits_preserve_multiline_quote_owners_and_endings() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (file_format, ending) in [(FileFormat::Unix, "\n"), (FileFormat::Dos, "\r\n"), (FileFormat::Mac, "\r")] {
            for source in [
                "> <img\n> src='old.png'\n> width=240\n> height=80>",
                "> > <img\n> > src='old.png'\n> > width=240\n> > height=80>",
                "> <img src=\n> 'old.png' width=240 height=80>",
                "> <img alt='a\n> cat' src='old.png' width=240 height=80>",
                "> before <img\n> src='old.png'\n> width=240 height=80> after",
            ] {
                let original = source.replace('\n', ending);
                let mut doc = Document::from_bytes_with_file_format(original.as_bytes().to_vec(), Encoding::Utf8, format, file_format).unwrap();
                let image = doc.projection().inline_images_for_region(&(0..doc.text().len())).into_iter().next()
                    .unwrap_or_else(|| panic!("missing image {format:?} {file_format:?}: {source}"));
                let (edit, _) = doc.prepare_image_edit(doc.id(), doc.revision(), ImageEditIntent::Edit { range: image.range, text: image.text, destination: "new.png".into() })
                    .unwrap_or_else(|error| panic!("{format:?} {file_format:?}: {source}: {error:?}"));
                doc.commit_model_transaction(edit).unwrap();
                assert_eq!(doc.source_bytes(), original.replace("'old.png'", "\"new.png\"").as_bytes(), "{format:?} {file_format:?}: {source}");
                assert!(doc.undo());
                assert_eq!(doc.source_bytes(), original.as_bytes());
            }
        }
    }
}

#[test]
fn images_are_atomic_and_source_remains_literal_without_loading_resources() {
    let source = "before ![a **bold** cat](https://never-fetch.invalid/cat.png \"title\") after";
    let rich = document(source, Format::Markdown);
    assert_eq!(rich.text(), "before \u{fffc} after");
    let image = rich
        .image_snapshot_at(rich.text_point(7).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(image.range, 7..10);
    assert_eq!(image.text, "a bold cat");
    assert_eq!(image.destination, "https://never-fetch.invalid/cat.png");
    assert!(rich.text_point(8).is_err());
    assert_eq!(rich.source_bytes(), source.as_bytes());
    let raw = document(source, Format::MarkdownSource);
    assert_eq!(raw.text(), source);
    for at in 7..source.len() - 6 {
        assert_eq!(
            raw.image_snapshot_at(raw.text_point(at).unwrap())
                .unwrap()
                .unwrap()
                .range,
            7..source.len() - 6
        );
    }
}
#[test]
fn image_insertion_edit_removal_and_undo_preserve_unrelated_bytes_and_title() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let original = "before ![a **bold** cat](old.png 'kept title') after";
        let mut doc = document(original, format);
        let image = doc
            .image_snapshot_at(doc.text_point(7).unwrap())
            .unwrap()
            .unwrap();
        apply(
            &mut doc,
            ImageEditIntent::Edit {
                range: image.range,
                text: image.text,
                destination: "new file.png?x=1&y=2".into(),
            },
        );
        assert_eq!(
            doc.source_bytes(),
            b"before ![a **bold** cat](<new file.png?x=1&amp;y=2> 'kept title') after"
        );
        let edited = doc.source_bytes();
        let image = doc
            .image_snapshot_at(doc.text_point(7).unwrap())
            .unwrap()
            .unwrap();
        apply(&mut doc, ImageEditIntent::Remove { range: image.range });
        assert_eq!(doc.source_bytes(), b"before  after");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), edited);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original.as_bytes());
        apply(
            &mut doc,
            ImageEditIntent::Insert {
                range: 0..6,
                text: String::new(),
                destination: "local.png".into(),
            },
        );
        assert!(String::from_utf8(doc.source_bytes())
            .unwrap()
            .starts_with("![](<local.png>) "));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original.as_bytes());
    }
}
#[test]
fn reference_images_resolve_without_changing_definitions_and_edit_only_occurrence() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let original = "![first][img] and ![second][img]\n\n[img]: file.png \"title\"";
        let mut doc = document(original, format);
        let image = doc
            .image_snapshot_at(doc.text_point(0).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(image.text, "first");
        assert_eq!(image.destination, "file.png");
        apply(
            &mut doc,
            ImageEditIntent::Edit {
                range: image.range,
                text: "new".into(),
                destination: "other.png".into(),
            },
        );
        assert_eq!(
            doc.source_bytes(),
            b"![new](<other.png>) and ![second][img]\n\n[img]: file.png \"title\""
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original.as_bytes());
    }
}
#[test]
fn generic_deletion_and_replacement_consume_the_entire_image() {
    for replacement in ["", "cat"] {
        let mut doc = document("a ![alt](file.png) z", Format::Markdown);
        doc.replace(2..5, replacement).unwrap();
        assert_eq!(doc.text(), format!("a {replacement} z"));
        assert_eq!(doc.source_bytes(), format!("a {replacement} z").as_bytes());
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), b"a ![alt](file.png) z");
    }
}
#[test]
fn escaped_code_and_unresolved_images_remain_literal() {
    for source in [
        "\\![alt](file.png)",
        "`![alt](file.png)`",
        "![missing][unknown]",
        "    ![alt](file.png)",
    ] {
        let doc = document(source, Format::Markdown);
        assert!(
            doc.projection()
                .inline_images_for_region(&(0..doc.text().len()))
                .is_empty(),
            "{source}"
        );
    }
}
#[test]
fn stale_failed_and_unsupported_edits_are_atomic() {
    let mut doc = document("before after", Format::Markdown);
    let bytes = doc.source_bytes();
    let revision = doc.revision();
    let history = doc.history_status();
    for (text, destination) in [("bad\nalt", "file.png"), ("alt", "file\n.png"), ("alt", "")] {
        assert!(doc
            .prepare_image_edit(
                doc.id(),
                revision,
                ImageEditIntent::Insert {
                    range: 0..6,
                    text: text.into(),
                    destination: destination.into()
                }
            )
            .is_err());
        assert_eq!(doc.source_bytes(), bytes);
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.history_status(), history);
    }
    let (prepared, _) = doc
        .prepare_image_edit(
            doc.id(),
            revision,
            ImageEditIntent::Insert {
                range: 0..6,
                text: "alt".into(),
                destination: "file.png".into(),
            },
        )
        .unwrap();
    doc.insert(0, "x ").unwrap();
    assert!(doc.commit_model_transaction(prepared).is_err());
    assert_eq!(doc.source_bytes(), b"x before after");
}
#[test]
fn image_metadata_rebases_after_local_edits_and_matches_reopened_source() {
    let mut doc = document("before\n\n![cat](cat.png)\n\nafter", Format::Markdown);
    doc.insert(0, "new ").unwrap();
    let image = doc
        .projection()
        .inline_images_for_region(&(0..doc.text().len()))
        .pop()
        .unwrap();
    assert_eq!(image.destination, "cat.png");
    apply(
        &mut doc,
        ImageEditIntent::Edit {
            range: image.range,
            text: "dog".into(),
            destination: "dog.png".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"new before\n\n![dog](<dog.png>)\n\nafter"
    );
    let reopened =
        Document::from_bytes(doc.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(doc.text(), reopened.text());
    assert_eq!(
        doc.projection()
            .inline_images_for_region(&(0..doc.text().len())),
        reopened
            .projection()
            .inline_images_for_region(&(0..reopened.text().len()))
    );
}
#[test]
fn images_form_object_boundaries_next_to_combining_and_prepend_characters() {
    for source in ["![cat](cat.png)\u{301}z", "\u{600}![cat](cat.png)z"] {
        let mut doc = document(source, Format::Markdown);
        let image = doc
            .projection()
            .inline_images_for_region(&(0..doc.text().len()))
            .pop()
            .unwrap();
        let snapshot = doc.hard_line_snapshot();
        assert!(snapshot.is_grapheme_boundary(image.range.start));
        assert!(snapshot.is_grapheme_boundary(image.range.end));
        assert_eq!(
            snapshot.next_grapheme_boundary(image.range.start),
            Some(image.range.end)
        );
        assert_eq!(
            snapshot.previous_grapheme_boundary(image.range.end),
            Some(image.range.start)
        );
        doc.replace(image.range, "").unwrap();
        assert!(!doc.text().contains('\u{fffc}'));
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}
#[test]
fn multiline_images_have_complete_source_popup_ranges() {
    let source = "![a\ncat](file.png)";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(source, format);
        let image = doc
            .image_snapshot_at(doc.text_point(0).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(image.text, "a cat");
        assert_eq!(
            image.range,
            if format.is_source_view() {
                0..source.len()
            } else {
                0..3
            }
        );
    }
}
#[test]
fn local_inline_image_edits_in_large_documents_retain_incremental_projection() {
    let mut source = "ordinary prose\n\n".repeat(20_000);
    source.push_str("![cat](cat.png)\n\nend");
    let mut doc = document(&source, Format::Markdown);
    let before = doc.projection().blocks().first().unwrap().id;
    let range = doc
        .projection()
        .inline_images_for_region(&(doc.text().len() - 16..doc.text().len()))
        .pop()
        .unwrap()
        .range;
    let (prepared, _) = doc
        .prepare_image_edit(
            doc.id(),
            doc.revision(),
            ImageEditIntent::Edit {
                range,
                text: "dog".into(),
                destination: "dog.png".into(),
            },
        )
        .unwrap();
    assert_eq!(
        prepared.summary().projection_work().scope(),
        ProjectionWorkScope::RegionalHardLines
    );
    assert!(prepared.summary().projection_work().decoded_source_bytes() < 1024);
    doc.commit_model_transaction(prepared).unwrap();
    assert_eq!(doc.projection().blocks().first().unwrap().id, before);
    assert!(doc.source_bytes().ends_with(b"![dog](<dog.png>)\n\nend"));
}
#[test]
fn native_authoring_and_normal_delete_share_command_undo() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(document("before after", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
    let expected = core.list_selection_identity(view).unwrap();
    core.edit_image(
        view,
        expected.clone(),
        ImageEditIntent::Insert {
            range: expected.range(),
            text: "cat".into(),
            destination: "cat.png".into(),
        },
    )
    .unwrap();
    let inserted = core.document().source_bytes();
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
    assert!(core
        .edit_image(
            view,
            expected,
            ImageEditIntent::Insert {
                range: 0..0,
                text: "dog".into(),
                destination: "dog.png".into()
            }
        )
        .is_err());
    core.handle(view, CoreEvent::Input(InputEvent::text("x")))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"before after");
    core.handle(view, CoreEvent::Input(InputEvent::text("u")))
        .unwrap();
    assert_eq!(core.document().source_bytes(), inserted);
    core.handle(view, CoreEvent::Input(InputEvent::text("u")))
        .unwrap();
    assert_eq!(core.document().source_bytes(), b"before after");
}
#[test]
fn image_authoring_preserves_utf16_and_latin1_encodings() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Latin1] {
        let original = match encoding {
            Encoding::Utf16Le => "a ![café](old.png) z"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
            Encoding::Utf16Be => "a ![café](old.png) z"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect(),
            _ => "a ![café](old.png) z".chars().map(|ch| ch as u8).collect(),
        };
        let mut doc = Document::from_bytes(original, encoding, Format::Markdown).unwrap();
        let original = doc.source_bytes();
        let image = doc
            .image_snapshot_at(doc.text_point(2).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(image.text, "café");
        apply(
            &mut doc,
            ImageEditIntent::Edit {
                range: image.range,
                text: image.text,
                destination: "new.png".into(),
            },
        );
        assert_eq!(doc.encoding(), encoding);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), original);
    }
}
#[test]
fn clicking_an_image_ignores_autoselect_and_backspace_removes_only_that_image() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for mode in ["", "i", "R"] {
        let mut core = Core::new(document("left ![cat](cat.png) right", Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        if !mode.is_empty() {
            core.handle(view, CoreEvent::Input(InputEvent::text(mode)))
                .unwrap();
        }
        core.select_image(view, core.document().id(), core.document().revision(), 5)
            .unwrap();
        assert!(core.command_state(view).unwrap().is_native_selection());
        assert_eq!(core.list_selection_identity(view).unwrap().range(), 5..8);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(core.document().source_bytes(), b"left  right");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("u")))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            b"left ![cat](cat.png) right"
        );
    }
}
#[test]
fn literal_object_replacement_characters_retain_unicode_grapheme_boundaries() {
    for format in [Format::PlainText, Format::Code, Format::MarkdownSource] {
        let doc = document("\u{fffc}\u{301}", format);
        assert!(!doc.hard_line_snapshot().is_grapheme_boundary(3));
    }
}
#[test]
fn system_clipboard_plain_image_text_matches_the_retained_private_register() {
    use viem_core::command::RegisterValue;
    use viem_core::ClipboardContent;
    let doc = document("before ![cat](cat.png) after", Format::Markdown);
    let fragment = doc.clipboard_fragment(7..10).unwrap();
    let register = RegisterValue::from_clipboard_fragment(fragment).unwrap();
    let content = ClipboardContent::from_register(register);
    assert_eq!(content.plain_text(), "![cat](<cat.png>)");
    let exported = content
        .portable_register()
        .unwrap()
        .clipboard_fragment()
        .unwrap();
    let imported = ClipboardFragment::from_json(exported.json(), content.plain_text()).unwrap();
    let register = RegisterValue::from_clipboard_fragment(imported).unwrap();
    assert_eq!(register.text, "\u{fffc}");
    ClipboardContent::try_new(content.plain_text(), Some(register)).unwrap();
}

#[test]
fn insert_replace_arrow_navigation_selects_images_and_arrows_resume_typing_mode() {
    use viem_core::command::{InputEvent, Key, Mode};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for entry in ["i", "R"] {
        let mut core = Core::new(document("A![alt](image.png)B", Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        core.handle(view, CoreEvent::Input(InputEvent::text(entry)))
            .unwrap();
        let typing_mode = core.command_state(view).unwrap().mode();
        for (movement, selection, offset) in [
            (Key::Right, true, 1),
            (Key::Right, false, 4),
            (Key::Left, true, 1),
            (Key::Left, false, 1),
        ] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(movement)))
                .unwrap();
            assert_eq!(
                core.command_state(view).unwrap().is_native_selection(),
                selection,
                "{entry} {movement:?}"
            );
            assert_eq!(core.command_state(view).unwrap().cursor(), offset);
            if selection {
                assert_eq!(core.list_selection_identity(view).unwrap().range(), 1..4);
            } else {
                assert_eq!(core.command_state(view).unwrap().mode(), typing_mode);
            }
        }
        assert!(matches!(typing_mode, Mode::Insert | Mode::Replace));
        assert_eq!(core.document().source_bytes(), b"A![alt](image.png)B");
    }
}

#[test]
fn image_navigation_selection_backspace_and_typing_are_separate_undo_units() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for entry in ["i", "R"] {
        for movement in [Key::Left, Key::Right] {
            for replace in [false, true] {
                let original = "A![alt](image.png)B";
                let mut core = Core::new(document(original, Format::Markdown));
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
                core.handle(view, CoreEvent::Input(InputEvent::text(entry)))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::text("x")))
                    .unwrap();
                let typed = core.document().source_bytes();
                let image = core
                    .document()
                    .projection()
                    .inline_images_for_region(&(0..core.document().text().len()))
                    .pop()
                    .unwrap();
                let at = if movement == Key::Right {
                    image.range.start - 1
                } else {
                    image.range.end
                };
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: at,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(movement)))
                    .unwrap();
                assert!(
                    core.command_state(view).unwrap().is_native_selection(),
                    "{entry} {movement:?}"
                );
                if replace {
                    core.handle(view, CoreEvent::Input(InputEvent::text("new")))
                        .unwrap();
                } else {
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
                        .unwrap();
                }
                assert!(!core
                    .document()
                    .source_bytes()
                    .windows(2)
                    .any(|window| window == b"!["));
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::text("u")))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), typed);
                core.handle(view, CoreEvent::Input(InputEvent::text("u")))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), original.as_bytes());
            }
        }
    }
}

#[test]
fn image_navigation_keeps_source_normal_and_plain_insert_entry_semantics() {
    use viem_core::command::{InputEvent, Key, Mode};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core = Core::new(document("A![alt](image.png)B", format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Right)))
            .unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        core.handle(view, CoreEvent::Input(InputEvent::text("i")))
            .unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert!(!core.command_state(view).unwrap().is_native_selection());
        if format.is_source_view() {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Right)))
                .unwrap();
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        }
    }
}

#[test]
fn dragging_from_a_selected_image_retains_the_whole_origin_object() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(document("A![cat](image.png)BC", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
    for (active, expected) in [(6, 1..6), (0, 0..4)] {
        core.select_image(view, core.document().id(), core.document().revision(), 1)
            .unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: active,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: true,
            },
        )
        .unwrap();
        assert!(core.command_state(view).unwrap().is_native_selection());
        assert_eq!(
            core.list_selection_identity(view).unwrap().range(),
            expected
        );
    }
    assert_eq!(core.document().source_bytes(), b"A![cat](image.png)BC");
}

#[test]
fn prose_edits_near_html_images_keep_projection_work_bounded_as_documents_grow() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for paragraphs in [10, 10_000] {
            let source = format!(
                "before <img src='cat.png' width=240 height=80> after\n\n{}",
                "untouched paragraph\n\n".repeat(paragraphs),
            );
            let mut doc = document(&source, format);
            let tail = doc.projection().hard_line_id(doc.line_count() - 1);
            let prepared = doc.prepare_model_request(ModelRequest::ApplyTextEdits {
                document: doc.id(), revision: doc.revision(),
                edits: vec![TextEdit::new(3..3, "x")],
            }).unwrap();
            let work = prepared.summary().projection_work();
            assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines, "{format:?}: {work:?}");
            assert!(work.decoded_source_bytes() < 1024, "{format:?}: {work:?}");
            assert_eq!(work.full_text_bytes_materialized(), 0, "{format:?}: {work:?}");
            doc.commit_model_transaction(prepared).unwrap();
            assert_eq!(doc.projection().hard_line_id(doc.line_count() - 1), tail);
            let fresh = Document::from_bytes(doc.source_bytes(), Encoding::Utf8, format).unwrap();
            assert_eq!(doc.text(), fresh.text());
            let range = 0..doc.text().len();
            assert_eq!(doc.projection().inline_images_for_region(&range), fresh.projection().inline_images_for_region(&range));
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
        }
    }
}
