//! Owned application file-mark snapshots and explicit native register inputs.
use super::*;
use crate::document::ArtifactPath;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ViemFileMarkTargetV1 {
    pub struct_size: u32,
    pub name: u32,
    pub generation: u64,
    pub document_id: u64,
    pub path_length: u64,
}

fn file_mark(name: u32, generation: u64) -> Result<crate::coordinator::FileMarkTarget, ViemStatus> {
    let name = char::from_u32(name)
        .filter(char::is_ascii_uppercase)
        .ok_or(ViemStatus::InvalidArgument)?;
    let registry = core_registry()
        .lock()
        .map_err(|_| ViemStatus::InternalError)?;
    let target = registry
        .file_marks
        .get(&name)
        .ok_or(ViemStatus::InvalidArgument)?;
    if generation != 0 && generation != target.generation {
        return Err(ViemStatus::StaleRevision);
    }
    if target.deleted {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(target.clone())
}

/// Read one bounded mark snapshot; generation zero queries the current owner.
/// # Safety
/// Writable info and path ranges must be valid, aligned and disjoint. A null
/// path is allowed only for a zero-capacity size query.
#[no_mangle]
pub unsafe extern "C" fn viem_file_mark_target(
    name: u32,
    generation: u64,
    out_info: *mut ViemFileMarkTargetV1,
    path: *mut u8,
    capacity: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(out_info, 1)?,
            typed_pointer_region(path, capacity)?,
        ])?;
        unsafe { out_info.write(ViemFileMarkTargetV1::default()) };
        let target = file_mark(name, generation)?;
        let bytes = target.path.as_ref().map_or(&[][..], |path| path.as_bytes());
        std::str::from_utf8(bytes).map_err(|_| ViemStatus::InvalidUtf8)?;
        unsafe {
            out_info.write(ViemFileMarkTargetV1 {
                struct_size: size_of::<ViemFileMarkTargetV1>() as u32,
                name,
                generation: target.generation,
                document_id: target.document.0,
                path_length: bytes.len() as u64,
            })
        };
        if capacity == 0 {
            return Ok(());
        }
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe { copy_output(bytes, path) };
        Ok(())
    })
}

/// Resolve a saved file location or the live rebased mark, then navigate using
/// the ordinary core cursor transaction. No synthetic keyboard input.
/// # Safety
/// `out_outcome` must be valid and aligned writable storage.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_jump_file_mark(
    handle: ViemCoreHandle,
    view: ViemViewId,
    name: u32,
    generation: u64,
    linewise: u32,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        unsafe { clear_outcome(out_outcome)? };
        if linewise > 1 || generation == 0 {
            return Err(ViemStatus::InvalidArgument);
        }
        let target = file_mark(name, generation)?;
        let outcome = with_core_mut(handle, |core| {
            let outcome = core
                .jump_file_mark(
                    ViewId(view),
                    char::from_u32(name).ok_or(ViemStatus::InvalidArgument)?,
                    &target,
                    linewise != 0,
                )
                .map_err(core_status)?;
            summarize_core_outcome(core, ViewId(view), Some(&outcome))
        })?;
        unsafe { out_outcome.write(outcome) };
        Ok(())
    })
}

/// Supply the view's alternate filename (`#`), or an accepted drop (`~`).
/// Empty filename clears the alternate. An empty drop is an empty register.
/// # Safety
/// Nonempty input must be readable for its declared length. It is copied.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_set_host_register(
    handle: ViemCoreHandle,
    view: ViemViewId,
    name: u32,
    text: *const u8,
    length: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let bytes = unsafe { input_bytes(text, length)? };
        let value = std::str::from_utf8(bytes)
            .map_err(|_| ViemStatus::InvalidUtf8)?
            .to_owned();
        with_core_mut(handle, |core| match name {
            35 => core
                .set_alternate_artifact(
                    ViewId(view),
                    (!value.is_empty()).then(|| ArtifactPath::from_utf8(value)),
                )
                .map_err(core_status),
            126 => core
                .set_drop_register(
                    ViewId(view),
                    crate::command::RegisterValue::characterwise(value),
                )
                .map_err(core_status),
            _ => Err(ViemStatus::InvalidArgument),
        })
    })
}

/// Paste an explicit native text drop and retain it in `~` only after success.
/// Clipboard writes are returned through the ordinary owned effect batch.
/// # Safety
/// All inputs must be readable and outputs writable, aligned and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_drop_text(
    handle: ViemCoreHandle,
    view: ViemViewId,
    text: *const u8,
    length: u64,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effect_batch: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let outputs = [
            typed_pointer_region(out_outcome, 1)?,
            typed_pointer_region(out_effect_batch, 1)?,
        ];
        validate_disjoint_regions(&[outputs[0], outputs[1], typed_pointer_region(text, length)?])?;
        let value = std::str::from_utf8(unsafe { input_bytes(text, length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?
            .to_owned();
        let clipboard = unsafe { read_command_turn_context(context, &outputs)? }.with_read(
            ClipboardSnapshot::new(
                ClipboardTarget::Clipboard,
                ClipboardGeneration(1),
                ClipboardContent::from_plain_text(&value),
            ),
        );
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let reservation = reserve_effect_batch()?;
        let (outcome, effects) = with_core_mut(handle, |core| {
            let result = dispatch_input_with_effects(
                core,
                view,
                InputEvent::Key(Key::PasteClipboard),
                clipboard,
            )?;
            if result.0.command_status == VIEM_COMMAND_STATUS_COMPLETE
                || result.0.flags & VIEM_OUTCOME_DOCUMENT_CHANGED != 0
            {
                core.set_drop_register(
                    ViewId(view),
                    crate::command::RegisterValue::characterwise(&value),
                )
                .map_err(core_status)?;
            }
            Ok(result)
        })?;
        let effect_batch = match effects {
            Some(effects) => reservation.commit(effects)?,
            None => {
                drop(reservation);
                0
            }
        };
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_batch);
        }
        Ok(())
    })
}
