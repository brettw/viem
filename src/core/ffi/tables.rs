//! Exact-revision table actions and snapshot-bound cell geometry.
use super::*;
use crate::document::{TableAlignment, TableEditIntent};

pub const VIEM_TABLE_CAN_INSERT: u32 = 1;
pub const VIEM_TABLE_IN_TABLE: u32 = 2;
pub const VIEM_TABLE_HEADER: u32 = 4;
pub const VIEM_TABLE_CELL_SELECTED: u32 = 8;
pub const VIEM_TABLE_ALIGN_UNSPECIFIED: u32 = 0;
pub const VIEM_TABLE_ALIGN_LEFT: u32 = 1;
pub const VIEM_TABLE_ALIGN_CENTER: u32 = 2;
pub const VIEM_TABLE_ALIGN_RIGHT: u32 = 3;
pub const VIEM_TABLE_INSERT_ROW_ABOVE: u32 = 1;
pub const VIEM_TABLE_INSERT_ROW_BELOW: u32 = 2;
pub const VIEM_TABLE_DELETE_ROW: u32 = 3;
pub const VIEM_TABLE_INSERT_COLUMN_LEFT: u32 = 4;
pub const VIEM_TABLE_INSERT_COLUMN_RIGHT: u32 = 5;
pub const VIEM_TABLE_DELETE_COLUMN: u32 = 6;
pub const VIEM_TABLE_SET_ALIGNMENT: u32 = 7;
pub const VIEM_TABLE_CLEAR_CELL: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemTableContextV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub selection: ViemLogicalSelectionIdentityV1,
    pub table_id: u64,
    pub row: u64,
    pub column: u64,
    pub rows: u64,
    pub columns: u64,
    pub alignment: u32,
    pub reserved: u32,
}
pub const VIEM_TABLE_CONTEXT_V1_SIZE: u32 = size_of::<ViemTableContextV1>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemInsertTableV1 {
    pub struct_size: u32,
    pub reserved: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
    pub columns: u32,
    pub body_rows: u32,
}
pub const VIEM_INSERT_TABLE_V1_SIZE: u32 = size_of::<ViemInsertTableV1>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemTableActionV1 {
    pub struct_size: u32,
    pub action: u32,
    pub expected: ViemTableContextV1,
    pub alignment: u32,
    pub reserved: u32,
}
pub const VIEM_TABLE_ACTION_V1_SIZE: u32 = size_of::<ViemTableActionV1>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemTableCellV1 {
    pub table_id: u64,
    pub row: u64,
    pub column: u64,
    pub cell_id: u64,
    pub text_start: u64,
    pub text_end: u64,
    pub rect: ViemLayoutRectV1,
    pub table_rect: ViemLayoutRectV1,
    pub alignment: u32,
    pub flags: u32,
}
pub const VIEM_TABLE_CELL_V1_SIZE: u32 = size_of::<ViemTableCellV1>() as u32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemTableSelectionV1 {
    pub struct_size: u32,
    pub active: u32,
    pub document_id: u64,
    pub document_revision: u64,
    pub table_id: u64,
    pub anchor_row: u64,
    pub anchor_column: u64,
    pub active_row: u64,
    pub active_column: u64,
}
pub const VIEM_TABLE_SELECTION_V1_SIZE: u32 = size_of::<ViemTableSelectionV1>() as u32;

fn alignment_to_ffi(value: TableAlignment) -> u32 {
    match value {
        TableAlignment::Unspecified => 0,
        TableAlignment::Left => 1,
        TableAlignment::Center => 2,
        TableAlignment::Right => 3,
    }
}
fn alignment_from_ffi(value: u32) -> Result<TableAlignment, ViemStatus> {
    Ok(match value {
        0 => TableAlignment::Unspecified,
        1 => TableAlignment::Left,
        2 => TableAlignment::Center,
        3 => TableAlignment::Right,
        _ => return Err(ViemStatus::InvalidArgument),
    })
}

