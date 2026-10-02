//! Table ABI transactions, exact identities, buffer ownership and geometry.
use super::ffi_core_surface::{table_ffi_fixture, TableFfiFixture};
use std::ptr;
use viem_core::ffi::*;

const TABLE: &[u8] =
    b"| A | B | C |\n| :-- | :-: | --: |\n| one | two | three |\n| four | five | six |";
fn context(f: &TableFfiFixture) -> ViemTableContextV1 {
    let mut out = ViemTableContextV1::default();
    assert_eq!(
        unsafe { viem_core_view_table_context(f.handle, f.view, &mut out) },
        ViemStatus::Ok
    );
    out
}
fn state(f: &TableFfiFixture) -> ViemDocumentStateV1 {
    let mut out = ViemDocumentStateV1::default();
    assert_eq!(
        unsafe { viem_core_document_state(f.handle, &mut out) },
        ViemStatus::Ok
    );
    out
}
fn source(f: &TableFfiFixture) -> Vec<u8> {
    let revision = state(f).document_revision;
    let mut count = 0;
    let result =
        unsafe { viem_core_copy_source_bytes(f.handle, revision, ptr::null_mut(), 0, &mut count) };
    assert!(matches!(
        result,
        ViemStatus::Ok | ViemStatus::BufferTooSmall
    ));
    let mut out = vec![0; count as usize];
    assert_eq!(
        unsafe {
            viem_core_copy_source_bytes(f.handle, revision, out.as_mut_ptr(), count, &mut count)
        },
        ViemStatus::Ok
    );
    out
}
fn info(f: &TableFfiFixture) -> ViemLayoutSnapshotInfoV1 {
    let mut info = ViemLayoutSnapshotInfoV1::default();
    assert_eq!(
        unsafe { viem_core_view_layout_snapshot_info(f.handle, f.view, &mut info) },
        ViemStatus::Ok
    );
    info
}
fn cells(f: &TableFfiFixture) -> Vec<ViemTableCellV1> {
    let identity = info(f).identity;
    let mut count = 0;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(
                f.handle,
                f.view,
                &identity,
                ptr::null_mut(),
                0,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    let mut cells = vec![ViemTableCellV1::default(); count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(
                f.handle,
                f.view,
                &identity,
                cells.as_mut_ptr(),
                count,
                &mut count,
            )
        },
        ViemStatus::Ok
    );
    cells
}
fn select(f: &TableFfiFixture) -> ViemTableSelectionV1 {
    let c = context(f);
    ViemTableSelectionV1 {
        struct_size: VIEM_TABLE_SELECTION_V1_SIZE,
        active: 1,
        document_id: c.selection.document_id,
        document_revision: c.selection.document_revision,
        table_id: c.table_id,
        anchor_row: 2,
        anchor_column: 2,
        active_row: 0,
        active_column: 1,
    }
}
fn action(c: ViemTableContextV1, action: u32) -> ViemTableActionV1 {
    ViemTableActionV1 {
        struct_size: VIEM_TABLE_ACTION_V1_SIZE,
        action,
        expected: c,
        ..Default::default()
    }
}

