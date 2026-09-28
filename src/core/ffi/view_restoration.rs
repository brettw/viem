//! Location recovery is shared by native document replacement adapters.
use super::*;

/// Presentation hints for an explicit replacement; never a snapshot edit point.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemViewRestorationV1 {
    pub struct_size: u32,
    pub cursor_affinity: u32,
    pub cursor_line: u64,
    pub cursor_column: u64,
    pub viewport_line: u64,
    pub viewport_column: u64,
    pub row_fraction: f32,
    pub left: f32,
}
pub const VIEM_VIEW_RESTORATION_V1_SIZE: u32 = size_of::<ViemViewRestorationV1>() as u32;

/// Capture a view before replacing the document.
/// # Safety
/// `out_state` must identify one aligned writable value.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_capture_restoration(handle: ViemCoreHandle,
    view: ViemViewId, out_state: *mut ViemViewRestorationV1) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_state, 1)?;
        unsafe { out_state.write(ViemViewRestorationV1::default()) };
        let state = with_core(handle, |core| core.capture_view_restoration(ViewId(view)).map_err(core_status))?;
        unsafe { out_state.write(ViemViewRestorationV1 {
            struct_size: VIEM_VIEW_RESTORATION_V1_SIZE,
            cursor_affinity: affinity_to_ffi(state.cursor_affinity),
            cursor_line: state.cursor.0 as u64, cursor_column: state.cursor.1 as u64,
            viewport_line: state.viewport.0 as u64, viewport_column: state.viewport.1 as u64,
            row_fraction: state.row_fraction, left: state.left,
        }) };
        Ok(())
    })
}

/// Recover positions in an explicitly identified replacement document. The
/// frontend publishes the prepared replacement only after this succeeds.
/// # Safety
/// Input and output must be aligned, valid for their sizes, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_restore(handle: ViemCoreHandle, view: ViemViewId,
    document: u64, revision: u64, state: *const ViemViewRestorationV1,
    out_outcome: *mut ViemCoreOutcomeV1) -> ViemStatus {
    ffi_boundary(|| {
        let state = unsafe { read_core_request(state, out_outcome)? };
        if state.struct_size < VIEM_VIEW_RESTORATION_V1_SIZE { return Err(ViemStatus::InvalidArgument); }
        let coordinate = |value| usize::try_from(value).map_err(|_| ViemStatus::LengthOverflow);
        let state = crate::coordinator::ViewRestoration {
            cursor: (coordinate(state.cursor_line)?, coordinate(state.cursor_column)?),
            cursor_affinity: parse_layout_affinity(state.cursor_affinity)?,
            viewport: (coordinate(state.viewport_line)?, coordinate(state.viewport_column)?),
            row_fraction: state.row_fraction, left: state.left,
        };
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            core.restore_view(ViewId(view), DocumentId(document), Revision(revision), state).map_err(core_status)?;
            summarize_core_outcome(core, ViewId(view), None)
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}
