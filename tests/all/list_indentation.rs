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
        ("<ul><li>a<ol><li>b<ul><li>c<ol><li>d<ul><li>e</li></ul></li></ol></li></ul></li></ol></li></ul>", Format::Html),
    ] {
        let document = open(source, format);
        let sheet = document.projection().style_sheet();
        assert_eq!(sheet.block_styles().filter(|style| style.id.is_internal_list()).count(), 8);
        assert!(!sheet.block_styles().any(|style| style.id.0.starts_with("List")));
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
        (
            "<!--keep--><ul><li>Alpha</li><li><b>Beta</b></li><li>Gamma</li></ul><!--end-->",
            Format::Html,
        ),
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
            "1. One\n   1. Two\n   2. Three\n4. Four",
            Format::Markdown,
        ),
        (
            "<ol><li>One</li><li>Two</li><li>Three</li><li>Four</li></ol>",
            "<ol><li>One<ol type=\"a\"><li>Two</li><li>Three</li></ol></li><li>Four</li></ol>",
            Format::Html,
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
fn html_cross_container_indent_restarts_numbering_and_preserves_other_attributes() {
    let source = "<ol><li>One</li></ol><ol data-keep='yes' start='5' class=x><li>Two</li></ol>";
    let mut document = open(source, Format::Html);
    let at = document.text().find("Two").unwrap();
    apply(&mut document, at..at, false).unwrap();
    assert_eq!(
        document.source_bytes(),
        b"<ol><li>One<ol data-keep='yes' class=x type=\"a\"><li>Two</li></ol></li></ol>"
    );
    assert_eq!(levels(&document), [(true, 0), (true, 1)]);
    assert_eq!(ordinals(&document), [1, 1]);
}

#[test]
fn html_unindenting_first_child_reparents_following_siblings_without_an_empty_list() {
    let source = "<ol><li>Parent<ol><li>First</li><li>Following</li></ol></li></ol>";
    let mut document = open(source, Format::Html);
    let at = document.text().find("First").unwrap();
    apply(&mut document, at..at, true).unwrap();
    assert_eq!(
        document.source_bytes(),
        b"<ol><li>Parent</li><li>First<ol><li>Following</li></ol></li></ol>"
    );
    assert_eq!(document.text(), "Parent\nFirst\nFollowing");
    assert_eq!(levels(&document), [(true, 0), (true, 0), (true, 1)]);
    assert_eq!(ordinals(&document), [1, 2, 1]);
    assert!(!String::from_utf8_lossy(&document.source_bytes()).contains("<ol></ol>"));
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
    let mut document = open("<ul><li>A</li><li>B</li></ul>", Format::Html);
    let source = document.source_bytes();
    let revision = document.revision();
    for unindent in [false, true] {
        assert!(apply(&mut document, 0..0, unindent).is_err());
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
    }
}

#[test]
fn indent_handles_mixed_html_containers_and_keeps_each_family() {
    for source in [
        "<ul><li>A</li></ul><!--keep--><ol start='5'><li>B</li><li>C</li></ol>",
        "<ul><li>A</li></ul><ol><li>B</li></ol>",
    ] {
        let mut document = open(source, Format::Html);
        let at = document.text().find('B').unwrap();
        assert!(
            document.list_indent_capabilities(at..at).0,
            "{source} {:?}: {:?}",
            document.text(),
            document.prepare_list_indent(at..at, false)
        );
        apply(&mut document, at..at, false).unwrap();
        assert_eq!(levels(&document)[1], (true, 1));
        assert!(
            document.list_indent_capabilities(at..at).1,
            "{}",
            String::from_utf8_lossy(&document.source_bytes())
        );
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(levels(&document)[1], (true, 0));
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn complete_item_continuations_and_code_move_with_the_selection() {
    for (source, format) in [
        (
            "- A\n- B\n\n  continuation\n\n  ```\n  literal\n  ```\n- C",
            Format::Markdown,
        ),
        (
            "<ul><li>A</li><li>B<p>continuation</p><pre>literal</pre></li><li>C</li></ul>",
            Format::Html,
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
        ("<ul><li>A</li><li>B</li></ul>", Format::HtmlSource),
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
fn legacy_source_list_definitions_remain_active_and_lossless() {
    let source="<style id=\"viem-styles\" data-viem-version=\"2\">li {\n  --viem-inherit: \"character-font-families character-size\";\n  font-family: 'SF Pro';\n  font-size: 14pt;\n  margin-inline-start: 16pt;\n}\n</style><ul><li>Bullet</li></ul><ol><li>Number</li></ol>";
    for family in ["'SF Pro'", "'Segoe UI'", "system-ui"] {
        let source = source.replace("'SF Pro'", family);
        let document = open(&source, Format::Html);
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(
            document.projection().blocks().iter().map(|b| b.style.0.as_str()).collect::<Vec<_>>(),
            ["List1", "List1"]
        );
        let sheet = document.projection().style_sheet();
        assert_eq!(sheet.block_style(&"List1".into()).unwrap().block.leading_indent, Some(48.0));
        assert_eq!(sheet.block_styles().filter(|style| style.id.is_internal_list()).count(), 8);
        assert_eq!(viem_core::layout::DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false)
            .unwrap().font_families, [DEFAULT_FONT_FAMILY]);

        // A foreign default must not relax validation of other declarations.
        let opaque = source.replace("font-size: 14pt;", "font-size: 14pt;\n  future-property: keep;");
        let document = open(&opaque, Format::Html);
        assert_eq!(document.source_bytes(), opaque.as_bytes());
        assert_eq!(document.projection().blocks()[0].style.0, "BulletedList1");
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
            .map(|p| p.leading_indent)
            .collect::<Vec<_>>(),
        [32.0, 64.0, 96.0, 128.0, 160.0]
    );
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn multi_item_selection_moves_every_selected_subtree_one_level() {
    for (source, format) in [
        ("- A\n- B\n  - child\n- C\n- D", Format::Markdown),
        (
            "<ul><li>A</li><li>B<ul><li>child</li></ul></li><li>C</li><li>D</li></ul>",
            Format::Html,
        ),
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
        apply(&mut document, at..at, false).unwrap();
        assert_eq!(levels(&document)[1].1, 1);
        assert!(String::from_utf8_lossy(&document.source_bytes()).contains("\ttwo"));
        apply(&mut document, at..at, true).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn list_commands_create_empty_html_items_then_typing_and_history_work() {
    use viem_core::command::InputEvent;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};
    for (list, tag) in [(ListStyle::Bullet, "ul"), (ListStyle::Numbered, "ol")] {
        for source in ["", "<p></p>", "<!--keep-->", "<body></body>"] {
            let mut core = Core::new(open(source, Format::Html));
            let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 200.0);
            core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                .unwrap();
            let expected = core.list_selection_identity(view).unwrap();
            core.handle(
                view,
                CoreEvent::SetListStyle {
                    expected,
                    style: Some(list),
                },
            )
            .unwrap_or_else(|error| panic!("{source:?} {list:?}: {error:?}"));
            let listed = core.document().source_bytes();
            assert!(
                String::from_utf8_lossy(&listed).contains(&format!("<{tag}><li></li></{tag}>")),
                "{source:?}: {:?}",
                listed
            );
            assert_eq!(core.document().text(), "");
            core.handle(view, CoreEvent::Input(InputEvent::text("a")))
                .unwrap();
            let typed = core.document().source_bytes();
            assert_eq!(core.document().text(), "a");
            assert!(String::from_utf8_lossy(&typed).contains("<li>a</li>"));
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), listed);
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), source.as_bytes());
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
            core.handle(
                view,
                CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo),
            )
            .unwrap();
            assert_eq!(core.document().source_bytes(), typed);
        }
    }
}

fn modern_rtf(levels: usize, ordered: bool, body: &str) -> String {
    let table = (0..levels)
        .map(|level| {
            let marker = if ordered {
                format!("{{\\leveltext\\'02\\'{level:02x}.;}}{{\\levelnumbers\\'01;}}")
            } else {
                "{\\leveltext\\'01\\u8226?;}{\\levelnumbers;}".to_owned()
            };
            format!(
                "{{\\listlevel\\levelnfc{}\\levelstartat1{marker}\\li{}\\fi-200}}",
                if ordered { 0 } else { 23 },
                640 * (level + 1)
            )
        })
        .collect::<String>();
    format!("{{\\rtf1{{\\*\\listtable{{\\list{table}\\listid42}}}}{{\\*\\listoverridetable{{\\listoverride\\listid42\\listoverridecount0\\ls1}}}}\\pard\\ls1\\ilvl0 {body}{{\\*\\unknown keep}}}}")
}
#[test]
fn modern_rtf_lists_indent_locally_and_restore_table_selectors() {
    for ordered in [false, true] {
        for body in ["A\\par {\\b B}\\par C", "A\\par B\\line hard\\par C"] {
            let source = modern_rtf(4, ordered, body);
            let mut document = open(&source, Format::Rtf);
            let at = document.text().find('B').unwrap();
            let text = document.text().to_owned();
            assert!(
                document.list_indent_capabilities(at..at).0,
                "{ordered}: {:?}",
                document.prepare_list_indent(at..at, false)
            );
            let committed = apply(&mut document, at..at, false).unwrap();
            assert!(committed
                .summary()
                .source_patches()
                .iter()
                .all(|patch| patch.range().is_empty()));
            assert_eq!(document.text(), text);
            assert_eq!(
                levels(&document),
                [(ordered, 0), (ordered, 1), (ordered, 0)]
            );
            let nested = document.source_bytes();
            assert!(document.list_indent_capabilities(at..at).1);
            apply(&mut document, at..at, true).unwrap();
            assert_eq!(
                levels(&document),
                [(ordered, 0), (ordered, 0), (ordered, 0)]
            );
            assert_eq!(
                open(
                    std::str::from_utf8(&document.source_bytes()).unwrap(),
                    Format::Rtf
                )
                .text(),
                text
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), nested);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), nested);
        }
    }
}
#[test]
fn rtf_without_a_compatible_nested_definition_is_disabled_atomically() {
    for source in [
        modern_rtf(1, false, "A\\par B"),
        r"{\rtf1{\*\pn\pnlvlblt}A\par B}".to_owned(),
    ] {
        let mut document = open(&source, Format::Rtf);
        let at = document.text().find('B').unwrap();
        let revision = document.revision();
        assert_eq!(document.list_indent_capabilities(at..at), (false, false));
        assert!(apply(&mut document, at..at, false).is_err());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision(), revision);
    }
}
#[test]
fn empty_final_modern_rtf_item_can_indent_and_unindent() {
    let source = modern_rtf(4, false, "A\\par\\ilvl0 ");
    let mut document = open(&source, Format::Rtf);
    assert_eq!(document.text(), "A\n");
    let at = document.text().len();
    assert!(
        document.list_indent_capabilities(at..at).0,
        "{:?}",
        document.prepare_list_indent(at..at, false)
    );
    apply(&mut document, at..at, false).unwrap();
    assert_eq!(levels(&document), [(false, 0), (false, 1)]);
    apply(&mut document, at..at, true).unwrap();
    assert_eq!(levels(&document), [(false, 0), (false, 0)]);
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
