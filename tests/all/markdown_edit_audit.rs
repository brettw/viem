use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

const SOURCES: &[&str] = &[
    "- first\n- second",
    "- first\n",
    "- first\n\n",
    "- first\n\nTail",
    "- first  \n  continued\n- tail",
    "- first\\\n  continued\n- tail",
    "- first\n  continued\n- tail",
    "- first\n\n  continued\n- tail",
    "-",
    "- ",
    "-\t",
    "- first\n-\n- tail",
    "- first\n- \n- tail",
    "1.",
    "1. first\n2.\n3. tail",
    "1.\tfirst\n2.\tlast",
    "- parent\n  9. child\n     continued\n  10. next\n- tail",
    "- a\n  - b\n    - c\n      deep\n- tail",
    "> - first\n> - second",
    "> - first\n>   continued\n> - tail",
    "> - first\n>\n>   continued\n> - tail",
    "> -\n> - tail",
    "before\n\n> quote\n> continued\n\nafter",
    "- a\n  ```\n  code\n  ```\n- b",
    "> ```\n> code\n> ```",
    "before\n```rust\nx\n```\nafter",
    "```\n\n```",
    "```",
    "    code\n\nTail",
    "one *two\nthree* [label](url) `x`",
    "Title\n---\nTail",
    "a\n\n\n\nb",
    "- café 👩‍💻\n- e\u{301}",
    "<div>\nx\n</div>\n\nTail",
];

fn report(name: &str, checked: usize, failures: &[String]) {
    eprintln!("{name}: {checked} cases, {} failures", failures.len());
    if let Some(directory) = std::env::var_os("VIEM_EDIT_AUDIT_REPORT_DIR") {
        std::fs::write(
            std::path::PathBuf::from(directory).join(format!("{name}.txt")),
            format!(
                "{checked} cases; {} failures\n{}\n",
                failures.len(),
                failures.join("\n")
            ),
        )
        .unwrap();
    }
}

fn verify_reopened(doc: &Document) -> Result<(), String> {
    let fresh = Document::from_bytes_with_file_format(
        doc.source_bytes(),
        doc.encoding(),
        doc.format(),
        doc.file_format(),
    )
    .unwrap();
    if doc.text() != fresh.text()
        || doc
            .projection()
            .blocks()
            .iter()
            .map(|b| (&b.range, &b.attributes))
            .ne(fresh
                .projection()
                .blocks()
                .iter()
                .map(|b| (&b.range, &b.attributes)))
        || doc.projection().style_spans() != fresh.projection().style_spans()
        || doc
            .hard_line_snapshot()
            .capture(0..doc.text().len())
            .unwrap()
            .break_offsets()
            != fresh
                .hard_line_snapshot()
                .capture(0..fresh.text().len())
                .unwrap()
                .break_offsets()
    {
        return Err(format!(
            "reopen mismatch: {:?}",
            String::from_utf8_lossy(&doc.source_bytes())
        ));
    }
    Ok(())
}

fn event(
    core: &mut Core<MockTextMeasurementProvider>,
    view: viem_core::ViewId,
    input: InputEvent,
) -> Result<(), String> {
    let outcome = core
        .handle_with_layout(view, CoreEvent::Input(input))
        .map_err(|e| format!("{e:?}"))?;
    if let Some(command) = outcome.command {
        if !matches!(
            command.status,
            CommandStatus::Complete | CommandStatus::Pending
        ) {
            return Err(format!("{:?}", command.status));
        }
    }
    Ok(())
}

fn check(source: &str, format: Format, at: usize, action: &str) -> Result<(), String> {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 500.);
    if !matches!(action, "o" | "O") {
        event(&mut core, view, InputEvent::key('i'))?;
    }
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .map_err(|e| format!("place: {e:?}"))?;
    let input = match action {
        "Enter" => InputEvent::Key(Key::Enter),
        "Backspace" => InputEvent::Key(Key::Backspace),
        "Delete" => InputEvent::Key(Key::Delete),
        "o" => InputEvent::key('o'),
        "O" => InputEvent::key('O'),
        text => InputEvent::text(text),
    };
    let mut expected = core.document().text().to_owned();
    let literal_input = matches!(action, "x" | "*" | "`" | " " | "x\ny");
    if literal_input {
        expected.insert_str(at, action);
    }
    event(&mut core, view, input)?;
    if format == Format::Markdown && literal_input && core.document().text() != expected {
        return Err(format!(
            "input differs: expected {expected:?}, got {:?}",
            core.document().text()
        ));
    }
    verify_reopened(core.document())?;
    // The new caret and line metadata must support the very next keystroke,
    // including after splitting the former EOF row or removing an empty item.
    if format == Format::MarkdownSource {
        let before = core
            .document()
            .source_bytes()
            .iter()
            .filter(|&&byte| byte == b'z')
            .count();
        event(&mut core, view, InputEvent::text("z"))?;
        let after = core
            .document()
            .source_bytes()
            .iter()
            .filter(|&&byte| byte == b'z')
            .count();
        if after != before + 1 {
            return Err("following source input was lost".into());
        }
        verify_reopened(core.document())?;
    }
    event(&mut core, view, InputEvent::Key(Key::Escape))?;
    let saved = core.document().source_bytes();
    verify_reopened(core.document())?;
    if saved != source.as_bytes() {
        event(&mut core, view, InputEvent::key('u'))?;
        if core.document().source_bytes() != source.as_bytes() {
            return Err("undo mismatch".into());
        }
        event(&mut core, view, InputEvent::Key(Key::Ctrl('r')))?;
        if core.document().source_bytes() != saved {
            return Err("redo mismatch".into());
        }
    }
    Ok(())
}

