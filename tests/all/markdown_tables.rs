use viem_core::document::{
    Document, Encoding, Format, ModelRequest, TableAlignment, TableEditIntent, TextEdit,
};
fn doc(source: &str, format: Format) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
}

#[test]
fn table_insertion_at_paragraph_boundaries_preserves_neighbors_and_history() {
    let mut failures = Vec::new();
    for source in ["", "\n\n", "\n\n\n\n", "## H\n\nP", "H\n---\n\nP",
        "\n\n## H\n\n\n\nP", "one\n\n\n\ntwo", "> H\n>\n>\n>\n> P",
        "- H\n\n  P", "- H\n\n  \n\n  P", "- H\n\n\n\n  P",
        "12. H\n\n    \n\n    P", "> - H\n>\n>   \n>\n>   P",
        "- > H\n  >\n  >\n  >\n  > P", "- H\n  - I\n\n    \n\n    P", "---\n\nP", "![alt](image.png)\n\nP",
        "-", "1.", "- *word*", "- **word**", "> -\tword",
        "- a\n\n  > q\n\n  ```\n  c\n  ```\n\n- b", "- > q\n\n  a",
        "**bold**\n\nP", "é猫\n\n\n\n終", "## \n\nP", "> ## \n>\n> P"] {
        let original = doc(source, Format::Markdown);
        let text = original.text().to_string();
        let offsets: Vec<_> = text.char_indices().map(|(at, _)| at).chain(std::iter::once(text.len())).collect();
        for at in offsets {
            let mut document = doc(source, Format::Markdown);
            if !document.can_insert_table(at..at) { continue; }
            let owner = document.projection().blocks().iter().find(|block|
                block.range.start <= at && at <= block.range.end).unwrap().clone();
            let result = document.prepare_table_edit(document.id(), document.revision(),
                TableEditIntent::Insert { range: at..at, columns: 2, body_rows: 1 });
            let (prepared, _) = match result {
                Ok(prepared) => prepared,
                Err(error) => { failures.push(format!("{source:?} at {at}: {error:?}")); continue; }
            };
            document.commit_model_transaction(prepared).unwrap();
            let table_start = document.projection().tables()[0].range.start;
            let header = document.projection().blocks().iter().find(|block|
                block.range.start == table_start).unwrap();
            assert_eq!(header.quote_depth, owner.quote_depth, "{source:?} at {at}: table quote owner");
            let list_depth = |block: &viem_core::document::Block| block.containers.iter()
                .filter(|container| matches!(container.container.kind, viem_core::document::ContainerKind::List { .. })).count();
            assert_eq!(list_depth(header), list_depth(&owner), "{source:?} at {at}: table list owner");
            assert_eq!(document.text().replace('\n', ""), text.replace('\n', ""), "{source:?} at {at}");
            let saved = document.source_bytes();
            assert_eq!(doc(std::str::from_utf8(&saved).unwrap(), Format::Markdown).text(), document.text());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
fn action(document: &mut Document, intent: TableEditIntent) -> usize {
    let (prepared, caret) = document
        .prepare_table_edit(document.id(), document.revision(), intent)
        .unwrap();
    document.commit_model_transaction(prepared).unwrap();
    caret
}
#[test]
fn gfm_alignment_ragged_extra_and_source_fidelity() {
    let source = "| a | b | c | d |\n| - | :- | :-: | -: |\n| x |\n| 1 | 2 | 3 | 4 | ignored |\n";
    for format in [Format::Markdown] {
        let document = doc(source, format);
        assert_eq!(document.source_bytes(), source.as_bytes());
        let table = &document.projection().tables()[0];
        assert_eq!(
            table.columns,
            vec![
                TableAlignment::Unspecified,
                TableAlignment::Left,
                TableAlignment::Center,
                TableAlignment::Right
            ]
        );
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[1].cells.len(), 4);
        assert_eq!(table.source_rows[3].source_cells.len(), 5);
        if format == Format::Markdown {
            assert!(!document.text().contains("ignored"));
        } else {
            assert!(document.text().contains("ignored"));
        }
    }
}
#[test]
fn gfm_inline_pipes_entities_breaks_and_code() {
    let document = doc(
        "| `a\\|b` | a\\|b | &#124; | **b**<br />c |\n| - | - | - | - |\n",
        Format::Markdown,
    );
    assert_eq!(document.text(), "a|b\na|b\n|\nb\nc");
    assert_eq!(document.projection().tables()[0].rows.len(), 1);
}
#[test]
fn gfm_invalid_header_and_literal_contexts() {
    for source in [
        "a | b\n-\n",
        "```\na | b\n- | -\n```",
        "<table>\na | b\n- | -\n</table>",
        "| a | b || --- | --- || c | d |",
    ] {
        assert!(
            doc(source, Format::Markdown)
                .projection()
                .tables()
                .is_empty(),
            "{source}"
        );
    }
}
#[test]
fn insert_empty_header_body_then_edit_and_undo_exactly() {
    for format in [Format::Markdown] {
        let mut document = doc("", format);
        let caret = action(
            &mut document,
            TableEditIntent::Insert {
                range: 0..0,
                columns: 3,
                body_rows: 4,
            },
        );
        let table = &document.projection().tables()[0];
        assert_eq!(table.rows.len(), 5);
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.rows[0].cells[0].range.start, caret);
        assert_eq!(
            document.source_bytes(),
            b"|  |  |  |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |\n|  |  |  |\n|  |  |  |\n\n"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"");
        assert!(document.redo());
        assert_eq!(document.projection().tables()[0].rows.len(), 5);
    }
}
#[test]
fn rows_columns_alignment_and_header_clearing() {
    let mut document = doc("| A | B |\n| :- | -: |\n| x | y |\n", Format::Markdown);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::InsertColumn {
            table,
            column: 0,
            after: true,
        },
    );
    let table = &document.projection().tables()[0];
    assert_eq!(
        table.columns,
        vec![
            TableAlignment::Left,
            TableAlignment::Left,
            TableAlignment::Right
        ]
    );
    assert_eq!(document.text(), "A\n\nB\nx\n\ny");
    let table = table.id;
    action(
        &mut document,
        TableEditIntent::SetAlignment {
            table,
            columns: 1..2,
            alignment: TableAlignment::Center,
        },
    );
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::DeleteColumns { table, columns: 1..2 },
    );
    assert_eq!(document.text(), "A\nB\nx\ny");
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
    assert_eq!(document.text(), "\n\nx\ny", "Deleting the header clears it instead of promoting a row");
    assert_eq!(document.projection().tables()[0].rows.len(), 2);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::DeleteColumns { table, columns: 1..2 },
    );
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::DeleteColumns { table, columns: 0..1 },
    );
    assert!(document.projection().tables().is_empty());
    assert_eq!(document.text(), "");
}
#[test]
fn row_groups_delete_body_rows_and_clear_a_retained_header() {
    let source = "| A | B |\n| - | - |\n| c | d |\n| e | f |\n| g | h |\n";
    let mut document = doc(source, Format::Markdown);
    let table = document.projection().tables()[0].id;
    let caret = action(&mut document, TableEditIntent::DeleteRows { table, rows: 1..3 });
    assert_eq!(document.source_bytes(), b"| A | B |\n| - | - |\n| g | h |\n");
    assert_eq!(document.projection().tables()[0].rows[1].cells[0].range.start, caret);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());

    // The header survives a group that does not cover the whole table.
    let table = document.projection().tables()[0].id;
    let caret = action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..2 });
    assert_eq!(document.source_bytes(), b"|  |  |\n| - | - |\n| e | f |\n| g | h |\n");
    assert_eq!(document.projection().tables()[0].rows[0].cells[0].range.start, caret);
    assert!(document.undo());
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
    assert_eq!(document.source_bytes(), b"|  |  |\n| - | - |\n| c | d |\n| e | f |\n| g | h |\n");
    // Clearing an already empty header alone changes nothing.
    let table = document.projection().tables()[0].id;
    let revision = document.revision();
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
    assert_eq!(document.revision(), revision);
    assert!(document.undo());

    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..4 });
    assert!(document.projection().tables().is_empty());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn column_groups_delete_together_including_short_rows() {
    let source = "| A | B | C | D |\n| - | :- | -: | - |\n| a | b |\n| w | x | y | z |\n";
    let mut document = doc(source, Format::Markdown);
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteColumns { table, columns: 1..3 });
    assert_eq!(document.source_bytes(), b"| A | D |\n| - | - |\n| a |\n| w | z |\n");
    assert_eq!(document.projection().tables()[0].columns.len(), 2);
    assert!(document.undo());
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteColumns { table, columns: 0..2 });
    assert_eq!(document.source_bytes(), b"| C | D |\n| -: | - |\n||\n| y | z |\n");
    assert!(document.undo());
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteColumns { table, columns: 0..4 });
    assert!(document.projection().tables().is_empty());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}
