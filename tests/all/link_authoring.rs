use viem_core::document::*;
fn document(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn apply(document: &mut Document, intent: LinkEditIntent) {
    let (prepared, _) = document
        .prepare_link_edit(document.id(), document.revision(), intent)
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
}
#[test]
fn insert_selected_label_edit_destination_and_unlink_are_exactly_undoable() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "before selected after";
        let mut doc = document(source, format);
        apply(
            &mut doc,
            LinkEditIntent::Insert {
                range: 7..15,
                text: "selected".into(),
                destination: "https://example.test/a b?x=1&y=2".into(),
            },
        );
        let inserted = doc.source_bytes();
        let link = doc
            .link_snapshot_at(doc.text_point(8).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(link.text, "selected");
        assert_eq!(link.destination, "https://example.test/a b?x=1&y=2");
        assert_eq!(link.range.start, 7);
        assert_eq!(
            link.range.end,
            if format.is_source_view() {
                doc.text().len() - 6
            } else {
                15
            }
        );
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: "renamed".into(),
                destination: "#heading".into(),
            },
        );
        let changed = doc.source_bytes();
        let link = doc
            .link_snapshot_at(doc.text_point(8).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(link.text, "renamed");
        apply(&mut doc, LinkEditIntent::Remove { range: link.range });
        assert_eq!(doc.text(), "before renamed after");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), changed);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), inserted);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), inserted);
    }
}
#[test]
fn source_popup_recognizes_opening_label_destination_and_closing_syntax() {
    let mut doc = document("[label](<docs/local file.md>)", Format::MarkdownSource);
    for at in 0..doc.text().len() {
        let link = doc
            .link_snapshot_at(doc.text_point(at).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(link.range, 0..doc.text().len());
    }
    let stale = doc.text_point(1).unwrap();
    doc.insert(0, "x ").unwrap();
    assert!(matches!(
        doc.link_snapshot_at(stale),
        Err(DocumentError::WrongSnapshot { .. })
    ));
}
#[test]
fn destination_only_edit_and_unlink_preserve_nested_label_formatting() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document("before [a **bold** word](old.md) after", format);
        let link = doc
            .link_snapshot_at(doc.text_point(8).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(link.text, "a bold word");
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: link.text,
                destination: "new.md".into(),
            },
        );
        assert!(String::from_utf8(doc.source_bytes())
            .unwrap()
            .contains("[a **bold** word]"));
        let link = doc
            .link_snapshot_at(doc.text_point(8).unwrap())
            .unwrap()
            .unwrap();
        apply(&mut doc, LinkEditIntent::Remove { range: link.range });
        assert_eq!(doc.source_bytes(), b"before a **bold** word after");
    }
}
#[test]
fn failed_link_edits_leave_bytes_revision_and_history_untouched() {
    let mut doc = document("before selection after", Format::Markdown);
    let bytes = doc.source_bytes();
    let revision = doc.revision();
    let history = doc.history_status();
    assert!(doc
        .prepare_link_edit(
            doc.id(),
            revision,
            LinkEditIntent::Insert {
                range: 7..16,
                text: "bad\nlabel".into(),
                destination: "url".into()
            }
        )
        .is_err());
    assert_eq!(doc.source_bytes(), bytes);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_status(), history);
    let (prepared, _) = doc
        .prepare_link_edit(
            doc.id(),
            revision,
            LinkEditIntent::Insert {
                range: 7..16,
                text: "selection".into(),
                destination: "url".into(),
            },
        )
        .unwrap();
    doc.insert(0, "new ").unwrap();
    assert!(doc.commit_model_transaction(prepared).is_err());
    assert_eq!(doc.source_bytes(), b"new before selection after");
}
#[test]
fn links_preserve_encoding_and_reject_unrepresentable_labels_without_changes() {
    for encoding in [Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be] {
        let source = "before café after";
        let bytes: Vec<u8> = match encoding {
            Encoding::Latin1 => source.chars().map(|ch| ch as u8).collect(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => unreachable!(),
        };
        let mut doc = Document::from_bytes(bytes.clone(), encoding, Format::Markdown).unwrap();
        apply(
            &mut doc,
            LinkEditIntent::Insert {
                range: 7..12,
                text: "café".into(),
                destination: "local.md".into(),
            },
        );
        assert_eq!(
            doc.link_at(doc.text_point(8).unwrap()).unwrap().as_deref(),
            Some("local.md")
        );
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), bytes);
        if encoding == Encoding::Latin1 {
            assert!(doc
                .prepare_link_edit(
                    doc.id(),
                    doc.revision(),
                    LinkEditIntent::Insert {
                        range: 7..12,
                        text: "🙂".into(),
                        destination: "url".into()
                    }
                )
                .is_err());
            assert_eq!(doc.source_bytes(), bytes);
        }
    }
}
#[test]
fn fragment_navigation_matches_headings_and_duplicate_suffixes() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(
            "# Hello, **World**!\n\ntext\n\n## Hello, **World**!\n\n# Café",
            format,
        );
        let first = doc.find_link_fragment("hello-world").unwrap().unwrap();
        let second = doc.find_link_fragment("hello-world-1").unwrap().unwrap();
        assert_eq!(first, 0);
        assert!(second > first);
        assert!(doc.find_link_fragment("café").unwrap().is_some());
        assert!(doc.find_link_fragment("missing").unwrap().is_none());
    }
}

