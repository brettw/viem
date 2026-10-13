use viem_core::document::*;
fn open(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}
fn apply(
    document: &mut Document,
    range: std::ops::Range<usize>,
    unindent: bool,
) -> Result<CommittedModelTransaction, ModelTransactionError> {
    document.apply_model_request(ModelRequest::IndentList {
        document: document.id(),
        revision: document.revision(),
        range,
        unindent,
    })
}
fn levels(document: &Document) -> Vec<(bool, u8)> {
    document
        .projection()
        .blocks()
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::ListItem { ordered, level, .. } => Some((ordered, level)),
            _ => None,
        })
        .collect()
}
fn ordinals(document: &Document) -> Vec<u64> {
    document
        .projection()
        .blocks()
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::ListItem {
                ordinal,
                item_start: true,
                ..
            } => Some(ordinal),
            _ => None,
        })
        .collect()
}
#[test]
fn generated_families_are_four_levels_and_deeper_source_is_preserved() {
    for (source, format) in [
        ("- a\n  1. b\n     - c\n       1. d\n          - e", Format::Markdown),
    ] {
        let document = open(source, format);
        let sheet = document.projection().style_sheet();
        assert_eq!(sheet.block_styles().filter(|style| style.id.is_internal_list()).count(), 8);
        assert!(!sheet.block_styles().any(|style| (1..=4).any(|level| style.id.0 == format!("List{level}"))));
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(levels(&document), [(false,0),(true,1),(false,2),(true,3),(false,4)]);
        let names = document.projection().blocks().iter().map(|block| block.style.0.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["BulletedList1", "NumberedList2", "BulletedList3", "NumberedList4", "BulletedList4"]);
    }
}
#[test]
fn indent_requires_a_previous_sibling_and_unindent_restores_source_exactly() {
    for (source, format) in [
        ("- Alpha\n- **Beta**\n- Gamma", Format::Markdown),
    ] {
        let mut document = open(source, format);
        assert_eq!(document.list_indent_capabilities(0..0), (false, false));
        let at = document.text().find("Beta").unwrap();
        assert_eq!(
            document.list_indent_capabilities(at..at),
            (true, false),
            "{format:?}"
        );
        let original = document.source_bytes();
        let text = document.text().to_owned();
        apply(&mut document, at..at, false).unwrap();
        assert_eq!(document.text(), text);
        assert_eq!(levels(&document), [(false, 0), (false, 1), (false, 0)]);
        let nested = document.source_bytes();
        assert_eq!(document.list_indent_capabilities(at..at), (false, true));
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(document.text(), text);
        assert_eq!(levels(&document), [(false, 0), (false, 0), (false, 0)]);
        let final_source = document.source_bytes();
        assert_eq!(
            open(std::str::from_utf8(&final_source).unwrap(), format).text(),
            text
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), nested);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), nested);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), final_source);
    }
}

#[test]
fn ordered_indent_starts_a_canonical_child_run_and_unindent_removes_its_container() {
    for (source, nested, format) in [
        (
            "1. One\n2. Two\n3. Three\n4. Four",
            "1. One\n   1. Two\n   2. Three\n2. Four",
            Format::Markdown,
        ),
    ] {
        let mut document = open(source, format);
        let start = document.text().find("Two").unwrap();
        let end = document.text().find("Three").unwrap() + "Three".len();
        apply(&mut document, start..end, false).unwrap();
        assert_eq!(document.source_bytes(), nested.as_bytes(), "{format:?}");
        assert_eq!(
            levels(&document),
            [(true, 0), (true, 1), (true, 1), (true, 0)]
        );
        assert_eq!(ordinals(&document), [1, 1, 2, 2]);

        let start = document.text().find("Two").unwrap();
        let end = document.text().find("Three").unwrap() + "Three".len();
        apply(&mut document, start..end, true).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes(), "{format:?}");
        assert_eq!(
            levels(&document),
            [(true, 0), (true, 0), (true, 0), (true, 0)]
        );
        assert_eq!(ordinals(&document), [1, 2, 3, 4]);
    }
}