#[test]
fn typing_pipe_multiline_and_missing_cell() {
    let mut document = doc("a | b\n- | -\nx\n", Format::Markdown);
    let missing = document.projection().tables()[0].rows[1].cells[1]
        .range
        .start;
    document
        .apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(missing..missing, "p|q\nr")],
        })
        .unwrap();
    assert_eq!(document.text(), "a\nb\nx\np|q\nr");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("p\\|q<br>r"));
}
#[test]
fn deleting_the_header_clears_it_and_keeps_extra_body_cells() {
    let mut document = doc("a | b\n- | -\nx | y | hidden\n", Format::Markdown);
    let original = document.source_bytes();
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
    assert_eq!(document.projection().tables()[0].rows.len(), 2);
    assert!(document.projection().tables()[0].rows[0].cells.iter().all(|cell| cell.range.is_empty()));
    assert!(String::from_utf8(document.source_bytes()).unwrap().contains("x | y | hidden"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::InsertColumn {
            table,
            column: 1,
            after: true,
        },
    );
    assert_eq!(document.text(), "a\nb\n\nx\ny\n");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("hidden"));
}
#[test]
fn rectangle_clear_retains_shape_and_reversed_replacement_anchor() {
    let mut document = doc("a | b\n- | -\nx | y\n", Format::Markdown);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::ReplaceCells {
            table,
            rows: 0..2,
            columns: 0..2,
            text: "new".into(),
            anchor_row: 1,
            anchor_column: 1,
        },
    );
    assert_eq!(document.text(), "\n\n\nnew");
    assert_eq!(document.projection().tables()[0].rows.len(), 2);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::ClearCells {
            table,
            rows: 0..2,
            columns: 0..2,
        },
    );
    assert_eq!(document.text(), "\n\n\n");
}
#[test]
fn export_semantic_tables_in_both_views() {
    for format in [Format::Markdown] {
        let document = doc("a | b\n- | -:\nx | y<br>z\n", format);
        let html = String::from_utf8(document.prepare_html_export().render().unwrap()).unwrap();
        assert!(html.contains("<table"));
        assert!(html.contains("<thead>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("text-align:right"));
        assert!(html.contains("scope=\"col\""));
    }
}
#[test]
fn gfm_owned_tables_and_termination() {
    for source in ["> a | b\n> - | -\n> c | d\n", "- a | b\n  - | -\n  c | d\n"] {
        let document = doc(source, Format::Markdown);
        assert_eq!(document.text(), "a\nb\nc\nd");
        assert_eq!(document.projection().tables()[0].rows.len(), 2);
        assert!(
            document.projection().blocks()[0].quote_depth > 0
                || !document.projection().blocks()[0].containers.is_empty()
        );
    }
    let document = doc(
        "a | b\n- | -\ncontinued\nnext | row\n\n# Heading",
        Format::Markdown,
    );
    assert_eq!(document.projection().tables()[0].rows.len(), 3);
    assert!(document.text().ends_with("Heading"));
}
#[test]
fn cell_local_typing_keeps_ids_and_bounded_projection_work_in_both_views() {
    for format in [Format::Markdown] {
        let source = format!("a | b\n- | -\n{}", "word | value\n".repeat(2000));
        let mut document = doc(&source, format);
        let table = &document.projection().tables()[0];
        let id = table.id;
        let cell_id = table.rows[1000].cells[1].id;
        let range = table.rows[1000].cells[0].range.clone();
        let at = range.start + 2;
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at, "Z")],
            })
            .unwrap();
        let work = prepared.summary().projection_work();
        assert!(work.decoded_source_bytes() < 256, "{format:?}: {work:?}");
        assert!(
            work.persistent_records_copied() < 2000,
            "{format:?}: {work:?}"
        );
        document.commit_model_transaction(prepared).unwrap();
        let table = &document.projection().tables()[0];
        assert_eq!(table.id, id);
        assert_eq!(table.rows[1000].cells[1].id, cell_id);
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        let fresh = &reopened.projection().tables()[0];
        assert_eq!(table.range, fresh.range);
        for (actual, expected) in table.rows.iter().zip(fresh.rows.iter()) {
            assert_eq!(actual.range, expected.range);
            assert_eq!(actual.source_range, expected.source_range);
            for (a, b) in actual.cells.iter().zip(&expected.cells) {
                assert_eq!(a.range, b.range);
                assert_eq!(a.source_range, b.source_range);
            }
        }
    }
}
#[test]
fn table_matrix_clipboard_is_quoted_and_pastes_rich_cells_atomically() {
    let source = doc(
        "a | b\n- | -\n**bold**<br>line | quote\"here\n",
        Format::Markdown,
    );
    let (fragment, text) = source
        .table_clipboard_fragment(source.projection().tables()[0].id, 1..2, 0..2)
        .unwrap();
    assert_eq!(text, "\"bold\nline\"\t\"quote\"\"here\"");
    let fragment =
        viem_core::document::ClipboardFragment::from_json(fragment.json(), &text).unwrap();
    let cells = fragment.table_cells().unwrap();
    let mut destination = doc("h\n-\nx\n", Format::Markdown);
    // A one-column table needs an explicit pipe to disambiguate Setext headings.
    if destination.projection().tables().is_empty() {
        destination = doc("| h |\n| - |\n| x |\n", Format::Markdown);
    }
    let table = destination.projection().tables()[0].id;
    action(
        &mut destination,
        TableEditIntent::PasteCells {
            table,
            row: 1,
            column: 0,
            cells,
        },
    );
    assert_eq!(destination.projection().tables()[0].columns.len(), 2);
    assert!(destination.text().contains("bold\nline"));
    assert!(String::from_utf8(destination.source_bytes())
        .unwrap()
        .contains("**bold**"));
}
#[test]
fn table_cell_breaks_and_boundary_space_typing() {
    let mut document = doc("| a | b |\n| - | - |\n| x | y |\n", Format::Markdown);
    for value in [" ", "|", "\n", "z"] {
        let at = document.projection().tables()[0].rows[1].cells[0].range.end;
        document
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(at..at, value)],
            })
            .unwrap();
    }
    assert_eq!(document.text(), "a\nb\nx |\nz\ny");
}
#[test]
fn insertion_preserves_split_prose_and_quote_owners() {
    for (source, at) in [("abcd", 2), ("> abcd", 2), ("- abcd", 2)] {
        let mut document = doc(source, Format::Markdown);
        action(
            &mut document,
            TableEditIntent::Insert {
                range: at..at,
                columns: 2,
                body_rows: 1,
            },
        );
        assert!(document.text().starts_with("ab"));
        assert!(document.text().ends_with("cd"));
        assert_eq!(document.projection().tables().len(), 1);
        let saved = document.source_bytes();
        let reopened = Document::from_bytes(saved, Encoding::Utf8, Format::Markdown).unwrap();
        assert_eq!(reopened.text(), document.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn inserting_a_table_preserves_spaces_folded_breaks_and_inline_scopes() {
    for source in ["a b", "a\nb", "a\n b", "**a b**", "*a b*", "[a b](x 'title')", "<ins>a b</ins>", "> a b", "- a b"] {
        for at in [1, 2] {
            let mut document = doc(source, Format::Markdown);
            let before = document.text().to_owned();
            let (prepared, _) = document.prepare_table_edit(document.id(), document.revision(),
                TableEditIntent::Insert { range: at..at, columns: 1, body_rows: 1 })
                .unwrap_or_else(|error| panic!("{source:?} at {at}: {error:?}"));
            document.commit_model_transaction(prepared).unwrap();
            let table = &document.projection().tables()[0];
            assert_eq!(document.text()[..table.range.start].trim_end_matches('\n'), &before[..at]);
            assert_eq!(document.text()[table.range.end..].trim_start_matches('\n'), &before[at..]);
            let saved = document.source_bytes();
            let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), document.text());
            assert_eq!(reopened.projection().style_spans(), document.projection().style_spans());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}

#[test]
fn table_insertion_preserves_both_heading_halves_and_has_no_empty_leading_heading() {
    for source in ["# ab", "## ab ##", "ab\n==", "> # ab", "# **ab**"] {
        for at in 0..=2 {
            let mut document = doc(source, Format::Markdown);
            let style = document.projection().blocks()[0].style.clone();
            action(&mut document, TableEditIntent::Insert { range: at..at, columns: 1, body_rows: 1 });
            let table = &document.projection().tables()[0];
            if at == 0 { assert_eq!(table.range.start, 0, "{source:?}"); }
            for block in document.projection().blocks().iter().filter(|block|
                !block.range.is_empty() && !block.range.start.eq(&table.rows[0].cells[0].range.start)
                && (block.range.end <= table.range.start || table.range.end <= block.range.start)) {
                assert_eq!(block.style, style, "{source:?} at {at}");
            }
            let saved = document.source_bytes();
            assert_eq!(Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap().text(), document.text());
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
}

#[test]
fn inserting_a_table_at_a_quoted_paragraph_start_adds_no_empty_paragraph() {
    for source in ["> ab", "> **ab**", ">> ab"] {
        let mut document = doc(source, Format::Markdown);
        action(&mut document, TableEditIntent::Insert { range: 0..0, columns: 1, body_rows: 1 });
        let table = &document.projection().tables()[0];
        assert_eq!(table.range.start, 0, "{source:?}");
        assert_eq!(document.text(), "\n\nab", "{source:?}");
        let saved = document.source_bytes();
        assert_eq!(Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap().text(), document.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }
}

#[test]
fn table_insertion_in_a_large_document_only_reprojects_its_local_paragraph() {
    let source = format!("{}**a b**\n\n{}", "before\n\n".repeat(2_000), "after\n\n".repeat(2_000));
    let mut document = doc(&source, Format::Markdown);
    let at = document.text().find("a b").unwrap() + 1;
    let source_at = source.find("**a b**").unwrap();
    let unaffected = document.projection().blocks()[100].id;
    let (prepared, _) = document.prepare_table_edit(document.id(), document.revision(),
        TableEditIntent::Insert { range: at..at, columns: 2, body_rows: 1 }).unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), viem_core::document::ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 512, "{work:?}");
    assert!(prepared.summary().source_patches().iter().all(|patch|
        source_at <= patch.range().start && patch.range().end <= source_at + "**a b**".len()));
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[100].id, unaffected);
    assert!(document.source_bytes().starts_with(&source.as_bytes()[..source_at]));
    assert!(document.source_bytes().ends_with(&source.as_bytes()[source_at + "**a b**".len()..]));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn table_insertion_into_an_empty_paragraph_keeps_local_work_and_source_patches() {
    let prefix = "before\n\n".repeat(2_000) + "## Heading\n\n";
    let suffix = "\n\nFollowing paragraph\n\n".to_owned() + &"after\n\n".repeat(2_000);
    let source = format!("{prefix}{suffix}");
    let mut document = doc(&source, Format::Markdown);
    let at = document.text().find("Heading").unwrap() + "Heading\n".len();
    assert!(document.paragraph_range_at(at).unwrap().is_empty());
    let (prepared, _) = document.prepare_table_edit(document.id(), document.revision(),
        TableEditIntent::Insert { range: at..at, columns: 2, body_rows: 1 }).unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), viem_core::document::ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 512, "{work:?}");
    assert!(prepared.summary().source_patches().iter().all(|patch|
        prefix.len() <= patch.range().start && patch.range().end <= prefix.len() + 2));
    document.commit_model_transaction(prepared).unwrap();
    let saved = document.source_bytes();
    assert!(saved.starts_with(prefix.as_bytes()));
    assert!(saved.ends_with(suffix.as_bytes()));
    assert_eq!(Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap().text(), document.text());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    assert!(document.redo());
    assert_eq!(document.source_bytes(), saved);
}

#[test]
fn inserting_below_header_retains_adjacent_delimiter_and_terminal_spelling() {
    let original = "| A | B |\n| :-- | --: |\n| one | two |";
    let mut document = doc(original, Format::Markdown);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::InsertRow {
            table,
            row: 0,
            after: true,
        },
    );
    assert_eq!(document.text(), "A\nB\n\n\none\ntwo");
    assert_eq!(
        document.source_bytes(),
        b"| A | B |\n| :-- | --: |\n|  |  |\n| one | two |"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original.as_bytes());
}
#[test]
fn table_cell_inline_style_can_span_internal_breaks_and_existing_traits() {
    use viem_core::document::SemanticInlineStyle;
    let mut document = doc(
        "| H |\n| - |\n| **bold**<br>Hello world |",
        Format::Markdown,
    );
    let range = document.projection().tables()[0].rows[1].cells[0]
        .range
        .clone();
    document
        .apply_model_request(ModelRequest::SetSemanticStyle {
            document: document.id(),
            revision: document.revision(),
            range: range.clone(),
            style: SemanticInlineStyle::Emphasis,
            enabled: true,
        })
        .unwrap();
    assert_eq!(document.text(), "H\nbold\nHello world");
    document
        .apply_model_request(ModelRequest::SetSemanticStyle {
            document: document.id(),
            revision: document.revision(),
            range,
            style: SemanticInlineStyle::Strong,
            enabled: false,
        })
        .unwrap();
    assert!(!document
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.application
            == viem_core::document::StyleApplication::Semantic(SemanticInlineStyle::Strong)));
}
#[test]
fn unicode_and_utf16_crlf_tables_preserve_untouched_artifact_bytes() {
    use viem_core::document::FileFormat;
    let source = "| 猫 | café |\r\n| :- | -: |\r\n| 😀 | fin |\r\n";
    let bytes = std::iter::once(0xFEFFu16)
        .chain(source.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut document = Document::from_bytes_with_file_format(
        bytes.clone(),
        Encoding::Utf16Le,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    assert_eq!(document.text(), "猫\ncafé\n😀\nfin");
    assert_eq!(document.source_bytes(), bytes);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::SetAlignment {
            table,
            columns: 0..1,
            alignment: TableAlignment::Center,
        },
    );
    let expected = std::iter::once(0xFEFFu16)
        .chain(source.replace(":-", ":-:").encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(document.source_bytes(), expected);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), bytes);
}
// GitHub Flavored Markdown specification 0.29, examples 198–205:
// https://github.github.com/gfm/#tables-extension-
#[test]
fn official_gfm_table_examples_198_through_205() {
    let cases = [
        (
            198,
            "| foo | bar |\n| --- | --- |\n| baz | bim |\n",
            Some((2, "foo\nbar\nbaz\nbim")),
        ),
        (
            199,
            "| abc | defghi |\n:-: | -----------:\nbar | baz\n",
            Some((2, "abc\ndefghi\nbar\nbaz")),
        ),
        (
            200,
            "| f\\|oo  |\n| ------ |\n| b `\\|` az |\n| b **\\|** im |\n",
            Some((3, "f|oo\nb | az\nb | im")),
        ),
        (
            201,
            "| abc | def |\n| --- | --- |\n| bar | baz |\n> bar\n",
            Some((2, "abc\ndef\nbar\nbaz")),
        ),
        (
            202,
            "| abc | def |\n| --- | --- |\n| bar | baz |\nbar\n\nbar\n",
            Some((3, "abc\ndef\nbar\nbaz\nbar\n")),
        ),
        (203, "| abc | def |\n| --- |\n| bar |\n", None),
        (
            204,
            "| abc | def |\n| --- | --- |\n| bar |\n| bar | baz | boo |\n",
            Some((3, "abc\ndef\nbar\n\nbar\nbaz")),
        ),
        (205, "| abc | def |\n| --- | --- |\n", Some((1, "abc\ndef"))),
    ];
    for (number, source, expected) in cases {
        for format in [Format::Markdown] {
            let document = doc(source, format);
            assert_eq!(document.source_bytes(), source.as_bytes(), "{number}");
            if let Some((rows, text)) = expected {
                let table = &document.projection().tables()[0];
                assert_eq!(table.rows.len(), rows, "{number}");
                if format == Format::Markdown {
                    assert_eq!(
                        &document.text()[table.range.clone()].trim_end_matches('\n'),
                        &text.trim_end_matches('\n'),
                        "{number}"
                    );
                }
                if number == 199 {
                    assert_eq!(
                        table.columns,
                        vec![TableAlignment::Center, TableAlignment::Right]
                    );
                }
            } else {
                assert!(document.projection().tables().is_empty(), "{number}");
            }
        }
    }
}
#[test]
fn table_actions_retain_utf8_bom_and_mixed_existing_endings() {
    use viem_core::document::FileFormat;
    let source = b"\xef\xbb\xbf| a | b |\r\n| - | - |\n| x | y |\r\n";
    let mut document = Document::from_bytes_with_file_format(
        source.to_vec(),
        Encoding::Utf8,
        Format::Markdown,
        FileFormat::Dos,
    )
    .unwrap();
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::InsertRow {
            table,
            row: 0,
            after: true,
        },
    );
    assert_eq!(
        document.source_bytes(),
        b"\xef\xbb\xbf| a | b |\r\n| - | - |\n|  |  |\r\n| x | y |\r\n"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}