#[test]
fn heading_picker_uses_navigation_fragments_and_omits_code_lookalikes() {
    let source = "# Hello, **World**!\n\n## Hello, **World**!\n\nSetext `title`\n===\n\n> ### Quoted heading\n\n- Parent\n  - #### ~~Nested~~ heading\n\n```\n# Not a heading\n```";
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(source, format);
        let list = doc.link_headings().unwrap();
        assert!(!list.truncated);
        assert_eq!(list.headings.iter().map(|h| (&*h.text, &*h.destination, h.level)).collect::<Vec<_>>(), vec![
            ("Hello, World!", "#hello-world", 1),
            ("Hello, World!", "#hello-world-1", 2),
            ("Setext title", "#setext-title", 1),
            ("Quoted heading", "#quoted-heading", 3),
            ("Nested heading", "#nested-heading", 4),
        ]);
        for heading in list.headings {
            assert_eq!(doc.find_link_fragment(&heading.destination[1..]).unwrap(), Some(heading.offset));
        }
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
    assert!(document(source, Format::PlainText).link_headings().unwrap().headings.is_empty());
}

#[test]
fn heading_picker_has_finite_retention_and_reports_truncation() {
    let doc = document(&"# Heading\n\n".repeat(1025), Format::Markdown);
    let list = doc.link_headings().unwrap();
    assert_eq!(list.headings.len(), 1024);
    assert!(list.truncated);
    assert_eq!(list.headings[1023].destination, "#heading-1023");
    let collisions = document("# foo\n\n# foo-1\n\n# foo\n\n# foo", Format::Markdown).link_headings().unwrap();
    assert_eq!(collisions.headings.iter().map(|heading| heading.destination.as_str()).collect::<Vec<_>>(),
        vec!["#foo", "#foo-1", "#foo-2", "#foo-3"]);
}

#[test]
fn heading_picker_reads_only_indexed_headings_and_bounds_giant_labels() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let doc = document(&format!("{}# Target\n", "unrelated prose\n\n".repeat(10_000)), format);
        let (result, work) = measure_document_work(|| doc.link_headings().unwrap());
        assert_eq!(result.headings.len(), 1);
        assert_eq!(result.headings[0].destination, "#target");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert!(work.source_decoded_bytes < 64, "{work:?}");
        assert!(work.list_capability_blocks_visited < 256, "{work:?}");
        let giant = document(&format!("# {}\n", "x".repeat(300_000)), format);
        let (result, work) = measure_document_work(|| giant.link_headings().unwrap());
        assert!(result.headings.is_empty() && result.truncated);
        assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
    }
}
#[test]
fn passive_link_refresh_never_decodes_unrelated_large_document_source() {
    let source = format!(
        "{}\n\n[label](#target)",
        "unrelated paragraph\n\n".repeat(2000)
    );
    let doc = document(&source, Format::Markdown);
    let at = doc.text().find("label").unwrap();
    let (result, work) =
        measure_document_work(|| doc.link_snapshot_at(doc.text_point(at).unwrap()).unwrap());
    assert!(result.is_some());
    assert!(work.source_decoded_bytes < 4096, "{work:?}");
}

