use viem_core::document::{Document, Encoding, Format, ReferenceRequest};
fn open(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
}
fn apply(doc: &mut Document, request: ReferenceRequest) {
    let (prepared, _) = doc
        .prepare_reference_edit(doc.id(), doc.revision(), &request)
        .unwrap();
    doc.commit_model_transaction(prepared).unwrap();
}
#[test]
fn all_reference_spellings_and_shared_image_counts() {
    let doc =
        open("[one][ref] [ref][] [ref] ![image][ref]\n\n[ref]: #heading \"Title\"\n\n# Heading");
    for spelling in ["one", "ref ref", "ref \u{fffc}"] {
        let at = doc.text().find(spelling).unwrap();
        let context = doc
            .reference_context_at(doc.text_point(at).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!((context.links, context.images), (3, 1));
        assert_eq!(context.destination, "#heading");
        assert_eq!(context.title, "Title");
        assert_eq!(
            doc.link_at(doc.text_point(at).unwrap()).unwrap().as_deref(),
            Some("#heading")
        );
    }
    let image = doc.text().find('\u{fffc}').unwrap();
    assert!(doc
        .reference_context_at(doc.text_point(image).unwrap())
        .unwrap()
        .is_none());
}
#[test]
fn shortcut_text_edit_preserves_target_and_exact_history() {
    for spelling in ["[ref]", "[ref][]", "[old][ref]"] {
        let source = format!("{spelling}\n\n[ref]: <target> 'Title'");
        let mut doc = open(&source);
        apply(
            &mut doc,
            ReferenceRequest::EditUse {
                offset: 0,
                text: "new".into(),
                name: "ref".into(),
            },
        );
        assert_eq!(doc.source_bytes(), b"[new][ref]\n\n[ref]: <target> 'Title'");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), b"[new][ref]\n\n[ref]: <target> 'Title'");
    }
}
#[test]
fn definition_edit_preserves_label_title_spelling_and_updates_images() {
    let mut doc = open("[ref] ![alt][ref]\n\n[ref]: old 'Keep this'");
    apply(
        &mut doc,
        ReferenceRequest::EditDefinition {
            offset: 0,
            destination: "new".into(),
            title: "Keep this".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"[ref] ![alt][ref]\n\n[ref]: <new> 'Keep this'"
    );
    assert_eq!(
        doc.projection()
            .inline_images_for_region(&(0..doc.text().len()))[0]
            .destination,
        "new"
    );
    apply(
        &mut doc,
        ReferenceRequest::EditDefinition {
            offset: 0,
            destination: "new".into(),
            title: "Changed".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"[ref] ![alt][ref]\n\n[ref]: <new> \"Changed\""
    );
}
#[test]
fn rename_preserves_labels_and_updates_shortcuts_and_images() {
    let mut doc = open("[ref] [ref][] [**bold**][ref] ![alt][ref]\n\n[ref]: target");
    apply(
        &mut doc,
        ReferenceRequest::Rename {
            offset: 0,
            name: "guide".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"[ref][guide] [ref][guide] [**bold**][guide] ![alt][guide]\n\n[guide]: target"
    );
}
#[test]
fn inline_conversion_preserves_label_markup_and_shared_definition() {
    let mut doc = open("[**bold**][ref]\n\n[ref]: url \"Title\"");
    apply(&mut doc, ReferenceRequest::ConvertInline { offset: 0 });
    assert_eq!(
        doc.source_bytes(),
        b"[**bold**](<url> \"Title\")\n\n[ref]: url \"Title\""
    );
}
#[test]
fn duplicate_definitions_report_inactive_and_only_edit_selected_definition() {
    let mut doc = open("[REF]\n\n[ref]: first\n[ref]: second");
    let offset = doc.text().find("[ref]: second").unwrap();
    let context = doc
        .reference_context_at(doc.text_point(offset).unwrap())
        .unwrap()
        .unwrap();
    assert!(context.inactive);
    apply(
        &mut doc,
        ReferenceRequest::EditDefinition {
            offset,
            destination: "third".into(),
            title: "".into(),
        },
    );
    assert_eq!(
        doc.link_at(doc.text_point(0).unwrap()).unwrap().as_deref(),
        Some("first")
    );
    assert!(String::from_utf8(doc.source_bytes())
        .unwrap()
        .ends_with("[ref]: <third>"));
}
#[test]
fn reference_navigation_records_jump_and_stale_actions_fail() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let mut core = Core::new(open("[ref]\n\n[ref]: #heading\n\n# Heading"));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    let expected = core.list_selection_identity(view).unwrap();
    core.edit_reference(
        view,
        expected.clone(),
        ReferenceRequest::GoToDefinition { offset: 0 },
    )
    .unwrap();
    assert!(core.command_state(view).unwrap().cursor() > 0);
    assert!(core
        .edit_reference(view, expected, ReferenceRequest::Remove { offset: 0 })
        .is_err());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('o'))))
        .unwrap();
    assert_eq!(core.command_state(view).unwrap().cursor(), 0);
}