#[test]
fn source_insertions_deletions_and_line_opening_accept_every_legal_caret() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for source in SOURCES {
        let doc = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        for at in 0..=doc.text().len() {
            if doc.text_point(at).is_err() {
                continue;
            }
            for action in [
                "x",
                "*",
                "`",
                " ",
                "x\ny",
                "Enter",
                "Backspace",
                "Delete",
                "o",
                "O",
            ] {
                checked += 1;
                if let Err(error) = check(source, Format::MarkdownSource, at, action) {
                    failures.push(format!("{action:?} at {at}: {error} | {source:?}"));
                }
            }
        }
    }
    report("source-carets", checked, &failures);
    assert!(
        failures.is_empty(),
        "{checked} cases; {} failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(70)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn wysiwyg_insertions_deletions_and_line_opening_accept_every_legal_caret() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for source in SOURCES.iter().copied() {
        let doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for at in 0..=doc.text().len() {
            if doc.text_point(at).is_err() {
                continue;
            }
            for action in [
                "x",
                "*",
                "`",
                " ",
                "x\ny",
                "Enter",
                "Backspace",
                "Delete",
                "o",
                "O",
            ] {
                checked += 1;
                if let Err(error) = check(source, Format::Markdown, at, action) {
                    failures.push(format!("{action:?} at {at}: {error} | {source:?}"));
                }
            }
        }
    }
    report("wysiwyg", checked, &failures);
    assert!(
        failures.is_empty(),
        "{checked} cases; {} failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn source_range_deletions_and_replacements_accept_every_legal_range() {
    use viem_core::document::TextEdit;
    let mut failures = Vec::new();
    let mut checked = 0;
    for source in SOURCES {
        let original = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        for start in 0..original.text().len() {
            for end in start + 1..=original.text().len() {
                if original.text_point(start).is_err() || original.text_point(end).is_err() {
                    continue;
                }
                for replacement in ["", "x", "x\ny"] {
                    checked += 1;
                    let mut doc = Document::from_bytes(
                        source.as_bytes().to_vec(),
                        Encoding::Utf8,
                        Format::MarkdownSource,
                    )
                    .unwrap();
                    if let Err(error) =
                        doc.apply_edits(vec![TextEdit::new(start..end, replacement)])
                    {
                        failures.push(format!(
                            "{start}..{end} -> {replacement:?}: {error:?} | {source:?}"
                        ));
                        continue;
                    }
                    let saved = doc.source_bytes();
                    if let Err(error) = verify_reopened(&doc) {
                        failures.push(format!(
                            "{start}..{end} -> {replacement:?}: {error} | {source:?}"
                        ));
                    }
                    if saved != source.as_bytes() {
                        assert!(doc.undo());
                        assert_eq!(doc.source_bytes(), source.as_bytes());
                        assert!(doc.redo());
                        assert_eq!(doc.source_bytes(), saved);
                    }
                }
            }
        }
    }
    report("source-ranges", checked, &failures);
    assert!(
        failures.is_empty(),
        "{checked} cases; {} failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn indented_markdown_code_editing() {
    for source in [
        "    code\n\nTail",
        "    first\n    second",
        "\tcode",
        ">     code\n>     next",
        "- item\n\n      code\n- tail",
        "-     code",
        "- x\n\n\t\tcode",
    ] {
        for at in
            0..=Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap()
                .text()
                .len()
        {
            for action in [
                "x",
                "*",
                "`",
                " ",
                "x\ny",
                "Enter",
                "Backspace",
                "Delete",
                "o",
                "O",
            ] {
                check(source, Format::Markdown, at, action)
                    .unwrap_or_else(|error| panic!("{source:?}: {action} at {at}: {error}"));
            }
        }
    }
}

#[test]
fn source_bullet_enter_keeps_typing_in_the_new_item_across_encodings_and_line_modes() {
    use viem_core::command::LineMode;
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        let encode = |text: &str| match encoding {
            Encoding::Utf16Le => text
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
            Encoding::Utf16Be => text
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>(),
            _ => text.as_bytes().to_vec(),
        };
        for ending in ["\n", "\r\n", "\r"] {
            for line_mode in [LineMode::Visual, LineMode::PhysicalSource] {
                for (first, tail, marker) in [
                    ("- first", "first", "- "),
                    ("> - first", "first", "> - "),
                    ("> - first\n>\n>   continued", "continued", "> - "),
                ] {
                    let source = format!("{first}\n{marker}last").replace('\n', ending);
                    let original = encode(&source);
                    let document = Document::from_bytes_with_file_format(
                        original.clone(),
                        encoding,
                        Format::MarkdownSource,
                        match ending {
                            "\r" => viem_core::document::FileFormat::Mac,
                            "\r\n" => viem_core::document::FileFormat::Dos,
                            _ => viem_core::document::FileFormat::Unix,
                        },
                    )
                    .unwrap();
                    let mut core = Core::new(document);
                    let view = core.add_view(MockTextMeasurementProvider::new(), 180., 500.);
                    core.handle(view, CoreEvent::SetLineMode(line_mode))
                        .unwrap();
                    event(&mut core, view, InputEvent::key('i')).unwrap();
                    let at = core.document().text().find(tail).unwrap() + tail.len();
                    core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset: at,
                            affinity: BoundaryAffinity::Upstream,
                            extend_selection: false,
                        },
                    )
                    .unwrap();
                    for input in [
                        InputEvent::Key(Key::Enter),
                        InputEvent::text("Added"),
                        InputEvent::Key(Key::Escape),
                    ] {
                        event(&mut core, view, input).unwrap();
                    }
                    assert_eq!(
                        core.document().source_bytes(),
                        encode(
                            &format!("{first}\n{marker}Added\n{marker}last").replace('\n', ending)
                        )
                    );
                    verify_reopened(core.document()).unwrap();
                    event(&mut core, view, InputEvent::key('u')).unwrap();
                    assert_eq!(core.document().source_bytes(), original);
                }
            }
        }
    }
}