#[test]
fn editing_existing_link_retains_authored_title_and_unchanged_destination_spelling() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document("[label](old.md 'authored title')", format);
        let link = doc
            .link_snapshot_at(doc.text_point(1).unwrap())
            .unwrap()
            .unwrap();
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: "new label".into(),
                destination: link.destination,
            },
        );
        assert_eq!(doc.source_bytes(), b"[new label](old.md 'authored title')");
        let link = doc
            .link_snapshot_at(doc.text_point(1).unwrap())
            .unwrap()
            .unwrap();
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: link.text,
                destination: "new.md".into(),
            },
        );
        assert_eq!(
            doc.source_bytes(),
            b"[new label](<new.md> 'authored title')"
        );
    }
}

#[test]
fn inserting_over_selected_rich_label_keeps_selected_and_neighbor_formatting() {
    let mut doc = document("before a **bold** word after", Format::Markdown);
    apply(
        &mut doc,
        LinkEditIntent::Insert {
            range: 7..18,
            text: "a bold word".into(),
            destination: "local.md".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"before [a **bold** word](<local.md>) after"
    );
    let reopened = Document::from_bytes(doc.source_bytes(), doc.encoding(), doc.format()).unwrap();
    assert_eq!(reopened.text(), "before a bold word after");
}

#[test]
fn automatic_links_can_be_edited_and_removed_without_reactivation() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in ["<https://example.test>", "https://example.test"] {
            let mut doc = document(source, format);
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            assert!(link.editable);
            apply(
                &mut doc,
                LinkEditIntent::Edit {
                    range: link.range,
                    text: "Example".into(),
                    destination: "https://new.test".into(),
                },
            );
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(link.text, "Example");
            assert!(doc.undo());
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            apply(&mut doc, LinkEditIntent::Remove { range: link.range });
            for (at, _) in doc.text().char_indices() {
                assert_eq!(
                    doc.link_at(doc.text_point(at).unwrap()).unwrap(),
                    None,
                    "{format:?} {source}"
                );
            }
        }
    }
}

#[test]
fn native_noop_and_stale_selection_preserve_controller_and_history() {
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(document("[label](target.md) tail", Format::Markdown));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 200.0);
    let selection = core.list_selection_identity(view).unwrap();
    let history = core.document().history_status();
    let revision = core.document().revision();
    let result = core
        .edit_link(
            view,
            selection.clone(),
            LinkEditIntent::Edit {
                range: 0..5,
                text: "label".into(),
                destination: "target.md".into(),
            },
        )
        .unwrap();
    assert!(!result.document_changed);
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.list_selection_identity(view).unwrap(), selection);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: revision,
            text_offset: 1,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert!(core
        .edit_link(view, selection, LinkEditIntent::Remove { range: 0..5 })
        .is_err());
    assert_eq!(core.document().source_bytes(), b"[label](target.md) tail");
    assert_eq!(core.document().history_status(), history);
}

#[test]
fn label_whitespace_and_literal_punctuation_are_preserved_in_both_views() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document("before after", format);
        let label = " *quoted* [label] ";
        apply(
            &mut doc,
            LinkEditIntent::Insert {
                range: 7..7,
                text: label.into(),
                destination: "doc name.md".into(),
            },
        );
        let link = doc
            .link_snapshot_at(doc.text_point(8).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(link.text, label);
        let reopened =
            Document::from_bytes(doc.source_bytes(), doc.encoding(), Format::Markdown).unwrap();
        assert_eq!(reopened.text(), "before  *quoted* [label] after");
    }
}

#[test]
fn local_link_edit_keeps_large_document_projection_work_regional() {
    let source = format!(
        "{}\n\n[label](#target) tail",
        "unrelated paragraph\n\n".repeat(2000)
    );
    let mut doc = document(&source, Format::Markdown);
    let at = doc.text().find("label").unwrap();
    let link = doc
        .link_snapshot_at(doc.text_point(at).unwrap())
        .unwrap()
        .unwrap();
    let (_, work) = measure_document_work(|| {
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: link.text,
                destination: "#new-target".into(),
            },
        )
    });
    assert_eq!(work.source_full_materializations, 0, "{work:?}");
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 8192, "{work:?}");
}

