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
