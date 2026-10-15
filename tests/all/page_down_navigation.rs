use viem_core::command::{layout_motion::LayoutMotionError, CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::{
    compute_layout_job, LayoutCancellationToken, LayoutEngine, LayoutExecutionContext,
    LayoutJobPriority, MockTextMeasurementProvider,
};
use viem_core::{Core, CoreEvent, Document, ViewId};

fn completed_page(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let result = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    assert_eq!(result.command.unwrap().status, CommandStatus::Complete);
    let state = core.command_state(view).unwrap();
    let layout = core.layout(view).unwrap();
    let row = layout
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| {
            row.carets.iter().any(|caret| {
                caret.point.text_offset == state.cursor()
                    && caret.point.affinity == state.boundary_affinity()
            })
        })
        .expect("paging installs the active caret's geometry");
    let ink = row.ink_bounds();
    let top = ink.map_or(row.y, |ink| row.y.min(ink.y));
    let bottom = ink.map_or(row.y + row.natural_height(), |ink| {
        (row.y + row.natural_height()).max(ink.y + ink.height)
    });
    assert!(top >= layout.viewport_top() - 0.05);
    assert!(bottom <= layout.viewport_top() + layout.height() + 0.05);
}

fn three_column_table(rows: usize) -> String {
    let mut source = String::from("| Left | Middle | Right |\n| --- | --- | --- |\n");
    for row in 0..rows {
        source.push_str(&format!(
            "| Left {row:03} | Middle {row:03} | Right {row:03} |\n"
        ));
    }
    source.push_str("\n");
    source
}

fn mixed_paging_source(reports: usize) -> String {
    let mut source = String::from("# Bug summary\n\n");
    for index in 0..18 {
        source.push_str(&format!("Issue {index}: a paragraph with **formatted words** and enough detail to wrap across visual rows.\n\n"));
    }
    source.push_str("## Summary table\n\n");
    source.push_str(&three_column_table(35));
    source.push_str("# Detailed reports\n\n");
    for index in 0..reports {
        source.push_str(&format!(
            "### Report {index}\n\nAnother paragraph with detail about editing and navigation.\n\n"
        ));
    }
    source
}

#[test]
fn wysiwyg_pages_retrace_the_same_screen_distance_across_tables_and_headings() {
    let source = mixed_paging_source(2_000);
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 660., 320.);
    let revision = core.document().revision();
    let mut tops = vec![core.layout(view).unwrap().viewport_top()];
    let mut table_seen = false;
    for _ in 0..14 {
        completed_page(&mut core, view, Key::PageDown);
        let layout = core.layout(view).unwrap();
        let snapshot = layout.snapshot().unwrap();
        assert!(!snapshot.coverage.is_full_document());
        assert!(snapshot.coverage.hard_lines().len() < 256);
        assert!(
            snapshot.rows.len() < 512,
            "paging a large document materializes a bounded local region"
        );
        let top = layout.viewport_top();
        let amount = top - tops.last().unwrap();
        let expected_amount = layout.height() - 2. * snapshot.default_row_advance;
        assert!((amount - expected_amount).abs() < 0.05,
            "a full page keeps two base-text rows of overlap: expected {expected_amount}, got {amount}");
        assert!(
            amount > layout.height() / 2. && amount < layout.height(),
            "one page retains overlap and moves close to one screen, got {amount}"
        );
        if tops.len() > 1 {
            let first_amount = tops[1] - tops[0];
            assert!((amount - first_amount).abs() < 0.05,
                "table cells and heading spacing must not change page distance: {amount} versus {first_amount}");
        }
        table_seen |= layout.snapshot().unwrap().rows.iter().any(|row| {
            row.table_cell.is_some() && row.y < top + layout.height() && row.y + row.height() > top
        });
        tops.push(top);
    }
    assert!(
        table_seen,
        "the regression must page through the three-column table"
    );
    for expected in tops[..tops.len() - 1].iter().rev() {
        completed_page(&mut core, view, Key::PageUp);
        let actual = core.layout(view).unwrap().viewport_top();
        assert!(
            (actual - expected).abs() < 0.05,
            "Page Up must retrace Page Down: expected {expected}, got {actual}"
        );
    }
    assert_eq!(core.document().revision(), revision);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn wysiwyg_page_count_multiplies_the_same_screen_distance_in_both_directions() {
    let source = mixed_paging_source(80);
    let mut core = Core::new(
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 660., 320.);
    completed_page(&mut core, view, Key::PageDown);
    let initial = core.layout(view).unwrap().viewport_top();
    let amount = initial;
    for key in [Key::PageDown, Key::PageUp] {
        let before = core.layout(view).unwrap().viewport_top();
        let count = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::key('3')))
            .unwrap();
        assert_eq!(count.command.unwrap().status, CommandStatus::Pending);
        completed_page(&mut core, view, key);
        let after = core.layout(view).unwrap().viewport_top();
        assert!(
            ((after - before).abs() - amount * 3.).abs() < 0.05,
            "a count multiplies page distance across changes in row density"
        );
    }
    assert!((core.layout(view).unwrap().viewport_top() - initial).abs() < 0.05);
}