#[test]
fn insertion_rejects_invalid_sizes_and_stale_picker_without_changes() {
    for format in [VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_MARKDOWN_SOURCE] {
        let f = table_ffi_fixture(b"alpha beta", format);
        let original = context(&f);
        assert_eq!(original.flags, VIEM_TABLE_CAN_INSERT);
        let valid = ViemInsertTableV1 {
            struct_size: VIEM_INSERT_TABLE_V1_SIZE,
            reserved: 0,
            expected_selection: original.selection,
            columns: 3,
            body_rows: 4,
        };
        for invalid in [
            ViemInsertTableV1 {
                struct_size: 0,
                ..valid
            },
            ViemInsertTableV1 {
                reserved: 1,
                ..valid
            },
            ViemInsertTableV1 {
                columns: 0,
                ..valid
            },
            ViemInsertTableV1 {
                columns: 21,
                ..valid
            },
            ViemInsertTableV1 {
                body_rows: 0,
                ..valid
            },
            ViemInsertTableV1 {
                body_rows: 51,
                ..valid
            },
        ] {
            let mut out = ViemCoreOutcomeV1::default();
            assert_eq!(
                unsafe { viem_core_view_insert_table(f.handle, f.view, &invalid, &mut out) },
                ViemStatus::InvalidArgument
            );
            assert_eq!(context(&f), original);
            assert_eq!(source(&f), b"alpha beta");
        }
        let mut out = ViemCoreOutcomeV1::default();
        let place = ViemPlaceCursorV1 {
            struct_size: VIEM_PLACE_CURSOR_V1_SIZE,
            document_revision: original.selection.document_revision,
            text_offset: 3,
            affinity: VIEM_BOUNDARY_AFFINITY_DOWNSTREAM,
            ..Default::default()
        };
        assert_eq!(
            unsafe { viem_core_view_place_cursor(f.handle, f.view, &place, &mut out) },
            ViemStatus::Ok
        );
        let moved = context(&f);
        assert_eq!(
            unsafe { viem_core_view_insert_table(f.handle, f.view, &valid, &mut out) },
            ViemStatus::StaleRevision
        );
        assert_eq!(context(&f), moved);
        assert_eq!(source(&f), b"alpha beta");
        let current = ViemInsertTableV1 {
            expected_selection: moved.selection,
            ..valid
        };
        assert_eq!(
            unsafe { viem_core_view_insert_table(f.handle, f.view, &current, &mut out) },
            ViemStatus::Ok
        );
        assert_ne!(source(&f), b"alpha beta");
    }
}

#[test]
fn exact_context_actions_validate_fields_and_header_constraints() {
    let f = table_ffi_fixture(TABLE, VIEM_FORMAT_MARKDOWN);
    let original = context(&f);
    assert_eq!(
        (
            original.rows,
            original.columns,
            original.row,
            original.column
        ),
        (3, 3, 0, 0)
    );
    assert_eq!(original.flags, VIEM_TABLE_IN_TABLE | VIEM_TABLE_HEADER);
    assert_eq!(original.alignment, VIEM_TABLE_ALIGN_LEFT);
    let mut out = ViemCoreOutcomeV1::default();
    let valid = action(original, VIEM_TABLE_INSERT_ROW_BELOW);
    for invalid in [
        ViemTableActionV1 {
            struct_size: 0,
            ..valid
        },
        ViemTableActionV1 {
            reserved: 1,
            ..valid
        },
        ViemTableActionV1 {
            action: 99,
            ..valid
        },
        ViemTableActionV1 {
            alignment: 1,
            ..valid
        },
        ViemTableActionV1 {
            expected: ViemTableContextV1 {
                flags: original.flags | 128,
                ..original
            },
            ..valid
        },
    ] {
        assert_eq!(
            unsafe { viem_core_view_table_action(f.handle, f.view, &invalid, &mut out) },
            ViemStatus::InvalidArgument
        );
        assert_eq!(source(&f), TABLE);
        assert_eq!(context(&f), original);
    }
    for wrong in [
        ViemTableContextV1 {
            rows: 9,
            ..original
        },
        ViemTableContextV1 {
            table_id: u64::MAX,
            ..original
        },
        ViemTableContextV1 {
            alignment: VIEM_TABLE_ALIGN_CENTER,
            ..original
        },
        ViemTableContextV1 {
            flags: VIEM_TABLE_IN_TABLE,
            ..original
        },
    ] {
        assert_eq!(
            unsafe {
                viem_core_view_table_action(
                    f.handle,
                    f.view,
                    &action(wrong, VIEM_TABLE_CLEAR_CELL),
                    &mut out,
                )
            },
            ViemStatus::StaleRevision
        );
        assert_eq!(source(&f), TABLE);
    }
    assert_eq!(
        unsafe {
            viem_core_view_table_action(
                f.handle,
                f.view,
                &action(original, VIEM_TABLE_INSERT_ROW_ABOVE),
                &mut out,
            )
        },
        ViemStatus::UnsupportedOperation
    );
    assert_eq!(
        unsafe { viem_core_view_table_action(f.handle, f.view, &valid, &mut out) },
        ViemStatus::Ok
    );
    assert_eq!(context(&f).rows, 4);
    let after = source(&f);
    assert_eq!(
        unsafe { viem_core_view_table_action(f.handle, f.view, &valid, &mut out) },
        ViemStatus::StaleRevision
    );
    assert_eq!(source(&f), after);
}