#[test]
fn markdown_mixed_family_indent_moves_subtree_and_stops_at_fourth_level() {
    let mut document = open(
        "- Parent\n1. Child\n   - Grandchild\n     - Deep\n- Tail",
        Format::Markdown,
    );
    let at = document.text().find("Child").unwrap();
    assert_eq!(document.list_indent_capabilities(at..at).0, true);
    apply(&mut document, at..at, false).unwrap();
    assert_eq!(
        levels(&document),
        [(false, 0), (true, 1), (false, 2), (false, 3), (false, 0)]
    );
    assert!(!document.list_indent_capabilities(at..at).0);
    apply(&mut document, at..at, true).unwrap();
    assert_eq!(
        levels(&document),
        [(false, 0), (true, 0), (false, 1), (false, 2), (false, 0)]
    );
}
#[test]
fn unavailable_actions_are_atomic() {
    let mut document = open("- A\n- B", Format::Markdown);
    let source = document.source_bytes();
    let revision = document.revision();
    for unindent in [false, true] {
        assert!(apply(&mut document, 0..0, unindent).is_err());
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
    }
}

#[test]
fn complete_item_continuations_and_code_move_with_the_selection() {
    for (source, format) in [
        (
            "- A\n- B\n\n  continuation\n\n  ```\n  literal\n  ```\n- C",
            Format::Markdown,
        ),
    ] {
        let mut document = open(source, format);
        let text = document.text().to_owned();
        let at = document.text().find('B').unwrap();
        assert!(document.list_indent_capabilities(at..at).0, "{format:?}");
        apply(&mut document, at..at, false).unwrap();
        assert_eq!(document.text(), text);
        assert!(document.list_indent_capabilities(at..at).1);
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(document.text(), text);
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn source_modes_use_the_same_structural_policy_and_exact_history() {
    for (source, format) in [
        ("- A\n- B", Format::MarkdownSource),
    ] {
        let mut document = open(source, format);
        let at = document.text().find('B').unwrap();
        assert_eq!(
            document.list_indent_capabilities(at..at),
            (true, false),
            "{format:?}"
        );
        apply(&mut document, at..at, false).unwrap();
        let saved = document.source_bytes();
        let at = document.text().find('B').unwrap();
        assert_eq!(
            document.list_indent_capabilities(at..at),
            (false, true),
            "{format:?}"
        );
        apply(&mut document, at..at, true).unwrap();
        assert!(document.undo());
        assert_eq!(document.source_bytes(), saved);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn quoted_markdown_list_indentation_preserves_every_quote_marker() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = "> - parent\n> - **child**\n> - tail";
        let mut document = open(source, format);
        let at = document.text().find("child").unwrap();
        assert!(
            document.list_indent_capabilities(at..at).0,
            "{format:?}: {:?}",
            document.prepare_list_indent(at..at, false)
        );
        apply(&mut document, at..at, false).unwrap();
        assert_eq!(
            document.source_bytes(),
            b"> - parent\n>   - **child**\n> - tail"
        );
        let at = document.text().find("child").unwrap();
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn deeper_authored_levels_use_fourth_style_plus_structural_inset() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = "- a\n  - b\n    - c\n      - d\n        - e";
    let document = open(source, Format::Markdown);
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    assert_eq!(
        styles
            .paragraphs
            .iter()
            .map(|p| p.leading_indent + p.containers.iter().map(|c| c.style.left()).sum::<f32>())
            .collect::<Vec<_>>(),
        [32.0, 64.0, 96.0, 128.0, 160.0]
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn multi_item_selection_moves_every_selected_subtree_one_level() {
    for (source, format) in [
        ("- A\n- B\n  - child\n- C\n- D", Format::Markdown),
    ] {
        let mut document = open(source, format);
        let start = document.text().find('B').unwrap();
        let end = document.text().find('C').unwrap() + 1;
        let original = document.source_bytes();
        let text = document.text().to_owned();
        assert!(document.list_indent_capabilities(start..end).0);
        apply(&mut document, start..end, false).unwrap();
        assert_eq!(
            levels(&document),
            [(false, 0), (false, 1), (false, 2), (false, 1), (false, 0)]
        );
        assert_eq!(document.text(), text);
        apply(&mut document, start..end, true).unwrap();
        assert_eq!(
            levels(&document),
            [(false, 0), (false, 0), (false, 1), (false, 0), (false, 0)]
        );
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), original);
    }
}
#[test]
fn tabbed_markers_use_columns_and_preserve_the_original_marker_bytes() {
    for source in ["1.\tone\n2.\ttwo", "-\tone\n-\ttwo", "> 1.\tone\n> 2.\ttwo"] {
        let mut document = open(source, Format::Markdown);
        let at = document.text().find("two").unwrap();
        assert!(document.list_indent_capabilities(at..at).0, "{source:?}");
        apply(&mut document, at..at, false).unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(levels(&document)[1].1, 1);
        assert!(String::from_utf8_lossy(&document.source_bytes()).contains("\ttwo"));
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn toolbar_capabilities_do_not_prepare_edits_or_scan_large_lists() {
    for count in [32, 12_000] {
        let markdown = format!("- Parent\n{}  - Target\n- Following", "  - Child\n".repeat(count));
        for (source, format) in [
            (&markdown, Format::Markdown), (&markdown, Format::MarkdownSource),
        ] {
            let document = open(source, format);
            let at = document.text().find("Target").unwrap();
            let (capabilities, work) = measure_document_work(|| document.list_indent_capabilities(at..at));
            assert_eq!(capabilities, (true, true), "{format:?}: {work:?}");
            assert_eq!(work.source_full_materializations, 0, "{format:?}: {work:?}");
            assert_eq!(work.source_decoded_bytes, 0, "{format:?}: {work:?}");
            assert_eq!(work.full_projection_candidates, 0, "{format:?}: {work:?}");
            assert_eq!(work.formatted_full_materialized_bytes, 0, "{format:?}: {work:?}");
            assert_eq!(work.scratch_documents, 0, "{format:?}: {work:?}");
            assert!(work.list_capability_nodes_visited < 200, "{format:?}: {work:?}");
            assert!(work.list_capability_blocks_visited < 400, "{format:?}: {work:?}");
        }
    }
}

#[test]
fn capability_index_follows_local_edits_structure_changes_and_history() {
    for (source, format) in [
        ("- A\n- Target\n\nOutside", Format::Markdown),
        ("- A\n- Target\n\nOutside", Format::MarkdownSource),
    ] {
        let mut document = open(source, format);
        let at = document.text().find("Target").unwrap();
        assert_eq!(document.list_indent_capabilities(at..at), (true, false));
        document.replace(at..at, "New ").unwrap();
        let at = document.text().find("Target").unwrap();
        assert_eq!(document.list_indent_capabilities(at..at), (true, false));
        apply(&mut document, at..at, false).unwrap();
        let at = document.text().find("Target").unwrap();
        assert_eq!(document.list_indent_capabilities(at..at), (false, true));
        assert!(document.undo());
        let at = document.text().find("Target").unwrap();
        assert_eq!(document.list_indent_capabilities(at..at), (true, false));
        assert!(document.redo());
        let at = document.text().find("Target").unwrap();
        assert_eq!(document.list_indent_capabilities(at..at), (false, true));
    }
}

fn assert_indent_projection_reopens(document: &Document) {
    let fresh = Document::from_bytes(document.source_bytes(), document.encoding(), document.format()).unwrap();
    let shape = |doc: &Document| doc.projection().blocks().iter().map(|block| (
        block.range.clone(), block.kind.clone(), block.style.clone(), block.list_loose,
        block.containers.iter().map(|member| (member.container.kind, member.starts_here, member.ends_here))
            .collect::<Vec<_>>(),
    )).collect::<Vec<_>>();
    assert_eq!(document.text(), fresh.text());
    assert_eq!(shape(document), shape(&fresh));
    assert_eq!(document.projection().style_spans(), fresh.projection().style_spans());
}

fn list_only_separator(document: &Document) -> usize {
    document.projection().blocks().iter().find(|block|
        block.kind == BlockKind::Paragraph && block.range.is_empty()
            && block.containers.iter().rev().find(|member|
                matches!(member.container.kind, ContainerKind::List { .. } | ContainerKind::ListItem))
                .is_some_and(|member| matches!(member.container.kind, ContainerKind::List { .. }))
    ).unwrap().range.start
}

#[test]
fn insert_tab_crosses_list_only_separator_and_retains_empty_parent() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    let source = "8. first\n\n   <span></span>\n9. \n\n\n\n10. last\n11. tail";
    let expected = "8. first\n\n   <span></span>\n9. <span></span>\n\n\n\n   1. last\n10. tail";
    for quoted in [false, true] {
        let quote = |text: &str| if quoted {
            text.split('\n').map(|row| format!("> {row}")).collect::<Vec<_>>().join("\n")
        } else { text.to_owned() };
        let source = quote(source);
        let expected = quote(expected);
        let mut document = open(&source, Format::Markdown);
        let gap = list_only_separator(&document);
        assert_eq!(document.list_indent_capabilities(gap..gap), (false, false));
        assert!(!document.can_insert_list_paragraph(gap));
        assert!(apply(&mut document, gap..gap, false).is_err());
        assert_eq!(document.source_bytes(), source.as_bytes());
        let at = document.text().find("last").unwrap();
        let text = document.text().to_owned();
        assert_eq!(document.list_indent_capabilities(at..at), (true, false));
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i')))).unwrap();
        core.handle(view, CoreEvent::PlaceCursor { document_revision: core.document().revision(),
            text_offset: at, affinity: BoundaryAffinity::Downstream, extend_selection: false }).unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Tab)))
            .unwrap_or_else(|error| panic!("quoted={quoted}: {error:?}"));
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape))).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes(), "quoted={quoted}");
        assert_eq!(core.document().text(), text);
        assert_eq!(ordinals(core.document()), [8, 9, 1, 10]);
        assert_indent_projection_reopens(core.document());
        let structure = core.document().projection().list_structure();
        let parent = &structure.lists[0].items[1];
        assert_eq!(parent.paragraph_ids.len(), 2);
        assert_eq!(parent.child_lists.len(), 1);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).unwrap();
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(list_only_separator(core.document()), gap);
        core.handle(view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).unwrap();
        assert_eq!(core.document().source_bytes(), expected.as_bytes());
        assert_indent_projection_reopens(core.document());
    }
}