#[test]
fn unindenting_a_numbered_item_keeps_later_siblings_nested_with_marker_padding() {
    use viem_core::document::{BlockKind, ModelRequest};
    for source in [
        "- parent\n  9. child\n     continued\n  10. next\n- tail",
        "- parent\n  9.\tchild\n      continued\n\n  10. next\n- tail",
    ] {
        let mut doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let at = doc.text().find("child").unwrap();
        let expected = doc.text().to_owned();
        doc.apply_model_request(ModelRequest::IndentList {
            document: doc.id(),
            revision: doc.revision(),
            range: at..at,
            unindent: true,
        })
        .unwrap();
        assert_eq!(doc.text(), expected);
        let next = doc.text().find("next").unwrap();
        assert!(doc.projection().blocks().iter().any(
            |b| b.range.start == next && matches!(b.kind, BlockKind::ListItem { level: 1, .. })
        ));
        verify_reopened(&doc).unwrap();
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn paragraphs_opened_beside_code_keep_their_neighbors_in_encoded_sources() {
    use viem_core::document::FileFormat;
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for (ending, file_format) in [
            ("\n", FileFormat::Unix),
            ("\r\n", FileFormat::Dos),
            ("\r", FileFormat::Mac),
        ] {
            let source = "before\n```rust\nx\n```\nafter".replace('\n', ending);
            let bytes = match encoding {
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => source.as_bytes().to_vec(),
            };
            for (key, expected) in [
                ('o', "before\nx\nnew\nafter"),
                ('O', "before\nnew\nx\nafter"),
            ] {
                let document = Document::from_bytes_with_file_format(
                    bytes.clone(),
                    encoding,
                    Format::Markdown,
                    file_format,
                )
                .unwrap();
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 180., 500.);
                core.handle(
                    view,
                    CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: 7,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: false,
                    },
                )
                .unwrap();
                for input in [
                    InputEvent::key(key),
                    InputEvent::text("new"),
                    InputEvent::Key(Key::Escape),
                ] {
                    event(&mut core, view, input).unwrap();
                }
                assert_eq!(core.document().text(), expected);
                verify_reopened(core.document()).unwrap();
                event(&mut core, view, InputEvent::key('u')).unwrap();
                assert_eq!(core.document().source_bytes(), bytes);
            }
        }
    }
}