#[test]
fn geometry_is_snapshot_bound_and_short_or_aliased_buffers_are_untouched() {
    let f = table_ffi_fixture(TABLE, VIEM_FORMAT_MARKDOWN);
    let identity = info(&f).identity;
    let all = cells(&f);
    assert_eq!(all.len(), 9);
    assert!(all
        .iter()
        .all(|c| c.rect.width > 0.0 && c.rect.height > 0.0));
    assert_eq!(all[0].rect.y, all[2].rect.y);
    assert_eq!(all[0].table_rect, all[8].table_rect);
    let sentinel = ViemTableCellV1 {
        cell_id: u64::MAX,
        ..Default::default()
    };
    let mut small = [sentinel; 1];
    let mut required = 99;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(
                f.handle,
                f.view,
                &identity,
                small.as_mut_ptr(),
                1,
                &mut required,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(required, 9);
    assert_eq!(small, [sentinel]);
    let stale = ViemLayoutSnapshotIdentityV1 {
        layout_revision: identity.layout_revision + 1,
        ..identity
    };
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(
                f.handle,
                f.view,
                &stale,
                small.as_mut_ptr(),
                1,
                &mut required,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(required, 0);
    assert_eq!(small, [sentinel]);
    let mut backing = [0u64; 128];
    let p = backing.as_mut_ptr();
    unsafe {
        (p as *mut ViemLayoutSnapshotIdentityV1).write(identity);
    }
    let saved = backing;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(f.handle, f.view, p.cast(), p.cast(), 9, &mut required)
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(backing, saved);
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_cells(f.handle, f.view, p.cast(), ptr::null_mut(), 0, p)
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(backing, saved);
    assert_eq!(
        unsafe { viem_core_view_copy_table_cells(f.handle, f.view, &identity, p.cast(), 9, p) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(backing, saved);
}

#[test]
fn rectangle_selection_exports_exact_text_and_all_cell_ranges() {
    let f = table_ffi_fixture(TABLE, VIEM_FORMAT_MARKDOWN);
    let request = select(&f);
    let mut out = ViemCoreOutcomeV1::default();
    assert_eq!(
        unsafe { viem_core_view_select_table_cells(f.handle, f.view, &request, &mut out) },
        ViemStatus::Ok
    );
    let mut actual = ViemTableSelectionV1::default();
    assert_eq!(
        unsafe { viem_core_view_table_selection(f.handle, f.view, &mut actual) },
        ViemStatus::Ok
    );
    assert_eq!(actual, request);
    assert_eq!(
        context(&f).selection.kind,
        VIEM_LOGICAL_SELECTION_KIND_CELLS
    );
    let all = cells(&f);
    assert_eq!(
        all.iter()
            .filter(|c| c.flags & VIEM_TABLE_CELL_SELECTED != 0)
            .count(),
        6
    );
    let mut count = 0;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_text(
                f.handle,
                f.view,
                &actual,
                ptr::null_mut(),
                0,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    let mut text = vec![0; count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_text(
                f.handle,
                f.view,
                &actual,
                text.as_mut_ptr(),
                count,
                &mut count,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(text, b"B\tC\ntwo\tthree\nfive\tsix");
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_ranges(
                f.handle,
                f.view,
                &actual,
                ptr::null_mut(),
                0,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(count, 6);
    let mut ranges = vec![ViemFormattedUtf8RangeV1::default(); count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_ranges(
                f.handle,
                f.view,
                &actual,
                ranges.as_mut_ptr(),
                count,
                &mut count,
            )
        },
        ViemStatus::Ok
    );
    for (range, cell) in ranges.iter().zip(all.iter().filter(|c| c.column > 0)) {
        assert_eq!(
            (range.utf8_start, range.utf8_end),
            (cell.text_start, cell.text_end)
        );
    }
    for invalid in [
        ViemTableSelectionV1 {
            active: 2,
            ..actual
        },
        ViemTableSelectionV1 {
            struct_size: 0,
            ..actual
        },
    ] {
        assert_eq!(
            unsafe { viem_core_view_select_table_cells(f.handle, f.view, &invalid, &mut out) },
            ViemStatus::InvalidArgument
        );
    }
    let stale = ViemTableSelectionV1 {
        active_column: 0,
        ..actual
    };
    let mut sentinel = [0xff; 128];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_text(
                f.handle,
                f.view,
                &stale,
                sentinel.as_mut_ptr(),
                128,
                &mut count,
            )
        },
        ViemStatus::StaleRevision
    );
    assert!(sentinel.iter().all(|byte| *byte == 0xff));
    let invalid = ViemTableSelectionV1 {
        active_row: 999,
        ..actual
    };
    assert_ne!(
        unsafe { viem_core_view_select_table_cells(f.handle, f.view, &invalid, &mut out) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_view_table_selection(f.handle, f.view, &mut actual) },
        ViemStatus::Ok
    );
    assert_eq!(actual, request);
    assert_eq!(source(&f), TABLE);
}

#[test]
fn null_misaligned_and_overlapping_requests_fail_before_mutation() {
    let f = table_ffi_fixture(b"text", VIEM_FORMAT_MARKDOWN);
    let original = context(&f);
    assert_eq!(
        unsafe { viem_core_view_table_context(f.handle, f.view, ptr::null_mut()) },
        ViemStatus::NullPointer
    );
    let mut backing = [0u64; 128];
    let p = backing.as_mut_ptr();
    assert_eq!(
        unsafe { viem_core_view_table_context(f.handle, f.view, p.cast::<u8>().add(1).cast()) },
        ViemStatus::InvalidArgument
    );
    let request = ViemInsertTableV1 {
        struct_size: VIEM_INSERT_TABLE_V1_SIZE,
        expected_selection: original.selection,
        columns: 3,
        body_rows: 4,
        ..Default::default()
    };
    unsafe {
        (p as *mut ViemInsertTableV1).write(request);
    }
    let saved = backing;
    assert_eq!(
        unsafe { viem_core_view_insert_table(f.handle, f.view, p.cast(), p.cast()) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(backing, saved);
    assert_eq!(context(&f), original);
    let mut out = original;
    assert_eq!(
        unsafe {
            viem_core_view_table_context_at(
                f.handle,
                f.view,
                original.selection.document_id,
                original.selection.document_revision + 1,
                0,
                &mut out,
            )
        },
        ViemStatus::StaleRevision
    );
    assert_eq!(out, ViemTableContextV1::default());
    assert_eq!(source(&f), b"text");
}

#[test]
fn selected_ranges_include_offscreen_cells_and_failed_copies_do_not_write() {
    let mut text = String::from("| A | B |\n| --- | --- |\n");
    for row in 0..350 {
        text.push_str(&format!("| row {row} | value {row} |\n"));
    }
    let f = table_ffi_fixture(text.as_bytes(), VIEM_FORMAT_MARKDOWN);
    let c = context(&f);
    let selection = ViemTableSelectionV1 {
        struct_size: VIEM_TABLE_SELECTION_V1_SIZE,
        active: 1,
        document_id: c.selection.document_id,
        document_revision: c.selection.document_revision,
        table_id: c.table_id,
        anchor_row: 0,
        anchor_column: 1,
        active_row: 350,
        active_column: 1,
    };
    let mut out = ViemCoreOutcomeV1::default();
    assert_eq!(
        unsafe { viem_core_view_select_table_cells(f.handle, f.view, &selection, &mut out) },
        ViemStatus::Ok
    );
    let mut count = 0;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_ranges(
                f.handle,
                f.view,
                &selection,
                ptr::null_mut(),
                0,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(count, 351);
    let sentinel = ViemFormattedUtf8RangeV1 {
        utf8_start: u64::MAX,
        ..Default::default()
    };
    let mut short = [sentinel; 2];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_ranges(
                f.handle,
                f.view,
                &selection,
                short.as_mut_ptr(),
                2,
                &mut count,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(short, [sentinel; 2]);
    let mut ranges = vec![ViemFormattedUtf8RangeV1::default(); count as usize];
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_ranges(
                f.handle,
                f.view,
                &selection,
                ranges.as_mut_ptr(),
                count,
                &mut count,
            )
        },
        ViemStatus::Ok
    );
    assert!(ranges
        .windows(2)
        .all(|pair| pair[0].utf8_end < pair[1].utf8_start));
    let mut aliased = selection;
    let p = &mut aliased as *mut ViemTableSelectionV1;
    assert_eq!(
        unsafe {
            viem_core_view_copy_table_selection_text(
                f.handle,
                f.view,
                p,
                ptr::null_mut(),
                0,
                p.cast(),
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(aliased, selection);
    assert_eq!(source(&f), text.as_bytes());
}
