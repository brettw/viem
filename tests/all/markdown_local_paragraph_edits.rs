//! Splitting and joining ordinary paragraphs in formatted Markdown is local:
//! it must not reproject or flatten the whole document, and the result must
//! reopen exactly as displayed.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{measure_document_work, BoundaryAffinity, Document, Encoding, Format, HistoryNavigationRequest};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

fn shape(document: &Document) -> (String, Vec<String>) {
    let mut owners = std::collections::BTreeMap::new();
    let blocks = document.projection().blocks().iter()
        .map(|block| {
            let containers: Vec<_> = block.containers.iter().map(|member| {
                let next = owners.len();
                let owner = *owners.entry(member.container.id).or_insert(next);
                (owner, member.container.kind, member.starts_here, member.ends_here)
            }).collect();
            format!("{:?} {:?} {:?} q{} loose={} {:?}", block.range, block.kind,
                block.style, block.quote_depth, block.list_loose, containers)
        })
        .collect();
    (document.text().to_owned(), blocks)
}

#[test]
fn creating_list_items_refreshes_adjacent_list_owners_and_spacing() {
    use viem_core::command::CommandInterpreter;
    for (source, target) in [
        ("\n\n- b", ""),
        ("plain\n\n- b", "plain"),
        ("- a\n\nplain", "plain"),
        ("- a\n\n\n\nplain", "plain"),
        ("plain\n\n\n\n- b", "plain"),
        ("- a\n\n\n\nplain\n\n- b", "plain"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        let at = document.text().find(target).unwrap();
        commands.set_cursor(&document, at);
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        for key in "- ".chars() {
            commands.handle(&mut document, InputEvent::key(key)).unwrap();
            let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(&document), shape(&fresh), "{source:?}, typed {key:?}");
        }
        let saved = document.source_bytes();
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_eq!(document.source_bytes(), saved);
        let fresh = Document::from_bytes(saved, Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(shape(&document), shape(&fresh));
    }
}

#[test]
fn list_grammar_refresh_excludes_unrelated_document_suffix() {
    use viem_core::command::CommandInterpreter;
    for items in [2, 130] {
        let mut source = "plain\n\n".to_owned();
        for index in 0..items { source.push_str(&format!("- item {index}\n")); }
        source.push_str("\n# Independent\n\n");
        source.push_str(&document(400, ""));
        let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.handle(&mut document, InputEvent::key('i')).unwrap();
        for key in "- ".chars() {
            let (result, work) = measure_document_work(|| commands.handle(&mut document, InputEvent::key(key)));
            result.unwrap();
            assert_eq!(work.full_projection_candidates, 0, "items={items}, key={key:?}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "items={items}, key={key:?}: {work:?}");
            assert!(work.source_decoded_bytes < 20000, "items={items}, key={key:?}: {work:?}");
            let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(&document), shape(&fresh), "items={items}, key={key:?}");
        }
    }
}

#[test]
fn ordinary_typing_inside_large_list_keeps_bounded_projection_work() {
    use viem_core::command::CommandInterpreter;
    let source = (0..2000).map(|index| format!("- item {index}\n")).collect::<String>();
    let mut document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut commands = CommandInterpreter::new();
    commands.set_cursor(&document, document.text().find("item 1000").unwrap() + 4);
    commands.handle(&mut document, InputEvent::key('i')).unwrap();
    let (result, work) = measure_document_work(|| commands.handle(&mut document, InputEvent::text("changed")));
    result.unwrap();
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 1024, "{work:?}");
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(shape(&document), shape(&fresh));
}

#[test]
fn reparsing_or_emptying_items_refreshes_their_entire_list_context() {
    use viem_core::command::CommandInterpreter;
    for (source, target, input) in [
        ("- a\n\n\n\n- b", "b", "i- "),
        ("- a\n\n\n\n- b\n- c", "b", "i## "),
        ("- a\n\n- b", "a", "x"),
        ("- a\n- b", "a", "x"),
        ("- \n\n- b", "", "ia"),
        ("- a\n\n\n\n- b", "b", "i"),
    ] {
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        commands.set_cursor(&document, document.text().find(target).unwrap());
        for key in input.chars() {
            commands.handle(&mut document, InputEvent::key(key)).unwrap();
            let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(&document), shape(&fresh), "{source:?}, {input:?}, key {key:?}");
        }
        if input == "i" {
            commands.handle(&mut document, InputEvent::Key(Key::Backspace)).unwrap();
            let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(&document), shape(&fresh), "removed treatment: {source:?}");
        }
        let saved = document.source_bytes();
        commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
        commands.handle(&mut document, InputEvent::key('u')).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes(), "{source:?}, {input:?}");
        commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap();
        assert_eq!(document.source_bytes(), saved);
        let fresh = Document::from_bytes(saved, Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(shape(&document), shape(&fresh));
    }
}

