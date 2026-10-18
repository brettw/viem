//! Native lifecycle event bridge. Autocommands run on the buffer owner and
//! return ordinary effects; native I/O remains outside this call.
use super::*;

/// Fire a supported lifecycle event. An empty matched name uses the current
/// buffer filename. The host calls write-pre before capturing save bytes and
/// write-post only after a successful save.
/// # Safety
/// All input and output regions must be valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_fire_autocmd(
    handle: ViemCoreHandle, view: ViemViewId,
    event: *const u8, event_length: u64, matched: *const u8, matched_length: u64,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1, out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [typed_pointer_region(event, event_length)?, typed_pointer_region(matched, matched_length)?,
            typed_pointer_region(out_outcome, 1)?, typed_pointer_region(out_effects, 1)?];
        validate_disjoint_regions(&regions)?;
        let clipboard = unsafe { read_command_turn_context(context, &regions)? };
        let event = str::from_utf8(unsafe { input_bytes(event, event_length)? }).map_err(|_| ViemStatus::InvalidUtf8)?;
        let matched = str::from_utf8(unsafe { input_bytes(matched, matched_length)? }).map_err(|_| ViemStatus::InvalidUtf8)?;
        if !crate::vimscript::autocmd::event_index(event).is_some_and(crate::vimscript::autocmd::is_fired) {
            return Err(ViemStatus::InvalidArgument);
        }
        unsafe { clear_outcome(out_outcome)?; out_effects.write(0); }
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            let view = ViewId(view);
            let outcome = core.fire_autocmd_with_clipboard(view, event, (!matched.is_empty()).then_some(matched), Some(clipboard.clone())).map_err(core_status)?;
            let mut summary = summarize_core_outcome(core, view, Some(&outcome))?;
            let effects = OwnedEffectBatch::from_command(core.document(), core.command_state(view).ok_or(ViemStatus::InvalidView)?, &clipboard, outcome.command);
            if effects.is_some() { summary.flags |= VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS; }
            Ok((summary, effects))
        })?;
        let effects = match effects { Some(effects) => reservation.commit(effects)?, None => 0 };
        unsafe { out_outcome.write(summary); out_effects.write(effects); }
        Ok(())
    })
}