#[test]
fn source_empty_labels_empty_destinations_and_long_destinations_remain_editable() {
    for source in [
        "[label]()".to_owned(),
        "[](#target)".to_owned(),
        format!("[label](https://example.test/{})", "path/".repeat(800)),
    ] {
        let mut doc = document(&source, Format::MarkdownSource);
        let link = doc
            .link_snapshot_at(doc.text_point(0).unwrap())
            .unwrap()
            .unwrap();
        assert!(link.editable);
        assert_eq!(link.range, 0..source.len());
        apply(
            &mut doc,
            LinkEditIntent::Edit {
                range: link.range,
                text: "new label".into(),
                destination: "#new".into(),
            },
        );
        assert_eq!(
            doc.link_at(doc.text_point(2).unwrap()).unwrap().as_deref(),
            Some("#new")
        );
    }
}

#[test]
fn removing_explicit_link_does_not_reactivate_an_automatic_url_in_the_label() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut doc = document("[https://example.test](old.md)", format);
        let link = doc
            .link_snapshot_at(doc.text_point(1).unwrap())
            .unwrap()
            .unwrap();
        apply(&mut doc, LinkEditIntent::Remove { range: link.range });
        for (at, _) in doc.text().char_indices() {
            assert_eq!(doc.link_at(doc.text_point(at).unwrap()).unwrap(), None);
        }
    }
}

#[test]
fn fragment_navigation_exact_source_fixture_points_at_the_requested_heading() {
    let doc = document(
        "[Jump](#second-heading)\n\n# First heading\n\n# Second heading\n\nbody",
        Format::MarkdownSource,
    );
    let offset = doc.find_link_fragment("second-heading").unwrap().unwrap();
    assert!(doc.text()[offset..].starts_with("# Second heading"));
}

#[test]
fn native_link_publication_resumes_native_and_select_typing_in_a_separate_undo_unit() {
    use viem_core::command::{InputEvent, Key, Mode};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        for select_mode in [false, true] {
            let mut core = Core::new(document("label tail", format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 200.0);
            if select_mode {
                core.handle(view, CoreEvent::Input(InputEvent::text("gh$")))
                    .unwrap();
                assert!(core.command_state(view).unwrap().is_select_mode());
            } else {
                core.handle(
                    view,
                    CoreEvent::SelectAll {
                        document: core.document().id(),
                        revision: core.document().revision(),
                    },
                )
                .unwrap();
                assert!(core.command_state(view).unwrap().is_native_selection());
            }
            let expected = core.list_selection_identity(view).unwrap();
            core.edit_link(
                view,
                expected.clone(),
                LinkEditIntent::Insert {
                    range: expected.range(),
                    text: "linked label".into(),
                    destination: "target.md".into(),
                },
            )
            .unwrap();
            let linked = core.document().source_bytes();
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
            assert!(!core.command_state(view).unwrap().is_text_selection());
            assert_eq!(core.list_selection_identity(view).unwrap().range().len(), 0);
            core.handle(view, CoreEvent::Input(InputEvent::text("!")))
                .unwrap();
            let typed = core.document().source_bytes();
            assert_ne!(typed, linked);
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::text("u")))
                .unwrap();
            assert_eq!(
                core.document().source_bytes(),
                linked,
                "typing must not undo the native link edit"
            );
            core.handle(view, CoreEvent::Input(InputEvent::text("u")))
                .unwrap();
            assert_eq!(core.document().source_bytes(), b"label tail");
        }
    }
}

#[test]
fn native_link_publication_keeps_visual_normal_and_restarts_insert_replace_sessions() {
    use viem_core::command::{InputEvent, Key, Mode};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for entry in ["v$", "i", "R"] {
        let mut core = Core::new(document("label tail", Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 200.0);
        core.handle(view, CoreEvent::Input(InputEvent::text(entry)))
            .unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        core.edit_link(
            view,
            expected.clone(),
            LinkEditIntent::Insert {
                range: expected.range(),
                text: "linked".into(),
                destination: "target.md".into(),
            },
        )
        .unwrap();
        let linked = core.document().source_bytes();
        let wanted = match entry {
            "v$" => Mode::Normal,
            "i" => Mode::Insert,
            _ => Mode::Replace,
        };
        assert_eq!(core.command_state(view).unwrap().mode(), wanted);
        assert!(!core.command_state(view).unwrap().is_text_selection());
        if wanted == Mode::Normal {
            core.handle(view, CoreEvent::Input(InputEvent::text("a")))
                .unwrap();
        }
        core.handle(view, CoreEvent::Input(InputEvent::text("!")))
            .unwrap();
        let typed = core.document().source_bytes();
        assert_ne!(typed, linked);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("u")))
            .unwrap();
        assert_eq!(core.document().source_bytes(), linked);
        core.handle(view, CoreEvent::Input(InputEvent::text("u")))
            .unwrap();
        assert_eq!(core.document().source_bytes(), b"label tail");
    }
}