#[test]
fn multi_item_indent_and_unindent_cross_list_only_separator() {
    let source = "8. first\n\n   <span></span>\n9. \n\n\n\n10. last\n11. tail";
    let expected = "8. first\n\n   <span></span>\n\n   1. \n\n\n\n   2. last\n9. tail";
    let mut document = open(source, Format::Markdown);
    let start = document.projection().blocks().iter().find(|block|
        matches!(block.kind, BlockKind::ListItem { ordinal: 9, item_start: true, .. })).unwrap().range.start;
    let end = document.text().find("last").unwrap() + "last".len();
    let text = document.text().to_owned();
    assert_eq!(document.list_indent_capabilities(start..end), (true, false));
    apply(&mut document, start..end, false).unwrap();
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert_eq!(document.text(), text);
    assert_eq!(ordinals(&document), [8, 1, 2, 9]);
    assert_indent_projection_reopens(&document);
    let gap = list_only_separator(&document);
    assert_eq!(document.list_indent_capabilities(gap..gap), (false, false));
    assert!(!document.can_insert_list_paragraph(gap));
    let gap_id = document.projection().blocks().iter().find(|block| block.range == (gap..gap)).unwrap().id;
    assert!(document.projection().list_structure().lists.iter().flat_map(|list| &list.items)
        .all(|item| !item.paragraph_ids.contains(&gap_id)));
    assert_eq!(document.list_indent_capabilities(start..end), (false, true));
    apply(&mut document, start..end, true).unwrap();
    assert_eq!(document.text(), text);
    assert_eq!(ordinals(&document), [8, 9, 10, 11]);
    assert_indent_projection_reopens(&document);
    let restored = document.source_bytes();
    assert!(document.undo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), restored);
}

