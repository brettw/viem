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