fn context_at(
    core: &Core<CTextMeasurementProvider>,
    view: ViewId,
    offset: usize,
) -> Result<ViemTableContextV1, ViemStatus> {
    let selection = core.list_selection_identity(view).map_err(core_status)?;
    core.document()
        .text_point(offset)
        .map_err(document_status)?;
    let mut result = ViemTableContextV1 {
        struct_size: VIEM_TABLE_CONTEXT_V1_SIZE,
        selection: logical_selection_identity_to_ffi(&selection)?,
        ..Default::default()
    };
    if let Some((table, row, _cell)) = core.document().projection().table_cell_at(offset) {
        result.flags = VIEM_TABLE_IN_TABLE;
        result.table_id = table.id;
        result.row = table
            .rows
            .partition_point(|value| value.range.start <= offset)
            .checked_sub(1)
            .ok_or(ViemStatus::CoreFailure)? as u64;
        result.column = row
            .cells
            .partition_point(|value| value.range.start <= offset)
            .checked_sub(1)
            .ok_or(ViemStatus::CoreFailure)? as u64;
        result.rows = table.rows.len() as u64;
        result.columns = table.columns.len() as u64;
        result.alignment = alignment_to_ffi(table.columns[result.column as usize]);
        if result.row == 0 {
            result.flags |= VIEM_TABLE_HEADER;
        }
    } else if selection.kind() != LogicalSelectionKind::Cells
        && core.document().can_insert_table(selection.range())
    {
        result.flags = VIEM_TABLE_CAN_INSERT;
    }
    Ok(result)
}