#[test]
fn whole_cell_replacement_keeps_first_character_style() {
    let mut document = doc("| H |\n| - |\n| **old** |", Format::Markdown);
    let range = document.projection().tables()[0].rows[1].cells[0]
        .range
        .clone();
    document
        .apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(range, "new")],
        })
        .unwrap();
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("**new**"));
}
#[test]
fn missing_cells_can_clear_and_a_ragged_table_header_clears() {
    let mut document = doc("a | b | c\n- | - | -\nvalue\n", Format::Markdown);
    let table = document.projection().tables()[0].id;
    action(
        &mut document,
        TableEditIntent::ClearCells {
            table,
            rows: 1..2,
            columns: 0..3,
        },
    );
    assert_eq!(document.text(), "a\nb\nc\n\n\n");
    assert!(document.undo());
    let table = document.projection().tables()[0].id;
    action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
    assert_eq!(document.text(), "\n\n\nvalue\n\n");
    assert_eq!(document.projection().tables()[0].columns.len(), 3);
}
#[test]
fn table_insertion_splits_inline_style_without_exposing_delimiters() {
    let mut document = doc("**abcd**", Format::Markdown);
    action(
        &mut document,
        TableEditIntent::Insert {
            range: 2..2,
            columns: 1,
            body_rows: 1,
        },
    );
    assert!(document.text().starts_with("ab"));
    assert!(document.text().ends_with("cd"));
    assert!(!document.text().contains('*'));
    let source = String::from_utf8(document.source_bytes()).unwrap();
    assert!(source.starts_with("**ab**"));
    assert!(source.ends_with("**cd**"));
}
#[test]
fn table_insertion_at_inline_style_edges_keeps_delimiters_with_text() {
    for at in [0, 4] {
        let mut document = doc("**abcd**", Format::Markdown);
        action(
            &mut document,
            TableEditIntent::Insert {
                range: at..at,
                columns: 1,
                body_rows: 1,
            },
        );
        assert!(!document.text().contains('*'));
        assert!(String::from_utf8(document.source_bytes())
            .unwrap()
            .contains("**abcd**"));
    }
}
#[test]
fn table_noop_actions_do_not_create_history_or_dirty_source() {
    let mut document = doc("| a |\n| :--: |\n|  |\n", Format::Markdown);
    let revision = document.revision();
    let table = document.projection().tables()[0].id;
    for intent in [
        TableEditIntent::SetAlignment {
            table,
            columns: 0..1,
            alignment: TableAlignment::Center,
        },
        TableEditIntent::ClearCells {
            table,
            rows: 1..2,
            columns: 0..1,
        },
        TableEditIntent::SetCellsSemanticStyle {
            table,
            rows: 1..2,
            columns: 0..1,
            style: viem_core::document::SemanticInlineStyle::Strong,
            enabled: true,
        },
    ] {
        let (prepared, _) = document
            .prepare_table_edit(document.id(), revision, intent)
            .unwrap();
        assert!(prepared.is_no_op());
        document.commit_model_transaction(prepared).unwrap();
        assert_eq!(document.revision(), revision);
    }
}
#[test]
fn owned_table_header_insertion_and_clearing_keep_list_and_quote_prefixes() {
    for source in ["- a | b\n  - | -\n  c | d\n", "> a | b\n> - | -\n> c | d\n"] {
        let mut document = doc(source, Format::Markdown);
        let table = document.projection().tables()[0].id;
        action(
            &mut document,
            TableEditIntent::InsertRow {
                table,
                row: 0,
                after: true,
            },
        );
        assert_eq!(document.projection().tables()[0].rows.len(), 3);
        assert!(document.undo());
        let table = document.projection().tables()[0].id;
        action(&mut document, TableEditIntent::DeleteRows { table, rows: 0..1 });
        assert_eq!(document.text(), "\n\nc\nd");
        assert!(
            document.projection().blocks()[0].quote_depth > 0
                || !document.projection().blocks()[0].containers.is_empty()
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
#[test]
fn typed_code_pipe_is_protected_and_code_newline_rejection_is_atomic() {
    let mut document = doc("| `ab` | plain |\n| - | - |", Format::Markdown);
    document
        .apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(1..1, "|")],
        })
        .unwrap();
    assert_eq!(document.text(), "a|b\nplain");
    assert!(String::from_utf8(document.source_bytes())
        .unwrap()
        .contains("`a\\|b`"));
    let before = document.source_bytes();
    assert!(document
        .apply_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(1..1, "\n")]
        })
        .is_err());
    assert_eq!(document.source_bytes(), before);
}