#[test]
fn changed_label_inherits_only_the_first_selected_character_formatting() {
    use viem_core::layout::DocumentLayoutStyles;
    for format in [Format::Markdown, Format::MarkdownSource] {
        for (source, bold, italic) in [
            ("[**bold**](target.md)", true, false),
            ("[**bold** regular](target.md)", true, false),
            ("[*italic* regular](target.md)", false, true),
            ("[regular **bold**](target.md)", false, false),
        ] {
            let mut doc = document(source, format);
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            apply(
                &mut doc,
                LinkEditIntent::Edit {
                    range: link.range,
                    text: "renamed".into(),
                    destination: "new.md".into(),
                },
            );
            let reopened =
                Document::from_bytes(doc.source_bytes(), doc.encoding(), Format::Markdown).unwrap();
            assert_eq!(reopened.text(), "renamed");
            let style =
                DocumentLayoutStyles::semantic_character_at(reopened.projection(), 0, false)
                    .unwrap();
            assert_eq!(style.bold, bold, "{source} {format:?}");
            assert_eq!(
                style.slant != FontSlant::Upright,
                italic,
                "{source} {format:?}"
            );
            assert_eq!(
                reopened
                    .link_at(reopened.text_point(0).unwrap())
                    .unwrap()
                    .as_deref(),
                Some("new.md")
            );
        }
    }
}

#[test]
fn unlink_preserves_url_code_spans_without_escaping_their_literal_body() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in [
            "[`https://example.com`](target.md)",
            "[code `https://example.com` tail](target.md)",
        ] {
            let mut doc = document(source, format);
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            apply(&mut doc, LinkEditIntent::Remove { range: link.range });
            assert!(!String::from_utf8(doc.source_bytes())
                .unwrap()
                .contains('\\'));
            let reopened =
                Document::from_bytes(doc.source_bytes(), doc.encoding(), Format::Markdown).unwrap();
            assert!(reopened.text().contains("https://example.com"));
            for (at, _) in reopened.text().char_indices() {
                assert_eq!(
                    reopened.link_at(reopened.text_point(at).unwrap()).unwrap(),
                    None
                );
            }
        }
    }
}

#[test]
fn pending_unlink_splits_source_retains_inline_styles_and_restores_exact_history() {
    use viem_core::command::{InputEvent, Key, Mode};
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for (format, source, needle) in [
        (
            Format::Markdown,
            "before [ab**cd**ef](<target.md> 'title') after",
            "d",
        ),
        (
            Format::MarkdownSource,
            "before [ab**cd**ef](<target.md> 'title') after",
            "d",
        ),
        (Format::Markdown, "before [ab`cd`ef](target.md) after", "d"),
        (
            Format::MarkdownSource,
            "before [ab`cd`ef](target.md) after",
            "d",
        ),
    ] {
        let mut core = Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let at = core.document().text().find(needle).unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: viem_core::document::BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        core.exit_link_typing(view, expected).unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap_or_else(|error| panic!("{format:?} {source}: {error:?}"));
        let changed = core.document().source_bytes();
        let reopened = document(std::str::from_utf8(&changed).unwrap(), Format::Markdown);
        let x = reopened.text().find('X').unwrap();
        assert!(
            reopened
                .link_at(reopened.text_point(x).unwrap())
                .unwrap()
                .is_none(),
            "{format:?}: {:?}",
            String::from_utf8_lossy(&changed)
        );
        if source.contains("**") {
            assert!(
                DocumentLayoutStyles::character_at(reopened.projection(), x, false)
                    .unwrap()
                    .bold
            );
        }
        if source.contains('`') {
            assert_eq!(
                core.selected_named_styles(view).unwrap().character,
                Some("Code".into())
            );
        }
        assert_eq!(reopened.text(), "before abcXdef after");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("u")))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), changed);
    }
}

