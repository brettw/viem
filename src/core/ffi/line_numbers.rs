//! Batch physical source-line geometry for native gutter presentation.
use super::*;

pub const VIEM_LINE_NUMBER_SNAPSHOT_ENABLED: u32 = 1;
pub const VIEM_LINE_NUMBER_REGION_ESTIMATED: u32 = 1;
/// The displayed representative owns the visual row containing the caret.
pub const VIEM_LINE_NUMBER_REGION_CURRENT: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLineNumberSnapshotInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub identity: ViemLayoutSnapshotIdentityV1,
    pub viewport: ViemLayoutRectV1,
    pub source_line_count: u64,
    pub current_source_line: u64,
    pub region_count: u64,
}
pub const VIEM_LINE_NUMBER_SNAPSHOT_INFO_V1_SIZE: u32 =
    size_of::<ViemLineNumberSnapshotInfoV1>() as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViemLineNumberRegionV1 {
    pub source_line: u64,
    pub flags: u32,
    pub reserved: u32,
    pub top: f32,
    pub bottom: f32,
}

#[no_mangle]
pub unsafe extern "C" fn viem_core_view_number(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_enabled: *mut u32,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_enabled, 1)?;
        unsafe { out_enabled.write(0) };
        let value = with_core(handle, |core| {
            core.number(ViewId(view)).map_err(core_status)
        })?;
        unsafe { out_enabled.write(u32::from(value)) };
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn viem_core_view_set_number(
    handle: ViemCoreHandle,
    view: ViemViewId,
    enabled: u32,
) -> ViemStatus {
    ffi_boundary(|| {
        if enabled > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            core.set_number(ViewId(view), enabled != 0)
                .map_err(core_status)
        })
    })
}

/// Begin or extend a gutter gesture using exact current layout geometry.
/// The core retains the original whole row as stable selection anchors.
///
/// # Safety
/// `expected` and `out_outcome` identify distinct, aligned readable/writable
/// values. `y` is a document-layout coordinate; `extend_selection` is 0 or 1.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_select_pointer_row(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    y: f32,
    extend_selection: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(expected, 1)?, typed_pointer_region(out_outcome, 1)?])?;
        let expected = unsafe { read_layout_identity(expected)? };
        if !y.is_finite() || extend_selection > 1 { return Err(ViemStatus::InvalidArgument); }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            validate_snapshot_identity(expected, current_ffi_layout_snapshot(core, ViewId(view))?, ViewId(view))?;
            dispatch_event(core, view, CoreEvent::SelectPointerRow {
                document_revision: Revision(expected.document_revision),
                layout_revision: crate::layout::LayoutRevision(expected.layout_revision),
                y, extend_selection: extend_selection != 0,
            })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

fn viewport(
    core: &Core<CTextMeasurementProvider>,
    id: ViewId,
) -> Result<ViemLayoutRectV1, ViemStatus> {
    let layout = core
        .presentation_layout(id)
        .ok_or(ViemStatus::InvalidView)?;
    Ok(ViemLayoutRectV1 {
        x: layout.viewport_left(),
        y: layout.viewport_top(),
        width: layout.width(),
        height: layout.height(),
    })
}

#[no_mangle]
pub unsafe extern "C" fn viem_core_view_line_number_snapshot_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemLineNumberSnapshotInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemLineNumberSnapshotInfoV1::default()) };
        let info = with_core(handle, |core| {
            let id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, id)?;
            Ok(ViemLineNumberSnapshotInfoV1 {
                struct_size: VIEM_LINE_NUMBER_SNAPSHOT_INFO_V1_SIZE,
                flags: if core.number(id).map_err(core_status)? {
                    VIEM_LINE_NUMBER_SNAPSHOT_ENABLED
                } else {
                    0
                },
                identity: snapshot_identity(snapshot, id),
                viewport: viewport(core, id)?,
                source_line_count: checked_export_count(
                    core.document().physical_line_count().map_err(|error| core_status(error.into()))?,
                )?,
                current_source_line: checked_export_count(
                    core.current_physical_source_line(id).map_err(core_status)?,
                )?,
                region_count: checked_export_count(
                    core.line_number_regions(id).map_err(core_status)?.len(),
                )?,
            })
        })?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_line_number_regions(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    expected_viewport: *const ViemLayoutRectV1,
    output: *mut ViemLineNumberRegionV1,
    capacity: u64,
    out_count: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(expected_viewport, 1)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(out_count, 1)?,
        ])?;
        let expected = unsafe { read_layout_identity(expected)? };
        let expected_viewport = unsafe { expected_viewport.read() };
        unsafe { out_count.write(0) };
        let regions = with_core(handle, |core| {
            let id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, id)?;
            validate_snapshot_identity(expected, snapshot, id)?;
            if expected_viewport != viewport(core, id)? {
                return Err(ViemStatus::StaleRevision);
            }
            let regions = core.line_number_regions(id).map_err(core_status)?;
            regions
                .into_iter()
                .map(|region| {
                    Ok(ViemLineNumberRegionV1 {
                        source_line: checked_export_count(region.source_line)?,
                        flags: (if region.estimated { VIEM_LINE_NUMBER_REGION_ESTIMATED } else { 0 })
                            | (if region.current { VIEM_LINE_NUMBER_REGION_CURRENT } else { 0 }),
                        reserved: 0,
                        top: region.top,
                        bottom: region.bottom,
                    })
                })
                .collect::<Result<Vec<_>, ViemStatus>>()
        })?;
        unsafe { out_count.write(checked_export_count(regions.len())?) };
        if capacity < regions.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe { copy_output(&regions, output) };
        Ok(())
    })
}