#[test]
fn whole_table_text_replacement_removes_hidden_owner_and_retains_neighboring_prose() {
    for source in [
        "| A | B |\n| - | - |\n| x | y |",
        "before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter",
    ] {
        for replacement in ["", "text", " ", "\n", "a\nb"] {
            let mut document = doc(source, Format::Markdown);
            let range = document.projection().tables()[0].range.clone();
            let mut expected = document.text().to_owned();
            expected.replace_range(range.clone(), replacement);
            let prepared = document
                .prepare_model_request(ModelRequest::ApplyTextEdits {
                    document: document.id(),
                    revision: document.revision(),
                    edits: vec![TextEdit::new(range, replacement)],
                })
                .unwrap_or_else(|error| panic!("{source:?} -> {replacement:?}: {error:?}"));
            document.commit_model_transaction(prepared).unwrap();
            assert_eq!(document.text(), expected);
            assert!(document.projection().tables().is_empty());
            assert_eq!(
                doc(
                    &String::from_utf8(document.source_bytes()).unwrap(),
                    Format::Markdown
                )
                .text(),
                expected
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.text(), expected);
        }
    }
}

#[test]
fn structural_table_action_does_not_reparse_unrelated_document() {
    for format in [Format::Markdown] {
        let source = "paragraph\n\n".repeat(2000) + "| A | B |\n| - | - |\n| x | y |\n\nafter";
        let document = doc(&source, format);
        let table = document.projection().tables()[0].id;
        let (prepared, _) = document
            .prepare_table_edit(
                document.id(),
                document.revision(),
                TableEditIntent::InsertRow {
                    table,
                    row: 0,
                    after: true,
                },
            )
            .unwrap();
        assert!(
            prepared.summary().projection_work().decoded_source_bytes() < 1000,
            "{format:?}: {:?}",
            prepared.summary().projection_work()
        );
    }
}