#[test]
fn selected_unlink_keeps_unselected_links_and_extended_graphemes() {
    let source = "before [a **👩🏽‍💻é** z](<target.md> 'title') after";
    let mut doc = document(source, Format::Markdown);
    let start = doc.text().find('👩').unwrap();
    let end = doc.text().find(" z").unwrap();
    apply(
        &mut doc,
        LinkEditIntent::RemoveSelection { range: start..end },
    );
    assert_eq!(doc.text(), "before a 👩🏽‍💻é z after");
    assert!(doc
        .link_at(doc.text_point(start).unwrap())
        .unwrap()
        .is_none());
    assert_eq!(
        doc.link_at(doc.text_point(start - 1).unwrap())
            .unwrap()
            .as_deref(),
        Some("target.md")
    );
    assert_eq!(
        doc.link_at(doc.text_point(end).unwrap())
            .unwrap()
            .as_deref(),
        Some("target.md")
    );
    assert!(
        viem_core::layout::DocumentLayoutStyles::character_at(doc.projection(), start, false)
            .unwrap()
            .bold
    );
}

#[test]
fn native_link_insert_from_normal_starts_typing_inside_the_new_link() {
    use viem_core::command::{InputEvent, Mode};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core = Core::new(document("tail", format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let expected = core.list_selection_identity(view).unwrap();
        core.edit_link(
            view,
            expected.clone(),
            LinkEditIntent::Insert {
                range: expected.range(),
                text: "label".into(),
                destination: "target.md".into(),
            },
        )
        .unwrap();
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(reopened.text(), "labelXtail");
        let x = reopened.text().find('X').unwrap();
        assert_eq!(
            reopened
                .link_at(reopened.text_point(x).unwrap())
                .unwrap()
                .as_deref(),
            Some("target.md")
        );
    }
}

#[test]
fn pending_unlink_ime_commit_and_subsequent_typing_keep_the_override() {
    use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "before [abcdef](target.md) after";
        let mut core = Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let at = core.document().text().find("cd").unwrap();
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
        core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
            .unwrap();
        let target = CompositionTarget::at_offsets(core.document(), at..at).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3))),
        )
        .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        let composed = core.document().source_bytes();
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(reopened.text(), "before ab猫Xcdef after");
        let cat = reopened.text().find('猫').unwrap();
        assert!(reopened
            .link_at(reopened.text_point(cat).unwrap())
            .unwrap()
            .is_none());
        assert!(reopened
            .link_at(reopened.text_point(cat + 3).unwrap())
            .unwrap()
            .is_none());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), composed);
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn pending_unlink_replace_restores_original_source_and_repeat_keeps_unlinked_text() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "[abcdef](target.md)";
        let mut core = Core::new(document(source, format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        core.handle(view, CoreEvent::Input(InputEvent::key('R')))
            .unwrap();
        let at = core.document().text().find("cd").unwrap();
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
        core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("XY")))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(reopened.text(), "abXYef", "{format:?}");
        assert!(reopened
            .link_at(reopened.text_point(2).unwrap())
            .unwrap()
            .is_none());
        assert!(reopened
            .link_at(reopened.text_point(3).unwrap())
            .unwrap()
            .is_none());
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Backspace)))
            .unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        let d = core.document().text().find("de").unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: d,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('.')))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(reopened.text(), "abXXef", "{format:?}");
        assert!(reopened
            .link_at(reopened.text_point(3).unwrap())
            .unwrap()
            .is_none());
    }
}

#[test]
fn pending_unlink_preserves_multiline_payload_and_quote_owner() {
    use viem_core::command::InputEvent;
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core = Core::new(document("> [abcdef](target.md)", format));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let at = core.document().text().find("cd").unwrap();
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
        core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("X\nY")))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(
            reopened.text(),
            if format == Format::Markdown {
                "abX\nYcdef"
            } else {
                "abX Ycdef"
            },
            "{format:?}"
        );
        let x = reopened.text().find('X').unwrap();
        let y = reopened.text().find('Y').unwrap();
        assert!(reopened
            .link_at(reopened.text_point(x).unwrap())
            .unwrap()
            .is_none());
        assert!(reopened
            .link_at(reopened.text_point(y).unwrap())
            .unwrap()
            .is_none());
        if format == Format::Markdown {
            assert!(reopened
                .projection()
                .blocks()
                .iter()
                .all(|block| block.quote_depth > 0));
        }
    }
}

