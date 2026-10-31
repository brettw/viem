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
fn empty_quote_autoformat_after_surplus_separators_stays_regional() {
    for paragraphs in [3, 2000] {
        for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
            for suffix in [String::new(), format!("\n\n{}", document(400, ""))] {
                let prefix = format!("a{}", "\n".repeat(2 * (paragraphs - 1)));
                let source = format!("{prefix}\n\nb{suffix}");
                let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
                let mut core = Core::new(original);
                let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
                core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
                core.handle(view, CoreEvent::PlaceCursor {
                    document_revision: core.document().revision(), text_offset: paragraphs,
                    affinity, extend_selection: false,
                }).unwrap();
                for text in [">", " "] {
                    let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::text(text))));
                    result.unwrap();
                    let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
                    assert_eq!(shape(core.document()), shape(&fresh), "{affinity:?}, {text:?}");
                    assert_eq!(work.full_projection_candidates, 0, "{affinity:?}, {text:?}: {work:?}");
                    assert_eq!(work.source_full_materializations, 0, "{affinity:?}, {text:?}: {work:?}");
                    assert_eq!(work.formatted_full_materialized_bytes, 0, "{affinity:?}, {text:?}: {work:?}");
                    assert!(work.list_capability_nodes_visited < 512, "{affinity:?}, {text:?}: {work:?}");
                    assert!(work.list_capability_blocks_visited < 512, "{affinity:?}, {text:?}: {work:?}");
                    assert!(work.source_decoded_bytes < 4096, "{affinity:?}, {text:?}: {work:?}");
                    eprintln!("empty quote paragraphs={paragraphs} {affinity:?} suffix={} text={text:?}: {work:?}", suffix.len());
                }
                let saved = format!("{prefix}> \n\nb{suffix}").into_bytes();
                assert_eq!(core.document().source_bytes(), saved);
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
                assert_eq!(core.document().source_bytes(), saved);
                let fresh = Document::from_bytes(saved, Encoding::Utf8, Format::Markdown).unwrap();
                assert_eq!(shape(core.document()), shape(&fresh));
            }
        }
    }
}

#[test]
fn autoformat_refreshes_cached_neighbor_spacing_and_list_decorations() {
    use viem_core::layout::{LayoutSnapshot, VisualRow};
    fn comparable_rows(snapshot: &LayoutSnapshot, _identity: &LayoutSnapshot) -> Vec<VisualRow> {
        let mut rows = snapshot.rows.as_ref().clone();
        for row in &mut rows {
            row.paragraph_id = None;
            for cluster in &mut row.clusters { cluster.render_run = None; }
            for decoration in &mut row.decorations {
                decoration.owner = None;
                decoration.render_run = None;
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
        assert_eq!(work.compatibility_record_materializations, 0, "{label}: {work:?}");
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

#[test]
fn guarded_structural_edits_stay_regional_and_restore_exact_source() {
    for (label, middle, format, target_offset, keys) in [
        ("list split", "- first item\n- EDITME second item words\n- third item\n\n", Format::Markdown, 3, vec![InputEvent::key('i'), InputEvent::Key(Key::Enter)]),
        ("quote split", "> first quoted paragraph\n>\n> EDITME quoted words\n\n", Format::Markdown, 3, vec![InputEvent::key('i'), InputEvent::Key(Key::Enter)]),
        ("HTML split", "<div>\n<p>EDITME html words</p>\n</div>\n\n", Format::Markdown, 3, vec![InputEvent::key('i'), InputEvent::Key(Key::Enter)]),
        ("after list", "- first item\n- second item\n\nEDITME paragraph after list\n\n", Format::Markdown, 3, vec![InputEvent::key('i'), InputEvent::Key(Key::Enter)]),
        ("heading start", "## EDITME heading words\n\nFollowing paragraph\n\n", Format::Markdown, 0, vec![InputEvent::key('i'), InputEvent::Key(Key::Enter)]),
    ] {
        for blocks in [20, 400] {
            let source = document(blocks, middle);
            let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let at = original.projection().text_tree().slice(0..original.projection().text_tree().byte_len())
                .unwrap().find("EDITME").unwrap() + target_offset;
            let original_prefix = &source[..source.find(middle).unwrap()];
            let original_suffix = &source[source.find(middle).unwrap() + middle.len()..];
            let mut core = Core::new(original);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
            core.handle(view, CoreEvent::PlaceCursor {
                document_revision: core.document().revision(), text_offset: at,
                affinity: BoundaryAffinity::Downstream, extend_selection: false,
            }).unwrap();
            let (result, work) = measure_document_work(|| {
                for key in &keys { core.handle(view, CoreEvent::Input(key.clone()))?; }
                Ok::<_, viem_core::CoreError>(())
            });
            result.unwrap();
            assert_eq!(work.full_projection_candidates, 0, "{label} {blocks}: {work:?}");
            assert_eq!(work.source_full_materialized_bytes, 0, "{label} {blocks}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{label} {blocks}: {work:?}");
            assert!(work.source_decoded_bytes < 8192, "{label} {blocks}: {work:?}");
            assert_eq!(work.formatted_compatibility_materialized_bytes, 0, "{label} {blocks}: {work:?}");
            let saved = core.document().source_bytes();
            assert!(saved.starts_with(original_prefix.as_bytes()), "{label}: prefix changed");
            assert!(saved.ends_with(original_suffix.as_bytes()), "{label}: suffix changed");
            let fresh = Document::from_bytes(saved.clone(), Encoding::Utf8, format).unwrap();
            assert_eq!(core.document().text(), fresh.text(), "{label}");
            let actual = core.document().projection().blocks();
            let expected = fresh.projection().blocks();
            assert_eq!(actual.len(), expected.len(), "{label}");
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.range, expected.range, "{label}");
                assert_eq!(actual.attributes, expected.attributes, "{label}");
            }
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{label}: undo");
            core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
            assert_eq!(core.document().source_bytes(), saved, "{label}: redo");
        }
    }
}

#[test]
fn structural_reprojection_keeps_separator_pairing_and_empty_owner_carets() {
    for (source, format, at, keys) in [
        ("```\n\n\n```", Format::Markdown, 1, vec![InputEvent::Key(Key::Backspace), InputEvent::Text("x".into())]),
        ("```\n\n\n```", Format::Markdown, 0, vec![InputEvent::Key(Key::Delete), InputEvent::Text("x".into())]),
        ("```\n\n\n```", Format::Markdown, 1, vec![InputEvent::Key(Key::Backspace), InputEvent::Key(Key::Backspace), InputEvent::Text("x".into())]),
    ] {
        let original = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(original);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 2000.);
        core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor {
            document_revision: core.document().revision(), text_offset: at,
            affinity: BoundaryAffinity::Downstream, extend_selection: false,
        }).unwrap();
        for key in keys {
            core.handle(view, CoreEvent::Input(key.clone())).unwrap();
            let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, format).unwrap();
            assert_eq!(shape(core.document()), shape(&fresh), "{source:?} {key:?}");
            let provenance = |document: &Document| document.projection().provenance().into_iter()
                // A valid retained empty caret at an existing text boundary
                // need not appear in a fresh nonempty projection. When the
                // owner is empty, its caret contributors must agree exactly.
                .filter(|span| document.text().is_empty() || !span.formatted.is_empty())
                .collect::<Vec<_>>();
            assert_eq!(provenance(core.document()), provenance(&fresh), "{source:?} {key:?}");
        }
        let saved = core.document().source_bytes();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), saved);
    }
}