/// # Safety
/// `output` must be writable and aligned for one context.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_table_context(
    handle: ViemCoreHandle,
    view: ViemViewId,
    output: *mut ViemTableContextV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(output, 1)?;
        unsafe { output.write(ViemTableContextV1::default()) };
        let result = with_core(handle, |core| {
            let cursor = core
                .command_state(ViewId(view))
                .ok_or(ViemStatus::InvalidView)?
                .cursor();
            context_at(core, ViewId(view), cursor)
        })?;
        unsafe { output.write(result) };
        Ok(())
    })
}
/// # Safety
/// `output` must be writable and aligned for one context.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_table_context_at(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    offset: u64,
    output: *mut ViemTableContextV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(output, 1)?;
        unsafe { output.write(ViemTableContextV1::default()) };
        let result = with_core(handle, |core| {
            if core.document().id().0 != document || core.document().revision().0 != revision {
                return Err(ViemStatus::StaleRevision);
            }
            context_at(
                core,
                ViewId(view),
                usize::try_from(offset).map_err(|_| ViemStatus::LengthOverflow)?,
            )
        })?;
        unsafe { output.write(result) };
        Ok(())
    })
}
/// # Safety
/// Request and output must be valid, aligned, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_insert_table(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemInsertTableV1,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, output)? };
        if request.struct_size < VIEM_INSERT_TABLE_V1_SIZE
            || request.reserved != 0
            || !(1..=20).contains(&request.columns)
            || !(1..=50).contains(&request.body_rows)
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(output)? };
        let result = with_core_mut(handle, |core| {
            let selection = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected_selection,
                logical_selection_identity_to_ffi(&selection)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::TableEdit {
                    document: selection.document(),
                    revision: selection.revision(),
                    intent: TableEditIntent::Insert {
                        range: selection.range(),
                        columns: request.columns as usize,
                        body_rows: request.body_rows as usize,
                    },
                },
            )
        })?;
        unsafe { output.write(result) };
        Ok(())
    })
}
/// # Safety
/// Request and output must be valid, aligned, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_table_action(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemTableActionV1,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, output)? };
        if request.struct_size < VIEM_TABLE_ACTION_V1_SIZE
            || request.reserved != 0
            || request.expected.struct_size < VIEM_TABLE_CONTEXT_V1_SIZE
            || request.expected.reserved != 0
            || request.expected.flags & !(VIEM_TABLE_IN_TABLE | VIEM_TABLE_HEADER) != 0
            || request.expected.flags & VIEM_TABLE_IN_TABLE == 0
            || request.expected.alignment > VIEM_TABLE_ALIGN_RIGHT
            || (request.action != VIEM_TABLE_SET_ALIGNMENT && request.alignment != 0)
            || request.alignment > VIEM_TABLE_ALIGN_RIGHT
        {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(output)? };
        let result = with_core_mut(handle, |core| {
            let selection = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                request.expected.selection,
                logical_selection_identity_to_ffi(&selection)?,
            )?;
            let table = core
                .document()
                .projection()
                .tables()
                .iter()
                .find(|table| table.id == request.expected.table_id)
                .ok_or(ViemStatus::StaleRevision)?;
            let row =
                usize::try_from(request.expected.row).map_err(|_| ViemStatus::LengthOverflow)?;
            let column =
                usize::try_from(request.expected.column).map_err(|_| ViemStatus::LengthOverflow)?;
            if row >= table.rows.len()
                || column >= table.columns.len()
                || request.expected.rows != table.rows.len() as u64
                || request.expected.columns != table.columns.len() as u64
            {
                return Err(ViemStatus::StaleRevision);
            }
            if request.expected.alignment != alignment_to_ffi(table.columns[column])
                || (request.expected.flags & VIEM_TABLE_HEADER != 0) != (row == 0)
            {
                return Err(ViemStatus::StaleRevision);
            }
            if request.action == VIEM_TABLE_INSERT_ROW_ABOVE && row == 0 {
                return Err(ViemStatus::UnsupportedOperation);
            }
            let table = table.id;
            let intent = match request.action {
                VIEM_TABLE_INSERT_ROW_ABOVE => TableEditIntent::InsertRow {
                    table,
                    row,
                    after: false,
                },
                VIEM_TABLE_INSERT_ROW_BELOW => TableEditIntent::InsertRow {
                    table,
                    row,
                    after: true,
                },
                VIEM_TABLE_DELETE_ROW => TableEditIntent::DeleteRow { table, row },
                VIEM_TABLE_INSERT_COLUMN_LEFT => TableEditIntent::InsertColumn {
                    table,
                    column,
                    after: false,
                },
                VIEM_TABLE_INSERT_COLUMN_RIGHT => TableEditIntent::InsertColumn {
                    table,
                    column,
                    after: true,
                },
                VIEM_TABLE_DELETE_COLUMN => TableEditIntent::DeleteColumn { table, column },
                VIEM_TABLE_SET_ALIGNMENT => TableEditIntent::SetAlignment {
                    table,
                    column,
                    alignment: alignment_from_ffi(request.alignment)?,
                },
                VIEM_TABLE_CLEAR_CELL => TableEditIntent::ClearCells {
                    table,
                    rows: row..row + 1,
                    columns: column..column + 1,
                },
                _ => return Err(ViemStatus::InvalidArgument),
            };
            dispatch_event(
                core,
                view,
                CoreEvent::TableEdit {
                    document: selection.document(),
                    revision: selection.revision(),
                    intent,
                },
            )
        })?;
        unsafe { output.write(result) };
        Ok(())
    })
}
/// # Safety
/// All pointer regions must be valid, aligned, and pairwise disjoint. Null
/// output is allowed only with zero capacity; the required count is always set.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_table_cells(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    output: *mut ViemTableCellV1,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        let expected = unsafe { expected.read() };
        unsafe { required.write(0) };
        with_core(handle, |core| {
            let snapshot = current_ffi_layout_snapshot(core, ViewId(view))?;
            validate_snapshot_identity(expected, snapshot, ViewId(view))?;
            let cells = snapshot.table_cells();
            unsafe { required.write(cells.len() as u64) };
            if capacity < cells.len() as u64 {
                return Err(ViemStatus::BufferTooSmall);
            }
            let selected = core.table_selection(ViewId(view)).map_err(core_status)?;
            let rect = |r: crate::layout::LayoutRect| ViemLayoutRectV1 {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            };
            let table_rects: std::collections::HashMap<_, _> = snapshot
                .tables()
                .iter()
                .map(|table| (table.table_id, table.rect))
                .collect();
            // Validate every dependency before touching caller cell storage. A
            // failed geometry export must not leave a partially current array.
            let mut export = Vec::new();
            export
                .try_reserve_exact(cells.len())
                .map_err(|_| ViemStatus::ResourceExhausted)?;
            for cell in cells {
                let table = core
                    .document()
                    .projection()
                    .table_at(cell.text_range.start)
                    .filter(|table| table.id == cell.table_id)
                    .ok_or(ViemStatus::StaleRevision)?;
                let table_rect = *table_rects
                    .get(&cell.table_id)
                    .ok_or(ViemStatus::StaleRevision)?;
                let alignment = *table
                    .columns
                    .get(cell.column)
                    .ok_or(ViemStatus::StaleRevision)?;
                let mut flags = if cell.row == 0 { VIEM_TABLE_HEADER } else { 0 };
                if selected.as_ref().is_some_and(|s| {
                    s.table == cell.table_id
                        && s.rows().contains(&cell.row)
                        && s.columns().contains(&cell.column)
                }) {
                    flags |= VIEM_TABLE_CELL_SELECTED;
                }
                export.push(ViemTableCellV1 {
                    table_id: cell.table_id,
                    row: cell.row as u64,
                    column: cell.column as u64,
                    cell_id: cell.cell_id,
                    text_start: cell.text_range.start as u64,
                    text_end: cell.text_range.end as u64,
                    rect: rect(cell.rect),
                    table_rect: rect(table_rect),
                    alignment: alignment_to_ffi(alignment),
                    flags,
                });
            }
            if !export.is_empty() {
                unsafe { std::ptr::copy_nonoverlapping(export.as_ptr(), output, export.len()) };
            }
            Ok(())
        })
    })
}
/// # Safety
/// Output must be valid writable aligned storage.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_table_selection(
    handle: ViemCoreHandle,
    view: ViemViewId,
    output: *mut ViemTableSelectionV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(output, 1)?;
        unsafe { output.write(ViemTableSelectionV1::default()) };
        let value = with_core(handle, |core| {
            let mut value = ViemTableSelectionV1 {
                struct_size: VIEM_TABLE_SELECTION_V1_SIZE,
                document_id: core.document().id().0,
                document_revision: core.document().revision().0,
                ..Default::default()
            };
            if let Some(s) = core.table_selection(ViewId(view)).map_err(core_status)? {
                value.active = 1;
                value.table_id = s.table;
                value.anchor_row = s.anchor_row as u64;
                value.anchor_column = s.anchor_column as u64;
                value.active_row = s.active_row as u64;
                value.active_column = s.active_column as u64;
            }
            Ok(value)
        })?;
        unsafe { output.write(value) };
        Ok(())
    })
}
/// # Safety
/// Request and output must be valid, aligned, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_select_table_cells(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemTableSelectionV1,
    output: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, output)? };
        if request.struct_size < VIEM_TABLE_SELECTION_V1_SIZE || request.active > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(output)? };
        let value = with_core_mut(handle, |core| {
            let checked = |value| usize::try_from(value).map_err(|_| ViemStatus::LengthOverflow);
            let selected = if request.active == 0 {
                None
            } else {
                Some(crate::command::TableSelectionExtent {
                    table: request.table_id,
                    anchor_row: checked(request.anchor_row)?,
                    anchor_column: checked(request.anchor_column)?,
                    active_row: checked(request.active_row)?,
                    active_column: checked(request.active_column)?,
                })
            };
            let outcome = core
                .select_table_cells(
                    ViewId(view),
                    DocumentId(request.document_id),
                    Revision(request.document_revision),
                    selected,
                )
                .map_err(core_status)?;
            summarize_core_outcome(core, ViewId(view), Some(&outcome))
        })?;
        unsafe { output.write(value) };
        Ok(())
    })
}