#[test]
fn local_table_row_edit_preserves_unselected_malformed_bytes_and_diagnostics() {
    let bytes = b"| H | V |\n| - | - |\n| a\xffz | safe |\n\nafter";
    for format in [Format::Markdown] {
        let mut document = Document::from_bytes(bytes.to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(document.decoding_diagnostics().len(), 1);
        let range = document.projection().tables()[0].rows[1].cells[1]
            .range
            .clone();
        let offset = range.start
            + document
                .projection()
                .text_tree()
                .slice(range)
                .unwrap()
                .find("safe")
                .unwrap()
            + 2;
        document
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(offset..offset, ".")],
            })
            .unwrap();
        assert_eq!(document.decoding_diagnostics().len(), 1);
        let fresh = Document::from_bytes(document.source_bytes(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            document.decoding_diagnostics()[0].formatted_range,
            fresh.decoding_diagnostics()[0].formatted_range
        );
        assert_eq!(
            document.decoding_diagnostics()[0].source_range,
            fresh.decoding_diagnostics()[0].source_range
        );
        assert!(document.source_bytes().contains(&0xff));
        assert!(document.undo());
        assert_eq!(document.source_bytes(), bytes);
    }
}

#[test]
fn table_following_prose_has_independent_block_ownership_and_header_inherits_cell_box() {
    use viem_core::document::{BlockProperties, CharacterProperties};
    for (separator, following, style) in [
        ("\n\n", "After", "Paragraph"),
        ("\n", "## After", "Heading2"),
    ] {
        let source = format!("| H | V |\n| - | - |\n| x | y |{separator}{following}");
        let document = doc(&source, Format::Markdown);
        let at = document.text().find("After").unwrap();
        let block = document
            .projection()
            .blocks()
            .iter()
            .find(|block| block.range.contains(&at))
            .unwrap();
        assert_eq!(block.style.0, style);
        assert!(block.containers.is_empty());
        assert!(document.projection().tables()[0].range.end < at);
        let sheet = document.projection().style_sheet();
        let header = sheet
            .resolve_paragraph_style(
                &"Paragraph".into(),
                &"Table header".into(),
                None,
                &BlockProperties::default(),
                &CharacterProperties::default(),
            )
            .unwrap();
        assert_eq!(
            (
                header.border_top_width,
                header.border_bottom_width,
                header.border_left_width,
                header.border_right_width
            ),
            (1., 1., 1., 1.)
        );
        assert_eq!(
            (
                header.padding_top,
                header.padding_bottom,
                header.padding_left,
                header.padding_right
            ),
            (6., 6., 10., 10.)
        );
    }
    let document = doc("| H | V |\n| - | - |\n| x | y |\nAfter", Format::Markdown);
    assert_eq!(
        document.projection().tables()[0].rows.len(),
        3,
        "plain text without a blank line remains a GFM body row"
    );
}