#[test]
fn list_editing_reads_only_the_surrounding_lists() {
    let middle = "1. First item\n2. Second item\n3. Third item\n\n";
    let source = document(400, middle);
    let original = Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = original.text().find("Second item").unwrap() + "Second item".len();
    let mut core = Core::new(original);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::Input(InputEvent::key('i'))).unwrap();
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    // Continuing the list renumbers the following item. Exiting the new empty
    // item creates an unindented paragraph and restarts the following list.
    // Both inspect only the surrounding list runs.
    for (label, key, expected) in [
        ("Enter", Key::Enter, "1. First item\n2. Second item\n3. \n4. Third item\n\n"),
        ("Backspace", Key::Backspace, "1. First item\n2. Second item\n\n<span></span>\n\n1. Third item\n\n"),
    ] {
        let (result, work) = measure_document_work(|| core.handle(view, CoreEvent::Input(InputEvent::Key(key))));
        result.unwrap();
        assert_eq!(work.compatibility_record_materializations, 0, "{label}: {work:?}");
        let text = String::from_utf8(core.document().source_bytes()).unwrap();
        let start = text.find("1. First item").unwrap();
        assert!(text[start..].starts_with(expected), "{label}: {:?}", &text[start..start + expected.len() + 8]);
        reopens_as_displayed(&core, label);
    }
}

#[test]
fn deleting_lines_beside_inert_brackets_stays_regional_despite_a_matching_definition() {
    // Brackets in code spans and task markers never resolve through a
    // definition, so their bounded reparse needs no document-wide context.
    let middle = "Type `[x]` or `a[i]` to start,\nthen a closing line.\n\n- [ ] open item\n- [x] done item\n- [ ] last item\n\n";
    let source = format!("{}[x]: /url\n", document(200, middle));
    for target in ["Type `[x]`", "then a closing line.", "- [x] done item", "- [ ] last item"] {
        let line = source[..source.find(target).unwrap()].matches('\n').count() + 1;
        let mut core = Core::new(Document::from_bytes(source.clone().into_bytes(), Encoding::Utf8, Format::Markdown).unwrap());
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        let (_, work) = measure_document_work(|| {
            for key in format!(":{line}d").chars() { core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap(); }
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap()
        });
        assert!(!String::from_utf8_lossy(&core.document().source_bytes()).contains(target), "{target}");
        assert_eq!(work.full_projection_candidates, 0, "{target}: {work:?}");
        assert!(work.source_decoded_bytes < 4096, "{target}: {work:?}");
        reopens_as_displayed(&core, target);
        let deleted = core.document().source_bytes();
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes(), "{target}");
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), deleted, "{target}");
    }
}

#[test]
fn typing_a_bracket_beside_inert_brackets_stays_literal_despite_the_definition() {
    // The bounded parse treats `[y]` in a code span as inert, but a typed
    // bracket could resolve through the definition outside it. It needs that
    // context to stay the literal text the user typed.
    let middle = "Before `[y]` and after.\n\n";
    let source = format!("{}[x]: /url\n", document(20, middle));
    let original = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    let at = original.text().find(" and after").unwrap();
    let mut core = Core::new(original);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    for key in "i [x]".chars() { core.handle(view, CoreEvent::Input(InputEvent::key(key))).unwrap(); }
    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
    assert!(core.document().text().contains("Before [y] [x] and after."));
    assert!(String::from_utf8_lossy(&core.document().source_bytes()).contains("Before `[y]` \\[x\\] and after."));
    reopens_as_displayed(&core, "typed bracket");
    let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(core.document().projection().style_spans(), fresh.projection().style_spans());
}