fn checked_table_selection(
    core: &Core<CTextMeasurementProvider>,
    view: ViewId,
    expected: ViemTableSelectionV1,
) -> Result<crate::command::TableSelectionExtent, ViemStatus> {
    if expected.struct_size < VIEM_TABLE_SELECTION_V1_SIZE || expected.active != 1 {
        return Err(ViemStatus::InvalidArgument);
    }
    if expected.document_id != core.document().id().0
        || expected.document_revision != core.document().revision().0
    {
        return Err(ViemStatus::StaleRevision);
    }
    let actual = core
        .table_selection(view)
        .map_err(core_status)?
        .ok_or(ViemStatus::StaleRevision)?;
    if actual.table != expected.table_id
        || actual.anchor_row as u64 != expected.anchor_row
        || actual.anchor_column as u64 != expected.anchor_column
        || actual.active_row as u64 != expected.active_row
        || actual.active_column as u64 != expected.active_column
    {
        return Err(ViemStatus::StaleRevision);
    }
    Ok(actual)
}

/// Read the complete selected matrix as TSV without changing clipboard/registers.
/// # Safety
/// Pointer regions must be valid, aligned, and pairwise disjoint. Null output
/// is valid only with zero capacity. No output bytes are written on failure.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_table_selection_text(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemTableSelectionV1,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        let expected = unsafe { expected.read() };
        unsafe { required.write(0) };
        with_core(handle, |core| {
            let selection = checked_table_selection(core, ViewId(view), expected)?;
            let (_, text) = core
                .document()
                .table_clipboard_fragment(selection.table, selection.rows(), selection.columns())
                .map_err(document_status)?;
            unsafe { required.write(text.len() as u64) };
            if capacity < text.len() as u64 {
                return Err(ViemStatus::BufferTooSmall);
            }
            if !text.is_empty() {
                unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), output, text.len()) };
            }
            Ok(())
        })
    })
}