#[test]
fn literal_carriage_returns_are_line_content_rather_than_grammar_breaks() {
    // Bare LF selects Unix endings, so each CR is content of its physical line.
    for source in [
        "| a |\r\n| - |\n",
        "| abc | def |\n| --- | -\r-- |\n",
        "|a|\r|-|\na\r# b",
        "|a|\r|-|",
        "a\r# b",
    ] {
        let wysiwyg = doc(source, Format::Markdown);
        assert!(wysiwyg.projection().tables().is_empty(), "{source:?}");
        assert!(wysiwyg.text().contains('\r'), "{source:?}");
        assert!(!wysiwyg.projection().blocks().iter().any(|block| block.style.0.starts_with("Heading")), "{source:?}");
    }
}

#[test]
fn audit_empty_table_cells_keep_padding_and_quote_ownership() {
    for prefix in ["", "> ", "> > "] {
        let source = format!("{prefix}|  |  |\n{prefix}| - | - |\n{prefix}|  |  |");
        for row in 0..2 {
            for column in 0..2 {
                let mut document = doc(&source, Format::Markdown);
                let at = document.projection().tables()[0].rows[row].cells[column].range.start;
                document.insert(at, "x").unwrap();
                let updated = String::from_utf8(document.source_bytes()).unwrap();
                assert!(updated.contains("| x |"), "{updated}");
                let reopened = doc(&updated, Format::Markdown);
                assert_eq!(reopened.text(), document.text());
                assert_eq!(reopened.projection().tables()[0].rows[row].cells[column].range.len(), 1);
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), updated.as_bytes());
            }
        }
    }
}

