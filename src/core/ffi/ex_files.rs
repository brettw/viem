//! Revision-bound host file continuations. Host I/O never runs under a core lease.
use super::*;

/// Insert host-loaded file bytes after the addressed hard line (zero: before
/// the first line). Decoding and linewise edit semantics are owned by core.
/// # Safety
/// Input/output regions must be valid for their sizes and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_read_file(
    handle: ViemCoreHandle,
    view: ViemViewId,
    document: u64,
    revision: u64,
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
        let after = usize::try_from(after).map_err(|_| ViemStatus::LengthOverflow)?;
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

/// Execute one sourced Ex line through the normal command pipeline. Blank and
/// comment lines are no-ops. `depth` is the host's active source stack depth.
/// # Safety
/// Input/context and disjoint outputs obey the ordinary command-turn contract.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_source_line(
    handle: ViemCoreHandle,
    view: ViemViewId,
    depth: u32,
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
            if !core
                .queue_sourced_line(ViewId(view), text, depth)
                .map_err(|_| ViemStatus::InvalidArgument)?
            {
                return Ok((summarize_core_outcome(core, ViewId(view), None)?, None));
            }
            let result =
                dispatch_input_with_effects(core, view, InputEvent::Key(Key::Enter), clipboard);
            core.finish_sourced_line(ViewId(view));
            result
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_continuations_validate_regions_before_touching_core_state() {
        let mut outcome = ViemCoreOutcomeV1::default();
        unsafe {
            assert_eq!(
                viem_core_view_read_file(0, 0, 0, 0, 0, std::ptr::null(), 1, &mut outcome),
                ViemStatus::NullPointer
            );
            let aliased = (&outcome as *const ViemCoreOutcomeV1).cast::<u8>();
            assert_eq!(
                viem_core_view_read_file(0, 0, 0, 0, 0, aliased, 1, &mut outcome),
                ViemStatus::InvalidArgument
            );
            let mut effects = 0;
            assert_eq!(
                viem_core_view_source_line(
                    0,
                    0,
                    1,
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    &mut outcome,
                    &mut effects
                ),
                ViemStatus::NullPointer
            );
        }
    }
}
