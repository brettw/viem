use viem_core::command::{InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

#[test]
fn deleting_blank_code_tail_keeps_mixed_endings_and_bounded_work() {
    use viem_core::document::{measure_document_work, FileFormat};
    let source = b"```rust\r\nfirst\nsecond\r\n\r\n```\n\r\nFollowing";
    let mut document = Document::from_bytes_with_file_format(
        source.to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    document.delete_lines(13..14).unwrap();
    assert_eq!(
        document.source_bytes(),
        b"```rust\r\nfirst\nsecond\r\n```\n\r\nFollowing"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
    for count in [100, 5000] {
        let source = format!(
            "```rust\nfirst\nsecond\n\n```\n\n## Following\n\n{}",
            "Unrelated *prose*.\n\n".repeat(count)
        );
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let (result, work) = measure_document_work(|| document.delete_lines(13..14));
        result.unwrap();
        assert_eq!(work.source_full_materializations, 0, "{count}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{count}: {work:?}");
        assert_eq!(
            work.formatted_full_materialized_bytes, 0,
            "{count}: {work:?}"
        );
        assert!(work.source_decoded_bytes < 4096, "{count}: {work:?}");
        assert_eq!(
            document.source_bytes(),
            source.replacen("second\n\n", "second\n", 1).as_bytes()
        );
    }
}

#[test]
fn code_tail_deletion_preserves_encodings_source_and_join_policy() {
    use super::markdown_test_support::EncodedFixture;
    use viem_core::document::FileFormat;
    for encoding in [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Latin1,
    ] {
        for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
            for command in ["dd", "dk"] {
                let fixture = EncodedFixture {
                    encoding,
                    file_format,
                    bom: encoding != Encoding::Latin1,
                };
                let source = "```rust\nfirst\nsecond\n\n```\n\n## Following";
                let mut core = Core::new(fixture.open(source));
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
                for key in format!("jj{command}").chars() {
                    core.handle(view, CoreEvent::Input(InputEvent::key(key)))
                        .unwrap();
                }
                let expected = if command == "dd" {
                    "```rust\nfirst\nsecond\n```\n\n## Following"
                } else {
                    "```rust\nfirst\n```\n\n## Following"
                };
                assert_eq!(
                    core.document().source_bytes(),
                    fixture.bytes(expected),
                    "{fixture:?} {command}"
                );
                let reopened = fixture.open(expected);
                assert_eq!(core.document().text(), reopened.text());
                core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), fixture.bytes(source));
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                    .unwrap();
                assert_eq!(core.document().source_bytes(), fixture.bytes(expected));
            }
        }
    }
    let source = "```rust\nfirst\n\n```\n\nFollowing";
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 600.);
    for key in "jJ".chars() {
        core.handle(view, CoreEvent::Input(InputEvent::key(key)))
            .unwrap();
    }
    assert_eq!(core.document().projection().blocks().len(), 1);
    assert_eq!(
        core.document().projection().blocks()[0].style.0,
        "Code Block"
    );
    assert!(core.document().text().ends_with("Following"));
}

#[test]
fn deleting_bottom_code_rows_keeps_following_paragraph_outside() {
    let mut failures = Vec::new();
    for body in ["first line\nsecond line has several words that wrap\nlast line has several words that wrap", "first\nsecond\n", "first\n\n", "\n", ""] {
    for form in ["fence", "tilde", "quote", "list", "html", "front-matter", "indented"] {
    for width in [600., 100.] {
        'case: for command in ["dd", "dk", "d-up", "Vd", "Vkd", "native-delete", "native-up-delete"] {
            let source = match form {
                "tilde" => format!("~~~~rust\n{body}\n~~~~\n\nFollowing paragraph"),
                "quote" => format!("> ```rust\n> {}\n> ```\n\nFollowing paragraph", body.replace('\n', "\n> ")),
                "list" => format!("- item\n\n  ```rust\n  {}\n  ```\n\nFollowing paragraph", body.replace('\n', "\n  ")),
                "html" => format!("<pre><code>{body}</code></pre>\n\nFollowing paragraph"),
                "front-matter" => format!("---\n{body}\n---\n\nFollowing paragraph"),
                "indented" => format!("before\n\n    {}\n\nFollowing paragraph", body.replace('\n', "\n    ")),
                _ => format!("```rust\n{body}\n```\n\nFollowing paragraph"),
            };
            let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap());
            let view = core.add_view(MockTextMeasurementProvider::new(), width, 1200.);
            let Some(code) = core.document().projection().blocks().iter().find(|block| block.style.0 == "Code Block") else { continue; };
            let code_end = code.range.end;
            let original_owner = code.clone();
            let before_text = core.document().text().to_owned();
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let index = snapshot.rows.iter().position(|row| row.text_range.end == code_end).unwrap();
            let up = matches!(command, "dk" | "d-up" | "Vkd" | "native-up-delete");
            let first = &snapshot.rows[if up { index.saturating_sub(1) } else { index }];
            let retained = if first.text_range.start > original_owner.range.start {
                let prefix = &before_text[original_owner.range.start..first.text_range.start];
                Some(if first.fragment_index == 0 { prefix.strip_suffix('\n').unwrap_or(prefix) } else { prefix }.to_owned())
            } else { None };
            let at = snapshot.rows[index].text_range.start;
            core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
                text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
            let events = match command {
                "d-up" => vec![Key::Char('d'), Key::Up],
                "native-delete" => vec![Key::Char('V')],
                "native-up-delete" => vec![Key::Char('V'), Key::Up],
                _ => command.chars().map(Key::Char).collect(),
            };
            for key in events {
                if let Err(error) = core.handle(view, CoreEvent::Input(InputEvent::Key(key))) {
                    failures.push(format!("form={form} body={body:?} width={width} {command}: {error:?}"));
                    continue 'case;
                }
            }
            if command.starts_with("native") {
                core.set_selection_origin(view, SelectionOrigin::Mouse, Mode::Normal).unwrap();
                if let Err(error) = core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Delete))) {
                    failures.push(format!("form={form} body={body:?} width={width} {command}: {error:?}"));
                    continue 'case;
                }
            }
            let after = core.document().source_bytes();
            let following = core.document().projection().blocks().last().unwrap();
            if following.style.0 != "Paragraph" || &core.document().text()[following.range.clone()] != "Following paragraph" {
                failures.push(format!("form={form} body={body:?} width={width} {command}: {:?}; owner={original_owner:?}", String::from_utf8_lossy(&after)));
            }
            if let Some(expected) = retained {
                let actual = core.document().projection().blocks().iter().find(|block| block.range.start == original_owner.range.start);
                if !actual.is_some_and(|block| block.style == original_owner.style && block.code_language == original_owner.code_language
                    && &core.document().text()[block.range.clone()] == expected) {
                    failures.push(format!("retained code changed: {form} {body:?} width={width} {command}: expected {expected:?}, got {actual:?}; source={:?}", String::from_utf8_lossy(&after)));
                }
            }
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::key('u'))).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r')))).unwrap();
            assert_eq!(core.document().source_bytes(), after);
        }
    }
    }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
