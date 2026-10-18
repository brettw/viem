//! Revision-bound host file continuations. Host I/O never runs under a core lease.
use super::*;

/// Insert host-loaded file bytes at a revision-bound source byte boundary
/// (`source = 1`) or formatted byte boundary (`source = 0`). Decoding and
/// linewise edit semantics are owned by core.
/// # Safety
/// Input/output regions must be valid for their sizes and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_read_file(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
    source: u32,
    after: u64,
    input: *const u8,
    length: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(input, length)?,
            typed_pointer_region(out_outcome, 1)?,
        ])?;
        let bytes = unsafe { input_bytes(input, length)? }.to_vec();
        unsafe { clear_outcome(out_outcome)? };
        if source > 1 { return Err(ViemStatus::InvalidArgument); }
        let after = crate::command::ex_execute::ExInsertionPoint {
            offset: usize::try_from(after).map_err(|_| ViemStatus::LengthOverflow)?, source: source != 0,
        };
        let outcome = with_core_mut(handle, |core| {
            dispatch_event(
                core,
                view,
                CoreEvent::ReadFile {
                    document: DocumentId(document),
                    revision: Revision(revision),
                    after,
                    bytes,
                },
            )
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Run one command line as if typed after `:`, through the ordinary command
/// pipeline: it is Vim script, so `|`-separated commands, `:let`, `:call`
/// and `:source` all work. The view must be in Normal mode.
/// # Safety
/// Input/context and disjoint outputs obey the ordinary command-turn contract.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_execute_command_line(
    handle: ViemCoreHandle,
    view: ViemViewId,
    input: *const u8,
    length: u64,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(input, length)?,
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effects, 1)?,
        ];
        validate_disjoint_regions(&regions)?;
        let clipboard = unsafe { read_command_turn_context(context, &regions)? };
        let text = str::from_utf8(unsafe { input_bytes(input, length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?;
        unsafe {
            clear_outcome(out_outcome)?;
            out_effects.write(0);
        }
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            core.stage_command_line(ViewId(view), text).map_err(|_| ViemStatus::InvalidArgument)?;
            dispatch_input_with_effects(core, view, InputEvent::Key(Key::Enter), clipboard)
        })?;
        let effects = match effects {
            Some(effects) => reservation.commit(effects)?,
            None => 0,
        };
        unsafe {
            out_outcome.write(summary);
            out_effects.write(effects);
        }
        Ok(())
    })
}

/// Replace the theme names `:colorscheme` matches (ignoring case). `names`
/// holds UTF-8 names separated by NUL bytes; the frontend calls this at
/// startup and whenever its themes change.
/// # Safety
/// `names` must be readable for `length` bytes.
#[no_mangle]
pub unsafe extern "C" fn viem_set_theme_names(names: *const u8, length: u64) -> ViemStatus {
    ffi_boundary(|| {
        if length > 1024 * 1024 {
            return Err(ViemStatus::InvalidArgument);
        }
        let bytes = unsafe { input_bytes(names, length)? };
        let text = str::from_utf8(bytes).map_err(|_| ViemStatus::InvalidUtf8)?;
        let names = text.split('\0').filter(|name| !name.is_empty()).map(str::to_owned).collect();
        crate::command::options::set_theme_names(names);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_continuations_validate_regions_before_touching_core_state() {
        let mut outcome = ViemCoreOutcomeV1::default();
        unsafe {
            assert_eq!(
                viem_core_view_read_file(0, 0, 0, 0, 0, 0, std::ptr::null(), 1, &mut outcome),
                ViemStatus::NullPointer
            );
            let aliased = (&outcome as *const ViemCoreOutcomeV1).cast::<u8>();
            assert_eq!(
                viem_core_view_read_file(0, 0, 0, 0, 0, 0, aliased, 1, &mut outcome),
                ViemStatus::InvalidArgument
            );
            let mut effects = 0;
            assert_eq!(
                viem_core_view_execute_command_line(
                    0,
                    0,
                    std::ptr::null(),
                    1,
                    std::ptr::null(),
                    &mut outcome,
                    &mut effects
                ),
                ViemStatus::NullPointer
            );
        }
    }
}

/// Resume/cancel exactly the script token returned in the last effect batch.
/// Native I/O must complete outside the serial core lease. `success = 0`
/// discards the remaining script after cancellation/failure; a stale token
/// never consumes another pending script.
/// # Safety
/// Context and mutually disjoint outputs obey the command-turn contract.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_resume_script(
    handle: ViemCoreHandle, view: ViemViewId, token: u64, success: u32,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1, out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [typed_pointer_region(out_outcome, 1)?, typed_pointer_region(out_effects, 1)?];
        validate_disjoint_regions(&regions)?;
        let clipboard = unsafe { read_command_turn_context(context, &regions)? };
        unsafe { clear_outcome(out_outcome)?; out_effects.write(0); }
        if success > 1 || token == 0 { return Err(ViemStatus::InvalidArgument); }
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            let outcome = core.resume_script(ViewId(view), token, success != 0, Some(clipboard.clone())).map_err(core_status)?;
            export_script_outcome(core, view, outcome, &clipboard)
        })?;
        let effects = match effects { Some(effects) => reservation.commit(effects)?, None => 0 };
        unsafe { out_outcome.write(summary); out_effects.write(effects); }
        Ok(())
    })
}

/// Drain startup application effects after the first native view is ready.
/// # Safety
/// Outputs are valid, aligned and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_take_startup_effects(
    handle: ViemCoreHandle, view: ViemViewId,
    out_outcome: *mut ViemCoreOutcomeV1, out_effects: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[typed_pointer_region(out_outcome, 1)?, typed_pointer_region(out_effects, 1)?])?;
        unsafe { clear_outcome(out_outcome)?; out_effects.write(0); }
        let reservation = reserve_effect_batch()?;
        let (summary, effects) = with_core_mut(handle, |core| {
            let outcome = core.take_startup_effects(ViewId(view)).map_err(core_status)?;
            export_script_outcome(core, view, outcome, &ClipboardCommandContext::default())
        })?;
        let effects = match effects { Some(effects) => reservation.commit(effects)?, None => 0 };
        unsafe { out_outcome.write(summary); out_effects.write(effects); }
        Ok(())
    })
}

fn export_script_outcome(core: &Core<CTextMeasurementProvider>, view: ViemViewId,
    outcome: crate::coordinator::CoreOutcome, clipboard: &ClipboardCommandContext,
) -> Result<(ViemCoreOutcomeV1, Option<OwnedEffectBatch>), ViemStatus> {
    let id = ViewId(view);
    let mut summary = summarize_core_outcome(core, id, Some(&outcome))?;
    let effects = OwnedEffectBatch::from_command(core.document(),
        core.command_state(id).ok_or(ViemStatus::InvalidView)?, clipboard, outcome.command);
    if effects.is_some() { summary.flags |= VIEM_OUTCOME_HAS_EXTERNAL_EFFECTS; }
    Ok((summary, effects))
}
