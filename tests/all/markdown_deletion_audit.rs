use viem_core::document::*;

#[test]
fn every_legal_deletion_range_preserves_markdown_projection() {
    let fixtures = [
        "a `b` c",
        "a **b** c",
        "a *b* c",
        "first `code`\nsecond `more` end",
        "first `code`\n\nsecond `more` end",
        "a `bc` d",
        "` a `",
        "`  `",
        "a **b `c` d** e",
        "one\nsecond\nthird",
        "a [b](https://example.com) c",
        "- first `code`\n- second `more` end",
        "# title\n\nbody `c`",
        "a ` b` c",
        "a `b ` c",
    ];
    let mut failures = Vec::new();
    let extra = [
        "a `` b`c `` d",
        "**a**_b_ c",
        "a **b c** d",
        "a\n\nb\n\nc",
        "a  \nb",
        "a\\\nb",
        "> a `b`\n> c `d`",
        "a &amp; b",
        "a ` b c ` d",
        "a `  b  ` c",
        "- a  \n  b\n- c",
        "first\n\n```\ncode\nmore\n```\n\nlast",
        "` a` `b `",
        "a `x`b`y` c",
        "a é `👩‍💻` b",
    ];
    for source in fixtures.into_iter().chain(extra) {
        let doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let text = doc.text().to_owned();
        for start in 0..text.len() {
            for end in start + 1..=text.len() {
                if doc.text_point(start).is_err() || doc.text_point(end).is_err() {
                    continue;
                }
                let result = doc.prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: doc.id(),
                    revision: doc.revision(),
                    edits: vec![TextEdit::new(start..end, "")],
                });
                if let Err(e) = result {
                    failures.push(format!("{source:?} {text:?} {start}..{end}: {e:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} deletion failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(150)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;
fn key(core: &mut Editor, view: ViewId, key: Key) {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    assert!(
        matches!(
            output.command.unwrap().status,
            CommandStatus::Complete | CommandStatus::Pending
        ),
        "{key:?}"
    );
}

#[test]
fn visual_x_across_wrapped_physical_lines_keeps_register_and_exact_undo() {
    let source = "start `code`\nthen text in the next physical line and `more` at the end.\n\nlast paragraph";
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 120., 500.);
    let text = core.document().text().to_owned();
    for character in "v3j".chars() {
        key(&mut core, view, Key::Char(character));
    }
    let range = core.list_selection_identity(view).unwrap().range();
    let selected = text[range.clone()].to_owned();
    assert!(selected.contains("code then"), "{selected:?}");
    let expected = format!("{}{}", &text[..range.start], &text[range.end..]);
    for character in "\"ax".chars() {
        key(&mut core, view, Key::Char(character));
    }
    assert_eq!(core.document().text(), expected);
    assert_eq!(
        core.command_state(view)
            .unwrap()
            .register('a')
            .unwrap()
            .text,
        selected
    );
    let saved = core.document().source_bytes();
    assert_eq!(
        Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown)
            .unwrap()
            .text(),
        expected
    );
    key(&mut core, view, Key::Char('u'));
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    key(&mut core, view, Key::Ctrl('r'));
    assert_eq!(core.document().source_bytes(), saved);
}

#[test]
fn normal_x_removes_final_code_grapheme_and_insert_backspace_uses_same_policy() {
    for source in ["left `é` right", "left `` 👩‍💻 `` right"] {
        for insert in [false, true] {
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 180., 300.);
            core.handle(
                view,
                CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(),
                    text_offset: 5,
                    affinity: BoundaryAffinity::Downstream,
                    extend_selection: false,
                },
            )
            .unwrap();
            if insert {
                key(&mut core, view, Key::Char('a'));
                key(&mut core, view, Key::Backspace);
                key(&mut core, view, Key::Escape);
            } else {
                key(&mut core, view, Key::Char('x'));
            }
            assert_eq!(core.document().text(), "left  right");
            assert_eq!(core.document().source_bytes(), b"left  right");
            key(&mut core, view, Key::Char('u'));
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}

#[test]
fn text_and_backtick_replacements_across_code_boundaries_are_representable() {
    let mut failures = Vec::new();
    for source in [
        "a `b` c",
        "a ` b c ` d",
        "first `code`\nsecond `more` end",
        "> a `b`\n> c `d`",
        "- a  \n  b\n- c",
        "a [b](https://example.com) c",
        "**ab**_cd_",
    ] {
        let doc =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        for start in 0..=doc.text().len() {
            for end in start..=doc.text().len() {
                for replacement in ["x", " ", "`"] {
                    let result = doc.prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: doc.id(),
                        revision: doc.revision(),
                        edits: vec![TextEdit::new(start..end, replacement)],
                    });
                    if let Err(e) = result {
                        failures.push(format!(
                            "{source:?} {start}..{end} => {replacement:?}: {e:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} replacement failures\n{}",
        failures.len(),
        failures
            .iter()
            .take(100)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn encoded(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => [
            vec![0xff, 0xfe],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        Encoding::Utf16Be => [
            vec![0xfe, 0xff],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
    }
}

#[test]
fn code_repairs_preserve_source_bytes_encodings_line_endings_and_history() {
    for (source, selected, replacement) in [
        ("before\n\na `b` c\n\nafter", "b", ""),
        (
            "before\n\nfirst `code`\nsecond `more` end\n\nafter",
            "ode second m",
            "",
        ),
        (
            "before\n\nfirst `code`\nsecond `more` end\n\nafter",
            "ode second m",
            "`",
        ),
        ("before\n\n> a `b`\n> c `d`\n\nafter", "a b", ""),
        ("before\n\n- a  \n  b\n- c\n\nafter", "b\nc", ""),
    ] {
        for encoding in [
            Encoding::Utf8,
            Encoding::Latin1,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for (ending, format) in [
                ("\n", FileFormat::Unix),
                ("\r\n", FileFormat::Dos),
                ("\r", FileFormat::Mac),
            ] {
                let original = encoded(&source.replace('\n', ending), encoding);
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::Markdown,
                    format,
                )
                .unwrap();
                let start = document.text().find(selected).unwrap();
                let range = start..start + selected.len();
                let expected = format!(
                    "{}{}{}",
                    &document.text()[..range.start],
                    replacement,
                    &document.text()[range.end..]
                );
                let prepared = document
                    .prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(range, replacement)],
                    })
                    .unwrap_or_else(|error| {
                        panic!("{source:?}, {selected:?}, {encoding:?}, {format:?}: {error:?}")
                    });
                let patches = prepared.summary().source_patches().to_vec();
                document.commit_model_transaction(prepared).unwrap();
                let after = document.source_bytes();
                let (mut old_at, mut new_at) = (0, 0);
                for patch in patches {
                    let untouched = patch.range().start - old_at;
                    assert_eq!(
                        &original[old_at..old_at + untouched],
                        &after[new_at..new_at + untouched]
                    );
                    old_at = patch.range().end;
                    new_at += untouched + patch.replacement().len();
                }
                assert_eq!(&original[old_at..], &after[new_at..]);
                assert_eq!(document.text(), expected);
                assert_eq!(
                    Document::from_bytes_with_file_format(
                        after.clone(),
                        encoding,
                        Format::Markdown,
                        format
                    )
                    .unwrap()
                    .text(),
                    expected
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), after);
            }
        }
    }
}

#[test]
fn disjoint_deletions_that_empty_one_code_span_commit_atomically() {
    let original = b"before **`abcd`** after";
    let mut document =
        Document::from_bytes(original.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = document.text().find("abcd").unwrap();
    document
        .apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![
                TextEdit::new(at..at + 2, ""),
                TextEdit::new(at + 2..at + 4, ""),
            ],
        })
        .unwrap();
    assert_eq!(document.text(), "before  after");
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    assert!(!document.undo());
}
