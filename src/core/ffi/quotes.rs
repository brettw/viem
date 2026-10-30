//! Structural quote toggling preserves the contained paragraph treatments.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemSetBlockQuoteV1 {
    pub struct_size: u32,
    pub enabled: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}
pub const VIEM_SET_BLOCK_QUOTE_V1_SIZE: u32 = size_of::<ViemSetBlockQuoteV1>() as u32;

/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_block_quote(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetBlockQuoteV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        if request.struct_size < VIEM_SET_BLOCK_QUOTE_V1_SIZE || request.enabled > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core.list_selection_identity(ViewId(view)).map_err(core_status)?;
            validate_logical_selection_identity(request.expected_selection, logical_selection_identity_to_ffi(&expected)?)?;
            dispatch_event(core, view, CoreEvent::SetBlockQuote { expected, enabled: request.enabled != 0 })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

pub const VIEM_ALERT_NONE: u32 = 0;
pub const VIEM_ALERT_NOTE: u32 = 1;
pub const VIEM_ALERT_TIP: u32 = 2;
pub const VIEM_ALERT_IMPORTANT: u32 = 3;
pub const VIEM_ALERT_WARNING: u32 = 4;
pub const VIEM_ALERT_CAUTION: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemSetAlertV1 {
    pub struct_size: u32,
    /// A `VIEM_ALERT_*` kind; `VIEM_ALERT_NONE` makes a plain quote.
    pub kind: u32,
    pub expected_selection: ViemLogicalSelectionIdentityV1,
}
pub const VIEM_SET_ALERT_V1_SIZE: u32 = size_of::<ViemSetAlertV1>() as u32;

/// Make the exact selection a GitHub alert, quoting unquoted blocks, or turn
/// its alerts back into plain quotes. Visible text never changes.
///
/// # Safety
/// Request and outcome must be distinct aligned readable/writable values.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_alert(
    handle: ViemCoreHandle,
    view: ViemViewId,
    request: *const ViemSetAlertV1,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        let request = unsafe { read_core_request(request, out_outcome)? };
        let kind = crate::document::AlertKind::from_ordinal(request.kind);
        if request.struct_size < VIEM_SET_ALERT_V1_SIZE || (request.kind != VIEM_ALERT_NONE && kind.is_none()) {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)? };
        let outcome = with_core_mut(handle, |core| {
            let expected = core.list_selection_identity(ViewId(view)).map_err(core_status)?;
            validate_logical_selection_identity(request.expected_selection, logical_selection_identity_to_ffi(&expected)?)?;
            dispatch_event(core, view, CoreEvent::SetAlert { expected, kind })
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}