/// Read every selected cell's logical range, in row-major order. This includes
/// offscreen and empty cells and never expands the selection to a text hull.
/// # Safety
/// Pointer regions must be valid, aligned, and pairwise disjoint. Null output
/// is valid only with zero capacity. No output ranges are written on failure.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_table_selection_ranges(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemTableSelectionV1,
    output: *mut ViemFormattedUtf8RangeV1,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        let expected = unsafe { expected.read() };
        unsafe { required.write(0) };
        with_core(handle, |core| {
            let selection = checked_table_selection(core, ViewId(view), expected)?;
            let table = core
                .document()
                .projection()
                .tables()
                .iter()
                .find(|table| table.id == selection.table)
                .ok_or(ViemStatus::StaleRevision)?;
            let count = selection
                .rows()
                .len()
                .checked_mul(selection.columns().len())
                .ok_or(ViemStatus::LengthOverflow)?;
            unsafe { required.write(count as u64) };
            if capacity < count as u64 {
                return Err(ViemStatus::BufferTooSmall);
            }
            let identity = formatted_snapshot_identity(core.document());
            let mut index = 0;
            for row in selection.rows() {
                for column in selection.columns() {
                    let range = &table.rows[row].cells[column].range;
                    unsafe {
                        output.add(index).write(ViemFormattedUtf8RangeV1 {
                            struct_size: VIEM_FORMATTED_UTF8_RANGE_V1_SIZE,
                            reserved: 0,
                            identity,
                            utf8_start: range.start as u64,
                            utf8_end: range.end as u64,
                        });
                    }
                    index += 1;
                }
            }
            Ok(())
        })
    })
}