#[test]
fn pending_unlink_stale_and_encoding_failures_leave_source_and_requested_typing_unchanged() {
    use viem_core::command::{InputEvent, Mode};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "[abcdef](target.md)";
    let mut core = Core::new(
        Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Latin1,
            Format::Markdown,
        )
        .unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
    let stale = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: 2,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    assert!(core.exit_link_typing(view, stale).is_err());
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
    core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
        .unwrap();
    let history = core.document().history_status();
    let cursor = core.command_state(view).unwrap().cursor();
    // A literal Code context cannot use prose numeric-reference fallbacks.
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(
        view,
        CoreEvent::AssignNamedStyle {
            expected,
            style_sheet_revision: core.document().projection().style_sheet().revision,
            namespace: viem_core::document::StyleNamespace::Character,
            style: "Code".into(),
        },
    )
    .unwrap();
    assert!(core
        .handle(view, CoreEvent::Input(InputEvent::text("猫")))
        .is_err());
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert_eq!(core.document().history_status(), history);
    assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Insert);
    assert_eq!(
        core.selected_named_styles(view).unwrap().character,
        Some("Code".into())
    );
    core.handle(view, CoreEvent::Input(InputEvent::text("X")))
        .unwrap();
    let reopened = Document::from_bytes(
        core.document().source_bytes(),
        Encoding::Latin1,
        Format::Markdown,
    )
    .unwrap();
    let x = reopened.text().find('X').unwrap();
    assert!(reopened
        .link_at(reopened.text_point(x).unwrap())
        .unwrap()
        .is_none());
    assert!(reopened
        .is_code_at(x, BoundaryAffinity::Downstream)
        .unwrap());
}

#[test]
fn pending_unlink_can_insert_a_new_link_without_changing_the_remaining_destinations() {
    use viem_core::command::InputEvent;
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core = Core::new(document(
            "before [ab**cd**ef](<original.md> 'title') after",
            format,
        ));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let at = core.document().text().find('d').unwrap();
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
        core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
            .unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        core.edit_link(
            view,
            expected.clone(),
            LinkEditIntent::Insert {
                range: expected.range(),
                text: "NEW".into(),
                destination: "new.md".into(),
            },
        )
        .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("X")))
            .unwrap();
        let reopened = document(
            std::str::from_utf8(&core.document().source_bytes()).unwrap(),
            Format::Markdown,
        );
        assert_eq!(reopened.text(), "before abcNEWXdef after");
        let start = reopened.text().find("NEW").unwrap();
        assert_eq!(
            reopened
                .link_at(reopened.text_point(start).unwrap())
                .unwrap()
                .as_deref(),
            Some("new.md")
        );
        assert_eq!(
            reopened
                .link_at(reopened.text_point(start + 3).unwrap())
                .unwrap()
                .as_deref(),
            Some("new.md")
        );
        assert_eq!(
            reopened
                .link_at(reopened.text_point(start - 1).unwrap())
                .unwrap()
                .as_deref(),
            Some("original.md")
        );
        assert_eq!(
            reopened
                .link_at(reopened.text_point(start + 4).unwrap())
                .unwrap()
                .as_deref(),
            Some("original.md")
        );
        assert!(
            DocumentLayoutStyles::character_at(reopened.projection(), start, false)
                .unwrap()
                .bold
        );
    }
}

#[test]
fn unsupported_link_typing_context_keeps_display_and_caret_state_without_source_changes() {
    use viem_core::command::Mode;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::Core;
    for source in ["<a href='target.md'>label</a>"] {
        let mut core = Core::new(document(source, Format::Markdown));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        assert_eq!(
            core.selected_named_styles(view).unwrap().character,
            Some("Link".into())
        );
        let selection = core.list_selection_identity(view).unwrap();
        let history = core.document().history_status();
        assert!(core.exit_link_typing(view, selection.clone()).is_err());
        assert_eq!(core.list_selection_identity(view).unwrap(), selection);
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.document().history_status(), history);
    }
}