#[test]
fn missing_explicit_reference_can_create_definition_but_shortcut_is_plain_text() {
    let mut doc = open("[Read][missing] and [plain]");
    let context = doc
        .reference_context_at(doc.text_point(1).unwrap())
        .unwrap()
        .unwrap();
    assert!(context.definition_offset.is_none());
    let plain = doc.text().find("[plain]").unwrap();
    assert!(doc
        .reference_context_at(doc.text_point(plain).unwrap())
        .unwrap()
        .is_none());
    apply(
        &mut doc,
        ReferenceRequest::CreateDefinition {
            offset: 0,
            destination: "#heading".into(),
            title: "".into(),
        },
    );
    assert_eq!(
        doc.source_bytes(),
        b"[Read][missing] and [plain]\n\n[missing]: <#heading>"
    );
}

#[test]
fn normalized_names_unicode_and_multiline_definitions_resolve() {
    for source in [
        "[Read][SS]\n\n[ẞ]: target",
        "[Read][A   B]\n\n[a b]: target",
        "[Read][ref]\n\n[ref]:\n  target\n  'Title'",
        "[Read][ref]\n\n> [ref]: target 'Title'",
        "[Read][ref]\n\n- [ref]: target 'Title'",
    ] {
        let doc = open(source);
        let context = doc
            .reference_context_at(doc.text_point(0).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(context.destination, "target", "{source}");
        if source.contains("Title") {
            assert_eq!(context.title, "Title", "{source}");
        }
    }
}
#[test]
fn utf16_and_crlf_reference_edits_preserve_untouched_bytes() {
    let source = "[ref]\r\n\r\n[ref]: old 'Title'";
    let bytes: Vec<_> = [0xff, 0xfe]
        .into_iter()
        .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let mut doc = Document::from_bytes(bytes.clone(), Encoding::Utf16Le, Format::Markdown).unwrap();
    apply(
        &mut doc,
        ReferenceRequest::EditDefinition {
            offset: 0,
            destination: "new".into(),
            title: "Title".into(),
        },
    );
    let expected: Vec<_> = [0xff, 0xfe]
        .into_iter()
        .chain(
            source
                .replace("old", "<new>")
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    assert_eq!(doc.source_bytes(), expected);
    assert!(doc.undo());
    assert_eq!(doc.source_bytes(), bytes);
}

#[test]
fn multiline_container_definitions_keep_prefixes_when_destination_changes() {
    for source in [
        "[ref]\n\n> [ref]: old\n>   'Title'",
        "[ref]\n\n- [ref]: old\n    'Title'",
    ] {
        let mut doc = open(source);
        let context = doc
            .reference_context_at(doc.text_point(0).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(context.title, "Title", "{source}");
        apply(
            &mut doc,
            ReferenceRequest::EditDefinition {
                offset: 0,
                destination: "new".into(),
                title: "Title".into(),
            },
        );
        assert_eq!(
            doc.source_bytes(),
            source.replace("old", "<new>").as_bytes()
        );
    }
}

#[test]
fn invalid_or_colliding_rename_is_atomic() {
    for name in ["other", "bad]name", "bad\\"] {
        let source = "[ref]\n\n[ref]: target\n[other]: elsewhere";
        let doc = open(source);
        assert!(
            doc.prepare_reference_edit(
                doc.id(),
                doc.revision(),
                &ReferenceRequest::Rename {
                    offset: 0,
                    name: name.into()
                }
            )
            .is_err(),
            "{name}"
        );
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(!doc.history_status().is_dirty);
    }
}

#[test]
fn definition_jumps_center_within_scroll_limits_without_editing() {
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::{EdgeInsets, MockTextMeasurementProvider};
    use viem_core::{Core, CoreEvent};
    for definition_position in [0, 40, 80] {
        let mut paragraphs: Vec<_> = (0..80).map(|i| format!("Paragraph {i}")).collect();
        paragraphs.insert(definition_position, "[ref]: target".into());
        paragraphs.insert(1, "See [ref]".into());
        let source = paragraphs.join("\n\n");
        let mut core = Core::new(open(&source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 180.);
        let other = core.add_view(MockTextMeasurementProvider::new(), 600., 180.);
        core.set_view_insets(
            view,
            EdgeInsets {
                top: 17.,
                bottom: 29.,
                ..Default::default()
            },
        )
        .unwrap();
        let use_offset = core.document().text().find("See ref").unwrap() + 4;
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: use_offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let history = core.document().history_status();
        let expected = core.list_selection_identity(view).unwrap();
        let outcome = core
            .edit_reference(
                view,
                expected,
                ReferenceRequest::GoToDefinition { offset: use_offset },
            )
            .unwrap();
        assert!(!outcome.document_changed);
        let offset = core.command_state(view).unwrap().cursor();
        assert!(
            core.document()
                .reference_context_at(core.document().text_point(offset).unwrap())
                .unwrap()
                .unwrap()
                .definition
        );
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        let geometry = snapshot
            .logical_endpoint_geometry(offset, BoundaryAffinity::Downstream)
            .unwrap();
        let row = &snapshot.rows[geometry.row_index];
        let ink = row.ink_bounds();
        let start = ink.map_or(row.y, |ink| row.y.min(ink.y));
        let end = ink.map_or(row.y + row.natural_height(), |ink| {
            (row.y + row.natural_height()).max(ink.y + ink.height)
        });
        let requested = (start + end - 17. - (layout.height() - 29.)) / 2.;
        let expected = requested.max(0.).min(
            snapshot
                .maximum_viewport_top(layout.height())
                .unwrap_or(f32::INFINITY),
        );
        assert!(
            (layout.viewport_top() - expected).abs() < 0.05,
            "definition {definition_position}: {} vs {expected}",
            layout.viewport_top()
        );
        assert_eq!(core.layout(other).unwrap().viewport_top(), 0.);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.document().history_status(), history);
    }
}

#[test]
fn reference_preview_renders_labels_and_keeps_source_and_label_styles() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = "[**bold** &amp; `code`][ref] [ref][] [ref]\n\n[ref]: target";
    let doc = open(source);
    assert_eq!(doc.text(), "bold & code ref ref\n[ref]: target");
    assert_eq!(doc.source_bytes(), source.as_bytes());
    assert!(DocumentLayoutStyles::semantic_character_at(doc.projection(), 0, false).unwrap().bold);
    let refs = doc.projection().inline_references_for_region(&(0..doc.text().len()));
    assert_eq!(refs.len(), 3);
}

#[test]
fn insert_reference_uses_existing_or_creates_definition_with_exact_undo() {
    for source in ["before **selected** after", "before **selected** after\n\n[ref]: existing 'Title'"] {
        let mut doc = open(source);
        apply(&mut doc, ReferenceRequest::Insert { offset: 7, end: 15, text: "selected".into(), name: "ref".into(), destination: "new".into(), title: "Title".into() });
        let reference = doc.reference_context_at(doc.text_point(7).unwrap()).unwrap().unwrap();
        assert_eq!(reference.text, "selected");
        assert_eq!(reference.destination, if source.contains("existing") { "existing" } else { "new" });
        assert!(String::from_utf8(doc.source_bytes()).unwrap().contains("[**selected**][ref]"));
        let after = doc.source_bytes();
        assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo()); assert_eq!(doc.source_bytes(), after);
    }
    let mut doc = open("word");
    apply(&mut doc, ReferenceRequest::Insert { offset: 0, end: 0, text: "Guide".into(), name: "guide".into(), destination: "#heading".into(), title: "".into() });
    assert_eq!(doc.text(), "Guideword\n[guide]: <#heading>");
}

#[test]
fn ordinary_reference_label_edits_preserve_resolution_and_undo() {
    for spelling in ["[ref]", "[ref][]", "[ref][ref]"] {
        let source = format!("{spelling}\n\n[ref]: target");
        let mut doc = open(&source);
        doc.replace(1..2, "X").unwrap();
        assert_eq!(doc.text(), "rXf\n[ref]: target");
        assert_eq!(doc.link_at(doc.text_point(1).unwrap()).unwrap().as_deref(), Some("target"));
        assert_eq!(doc.source_bytes(), b"[rXf][ref]\n\n[ref]: target");
        let fresh = open(std::str::from_utf8(&doc.source_bytes()).unwrap());
        assert_eq!(doc.projection().inline_references_for_region(&(0..doc.text().len())), fresh.projection().inline_references_for_region(&(0..fresh.text().len())));
        assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn deleting_rendered_reference_and_definition_preserves_other_content() {
    let source = "before [**label**][ref] after\n\n[ref]: target";
    let mut doc = open(source);
    doc.replace(7..12, "").unwrap();
    assert_eq!(doc.text(), "before  after\n[ref]: target");
    assert_eq!(doc.source_bytes(), b"before  after\n\n[ref]: target");
    assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
    let start = doc.text().find("[ref]:").unwrap();
    let end = doc.text().len();
    doc.replace(start..end, "").unwrap();
    assert!(doc.text().starts_with("before label after"));
    assert_eq!(doc.link_at(doc.text_point(7).unwrap()).unwrap().as_deref(), Some("target"));
}

#[test]
fn replacing_through_a_whole_label_removes_the_use_and_keeps_edge_spaces() {
    for (range, replacement, text, expected) in [
        // Deleting the label alone leaves a protected leading space.
        (9..10, "", "[a]: url\n b", "[a]: url\n\n&#32;b"),
        // Text replacing from before the use belongs to that earlier content;
        // the label is not edited and promoted to a full reference.
        (8..10, "x", "[a]: urlx b", "[a]: urlx b"),
        (1..10, "x", "[x b", "[x b"),
    ] {
        let source = "[a]: url\n\n[a] b";
        let mut doc = open(source);
        doc.replace(range, replacement).unwrap();
        assert_eq!(doc.text(), text);
        assert_eq!(doc.source_bytes(), expected.as_bytes());
        assert_eq!(open(expected).text(), text);
        assert!(doc.undo()); assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}

#[test]
fn removing_reference_retains_formatting_without_reactivating_autolinks() {
    let mut doc = open("[**https://example.com**][ref]\n\n[ref]: target");
    apply(&mut doc, ReferenceRequest::Remove { offset: 0 });
    assert!(doc.text().starts_with("https://example.com"));
    assert!(doc.link_at(doc.text_point(1).unwrap()).unwrap().is_none());
    assert!(viem_core::layout::DocumentLayoutStyles::semantic_character_at(doc.projection(), 1, false).unwrap().bold);
}

#[test]
fn removing_code_from_a_defined_label_keeps_its_brackets_literal() {
    use viem_core::document::SemanticInlineStyle;
    // Without its backticks `[x]` would resolve through the definition and
    // hide its brackets. Escapes keep the text the code span showed.
    for (source, shown, expected) in [
        ("Use `[x]` here.\n\n[x]: /url\n", "[x]", "Use \\[x\\] here.\n\n[x]: /url\n"),
        ("Use `[x] and a_b *c*` here.\n\n[x]: /url\n", "[x] and a_b *c*",
            "Use \\[x\\] and a_b \\*c* here.\n\n[x]: /url\n"),
        ("Use `[z]` here.\n\n[x]: /url\n", "[z]", "Use [z] here.\n\n[x]: /url\n"),
    ] {
        let mut doc = open(source);
        let text = doc.text().to_owned();
        let at = text.find(shown).unwrap();
        doc.set_semantic_style(at..at + shown.len(), SemanticInlineStyle::Code, false).unwrap();
        assert_eq!(doc.text(), text, "{source:?}");
        assert_eq!(String::from_utf8(doc.source_bytes()).unwrap(), expected);
        let fresh = open(expected);
        assert_eq!(fresh.text(), text, "{source:?}");
        assert_eq!(doc.projection().style_spans(), fresh.projection().style_spans(), "{source:?}");
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
        assert!(doc.redo());
        assert_eq!(doc.source_bytes(), expected.as_bytes());
    }
}

#[test]
fn selection_code_removal_beside_a_definition_is_one_literal_undo_unit() {
    use viem_core::command::InputEvent;
    use viem_core::document::{BoundaryAffinity, HistoryNavigationRequest, SemanticInlineStyle};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "Use `[x]` here.\n\n[x]: /url\n";
    let mut core = Core::new(open(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let at = core.document().text().find("[x]").unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::BeginPointerGesture {
        document_revision: core.document().revision(), text_offset: at, affinity: BoundaryAffinity::Downstream,
    }).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at + 3,
        affinity: BoundaryAffinity::Upstream, extend_selection: true,
    }).unwrap();
    let expected = core.list_selection_identity(view).unwrap();
    core.handle(view, CoreEvent::SetSelectionSemanticStyle {
        expected, style: SemanticInlineStyle::Code, enabled: false,
    }).unwrap();
    assert_eq!(core.document().source_bytes(), b"Use \\[x\\] here.\n\n[x]: /url\n");
    assert_eq!(core.document().text(), "Use [x] here.\n[x]: /url");
    core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn inserting_a_defined_label_keeps_its_brackets_literal() {
    // Spelling cleanup cannot see the definition, so it must not remove the
    // bracket escapes that keep the inserted label literal.
    let source = "Before and after.\n\n[x]: /url\n";
    for (inserted, expected) in [
        (" [x]", "Before \\[x\\] and after.\n\n[x]: /url\n"),
        (" *a* [x]", "Before \\*a* \\[x\\] and after.\n\n[x]: /url\n"),
        (" [z]", "Before [z] and after.\n\n[x]: /url\n"),
    ] {
        let mut doc = open(source);
        let at = doc.text().find(" and after").unwrap();
        doc.insert(at, inserted).unwrap();
        assert_eq!(doc.text(), format!("Before{inserted} and after.\n[x]: /url"));
        assert_eq!(String::from_utf8(doc.source_bytes()).unwrap(), expected);
        assert!(doc.undo());
        assert_eq!(doc.source_bytes(), source.as_bytes());
    }
}