#[test]
fn audit_no_op_inline_formatting_through_cell_boundary() {
    use viem_core::document::SemanticInlineStyle;
    for (source, end, style) in [
        ("| *a* | b |\n| - | - |", 2, SemanticInlineStyle::Emphasis),
        ("| **bold** | _it_ |\n| - | - |", 5, SemanticInlineStyle::Strong),
    ] {
        let mut document = doc(source, Format::Markdown);
        document.apply_model_request(ModelRequest::SetSemanticStyle {
            document: document.id(), revision: document.revision(), range: 0..end,
            style, enabled: true,
        }).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

/// Tables nested in list and quote owners are reparsed alone; their cells
/// inherit the owners' context. Every table intent must stay regional and
/// match a fresh parse, including the owners' identities around the table.
#[test]
fn nested_table_intents_stay_regional_and_match_a_fresh_parse() {
    use viem_core::document::{ProjectionWorkScope, SemanticInlineStyle};
    fn table_shape(document: &Document) -> Vec<String> {
        document.projection().tables().iter().map(|table| format!("{:?} {:?} {:?} {:?} {:?}",
            table.range, table.source_range, table.columns,
            table.rows.iter().map(|row| (row.range.clone(), row.source_range.clone(),
                row.cells.iter().map(|cell| (cell.range.clone(), cell.source_range.clone(), cell.missing)).collect::<Vec<_>>()))
                .collect::<Vec<_>>(),
            table.source_rows.iter().collect::<Vec<_>>())).collect()
    }
    // Owner identities are compared by first appearance, so equal owner trees
    // compare equal across independently assigned identities.
    fn block_shape(document: &Document) -> Vec<String> {
        let mut owners = Vec::new();
        document.projection().blocks().iter().map(|block| {
            let containers = block.containers.iter().map(|member| {
                let owner = owners.iter().position(|id| *id == member.container.id).unwrap_or_else(|| {
                    owners.push(member.container.id);
                    owners.len() - 1
                });
                (owner, member.container.kind, member.starts_here, member.ends_here)
            }).collect::<Vec<_>>();
            format!("{:?} {:?} {:?} {} {} {containers:?}", block.range, block.kind, block.style,
                block.quote_depth, block.list_loose)
        }).collect()
    }
    let sources = [
        "- A list item with a table\n\n  | Item | Value |\n  | --- | ---: |\n  | Inside the list | 4 |\n  | x | y |\n\nafter",
        "> | Quoted item | Value |\n> | --- | ---: |\n> | Inside the quotation | 7 |\n> | x | y |\n\nafter",
        "before\n\n- item\n\n  > | A | B |\n  > | - | - |\n  > | 1 | 2 |\n  > | 3 | 4 |\n- next",
        "intro\n\n- | A | B |\n  | - | - |\n  | 1 | 2 |\n  | 3 | 4 |\n- next",
        "> - item\n>\n>   | A | B |\n>   | :- | -: |\n>   | 1 | 2 |\n>   | 3 | 4 |",
    ];
    let mut failures = Vec::new();
    for source in sources {
        let probe = doc(source, Format::Markdown);
        let table = &probe.projection().tables()[0];
        let (id, rows, columns) = (table.id, table.rows.len(), table.columns.len());
        let mut intents = vec![
            TableEditIntent::ClearCells { table: id, rows: 0..rows, columns: 0..columns },
            TableEditIntent::ClearCells { table: id, rows: 1..2, columns: 1..2 },
            TableEditIntent::ReplaceCells { table: id, rows: 0..rows, columns: 0..columns,
                text: "new".into(), anchor_row: 0, anchor_column: 0 },
            TableEditIntent::ReplaceCells { table: id, rows: 1..rows, columns: 0..1,
                text: "a|b\nc".into(), anchor_row: 2, anchor_column: 0 },
            TableEditIntent::InsertColumn { table: id, column: 0, after: false },
            TableEditIntent::InsertColumn { table: id, column: columns - 1, after: true },
            TableEditIntent::DeleteColumns { table: id, columns: 0..1 },
            TableEditIntent::SetAlignment { table: id, columns: 0..1, alignment: TableAlignment::Center },
            TableEditIntent::SetCellsSemanticStyle { table: id, rows: 0..rows, columns: 0..1,
                style: SemanticInlineStyle::Strong, enabled: true },
        ];
        for row in 0..rows {
            intents.push(TableEditIntent::InsertRow { table: id, row, after: true });
            intents.push(TableEditIntent::DeleteRows { table: id, rows: row..row + 1 });
            if row > 0 {
                intents.push(TableEditIntent::InsertRow { table: id, row, after: false });
            }
        }
        for intent in intents {
            let mut document = doc(source, Format::Markdown);
            let (prepared, _) = match document.prepare_table_edit(document.id(), document.revision(), intent.clone()) {
                Ok(prepared) => prepared,
                Err(error) => { failures.push(format!("{source:?} {intent:?}: {error:?}")); continue; }
            };
            // Like a top-level table's, the final row's ending separates the
            // table from following content, so changing it is verified completely.
            let final_ending = match &intent {
                TableEditIntent::InsertRow { row, after: true, .. } => row + 1 == rows,
                TableEditIntent::DeleteRows { rows: deleted, .. } => deleted.end == rows,
                _ => false,
            };
            if !final_ending
                && prepared.summary().projection_work().scope() != ProjectionWorkScope::RegionalHardLines {
                failures.push(format!("{source:?} {intent:?}: {:?}", prepared.summary().projection_work().scope()));
            }
            document.commit_model_transaction(prepared).unwrap();
            let fresh = doc(&String::from_utf8(document.source_bytes()).unwrap(), Format::Markdown);
            assert_eq!(document.text(), fresh.text(), "{source:?} {intent:?}");
            assert_eq!(table_shape(&document), table_shape(&fresh), "{source:?} {intent:?}");
            assert_eq!(block_shape(&document), block_shape(&fresh), "{source:?} {intent:?}");
            let edited = document.source_bytes();
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes(), "{source:?} {intent:?}");
            assert!(document.redo());
            assert_eq!(document.source_bytes(), edited, "{source:?} {intent:?}");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Text beside a whole table is edited together with it: the table leaves an
/// empty paragraph that the text edit joins, as if the table were one paragraph.
#[test]
fn text_edits_reaching_exactly_a_whole_table_edge_remove_the_table() {
    for source in [
        "before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter",
        "- item\n\n  | A | B |\n  | - | - |\n  | x | y |\n\n- next",
        "> before\n>\n> | A | B |\n> | - | - |\n> | x | y |\n>\n> after",
    ] {
        let probe = doc(source, Format::Markdown);
        let table = probe.projection().tables()[0].range.clone();
        let len = probe.text().len();
        for range in [2..table.end, table.start..len - 2] {
            for replacement in ["", "new"] {
                let mut document = doc(source, Format::Markdown);
                let mut expected = document.text().to_owned();
                expected.replace_range(range.clone(), replacement);
                document
                    .apply_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(range.clone(), replacement)],
                    })
                    .unwrap_or_else(|error| panic!("{source:?} {range:?} {replacement:?}: {error:?}"));
                assert_eq!(document.text(), expected, "{source:?} {range:?}");
                assert!(document.projection().tables().is_empty());
                let edited = document.source_bytes();
                assert_eq!(doc(&String::from_utf8(edited.clone()).unwrap(), Format::Markdown).text(), expected);
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
                assert!(document.redo());
                assert_eq!(document.source_bytes(), edited);
            }
        }
    }
}