#[test]
fn pending_unlink_handles_editable_automatic_links_and_protects_plain_url_payloads() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in [
            "before https://example.test/path after",
            "before <https://example.test/path> after",
            "before <writer@example.test> after",
            "before [abcdef](old.md) after",
        ] {
            let mut core = Core::new(document(source, format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
            let at = core
                .document()
                .text()
                .find("example")
                .or_else(|| core.document().text().find('d'))
                .unwrap();
            let prior = document(source, Format::Markdown).text().to_owned();
            let old_source = core.document().source_bytes();
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
            core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
                .unwrap();
            core.handle(
                view,
                CoreEvent::Input(InputEvent::text(" https://new.test/path ")),
            )
            .unwrap();
            let after = core.document().source_bytes();
            let reopened =
                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            let start = reopened.text().find("https://new.test/path").unwrap();
            assert!(
                reopened
                    .link_at(reopened.text_point(start).unwrap())
                    .unwrap()
                    .is_none(),
                "{format:?}: {source}"
            );
            assert!(reopened.text().len() > prior.len());
            core.handle(
                view,
                CoreEvent::Input(InputEvent::text("https://next.test/path ")),
            )
            .unwrap();
            let reopened = Document::from_bytes(
                core.document().source_bytes(),
                Encoding::Utf8,
                Format::Markdown,
            )
            .unwrap();
            let next = reopened.text().find("https://next.test/path").unwrap();
            assert!(reopened
                .link_at(reopened.text_point(next).unwrap())
                .unwrap()
                .is_none());
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                .unwrap();
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
                .unwrap();
            assert_eq!(core.document().source_bytes(), old_source);
        }
    }
}

#[test]
fn pending_unlink_preserves_utf16_bom_non_ascii_title_and_untouched_encoded_tails() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "α before [é**猫犬**z](<docs/é.md> '題') after Ω";
    let original = [
        vec![0xff, 0xfe],
        source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    ]
    .concat();
    let prefix: Vec<u8> = "α before "
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let tail: Vec<u8> = " after Ω"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for format in [Format::Markdown, Format::MarkdownSource] {
        let mut core =
            Core::new(Document::from_bytes(original.clone(), Encoding::Utf16Le, format).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 200.);
        let at = core.document().text().find('犬').unwrap();
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
        core.exit_link_typing(view, core.list_selection_identity(view).unwrap())
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::text("鳥")))
            .unwrap();
        let after = core.document().source_bytes();
        assert!(after.starts_with(&[vec![0xff, 0xfe], prefix.clone()].concat()));
        assert!(after.ends_with(&tail));
        let reopened =
            Document::from_bytes(after.clone(), Encoding::Utf16Le, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), "α before é猫鳥犬z after Ω");
        let at = reopened.text().find('鳥').unwrap();
        assert!(reopened
            .link_at(reopened.text_point(at).unwrap())
            .unwrap()
            .is_none());
        let left = reopened.text().find('猫').unwrap();
        assert_eq!(
            reopened
                .link_at(reopened.text_point(left).unwrap())
                .unwrap()
                .as_deref(),
            Some("docs/é.md")
        );
        assert!(String::from_utf16(
            &after[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>()
        )
        .unwrap()
        .contains("'題'"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('u'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), original);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
            .unwrap();
        assert_eq!(core.document().source_bytes(), after);
    }
}

#[test]
fn selected_automatic_link_removal_preserves_linked_label_halves_and_exact_undo() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        for source in [
            "before https://example.test/path after",
            "before <https://example.test/path> after",
            "before <writer@example.test> after",
        ] {
            let mut doc = document(source, format);
            let start = doc.text().find("example").unwrap();
            let expected = document(source, Format::Markdown).text().to_owned();
            apply(
                &mut doc,
                LinkEditIntent::RemoveSelection {
                    range: start..start + "example".len(),
                },
            );
            let after = doc.source_bytes();
            let reopened =
                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            let start = reopened.text().find("example").unwrap();
            assert!(reopened
                .link_at(reopened.text_point(start).unwrap())
                .unwrap()
                .is_none());
            assert!(reopened
                .link_at(reopened.text_point(start - 1).unwrap())
                .unwrap()
                .is_some());
            assert!(reopened
                .link_at(reopened.text_point(start + "example".len()).unwrap())
                .unwrap()
                .is_some());
            assert!(doc.undo());
            assert_eq!(doc.source_bytes(), source.as_bytes());
            assert!(doc.redo());
            assert_eq!(doc.source_bytes(), after);
        }
    }
}

#[test]
fn source_label_fields_share_inline_context_and_gfm_strikethrough_with_rich_view() {
    for (source, expected) in [
        ("[# Heading](url)", "# Heading"),
        ("[~~label~~](url)", "label"),
    ] {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let doc = document(source, format);
            let link = doc
                .link_snapshot_at(doc.text_point(1).unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(link.text, expected, "{source} {format:?}");
        }
    }
}