#[test]
fn filling_empty_markers_reparses_neighbor_ownership_regionally() {
    for prefix in ["-\n  continuation\n- next", "1.\n   continuation\n2. next", "- \n\n  continuation\n- next"] {
        let source = format!("{prefix}\n\n# Independent\n\n{}", document(400, ""));
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let (result, work) = measure_document_work(|| document.insert(0, "x"));
        result.unwrap_or_else(|error| panic!("{prefix:?}: {error:?}"));
        assert_eq!(work.full_projection_candidates, 0, "{prefix:?}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{prefix:?}: {work:?}");
        let saved = document.source_bytes();
        let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(shape(&document), shape(&fresh), "{prefix:?}");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn typing_in_a_long_run_of_ordinary_empty_paragraphs_stays_local() {
    let source = format!("head\n\n{}tail\n\n{}", "\n".repeat(300), "unrelated paragraph\n\n".repeat(1000));
    let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let (result, work) = measure_document_work(|| document.insert(5, "x"));
    result.unwrap();
    assert_eq!(work.full_projection_candidates, 0, "{work:?}");
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    assert!(work.source_decoded_bytes < 128, "{work:?}");
    let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(shape(&document), shape(&fresh));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn autoformat_refreshes_cached_neighbor_spacing_and_list_decorations() {
    use viem_core::layout::{LayoutSnapshot, VisualRow};
    fn comparable_rows(snapshot: &LayoutSnapshot, identity: &LayoutSnapshot) -> Vec<VisualRow> {
        let mut rows = snapshot.rows.as_ref().clone();
        for row in &mut rows {
            row.paragraph_id = None;
            for cluster in &mut row.clusters { cluster.render_run = None; }
            for decoration in &mut row.decorations {
                decoration.owner = None;
                decoration.render_run = None;
            }
            for caret in &mut row.carets {
                caret.point.document_id = identity.document_id;
                caret.point.document_revision = identity.document_revision;
                caret.point.layout_revision = identity.revision;
            }
        }
        rows
    }
    for (source, target) in [
        ("\n\n- following item with enough words to wrap across rows", ""),
        ("plain\n\n- following item with enough words to wrap across rows", "plain"),
        ("- preceding item with enough words to wrap across rows\n\n\n\nplain", "plain"),
    ] {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = document.text().find(target).unwrap();
        let mut cached = Core::new(document);
        let view = cached.add_view(MockTextMeasurementProvider::new(), 240., 1000.);
        // Populate geometry and marker caches before the neighboring paragraph
        // becomes a list item; unchanged text still needs its new list spacing.
        assert!(cached.layout(view).unwrap().snapshot().unwrap().rows.len() >= 2);
        cached.handle(view, CoreEvent::PlaceCursor {
            document_revision: cached.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in "i- ".chars() {
            cached.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
        }
        let document = Document::from_bytes(cached.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut fresh = Core::new(document);
        let fresh_view = fresh.add_view(MockTextMeasurementProvider::new(), 240., 1000.);
        let actual = cached.layout(view).unwrap().snapshot().unwrap();
        let expected = fresh.layout(fresh_view).unwrap().snapshot().unwrap();
        assert_eq!(actual.total_height, expected.total_height, "{source:?}");
        assert_eq!(actual.content_width, expected.content_width, "{source:?}");
        assert_eq!(comparable_rows(actual, expected), comparable_rows(expected, expected), "{source:?}");
    }
}

#[test]
fn expanded_list_context_retains_external_reference_definitions() {
    use viem_core::command::CommandInterpreter;
    use viem_core::document::BlockKind;
    for reference in ["![photo][img]", "![img]", "[photo][img]"] {
        let source = format!("- {reference}\n- next\n- \n\noutside\n\n[img]: local.png");
        for command_entry in [false, true] {
            let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
            let at = document.projection().blocks().iter().find(|block|
                block.range.is_empty() && matches!(block.kind, BlockKind::ListItem { .. }))
                .unwrap().range.start;
            let mut expected = document.text().to_owned();
            expected.insert(at, 'x');
            if command_entry {
                let mut commands = CommandInterpreter::new();
                commands.set_cursor(&document, at);
                for key in "ix".chars() { commands.handle(&mut document, InputEvent::key(key)).unwrap(); }
                commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
            } else {
                document.insert(at, "x").unwrap();
            }
            assert_eq!(document.text(), expected);
            let saved = document.source_bytes();
            assert_eq!(saved, source.replacen("- \n\noutside", "- x\n\noutside", 1).as_bytes());
            let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(shape(&document), shape(&fresh), "{reference:?}, command={command_entry}");
            let all = 0..document.text().len();
            assert_eq!(document.projection().inline_images_for_region(&all), fresh.projection().inline_images_for_region(&all));
            assert_eq!(document.projection().style_spans(), fresh.projection().style_spans());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}

fn reopens_as_displayed(core: &Core<MockTextMeasurementProvider>, context: &str) {
    let saved = core.document().source_bytes();
    let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(shape(core.document()), shape(&fresh), "{context}: saved {:?}", String::from_utf8_lossy(&saved));
}

fn document(blocks: usize, middle: &str) -> String {
    let mut source = String::new();
    for index in 0..blocks {
        source.push_str(&format!("# Heading {index}\n\nParagraph {index} with several plain words.\n\n```\ncode {index}\n```\n\n"));
        if index == blocks / 2 {
            source.push_str(middle);
        }
    }
    source
}

#[test]
fn splitting_and_joining_independent_paragraphs_stays_regional() {
    let middle = "First half of the edited paragraph and its second half.\n\n```\ncode\n```\n\n";
    let source = document(400, middle);
    let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = original.text().find("and its second").unwrap();
    let mut core = Core::new(original);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    for (label, key) in [("Enter", Key::Enter), ("Backspace", Key::Backspace)] {
        let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::Key(key))));
        result.unwrap();
        assert_eq!(work.full_projection_candidates, 0, "{label}: {work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{label}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{label}: {work:?}");
        reopens_as_displayed(&core, label);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    for _ in 0..2 {
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn paragraphs_beside_lists_and_quotes_split_and_join_consistently() {
    for middle in [
        "- item\n- item\n\nAfter the list there is a paragraph here.\n\n",
        "Before the list there is a paragraph here.\n\n- item\n- item\n\n",
        "> quoted\n\nAfter the quote there is a paragraph here.\n\n",
        "    indented code\n\nAfter the code there is a paragraph here.\n\n",
        "Text before the rule there is a paragraph here.\n\n***\n\n",
    ] {
        let source = document(4, middle);
        let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find("there is").unwrap();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in [Key::Enter, Key::Backspace] {
            core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            reopens_as_displayed(&core, &format!("{middle:?} {key:?}"));
        }
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{middle:?}");
    }
}

#[test]
fn enter_after_an_atx_heading_stays_regional_and_setext_stays_consistent() {
    for (middle, regional) in [
        ("## Section heading\n\nBody of the section.\n\n", true),
        ("Section heading\n---------------\n\nBody of the section.\n\n", false),
    ] {
        let source = document(300, middle);
        let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find("Section heading").unwrap() + "Section heading".len();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))));
        result.unwrap();
        if regional {
            assert_eq!(work.full_projection_candidates, 0, "{middle:?}: {work:?}");
        }
        reopens_as_displayed(&core, middle);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{middle:?}");
    }
}

#[test]
fn opening_beside_headings_without_blank_lines_keeps_new_paragraphs_separate() {
    // An ATX heading may interrupt a paragraph or be followed directly by
    // one; a single ending then separates them, and new paragraphs opened
    // there must not become lazy continuations.
    for (source, target, key) in [
        ("#heading-ish\n#\t tab heading", "tab", 'O'),
        ("#heading-ish\n#\t tab heading", "heading-ish", 'o'),
        ("# Heading\nbody text", "body", 'O'),
        ("# Heading\nbody text", "Heading", 'o'),
        ("body text\n# Heading", "body", 'o'),
    ] {
        let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let at = original.text().find(target).unwrap();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in [key, 'x'] {
            core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap();
            reopens_as_displayed(&core, &format!("{source:?} {key}"));
        }
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        reopens_as_displayed(&core, &format!("{source:?} Escape"));
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{source:?}");
    }
}
