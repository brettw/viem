//! Exercise structural deletion through the same commands used by native line selection.
use viem_core::command::{CommandStatus, InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const FIXTURES: &[(Format, &str)] = &[
    (Format::Markdown, "- a\n- b"),
    (Format::Markdown, "- a\n  - b\n  - c\n- d"),
    (Format::Markdown, "- a\n\n  b\n- c"),
    (Format::Markdown, "1. a\n2. b\n   - c"),
    (Format::Markdown, "- a  \n  b\n- c"),
    (Format::Markdown, "> a\n>\n> b"),
    (Format::Markdown, "# a\n\nb\n\n```\nc\nd\n```"),
    (Format::Markdown, "a\n\n---\n\nb"),
    (Format::Markdown, "<details>\n<summary>s</summary>\n\nbody\n\n</details>\n\nafter"),
    (Format::Markdown, "a\n\n```text\nb\n\n\nc\n```\n\n## d"),

];

fn editor(source: &[u8], format: Format) -> (Editor, ViewId) {
    let mut core =
        Core::new(Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 10_000., 3000.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) -> Result<(), String> {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .map_err(|error| format!("{key:?}: {error:?}"))?;
    let status = output.command.unwrap().status;
    if !matches!(
        status,
        CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
    ) {
        return Err(format!("{key:?}: {status:?}"));
    }
    Ok(())
}

fn delete_lines(
    core: &mut Editor,
    view: ViewId,
    start: usize,
    extra_lines: usize,
    reverse: bool,
    native: Option<Mode>,
    deletion: Key,
) -> Result<(), String> {
    delete_lines_observing(core, view, start, extra_lines, reverse, native, deletion, |_| ())
}

#[allow(clippy::too_many_arguments)]
fn delete_lines_observing(
    core: &mut Editor,
    view: ViewId,
    start: usize,
    extra_lines: usize,
    reverse: bool,
    native: Option<Mode>,
    deletion: Key,
    selected: impl FnOnce(&Editor),
) -> Result<(), String> {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: start,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .map_err(|error| format!("place: {error:?}"))?;
    key(core, view, Key::Char('V'))?;
    for _ in 0..extra_lines {
        key(core, view, Key::Char(if reverse { 'k' } else { 'j' }))?;
    }
    if let Some(return_mode) = native {
        core.set_selection_origin(view, SelectionOrigin::Mouse, return_mode)
            .map_err(|error| format!("origin: {error:?}"))?;
    }
    selected(core);
    key(core, view, deletion)
}

#[test]
fn all_contiguous_line_selections_delete_like_plain_text_and_round_trip() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &(format, source) in FIXTURES {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = document.text().to_owned();
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect::<Vec<_>>();
        for (line, &start) in starts.iter().enumerate() {
            for extra_lines in 0..starts.len() - line {
                for reverse in [false, true] {
                    let start = if reverse {
                        starts[line + extra_lines]
                    } else {
                        start
                    };
                    for (native, deletion) in [
                        (None, Key::Char('d')),
                        (None, Key::Delete),
                        (None, Key::Backspace),
                        (Some(Mode::Normal), Key::Delete),
                        (Some(Mode::Normal), Key::Backspace),
                        (Some(Mode::Insert), Key::Delete),
                        (Some(Mode::Insert), Key::Backspace),
                    ] {
                        checked += 1;
                        let case = format!("{format:?} {source:?}, lines {line}..={}, reverse={reverse}, native={native:?}, {deletion:?}", line + extra_lines);
                        let (mut plain, plain_view) = editor(text.as_bytes(), Format::PlainText);
                        delete_lines(
                            &mut plain,
                            plain_view,
                            start,
                            extra_lines,
                            reverse,
                            native,
                            deletion,
                        )
                        .unwrap();
                        let (mut core, view) = editor(source.as_bytes(), format);
                        if let Err(error) = delete_lines(
                            &mut core,
                            view,
                            start,
                            extra_lines,
                            reverse,
                            native,
                            deletion,
                        ) {
                            failures.push(format!("{case}: {error}"));
                            continue;
                        }
                        let after = core.document().source_bytes();
                        let visible = core.document().text().replace('\u{a0}', " ");
                        let expected = plain.document().text().replace('\u{a0}', " ");
                        if visible != expected {
                            failures.push(format!(
                                "{case}: expected {expected:?}, got {visible:?}; source={:?}",
                                String::from_utf8_lossy(&after)
                            ));
                        }
                        assert_eq!(
                            core.command_state(view).unwrap().mode(),
                            plain.command_state(plain_view).unwrap().mode(),
                            "{case}"
                        );
                        assert_eq!(
                            core.command_state(view)
                                .unwrap()
                                .register('"')
                                .map(|r| (&r.text, r.kind)),
                            plain
                                .command_state(plain_view)
                                .unwrap()
                                .register('"')
                                .map(|r| (&r.text, r.kind)),
                            "{case}"
                        );
                        let reopened =
                            Document::from_bytes(after.clone(), Encoding::Utf8, format).unwrap();
                        assert_eq!(reopened.text(), core.document().text(), "{case}");
                        if after != source.as_bytes() {
                            key(&mut core, view, Key::Escape).unwrap();
                            key(&mut core, view, Key::Char('u')).unwrap();
                            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{case}");
                            key(&mut core, view, Key::Ctrl('r')).unwrap();
                            assert_eq!(core.document().source_bytes(), after, "{case}");
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "checked {checked} structural command deletions; {} failures",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(180)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Table rows are one displayed row but one text line per cell, so these
/// fixtures check that each deletion removes exactly the yanked whole lines.
const ROW_FIXTURES: &[&str] = &[
    "# a\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\nb",
    "a\n\n<details>\n<summary>s</summary>\n\n| x |\n|---|\n| 1 |\n\n</details>\n\n**b** c\n\n```text\nd\n\ne\n```\n\n## f",
    "| x |\n|---|\n| 1 |\n\n| y |\n|---|\n| 2 |",
    "> | x |\n> |---|\n> | 1 |\n>\n> q\n\n- l",
    "- a\n\n  | x |\n  |---|\n  | 1 |\n- b",
    "a\n\n| x |\n|---|\n| 1 |\n\n```\nc\n```\n\n> q",
    "## h\n\n\n<details>\n<summary>s</summary>\n\n| x | y |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n</details>\n\n**b**\n\n```text\nc\n\nd\n```\n\n## e",
];

/// Selected lines disappear, except that cells in partially selected rows,
/// and the header of a surviving table, are cleared in place.
fn expected_line_deletion(document: &Document, anchor: usize, cursor: usize) -> String {
    let text = document.text();
    let mut lines = Vec::new();
    let mut start = 0;
    for (at, _) in text.match_indices('\n') {
        lines.push(start..at);
        start = at + 1;
    }
    lines.push(start..text.len());
    let line_of = |offset: usize| lines.iter().position(|line| offset <= line.end).unwrap();
    let (first, last) = (line_of(anchor.min(cursor)), line_of(anchor.max(cursor)));
    let selection = lines[first].start..lines[last].end;
    let covers = |range: &std::ops::Range<usize>| selection.start <= range.start && range.end <= selection.end;
    let tables = document.projection().tables();
    let mut kept = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index < first || index > last {
            kept.push(&text[line.clone()]);
            continue;
        }
        let cleared = tables.iter().any(|table| {
            table.range.start <= line.start && line.end <= table.range.end && !covers(&table.range)
                && table.rows.iter().enumerate().any(|(row_index, row)| {
                    row.range.start <= line.start && line.end <= row.range.end
                        && (row_index == 0 || !covers(&row.range))
                })
        });
        if cleared {
            kept.push("");
        }
    }
    kept.join("\n")
}

#[test]
fn line_selections_crossing_tables_delete_exactly_the_selected_rows() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &source in ROW_FIXTURES {
        let (probe, _) = editor(source.as_bytes(), Format::Markdown);
        let text = probe.document().text().to_owned();
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect::<Vec<_>>();
        for &start in &starts {
            for reverse in [false, true] {
                for extra_lines in 0..starts.len() {
                    for (native, deletion) in [
                        (None, Key::Char('d')),
                        (Some(Mode::Normal), Key::Delete),
                        (Some(Mode::Insert), Key::Backspace),
                    ] {
                        checked += 1;
                        let case = format!("{source:?} start={start} extra={extra_lines} reverse={reverse} native={native:?} {deletion:?}");
                        let (mut core, view) = editor(source.as_bytes(), Format::Markdown);
                        let mut expected = None;
                        if let Err(error) = delete_lines_observing(&mut core, view, start, extra_lines, reverse, native, deletion,
                            |core| expected = Some(expected_line_deletion(core.document(),
                                core.command_state(view).unwrap().visual_anchor().unwrap(),
                                core.command_state(view).unwrap().cursor()))) {
                            failures.push(format!("{case}: {error}"));
                            continue;
                        }
                        let after = core.document().source_bytes();
                        let visible = core.document().text().replace('\u{a0}', " ");
                        let expected = expected.unwrap().replace('\u{a0}', " ");
                        if visible != expected {
                            failures.push(format!("{case}: expected {expected:?}, got {visible:?}; source={:?}", String::from_utf8_lossy(&after)));
                            continue;
                        }
                        let reopened = Document::from_bytes(after.clone(), Encoding::Utf8, Format::Markdown).unwrap();
                        if reopened.text() != core.document().text() {
                            failures.push(format!("{case}: reopened {:?} != {:?}", reopened.text(), core.document().text()));
                            continue;
                        }
                        if after != source.as_bytes() {
                            key(&mut core, view, Key::Escape).unwrap();
                            key(&mut core, view, Key::Char('u')).unwrap();
                            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{case}");
                            key(&mut core, view, Key::Ctrl('r')).unwrap();
                            assert_eq!(core.document().source_bytes(), after, "{case}");
                        }
                    }
                }
            }
        }
    }
    eprintln!("checked {checked} table line deletions; {} failures", failures.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(),
        failures.iter().take(120).cloned().collect::<Vec<_>>().join("\n"));
}

fn delete_rows_and_check_undo(source: &str, from: &str, extra_lines: usize) -> String {
    let (mut core, view) = editor(source.as_bytes(), Format::Markdown);
    let start = core.document().text().find(from).unwrap();
    delete_lines(&mut core, view, start, extra_lines, false, None, Key::Char('d')).unwrap();
    let after = String::from_utf8(core.document().source_bytes()).unwrap();
    key(&mut core, view, Key::Char('u')).unwrap();
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    key(&mut core, view, Key::Ctrl('r')).unwrap();
    assert_eq!(core.document().source_bytes(), after.as_bytes());
    after
}

fn large_details_table(rows: usize) -> String {
    let rows = (0..rows).map(|row| format!("| r{row} | v{row} |")).collect::<Vec<_>>().join("\n");
    format!("<details>\n<summary>s</summary>\n\n| x | y |\n|---|---|\n{rows}\n\n</details>\n\nafter")
}

#[test]
fn line_deletion_removes_heading_details_table_and_code_block_together() {
    let source = "## a\n\n## h\n\n\n<details>\n<summary>s</summary>\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\n</details>\n\n**b**\n\n```text\nc\n\nd\n```\n\n## e";
    // h, s, the two table rows, the closing row, b, and the three code rows.
    assert_eq!(delete_rows_and_check_undo(source, "h\n", 8), "## a\n\n## e");
}

#[test]
fn line_deletion_into_a_large_details_table_keeps_its_later_rows() {
    // Forty rows exceed the bounded paragraph walk; the table bounds the
    // HTML owner instead. The last selected row is only partly selected.
    let source = format!("## h\n\n{}", large_details_table(40));
    let after = delete_rows_and_check_undo(&source, "h\n", 5);
    let expected = format!("|  |  |\n|---|---|\n|  | v2 |\n{}", &source[source.find("| r3 ").unwrap()..]);
    assert_eq!(after, expected);
}

#[test]
fn line_deletion_before_details_keeps_its_hidden_opening_tag() {
    for rows in [1, 40] {
        let details = large_details_table(rows);
        for before in ["## h", "p", "- l"] {
            let source = format!("{before}\n\n{details}");
            let from = before.trim_start_matches(['#', '-', ' ']);
            assert_eq!(delete_rows_and_check_undo(&source, from, 0), details, "{before:?} with {rows} rows");
        }
    }
}