#[test]
fn wysiwyg_paging_stays_in_the_table_column_at_desired_x() {
    let source = three_column_table(90);
    let mut core = Core::new(
        Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 660., 240.);
    let at = core.document().text().find("Middle 008").unwrap();
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
    let row_y = core
        .layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.text_range.contains(&at))
        .unwrap()
        .y;
    core.handle(
        view,
        CoreEvent::SetViewportOrigin {
            left: 0.,
            top: Some(row_y - 30.),
        },
    )
    .unwrap();
    let start_top = core.layout(view).unwrap().viewport_top();
    let start_row = 9; // Table row zero is the header.
    for (key, down) in [(Key::PageDown, true), (Key::PageUp, false)] {
        completed_page(&mut core, view, key);
        let state = core.command_state(view).unwrap();
        let row = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| {
                row.carets.iter().any(|caret| {
                    caret.point.text_offset == state.cursor()
                        && caret.point.affinity == state.boundary_affinity()
                })
            })
            .unwrap();
        let cell = row
            .table_cell
            .as_ref()
            .expect("the target remains inside the long table");
        assert_eq!(
            cell.column, 1,
            "paging follows desired x rather than counting cell rows"
        );
        if down {
            assert!(cell.row > start_row);
        }
    }
    assert!((core.layout(view).unwrap().viewport_top() - start_top).abs() < 0.05);
}

#[test]
fn page_down_demand_is_atomic_and_resumes_after_exact_layout_is_installed() {
    let source = (0..120)
        .map(|index| format!("## Section {index}\n\nA paragraph with **formatted words** that wraps across several visual rows.\n\n"))
        .collect::<String>();
    for (format, flow) in [
        (Format::Markdown, false),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 320., 160.);
        let revision = core.document().revision();
        let mut demand_observed = false;
        for _ in 0..100 {
            let cursor = core.command_state(view).unwrap().cursor();
            let top = core.layout(view).unwrap().viewport_top();
            let result = core
                .handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
                .unwrap();
            let status = result.command.unwrap().status;
            if let CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(
                demand,
            )) = status
            {
                assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
                assert_eq!(core.layout(view).unwrap().viewport_top(), top);
                assert_eq!(core.document().revision(), revision);
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                assert!(cursor < core.document().text().len() / 2);
                let request = core
                    .prepare_view_layout_demand(
                        view,
                        LayoutJobPriority::NewlyExposedRows,
                        &demand,
                        LayoutCancellationToken::new(),
                    )
                    .unwrap();
                let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
                let candidate =
                    compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool)
                        .unwrap();
                core.install_view_layout_job(view, candidate).unwrap();
                let retry = core
                    .handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
                    .unwrap();
                assert_eq!(retry.command.unwrap().status, CommandStatus::Complete);
                assert!(core.command_state(view).unwrap().cursor() > cursor);
                assert!(core.layout(view).unwrap().viewport_top() > top);
                demand_observed = true;
                break;
            }
            assert_eq!(status, CommandStatus::Complete);
        }
        assert!(demand_observed, "{format:?} flow={flow}");
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