#[test]
fn list_only_separator_capability_lookup_stays_indexed() {
    for count in [32, 4_000] {
        let source = format!("- parent\n{}- target\n- tail", "- \n\n\n\n".repeat(count));
        let document = open(&source, Format::Markdown);
        let at = document.text().find("target").unwrap();
        let (capabilities, work) = measure_document_work(|| document.list_indent_capabilities(at..at));
        assert_eq!(capabilities, (true, false), "{work:?}");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
        assert!(work.list_capability_nodes_visited < 200, "{work:?}");
        assert!(work.list_capability_blocks_visited < 400, "{work:?}");
    }
}

#[test]
fn selection_starting_in_list_only_separator_agrees_with_capabilities() {
    let source = "8. first\n\n   <span></span>\n9. \n\n\n\n10. last\n11. tail";
    let expected = "8. first\n\n   <span></span>\n9. <span></span>\n\n\n\n   1. last\n10. tail";
    let mut document = open(source, Format::Markdown);
    let gap = list_only_separator(&document);
    let at = document.text().find("last").unwrap();
    assert_eq!(document.list_indent_capabilities(gap..at), (false, false));
    assert!(apply(&mut document, gap..at, false).is_err());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert_eq!(document.list_indent_capabilities(gap..at + 4), (true, false));
    apply(&mut document, gap..at + 4, false).unwrap();
    assert_eq!(document.source_bytes(), expected.as_bytes());
    assert_indent_projection_reopens(&document);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), expected.as_bytes());
}
